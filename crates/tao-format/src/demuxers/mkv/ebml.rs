//! EBML (Extensible Binary Meta Language) 基础解析.
//!
//! EBML 使用变长整数编码元素 ID 和数据大小.
//!
//! # 变长整数 (VINT)
//! ```text
//! 首字节的前导 1 位之前的 0 的数量决定了字节长度:
//!   1xxxxxxx                  → 1 字节 (7 位数据)
//!   01xxxxxx xxxxxxxx         → 2 字节 (14 位数据)
//!   001xxxxx xxxxxxxx ×2      → 3 字节 (21 位数据)
//!   0001xxxx xxxxxxxx ×3      → 4 字节 (28 位数据)
//!   ...依此类推, 最多 8 字节
//! ```

use tao_core::{TaoError, TaoResult};

use crate::io::IoContext;

/// EBML 变长整数的最大值 (表示"未知大小")
pub const EBML_UNKNOWN_SIZE: u64 = u64::MAX;

/// 读取 EBML 变长整数 (用于元素 ID)
///
/// 元素 ID 保留前导位, 不做掩码处理.
pub fn read_element_id(io: &mut IoContext) -> TaoResult<(u32, u8)> {
    let first = io.read_u8()?;
    if first == 0 {
        return Err(TaoError::InvalidData("EBML: 无效的元素 ID".into()));
    }

    let len = first.leading_zeros() as u8 + 1;
    if len > 4 {
        return Err(TaoError::InvalidData("EBML: 元素 ID 超过 4 字节".into()));
    }

    let mut id = u32::from(first);
    for _ in 1..len {
        id = (id << 8) | u32::from(io.read_u8()?);
    }

    Ok((id, len))
}

/// 读取 EBML 变长整数 (用于数据大小)
///
/// 数据大小会掩掉前导标记位, 只保留纯数值.
/// 如果所有数据位都为 1, 表示"未知大小".
pub fn read_element_size(io: &mut IoContext) -> TaoResult<(u64, u8)> {
    let first = io.read_u8()?;
    if first == 0 {
        return Err(TaoError::InvalidData("EBML: 无效的元素大小".into()));
    }

    let len = first.leading_zeros() as u8 + 1;
    if len > 8 {
        return Err(TaoError::InvalidData("EBML: 元素大小超过 8 字节".into()));
    }

    // 掩掉前导标记位
    let mask = 0xFFu16 >> len;
    let mut size = u64::from(first & mask as u8);
    for _ in 1..len {
        size = (size << 8) | u64::from(io.read_u8()?);
    }

    // 检查是否为"未知大小" (所有数据位为 1)
    let max_val = (1u64 << (7 * len)) - 1;
    if size == max_val {
        return Ok((EBML_UNKNOWN_SIZE, len));
    }

    Ok((size, len))
}

/// 读取一个 EBML 元素头部 (ID + 大小)
///
/// 返回 (元素ID, 数据大小, 头部总字节数)
pub fn read_element_header(io: &mut IoContext) -> TaoResult<(u32, u64, u8)> {
    let (id, id_len) = read_element_id(io)?;
    let (size, size_len) = read_element_size(io)?;
    Ok((id, size, id_len + size_len))
}

/// 读取无符号整数 (大端, 1-8 字节)
pub fn read_uint(io: &mut IoContext, size: u64) -> TaoResult<u64> {
    if size == 0 || size > 8 {
        return Err(TaoError::InvalidData(format!(
            "EBML: 无效的 uint 大小: {size}"
        )));
    }
    let mut val = 0u64;
    for _ in 0..size {
        val = (val << 8) | u64::from(io.read_u8()?);
    }
    Ok(val)
}

/// 读取有符号整数 (大端, 符号扩展, 1-8 字节)
pub fn read_sint(io: &mut IoContext, size: u64) -> TaoResult<i64> {
    let u = read_uint(io, size)?;
    // 符号扩展
    let bits = size * 8;
    let mask = 1u64 << (bits - 1);
    if u & mask != 0 {
        // 负数: 符号扩展
        Ok((u | !((1u64 << bits) - 1)) as i64)
    } else {
        Ok(u as i64)
    }
}

/// 读取浮点数 (4 或 8 字节)
pub fn read_float(io: &mut IoContext, size: u64) -> TaoResult<f64> {
    match size {
        0 => Ok(0.0),
        4 => {
            let bits = read_uint(io, 4)? as u32;
            Ok(f64::from(f32::from_bits(bits)))
        }
        8 => {
            let bits = read_uint(io, 8)?;
            Ok(f64::from_bits(bits))
        }
        _ => Err(TaoError::InvalidData(format!(
            "EBML: 无效的浮点数大小: {size}"
        ))),
    }
}

/// 读取 UTF-8 字符串
pub fn read_string(io: &mut IoContext, size: u64) -> TaoResult<String> {
    let data = io.read_bytes(size as usize)?;
    // 去除尾部的 NUL 字符
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    Ok(String::from_utf8_lossy(&data[..end]).to_string())
}

/// 读取二进制数据
pub fn read_binary(io: &mut IoContext, size: u64) -> TaoResult<Vec<u8>> {
    io.read_bytes(size as usize)
}

// ========================
// 已知的 Matroska 元素 ID
// ========================

// EBML Header
pub const EBML_HEADER: u32 = 0x1A45_DFA3;
pub const EBML_DOC_TYPE: u32 = 0x4282;

// Segment
pub const SEGMENT: u32 = 0x1853_8067;

