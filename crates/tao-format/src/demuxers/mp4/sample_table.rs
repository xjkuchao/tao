//! MP4 采样表 (Sample Table) 解析.
//!
//! 采样表 (stbl) 包含了从 packet 索引到文件偏移的完整映射.
//! 由多个子 box 共同组成:
//! - stsd: 采样描述 (编解码器参数)
//! - stts: 解码时间→采样编号映射 (delta 编码)
//! - stsc: 采样→块映射 (压缩的 Run-Length 编码)
//! - stsz: 每个采样的字节大小
//! - stco/co64: 每个块的文件偏移
//! - stss: 同步采样 (关键帧) 索引列表
//! - ctts: 合成时间偏移 (B帧重排序)

use tao_codec::CodecId;
use tao_core::TaoResult;

use crate::io::IoContext;

/// 时间→采样条目 (stts)
#[derive(Debug, Clone)]
struct SttsEntry {
    /// 采样计数
    count: u32,
    /// 每采样的时间增量
    delta: u32,
}

/// 采样→块条目 (stsc)
#[derive(Debug, Clone)]
struct StscEntry {
    /// 起始块号 (1-based)
    first_chunk: u32,
    /// 每块的采样数
    samples_per_chunk: u32,
    /// 采样描述索引
    _sample_desc_idx: u32,
}

/// 合成时间偏移条目 (ctts)
#[derive(Debug, Clone)]
struct CttsEntry {
    /// 采样计数
    count: u32,
    /// 偏移量
    offset: i32,
}

/// 采样表
pub struct SampleTable {
    // === 来自 stsd 的信息 ===
    /// 编解码器 ID
    pub codec_id: CodecId,
    /// 额外数据 (编解码器特定, 如 SPS/PPS)
    pub extra_data: Vec<u8>,
    /// 视频宽度
    pub width: u32,
    /// 视频高度
    pub height: u32,
    /// 音频采样率
    pub sample_rate: u32,
    /// 声道数
    pub channel_count: u32,

    // === stts ===
    /// 时间→采样表
    stts_entries: Vec<SttsEntry>,
    // === stsc ===
    /// 采样→块表
    stsc_entries: Vec<StscEntry>,
    // === stsz ===
    /// 默认采样大小 (0 表示使用逐样本大小表)
    default_sample_size: u32,
    /// 逐样本大小表
    sample_sizes: Vec<u32>,
    /// 总采样数
    total_samples: u32,
    // === stco/co64 ===
    /// 块偏移表
    chunk_offsets: Vec<u64>,
    // === stss ===
    /// 同步采样 (关键帧) 列表 (1-based)
    sync_samples: Vec<u32>,
    /// 是否有 stss (无则所有采样都是关键帧)
    has_stss: bool,
    // === ctts ===
    /// 合成时间偏移表
    ctts_entries: Vec<CttsEntry>,
}

impl SampleTable {
    /// 创建空的采样表
    pub fn new() -> Self {
        Self {
            codec_id: CodecId::None,
            extra_data: Vec::new(),
            width: 0,
            height: 0,
            sample_rate: 0,
            channel_count: 0,
            stts_entries: Vec::new(),
            stsc_entries: Vec::new(),
            default_sample_size: 0,
            sample_sizes: Vec::new(),
            total_samples: 0,
            chunk_offsets: Vec::new(),
            sync_samples: Vec::new(),
            has_stss: false,
            ctts_entries: Vec::new(),
        }
    }

    /// 获取总采样数
    pub fn sample_count(&self) -> u32 {
        if self.total_samples > 0 {
            self.total_samples
        } else {
            self.sample_sizes.len() as u32
        }
    }

    /// 获取指定采样的字节大小
    pub fn sample_size(&self, sample_idx: u32) -> u32 {
        if self.default_sample_size > 0 {
            self.default_sample_size
        } else if (sample_idx as usize) < self.sample_sizes.len() {
            self.sample_sizes[sample_idx as usize]
        } else {
            0
        }
    }

