//! H.264 CAVLC (Context-Adaptive Variable Length Coding) 残差系数解码.
//!
//! 实现 coeff_token / level / total_zeros / run_before 语法元素的 VLC 解码,
//! 以及完整的 4x4/chroma DC 残差块重建管线.
//!
//! VLC 表数据来源: ITU-T H.264 Table 9-5 ~ Table 9-10.

use std::sync::atomic::{AtomicUsize, Ordering};
use tao_core::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

static COEFF_TOKEN_FALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);
static TOTAL_ZEROS_FALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

// ============================================================
// coeff_token VLC 表 (H.264 Table 9-5)
// ============================================================
//
// 布局: [TC * 4 + T] 其中 TC=0..16, T=0..3
// len=0 表示无效组合 (如 TC=0 时 T>0)
// bits 为 MSB-first 码字数值

/// nC < 2 (Num-VLC0)
const COEFF_TOKEN_LEN_0: [u8; 68] = [
    1, 0, 0, 0, 6, 2, 0, 0, 8, 6, 3, 0, 9, 8, 7, 5, 10, 9, 8, 6, 11, 10, 9, 7, 13, 11, 10, 8, 13,
    13, 11, 9, 13, 13, 13, 10, 14, 14, 13, 11, 14, 14, 14, 13, 15, 15, 14, 14, 15, 15, 15, 14, 16,
    15, 15, 15, 16, 16, 16, 15, 16, 16, 16, 16, 16, 16, 16, 16,
];

const COEFF_TOKEN_BITS_0: [u8; 68] = [
    1, 0, 0, 0, 5, 1, 0, 0, 7, 4, 1, 0, 7, 6, 5, 3, 7, 6, 5, 3, 7, 6, 5, 4, 15, 6, 5, 4, 11, 14, 5,
    4, 8, 10, 13, 4, 15, 14, 9, 4, 11, 10, 13, 12, 15, 14, 9, 12, 11, 10, 13, 8, 15, 1, 9, 12, 11,
    14, 13, 8, 7, 10, 9, 12, 4, 6, 5, 8,
];

/// 2 <= nC < 4 (Num-VLC1)
const COEFF_TOKEN_LEN_1: [u8; 68] = [
    2, 0, 0, 0, 6, 2, 0, 0, 6, 5, 3, 0, 7, 6, 6, 4, 8, 6, 6, 4, 8, 7, 7, 5, 9, 8, 8, 6, 11, 9, 9,
    6, 11, 11, 11, 7, 12, 11, 11, 9, 12, 12, 12, 11, 12, 12, 12, 11, 13, 13, 13, 12, 13, 13, 13,
    13, 13, 14, 13, 13, 14, 14, 14, 13, 14, 14, 14, 14,
];

const COEFF_TOKEN_BITS_1: [u8; 68] = [
    3, 0, 0, 0, 11, 2, 0, 0, 7, 7, 3, 0, 7, 10, 9, 5, 7, 6, 5, 4, 4, 6, 5, 6, 7, 6, 5, 8, 15, 6, 5,
    4, 11, 14, 13, 4, 15, 10, 9, 4, 11, 14, 13, 12, 8, 10, 9, 8, 15, 14, 13, 12, 11, 10, 9, 12, 7,
    11, 6, 8, 9, 8, 10, 1, 7, 6, 5, 4,
];

/// 4 <= nC < 8 (Num-VLC2)
const COEFF_TOKEN_LEN_2: [u8; 68] = [
    4, 0, 0, 0, 6, 4, 0, 0, 6, 5, 4, 0, 6, 5, 5, 4, 7, 5, 5, 4, 7, 5, 5, 4, 7, 6, 6, 4, 7, 6, 6, 4,
    8, 7, 7, 5, 8, 8, 7, 6, 9, 8, 8, 7, 9, 9, 8, 8, 9, 9, 9, 8, 10, 9, 9, 9, 10, 10, 10, 10, 10,
    10, 10, 10, 10, 10, 10, 10,
];

