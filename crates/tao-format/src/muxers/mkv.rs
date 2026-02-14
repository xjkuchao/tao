//! Matroska/WebM 封装器.
//!
//! 将音视频数据写入 Matroska (.mkv/.mka) 或 WebM (.webm) 容器.
//!
//! # 输出结构
//! ```text
//! EBML Header (DocType: "matroska" 或 "webm")
//! Segment (unknown size)
//! ├── Info (TimecodeScale, Duration 回填)
//! ├── Tracks (每个流一个 TrackEntry)
//! └── Cluster... (SimpleBlock 数据)
//! ```
//!
//! 写入流程:
//! 1. `write_header()` - 写入 EBML Header + Segment + Info + Tracks
//! 2. `write_packet()` - 写入 SimpleBlock (新 Cluster 按需创建)
//! 3. `write_trailer()` - 回填 Duration (如果可 seek)

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

// ============================================================
// EBML 元素 ID 常量
// ============================================================

const EBML_HEADER: u32 = 0x1A45_DFA3;
const EBML_VERSION: u32 = 0x4286;
const EBML_READ_VERSION: u32 = 0x42F7;
const EBML_MAX_ID_LENGTH: u32 = 0x42F2;
const EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
const EBML_DOC_TYPE: u32 = 0x4282;
const EBML_DOC_TYPE_VERSION: u32 = 0x4287;
const EBML_DOC_TYPE_READ_VERSION: u32 = 0x4285;

const SEGMENT: u32 = 0x1853_8067;
const SEGMENT_INFO: u32 = 0x1549_A966;
const INFO_TIMESCALE: u32 = 0x002A_D7B1;
const INFO_DURATION: u32 = 0x4489;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const TRACK_CODEC_ID: u32 = 0x86;
const TRACK_CODEC_PRIVATE: u32 = 0x63A2;
const TRACK_DEFAULT_DURATION: u32 = 0x0023_E383;
const VIDEO_SETTINGS: u32 = 0xE0;
const VIDEO_PIXEL_WIDTH: u32 = 0xB0;
const VIDEO_PIXEL_HEIGHT: u32 = 0xBA;
const AUDIO_SETTINGS: u32 = 0xE1;
const AUDIO_SAMPLING_FREQ: u32 = 0xB5;
const AUDIO_CHANNELS: u32 = 0x9F;
const AUDIO_BIT_DEPTH: u32 = 0x6264;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;

/// 默认 TimecodeScale: 1ms (1_000_000 纳秒)
const DEFAULT_TIMESCALE_NS: u64 = 1_000_000;

/// 每个 Cluster 的最大时长 (毫秒)
const MAX_CLUSTER_DURATION_MS: i64 = 5000;

/// Matroska/WebM 封装器
pub struct MkvMuxer {
    /// 是否输出 WebM (否则为 Matroska)
    webm: bool,
    /// 轨道信息
    tracks: Vec<MkvTrack>,
    /// 当前 Cluster 的时间戳 (毫秒)
    cluster_timestamp: i64,
    /// 是否已开始一个 Cluster
    cluster_open: bool,
    /// Cluster 开始位置 (用于大小计算)
    _cluster_start_offset: u64,
    /// Cluster 数据缓冲
    cluster_buf: Vec<u8>,
    /// Info 中 Duration 字段的偏移 (用于 trailer 回填)
    duration_offset: u64,
    /// 最大时间戳 (毫秒, 用于计算 Duration)
    max_timestamp_ms: i64,
}

struct MkvTrack {
    stream_index: usize,
    track_number: u8,
    timescale_num: i64, // stream time_base numerator
    timescale_den: i64, // stream time_base denominator
}

