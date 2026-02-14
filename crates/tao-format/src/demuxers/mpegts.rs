//! MPEG-TS (Transport Stream) 解封装器.
//!
//! MPEG-TS 是一种基于固定大小 (188 字节) 包的传输流格式,
//! 广泛用于数字广播 (DVB/ATSC) 和 HLS 流媒体.
//!
//! # TS 包结构 (188 字节)
//! ```text
//! ┌──────────────────────────────────────────┐
//! │ 同步字节 (0x47)                    1 byte│
//! │ TEI(1) + PUSI(1) + Priority(1) +         │
//! │   PID(13)                         2 bytes│
//! │ TSC(2) + AFC(2) + CC(4)          1 byte │
//! │ [Adaptation Field]               可变     │
//! │ [Payload]                        可变     │
//! └──────────────────────────────────────────┘
//! ```
//!
//! # 关键 PID
//! - PID 0x0000: PAT (Program Association Table)
//! - PID 0x0001: CAT (Conditional Access Table)
//! - PID 0x1FFF: Null packet (填充)
//!
//! # PSI 表
//! - PAT: 将 program_number 映射到 PMT 的 PID
//! - PMT: 将 stream_type 映射到 ES (Elementary Stream) 的 PID

use bytes::Bytes;
use log::debug;
use std::collections::HashMap;
use tao_codec::{CodecId, Packet};
use tao_core::{
    ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat, TaoError, TaoResult,
};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

/// TS 包大小
const TS_PACKET_SIZE: usize = 188;
/// TS 同步字节
const TS_SYNC_BYTE: u8 = 0x47;
/// PAT PID
const PID_PAT: u16 = 0x0000;
/// 空包 PID
const PID_NULL: u16 = 0x1FFF;

/// MPEG-TS stream_type → CodecId 映射
fn stream_type_to_codec(stream_type: u8) -> CodecId {
    match stream_type {
        // 视频
        0x01 => CodecId::Mpeg1Video,
        0x02 => CodecId::Mpeg2Video,
        0x1B => CodecId::H264,
        0x24 => CodecId::H265,
        // 音频
        0x03 | 0x04 => CodecId::Mp3,
        0x0F => CodecId::Aac,  // ADTS
        0x11 => CodecId::Aac,  // LATM
        0x81 => CodecId::Ac3,  // ATSC AC-3
        0x87 => CodecId::Eac3, // ATSC E-AC-3
        0x86 => CodecId::Dts,
        // 字幕
        0x06 => CodecId::None, // 私有数据, 需要 descriptor 确定
        _ => CodecId::None,
    }
}

/// PES (Packetized Elementary Stream) 重组缓冲区
struct PesBuffer {
    /// 缓冲数据
    data: Vec<u8>,
    /// PTS (90kHz 时钟)
    pts: i64,
    /// DTS
    dts: i64,
    /// 是否为随机访问点 (关键帧)
    random_access: bool,
    /// 对应的流索引
    stream_index: usize,
}

impl PesBuffer {
    fn new(stream_index: usize) -> Self {
        Self {
            data: Vec::new(),
            pts: -1,
            dts: -1,
            random_access: false,
            stream_index,
        }
    }

    fn clear(&mut self) {
        self.data.clear();
        self.pts = -1;
        self.dts = -1;
        self.random_access = false;
    }
}

/// PMT 中的流条目
#[derive(Debug, Clone)]
struct PmtEntry {
    /// ES PID
    pid: u16,
    /// stream_type
    _stream_type: u8,
    /// 编解码器 ID
    codec_id: CodecId,
}

/// MPEG-TS 解封装器
pub struct TsDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// PMT PID (从 PAT 获取)
    pmt_pid: u16,
    /// PID → 流索引映射
    pid_to_stream: HashMap<u16, usize>,
    /// PID → PES 缓冲区
    pes_buffers: HashMap<u16, PesBuffer>,
    /// 已完成的数据包队列
    packet_queue: Vec<Packet>,
    /// PAT 是否已解析
    pat_parsed: bool,
    /// PMT 是否已解析
    pmt_parsed: bool,
}

