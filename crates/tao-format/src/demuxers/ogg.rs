//! Ogg 容器解封装器.
//!
//! Ogg 是一个开放的多媒体容器格式, 支持多种编解码器:
//! - Vorbis (音频)
//! - Opus (音频)
//! - FLAC (音频)
//! - Theora (视频)
//!
//! # Ogg 页面结构
//! ```text
//! Capture pattern: "OggS" (4 bytes)
//! Version:         1 byte (always 0)
//! Header type:     1 byte (flags: continued=0x01, BOS=0x02, EOS=0x04)
//! Granule pos:     8 bytes (little-endian, codec-specific)
//! Serial number:   4 bytes (identifies logical stream)
//! Page seq no:     4 bytes
//! CRC checksum:    4 bytes
//! Num segments:    1 byte
//! Segment table:   N bytes (each 1 byte, packet sizes)
//! Page data:       sum(segment_table) bytes
//! ```
//!
//! 段表中连续的非 255 值段组合成一个完整的 packet.

use bytes::Bytes;
use log::debug;
use std::collections::HashMap;
use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

/// Ogg 同步字 (capture pattern)
const OGG_SYNC: &[u8; 4] = b"OggS";
/// Ogg CRC-32 多项式
const OGG_CRC_POLY: u32 = 0x04C11DB7;

/// 页面头部标志
const FLAG_CONTINUED: u8 = 0x01;
const FLAG_BOS: u8 = 0x02;
const FLAG_EOS: u8 = 0x04;

/// 已解析的 Ogg 页面
struct OggPage {
    /// 头部标志
    header_type: u8,
    /// 粒度位置
    granule_position: i64,
    /// 逻辑流序列号
    serial_number: u32,
    /// 页面序号 (用于乱序检测)
    page_sequence: u32,
    /// 段表
    segment_table: Vec<u8>,
    /// 页面数据
    data: Vec<u8>,
}

impl OggPage {
    /// 是否为 BOS (beginning of stream) 页面
    fn is_bos(&self) -> bool {
        self.header_type & FLAG_BOS != 0
    }

    /// 是否为 EOS (end of stream) 页面
    fn is_eos(&self) -> bool {
        self.header_type & FLAG_EOS != 0
    }

    /// 是否为续延页面 (前一个 packet 的延续)
    fn is_continued(&self) -> bool {
        self.header_type & FLAG_CONTINUED != 0
    }

    /// 从段表中提取 packet 边界
    ///
    /// 返回 (offset, length, is_complete) 列表
    fn extract_packets(&self) -> Vec<(usize, usize, bool)> {
        let mut packets = Vec::new();
        let mut offset = 0usize;
        let mut current_len = 0usize;

        for &seg_size in &self.segment_table {
            current_len += seg_size as usize;
            if seg_size < 255 {
                // packet 完成
                packets.push((offset, current_len, true));
                offset += current_len;
                current_len = 0;
            }
        }

        // 如果最后一个段是 255, 说明 packet 未完成 (跨页面)
        if current_len > 0 {
            packets.push((offset, current_len, false));
        }

        packets
    }
}

/// Ogg 逻辑流状态
struct OggLogicalStream {
    /// 序列号
    serial_number: u32,
    /// 流索引
    stream_index: usize,
    /// 编解码器 ID (用于 seek 和流特定处理)
    _codec_id: CodecId,
    /// 累积的不完整 packet 数据
    partial_packet: Vec<u8>,
    /// 正在丢弃无头续包 (缺少起始片段)
    discarding_orphan_continued: bool,
    /// 上一个粒度位置
    last_granule: i64,
    /// 上一个页面序号
    last_page_sequence: Option<u32>,
    /// 当前逻辑流是否已遇到 EOS
    ended: bool,
}

/// Ogg 解封装器
pub struct OggDemuxer {
    /// 流信息列表
    streams: Vec<Stream>,
    /// 逻辑流映射 (serial_number → index into logical_streams)
    logical_streams: Vec<OggLogicalStream>,
    /// 待发送的数据包队列
    packet_queue: Vec<Packet>,
    /// 是否已到达 EOF
    eof: bool,
    /// 容器时长 (秒)
    duration_sec: Option<f64>,
}

