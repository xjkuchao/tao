//! Vorbis 音频解码器.
//!
//! 使用 lewton 纯 Rust 库进行 Vorbis 音频解码.
//!
//! Vorbis 是一种开放的有损音频编码格式, 常用于 Ogg 容器中.
//!
//! 解码流程:
//! 1. 在 `open()` 中从 extra_data 解析 identification header
//! 2. 前两个 packet 为 comment header 和 setup header, 按序解析
//! 3. 后续 packet 为音频数据, 调用 lewton 解码为 S16 PCM 采样
//! 4. 输出交错格式的音频帧

use log::debug;
use tao_core::{ChannelLayout, Rational, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

/// Vorbis 解码器初始化阶段
enum InitPhase {
    /// 等待 comment header (第一个 send_packet 调用)
    WaitComment,
    /// 等待 setup header (第二个 send_packet 调用)
    WaitSetup,
    /// 所有头部已解析, 可以解码音频
    Ready,
}

/// Vorbis 解码器
///
/// 封装 lewton 纯 Rust Vorbis 解码库, 通过 Decoder trait 提供统一接口.
/// 三个 Vorbis 头部 (identification, comment, setup) 分别通过
/// `open()` 的 extra_data 和前两次 `send_packet()` 接收.
pub struct VorbisDecoder {
    /// 初始化阶段追踪
    phase: InitPhase,
    /// identification header (在 open() 中解析)
    ident: Option<lewton::header::IdentHeader>,
    /// setup header (从第二个数据包解析)
    setup: Option<lewton::header::SetupHeader>,
    /// MDCT 窗口重叠状态 (解码连续音频包所需)
    pwr: Option<lewton::audio::PreviousWindowRight>,
    /// 待输出的解码帧
    output_frame: Option<Frame>,
    /// PTS 追踪 (累积采样数)
    next_pts: i64,
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 是否已通过 open() 初始化
    opened: bool,
}

impl VorbisDecoder {
    /// 创建 Vorbis 解码器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            phase: InitPhase::WaitComment,
            ident: None,
            setup: None,
            pwr: None,
            output_frame: None,
            next_pts: 0,
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
            opened: false,
        }))
    }

    /// 检查数据是否为 Vorbis 头部包
    ///
    /// Vorbis 头部包以奇数类型字节开头, 后跟 "vorbis" 签名.
    fn is_header_packet(data: &[u8]) -> bool {
        data.len() >= 7 && data[0] & 1 == 1 && &data[1..7] == b"vorbis"
    }

    /// 将 lewton 解码结果 (每声道 i16) 转换为交错 S16 AudioFrame
    fn build_frame(
        decoded: &[Vec<i16>],
        pts: i64,
        sample_rate: u32,
        channel_layout: ChannelLayout,
    ) -> AudioFrame {
        let channels = decoded.len();
        let nb_samples = if channels > 0 { decoded[0].len() } else { 0 };

        // 构建交错 S16 数据: L0 R0 L1 R1 ...
        let mut interleaved = Vec::with_capacity(nb_samples * channels * 2);
        for i in 0..nb_samples {
            for ch in decoded.iter().take(channels) {
                let sample = ch[i];
                interleaved.extend_from_slice(&sample.to_le_bytes());
            }
        }

        let mut af = AudioFrame::new(
            nb_samples as u32,
            sample_rate,
            SampleFormat::S16,
            channel_layout,
        );
        af.data = vec![interleaved];
        af.pts = pts;
        af.time_base = Rational::new(1, sample_rate as i32);
        af.duration = nb_samples as i64;

        af
    }
}