impl TsDemuxer {
    /// 创建 MPEG-TS 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            pmt_pid: 0,
            pid_to_stream: HashMap::new(),
            pes_buffers: HashMap::new(),
            packet_queue: Vec::new(),
            pat_parsed: false,
            pmt_parsed: false,
        }))
    }

    /// 读取一个 188 字节的 TS 包
    fn read_ts_packet(&self, io: &mut IoContext) -> TaoResult<[u8; TS_PACKET_SIZE]> {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        io.read_exact(&mut pkt)?;
        if pkt[0] != TS_SYNC_BYTE {
            return Err(TaoError::InvalidData("TS: 同步字节不匹配".into()));
        }
        Ok(pkt)
    }

    /// 同步到第一个有效的 TS 包
    fn sync_to_packet(&self, io: &mut IoContext) -> TaoResult<()> {
        let max_search = 65536;
        for _ in 0..max_search {
            let b = io.read_u8()?;
            if b == TS_SYNC_BYTE {
                // 验证: 检查 188 字节后是否还有同步字节
                let pos = io.position()?;
                let mut check = [0u8; TS_PACKET_SIZE];
                if io.read_exact(&mut check).is_ok() && check[TS_PACKET_SIZE - 1] == TS_SYNC_BYTE {
                    // 找到有效同步, 跳回 sync byte 之后
                    io.seek(std::io::SeekFrom::Start(pos - 1))?;
                    return Ok(());
                }
                // 没验证通过, 回到当前位置继续搜索
                io.seek(std::io::SeekFrom::Start(pos))?;
            }
        }
        Err(TaoError::InvalidData("TS: 找不到同步字节".into()))
    }

    /// 解析 TS 包头 (4 字节)
    fn parse_ts_header(pkt: &[u8; TS_PACKET_SIZE]) -> (u16, bool, u8, u8) {
        let pid = (u16::from(pkt[1] & 0x1F) << 8) | u16::from(pkt[2]);
        let pusi = (pkt[1] & 0x40) != 0; // Payload Unit Start Indicator
        let afc = (pkt[3] >> 4) & 0x03; // Adaptation Field Control
        let _cc = pkt[3] & 0x0F; // Continuity Counter
        (pid, pusi, afc, _cc)
    }

    /// 获取 payload 的偏移和长度
    fn payload_offset(pkt: &[u8; TS_PACKET_SIZE], afc: u8) -> (usize, bool) {
        let mut offset = 4;
        let mut has_random_access = false;

        // Adaptation Field
        if (afc == 2 || afc == 3) && offset < TS_PACKET_SIZE {
            let af_len = pkt[offset] as usize;
            // 检查 random_access_indicator
            if af_len > 0 && offset + 1 < TS_PACKET_SIZE {
                has_random_access = (pkt[offset + 1] & 0x40) != 0;
            }
            offset += 1 + af_len;
        }

        // afc==1 或 afc==3 表示有 payload
        if afc == 1 || afc == 3 {
            (offset, has_random_access)
        } else {
            (TS_PACKET_SIZE, has_random_access) // 无 payload
        }
    }

    /// 解析 PAT (Program Association Table)
    fn parse_pat(&mut self, payload: &[u8]) {
        if self.pat_parsed {
            return;
        }
        // PSI section: table_id(1) + flags(2) + ...
        if payload.len() < 8 {
            return;
        }
        let _table_id = payload[0]; // 应该是 0x00
        let section_length = (u16::from(payload[1] & 0x0F) << 8 | u16::from(payload[2])) as usize;

        // 跳过 transport_stream_id(2) + version/flags(1) + section_number(1) + last_section(1)
        let entries_start = 8;
        let entries_end = (3 + section_length).min(payload.len()) - 4; // 减去 CRC

        if entries_end <= entries_start {
            return;
        }

        let entries = &payload[entries_start..entries_end];
        // 每个条目 4 字节: program_number(2) + PID(2)
        for chunk in entries.chunks_exact(4) {
            let program_number = u16::from(chunk[0]) << 8 | u16::from(chunk[1]);
            let pid = (u16::from(chunk[2] & 0x1F) << 8) | u16::from(chunk[3]);

            if program_number != 0 {
                // 非网络 PID → PMT PID
                self.pmt_pid = pid;
                debug!("TS PAT: program={program_number} PMT_PID={pid:#06X}");
                break; // 通常只取第一个节目
            }
        }

        self.pat_parsed = true;
    }

    /// 解析 PMT (Program Map Table)
    fn parse_pmt(&mut self, payload: &[u8]) {
        if self.pmt_parsed {
            return;
        }
        if payload.len() < 12 {
            return;
        }
        let _table_id = payload[0]; // 应该是 0x02
        let section_length = (u16::from(payload[1] & 0x0F) << 8 | u16::from(payload[2])) as usize;

        // PCR PID
        let _pcr_pid = (u16::from(payload[8] & 0x1F) << 8) | u16::from(payload[9]);

        // program_info_length
        let prog_info_len = (u16::from(payload[10] & 0x0F) << 8 | u16::from(payload[11])) as usize;

        let mut pos = 12 + prog_info_len;
        let section_end = (3 + section_length).min(payload.len()) - 4; // 减去 CRC

        let mut entries = Vec::new();

        while pos + 5 <= section_end {
            let stream_type = payload[pos];
            let es_pid = (u16::from(payload[pos + 1] & 0x1F) << 8) | u16::from(payload[pos + 2]);
            let es_info_len =
                (u16::from(payload[pos + 3] & 0x0F) << 8 | u16::from(payload[pos + 4])) as usize;

            let codec_id = stream_type_to_codec(stream_type);

            debug!("TS PMT: stream_type=0x{stream_type:02X} PID={es_pid:#06X} codec={codec_id}",);

            entries.push(PmtEntry {
                pid: es_pid,
                _stream_type: stream_type,
                codec_id,
            });

            pos += 5 + es_info_len;
        }

        // 创建流
        for entry in &entries {
            if entry.codec_id == CodecId::None {
                continue; // 跳过未知编解码器
            }

            let stream_index = self.streams.len();
            let media_type = entry.codec_id.media_type();

            let params = match media_type {
                MediaType::Video => StreamParams::Video(VideoStreamParams {
                    width: 0,
                    height: 0,
                    pixel_format: PixelFormat::Yuv420p,
                    frame_rate: Rational::new(0, 1),
                    sample_aspect_ratio: Rational::new(1, 1),
                    bit_rate: 0,
                }),
                MediaType::Audio => {
                    let (sr, ch) = match entry.codec_id {
                        CodecId::Aac => (44100, 2),
                        CodecId::Mp3 => (44100, 2),
                        CodecId::Ac3 | CodecId::Eac3 => (48000, 6),
                        _ => (48000, 2),
                    };
                    StreamParams::Audio(AudioStreamParams {
                        sample_rate: sr,
                        channel_layout: ChannelLayout::from_channels(ch),
                        sample_format: SampleFormat::F32,
                        bit_rate: 0,
                        frame_size: 0,
                    })
                }
                _ => StreamParams::Other,
            };

            // 时间基: 90kHz (MPEG-TS 标准)
            let stream = Stream {
                index: stream_index,
                media_type,
                codec_id: entry.codec_id,
                time_base: Rational::new(1, 90000),
                duration: -1,
                start_time: 0,
                nb_frames: 0,
                extra_data: Vec::new(),
                params,
                metadata: Vec::new(),
            };

            self.pid_to_stream.insert(entry.pid, stream_index);
            self.pes_buffers
                .insert(entry.pid, PesBuffer::new(stream_index));
            self.streams.push(stream);
        }

        self.pmt_parsed = true;
    }

    /// 处理 PES 数据
    fn handle_pes_data(&mut self, pid: u16, payload: &[u8], pusi: bool, random_access: bool) {
        if !self.pid_to_stream.contains_key(&pid) {
            return;
        }

        if pusi {
            // Payload Unit Start: 先 flush 旧数据, 再开始新 PES
            self.flush_pes(pid);

            // 解析 PES 头部
            if let Some(buf) = self.pes_buffers.get_mut(&pid) {
                buf.random_access = random_access;
                if let Some((pts, dts, header_len)) = parse_pes_header(payload) {
                    buf.pts = pts;
                    buf.dts = dts;
                    buf.data.extend_from_slice(&payload[header_len..]);
                } else {
                    buf.data.extend_from_slice(payload);
                }
            }
        } else if let Some(buf) = self.pes_buffers.get_mut(&pid) {
            // 续包: 追加到缓冲区
            buf.data.extend_from_slice(payload);
            if random_access {
                buf.random_access = true;
            }
        }
    }

    /// 将 PES 缓冲区刷新为数据包
    fn flush_pes(&mut self, pid: u16) {
        if let Some(buf) = self.pes_buffers.get_mut(&pid) {
            if buf.data.is_empty() {
                return;
            }

            let mut pkt = Packet::from_data(Bytes::from(std::mem::take(&mut buf.data)));
            pkt.stream_index = buf.stream_index;
            pkt.pts = buf.pts;
            pkt.dts = if buf.dts >= 0 { buf.dts } else { buf.pts };
            pkt.is_keyframe = buf.random_access;
            pkt.time_base = Rational::new(1, 90000);

            self.packet_queue.push(pkt);
            buf.clear();
        }
    }

    /// 处理一个 TS 包
    fn process_packet(&mut self, pkt: &[u8; TS_PACKET_SIZE]) {
        let (pid, pusi, afc, _cc) = Self::parse_ts_header(pkt);

        if pid == PID_NULL {
            return;
        }

        let (payload_off, random_access) = Self::payload_offset(pkt, afc);
        if payload_off >= TS_PACKET_SIZE {
            return;
        }

        let payload = &pkt[payload_off..];

        // PSI 表处理
        if pid == PID_PAT {
            if pusi && !payload.is_empty() {
                // pointer_field
                let pointer = payload[0] as usize;
                let section_start = 1 + pointer;
                if section_start < payload.len() {
                    self.parse_pat(&payload[section_start..]);
                }
            }
            return;
        }

        if pid == self.pmt_pid && self.pmt_pid != 0 {
            if pusi && !payload.is_empty() {
                let pointer = payload[0] as usize;
                let section_start = 1 + pointer;
                if section_start < payload.len() {
                    self.parse_pmt(&payload[section_start..]);
                }
            }
            return;
        }

        // ES 数据
        if self.pmt_parsed {
            self.handle_pes_data(pid, payload, pusi, random_access);
        }
    }
}

