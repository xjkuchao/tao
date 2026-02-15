//! FLAC 原生容器解封装器.
//!
//! FLAC 文件结构:
//! ```text
//! Magic:     "fLaC" (4 bytes)
//! Metadata:  一系列 metadata block
//!   - STREAMINFO (必须, 第一个)
//!   - PADDING, APPLICATION, SEEKTABLE, VORBIS_COMMENT, CUESHEET, PICTURE (可选)
//! Frames:    FLAC 音频帧序列
//!   - 每帧以同步码 0xFFF8 或 0xFFF9 开头
//!   - 每帧包含完整的帧头、子帧数据、CRC-16
//! ```
//!
//! STREAMINFO 块 (34 bytes):
//! ```text
//! min_block_size:  16 bits
//! max_block_size:  16 bits
//! min_frame_size:  24 bits
//! max_frame_size:  24 bits
//! sample_rate:     20 bits
//! channels:        3 bits  (channels - 1)
//! bits_per_sample: 5 bits  (bits - 1)
//! total_samples:   36 bits
//! md5:             128 bits
//! ```

use log::{debug, warn};
use tao_codec::CodecId;
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult, crc};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::stream::{AudioStreamParams, Stream, StreamParams};

/// FLAC 同步码 (14 bits: 0b11111111111110)
const FLAC_SYNC_CODE: u16 = 0xFFF8;
/// FLAC 同步码掩码 (高 14 位)
const FLAC_SYNC_MASK: u16 = 0xFFFE;

/// FLAC 元数据块类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum MetadataBlockType {
    StreamInfo = 0,
    Padding = 1,
    Application = 2,
    SeekTable = 3,
    VorbisComment = 4,
    CueSheet = 5,
    Picture = 6,
}

impl MetadataBlockType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::StreamInfo),
            1 => Some(Self::Padding),
            2 => Some(Self::Application),
            3 => Some(Self::SeekTable),
            4 => Some(Self::VorbisComment),
            5 => Some(Self::CueSheet),
            6 => Some(Self::Picture),
            _ => None,
        }
    }
}

/// FLAC STREAMINFO 解析结果
#[derive(Debug, Clone)]
struct StreamInfo {
    min_block_size: u16,
    max_block_size: u16,
    #[allow(dead_code)]
    min_frame_size: u32,
    max_frame_size: u32,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
    total_samples: u64,
    #[allow(dead_code)]
    md5: [u8; 16],
}

/// FLAC 解封装器
pub struct FlacDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// STREAMINFO
    stream_info: Option<StreamInfo>,
    /// 音频帧数据开始的文件偏移
    frames_offset: u64,
    /// 当前读取位置
    current_pos: u64,
    /// 文件总大小
    file_size: u64,
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 位深
    bits_per_sample: u32,
    /// 当前帧序号 (用于生成 pts)
    frame_number: u64,
    /// extra_data (STREAMINFO 原始字节, 34 bytes)
    extra_data: Vec<u8>,
    /// 元数据
    metadata: Vec<(String, String)>,
    /// 最大帧大小 (用于读取缓冲区)
    max_frame_size: u32,
    /// 上一次返回 packet 的采样数
    last_block_size: u64,
}

