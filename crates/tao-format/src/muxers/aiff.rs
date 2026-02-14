//! AIFF 封装器.
//!
//! 将 PCM 音频数据写入标准 AIFF 文件 (大端 PCM).
//!
//! 写入流程:
//! 1. `write_header()` - 写入 FORM, COMM, SSND 头部, 预留大小字段
//! 2. `write_packet()` - 追加 PCM 数据
//! 3. `write_trailer()` - 回填 FORM 大小和 SSND 大小

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// AIFF 封装器
pub struct AiffMuxer {
    /// FORM 大小字段的文件偏移
    form_size_offset: u64,
    /// SSND chunk 大小字段的文件偏移
    ssnd_size_offset: u64,
    /// COMM chunk 中 numSampleFrames 的文件偏移
    num_frames_offset: u64,
    /// 已写入的音频数据字节数
    data_written: u64,
    /// 块对齐 (每个采样帧的字节数)
    block_align: u16,
}

impl AiffMuxer {
    /// 创建 AIFF 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            form_size_offset: 0,
            ssnd_size_offset: 0,
            num_frames_offset: 0,
            data_written: 0,
            block_align: 0,
        }))
    }

    /// 根据 CodecId 确定位深 (AIFF 只支持大端 PCM)
    fn resolve_bits_per_sample(codec_id: CodecId) -> TaoResult<u16> {
        match codec_id {
            CodecId::PcmU8 => Ok(8),
            CodecId::PcmS16be => Ok(16),
            CodecId::PcmS16le => Ok(16), // 输出时仍按大端写
            CodecId::PcmS24le => Ok(24),
            CodecId::PcmS32le => Ok(32),
            CodecId::PcmF32le => Ok(32),
            _ => Err(TaoError::Unsupported(format!(
                "AIFF 不支持编解码器: {}",
                codec_id,
            ))),
        }
    }
}

/// 将 f64 编码为 IEEE 754 80-bit 扩展精度浮点数 (大端)
fn encode_ieee_extended(value: f64) -> [u8; 10] {
    if value == 0.0 {
        return [0u8; 10];
    }

    let sign: u8 = if value < 0.0 { 1 } else { 0 };
    let val = value.abs();

    let log2 = val.log2().floor() as i32;
    let exponent = (log2 + 16383) as u16;
    let mantissa = (val / 2.0_f64.powi(log2) * (1u64 << 63) as f64) as u64;

    let mut result = [0u8; 10];
    result[0] = (sign << 7) | ((exponent >> 8) as u8 & 0x7F);
    result[1] = (exponent & 0xFF) as u8;
    let m_bytes = mantissa.to_be_bytes();
    result[2..10].copy_from_slice(&m_bytes);
    result
}

