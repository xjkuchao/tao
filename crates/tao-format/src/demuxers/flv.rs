//! FLV (Flash Video) 容器解封装器.
//!
//! FLV 是 Adobe Flash 使用的视频容器格式, 至今仍广泛用于直播推流 (RTMP).
//!
//! # FLV 文件结构
//! ```text
//! FLV Header (9 bytes):
//!   "FLV" (3 bytes)
//!   Version (1 byte, 通常 = 1)
//!   Flags (1 byte): bit0=audio, bit2=video
//!   DataOffset (4 bytes, BE): 头部大小 (通常 9)
//!
//! PreviousTagSize0 (4 bytes, BE): 0
//!
//! FLV Tag #1:
//!   TagType (1 byte): 8=Audio, 9=Video, 18=Script
//!   DataSize (3 bytes, BE)
//!   Timestamp (3 bytes, BE) + TimestampExtended (1 byte, 高8位)
//!   StreamID (3 bytes, BE): always 0
//!   TagData (DataSize bytes)
//! PreviousTagSize1 (4 bytes, BE)
//!
//! FLV Tag #2: ...
//! ```
//!
//! # 音频 Tag 数据
//! ```text
//! SoundFormat (4 bits): 10=AAC, 2=MP3, 14=MP3-8kHz, ...
//! SoundRate (2 bits): 0=5.5kHz, 1=11kHz, 2=22kHz, 3=44kHz
//! SoundSize (1 bit): 0=8bit, 1=16bit
//! SoundType (1 bit): 0=mono, 1=stereo
//! [AAC: AACPacketType (1 byte): 0=Sequence Header, 1=Raw]
//! ```
//!
//! # 视频 Tag 数据
//! ```text
//! FrameType (4 bits): 1=keyframe, 2=inter, ...
//! CodecID (4 bits): 7=AVC(H.264), 12=HEVC(H.265), 13=AV1
//! [AVC: AVCPacketType (1 byte): 0=SeqHeader, 1=NALU, 2=EndOfSeq]
//! [AVC: CompositionTimeOffset (3 bytes, BE, signed)]
//! ```

use bytes::Bytes;
use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{
    ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat, TaoError, TaoResult,
};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

/// FLV Tag 类型
const TAG_AUDIO: u8 = 8;
const TAG_VIDEO: u8 = 9;
const TAG_SCRIPT: u8 = 18;

/// FLV 视频编解码器 ID
const FLV_CODEC_H263: u8 = 2;
const FLV_CODEC_VP6: u8 = 4;
const FLV_CODEC_AVC: u8 = 7;
const FLV_CODEC_HEVC: u8 = 12;
const FLV_CODEC_AV1: u8 = 13;

/// FLV 音频编解码器 ID (SoundFormat)
const FLV_AUDIO_MP3: u8 = 2;
const FLV_AUDIO_AAC: u8 = 10;
const FLV_AUDIO_SPEEX: u8 = 11;

/// FLV 解封装器
pub struct FlvDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// 音频流索引 (None 表示还未创建)
    audio_stream_idx: Option<usize>,
    /// 视频流索引
    video_stream_idx: Option<usize>,
    /// 音频编解码器
    audio_codec_id: CodecId,
    /// 视频编解码器
    video_codec_id: CodecId,
    /// 文件时长 (毫秒, 来自 onMetaData)
    duration_ms: Option<f64>,
    /// 数据区起始偏移
    data_offset: u64,
    /// 是否已收到音频 sequence header
    audio_config_received: bool,
    /// 是否已收到视频 sequence header
    video_config_received: bool,
}

