//! AVI (Audio Video Interleave) 解封装器.
//!
//! AVI 基于 RIFF 格式, 结构如下:
//! ```text
//! RIFF 'AVI '
//!   LIST 'hdrl'
//!     avih (主 AVI 头: 56 字节)
//!     LIST 'strl' (每流一个)
//!       strh (流头)
//!       strf (流格式: BITMAPINFOHEADER 或 WAVEFORMATEX)
//!   LIST 'movi' (数据块)
//!     00dc (视频数据)
//!     01wb (音频数据)
//!   idx1 (可选旧式索引)
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
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

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
#[derive(Debug, Clone)]
struct Idx1Entry {
    /// 块 ID (如 0x00636430 = "00dc")
    chunk_id: [u8; 4],
    /// 标志
    flags: u32,
    /// 相对于 movi 列表起始的偏移
    offset: u32,
    /// 数据大小
    size: u32,
}

/// AVI 解封装器
pub struct AviDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// movi 列表数据起始位置 (跳过 LIST + size + 'movi')
    movi_data_start: u64,
    /// movi 列表的字节大小 (不含 LIST 头)
    movi_list_size: u64,
    /// idx1 索引条目 (按时间顺序)
    idx1_entries: Vec<Idx1Entry>,
    /// 当前读取的索引位置
    idx_pos: usize,
    /// 每流已读取的帧计数 (用于 PTS)
    frame_counts: Vec<i64>,
    /// 元数据
    metadata: Vec<(String, String)>,
}

