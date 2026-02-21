//! H.264 残差数据解码.
//!
//! 包含 CABAC 残差语法元素解码, 反扫描, 反量化, 反 Hadamard 变换.

use super::cabac::{CabacCtx, CabacDecoder};

// ============================================================
// 块类别定义
// ============================================================

/// 残差块类别
#[derive(Clone, Copy)]
pub struct BlockCat {
    /// coded_block_flag 上下文偏移
    pub cbf_offset: usize,
    /// significant_coeff_flag 上下文偏移
    pub sig_offset: usize,
    /// last_significant_coeff_flag 上下文偏移
    pub last_offset: usize,
    /// coeff_abs_level_minus1 上下文偏移
    pub abs_offset: usize,
    /// 最大系数数量
    pub max_coeff: usize,
    /// 是否跳过 coded_block_flag 解码.
    pub skip_cbf: bool,
    /// 是否使用 8x8 专用显著性上下文映射.
    pub use_sig_map_8x8: bool,
}

/// Luma DC (I_16x16), 块类别 0
pub const CAT_LUMA_DC: BlockCat = BlockCat {
    cbf_offset: 85,
    sig_offset: 105,
    last_offset: 166,
    abs_offset: 227,
    max_coeff: 16,
    skip_cbf: false,
    use_sig_map_8x8: false,
};

/// Luma AC (I_16x16), 块类别 1
pub const CAT_LUMA_AC: BlockCat = BlockCat {
    cbf_offset: 89,
    sig_offset: 120,
    last_offset: 181,
    abs_offset: 237,
    max_coeff: 15,
    skip_cbf: false,
    use_sig_map_8x8: false,
};

/// Chroma DC (4:2:0), 块类别 2
pub const CAT_CHROMA_DC: BlockCat = BlockCat {
    cbf_offset: 97,
    sig_offset: 149,
    last_offset: 210,
    abs_offset: 257,
    max_coeff: 4,
    skip_cbf: false,
    use_sig_map_8x8: false,
};

/// Chroma AC, 块类别 3
pub const CAT_CHROMA_AC: BlockCat = BlockCat {
    cbf_offset: 101,
    sig_offset: 152,
    last_offset: 213,
    abs_offset: 266,
    max_coeff: 15,
    skip_cbf: false,
    use_sig_map_8x8: false,
};

/// Luma 4x4 (I_4x4), 块类别 4
pub const CAT_LUMA_4X4: BlockCat = BlockCat {
    cbf_offset: 93,
    sig_offset: 134,
    last_offset: 195,
    abs_offset: 247,
    max_coeff: 16,
    skip_cbf: false,
    use_sig_map_8x8: false,
};

/// Luma 8x8 块类别.
pub const CAT_LUMA_8X8: BlockCat = BlockCat {
    cbf_offset: 1012,
    sig_offset: 402,
    last_offset: 417,
    abs_offset: 426,
    max_coeff: 64,
    skip_cbf: false,
    use_sig_map_8x8: true,
};

// ============================================================
// CABAC 残差块解码
// ============================================================

/// 通过 CABAC 解码一个残差块的系数
///
/// 返回解码后的系数数组 (扫描顺序), 长度为 max_coeff.
/// `cbf_ctx_inc` 为 coded_block_flag 的上下文增量 (基于邻居).
pub fn decode_residual_block(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
    cbf_ctx_inc: usize,
) -> Vec<i32> {
    let n = cat.max_coeff;
    let mut coeffs = vec![0i32; n];

    if !cat.skip_cbf {
        // 解码 coded_block_flag
        let cbf_idx = cat.cbf_offset + cbf_ctx_inc.min(3);
        let cbf = cabac.decode_decision(&mut ctxs[cbf_idx]);
        if cbf == 0 {
            return coeffs;
        }
    }

    // 找出非零系数位置
    let sig_positions = if cat.use_sig_map_8x8 {
        decode_significance_map_8x8(cabac, ctxs, cat)
    } else {
        decode_significance_map(cabac, ctxs, cat)
    };

    // 解码系数值 (从最后一个非零系数开始, 反向解码)
    decode_coeff_values(cabac, ctxs, cat, &sig_positions, &mut coeffs);

    coeffs
}

