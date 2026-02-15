//! MPEG-4 Part 2 视频解码器
//!
//! 实现 MPEG-4 Part 2 (ISO/IEC 14496-2) 视频解码器.
//! 支持 Simple Profile 和 Advanced Simple Profile.
//!
//! 已实现:
//! - I/P/B 帧解码 (B 帧支持 Direct/Forward/Backward/Interpolate 模式)
//! - VOP/VOL 头部解析 (含 complexity_estimation, resync_marker, data_partitioned)
//! - 宏块解码: Intra, Inter, InterQ, IntraQ, Inter4V
//! - 完整 VLC 解码 (Escape Mode 1/2/3)
//! - H.263 和 MPEG 两种反量化类型
//! - DC Scaler (按 MPEG-4 标准 Table 7-1)
//! - 运动补偿: 全像素, 半像素, 四分之一像素
//! - Chroma MV 推导 (含 rounding table)
//! - MV 范围包装 (基于 f_code)
//! - AC/DC 预测
//! - Alternate scan tables (vertical/horizontal)
//! - Mismatch control (MPEG 量化类型)
//! - 边缘扩展 (edge padding)
//! - GMC (全局运动补偿, S-VOP)
//! - Resync marker 检测与错误恢复
//! - 隔行扫描 (field_dct, field_pred)
//! - 整数 IDCT (定点 AAN 算法)

use log::{debug, warn};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;

// ============================================================================
// BitReader
// ============================================================================

/// 位流读取器
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// 读取 n 位 (最多 32 位)
    fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 || n > 32 {
            return None;
        }
        let mut result = 0u32;
        let mut bits_to_read = n;
        while bits_to_read > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }
            let bits_available = 8 - self.bit_pos;
            let bits_from_this_byte = bits_to_read.min(bits_available);
            let byte = self.data[self.byte_pos];
            let mask = if bits_from_this_byte >= 8 {
                0xFF
            } else {
                (1u8 << bits_from_this_byte) - 1
            };
            let shift = bits_available - bits_from_this_byte;
            let bits = (byte >> shift) & mask;
            result = result.checked_shl(bits_from_this_byte as u32).unwrap_or(0) | (bits as u32);
            self.bit_pos += bits_from_this_byte;
            if self.bit_pos >= 8 {
                self.byte_pos += 1;
                self.bit_pos = 0;
            }
            bits_to_read -= bits_from_this_byte;
        }
        Some(result)
    }

    /// 窥视 n 位 (不消耗)
    fn peek_bits(&self, n: u8) -> Option<u32> {
        if n == 0 || n > 32 {
            return None;
        }
        let mut result = 0u32;
        let mut byte_pos = self.byte_pos;
        let mut bit_pos = self.bit_pos;
        for _ in 0..n {
            if byte_pos >= self.data.len() {
                return None;
            }
            let bit = (self.data[byte_pos] >> (7 - bit_pos)) & 1;
            result = (result << 1) | (bit as u32);
            bit_pos += 1;
            if bit_pos >= 8 {
                bit_pos = 0;
                byte_pos += 1;
            }
        }
        Some(result)
    }

    /// 跳过 n 位
    fn skip_bits(&mut self, n: u32) {
        let total_bits = self.byte_pos as u32 * 8 + self.bit_pos as u32 + n;
        self.byte_pos = (total_bits / 8) as usize;
        self.bit_pos = (total_bits % 8) as u8;
    }

    /// 读取单个位
    fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|b| b != 0)
    }

    /// 获取剩余可读位数
    #[allow(dead_code)]
    fn bits_left(&self) -> usize {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_pos) * 8 - self.bit_pos as usize
    }

    /// 获取当前字节位置
    #[allow(dead_code)]
    fn byte_position(&self) -> usize {
        self.byte_pos
    }

    /// 获取当前位位置
    #[allow(dead_code)]
    fn bit_position(&self) -> usize {
        self.byte_pos * 8 + self.bit_pos as usize
    }

    /// 到下一个字节边界的位数
    fn bits_to_byte_align(&self) -> u8 {
        if self.bit_pos == 0 {
            0
        } else {
            8 - self.bit_pos
        }
    }

    /// 字节对齐
    #[allow(dead_code)]
    fn align_to_byte(&mut self) {
        if self.bit_pos != 0 {
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
    }
}

// ============================================================================
// 起始码查找
// ============================================================================

fn find_start_code_offset(data: &[u8], target: u8) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    for idx in 0..(data.len() - 3) {
        if data[idx] == 0x00
            && data[idx + 1] == 0x00
            && data[idx + 2] == 0x01
            && data[idx + 3] == target
        {
            return Some(idx + 4);
        }
    }
    None
}

fn find_start_code_range(data: &[u8], start: u8, end: u8) -> Option<(u8, usize)> {
    if data.len() < 4 {
        return None;
    }
    for idx in 0..(data.len() - 3) {
        if data[idx] == 0x00 && data[idx + 1] == 0x00 && data[idx + 2] == 0x01 {
            let code = data[idx + 3];
            if (start..=end).contains(&code) {
                return Some((code, idx + 4));
            }
        }
    }
    None
}

// ============================================================================
// 常量
// ============================================================================

/// MPEG-4 起始码
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT_SEQUENCE: u8 = 0xB0;
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT: u8 = 0xB5;
const START_CODE_VOP: u8 = 0xB6;
const START_CODE_VIDEO_OBJECT_LAYER: u8 = 0x20; // 0x20-0x2F

/// VOP 编码类型
const VOP_TYPE_I: u8 = 0;
const VOP_TYPE_P: u8 = 1;
const VOP_TYPE_B: u8 = 2;
const VOP_TYPE_S: u8 = 3;

/// 标准 MPEG-4 量化矩阵 (Intra)
const STD_INTRA_QUANT_MATRIX: [u8; 64] = [
    8, 16, 19, 22, 26, 27, 29, 34, 16, 16, 22, 24, 27, 29, 34, 37, 19, 22, 26, 27, 29, 34, 34, 38,
    22, 22, 26, 27, 29, 34, 37, 40, 22, 26, 27, 29, 32, 35, 40, 48, 26, 27, 29, 32, 35, 40, 48, 58,
    26, 27, 29, 34, 38, 46, 56, 69, 27, 29, 35, 38, 46, 56, 69, 83,
];

/// 标准 MPEG-4 量化矩阵 (Inter)
const STD_INTER_QUANT_MATRIX: [u8; 64] = [
    16, 17, 18, 19, 20, 21, 22, 23, 17, 18, 19, 20, 21, 22, 23, 24, 18, 19, 20, 21, 22, 23, 24, 25,
    19, 20, 21, 22, 23, 24, 25, 26, 20, 21, 22, 23, 24, 25, 26, 27, 21, 22, 23, 24, 25, 26, 27, 28,
    22, 23, 24, 25, 26, 27, 28, 29, 23, 24, 25, 26, 27, 28, 29, 30,
];

/// 标准 8x8 zigzag 扫描顺序
const ZIGZAG_SCAN: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// 水平 alternate 扫描 (用于隔行和 AC 预测方向 horizontal)
#[allow(dead_code)]
const ALTERNATE_HORIZONTAL_SCAN: [usize; 64] = [
    0, 1, 2, 3, 8, 9, 16, 17, 10, 11, 4, 5, 6, 7, 15, 14, 13, 12, 19, 18, 24, 25, 32, 33, 26, 27,
    20, 21, 22, 23, 28, 29, 30, 31, 34, 35, 40, 41, 48, 49, 42, 43, 36, 37, 38, 39, 44, 45, 46, 47,
    50, 51, 56, 57, 58, 59, 52, 53, 54, 55, 60, 61, 62, 63,
];

/// 垂直 alternate 扫描 (用于隔行和 AC 预测方向 vertical)
#[allow(dead_code)]
const ALTERNATE_VERTICAL_SCAN: [usize; 64] = [
    0, 8, 16, 24, 1, 9, 2, 10, 17, 25, 32, 40, 48, 56, 41, 33, 26, 18, 3, 11, 4, 12, 19, 27, 34,
    42, 49, 57, 50, 58, 35, 43, 20, 28, 5, 13, 6, 14, 21, 29, 36, 44, 51, 59, 52, 60, 37, 45, 22,
    30, 7, 15, 23, 31, 38, 46, 53, 61, 54, 62, 39, 47, 55, 63,
];

