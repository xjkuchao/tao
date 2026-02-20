//! MP3 Huffman 解码器
//!
//! 使用快速查找表 (Lookup Table) 实现 O(1) Huffman 解码.
//! Big Values 表使用 Canonical Huffman + 直接查表.
//! Count1 表使用专用的直接查表.

use super::bitreader::BitReader;
use super::huffman_explicit_tables as explicit;
use super::tables::{MPA_HUFF_LENS, MPA_HUFF_OFFSET, MPA_HUFF_SYMS};
use log::warn;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use tao_core::{TaoError, TaoResult};

// ============================================================================
// Big Values 快速查找表 (表 1-31)
// ============================================================================

/// 查找表条目
#[derive(Debug, Clone, Copy, Default)]
struct LutEntry {
    /// 解码后的符号
    symbol: u8,
    /// 需要消费的位数 (0 表示未找到)
    bits: u8,
}

/// Big Values 查找表 (peek PEEK_BITS 位直接查表)
const PEEK_BITS: usize = 10;
const PEEK_SIZE: usize = 1 << PEEK_BITS;

/// 每张表的查找表
#[derive(Debug, Clone, Default)]
struct BigValueTable {
    /// 快速查找表 (PEEK_BITS 位)
    lut: Vec<LutEntry>,
    /// 溢出条目 (码长 > PEEK_BITS 的条目), 按 (code, len, symbol) 存储
    overflow: Vec<(u32, u8, u8)>,
    /// 最大码长
    max_len: u8,
}

/// 全局 Big Values 表缓存
static BIG_VALUE_TABLES: OnceLock<Vec<BigValueTable>> = OnceLock::new();
static HUFFMAN_MISMATCHES: AtomicUsize = AtomicUsize::new(0);

fn get_big_value_tables() -> &'static Vec<BigValueTable> {
    BIG_VALUE_TABLES.get_or_init(|| {
        let mut tables = Vec::with_capacity(32);
        for i in 0..32 {
            tables.push(build_big_value_table(i as u8));
        }
        tables
    })
}

/// 从显式 (code, len) 表构建 Big Values 查找表.
/// symbol 使用 MP3 规范展开顺序: symbol = ((i / wrap) << 4) | (i % wrap).
fn build_big_value_table_explicit(codes: &[u32], lens: &[u8], wrap: usize) -> BigValueTable {
    let mut max_len = 0u8;
    let mut entries = Vec::with_capacity(codes.len());

    for i in 0..codes.len() {
        let len = lens[i];
        if len == 0 {
            continue;
        }
        let symbol = (((i / wrap) as u8) << 4) | (i % wrap) as u8;
        entries.push((codes[i], len, symbol));
        if len > max_len {
            max_len = len;
        }
    }

    build_big_value_table_from_entries(entries, max_len)
}

fn explicit_codebook(table_id: u8) -> Option<(&'static [u32], &'static [u8], usize)> {
    match table_id {
        1 => Some((&explicit::MPEG_CODES_1, &explicit::MPEG_BITS_1, 2)),
        2 => Some((&explicit::MPEG_CODES_2, &explicit::MPEG_BITS_2, 3)),
        3 => Some((&explicit::MPEG_CODES_3, &explicit::MPEG_BITS_3, 3)),
        5 => Some((&explicit::MPEG_CODES_5, &explicit::MPEG_BITS_5, 4)),
        6 => Some((&explicit::MPEG_CODES_6, &explicit::MPEG_BITS_6, 4)),
        7 => Some((&explicit::MPEG_CODES_7, &explicit::MPEG_BITS_7, 6)),
        8 => Some((&explicit::MPEG_CODES_8, &explicit::MPEG_BITS_8, 6)),
        9 => Some((&explicit::MPEG_CODES_9, &explicit::MPEG_BITS_9, 6)),
        10 => Some((&explicit::MPEG_CODES_10, &explicit::MPEG_BITS_10, 8)),
        11 => Some((&explicit::MPEG_CODES_11, &explicit::MPEG_BITS_11, 8)),
        12 => Some((&explicit::MPEG_CODES_12, &explicit::MPEG_BITS_12, 8)),
        13 => Some((&explicit::MPEG_CODES_13, &explicit::MPEG_BITS_13, 16)),
        15 => Some((&explicit::MPEG_CODES_15, &explicit::MPEG_BITS_15, 16)),
        16..=23 => Some((&explicit::MPEG_CODES_16, &explicit::MPEG_BITS_16, 16)),
        24..=31 => Some((&explicit::MPEG_CODES_24, &explicit::MPEG_BITS_24, 16)),
        _ => None,
    }
}