/// 8x8 变换块的显著性图解码.
fn decode_significance_map_8x8(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
) -> Vec<usize> {
    let mut positions = Vec::new();
    for i in 0..63usize {
        let sig_idx = cat.sig_offset + usize::from(SIG_COEFF_FLAG_OFFSET_8X8[i]);
        let sig = cabac.decode_decision(&mut ctxs[sig_idx]);
        if sig == 1 {
            positions.push(i);
            let last_idx = cat.last_offset + usize::from(LAST_COEFF_FLAG_OFFSET_8X8[i]);
            let last = cabac.decode_decision(&mut ctxs[last_idx]);
            if last == 1 {
                positions.reverse();
                return positions;
            }
        }
    }
    positions.push(63);
    positions.reverse();
    positions
}

/// 解码显著性图: 返回非零系数的扫描位置 (降序排列)
fn decode_significance_map(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
) -> Vec<usize> {
    let last_pos = cat.max_coeff - 1;
    let mut positions = Vec::new();

    for i in 0..last_pos {
        let sig_idx = cat.sig_offset + i;
        let sig = cabac.decode_decision(&mut ctxs[sig_idx]);
        if sig == 1 {
            positions.push(i);
            let last_idx = cat.last_offset + i;
            let last = cabac.decode_decision(&mut ctxs[last_idx]);
            if last == 1 {
                positions.reverse();
                return positions;
            }
        }
    }
    // 最后一个位置默认为显著
    positions.push(last_pos);
    positions.reverse();
    positions
}

/// 解码系数绝对值和符号 (从最后一个开始反向)
fn decode_coeff_values(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
    positions: &[usize],
    coeffs: &mut [i32],
) {
    // 与 FFmpeg 一致的节点上下文状态机.
    let mut node_ctx = 0usize;

    for &pos in positions {
        let level = decode_abs_level(cabac, ctxs, cat, &mut node_ctx);
        let sign = cabac.decode_bypass();
        coeffs[pos] = if sign == 1 {
            -(level as i32)
        } else {
            level as i32
        };
    }
}

/// 解码单个系数的绝对值
fn decode_abs_level(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
    node_ctx: &mut usize,
) -> u32 {
    const COEFF_ABS_LEVEL1_CTX: [usize; 8] = [1, 2, 3, 4, 0, 0, 0, 0];
    const COEFF_ABS_LEVELGT1_CTX: [usize; 8] = [5, 5, 5, 5, 6, 7, 8, 9];
    const TRANS_EQ1: [usize; 8] = [1, 2, 3, 3, 4, 5, 6, 7];
    const TRANS_GT1: [usize; 8] = [4, 4, 4, 4, 5, 6, 7, 7];

    let idx_level1 = cat.abs_offset + COEFF_ABS_LEVEL1_CTX[*node_ctx];
    if cabac.decode_decision(&mut ctxs[idx_level1]) == 0 {
        *node_ctx = TRANS_EQ1[*node_ctx];
        return 1;
    }

    let idx_level_gt1 = cat.abs_offset + COEFF_ABS_LEVELGT1_CTX[*node_ctx];
    *node_ctx = TRANS_GT1[*node_ctx];

    let mut coeff_abs = 2u32;
    while coeff_abs < 15 && cabac.decode_decision(&mut ctxs[idx_level_gt1]) == 1 {
        coeff_abs += 1;
    }

    if coeff_abs >= 15 {
        coeff_abs = decode_abs_suffix_bypass(cabac) + 14;
    }
    coeff_abs
}