impl AviDemuxer {
    /// 创建 AVI 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            movi_data_start: 0,
            movi_list_size: 0,
            idx1_entries: Vec::new(),
            idx_pos: 0,
            frame_counts: Vec::new(),
            metadata: Vec::new(),
        }))
    }

    /// 根据 FourCC 和 WAV 格式解析视频 CodecId
    fn resolve_video_codec(fcc_handler: &[u8; 4], bi_compression: u32) -> TaoResult<CodecId> {
        let s = String::from_utf8_lossy(fcc_handler);
        let s = s.trim_end_matches('\0').trim();
        match s.to_uppercase().as_str() {
            "H264" | "X264" | "AVC1" => Ok(CodecId::H264),
            "H265" | "HEVC" | "HVC1" => Ok(CodecId::H265),
            "VP80" => Ok(CodecId::Vp8),
            "VP90" => Ok(CodecId::Vp9),
            "AV01" => Ok(CodecId::Av1),
            "MP4V" | "XVID" | "DIVX" => Ok(CodecId::Mpeg4),
            "MJPG" | "JPEG" => Ok(CodecId::Mjpeg),
            "THEO" => Ok(CodecId::Theora),
            _ => {
                // 尝试 biCompression (如 0x48323634 = "H264")
                if bi_compression == 0 {
                    Ok(CodecId::RawVideo)
                } else {
                    let b0 = bi_compression & 0xFF;
                    let b1 = (bi_compression >> 8) & 0xFF;
                    let b2 = (bi_compression >> 16) & 0xFF;
                    let b3 = (bi_compression >> 24) & 0xFF;
                    let fourcc = format!(
                        "{}{}{}{}",
                        b0 as u8 as char, b1 as u8 as char, b2 as u8 as char, b3 as u8 as char
                    );
                    match fourcc.to_uppercase().as_str() {
                        "H264" | "AVC1" => Ok(CodecId::H264),
                        "HEVC" | "HVC1" => Ok(CodecId::H265),
                        "VP80" => Ok(CodecId::Vp8),
                        "VP90" => Ok(CodecId::Vp9),
                        "MJPG" => Ok(CodecId::Mjpeg),
                        "MP4V" | "XVID" | "DIVX" | "DX50" => Ok(CodecId::Mpeg4),
                        _ => Ok(CodecId::None),
                    }
                }
            }
        }
    }

    /// 根据 WAV 格式码和位深解析音频 CodecId
    fn resolve_audio_codec(format_tag: u16, bits_per_sample: u16) -> TaoResult<CodecId> {
        match format_tag {
            WAV_FORMAT_PCM => match bits_per_sample {
                8 => Ok(CodecId::PcmU8),
                16 => Ok(CodecId::PcmS16le),
                24 => Ok(CodecId::PcmS24le),
                32 => Ok(CodecId::PcmS32le),
                _ => Err(TaoError::Unsupported(format!(
                    "不支持的 PCM 位深: {}",
                    bits_per_sample
                ))),
            },
            WAV_FORMAT_IEEE_FLOAT => match bits_per_sample {
                32 => Ok(CodecId::PcmF32le),
                _ => Err(TaoError::Unsupported(format!(
                    "不支持的浮点位深: {}",
                    bits_per_sample
                ))),
            },
            0x0050 => Ok(CodecId::Mp3), // MPEG Layer 3
            0x0055 => Ok(CodecId::Mp3), // MPEG Layer 3
            0x0160 => Ok(CodecId::Aac), // WAVE_FORMAT_AAC
            _ => Err(TaoError::Unsupported(format!(
                "不支持的音频格式码: 0x{:04X}",
                format_tag
            ))),
        }
    }

    /// 根据 CodecId 确定采样格式
    fn resolve_sample_format(codec_id: CodecId) -> SampleFormat {
        match codec_id {
            CodecId::PcmU8 => SampleFormat::U8,
            CodecId::PcmS16le => SampleFormat::S16,
            CodecId::PcmS24le | CodecId::PcmS32le => SampleFormat::S32,
            CodecId::PcmF32le => SampleFormat::F32,
            _ => SampleFormat::None,
        }
    }

    /// 根据 biCompression 确定像素格式
    fn resolve_pixel_format(bi_compression: u32, bi_bit_count: u16) -> PixelFormat {
        if bi_compression == 0 {
            match bi_bit_count {
                24 => PixelFormat::Bgr24,
                32 => PixelFormat::Bgra,
                8 => PixelFormat::Gray8,
                _ => PixelFormat::None,
            }
        } else {
            PixelFormat::Yuv420p
        }
    }

    /// 解析 RIFF 块 (返回块 ID 或列表类型, 大小, 是否为 LIST)
    fn read_riff_chunk_header(io: &mut IoContext) -> TaoResult<([u8; 4], u32, bool)> {
        let chunk_id = io.read_tag()?;
        let chunk_size = io.read_u32_le()?;
        let is_list = &chunk_id == b"LIST";
        if is_list {
            let list_type = io.read_tag()?;
            Ok((list_type, chunk_size - 4, true))
        } else {
            Ok((chunk_id, chunk_size, false))
        }
    }

    /// 解析 hdrl 列表
    fn parse_hdrl(&mut self, io: &mut IoContext, list_size: u32) -> TaoResult<()> {
        let start = io.position()?;
        let end = start + list_size as u64;
        let mut stream_index = 0;

        debug!("开始解析 hdrl, list_size={}", list_size);

        while io.position()? < end {
            let (chunk_id, chunk_size, is_list) = Self::read_riff_chunk_header(io)?;
            debug!(
                "hdrl 中的块: {:?}, size={}, is_list={}",
                String::from_utf8_lossy(&chunk_id),
                chunk_size,
                is_list
            );

            match (&chunk_id, is_list) {
                (b"avih", false) => {
                    if chunk_size < 56 {
                        return Err(TaoError::InvalidData("avih 块不足 56 字节".into()));
                    }
                    let _micro_sec_per_frame = io.read_u32_le()?;
                    let _max_bytes_per_sec = io.read_u32_le()?;
                    let _padding = io.read_u32_le()?;
                    let _flags = io.read_u32_le()?;
                    let _total_frames = io.read_u32_le()?;
                    let _initial_frames = io.read_u32_le()?;
                    let _streams = io.read_u32_le()?;
                    let _suggested_buffer_size = io.read_u32_le()?;
                    let _width = io.read_u32_le()?;
                    let _height = io.read_u32_le()?;
                    let _reserved = io.read_u32_le()?; // reserved[0]
                    let _reserved = io.read_u32_le()?; // reserved[1]
                    let _reserved = io.read_u32_le()?; // reserved[2]
                    let _reserved = io.read_u32_le()?; // reserved[3]
                    // 总共 14 * 4 = 56 字节
                    if chunk_size > 56 {
                        io.skip((chunk_size - 56) as usize)?;
                    }
                    debug!("avih 解析完成");
                }
                (b"strl", true) => {
                    debug!("进入 strl 块处理, chunk_size={}", chunk_size);
                    let strl_end = io.position()? + chunk_size as u64;
                    let mut fcc_type = [0u8; 4];
                    let mut fcc_handler = [0u8; 4];
                    let mut scale: u32 = 1;
                    let mut rate: u32 = 1;
                    let mut stream_format = Vec::new();

                    while io.position()? < strl_end {
                        let (sub_id, sub_size, sub_is_list) = Self::read_riff_chunk_header(io)?;
                        debug!(
                            "strl 中的子块: {:?}, size={}, is_list={}",
                            String::from_utf8_lossy(&sub_id),
                            sub_size,
                            sub_is_list
                        );
                        if sub_is_list {
                            io.skip(sub_size as usize)?;
                            continue;
                        }

                        match &sub_id {
                            b"strh" => {
                                if sub_size < 48 {
                                    return Err(TaoError::InvalidData(
                                        "strh 块不足 48 字节".into(),
                                    ));
                                }
                                fcc_type = io.read_tag()?; // 4 bytes
                                fcc_handler = io.read_tag()?; // 4 bytes
                                let _flags = io.read_u32_le()?; // 4 bytes
                                let _priority = io.read_u16_le()?; // 2 bytes
                                let _language = io.read_u16_le()?; // 2 bytes
                                let _initial_frames = io.read_u32_le()?; // 4 bytes
                                scale = io.read_u32_le()?; // 4 bytes
                                rate = io.read_u32_le()?; // 4 bytes
                                let _start = io.read_u32_le()?; // 4 bytes
                                let length = io.read_u32_le()?; // 4 bytes
                                // 已读取 36 字节
                                // 跳过剩余字段 (dwSuggestedBufferSize, dwQuality, dwSampleSize, rcFrame)
                                io.skip((sub_size - 36) as usize)?;

                                debug!(
                                    "strh: type={:?}, handler={:?}, scale={}, rate={}",
                                    String::from_utf8_lossy(&fcc_type),
                                    String::from_utf8_lossy(&fcc_handler),
                                    scale,
                                    rate
                                );

                                if &fcc_type == FCC_VIDS {
                                    let codec_id = Self::resolve_video_codec(&fcc_handler, 0)?;
                                    debug!("视频流 codec_id: {:?}", codec_id);
                                    if codec_id == CodecId::None {
                                        debug!("跳过不支持的视频编解码器");
                                        continue;
                                    }
                                    let time_base = Rational::new(scale as i32, rate as i32);
                                    let stream = Stream {
                                        index: stream_index,
                                        media_type: MediaType::Video,
                                        codec_id,
                                        time_base,
                                        duration: length as i64,
                                        start_time: 0,
                                        nb_frames: length as u64,
                                        extra_data: Vec::new(),
                                        params: StreamParams::Video(VideoStreamParams {
                                            width: 0,
                                            height: 0,
                                            pixel_format: PixelFormat::Yuv420p,
                                            frame_rate: Rational::new(rate as i32, scale as i32),
                                            sample_aspect_ratio: Rational::new(1, 1),
                                            bit_rate: 0,
                                        }),
                                        metadata: Vec::new(),
                                    };
                                    self.streams.push(stream);
                                    stream_index += 1;
                                } else if &fcc_type == FCC_AUDS {
                                    stream_format.clear();
                                }
                            }
                            b"strf" => {
                                stream_format = io.read_bytes(sub_size as usize)?;
                                if &fcc_type == FCC_AUDS && stream_format.len() >= 18 {
                                    let format_tag =
                                        u16::from_le_bytes([stream_format[0], stream_format[1]]);
                                    let channels =
                                        u16::from_le_bytes([stream_format[2], stream_format[3]]);
                                    let sample_rate = u32::from_le_bytes([
                                        stream_format[4],
                                        stream_format[5],
                                        stream_format[6],
                                        stream_format[7],
                                    ]);
                                    let _avg_bytes = u32::from_le_bytes([
                                        stream_format[8],
                                        stream_format[9],
                                        stream_format[10],
                                        stream_format[11],
                                    ]);
                                    let block_align =
                                        u16::from_le_bytes([stream_format[12], stream_format[13]]);
                                    let bits_per_sample =
                                        u16::from_le_bytes([stream_format[14], stream_format[15]]);

                                    let codec_id =
                                        Self::resolve_audio_codec(format_tag, bits_per_sample)?;
                                    let sample_format = Self::resolve_sample_format(codec_id);
                                    let channel_layout =
                                        ChannelLayout::from_channels(channels as u32);
                                    let time_base = Rational::new(1, sample_rate as i32);

                                    let stream = Stream {
                                        index: stream_index,
                                        media_type: MediaType::Audio,
                                        codec_id,
                                        time_base,
                                        duration: -1,
                                        start_time: 0,
                                        nb_frames: 0,
                                        extra_data: stream_format.clone(),
                                        params: StreamParams::Audio(AudioStreamParams {
                                            sample_rate,
                                            channel_layout,
                                            sample_format,
                                            bit_rate: 0,
                                            frame_size: block_align as u32,
                                        }),
                                        metadata: Vec::new(),
                                    };
                                    self.streams.push(stream);
                                    stream_index += 1;
                                } else if &fcc_type == FCC_VIDS && stream_format.len() >= 40 {
                                    let _bi_size = u32::from_le_bytes([
                                        stream_format[0],
                                        stream_format[1],
                                        stream_format[2],
                                        stream_format[3],
                                    ]);
                                    let width = u32::from_le_bytes([
                                        stream_format[4],
                                        stream_format[5],
                                        stream_format[6],
                                        stream_format[7],
                                    ]);
                                    let height = u32::from_le_bytes([
                                        stream_format[8],
                                        stream_format[9],
                                        stream_format[10],
                                        stream_format[11],
                                    ]);
                                    let _planes =
                                        u16::from_le_bytes([stream_format[12], stream_format[13]]);
                                    let bi_bit_count =
                                        u16::from_le_bytes([stream_format[14], stream_format[15]]);
                                    let bi_compression = u32::from_le_bytes([
                                        stream_format[16],
                                        stream_format[17],
                                        stream_format[18],
                                        stream_format[19],
                                    ]);

                                    if let Some(s) = self.streams.iter_mut().find(|s| {
                                        s.index == stream_index - 1
                                            && s.media_type == MediaType::Video
                                    }) {
                                        s.codec_id = Self::resolve_video_codec(
                                            &fcc_handler,
                                            bi_compression,
                                        )?;
                                        s.extra_data = stream_format.clone();
                                        if let StreamParams::Video(ref mut v) = s.params {
                                            v.width = width;
                                            v.height = height;
                                            v.pixel_format = Self::resolve_pixel_format(
                                                bi_compression,
                                                bi_bit_count,
                                            );
                                        }
                                    } else {
                                        let codec_id = Self::resolve_video_codec(
                                            &fcc_handler,
                                            bi_compression,
                                        )?;
                                        let pixel_format = Self::resolve_pixel_format(
                                            bi_compression,
                                            bi_bit_count,
                                        );
                                        let stream = Stream {
                                            index: stream_index,
                                            media_type: MediaType::Video,
                                            codec_id,
                                            time_base: Rational::new(scale as i32, rate as i32),
                                            duration: -1,
                                            start_time: 0,
                                            nb_frames: 0,
                                            extra_data: stream_format.clone(),
                                            params: StreamParams::Video(VideoStreamParams {
                                                width,
                                                height,
                                                pixel_format,
                                                frame_rate: Rational::new(
                                                    rate as i32,
                                                    scale as i32,
                                                ),
                                                sample_aspect_ratio: Rational::new(1, 1),
                                                bit_rate: 0,
                                            }),
                                            metadata: Vec::new(),
                                        };
                                        self.streams.push(stream);
                                        stream_index += 1;
                                    }
                                }
                            }
                            _ => {
                                io.skip(sub_size as usize)?;
                            }
                        }
                        if sub_size % 2 != 0 {
                            io.skip(1)?;
                        }
                    }
                }
                _ => {
                    io.skip(chunk_size as usize)?;
                }
            }
            if chunk_size % 2 != 0 && !is_list {
                io.skip(1)?;
            }
        }

        self.frame_counts = vec![0; self.streams.len()];
        Ok(())
    }

    /// 解析 idx1 索引
    fn parse_idx1(
        &mut self,
        io: &mut IoContext,
        _movi_start: u64,
        chunk_size: u32,
    ) -> TaoResult<()> {
        let num_entries = chunk_size as usize / 16;

        for _ in 0..num_entries {
            let chunk_id = io.read_tag()?;
            let flags = io.read_u32_le()?;
            let offset = io.read_u32_le()?;
            let size = io.read_u32_le()?;

            self.idx1_entries.push(Idx1Entry {
                chunk_id,
                flags,
                offset,
                size,
            });
        }

        debug!("idx1: {} 个索引条目", self.idx1_entries.len());
        Ok(())
    }

    /// 无 idx1 索引时的回退 seek: 从 movi 起始扫描块头定位到目标帧
    fn seek_no_idx1(
        &mut self,
        io: &mut IoContext,
        stream_index: usize,
        timestamp: i64,
    ) -> TaoResult<()> {
        let target_frame = timestamp.max(0);

        // 回到 movi 数据起始位置
        io.seek(std::io::SeekFrom::Start(self.movi_data_start))?;
        self.frame_counts = vec![0; self.streams.len()];

        let movi_end = self.movi_data_start + self.movi_list_size;

        // 扫描块头, 跳过数据, 直到找到目标流的目标帧
        while io.position()? < movi_end {
            let chunk_start = io.position()?;
            let chunk_id = match io.read_tag() {
                Ok(tag) => tag,
                Err(_) => break,
            };
            let chunk_size = match io.read_u32_le() {
                Ok(s) => s,
                Err(_) => break,
            };

            // 解析流号: 前两个字符必须是 ASCII 数字
            if chunk_id.len() >= 4 && chunk_id[0].is_ascii_digit() && chunk_id[1].is_ascii_digit() {
                let snum = ((chunk_id[0] - b'0') * 10 + (chunk_id[1] - b'0')) as usize;
                if snum < self.streams.len() {
                    if snum == stream_index && self.frame_counts[snum] >= target_frame {
                        // 找到目标帧, 回退到块头
                        io.seek(std::io::SeekFrom::Start(chunk_start))?;
                        debug!(
                            "无索引 seek: 流 {} 帧 {} (扫描到 {})",
                            stream_index, target_frame, chunk_start
                        );
                        return Ok(());
                    }
                    self.frame_counts[snum] += 1;
                }
            }

            // 跳过块数据
            let skip = chunk_size as i64 + i64::from(chunk_size % 2);
            io.seek(std::io::SeekFrom::Current(skip))?;
        }

        // 已到文件末尾, 回到起始位置
        io.seek(std::io::SeekFrom::Start(self.movi_data_start))?;
        self.frame_counts = vec![0; self.streams.len()];
        Err(TaoError::Eof)
    }
}