/// 从 (code, len, symbol) 条目构建 LUT 与 overflow.
fn build_big_value_table_from_entries(entries: Vec<(u32, u8, u8)>, max_len: u8) -> BigValueTable {
    let mut lut = vec![LutEntry::default(); PEEK_SIZE];
    let mut overflow = Vec::new();

    for &(code_val, len, symbol) in &entries {
        if (len as usize) <= PEEK_BITS {
            let pad_bits = PEEK_BITS - len as usize;
            let base_idx = (code_val as usize) << pad_bits;
            let fill_count = 1 << pad_bits;
            if base_idx + fill_count <= PEEK_SIZE {
                for j in 0..fill_count {
                    lut[base_idx | j] = LutEntry { symbol, bits: len };
                }
            } else {
                overflow.push((code_val, len, symbol));
            }
        } else {
            overflow.push((code_val, len, symbol));
        }
    }

    BigValueTable {
        lut,
        overflow,
        max_len,
    }
}

/// 构建单张 Big Values 查找表
///
/// 使用 FFmpeg 风格的 MSB 对齐 canonical 码字生成算法.
/// 此算法按输入顺序处理 LENS/SYMS, 使用 32 位左对齐计数器,
/// 确保生成的码字与 ISO 11172-3 标准完全一致.
fn build_big_value_table(table_id: u8) -> BigValueTable {
    if table_id == 0 || table_id == 4 || table_id == 14 {
        return BigValueTable::default();
    }

    // 对所有有效 big-values 表使用显式 code+bits, 消除 LENS/SYMS 推导歧义.
    if let Some((codes, lens, wrap)) = explicit_codebook(table_id) {
        return build_big_value_table_explicit(codes, lens, wrap);
    }

    let offset = MPA_HUFF_OFFSET[table_id as usize];

    let count = match table_id {
        1 => 4,
        2..=3 => 9,
        5..=6 => 16,
        7..=9 => 36,
        10..=12 => 64,
        13 | 15 => 256,
        16..=23 => 256,
        24..=31 => 256,
        _ => 0,
    };

    if count == 0 {
        return BigValueTable::default();
    }

    let lens = &MPA_HUFF_LENS[offset..offset + count];
    let syms = &MPA_HUFF_SYMS[offset..offset + count];

    // MSB 对齐 canonical 码字生成
    //
    // FFmpeg 的 LENS/SYMS 数组按特定顺序排列 (大致为码长降序),
    // 使得按数组顺序用 MSB 对齐计数器就能产生与 ISO 11172-3 一致的码字.
    // 此算法等价于 FFmpeg 的 ff_vlc_init_tables_from_lengths.
    let mut code: u64 = 0;
    let mut max_len: u8 = 0;
    let mut entries: Vec<(u32, u8, u8)> = Vec::with_capacity(count);

    for i in 0..count {
        let len = lens[i];
        if len > 0 {
            let code_val = (code >> (32 - len as u64)) as u32;
            entries.push((code_val, len, syms[i]));
            code += 1u64 << (32 - len as u64);
            if len > max_len {
                max_len = len;
            }
        }
    }

    build_big_value_table_from_entries(entries, max_len)
}

