//! AAC Huffman 码本数据和解码器.
//!
//! 基于 ISO/IEC 14496-3 定义的 Huffman 码本.
//! 频谱码本数据来源: FFmpeg (libavcodec/aactab.c, LGPL-2.1).

use tao_core::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

// ============================================================
// Huffman 二叉树
// ============================================================

/// Huffman 二叉树 (运行时构建)
pub struct HuffTree {
    /// 节点数组: 每个节点 = [bit0_child, bit1_child]
    /// 正值: 子节点索引
    /// 负值: 叶子, 解码值 = -(value + 1)
    nodes: Vec<[i32; 2]>,
}

impl HuffTree {
    /// 从 (码字, 码字长度, 叶子值) 表构建二叉树
    pub fn build(entries: &[(u32, u8, i32)]) -> Self {
        let mut nodes = vec![[0i32; 2]]; // 根节点
        for &(code, len, value) in entries {
            let leaf = -(value + 1);
            let mut idx = 0usize;
            for bit_pos in (0..len).rev() {
                let bit = ((code >> bit_pos) & 1) as usize;
                if bit_pos == 0 {
                    nodes[idx][bit] = leaf;
                } else if nodes[idx][bit] > 0 {
                    idx = nodes[idx][bit] as usize;
                } else {
                    let new_idx = nodes.len();
                    nodes.push([0; 2]);
                    nodes[idx][bit] = new_idx as i32;
                    idx = new_idx;
                }
            }
        }
        Self { nodes }
    }

    /// 从比特流解码一个值
    pub fn decode(&self, br: &mut BitReader) -> TaoResult<i32> {
        let mut idx = 0usize;
        for _ in 0..20 {
            let bit = br.read_bit()? as usize;
            let child = self.nodes[idx][bit];
            if child < 0 {
                return Ok(-(child + 1));
            }
            if child == 0 {
                return Err(TaoError::InvalidData("AAC: Huffman 树无效节点".into()));
            }
            idx = child as usize;
        }
        Err(TaoError::InvalidData("AAC: Huffman 码字超过 20 位".into()))
    }
}

// ============================================================
// 频谱码本
// ============================================================

/// 频谱码本 (用于解码频谱系数)
pub struct SpectralCodebook {
    /// Huffman 树
    tree: HuffTree,
    /// 每个码字对应的值元组 (最多 4 个)
    values: Vec<[i16; 4]>,
    /// 维度 (2=pair, 4=quad)
    pub dim: usize,
    /// 值是否自带符号 (true: CB1/2/5/6, false: 其他需读符号位)
    is_signed: bool,
    /// 是否为 ESC 码本 (CB11)
    is_esc: bool,
    /// ESC 触发值 (仅 CB11 使用, 值 = 16)
    esc_val: i16,
}

impl SpectralCodebook {
    /// 解码一组频谱值
    pub fn decode_values(&self, br: &mut BitReader) -> TaoResult<[i32; 4]> {
        let entry_idx = self.tree.decode(br)? as usize;
        if entry_idx >= self.values.len() {
            return Err(TaoError::InvalidData("AAC: 频谱 Huffman 索引越界".into()));
        }
        let raw = &self.values[entry_idx];
        let mut out = [0i32; 4];
        for i in 0..self.dim {
            let v = raw[i] as i32;
            if self.is_signed {
                // 有符号码本: 值已包含正确符号
                out[i] = v;
            } else if v != 0 {
                // 无符号码本: 读取 1 位符号
                let sign_bit = br.read_bit()?;
                out[i] = if sign_bit != 0 { -v } else { v };
            }
            // ESC 码本: magnitude == esc_val 时读取 escape 序列
            if self.is_esc && out[i].unsigned_abs() as i16 == self.esc_val {
                let esc_mag = Self::read_escape(br)?;
                out[i] = if out[i] < 0 { -esc_mag } else { esc_mag };
            }
        }
        Ok(out)
    }

    /// 读取 AAC escape 序列 (ISO 14496-3, 4.6.3.3)
    fn read_escape(br: &mut BitReader) -> TaoResult<i32> {
        let mut n = 4u32;
        while br.read_bit()? != 0 {
            n += 1;
            if n > 15 {
                return Err(TaoError::InvalidData("AAC: ESC 指数超过限制".into()));
            }
        }
        let mantissa = br.read_bits(n)? as i32;
        Ok((1 << n) + mantissa)
    }
}

// ============================================================
// 码本构建
// ============================================================

/// 所有 AAC Huffman 码本集合
pub struct AacCodebooks {
    /// Scale factor Huffman 树
    pub sf_tree: HuffTree,
    /// 频谱码本 1-11 (索引 0=CB1, ..., 10=CB11)
    pub spectral: [Option<SpectralCodebook>; 11],
}

impl AacCodebooks {
    /// 构建所有码本 (在解码器打开时调用一次)
    pub fn build() -> Self {
        Self {
            sf_tree: HuffTree::build(&SF_TABLE),
            spectral: [
                Some(build_cb(&CODES_1, &BITS_1, 4, 3, -1, true, false)),
                Some(build_cb(&CODES_2, &BITS_2, 4, 3, -1, true, false)),
                Some(build_cb(&CODES_3, &BITS_3, 4, 3, 0, false, false)),
                Some(build_cb(&CODES_4, &BITS_4, 4, 3, 0, false, false)),
                Some(build_cb(&CODES_5, &BITS_5, 2, 9, -4, true, false)),
                Some(build_cb(&CODES_6, &BITS_6, 2, 9, -4, true, false)),
                Some(build_cb(&CODES_7, &BITS_7, 2, 8, 0, false, false)),
                Some(build_cb(&CODES_8, &BITS_8, 2, 8, 0, false, false)),
                Some(build_cb(&CODES_9, &BITS_9, 2, 13, 0, false, false)),
                Some(build_cb(&CODES_10, &BITS_10, 2, 13, 0, false, false)),
                Some(build_cb(&CODES_11, &BITS_11, 2, 17, 0, false, true)),
            ],
        }
    }
}