impl MkvMuxer {
    /// 创建 Matroska 封装器
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            webm: false,
            tracks: Vec::new(),
            cluster_timestamp: -1,
            cluster_open: false,
            _cluster_start_offset: 0,
            cluster_buf: Vec::new(),
            duration_offset: 0,
            max_timestamp_ms: 0,
        }))
    }

    /// 创建 WebM 封装器
    pub fn create_webm() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            webm: true,
            tracks: Vec::new(),
            cluster_timestamp: -1,
            cluster_open: false,
            _cluster_start_offset: 0,
            cluster_buf: Vec::new(),
            duration_offset: 0,
            max_timestamp_ms: 0,
        }))
    }

    /// 将 packet 时间戳转换为毫秒
    fn pts_to_ms(&self, packet: &Packet) -> i64 {
        let track = self
            .tracks
            .iter()
            .find(|t| t.stream_index == packet.stream_index);
        if let Some(t) = track {
            if t.timescale_den > 0 {
                packet.pts * 1000 * t.timescale_num / t.timescale_den
            } else {
                packet.pts
            }
        } else {
            packet.pts
        }
    }

    /// 刷新当前 Cluster 到输出
    fn flush_cluster(&mut self, io: &mut IoContext) -> TaoResult<()> {
        if !self.cluster_open || self.cluster_buf.is_empty() {
            return Ok(());
        }

        write_element_id(io, CLUSTER)?;
        write_element_size(io, self.cluster_buf.len() as u64)?;
        io.write_all(&self.cluster_buf)?;

        self.cluster_buf.clear();
        self.cluster_open = false;
        Ok(())
    }

    /// 开始新 Cluster
    fn start_cluster(&mut self, timestamp_ms: i64) {
        self.cluster_buf.clear();

        // ClusterTimestamp
        let mut ts_buf = Vec::new();
        write_element_id_buf(&mut ts_buf, CLUSTER_TIMESTAMP);
        write_uint_element_buf(&mut ts_buf, timestamp_ms as u64);

        self.cluster_buf.extend_from_slice(&ts_buf);
        self.cluster_timestamp = timestamp_ms;
        self.cluster_open = true;
    }
}

impl Muxer for MkvMuxer {
    fn format_id(&self) -> FormatId {
        if self.webm {
            FormatId::Webm
        } else {
            FormatId::Matroska
        }
    }

    fn name(&self) -> &str {
        if self.webm { "webm" } else { "matroska" }
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("MKV: 至少需要一个流".into()));
        }

        // 初始化轨道
        for (i, stream) in streams.iter().enumerate() {
            self.tracks.push(MkvTrack {
                stream_index: stream.index,
                track_number: i as u8 + 1,
                timescale_num: stream.time_base.num as i64,
                timescale_den: stream.time_base.den as i64,
            });
        }

        let doc_type = if self.webm { "webm" } else { "matroska" };

        // EBML Header
        let mut ebml_content = Vec::new();
        write_uint_full_element(&mut ebml_content, EBML_VERSION, 1);
        write_uint_full_element(&mut ebml_content, EBML_READ_VERSION, 1);
        write_uint_full_element(&mut ebml_content, EBML_MAX_ID_LENGTH, 4);
        write_uint_full_element(&mut ebml_content, EBML_MAX_SIZE_LENGTH, 8);
        write_string_element_buf(&mut ebml_content, EBML_DOC_TYPE, doc_type);
        write_uint_full_element(&mut ebml_content, EBML_DOC_TYPE_VERSION, 4);
        write_uint_full_element(&mut ebml_content, EBML_DOC_TYPE_READ_VERSION, 2);

        write_element_id(io, EBML_HEADER)?;
        write_element_size(io, ebml_content.len() as u64)?;
        io.write_all(&ebml_content)?;

        // Segment (unknown size)
        write_element_id(io, SEGMENT)?;
        // 写 8 字节的 "未知大小" VINT
        io.write_all(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])?;

        // Info
        let mut info_content = Vec::new();
        write_uint_full_element(&mut info_content, INFO_TIMESCALE, DEFAULT_TIMESCALE_NS);
        // Duration (float64, 先写 0.0, trailer 回填)
        let _duration_content_start = info_content.len();
        write_element_id_buf(&mut info_content, INFO_DURATION);
        write_element_size_buf(&mut info_content, 8);
        let duration_data_pos = info_content.len();
        info_content.extend_from_slice(&0.0f64.to_be_bytes());

        let _info_start = io.position()?;
        write_element_id(io, SEGMENT_INFO)?;
        write_element_size(io, info_content.len() as u64)?;
        let info_data_start = io.position()?;
        io.write_all(&info_content)?;

        // Duration 在文件中的绝对偏移
        self.duration_offset = info_data_start + duration_data_pos as u64;

        // Tracks
        let mut tracks_content = Vec::new();
        for (i, stream) in streams.iter().enumerate() {
            let track_entry = build_track_entry(stream, i as u8 + 1)?;
            tracks_content.extend_from_slice(&track_entry);
        }

        write_element_id(io, TRACKS)?;
        write_element_size(io, tracks_content.len() as u64)?;
        io.write_all(&tracks_content)?;

        debug!(
            "MKV: 写入 EBML header + Segment + Info + Tracks, {} 个轨道",
            streams.len()
        );
        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        let timestamp_ms = self.pts_to_ms(packet);

        // 需要新 Cluster?
        let need_new_cluster = !self.cluster_open
            || (timestamp_ms - self.cluster_timestamp >= MAX_CLUSTER_DURATION_MS);

        if need_new_cluster {
            self.flush_cluster(io)?;
            self.start_cluster(timestamp_ms);
        }

        // 更新最大时间戳
        if timestamp_ms > self.max_timestamp_ms {
            self.max_timestamp_ms = timestamp_ms;
        }

        // 构建 SimpleBlock
        let track_number = self
            .tracks
            .iter()
            .find(|t| t.stream_index == packet.stream_index)
            .map(|t| t.track_number)
            .unwrap_or(1);

        let relative_ts = (timestamp_ms - self.cluster_timestamp) as i16;
        let flags: u8 = if packet.is_keyframe { 0x80 } else { 0x00 };

        // SimpleBlock 格式:
        // track_number (VINT) + timestamp_delta (2 bytes BE) + flags (1 byte) + frame_data
        let mut block_data = Vec::new();
        // Track number as VINT (简化: 1 字节 VINT, 支持 track 1-127)
        block_data.push(0x80 | track_number);
        block_data.extend_from_slice(&relative_ts.to_be_bytes());
        block_data.push(flags);
        block_data.extend_from_slice(&packet.data);

        // 写入到 Cluster 缓冲
        write_element_id_buf(&mut self.cluster_buf, SIMPLE_BLOCK);
        write_element_size_buf(&mut self.cluster_buf, block_data.len() as u64);
        self.cluster_buf.extend_from_slice(&block_data);

        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 刷新最后一个 Cluster
        self.flush_cluster(io)?;

        // 回填 Duration
        if io.is_seekable() && self.duration_offset > 0 {
            let duration_ms = self.max_timestamp_ms as f64;
            let current = io.position()?;
            io.seek(std::io::SeekFrom::Start(self.duration_offset))?;
            io.write_all(&duration_ms.to_be_bytes())?;
            io.seek(std::io::SeekFrom::Start(current))?;
        }

        debug!("MKV: trailer 完成, duration={}ms", self.max_timestamp_ms);
        Ok(())
    }
}