impl Muxer for AiffMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Aiff
    }

    fn name(&self) -> &str {
        "aiff"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.len() != 1 {
            return Err(TaoError::InvalidArgument("AIFF 仅支持单个音频流".into()));
        }

        let stream = &streams[0];
        let audio = match &stream.params {
            StreamParams::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("AIFF 仅支持音频流".into()));
            }
        };

        let bits_per_sample = Self::resolve_bits_per_sample(stream.codec_id)?;
        let channels = audio.channel_layout.channels as u16;
        let sample_rate = audio.sample_rate;
        let block_align = channels * (bits_per_sample / 8);

        self.block_align = block_align;

        // FORM header
        io.write_tag(b"FORM")?;
        self.form_size_offset = 4; // 文件偏移 4
        io.write_u32_be(0)?; // 占位, trailer 回填

        io.write_tag(b"AIFF")?;

        // COMM chunk (18 bytes)
        io.write_tag(b"COMM")?;
        io.write_u32_be(18)?;
        io.write_u16_be(channels)?;
        self.num_frames_offset = io.position()?;
        io.write_u32_be(0)?; // numSampleFrames - 占位, trailer 回填
        io.write_u16_be(bits_per_sample)?;

        let sr_bytes = encode_ieee_extended(sample_rate as f64);
        io.write_all(&sr_bytes)?;

        // SSND chunk header
        io.write_tag(b"SSND")?;
        self.ssnd_size_offset = io.position()?;
        io.write_u32_be(0)?; // chunk size - 占位, trailer 回填
        io.write_u32_be(0)?; // offset
        io.write_u32_be(0)?; // blockSize

        self.data_written = 0;

        debug!(
            "AIFF 写入头部: {} Hz, {} 声道, {} 位",
            sample_rate, channels, bits_per_sample,
        );

        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        io.write_all(&packet.data)?;
        self.data_written += packet.data.len() as u64;
        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        if !io.is_seekable() {
            debug!("AIFF 输出不支持 seek, 无法回填大小字段");
            return Ok(());
        }

        let data_size = self.data_written as u32;
        let ssnd_chunk_size = 8 + data_size; // offset(4) + blockSize(4) + data
        // FORM size = 4 (AIFF) + 8+18 (COMM) + 8+ssnd_chunk_size (SSND)
        let form_size = 4 + 26 + 8 + ssnd_chunk_size;

        let num_frames = if self.block_align > 0 {
            data_size / self.block_align as u32
        } else {
            0
        };

        // 回填 FORM 大小
        io.seek(std::io::SeekFrom::Start(self.form_size_offset))?;
        io.write_u32_be(form_size)?;

        // 回填 numSampleFrames
        io.seek(std::io::SeekFrom::Start(self.num_frames_offset))?;
        io.write_u32_be(num_frames)?;

        // 回填 SSND 大小
        io.seek(std::io::SeekFrom::Start(self.ssnd_size_offset))?;
        io.write_u32_be(ssnd_chunk_size)?;

        debug!(
            "AIFF 写入尾部: form_size={}, ssnd_size={}, frames={}",
            form_size, ssnd_chunk_size, num_frames,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demuxers::aiff::AiffDemuxer;
    use crate::io::MemoryBackend;
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat};

    use crate::stream::AudioStreamParams;

    fn make_audio_stream(codec_id: CodecId, sample_rate: u32, channels: u32) -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id,
            time_base: Rational::new(1, sample_rate as i32),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::S16,
                bit_rate: 0,
                frame_size: 0,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_封装_基本写入() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_audio_stream(CodecId::PcmS16be, 44100, 1);
        let mut muxer = AiffMuxer::create().unwrap();

        muxer.write_header(&mut io, &[stream]).unwrap();

        let pcm = vec![0x00, 0x01, 0x7F, 0xFF, 0x80, 0x00, 0x00, 0x01];
        let pkt = Packet::from_data(bytes::Bytes::from(pcm));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_封装解封装_往返() {
        let backend = MemoryBackend::new();
        let mut io_w = IoContext::new(Box::new(backend));

        let stream = make_audio_stream(CodecId::PcmS16be, 44100, 1);
        let mut muxer = AiffMuxer::create().unwrap();
        muxer.write_header(&mut io_w, &[stream]).unwrap();

        // 4 采样的 S16BE 单声道 = 8 字节
        let pcm_data = vec![0x00, 0x01, 0x7F, 0xFF, 0x80, 0x00, 0x00, 0x01];
        let pkt = Packet::from_data(bytes::Bytes::from(pcm_data.clone()));
        muxer.write_packet(&mut io_w, &pkt).unwrap();
        muxer.write_trailer(&mut io_w).unwrap();

        // 解封装
        io_w.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut demuxer = AiffDemuxer::create().unwrap();
        demuxer.open(&mut io_w).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::PcmS16be);

        if let StreamParams::Audio(a) = &streams[0].params {
            assert_eq!(a.sample_rate, 44100);
            assert_eq!(a.channel_layout.channels, 1);
        } else {
            panic!("期望音频参数");
        }

        let read_pkt = demuxer.read_packet(&mut io_w).unwrap();
        assert_eq!(&read_pkt.data[..], &pcm_data[..]);
    }
}