/// 系数绝对值扩展后缀旁路解码.
///
/// 返回值等价于 FFmpeg 中的 `coeff_abs` 初值, 上层需再加 14.
fn decode_abs_suffix_bypass(cabac: &mut CabacDecoder) -> u32 {
    let mut j = 0u32;
    while {
        let bit = cabac.decode_bypass();
        bit == 1 && j < 23
    } {
        j += 1;
    }

    let mut coeff_abs = 1u32;
    for _ in 0..j {
        coeff_abs = coeff_abs + coeff_abs + cabac.decode_bypass();
    }
    coeff_abs
}

// ============================================================
// 反变换
// ============================================================

/// 4x4 Luma DC 反 Hadamard 变换 (I_16x16)
pub fn inverse_hadamard_4x4(block: &mut [i32; 16]) {
    let mut temp = [0i32; 16];

    // 行变换
    for i in 0..4 {
        let s = i * 4;
        let a = block[s] + block[s + 2];
        let b = block[s] - block[s + 2];
        let c = block[s + 1] - block[s + 3];
        let d = block[s + 1] + block[s + 3];
        temp[s] = a + d;
        temp[s + 1] = b + c;
        temp[s + 2] = b - c;
        temp[s + 3] = a - d;
    }

    // 列变换
    for j in 0..4 {
        let a = temp[j] + temp[8 + j];
        let b = temp[j] - temp[8 + j];
        let c = temp[4 + j] - temp[12 + j];
        let d = temp[4 + j] + temp[12 + j];
        block[j] = a + d;
        block[4 + j] = b + c;
        block[8 + j] = b - c;
        block[12 + j] = a - d;
    }
}

/// 2x2 Chroma DC 反 Hadamard 变换 (4:2:0)
pub fn inverse_hadamard_2x2(block: &mut [i32; 4]) {
    let a = block[0] + block[1];
    let b = block[0] - block[1];
    let c = block[2] + block[3];
    let d = block[2] - block[3];
    block[0] = a + c;
    block[1] = a - c;
    block[2] = b + d;
    block[3] = b - d;
}

// ============================================================
// 反量化
// ============================================================

/// Luma DC 系数反量化 (I_16x16)
///
/// 对 Hadamard 变换后的 DC 系数进行反量化
pub fn dequant_luma_dc(coeffs: &mut [i32; 16], qp: i32) {
    let qp_per = qp / 6;
    let qp_rem = qp % 6;
    let scale = LEVEL_SCALE[qp_rem as usize][0];

    for c in coeffs.iter_mut() {
        if qp_per >= 2 {
            *c = (*c * scale) << (qp_per - 2);
        } else {
            *c = (*c * scale + (1 << (1 - qp_per))) >> (2 - qp_per);
        }
    }
}

/// Chroma DC 系数反量化 (4:2:0)
pub fn dequant_chroma_dc(coeffs: &mut [i32; 4], qp: i32) {
    let qp_per = qp / 6;
    let qp_rem = qp % 6;
    let scale = LEVEL_SCALE[qp_rem as usize][0];

    for c in coeffs.iter_mut() {
        if qp_per >= 1 {
            *c = (*c * scale) << (qp_per - 1);
        } else {
            *c = (*c * scale) >> 1;
        }
    }
}

/// 4x4 AC 系数反量化 (通用, AC 残差解码时使用)
#[allow(dead_code)]
pub fn dequant_4x4_ac(coeffs: &mut [i32; 16], qp: i32) {
    let qp_per = qp / 6;
    let qp_rem = qp % 6;

    for (i, c) in coeffs.iter_mut().enumerate() {
        if *c == 0 {
            continue;
        }
        // 位置索引 → 缩放因子类别
        let (row, col) = ZIGZAG_4X4[i];
        let si = scale_index(row, col);
        let scale = LEVEL_SCALE[qp_rem as usize][si];
        let scaled = *c * scale;
        if qp_per >= 4 {
            *c = scaled << (qp_per - 4);
        } else {
            let shift = 4 - qp_per;
            *c = (scaled + (1 << (shift - 1))) >> shift;
        }
    }
}