impl Demuxer for AviDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Avi
    }

    fn name(&self) -> &str {
        "avi"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let riff_tag = io.read_tag()?;
        if &riff_tag != b"RIFF" {
            return Err(TaoError::InvalidData("不是有效的 RIFF 文件".into()));
        }

        let _file_size = io.read_u32_le()?;

        let avi_tag = io.read_tag()?;
        if &avi_tag != b"AVI " {
            return Err(TaoError::InvalidData("不是有效的 AVI 文件".into()));
        }

        debug!("检测到 RIFF/AVI 文件");

        let mut movi_start: u64 = 0;
        let mut movi_data_start: u64 = 0;
        let mut movi_list_size: u64 = 0;

        loop {
            let (chunk_id, chunk_size, is_list) = match Self::read_riff_chunk_header(io) {
                Ok(v) => v,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };

            match (&chunk_id, is_list) {
                (b"hdrl", true) => {
                    self.parse_hdrl(io, chunk_size)?;
                }
                (b"movi", true) => {
                    movi_start = io.position()? - 8;
                    movi_data_start = io.position()?;
                    movi_list_size = chunk_size as u64;
                    io.seek(std::io::SeekFrom::Current(chunk_size as i64))?;
                }
                (b"idx1", false) => {
                    if movi_start > 0 {
                        self.parse_idx1(io, movi_data_start, chunk_size)?;
                    }
                    break;
                }
                _ => {
                    io.skip(chunk_size as usize)?;
                }
            }

            if chunk_size % 2 != 0 && !is_list {
                io.skip(1)?;
            }

            if &chunk_id == b"idx1" {
                break;
            }
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData("AVI 文件中未找到有效流".into()));
        }

        self.movi_data_start = movi_data_start;
        self.movi_list_size = movi_list_size;

        if self.idx1_entries.is_empty() {
            io.seek(std::io::SeekFrom::Start(movi_data_start))?;
        }

        debug!(
            "AVI 打开完成: {} 个流, movi 起始={}",
            self.streams.len(),
            movi_data_start
        );

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        if !self.idx1_entries.is_empty() {
            if self.idx_pos >= self.idx1_entries.len() {
                return Err(TaoError::Eof);
            }

            let entry = &self.idx1_entries[self.idx_pos];
            self.idx_pos += 1;

            let chunk_offset = self.movi_data_start + u64::from(entry.offset) + 8;
            io.seek(std::io::SeekFrom::Start(chunk_offset))?;

            let data = io.read_bytes(entry.size as usize)?;

            let stream_num =
                ((entry.chunk_id[0] - b'0') * 10 + (entry.chunk_id[1] - b'0')) as usize;
            let stream_index = stream_num.min(self.streams.len().saturating_sub(1));

            let stream = &self.streams[stream_index];
            let pts = self.frame_counts[stream_index];
            self.frame_counts[stream_index] += 1;

            let is_keyframe =
                (entry.flags & AVIIF_KEYFRAME) != 0 || stream.media_type == MediaType::Audio;

            let mut pkt = Packet::from_data(Bytes::from(data));
            pkt.stream_index = stream_index;
            pkt.pts = pts;
            pkt.dts = pts;
            pkt.duration = 1;
            pkt.time_base = stream.time_base;
            pkt.is_keyframe = is_keyframe;
            pkt.pos = chunk_offset as i64;

            return Ok(pkt);
        }

        loop {
            let pos = io.position()?;
            if pos >= self.movi_data_start + self.movi_list_size {
                return Err(TaoError::Eof);
            }

            let chunk_id = io.read_tag()?;
            let chunk_size = io.read_u32_le()?;

            let stream_num = if chunk_id.len() >= 2
                && chunk_id[0].is_ascii_digit()
                && chunk_id[1].is_ascii_digit()
            {
                ((chunk_id[0] - b'0') * 10 + (chunk_id[1] - b'0')) as usize
            } else {
                io.skip(chunk_size as usize)?;
                if chunk_size % 2 != 0 {
                    io.skip(1)?;
                }
                continue;
            };

            let code = if chunk_id.len() >= 4 {
                &chunk_id[2..4]
            } else {
                io.skip(chunk_size as usize)?;
                if chunk_size % 2 != 0 {
                    io.skip(1)?;
                }
                continue;
            };

            let stream_index = stream_num.min(self.streams.len().saturating_sub(1));
            let is_video = code == b"dc" || code == b"db";
            let is_audio = code == b"wb";

            if !is_video && !is_audio {
                io.skip(chunk_size as usize)?;
                if chunk_size % 2 != 0 {
                    io.skip(1)?;
                }
                continue;
            }

            let data = io.read_bytes(chunk_size as usize)?;
            if chunk_size % 2 != 0 {
                io.skip(1)?;
            }

            let stream = &self.streams[stream_index];
            let pts = self.frame_counts[stream_index];
            self.frame_counts[stream_index] += 1;

            let is_keyframe = is_audio || code == b"db" || code == b"dc";

            let mut pkt = Packet::from_data(Bytes::from(data));
            pkt.stream_index = stream_index;
            pkt.pts = pts;
            pkt.dts = pts;
            pkt.duration = 1;
            pkt.time_base = stream.time_base;
            pkt.is_keyframe = is_keyframe;
            pkt.pos = pos as i64;

            return Ok(pkt);
        }
    }

    fn seek(
        &mut self,
        io: &mut IoContext,
        stream_index: usize,
        timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        if !io.is_seekable() {
            return Err(TaoError::Unsupported("不支持在非可寻址流上 seek".into()));
        }

        if self.idx1_entries.is_empty() {
            return self.seek_no_idx1(io, stream_index, timestamp);
        }

        let target = timestamp.max(0) as usize;
        let mut idx_pos = 0;
        let mut count = 0;

        for (i, entry) in self.idx1_entries.iter().enumerate() {
            let snum = ((entry.chunk_id[0].saturating_sub(b'0')) * 10
                + (entry.chunk_id[1].saturating_sub(b'0'))) as usize;
            if snum == stream_index {
                if count == target {
                    idx_pos = i;
                    break;
                }
                count += 1;
            }
        }

        self.idx_pos = idx_pos;
        self.frame_counts = vec![0; self.streams.len()];
        for (i, entry) in self.idx1_entries.iter().enumerate() {
            if i >= idx_pos {
                break;
            }
            let snum = ((entry.chunk_id[0].saturating_sub(b'0')) * 10
                + (entry.chunk_id[1].saturating_sub(b'0'))) as usize;
            if snum < self.frame_counts.len() {
                self.frame_counts[snum] += 1;
            }
        }

        if idx_pos < self.idx1_entries.len() {
            let entry = &self.idx1_entries[idx_pos];
            let chunk_offset = self.movi_data_start + u64::from(entry.offset) + 8;
            io.seek(std::io::SeekFrom::Start(chunk_offset))?;
        }

        Ok(())
    }

    fn duration(&self) -> Option<f64> {
        if self.streams.is_empty() {
            return None;
        }
        let video_stream = self
            .streams
            .iter()
            .find(|s| s.media_type == MediaType::Video);
        if let Some(s) = video_stream {
            if s.time_base.is_valid() && s.duration > 0 {
                return Some(s.duration as f64 * s.time_base.to_f64());
            }
        }
        None
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

/// AVI 格式探测器
pub struct AviProbe;

impl FormatProbe for AviProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"AVI " {
            return Some(SCORE_MAX);
        }

        if let Some(name) = filename {
            let lower = name.to_lowercase();
            if lower.ends_with(".avi") {
                return Some(SCORE_EXTENSION);
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Avi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_avi() -> Vec<u8> {
        let mut buf = Vec::new();

        // strh 标准结构 = 56 字节, strf 数据 = 40 字节
        // strh chunk = 8 + 56 = 64, strf chunk = 8 + 40 = 48
        // strl_size = 4("strl") + 64 + 48 = 116
        // strl_list_total = 8 + 116 = 124
        // avih chunk = 8 + 56 = 64
        // hdrl_size = 4("hdrl") + 64 + 124 = 192
        // hdrl_list_total = 8 + 192 = 200
        // movi_list_total = 8 + (4 + 108) = 120
        // file_size = 4("AVI ") + 200 + 120 = 324

        buf.extend_from_slice(b"RIFF");
        let file_size: u32 = 4 + 200 + 120;
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"AVI ");

        let hdrl_size: u32 = 4 + 64 + 124;
        buf.extend_from_slice(b"LIST");
        buf.extend_from_slice(&hdrl_size.to_le_bytes());
        buf.extend_from_slice(b"hdrl");

        // avih 块 (56 字节数据)
        buf.extend_from_slice(b"avih");
        buf.extend_from_slice(&56u32.to_le_bytes());
        buf.extend_from_slice(&33367u32.to_le_bytes()); // dwMicroSecPerFrame
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwMaxBytesPerSec
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwPaddingGranularity
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwFlags
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwTotalFrames
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwInitialFrames
        buf.extend_from_slice(&1u32.to_le_bytes()); // dwStreams
        buf.extend_from_slice(&[0u8; 28]); // dwSuggestedBufferSize + dwWidth + dwHeight + reserved[4]

        // strl LIST
        let strl_size: u32 = 4 + 64 + 48;
        buf.extend_from_slice(b"LIST");
        buf.extend_from_slice(&strl_size.to_le_bytes());
        buf.extend_from_slice(b"strl");

        // strh 块 (56 字节数据 = 标准大小)
        buf.extend_from_slice(b"strh");
        buf.extend_from_slice(&56u32.to_le_bytes());
        buf.extend_from_slice(b"vids"); // fccType
        buf.extend_from_slice(b"H264"); // fccHandler
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwFlags
        buf.extend_from_slice(&0u16.to_le_bytes()); // wPriority
        buf.extend_from_slice(&0u16.to_le_bytes()); // wLanguage
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwInitialFrames
        buf.extend_from_slice(&1u32.to_le_bytes()); // dwScale
        buf.extend_from_slice(&25u32.to_le_bytes()); // dwRate
        buf.extend_from_slice(&0u32.to_le_bytes()); // dwStart
        buf.extend_from_slice(&100u32.to_le_bytes()); // dwLength
        buf.extend_from_slice(&[0u8; 20]); // dwSuggestedBufferSize + dwQuality + dwSampleSize + rcFrame

        // strf 块 (40 字节数据 = BITMAPINFOHEADER)
        buf.extend_from_slice(b"strf");
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
        buf.extend_from_slice(&320u32.to_le_bytes()); // biWidth
        buf.extend_from_slice(&240u32.to_le_bytes()); // biHeight
        buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        buf.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
        buf.extend_from_slice(&0x34363248u32.to_le_bytes()); // biCompression = "H264"
        buf.extend_from_slice(&[0u8; 20]); // biSizeImage + rest

        // movi LIST
        let movi_size: u32 = 4 + 108;
        buf.extend_from_slice(b"LIST");
        buf.extend_from_slice(&movi_size.to_le_bytes());
        buf.extend_from_slice(b"movi");

        buf.extend_from_slice(b"00dc");
        buf.extend_from_slice(&100u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 100]);

        buf
    }

    #[test]
    fn test_探测_avi_魔数() {
        let avi = make_minimal_avi();
        let probe = AviProbe;
        assert_eq!(probe.probe(&avi, None), Some(SCORE_MAX));
    }

    #[test]
    fn test_探测_avi_扩展名() {
        let probe = AviProbe;
        assert_eq!(probe.probe(&[], Some("test.avi")), Some(SCORE_EXTENSION));
        assert_eq!(probe.probe(&[], Some("test.mp4")), None);
    }

    #[test]
    fn test_解析_最小avi() {
        let avi = make_minimal_avi();
        let backend = crate::io::MemoryBackend::from_data(avi);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = AviDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].media_type, MediaType::Video);
        assert_eq!(streams[0].codec_id, CodecId::H264);

        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.stream_index, 0);
        assert_eq!(pkt.data.len(), 100);
        assert!(pkt.is_keyframe);

        let err = demuxer.read_packet(&mut io).unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }
}
