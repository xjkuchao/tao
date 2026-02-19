//! PCM 音频编码器.
//!
//! 将 AudioFrame 的采样数据转换为 Packet.
//! 支持 6 种 PCM 变体, 共用编码逻辑.

use bytes::Bytes;
use log::debug;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::encoder::Encoder;
use crate::frame::Frame;
use crate::packet::Packet;

/// PCM 编码格式描述
struct PcmEncodeDesc {
    /// 编解码器 ID
    codec_id: CodecId,
    /// 期望的输入采样格式
    input_format: SampleFormat,
    /// 输出码流中每个样本的字节数
    bytes_per_sample: u32,
    /// 编码转换函数
    encode_fn: fn(&[u8], &mut Vec<u8>),
}

/// 直接拷贝
fn encode_copy(src: &[u8], dst: &mut Vec<u8>) {
    dst.extend_from_slice(src);
}

/// S16 小端转大端: 每 2 字节翻转
fn encode_s16be(src: &[u8], dst: &mut Vec<u8>) {
    for chunk in src.chunks_exact(2) {
        dst.push(chunk[1]);
        dst.push(chunk[0]);
    }
}

/// S32 截断为 S24LE: 4 字节 -> 3 字节 (取低 3 字节)
fn encode_s24le(src: &[u8], dst: &mut Vec<u8>) {
    for chunk in src.chunks_exact(4) {
        dst.push(chunk[0]);
        dst.push(chunk[1]);
        dst.push(chunk[2]);
    }
}

/// 获取指定 CodecId 的 PCM 编码格式描述
fn get_pcm_encode_desc(codec_id: CodecId) -> Option<PcmEncodeDesc> {
    Some(match codec_id {
        CodecId::PcmU8 => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::U8,
            bytes_per_sample: 1,
            encode_fn: encode_copy,
        },
        CodecId::PcmS16le => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::S16,
            bytes_per_sample: 2,
            encode_fn: encode_copy,
        },
        CodecId::PcmS16be => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::S16,
            bytes_per_sample: 2,
            encode_fn: encode_s16be,
        },
        CodecId::PcmS24le => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::S32,
            bytes_per_sample: 3,
            encode_fn: encode_s24le,
        },
        CodecId::PcmS32le => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::S32,
            bytes_per_sample: 4,
            encode_fn: encode_copy,
        },
        CodecId::PcmF32le => PcmEncodeDesc {
            codec_id,
            input_format: SampleFormat::F32,
            bytes_per_sample: 4,
            encode_fn: encode_copy,
        },
        _ => return None,
    })
}

/// PCM 音频编码器
pub struct PcmEncoder {
    /// 编码格式描述
    desc: PcmEncodeDesc,
    /// 采样率
    sample_rate: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 输出数据包缓冲
    output_packet: Option<Packet>,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
}

impl PcmEncoder {
    fn create(codec_id: CodecId) -> TaoResult<Box<dyn Encoder>> {
        let desc = get_pcm_encode_desc(codec_id)
            .ok_or_else(|| TaoError::CodecNotFound(format!("不支持的 PCM 格式: {}", codec_id)))?;
        Ok(Box::new(Self {
            desc,
            sample_rate: 0,
            channel_layout: ChannelLayout::MONO,
            output_packet: None,
            opened: false,
            flushing: false,
        }))
    }

    pub fn new_u8() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmU8)
    }

    pub fn new_s16le() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmS16le)
    }

    pub fn new_s16be() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmS16be)
    }

    pub fn new_s24le() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmS24le)
    }

    pub fn new_s32le() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmS32le)
    }

    pub fn new_f32le() -> TaoResult<Box<dyn Encoder>> {
        Self::create(CodecId::PcmF32le)
    }
}