/// 构建频谱码本
///
/// - `codes`/`bits`: FFmpeg 格式的码字/码字长度数组
/// - `dim`: 维度 (4=quad, 2=pair)
/// - `mod_val`: 每维的值个数 (base)
/// - `offset`: 值偏移 (有符号码本用于居中)
/// - `is_signed`: true 表示值已含符号
/// - `is_esc`: true 表示 ESC 码本 (CB11)
fn build_cb(
    codes: &[u16],
    bits: &[u8],
    dim: usize,
    mod_val: usize,
    offset: i16,
    is_signed: bool,
    is_esc: bool,
) -> SpectralCodebook {
    let num_entries = codes.len();
    let mut tree_entries = Vec::with_capacity(num_entries);
    let mut values = Vec::with_capacity(num_entries);

    for i in 0..num_entries {
        tree_entries.push((codes[i] as u32, bits[i], i as i32));
        values.push(index_to_values(i, dim, mod_val, offset));
    }

    let esc_val = if is_esc { (mod_val - 1) as i16 } else { 0 };

    SpectralCodebook {
        tree: HuffTree::build(&tree_entries),
        values,
        dim,
        is_signed,
        is_esc,
        esc_val,
    }
}

/// 将线性索引转换为值元组
///
/// 例: CB7 (dim=2, mod=8, offset=0):
///   index=9 → v0=9/8=1, v1=9%8=1 → [1, 1, 0, 0]
fn index_to_values(idx: usize, dim: usize, mod_val: usize, offset: i16) -> [i16; 4] {
    let mut vals = [0i16; 4];
    if dim == 4 {
        let m3 = mod_val * mod_val * mod_val;
        let m2 = mod_val * mod_val;
        vals[0] = (idx / m3) as i16 + offset;
        vals[1] = ((idx / m2) % mod_val) as i16 + offset;
        vals[2] = ((idx / mod_val) % mod_val) as i16 + offset;
        vals[3] = (idx % mod_val) as i16 + offset;
    } else {
        vals[0] = (idx / mod_val) as i16 + offset;
        vals[1] = (idx % mod_val) as i16 + offset;
    }
    vals
}

// ============================================================
// Scale Factor Huffman 表 (ISO 14496-3 Table 4.A.1)
// (码字, 码字长度, SF 索引 0-120, delta = index - 60)
// ============================================================

#[rustfmt::skip]
const SF_TABLE: [(u32, u8, i32); 121] = [
    (0x3FFE8, 18, 0), (0x3FFE6, 18, 1), (0x3FFE7, 18, 2), (0x3FFE5, 18, 3),
    (0x7FFF5, 19, 4), (0x7FFF1, 19, 5), (0x7FFED, 19, 6), (0x7FFF6, 19, 7),
    (0x7FFEE, 19, 8), (0x7FFEF, 19, 9), (0x7FFF0, 19, 10), (0x7FFFC, 19, 11),
    (0x7FFFD, 19, 12), (0x7FFFF, 19, 13), (0x7FFFE, 19, 14), (0x7FFF7, 19, 15),
    (0x7FFF8, 19, 16), (0x7FFFB, 19, 17), (0x7FFF9, 19, 18), (0x3FFE4, 18, 19),
    (0x7FFFA, 19, 20), (0x3FFE3, 18, 21), (0x1FFEF, 17, 22), (0x1FFF0, 17, 23),
    (0xFFF5, 16, 24), (0x1FFEE, 17, 25), (0xFFF2, 16, 26), (0xFFF3, 16, 27),
    (0xFFF4, 16, 28), (0xFFF1, 16, 29), (0x7FF6, 15, 30), (0x7FF7, 15, 31),
    (0x3FF9, 14, 32), (0x3FF5, 14, 33), (0x3FF7, 14, 34), (0x3FF3, 14, 35),
    (0x3FF6, 14, 36), (0x3FF2, 14, 37), (0x1FF7, 13, 38), (0x1FF5, 13, 39),
    (0xFF9, 12, 40), (0xFF7, 12, 41), (0xFF6, 12, 42), (0x7F9, 11, 43),
    (0xFF4, 12, 44), (0x7F8, 11, 45), (0x3F9, 10, 46), (0x3F7, 10, 47),
    (0x3F5, 10, 48), (0x1F8, 9, 49), (0x1F7, 9, 50), (0xFA, 8, 51),
    (0xF8, 8, 52), (0xF6, 8, 53), (0x79, 7, 54), (0x3A, 6, 55),
    (0x38, 6, 56), (0x1A, 5, 57), (0xB, 4, 58), (0x4, 3, 59),
    (0x0, 1, 60),
    (0xA, 4, 61), (0xC, 4, 62), (0x1B, 5, 63), (0x39, 6, 64),
    (0x3B, 6, 65), (0x78, 7, 66), (0x7A, 7, 67), (0xF7, 8, 68),
    (0xF9, 8, 69), (0x1F6, 9, 70), (0x1F9, 9, 71), (0x3F4, 10, 72),
    (0x3F6, 10, 73), (0x3F8, 10, 74), (0x7F5, 11, 75), (0x7F4, 11, 76),
    (0x7F6, 11, 77), (0x7F7, 11, 78), (0xFF5, 12, 79), (0xFF8, 12, 80),
    (0x1FF4, 13, 81), (0x1FF6, 13, 82), (0x1FF8, 13, 83), (0x3FF8, 14, 84),
    (0x3FF4, 14, 85), (0xFFF0, 16, 86), (0x7FF4, 15, 87), (0xFFF6, 16, 88),
    (0x7FF5, 15, 89), (0x3FFE2, 18, 90), (0x7FFD9, 19, 91), (0x7FFDA, 19, 92),
    (0x7FFDB, 19, 93), (0x7FFDC, 19, 94), (0x7FFDD, 19, 95), (0x7FFDE, 19, 96),
    (0x7FFD8, 19, 97), (0x7FFD2, 19, 98), (0x7FFD3, 19, 99), (0x7FFD4, 19, 100),
    (0x7FFD5, 19, 101), (0x7FFD6, 19, 102), (0x7FFF2, 19, 103), (0x7FFDF, 19, 104),
    (0x7FFE7, 19, 105), (0x7FFE8, 19, 106), (0x7FFE9, 19, 107), (0x7FFEA, 19, 108),
    (0x7FFEB, 19, 109), (0x7FFE6, 19, 110), (0x7FFE0, 19, 111), (0x7FFE1, 19, 112),
    (0x7FFE2, 19, 113), (0x7FFE3, 19, 114), (0x7FFE4, 19, 115), (0x7FFE5, 19, 116),
    (0x7FFD7, 19, 117), (0x7FFEC, 19, 118), (0x7FFF4, 19, 119), (0x7FFF3, 19, 120),
];

