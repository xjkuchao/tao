//! PCM 音频解码器.
//!
//! 将未压缩的 PCM 数据从 Packet 转换为 AudioFrame.
//! 支持 6 种 PCM 变体, 共用解码逻辑.

use log::debug;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

/// PCM 格式描述, 描述各 PCM 变体的差异
struct PcmFormatDesc {
    /// 编解码器 ID
    codec_id: CodecId,
    /// 码流中每个样本的字节数
    bytes_per_sample: u32,
    /// 输出的采样格式
    output_format: SampleFormat,
    /// 解码转换函数: 将码流字节转换为输出格式字节
    decode_fn: fn(&[u8], &mut Vec<u8>),
}

/// 直接拷贝
fn decode_copy(src: &[u8], dst: &mut Vec<u8>) {
    dst.extend_from_slice(src);
}

/// S16 大端转小端: 每 2 字节翻转
fn decode_s16be(src: &[u8], dst: &mut Vec<u8>) {
    for chunk in src.chunks_exact(2) {
        dst.push(chunk[1]);
        dst.push(chunk[0]);
    }
}

/// S24LE 符号扩展到 S32: 3 字节 -> 4 字节
fn decode_s24le(src: &[u8], dst: &mut Vec<u8>) {
    for chunk in src.chunks_exact(3) {
        // 24 位小端: [低字节, 中字节, 高字节]
        // 检查高字节的符号位 (bit 7), 扩展到第 4 字节
        let sign_ext = if chunk[2] & 0x80 != 0 { 0xFF } else { 0x00 };
        dst.push(chunk[0]);
        dst.push(chunk[1]);
        dst.push(chunk[2]);
        dst.push(sign_ext);
    }
}

/// 获取指定 CodecId 的 PCM 格式描述
fn get_pcm_format_desc(codec_id: CodecId) -> Option<PcmFormatDesc> {
    Some(match codec_id {
        CodecId::PcmU8 => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 1,
            output_format: SampleFormat::U8,
            decode_fn: decode_copy,
        },
        CodecId::PcmS16le => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 2,
            output_format: SampleFormat::S16,
            decode_fn: decode_copy,
        },
        CodecId::PcmS16be => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 2,
            output_format: SampleFormat::S16,
            decode_fn: decode_s16be,
        },
        CodecId::PcmS24le => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 3,
            output_format: SampleFormat::S32,
            decode_fn: decode_s24le,
        },
        CodecId::PcmS32le => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 4,
            output_format: SampleFormat::S32,
            decode_fn: decode_copy,
        },
        CodecId::PcmF32le => PcmFormatDesc {
            codec_id,
            bytes_per_sample: 4,
            output_format: SampleFormat::F32,
            decode_fn: decode_copy,
        },
        _ => return None,
    })
}

/// PCM 音频解码器
pub struct PcmDecoder {
    /// 格式描述
    desc: PcmFormatDesc,
    /// 采样率
    sample_rate: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 每帧采样数 (0 表示可变, 由数据包大小决定)
    frame_size: u32,
    /// 每个样本块的字节数 (每样本字节数 * 声道数)
    block_align: u32,
    /// 已解码帧缓冲
    output_frame: Option<Frame>,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
}

impl PcmDecoder {
    /// 创建指定 PCM 变体的解码器工厂函数
    fn create(codec_id: CodecId) -> TaoResult<Box<dyn Decoder>> {
        let desc = get_pcm_format_desc(codec_id)
            .ok_or_else(|| TaoError::CodecNotFound(format!("不支持的 PCM 格式: {}", codec_id)))?;
        Ok(Box::new(Self {
            desc,
            sample_rate: 0,
            channel_layout: ChannelLayout::MONO,
            frame_size: 0,
            block_align: 0,
            output_frame: None,
            opened: false,
            flushing: false,
        }))
    }

    pub fn new_u8() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmU8)
    }

    pub fn new_s16le() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmS16le)
    }

    pub fn new_s16be() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmS16be)
    }

    pub fn new_s24le() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmS24le)
    }

    pub fn new_s32le() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmS32le)
    }

    pub fn new_f32le() -> TaoResult<Box<dyn Decoder>> {
        Self::create(CodecId::PcmF32le)
    }
}

impl Decoder for PcmDecoder {
    fn codec_id(&self) -> CodecId {
        self.desc.codec_id
    }