impl Encoder for PcmEncoder {
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
                return Err(TaoError::InvalidArgument("PCM 编码器需要音频参数".into()));
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
        self.output_packet = None;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 {} 编码器: {} Hz, {} 声道, 输入格式={}",
            self.name(),
            self.sample_rate,
            self.channel_layout.channels,
            self.desc.input_format,
        );
        Ok(())
    }

    fn send_frame(&mut self, frame: Option<&Frame>) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("编码器未打开, 请先调用 open()".into()));
        }
        if self.output_packet.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        let frame = match frame {
            Some(f) => f,
            None => {
                self.flushing = true;
                return Ok(());
            }
        };

        let audio = match frame {
            Frame::Audio(a) => a,
            Frame::Video(_) => {
                return Err(TaoError::InvalidArgument("PCM 编码器不接受视频帧".into()));
            }
        };

        if audio.sample_format != self.desc.input_format {
            return Err(TaoError::InvalidArgument(format!(
                "期望采样格式 {}, 实际为 {}",
                self.desc.input_format, audio.sample_format,
            )));
        }

        // 编码: 交错格式音频数据在 data[0] 中
        let output_size = audio.nb_samples as usize
            * self.channel_layout.channels as usize
            * self.desc.bytes_per_sample as usize;
        let mut encoded = Vec::with_capacity(output_size);
        (self.desc.encode_fn)(&audio.data[0], &mut encoded);

        let mut pkt = Packet::from_data(Bytes::from(encoded));
        pkt.pts = audio.pts;
        pkt.dts = audio.pts;
        pkt.duration = audio.duration;
        pkt.time_base = audio.time_base;
        pkt.is_keyframe = true;

        self.output_packet = Some(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> TaoResult<Packet> {
        if let Some(pkt) = self.output_packet.take() {
            return Ok(pkt);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_packet = None;
        self.flushing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::AudioCodecParams;
    use crate::frame::AudioFrame;
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
    fn test_pcm_u8_encode() {
        let mut enc = PcmEncoder::new_u8().unwrap();
        enc.open(&make_audio_params(CodecId::PcmU8, 1)).unwrap();

        let data = vec![128u8, 64, 192, 255];
        let mut af = AudioFrame::new(4, 44100, SampleFormat::U8, ChannelLayout::MONO);
        af.data[0] = data.clone();

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(&pkt.data[..], &data[..]);
    }

    #[test]
    fn test_pcm_s16be_little_to_big_endian() {
        let mut enc = PcmEncoder::new_s16be().unwrap();
        enc.open(&make_audio_params(CodecId::PcmS16be, 1)).unwrap();

        // 小端输入: [0x00, 0x01] -> 大端输出: [0x01, 0x00]
        let data = vec![0x00, 0x01, 0xFF, 0x7F];
        let mut af = AudioFrame::new(2, 44100, SampleFormat::S16, ChannelLayout::MONO);
        af.data[0] = data;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(&pkt.data[..], &[0x01, 0x00, 0x7F, 0xFF]);
    }

    #[test]
    fn test_pcm_s24le_truncate() {
        let mut enc = PcmEncoder::new_s24le().unwrap();
        enc.open(&make_audio_params(CodecId::PcmS24le, 1)).unwrap();

        // S32 输入: [0x56, 0x34, 0x12, 0x00] -> S24 输出: [0x56, 0x34, 0x12]
        let data = vec![0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x80, 0xFF];
        let mut af = AudioFrame::new(2, 44100, SampleFormat::S32, ChannelLayout::MONO);
        af.data[0] = data;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(&pkt.data[..], &[0x56, 0x34, 0x12, 0x00, 0x00, 0x80]);
    }

    #[test]
    fn test_not_open_error() {
        let mut enc = PcmEncoder::new_s16le().unwrap();
        let af = AudioFrame::new(1, 44100, SampleFormat::S16, ChannelLayout::MONO);
        let err = enc.send_frame(Some(&Frame::Audio(af))).unwrap_err();
        assert!(matches!(err, TaoError::Codec(_)));
    }

    #[test]
    fn test_flush_and_eof() {
        let mut enc = PcmEncoder::new_u8().unwrap();
        enc.open(&make_audio_params(CodecId::PcmU8, 1)).unwrap();
        enc.send_frame(None).unwrap();
        let err = enc.receive_packet().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_codec_roundtrip_s16le() {
        use crate::decoders::pcm::PcmDecoder;

        let params = make_audio_params(CodecId::PcmS16le, 2);

        // 编码
        let mut enc = PcmEncoder::new_s16le().unwrap();
        enc.open(&params).unwrap();

        let original = vec![0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00];
        let mut af = AudioFrame::new(2, 44100, SampleFormat::S16, ChannelLayout::STEREO);
        af.data[0] = original.clone();
        af.pts = 42;
        af.time_base = Rational::new(1, 44100);

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        // 解码
        let mut dec = PcmDecoder::new_s16le().unwrap();
        dec.open(&params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(decoded) => {
                assert_eq!(decoded.data[0], original);
                assert_eq!(decoded.pts, 42);
                assert_eq!(decoded.nb_samples, 2);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_codec_roundtrip_s16be() {
        use crate::decoders::pcm::PcmDecoder;

        let params = make_audio_params(CodecId::PcmS16be, 1);

        // 编码: 小端 S16 -> 大端数据包
        let mut enc = PcmEncoder::new_s16be().unwrap();
        enc.open(&params).unwrap();

        let original = vec![0x34, 0x12, 0x78, 0x56]; // 小端 S16: 0x1234, 0x5678
        let mut af = AudioFrame::new(2, 44100, SampleFormat::S16, ChannelLayout::MONO);
        af.data[0] = original.clone();

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        // 解码: 大端数据包 -> 小端 S16
        let mut dec = PcmDecoder::new_s16be().unwrap();
        dec.open(&params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(decoded) => {
                assert_eq!(decoded.data[0], original);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_codec_roundtrip_s24le() {
        use crate::decoders::pcm::PcmDecoder;

        let params = make_audio_params(CodecId::PcmS24le, 1);

        // 编码: S32 -> S24LE 数据包
        let mut enc = PcmEncoder::new_s24le().unwrap();
        enc.open(&params).unwrap();

        // 正数: 0x00123456 -> 截断为 24 位 -> 符号扩展回 0x00123456
        let input_s32 = vec![0x56, 0x34, 0x12, 0x00];
        let mut af = AudioFrame::new(1, 44100, SampleFormat::S32, ChannelLayout::MONO);
        af.data[0] = input_s32.clone();

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(&pkt.data[..], &[0x56, 0x34, 0x12]);

        // 解码
        let mut dec = PcmDecoder::new_s24le().unwrap();
        dec.open(&params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(decoded) => {
                assert_eq!(decoded.data[0], input_s32);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_sample_format_mismatch_error() {
        let mut enc = PcmEncoder::new_s16le().unwrap();
        enc.open(&make_audio_params(CodecId::PcmS16le, 1)).unwrap();

        // 发送 U8 格式帧给 S16LE 编码器
        let af = AudioFrame::new(1, 44100, SampleFormat::U8, ChannelLayout::MONO);
        let err = enc.send_frame(Some(&Frame::Audio(af))).unwrap_err();
        assert!(matches!(err, TaoError::InvalidArgument(_)));
    }
}