// ============================================================
// 频谱码本表数据 (来源: FFmpeg libavcodec/aactab.c, LGPL-2.1)
// ============================================================

// --- CB1: dim=4, signed, LAV=1, 81 entries ---

#[rustfmt::skip]
const CODES_1: [u16; 81] = [
    0x7f8, 0x1f1, 0x7fd, 0x3f5, 0x068, 0x3f0, 0x7f7, 0x1ec,
    0x7f5, 0x3f1, 0x072, 0x3f4, 0x074, 0x011, 0x076, 0x1eb,
    0x06c, 0x3f6, 0x7fc, 0x1e1, 0x7f1, 0x1f0, 0x061, 0x1f6,
    0x7f2, 0x1ea, 0x7fb, 0x1f2, 0x069, 0x1ed, 0x077, 0x017,
    0x06f, 0x1e6, 0x064, 0x1e5, 0x067, 0x015, 0x062, 0x012,
    0x000, 0x014, 0x065, 0x016, 0x06d, 0x1e9, 0x063, 0x1e4,
    0x06b, 0x013, 0x071, 0x1e3, 0x070, 0x1f3, 0x7fe, 0x1e7,
    0x7f3, 0x1ef, 0x060, 0x1ee, 0x7f0, 0x1e2, 0x7fa, 0x3f3,
    0x06a, 0x1e8, 0x075, 0x010, 0x073, 0x1f4, 0x06e, 0x3f7,
    0x7f6, 0x1e0, 0x7f9, 0x3f2, 0x066, 0x1f5, 0x7ff, 0x1f7,
    0x7f4,
];

#[rustfmt::skip]
const BITS_1: [u8; 81] = [
    11,  9, 11, 10,  7, 10, 11,  9, 11, 10,  7, 10,  7,  5,  7,  9,
     7, 10, 11,  9, 11,  9,  7,  9, 11,  9, 11,  9,  7,  9,  7,  5,
     7,  9,  7,  9,  7,  5,  7,  5,  1,  5,  7,  5,  7,  9,  7,  9,
     7,  5,  7,  9,  7,  9, 11,  9, 11,  9,  7,  9, 11,  9, 11, 10,
     7,  9,  7,  5,  7,  9,  7, 10, 11,  9, 11, 10,  7,  9, 11,  9,
    11,
];

// --- CB2: dim=4, signed, LAV=1, 81 entries ---

#[rustfmt::skip]
const CODES_2: [u16; 81] = [
    0x1f3, 0x06f, 0x1fd, 0x0eb, 0x023, 0x0ea, 0x1f7, 0x0e8,
    0x1fa, 0x0f2, 0x02d, 0x070, 0x020, 0x006, 0x02b, 0x06e,
    0x028, 0x0e9, 0x1f9, 0x066, 0x0f8, 0x0e7, 0x01b, 0x0f1,
    0x1f4, 0x06b, 0x1f5, 0x0ec, 0x02a, 0x06c, 0x02c, 0x00a,
    0x027, 0x067, 0x01a, 0x0f5, 0x024, 0x008, 0x01f, 0x009,
    0x000, 0x007, 0x01d, 0x00b, 0x030, 0x0ef, 0x01c, 0x064,
    0x01e, 0x00c, 0x029, 0x0f3, 0x02f, 0x0f0, 0x1fc, 0x071,
    0x1f2, 0x0f4, 0x021, 0x0e6, 0x0f7, 0x068, 0x1f8, 0x0ee,
    0x022, 0x065, 0x031, 0x002, 0x026, 0x0ed, 0x025, 0x06a,
    0x1fb, 0x072, 0x1fe, 0x069, 0x02e, 0x0f6, 0x1ff, 0x06d,
    0x1f6,
];

#[rustfmt::skip]
const BITS_2: [u8; 81] = [
     9,  7,  9,  8,  6,  8,  9,  8,  9,  8,  6,  7,  6,  5,  6,  7,
     6,  8,  9,  7,  8,  8,  6,  8,  9,  7,  9,  8,  6,  7,  6,  5,
     6,  7,  6,  8,  6,  5,  6,  5,  3,  5,  6,  5,  6,  8,  6,  7,
     6,  5,  6,  8,  6,  8,  9,  7,  9,  8,  6,  8,  8,  7,  9,  8,
     6,  7,  6,  4,  6,  8,  6,  7,  9,  7,  9,  7,  6,  8,  9,  7,
     9,
];

// --- CB3: dim=4, unsigned, LAV=2, 81 entries ---

#[rustfmt::skip]
const CODES_3: [u16; 81] = [
    0x0000, 0x0009, 0x00ef, 0x000b, 0x0019, 0x00f0, 0x01eb, 0x01e6,
    0x03f2, 0x000a, 0x0035, 0x01ef, 0x0034, 0x0037, 0x01e9, 0x01ed,
    0x01e7, 0x03f3, 0x01ee, 0x03ed, 0x1ffa, 0x01ec, 0x01f2, 0x07f9,
    0x07f8, 0x03f8, 0x0ff8, 0x0008, 0x0038, 0x03f6, 0x0036, 0x0075,
    0x03f1, 0x03eb, 0x03ec, 0x0ff4, 0x0018, 0x0076, 0x07f4, 0x0039,
    0x0074, 0x03ef, 0x01f3, 0x01f4, 0x07f6, 0x01e8, 0x03ea, 0x1ffc,
    0x00f2, 0x01f1, 0x0ffb, 0x03f5, 0x07f3, 0x0ffc, 0x00ee, 0x03f7,
    0x7ffe, 0x01f0, 0x07f5, 0x7ffd, 0x1ffb, 0x3ffa, 0xffff, 0x00f1,
    0x03f0, 0x3ffc, 0x01ea, 0x03ee, 0x3ffb, 0x0ff6, 0x0ffa, 0x7ffc,
    0x07f2, 0x0ff5, 0xfffe, 0x03f4, 0x07f7, 0x7ffb, 0x0ff7, 0x0ff9,
    0x7ffa,
];