// Segment Information
pub const SEGMENT_INFO: u32 = 0x1549_A966;
pub const INFO_TIMESCALE: u32 = 0x002A_D7B1;
pub const INFO_DURATION: u32 = 0x4489;
pub const INFO_TITLE: u32 = 0x7BA9;
pub const INFO_MUXING_APP: u32 = 0x4D80;
pub const INFO_WRITING_APP: u32 = 0x5741;

// Tracks
pub const TRACKS: u32 = 0x1654_AE6B;
pub const TRACK_ENTRY: u32 = 0xAE;
pub const TRACK_NUMBER: u32 = 0xD7;
pub const TRACK_UID: u32 = 0x73C5;
pub const TRACK_TYPE: u32 = 0x83;
pub const TRACK_CODEC_ID: u32 = 0x86;
pub const TRACK_CODEC_PRIVATE: u32 = 0x63A2;
pub const TRACK_DEFAULT_DURATION: u32 = 0x0023_E383;

// Video settings
pub const VIDEO_SETTINGS: u32 = 0xE0;
pub const VIDEO_PIXEL_WIDTH: u32 = 0xB0;
pub const VIDEO_PIXEL_HEIGHT: u32 = 0xBA;
pub const VIDEO_DISPLAY_WIDTH: u32 = 0x54B0;
pub const VIDEO_DISPLAY_HEIGHT: u32 = 0x54BA;

// Audio settings
pub const AUDIO_SETTINGS: u32 = 0xE1;
pub const AUDIO_SAMPLING_FREQ: u32 = 0xB5;
pub const AUDIO_CHANNELS: u32 = 0x9F;
pub const AUDIO_BIT_DEPTH: u32 = 0x6264;

// Cluster
pub const CLUSTER: u32 = 0x1F43_B675;
pub const CLUSTER_TIMESTAMP: u32 = 0xE7;
pub const SIMPLE_BLOCK: u32 = 0xA3;
pub const BLOCK_GROUP: u32 = 0xA0;
pub const BLOCK: u32 = 0xA1;

// Cues (索引, 暂不解析)
pub const CUES: u32 = 0x1C53_BB6B;

// SeekHead
pub const SEEK_HEAD: u32 = 0x114D_9B74;

// Tags
pub const TAGS: u32 = 0x1254_C367;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    #[test]
    fn test_read_1_byte_vint_id() {
        // 0x81 = 1000_0001 → ID=0x81 (1 byte)
        let backend = MemoryBackend::from_data(vec![0x81]);
        let mut io = IoContext::new(Box::new(backend));
        let (id, len) = read_element_id(&mut io).unwrap();
        assert_eq!(id, 0x81);
        assert_eq!(len, 1);
    }

    #[test]
    fn test_read_2_byte_vint_id() {
        // 0x42, 0x86 → ID=0x4286 (2 bytes)
        let backend = MemoryBackend::from_data(vec![0x42, 0x86]);
        let mut io = IoContext::new(Box::new(backend));
        let (id, len) = read_element_id(&mut io).unwrap();
        assert_eq!(id, 0x4286);
        assert_eq!(len, 2);
    }

    #[test]
    fn test_read_4_byte_vint_id() {
        // EBML Header ID: 0x1A45DFA3
        let backend = MemoryBackend::from_data(vec![0x1A, 0x45, 0xDF, 0xA3]);
        let mut io = IoContext::new(Box::new(backend));
        let (id, len) = read_element_id(&mut io).unwrap();
        assert_eq!(id, EBML_HEADER);
        assert_eq!(len, 4);
    }

    #[test]
    fn test_read_vint_size() {
        // 0x85 → size = 0x05 (1 byte, 掩掉标记位 0x80)
        let backend = MemoryBackend::from_data(vec![0x85]);
        let mut io = IoContext::new(Box::new(backend));
        let (size, len) = read_element_size(&mut io).unwrap();
        assert_eq!(size, 5);
        assert_eq!(len, 1);
    }

    #[test]
    fn test_read_2_byte_vint_size() {
        // 0x40, 0x20 → size = 0x0020 = 32 (2 bytes)
        let backend = MemoryBackend::from_data(vec![0x40, 0x20]);
        let mut io = IoContext::new(Box::new(backend));
        let (size, len) = read_element_size(&mut io).unwrap();
        assert_eq!(size, 32);
        assert_eq!(len, 2);
    }

    #[test]
    fn test_unknown_size() {
        // 0xFF → 所有 7 位数据位为 1 → 未知大小
        let backend = MemoryBackend::from_data(vec![0xFF]);
        let mut io = IoContext::new(Box::new(backend));
        let (size, _) = read_element_size(&mut io).unwrap();
        assert_eq!(size, EBML_UNKNOWN_SIZE);
    }

    #[test]
    fn test_read_uint() {
        // 2 字节 uint = 0x0100 = 256
        let backend = MemoryBackend::from_data(vec![0x01, 0x00]);
        let mut io = IoContext::new(Box::new(backend));
        assert_eq!(read_uint(&mut io, 2).unwrap(), 256);
    }

    #[test]
    fn test_read_float_4_byte() {
        // f32: 1.0 = 0x3F800000
        let bits = 1.0f32.to_bits();
        let data = bits.to_be_bytes().to_vec();
        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let val = read_float(&mut io, 4).unwrap();
        assert!((val - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_read_string() {
        let data = b"hello\x00\x00".to_vec();
        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let s = read_string(&mut io, 7).unwrap();
        assert_eq!(s, "hello");
    }
}