impl FlvDemuxer {
    /// 创建 FLV 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            audio_stream_idx: None,
            video_stream_idx: None,
            audio_codec_id: CodecId::None,
            video_codec_id: CodecId::None,
            duration_ms: None,
            data_offset: 0,
            audio_config_received: false,
            video_config_received: false,
        }))
    }

    /// 读取 FLV 文件头部
    fn read_header(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 读取 "FLV"
        let sig = io.read_bytes(3)?;
        if sig != b"FLV" {
            return Err(TaoError::InvalidData("不是 FLV 文件".into()));
        }

        let version = io.read_u8()?;
        let flags = io.read_u8()?;
        let data_offset = io.read_u32_be()?;

        debug!("FLV: version={version} flags=0x{flags:02X} data_offset={data_offset}");

        let has_audio = (flags & 0x04) != 0;
        let has_video = (flags & 0x01) != 0;

        self.data_offset = u64::from(data_offset);

        // 跳到数据区
        io.seek(std::io::SeekFrom::Start(self.data_offset))?;

        // 读取 PreviousTagSize0 (应该是 0)
        let _prev_size = io.read_u32_be()?;

        debug!("FLV: has_audio={has_audio} has_video={has_video}");
        Ok(())
    }

    /// 读取一个 FLV Tag 头部
    fn read_tag_header(&self, io: &mut IoContext) -> TaoResult<(u8, u32, u32)> {
        let tag_type = io.read_u8()?;
        let data_size = io.read_u24_be()?;
        let timestamp_low = io.read_u24_be()?;
        let timestamp_ext = io.read_u8()?;
        let timestamp = ((timestamp_ext as u32) << 24) | timestamp_low;
        let _stream_id = io.read_u24_be()?;

        Ok((tag_type, data_size, timestamp))
    }

    /// 处理音频 Tag
    fn handle_audio_tag(
        &mut self,
        io: &mut IoContext,
        data_size: u32,
        timestamp: u32,
    ) -> TaoResult<Option<Packet>> {
        if data_size == 0 {
            return Ok(None);
        }

        let audio_header = io.read_u8()?;
        let sound_format = (audio_header >> 4) & 0x0F;
        let sound_rate_idx = (audio_header >> 2) & 0x03;
        let _sound_size = (audio_header >> 1) & 0x01;
        let sound_type = audio_header & 0x01;

        let codec_id = match sound_format {
            FLV_AUDIO_AAC => CodecId::Aac,
            FLV_AUDIO_MP3 | 14 => CodecId::Mp3,
            FLV_AUDIO_SPEEX => CodecId::Opus, // 近似
            0 => CodecId::PcmS16le,           // Linear PCM
            3 => CodecId::PcmS16le,           // Linear PCM LE
            _ => CodecId::None,
        };

        // 创建音频流 (如果还没有)
        if self.audio_stream_idx.is_none() {
            let sample_rate = match sound_rate_idx {
                0 => 5512,
                1 => 11025,
                2 => 22050,
                3 => 44100,
                _ => 44100,
            };
            // AAC 总是 44100 或由 sequence header 决定
            let sr = if sound_format == FLV_AUDIO_AAC {
                44100
            } else {
                sample_rate
            };
            let channels = if sound_type == 1 { 2u32 } else { 1 };

            let idx = self.streams.len();
            self.audio_stream_idx = Some(idx);
            self.audio_codec_id = codec_id;

            let stream = Stream {
                index: idx,
                media_type: MediaType::Audio,
                codec_id,
                time_base: Rational::new(1, 1000), // 毫秒
                duration: -1,
                start_time: 0,
                nb_frames: 0,
                extra_data: Vec::new(),
                params: StreamParams::Audio(AudioStreamParams {
                    sample_rate: sr,
                    channel_layout: ChannelLayout::from_channels(channels),
                    sample_format: SampleFormat::F32,
                    bit_rate: 0,
                    frame_size: 1024,
                }),
                metadata: Vec::new(),
            };
            self.streams.push(stream);
        }

        let remaining = data_size - 1; // 减去 audio_header

        // AAC: 检查 AACPacketType
        if sound_format == FLV_AUDIO_AAC {
            if remaining < 1 {
                return Ok(None);
            }
            let aac_packet_type = io.read_u8()?;
            let payload_size = remaining - 1;

            if aac_packet_type == 0 {
                // Sequence Header (AudioSpecificConfig)
                let config = io.read_bytes(payload_size as usize)?;
                debug!("FLV: 收到 AAC sequence header, {} 字节", config.len());
                if let Some(idx) = self.audio_stream_idx {
                    self.streams[idx].extra_data = config;
                }
                self.audio_config_received = true;
                return Ok(None); // 不产生数据包
            }

            // Raw AAC data
            let data = io.read_bytes(payload_size as usize)?;
            let stream_index = self.audio_stream_idx.unwrap_or(0);
            let mut pkt = Packet::from_data(Bytes::from(data));
            pkt.stream_index = stream_index;
            pkt.pts = i64::from(timestamp);
            pkt.dts = i64::from(timestamp);
            pkt.is_keyframe = true;
            pkt.time_base = Rational::new(1, 1000);
            return Ok(Some(pkt));
        }

        // 非 AAC 音频
        let data = io.read_bytes(remaining as usize)?;
        let stream_index = self.audio_stream_idx.unwrap_or(0);
        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = stream_index;
        pkt.pts = i64::from(timestamp);
        pkt.dts = i64::from(timestamp);
        pkt.is_keyframe = true;
        pkt.time_base = Rational::new(1, 1000);
        Ok(Some(pkt))
    }

    /// 处理视频 Tag
    fn handle_video_tag(
        &mut self,
        io: &mut IoContext,
        data_size: u32,
        timestamp: u32,
    ) -> TaoResult<Option<Packet>> {
        if data_size == 0 {
            return Ok(None);
        }

        let video_header = io.read_u8()?;
        let frame_type = (video_header >> 4) & 0x0F;
        let codec_id_byte = video_header & 0x0F;
        let is_keyframe = frame_type == 1;

        let codec_id = match codec_id_byte {
            FLV_CODEC_H263 => CodecId::Mpeg4,
            FLV_CODEC_VP6 => CodecId::Vp8, // 近似
            FLV_CODEC_AVC => CodecId::H264,
            FLV_CODEC_HEVC => CodecId::H265,
            FLV_CODEC_AV1 => CodecId::Av1,
            _ => CodecId::None,
        };

        // 创建视频流 (如果还没有)
        if self.video_stream_idx.is_none() {
            let idx = self.streams.len();
            self.video_stream_idx = Some(idx);
            self.video_codec_id = codec_id;

            let stream = Stream {
                index: idx,
                media_type: MediaType::Video,
                codec_id,
                time_base: Rational::new(1, 1000),
                duration: -1,
                start_time: 0,
                nb_frames: 0,
                extra_data: Vec::new(),
                params: StreamParams::Video(VideoStreamParams {
                    width: 0,  // 将从 sequence header 或 metadata 获取
                    height: 0, // 同上
                    pixel_format: PixelFormat::Yuv420p,
                    frame_rate: Rational::new(0, 1),
                    sample_aspect_ratio: Rational::new(1, 1),
                    bit_rate: 0,
                }),
                metadata: Vec::new(),
            };
            self.streams.push(stream);
        }

        let remaining = data_size - 1; // 减去 video_header

        // AVC/HEVC: 解析 AVCPacketType + CompositionTimeOffset
        if matches!(
            codec_id_byte,
            FLV_CODEC_AVC | FLV_CODEC_HEVC | FLV_CODEC_AV1
        ) {
            if remaining < 4 {
                io.skip(remaining as usize)?;
                return Ok(None);
            }

            let avc_packet_type = io.read_u8()?;
            let cts_bytes = io.read_u24_be()?;
            // 有符号 24 位
            let cts = if cts_bytes & 0x800000 != 0 {
                cts_bytes as i32 - 0x1000000
            } else {
                cts_bytes as i32
            };

            let payload_size = remaining - 4;

            if avc_packet_type == 0 {
                // Sequence Header (AVCDecoderConfigurationRecord)
                let config = io.read_bytes(payload_size as usize)?;
                debug!("FLV: 收到视频 sequence header, {} 字节", config.len());
                if let Some(idx) = self.video_stream_idx {
                    self.streams[idx].extra_data = config;
                }
                self.video_config_received = true;
                return Ok(None);
            }

            if avc_packet_type == 2 {
                // End of Sequence
                io.skip(payload_size as usize)?;
                return Ok(None);
            }

            // NALU data
            let data = io.read_bytes(payload_size as usize)?;
            let stream_index = self.video_stream_idx.unwrap_or(0);
            let dts = i64::from(timestamp);
            let pts = dts + i64::from(cts);

            let mut pkt = Packet::from_data(Bytes::from(data));
            pkt.stream_index = stream_index;
            pkt.pts = pts;
            pkt.dts = dts;
            pkt.is_keyframe = is_keyframe;
            pkt.time_base = Rational::new(1, 1000);
            return Ok(Some(pkt));
        }

        // 其他视频编解码器
        let data = io.read_bytes(remaining as usize)?;
        let stream_index = self.video_stream_idx.unwrap_or(0);
        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = stream_index;
        pkt.pts = i64::from(timestamp);
        pkt.dts = i64::from(timestamp);
        pkt.is_keyframe = is_keyframe;
        pkt.time_base = Rational::new(1, 1000);
        Ok(Some(pkt))
    }

    /// 简单解析 onMetaData (AMF0), 只提取 duration
    fn parse_script_tag(&mut self, io: &mut IoContext, data_size: u32) -> TaoResult<()> {
        // AMF0: 简化解析, 只查找 "duration" 字段
        let data = io.read_bytes(data_size as usize)?;

        // 搜索 "duration" 字符串并在其后读取 AMF0 number
        let needle = b"duration";
        if let Some(pos) = data.windows(needle.len()).position(|w| w == needle) {
            let after = pos + needle.len();
            // AMF0 Number: type(0x00) + 8 bytes IEEE 754
            if after + 9 <= data.len() && data[after] == 0x00 {
                let dur_bytes = &data[after + 1..after + 9];
                let bits = u64::from_be_bytes(dur_bytes.try_into().unwrap_or([0; 8]));
                let dur = f64::from_bits(bits);
                if dur > 0.0 && dur.is_finite() {
                    self.duration_ms = Some(dur * 1000.0);
                    debug!("FLV: onMetaData duration={dur}s");
                }
            }
        }

        // 搜索 "width" 和 "height"
        for key in [b"width" as &[u8], b"height"] {
            if let Some(pos) = data.windows(key.len()).position(|w| w == key) {
                let after = pos + key.len();
                if after + 9 <= data.len() && data[after] == 0x00 {
                    let val_bytes = &data[after + 1..after + 9];
                    let bits = u64::from_be_bytes(val_bytes.try_into().unwrap_or([0; 8]));
                    let val = f64::from_bits(bits);
                    if val > 0.0 && val.is_finite() {
                        if let Some(idx) = self.video_stream_idx {
                            if let StreamParams::Video(ref mut vp) = self.streams[idx].params {
                                if key == b"width" {
                                    vp.width = val as u32;
                                } else {
                                    vp.height = val as u32;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Demuxer for FlvDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Flv
    }

    fn name(&self) -> &str {
        "flv"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        self.read_header(io)?;

        // 预读若干 Tag 以建立流信息
        let max_probe_tags = 32;
        let mut tags_read = 0;

        while tags_read < max_probe_tags {
            let _tag_pos = io.position()?;
            let (tag_type, data_size, timestamp) = match self.read_tag_header(io) {
                Ok(v) => v,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };

            match tag_type {
                TAG_AUDIO => {
                    self.handle_audio_tag(io, data_size, timestamp)?;
                }
                TAG_VIDEO => {
                    self.handle_video_tag(io, data_size, timestamp)?;
                }
                TAG_SCRIPT => {
                    self.parse_script_tag(io, data_size)?;
                }
                _ => {
                    io.skip(data_size as usize)?;
                }
            }

            // PreviousTagSize
            let _prev_tag_size = io.read_u32_be()?;

            tags_read += 1;

            // 如果已经获得了足够的流信息, 停止预读
            let has_audio = self.audio_stream_idx.is_some();
            let has_video = self.video_stream_idx.is_some();
            if tags_read > 5 || (has_audio && has_video) {
                break;
            }
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData("FLV: 未找到音频或视频流".into()));
        }

        // 更新时长
        if let Some(dur_ms) = self.duration_ms {
            for stream in &mut self.streams {
                stream.duration = dur_ms as i64; // time_base = 1/1000
            }
        }

        // 回到数据区开始, 准备顺序读取
        io.seek(std::io::SeekFrom::Start(self.data_offset))?;
        let _prev = io.read_u32_be()?; // PreviousTagSize0
        self.audio_config_received = false;
        self.video_config_received = false;

        debug!("FLV: 打开完成, {} 个流", self.streams.len(),);
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        loop {
            let (tag_type, data_size, timestamp) = match self.read_tag_header(io) {
                Ok(v) => v,
                Err(TaoError::Eof) => return Err(TaoError::Eof),
                Err(e) => return Err(e),
            };

            let pkt = match tag_type {
                TAG_AUDIO => self.handle_audio_tag(io, data_size, timestamp)?,
                TAG_VIDEO => self.handle_video_tag(io, data_size, timestamp)?,
                TAG_SCRIPT => {
                    self.parse_script_tag(io, data_size)?;
                    None
                }
                _ => {
                    io.skip(data_size as usize)?;
                    None
                }
            };

            // PreviousTagSize
            let _prev_tag_size = io.read_u32_be()?;

            if let Some(p) = pkt {
                return Ok(p);
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
        Err(TaoError::NotImplemented("FLV seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        self.duration_ms.map(|ms| ms / 1000.0)
    }
}

/// FLV 格式探测器
pub struct FlvProbe;

impl FormatProbe for FlvProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // 检查 "FLV" 签名 + version + flags
        if data.len() >= 9 && data[0] == b'F' && data[1] == b'L' && data[2] == b'V' && data[3] == 1
        {
            return Some(crate::probe::SCORE_MAX);
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                if ext.eq_ignore_ascii_case("flv") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Flv
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构造 FLV 文件头部
    fn build_flv_header(has_audio: bool, has_video: bool) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"FLV");
        data.push(1); // version
        let flags = if has_audio { 0x04 } else { 0 } | if has_video { 0x01 } else { 0 };
        data.push(flags);
        data.extend_from_slice(&9u32.to_be_bytes()); // data offset
        data.extend_from_slice(&0u32.to_be_bytes()); // PreviousTagSize0
        data
    }

    /// 构造 FLV 音频 Tag (AAC raw)
    fn build_audio_tag(timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut tag = Vec::new();
        // audio header: AAC(10)=0xA0, rate=3(44kHz), size=1(16bit), type=1(stereo) → 0xAF
        let audio_header: u8 = 0xAF;
        let aac_packet_type: u8 = 1; // raw
        let data_size = 1 + 1 + payload.len() as u32; // audio_header + aac_type + payload

        tag.push(TAG_AUDIO);
        tag.push((data_size >> 16) as u8);
        tag.push((data_size >> 8) as u8);
        tag.push(data_size as u8);
        // timestamp (24 bits LE of BE actually)
        tag.push((timestamp >> 16) as u8);
        tag.push((timestamp >> 8) as u8);
        tag.push(timestamp as u8);
        tag.push((timestamp >> 24) as u8); // timestamp extended
        tag.extend_from_slice(&[0, 0, 0]); // stream ID

        tag.push(audio_header);
        tag.push(aac_packet_type);
        tag.extend_from_slice(payload);

        // PreviousTagSize
        let total_tag_size = 11 + data_size;
        tag.extend_from_slice(&total_tag_size.to_be_bytes());

        tag
    }

    /// 构造 FLV 视频 Tag (AVC NALU)
    fn build_video_tag(timestamp: u32, is_keyframe: bool, payload: &[u8]) -> Vec<u8> {
        let mut tag = Vec::new();
        let frame_type: u8 = if is_keyframe { 1 } else { 2 };
        let video_header = (frame_type << 4) | FLV_CODEC_AVC;
        let avc_packet_type: u8 = 1; // NALU
        let cts = 0u32;
        let data_size = 1 + 1 + 3 + payload.len() as u32;

        tag.push(TAG_VIDEO);
        tag.push((data_size >> 16) as u8);
        tag.push((data_size >> 8) as u8);
        tag.push(data_size as u8);
        tag.push((timestamp >> 16) as u8);
        tag.push((timestamp >> 8) as u8);
        tag.push(timestamp as u8);
        tag.push((timestamp >> 24) as u8);
        tag.extend_from_slice(&[0, 0, 0]);

        tag.push(video_header);
        tag.push(avc_packet_type);
        tag.push((cts >> 16) as u8);
        tag.push((cts >> 8) as u8);
        tag.push(cts as u8);
        tag.extend_from_slice(payload);

        let total_tag_size = 11 + data_size;
        tag.extend_from_slice(&total_tag_size.to_be_bytes());

        tag
    }

    /// 构造最小的 FLV 文件
    fn build_minimal_flv() -> Vec<u8> {
        let mut data = build_flv_header(true, true);
        // 视频关键帧
        data.extend_from_slice(&build_video_tag(0, true, &[0xDE, 0xAD]));
        // 音频帧
        data.extend_from_slice(&build_audio_tag(0, &[0xBE, 0xEF]));
        // 视频非关键帧
        data.extend_from_slice(&build_video_tag(33, false, &[0xCA, 0xFE]));
        // 音频帧
        data.extend_from_slice(&build_audio_tag(23, &[0xF0, 0x0D]));
        data
    }

    #[test]
    fn test_probe_flv_signature() {
        let probe = FlvProbe;
        let data = build_flv_header(true, true);
        assert_eq!(probe.probe(&data, None), Some(crate::probe::SCORE_MAX));
    }

    #[test]
    fn test_probe_flv_extension() {
        let probe = FlvProbe;
        assert!(probe.probe(&[], Some("video.flv")).is_some());
        assert!(probe.probe(&[], Some("video.mp4")).is_none());
    }

    #[test]
    fn test_parse_minimal_flv() {
        let flv = build_minimal_flv();
        let backend = MemoryBackend::from_data(flv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = FlvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert!(
            streams.len() >= 2,
            "应该至少有 2 个流, 实际={}",
            streams.len()
        );

        // 检查有视频和音频流
        let has_video = streams.iter().any(|s| s.media_type == MediaType::Video);
        let has_audio = streams.iter().any(|s| s.media_type == MediaType::Audio);
        assert!(has_video, "应该有视频流");
        assert!(has_audio, "应该有音频流");
    }

    #[test]
    fn test_read_flv_packets() {
        let flv = build_minimal_flv();
        let backend = MemoryBackend::from_data(flv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = FlvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let mut packets = Vec::new();
        loop {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => packets.push(pkt),
                Err(TaoError::Eof) => break,
                Err(e) => panic!("读取数据包失败: {e}"),
            }
        }

        assert!(
            packets.len() >= 4,
            "应该至少有 4 个数据包, 实际={}",
            packets.len()
        );
    }

    #[test]
    fn test_video_keyframe_flag() {
        let flv = build_minimal_flv();
        let backend = MemoryBackend::from_data(flv);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = FlvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let mut video_keyframes = 0;
        let mut video_non_keyframes = 0;

        loop {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if let Some(idx) = demuxer
                        .streams()
                        .iter()
                        .position(|s| s.media_type == MediaType::Video)
                    {
                        if pkt.stream_index == idx {
                            if pkt.is_keyframe {
                                video_keyframes += 1;
                            } else {
                                video_non_keyframes += 1;
                            }
                        }
                    }
                }
                Err(TaoError::Eof) => break,
                Err(e) => panic!("读取数据包失败: {e}"),
            }
        }

        assert!(video_keyframes >= 1, "应该至少有 1 个关键帧");
        assert!(video_non_keyframes >= 1, "应该至少有 1 个非关键帧");
    }

    #[test]
    fn test_audio_only_flv() {
        let mut data = build_flv_header(true, false);
        data.extend_from_slice(&build_audio_tag(0, &[0xAA; 50]));
        data.extend_from_slice(&build_audio_tag(23, &[0xBB; 50]));

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = FlvDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        assert_eq!(demuxer.streams().len(), 1);
        assert_eq!(demuxer.streams()[0].media_type, MediaType::Audio);
        assert_eq!(demuxer.streams()[0].codec_id, CodecId::Aac);
    }
}