impl FlacDemuxer {
    /// 创建 FLAC 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            stream_info: None,
            frames_offset: 0,
            current_pos: 0,
            file_size: 0,
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            frame_number: 0,
            extra_data: Vec::new(),
            metadata: Vec::new(),
            max_frame_size: 0,
            last_block_size: 0,
        }))
    }

    /// 解析 STREAMINFO 块
    fn parse_stream_info(data: &[u8]) -> TaoResult<StreamInfo> {
        if data.len() < 34 {
            return Err(TaoError::InvalidData(format!(
                "STREAMINFO 块大小不足: {} < 34",
                data.len(),
            )));
        }

        let min_block_size = u16::from_be_bytes([data[0], data[1]]);
        let max_block_size = u16::from_be_bytes([data[2], data[3]]);

        let min_frame_size =
            (u32::from(data[4]) << 16) | (u32::from(data[5]) << 8) | u32::from(data[6]);
        let max_frame_size =
            (u32::from(data[7]) << 16) | (u32::from(data[8]) << 8) | u32::from(data[9]);

        // bytes 10-13: sample_rate(20) + channels(3) + bps(5) + total_samples_hi(4)
        let sr_hi = u32::from(data[10]) << 12;
        let sr_mid = u32::from(data[11]) << 4;
        let sr_lo = u32::from(data[12]) >> 4;
        let sample_rate = sr_hi | sr_mid | sr_lo;

        let channels = ((u32::from(data[12]) >> 1) & 0x07) + 1;
        let bps_hi = (u32::from(data[12]) & 0x01) << 4;
        let bps_lo = u32::from(data[13]) >> 4;
        let bits_per_sample = (bps_hi | bps_lo) + 1; // 存储值 = bps-1, 加 1 得到实际值

        let total_hi = u64::from(data[13] & 0x0F) << 32;
        let total_lo = u64::from(u32::from_be_bytes([data[14], data[15], data[16], data[17]]));
        let total_samples = total_hi | total_lo;

        let mut md5 = [0u8; 16];
        md5.copy_from_slice(&data[18..34]);

        Ok(StreamInfo {
            min_block_size,
            max_block_size,
            min_frame_size,
            max_frame_size,
            sample_rate,
            channels,
            bits_per_sample,
            total_samples,
            md5,
        })
    }

    /// 根据位深确定采样格式
    fn resolve_sample_format(bits_per_sample: u32) -> SampleFormat {
        match bits_per_sample {
            8 => SampleFormat::U8,
            16 => SampleFormat::S16,
            24 | 32 => SampleFormat::S32,
            _ => SampleFormat::S32,
        }
    }

    /// 在缓冲区中搜索下一个 FLAC 帧同步码
    ///
    /// 返回同步码在缓冲区中的偏移.
    /// 对每个候选位置都验证帧头有效性 (CRC-8), 以排除假同步码.
    fn find_sync_code(buf: &[u8]) -> Option<usize> {
        Self::find_sync_code_from(buf, 0)
    }

    /// 从指定偏移开始搜索下一个有效 FLAC 帧同步码
    fn find_sync_code_from(buf: &[u8], start: usize) -> Option<usize> {
        if buf.len() < start + 2 {
            return None;
        }
        for i in start..buf.len() - 1 {
            let word = u16::from_be_bytes([buf[i], buf[i + 1]]);
            if word & FLAC_SYNC_MASK == FLAC_SYNC_CODE && Self::validate_frame_header(&buf[i..]) {
                return Some(i);
            }
        }
        None
    }

    /// 验证候选帧头的有效性
    ///
    /// 检查帧头字段合法性并验证 CRC-8, 以区分真正的帧同步码和数据中的巧合匹配.
    fn validate_frame_header(data: &[u8]) -> bool {
        // 至少需要 5 字节: sync(2) + byte2 + byte3 + utf8(1至少) 才能做基本验证
        if data.len() < 5 {
            return false;
        }

        let byte2 = data[2];
        let byte3 = data[3];

        let bs_code = byte2 >> 4;
        let sr_code = byte2 & 0x0F;
        let ch_code = byte3 >> 4;
        let ss_code = (byte3 >> 1) & 0x07;
        let reserved_bit = byte3 & 0x01;

        // 保留位必须为 0
        if reserved_bit != 0 {
            return false;
        }

        // 块大小编码 0 是保留的
        if bs_code == 0 {
            return false;
        }

        // 采样率编码 15 是无效的
        if sr_code == 15 {
            return false;
        }

        // 声道分配: 0-10 有效, 11-15 保留
        if ch_code > 10 {
            return false;
        }

        // 采样大小: 3 是保留的
        if ss_code == 3 {
            return false;
        }

        // 尝试计算帧头长度并验证 CRC-8
        Self::verify_header_crc8(data, bs_code, sr_code)
    }

    /// 计算帧头长度并验证 CRC-8
    ///
    /// 帧头结构: sync(2) + byte2 + byte3 + utf8(1-7) + [ext_bs] + [ext_sr] + crc8(1)
    fn verify_header_crc8(data: &[u8], bs_code: u8, sr_code: u8) -> bool {
        // UTF-8 编码的帧/采样号从 byte 4 开始
        if data.len() < 5 {
            return false;
        }

        let first_utf8 = data[4];
        let utf8_len = if first_utf8 & 0x80 == 0 {
            1
        } else if first_utf8 & 0xE0 == 0xC0 {
            2
        } else if first_utf8 & 0xF0 == 0xE0 {
            3
        } else if first_utf8 & 0xF8 == 0xF0 {
            4
        } else if first_utf8 & 0xFC == 0xF8 {
            5
        } else if first_utf8 & 0xFE == 0xFC {
            6
        } else if first_utf8 == 0xFE {
            7
        } else {
            // 无效的 UTF-8 首字节
            return false;
        };

        // 验证 UTF-8 后续字节 (必须以 10xxxxxx 开头)
        let utf8_end = 4 + utf8_len;
        if utf8_end > data.len() {
            return false;
        }
        for &b in &data[5..utf8_end] {
            if b & 0xC0 != 0x80 {
                return false;
            }
        }

        let mut pos = utf8_end;

        // 扩展块大小
        match bs_code {
            6 => pos += 1, // 8-bit
            7 => pos += 2, // 16-bit
            _ => {}
        }

        // 扩展采样率
        match sr_code {
            12 => pos += 1,      // 8-bit (kHz)
            13 | 14 => pos += 2, // 16-bit (Hz 或 10*Hz)
            _ => {}
        }

        // CRC-8 在 pos 位置
        if pos >= data.len() {
            // 数据不足以包含 CRC-8, 按基本验证结果通过
            return true;
        }

        let crc_read = data[pos];
        let crc_calc = crc::crc8(&data[..pos]);
        crc_read == crc_calc
    }
}