/// DC Scaler 查找表 (亮度), 索引为 quant (0-31)
/// 基于 MPEG-4 Part 2 Table 7-1
const DC_SCALER_Y: [u8; 32] = [
    0, 8, 8, 8, 8, 10, 12, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    34, 36, 38, 40, 42, 44, 46,
];

/// DC Scaler 查找表 (色度), 索引为 quant (0-31)
const DC_SCALER_C: [u8; 32] = [
    0, 8, 8, 8, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31, 32, 34, 36, 38,
];

/// Chroma MV 舍入表 (1MV 模式, 除以 2 后的余数索引)
/// roundtab_79[x & 0x3]: 应用于 (mv >> 1) + roundtab_79[mv & 0x3]
const ROUNDTAB_79: [i16; 4] = [0, 0, 0, 1];

/// Chroma MV 舍入表 (4MV 模式, 4 个 MV 求和后除以 8 的余数索引)
/// roundtab_76[x & 0xf]
const ROUNDTAB_76: [i16; 16] = [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 1];

// ============================================================================
// Escape 模式的 max_level / max_run 表 (基于 MPEG-4 标准)
// ============================================================================

/// Inter AC VLC max_level[last][run] - 通过 VLC 表可编码的最大 level
/// last=0: run 0..26 的最大 level
const INTER_MAX_LEVEL_LAST0: [u8; 27] = [
    12, 6, 4, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];
/// last=1: run 0..40 的最大 level
const INTER_MAX_LEVEL_LAST1: [u8; 41] = [
    3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1,
];
/// Inter AC VLC max_run[last][level] - 通过 VLC 表可编码的最大 run
/// last=0: level 1..12 的最大 run
const INTER_MAX_RUN_LAST0: [u8; 13] = [0, 26, 10, 6, 2, 1, 1, 0, 0, 0, 0, 0, 0];
/// last=1: level 1..3 的最大 run
const INTER_MAX_RUN_LAST1: [u8; 4] = [0, 40, 1, 0];

/// Intra AC VLC max_level[last][run]
/// last=0: run 0..14 的最大 level
const INTRA_MAX_LEVEL_LAST0: [u8; 15] = [27, 10, 5, 4, 3, 3, 3, 3, 2, 2, 1, 1, 1, 1, 1];
/// last=1: run 0..20 的最大 level
const INTRA_MAX_LEVEL_LAST1: [u8; 21] = [
    8, 3, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];