    /// 获取指定采样在文件中的偏移量
    pub fn sample_offset(&self, sample_idx: u32) -> u64 {
        // 根据 stsc 找到采样所在的块和块内偏移
        let (chunk_idx, _offset_in_chunk) = self.sample_to_chunk(sample_idx);

        let chunk_offset = if (chunk_idx as usize) < self.chunk_offsets.len() {
            self.chunk_offsets[chunk_idx as usize]
        } else {
            0
        };

        // 计算块内偏移: 累加该块内之前的采样大小
        let mut byte_offset = 0u64;
        let chunk_start_sample = self.chunk_start_sample(chunk_idx);
        for i in chunk_start_sample..sample_idx {
            byte_offset += u64::from(self.sample_size(i));
        }

        chunk_offset + byte_offset
    }

    /// 获取指定采样的 PTS
    pub fn sample_pts(&self, sample_idx: u32) -> i64 {
        let mut pts = 0i64;
        let mut remaining = sample_idx;

        for entry in &self.stts_entries {
            if remaining < entry.count {
                pts += i64::from(remaining) * i64::from(entry.delta);
                break;
            }
            pts += i64::from(entry.count) * i64::from(entry.delta);
            remaining -= entry.count;
        }

        // 加上 ctts 偏移 (如果有)
        if !self.ctts_entries.is_empty() {
            pts += i64::from(self.cts_offset(sample_idx));
        }

        pts
    }

    /// 是否为同步采样 (关键帧)
    pub fn is_sync_sample(&self, sample_idx: u32) -> bool {
        if !self.has_stss {
            return true; // 无 stss 表示所有采样都是关键帧
        }
        let sample_num = sample_idx + 1; // stss 使用 1-based
        self.sync_samples.binary_search(&sample_num).is_ok()
    }

    // === 解析方法 ===

    /// 解析 stsd (Sample Description Box)
    pub fn parse_stsd(&mut self, io: &mut IoContext, box_end: u64) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        if entry_count == 0 {
            return Ok(());
        }

        // 只解析第一个条目
        let entry_size = io.read_u32_be()?;
        let entry_format = io.read_tag()?;
        let entry_end = io.position()? - 4 + u64::from(entry_size) - 4;

        self.codec_id = fourcc_to_codec_id(&entry_format);

        // 跳过保留字段 (6 bytes) + data_reference_index (2 bytes)
        io.read_bytes(6)?;
        let _data_ref_idx = io.read_u16_be()?;

        // 根据 handler type 解析不同内容
        match self.codec_id.media_type() {
            tao_core::MediaType::Video => {
                self.parse_video_sample_entry(io, entry_end)?;
            }
            tao_core::MediaType::Audio => {
                self.parse_audio_sample_entry(io, entry_end)?;
            }
            _ => {}
        }

        // 跳到 box 末尾
        if io.position()? < box_end {
            io.seek(std::io::SeekFrom::Start(box_end))?;
        }