#[rustfmt::skip]
const BITS_3: [u8; 81] = [
     1,  4,  8,  4,  5,  8,  9,  9, 10,  4,  6,  9,  6,  6,  9,  9,
     9, 10,  9, 10, 13,  9,  9, 11, 11, 10, 12,  4,  6, 10,  6,  7,
    10, 10, 10, 12,  5,  7, 11,  6,  7, 10,  9,  9, 11,  9, 10, 13,
     8,  9, 12, 10, 11, 12,  8, 10, 15,  9, 11, 15, 13, 14, 16,  8,
    10, 14,  9, 10, 14, 12, 12, 15, 11, 12, 16, 10, 11, 15, 12, 12,
    15,
];

// --- CB4: dim=4, unsigned, LAV=2, 81 entries ---

#[rustfmt::skip]
const CODES_4: [u16; 81] = [
    0x007, 0x016, 0x0f6, 0x018, 0x008, 0x0ef, 0x1ef, 0x0f3,
    0x7f8, 0x019, 0x017, 0x0ed, 0x015, 0x001, 0x0e2, 0x0f0,
    0x070, 0x3f0, 0x1ee, 0x0f1, 0x7fa, 0x0ee, 0x0e4, 0x3f2,
    0x7f6, 0x3ef, 0x7fd, 0x005, 0x014, 0x0f2, 0x009, 0x004,
    0x0e5, 0x0f4, 0x0e8, 0x3f4, 0x006, 0x002, 0x0e7, 0x003,
    0x000, 0x06b, 0x0e3, 0x069, 0x1f3, 0x0eb, 0x0e6, 0x3f6,
    0x06e, 0x06a, 0x1f4, 0x3ec, 0x1f0, 0x3f9, 0x0f5, 0x0ec,
    0x7fb, 0x0ea, 0x06f, 0x3f7, 0x7f9, 0x3f3, 0x0fff, 0x0e9,
    0x06d, 0x3f8, 0x06c, 0x068, 0x1f5, 0x3ee, 0x1f2, 0x7f4,
    0x7f7, 0x3f1, 0x0ffe, 0x3ed, 0x1f1, 0x7f5, 0x7fe, 0x3f5,
    0x7fc,
];

#[rustfmt::skip]
const BITS_4: [u8; 81] = [
     4,  5,  8,  5,  4,  8,  9,  8, 11,  5,  5,  8,  5,  4,  8,  8,
     7, 10,  9,  8, 11,  8,  8, 10, 11, 10, 11,  4,  5,  8,  4,  4,
     8,  8,  8, 10,  4,  4,  8,  4,  4,  7,  8,  7,  9,  8,  8, 10,
     7,  7,  9, 10,  9, 10,  8,  8, 11,  8,  7, 10, 11, 10, 12,  8,
     7, 10,  7,  7,  9, 10,  9, 11, 11, 10, 12, 10,  9, 11, 11, 10,
    11,
];

// --- CB5: dim=2, signed, LAV=4, 81 entries ---

#[rustfmt::skip]
const CODES_5: [u16; 81] = [
    0x1fff, 0x0ff7, 0x07f4, 0x07e8, 0x03f1, 0x07ee, 0x07f9, 0x0ff8,
    0x1ffd, 0x0ffd, 0x07f1, 0x03e8, 0x01e8, 0x00f0, 0x01ec, 0x03ee,
    0x07f2, 0x0ffa, 0x0ff4, 0x03ef, 0x01f2, 0x00e8, 0x0070, 0x00ec,
    0x01f0, 0x03ea, 0x07f3, 0x07eb, 0x01eb, 0x00ea, 0x001a, 0x0008,
    0x0019, 0x00ee, 0x01ef, 0x07ed, 0x03f0, 0x00f2, 0x0073, 0x000b,
    0x0000, 0x000a, 0x0071, 0x00f3, 0x07e9, 0x07ef, 0x01ee, 0x00ef,
    0x0018, 0x0009, 0x001b, 0x00eb, 0x01e9, 0x07ec, 0x07f6, 0x03eb,
    0x01f3, 0x00ed, 0x0072, 0x00e9, 0x01f1, 0x03ed, 0x07f7, 0x0ff6,
    0x07f0, 0x03e9, 0x01ed, 0x00f1, 0x01ea, 0x03ec, 0x07f8, 0x0ff9,
    0x1ffc, 0x0ffc, 0x0ff5, 0x07ea, 0x03f3, 0x03f2, 0x07f5, 0x0ffb,
    0x1ffe,
];

#[rustfmt::skip]
const BITS_5: [u8; 81] = [
    13, 12, 11, 11, 10, 11, 11, 12, 13, 12, 11, 10,  9,  8,  9, 10,
    11, 12, 12, 10,  9,  8,  7,  8,  9, 10, 11, 11,  9,  8,  5,  4,
     5,  8,  9, 11, 10,  8,  7,  4,  1,  4,  7,  8, 11, 11,  9,  8,
     5,  4,  5,  8,  9, 11, 11, 10,  9,  8,  7,  8,  9, 10, 11, 12,
    11, 10,  9,  8,  9, 10, 11, 12, 13, 12, 12, 11, 10, 10, 11, 12,
    13,
];

// --- CB6: dim=2, signed, LAV=4, 81 entries ---