/// 根据 4x4 块内位置确定缩放因子索引
#[allow(dead_code)]
fn scale_index(row: usize, col: usize) -> usize {
    let r = row & 1;
    let c = col & 1;
    if r == 0 && c == 0 {
        0
    } else if r == 1 && c == 1 {
        2
    } else {
        1
    }
}

/// 4x4 反整数 DCT 变换 (AC 残差解码时使用)
#[allow(dead_code)]
pub fn idct_4x4(coeffs: &[i32; 16], out: &mut [i32; 16]) {
    let mut temp = [0i32; 16];

    // 行变换
    for i in 0..4 {
        let s = i * 4;
        let s0 = coeffs[s];
        let s1 = coeffs[s + 1];
        let s2 = coeffs[s + 2];
        let s3 = coeffs[s + 3];
        let e0 = s0 + s2;
        let e1 = s0 - s2;
        let e2 = (s1 >> 1) - s3;
        let e3 = s1 + (s3 >> 1);
        temp[s] = e0 + e3;
        temp[s + 1] = e1 + e2;
        temp[s + 2] = e1 - e2;
        temp[s + 3] = e0 - e3;
    }

    // 列变换
    for j in 0..4 {
        let s0 = temp[j];
        let s1 = temp[4 + j];
        let s2 = temp[8 + j];
        let s3 = temp[12 + j];
        let e0 = s0 + s2;
        let e1 = s0 - s2;
        let e2 = (s1 >> 1) - s3;
        let e3 = s1 + (s3 >> 1);
        // 结果需要右移 6 位 (DCT 归一化)
        out[j] = (e0 + e3 + 32) >> 6;
        out[4 + j] = (e1 + e2 + 32) >> 6;
        out[8 + j] = (e1 - e2 + 32) >> 6;
        out[12 + j] = (e0 - e3 + 32) >> 6;
    }
}

/// 8x8 AC 系数反量化.
///
/// 输入系数需为 raster 顺序.
pub fn dequant_8x8_ac(coeffs: &mut [i32; 64], qp: i32) {
    let qp_per = qp / 6;
    let qp_rem = qp % 6;
    let rem_idx = qp_rem as usize;

    for (idx, coeff) in coeffs.iter_mut().enumerate() {
        if *coeff == 0 {
            continue;
        }

        let scale_idx = DEQUANT_8X8_SCALE_INDEX[idx];
        let scale = LEVEL_SCALE_8X8[rem_idx][scale_idx];
        let scaled = *coeff * scale;
        if qp_per >= 6 {
            *coeff = scaled << (qp_per - 6);
        } else {
            let shift = 6 - qp_per;
            *coeff = (scaled + (1 << (shift - 1))) >> shift;
        }
    }
}

/// H.264 8x8 反整数变换.
pub fn idct_8x8(coeffs: &[i32; 64], out: &mut [i32; 64]) {
    let mut tmp = [0i32; 64];

    for row in 0..8 {
        let mut src = [0i32; 8];
        src.copy_from_slice(&coeffs[row * 8..row * 8 + 8]);
        let dst = inverse_transform_1d_8(src);
        tmp[row * 8..row * 8 + 8].copy_from_slice(&dst);
    }

    for col in 0..8 {
        let src = [
            tmp[col],
            tmp[8 + col],
            tmp[16 + col],
            tmp[24 + col],
            tmp[32 + col],
            tmp[40 + col],
            tmp[48 + col],
            tmp[56 + col],
        ];
        let dst = inverse_transform_1d_8(src);
        for row in 0..8 {
            out[row * 8 + col] = (dst[row] + 32) >> 6;
        }
    }
}