fn build_big_value_entries(table_id: u8) -> Vec<(u32, u8, u8)> {
    if table_id == 0 || table_id == 4 || table_id == 14 {
        return Vec::new();
    }

    if let Some((codes, lens, wrap)) = explicit_codebook(table_id) {
        let mut entries = Vec::with_capacity(codes.len());
        for i in 0..codes.len() {
            let len = lens[i];
            if len == 0 {
                continue;
            }
            let symbol = (((i / wrap) as u8) << 4) | (i % wrap) as u8;
            entries.push((codes[i], len, symbol));
        }
        return entries;
    }

    let offset = MPA_HUFF_OFFSET[table_id as usize];
    let count = match table_id {
        1 => 4,
        2..=3 => 9,
        5..=6 => 16,
        7..=9 => 36,
        10..=12 => 64,
        13 | 15 => 256,
        16..=23 => 256,
        24..=31 => 256,
        _ => 0,
    };

    if count == 0 {
        return Vec::new();
    }

    let lens = &MPA_HUFF_LENS[offset..offset + count];
    let syms = &MPA_HUFF_SYMS[offset..offset + count];
    let mut code: u64 = 0;
    let mut entries: Vec<(u32, u8, u8)> = Vec::with_capacity(count);

    for i in 0..count {
        let len = lens[i];
        if len > 0 {
            let code_val = (code >> (32 - len as u64)) as u32;
            entries.push((code_val, len, syms[i]));
            code += 1u64 << (32 - len as u64);
        }
    }

    entries
}

fn decode_big_value_reference_vlc(br: &mut BitReader, entries: &[(u32, u8, u8)]) -> TaoResult<u8> {
    let mut max_len = 0u8;
    for &(_, len, _) in entries {
        if len > max_len {
            max_len = len;
        }
    }

    for len in 1..=max_len {
        let Some(bits) = br.peek_bits(len) else {
            break;
        };
        for &(code, code_len, symbol) in entries {
            if code_len == len && code == bits {
                br.skip_bits(len as usize);
                return Ok(symbol);
            }
        }
    }

    Err(TaoError::InvalidData(
        "BigValues Huffman 参考解码失败".to_string(),
    ))
}

fn decode_big_values_reference(
    br: &mut BitReader,
    table_id: u8,
    linbits: u8,
) -> TaoResult<(i32, i32)> {
    if table_id == 0 {
        return Ok((0, 0));
    }

    let entries = build_big_value_entries(table_id);
    if entries.is_empty() {
        return Ok((0, 0));
    }

    let symbol = decode_big_value_reference_vlc(br, &entries)?;
    let mut x = (symbol >> 4) as i32;
    let mut y = (symbol & 0x0F) as i32;

    if table_id > 15 && x == 15 && linbits > 0 {
        x += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
    }
    if x > 0 && br.read_bool().ok_or(TaoError::Eof)? {
        x = -x;
    }
    if table_id > 15 && y == 15 && linbits > 0 {
        y += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
    }
    if y > 0 && br.read_bool().ok_or(TaoError::Eof)? {
        y = -y;
    }

    Ok((x, y))
}

// ============================================================================
// Count1 快速查找表 (表 32, 33)
// ============================================================================

/// Count1 Table A (table 32) 的查找表
/// 最大码长为 6, 使用 6 位直接查表
const COUNT1A_PEEK_BITS: usize = 6;
const COUNT1A_PEEK_SIZE: usize = 1 << COUNT1A_PEEK_BITS;

// 来自 FFmpeg mpa_quad_bits[0] 和 mpa_quad_codes[0]
const COUNT1A_BITS: [u8; 16] = [1, 4, 4, 5, 4, 6, 5, 6, 4, 5, 5, 6, 5, 6, 6, 6];
const COUNT1A_CODES: [u8; 16] = [1, 5, 4, 5, 6, 5, 4, 4, 7, 3, 6, 0, 7, 2, 3, 1];

static COUNT1A_LUT: OnceLock<Vec<LutEntry>> = OnceLock::new();