        Ok(())
    }

    /// 解析视频采样条目
    fn parse_video_sample_entry(&mut self, io: &mut IoContext, entry_end: u64) -> TaoResult<()> {
        let _pre_defined = io.read_u16_be()?;
        let _reserved = io.read_u16_be()?;
        io.read_bytes(12)?; // pre_defined + reserved
        self.width = u32::from(io.read_u16_be()?);
        self.height = u32::from(io.read_u16_be()?);
        let _horiz_res = io.read_u32_be()?; // 72 dpi
        let _vert_res = io.read_u32_be()?;
        let _reserved2 = io.read_u32_be()?;
        let _frame_count = io.read_u16_be()?;
        io.read_bytes(32)?; // compressor name
        let _depth = io.read_u16_be()?;
        let _pre_defined2 = io.read_i16_be()?;

        // 解析嵌套的编解码器配置 box (avcC, hvcC 等)
        self.parse_codec_config_boxes(io, entry_end)?;

        Ok(())
    }

    /// 解析音频采样条目
    fn parse_audio_sample_entry(&mut self, io: &mut IoContext, entry_end: u64) -> TaoResult<()> {
        // Audio Sample Entry (ISO 14496-12)
        let _reserved = io.read_bytes(8)?;
        self.channel_count = u32::from(io.read_u16_be()?);
        let _sample_size = io.read_u16_be()?;
        let _pre_defined = io.read_u16_be()?;
        let _reserved2 = io.read_u16_be()?;
        let sr_fixed = io.read_u32_be()?;
        self.sample_rate = sr_fixed >> 16;

        // 解析嵌套的编解码器配置 box (esds, dOps 等)
        self.parse_codec_config_boxes(io, entry_end)?;

        Ok(())
    }

    /// 解析编解码器配置子 box (avcC, hvcC, esds, dOps 等)
    fn parse_codec_config_boxes(&mut self, io: &mut IoContext, end: u64) -> TaoResult<()> {
        while io.position()? + 8 <= end {
            let pos = io.position()?;
            let size = io.read_u32_be()?;
            let tag = io.read_tag()?;

            if size < 8 {
                break;
            }
            let content_size = (size as u64) - 8;

            match &tag {
                b"esds" => {
                    let data = io.read_bytes(content_size as usize)?;
                    // 从 esds 描述符中提取 DecoderSpecificInfo (AudioSpecificConfig)
                    self.extra_data = extract_decoder_specific_info(&data).unwrap_or(data);
                }
                b"avcC" | b"hvcC" | b"av1C" | b"vpcC" | b"dOps" => {
                    let data = io.read_bytes(content_size as usize)?;
                    self.extra_data = data;
                }
                _ => {}
            }

            io.seek(std::io::SeekFrom::Start(pos + size as u64))?;
        }
        Ok(())
    }

    /// 解析 stts (Time-to-Sample Box)
    pub fn parse_stts(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        self.stts_entries.reserve(entry_count as usize);
        for _ in 0..entry_count {
            let count = io.read_u32_be()?;
            let delta = io.read_u32_be()?;
            self.stts_entries.push(SttsEntry { count, delta });
        }

        Ok(())
    }

    /// 解析 stsc (Sample-to-Chunk Box)
    pub fn parse_stsc(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        self.stsc_entries.reserve(entry_count as usize);
        for _ in 0..entry_count {
            let first_chunk = io.read_u32_be()?;
            let samples_per_chunk = io.read_u32_be()?;
            let sample_desc_idx = io.read_u32_be()?;
            self.stsc_entries.push(StscEntry {
                first_chunk,
                samples_per_chunk,
                _sample_desc_idx: sample_desc_idx,
            });
        }

        Ok(())
    }

    /// 解析 stsz (Sample Size Box)
    pub fn parse_stsz(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        self.default_sample_size = io.read_u32_be()?;
        self.total_samples = io.read_u32_be()?;

        if self.default_sample_size == 0 {
            self.sample_sizes.reserve(self.total_samples as usize);
            for _ in 0..self.total_samples {
                self.sample_sizes.push(io.read_u32_be()?);
            }
        }

        Ok(())
    }

    /// 解析 stco/co64 (Chunk Offset Box)
    pub fn parse_stco(&mut self, io: &mut IoContext, is_64bit: bool) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        self.chunk_offsets.reserve(entry_count as usize);
        for _ in 0..entry_count {
            let offset = if is_64bit {
                let hi = io.read_u32_be()? as u64;
                let lo = io.read_u32_be()? as u64;
                (hi << 32) | lo
            } else {
                u64::from(io.read_u32_be()?)
            };
            self.chunk_offsets.push(offset);
        }

        Ok(())
    }

    /// 解析 stss (Sync Sample Box)
    pub fn parse_stss(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        self.has_stss = true;
        self.sync_samples.reserve(entry_count as usize);
        for _ in 0..entry_count {
            self.sync_samples.push(io.read_u32_be()?);
        }

        Ok(())
    }

    /// 解析 ctts (Composition Time-to-Sample Box)
    pub fn parse_ctts(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let entry_count = io.read_u32_be()?;

        self.ctts_entries.reserve(entry_count as usize);
        for _ in 0..entry_count {
            let count = io.read_u32_be()?;
            let offset = if version == 0 {
                io.read_u32_be()? as i32
            } else {
                io.read_i32_be()?
            };
            self.ctts_entries.push(CttsEntry { count, offset });
        }

        Ok(())
    }

    // === 内部辅助方法 ===

    /// 从采样索引查找所在的块号和块内偏移
    fn sample_to_chunk(&self, sample_idx: u32) -> (u32, u32) {
        if self.stsc_entries.is_empty() || self.chunk_offsets.is_empty() {
            return (0, 0);
        }

        let total_chunks = self.chunk_offsets.len() as u32;
        let mut sample_count = 0u32;

        for (i, entry) in self.stsc_entries.iter().enumerate() {
            let first_chunk = entry.first_chunk - 1; // 转为 0-based
            let next_first = if i + 1 < self.stsc_entries.len() {
                self.stsc_entries[i + 1].first_chunk - 1
            } else {
                total_chunks
            };

            let chunks_in_run = next_first - first_chunk;
            let samples_in_run = chunks_in_run * entry.samples_per_chunk;

            if sample_idx < sample_count + samples_in_run {
                let offset = sample_idx - sample_count;
                let chunk_in_run = offset / entry.samples_per_chunk;
                let sample_in_chunk = offset % entry.samples_per_chunk;
                return (first_chunk + chunk_in_run, sample_in_chunk);
            }

            sample_count += samples_in_run;
        }

        (0, 0)
    }

    /// 获取指定块的起始采样索引
    fn chunk_start_sample(&self, chunk_idx: u32) -> u32 {
        if self.stsc_entries.is_empty() {
            return 0;
        }

        let total_chunks = self.chunk_offsets.len() as u32;
        let mut sample_count = 0u32;

        for (i, entry) in self.stsc_entries.iter().enumerate() {
            let first_chunk = entry.first_chunk - 1;
            let next_first = if i + 1 < self.stsc_entries.len() {
                self.stsc_entries[i + 1].first_chunk - 1
            } else {
                total_chunks
            };

            if chunk_idx < next_first {
                let chunks_before = chunk_idx - first_chunk;
                return sample_count + chunks_before * entry.samples_per_chunk;
            }

            let chunks_in_run = next_first - first_chunk;
            sample_count += chunks_in_run * entry.samples_per_chunk;
        }

        sample_count
    }

    /// 获取 ctts 偏移
    fn cts_offset(&self, sample_idx: u32) -> i32 {
        let mut remaining = sample_idx;
        for entry in &self.ctts_entries {
            if remaining < entry.count {
                return entry.offset;
            }
            remaining -= entry.count;
        }
        0
    }
}

