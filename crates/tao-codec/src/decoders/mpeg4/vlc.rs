//! VLC (变长编码) 表定义与解码函数
//!
//! 包含 MCBPC, CBPY, Intra DC, MVD, AC 系数的 VLC 表和解码逻辑.

use std::sync::OnceLock;

use log::warn;

use super::bitreader::BitReader;
use super::tables::*;
use super::types::{BframeMbMode, MbType};

// ============================================================================
// VLC 表定义
// ============================================================================

/// Intra DC VLC 表 (Y 亮度通道)
/// 格式: (位数, 码字, dc_size)
pub(super) const INTRA_DC_VLC_Y: &[(u8, u16, i16)] = &[
    (3, 0b011, 0),
    (2, 0b11, 1),
    (2, 0b10, 2),
    (3, 0b010, 3),
    (3, 0b001, 4),
    (4, 0b0001, 5),
    (5, 0b00001, 6),
    (6, 0b000001, 7),
    (7, 0b0000001, 8),
    (8, 0b00000001, 9),
    (9, 0b000000001, 10),
    (10, 0b0000000001, 11),
    (11, 0b00000000001, 12),
];

/// Intra DC VLC 表 (UV 色度通道)
pub(super) const INTRA_DC_VLC_UV: &[(u8, u16, i16)] = &[
    (2, 0b11, 0),
    (2, 0b10, 1),
    (2, 0b01, 2),
    (3, 0b001, 3),
    (4, 0b0001, 4),
    (5, 0b00001, 5),
    (6, 0b000001, 6),
    (7, 0b0000001, 7),
    (8, 0b00000001, 8),
    (9, 0b000000001, 9),
    (10, 0b0000000001, 10),
    (11, 0b00000000001, 11),
    (12, 0b000000000001, 12),
];

/// MPEG-4 Intra AC VLC 表
/// 格式: (位数, 码字, last, run, level)
pub(super) const INTRA_AC_VLC: &[(u8, u16, bool, u8, i8)] = &[
    (2, 0x3, true, 0, 0),  // EOB
    (3, 0x3, false, 0, 1), // last=0
    (4, 0x3, false, 1, 1),
    (5, 0x3, false, 0, 2),
    (5, 0x5, false, 2, 1),
    (5, 0x4, false, 3, 1),
    (6, 0x6, false, 1, 2),
    (6, 0x9, false, 4, 1),
    (6, 0x8, false, 5, 1),
    (6, 0x7, false, 6, 1),
    (7, 0xB, false, 7, 1),
    (7, 0xA, false, 8, 1),
    (8, 0x13, false, 9, 1),
    (8, 0x12, false, 10, 1),
    (4, 0x2, true, 0, 1), // last=1
    (6, 0x5, true, 1, 1),
    (7, 0x9, true, 2, 1),
    (8, 0xD, true, 3, 1),
    (6, 0xB, true, 0, 2),
    (9, 0x17, true, 1, 2),
    (8, 0xC, true, 0, 3),
    (7, 0x3, false, 0, 0), // Escape
];

/// MCBPC VLC 表 (I-VOP)
const MCBPC_I: &[(u8, u16, u8, u8)] = &[
    (1, 0b1, 0, 0),
    (3, 0b001, 0, 1),
    (3, 0b010, 0, 2),
    (3, 0b011, 0, 3),
    (4, 0b0001, 1, 0),
    (6, 0b000001, 1, 1),
    (6, 0b000010, 1, 2),
    (6, 0b000011, 1, 3),
    (9, 0b000000001, 255, 0),
];

/// MCBPC VLC 表 (P-VOP)
const MCBPC_P: &[(u8, u16, u8, u8)] = &[
    (1, 1, 0, 0),
    (3, 0b001, 0, 1),
    (3, 0b010, 0, 2),
    (3, 0b011, 0, 3),
    (4, 0b0001, 1, 0),
    (5, 0b00001, 1, 1),
    (5, 0b00000, 1, 2),
    (6, 0b000110, 1, 3),
    (6, 0b000111, 3, 0),
    (7, 0b0001000, 3, 1),
    (7, 0b0001001, 3, 2),
    (7, 0b0001010, 3, 3),
    (8, 0b00010110, 4, 0),
    (8, 0b00010111, 4, 1),
    (9, 0b000110000, 4, 2),
    (9, 0b000110001, 4, 3),
    (7, 0b0001011, 2, 0),
    (8, 0b00011000, 2, 1),
    (8, 0b00011001, 2, 2),
    (8, 0b00011010, 2, 3),
    (9, 0b000000001, 255, 0),
];

