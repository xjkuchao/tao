//! AVI (Audio Video Interleave) 封装器.
//!
//! 将音视频数据包封装到 AVI 容器.
//!
//! 写入流程:
//! 1. `write_header()` - 写入 RIFF/AVI 头, hdrl 列表 (avih, strl/strh/strf)
//! 2. `write_packet()` - 写入数据块 (00dc/01wb 等) 并记录 idx1 条目
//! 3. `write_trailer()` - 写入 idx1 索引并回填 RIFF 大小

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// 视频流类型 FourCC
const FCC_VIDS: &[u8; 4] = b"vids";
/// 音频流类型 FourCC
const FCC_AUDS: &[u8; 4] = b"auds";

/// WAV 格式码: PCM
const WAV_FORMAT_PCM: u16 = 0x0001;
/// WAV 格式码: IEEE 浮点
const WAV_FORMAT_IEEE_FLOAT: u16 = 0x0003;

/// idx1 索引条目标志: 关键帧
const AVIIF_KEYFRAME: u32 = 0x10;

/// idx1 索引条目
struct Idx1Entry {
    chunk_id: [u8; 4],
    flags: u32,
    offset: u32,
    size: u32,
}

/// AVI 封装器
pub struct AviMuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// movi 列表数据起始位置 (用于 idx1 的 offset 计算)
    movi_data_start: u64,
    /// movi 列表大小字段的文件偏移
    movi_size_offset: u64,
    /// RIFF 大小字段的文件偏移
    riff_size_offset: u64,
    /// idx1 索引条目
    idx1_entries: Vec<Idx1Entry>,
}

impl AviMuxer {
    /// 创建 AVI 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            movi_data_start: 0,
            movi_size_offset: 0,
            riff_size_offset: 0,
            idx1_entries: Vec::new(),
        }))
    }

    /// 视频 CodecId -> FourCC handler
    fn video_codec_to_fourcc(codec_id: CodecId) -> TaoResult<[u8; 4]> {
        match codec_id {
            CodecId::H264 => Ok(*b"H264"),
            CodecId::H265 => Ok(*b"H265"),
            CodecId::Mpeg4 => Ok(*b"MP4V"),
            CodecId::Mjpeg => Ok(*b"MJPG"),
            CodecId::RawVideo => Ok(*b"    "),
            _ => Err(TaoError::Unsupported(format!(
                "AVI 不支持视频编解码器: {}",
                codec_id
            ))),
        }
    }

    /// 音频 CodecId -> WAV 格式码和位深
    fn audio_codec_to_wav_format(codec_id: CodecId) -> TaoResult<(u16, u16)> {
        match codec_id {
            CodecId::PcmU8 => Ok((WAV_FORMAT_PCM, 8)),
            CodecId::PcmS16le => Ok((WAV_FORMAT_PCM, 16)),
            CodecId::PcmS24le => Ok((WAV_FORMAT_PCM, 24)),
            CodecId::PcmS32le => Ok((WAV_FORMAT_PCM, 32)),
            CodecId::PcmF32le => Ok((WAV_FORMAT_IEEE_FLOAT, 32)),
            CodecId::Mp3 => Err(TaoError::Unsupported(
                "AVI 暂不支持 MP3 封装, 请使用 PCM".into(),
            )),
            _ => Err(TaoError::Unsupported(format!(
                "AVI 不支持音频编解码器: {}",
                codec_id
            ))),
        }
    }

    /// 写入 strh 块
    fn write_strh(
        io: &mut IoContext,
        _stream_index: usize,
        stream: &Stream,
        scale: u32,
        rate: u32,
    ) -> TaoResult<()> {
        io.write_tag(b"strh")?;
        io.write_u32_le(48)?;

        match stream.media_type {
            MediaType::Video => {
                io.write_tag(FCC_VIDS)?;
                let handler = Self::video_codec_to_fourcc(stream.codec_id)?;
                io.write_tag(&handler)?;
            }
            MediaType::Audio => {
                io.write_tag(FCC_AUDS)?;
                io.write_tag(b"    ")?;
            }
            _ => {
                return Err(TaoError::Unsupported(format!(
                    "AVI 不支持媒体类型: {:?}",
                    stream.media_type
                )));
            }
        }

        io.write_u32_le(0)?;
        io.write_u16_le(0)?;
        io.write_u16_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(scale)?;
        io.write_u32_le(rate)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;

        Ok(())
    }

    /// 写入 strf 块 (视频: BITMAPINFOHEADER)
    fn write_strf_video(io: &mut IoContext, stream: &Stream) -> TaoResult<()> {
        let params = match &stream.params {
            StreamParams::Video(v) => v,
            _ => {
                return Err(TaoError::InvalidArgument("期望视频流参数".into()));
            }
        };

        io.write_tag(b"strf")?;
        io.write_u32_le(40)?;

        io.write_u32_le(40)?;
        io.write_u32_le(params.width)?;
        io.write_u32_le(params.height)?;
        io.write_u16_le(1)?;
        io.write_u16_le(24)?;

        let bi_compression = match stream.codec_id {
            CodecId::H264 => 0x34363248u32,
            CodecId::H265 => 0x43564548u32,
            CodecId::Mpeg4 => 0x34564D50u32,
            CodecId::Mjpeg => 0x47504A4Du32,
            _ => 0,
        };
        io.write_u32_le(bi_compression)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;

        Ok(())
    }

    /// 写入 strf 块 (音频: WAVEFORMATEX)
    fn write_strf_audio(io: &mut IoContext, stream: &Stream) -> TaoResult<()> {
        let params = match &stream.params {
            StreamParams::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("期望音频流参数".into()));
            }
        };

        let (format_tag, bits_per_sample) = Self::audio_codec_to_wav_format(stream.codec_id)?;
        let channels = params.channel_layout.channels as u16;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = params.sample_rate * u32::from(block_align);

        io.write_tag(b"strf")?;
        io.write_u32_le(18)?;

        io.write_u16_le(format_tag)?;
        io.write_u16_le(channels)?;
        io.write_u32_le(params.sample_rate)?;
        io.write_u32_le(byte_rate)?;
        io.write_u16_le(block_align)?;
        io.write_u16_le(bits_per_sample)?;
        io.write_u16_le(0)?;

        Ok(())
    }
}