fn get_count1a_lut() -> &'static Vec<LutEntry> {
    COUNT1A_LUT.get_or_init(|| {
        let mut lut = vec![LutEntry::default(); COUNT1A_PEEK_SIZE];

        for symbol in 0..16u8 {
            let len = COUNT1A_BITS[symbol as usize];
            let code = COUNT1A_CODES[symbol as usize] as u32;
            let pad = COUNT1A_PEEK_BITS - len as usize;
            let base = (code as usize) << pad;
            let fill = 1 << pad;
            for j in 0..fill {
                lut[base | j] = LutEntry { symbol, bits: len };
            }
        }
        lut
    })
}

// ============================================================================
// Huffman 解码器
// ============================================================================

pub struct HuffmanDecoder;

impl HuffmanDecoder {
    pub fn new() -> Self {
        Self
    }

    /// 解码 Big Values (x, y)
    /// table_id: 1..31
    pub fn decode_big_values(
        &self,
        br: &mut BitReader,
        table_id: u8,
        linbits: u8,
    ) -> TaoResult<(i32, i32)> {
        if std::env::var("TAO_MP3_FORCE_HUFFMAN_REF").is_ok() {
            return decode_big_values_reference(br, table_id, linbits);
        }
        let debug_ref = std::env::var("TAO_MP3_DEBUG_HUFFMAN").is_ok();
        let start_bit = br.bit_offset();
        let ref_result = if debug_ref {
            let mut br_ref = *br;
            decode_big_values_reference(&mut br_ref, table_id, linbits)
                .ok()
                .map(|val| (val, br_ref.bit_offset()))
        } else {
            None
        };

        if table_id == 0 {
            return Ok((0, 0));
        }

        let tables = get_big_value_tables();
        let table = &tables[table_id as usize];
        if table.lut.is_empty() {
            return Ok((0, 0));
        }

        let symbol = self.decode_big_value_vlc(br, table)?;

        let mut x = (symbol >> 4) as i32;
        let mut y = (symbol & 0x0F) as i32;

        // Escaped values (linbits), 表 16-31
        if table_id > 15 && x == 15 && linbits > 0 {
            x += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
        }
        if x > 0 && br.read_bool().ok_or(TaoError::Eof)? {
            x = -x;
        }
        if table_id > 15 && y == 15 && linbits > 0 {
            y += br.read_bits(linbits).ok_or(TaoError::Eof)? as i32;
        }
        if y > 0 && br.read_bool().ok_or(TaoError::Eof)? {
            y = -y;
        }

        let result = (x, y);

        if let Some((ref_val, ref_end)) = ref_result {
            let actual_end = br.bit_offset();
            if ref_val != result || ref_end != actual_end {
                let count = HUFFMAN_MISMATCHES.fetch_add(1, Ordering::Relaxed);
                if count < 10 {
                    warn!(
                        "BigValues 解码不一致: table={}, ref=({},{}) bits_ref={}, actual=({},{}) bits_actual={}",
                        table_id,
                        ref_val.0,
                        ref_val.1,
                        ref_end.saturating_sub(start_bit),
                        result.0,
                        result.1,
                        actual_end.saturating_sub(start_bit)
                    );
                }
            }
        }

        Ok(result)
    }

    /// 快速 VLC 解码 (Big Values)
    fn decode_big_value_vlc(&self, br: &mut BitReader, table: &BigValueTable) -> TaoResult<u8> {
        let bits_left = br.bits_left();

        // 快速路径: 剩余位数足够时直接 peek 固定窗口.
        if bits_left >= PEEK_BITS {
            let peek_val = br.peek_bits(PEEK_BITS as u8).ok_or(TaoError::Eof)?;
            let entry = table.lut[peek_val as usize];
            if entry.bits > 0 {
                br.skip_bits(entry.bits as usize);
                return Ok(entry.symbol);
            }
        } else if bits_left > 0 {
            // 尾部路径: 剩余位不足 PEEK_BITS 时使用左对齐索引.
            let peek_val = br.peek_bits(bits_left as u8).ok_or(TaoError::Eof)? as usize;
            let idx = peek_val << (PEEK_BITS - bits_left);
            let entry = table.lut[idx];
            if entry.bits > 0 && (entry.bits as usize) <= bits_left {
                br.skip_bits(entry.bits as usize);
                return Ok(entry.symbol);
            }
        }

        // 尝试溢出表 (长码)。
        // 不能一次性 peek max_len 位后统一右移比较:
        // 当剩余位数不足 max_len 时, 某些合法短码会被误判为失败.
        for len in (PEEK_BITS as u8 + 1)..=table.max_len {
            let Some(bits) = br.peek_bits(len) else {
                break;
            };
            for &(code, code_len, symbol) in &table.overflow {
                if code_len == len && bits == code {
                    br.skip_bits(len as usize);
                    return Ok(symbol);
                }
            }
        }

        Err(TaoError::InvalidData(
            "BigValues Huffman 解码失败".to_string(),
        ))
    }