/// CBPY VLC 表
/// 对于 Intra 块直接使用; 对于 Inter 块需要取反 (15 - cbpy)
const CBPY: &[(u8, u16, u8)] = &[
    (4, 0x3, 0),
    (5, 0x5, 1),
    (5, 0x4, 2),
    (4, 0x9, 3),
    (5, 0x3, 4),
    (4, 0x7, 5),
    (6, 0x2, 6),
    (6, 0xC, 7),
    (10, 0x1, 8),
    (7, 0x1, 9),
    (8, 0x1, 10),
    (10, 0x2, 11),
    (10, 0x3, 12),
    (7, 0x0, 13),
    (8, 0x0, 14),
    (4, 0xB, 15),
];

/// MVD VLC 表
pub(super) const MVD_VLC: &[(u8, u16, u8)] = &[
    (1, 0b1, 0),
    (2, 0b01, 1),
    (3, 0b001, 2),
    (4, 0b0001, 3),
    (6, 0b000011, 4),
    (7, 0b0000101, 5),
    (7, 0b0000100, 6),
    (7, 0b0000011, 7),
    (8, 0b00000101, 8),
    (8, 0b00000100, 9),
    (8, 0b00000011, 10),
    (10, 0b0000001001, 11),
    (10, 0b0000001000, 12),
    (10, 0b0000000111, 13),
    (10, 0b0000000110, 14),
    (10, 0b0000000101, 15),
    (10, 0b0000000100, 16),
    (10, 0b0000000011, 17),
    (10, 0b0000000010, 18),
    (10, 0b0000000001, 19),
    (10, 0b0000000000, 20),
    (10, 0b0000010011, 21),
    (10, 0b0000010010, 22),
    (10, 0b0000010001, 23),
    (10, 0b0000010000, 24),
    (11, 0b00000101011, 25),
    (11, 0b00000101010, 26),
    (11, 0b00000101001, 27),
    (11, 0b00000101000, 28),
    (11, 0b00000101111, 29),
    (11, 0b00000101110, 30),
    (12, 0b000001011011, 31),
    (12, 0b000001011010, 32),
];

