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
}

/// Luma DC (I_16x16), 块类别 0
pub const CAT_LUMA_DC: BlockCat = BlockCat {
    cbf_offset: 85,
    sig_offset: 105,
    last_offset: 166,
    abs_offset: 227,
    max_coeff: 16,
};

/// Luma AC (I_16x16), 块类别 1
pub const CAT_LUMA_AC: BlockCat = BlockCat {
    cbf_offset: 89,
    sig_offset: 105,
    last_offset: 166,
    abs_offset: 232,
    max_coeff: 15,
};

/// Chroma DC (4:2:0), 块类别 2
pub const CAT_CHROMA_DC: BlockCat = BlockCat {
    cbf_offset: 97,
    sig_offset: 120,
    last_offset: 181,
    abs_offset: 237,
    max_coeff: 4,
};

/// Chroma AC, 块类别 3
pub const CAT_CHROMA_AC: BlockCat = BlockCat {
    cbf_offset: 101,
    sig_offset: 105,
    last_offset: 166,
    abs_offset: 242,
    max_coeff: 15,
};

/// Luma 4x4 (I_4x4), 块类别 4
pub const CAT_LUMA_4X4: BlockCat = BlockCat {
    cbf_offset: 85,
    sig_offset: 105,
    last_offset: 166,
    abs_offset: 227,
    max_coeff: 16,
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

    // 解码 coded_block_flag
    let cbf_idx = cat.cbf_offset + cbf_ctx_inc.min(3);
    let cbf = cabac.decode_decision(&mut ctxs[cbf_idx]);
    if cbf == 0 {
        return coeffs;
    }

    // 找出非零系数位置
    let sig_positions = decode_significance_map(cabac, ctxs, cat);

    // 解码系数值 (从最后一个非零系数开始, 反向解码)
    decode_coeff_values(cabac, ctxs, cat, &sig_positions, &mut coeffs);

    coeffs
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
    let mut num_eq1 = 0u32;
    let mut num_gt1 = 0u32;

    for &pos in positions {
        let level = decode_abs_level(cabac, ctxs, cat, num_eq1, num_gt1);
        let sign = cabac.decode_bypass();
        coeffs[pos] = if sign == 1 {
            -(level as i32)
        } else {
            level as i32
        };

        if level == 1 {
            num_eq1 += 1;
        } else {
            num_gt1 += 1;
        }
    }
}

/// 解码单个系数的绝对值
fn decode_abs_level(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
    num_eq1: u32,
    num_gt1: u32,
) -> u32 {
    // 前缀: 截断一元码, 最大值 14
    let ctx_inc_0 = if num_gt1 > 0 {
        0
    } else {
        (1 + num_eq1).min(4) as usize
    };
    let prefix = decode_abs_prefix(cabac, ctxs, cat, ctx_inc_0, num_gt1);

    if prefix < 14 {
        return prefix + 1;
    }

    // 后缀: Exp-Golomb k=0 旁路解码
    let suffix = decode_eg0_bypass(cabac);
    prefix + 1 + suffix
}

/// 解码系数绝对值的截断一元前缀
fn decode_abs_prefix(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    cat: &BlockCat,
    ctx_inc_0: usize,
    num_gt1: u32,
) -> u32 {
    // binIdx == 0
    let idx0 = cat.abs_offset + ctx_inc_0;
    let bin0 = cabac.decode_decision(&mut ctxs[idx0]);
    if bin0 == 0 {
        return 0;
    }

    // binIdx >= 1: 使用不同的上下文
    let ctx_inc_n = 5 + num_gt1.min(4) as usize;
    let idx_n = cat.abs_offset + ctx_inc_n;

    for i in 1..14u32 {
        let bin = cabac.decode_decision(&mut ctxs[idx_n]);
        if bin == 0 {
            return i;
        }
    }
    14
}

/// Exp-Golomb k=0 旁路解码
fn decode_eg0_bypass(cabac: &mut CabacDecoder) -> u32 {
    let mut k = 0u32;
    // 读取前缀 (连续 1 的数量)
    while cabac.decode_bypass() == 1 {
        k += 1;
        if k >= 16 {
            break;
        }
    }
    // 读取后缀
    let mut val = 0u32;
    for _ in 0..k {
        val = (val << 1) | cabac.decode_bypass();
    }
    (1 << k) - 1 + val
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
        *c = (*c * scale) << qp_per;
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