    /// 解码 Count1 (v, w, x, y)
    /// table_id: 32 or 33
    pub fn decode_count1(
        &self,
        br: &mut BitReader,
        table_id: u8,
    ) -> TaoResult<(i32, i32, i32, i32)> {
        if std::env::var("TAO_MP3_FORCE_HUFFMAN_REF").is_ok() {
            let (symbol, _) = decode_count1_symbol_reference(br, table_id)?;
            return decode_count1_signs(br, symbol);
        }
        let debug_ref = std::env::var("TAO_MP3_DEBUG_HUFFMAN").is_ok();
        let start_bit = br.bit_offset();
        let ref_result = if debug_ref {
            let mut br_ref = *br;
            decode_count1_symbol_reference(&mut br_ref, table_id)
                .ok()
                .and_then(|(symbol, _)| {
                    decode_count1_signs(&mut br_ref, symbol)
                        .ok()
                        .map(|v| (v, br_ref.bit_offset()))
                })
        } else {
            None
        };

        let symbol = if table_id == 33 {
            // Table B: 复用 minimp3 的两级表解码逻辑
            decode_count1_table_b(br)?
        } else {
            // Table A (变长, 最大 7 位): 使用查找表
            let lut = get_count1a_lut();
            let bits_left = br.bits_left();
            if bits_left == 0 {
                return Err(TaoError::Eof);
            }
            let probe_bits = bits_left.min(COUNT1A_PEEK_BITS);
            let peek = br.peek_bits(probe_bits as u8).ok_or(TaoError::Eof)? as usize;
            let idx = if probe_bits < COUNT1A_PEEK_BITS {
                peek << (COUNT1A_PEEK_BITS - probe_bits)
            } else {
                peek
            };
            let entry = lut[idx];
            if entry.bits > 0 {
                if (entry.bits as usize) > bits_left {
                    return Err(TaoError::Eof);
                }
                br.skip_bits(entry.bits as usize);
                entry.symbol
            } else {
                return Err(TaoError::InvalidData("Count1 Huffman 解码失败".to_string()));
            }
        };

        let result = decode_count1_signs(br, symbol)?;

        if let Some((ref_val, ref_end)) = ref_result {
            let actual_end = br.bit_offset();
            if ref_val != result || ref_end != actual_end {
                let count = HUFFMAN_MISMATCHES.fetch_add(1, Ordering::Relaxed);
                if count < 10 {
                    warn!(
                        "Count1 解码不一致: table={}, ref=({},{},{},{}), actual=({},{},{},{}), bits_ref={}, bits_actual={}",
                        table_id,
                        ref_val.0,
                        ref_val.1,
                        ref_val.2,
                        ref_val.3,
                        result.0,
                        result.1,
                        result.2,
                        result.3,
                        ref_end.saturating_sub(start_bit),
                        actual_end.saturating_sub(start_bit)
                    );
                }
            }
        }

        Ok(result)
    }
}