/// Inter AC VLC 表
pub(super) const INTER_AC_VLC: &[(u8, u16, bool, u8, i8)] = &[
    (2, 0x2, false, 0, 1),
    (4, 0xf, false, 0, 2),
    (6, 0x15, false, 0, 3),
    (7, 0x17, false, 0, 4),
    (8, 0x1f, false, 0, 5),
    (9, 0x25, false, 0, 6),
    (9, 0x24, false, 0, 7),
    (10, 0x21, false, 0, 8),
    (10, 0x20, false, 0, 9),
    (11, 0x7, false, 0, 10),
    (11, 0x6, false, 0, 11),
    (11, 0x20, false, 0, 12),
    (3, 0x6, false, 1, 1),
    (6, 0x14, false, 1, 2),
    (8, 0x1e, false, 1, 3),
    (10, 0xf, false, 1, 4),
    (11, 0x21, false, 1, 5),
    (12, 0x50, false, 1, 6),
    (4, 0xe, false, 2, 1),
    (8, 0x1d, false, 2, 2),
    (10, 0xe, false, 2, 3),
    (12, 0x51, false, 2, 4),
    (5, 0xd, false, 3, 1),
    (9, 0x23, false, 3, 2),
    (10, 0xd, false, 3, 3),
    (5, 0xc, false, 4, 1),
    (9, 0x22, false, 4, 2),
    (12, 0x52, false, 4, 3),
    (5, 0xb, false, 5, 1),
    (10, 0xc, false, 5, 2),
    (12, 0x53, false, 5, 3),
    (6, 0x13, false, 6, 1),
    (10, 0xb, false, 6, 2),
    (12, 0x54, false, 6, 3),
    (6, 0x12, false, 7, 1),
    (10, 0xa, false, 7, 2),
    (6, 0x11, false, 8, 1),
    (10, 0x9, false, 8, 2),
    (6, 0x10, false, 9, 1),
    (10, 0x8, false, 9, 2),
    (7, 0x16, false, 10, 1),
    (12, 0x55, false, 10, 2),
    (7, 0x15, false, 11, 1),
    (7, 0x14, false, 12, 1),
    (8, 0x1c, false, 13, 1),
    (8, 0x1b, false, 14, 1),
    (9, 0x21, false, 15, 1),
    (9, 0x20, false, 16, 1),
    (9, 0x1f, false, 17, 1),
    (9, 0x1e, false, 18, 1),
    (9, 0x1d, false, 19, 1),
    (9, 0x1c, false, 20, 1),
    (9, 0x1b, false, 21, 1),
    (9, 0x1a, false, 22, 1),
    (11, 0x22, false, 23, 1),
    (11, 0x23, false, 24, 1),
    (12, 0x56, false, 25, 1),
    (12, 0x57, false, 26, 1),
    (4, 0x7, true, 0, 1),
    (9, 0x19, true, 0, 2),
    (11, 0x5, true, 0, 3),
    (6, 0xf, true, 1, 1),
    (11, 0x4, true, 1, 2),
    (6, 0xe, true, 2, 1),
    (6, 0xd, true, 3, 1),
    (6, 0xc, true, 4, 1),
    (7, 0x13, true, 5, 1),
    (7, 0x12, true, 6, 1),
    (7, 0x11, true, 7, 1),
    (7, 0x10, true, 8, 1),
    (8, 0x1a, true, 9, 1),
    (8, 0x19, true, 10, 1),
    (8, 0x18, true, 11, 1),
    (8, 0x17, true, 12, 1),
    (8, 0x16, true, 13, 1),
    (8, 0x15, true, 14, 1),
    (8, 0x14, true, 15, 1),
    (8, 0x13, true, 16, 1),
    (9, 0x18, true, 17, 1),
    (9, 0x17, true, 18, 1),
    (9, 0x16, true, 19, 1),
    (9, 0x15, true, 20, 1),
    (9, 0x14, true, 21, 1),
    (9, 0x13, true, 22, 1),
    (9, 0x12, true, 23, 1),
    (9, 0x11, true, 24, 1),
    (10, 0x7, true, 25, 1),
    (10, 0x6, true, 26, 1),
    (10, 0x5, true, 27, 1),
    (10, 0x4, true, 28, 1),
    (11, 0x24, true, 29, 1),
    (11, 0x25, true, 30, 1),
    (11, 0x26, true, 31, 1),
    (11, 0x27, true, 32, 1),
    (12, 0x58, true, 33, 1),
    (12, 0x59, true, 34, 1),
    (12, 0x5a, true, 35, 1),
    (12, 0x5b, true, 36, 1),
    (12, 0x5c, true, 37, 1),
    (12, 0x5d, true, 38, 1),
    (12, 0x5e, true, 39, 1),
    (12, 0x5f, true, 40, 1),
    (7, 0x3, false, 0, 0), // Escape
];

// ============================================================================
// VLC O(1) 快速查找表 (阶段 6.1 性能优化)
// ============================================================================

/// AC VLC 快速查找表条目
#[derive(Clone, Copy, Default)]
struct AcVlcFastEntry {
    /// 码长 (0 = 无效条目)
    len: u8,
    /// 特殊标记: 0=普通系数, 1=EOB, 2=Escape
    special: u8,
    /// last 标志
    last: bool,
    /// 游程
    run: u8,
    /// 级别绝对值
    level: u8,
}

/// 快速查找表位宽 (12 bits 覆盖所有 AC VLC 码字)
const AC_FAST_BITS: u8 = 12;
const AC_FAST_SIZE: usize = 1 << AC_FAST_BITS;