// ============================================================
// EBML 写入工具
// ============================================================

/// 写 EBML 元素 ID 到 IoContext
fn write_element_id(io: &mut IoContext, id: u32) -> TaoResult<()> {
    let bytes = id_to_bytes(id);
    io.write_all(&bytes)
}

/// 写 EBML 元素 ID 到缓冲区
fn write_element_id_buf(buf: &mut Vec<u8>, id: u32) {
    buf.extend_from_slice(&id_to_bytes(id));
}

/// 将 ID 转为字节 (保留前导位)
fn id_to_bytes(id: u32) -> Vec<u8> {
    if id <= 0xFF {
        vec![id as u8]
    } else if id <= 0xFFFF {
        vec![(id >> 8) as u8, id as u8]
    } else if id <= 0xFF_FFFF {
        vec![(id >> 16) as u8, (id >> 8) as u8, id as u8]
    } else {
        vec![
            (id >> 24) as u8,
            (id >> 16) as u8,
            (id >> 8) as u8,
            id as u8,
        ]
    }
}

/// 写 EBML 元素大小 (VINT 编码) 到 IoContext
fn write_element_size(io: &mut IoContext, size: u64) -> TaoResult<()> {
    let bytes = size_to_vint(size);
    io.write_all(&bytes)
}

/// 写 EBML 元素大小到缓冲区
fn write_element_size_buf(buf: &mut Vec<u8>, size: u64) {
    buf.extend_from_slice(&size_to_vint(size));
}

/// 将大小编码为 VINT
fn size_to_vint(size: u64) -> Vec<u8> {
    if size < 0x7F {
        vec![0x80 | size as u8]
    } else if size < 0x3FFF {
        vec![0x40 | (size >> 8) as u8, size as u8]
    } else if size < 0x1F_FFFF {
        vec![0x20 | (size >> 16) as u8, (size >> 8) as u8, size as u8]
    } else if size < 0x0FFF_FFFF {
        vec![
            0x10 | (size >> 24) as u8,
            (size >> 16) as u8,
            (size >> 8) as u8,
            size as u8,
        ]
    } else {
        // 使用 8 字节
        let mut bytes = vec![0x01];
        for i in (0..7).rev() {
            bytes.push((size >> (i * 8)) as u8);
        }
        bytes
    }
}