impl Decoder for VorbisDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Vorbis
    }

    fn name(&self) -> &str {
        "vorbis"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        // 从 extra_data 解析 identification header
        let extra = &params.extra_data;
        if extra.is_empty() {
            return Err(TaoError::InvalidData(
                "Vorbis: extra_data 为空, 缺少 identification header".into(),
            ));
        }

        let ident = lewton::header::read_header_ident(extra).map_err(|e| {
            TaoError::Codec(format!("Vorbis: 解析 identification header 失败: {e:?}"))
        })?;

        self.sample_rate = ident.audio_sample_rate;
        self.channels = ident.audio_channels as u32;
        self.channel_layout = ChannelLayout::from_channels(self.channels);

        debug!(
            "Vorbis: 采样率={}, 声道数={}, 块大小=({}, {})",
            ident.audio_sample_rate, ident.audio_channels, ident.blocksize_0, ident.blocksize_1,
        );

        self.ident = Some(ident);
        self.phase = InitPhase::WaitComment;
        self.opened = true;
        self.next_pts = 0;
        self.output_frame = None;

        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("Vorbis 解码器未打开".into()));
        }

        let data = &packet.data;
        if data.is_empty() {
            // flush 空包
            return Ok(());
        }

        match self.phase {
            InitPhase::WaitComment => {
                // 解析 comment header
                lewton::header::read_header_comment(data).map_err(|e| {
                    TaoError::Codec(format!("Vorbis: 解析 comment header 失败: {e:?}"))
                })?;
                debug!("Vorbis: comment header 已解析");
                self.phase = InitPhase::WaitSetup;
                Ok(())
            }
            InitPhase::WaitSetup => {
                // 解析 setup header
                let ident = self.ident.as_ref().ok_or_else(|| {
                    TaoError::Codec("Vorbis: identification header 未初始化".into())
                })?;
                let setup = lewton::header::read_header_setup(
                    data,
                    ident.audio_channels,
                    (ident.blocksize_0, ident.blocksize_1),
                )
                .map_err(|e| TaoError::Codec(format!("Vorbis: 解析 setup header 失败: {e:?}")))?;
                debug!("Vorbis: setup header 已解析, 解码器就绪");
                self.setup = Some(setup);
                self.pwr = Some(lewton::audio::PreviousWindowRight::new());
                self.phase = InitPhase::Ready;
                Ok(())
            }
            InitPhase::Ready => {
                // 跳过任何意外的头部包
                if Self::is_header_packet(data) {
                    debug!("Vorbis: 跳过额外的头部包 (类型={})", data[0]);
                    return Ok(());
                }

                // 解码音频数据包
                let ident = self.ident.as_ref().unwrap();
                let setup = self.setup.as_ref().unwrap();
                let decoded_result = {
                    let pwr = self.pwr.as_mut().unwrap();
                    lewton::audio::read_audio_packet(ident, setup, data, pwr)
                };

                match decoded_result {
                    Ok(decoded) => {
                        // decoded: Vec<Vec<i16>>, 每声道一个 Vec
                        let nb_samples = if !decoded.is_empty() {
                            decoded[0].len()
                        } else {
                            0
                        };

                        if nb_samples > 0 {
                            let af = Self::build_frame(
                                &decoded,
                                self.next_pts,
                                self.sample_rate,
                                self.channel_layout,
                            );
                            self.next_pts += nb_samples as i64;
                            self.output_frame = Some(Frame::Audio(af));
                        }
                        // nb_samples == 0 时 (如首个音频包) 不输出帧
                        Ok(())
                    }
                    Err(lewton::audio::AudioReadError::AudioIsHeader) => {
                        debug!("Vorbis: 音频包被识别为头部, 已跳过");
                        // 该错误通常表示遇到链式 bitstream 的头部或损坏包.
                        // 重置窗口状态, 防止坏包污染后续重叠计算.
                        self.pwr = Some(lewton::audio::PreviousWindowRight::new());
                        Ok(())
                    }
                    Err(e) => {
                        debug!("Vorbis: 解码音频包失败: {e:?}, 已重置窗口状态");
                        // 对坏包执行软恢复, 尽快恢复后续可解码音频.
                        self.pwr = Some(lewton::audio::PreviousWindowRight::new());
                        Ok(())
                    }
                }
            }
        }
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        self.output_frame.take().ok_or(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.next_pts = 0;
        // 重建窗口重叠状态
        if self.pwr.is_some() {
            self.pwr = Some(lewton::audio::PreviousWindowRight::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};

    /// 构造 Vorbis identification header
    fn make_ident_header() -> Vec<u8> {
        let mut data = Vec::new();
        data.push(1u8); // packet type = identification
        data.extend_from_slice(b"vorbis"); // codec 标识
        data.extend_from_slice(&0u32.to_le_bytes()); // version = 0
        data.push(2); // channels = 2
        data.extend_from_slice(&44100u32.to_le_bytes()); // sample_rate
        data.extend_from_slice(&0i32.to_le_bytes()); // bitrate_max
        data.extend_from_slice(&128000i32.to_le_bytes()); // bitrate_nominal
        data.extend_from_slice(&0i32.to_le_bytes()); // bitrate_min
        data.push(0x88); // blocksize_0=8, blocksize_1=8 (2^8=256)
        data.push(1); // framing bit
        data
    }

    fn default_params() -> CodecParameters {
        CodecParameters {
            codec_id: CodecId::Vorbis,
            bit_rate: 0,
            extra_data: make_ident_header(),
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::S16,
                frame_size: 0,
            }),
        }
    }

    #[test]
    fn test_创建解码器() {
        let decoder = VorbisDecoder::create();
        assert!(decoder.is_ok());
        let decoder = decoder.unwrap();
        assert_eq!(decoder.codec_id(), CodecId::Vorbis);
        assert_eq!(decoder.name(), "vorbis");
    }

    #[test]
    fn test_打开解码器() {
        let mut decoder = VorbisDecoder::create().unwrap();
        let result = decoder.open(&default_params());
        assert!(result.is_ok());
    }

    #[test]
    fn test_空extra_data报错() {
        let mut decoder = VorbisDecoder::create().unwrap();
        let params = CodecParameters {
            codec_id: CodecId::Vorbis,
            bit_rate: 0,
            extra_data: Vec::new(),
            params: CodecParamsType::None,
        };
        assert!(decoder.open(&params).is_err());
    }

    #[test]
    fn test_未打开发包报错() {
        let mut decoder = VorbisDecoder::create().unwrap();
        let packet = Packet::from_data(vec![0u8; 10]);
        assert!(decoder.send_packet(&packet).is_err());
    }

    #[test]
    fn test_flush不崩溃() {
        let mut decoder = VorbisDecoder::create().unwrap();
        decoder.open(&default_params()).unwrap();
        decoder.flush();
        assert!(decoder.receive_frame().is_err());
    }

    #[test]
    fn test_识别头部包() {
        // 类型 1 (identification)
        let mut data = vec![1u8];
        data.extend_from_slice(b"vorbis");
        assert!(VorbisDecoder::is_header_packet(&data));

        // 类型 3 (comment)
        let mut data = vec![3u8];
        data.extend_from_slice(b"vorbis");
        assert!(VorbisDecoder::is_header_packet(&data));

        // 类型 5 (setup)
        let mut data = vec![5u8];
        data.extend_from_slice(b"vorbis");
        assert!(VorbisDecoder::is_header_packet(&data));

        // 非头部包 (偶数类型)
        let mut data = vec![0u8];
        data.extend_from_slice(b"vorbis");
        assert!(!VorbisDecoder::is_header_packet(&data));
    }
}