/// 构建 AC VLC 快速查找表
///
/// 对每个 VLC 条目, 将其映射到所有可能的 peek(12) 值:
/// - 码字左移 (12-len) 位作为 base index
/// - 填充低位 2^(12-len) 个条目
fn build_ac_vlc_fast(table: &[(u8, u16, bool, u8, i8)]) -> Box<[AcVlcFastEntry; AC_FAST_SIZE]> {
    let mut entries = vec![AcVlcFastEntry::default(); AC_FAST_SIZE];

    for &(len, code, last, run, level) in table {
        if len == 0 || len > AC_FAST_BITS {
            continue;
        }

        let special = if last && run == 0 && level == 0 {
            1 // EOB
        } else if !last && run == 0 && level == 0 {
            2 // Escape
        } else {
            0 // 普通系数
        };

        let padding = AC_FAST_BITS - len;
        let base = (code as usize) << padding;
        for extra in 0..(1usize << padding) {
            entries[base | extra] = AcVlcFastEntry {
                len,
                special,
                last,
                run,
                level: level.unsigned_abs(),
            };
        }
    }

    // Vec<T> -> Box<[T; N]>
    let boxed_slice = entries.into_boxed_slice();
    // SAFETY: 长度已确保为 AC_FAST_SIZE
    unsafe {
        let raw = Box::into_raw(boxed_slice) as *mut [AcVlcFastEntry; AC_FAST_SIZE];
        Box::from_raw(raw)
    }
}

/// 全局 Intra AC 快速查找表 (延迟初始化)
static INTRA_AC_FAST: OnceLock<Box<[AcVlcFastEntry; AC_FAST_SIZE]>> = OnceLock::new();
/// 全局 Inter AC 快速查找表 (延迟初始化)
static INTER_AC_FAST: OnceLock<Box<[AcVlcFastEntry; AC_FAST_SIZE]>> = OnceLock::new();

fn get_intra_ac_fast() -> &'static [AcVlcFastEntry; AC_FAST_SIZE] {
    INTRA_AC_FAST.get_or_init(|| build_ac_vlc_fast(INTRA_AC_VLC))
}

fn get_inter_ac_fast() -> &'static [AcVlcFastEntry; AC_FAST_SIZE] {
    INTER_AC_FAST.get_or_init(|| build_ac_vlc_fast(INTER_AC_VLC))
}

// ============================================================================
// VLC 解码函数
// ============================================================================

/// 解码 MCBPC (I-VOP), 跳过 stuffing code
pub(super) fn decode_mcbpc_i(reader: &mut BitReader) -> Option<(MbType, u8)> {
    while reader.peek_bits(9) == Some(1) {
        reader.skip_bits(9);
    }
    for &(len, code, mb_type_val, cbpc) in MCBPC_I {
        let Some(bits) = reader.peek_bits(len) else {
            continue;
        };
        if bits as u16 == code {
            reader.read_bits(len)?;
            if mb_type_val == 255 {
                return decode_mcbpc_i(reader);
            }
            let mb_type = if mb_type_val == 0 {
                MbType::Intra
            } else {
                MbType::IntraQ
            };
            return Some((mb_type, cbpc));
        }
    }
    None
}

/// 解码 MCBPC (P-VOP), 跳过 stuffing code
pub(super) fn decode_mcbpc_p(reader: &mut BitReader) -> Option<(MbType, u8)> {
    while reader.peek_bits(10) == Some(1) {
        reader.skip_bits(10);
    }
    for &(len, code, mb_type_val, cbpc) in MCBPC_P {
        let Some(bits) = reader.peek_bits(len) else {
            continue;
        };
        if bits as u16 == code {
            reader.read_bits(len)?;
            if mb_type_val == 255 {
                return decode_mcbpc_p(reader);
            }
            let mb_type = match mb_type_val {
                0 => MbType::Inter,
                1 => MbType::InterQ,
                2 => MbType::Inter4V,
                3 => MbType::Intra,
                4 => MbType::IntraQ,
                _ => MbType::Inter,
            };
            return Some((mb_type, cbpc));
        }
    }
    None
}