/// 从 esds box 内容中提取 DecoderSpecificInfo (AudioSpecificConfig)
///
/// esds 结构: version(1) + flags(3) + ES_Descriptor(tag=0x03)
///   → DecoderConfigDescriptor(tag=0x04)
///     → DecoderSpecificInfo(tag=0x05) = AudioSpecificConfig
fn extract_decoder_specific_info(esds_data: &[u8]) -> Option<Vec<u8>> {
    if esds_data.len() < 4 {
        return None;
    }
    // 跳过 version(1) + flags(3)
    search_descriptor(&esds_data[4..], 0x05)
}

/// 在 MPEG-4 描述符数据中递归搜索指定 tag 的 payload
fn search_descriptor(data: &[u8], target_tag: u8) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos < data.len() {
        let tag = data[pos];
        pos += 1;

        // 读取可变长度 (每字节高位为续标志, 低 7 位为值)
        let mut len = 0usize;
        for _ in 0..4 {
            if pos >= data.len() {
                return None;
            }
            let b = data[pos];
            pos += 1;
            len = (len << 7) | (b & 0x7F) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }

        let desc_end = (pos + len).min(data.len());
        if tag == target_tag {
            return Some(data[pos..desc_end].to_vec());
        }

        // 跳过当前描述符的固定头部, 递归搜索子描述符
        let header_skip = descriptor_header_size(tag, &data[pos..desc_end]);
        let child_start = (pos + header_skip).min(desc_end);
        if child_start < desc_end {
            if let Some(result) = search_descriptor(&data[child_start..desc_end], target_tag) {
                return Some(result);
            }
        }

        pos = desc_end;
    }
    None
}

