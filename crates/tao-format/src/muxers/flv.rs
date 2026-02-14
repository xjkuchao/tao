//! FLV (Flash Video) 封装器.
//!
//! 对标 FFmpeg 的 FLV 封装器, 将音视频数据包封装到 FLV 容器.
//!
//! FLV 文件结构:
//! - FLV Header (9 bytes)
//! - PreviousTagSize0 (4 bytes = 0)
//! - [FLV Tag + PreviousTagSize] ...

use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// FLV Tag 类型
const TAG_AUDIO: u8 = 8;
const TAG_VIDEO: u8 = 9;

/// FLV 封装器
pub struct FlvMuxer {
    /// 是否有音频流
    has_audio: bool,
    /// 是否有视频流
    has_video: bool,
    /// 音频流索引
    audio_stream_idx: Option<usize>,
    /// 视频流索引
    video_stream_idx: Option<usize>,
    /// 是否已写入序列头
    audio_seq_written: bool,
    video_seq_written: bool,
    /// 流信息缓存
    streams: Vec<Stream>,
}

impl FlvMuxer {
    /// 创建 FLV 封装器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            has_audio: false,
            has_video: false,
            audio_stream_idx: None,
            video_stream_idx: None,
            audio_seq_written: false,
            video_seq_written: false,
            streams: Vec::new(),
        }))
    }

    /// 编解码器 -> FLV SoundFormat
    fn codec_to_sound_format(codec_id: CodecId) -> TaoResult<u8> {
        match codec_id {
            CodecId::Aac => Ok(10),
            CodecId::Mp3 => Ok(2),
            CodecId::PcmS16le | CodecId::PcmS16be => Ok(3), // Linear PCM
            _ => Err(TaoError::Unsupported(format!(
                "FLV 不支持音频编解码器: {}",
                codec_id
            ))),
        }
    }

    /// 编解码器 -> FLV CodecID (video)
    fn codec_to_video_id(codec_id: CodecId) -> TaoResult<u8> {
        match codec_id {
            CodecId::H264 => Ok(7),
            CodecId::H265 => Ok(12),
            _ => Err(TaoError::Unsupported(format!(
                "FLV 不支持视频编解码器: {}",
                codec_id
            ))),
        }
    }

    /// 写入 FLV Tag
    fn write_tag(io: &mut IoContext, tag_type: u8, timestamp: u32, data: &[u8]) -> TaoResult<()> {
        let data_size = data.len() as u32;

        // Tag header (11 bytes)
        io.write_u8(tag_type)?;
        // DataSize (3 bytes, BE)
        io.write_u8((data_size >> 16) as u8)?;
        io.write_u8((data_size >> 8) as u8)?;
        io.write_u8(data_size as u8)?;
        // Timestamp (3 bytes, BE) + TimestampExtended (1 byte)
        io.write_u8((timestamp >> 16) as u8)?;
        io.write_u8((timestamp >> 8) as u8)?;
        io.write_u8(timestamp as u8)?;
        io.write_u8((timestamp >> 24) as u8)?;
        // StreamID (3 bytes, always 0)
        io.write_u8(0)?;
        io.write_u8(0)?;
        io.write_u8(0)?;
        // Tag data
        io.write_all(data)?;
        // PreviousTagSize
        let tag_size = 11 + data_size;
        io.write_u32_be(tag_size)?;

        Ok(())
    }
}