/// 解码 CBPY
/// `is_intra`: Intra 块直接返回, Inter 块取反 (15 - cbpy)
pub(super) fn decode_cbpy(reader: &mut BitReader, is_intra: bool) -> Option<u8> {
    for &(len, code, cbpy_val) in CBPY {
        if let Some(bits) = reader.peek_bits(len) {
            if bits as u16 == code {
                reader.read_bits(len)?;
                return Some(if is_intra { cbpy_val } else { 15 - cbpy_val });
            }
        }
    }
    warn!("CBPY 解码失败: 字节位置 = {}", reader.byte_position());
    None
}

/// 解码 Intra DC 系数
pub(super) fn decode_intra_dc_vlc(reader: &mut BitReader, is_luma: bool) -> Option<i16> {
    let table = if is_luma {
        INTRA_DC_VLC_Y
    } else {
        INTRA_DC_VLC_UV
    };
    for &(len, code, dc_size) in table {
        let Some(bits) = reader.peek_bits(len) else {
            continue;
        };
        if bits as u16 == code {
            reader.read_bits(len)?;
            if dc_size == 0 {
                return Some(0);
            }
            let diff = reader.read_bits(dc_size as u8)? as i16;
            if dc_size > 8 {
                reader.read_bit(); // marker bit
            }
            let dc_diff = if diff < (1 << (dc_size - 1)) {
                diff - (1 << dc_size) + 1
            } else {
                diff
            };
            return Some(dc_diff);
        }
    }
    None
}

/// RVLC (Reversible VLC) 可逆解码
///
/// RVLC 支持双向可逆解码，常用于 data_partitioned 流的分区 B 中以支持错误恢复。
/// 当前实现支持前向解码路径 (从低频到高频)；后向解码为未来优化。
///
/// # 参数
/// - `reader`: 位流读取器
/// - `table`: 使用的 AC VLC 表 (Intra 或 Inter)
/// - `is_intra`: 是否 Intra 块
/// - `forward`: 若 true 前向解码; 若 false 后向解码 (当前两者等价，使用相同表)
///
/// # 返回
/// - `Ok(Some((last, run, level)))` - 成功解码; last = true 表示最后一个非零系数
/// - `Ok(None)` - EOB (end-of-block)
/// - `Err(())` - 解码失败
///
/// # 实现备注
/// 当前作为标准 AC VLC 解码的正向路径实现。完整的可逆 VLC 需要独立的 RVLC 码表，
/// 支持从末尾开始的后向解码，目前暂不处理。单向解码足以支持大多数 data_partitioned 流。
#[allow(dead_code)]
pub(super) fn decode_ac_rvlc(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
    is_intra: bool,
    forward: bool,
) -> Result<Option<(bool, u8, i16)>, ()> {
    // 当前版本统一使用正向解码路径
    // 完整的后向解码需要从比特流末尾倒序读取，这将在后续版本实现

    if forward {
        // 前向解码：普通 VLC 路径
        decode_ac_vlc(reader, table, is_intra)
    } else {
        // 后向解码：暂时仍用前向路径
        // 警告用户仅输出一次，避免日志爆满
        static RVLC_REVERSE_WARN: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(true);
        if RVLC_REVERSE_WARN.swap(false, std::sync::atomic::Ordering::Relaxed) {
            warn!("RVLC 后向解码未完全实现，使用前向路径代替（仍可使用）");
        }
        decode_ac_vlc(reader, table, is_intra)
    }
}

/// 获取 escape mode 的 max_level 值
fn get_max_level(is_intra: bool, last: bool, run: usize) -> u8 {
    let last_idx = last as usize;
    if is_intra {
        match last_idx {
            0 => INTRA_MAX_LEVEL_LAST0.get(run).copied().unwrap_or(0),
            _ => INTRA_MAX_LEVEL_LAST1.get(run).copied().unwrap_or(0),
        }
    } else {
        match last_idx {
            0 => INTER_MAX_LEVEL_LAST0.get(run).copied().unwrap_or(0),
            _ => INTER_MAX_LEVEL_LAST1.get(run).copied().unwrap_or(0),
        }
    }
}