impl Muxer for AviMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Avi
    }

    fn name(&self) -> &str {
        "avi"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("AVI: 没有输入流".into()));
        }

        self.streams = streams.to_vec();

        io.write_tag(b"RIFF")?;
        self.riff_size_offset = io.position()?;
        io.write_u32_le(0)?;
        io.write_tag(b"AVI ")?;

        let hdrl_start = io.position()?;
        io.write_tag(b"LIST")?;
        let hdrl_size_offset = io.position()?;
        io.write_u32_le(0)?;
        io.write_tag(b"hdrl")?;

        io.write_tag(b"avih")?;
        io.write_u32_le(56)?;
        io.write_u32_le(33367)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0x10)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(streams.len() as u32)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;
        io.write_u32_le(0)?;

        for (idx, stream) in streams.iter().enumerate() {
            let strl_start = io.position()?;
            io.write_tag(b"LIST")?;
            let strl_size_offset = io.position()?;
            io.write_u32_le(0)?;
            io.write_tag(b"strl")?;

            let (scale, rate) = match stream.media_type {
                MediaType::Video => {
                    if let StreamParams::Video(v) = &stream.params {
                        (1u32, v.frame_rate.num as u32)
                    } else {
                        (1, 25)
                    }
                }
                MediaType::Audio => {
                    if let StreamParams::Audio(a) = &stream.params {
                        (1, a.sample_rate)
                    } else {
                        (1, 44100)
                    }
                }
                _ => (1, 25),
            };

            Self::write_strh(io, idx, stream, scale, rate)?;

            match stream.media_type {
                MediaType::Video => Self::write_strf_video(io, stream)?,
                MediaType::Audio => Self::write_strf_audio(io, stream)?,
                _ => {}
            }

            let strl_end = io.position()?;
            let strl_size = (strl_end - strl_start - 8) as u32;
            io.seek(std::io::SeekFrom::Start(strl_size_offset))?;
            io.write_u32_le(strl_size)?;
            io.seek(std::io::SeekFrom::Start(strl_end))?;
        }

        let hdrl_end = io.position()?;
        let hdrl_size = (hdrl_end - hdrl_start - 8) as u32;
        io.seek(std::io::SeekFrom::Start(hdrl_size_offset))?;
        io.write_u32_le(hdrl_size)?;
        io.seek(std::io::SeekFrom::Start(hdrl_end))?;

        io.write_tag(b"LIST")?;
        self.movi_size_offset = io.position()?;
        io.write_u32_le(0)?;
        io.write_tag(b"movi")?;
        self.movi_data_start = io.position()?;

        debug!(
            "AVI 写入头部: {} 个流, movi 起始={}",
            streams.len(),
            self.movi_data_start
        );

        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        if packet.data.is_empty() {
            return Ok(());
        }

        let stream_index = packet
            .stream_index
            .min(self.streams.len().saturating_sub(1));
        let stream = &self.streams[stream_index];

        let (chunk_id, is_keyframe) = match stream.media_type {
            MediaType::Video => {
                let id = format!("{:02}dc", stream_index);
                let mut cid = [0u8; 4];
                cid.copy_from_slice(id.as_bytes());
                (cid, packet.is_keyframe)
            }
            MediaType::Audio => {
                let id = format!("{:02}wb", stream_index);
                let mut cid = [0u8; 4];
                cid.copy_from_slice(id.as_bytes());
                (cid, true)
            }
            _ => return Ok(()),
        };

        let offset = (io.position()? - self.movi_data_start) as u32;
        let size = packet.data.len() as u32;

        io.write_tag(&chunk_id)?;
        io.write_u32_le(size)?;
        io.write_all(&packet.data)?;
        if size % 2 != 0 {
            io.write_u8(0)?;
        }

        self.idx1_entries.push(Idx1Entry {
            chunk_id,
            flags: if is_keyframe { AVIIF_KEYFRAME } else { 0 },
            offset,
            size,
        });

        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let pos_before_idx1 = io.position()?;
        let movi_size = (pos_before_idx1 - self.movi_data_start + 4) as u32;

        io.write_tag(b"idx1")?;
        let idx1_size = (self.idx1_entries.len() * 16) as u32;
        io.write_u32_le(idx1_size)?;
        for entry in &self.idx1_entries {
            io.write_tag(&entry.chunk_id)?;
            io.write_u32_le(entry.flags)?;
            io.write_u32_le(entry.offset)?;
            io.write_u32_le(entry.size)?;
        }

        if io.is_seekable() {
            let file_size = io.position()?;
            let riff_size = (file_size - 8) as u32;
            io.seek(std::io::SeekFrom::Start(self.riff_size_offset))?;
            io.write_u32_le(riff_size)?;
            io.seek(std::io::SeekFrom::Start(self.movi_size_offset))?;
            io.write_u32_le(movi_size)?;
        }

        debug!("AVI 写入尾部: idx1 条目数={}", self.idx1_entries.len());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;
    use tao_core::{ChannelLayout, Rational, SampleFormat};

    use crate::stream::{AudioStreamParams, VideoStreamParams};

    fn make_video_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 25),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Video(VideoStreamParams {
                width: 320,
                height: 240,
                pixel_format: tao_core::PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        }
    }

    fn make_audio_stream() -> Stream {
        Stream {
            index: 1,
            media_type: MediaType::Audio,
            codec_id: CodecId::PcmS16le,
            time_base: Rational::new(1, 44100),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::STEREO,
                sample_format: SampleFormat::S16,
                bit_rate: 0,
                frame_size: 4,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_avi_write_header() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_video_stream();
        let mut muxer = AviMuxer::create().unwrap();
        muxer.write_header(&mut io, &[stream]).unwrap();

        let data = io.position().unwrap();
        assert!(data > 100);
    }

    #[test]
    fn test_avi_write_packets() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_video_stream();
        let mut muxer = AviMuxer::create().unwrap();
        muxer.write_header(&mut io, &[stream]).unwrap();

        let pkt = Packet::from_data(bytes::Bytes::from(vec![0u8; 100]));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_avi_av() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let video = make_video_stream();
        let mut audio = make_audio_stream();
        audio.index = 1;
        let mut muxer = AviMuxer::create().unwrap();
        muxer.write_header(&mut io, &[video, audio]).unwrap();

        let pkt = Packet::from_data(bytes::Bytes::from(vec![0u8; 50]));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_empty_stream_error() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let mut muxer = AviMuxer::create().unwrap();
        let err = muxer.write_header(&mut io, &[]).unwrap_err();
        assert!(matches!(err, TaoError::InvalidArgument(_)));
    }
}