#[rustfmt::skip]
const CODES_6: [u16; 81] = [
    0x7fe, 0x3fd, 0x1f1, 0x1eb, 0x1f4, 0x1ea, 0x1f0, 0x3fc,
    0x7fd, 0x3f6, 0x1e5, 0x0ea, 0x06c, 0x071, 0x068, 0x0f0,
    0x1e6, 0x3f7, 0x1f3, 0x0ef, 0x032, 0x027, 0x028, 0x026,
    0x031, 0x0eb, 0x1f7, 0x1e8, 0x06f, 0x02e, 0x008, 0x004,
    0x006, 0x029, 0x06b, 0x1ee, 0x1ef, 0x072, 0x02d, 0x002,
    0x000, 0x003, 0x02f, 0x073, 0x1fa, 0x1e7, 0x06e, 0x02b,
    0x007, 0x001, 0x005, 0x02c, 0x06d, 0x1ec, 0x1f9, 0x0ee,
    0x030, 0x024, 0x02a, 0x025, 0x033, 0x0ec, 0x1f2, 0x3f8,
    0x1e4, 0x0ed, 0x06a, 0x070, 0x069, 0x074, 0x0f1, 0x3fa,
    0x7ff, 0x3f9, 0x1f6, 0x1ed, 0x1f8, 0x1e9, 0x1f5, 0x3fb,
    0x7fc,
];

#[rustfmt::skip]
const BITS_6: [u8; 81] = [
    11, 10,  9,  9,  9,  9,  9, 10, 11, 10,  9,  8,  7,  7,  7,  8,
     9, 10,  9,  8,  6,  6,  6,  6,  6,  8,  9,  9,  7,  6,  4,  4,
     4,  6,  7,  9,  9,  7,  6,  4,  4,  4,  6,  7,  9,  9,  7,  6,
     4,  4,  4,  6,  7,  9,  9,  8,  6,  6,  6,  6,  6,  8,  9, 10,
     9,  8,  7,  7,  7,  7,  8, 10, 11, 10,  9,  9,  9,  9,  9, 10,
    11,
];

// --- CB7: dim=2, unsigned, LAV=7, 64 entries ---

#[rustfmt::skip]
const CODES_7: [u16; 64] = [
    0x000, 0x005, 0x037, 0x074, 0x0f2, 0x1eb, 0x3ed, 0x7f7,
    0x004, 0x00c, 0x035, 0x071, 0x0ec, 0x0ee, 0x1ee, 0x1f5,
    0x036, 0x034, 0x072, 0x0ea, 0x0f1, 0x1e9, 0x1f3, 0x3f5,
    0x073, 0x070, 0x0eb, 0x0f0, 0x1f1, 0x1f0, 0x3ec, 0x3fa,
    0x0f3, 0x0ed, 0x1e8, 0x1ef, 0x3ef, 0x3f1, 0x3f9, 0x7fb,
    0x1ed, 0x0ef, 0x1ea, 0x1f2, 0x3f3, 0x3f8, 0x7f9, 0x7fc,
    0x3ee, 0x1ec, 0x1f4, 0x3f4, 0x3f7, 0x7f8, 0xffd, 0xffe,
    0x7f6, 0x3f0, 0x3f2, 0x3f6, 0x7fa, 0x7fd, 0xffc, 0xfff,
];

#[rustfmt::skip]
const BITS_7: [u8; 64] = [
     1,  3,  6,  7,  8,  9, 10, 11,  3,  4,  6,  7,  8,  8,  9,  9,
     6,  6,  7,  8,  8,  9,  9, 10,  7,  7,  8,  8,  9,  9, 10, 10,
     8,  8,  9,  9, 10, 10, 10, 11,  9,  8,  9,  9, 10, 10, 11, 11,
    10,  9,  9, 10, 10, 11, 12, 12, 11, 10, 10, 10, 11, 11, 12, 12,
];

// --- CB8: dim=2, unsigned, LAV=7, 64 entries ---

#[rustfmt::skip]
const CODES_8: [u16; 64] = [
    0x00e, 0x005, 0x010, 0x030, 0x06f, 0x0f1, 0x1fa, 0x3fe,
    0x003, 0x000, 0x004, 0x012, 0x02c, 0x06a, 0x075, 0x0f8,
    0x00f, 0x002, 0x006, 0x014, 0x02e, 0x069, 0x072, 0x0f5,
    0x02f, 0x011, 0x013, 0x02a, 0x032, 0x06c, 0x0ec, 0x0fa,
    0x071, 0x02b, 0x02d, 0x031, 0x06d, 0x070, 0x0f2, 0x1f9,
    0x0ef, 0x068, 0x033, 0x06b, 0x06e, 0x0ee, 0x0f9, 0x3fc,
    0x1f8, 0x074, 0x073, 0x0ed, 0x0f0, 0x0f6, 0x1f6, 0x1fd,
    0x3fd, 0x0f3, 0x0f4, 0x0f7, 0x1f7, 0x1fb, 0x1fc, 0x3ff,
];

#[rustfmt::skip]
const BITS_8: [u8; 64] = [
     5,  4,  5,  6,  7,  8,  9, 10,  4,  3,  4,  5,  6,  7,  7,  8,
     5,  4,  4,  5,  6,  7,  7,  8,  6,  5,  5,  6,  6,  7,  8,  8,
     7,  6,  6,  6,  7,  7,  8,  9,  8,  7,  6,  7,  7,  8,  8, 10,
     9,  7,  7,  8,  8,  8,  9,  9, 10,  8,  8,  8,  9,  9,  9, 10,
];

// --- CB9: dim=2, unsigned, LAV=12, 169 entries ---