/// 获取 escape mode 的 max_run 值
fn get_max_run(is_intra: bool, last: bool, level: usize) -> u8 {
    let last_idx = last as usize;
    if is_intra {
        match last_idx {
            0 => INTRA_MAX_RUN_LAST0.get(level).copied().unwrap_or(0),
            _ => INTRA_MAX_RUN_LAST1.get(level).copied().unwrap_or(0),
        }
    } else {
        match last_idx {
            0 => INTER_MAX_RUN_LAST0.get(level).copied().unwrap_or(0),
            _ => INTER_MAX_RUN_LAST1.get(level).copied().unwrap_or(0),
        }
    }
}

/// 使用 VLC 表解码 AC 系数 (支持 Escape Mode 1/2/3)
///
/// 使用 O(1) 快速查找表 (4096 项) + escape 慢路径.
///
/// 返回:
/// - `Ok(Some((last, run, level)))` - 成功解码
/// - `Ok(None)` - EOB
/// - `Err(())` - 解码错误
pub(super) fn decode_ac_vlc(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
    is_intra: bool,
) -> Result<Option<(bool, u8, i16)>, ()> {
    // O(1) 快速路径: peek 12 bits, 查找表直接命中
    let fast = if is_intra {
        get_intra_ac_fast()
    } else {
        get_inter_ac_fast()
    };

    if let Some(peek) = reader.peek_bits(AC_FAST_BITS) {
        let entry = &fast[peek as usize];

        if entry.len > 0 {
            match entry.special {
                1 => {
                    // EOB
                    reader.read_bits(entry.len).ok_or(())?;
                    return Ok(None);
                }
                2 => {
                    // Escape: 消耗 7-bit escape 码, 进入 escape 模式处理
                    reader.read_bits(7).ok_or(())?;
                    return decode_ac_escape_modes(reader, table, is_intra);
                }
                _ => {
                    // 普通系数: 消耗码字 + 读取符号位
                    reader.read_bits(entry.len).ok_or(())?;
                    let sign = reader.read_bit().ok_or(())?;
                    let level = if sign {
                        -(entry.level as i16)
                    } else {
                        entry.level as i16
                    };
                    return Ok(Some((entry.last, entry.run, level)));
                }
            }
        }
    }

    // 回退: 数据不足, 尝试逐条匹配 (处理剩余位不足 12 bits 的边界情况)
    for &(len, code, last, run, level) in table {
        let Some(bits) = reader.peek_bits(len) else {
            continue;
        };
        if bits as u16 == code {
            reader.read_bits(len).ok_or(())?;
            if last && run == 0 && level == 0 {
                return Ok(None);
            }
            if !last && run == 0 && level == 0 {
                return decode_ac_escape_flc(reader).map(Some).ok_or(());
            }
            let sign = reader.read_bit().ok_or(())?;
            let actual_level = if !sign { level as i16 } else { -(level as i16) };
            return Ok(Some((last, run, actual_level)));
        }
    }

    warn!("AC VLC 解码失败: 字节位置 = {}", reader.byte_position());
    Err(())
}

/// Escape 模式处理 (Mode 1/2/3)
///
/// 在 7-bit escape 码已被消耗后调用.
fn decode_ac_escape_modes(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
    is_intra: bool,
) -> Result<Option<(bool, u8, i16)>, ()> {
    let mode_bit1 = reader.peek_bits(1).ok_or(())?;
    if mode_bit1 == 0 {
        // Escape Mode 1: level 偏移
        reader.read_bits(1).ok_or(())?;
        if let Some(result) = decode_vlc_entry(reader, table)? {
            let (last, run, level) = result;
            let max_lev = get_max_level(is_intra, last, run as usize) as i16;
            let abs_level = level.unsigned_abs() + max_lev as u16;
            let final_level = if level < 0 {
                -(abs_level as i16)
            } else {
                abs_level as i16
            };
            return Ok(Some((last, run, final_level)));
        }
        return Err(());
    }
    let mode_bit2 = reader.peek_bits(2).ok_or(())?;
    if mode_bit2 == 2 {
        // 10 → Escape Mode 2: run 偏移
        reader.read_bits(2).ok_or(())?;
        if let Some(result) = decode_vlc_entry(reader, table)? {
            let (last, run, level) = result;
            let max_r = get_max_run(is_intra, last, level.unsigned_abs() as usize);
            let final_run = run + max_r + 1;
            return Ok(Some((last, final_run, level)));
        }
        return Err(());
    }
    // 11 → Escape Mode 3: FLC
    reader.read_bits(2).ok_or(())?;
    decode_ac_escape_flc(reader).map(Some).ok_or(())
}

