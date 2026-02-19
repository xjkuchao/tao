//! Matroska/WebM 解封装器.
//!
//! 支持 Matroska (.mkv/.mka) 和 WebM (.webm) 容器格式.
//! 基于 EBML (Extensible Binary Meta Language) 编码.
//!
//! # Matroska 结构概览
//! ```text
//! EBML Header        文件头 (DocType: "matroska" 或 "webm")
//! Segment            根容器
//! ├── SeekHead       索引 (可选)
//! ├── Info           段信息 (时间刻度, 时长)
//! ├── Tracks         轨道定义 (编解码器, 参数)
//! ├── Cluster        数据簇
//! │   ├── Timestamp  簇时间戳
//! │   └── SimpleBlock / BlockGroup  音视频数据块
//! ├── Cues           时间索引 (可选)
//! └── Tags           元数据标签 (可选)
//! ```

pub mod ebml;

use bytes::Bytes;
use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

use self::ebml::*;

/// Matroska 轨道信息 (解析 Tracks 时暂存)
struct TrackInfo {
    track_number: u64,
    track_type: u64,
    codec_id_str: String,
    codec_private: Vec<u8>,
    default_duration: u64,
    // 视频
    pixel_width: u32,
    pixel_height: u32,
    // 音频
    sample_rate: f64,
    channels: u32,
    bit_depth: u32,
}

impl TrackInfo {
    fn new() -> Self {
        Self {
            track_number: 0,
            track_type: 0,
            codec_id_str: String::new(),
            codec_private: Vec::new(),
            default_duration: 0,
            pixel_width: 0,
            pixel_height: 0,
            sample_rate: 0.0,
            channels: 0,
            bit_depth: 0,
        }
    }
}

/// Matroska 解封装器
pub struct MkvDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// 轨道号 → 流索引的映射
    track_map: Vec<(u64, usize)>,
    /// 时间刻度 (纳秒/tick, 默认 1_000_000 即 1ms)
    timescale_ns: u64,
    /// 时长 (纳秒)
    duration_ns: Option<f64>,
    /// Segment 数据区起始偏移
    segment_offset: u64,
    /// Segment 数据区结束偏移
    segment_end: u64,
    /// 当前 Cluster 时间戳 (tick)
    cluster_timestamp: i64,
    /// 当前 Cluster 剩余大小
    cluster_remaining: u64,
    /// 是否已进入 Cluster 区域
    in_cluster: bool,
    /// 是否为 WebM 格式
    is_webm: bool,
}