fn inverse_transform_1d_8(src: [i32; 8]) -> [i32; 8] {
    let a0 = src[0] + src[4];
    let a2 = src[0] - src[4];
    let a4 = (src[2] >> 1) - src[6];
    let a6 = src[2] + (src[6] >> 1);

    let b0 = a0 + a6;
    let b2 = a2 + a4;
    let b4 = a2 - a4;
    let b6 = a0 - a6;

    let a1 = -src[3] + src[5] - src[7] - (src[7] >> 1);
    let a3 = src[1] + src[7] - src[3] - (src[3] >> 1);
    let a5 = -src[1] + src[7] + src[5] + (src[5] >> 1);
    let a7 = src[3] + src[5] + src[1] + (src[1] >> 1);

    let b1 = a1 + (a7 >> 2);
    let b7 = a7 - (a1 >> 2);
    let b3 = a3 + (a5 >> 2);
    let b5 = (a3 >> 2) - a5;

    [
        b0 + b7,
        b2 + b5,
        b4 + b3,
        b6 + b1,
        b6 - b1,
        b4 - b3,
        b2 - b5,
        b0 - b7,
    ]
}

// ============================================================
// 扫描顺序表
// ============================================================

/// 4x4 zigzag 扫描顺序 (帧编码): scan_pos → (row, col)
#[allow(dead_code)]
pub const ZIGZAG_4X4: [(usize, usize); 16] = [
    (0, 0),
    (0, 1),
    (1, 0),
    (2, 0),
    (1, 1),
    (0, 2),
    (0, 3),
    (1, 2),
    (2, 1),
    (3, 0),
    (3, 1),
    (2, 2),
    (1, 3),
    (2, 3),
    (3, 2),
    (3, 3),
];

/// 8x8 zigzag 扫描顺序 (帧编码): scan_pos -> raster_idx.
#[allow(dead_code)]
pub const ZIGZAG_8X8: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// 2x2 chroma DC 扫描顺序
#[allow(dead_code)]
pub const SCAN_CHROMA_DC: [(usize, usize); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];

// ============================================================
// 量化参数表
// ============================================================

/// LevelScale 表 (H.264 Table 8-14): [qP_rem][scale_index]
/// scale_index: 0=偶行偶列, 1=偶行奇列/奇行偶列, 2=奇行奇列
const LEVEL_SCALE: [[i32; 3]; 6] = [
    [10, 13, 16],
    [11, 14, 18],
    [13, 16, 20],
    [14, 18, 23],
    [16, 20, 25],
    [18, 23, 29],
];

/// 8x8 反量化缩放表 (H.264 Table 8-16, 默认 scaling list).
///
/// 索引顺序:
/// `0=V0, 1=V1, 2=V2, 3=V3, 4=V4, 5=V5`.
const LEVEL_SCALE_8X8: [[i32; 6]; 6] = [
    [20, 18, 32, 19, 25, 24],
    [22, 19, 35, 21, 28, 26],
    [26, 23, 42, 24, 33, 31],
    [28, 25, 45, 26, 35, 33],
    [32, 28, 51, 30, 40, 38],
    [36, 32, 58, 34, 46, 43],
];

/// 8x8 反量化位置映射 (raster idx -> scale idx).
const DEQUANT_8X8_SCALE_INDEX: [usize; 64] = [
    0, 3, 4, 3, 0, 3, 4, 3, 3, 1, 5, 1, 3, 1, 5, 1, 4, 5, 2, 5, 4, 5, 2, 5, 3, 1, 5, 1, 3, 1, 5, 1,
    0, 3, 4, 3, 0, 3, 4, 3, 3, 1, 5, 1, 3, 1, 5, 1, 4, 5, 2, 5, 4, 5, 2, 5, 3, 1, 5, 1, 3, 1, 5, 1,
];