/// 从 VLC 表解码一个条目 (用于 escape mode 1/2 后续解码)
fn decode_vlc_entry(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
) -> Result<Option<(bool, u8, i16)>, ()> {
    for &(len, code, last, run, level) in table {
        if run == 0 && level == 0 {
            continue;
        }
        let bits = reader.peek_bits(len).ok_or(())?;
        if bits as u16 == code {
            reader.read_bits(len).ok_or(())?;
            let sign = reader.read_bit().ok_or(())?;
            let actual_level = if !sign { level as i16 } else { -(level as i16) };
            return Ok(Some((last, run, actual_level)));
        }
    }
    Ok(None)
}

/// Escape Mode 3: FLC 解码 (last:1 + run:6 + marker:1 + level:12 + marker:1)
fn decode_ac_escape_flc(reader: &mut BitReader) -> Option<(bool, u8, i16)> {
    let last = reader.read_bits(1)? != 0;
    let run = reader.read_bits(6)? as u8;
    let _marker1 = reader.read_bits(1)?;
    let level_bits = reader.read_bits(12)? as i16;
    let _marker2 = reader.read_bits(1)?;
    let level = if level_bits >= 2048 {
        level_bits - 4096
    } else {
        level_bits
    };
    Some((last, run, level))
}

// ============================================================================
// B 帧 VLC 解码函数
// ============================================================================

/// 解码 MODB (B-VOP 宏块类型标志)
///
/// 返回 `(mb_type_present, cbp_present)`:
/// - MODB = "1": 两者均不存在, 使用 Direct 模式, CBP=0
/// - MODB = "01": mb_type 存在, CBP=0
/// - MODB = "00": 两者均存在
pub(super) fn decode_modb(reader: &mut BitReader) -> (bool, bool) {
    match reader.read_bit() {
        Some(true) => (false, false),
        Some(false) => match reader.read_bit() {
            Some(true) => (true, false),
            Some(false) => (true, true),
            None => (false, false),
        },
        None => (false, false),
    }
}

/// 解码 B-VOP 宏块类型
///
/// VLC 编码:
/// - "1" → Direct
/// - "01" → Interpolate
/// - "001" → Backward
/// - "0001" → Forward
/// - "00001" → DirectNoneMv (fallback)
pub(super) fn decode_b_mb_type(reader: &mut BitReader) -> BframeMbMode {
    for mode_idx in 0..4u8 {
        match reader.read_bit() {
            Some(true) => {
                return match mode_idx {
                    0 => BframeMbMode::Direct,
                    1 => BframeMbMode::Interpolate,
                    2 => BframeMbMode::Backward,
                    3 => BframeMbMode::Forward,
                    _ => BframeMbMode::Direct,
                };
            }
            Some(false) => continue,
            None => return BframeMbMode::Direct,
        }
    }
    BframeMbMode::DirectNoneMv
}

/// 解码 DBQUANT (B-VOP 量化变化)
///
/// - "0" → 0 (无变化)
/// - "10" → -2
/// - "11" → +2
pub(super) fn decode_dbquant(reader: &mut BitReader) -> i32 {
    match reader.read_bit() {
        Some(false) => 0,
        Some(true) => match reader.read_bit() {
            Some(false) => -2,
            Some(true) => 2,
            None => 0,
        },
        None => 0,
    }
}