const COEFF_TOKEN_BITS_2: [u8; 68] = [
    15, 0, 0, 0, 15, 14, 0, 0, 11, 15, 13, 0, 8, 12, 14, 12, 15, 10, 11, 11, 11, 8, 9, 10, 9, 14,
    13, 9, 8, 10, 9, 8, 15, 14, 13, 13, 11, 14, 10, 12, 15, 10, 13, 12, 11, 14, 9, 12, 8, 10, 13,
    8, 13, 7, 9, 12, 9, 12, 11, 10, 5, 8, 7, 6, 1, 4, 3, 2,
];

/// nC >= 8 (Num-FLC): 固定 6 位码
const COEFF_TOKEN_LEN_3: [u8; 68] = [
    6, 0, 0, 0, 6, 6, 0, 0, 6, 6, 6, 0, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6,
];

const COEFF_TOKEN_BITS_3: [u8; 68] = [
    3, 0, 0, 0, 0, 1, 0, 0, 4, 5, 6, 0, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46,
    47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
];

/// Chroma DC coeff_token (4:2:0, max 4 系数)
const CHROMA_DC_COEFF_TOKEN_LEN: [u8; 20] =
    [2, 0, 0, 0, 6, 1, 0, 0, 6, 6, 3, 0, 6, 7, 7, 6, 6, 8, 8, 7];

const CHROMA_DC_COEFF_TOKEN_BITS: [u8; 20] =
    [1, 0, 0, 0, 7, 1, 0, 0, 4, 6, 1, 0, 3, 3, 2, 5, 2, 3, 2, 0];

// ============================================================
// total_zeros VLC 表 (H.264 Table 9-7)
// ============================================================
//
// total_zeros_len[total_coeff-1][total_zeros]
// total_zeros_bits[total_coeff-1][total_zeros]