#[rustfmt::skip]
const CODES_9: [u16; 169] = [
    0x0000, 0x0005, 0x0037, 0x00e7, 0x01de, 0x03ce, 0x03d9, 0x07c8,
    0x07cd, 0x0fc8, 0x0fdd, 0x1fe4, 0x1fec, 0x0004, 0x000c, 0x0035,
    0x0072, 0x00ea, 0x00ed, 0x01e2, 0x03d1, 0x03d3, 0x03e0, 0x07d8,
    0x0fcf, 0x0fd5, 0x0036, 0x0034, 0x0071, 0x00e8, 0x00ec, 0x01e1,
    0x03cf, 0x03dd, 0x03db, 0x07d0, 0x0fc7, 0x0fd4, 0x0fe4, 0x00e6,
    0x0070, 0x00e9, 0x01dd, 0x01e3, 0x03d2, 0x03dc, 0x07cc, 0x07ca,
    0x07de, 0x0fd8, 0x0fea, 0x1fdb, 0x01df, 0x00eb, 0x01dc, 0x01e6,
    0x03d5, 0x03de, 0x07cb, 0x07dd, 0x07dc, 0x0fcd, 0x0fe2, 0x0fe7,
    0x1fe1, 0x03d0, 0x01e0, 0x01e4, 0x03d6, 0x07c5, 0x07d1, 0x07db,
    0x0fd2, 0x07e0, 0x0fd9, 0x0feb, 0x1fe3, 0x1fe9, 0x07c4, 0x01e5,
    0x03d7, 0x07c6, 0x07cf, 0x07da, 0x0fcb, 0x0fda, 0x0fe3, 0x0fe9,
    0x1fe6, 0x1ff3, 0x1ff7, 0x07d3, 0x03d8, 0x03e1, 0x07d4, 0x07d9,
    0x0fd3, 0x0fde, 0x1fdd, 0x1fd9, 0x1fe2, 0x1fea, 0x1ff1, 0x1ff6,
    0x07d2, 0x03d4, 0x03da, 0x07c7, 0x07d7, 0x07e2, 0x0fce, 0x0fdb,
    0x1fd8, 0x1fee, 0x3ff0, 0x1ff4, 0x3ff2, 0x07e1, 0x03df, 0x07c9,
    0x07d6, 0x0fca, 0x0fd0, 0x0fe5, 0x0fe6, 0x1feb, 0x1fef, 0x3ff3,
    0x3ff4, 0x3ff5, 0x0fe0, 0x07ce, 0x07d5, 0x0fc6, 0x0fd1, 0x0fe1,
    0x1fe0, 0x1fe8, 0x1ff0, 0x3ff1, 0x3ff8, 0x3ff6, 0x7ffc, 0x0fe8,
    0x07df, 0x0fc9, 0x0fd7, 0x0fdc, 0x1fdc, 0x1fdf, 0x1fed, 0x1ff5,
    0x3ff9, 0x3ffb, 0x7ffd, 0x7ffe, 0x1fe7, 0x0fcc, 0x0fd6, 0x0fdf,
    0x1fde, 0x1fda, 0x1fe5, 0x1ff2, 0x3ffa, 0x3ff7, 0x3ffc, 0x3ffd,
    0x7fff,
];

#[rustfmt::skip]
const BITS_9: [u8; 169] = [
     1,  3,  6,  8,  9, 10, 10, 11, 11, 12, 12, 13, 13,  3,  4,  6,
     7,  8,  8,  9, 10, 10, 10, 11, 12, 12,  6,  6,  7,  8,  8,  9,
    10, 10, 10, 11, 12, 12, 12,  8,  7,  8,  9,  9, 10, 10, 11, 11,
    11, 12, 12, 13,  9,  8,  9,  9, 10, 10, 11, 11, 11, 12, 12, 12,
    13, 10,  9,  9, 10, 11, 11, 11, 12, 11, 12, 12, 13, 13, 11,  9,
    10, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 11, 10, 10, 11, 11,
    12, 12, 13, 13, 13, 13, 13, 13, 11, 10, 10, 11, 11, 11, 12, 12,
    13, 13, 14, 13, 14, 11, 10, 11, 11, 12, 12, 12, 12, 13, 13, 14,
    14, 14, 12, 11, 11, 12, 12, 12, 13, 13, 13, 14, 14, 14, 15, 12,
    11, 12, 12, 12, 13, 13, 13, 13, 14, 14, 15, 15, 13, 12, 12, 12,
    13, 13, 13, 13, 14, 14, 14, 14, 15,
];

// --- CB10: dim=2, unsigned, LAV=12, 169 entries ---

#[rustfmt::skip]
const CODES_10: [u16; 169] = [
    0x022, 0x008, 0x01d, 0x026, 0x05f, 0x0d3, 0x1cf, 0x3d0,
    0x3d7, 0x3ed, 0x7f0, 0x7f6, 0xffd, 0x007, 0x000, 0x001,
    0x009, 0x020, 0x054, 0x060, 0x0d5, 0x0dc, 0x1d4, 0x3cd,
    0x3de, 0x7e7, 0x01c, 0x002, 0x006, 0x00c, 0x01e, 0x028,
    0x05b, 0x0cd, 0x0d9, 0x1ce, 0x1dc, 0x3d9, 0x3f1, 0x025,
    0x00b, 0x00a, 0x00d, 0x024, 0x057, 0x061, 0x0cc, 0x0dd,
    0x1cc, 0x1de, 0x3d3, 0x3e7, 0x05d, 0x021, 0x01f, 0x023,
    0x027, 0x059, 0x064, 0x0d8, 0x0df, 0x1d2, 0x1e2, 0x3dd,
    0x3ee, 0x0d1, 0x055, 0x029, 0x056, 0x058, 0x062, 0x0ce,
    0x0e0, 0x0e2, 0x1da, 0x3d4, 0x3e3, 0x7eb, 0x1c9, 0x05e,
    0x05a, 0x05c, 0x063, 0x0ca, 0x0da, 0x1c7, 0x1ca, 0x1e0,
    0x3db, 0x3e8, 0x7ec, 0x1e3, 0x0d2, 0x0cb, 0x0d0, 0x0d7,
    0x0db, 0x1c6, 0x1d5, 0x1d8, 0x3ca, 0x3da, 0x7ea, 0x7f1,
    0x1e1, 0x0d4, 0x0cf, 0x0d6, 0x0de, 0x0e1, 0x1d0, 0x1d6,
    0x3d1, 0x3d5, 0x3f2, 0x7ee, 0x7fb, 0x3e9, 0x1cd, 0x1c8,
    0x1cb, 0x1d1, 0x1d7, 0x1df, 0x3cf, 0x3e0, 0x3ef, 0x7e6,
    0x7f8, 0xffa, 0x3eb, 0x1dd, 0x1d3, 0x1d9, 0x1db, 0x3d2,
    0x3cc, 0x3dc, 0x3ea, 0x7ed, 0x7f3, 0x7f9, 0xff9, 0x7f2,
    0x3ce, 0x1e4, 0x3cb, 0x3d8, 0x3d6, 0x3e2, 0x3e5, 0x7e8,
    0x7f4, 0x7f5, 0x7f7, 0xffb, 0x7fa, 0x3ec, 0x3df, 0x3e1,
    0x3e4, 0x3e6, 0x3f0, 0x7e9, 0x7ef, 0xff8, 0xffe, 0xffc,
    0xfff,
];