impl Muxer for FlvMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Flv
    }

    fn name(&self) -> &str {
        "flv"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("FLV: 没有输入流".into()));
        }

        self.streams = streams.to_vec();

        for stream in streams {
            match stream.media_type {
                MediaType::Audio => {
                    self.has_audio = true;
                    self.audio_stream_idx = Some(stream.index);
                }
                MediaType::Video => {
                    self.has_video = true;
                    self.video_stream_idx = Some(stream.index);
                }
                _ => {}
            }
        }

        // FLV Header
        io.write_all(b"FLV")?; // Signature
        io.write_u8(1)?; // Version
        let mut flags: u8 = 0;
        if self.has_audio {
            flags |= 0x04;
        }
        if self.has_video {
            flags |= 0x01;
        }
        io.write_u8(flags)?; // TypeFlags
        io.write_u32_be(9)?; // DataOffset (header size = 9)
        io.write_u32_be(0)?; // PreviousTagSize0

        // 写入序列头 (extradata)
        for stream in streams {
            if stream.media_type == MediaType::Video && !stream.extra_data.is_empty() {
                let video_codec_id = Self::codec_to_video_id(stream.codec_id)?;
                let mut tag_data = Vec::new();
                // FrameType=1(keyframe) | CodecID
                tag_data.push((1 << 4) | video_codec_id);
                tag_data.push(0); // AVCPacketType = 0 (Sequence Header)
                tag_data.extend_from_slice(&[0, 0, 0]); // CompositionTimeOffset
                tag_data.extend_from_slice(&stream.extra_data);
                Self::write_tag(io, TAG_VIDEO, 0, &tag_data)?;
                self.video_seq_written = true;
            }
            if stream.media_type == MediaType::Audio && !stream.extra_data.is_empty() {
                let sound_format = Self::codec_to_sound_format(stream.codec_id)?;
                let mut tag_data = Vec::new();
                // SoundFormat(4) | SoundRate(2)=3(44kHz) | SoundSize(1)=1(16bit) | SoundType(1)=1(stereo)
                tag_data.push((sound_format << 4) | 0x0F);
                if sound_format == 10 {
                    // AAC
                    tag_data.push(0); // AACPacketType = 0 (Sequence Header)
                }
                tag_data.extend_from_slice(&stream.extra_data);
                Self::write_tag(io, TAG_AUDIO, 0, &tag_data)?;
                self.audio_seq_written = true;
            }
        }

        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        let idx = packet.stream_index;
        if idx >= self.streams.len() {
            return Err(TaoError::StreamNotFound(idx));
        }

        let stream = &self.streams[idx];
        let timestamp_ms = if stream.time_base.den > 0 {
            (packet.pts as f64 * stream.time_base.num as f64 / stream.time_base.den as f64 * 1000.0)
                as u32
        } else {
            0
        };

        match stream.media_type {
            MediaType::Video => {
                let video_codec_id = Self::codec_to_video_id(stream.codec_id)?;
                let mut tag_data = Vec::new();
                let frame_type = if packet.is_keyframe { 1u8 } else { 2u8 };
                tag_data.push((frame_type << 4) | video_codec_id);
                tag_data.push(1); // AVCPacketType = 1 (NALU)
                // CompositionTimeOffset (CTS)
                let cts = if packet.pts >= packet.dts {
                    (packet.pts - packet.dts) as i32
                } else {
                    0
                };
                let cts_bytes = cts.to_be_bytes();
                tag_data.extend_from_slice(&cts_bytes[1..4]); // 3 bytes
                tag_data.extend_from_slice(&packet.data);
                Self::write_tag(io, TAG_VIDEO, timestamp_ms, &tag_data)?;
            }
            MediaType::Audio => {
                let sound_format = Self::codec_to_sound_format(stream.codec_id)?;
                let mut tag_data = Vec::new();
                // 音频头字节
                let sr_code = match stream.params {
                    StreamParams::Audio(ref a) => match a.sample_rate {
                        44100 => 3,
                        22050 => 2,
                        11025 => 1,
                        _ => 3,
                    },
                    _ => 3,
                };
                let channels_bit = match &stream.params {
                    StreamParams::Audio(a) => {
                        if a.channel_layout.channels >= 2 {
                            1u8
                        } else {
                            0u8
                        }
                    }
                    _ => 1,
                };
                tag_data.push((sound_format << 4) | (sr_code << 2) | (1 << 1) | channels_bit);
                if sound_format == 10 {
                    // AAC raw data
                    tag_data.push(1); // AACPacketType = 1 (Raw)
                }
                tag_data.extend_from_slice(&packet.data);
                Self::write_tag(io, TAG_AUDIO, timestamp_ms, &tag_data)?;
            }
            _ => {
                return Err(TaoError::Unsupported("FLV: 不支持的流类型".into()));
            }
        }

        Ok(())
    }

    fn write_trailer(&mut self, _io: &mut IoContext) -> TaoResult<()> {
        // FLV 没有需要回填的尾部结构
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{IoContext, MemoryBackend};
    use crate::stream::{AudioStreamParams, VideoStreamParams};
    use tao_core::PixelFormat;
    use tao_core::{ChannelLayout, Rational, SampleFormat};

    fn make_audio_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Aac,
            time_base: Rational::new(1, 44100),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: vec![0x12, 0x10],
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::S16,
                bit_rate: 128000,
                frame_size: 1024,
            }),
            metadata: Vec::new(),
        }
    }

    fn make_video_stream() -> Stream {
        Stream {
            index: 1,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 90000),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: vec![0x01, 0x42, 0x00, 0x1E, 0xFF],
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(30, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_flv_写入头部() {
        let mut muxer = FlvMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();
        let pos = io.position().unwrap();
        // FLV header (9) + PrevTagSize0 (4) + 序列头 Tag
        assert!(pos > 13, "应写入 FLV 头部和序列头");
    }

    #[test]
    fn test_flv_音视频() {
        let mut muxer = FlvMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream(), make_video_stream()];
        muxer.write_header(&mut io, &streams).unwrap();
        let pos = io.position().unwrap();
        // 应包含头部 + 两个序列头 tag
        assert!(pos > 30, "应写入头部和两个序列头");
    }

    #[test]
    fn test_flv_写入数据包() {
        let mut muxer = FlvMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();

        let mut packet = Packet::from_data(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        packet.pts = 0;
        packet.dts = 0;
        packet.duration = 1024;
        packet.stream_index = 0;
        packet.is_keyframe = true;
        muxer.write_packet(&mut io, &packet).unwrap();

        let pos = io.position().unwrap();
        assert!(pos > 40, "应写入头部 + 序列头 + 数据包");
    }

    #[test]
    fn test_空流报错() {
        let mut muxer = FlvMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        assert!(muxer.write_header(&mut io, &[]).is_err());
    }
}