/// 写 uint 元素 (ID + size + uint data)
fn write_uint_full_element(buf: &mut Vec<u8>, id: u32, value: u64) {
    write_element_id_buf(buf, id);
    write_uint_element_buf(buf, value);
}

/// 写 uint 数据 (size + data)
fn write_uint_element_buf(buf: &mut Vec<u8>, value: u64) {
    let bytes = uint_to_bytes(value);
    write_element_size_buf(buf, bytes.len() as u64);
    buf.extend_from_slice(&bytes);
}

/// uint 转最小字节数
fn uint_to_bytes(value: u64) -> Vec<u8> {
    if value == 0 {
        vec![0]
    } else if value <= 0xFF {
        vec![value as u8]
    } else if value <= 0xFFFF {
        vec![(value >> 8) as u8, value as u8]
    } else if value <= 0xFF_FFFF {
        vec![(value >> 16) as u8, (value >> 8) as u8, value as u8]
    } else if value <= 0xFFFF_FFFF {
        value.to_be_bytes()[4..].to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

/// 写 string 元素
fn write_string_element_buf(buf: &mut Vec<u8>, id: u32, value: &str) {
    write_element_id_buf(buf, id);
    write_element_size_buf(buf, value.len() as u64);
    buf.extend_from_slice(value.as_bytes());
}

/// 写 float64 元素
fn write_float_element_buf(buf: &mut Vec<u8>, id: u32, value: f64) {
    write_element_id_buf(buf, id);
    write_element_size_buf(buf, 8);
    buf.extend_from_slice(&value.to_be_bytes());
}

/// 写 binary 元素
fn write_binary_element_buf(buf: &mut Vec<u8>, id: u32, data: &[u8]) {
    write_element_id_buf(buf, id);
    write_element_size_buf(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

// ============================================================
// TrackEntry 构建
// ============================================================

/// CodecId → Matroska codec string
fn codec_id_to_mkv(codec_id: CodecId) -> TaoResult<&'static str> {
    match codec_id {
        CodecId::H264 => Ok("V_MPEG4/ISO/AVC"),
        CodecId::H265 => Ok("V_MPEGH/ISO/HEVC"),
        CodecId::Vp9 => Ok("V_VP9"),
        CodecId::Av1 => Ok("V_AV1"),
        CodecId::Aac => Ok("A_AAC"),
        CodecId::Mp3 => Ok("A_MPEG/L3"),
        CodecId::Opus => Ok("A_OPUS"),
        CodecId::Flac => Ok("A_FLAC"),
        CodecId::Vorbis => Ok("A_VORBIS"),
        CodecId::Ac3 => Ok("A_AC3"),
        CodecId::Eac3 => Ok("A_EAC3"),
        _ => Err(TaoError::Unsupported(format!(
            "MKV: 不支持编解码器 {}",
            codec_id
        ))),
    }
}

/// 构建一个 TrackEntry
fn build_track_entry(stream: &Stream, track_number: u8) -> TaoResult<Vec<u8>> {
    let codec_id_str = codec_id_to_mkv(stream.codec_id)?;

    let mut content = Vec::new();

    write_uint_full_element(&mut content, TRACK_NUMBER, track_number as u64);
    write_uint_full_element(&mut content, TRACK_UID, track_number as u64);

    let track_type: u64 = match stream.media_type {
        MediaType::Video => 1,
        MediaType::Audio => 2,
        _ => 0,
    };
    write_uint_full_element(&mut content, TRACK_TYPE, track_type);

    write_string_element_buf(&mut content, TRACK_CODEC_ID, codec_id_str);

    // CodecPrivate
    if !stream.extra_data.is_empty() {
        write_binary_element_buf(&mut content, TRACK_CODEC_PRIVATE, &stream.extra_data);
    }

    // DefaultDuration (nanoseconds per frame)
    if stream.time_base.num > 0 && stream.time_base.den > 0 {
        let duration_ns = stream.time_base.num as u64 * 1_000_000_000 / stream.time_base.den as u64;
        if duration_ns > 0 {
            write_uint_full_element(&mut content, TRACK_DEFAULT_DURATION, duration_ns);
        }
    }

    match &stream.params {
        StreamParams::Video(v) => {
            let mut video = Vec::new();
            write_uint_full_element(&mut video, VIDEO_PIXEL_WIDTH, v.width as u64);
            write_uint_full_element(&mut video, VIDEO_PIXEL_HEIGHT, v.height as u64);

            write_element_id_buf(&mut content, VIDEO_SETTINGS);
            write_element_size_buf(&mut content, video.len() as u64);
            content.extend_from_slice(&video);
        }
        StreamParams::Audio(a) => {
            let mut audio = Vec::new();
            write_float_element_buf(&mut audio, AUDIO_SAMPLING_FREQ, a.sample_rate as f64);
            write_uint_full_element(&mut audio, AUDIO_CHANNELS, a.channel_layout.channels as u64);
            if a.sample_format.bytes_per_sample() > 0 {
                write_uint_full_element(
                    &mut audio,
                    AUDIO_BIT_DEPTH,
                    (a.sample_format.bytes_per_sample() * 8) as u64,
                );
            }

            write_element_id_buf(&mut content, AUDIO_SETTINGS);
            write_element_size_buf(&mut content, audio.len() as u64);
            content.extend_from_slice(&audio);
        }
        _ => {}
    }

    let mut buf = Vec::new();
    write_element_id_buf(&mut buf, TRACK_ENTRY);
    write_element_size_buf(&mut buf, content.len() as u64);
    buf.extend_from_slice(&content);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;
    use tao_core::{ChannelLayout, PixelFormat, Rational, SampleFormat};

    use crate::stream::{AudioStreamParams, VideoStreamParams};

    fn make_video_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 1000),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: vec![0x01, 0x42, 0x00, 0x1E],
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

    fn make_audio_stream() -> Stream {
        Stream {
            index: 1,
            media_type: MediaType::Audio,
            codec_id: CodecId::Aac,
            time_base: Rational::new(1, 1000),
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

    #[test]
    fn test_codec_id_映射() {
        assert_eq!(codec_id_to_mkv(CodecId::H264).unwrap(), "V_MPEG4/ISO/AVC");
        assert_eq!(codec_id_to_mkv(CodecId::H265).unwrap(), "V_MPEGH/ISO/HEVC");
        assert_eq!(codec_id_to_mkv(CodecId::Aac).unwrap(), "A_AAC");
        assert!(codec_id_to_mkv(CodecId::None).is_err());
    }

    #[test]
    fn test_vint_编码() {
        assert_eq!(size_to_vint(0), vec![0x80]);
        assert_eq!(size_to_vint(1), vec![0x81]);
        assert_eq!(size_to_vint(126), vec![0xFE]);
        assert_eq!(size_to_vint(127), vec![0x40, 0x7F]);
    }

    #[test]
    fn test_写入_仅视频() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream()];

        let mut muxer = MkvMuxer::create().unwrap();
        muxer.write_header(&mut io, &streams).unwrap();

        for i in 0..5 {
            let mut pkt = Packet::from_data(vec![0xAA; 100]);
            pkt.stream_index = 0;
            pkt.pts = i * 33;
            pkt.dts = i * 33;
            pkt.is_keyframe = i == 0;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        muxer.write_trailer(&mut io).unwrap();
        let pos = io.position().unwrap();
        assert!(pos > 100, "应写入了数据");
    }

    #[test]
    fn test_写入_音视频() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream(), make_audio_stream()];

        let mut muxer = MkvMuxer::create().unwrap();
        muxer.write_header(&mut io, &streams).unwrap();

        // 视频
        for i in 0..3 {
            let mut pkt = Packet::from_data(vec![0xBB; 200]);
            pkt.stream_index = 0;
            pkt.pts = i * 33;
            pkt.dts = i * 33;
            pkt.is_keyframe = i == 0;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        // 音频
        for i in 0..5 {
            let mut pkt = Packet::from_data(vec![0xCC; 50]);
            pkt.stream_index = 1;
            pkt.pts = i * 23;
            pkt.dts = i * 23;
            pkt.is_keyframe = true;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        muxer.write_trailer(&mut io).unwrap();
        let pos = io.position().unwrap();
        assert!(pos > 500, "应有 EBML + Segment + Clusters");
    }

    #[test]
    fn test_webm_格式() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let mut stream = make_video_stream();
        stream.codec_id = CodecId::Vp9;

        let mut muxer = MkvMuxer::create_webm().unwrap();
        assert_eq!(muxer.format_id(), FormatId::Webm);
        assert_eq!(muxer.name(), "webm");
        muxer.write_header(&mut io, &[stream]).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_空流报错() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let mut muxer = MkvMuxer::create().unwrap();
        assert!(muxer.write_header(&mut io, &[]).is_err());
    }
}
