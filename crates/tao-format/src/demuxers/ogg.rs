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
use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

/// Ogg 同步字 (capture pattern)
const OGG_SYNC: &[u8; 4] = b"OggS";

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
    _page_sequence: u32,
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
    /// 上一个粒度位置
    last_granule: i64,
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
}

impl OggDemuxer {
    /// 创建 Ogg 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            logical_streams: Vec::new(),
            packet_queue: Vec::new(),
            eof: false,
        }))
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
        let granule_low = io.read_u32_le()? as u64;
        let granule_high = io.read_u32_le()? as u64;
        let granule_position = (granule_high << 32 | granule_low) as i64;

        let serial_number = io.read_u32_le()?;
        let page_sequence = io.read_u32_le()?;

        let _crc = io.read_u32_le()?; // CRC (暂不校验)
        let num_segments = io.read_u8()? as usize;

        // 读取段表
        let mut segment_table = vec![0u8; num_segments];
        io.read_exact(&mut segment_table)?;

        // 计算页面数据大小
        let data_size: usize = segment_table.iter().map(|&s| s as usize).sum();
        let mut data = vec![0u8; data_size];
        io.read_exact(&mut data)?;

        Ok(OggPage {
            header_type,
            granule_position,
            serial_number,
            _page_sequence: page_sequence,
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
                let _crc = io.read_u32_le()?;
                let num_segments = io.read_u8()? as usize;
                let mut segment_table = vec![0u8; num_segments];
                io.read_exact(&mut segment_table)?;
                let data_size: usize = segment_table.iter().map(|&s| s as usize).sum();
                let mut data = vec![0u8; data_size];
                io.read_exact(&mut data)?;
                return Ok(OggPage {
                    header_type,
                    granule_position,
                    serial_number,
                    _page_sequence: page_sequence,
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

                StreamParams::Audio(AudioStreamParams {
                    sample_rate,
                    channel_layout: ChannelLayout::from_channels(channels),
                    sample_format: SampleFormat::F32,
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
            last_granule: -1,
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

        for (i, &(offset, length, complete)) in packets.iter().enumerate() {
            let chunk = &page.data[offset..offset + length];

            // 如果是第一个 packet 且页面标记为 continued
            if i == 0 && page.is_continued() {
                self.logical_streams[ls_idx]
                    .partial_packet
                    .extend_from_slice(chunk);

                if complete {
                    let data = std::mem::take(&mut self.logical_streams[ls_idx].partial_packet);
                    let stream_idx = self.logical_streams[ls_idx].stream_index;
                    self.emit_packet(stream_idx, page.granule_position, data);
                }
            } else if complete {
                let stream_idx = self.logical_streams[ls_idx].stream_index;
                self.emit_packet(stream_idx, page.granule_position, chunk.to_vec());
            } else {
                // packet 未完成, 缓存
                self.logical_streams[ls_idx]
                    .partial_packet
                    .extend_from_slice(chunk);
            }
        }

        // 更新粒度位置
        if page.granule_position >= 0 {
            self.logical_streams[ls_idx].last_granule = page.granule_position;
        }

        // 检测 EOS
        if page.is_eos() {
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
        pkt.pts = granule;
        pkt.dts = granule;
        pkt.is_keyframe = true; // Ogg 不直接提供关键帧信息

        if let Some(stream) = self.streams.get(stream_index) {
            pkt.time_base = stream.time_base;
        }

        self.packet_queue.push(pkt);
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
            let page = Self::read_page(io)?;
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
                    if page.is_eos() {
                        self.process_page(page);
                        // 检查是否所有流都已结束
                        // 先尝试返回队列中的包
                        if !self.packet_queue.is_empty() {
                            return Ok(self.packet_queue.remove(0));
                        }
                        self.eof = true;
                        return Err(TaoError::Eof);
                    }
                    self.process_page(page);
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
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        // TODO: 实现 Ogg seek
        Err(TaoError::NotImplemented("Ogg seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        None
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

    /// 构建一个 Ogg 页面
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
        // CRC (暂时填 0)
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

        page
    }

    #[test]
    fn test_探测_ogg_魔数() {
        let probe = OggProbe;
        assert!(probe.probe(b"OggS", None).is_some());
        assert!(probe.probe(b"RIFF", None).is_none());
        assert_eq!(probe.format_id(), FormatId::Ogg);
    }

    #[test]
    fn test_探测_ogg_扩展名() {
        let probe = OggProbe;
        assert!(probe.probe(&[], Some("test.ogg")).is_some());
        assert!(probe.probe(&[], Some("test.oga")).is_some());
        assert!(probe.probe(&[], Some("test.ogv")).is_some());
        assert!(probe.probe(&[], Some("test.mp3")).is_none());
    }

    #[test]
    fn test_识别_vorbis() {
        let mut data = vec![1u8]; // packet type
        data.extend_from_slice(b"vorbis");
        assert_eq!(OggDemuxer::identify_codec(&data), CodecId::Vorbis);
    }

    #[test]
    fn test_识别_opus() {
        let data = b"OpusHead\x01\x02\x00\x00\x80\xbb\x00\x00";
        assert_eq!(OggDemuxer::identify_codec(data), CodecId::Opus);
    }

    #[test]
    fn test_识别_flac() {
        let data = b"\x7fFLAC\x01\x00";
        assert_eq!(OggDemuxer::identify_codec(data), CodecId::Flac);
    }

    #[test]
    fn test_识别_theora() {
        let mut data = vec![0x80];
        data.extend_from_slice(b"theora");
        assert_eq!(OggDemuxer::identify_codec(&data), CodecId::Theora);
    }

    #[test]
    fn test_解封装_vorbis_单流() {
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
    fn test_读取数据包() {
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
    fn test_页面提取_packets() {
        // 段表 [100, 50, 255, 200]:
        // 100 < 255 → packet 1 完成 (100 字节)
        // 50 < 255 → packet 2 完成 (50 字节)
        // 255 = 255 → 累积
        // 200 < 255 → packet 3 完成 (255+200=455 字节)
        let page = OggPage {
            header_type: 0,
            granule_position: 100,
            serial_number: 1,
            _page_sequence: 0,
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
            _page_sequence: 0,
            segment_table: vec![100, 255],
            data: vec![0u8; 100 + 255],
        };
        let packets2 = page2.extract_packets();
        assert_eq!(packets2.len(), 2);
        assert_eq!(packets2[0], (0, 100, true));
        assert_eq!(packets2[1], (100, 255, false)); // 未完成
    }
}