/// 获取 MPEG-4 描述符固定头部大小
fn descriptor_header_size(tag: u8, payload: &[u8]) -> usize {
    match tag {
        0x03 => {
            // ES_Descriptor: ES_ID(2) + flags(1) + 可选字段
            if payload.len() < 3 {
                return payload.len();
            }
            let flags = payload[2];
            let mut skip = 3;
            if flags & 0x80 != 0 {
                skip += 2; // dependsOn_ES_ID
            }
            if flags & 0x40 != 0 {
                // URL
                if skip < payload.len() {
                    skip += 1 + payload[skip] as usize;
                }
            }
            if flags & 0x20 != 0 {
                skip += 2; // OCR_ES_Id
            }
            skip
        }
        0x04 => 13, // DecoderConfigDescriptor: objectType(1)+stream(1)+buf(3)+max(4)+avg(4)
        _ => 0,
    }
}

/// FourCC 到 CodecId 映射
fn fourcc_to_codec_id(fourcc: &[u8; 4]) -> CodecId {
    match fourcc {
        // 视频
        b"avc1" | b"avc3" | b"h264" => CodecId::H264,
        b"hvc1" | b"hev1" => CodecId::H265,
        b"vp08" => CodecId::Vp8,
        b"vp09" => CodecId::Vp9,
        b"av01" => CodecId::Av1,
        b"mp4v" => CodecId::Mpeg4,
        b"mjpa" | b"mjpb" => CodecId::Mjpeg,
        // 音频
        b"mp4a" => CodecId::Aac,
        b"Opus" => CodecId::Opus,
        b"fLaC" => CodecId::Flac,
        b"alac" => CodecId::Alac,
        b"ac-3" => CodecId::Ac3,
        b"ec-3" => CodecId::Eac3,
        b".mp3" => CodecId::Mp3,
        // 未知
        _ => CodecId::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    #[test]
    fn test_fourcc_映射() {
        assert_eq!(fourcc_to_codec_id(b"avc1"), CodecId::H264);
        assert_eq!(fourcc_to_codec_id(b"hvc1"), CodecId::H265);
        assert_eq!(fourcc_to_codec_id(b"mp4a"), CodecId::Aac);
        assert_eq!(fourcc_to_codec_id(b"Opus"), CodecId::Opus);
        assert_eq!(fourcc_to_codec_id(b"fLaC"), CodecId::Flac);
        assert_eq!(fourcc_to_codec_id(b"xxxx"), CodecId::None);
    }

    #[test]
    fn test_stts_解析() {
        let mut data = Vec::new();
        data.push(0); // version
        data.extend_from_slice(&[0, 0, 0]); // flags
        data.extend_from_slice(&2u32.to_be_bytes()); // entry_count
        data.extend_from_slice(&100u32.to_be_bytes()); // count
        data.extend_from_slice(&1024u32.to_be_bytes()); // delta
        data.extend_from_slice(&50u32.to_be_bytes()); // count
        data.extend_from_slice(&512u32.to_be_bytes()); // delta

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut st = SampleTable::new();
        st.parse_stts(&mut io).unwrap();

        assert_eq!(st.stts_entries.len(), 2);
        // 采样 0: PTS=0, 采样 99: PTS=99*1024
        assert_eq!(st.sample_pts(0), 0);
        assert_eq!(st.sample_pts(99), 99 * 1024);
        // 采样 100: PTS=100*1024+0*512
        assert_eq!(st.sample_pts(100), 100 * 1024);
        assert_eq!(st.sample_pts(101), 100 * 1024 + 512);
    }

    #[test]
    fn test_stsz_解析() {
        let mut data = Vec::new();
        data.push(0); // version
        data.extend_from_slice(&[0, 0, 0]); // flags
        data.extend_from_slice(&0u32.to_be_bytes()); // default_size = 0
        data.extend_from_slice(&3u32.to_be_bytes()); // sample_count
        data.extend_from_slice(&100u32.to_be_bytes()); // size[0]
        data.extend_from_slice(&200u32.to_be_bytes()); // size[1]
        data.extend_from_slice(&150u32.to_be_bytes()); // size[2]

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut st = SampleTable::new();
        st.parse_stsz(&mut io).unwrap();

        assert_eq!(st.sample_count(), 3);
        assert_eq!(st.sample_size(0), 100);
        assert_eq!(st.sample_size(1), 200);
        assert_eq!(st.sample_size(2), 150);
    }

    #[test]
    fn test_stsz_统一大小() {
        let mut data = Vec::new();
        data.push(0);
        data.extend_from_slice(&[0, 0, 0]);
        data.extend_from_slice(&1024u32.to_be_bytes()); // default_size
        data.extend_from_slice(&500u32.to_be_bytes()); // sample_count

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut st = SampleTable::new();
        st.parse_stsz(&mut io).unwrap();

        assert_eq!(st.sample_count(), 500);
        assert_eq!(st.sample_size(0), 1024);
        assert_eq!(st.sample_size(499), 1024);
    }

    #[test]
    fn test_stss_解析() {
        let mut data = Vec::new();
        data.push(0);
        data.extend_from_slice(&[0, 0, 0]);
        data.extend_from_slice(&3u32.to_be_bytes()); // entry_count
        data.extend_from_slice(&1u32.to_be_bytes()); // sample 1 (1-based)
        data.extend_from_slice(&30u32.to_be_bytes()); // sample 30
        data.extend_from_slice(&60u32.to_be_bytes()); // sample 60

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut st = SampleTable::new();
        st.parse_stss(&mut io).unwrap();

        assert!(st.is_sync_sample(0)); // sample 1 (0-based → 1-based=1)
        assert!(!st.is_sync_sample(1)); // sample 2 不是关键帧
        assert!(st.is_sync_sample(29)); // sample 30
        assert!(st.is_sync_sample(59)); // sample 60
    }

    #[test]
    fn test_无stss_所有帧都是关键帧() {
        let st = SampleTable::new();
        assert!(st.is_sync_sample(0));
        assert!(st.is_sync_sample(100));
    }

    #[test]
    fn test_sample_to_chunk() {
        let mut st = SampleTable::new();
        // stsc: 块1开始每块2个采样, 块3开始每块1个采样
        st.stsc_entries = vec![
            StscEntry {
                first_chunk: 1,
                samples_per_chunk: 2,
                _sample_desc_idx: 1,
            },
            StscEntry {
                first_chunk: 3,
                samples_per_chunk: 1,
                _sample_desc_idx: 1,
            },
        ];
        st.chunk_offsets = vec![1000, 2000, 3000, 4000];

        // 块0: 采样0,1 (2个), 块1: 采样2,3 (2个), 块2: 采样4 (1个), 块3: 采样5 (1个)
        assert_eq!(st.sample_to_chunk(0), (0, 0)); // 块0, 第0个
        assert_eq!(st.sample_to_chunk(1), (0, 1)); // 块0, 第1个
        assert_eq!(st.sample_to_chunk(2), (1, 0)); // 块1, 第0个
        assert_eq!(st.sample_to_chunk(3), (1, 1)); // 块1, 第1个
        assert_eq!(st.sample_to_chunk(4), (2, 0)); // 块2, 第0个
        assert_eq!(st.sample_to_chunk(5), (3, 0)); // 块3, 第0个
    }
}
