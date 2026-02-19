//! MP3 裸流封装器.
//!
//! 输出原始 MP3 帧, 无容器开销 (passthrough 模式).
//! 适用于 .mp3 文件的写入.

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// MP3 裸流封装器
pub struct Mp3Muxer {
    /// 是否已写入头部
    header_written: bool,
}

impl Mp3Muxer {
    /// 创建 MP3 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            header_written: false,
        }))
    }
}

impl Muxer for Mp3Muxer {
    fn format_id(&self) -> FormatId {
        FormatId::Mp3Container
    }

    fn name(&self) -> &str {
        "mp3"
    }

    fn write_header(&mut self, _io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("MP3 需要至少一个流".into()));
        }

        if streams.len() != 1 {
            return Err(TaoError::InvalidArgument("MP3 裸流仅支持单个音频流".into()));
        }

        let stream = &streams[0];
        if stream.codec_id != CodecId::Mp3 {
            return Err(TaoError::InvalidArgument(format!(
                "MP3 需要 MP3 编解码器, 当前: {}",
                stream.codec_id
            )));
        }

        match &stream.params {
            StreamParams::Audio(a) => {
                debug!(
                    "MP3 写入头部: {} Hz, {} 声道 (裸流模式, 无容器头部)",
                    a.sample_rate, a.channel_layout.channels,
                );
            }
            _ => {
                return Err(TaoError::InvalidArgument("MP3 仅支持音频流".into()));
            }
        }

        self.header_written = true;
        // 裸流模式: 不写入任何头部, 直接透传 MP3 帧
        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        if packet.data.is_empty() {
            return Ok(());
        }
        io.write_all(&packet.data)?;
        Ok(())
    }

    fn write_trailer(&mut self, _io: &mut IoContext) -> TaoResult<()> {
        // 裸流格式无需尾部
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat};

    use crate::stream::{AudioStreamParams, StreamParams};

    fn make_mp3_stream(sample_rate: u32, channels: u32) -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Mp3,
            time_base: Rational::new(1, 44100),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::S16,
                bit_rate: 128000,
                frame_size: 1152,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_mp3_write_header() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_mp3_stream(44100, 2);
        let mut muxer = Mp3Muxer::create().unwrap();

        muxer.write_header(&mut io, &[stream]).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_mp3_write_packets() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_mp3_stream(44100, 1);
        let mut muxer = Mp3Muxer::create().unwrap();
        muxer.write_header(&mut io, &[stream]).unwrap();

        // 模拟 MP3 帧数据 (任意字节)
        let data = vec![0xFF, 0xFB, 0x90, 0x00, 0x01, 0x02, 0x03];
        let pkt = Packet::from_data(bytes::Bytes::from(data.clone()));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();

        // 验证数据透传
        io.seek(std::io::SeekFrom::Start(0)).unwrap();
        let size = io.size().unwrap() as usize;
        let output = io.read_bytes(size).unwrap();
        assert_eq!(output, data, "MP3 裸流应透传原始帧数据");
    }

    #[test]
    fn test_mp3_empty_stream_error() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let mut muxer = Mp3Muxer::create().unwrap();
        let err = muxer.write_header(&mut io, &[]).unwrap_err();
        assert!(matches!(err, TaoError::InvalidArgument(_)));
    }
}