/// 8x8 significant_coeff_flag 的上下文偏移映射 (frame).
const SIG_COEFF_FLAG_OFFSET_8X8: [u8; 63] = [
    0, 1, 2, 3, 4, 5, 5, 4, 4, 3, 3, 4, 4, 4, 5, 5, 4, 4, 4, 4, 3, 3, 6, 7, 7, 7, 8, 9, 10, 9, 8,
    7, 7, 6, 11, 12, 13, 11, 6, 7, 8, 9, 14, 10, 9, 8, 6, 11, 12, 13, 11, 6, 9, 14, 10, 9, 11, 12,
    13, 11, 14, 10, 12,
];

/// 8x8 last_significant_coeff_flag 的上下文偏移映射.
const LAST_COEFF_FLAG_OFFSET_8X8: [u8; 63] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8,
];
// ============================================================
// 量化反演和应用
// ============================================================

/// 将 4x4 残差块应用到平面上 (反扫描 + IDCT + 逐像素加法)
pub fn apply_4x4_ac_residual(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    coeffs_scan: &[i32; 16],
) {
    let mut coeffs_raster = [0i32; 16];
    for (scan_pos, &(row, col)) in ZIGZAG_4X4.iter().enumerate() {
        coeffs_raster[row * 4 + col] = coeffs_scan[scan_pos];
    }

    let mut spatial = [0i32; 16];
    idct_4x4(&coeffs_raster, &mut spatial);

    for dy in 0..4 {
        for dx in 0..4 {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                let coeff = spatial[dy * 4 + dx];
                let val = plane[idx] as i32 + coeff;
                plane[idx] = val.clamp(0, 255) as u8;
            }
        }
    }
}

/// 将 8x8 残差块应用到平面上 (反扫描 + 反量化 + IDCT + 逐像素加法)
pub fn apply_8x8_ac_residual(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    coeffs_scan: &[i32; 64],
    qp: i32,
) {
    let mut coeffs_raster = [0i32; 64];
    for (scan_pos, &raster_idx) in ZIGZAG_8X8.iter().enumerate() {
        coeffs_raster[raster_idx] = coeffs_scan[scan_pos];
    }

    dequant_8x8_ac(&mut coeffs_raster, qp);

    let mut spatial = [0i32; 64];
    idct_8x8(&coeffs_raster, &mut spatial);

    for dy in 0..8 {
        for dx in 0..8 {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                let val = plane[idx] as i32 + spatial[dy * 8 + dx];
                plane[idx] = val.clamp(0, 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idct_8x8_dc_only_produces_uniform_output() {
        let mut coeffs = [0i32; 64];
        coeffs[0] = 64;
        let mut out = [0i32; 64];

        idct_8x8(&coeffs, &mut out);

        let first = out[0];
        assert!(
            out.iter().all(|&v| v == first),
            "仅 DC 系数时 8x8 反变换输出应为常量"
        );
    }

    #[test]
    fn test_apply_8x8_ac_residual_spreads_across_all_4x4_sub_blocks() {
        let mut plane = vec![128u8; 16 * 16];
        let mut coeffs_scan = [0i32; 64];
        coeffs_scan[0] = 64;

        apply_8x8_ac_residual(&mut plane, 16, 4, 4, &coeffs_scan, 26);

        let changed = |x_begin: usize, y_begin: usize| -> usize {
            let mut count = 0usize;
            for y in y_begin..(y_begin + 4) {
                for x in x_begin..(x_begin + 4) {
                    if plane[y * 16 + x] != 128 {
                        count += 1;
                    }
                }
            }
            count
        };

        assert!(changed(4, 4) > 0, "左上 4x4 子块应有变化, 证明残差已生效");
        assert!(
            changed(8, 4) > 0,
            "右上 4x4 子块应有变化, 证明非 4x4 独立近似"
        );
        assert!(
            changed(4, 8) > 0,
            "左下 4x4 子块应有变化, 证明非 4x4 独立近似"
        );
        assert!(
            changed(8, 8) > 0,
            "右下 4x4 子块应有变化, 证明非 4x4 独立近似"
        );
    }
}