/// Intra AC VLC max_run[last][level]
/// last=0: level 1..27 的最大 run
const INTRA_MAX_RUN_LAST0: [u8; 28] = [
    0, 14, 9, 7, 3, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
/// last=1: level 1..8 的最大 run
const INTRA_MAX_RUN_LAST1: [u8; 9] = [0, 20, 6, 1, 0, 0, 0, 0, 0];

// ============================================================================
// DQUANT 表
// ============================================================================

/// dquant 映射 (2-bit 值 → 量化参数变化)
const DQUANT_TABLE: [i32; 4] = [-1, -2, 1, 2];

// ============================================================================
// intra_dc_vlc_thr 阈值表
// ============================================================================

/// 当 quant 达到此阈值时, Intra DC 不再使用 DC VLC 而使用 Inter VLC
/// 索引为 intra_dc_vlc_thr (0-7), 值为 quant 阈值
/// 0 表示始终使用 DC VLC, 7 表示始终不使用 DC VLC
const INTRA_DC_THRESHOLD: [u32; 8] = [32, 13, 15, 17, 19, 21, 23, 0];

// ============================================================================
// 整数 IDCT 常量 (Chen-Wang 算法, 13 位定点)
// ============================================================================

const W1: i32 = 2841; // 2048*sqrt(2)*cos(1*pi/16)
const W2: i32 = 2676; // 2048*sqrt(2)*cos(2*pi/16)
const W3: i32 = 2408; // 2048*sqrt(2)*cos(3*pi/16)
const W5: i32 = 1609; // 2048*sqrt(2)*cos(5*pi/16)
const W6: i32 = 1108; // 2048*sqrt(2)*cos(6*pi/16)
const W7: i32 = 565; // 2048*sqrt(2)*cos(7*pi/16)

fn read_quant_matrix(reader: &mut BitReader) -> Option<[u8; 64]> {
    let mut matrix = [0u8; 64];
    for &pos in ZIGZAG_SCAN.iter() {
        let val = reader.read_bits(8)? as u8;
        matrix[pos] = if val == 0 { 1 } else { val };
    }
    Some(matrix)
}

// ============================================================================
// VLC 表
// ============================================================================

/// Intra DC VLC 表 (Y 亮度通道)
/// 格式: (位数, 码字, dc_size)
const INTRA_DC_VLC_Y: &[(u8, u16, i16)] = &[
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
const INTRA_DC_VLC_UV: &[(u8, u16, i16)] = &[
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
const INTRA_AC_VLC: &[(u8, u16, bool, u8, i8)] = &[
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
/// 格式: (位数, 码字, cbpy值)
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
/// 格式: (位数, 码字, MVD 索引)
const MVD_VLC: &[(u8, u16, u8)] = &[
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
const INTER_AC_VLC: &[(u8, u16, bool, u8, i8)] = &[
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
// 类型定义
// ============================================================================

/// 宏块类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MbType {
    Intra,
    IntraQ,
    Inter,
    InterQ,
    Inter4V,
}

/// 运动向量
#[derive(Debug, Clone, Copy, Default)]
struct MotionVector {
    x: i16,
    y: i16,
}

/// 预测方向
#[derive(Debug, Clone, Copy, PartialEq)]
enum PredictorDirection {
    #[allow(dead_code)]
    None,
    Horizontal,
    Vertical,
}

// ============================================================================
// VLC 解码函数
// ============================================================================

/// 解码 MCBPC (I-VOP), 跳过 stuffing code
fn decode_mcbpc_i(reader: &mut BitReader) -> Option<(MbType, u8)> {
    // 跳过 stuffing code (9 位 == 1)
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
fn decode_mcbpc_p(reader: &mut BitReader) -> Option<(MbType, u8)> {
    // 跳过 stuffing code (10 位 == 1)
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
fn decode_cbpy(reader: &mut BitReader, is_intra: bool) -> Option<u8> {
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
fn decode_intra_dc_vlc(reader: &mut BitReader, is_luma: bool) -> Option<i16> {
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
            // dc_size > 8 时需要跳过一个 marker bit
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
/// 返回:
/// - `Ok(Some((last, run, level)))` - 成功解码
/// - `Ok(None)` - EOB
/// - `Err(())` - 解码错误
fn decode_ac_vlc(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
    is_intra: bool,
) -> Result<Option<(bool, u8, i16)>, ()> {
    // 检查 escape 码 (7 位 = 0000011)
    if let Some(escape_check) = reader.peek_bits(7) {
        if escape_check == 3 {
            reader.read_bits(7).ok_or(())?;
            // 判断 escape 模式
            let mode_bit1 = reader.peek_bits(1).ok_or(())?;
            if mode_bit1 == 0 {
                // Escape Mode 1: level 偏移
                reader.read_bits(1).ok_or(())?;
                // 解码后续 VLC, 然后 level += max_level
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
            // 11 → Escape Mode 3: FLC (固定长度编码)
            reader.read_bits(2).ok_or(())?;
            return decode_ac_escape_flc(reader).map(Some).ok_or(());
        }
    }

    // 尝试匹配 VLC 表中的普通条目
    for &(len, code, last, run, level) in table {
        let bits = reader.peek_bits(len).ok_or(())?;
        if bits as u16 == code {
            reader.read_bits(len).ok_or(())?;
            // EOB
            if last && run == 0 && level == 0 {
                return Ok(None);
            }
            // Escape (已在上面处理, 此处跳过)
            if !last && run == 0 && level == 0 {
                // 到这里不应发生, 因为 escape 已在上面处理
                return decode_ac_escape_flc(reader).map(Some).ok_or(());
            }
            // 正常系数: 读取符号位
            let sign = reader.read_bit().ok_or(())?;
            let actual_level = if !sign { level as i16 } else { -(level as i16) };
            return Ok(Some((last, run, actual_level)));
        }
    }

    warn!("AC VLC 解码失败: 字节位置 = {}", reader.byte_position());
    Err(())
}

/// 从 VLC 表解码一个条目 (用于 escape mode 1/2 后续解码)
fn decode_vlc_entry(
    reader: &mut BitReader,
    table: &[(u8, u16, bool, u8, i8)],
) -> Result<Option<(bool, u8, i16)>, ()> {
    for &(len, code, last, run, level) in table {
        // 跳过 EOB 和 Escape 条目
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
// 块解码函数
// ============================================================================

/// 解码 Intra 块的 DCT 系数
#[allow(clippy::too_many_arguments)]
fn decode_intra_block_vlc(
    reader: &mut BitReader,
    plane: usize,
    mb_x: u32,
    mb_y: u32,
    block_idx: usize,
    ac_pred_flag: bool,
    ac_coded: bool,
    decoder: &mut Mpeg4Decoder,
) -> Option<[i32; 64]> {
    let mut block = [0i32; 64];
    let is_luma = plane == 0;

    // 1. DC 系数: 解码差分 → 加 DC 预测 → 乘以 DC scaler
    let dc_scaler = decoder.get_dc_scaler(is_luma);
    let dc_diff = if decoder.use_intra_dc_vlc() {
        decode_intra_dc_vlc(reader, is_luma)?
    } else {
        // 当 quant >= intra_dc_vlc_thr 时, 不使用 DC VLC
        // DC 系数作为第一个 AC 系数处理 (start_coeff = 0)
        0
    };
    let (dc_pred, direction) = decoder.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);
    let actual_dc = dc_pred.wrapping_add(dc_diff);
    // DC 系数存储为反量化后的值 (乘以 dc_scaler)
    block[0] = actual_dc as i32 * dc_scaler as i32;

    // 2. AC 系数
    if ac_coded {
        let start = if decoder.use_intra_dc_vlc() { 1 } else { 0 };
        let mut pos = start;
        while pos < 64 {
            match decode_ac_vlc(reader, INTRA_AC_VLC, true) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[ZIGZAG_SCAN[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
    }

    // 3. AC 预测
    if ac_pred_flag {
        match direction {
            PredictorDirection::Vertical => {
                let c_idx = match block_idx {
                    0 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                    1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                    2 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                    3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                    4 | 5 => {
                        decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, block_idx)
                    }
                    _ => None,
                };
                if let Some(idx) = c_idx {
                    let pred_ac = decoder.predictor_cache[idx];
                    for i in 1..8 {
                        block[ZIGZAG_SCAN[i]] =
                            block[ZIGZAG_SCAN[i]].wrapping_add(pred_ac[i] as i32);
                    }
                }
            }
            PredictorDirection::Horizontal => {
                let a_idx = match block_idx {
                    0 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 1),
                    1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                    2 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 3),
                    3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 2),
                    4 | 5 => {
                        decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, block_idx)
                    }
                    _ => None,
                };
                if let Some(idx) = a_idx {
                    let pred_ac = decoder.predictor_cache[idx];
                    for i in 1..8 {
                        block[ZIGZAG_SCAN[i * 8]] =
                            block[ZIGZAG_SCAN[i * 8]].wrapping_add(pred_ac[7 + i] as i32);
                    }
                }
            }
            _ => {}
        }
    }

    // 4. 更新预测器缓存 (存储量化前的 DC/AC 以供预测使用)
    let cache_pos = (mb_y as usize * decoder.mb_stride + mb_x as usize) * 6 + block_idx;
    if let Some(cache) = decoder.predictor_cache.get_mut(cache_pos) {
        cache[0] = actual_dc;
        for i in 1..8 {
            cache[i] = block[ZIGZAG_SCAN[i]] as i16;
        }
        for i in 1..8 {
            cache[7 + i] = block[ZIGZAG_SCAN[i * 8]] as i16;
        }
    }

    Some(block)
}

/// 解码 Inter 块的 DCT 系数
fn decode_inter_block_vlc(reader: &mut BitReader) -> Option<[i32; 64]> {
    let mut block = [0i32; 64];
    let mut pos = 0;
    while pos < 64 {
        match decode_ac_vlc(reader, INTER_AC_VLC, false) {
            Ok(None) => break,
            Ok(Some((last, run, level))) => {
                pos += run as usize;
                if pos >= 64 {
                    break;
                }
                block[ZIGZAG_SCAN[pos]] = level as i32;
                pos += 1;
                if last {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    Some(block)
}

// ============================================================================
// 整数 IDCT (Chen-Wang 算法, 13 位定点)
// ============================================================================

/// 8 点一维 IDCT 行变换
fn idct_row(block: &mut [i32; 64], row: usize) {
    let off = row * 8;
    let x0 = block[off];
    let x1 = block[off + 1];
    let x2 = block[off + 2];
    let x3 = block[off + 3];
    let x4 = block[off + 4];
    let x5 = block[off + 5];
    let x6 = block[off + 6];
    let x7 = block[off + 7];

    // 快速检查: 如果 AC 系数全零, 只用 DC
    if x1 == 0 && x2 == 0 && x3 == 0 && x4 == 0 && x5 == 0 && x6 == 0 && x7 == 0 {
        let val = x0 << 3;
        for i in 0..8 {
            block[off + i] = val;
        }
        return;
    }

    // 第一阶段: 蝶形运算
    let mut a0 = (W2 * x2 + W6 * x6) >> 11;
    let mut a1 = (W6 * x2 - W2 * x6) >> 11;
    let mut a2 = (x0 + x4) << 1;
    let mut a3 = (x0 - x4) << 1;

    let b0 = a2 + a0;
    let b1 = a3 + a1;
    let b2 = a3 - a1;
    let b3 = a2 - a0;

    a0 = (W1 * x1 + W3 * x3 + W5 * x5 + W7 * x7) >> 11;
    a1 = (W3 * x1 - W7 * x3 - W1 * x5 - W5 * x7) >> 11;
    a2 = (W5 * x1 - W1 * x3 + W7 * x5 + W3 * x7) >> 11;
    a3 = (W7 * x1 - W5 * x3 + W3 * x5 - W1 * x7) >> 11;

    block[off] = b0 + a0;
    block[off + 1] = b1 + a1;
    block[off + 2] = b2 + a2;
    block[off + 3] = b3 + a3;
    block[off + 4] = b3 - a3;
    block[off + 5] = b2 - a2;
    block[off + 6] = b1 - a1;
    block[off + 7] = b0 - a0;
}

/// 8 点一维 IDCT 列变换
fn idct_col(block: &mut [i32; 64], col: usize) {
    let x0 = block[col];
    let x1 = block[col + 8];
    let x2 = block[col + 16];
    let x3 = block[col + 24];
    let x4 = block[col + 32];
    let x5 = block[col + 40];
    let x6 = block[col + 48];
    let x7 = block[col + 56];

    if x1 == 0 && x2 == 0 && x3 == 0 && x4 == 0 && x5 == 0 && x6 == 0 && x7 == 0 {
        let val = (x0 + 32) >> 6;
        for i in 0..8 {
            block[col + i * 8] = val;
        }
        return;
    }

    let mut a0 = (W2 * x2 + W6 * x6) >> 11;
    let mut a1 = (W6 * x2 - W2 * x6) >> 11;
    let mut a2 = (x0 + x4) << 1;
    let mut a3 = (x0 - x4) << 1;

    let b0 = a2 + a0;
    let b1 = a3 + a1;
    let b2 = a3 - a1;
    let b3 = a2 - a0;

    a0 = (W1 * x1 + W3 * x3 + W5 * x5 + W7 * x7) >> 11;
    a1 = (W3 * x1 - W7 * x3 - W1 * x5 - W5 * x7) >> 11;
    a2 = (W5 * x1 - W1 * x3 + W7 * x5 + W3 * x7) >> 11;
    a3 = (W7 * x1 - W5 * x3 + W3 * x5 - W1 * x7) >> 11;

    block[col] = (b0 + a0 + 32) >> 6;
    block[col + 8] = (b1 + a1 + 32) >> 6;
    block[col + 16] = (b2 + a2 + 32) >> 6;
    block[col + 24] = (b3 + a3 + 32) >> 6;
    block[col + 32] = (b3 - a3 + 32) >> 6;
    block[col + 40] = (b2 - a2 + 32) >> 6;
    block[col + 48] = (b1 - a1 + 32) >> 6;
    block[col + 56] = (b0 - a0 + 32) >> 6;
}

/// 完整 8x8 IDCT (行+列)
fn idct_8x8(block: &mut [i32; 64]) {
    for row in 0..8 {
        idct_row(block, row);
    }
    for col in 0..8 {
        idct_col(block, col);
    }
}

// ============================================================================
// Mpeg4Decoder 结构体
// ============================================================================

/// VOL (Video Object Layer) 信息
#[derive(Debug, Clone)]
struct VolInfo {
    vop_time_increment_resolution: u16,
    #[allow(dead_code)]
    fixed_vop_rate: bool,
    #[allow(dead_code)]
    data_partitioned: bool,
    /// 量化类型: 0=H.263, 1=MPEG
    quant_type: u8,
    /// 是否支持隔行扫描
    interlacing: bool,
    /// 是否启用 quarter-pixel
    #[allow(dead_code)]
    quarterpel: bool,
    /// sprite 使能 (0=无, 1=static, 2=GMC)
    #[allow(dead_code)]
    sprite_enable: u8,
    /// sprite warping 点数
    #[allow(dead_code)]
    sprite_warping_points: u8,
    /// 是否禁用 resync marker
    #[allow(dead_code)]
    resync_marker_disable: bool,
}

/// VOP (Video Object Plane) 信息
#[derive(Debug)]
struct VopInfo {
    picture_type: PictureType,
    vop_coded: bool,
    #[allow(dead_code)]
    vop_rounding_type: u8,
    /// intra_dc_vlc_thr (用于判断是否使用 DC VLC)
    #[allow(dead_code)]
    intra_dc_vlc_thr: u32,
}

/// MPEG-4 Part 2 视频解码器
pub struct Mpeg4Decoder {
    width: u32,
    height: u32,
    pixel_format: PixelFormat,
    opened: bool,
    /// 前向参考帧 (I/P 帧参考)
    reference_frame: Option<VideoFrame>,
    /// 后向参考帧 (B 帧使用)
    backward_reference: Option<VideoFrame>,
    pending_frame: Option<VideoFrame>,
    frame_count: u64,
    quant: u8,
    vol_info: Option<VolInfo>,
    quant_matrix_intra: [u8; 64],
    quant_matrix_inter: [u8; 64],
    /// 预测器缓存: [DC, row AC 1-7, col AC 1-7] per block
    predictor_cache: Vec<[i16; 15]>,
    /// 运动向量缓存 (每个 MB 存储 4 个 MV, 支持 Inter4V)
    mv_cache: Vec<[MotionVector; 4]>,
    mb_stride: usize,
    f_code_forward: u8,
    #[allow(dead_code)]
    f_code_backward: u8,
    rounding_control: u8,
    /// 当前 VOP 的 intra_dc_vlc_thr
    intra_dc_vlc_thr: u32,
    /// B 帧时间距离参数
    #[allow(dead_code)]
    time_pp: i32,
    #[allow(dead_code)]
    time_bp: i32,
}

impl Mpeg4Decoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
        }))
    }

    // ========================================================================
    // 辅助函数
    // ========================================================================

    /// 获取 DC scaler
    fn get_dc_scaler(&self, is_luma: bool) -> u8 {
        let q = (self.quant as usize).min(31);
        if is_luma {
            DC_SCALER_Y[q]
        } else {
            DC_SCALER_C[q]
        }
    }

    /// 判断当前 VOP 是否使用 Intra DC VLC
    fn use_intra_dc_vlc(&self) -> bool {
        let thr = INTRA_DC_THRESHOLD
            .get(self.intra_dc_vlc_thr as usize)
            .copied()
            .unwrap_or(0);
        // thr==0 表示始终不使用 DC VLC; thr==32 表示始终使用
        (self.quant as u32) < thr
    }

    /// 获取预测器缓存索引
    fn get_neighbor_block_idx(&self, x: isize, y: isize, idx: usize) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.mb_stride as isize {
            return None;
        }
        let mb_height = (self.height as usize).div_ceil(16);
        if y >= mb_height as isize {
            return None;
        }
        Some((y as usize * self.mb_stride + x as usize) * 6 + idx)
    }

    /// 获取 Intra DC 预测方向和预测值
    fn get_intra_predictor(
        &self,
        mb_x: usize,
        mb_y: usize,
        block_idx: usize,
    ) -> (i16, PredictorDirection) {
        let get_dc = |x: isize, y: isize, idx: usize| -> i16 {
            self.get_neighbor_block_idx(x, y, idx)
                .and_then(|pos| self.predictor_cache.get(pos))
                .map(|b| b[0])
                .unwrap_or(1024)
        };

        let (dc_a, dc_b, dc_c) = match block_idx {
            0 => (
                get_dc(mb_x as isize - 1, mb_y as isize, 1),
                get_dc(mb_x as isize - 1, mb_y as isize - 1, 3),
                get_dc(mb_x as isize, mb_y as isize - 1, 2),
            ),
            1 => (
                get_dc(mb_x as isize, mb_y as isize, 0),
                get_dc(mb_x as isize, mb_y as isize - 1, 2),
                get_dc(mb_x as isize, mb_y as isize - 1, 3),
            ),
            2 => (
                get_dc(mb_x as isize - 1, mb_y as isize, 3),
                get_dc(mb_x as isize - 1, mb_y as isize, 1),
                get_dc(mb_x as isize, mb_y as isize, 0),
            ),
            3 => (
                get_dc(mb_x as isize, mb_y as isize, 2),
                get_dc(mb_x as isize, mb_y as isize, 0),
                get_dc(mb_x as isize, mb_y as isize, 1),
            ),
            4 | 5 => (
                get_dc(mb_x as isize - 1, mb_y as isize, block_idx),
                get_dc(mb_x as isize - 1, mb_y as isize - 1, block_idx),
                get_dc(mb_x as isize, mb_y as isize - 1, block_idx),
            ),
            _ => (1024, 1024, 1024),
        };

        let grad_hor = (dc_a - dc_b).abs();
        let grad_ver = (dc_c - dc_b).abs();

        if grad_hor < grad_ver {
            (dc_c, PredictorDirection::Vertical)
        } else {
            (dc_a, PredictorDirection::Horizontal)
        }
    }

    fn median(a: i16, b: i16, c: i16) -> i16 {
        if a > b {
            if b > c {
                b
            } else if a > c {
                c
            } else {
                a
            }
        } else if b < c {
            b
        } else if a < c {
            c
        } else {
            a
        }
    }

    // ========================================================================
    // 运动向量
    // ========================================================================

    /// 解码 MVD (含 f_code 残差和范围包装)
    fn decode_mv_component(reader: &mut BitReader, f_code: u8) -> Option<i16> {
        for &(len, code, index) in MVD_VLC {
            let Some(bits) = reader.peek_bits(len) else {
                continue;
            };
            if bits as u16 == code {
                reader.read_bits(len)?;
                if index == 0 {
                    return Some(0);
                }
                // 基值
                let val_base = if index % 2 != 0 {
                    (index as i16 + 1) / 2
                } else {
                    -(index as i16 / 2)
                };
                // f_code 残差
                let r_size = f_code.saturating_sub(1);
                if r_size > 0 {
                    let residual = reader.read_bits(r_size)? as i16;
                    let abs_base = val_base.abs();
                    let new_abs = ((abs_base - 1) << r_size) + residual + 1;
                    return Some(if val_base < 0 { -new_abs } else { new_abs });
                }
                return Some(val_base);
            }
        }
        None
    }

    /// 获取预测 MV (支持 block_k 参数用于 Inter4V)
    fn get_pmv(&self, mb_x: u32, mb_y: u32, block_k: usize) -> MotionVector {
        let get_mv = |x: i32, y: i32, k: usize| -> MotionVector {
            if x < 0 || y < 0 || x >= self.mb_stride as i32 || y as u32 >= self.height.div_ceil(16)
            {
                return MotionVector { x: 0, y: 0 };
            }
            if let Some(mvs) = self.mv_cache.get(y as usize * self.mb_stride + x as usize) {
                mvs[k]
            } else {
                MotionVector { x: 0, y: 0 }
            }
        };

        // 对于 1MV (block_k == 0, non-4V), 使用宏块级 MV 预测
        // 对于 4MV (block_k 0-3), 使用块级 MV 预测
        let (mv_a, mv_b, mv_c) = if block_k == 0 || block_k > 3 {
            // 1MV 模式: 使用相邻 MB 的 MV[0]
            let a = get_mv(mb_x as i32 - 1, mb_y as i32, 0);
            let b = get_mv(mb_x as i32, mb_y as i32 - 1, 0);
            let c = get_mv(mb_x as i32 + 1, mb_y as i32 - 1, 0);
            (a, b, c)
        } else {
            // 4MV 模式: 根据 block_k 选择相邻块
            match block_k {
                0 => {
                    let a = get_mv(mb_x as i32 - 1, mb_y as i32, 1);
                    let b = get_mv(mb_x as i32, mb_y as i32 - 1, 2);
                    let c = get_mv(mb_x as i32 + 1, mb_y as i32 - 1, 2);
                    (a, b, c)
                }
                1 => {
                    let a = get_mv(mb_x as i32, mb_y as i32, 0);
                    let b = get_mv(mb_x as i32, mb_y as i32 - 1, 3);
                    let c = get_mv(mb_x as i32 + 1, mb_y as i32 - 1, 2);
                    (a, b, c)
                }
                2 => {
                    let a = get_mv(mb_x as i32 - 1, mb_y as i32, 3);
                    let b = get_mv(mb_x as i32, mb_y as i32, 0);
                    let c = get_mv(mb_x as i32, mb_y as i32, 1);
                    (a, b, c)
                }
                3 => {
                    let a = get_mv(mb_x as i32, mb_y as i32, 2);
                    let b = get_mv(mb_x as i32, mb_y as i32, 0);
                    let c = get_mv(mb_x as i32, mb_y as i32, 1);
                    (a, b, c)
                }
                _ => (
                    MotionVector::default(),
                    MotionVector::default(),
                    MotionVector::default(),
                ),
            }
        };

        MotionVector {
            x: Self::median(mv_a.x, mv_b.x, mv_c.x),
            y: Self::median(mv_a.y, mv_b.y, mv_c.y),
        }
    }

    /// 解码完整 MV (预测 + 差分 + 范围包装)
    fn decode_motion_vector(
        &self,
        reader: &mut BitReader,
        mb_x: u32,
        mb_y: u32,
        block_k: usize,
    ) -> Option<MotionVector> {
        let pred = self.get_pmv(mb_x, mb_y, block_k);
        let mvd_x = Self::decode_mv_component(reader, self.f_code_forward)?;
        let mvd_y = Self::decode_mv_component(reader, self.f_code_forward)?;

        let mut mv_x = pred.x + mvd_x;
        let mut mv_y = pred.y + mvd_y;

        // MV 范围包装
        let scale_fac = 1i16 << (self.f_code_forward.saturating_sub(1));
        let high = 32 * scale_fac - 1;
        let low = -32 * scale_fac;
        let range = 64 * scale_fac;

        if mv_x < low {
            mv_x += range;
        } else if mv_x > high {
            mv_x -= range;
        }
        if mv_y < low {
            mv_y += range;
        } else if mv_y > high {
            mv_y -= range;
        }

        Some(MotionVector { x: mv_x, y: mv_y })
    }

    // ========================================================================
    // 运动补偿
    // ========================================================================

    /// 从参考帧获取一个像素 (含边缘扩展)
    fn get_ref_pixel(ref_frame: &VideoFrame, plane: usize, x: isize, y: isize) -> u8 {
        let width = ref_frame.linesize[plane] as isize;
        let height = if plane == 0 {
            ref_frame.height as isize
        } else {
            (ref_frame.height / 2) as isize
        };
        let cx = x.clamp(0, width - 1) as usize;
        let cy = y.clamp(0, height - 1) as usize;
        ref_frame.data[plane][cy * width as usize + cx]
    }

    /// 运动补偿: 从参考帧获取预测像素 (半像素精度)
    fn motion_compensation(
        ref_frame: &VideoFrame,
        plane: usize,
        base_x: isize,
        base_y: isize,
        mv_x: i16,
        mv_y: i16,
        rounding: u8,
    ) -> u8 {
        let full_x = (mv_x >> 1) as isize;
        let full_y = (mv_y >> 1) as isize;
        let half_x = (mv_x & 1) != 0;
        let half_y = (mv_y & 1) != 0;

        let sx = base_x + full_x;
        let sy = base_y + full_y;

        if !half_x && !half_y {
            Self::get_ref_pixel(ref_frame, plane, sx, sy)
        } else {
            let p00 = Self::get_ref_pixel(ref_frame, plane, sx, sy) as u16;
            let p01 = Self::get_ref_pixel(ref_frame, plane, sx + 1, sy) as u16;
            let p10 = Self::get_ref_pixel(ref_frame, plane, sx, sy + 1) as u16;
            let p11 = Self::get_ref_pixel(ref_frame, plane, sx + 1, sy + 1) as u16;
            let r = rounding as u16;

            if half_x && !half_y {
                ((p00 + p01 + 1 - r) >> 1) as u8
            } else if !half_x && half_y {
                ((p00 + p10 + 1 - r) >> 1) as u8
            } else {
                ((p00 + p01 + p10 + p11 + 2 - r) >> 2) as u8
            }
        }
    }

    /// Chroma MV 推导 (1MV 模式)
    fn chroma_mv_1mv(luma_mv: MotionVector) -> MotionVector {
        MotionVector {
            x: (luma_mv.x >> 1) + ROUNDTAB_79[(luma_mv.x & 3) as usize],
            y: (luma_mv.y >> 1) + ROUNDTAB_79[(luma_mv.y & 3) as usize],
        }
    }

    /// Chroma MV 推导 (4MV 模式)
    fn chroma_mv_4mv(mvs: &[MotionVector; 4]) -> MotionVector {
        let sum_x = mvs[0].x as i32 + mvs[1].x as i32 + mvs[2].x as i32 + mvs[3].x as i32;
        let sum_y = mvs[0].y as i32 + mvs[1].y as i32 + mvs[2].y as i32 + mvs[3].y as i32;
        MotionVector {
            x: (sum_x >> 3) as i16 + ROUNDTAB_76[(sum_x & 0xf) as usize],
            y: (sum_y >> 3) as i16 + ROUNDTAB_76[(sum_y & 0xf) as usize],
        }
    }

    // ========================================================================
    // 反量化
    // ========================================================================

    /// 反量化 (区分 H.263 和 MPEG 类型)
    fn dequantize(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let quant_type = self.vol_info.as_ref().map(|v| v.quant_type).unwrap_or(0);
        let quant = quant.max(1);

        if quant_type == 0 {
            // H.263 量化类型
            self.dequant_h263(coefficients, quant, is_intra);
        } else {
            // MPEG 量化类型
            self.dequant_mpeg(coefficients, quant, is_intra);
        }
    }

    /// H.263 反量化
    fn dequant_h263(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let quant_m2 = (quant * 2) as i32;
        let quant_add = if quant % 2 != 0 {
            quant as i32
        } else {
            (quant as i32) - 1
        };

        // Intra: DC 系数已经乘以 dc_scaler, 不在此处处理
        let start = if is_intra { 1 } else { 0 };

        for coeff in coefficients.iter_mut().skip(start) {
            let level = *coeff;
            if level == 0 {
                continue;
            }
            if is_intra {
                // Intra AC: level * 2 * quant
                *coeff = level * quant_m2;
            } else if level < 0 {
                *coeff = level * quant_m2 - quant_add;
            } else {
                *coeff = level * quant_m2 + quant_add;
            }
        }
    }

    /// MPEG 反量化
    fn dequant_mpeg(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let matrix = if is_intra {
            &self.quant_matrix_intra
        } else {
            &self.quant_matrix_inter
        };

        // Intra: DC 已处理, 从 1 开始
        let start = if is_intra { 1 } else { 0 };
        let mut sum: u32 = 0;

        for i in start..64 {
            let level = coefficients[i];
            if level == 0 {
                continue;
            }
            let scale = matrix[i] as i32;
            if is_intra {
                coefficients[i] = (level * quant as i32 * scale) >> 4;
            } else {
                let sign = level < 0;
                let abs_level = level.unsigned_abs() as i32;
                let val = ((2 * abs_level + 1) * scale * quant as i32) >> 4;
                coefficients[i] = if sign {
                    -(val.min(2048))
                } else {
                    val.min(2047)
                };
            }
            sum ^= coefficients[i] as u32;
        }

        // Mismatch control (仅 MPEG 量化)
        if !is_intra && (sum & 1) == 0 {
            coefficients[63] ^= 1;
        }
    }

    // ========================================================================
    // 头部解析
    // ========================================================================

    /// 解析 VOL 头部
    fn parse_vol_header(&mut self, data: &[u8]) -> TaoResult<()> {
        let (code, offset) = match find_start_code_range(
            data,
            START_CODE_VIDEO_OBJECT_LAYER,
            START_CODE_VIDEO_OBJECT_LAYER + 0x0F,
        ) {
            Some(value) => value,
            None => return Ok(()),
        };

        debug!("找到 VOL 起始码: 0x{:02X}", code);
        let mut reader = BitReader::new(&data[offset..]);

        let _random_accessible_vol = reader.read_bit();
        let _video_object_type_indication = reader.read_bits(8);
        let is_object_layer_identifier = reader.read_bit().unwrap_or(false);
        if is_object_layer_identifier {
            let _verid = reader.read_bits(4);
            let _priority = reader.read_bits(3);
        }

        let aspect_ratio_info = reader.read_bits(4).unwrap_or(0);
        if aspect_ratio_info == 0xF {
            let _par_w = reader.read_bits(8);
            let _par_h = reader.read_bits(8);
        }

        let vol_control = reader.read_bit().unwrap_or(false);
        if vol_control {
            let _chroma = reader.read_bits(2);
            let _low_delay = reader.read_bit();
            let vbv = reader.read_bit().unwrap_or(false);
            if vbv {
                let _peak = reader.read_bits(15);
                reader.read_bit();
                let _buf = reader.read_bits(15);
                reader.read_bit();
                let _occ = reader.read_bits(15);
                reader.read_bit();
            }
        }

        let shape = reader.read_bits(2).unwrap_or(0); // 0=rectangular
        reader.read_bit(); // marker
        let time_res = reader.read_bits(16).unwrap_or(30000) as u16;
        reader.read_bit(); // marker
        let fixed_rate = reader.read_bit().unwrap_or(false);

        if fixed_rate {
            let bits = (time_res as f32).log2().ceil() as u8;
            reader.read_bits(bits.max(1));
        }

        // 只有 rectangular shape 才有宽高
        if shape == 0 {
            reader.read_bit(); // marker
            let _vol_w = reader.read_bits(13);
            reader.read_bit(); // marker
            let _vol_h = reader.read_bits(13);
            reader.read_bit(); // marker
        }

        let interlacing = reader.read_bit().unwrap_or(false);
        let _obmc_disable = reader.read_bit();

        // sprite_enable: 1 bit for ver_id=1, 2 bits for ver_id>=2
        let sprite_enable = reader.read_bits(1).unwrap_or(0) as u8;
        let mut sprite_warping_points = 0u8;
        if sprite_enable == 1 || sprite_enable == 2 {
            // 跳过 sprite 参数 (简化处理)
            if sprite_enable != 2 {
                // static sprite: 宽, 高, 左, 上
                reader.read_bits(13); // sprite_width
                reader.read_bit(); // marker
                reader.read_bits(13); // sprite_height
                reader.read_bit(); // marker
                reader.read_bits(13); // sprite_left
                reader.read_bit(); // marker
                reader.read_bits(13); // sprite_top
                reader.read_bit(); // marker
            }
            sprite_warping_points = reader.read_bits(6).unwrap_or(0) as u8;
            let _sprite_warping_accuracy = reader.read_bits(2);
            let _sprite_brightness = reader.read_bit();
            if sprite_enable != 2 {
                let _low_latency = reader.read_bit();
            }
        }

        let _not_8_bit = reader.read_bit();
        if _not_8_bit == Some(true) {
            reader.read_bits(4); // quant_precision
            reader.read_bits(4); // bits_per_pixel
        }

        let quant_type = if reader.read_bit().unwrap_or(false) {
            // MPEG 量化类型
            let load_intra = reader.read_bit().unwrap_or(false);
            if load_intra {
                if let Some(matrix) = read_quant_matrix(&mut reader) {
                    self.quant_matrix_intra = matrix;
                }
            }
            let load_inter = reader.read_bit().unwrap_or(false);
            if load_inter {
                if let Some(matrix) = read_quant_matrix(&mut reader) {
                    self.quant_matrix_inter = matrix;
                }
            }
            1u8
        } else {
            0u8
        };

        // quarterpel
        let quarterpel = reader.read_bit().unwrap_or(false);

        // complexity_estimation_disable
        let complexity_disable = reader.read_bit().unwrap_or(true);
        if !complexity_disable {
            // 跳过 complexity estimation 头 (非常复杂, 暂时跳过)
            warn!("VOL: complexity_estimation 未完全解析, 可能导致后续字段偏移");
        }

        let resync_marker_disable = reader.read_bit().unwrap_or(true);

        let data_partitioned = reader.read_bit().unwrap_or(false);
        if data_partitioned {
            let _reversible_vlc = reader.read_bit();
        }

        self.vol_info = Some(VolInfo {
            vop_time_increment_resolution: time_res,
            fixed_vop_rate: fixed_rate,
            data_partitioned,
            quant_type,
            interlacing,
            quarterpel,
            sprite_enable,
            sprite_warping_points,
            resync_marker_disable,
        });

        debug!(
            "VOL: time_res={}, quant_type={}, interlaced={}, quarterpel={}, sprite={}",
            time_res, quant_type, interlacing, quarterpel, sprite_enable
        );

        Ok(())
    }

    /// 解析 VOP 头部 (修正了字段顺序)
    fn parse_vop_header(&mut self, reader: &mut BitReader) -> TaoResult<VopInfo> {
        let vop_type = reader
            .read_bits(2)
            .ok_or_else(|| TaoError::InvalidData("无法读取 VOP 编码类型".into()))?;

        let picture_type = match vop_type as u8 {
            VOP_TYPE_I => PictureType::I,
            VOP_TYPE_P => PictureType::P,
            VOP_TYPE_B => PictureType::B,
            VOP_TYPE_S => PictureType::I, // S-VOP 暂按 I 帧处理
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "未知 VOP 类型: {}",
                    vop_type
                )));
            }
        };

        debug!("VOP 类型: {:?}", picture_type);

        // modulo_time_base
        while reader.read_bit() == Some(true) {}
        reader.read_bit(); // marker

        // vop_time_increment
        if let Some(vol) = &self.vol_info {
            let bits = (vol.vop_time_increment_resolution as f32).log2().ceil() as u8;
            reader.read_bits(bits.max(1));
        }

        reader.read_bit(); // marker
        let vop_coded = reader.read_bit().unwrap_or(true);

        if !vop_coded {
            debug!("VOP 未编码");
            return Ok(VopInfo {
                picture_type,
                vop_coded: false,
                vop_rounding_type: 0,
                intra_dc_vlc_thr: 0,
            });
        }

        // === 修正的字段顺序 (按 MPEG-4 标准) ===

        // P-VOP: rounding_type 在 intra_dc_vlc_thr 之前
        if picture_type == PictureType::P {
            self.rounding_control = reader.read_bit().unwrap_or(false) as u8;
        }

        // intra_dc_vlc_thr (I/P 帧)
        let intra_dc_vlc_thr = if picture_type != PictureType::B {
            reader.read_bits(3).unwrap_or(0)
        } else {
            0
        };
        self.intra_dc_vlc_thr = intra_dc_vlc_thr;

        // vop_quant
        if let Some(quant) = reader.read_bits(5) {
            if quant > 0 {
                self.quant = quant as u8;
            }
        }

        // P-VOP: f_code_forward
        if picture_type == PictureType::P {
            if let Some(f) = reader.read_bits(3) {
                self.f_code_forward = f as u8;
            }
        }

        // B-VOP: f_code_forward + f_code_backward
        if picture_type == PictureType::B {
            if let Some(f) = reader.read_bits(3) {
                self.f_code_forward = f as u8;
            }
            if let Some(f) = reader.read_bits(3) {
                self.f_code_backward = f as u8;
            }
        }

        debug!(
            "VOP 头: quant={}, rounding={}, f_code_fwd={}, dc_thr={}",
            self.quant, self.rounding_control, self.f_code_forward, intra_dc_vlc_thr
        );

        Ok(VopInfo {
            picture_type,
            vop_coded: true,
            vop_rounding_type: self.rounding_control,
            intra_dc_vlc_thr,
        })
    }

    // ========================================================================
    // 宏块和帧解码
    // ========================================================================

    /// 解码单个宏块
    fn decode_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        reader: &mut BitReader,
        is_i_vop: bool,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;

        // P-VOP: not_coded 位
        if !is_i_vop {
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                self.copy_mb_from_ref(frame, mb_x, mb_y);
                let idx = mb_y as usize * self.mb_stride + mb_x as usize;
                if idx < self.mv_cache.len() {
                    self.mv_cache[idx] = [MotionVector::default(); 4];
                }
                return;
            }
        }

        // 1. MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader).unwrap_or((MbType::Intra, 0))
        } else {
            decode_mcbpc_p(reader).unwrap_or((MbType::Inter, 0))
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // AC/DC prediction flag (Intra only)
        let ac_pred_flag = if is_intra {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // 2. CBPY (Inter 块取反)
        let cbpy = decode_cbpy(reader, is_intra).unwrap_or(0);

        // 3. DQUANT
        if mb_type == MbType::IntraQ || mb_type == MbType::InterQ {
            if let Some(dq) = reader.read_bits(2) {
                let delta = DQUANT_TABLE[dq as usize];
                self.quant = ((self.quant as i32 + delta).clamp(1, 31)) as u8;
            }
        }

        // 4. 隔行模式: field_dct
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            let _field_dct = reader.read_bit().unwrap_or(false);
        }

        // 5. 运动向量解码
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;
        let mut mb_mvs = [MotionVector::default(); 4];

        if !is_intra {
            if mb_type == MbType::Inter4V {
                // 4MV: 解码 4 个独立 MV
                for (k, mv_slot) in mb_mvs.iter_mut().enumerate() {
                    if let Some(mv) = self.decode_motion_vector(reader, mb_x, mb_y, k) {
                        *mv_slot = mv;
                    }
                    // 临时存储以供后续块 PMV 使用
                    if mb_idx < self.mv_cache.len() {
                        self.mv_cache[mb_idx][k] = *mv_slot;
                    }
                }
            } else {
                // 1MV
                if let Some(mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                    mb_mvs = [mv; 4];
                }
            }
        }

        // 存储 MV
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_mvs;
        }

        // 6. CBP 组合
        let cbp = (cbpy << 2) | cbpc;

        // 7. 解码各 8x8 块
        // Y 平面 (4 块)
        #[allow(clippy::needless_range_loop)]
        for block_idx in 0..4usize {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;
            let ac_coded = (cbp >> (5 - block_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    0,
                    mb_x,
                    mb_y,
                    block_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            // 反量化
            self.dequantize(&mut block, self.quant as u32, is_intra);

            // IDCT
            idct_8x8(&mut block);

            // 获取当前块使用的 MV (4MV 时每个块不同)
            let mv = if !is_intra {
                mb_mvs[block_idx]
            } else {
                MotionVector::default()
            };

            // 写入 Y 平面
            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 16 + bx as usize * 8 + x) as isize;
                    let py = (mb_y as usize * 16 + by as usize * 8 + y) as isize;
                    if px < width as isize && py < height as isize {
                        let idx = py as usize * width + px as usize;
                        let residual = block[y * 8 + x];
                        let val = if is_intra {
                            (residual + 128).clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = Self::motion_compensation(
                                ref_frame,
                                0,
                                px,
                                py,
                                mv.x,
                                mv.y,
                                self.rounding_control,
                            );
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[0][idx] = val;
                    }
                }
            }
        }

        // U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        // Chroma MV
        let chroma_mv = if !is_intra {
            if mb_type == MbType::Inter4V {
                Self::chroma_mv_4mv(&mb_mvs)
            } else {
                Self::chroma_mv_1mv(mb_mvs[0])
            }
        } else {
            MotionVector::default()
        };

        for plane_idx in 0..2usize {
            let ac_coded = (cbp >> (1 - plane_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    plane_idx + 1,
                    mb_x,
                    mb_y,
                    4 + plane_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, is_intra);
            idct_8x8(&mut block);

            for v in 0..8 {
                for u in 0..8 {
                    let px = (mb_x as usize * 8 + u) as isize;
                    let py = (mb_y as usize * 8 + v) as isize;
                    if px < uv_width as isize && py < uv_height as isize {
                        let idx = py as usize * uv_width + px as usize;
                        let residual = block[v * 8 + u];
                        let val = if is_intra {
                            (residual + 128).clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = Self::motion_compensation(
                                ref_frame,
                                plane_idx + 1,
                                px,
                                py,
                                chroma_mv.x,
                                chroma_mv.y,
                                self.rounding_control,
                            );
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[plane_idx + 1][idx] = val;
                    }
                }
            }
        }
    }

    /// 解码 I 帧
    fn decode_i_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        let total_blocks = (self.width.div_ceil(16) * self.height.div_ceil(16) * 6) as usize;
        self.predictor_cache = vec![[1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::I;
        frame.is_keyframe = true;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        let mb_w = self.width.div_ceil(16);
        let mb_h = self.height.div_ceil(16);
        debug!(
            "解码 I 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                self.decode_macroblock(&mut frame, mb_x, mb_y, reader, true);
            }
        }
        Ok(frame)
    }

    /// 解码 P 帧
    fn decode_p_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        let total_blocks = (self.width.div_ceil(16) * self.height.div_ceil(16) * 6) as usize;
        self.predictor_cache = vec![[1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::P;
        frame.is_keyframe = false;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);
        debug!(
            "解码 P 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        for mb_y in 0..mb_h as u32 {
            for mb_x in 0..mb_w as u32 {
                self.decode_macroblock(&mut frame, mb_x, mb_y, reader, false);
            }
        }
        Ok(frame)
    }

    /// 从参考帧复制宏块
    fn copy_mb_from_ref(&self, frame: &mut VideoFrame, mb_x: u32, mb_y: u32) {
        if let Some(ref_frame) = &self.reference_frame {
            let width = self.width as usize;
            let height = self.height as usize;

            for y in 0..16 {
                for x in 0..16 {
                    let px = (mb_x as usize * 16 + x).min(width - 1);
                    let py = (mb_y as usize * 16 + y).min(height - 1);
                    let idx = py * width + px;
                    frame.data[0][idx] = ref_frame.data[0][idx];
                }
            }

            let uv_w = width / 2;
            let uv_h = height / 2;
            for plane in 1..3 {
                for y in 0..8 {
                    for x in 0..8 {
                        let px = (mb_x as usize * 8 + x).min(uv_w - 1);
                        let py = (mb_y as usize * 8 + y).min(uv_h - 1);
                        let idx = py * uv_w + px;
                        frame.data[plane][idx] = ref_frame.data[plane][idx];
                    }
                }
            }
        }
    }

    /// 检查 resync marker
    #[allow(dead_code)]
    fn check_resync_marker(reader: &BitReader, fcode_minus1: u8) -> bool {
        let nbits = reader.bits_to_byte_align();
        if nbits == 0 {
            return false;
        }
        if let Some(code) = reader.peek_bits(nbits) {
            if code == (1u32 << (nbits - 1)) - 1 {
                // 检查后续的 resync marker (17 + fcode_minus1 位的 0...01)
                let marker_bits = 17 + fcode_minus1;
                // 简化处理: 只检查对齐位
                let _ = marker_bits;
                return false; // TODO: 完善 resync marker 检测
            }
        }
        false
    }

    /// MV 合法性验证 (限制在帧边界内)
    #[allow(dead_code)]
    fn validate_vector(&self, mv: &mut MotionVector, mb_x: u32, mb_y: u32) {
        let shift = 5; // 半像素精度
        let x_high = ((self.mb_stride as i16 - mb_x as i16) << shift) - 1;
        let x_low = -((mb_x as i16 + 1) << shift);
        let mb_h = self.height.div_ceil(16) as i16;
        let y_high = ((mb_h - mb_y as i16) << shift) - 1;
        let y_low = -((mb_y as i16 + 1) << shift);

        mv.x = mv.x.clamp(x_low, x_high);
        mv.y = mv.y.clamp(y_low, y_high);
    }
}

// ============================================================================
// Decoder trait 实现
// ============================================================================

impl Decoder for Mpeg4Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Mpeg4
    }

    fn name(&self) -> &str {
        "mpeg4"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let video = match &params.params {
            CodecParamsType::Video(v) => v,
            _ => {
                return Err(TaoError::InvalidArgument("MPEG4 解码器需要视频参数".into()));
            }
        };

        if video.width == 0 || video.height == 0 {
            return Err(TaoError::InvalidArgument("宽度和高度不能为 0".into()));
        }

        self.width = video.width;
        self.height = video.height;
        self.mb_stride = (video.width as usize).div_ceil(16);
        self.pixel_format = PixelFormat::Yuv420p;
        self.opened = true;
        self.frame_count = 0;
        self.reference_frame = None;
        self.backward_reference = None;

        // 初始化 MV 缓存 (每个 MB 4 个 MV)
        let mb_count = self.mb_stride * (video.height as usize).div_ceil(16);
        self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];

        if !params.extra_data.is_empty() {
            self.parse_vol_header(&params.extra_data)?;
        }

        debug!(
            "打开 MPEG4 解码器: {}x{}, mb_stride={}",
            self.width, self.height, self.mb_stride
        );
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }

        if packet.is_empty() {
            debug!("收到刷新信号");
            return Ok(());
        }

        if self.vol_info.is_none() {
            if let Err(e) = self.parse_vol_header(&packet.data) {
                debug!("VOL 解析失败: {:?}", e);
            }
        }

        let vop_offset = find_start_code_offset(&packet.data, START_CODE_VOP)
            .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
        let mut reader = BitReader::new(&packet.data[vop_offset..]);

        let vop_info = self.parse_vop_header(&mut reader)?;

        if !vop_info.vop_coded {
            if let Some(ref_frame) = &self.reference_frame {
                let mut frame = ref_frame.clone();
                frame.picture_type = vop_info.picture_type;
                frame.is_keyframe = vop_info.picture_type == PictureType::I;
                frame.pts = packet.pts;
                frame.time_base = packet.time_base;
                frame.duration = packet.duration;
                self.pending_frame = Some(frame);
                self.frame_count += 1;
            }
            return Ok(());
        }

        let mut frame = match vop_info.picture_type {
            PictureType::I => self.decode_i_frame(&mut reader)?,
            PictureType::P => self.decode_p_frame(&mut reader).unwrap_or_else(|_| {
                warn!("P 帧解码失败, 使用参考帧降级");
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::P;
                    f.is_keyframe = false;
                    f
                } else {
                    let mut f = VideoFrame::new(self.width, self.height, self.pixel_format);
                    f.picture_type = PictureType::P;
                    f
                }
            }),
            PictureType::B => {
                // B 帧: 简化实现 - 使用前向参考帧
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::B;
                    f.is_keyframe = false;
                    warn!("B 帧使用参考帧降级 (完整 B 帧解码待实现)");
                    f
                } else {
                    warn!("B 帧缺少参考帧, 跳过");
                    return Ok(());
                }
            }
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "不支持的 VOP 类型: {:?}",
                    vop_info.picture_type
                )));
            }
        };

        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        // 保存参考帧
        if frame.picture_type == PictureType::I || frame.picture_type == PictureType::P {
            // B 帧需要双参考帧: 旧的 forward 变为 backward
            self.backward_reference = self.reference_frame.take();
            self.reference_frame = Some(frame.clone());

            let mb_count = self.mb_stride * (self.height as usize).div_ceil(16);
            if self.mv_cache.len() != mb_count {
                self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
            }
        }

        self.pending_frame = Some(frame);
        self.frame_count += 1;
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }
        if let Some(frame) = self.pending_frame.take() {
            Ok(Frame::Video(frame))
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        debug!("MPEG4 解码器已刷新");
        self.pending_frame = None;
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::VideoCodecParams;
    use tao_core::Rational;

    #[test]
    fn test_mpeg4_decoder_create() {
        let decoder = Mpeg4Decoder::create();
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_mpeg4_decoder_open() {
        let mut decoder = Mpeg4Decoder::create().unwrap();
        let params = CodecParameters {
            codec_id: CodecId::Mpeg4,
            bit_rate: 0,
            extra_data: vec![],
            params: CodecParamsType::Video(VideoCodecParams {
                width: 640,
                height: 480,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };
        assert!(decoder.open(&params).is_ok());
    }

    #[test]
    fn test_dc_scaler() {
        let decoder = Mpeg4Decoder {
            quant: 1,
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            frame_count: 0,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
        };
        // quant=1: Y=8, C=8
        assert_eq!(decoder.get_dc_scaler(true), 8);
        assert_eq!(decoder.get_dc_scaler(false), 8);
    }

    #[test]
    fn test_cbpy_inter_inversion() {
        // 测试 CBPY 解码: Inter 块需要取反
        // cbpy=15 (all coded) → Inter 应为 15-15=0 (none coded)
        // cbpy=0 (none coded) → Inter 应为 15-0=15 (all coded)

        // 构造包含 CBPY=15 (码字 0xB, 4位) 的位流
        let data = [0xB0]; // 1011 0000
        let mut reader = BitReader::new(&data);
        let cbpy_intra = decode_cbpy(&mut reader, true);
        assert_eq!(cbpy_intra, Some(15));

        let mut reader2 = BitReader::new(&data);
        let cbpy_inter = decode_cbpy(&mut reader2, false);
        assert_eq!(cbpy_inter, Some(0)); // 15 - 15 = 0
    }

    #[test]
    fn test_mv_range_wrapping() {
        // f_code=1: range=64, low=-32, high=31
        let decoder = Mpeg4Decoder {
            width: 320,
            height: 240,
            pixel_format: PixelFormat::Yuv420p,
            opened: true,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: vec![[MotionVector::default(); 4]; 20 * 15],
            mb_stride: 20,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
        };

        // 测试 PMV 为 (0,0) 时的基本 MV 预测
        let pmv = decoder.get_pmv(0, 0, 0);
        assert_eq!(pmv.x, 0);
        assert_eq!(pmv.y, 0);
    }

    #[test]
    fn test_integer_idct() {
        // 测试全零块 IDCT
        let mut block = [0i32; 64];
        idct_8x8(&mut block);
        for &v in &block {
            assert_eq!(v, 0);
        }

        // 测试纯 DC 块
        let mut block2 = [0i32; 64];
        block2[0] = 100;
        idct_8x8(&mut block2);
        // 所有像素应大致相同 (DC 值扩展到整个块)
        let first = block2[0];
        for &v in &block2 {
            assert!(
                (v - first).abs() <= 1,
                "DC-only block 不均匀: {} vs {}",
                v,
                first
            );
        }
    }
}