    fn name(&self) -> &str {
        self.desc.codec_id.name()
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let audio = match &params.params {
            CodecParamsType::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("PCM 解码器需要音频参数".into()));
            }
        };

        if audio.sample_rate == 0 {
            return Err(TaoError::InvalidArgument("采样率不能为 0".into()));
        }
        if audio.channel_layout.channels == 0 {
            return Err(TaoError::InvalidArgument("声道数不能为 0".into()));
        }

        self.sample_rate = audio.sample_rate;
        self.channel_layout = audio.channel_layout;
        self.frame_size = audio.frame_size;
        self.block_align = self.desc.bytes_per_sample * audio.channel_layout.channels;
        self.output_frame = None;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 {} 解码器: {} Hz, {} 声道, 输出格式={}",
            self.name(),
            self.sample_rate,
            self.channel_layout.channels,
            self.desc.output_format,
        );
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开, 请先调用 open()".into()));
        }
        if self.output_frame.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        // 空包 = flush
        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }

        let data_len = packet.data.len() as u32;
        if data_len % self.block_align != 0 {
            return Err(TaoError::InvalidData(format!(
                "数据大小 {} 不是 block_align {} 的整数倍",
                data_len, self.block_align,
            )));
        }

        let nb_samples = data_len / self.block_align;
        let mut frame = AudioFrame::new(
            nb_samples,
            self.sample_rate,
            self.desc.output_format,
            self.channel_layout,
        );
        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        // 解码到交错格式 (单平面)
        let output_sample_bytes = self.desc.output_format.bytes_per_sample();
        let output_size = nb_samples as usize
            * self.channel_layout.channels as usize
            * output_sample_bytes as usize;
        let mut decoded = Vec::with_capacity(output_size);
        (self.desc.decode_fn)(&packet.data, &mut decoded);
        frame.data[0] = decoded;

        self.output_frame = Some(Frame::Audio(frame));
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_frame.take() {
            return Ok(frame);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.flushing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::AudioCodecParams;
    use bytes::Bytes;
    use tao_core::Rational;

    fn make_audio_params(codec_id: CodecId, channels: u32) -> CodecParameters {
        CodecParameters {
            codec_id,
            extra_data: Vec::new(),
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::None,
                frame_size: 0,
            }),
        }
    }

    #[test]
    fn test_pcm_u8_decode() {
        let mut dec = PcmDecoder::new_u8().unwrap();
        dec.open(&make_audio_params(CodecId::PcmU8, 1)).unwrap();

        let data = vec![128u8, 64, 192, 255];
        let pkt = Packet::from_data(Bytes::from(data.clone()));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 4);
                assert_eq!(af.sample_format, SampleFormat::U8);
                assert_eq!(af.data[0], data);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_pcm_s16le_decode() {
        let mut dec = PcmDecoder::new_s16le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS16le, 2)).unwrap();

        // 2 声道, 2 采样 -> 8 字节
        let data = vec![0x00, 0x01, 0xFF, 0x7F, 0x00, 0x80, 0x01, 0x00];
        let pkt = Packet::from_data(Bytes::from(data.clone()));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 2);
                assert_eq!(af.sample_format, SampleFormat::S16);
                assert_eq!(af.data[0], data); // 直接拷贝
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_pcm_s16be_byte_order_swap() {
        let mut dec = PcmDecoder::new_s16be().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS16be, 1)).unwrap();

        // 大端: [0x01, 0x00] -> 小端: [0x00, 0x01]
        let data = vec![0x01, 0x00, 0x7F, 0xFF];
        let pkt = Packet::from_data(Bytes::from(data));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.sample_format, SampleFormat::S16);
                assert_eq!(af.data[0], vec![0x00, 0x01, 0xFF, 0x7F]);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_pcm_s24le_sign_extend() {
        let mut dec = PcmDecoder::new_s24le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS24le, 1)).unwrap();

        // 正数: [0x56, 0x34, 0x12] -> [0x56, 0x34, 0x12, 0x00]
        // 负数: [0x00, 0x00, 0x80] -> [0x00, 0x00, 0x80, 0xFF]
        let data = vec![0x56, 0x34, 0x12, 0x00, 0x00, 0x80];
        let pkt = Packet::from_data(Bytes::from(data));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 2);
                assert_eq!(af.sample_format, SampleFormat::S32);
                assert_eq!(
                    af.data[0],
                    vec![0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x80, 0xFF]
                );
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_pcm_s32le_decode() {
        let mut dec = PcmDecoder::new_s32le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS32le, 1)).unwrap();

        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let pkt = Packet::from_data(Bytes::from(data.clone()));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 2);
                assert_eq!(af.sample_format, SampleFormat::S32);
                assert_eq!(af.data[0], data);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_pcm_f32le_decode() {
        let mut dec = PcmDecoder::new_f32le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmF32le, 1)).unwrap();

        // f32: 1.0 = [0x00, 0x00, 0x80, 0x3F]
        let data = vec![0x00, 0x00, 0x80, 0x3F];
        let pkt = Packet::from_data(Bytes::from(data.clone()));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 1);
                assert_eq!(af.sample_format, SampleFormat::F32);
                assert_eq!(af.data[0], data);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_not_open_error() {
        let mut dec = PcmDecoder::new_s16le().unwrap();
        let pkt = Packet::from_data(Bytes::from(vec![0u8; 4]));
        let err = dec.send_packet(&pkt).unwrap_err();
        assert!(matches!(err, TaoError::Codec(_)));
    }

    #[test]
    fn test_data_alignment_error() {
        let mut dec = PcmDecoder::new_s16le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS16le, 2)).unwrap();
        // block_align = 2 * 2 = 4, 但数据大小为 3
        let pkt = Packet::from_data(Bytes::from(vec![0u8; 3]));
        let err = dec.send_packet(&pkt).unwrap_err();
        assert!(matches!(err, TaoError::InvalidData(_)));
    }

    #[test]
    fn test_flush_and_eof() {
        let mut dec = PcmDecoder::new_u8().unwrap();
        dec.open(&make_audio_params(CodecId::PcmU8, 1)).unwrap();

        dec.send_packet(&Packet::empty()).unwrap();
        let err = dec.receive_frame().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_stereo_decode() {
        let mut dec = PcmDecoder::new_s16le().unwrap();
        dec.open(&make_audio_params(CodecId::PcmS16le, 2)).unwrap();

        // 2 声道, 每声道 1 采样 -> 4 字节
        let data = vec![0x01, 0x00, 0x02, 0x00];
        let mut pkt = Packet::from_data(Bytes::from(data.clone()));
        pkt.pts = 100;
        pkt.time_base = Rational::new(1, 44100);
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 1);
                assert_eq!(af.channel_layout.channels, 2);
                assert_eq!(af.pts, 100);
                assert_eq!(af.data[0], data);
            }
            _ => panic!("期望音频帧"),
        }
    }
}