#[rustfmt::skip]
const BITS_10: [u8; 169] = [
     6,  5,  6,  6,  7,  8,  9, 10, 10, 10, 11, 11, 12,  5,  4,  4,
     5,  6,  7,  7,  8,  8,  9, 10, 10, 11,  6,  4,  5,  5,  6,  6,
     7,  8,  8,  9,  9, 10, 10,  6,  5,  5,  5,  6,  7,  7,  8,  8,
     9,  9, 10, 10,  7,  6,  6,  6,  6,  7,  7,  8,  8,  9,  9, 10,
    10,  8,  7,  6,  7,  7,  7,  8,  8,  8,  9, 10, 10, 11,  9,  7,
     7,  7,  7,  8,  8,  9,  9,  9, 10, 10, 11,  9,  8,  8,  8,  8,
     8,  9,  9,  9, 10, 10, 11, 11,  9,  8,  8,  8,  8,  8,  9,  9,
    10, 10, 10, 11, 11, 10,  9,  9,  9,  9,  9,  9, 10, 10, 10, 11,
    11, 12, 10,  9,  9,  9,  9, 10, 10, 10, 10, 11, 11, 11, 12, 11,
    10,  9, 10, 10, 10, 10, 10, 11, 11, 11, 11, 12, 11, 10, 10, 10,
    10, 10, 10, 11, 11, 12, 12, 12, 12,
];

// --- CB11: dim=2, unsigned+ESC, LAV=16, 289 entries ---

#[rustfmt::skip]
const CODES_11: [u16; 289] = [
    0x000, 0x006, 0x019, 0x03d, 0x09c, 0x0c6, 0x1a7, 0x390,
    0x3c2, 0x3df, 0x7e6, 0x7f3, 0xffb, 0x7ec, 0xffa, 0xffe,
    0x38e, 0x005, 0x001, 0x008, 0x014, 0x037, 0x042, 0x092,
    0x0af, 0x191, 0x1a5, 0x1b5, 0x39e, 0x3c0, 0x3a2, 0x3cd,
    0x7d6, 0x0ae, 0x017, 0x007, 0x009, 0x018, 0x039, 0x040,
    0x08e, 0x0a3, 0x0b8, 0x199, 0x1ac, 0x1c1, 0x3b1, 0x396,
    0x3be, 0x3ca, 0x09d, 0x03c, 0x015, 0x016, 0x01a, 0x03b,
    0x044, 0x091, 0x0a5, 0x0be, 0x196, 0x1ae, 0x1b9, 0x3a1,
    0x391, 0x3a5, 0x3d5, 0x094, 0x09a, 0x036, 0x038, 0x03a,
    0x041, 0x08c, 0x09b, 0x0b0, 0x0c3, 0x19e, 0x1ab, 0x1bc,
    0x39f, 0x38f, 0x3a9, 0x3cf, 0x093, 0x0bf, 0x03e, 0x03f,
    0x043, 0x045, 0x09e, 0x0a7, 0x0b9, 0x194, 0x1a2, 0x1ba,
    0x1c3, 0x3a6, 0x3a7, 0x3bb, 0x3d4, 0x09f, 0x1a0, 0x08f,
    0x08d, 0x090, 0x098, 0x0a6, 0x0b6, 0x0c4, 0x19f, 0x1af,
    0x1bf, 0x399, 0x3bf, 0x3b4, 0x3c9, 0x3e7, 0x0a8, 0x1b6,
    0x0ab, 0x0a4, 0x0aa, 0x0b2, 0x0c2, 0x0c5, 0x198, 0x1a4,
    0x1b8, 0x38c, 0x3a4, 0x3c4, 0x3c6, 0x3dd, 0x3e8, 0x0ad,
    0x3af, 0x192, 0x0bd, 0x0bc, 0x18e, 0x197, 0x19a, 0x1a3,
    0x1b1, 0x38d, 0x398, 0x3b7, 0x3d3, 0x3d1, 0x3db, 0x7dd,
    0x0b4, 0x3de, 0x1a9, 0x19b, 0x19c, 0x1a1, 0x1aa, 0x1ad,
    0x1b3, 0x38b, 0x3b2, 0x3b8, 0x3ce, 0x3e1, 0x3e0, 0x7d2,
    0x7e5, 0x0b7, 0x7e3, 0x1bb, 0x1a8, 0x1a6, 0x1b0, 0x1b2,
    0x1b7, 0x39b, 0x39a, 0x3ba, 0x3b5, 0x3d6, 0x7d7, 0x3e4,
    0x7d8, 0x7ea, 0x0ba, 0x7e8, 0x3a0, 0x1bd, 0x1b4, 0x38a,
    0x1c4, 0x392, 0x3aa, 0x3b0, 0x3bc, 0x3d7, 0x7d4, 0x7dc,
    0x7db, 0x7d5, 0x7f0, 0x0c1, 0x7fb, 0x3c8, 0x3a3, 0x395,
    0x39d, 0x3ac, 0x3ae, 0x3c5, 0x3d8, 0x3e2, 0x3e6, 0x7e4,
    0x7e7, 0x7e0, 0x7e9, 0x7f7, 0x190, 0x7f2, 0x393, 0x1be,
    0x1c0, 0x394, 0x397, 0x3ad, 0x3c3, 0x3c1, 0x3d2, 0x7da,
    0x7d9, 0x7df, 0x7eb, 0x7f4, 0x7fa, 0x195, 0x7f8, 0x3bd,
    0x39c, 0x3ab, 0x3a8, 0x3b3, 0x3b9, 0x3d0, 0x3e3, 0x3e5,
    0x7e2, 0x7de, 0x7ed, 0x7f1, 0x7f9, 0x7fc, 0x193, 0xffd,
    0x3dc, 0x3b6, 0x3c7, 0x3cc, 0x3cb, 0x3d9, 0x3da, 0x7d3,
    0x7e1, 0x7ee, 0x7ef, 0x7f5, 0x7f6, 0xffc, 0xfff, 0x19d,
    0x1c2, 0x0b5, 0x0a1, 0x096, 0x097, 0x095, 0x099, 0x0a0,
    0x0a2, 0x0ac, 0x0a9, 0x0b1, 0x0b3, 0x0bb, 0x0c0, 0x18f,
    0x004,
];