fn decode_count1_signs(br: &mut BitReader, symbol: u8) -> TaoResult<(i32, i32, i32, i32)> {
    // symbol 的 4 位对应 (v,w,x,y).
    // 默认使用 v->w->x->y 的符号位读取顺序.
    // 可通过环境变量切换到 LSB 顺序(y->x->w->v)以进行对照诊断.
    let use_lsb_order = std::env::var("TAO_MP3_COUNT1_SIGN_LSB")
        .ok()
        .is_some_and(|v| v == "1");

    let mut v = 0i32;
    let mut w = 0i32;
    let mut x = 0i32;
    let mut y = 0i32;

    if use_lsb_order {
        if (symbol & 0x1) != 0 {
            y = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x2) != 0 {
            x = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x4) != 0 {
            w = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x8) != 0 {
            v = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
    } else {
        if (symbol & 0x8) != 0 {
            v = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x4) != 0 {
            w = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x2) != 0 {
            x = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
        if (symbol & 0x1) != 0 {
            y = if br.read_bool().ok_or(TaoError::Eof)? {
                -1
            } else {
                1
            };
        }
    }

    Ok((v, w, x, y))
}

/// Count1 Table B 解码
/// 基于 minimp3 的 tab33 解码逻辑, 保持与规范一致的码字/长度。
fn decode_count1_table_b(br: &mut BitReader) -> TaoResult<u8> {
    // Table B 为固定 4 位码字, code=15..0 对应 symbol=0..15.
    let bits = br.read_bits(4).ok_or(TaoError::Eof)? as u8;
    Ok(15u8.saturating_sub(bits & 0x0F))
}

fn decode_count1_symbol_reference(br: &mut BitReader, table_id: u8) -> TaoResult<(u8, usize)> {
    if table_id == 33 {
        let bits = br.read_bits(4).ok_or(TaoError::Eof)? as u8;
        return Ok((15u8.saturating_sub(bits & 0x0F), 4));
    }

    let mut max_len = 0u8;
    for &len in &COUNT1A_BITS {
        if len > max_len {
            max_len = len;
        }
    }

    for len in 1..=max_len {
        let Some(bits) = br.peek_bits(len) else {
            break;
        };
        for symbol in 0..16u8 {
            let code_len = COUNT1A_BITS[symbol as usize];
            let code = COUNT1A_CODES[symbol as usize] as u32;
            if code_len == len && code == bits {
                br.skip_bits(len as usize);
                return Ok((symbol, len as usize));
            }
        }
    }

    Err(TaoError::InvalidData(
        "Count1 Huffman 参考解码失败".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_explicit_entries(codes: &[u32], lens: &[u8], wrap: usize) -> Vec<(u32, u8, u8)> {
        let mut entries = Vec::with_capacity(codes.len());
        for i in 0..codes.len() {
            let len = lens[i];
            if len == 0 {
                continue;
            }
            let symbol = (((i / wrap) as u8) << 4) | (i % wrap) as u8;
            entries.push((codes[i], len, symbol));
        }
        entries
    }

    /// 验证 Table 1 的 LUT 正确性
    /// ISO 11172-3 Table B.7:
    /// (0,0): code=1,   len=1
    /// (1,0): code=01,  len=2
    /// (0,1): code=001, len=3
    /// (1,1): code=000, len=3
    #[test]
    fn test_table1_lut() {
        let tables = get_big_value_tables();
        let t = &tables[1];

        // 1_000000000 -> code=1 (len=1) -> sym=(0,0)=0x00
        let entry = t.lut[0b10_0000_0000];
        assert_eq!(entry.symbol, 0x00, "code 1 -> (0,0)");
        assert_eq!(entry.bits, 1);

        // 01_00000000 -> code=01 (len=2) -> sym=(1,0)=0x10
        let entry = t.lut[0b01_0000_0000];
        assert_eq!(entry.symbol, 0x10, "code 01 -> (1,0)");
        assert_eq!(entry.bits, 2);

        // 001_0000000 -> code=001 (len=3) -> sym=(0,1)=0x01
        let entry = t.lut[0b001_0000000];
        assert_eq!(entry.symbol, 0x01, "code 001 -> (0,1)");
        assert_eq!(entry.bits, 3);

        // 000_0000000 -> code=000 (len=3) -> sym=(1,1)=0x11
        let entry = t.lut[0b000_0000000];
        assert_eq!(entry.symbol, 0x11, "code 000 -> (1,1)");
        assert_eq!(entry.bits, 3);
    }

    /// 验证 Table 24 的 LUT 基本属性
    #[test]
    fn test_table24_lut_coverage() {
        let tables = get_big_value_tables();
        let t = &tables[24];

        // LUT 应该有 1024 个条目
        assert_eq!(t.lut.len(), 1024);

        // Table 24 存在 >10bit 的码字, 因此 LUT 允许出现 bits=0 入口,
        // 这些入口会通过 overflow 长码分支解码.
        let mut zero_bits = 0;
        let mut sym_count = [0u32; 256];
        for entry in &t.lut {
            if entry.bits == 0 {
                zero_bits += 1;
            } else {
                sym_count[entry.symbol as usize] += 1;
            }
        }
        assert!(zero_bits > 0, "Table 24 预期存在 overflow 入口");
        assert!(!t.overflow.is_empty(), "Table 24 overflow 不应为空");

        // table24 显式码表中 0x00 (0,0) 是 4 位最短码, 覆盖 64 个 LUT 条目
        assert_eq!(sym_count[0x00], 64, "0x00 应覆盖 64 个 LUT 条目 (4位码)");

        // 检查不同符号的数量 - 应该 > 100 (256个符号中大部分在LUT中)
        let distinct_syms = sym_count.iter().filter(|&&c| c > 0).count();
        assert!(distinct_syms > 50, "LUT 应包含多种不同符号");
    }

    /// 全面验证: 对所有 Big Value 表的每个符号进行编码→解码往返测试
    /// 这验证了 canonical code 生成和 LUT 填充的正确性
    #[test]
    fn test_all_big_value_tables_roundtrip() {
        let tables = get_big_value_tables();

        for table_id in 1..32u8 {
            if table_id == 0 || table_id == 4 || table_id == 14 {
                continue;
            }

            let count = match table_id {
                1 => 4,
                2..=3 => 9,
                5..=6 => 16,
                7..=9 => 36,
                10..=12 => 64,
                13 | 15 => 256,
                16..=31 => 256,
                _ => 0,
            };

            let table = &tables[table_id as usize];

            let entries: Vec<(u32, u8, u8)> =
                if let Some((codes, lens, wrap)) = explicit_codebook(table_id) {
                    build_explicit_entries(codes, lens, wrap)
                } else {
                    let offset = MPA_HUFF_OFFSET[table_id as usize];
                    let lens = &MPA_HUFF_LENS[offset..offset + count];
                    let syms = &MPA_HUFF_SYMS[offset..offset + count];

                    // 重新生成 canonical codes (与 build_big_value_table 相同的算法)
                    let mut code: u64 = 0;
                    let mut entries: Vec<(u32, u8, u8)> = Vec::new();
                    for i in 0..count {
                        let len = lens[i];
                        if len > 0 {
                            let code_val = (code >> (32 - len as u64)) as u32;
                            entries.push((code_val, len, syms[i]));
                            code += 1u64 << (32 - len as u64);
                        }
                    }
                    entries
                };

            // 对每个 entry 编码到 bitstream, 再用 LUT 解码
            let mut errors = 0;
            for &(code_val, len, expected_sym) in &entries {
                // 将 code_val (len 位) 编码到字节数组
                // 需要至少 (len + PEEK_BITS) 位的空间来确保 peek 能工作
                let total_bits = len as usize + PEEK_BITS;
                let total_bytes = total_bits.div_ceil(8);
                let mut buf = vec![0u8; total_bytes + 2];

                // MSB first: 将 code_val 放在 buf 的最前面
                for bit in 0..len {
                    let bit_val = (code_val >> (len - 1 - bit)) & 1;
                    if bit_val == 1 {
                        buf[bit as usize / 8] |= 0x80 >> (bit as usize % 8);
                    }
                }

                // 通过 LUT 解码
                let peek_val = if len as usize <= PEEK_BITS {
                    // 构建 peek 值: code_val 左移 padding 位
                    let pad = PEEK_BITS - len as usize;
                    (code_val << pad) as usize
                } else {
                    // 溢出情况, 需要通过 BitReader 测试
                    let br = BitReader::new(&buf);
                    let peek = br.peek_bits(PEEK_BITS as u8).unwrap_or(0);
                    peek as usize
                };

                if (len as usize) <= PEEK_BITS {
                    let entry = table.lut[peek_val];
                    if entry.symbol != expected_sym || entry.bits != len {
                        errors += 1;
                    }
                } else {
                    // 溢出条目: 通过完整的 decode 流程测试
                    let mut br = BitReader::new(&buf);
                    let decoder = HuffmanDecoder::new();
                    match decoder.decode_big_value_vlc(&mut br, table) {
                        Ok(decoded_sym) => {
                            if decoded_sym != expected_sym {
                                errors += 1;
                            }
                        }
                        Err(_) => {
                            errors += 1;
                        }
                    }
                }
            }

            if errors > 0 {
                panic!("Table {}: {} 个符号解码错误!", table_id, errors);
            }
        }
    }

    /// 端到端测试: 编码已知值, 解码验证
    #[test]
    fn test_table1_round_trip() {
        // Table 1 LENS=[3,3,2,1] SYMS=[0x11,0x01,0x10,0x00]
        // Canonical Huffman 码 (长码先分配):
        //   sym=0x11 (1,1): len=3, code="000"
        //   sym=0x01 (0,1): len=3, code="001"
        //   sym=0x10 (1,0): len=2, code="01"
        //   sym=0x00 (0,0): len=1, code="1"
        //
        // 非零值后跟 sign bit (0=正, 1=负)
        // 编码序列: (0,0) (+1,0) (0,+1) (+1,+1)
        //   (0,0):   "1"                            = 1 bit
        //   (+1,0):  "01" + sign_x="0"              = 3 bits
        //   (0,+1):  "001" + sign_y="0"             = 4 bits
        //   (+1,+1): "000" + sign_x="0" sign_y="0"  = 5 bits
        // 合计: 1 010 0010 000 00 _ = 0xA2 0x00 0x00
        let data = [0xA2, 0x00, 0x00];
        let mut br = BitReader::new(&data);
        let decoder = HuffmanDecoder::new();

        let (x, y) = decoder.decode_big_values(&mut br, 1, 0).unwrap();
        assert_eq!((x, y), (0, 0), "第1对应为 (0,0)");

        let (x, y) = decoder.decode_big_values(&mut br, 1, 0).unwrap();
        assert_eq!((x, y), (1, 0), "第2对应为 (+1,0)");

        let (x, y) = decoder.decode_big_values(&mut br, 1, 0).unwrap();
        assert_eq!((x, y), (0, 1), "第3对应为 (0,+1)");

        let (x, y) = decoder.decode_big_values(&mut br, 1, 0).unwrap();
        assert_eq!((x, y), (1, 1), "第4对应为 (+1,+1)");
    }

    /// 与 symphonia/FFmpeg 显式码表对照的最小样例:
    /// table24 的若干短码映射.
    #[test]
    fn test_table24_known_codes() {
        let decoder = HuffmanDecoder::new();
        let tables = get_big_value_tables();
        let table24 = &tables[24];

        // table24: value=0x00 -> code=1111 (len=4)
        let data = [0b1111_0000u8];
        let mut br = BitReader::new(&data);
        let sym = decoder.decode_big_value_vlc(&mut br, table24).unwrap();
        assert_eq!(sym, 0x00, "table24 code 1111 应映射到 0x00");

        // table24: value=0x01 -> code=1101 (len=4)
        let data = [0b1101_0000u8];
        let mut br = BitReader::new(&data);
        let sym = decoder.decode_big_value_vlc(&mut br, table24).unwrap();
        assert_eq!(sym, 0x01, "table24 code 1101 应映射到 0x01");

        // table24: value=0x10 -> code=1110 (len=4)
        let data = [0b1110_0000u8];
        let mut br = BitReader::new(&data);
        let sym = decoder.decode_big_value_vlc(&mut br, table24).unwrap();
        assert_eq!(sym, 0x10, "table24 code 1110 应映射到 0x10");
    }
}
