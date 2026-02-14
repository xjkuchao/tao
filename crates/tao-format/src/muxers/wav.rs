//! WAV (RIFF WAVE) 封装器.
//!
//! 将 PCM 音频数据写入标准 WAV 文件.
//!
//! 写入流程:
//! 1. `write_header()` - 写入 RIFF 和 fmt 块, 预留 data 块大小
//! 2. `write_packet()` - 追加 PCM 数据
//! 3. `write_trailer()` - 回填 RIFF 大小和 data 块大小

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// WAV 音频格式码: PCM 整数
const WAV_FORMAT_PCM: u16 = 0x0001;
/// WAV 音频格式码: IEEE 浮点
const WAV_FORMAT_IEEE_FLOAT: u16 = 0x0003;

/// WAV 封装器
pub struct WavMuxer {
    /// RIFF 大小字段的文件偏移 (需要回填)
    riff_size_offset: u64,
    /// data 块大小字段的文件偏移 (需要回填)
    data_size_offset: u64,
    /// 已写入的数据字节数
    data_written: u64,
}

impl WavMuxer {
    /// 创建 WAV 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            riff_size_offset: 0,
            data_size_offset: 0,
            data_written: 0,
        }))
    }

    /// 根据 CodecId 确定 WAV 格式码和位深
    fn resolve_wav_format(codec_id: CodecId) -> TaoResult<(u16, u16)> {
        match codec_id {
            CodecId::PcmU8 => Ok((WAV_FORMAT_PCM, 8)),
            CodecId::PcmS16le => Ok((WAV_FORMAT_PCM, 16)),
            CodecId::PcmS24le => Ok((WAV_FORMAT_PCM, 24)),
            CodecId::PcmS32le => Ok((WAV_FORMAT_PCM, 32)),
            CodecId::PcmF32le => Ok((WAV_FORMAT_IEEE_FLOAT, 32)),
            _ => Err(TaoError::Unsupported(format!(
                "WAV 不支持编解码器: {}",
                codec_id
            ))),
        }
    }
}

impl Muxer for WavMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Wav
    }

    fn name(&self) -> &str {
        "wav"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        // WAV 只支持单个音频流
        if streams.len() != 1 {
            return Err(TaoError::InvalidArgument("WAV 仅支持单个音频流".into()));
        }

        let stream = &streams[0];
        let audio = match &stream.params {
            StreamParams::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("WAV 仅支持音频流".into()));
            }
        };

        let (audio_format, bits_per_sample) = Self::resolve_wav_format(stream.codec_id)?;
        let channels = audio.channel_layout.channels as u16;
        let sample_rate = audio.sample_rate;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * u32::from(block_align);

        // RIFF header
        io.write_tag(b"RIFF")?;
        self.riff_size_offset = io.position()? + 4; // 写完后才知道实际偏移
        // 先记录当前位置
        self.riff_size_offset = 4; // "RIFF" 后面就是 size 字段
        io.write_u32_le(0)?; // 占位, trailer 中回填
        io.write_tag(b"WAVE")?;

        // fmt chunk
        io.write_tag(b"fmt ")?;
        io.write_u32_le(16)?; // 标准 PCM fmt 块大小
        io.write_u16_le(audio_format)?;
        io.write_u16_le(channels)?;
        io.write_u32_le(sample_rate)?;
        io.write_u32_le(byte_rate)?;
        io.write_u16_le(block_align)?;
        io.write_u16_le(bits_per_sample)?;

        // data chunk header
        io.write_tag(b"data")?;
        self.data_size_offset = 40; // 固定偏移: 12 (RIFF) + 24 (fmt) + 4 (data tag) = 40
        io.write_u32_le(0)?; // 占位, trailer 中回填

        self.data_written = 0;

        debug!(
            "WAV 写入头部: {} Hz, {} 声道, {} 位",
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
            debug!("WAV 输出不支持 seek, 无法回填大小字段");
            return Ok(());
        }

        let data_size = self.data_written as u32;
        let riff_size = 36 + data_size; // 整个文件大小 - 8

        // 回填 RIFF 大小
        io.seek(std::io::SeekFrom::Start(self.riff_size_offset))?;
        io.write_u32_le(riff_size)?;

        // 回填 data 块大小
        io.seek(std::io::SeekFrom::Start(self.data_size_offset))?;
        io.write_u32_le(data_size)?;

        debug!(
            "WAV 写入尾部: riff_size={}, data_size={}",
            riff_size, data_size,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demuxers::wav::WavDemuxer;
    use crate::io::MemoryBackend;
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat};

    use crate::stream::{AudioStreamParams, VideoStreamParams};

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

        let stream = make_audio_stream(CodecId::PcmS16le, 44100, 1);
        let mut muxer = WavMuxer::create().unwrap();

        muxer.write_header(&mut io, &[stream]).unwrap();

        // 写入 4 采样 = 8 字节
        let pcm = vec![0x00, 0x01, 0xFF, 0x7F, 0x00, 0x80, 0x01, 0x00];
        let pkt = Packet::from_data(bytes::Bytes::from(pcm));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_封装解封装_往返() {
        // 写入 WAV
        let backend = MemoryBackend::new();
        let mut io_w = IoContext::new(Box::new(backend));

        let stream = make_audio_stream(CodecId::PcmS16le, 44100, 2);
        let mut muxer = WavMuxer::create().unwrap();
        muxer.write_header(&mut io_w, &[stream]).unwrap();

        // 2 声道, 3 采样 = 12 字节
        let pcm_data = vec![
            0x01, 0x00, 0x02, 0x00, // 采样 0: L, R
            0x03, 0x00, 0x04, 0x00, // 采样 1: L, R
            0x05, 0x00, 0x06, 0x00, // 采样 2: L, R
        ];
        let pkt = Packet::from_data(bytes::Bytes::from(pcm_data.clone()));
        muxer.write_packet(&mut io_w, &pkt).unwrap();
        muxer.write_trailer(&mut io_w).unwrap();

        // 获取写入的数据
        // 我们需要从 IoContext 中提取 MemoryBackend 的数据
        // 但 IoContext 不暴露 inner, 所以换一种方式: seek 到开头然后用读取
        // 更好的方式: 直接重新创建 IoContext
        io_w.seek(std::io::SeekFrom::Start(0)).unwrap();
        let pos = io_w.position().unwrap();
        assert_eq!(pos, 0);

        // 解封装
        let mut demuxer = WavDemuxer::create().unwrap();
        demuxer.open(&mut io_w).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::PcmS16le);

        if let StreamParams::Audio(a) = &streams[0].params {
            assert_eq!(a.sample_rate, 44100);
            assert_eq!(a.channel_layout.channels, 2);
        } else {
            panic!("期望音频参数");
        }

        let read_pkt = demuxer.read_packet(&mut io_w).unwrap();
        assert_eq!(&read_pkt.data[..], &pcm_data[..]);
    }

    #[test]
    fn test_不支持的编解码器() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        // 尝试用 H264 编解码器创建 WAV
        let stream = Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 25),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: tao_core::PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        };

        let mut muxer = WavMuxer::create().unwrap();
        let err = muxer.write_header(&mut io, &[stream]).unwrap_err();
        assert!(matches!(err, TaoError::InvalidArgument(_)));
    }
}