#[rustfmt::skip]
const BITS_11: [u8; 289] = [
     4,  5,  6,  7,  8,  8,  9, 10, 10, 10, 11, 11, 12, 11, 12, 12,
    10,  5,  4,  5,  6,  7,  7,  8,  8,  9,  9,  9, 10, 10, 10, 10,
    11,  8,  6,  5,  5,  6,  7,  7,  8,  8,  8,  9,  9,  9, 10, 10,
    10, 10,  8,  7,  6,  6,  6,  7,  7,  8,  8,  8,  9,  9,  9, 10,
    10, 10, 10,  8,  8,  7,  7,  7,  7,  8,  8,  8,  8,  9,  9,  9,
    10, 10, 10, 10,  8,  8,  7,  7,  7,  7,  8,  8,  8,  9,  9,  9,
     9, 10, 10, 10, 10,  8,  9,  8,  8,  8,  8,  8,  8,  8,  9,  9,
     9, 10, 10, 10, 10, 10,  8,  9,  8,  8,  8,  8,  8,  8,  9,  9,
     9, 10, 10, 10, 10, 10, 10,  8, 10,  9,  8,  8,  9,  9,  9,  9,
     9, 10, 10, 10, 10, 10, 10, 11,  8, 10,  9,  9,  9,  9,  9,  9,
     9, 10, 10, 10, 10, 10, 10, 11, 11,  8, 11,  9,  9,  9,  9,  9,
     9, 10, 10, 10, 10, 10, 11, 10, 11, 11,  8, 11, 10,  9,  9, 10,
     9, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11,  8, 11, 10, 10, 10,
    10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11,  9, 11, 10,  9,
     9, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11,  9, 11, 10,
    10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11,  9, 12,
    10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12,  9,
     9,  8,  8,  8,  8,  8,  8,  8,  8,  8,  8,  8,  8,  8,  8,  9,
     5,
];

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sf_树解码() {
        let cbs = AacCodebooks::build();
        // delta=0 (index=60): 码字 "0" (1 bit)
        let data = [0x00u8];
        let mut br = BitReader::new(&data);
        let val = cbs.sf_tree.decode(&mut br).unwrap();
        assert_eq!(val, 60);
    }

    #[test]
    fn test_所有码本已构建() {
        let cbs = AacCodebooks::build();
        for (i, cb) in cbs.spectral.iter().enumerate() {
            assert!(cb.is_some(), "码本 {} 未构建", i + 1);
        }
    }

    #[test]
    fn test_cb1_零向量() {
        // CB1: signed, dim=4, mod=3, offset=-1
        // (0,0,0,0) → index = 1*27+1*9+1*3+1 = 40
        // codes1[40] = 0x000, bits1[40] = 1
        let cbs = AacCodebooks::build();
        let cb = cbs.spectral[0].as_ref().unwrap();
        let data = [0x00u8]; // 1 bit: 0
        let mut br = BitReader::new(&data);
        let vals = cb.decode_values(&mut br).unwrap();
        assert_eq!(vals, [0, 0, 0, 0]);
    }

    #[test]
    fn test_cb7_零对() {
        // CB7: unsigned, dim=2, mod=8, offset=0
        // (0,0) → index=0, code=0x000, bits=1
        let cbs = AacCodebooks::build();
        let cb = cbs.spectral[6].as_ref().unwrap();
        let data = [0x00u8]; // 1 bit: 0
        let mut br = BitReader::new(&data);
        let vals = cb.decode_values(&mut br).unwrap();
        assert_eq!(vals, [0, 0, 0, 0]);
    }

    #[test]
    fn test_cb8_已实现() {
        let cbs = AacCodebooks::build();
        assert!(cbs.spectral[7].is_some());
        let cb = cbs.spectral[7].as_ref().unwrap();
        assert_eq!(cb.dim, 2);
    }

    #[test]
    fn test_cb10_已实现() {
        let cbs = AacCodebooks::build();
        assert!(cbs.spectral[9].is_some());
        let cb = cbs.spectral[9].as_ref().unwrap();
        assert_eq!(cb.dim, 2);
    }

    #[test]
    fn test_cb11_已实现_带esc() {
        let cbs = AacCodebooks::build();
        assert!(cbs.spectral[10].is_some());
        let cb = cbs.spectral[10].as_ref().unwrap();
        assert_eq!(cb.dim, 2);
        assert!(cb.is_esc);
    }

    #[test]
    fn test_index_to_values_quad() {
        // CB3: dim=4, mod=3, offset=0
        let vals = index_to_values(0, 4, 3, 0);
        assert_eq!(vals, [0, 0, 0, 0]);

        let vals = index_to_values(27, 4, 3, 0);
        assert_eq!(vals, [1, 0, 0, 0]);

        let vals = index_to_values(80, 4, 3, 0);
        assert_eq!(vals, [2, 2, 2, 2]);
    }

    #[test]
    fn test_index_to_values_pair() {
        // CB7: dim=2, mod=8, offset=0
        let vals = index_to_values(0, 2, 8, 0);
        assert_eq!(vals, [0, 0, 0, 0]);

        let vals = index_to_values(9, 2, 8, 0);
        assert_eq!(vals, [1, 1, 0, 0]);

        let vals = index_to_values(63, 2, 8, 0);
        assert_eq!(vals, [7, 7, 0, 0]);
    }

    #[test]
    fn test_index_to_values_signed() {
        // CB5: dim=2, mod=9, offset=-4
        let vals = index_to_values(0, 2, 9, -4);
        assert_eq!(vals, [-4, -4, 0, 0]);

        let vals = index_to_values(40, 2, 9, -4);
        assert_eq!(vals, [0, 0, 0, 0]);

        let vals = index_to_values(80, 2, 9, -4);
        assert_eq!(vals, [4, 4, 0, 0]);
    }
}