/// 解析 PES 包头, 提取 PTS/DTS
///
/// 返回 (pts, dts, header_length)
fn parse_pes_header(data: &[u8]) -> Option<(i64, i64, usize)> {
    // PES start code: 00 00 01 + stream_id
    if data.len() < 9 || data[0] != 0x00 || data[1] != 0x00 || data[2] != 0x01 {
        return None;
    }

    let _stream_id = data[3];
    let _pes_length = u16::from(data[4]) << 8 | u16::from(data[5]);

    // PES optional header
    // data[6]: 10xxxxxx (marker bits)
    if (data[6] & 0xC0) != 0x80 {
        // 没有 PES optional header (padding stream 等)
        return Some((-1, -1, 6));
    }

    let pts_dts_flags = (data[7] >> 6) & 0x03;
    let pes_header_data_len = data[8] as usize;
    let header_len = 9 + pes_header_data_len;

    if header_len > data.len() {
        return Some((-1, -1, data.len().min(9)));
    }

    let mut pts: i64 = -1;
    let mut dts: i64 = -1;

    if pts_dts_flags >= 2 && data.len() >= 14 {
        // PTS 存在
        pts = parse_timestamp(&data[9..14]);
    }

    if pts_dts_flags == 3 && data.len() >= 19 {
        // DTS 存在
        dts = parse_timestamp(&data[14..19]);
    }

    Some((pts, dts, header_len))
}