impl MkvDemuxer {
    /// 创建 Matroska 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            track_map: Vec::new(),
            timescale_ns: 1_000_000,
            duration_ns: None,
            segment_offset: 0,
            segment_end: u64::MAX,
            cluster_timestamp: 0,
            cluster_remaining: 0,
            in_cluster: false,
            is_webm: false,
        }))
    }

    /// 解析 EBML 头部
    fn parse_ebml_header(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let (id, size, _) = read_element_header(io)?;
        if id != EBML_HEADER {
            return Err(TaoError::InvalidData(
                "不是有效的 EBML/Matroska 文件".into(),
            ));
        }

        let end = io.position()? + size;
        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            match eid {
                EBML_DOC_TYPE => {
                    let doc_type = read_string(io, esize)?;
                    debug!("MKV: DocType = {doc_type}");
                    self.is_webm = doc_type == "webm";
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(())
    }

    /// 解析 Segment Info
    fn parse_segment_info(&mut self, io: &mut IoContext, size: u64) -> TaoResult<()> {
        let end = io.position()? + size;
        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            match eid {
                INFO_TIMESCALE => {
                    self.timescale_ns = read_uint(io, esize)?;
                    debug!("MKV: TimescaleNs = {}", self.timescale_ns);
                }
                INFO_DURATION => {
                    let dur = read_float(io, esize)?;
                    self.duration_ns = Some(dur * self.timescale_ns as f64);
                    debug!("MKV: Duration = {dur} ticks");
                }
                INFO_TITLE | INFO_MUXING_APP | INFO_WRITING_APP => {
                    let _s = read_string(io, esize)?;
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(())
    }

    /// 解析 Tracks 元素
    fn parse_tracks(&mut self, io: &mut IoContext, size: u64) -> TaoResult<()> {
        let end = io.position()? + size;
        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            if eid == TRACK_ENTRY {
                let track = self.parse_track_entry(io, esize)?;
                self.add_track(track);
            } else {
                io.skip(esize as usize)?;
            }
        }
        Ok(())
    }

    /// 解析单个 TrackEntry
    fn parse_track_entry(&self, io: &mut IoContext, size: u64) -> TaoResult<TrackInfo> {
        let end = io.position()? + size;
        let mut track = TrackInfo::new();

        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            match eid {
                TRACK_NUMBER => track.track_number = read_uint(io, esize)?,
                TRACK_UID => {
                    let _uid = read_uint(io, esize)?;
                }
                TRACK_TYPE => track.track_type = read_uint(io, esize)?,
                TRACK_CODEC_ID => {
                    track.codec_id_str = read_string(io, esize)?;
                }
                TRACK_CODEC_PRIVATE => {
                    track.codec_private = read_binary(io, esize)?;
                }
                TRACK_DEFAULT_DURATION => {
                    track.default_duration = read_uint(io, esize)?;
                }
                VIDEO_SETTINGS => {
                    self.parse_video_settings(io, esize, &mut track)?;
                }
                AUDIO_SETTINGS => {
                    self.parse_audio_settings(io, esize, &mut track)?;
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(track)
    }

    /// 解析视频设置
    fn parse_video_settings(
        &self,
        io: &mut IoContext,
        size: u64,
        track: &mut TrackInfo,
    ) -> TaoResult<()> {
        let end = io.position()? + size;
        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            match eid {
                VIDEO_PIXEL_WIDTH => {
                    track.pixel_width = read_uint(io, esize)? as u32;
                }
                VIDEO_PIXEL_HEIGHT => {
                    track.pixel_height = read_uint(io, esize)? as u32;
                }
                VIDEO_DISPLAY_WIDTH | VIDEO_DISPLAY_HEIGHT => {
                    let _v = read_uint(io, esize)?;
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(())
    }

    /// 解析音频设置
    fn parse_audio_settings(
        &self,
        io: &mut IoContext,
        size: u64,
        track: &mut TrackInfo,
    ) -> TaoResult<()> {
        let end = io.position()? + size;
        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            match eid {
                AUDIO_SAMPLING_FREQ => {
                    track.sample_rate = read_float(io, esize)?;
                }
                AUDIO_CHANNELS => {
                    track.channels = read_uint(io, esize)? as u32;
                }
                AUDIO_BIT_DEPTH => {
                    track.bit_depth = read_uint(io, esize)? as u32;
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(())
    }

    /// 将 TrackInfo 转换为 Stream 并添加
    fn add_track(&mut self, track: TrackInfo) {
        let stream_index = self.streams.len();
        let codec_id = mkv_codec_to_id(&track.codec_id_str);

        let (media_type, params) = match track.track_type {
            1 => {
                // 视频
                let frame_rate = if track.default_duration > 0 {
                    let fps = 1_000_000_000.0 / track.default_duration as f64;
                    Rational::new((fps * 1000.0) as i32, 1000)
                } else {
                    Rational::new(0, 1)
                };

                (
                    MediaType::Video,
                    StreamParams::Video(VideoStreamParams {
                        width: track.pixel_width,
                        height: track.pixel_height,
                        pixel_format: tao_core::PixelFormat::Yuv420p,
                        frame_rate,
                        sample_aspect_ratio: Rational::new(1, 1),
                        bit_rate: 0,
                    }),
                )
            }
            2 => {
                // 音频
                let sr = if track.sample_rate > 0.0 {
                    track.sample_rate as u32
                } else {
                    48000
                };
                let ch = if track.channels > 0 {
                    track.channels
                } else {
                    2
                };

                (
                    MediaType::Audio,
                    StreamParams::Audio(AudioStreamParams {
                        sample_rate: sr,
                        channel_layout: ChannelLayout::from_channels(ch),
                        sample_format: SampleFormat::F32,
                        bit_rate: 0,
                        frame_size: 0,
                    }),
                )
            }
            17 => {
                // 字幕
                (MediaType::Subtitle, StreamParams::Subtitle)
            }
            _ => (MediaType::Data, StreamParams::Other),
        };

        // 时间基: 1ns * timescale_ns → 以 timescale_ns 纳秒为单位
        // 为了方便, 使用 1/1000 (毫秒) 作为时间基
        let time_base = Rational::new(1, 1000);

        let stream = Stream {
            index: stream_index,
            media_type,
            codec_id,
            time_base,
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: track.codec_private.clone(),
            params,
            metadata: Vec::new(),
        };

        debug!(
            "MKV: 轨道 #{stream_index} (num={}) type={} codec={}",
            track.track_number, track.track_type, track.codec_id_str,
        );

        self.track_map.push((track.track_number, stream_index));
        self.streams.push(stream);
    }

    /// 查找轨道号对应的流索引
    fn find_stream_index(&self, track_number: u64) -> Option<usize> {
        self.track_map
            .iter()
            .find(|(tn, _)| *tn == track_number)
            .map(|(_, idx)| *idx)
    }

    /// 读取 Cluster 内的下一个 SimpleBlock 或 Block
    fn read_block_from_cluster(&mut self, io: &mut IoContext) -> TaoResult<Option<Packet>> {
        while self.cluster_remaining > 0 {
            let (eid, esize, hdr_len) = match read_element_header(io) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            };

            let consumed = u64::from(hdr_len) + esize;
            if consumed > self.cluster_remaining {
                self.cluster_remaining = 0;
                return Ok(None);
            }
            self.cluster_remaining -= consumed;

            match eid {
                CLUSTER_TIMESTAMP => {
                    self.cluster_timestamp = read_uint(io, esize)? as i64;
                }
                SIMPLE_BLOCK => {
                    return self.parse_simple_block(io, esize).map(Some);
                }
                BLOCK_GROUP => {
                    return self.parse_block_group(io, esize);
                }
                _ => {
                    io.skip(esize as usize)?;
                }
            }
        }
        Ok(None)
    }

    /// 解析 SimpleBlock
    fn parse_simple_block(&self, io: &mut IoContext, size: u64) -> TaoResult<Packet> {
        if size < 4 {
            return Err(TaoError::InvalidData("MKV: SimpleBlock 太小".into()));
        }

        // 读取轨道号 (EBML 变长整数, 但保留标记位)
        let (track_vint, vint_len) = read_element_size(io)?;
        let track_number = track_vint;

        // 16-bit 相对时间戳 (有符号, 大端)
        let ts_hi = io.read_u8()? as i16;
        let ts_lo = io.read_u8()?;
        let relative_ts = ((ts_hi << 8) | ts_lo as i16) as i64;

        // 标志字节
        let flags = io.read_u8()?;
        let is_keyframe = (flags & 0x80) != 0;

        // 剩余数据是帧数据
        let header_consumed = u64::from(vint_len) + 3;
        let data_size = size - header_consumed;
        let data = io.read_bytes(data_size as usize)?;

        let abs_ts = self.cluster_timestamp + relative_ts;
        // 转换为毫秒 (time_base = 1/1000)
        let pts_ms = abs_ts * self.timescale_ns as i64 / 1_000_000;

        let stream_index = self.find_stream_index(track_number).unwrap_or(0);

        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = stream_index;
        pkt.pts = pts_ms;
        pkt.dts = pts_ms;
        pkt.is_keyframe = is_keyframe;

        if let Some(stream) = self.streams.get(stream_index) {
            pkt.time_base = stream.time_base;
        }

        Ok(pkt)
    }

    /// 解析 BlockGroup (提取 Block)
    fn parse_block_group(&self, io: &mut IoContext, size: u64) -> TaoResult<Option<Packet>> {
        let end = io.position()? + size;
        let mut result = None;

        while io.position()? < end {
            let (eid, esize, _) = read_element_header(io)?;
            if eid == BLOCK {
                result = Some(self.parse_simple_block(io, esize)?);
            } else {
                io.skip(esize as usize)?;
            }
        }

        Ok(result)
    }
}

impl Demuxer for MkvDemuxer {
    fn format_id(&self) -> FormatId {
        if self.is_webm {
            FormatId::Webm
        } else {
            FormatId::Matroska
        }
    }

    fn name(&self) -> &str {
        if self.is_webm { "webm" } else { "matroska" }
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 1) 解析 EBML 头部
        self.parse_ebml_header(io)?;

        // 2) 解析 Segment
        let (seg_id, seg_size, _) = read_element_header(io)?;
        if seg_id != SEGMENT {
            return Err(TaoError::InvalidData("MKV: 未找到 Segment 元素".into()));
        }
        self.segment_offset = io.position()?;
        self.segment_end = if seg_size == EBML_UNKNOWN_SIZE {
            u64::MAX
        } else {
            self.segment_offset + seg_size
        };

        // 3) 扫描 Segment 的顶层元素直到遇到第一个 Cluster
        while io.position()? < self.segment_end {
            let pos = io.position()?;
            let (eid, esize, _) = match read_element_header(io) {
                Ok(v) => v,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };

            match eid {
                SEGMENT_INFO => {
                    self.parse_segment_info(io, esize)?;
                }
                TRACKS => {
                    self.parse_tracks(io, esize)?;
                }
                CLUSTER => {
                    // 到达第一个 Cluster, 记录位置并回退
                    io.seek(std::io::SeekFrom::Start(pos))?;
                    break;
                }
                _ => {
                    // SeekHead, Cues, Tags 等 → 跳过
                    if esize != EBML_UNKNOWN_SIZE {
                        io.skip(esize as usize)?;
                    } else {
                        break;
                    }
                }
            }
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData("MKV: 未找到任何轨道".into()));
        }

        // 更新时长
        if let Some(dur_ns) = self.duration_ns {
            for stream in &mut self.streams {
                // time_base = 1/1000, 时长转为毫秒
                stream.duration = (dur_ns / 1_000_000.0) as i64;
            }
        }

        debug!(
            "打开 MKV: {} 个轨道, webm={}",
            self.streams.len(),
            self.is_webm,
        );
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        loop {
            // 如果在 Cluster 内, 尝试读取 Block
            if self.in_cluster {
                if let Some(pkt) = self.read_block_from_cluster(io)? {
                    return Ok(pkt);
                }
                self.in_cluster = false;
            }

            // 读取下一个顶层元素
            if io.position()? >= self.segment_end {
                return Err(TaoError::Eof);
            }

            let (eid, esize, _) = match read_element_header(io) {
                Ok(v) => v,
                Err(TaoError::Eof) => return Err(TaoError::Eof),
                Err(e) => return Err(e),
            };

            match eid {
                CLUSTER => {
                    self.in_cluster = true;
                    self.cluster_remaining = if esize == EBML_UNKNOWN_SIZE {
                        u64::MAX
                    } else {
                        esize
                    };
                    self.cluster_timestamp = 0;
                }
                _ => {
                    // 跳过非 Cluster 元素
                    if esize != EBML_UNKNOWN_SIZE {
                        io.skip(esize as usize)?;
                    } else {
                        return Err(TaoError::Eof);
                    }
                }
            }
        }
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("MKV seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        self.duration_ns.map(|ns| ns / 1_000_000_000.0)
    }
}

/// Matroska CodecID → tao CodecId 映射
fn mkv_codec_to_id(codec_str: &str) -> CodecId {
    match codec_str {
        // 视频
        "V_MPEG4/ISO/AVC" => CodecId::H264,
        "V_MPEGH/ISO/HEVC" => CodecId::H265,
        "V_VP8" => CodecId::Vp8,
        "V_VP9" => CodecId::Vp9,
        "V_AV1" => CodecId::Av1,
        "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/ASP" | "V_MPEG4/ISO/AP" => CodecId::Mpeg4,
        "V_MPEG1" => CodecId::Mpeg1Video,
        "V_MPEG2" => CodecId::Mpeg2Video,
        "V_THEORA" => CodecId::Theora,
        "V_MS/VFW/FOURCC" => CodecId::RawVideo,
        // 音频
        "A_AAC" => CodecId::Aac,
        "A_MPEG/L3" => CodecId::Mp3,
        "A_MPEG/L2" => CodecId::Mp2,
        "A_OPUS" | "A_OPUS/EXPERIMENTAL" => CodecId::Opus,
        "A_VORBIS" => CodecId::Vorbis,
        "A_FLAC" => CodecId::Flac,
        "A_ALAC" => CodecId::Alac,
        "A_AC3" => CodecId::Ac3,
        "A_EAC3" => CodecId::Eac3,
        "A_DTS" => CodecId::Dts,
        "A_PCM/INT/LIT" => CodecId::PcmS16le,
        "A_PCM/INT/BIG" => CodecId::PcmS16be,
        "A_PCM/FLOAT/IEEE" => CodecId::PcmF32le,
        // 字幕
        "S_TEXT/UTF8" | "S_TEXT/SSA" | "S_TEXT/ASS" => CodecId::Ass,
        "S_TEXT/WEBVTT" => CodecId::Webvtt,
        _ => CodecId::None,
    }
}

/// Matroska 格式探测器
pub struct MkvProbe;

impl FormatProbe for MkvProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // EBML Header 魔数: 0x1A 0x45 0xDF 0xA3
        if data.len() >= 4
            && data[0] == 0x1A
            && data[1] == 0x45
            && data[2] == 0xDF
            && data[3] == 0xA3
        {
            // 进一步检查 DocType (如果数据足够长)
            return Some(crate::probe::SCORE_MAX);
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                let ext_lower = ext.to_lowercase();
                if matches!(ext_lower.as_str(), "mkv" | "mka" | "webm") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Matroska
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 辅助: 写入 EBML 变长整数 (ID, 不掩码)
    fn write_vint_id(buf: &mut Vec<u8>, id: u32) {
        if id < 0x80 {
            unreachable!("ID 不能小于 0x80");
        } else if id <= 0xFF {
            buf.push(id as u8);
        } else if id <= 0xFFFF {
            buf.push((id >> 8) as u8);
            buf.push(id as u8);
        } else if id <= 0xFF_FFFF {
            buf.push((id >> 16) as u8);
            buf.push((id >> 8) as u8);
            buf.push(id as u8);
        } else {
            buf.push((id >> 24) as u8);
            buf.push((id >> 16) as u8);
            buf.push((id >> 8) as u8);
            buf.push(id as u8);
        }
    }

    /// 辅助: 写入 EBML 变长整数 (大小, 加标记位)
    fn write_vint_size(buf: &mut Vec<u8>, size: u64) {
        if size < 0x7F {
            buf.push(0x80 | size as u8);
        } else if size < 0x3FFF {
            buf.push(0x40 | (size >> 8) as u8);
            buf.push(size as u8);
        } else {
            // 简化: 3 字节
            buf.push(0x20 | (size >> 16) as u8);
            buf.push((size >> 8) as u8);
            buf.push(size as u8);
        }
    }

    /// 辅助: 写入 EBML 元素 (ID + size + content)
    fn write_element(buf: &mut Vec<u8>, id: u32, content: &[u8]) {
        write_vint_id(buf, id);
        write_vint_size(buf, content.len() as u64);
        buf.extend_from_slice(content);
    }

    /// 辅助: 写入 uint 元素
    fn write_uint_element(buf: &mut Vec<u8>, id: u32, val: u64) {
        let bytes = if val == 0 {
            vec![0]
        } else {
            let mut b = val.to_be_bytes().to_vec();
            while b.len() > 1 && b[0] == 0 {
                b.remove(0);
            }
            b
        };
        write_element(buf, id, &bytes);
    }

    /// 辅助: 写入 float 元素 (8 字节)
    fn write_float_element(buf: &mut Vec<u8>, id: u32, val: f64) {
        let bytes = val.to_bits().to_be_bytes();
        write_element(buf, id, &bytes);
    }

    /// 辅助: 写入 string 元素
    fn write_string_element(buf: &mut Vec<u8>, id: u32, s: &str) {
        write_element(buf, id, s.as_bytes());
    }

    /// 构造一个最小的 MKV 文件
    fn build_minimal_mkv() -> Vec<u8> {
        let mut data = Vec::new();

        // EBML Header
        let mut ebml_content = Vec::new();
        write_string_element(&mut ebml_content, EBML_DOC_TYPE, "matroska");
        write_element(&mut data, EBML_HEADER, &ebml_content);

        // Segment (未知大小)
        write_vint_id(&mut data, SEGMENT);
        data.push(0x01); // 8 字节 size
        data.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

        // Segment Info
        let mut info_content = Vec::new();
        write_uint_element(&mut info_content, INFO_TIMESCALE, 1_000_000);
        write_float_element(&mut info_content, INFO_DURATION, 5000.0);
        write_element(&mut data, SEGMENT_INFO, &info_content);

        // Tracks
        let mut tracks_content = Vec::new();
        {
            // Video track
            let mut track_content = Vec::new();
            write_uint_element(&mut track_content, TRACK_NUMBER, 1);
            write_uint_element(&mut track_content, TRACK_TYPE, 1); // video
            write_string_element(&mut track_content, TRACK_CODEC_ID, "V_VP9");
            // Video settings
            let mut video = Vec::new();
            write_uint_element(&mut video, VIDEO_PIXEL_WIDTH, 1280);
            write_uint_element(&mut video, VIDEO_PIXEL_HEIGHT, 720);
            write_element(&mut track_content, VIDEO_SETTINGS, &video);

            write_element(&mut tracks_content, TRACK_ENTRY, &track_content);
        }
        {
            // Audio track
            let mut track_content = Vec::new();
            write_uint_element(&mut track_content, TRACK_NUMBER, 2);
            write_uint_element(&mut track_content, TRACK_TYPE, 2); // audio
            write_string_element(&mut track_content, TRACK_CODEC_ID, "A_OPUS");
            // Audio settings
            let mut audio = Vec::new();
            let sr_bytes = (48000.0f64).to_bits().to_be_bytes();
            write_element(&mut audio, AUDIO_SAMPLING_FREQ, &sr_bytes);
            write_uint_element(&mut audio, AUDIO_CHANNELS, 2);
            write_element(&mut track_content, AUDIO_SETTINGS, &audio);

            write_element(&mut tracks_content, TRACK_ENTRY, &track_content);
        }
        write_element(&mut data, TRACKS, &tracks_content);

        // Cluster with SimpleBlocks
        let mut cluster_content = Vec::new();
        write_uint_element(&mut cluster_content, CLUSTER_TIMESTAMP, 0);

        // SimpleBlock for track 1 (video): keyframe
        {
            let mut block = vec![
                0x81, // track number = 1 (vint)
                0x00, // timestamp relative = 0 (high)
                0x00, // timestamp relative = 0 (low)
                0x80, // flags: keyframe
            ];
            block.extend_from_slice(&[0xDE, 0xAD]); // frame data
            write_element(&mut cluster_content, SIMPLE_BLOCK, &block);
        }
        // SimpleBlock for track 2 (audio)
        {
            let mut block = vec![
                0x82, // track number = 2
                0x00, 0x00, 0x80, // timestamp=0, keyframe
            ];
            block.extend_from_slice(&[0xBE, 0xEF, 0xCA, 0xFE]); // frame data
            write_element(&mut cluster_content, SIMPLE_BLOCK, &block);
        }

        write_element(&mut data, CLUSTER, &cluster_content);

        data
    }

    #[test]
    fn test_probe_mkv_magic() {
        let probe = MkvProbe;
        let data = [0x1A, 0x45, 0xDF, 0xA3];
        assert_eq!(probe.probe(&data, None), Some(crate::probe::SCORE_MAX));
    }

    #[test]
    fn test_probe_mkv_extension() {
        let probe = MkvProbe;
        assert!(probe.probe(&[], Some("video.mkv")).is_some());
        assert!(probe.probe(&[], Some("video.webm")).is_some());
        assert!(probe.probe(&[], Some("audio.mka")).is_some());
        assert!(probe.probe(&[], Some("video.mp4")).is_none());
    }

    #[test]
    fn test_parse_minimal_mkv() {
        let mkv = build_minimal_mkv();
        let backend = MemoryBackend::from_data(mkv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = MkvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 2, "应该有 2 个轨道");

        // 视频轨道
        assert_eq!(streams[0].media_type, MediaType::Video);
        assert_eq!(streams[0].codec_id, CodecId::Vp9);
        if let StreamParams::Video(ref v) = streams[0].params {
            assert_eq!(v.width, 1280);
            assert_eq!(v.height, 720);
        } else {
            panic!("应该是视频参数");
        }

        // 音频轨道
        assert_eq!(streams[1].media_type, MediaType::Audio);
        assert_eq!(streams[1].codec_id, CodecId::Opus);
        if let StreamParams::Audio(ref a) = streams[1].params {
            assert_eq!(a.sample_rate, 48000);
            assert_eq!(a.channel_layout.channels, 2);
        } else {
            panic!("应该是音频参数");
        }
    }

    #[test]
    fn test_read_packets() {
        let mkv = build_minimal_mkv();
        let backend = MemoryBackend::from_data(mkv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = MkvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        // 第一个包: 视频
        let pkt0 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt0.stream_index, 0);
        assert!(pkt0.is_keyframe);
        assert_eq!(pkt0.data.as_ref(), &[0xDE, 0xAD]);

        // 第二个包: 音频
        let pkt1 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt1.stream_index, 1);
        assert_eq!(pkt1.data.as_ref(), &[0xBE, 0xEF, 0xCA, 0xFE]);
    }

    #[test]
    fn test_duration() {
        let mkv = build_minimal_mkv();
        let backend = MemoryBackend::from_data(mkv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = MkvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let dur = demuxer.duration().expect("应该有时长");
        // Duration = 5000 ticks * 1_000_000 ns/tick = 5s
        assert!((dur - 5.0).abs() < 0.01, "时长应约为 5 秒, 实际={dur}");
    }

    #[test]
    fn test_codec_id_mapping() {
        assert_eq!(mkv_codec_to_id("V_MPEG4/ISO/AVC"), CodecId::H264);
        assert_eq!(mkv_codec_to_id("V_VP9"), CodecId::Vp9);
        assert_eq!(mkv_codec_to_id("A_OPUS"), CodecId::Opus);
        assert_eq!(mkv_codec_to_id("A_AAC"), CodecId::Aac);
        assert_eq!(mkv_codec_to_id("A_FLAC"), CodecId::Flac);
        assert_eq!(mkv_codec_to_id("UNKNOWN"), CodecId::None);
    }
}