const TOTAL_ZEROS_LEN: [[u8; 16]; 15] = [
    [1, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 9],
    [3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 6, 6, 6, 6, 0],
    [4, 3, 3, 3, 4, 4, 3, 3, 4, 5, 5, 6, 5, 6, 0, 0],
    [5, 3, 4, 4, 3, 3, 3, 4, 3, 4, 5, 5, 5, 0, 0, 0],
    [4, 4, 4, 3, 3, 3, 3, 3, 4, 5, 4, 5, 0, 0, 0, 0],
    [6, 5, 3, 3, 3, 3, 3, 3, 4, 3, 6, 0, 0, 0, 0, 0],
    [6, 5, 3, 3, 3, 2, 3, 4, 3, 6, 0, 0, 0, 0, 0, 0],
    [6, 4, 5, 3, 2, 2, 3, 3, 6, 0, 0, 0, 0, 0, 0, 0],
    [6, 6, 4, 2, 2, 3, 2, 5, 0, 0, 0, 0, 0, 0, 0, 0],
    [5, 5, 3, 2, 2, 2, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [4, 4, 3, 3, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [4, 4, 2, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

const TOTAL_ZEROS_BITS: [[u8; 16]; 15] = [
    [1, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 1],
    [7, 6, 5, 4, 3, 5, 4, 3, 2, 3, 2, 3, 2, 1, 0, 0],
    [5, 7, 6, 5, 4, 3, 4, 3, 2, 3, 2, 1, 1, 0, 0, 0],
    [3, 7, 5, 4, 6, 5, 4, 3, 3, 2, 2, 1, 0, 0, 0, 0],
    [5, 4, 3, 7, 6, 5, 4, 3, 2, 1, 1, 0, 0, 0, 0, 0],
    [1, 1, 7, 6, 5, 4, 3, 2, 1, 1, 0, 0, 0, 0, 0, 0],
    [1, 1, 5, 4, 3, 3, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 3, 3, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 1, 3, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 1, 3, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 2, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

/// Chroma DC total_zeros (4:2:0, max 4 系数)
const CHROMA_DC_TOTAL_ZEROS_LEN: [[u8; 4]; 3] = [[1, 2, 3, 3], [1, 2, 2, 0], [1, 1, 0, 0]];

const CHROMA_DC_TOTAL_ZEROS_BITS: [[u8; 4]; 3] = [[1, 1, 1, 0], [1, 1, 0, 0], [1, 0, 0, 0]];

// ============================================================
// run_before VLC 表 (H.264 Table 9-10)
// ============================================================
//
// run_len[min(zeros_left,7)-1][run_before]
// run_bits[min(zeros_left,7)-1][run_before]

const RUN_LEN: [[u8; 16]; 7] = [
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 2, 2, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 2, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [2, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 3, 3, 3, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0],
];

const RUN_BITS: [[u8; 16]; 7] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 2, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 0, 1, 3, 2, 5, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [7, 6, 5, 4, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
];

// ============================================================
// 通用 VLC 解码
// ============================================================

/// 通用 VLC 解码: 从比特流中匹配最短的有效码字.
///
/// 返回匹配条目的索引.
fn decode_vlc(br: &mut BitReader, lens: &[u8], bits: &[u8], nb_codes: usize) -> TaoResult<usize> {
    let max_len = lens[..nb_codes].iter().copied().max().unwrap_or(0) as u32;
    if max_len == 0 {
        return Err(TaoError::InvalidData("CAVLC VLC 表为空".into()));
    }
    let avail = (br.bits_left() as u32).min(max_len);
    if avail == 0 {
        return Err(TaoError::InvalidData("CAVLC 比特流不足".into()));
    }
    let peeked = br.peek_bits(avail)?;

    let mut best_idx: Option<usize> = None;
    let mut best_len = u8::MAX;

    for idx in 0..nb_codes {
        let len = lens[idx];
        if len == 0 || len as u32 > avail || len >= best_len {
            continue;
        }
        let shift = avail - len as u32;
        if (peeked >> shift) == bits[idx] as u32 {
            best_idx = Some(idx);
            best_len = len;
        }
    }

    match best_idx {
        Some(idx) => {
            br.skip_bits(best_len as u32)?;
            Ok(idx)
        }
        None => Err(TaoError::InvalidData("CAVLC VLC 码字匹配失败".into())),
    }
}

// ============================================================
// coeff_token 解码
// ============================================================

/// 选择 coeff_token VLC 表 (基于 nC).
fn select_coeff_token_table_index(nc: i32) -> usize {
    match nc {
        0..=1 => 0,
        2..=3 => 1,
        4..=7 => 2,
        _ => 3,
    }
}

fn coeff_token_table_by_index(table_idx: usize) -> (&'static [u8; 68], &'static [u8; 68]) {
    match table_idx {
        0 => (&COEFF_TOKEN_LEN_0, &COEFF_TOKEN_BITS_0),
        1 => (&COEFF_TOKEN_LEN_1, &COEFF_TOKEN_BITS_1),
        2 => (&COEFF_TOKEN_LEN_2, &COEFF_TOKEN_BITS_2),
        _ => (&COEFF_TOKEN_LEN_3, &COEFF_TOKEN_BITS_3),
    }
}

fn decode_coeff_token_with_table(br: &mut BitReader, table_idx: usize) -> TaoResult<(u8, u8)> {
    let (lens, bits) = coeff_token_table_by_index(table_idx);
    let idx = decode_vlc(br, lens.as_slice(), bits.as_slice(), 68)?;
    let tc = (idx / 4) as u8;
    let t = (idx % 4) as u8;
    Ok((tc, t))
}

fn coeff_token_fallback_tables(primary_table: usize) -> &'static [usize] {
    match primary_table {
        0 => &[1],
        1 => &[0, 2],
        2 => &[1, 3],
        _ => &[2],
    }
}

fn coeff_token_fallback_enabled() -> bool {
    std::env::var("TAO_H264_CAVLC_ALLOW_COEFF_TOKEN_FALLBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn total_zeros_fallback_enabled() -> bool {
    std::env::var("TAO_H264_CAVLC_ALLOW_TOTAL_ZEROS_FALLBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 解码 coeff_token, 返回 (total_coeff, trailing_ones).
pub fn decode_coeff_token(br: &mut BitReader, nc: i32) -> TaoResult<(u8, u8)> {
    if nc == -1 {
        let idx = decode_vlc(
            br,
            &CHROMA_DC_COEFF_TOKEN_LEN,
            &CHROMA_DC_COEFF_TOKEN_BITS,
            20,
        )?;
        let tc = (idx / 4) as u8;
        let t = (idx % 4) as u8;
        return Ok((tc, t));
    }

    let primary_table = select_coeff_token_table_index(nc);
    if let Ok(parsed) = decode_coeff_token_with_table(br, primary_table) {
        return Ok(parsed);
    }

    if !coeff_token_fallback_enabled() {
        return Err(TaoError::InvalidData(format!(
            "CAVLC coeff_token 解码失败(nC={}, table_idx={})",
            nc, primary_table
        )));
    }

    // 仅尝试相邻 VLC 表, 避免跨级别回退带来的过度容错误解码.
    for &table_idx in coeff_token_fallback_tables(primary_table) {
        if let Ok(parsed) = decode_coeff_token_with_table(br, table_idx) {
            if std::env::var("TAO_H264_CAVLC_TRACE_FALLBACK")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
            {
                let idx = COEFF_TOKEN_FALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
                if idx < 64 {
                    println!(
                        "[H264-CAVLC-FALLBACK] nc={} primary={} fallback={} bits_read={}",
                        nc,
                        primary_table,
                        table_idx,
                        br.bits_read()
                    );
                }
            }
            return Ok(parsed);
        }
    }

    Err(TaoError::InvalidData(format!(
        "CAVLC coeff_token 解码失败(nC={}, table_idx={})",
        nc, primary_table
    )))
}

// ============================================================
// level 解码
// ============================================================

/// 读取 level_prefix: 计算前导零数量 (消费零与终止 "1" 位).
fn read_level_prefix(br: &mut BitReader) -> TaoResult<u32> {
    let mut prefix = 0u32;
    loop {
        let bit = br.read_bit()?;
        if bit == 1 {
            break;
        }
        prefix += 1;
        if prefix > 28 {
            return Err(TaoError::InvalidData("CAVLC level_prefix 过长".into()));
        }
    }
    Ok(prefix)
}

/// 解码单个 level 系数值 (含符号).
///
/// `suffix_length`: 当前后缀长度, 会在调用后更新.
/// `is_first_non_t1`: 是否为第一个非 trailing_one 系数.
/// `trailing_ones_lt3`: trailing_ones < 3.
fn decode_level(
    br: &mut BitReader,
    suffix_length: &mut u32,
    is_first_non_t1: bool,
    trailing_ones_lt3: bool,
) -> TaoResult<i32> {
    let prefix = read_level_prefix(br)?;

    let suffix_size = if prefix == 14 && *suffix_length == 0 {
        4u32
    } else if prefix >= 15 {
        prefix.saturating_sub(3)
    } else {
        *suffix_length
    };

    let level_suffix = if suffix_size > 0 {
        br.read_bits(suffix_size)? as i32
    } else {
        0i32
    };

    let mut level_code = ((prefix.min(15) as i32) << *suffix_length) + level_suffix;
    if prefix >= 15 && *suffix_length == 0 {
        level_code += 15;
    }
    if prefix >= 16 {
        level_code += (1i32 << (prefix as i32 - 3)) - 4096;
    }

    if is_first_non_t1 && trailing_ones_lt3 {
        level_code += 2;
    }

    // 更新 suffix_length
    if *suffix_length == 0 {
        *suffix_length = 1;
    }
    let abs_level_code = if level_code < 0 {
        level_code.unsigned_abs()
    } else {
        level_code as u32
    };
    let threshold = match *suffix_length {
        0 => 0u32,
        1 => 3,
        2 => 6,
        3 => 12,
        4 => 24,
        5 => 48,
        _ => u32::MAX,
    };
    // level_code 是无符号中间值, (level_code+2)/2 = |level|
    let abs_level = (abs_level_code + 2) / 2;
    if abs_level > threshold {
        *suffix_length += 1;
    }

    // 转换为有符号 level
    let sign = level_code & 1;
    let level = ((level_code + 2) >> 1) as i32;
    Ok(if sign != 0 { -level } else { level })
}

// ============================================================
// total_zeros 解码
// ============================================================

/// 解码 total_zeros.
pub fn decode_total_zeros(
    br: &mut BitReader,
    total_coeff: u8,
    is_chroma_dc: bool,
    max_num_coeff: usize,
) -> TaoResult<u8> {
    if is_chroma_dc {
        let tc_idx = (total_coeff as usize).saturating_sub(1).min(2);
        let idx = decode_vlc(
            br,
            &CHROMA_DC_TOTAL_ZEROS_LEN[tc_idx],
            &CHROMA_DC_TOTAL_ZEROS_BITS[tc_idx],
            4,
        )?;
        return Ok(idx as u8);
    }

    let tc_idx = (total_coeff as usize).saturating_sub(1).min(14);
    let max_coeff = max_num_coeff.clamp(1, 16) as u8;
    let max_zeros = max_coeff.saturating_sub(total_coeff);
    let nb_codes = (max_zeros as usize + 1).min(16);

    let primary = decode_vlc(
        br,
        &TOTAL_ZEROS_LEN[tc_idx],
        &TOTAL_ZEROS_BITS[tc_idx],
        nb_codes,
    );
    if let Ok(idx) = primary {
        return Ok(idx as u8);
    }

    if !total_zeros_fallback_enabled() || nb_codes >= 16 {
        return primary.map(|idx| idx as u8);
    }

    // 仅在显式开启时允许 total_zeros 扩表回退, 默认保持与参考实现一致.
    if let Ok(idx) = decode_vlc(br, &TOTAL_ZEROS_LEN[tc_idx], &TOTAL_ZEROS_BITS[tc_idx], 16) {
        let clamped_idx = if max_coeff < 16 {
            idx.min(max_zeros as usize)
        } else {
            idx
        };
        if std::env::var("TAO_H264_CAVLC_TRACE_FALLBACK")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            let seq = TOTAL_ZEROS_FALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
            if seq < 64 {
                println!(
                    "[H264-CAVLC-FALLBACK] kind=total_zeros tc={} max_coeff={} nb_codes={} raw_idx={} clamped_idx={} bits_read={}",
                    total_coeff,
                    max_coeff,
                    nb_codes,
                    idx,
                    clamped_idx,
                    br.bits_read()
                );
            }
        }
        return Ok(clamped_idx as u8);
    }

    primary.map(|idx| idx as u8)
}

// ============================================================
// run_before 解码
// ============================================================

/// 解码 run_before.
pub fn decode_run_before(br: &mut BitReader, zeros_left: u8) -> TaoResult<u8> {
    if zeros_left == 0 {
        return Ok(0);
    }
    let table_idx = (zeros_left as usize).min(7) - 1;
    let nb_codes = if table_idx < 6 {
        zeros_left as usize + 1
    } else {
        // zeros_left >= 7 时使用 Table 9-10 的完整 15 码字集合.
        15
    };
    let idx = decode_vlc(br, &RUN_LEN[table_idx], &RUN_BITS[table_idx], nb_codes)?;
    Ok(idx as u8)
}

// ============================================================
// 残差块解码主入口
// ============================================================

/// CAVLC 残差块解码.
///
/// 解码一个变换块的残差系数, 输出为扫描顺序 (scan order) 的系数数组.
///
/// - `nc`: 非零系数上下文 (nC). -1 表示 chroma DC.
/// - `max_num_coeff`: 块的最大系数数 (4=chroma DC 4:2:0, 15=I_16x16 AC, 16=普通 4x4).
/// - `coeffs`: 输出系数缓冲区, 长度至少 `max_num_coeff`.
///
/// 返回 `total_coeff` (非零系数数量), 供上层更新 nC 缓存.
pub fn decode_cavlc_residual_block(
    br: &mut BitReader,
    nc: i32,
    max_num_coeff: usize,
    coeffs: &mut [i32],
) -> TaoResult<u8> {
    debug_assert!(coeffs.len() >= max_num_coeff);
    for c in coeffs[..max_num_coeff].iter_mut() {
        *c = 0;
    }

    let is_chroma_dc = nc == -1;

    // 1. coeff_token → (total_coeff, trailing_ones)
    let (total_coeff, trailing_ones) = decode_coeff_token(br, nc)
        .map_err(|err| TaoError::InvalidData(format!("CAVLC coeff_token 解码失败: {}", err)))?;
    if total_coeff == 0 {
        return Ok(0);
    }
    if total_coeff as usize > max_num_coeff {
        return Err(TaoError::InvalidData(format!(
            "CAVLC total_coeff={} 超过 max_num_coeff={}",
            total_coeff, max_num_coeff
        )));
    }

    // 2. trailing_ones 符号位 (从高频到低频)
    let tc = total_coeff as usize;
    let t1 = trailing_ones as usize;
    let mut level = [0i32; 16];
    for lev in level[..t1].iter_mut() {
        let sign = br.read_bit().map_err(|err| {
            TaoError::InvalidData(format!("CAVLC trailing_ones 符号位读取失败: {}", err))
        })?;
        *lev = if sign == 1 { -1 } else { 1 };
    }

    // 3. 剩余 level 解码
    let trailing_ones_lt3 = trailing_ones < 3;
    let mut suffix_length: u32 = if total_coeff > 10 && trailing_ones_lt3 {
        1
    } else {
        0
    };

    for (i, lev) in level[t1..tc].iter_mut().enumerate() {
        let is_first = i == 0;
        *lev =
            decode_level(br, &mut suffix_length, is_first, trailing_ones_lt3).map_err(|err| {
                TaoError::InvalidData(format!("CAVLC level 解码失败(i={}): {}", i + t1, err))
            })?;
    }

    // 4. total_zeros
    let total_zeros = if (total_coeff as usize) < max_num_coeff {
        decode_total_zeros(br, total_coeff, is_chroma_dc, max_num_coeff).map_err(|err| {
            TaoError::InvalidData(format!(
                "CAVLC total_zeros 解码失败(nc={}, total_coeff={}, trailing_ones={}, max_num_coeff={}): {}",
                nc, total_coeff, trailing_ones, max_num_coeff, err
            ))
        })?
    } else {
        0u8
    };

    // 5. run_before 与系数放置
    let mut zeros_left = total_zeros;
    // 扫描位置从高频端开始: total_coeff + total_zeros - 1
    let mut scan_pos = (tc as i32 + total_zeros as i32 - 1) as usize;
    if scan_pos >= max_num_coeff {
        return Err(TaoError::InvalidData(format!(
            "CAVLC 扫描位置越界: scan_pos={}, max={}",
            scan_pos, max_num_coeff
        )));
    }

    // 放置第一个系数 (最高频)
    coeffs[scan_pos] = level[0];

    // 放置后续系数
    for (run_idx, lev) in level[1..tc].iter().enumerate() {
        let run = if zeros_left > 0 {
            decode_run_before(br, zeros_left).map_err(|err| {
                TaoError::InvalidData(format!(
                    "CAVLC run_before 解码失败(step={}, total_coeff={}, trailing_ones={}, total_zeros={}, zeros_left={}): {}",
                    run_idx + 1,
                    total_coeff,
                    trailing_ones,
                    total_zeros,
                    zeros_left,
                    err
                ))
            })?
        } else {
            0
        };
        if run > zeros_left {
            return Err(TaoError::InvalidData(format!(
                "CAVLC run_before={} 超过 zeros_left={}",
                run, zeros_left
            )));
        }
        zeros_left -= run;
        scan_pos = scan_pos
            .checked_sub(1 + run as usize)
            .ok_or_else(|| TaoError::InvalidData("CAVLC 扫描位置下溢".into()))?;
        coeffs[scan_pos] = *lev;
    }

    Ok(total_coeff)
}

// ============================================================
// 单元测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_br(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data)
    }

    #[test]
    fn test_coeff_token_nc0_zero_coeffs() {
        // nC=0, TC=0 T=0: 码字 "1" (len=1, bits=1)
        let mut br = make_br(&[0b10000000]);
        let (tc, t) = decode_coeff_token(&mut br, 0).unwrap();
        assert_eq!((tc, t), (0, 0), "全零块应返回 TC=0, T=0");
        assert_eq!(br.bits_read(), 1, "应消费 1 比特");
    }

    #[test]
    fn test_coeff_token_nc0_one_trailing() {
        // nC=0, TC=1 T=1: 码字 "01" (len=2, bits=1)
        let mut br = make_br(&[0b01000000]);
        let (tc, t) = decode_coeff_token(&mut br, 0).unwrap();
        assert_eq!((tc, t), (1, 1), "单个 trailing one");
        assert_eq!(br.bits_read(), 2);
    }

    #[test]
    fn test_coeff_token_nc0_two_trailing() {
        // nC=0, TC=2 T=2: 码字 "001" (len=3, bits=1)
        let mut br = make_br(&[0b00100000]);
        let (tc, t) = decode_coeff_token(&mut br, 0).unwrap();
        assert_eq!((tc, t), (2, 2));
        assert_eq!(br.bits_read(), 3);
    }

    #[test]
    fn test_coeff_token_nc8_flc() {
        // nC>=8 使用固定 6 位码
        // TC=1, T=0: 码字值=0, len=6 → "000000"
        let mut br = make_br(&[0b00000000]);
        let (tc, t) = decode_coeff_token(&mut br, 8).unwrap();
        assert_eq!((tc, t), (1, 0), "nC>=8 FLC TC=1 T=0");
    }

    #[test]
    fn test_coeff_token_chroma_dc_zero() {
        // Chroma DC, TC=0, T=0: 码字 "01" (len=2, bits=1)
        let mut br = make_br(&[0b01000000]);
        let (tc, t) = decode_coeff_token(&mut br, -1).unwrap();
        assert_eq!((tc, t), (0, 0));
    }

    #[test]
    fn test_total_zeros_tc1() {
        // total_coeff=1, total_zeros=0: 码字 "1" (len=1, bits=1)
        let mut br = make_br(&[0b10000000]);
        let tz = decode_total_zeros(&mut br, 1, false, 16).unwrap();
        assert_eq!(tz, 0);
    }

    #[test]
    fn test_total_zeros_respects_max_num_coeff() {
        // total_coeff=14 时:
        // - max_num_coeff=16: total_zeros 可为 0..2, 码字 "1" 对应 total_zeros=2
        // - max_num_coeff=15: total_zeros 仅可为 0..1, 默认关闭回退时应解码失败
        let mut br = make_br(&[0b10000000]);
        let tz = decode_total_zeros(&mut br, 14, false, 16).unwrap();
        assert_eq!(tz, 2, "max_num_coeff=16 时应允许 total_zeros=2");

        let mut br = make_br(&[0b10000000]);
        let err = decode_total_zeros(&mut br, 14, false, 15).unwrap_err();
        assert!(
            err.to_string().contains("CAVLC VLC 码字匹配失败"),
            "max_num_coeff=15 且默认关闭回退时应报错, got: {}",
            err
        );
    }

    #[test]
    fn test_run_before_zeros_left_1() {
        // zeros_left=1: run=0 → "1" (len=1), run=1 → "0" (len=1)
        let mut br = make_br(&[0b10000000]);
        assert_eq!(decode_run_before(&mut br, 1).unwrap(), 0);
        let mut br = make_br(&[0b00000000]);
        assert_eq!(decode_run_before(&mut br, 1).unwrap(), 1);
    }

    #[test]
    fn test_residual_block_all_zero() {
        // TC=0 → 码字 "1" (nC=0)
        let mut br = make_br(&[0b10000000]);
        let mut coeffs = [0i32; 16];
        let tc = decode_cavlc_residual_block(&mut br, 0, 16, &mut coeffs).unwrap();
        assert_eq!(tc, 0, "全零块 total_coeff 应为 0");
        assert!(coeffs.iter().all(|&c| c == 0), "所有系数应为零");
    }

    #[test]
    fn test_residual_block_single_trailing_one() {
        // nC=0, TC=1, T=1: 码字 "01" → 读 1 个符号位
        // trailing_one sign=0 → +1
        // total_zeros: TC=1, 码字取决于 total_zeros 值
        // total_zeros=0: "1" (len=1)
        // 整个比特流: "01" + "0"(sign=+1) + "1"(total_zeros=0)
        // = 01 0 1 = 0b01010000
        let mut br = make_br(&[0b01010000]);
        let mut coeffs = [0i32; 16];
        let tc = decode_cavlc_residual_block(&mut br, 0, 16, &mut coeffs).unwrap();
        assert_eq!(tc, 1, "应有 1 个非零系数");
        assert_eq!(coeffs[0], 1, "扫描位置 0 应为 +1");
    }

    #[test]
    fn test_level_prefix_basic() {
        // prefix=0: "1" → 0 个前导零
        let mut br = make_br(&[0b10000000]);
        assert_eq!(read_level_prefix(&mut br).unwrap(), 0);

        // prefix=3: "0001" → 3 个前导零
        let mut br = make_br(&[0b00010000]);
        assert_eq!(read_level_prefix(&mut br).unwrap(), 3);
    }
}