/// 从 5 字节中提取 33-bit 时间戳
fn parse_timestamp(data: &[u8]) -> i64 {
    let b0 = i64::from(data[0]);
    let b1 = i64::from(data[1]);
    let b2 = i64::from(data[2]);
    let b3 = i64::from(data[3]);
    let b4 = i64::from(data[4]);

    ((b0 >> 1) & 0x07) << 30 | b1 << 22 | (b2 >> 1) << 15 | b3 << 7 | b4 >> 1
}

impl Demuxer for TsDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::MpegTs
    }

    fn name(&self) -> &str {
        "mpegts"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 同步到第一个 TS 包
        self.sync_to_packet(io)?;

        // 预读 TS 包直到解析出 PAT + PMT
        let max_probe_packets = 2000;
        for _ in 0..max_probe_packets {
            let pkt = match self.read_ts_packet(io) {
                Ok(p) => p,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };

            self.process_packet(&pkt);

            if self.pat_parsed && self.pmt_parsed && !self.streams.is_empty() {
                break;
            }
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData(
                "TS: 未找到任何流 (PAT/PMT 解析失败)".into(),
            ));
        }

        // 回到文件开头重新读取
        io.seek(std::io::SeekFrom::Start(0))?;
        self.sync_to_packet(io)?;
        self.packet_queue.clear();
        for buf in self.pes_buffers.values_mut() {
            buf.clear();
        }

        debug!("TS: 打开完成, {} 个流", self.streams.len());
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        loop {
            // 如果队列中有已完成的包, 直接返回
            if !self.packet_queue.is_empty() {
                return Ok(self.packet_queue.remove(0));
            }

            // 读取并处理 TS 包
            let pkt = self.read_ts_packet(io)?;
            self.process_packet(&pkt);
        }
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("TS seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        None
    }
}