impl Demuxer for FlacDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::FlacContainer
    }

    fn name(&self) -> &str {
        "flac"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        self.file_size = io.size().unwrap_or(0);

        // 读取 "fLaC" 魔数
        let magic = io.read_tag()?;
        if &magic != b"fLaC" {
            return Err(TaoError::InvalidData("不是有效的 FLAC 文件".into()));
        }

        debug!("检测到 FLAC 文件");

        // 读取 metadata blocks
        let mut last_block = false;
        let mut first_block = true;

        while !last_block {
            // 每个 metadata block header: 1 byte (is_last:1 + type:7) + 3 bytes (size)
            let header_byte = io.read_u8()?;
            last_block = (header_byte & 0x80) != 0;
            let block_type_raw = header_byte & 0x7F;
            let block_size_bytes = io.read_bytes(3)?;
            let block_size = (u32::from(block_size_bytes[0]) << 16)
                | (u32::from(block_size_bytes[1]) << 8)
                | u32::from(block_size_bytes[2]);

            let block_type = MetadataBlockType::from_u8(block_type_raw);

            match block_type {
                Some(MetadataBlockType::StreamInfo) => {
                    if !first_block {
                        warn!("STREAMINFO 不是第一个 metadata block");
                    }
                    let data = io.read_bytes(block_size as usize)?;
                    let info = Self::parse_stream_info(&data)?;

                    debug!(
                        "STREAMINFO: rate={}, channels={}, bps={}, total_samples={}, block_size={}-{}",
                        info.sample_rate,
                        info.channels,
                        info.bits_per_sample,
                        info.total_samples,
                        info.min_block_size,
                        info.max_block_size,
                    );

                    self.extra_data = data;
                    self.stream_info = Some(info);
                }
                Some(MetadataBlockType::VorbisComment) => {
                    // 尝试解析 Vorbis comment
                    let data = io.read_bytes(block_size as usize)?;
                    self.parse_vorbis_comment(&data);
                }
                _ => {
                    // 跳过未处理的 metadata block
                    if let Some(bt) = block_type {
                        debug!("跳过 metadata block: {:?}, 大小={}", bt, block_size);
                    } else {
                        debug!(
                            "跳过未知 metadata block: type={}, 大小={}",
                            block_type_raw, block_size,
                        );
                    }
                    io.skip(block_size as usize)?;
                }
            }

            first_block = false;
        }

        let info = self
            .stream_info
            .as_ref()
            .ok_or_else(|| TaoError::InvalidData("FLAC 文件缺少 STREAMINFO block".into()))?;

        self.sample_rate = info.sample_rate;
        self.channels = info.channels;
        self.bits_per_sample = info.bits_per_sample;
        self.max_frame_size = if info.max_frame_size > 0 {
            info.max_frame_size
        } else {
            // 估算: max_block_size * channels * (bps/8) + 帧头/尾开销
            (u32::from(info.max_block_size) * info.channels * info.bits_per_sample.div_ceil(8))
                + 256
        };

        self.frames_offset = io.position()?;
        self.current_pos = self.frames_offset;

        let sample_format = Self::resolve_sample_format(info.bits_per_sample);
        let channel_layout = ChannelLayout::from_channels(info.channels);
        let time_base = Rational::new(1, info.sample_rate as i32);
        let bit_rate = u64::from(info.sample_rate)
            * u64::from(info.channels)
            * u64::from(info.bits_per_sample);

        let stream = Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Flac,
            time_base,
            duration: info.total_samples as i64,
            start_time: 0,
            nb_frames: 0, // FLAC 帧数未知
            extra_data: self.extra_data.clone(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: info.sample_rate,
                channel_layout,
                sample_format,
                bit_rate,
                frame_size: u32::from(info.max_block_size),
            }),
            metadata: Vec::new(),
        };

        self.streams = vec![stream];
        self.frame_number = 0;
        self.last_block_size = 0;

        debug!(
            "FLAC 打开完成: {} Hz, {} 声道, {} 位, 总采样={}",
            info.sample_rate, info.channels, info.bits_per_sample, info.total_samples,
        );

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<tao_codec::Packet> {
        // 读取足够多的数据来找到帧边界
        // FLAC 帧以 0xFFF8/0xFFF9 同步码开头
        let max_read = self.max_frame_size.max(16384) as usize;
        let buf_size = max_read + 16; // 多读一些以找到下一帧边界

        // 确保在正确位置
        io.seek(std::io::SeekFrom::Start(self.current_pos))?;

        // 计算实际可读大小 (不超过文件末尾)
        let remaining = if self.file_size > self.current_pos {
            (self.file_size - self.current_pos) as usize
        } else {
            // 无文件大小信息时, 尝试读取最大量
            buf_size
        };

        if remaining < 2 {
            return Err(TaoError::Eof);
        }

        let actual_read = buf_size.min(remaining);

        let buf = match io.read_bytes(actual_read) {
            Ok(buf) => buf,
            Err(TaoError::Eof) => return Err(TaoError::Eof),
            Err(e) => return Err(e),
        };

        if buf.len() < 2 {
            return Err(TaoError::Eof);
        }

        // 验证当前位置是帧同步码
        let first_word = u16::from_be_bytes([buf[0], buf[1]]);
        if first_word & FLAC_SYNC_MASK != FLAC_SYNC_CODE || !Self::validate_frame_header(&buf) {
            // 尝试搜索下一个有效同步码
            if let Some(offset) = Self::find_sync_code(&buf) {
                self.current_pos += offset as u64;
                io.seek(std::io::SeekFrom::Start(self.current_pos))?;
                return self.read_packet(io);
            }
            return Err(TaoError::Eof);
        }

        // 搜索下一个有效帧同步码来确定当前帧大小
        // 从偏移 2 开始搜索, 使用带 CRC-8 验证的搜索方法
        let frame_end = if buf.len() > 2 {
            Self::find_sync_code_from(&buf, 2)
        } else {
            None
        };

        let frame_size = frame_end.unwrap_or(buf.len());

        // 计算 PTS
        let pts = self.frame_number;

        let frame_data = buf[..frame_size].to_vec();
        let mut pkt = tao_codec::Packet::from_data(bytes::Bytes::from(frame_data));
        pkt.stream_index = 0;
        pkt.pts = pts as i64;
        pkt.dts = pkt.pts;
        pkt.time_base = Rational::new(1, self.sample_rate as i32);
        pkt.is_keyframe = true;
        pkt.pos = self.current_pos as i64;

        // 更新位置
        self.current_pos += frame_size as u64;

        // 从帧头中提取 block_size (用于 PTS 递增)
        // 帧头的 block_size 编码在字节 2 的高 4 位和字节 3
        let block_size = self.peek_block_size(&buf);
        pkt.duration = block_size as i64;
        self.frame_number += block_size;
        self.last_block_size = block_size;

        Ok(pkt)
    }

    fn seek(
        &mut self,
        io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        // 简单实现: seek 到开头
        self.current_pos = self.frames_offset;
        self.frame_number = 0;
        io.seek(std::io::SeekFrom::Start(self.frames_offset))?;
        Ok(())
    }

    fn duration(&self) -> Option<f64> {
        if let Some(info) = &self.stream_info {
            if info.sample_rate > 0 && info.total_samples > 0 {
                return Some(info.total_samples as f64 / f64::from(info.sample_rate));
            }
        }
        None
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

impl FlacDemuxer {
    /// 从帧头中提取 block_size
    fn peek_block_size(&self, frame_data: &[u8]) -> u64 {
        if frame_data.len() < 5 {
            return self
                .stream_info
                .as_ref()
                .map_or(4096, |i| u64::from(i.max_block_size));
        }

        // byte[2] 高 4 位 = block_size 编码
        let bs_code = (frame_data[2] >> 4) & 0x0F;
        match bs_code {
            0 => {
                // reserved
                self.stream_info
                    .as_ref()
                    .map_or(4096, |i| u64::from(i.max_block_size))
            }
            1 => 192,
            2..=5 => 576 * (1u64 << (bs_code - 2)),
            6 => {
                // 从帧头中读取 8-bit block_size - 1
                // 需要跳到帧头末尾才能读取, 这里估算
                self.stream_info
                    .as_ref()
                    .map_or(4096, |i| u64::from(i.max_block_size))
            }
            7 => {
                // 从帧头中读取 16-bit block_size - 1
                self.stream_info
                    .as_ref()
                    .map_or(4096, |i| u64::from(i.max_block_size))
            }
            8..=15 => 256 * (1u64 << (bs_code - 8)),
            _ => unreachable!(),
        }
    }

    /// 解析 Vorbis Comment 块
    fn parse_vorbis_comment(&mut self, data: &[u8]) {
        // Vorbis comment 格式 (小端!):
        // vendor_length(4) + vendor_string + comment_count(4)
        // 然后每个 comment: length(4) + "KEY=VALUE"
        if data.len() < 8 {
            return;
        }

        let vendor_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut pos = 4 + vendor_len;
        if pos + 4 > data.len() {
            return;
        }

        let count =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        for _ in 0..count {
            if pos + 4 > data.len() {
                break;
            }
            let len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
            pos += 4;
            if pos + len > data.len() {
                break;
            }

            if let Ok(comment) = std::str::from_utf8(&data[pos..pos + len]) {
                if let Some((key, value)) = comment.split_once('=') {
                    self.metadata.push((key.to_uppercase(), value.to_string()));
                }
            }
            pos += len;
        }
    }
}

/// FLAC 格式探测器
pub struct FlacProbe;

impl FormatProbe for FlacProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        // 检查 "fLaC" 魔数
        if data.len() >= 4 && &data[0..4] == b"fLaC" {
            return Some(SCORE_MAX);
        }

        // 仅根据扩展名
        if let Some(name) = filename {
            if name.to_ascii_lowercase().ends_with(".flac") {
                return Some(SCORE_EXTENSION);
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::FlacContainer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_探测_flac_魔数() {
        let data = b"fLaC\x00\x00\x00\x22"; // magic + metadata block header
        let probe = FlacProbe;
        assert_eq!(probe.probe(data, None), Some(SCORE_MAX));
    }

    #[test]
    fn test_探测_flac_扩展名() {
        let probe = FlacProbe;
        assert_eq!(probe.probe(&[], Some("test.flac")), Some(SCORE_EXTENSION));
        assert_eq!(probe.probe(&[], Some("test.wav")), None);
    }

    #[test]
    fn test_parse_stream_info() {
        // 构造一个 STREAMINFO 块 (34 bytes)
        // min_block_size=4096, max_block_size=4096
        // min_frame_size=0, max_frame_size=0
        // sample_rate=44100, channels=2, bits_per_sample=16
        // total_samples=441000 (10 seconds)
        let mut data = [0u8; 34];

        // min/max block size (2 bytes each)
        data[0..2].copy_from_slice(&4096u16.to_be_bytes());
        data[2..4].copy_from_slice(&4096u16.to_be_bytes());

        // min/max frame size (3 bytes each) = 0
        // already zero

        // sample_rate (20 bits) + channels-1 (3 bits) + bps-1 (5 bits)
        // 44100 = 0xAC44
        // 20 bits: 0x0AC44 = 0b 0000 1010 1100 0100 0100
        // channels-1 = 1 (stereo) = 0b001
        // bps-1 = 15 = 0b01111
        // combined: 0b 0000_1010_1100_0100_0100 001 01111 xxxx
        // byte 10: 0b00001010 = 0x0A
        // byte 11: 0b11000100 = 0xC4
        // byte 12: 0b0100_001_0 = 0b01000010 = 0x42 (rate低4=0100, chan=001, bps高1=0)
        // byte 13: 0b1111_xxxx = 0xF0 (bps低4=1111, total_hi=0)
        data[10] = 0x0A;
        data[11] = 0xC4;
        data[12] = 0x42;
        data[13] = 0xF0;

        // total_samples = 441000 = 0x6BA98
        // lower 32 bits in bytes 14-17
        data[13] |= 0x00; // high 4 bits of total_samples
        let total_low = 441000u32;
        data[14..18].copy_from_slice(&total_low.to_be_bytes());

        let info = FlacDemuxer::parse_stream_info(&data).unwrap();
        assert_eq!(info.min_block_size, 4096);
        assert_eq!(info.max_block_size, 4096);
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.total_samples, 441000);
    }
}