impl OggDemuxer {
    /// 创建 Ogg 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            logical_streams: Vec::new(),
            packet_queue: Vec::new(),
            eof: false,
            duration_sec: None,
        }))
    }

    /// 归一化 Ogg granule 值.
    ///
    /// Ogg 中负值 (常见为 -1) 表示当前页没有可用 granule 时间戳.
    /// 统一映射到框架的 NOPTS 表示, 避免被上层误判为有效 PTS.
    fn normalize_granule(granule: i64) -> i64 {
        if granule < 0 {
            tao_core::timestamp::NOPTS_VALUE
        } else {
            granule
        }
    }

    /// 计算 Ogg 页面 CRC-32
    fn ogg_crc32(data: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &byte in data {
            crc ^= u32::from(byte) << 24;
            for _ in 0..8 {
                if crc & 0x8000_0000 != 0 {
                    crc = (crc << 1) ^ OGG_CRC_POLY;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    /// 读取一个 Ogg 页面
    fn read_page(io: &mut IoContext) -> TaoResult<OggPage> {
        // 读取同步字
        let sync = io.read_tag()?;
        if &sync != OGG_SYNC {
            return Err(TaoError::InvalidData("无效的 Ogg 同步字".into()));
        }

        // 版本号 (必须为 0)
        let version = io.read_u8()?;
        if version != 0 {
            return Err(TaoError::InvalidData(format!(
                "不支持的 Ogg 版本: {}",
                version,
            )));
        }

        let header_type = io.read_u8()?;

        // 粒度位置 (8 bytes, little-endian)
        let granule_low_raw = io.read_u32_le()?;
        let granule_high_raw = io.read_u32_le()?;
        let granule_low = u64::from(granule_low_raw);
        let granule_high = u64::from(granule_high_raw);
        let granule_position = (granule_high << 32 | granule_low) as i64;

        let serial_number = io.read_u32_le()?;
        let page_sequence = io.read_u32_le()?;

        let crc = io.read_u32_le()?;
        let num_segments = io.read_u8()? as usize;

        // 读取段表
        let mut segment_table = vec![0u8; num_segments];
        io.read_exact(&mut segment_table)?;

        // 计算页面数据大小
        let data_size: usize = segment_table.iter().map(|&s| s as usize).sum();
        let mut data = vec![0u8; data_size];
        io.read_exact(&mut data)?;

        // Ogg CRC 覆盖整个页面, 其中 CRC 字段本身按 0 参与计算.
        let mut crc_page = Vec::with_capacity(27 + num_segments + data_size);
        crc_page.extend_from_slice(OGG_SYNC);
        crc_page.push(version);
        crc_page.push(header_type);
        crc_page.extend_from_slice(&granule_low_raw.to_le_bytes());
        crc_page.extend_from_slice(&granule_high_raw.to_le_bytes());
        crc_page.extend_from_slice(&serial_number.to_le_bytes());
        crc_page.extend_from_slice(&page_sequence.to_le_bytes());
        crc_page.extend_from_slice(&0u32.to_le_bytes());
        crc_page.push(num_segments as u8);
        crc_page.extend_from_slice(&segment_table);
        crc_page.extend_from_slice(&data);
        let crc_calc = Self::ogg_crc32(&crc_page);
        if crc != crc_calc {
            return Err(TaoError::InvalidData(format!(
                "Ogg 页面 CRC 校验失败: 读取=0x{crc:08X}, 计算=0x{crc_calc:08X}",
            )));
        }

        Ok(OggPage {
            header_type,
            granule_position,
            serial_number,
            page_sequence,
            segment_table,
            data,
        })
    }

    /// 尝试同步到下一个 Ogg 页面
    fn sync_to_page(io: &mut IoContext) -> TaoResult<OggPage> {
        // 尝试直接读取 (假设已对齐)
        match Self::read_page(io) {
            Ok(page) => return Ok(page),
            Err(TaoError::Eof) => return Err(TaoError::Eof),
            Err(TaoError::InvalidData(msg)) if msg.starts_with("Ogg 页面 CRC 校验失败") => {
                // CRC 失败说明坏页已被完整消费.
                // 连续跳过 CRC 坏页, 直到读取到下一个有效页面.
                loop {
                    match Self::read_page(io) {
                        Ok(page) => return Ok(page),
                        Err(TaoError::InvalidData(next_msg))
                            if next_msg.starts_with("Ogg 页面 CRC 校验失败") => {}
                        Err(TaoError::Eof) => return Err(TaoError::Eof),
                        Err(_) => break,
                    }
                }
            }
            Err(_) => {} // 不对齐, 需要重新同步
        }

        // 重新同步: 逐字节搜索 "OggS"
        let mut buf = [0u8; 4];
        io.read_exact(&mut buf)?;
        loop {
            if &buf == OGG_SYNC {
                // 找到同步字, 需要"回退" - 但我们已经读了同步字
                // 直接从版本号开始读
                let version = io.read_u8()?;
                if version != 0 {
                    // 误判, 继续搜索
                    buf = [buf[1], buf[2], buf[3], version];
                    continue;
                }
                let header_type = io.read_u8()?;
                let granule_low = io.read_u32_le()? as u64;
                let granule_high = io.read_u32_le()? as u64;
                let granule_position = (granule_high << 32 | granule_low) as i64;
                let serial_number = io.read_u32_le()?;
                let page_sequence = io.read_u32_le()?;
                let crc = io.read_u32_le()?;
                let num_segments = io.read_u8()? as usize;
                let mut segment_table = vec![0u8; num_segments];
                io.read_exact(&mut segment_table)?;
                let data_size: usize = segment_table.iter().map(|&s| s as usize).sum();
                let mut data = vec![0u8; data_size];
                io.read_exact(&mut data)?;

                // 重同步分支同样执行 CRC 校验.
                let mut crc_page = Vec::with_capacity(27 + num_segments + data_size);
                crc_page.extend_from_slice(OGG_SYNC);
                crc_page.push(version);
                crc_page.push(header_type);
                crc_page.extend_from_slice(&(granule_low as u32).to_le_bytes());
                crc_page.extend_from_slice(&(granule_high as u32).to_le_bytes());
                crc_page.extend_from_slice(&serial_number.to_le_bytes());
                crc_page.extend_from_slice(&page_sequence.to_le_bytes());
                crc_page.extend_from_slice(&0u32.to_le_bytes());
                crc_page.push(num_segments as u8);
                crc_page.extend_from_slice(&segment_table);
                crc_page.extend_from_slice(&data);
                let crc_calc = Self::ogg_crc32(&crc_page);
                if crc != crc_calc {
                    // 当前候选同步点对应坏页, 从当前位置继续搜索.
                    if io.read_exact(&mut buf).is_err() {
                        return Err(TaoError::Eof);
                    }
                    continue;
                }

                return Ok(OggPage {
                    header_type,
                    granule_position,
                    serial_number,
                    page_sequence,
                    segment_table,
                    data,
                });
            }
            // 滑动窗口
            buf = [buf[1], buf[2], buf[3], 0];
            io.read_exact(&mut buf[3..4])?;
        }
    }

    /// 从 BOS 页面的第一个 packet 识别编解码器
    fn identify_codec(packet_data: &[u8]) -> CodecId {
        if packet_data.len() >= 7 && &packet_data[1..7] == b"vorbis" {
            return CodecId::Vorbis;
        }
        if packet_data.len() >= 8 && &packet_data[0..8] == b"OpusHead" {
            return CodecId::Opus;
        }
        // 兼容旧版 FLAC-in-Ogg 映射:
        // 某些历史样本的 BOS 包只有原生 FLAC 标记 "fLaC".
        if packet_data.len() >= 4 && &packet_data[0..4] == b"fLaC" {
            return CodecId::Flac;
        }
        if packet_data.len() >= 5 && &packet_data[0..5] == b"\x7fFLAC" {
            return CodecId::Flac;
        }
        if packet_data.len() >= 7 && &packet_data[1..7] == b"theora" {
            return CodecId::Theora;
        }
        CodecId::None
    }

    /// 解析 Vorbis identification header
    fn parse_vorbis_header(data: &[u8]) -> Option<(u32, u32)> {
        // packet type (1) + "vorbis" (6) + version (4) + channels (1) + sample_rate (4)
        if data.len() < 16 || data[0] != 1 || &data[1..7] != b"vorbis" {
            return None;
        }
        let channels = u32::from(data[11]);
        let sample_rate = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        Some((sample_rate, channels))
    }

    /// 解析 Opus header
    fn parse_opus_header(data: &[u8]) -> Option<(u32, u32)> {
        // "OpusHead" (8) + version (1) + channels (1) + pre_skip (2) + sample_rate (4)
        if data.len() < 16 || &data[0..8] != b"OpusHead" {
            return None;
        }
        let channels = u32::from(data[9]);
        let sample_rate = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        Some((sample_rate, channels))
    }

    /// 处理 BOS 页面, 创建流
    fn handle_bos_page(&mut self, page: &OggPage) {
        let packets = page.extract_packets();
        if packets.is_empty() {
            return;
        }

        let (offset, length, _) = packets[0];
        let packet_data = &page.data[offset..offset + length];
        let codec_id = Self::identify_codec(packet_data);

        let stream_index = self.streams.len();
        let media_type = codec_id.media_type();

        let params = match media_type {
            MediaType::Audio => {
                let (sample_rate, channels) = match codec_id {
                    CodecId::Vorbis => Self::parse_vorbis_header(packet_data).unwrap_or((44100, 2)),
                    CodecId::Opus => Self::parse_opus_header(packet_data).unwrap_or((48000, 2)),
                    _ => (44100, 2),
                };

                let sample_format = match codec_id {
                    CodecId::Vorbis => SampleFormat::S16,
                    _ => SampleFormat::F32,
                };

                StreamParams::Audio(AudioStreamParams {
                    sample_rate,
                    channel_layout: ChannelLayout::from_channels(channels),
                    sample_format,
                    bit_rate: 0,
                    frame_size: 0,
                })
            }
            MediaType::Video => StreamParams::Video(VideoStreamParams {
                width: 0,
                height: 0,
                pixel_format: tao_core::PixelFormat::Yuv420p,
                frame_rate: Rational::new(0, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            _ => StreamParams::Other,
        };

        let time_base = match codec_id {
            CodecId::Opus => Rational::new(1, 48000),
            CodecId::Vorbis => {
                if let StreamParams::Audio(ref a) = params {
                    Rational::new(1, a.sample_rate as i32)
                } else {
                    Rational::new(1, 44100)
                }
            }
            _ => Rational::new(1, 1000),
        };

        let stream = Stream {
            index: stream_index,
            media_type,
            codec_id,
            time_base,
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: packet_data.to_vec(),
            params,
            metadata: Vec::new(),
        };

        debug!(
            "Ogg: 发现流 #{}: {} ({})",
            stream_index, codec_id, media_type,
        );

        self.streams.push(stream);
        self.logical_streams.push(OggLogicalStream {
            serial_number: page.serial_number,
            stream_index,
            _codec_id: codec_id,
            partial_packet: Vec::new(),
            discarding_orphan_continued: false,
            last_granule: tao_core::timestamp::NOPTS_VALUE,
            last_page_sequence: Some(page.page_sequence),
            ended: false,
        });
    }

    /// 查找逻辑流
    fn find_logical_stream(&self, serial: u32) -> Option<usize> {
        self.logical_streams
            .iter()
            .position(|s| s.serial_number == serial)
    }

    /// 处理非 BOS 页面, 提取数据包
    fn process_page(&mut self, page: OggPage) {
        let ls_idx = match self.find_logical_stream(page.serial_number) {
            Some(idx) => idx,
            None => return, // 未知流, 跳过
        };

        let packets = page.extract_packets();
        let packet_start_index = 0usize;
        if page.is_bos() {
            let looks_like_header = packets.first().map(|&(offset, length, complete)| {
                complete
                    && Self::identify_codec(&page.data[offset..offset + length]) != CodecId::None
            });
            if looks_like_header == Some(true) {
                // 运行中重新遇到合法 BOS 头页, 视为逻辑流重启边界.
                // 保留本页包输出给上游, 由解码器/调用方按头包进行重建处理.
                self.logical_streams[ls_idx].partial_packet.clear();
                self.logical_streams[ls_idx].discarding_orphan_continued = false;
                self.logical_streams[ls_idx].last_granule = tao_core::timestamp::NOPTS_VALUE;
                self.logical_streams[ls_idx].last_page_sequence = Some(page.page_sequence);
                self.logical_streams[ls_idx].ended = false;
            }
        }

        if self.logical_streams[ls_idx].ended {
            return;
        }

        let mut force_granule_nopts = false;
        if let Some(prev_seq) = self.logical_streams[ls_idx].last_page_sequence
            && page.page_sequence != prev_seq.wrapping_add(1)
        {
            // 页面序号断裂/回绕(非自然 +1), 标记为不可靠页:
            // 1) 清理残包状态，避免跨断点拼包;
            // 2) 本页不传播 granule，避免误导解码器裁剪策略.
            self.logical_streams[ls_idx].partial_packet.clear();
            self.logical_streams[ls_idx].discarding_orphan_continued = page.is_continued();
            force_granule_nopts = true;
        }
        self.logical_streams[ls_idx].last_page_sequence = Some(page.page_sequence);

        // 若当前页面未标记 continued, 但存在残留 partial:
        // 1) 正常情况: 上一页最后 lacing=255 且包恰好在页尾结束, 需要在此处补发.
        // 2) 异常情况: 处于 orphan continued 丢弃状态, 则直接清理残片.
        if !page.is_continued() && !self.logical_streams[ls_idx].partial_packet.is_empty() {
            if self.logical_streams[ls_idx].discarding_orphan_continued {
                debug!(
                    "Ogg: 流 #{} 结束 orphan 丢弃状态, 丢弃 {} 字节残片",
                    self.logical_streams[ls_idx].stream_index,
                    self.logical_streams[ls_idx].partial_packet.len(),
                );
                self.logical_streams[ls_idx].partial_packet.clear();
                self.logical_streams[ls_idx].discarding_orphan_continued = false;
            } else {
                let stream_idx = self.logical_streams[ls_idx].stream_index;
                let granule = self.logical_streams[ls_idx].last_granule;
                let data = std::mem::take(&mut self.logical_streams[ls_idx].partial_packet);
                debug!(
                    "Ogg: 流 #{} 检测到页边界完整包, 补发 {} 字节",
                    stream_idx,
                    data.len(),
                );
                self.emit_packet(stream_idx, granule, data);
            }
        }

        let last_complete_idx = packets.iter().rposition(|(_, _, complete)| *complete);

        for (i, &(offset, length, complete)) in packets.iter().enumerate().skip(packet_start_index)
        {
            let chunk = &page.data[offset..offset + length];

            // 如果是第一个 packet 且页面标记为 continued
            if i == 0 && page.is_continued() {
                // 没有前置残片时, 该 continued 包缺少起始数据, 需要整包丢弃.
                if self.logical_streams[ls_idx].partial_packet.is_empty() {
                    self.logical_streams[ls_idx].discarding_orphan_continued = !complete;
                    debug!(
                        "Ogg: 流 #{} 遇到无头续包, 丢弃当前片段 (len={}, complete={})",
                        self.logical_streams[ls_idx].stream_index, length, complete,
                    );
                    continue;
                }

                self.logical_streams[ls_idx]
                    .partial_packet
                    .extend_from_slice(chunk);

                if complete {
                    let data = std::mem::take(&mut self.logical_streams[ls_idx].partial_packet);
                    self.logical_streams[ls_idx].discarding_orphan_continued = false;
                    let stream_idx = self.logical_streams[ls_idx].stream_index;
                    let granule = if force_granule_nopts {
                        tao_core::timestamp::NOPTS_VALUE
                    } else if Some(i) == last_complete_idx {
                        Self::normalize_granule(page.granule_position)
                    } else {
                        tao_core::timestamp::NOPTS_VALUE
                    };
                    self.emit_packet(stream_idx, granule, data);
                }
            } else if complete {
                if self.logical_streams[ls_idx].discarding_orphan_continued {
                    // 还在丢弃缺失起始片段的续包, 直到遇到首个 complete 才恢复.
                    self.logical_streams[ls_idx].discarding_orphan_continued = false;
                    debug!(
                        "Ogg: 流 #{} 结束无头续包丢弃状态",
                        self.logical_streams[ls_idx].stream_index,
                    );
                    continue;
                }
                let stream_idx = self.logical_streams[ls_idx].stream_index;
                let granule = if force_granule_nopts {
                    tao_core::timestamp::NOPTS_VALUE
                } else if Some(i) == last_complete_idx {
                    Self::normalize_granule(page.granule_position)
                } else {
                    tao_core::timestamp::NOPTS_VALUE
                };
                self.emit_packet(stream_idx, granule, chunk.to_vec());
            } else {
                if self.logical_streams[ls_idx].discarding_orphan_continued {
                    continue;
                }
                // packet 未完成, 缓存
                self.logical_streams[ls_idx]
                    .partial_packet
                    .extend_from_slice(chunk);
            }
        }

        // 更新粒度位置
        if !force_granule_nopts && page.granule_position >= 0 {
            self.logical_streams[ls_idx].last_granule = page.granule_position;
        }

        // 检测 EOS
        if page.is_eos() {
            self.logical_streams[ls_idx].ended = true;
            debug!(
                "Ogg: 流 #{} (serial={}) 结束",
                self.logical_streams[ls_idx].stream_index, page.serial_number,
            );
        }
    }

    /// 创建并入队一个数据包
    fn emit_packet(&mut self, stream_index: usize, granule: i64, data: Vec<u8>) {
        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = stream_index;
        let granule = Self::normalize_granule(granule);
        pkt.pts = granule;
        pkt.dts = granule;
        pkt.is_keyframe = true; // Ogg 不直接提供关键帧信息

        if let Some(stream) = self.streams.get(stream_index) {
            pkt.time_base = stream.time_base;
        }

        self.packet_queue.push(pkt);
    }

    /// 重置 seek 后的运行态缓存
    fn reset_runtime_state(&mut self) {
        self.packet_queue.clear();
        self.eof = false;
        for ls in &mut self.logical_streams {
            ls.partial_packet.clear();
            ls.discarding_orphan_continued = false;
            ls.last_granule = tao_core::timestamp::NOPTS_VALUE;
            ls.last_page_sequence = None;
            ls.ended = false;
        }
    }

    /// 估算时长并回填流 duration
    ///
    /// 仅在可 seek 输入上启用. 通过扫描后续页面的 granule_position
    /// 估算每条逻辑流的末尾时间戳, 再换算为秒.
    fn estimate_duration(&mut self, io: &mut IoContext) -> TaoResult<()> {
        self.duration_sec = None;
        if !io.is_seekable() {
            return Ok(());
        }

        let resume_pos = io.position()?;
        let mut max_granule_by_serial: HashMap<u32, i64> = HashMap::new();

        loop {
            match Self::sync_to_page(io) {
                Ok(page) => {
                    if page.granule_position < 0 {
                        continue;
                    }
                    if self.find_logical_stream(page.serial_number).is_none() {
                        continue;
                    }
                    let entry = max_granule_by_serial
                        .entry(page.serial_number)
                        .or_insert(page.granule_position);
                    if page.granule_position > *entry {
                        *entry = page.granule_position;
                    }
                }
                Err(TaoError::Eof) => break,
                Err(_) => break,
            }
        }

        io.seek(std::io::SeekFrom::Start(resume_pos))?;

        for ls in &self.logical_streams {
            if let Some(max_granule) = max_granule_by_serial.get(&ls.serial_number).copied()
                && max_granule >= 0
                && let Some(stream) = self.streams.get_mut(ls.stream_index)
            {
                stream.duration = max_granule;
            }
        }

        let mut best = None::<f64>;
        for s in &self.streams {
            if s.duration > 0 && s.time_base.den > 0 {
                let sec = s.duration as f64 * s.time_base.num as f64 / s.time_base.den as f64;
                best = Some(best.map_or(sec, |v| v.max(sec)));
            }
        }
        self.duration_sec = best;

        Ok(())
    }
}

impl Demuxer for OggDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Ogg
    }

    fn name(&self) -> &str {
        "ogg"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 读取所有 BOS 页面
        loop {
            let page = Self::sync_to_page(io)?;
            if page.is_bos() {
                self.handle_bos_page(&page);
            } else {
                // 第一个非 BOS 页面 - 头部结束, 处理此页面的数据
                self.process_page(page);
                break;
            }
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData("Ogg 文件中未找到任何流".into()));
        }

        if let Err(e) = self.estimate_duration(io) {
            debug!("Ogg 时长估算失败: {}", e);
        }

        debug!("打开 Ogg: {} 个流", self.streams.len(),);

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        // 先返回队列中的数据包
        if !self.packet_queue.is_empty() {
            return Ok(self.packet_queue.remove(0));
        }

        if self.eof {
            return Err(TaoError::Eof);
        }

        // 读取页面直到有数据包可返回
        loop {
            match Self::sync_to_page(io) {
                Ok(page) => {
                    if page.is_bos() {
                        if self.find_logical_stream(page.serial_number).is_none() {
                            self.handle_bos_page(&page);
                        } else {
                            self.process_page(page);
                        }
                    } else {
                        self.process_page(page);
                    }
                    if !self.packet_queue.is_empty() {
                        return Ok(self.packet_queue.remove(0));
                    }
                }
                Err(TaoError::Eof) => {
                    self.eof = true;
                    // 返回队列中剩余的包
                    if !self.packet_queue.is_empty() {
                        return Ok(self.packet_queue.remove(0));
                    }
                    return Err(TaoError::Eof);
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn seek(
        &mut self,
        io: &mut IoContext,
        stream_index: usize,
        timestamp: i64,
        flags: SeekFlags,
    ) -> TaoResult<()> {
        if stream_index >= self.streams.len() {
            return Err(TaoError::InvalidData(format!(
                "Ogg seek 流索引无效: stream_index={stream_index}"
            )));
        }
        if flags.byte {
            return Err(TaoError::NotImplemented("Ogg 字节级 seek 尚未实现".into()));
        }
        if !io.is_seekable() {
            return Err(TaoError::Unsupported("不支持在非可寻址流上 seek".into()));
        }

        let target_serial = self
            .logical_streams
            .iter()
            .find(|s| s.stream_index == stream_index)
            .map(|s| s.serial_number)
            .ok_or_else(|| TaoError::InvalidData("Ogg seek 找不到目标逻辑流".into()))?;
        let codec_id = self.streams[stream_index].codec_id;
        let min_granule = match codec_id {
            // Vorbis/Opus 的 comment/setup 头包通常为 granule=0, seek 时跳过头包页.
            CodecId::Vorbis | CodecId::Opus => 1,
            _ => 0,
        };
        let target_granule = timestamp.max(0);

        io.seek(std::io::SeekFrom::Start(0))?;
        let mut first_non_bos: Option<u64> = None;
        let mut best_before: Option<(u64, i64)> = None;
        let mut first_after: Option<(u64, i64)> = None;

        loop {
            match Self::sync_to_page(io) {
                Ok(page) => {
                    let page_size =
                        27u64 + page.segment_table.len() as u64 + page.data.len() as u64;
                    let page_end = io.position()?;
                    let page_start = page_end.saturating_sub(page_size);

                    if !page.is_bos() && first_non_bos.is_none() {
                        first_non_bos = Some(page_start);
                    }
                    if page.serial_number != target_serial {
                        continue;
                    }
                    if page.data.is_empty() {
                        continue;
                    }
                    if page.granule_position < min_granule {
                        continue;
                    }

                    if page.granule_position <= target_granule {
                        best_before = Some((page_start, page.granule_position));
                        continue;
                    }
                    first_after = Some((page_start, page.granule_position));
                    break;
                }
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            }
        }

        let seek_offset = if flags.backward {
            best_before
                .map(|v| v.0)
                .or_else(|| first_after.map(|v| v.0))
                .or(first_non_bos)
                .unwrap_or(0)
        } else {
            first_after
                .map(|v| v.0)
                .or_else(|| best_before.map(|v| v.0))
                .or(first_non_bos)
                .unwrap_or(0)
        };

        io.seek(std::io::SeekFrom::Start(seek_offset))?;
        self.reset_runtime_state();

        debug!(
            "Ogg seek: stream={}, target={}, 定位偏移={}, backward={}",
            stream_index, target_granule, seek_offset, flags.backward
        );
        Ok(())
    }

    fn duration(&self) -> Option<f64> {
        self.duration_sec
    }
}

/// Ogg 格式探测器
pub struct OggProbe;

impl FormatProbe for OggProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // 魔数匹配
        if data.len() >= 4 && &data[0..4] == OGG_SYNC {
            return Some(crate::probe::SCORE_MAX);
        }
        // 某些文件会在 Ogg 前附带 ID3v2 标签，尝试从标签后匹配。
        if data.len() >= 14 && &data[0..3] == b"ID3" {
            let size = ((data[6] & 0x7F) as usize) << 21
                | ((data[7] & 0x7F) as usize) << 14
                | ((data[8] & 0x7F) as usize) << 7
                | (data[9] & 0x7F) as usize;
            let ogg_offset = 10 + size;
            if data.len() >= ogg_offset + 4 && &data[ogg_offset..ogg_offset + 4] == OGG_SYNC {
                return Some(crate::probe::SCORE_MAX - 2);
            }
        }

        // 扩展名匹配
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                let ext_lower = ext.to_lowercase();
                if matches!(ext_lower.as_str(), "ogg" | "ogv" | "oga" | "ogx" | "spx") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Ogg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 手动构造一个简单的 Ogg 文件 (含 Vorbis BOS 页面)
    fn build_minimal_ogg_vorbis() -> Vec<u8> {
        let mut data = Vec::new();

        // === BOS 页面 ===
        // Vorbis identification header packet:
        // type=1, "vorbis", version=0, channels=2, sample_rate=44100
        let mut vorbis_id = Vec::new();
        vorbis_id.push(1u8); // packet type
        vorbis_id.extend_from_slice(b"vorbis"); // codec id
        vorbis_id.extend_from_slice(&0u32.to_le_bytes()); // version
        vorbis_id.push(2); // channels
        vorbis_id.extend_from_slice(&44100u32.to_le_bytes()); // sample_rate
        vorbis_id.extend_from_slice(&0i32.to_le_bytes()); // bitrate_max
        vorbis_id.extend_from_slice(&128000i32.to_le_bytes()); // bitrate_nom
        vorbis_id.extend_from_slice(&0i32.to_le_bytes()); // bitrate_min
        vorbis_id.push(0x88); // blocksize info
        vorbis_id.push(1); // framing

        let bos_page = build_ogg_page(
            FLAG_BOS, 0,          // granule
            0x12345678, // serial
            0,          // page seq
            &vorbis_id,
        );
        data.extend_from_slice(&bos_page);

        // === 数据页面 (含简单 packet) ===
        let audio_data = vec![0u8; 100];
        let data_page = build_ogg_page(
            0,    // 普通页面
            1024, // granule
            0x12345678,
            1,
            &audio_data,
        );
        data.extend_from_slice(&data_page);

        // === EOS 页面 ===
        let eos_page = build_ogg_page(FLAG_EOS, 2048, 0x12345678, 2, &[]);
        data.extend_from_slice(&eos_page);

        data
    }

    /// 构造包含 Vorbis 头包页与多个音频页的 Ogg 样本, 用于 seek 单测
    fn build_vorbis_seek_test_ogg() -> Vec<u8> {
        let mut data = Vec::new();
        let serial = 0x87654321;
        let mut page_seq = 0u32;

        // identification 头包 (BOS)
        let mut vorbis_id = Vec::new();
        vorbis_id.push(1u8);
        vorbis_id.extend_from_slice(b"vorbis");
        vorbis_id.extend_from_slice(&0u32.to_le_bytes());
        vorbis_id.push(2);
        vorbis_id.extend_from_slice(&44100u32.to_le_bytes());
        vorbis_id.extend_from_slice(&0i32.to_le_bytes());
        vorbis_id.extend_from_slice(&128000i32.to_le_bytes());
        vorbis_id.extend_from_slice(&0i32.to_le_bytes());
        vorbis_id.push(0x88);
        vorbis_id.push(1);
        data.extend_from_slice(&build_ogg_page(FLAG_BOS, 0, serial, page_seq, &vorbis_id));
        page_seq += 1;

        // comment 头包页 (granule=0)
        let mut comment = Vec::new();
        comment.push(3u8);
        comment.extend_from_slice(b"vorbis");
        comment.extend_from_slice(&[0u8; 8]);
        data.extend_from_slice(&build_ogg_page(0, 0, serial, page_seq, &comment));
        page_seq += 1;

        // setup 头包页 (granule=0)
        let mut setup = Vec::new();
        setup.push(5u8);
        setup.extend_from_slice(b"vorbis");
        setup.extend_from_slice(&[0u8; 8]);
        data.extend_from_slice(&build_ogg_page(0, 0, serial, page_seq, &setup));
        page_seq += 1;

        // 音频页
        data.extend_from_slice(&build_ogg_page(0, 256, serial, page_seq, &[0x00, 0x11]));
        page_seq += 1;
        data.extend_from_slice(&build_ogg_page(0, 512, serial, page_seq, &[0x00, 0x22]));
        page_seq += 1;
        data.extend_from_slice(&build_ogg_page(0, 768, serial, page_seq, &[0x00, 0x33]));
        page_seq += 1;

        // EOS 页面
        data.extend_from_slice(&build_ogg_page(FLAG_EOS, 1024, serial, page_seq, &[]));

        data
    }

    /// 构建一个 Ogg 页面 (含正确的 CRC)
    fn build_ogg_page(
        header_type: u8,
        granule: i64,
        serial: u32,
        page_seq: u32,
        packet_data: &[u8],
    ) -> Vec<u8> {
        let mut page = Vec::new();

        // 同步字
        page.extend_from_slice(b"OggS");
        // 版本
        page.push(0);
        // 头部标志
        page.push(header_type);
        // 粒度位置 (8 bytes LE)
        page.extend_from_slice(&(granule as u64).to_le_bytes());
        // 序列号
        page.extend_from_slice(&serial.to_le_bytes());
        // 页面序号
        page.extend_from_slice(&page_seq.to_le_bytes());
        // CRC 占位 (先填 0, 稍后计算)
        let crc_offset = page.len();
        page.extend_from_slice(&0u32.to_le_bytes());

        // 段表
        let mut segments = Vec::new();
        let mut remaining = packet_data.len();
        while remaining >= 255 {
            segments.push(255u8);
            remaining -= 255;
        }
        segments.push(remaining as u8);

        page.push(segments.len() as u8);
        page.extend_from_slice(&segments);

        // 数据
        page.extend_from_slice(packet_data);

        // 计算 CRC (CRC 字段按 0 参与计算, 当前已为 0)
        let crc = OggDemuxer::ogg_crc32(&page);
        page[crc_offset..crc_offset + 4].copy_from_slice(&crc.to_le_bytes());

        page
    }

    #[test]
    fn test_probe_ogg_magic() {
        let probe = OggProbe;
        assert!(probe.probe(b"OggS", None).is_some());
        assert!(probe.probe(b"RIFF", None).is_none());
        assert_eq!(probe.format_id(), FormatId::Ogg);
    }

    #[test]
    fn test_probe_ogg_id3_prefix() {
        let probe = OggProbe;
        // ID3(size=0) + OggS
        let data = b"ID3\x04\x00\x00\x00\x00\x00\x00OggS";
        assert!(probe.probe(data, None).is_some());
    }

    #[test]
    fn test_probe_ogg_extension() {
        let probe = OggProbe;
        assert!(probe.probe(&[], Some("test.ogg")).is_some());
        assert!(probe.probe(&[], Some("test.oga")).is_some());
        assert!(probe.probe(&[], Some("test.ogv")).is_some());
        assert!(probe.probe(&[], Some("test.mp3")).is_none());
    }

    #[test]
    fn test_identify_vorbis() {
        let mut data = vec![1u8]; // packet type
        data.extend_from_slice(b"vorbis");
        assert_eq!(OggDemuxer::identify_codec(&data), CodecId::Vorbis);
    }

    #[test]
    fn test_identify_opus() {
        let data = b"OpusHead\x01\x02\x00\x00\x80\xbb\x00\x00";
        assert_eq!(OggDemuxer::identify_codec(data), CodecId::Opus);
    }

    #[test]
    fn test_identify_flac() {
        let data = b"\x7fFLAC\x01\x00";
        assert_eq!(OggDemuxer::identify_codec(data), CodecId::Flac);
    }

    #[test]
    fn test_identify_flac_legacy_ogg_packet_header() {
        let data = b"fLaC";
        assert_eq!(OggDemuxer::identify_codec(data), CodecId::Flac);
    }

    #[test]
    fn test_identify_theora() {
        let mut data = vec![0x80];
        data.extend_from_slice(b"theora");
        assert_eq!(OggDemuxer::identify_codec(&data), CodecId::Theora);
    }

    #[test]
    fn test_demux_vorbis_single_stream() {
        let ogg_data = build_minimal_ogg_vorbis();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));

        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::Vorbis);
        assert_eq!(streams[0].media_type, MediaType::Audio);

        match &streams[0].params {
            StreamParams::Audio(a) => {
                assert_eq!(a.sample_rate, 44100);
                assert_eq!(a.channel_layout.channels, 2);
            }
            _ => panic!("期望音频流参数"),
        }
    }

    #[test]
    fn test_demux_support_id3_prefix() {
        let mut data = Vec::new();
        // ID3 header + size=4 + 4 bytes payload
        data.extend_from_slice(b"ID3");
        data.extend_from_slice(&[4, 0, 0]);
        data.extend_from_slice(&[0, 0, 0, 4]);
        data.extend_from_slice(&[0u8; 4]);
        data.extend_from_slice(&build_minimal_ogg_vorbis());

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));

        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::Vorbis);
    }

    #[test]
    fn test_read_packets() {
        let ogg_data = build_minimal_ogg_vorbis();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));

        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        // 应该能读到至少一个 packet
        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.stream_index, 0);
        assert!(!pkt.data.is_empty());
    }

    #[test]
    fn test_duration_estimable() {
        let ogg_data = build_minimal_ogg_vorbis();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));

        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert!(streams[0].duration > 0, "应回填流 duration");
        let duration_sec = demuxer.duration().unwrap_or(0.0);
        assert!(
            duration_sec > 0.0,
            "Ogg demuxer 应返回可用总时长, 实际={duration_sec}"
        );
    }

    #[test]
    fn test_seek_skip_vorbis_header_packet_page() {
        let ogg_data = build_vorbis_seek_test_ogg();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        demuxer
            .seek(&mut io, 0, 0, SeekFlags::default())
            .expect("seek 到起点不应失败");
        let pkt = demuxer
            .read_packet(&mut io)
            .expect("seek 后应能读取音频数据包");
        assert_eq!(pkt.pts, 256, "seek 应跳过 granule=0 的头包页");
    }

    #[test]
    fn test_seek_backward_seek_to_page_before_target() {
        let ogg_data = build_vorbis_seek_test_ogg();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        demuxer
            .seek(&mut io, 0, 700, SeekFlags::default())
            .expect("seek backward 不应失败");
        let pkt = demuxer.read_packet(&mut io).expect("seek 后应能读取数据包");
        assert_eq!(pkt.pts, 512, "backward seek 应定位到目标之前最近页");
    }

    #[test]
    fn test_seek_forward_seek_to_page_after_target() {
        let ogg_data = build_vorbis_seek_test_ogg();
        let backend = MemoryBackend::from_data(ogg_data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = OggDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let flags = SeekFlags {
            backward: false,
            ..SeekFlags::default()
        };
        demuxer
            .seek(&mut io, 0, 700, flags)
            .expect("seek forward 不应失败");
        let pkt = demuxer.read_packet(&mut io).expect("seek 后应能读取数据包");
        assert_eq!(pkt.pts, 768, "forward seek 应定位到目标之后最近页");
    }

    #[test]
    fn test_page_extract_packets() {
        // 段表 [100, 50, 255, 200]:
        // 100 < 255 → packet 1 完成 (100 字节)
        // 50 < 255 → packet 2 完成 (50 字节)
        // 255 = 255 → 累积
        // 200 < 255 → packet 3 完成 (255+200=455 字节)
        let page = OggPage {
            header_type: 0,
            granule_position: 100,
            serial_number: 1,
            page_sequence: 0,
            segment_table: vec![100, 50, 255, 200],
            data: vec![0u8; 100 + 50 + 255 + 200],
        };
        let packets = page.extract_packets();
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0], (0, 100, true));
        assert_eq!(packets[1], (100, 50, true));
        assert_eq!(packets[2], (150, 455, true)); // 255+200, 完成

        // 段表以 255 结尾 → 最后一个 packet 未完成 (跨页面)
        let page2 = OggPage {
            header_type: 0,
            granule_position: 100,
            serial_number: 1,
            page_sequence: 0,
            segment_table: vec![100, 255],
            data: vec![0u8; 100 + 255],
        };
        let packets2 = page2.extract_packets();
        assert_eq!(packets2.len(), 2);
        assert_eq!(packets2[0], (0, 100, true));
        assert_eq!(packets2[1], (100, 255, false)); // 未完成
    }
}