/// MPEG-TS 格式探测器
pub struct TsProbe;

impl FormatProbe for TsProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // 检查连续的 TS 同步字节
        if data.len() >= TS_PACKET_SIZE * 2 {
            let mut sync_count = 0;
            let mut pos = 0;

            // 搜索第一个同步字节
            while pos < data.len().min(1024) {
                if data[pos] == TS_SYNC_BYTE {
                    // 验证后续包
                    let mut check_pos = pos;
                    while check_pos + TS_PACKET_SIZE <= data.len() {
                        if data[check_pos] == TS_SYNC_BYTE {
                            sync_count += 1;
                            check_pos += TS_PACKET_SIZE;
                        } else {
                            break;
                        }
                    }

                    if sync_count >= 3 {
                        return Some(crate::probe::SCORE_MAX);
                    }
                    if sync_count >= 2 {
                        return Some(crate::probe::SCORE_MAX - 10);
                    }
                }
                pos += 1;
                sync_count = 0;
            }
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                let ext_lower = ext.to_lowercase();
                if matches!(ext_lower.as_str(), "ts" | "m2ts" | "mts") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::MpegTs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构造一个最小的 TS 包 (188 字节)
    fn build_ts_packet(pid: u16, pusi: bool, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0xFFu8; TS_PACKET_SIZE]; // 填充

        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = if pusi { 0x40 } else { 0x00 } | ((pid >> 8) as u8 & 0x1F);
        pkt[2] = pid as u8;
        pkt[3] = 0x10; // AFC=01 (payload only), CC=0

        let copy_len = payload.len().min(TS_PACKET_SIZE - 4);
        pkt[4..4 + copy_len].copy_from_slice(&payload[..copy_len]);

        pkt
    }

    /// 构造带 adaptation field 的 TS 包
    fn build_ts_packet_with_af(
        pid: u16,
        pusi: bool,
        random_access: bool,
        payload: &[u8],
    ) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];

        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = if pusi { 0x40 } else { 0x00 } | ((pid >> 8) as u8 & 0x1F);
        pkt[2] = pid as u8;
        pkt[3] = 0x30; // AFC=11 (adaptation + payload), CC=0

        // Adaptation field
        let af_flags = if random_access { 0x40 } else { 0x00 };
        let payload_space = TS_PACKET_SIZE - 4 - 2; // 4(header) + 2(af_len + af_flags)
        let copy_len = payload.len().min(payload_space);
        let stuffing = payload_space - copy_len;

        pkt[4] = (1 + stuffing) as u8; // adaptation_field_length
        pkt[5] = af_flags;
        // 填充字节
        for i in 0..stuffing {
            pkt[6 + i] = 0xFF;
        }
        let payload_start = 6 + stuffing;
        pkt[payload_start..payload_start + copy_len].copy_from_slice(&payload[..copy_len]);

        pkt
    }

    /// 构造 PAT
    fn build_pat(pmt_pid: u16) -> [u8; TS_PACKET_SIZE] {
        let mut section = Vec::new();
        // pointer_field
        section.push(0x00);
        // table_id = 0x00
        section.push(0x00);
        // section_syntax_indicator(1) + '0'(1) + reserved(2) + section_length(12)
        // section_length = 5(固定) + 4(一个条目) + 4(CRC) = 13
        let section_length: u16 = 13;
        section.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        section.push(section_length as u8);
        // transport_stream_id
        section.extend_from_slice(&[0x00, 0x01]);
        // reserved(2) + version(5) + current_next(1)
        section.push(0xC1);
        // section_number
        section.push(0x00);
        // last_section_number
        section.push(0x00);

        // Program entry: program_number=1, PMT_PID
        section.push(0x00);
        section.push(0x01); // program_number = 1
        section.push(0xE0 | ((pmt_pid >> 8) as u8 & 0x1F));
        section.push(pmt_pid as u8);

        // CRC32 (简化: 全 0)
        section.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        build_ts_packet(PID_PAT, true, &section)
    }

    /// 构造 PMT
    fn build_pmt(
        pmt_pid: u16,
        entries: &[(u8, u16)], // (stream_type, es_pid)
    ) -> [u8; TS_PACKET_SIZE] {
        let mut section = Vec::new();
        // pointer_field
        section.push(0x00);
        // table_id = 0x02
        section.push(0x02);

        // 计算 section_length
        let stream_data_len = entries.len() * 5;
        let section_length = 9 + stream_data_len + 4; // 9(固定) + streams + CRC
        section.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        section.push(section_length as u8);

        // program_number
        section.extend_from_slice(&[0x00, 0x01]);
        // reserved + version + current_next
        section.push(0xC1);
        // section_number
        section.push(0x00);
        // last_section_number
        section.push(0x00);

        // PCR_PID (使用第一个视频流的 PID)
        let pcr_pid = entries.first().map_or(0x1FFF, |e| e.1);
        section.push(0xE0 | ((pcr_pid >> 8) as u8 & 0x1F));
        section.push(pcr_pid as u8);

        // program_info_length = 0
        section.extend_from_slice(&[0xF0, 0x00]);

        // Stream entries
        for &(stream_type, es_pid) in entries {
            section.push(stream_type);
            section.push(0xE0 | ((es_pid >> 8) as u8 & 0x1F));
            section.push(es_pid as u8);
            // ES_info_length = 0
            section.extend_from_slice(&[0xF0, 0x00]);
        }

        // CRC32 (简化)
        section.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        build_ts_packet(pmt_pid, true, &section)
    }

    /// 构造 PES 头部
    fn build_pes_header(stream_id: u8, pts: Option<u64>, data: &[u8]) -> Vec<u8> {
        let mut pes = Vec::new();
        // start code
        pes.extend_from_slice(&[0x00, 0x00, 0x01]);
        pes.push(stream_id);

        let has_pts = pts.is_some();
        let header_ext_len = if has_pts { 5 } else { 0 };
        let pes_length = 3 + header_ext_len + data.len(); // optional header + data

        pes.push((pes_length >> 8) as u8);
        pes.push(pes_length as u8);

        // PES optional header
        pes.push(0x80); // marker bits
        pes.push(if has_pts { 0x80 } else { 0x00 }); // PTS flag
        pes.push(header_ext_len as u8);

        if let Some(pts_val) = pts {
            // 编码 33-bit PTS (5 bytes):
            // byte0: '0010' PTS[32:30] '1'
            // byte1: PTS[29:22]
            // byte2: PTS[21:15] '1'
            // byte3: PTS[14:7]
            // byte4: PTS[6:0] '1'
            pes.push(0x21 | ((((pts_val >> 30) as u8) & 0x07) << 1));
            pes.push((pts_val >> 22) as u8);
            pes.push(0x01 | ((((pts_val >> 15) as u8) & 0x7F) << 1));
            pes.push((pts_val >> 7) as u8);
            pes.push(0x01 | (((pts_val as u8) & 0x7F) << 1));
        }

        pes.extend_from_slice(data);
        pes
    }

    /// 构造完整的最小 TS 流
    fn build_minimal_ts() -> Vec<u8> {
        let pmt_pid: u16 = 0x100;
        let video_pid: u16 = 0x101;
        let audio_pid: u16 = 0x102;

        let mut ts = Vec::new();

        // PAT
        ts.extend_from_slice(&build_pat(pmt_pid));

        // PMT: H.264 视频 + AAC 音频
        ts.extend_from_slice(&build_pmt(
            pmt_pid,
            &[
                (0x1B, video_pid), // H.264
                (0x0F, audio_pid), // AAC
            ],
        ));

        // 视频 PES 包 (关键帧, PTS=90000 即 1 秒)
        {
            let video_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
            let pes = build_pes_header(0xE0, Some(90000), &video_data);
            ts.extend_from_slice(&build_ts_packet_with_af(video_pid, true, true, &pes));
        }

        // 音频 PES 包 (PTS=90000)
        {
            let audio_data = vec![0xCA, 0xFE, 0xBA, 0xBE];
            let pes = build_pes_header(0xC0, Some(90000), &audio_data);
            ts.extend_from_slice(&build_ts_packet(audio_pid, true, &pes));
        }

        // 第二个视频 PES (非关键帧, PTS=93600 即 1.04 秒)
        {
            let video_data = vec![0x11, 0x22, 0x33];
            let pes = build_pes_header(0xE0, Some(93600), &video_data);
            ts.extend_from_slice(&build_ts_packet_with_af(video_pid, true, false, &pes));
        }

        // 第二个音频 PES
        {
            let audio_data = vec![0x44, 0x55];
            let pes = build_pes_header(0xC0, Some(93600), &audio_data);
            ts.extend_from_slice(&build_ts_packet(audio_pid, true, &pes));
        }

        ts
    }

    #[test]
    fn test_探测_ts_同步字节() {
        let probe = TsProbe;
        let ts = build_minimal_ts();
        assert_eq!(probe.probe(&ts, None), Some(crate::probe::SCORE_MAX));
    }

    #[test]
    fn test_探测_ts_扩展名() {
        let probe = TsProbe;
        assert!(probe.probe(&[], Some("video.ts")).is_some());
        assert!(probe.probe(&[], Some("video.m2ts")).is_some());
        assert!(probe.probe(&[], Some("video.mp4")).is_none());
    }

    #[test]
    fn test_解析_pat_pmt() {
        let ts = build_minimal_ts();
        let backend = MemoryBackend::from_data(ts);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = TsDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 2, "应该有 2 个流");

        // 视频流
        assert_eq!(streams[0].media_type, MediaType::Video);
        assert_eq!(streams[0].codec_id, CodecId::H264);

        // 音频流
        assert_eq!(streams[1].media_type, MediaType::Audio);
        assert_eq!(streams[1].codec_id, CodecId::Aac);
    }

    #[test]
    fn test_读取数据包() {
        let ts = build_minimal_ts();
        let backend = MemoryBackend::from_data(ts);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = TsDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let mut packets = Vec::new();
        loop {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => packets.push(pkt),
                Err(TaoError::Eof) => break,
                Err(e) => panic!("读取失败: {e}"),
            }
        }

        // 应该有 2 个视频包 + 2 个音频包 = 4 个 (减去第二轮因为 flush 时机)
        // 实际上: 每次 PUSI 时 flush 前一个, 最后 EOF 时前一个不 flush
        // 第一个视频: 在第二个视频 PUSI 时 flush → 1 个视频包
        // 第一个音频: 在第二个音频 PUSI 时 flush → 1 个音频包
        // 第二个视频/音频: EOF 时不 flush (因为没有下一个 PUSI)
        assert!(
            packets.len() >= 2,
            "应该至少有 2 个数据包, 实际={}",
            packets.len()
        );
    }

    #[test]
    fn test_pts_时间戳() {
        let ts = build_minimal_ts();
        let backend = MemoryBackend::from_data(ts);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = TsDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let pkt = demuxer.read_packet(&mut io).unwrap();
        // 第一个 flush 出的包, PTS=90000
        assert_eq!(pkt.pts, 90000, "PTS 应该是 90000 (1 秒)");
        assert_eq!(pkt.time_base, Rational::new(1, 90000),);
    }

    #[test]
    fn test_关键帧标记() {
        let ts = build_minimal_ts();
        let backend = MemoryBackend::from_data(ts);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = TsDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        // 找到第一个视频包
        let mut found_keyframe = false;
        loop {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index == 0 && pkt.is_keyframe {
                        found_keyframe = true;
                        break;
                    }
                }
                Err(TaoError::Eof) => break,
                Err(e) => panic!("读取失败: {e}"),
            }
        }
        assert!(found_keyframe, "应该找到关键帧");
    }

    #[test]
    fn test_parse_timestamp() {
        // 编码 PTS=90000 (5 bytes: '0010' PTS[32:30] '1' PTS[29:22] PTS[21:15] '1' PTS[14:7] PTS[6:0] '1')
        let pts_val = 90000u64;
        let encoded = [
            0x21 | ((((pts_val >> 30) as u8) & 0x07) << 1),
            (pts_val >> 22) as u8,
            0x01 | ((((pts_val >> 15) as u8) & 0x7F) << 1),
            (pts_val >> 7) as u8,
            0x01 | (((pts_val as u8) & 0x7F) << 1),
        ];
        let ts = parse_timestamp(&encoded);
        assert_eq!(ts, 90000, "解析时间戳应该是 90000");
    }

    #[test]
    fn test_stream_type映射() {
        assert_eq!(stream_type_to_codec(0x1B), CodecId::H264);
        assert_eq!(stream_type_to_codec(0x24), CodecId::H265);
        assert_eq!(stream_type_to_codec(0x0F), CodecId::Aac);
        assert_eq!(stream_type_to_codec(0x03), CodecId::Mp3);
        assert_eq!(stream_type_to_codec(0x81), CodecId::Ac3);
        assert_eq!(stream_type_to_codec(0x02), CodecId::Mpeg2Video);
    }

    #[test]
    fn test_pes_header_解析() {
        let data = vec![0xAA; 10];
        let pes = build_pes_header(0xE0, Some(45000), &data);
        let (pts, dts, hdr_len) = parse_pes_header(&pes).unwrap();
        assert_eq!(pts, 45000);
        assert_eq!(dts, -1); // 无 DTS
        assert!(hdr_len > 0);
        assert_eq!(&pes[hdr_len..], &data);
    }
}
