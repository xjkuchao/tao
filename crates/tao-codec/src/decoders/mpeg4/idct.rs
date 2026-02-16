//! 整数 IDCT (IEEE 1180 兼容实现)

use super::tables::{W1, W2, W3, W5, W6, W7};

/// 8 点一维 IDCT 行变换 (带 rounding 改进)
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

    // 偶数部分: x0, x2, x4, x6
    let a0 = (W2 * x2 + W6 * x6 + 1024) >> 11;
    let a1 = (W6 * x2 - W2 * x6 + 1024) >> 11;
    let a2 = (x0 + x4) << 1;
    let a3 = (x0 - x4) << 1;

    let b0 = a2 + a0;
    let b1 = a3 + a1;
    let b2 = a3 - a1;
    let b3 = a2 - a0;

    // 奇数部分: x1, x3, x5, x7
    let c0 = (W1 * x1 + W3 * x3 + W5 * x5 + W7 * x7 + 1024) >> 11;
    let c1 = (W3 * x1 - W7 * x3 - W1 * x5 - W5 * x7 + 1024) >> 11;
    let c2 = (W5 * x1 - W1 * x3 + W7 * x5 + W3 * x7 + 1024) >> 11;
    let c3 = (W7 * x1 - W5 * x3 + W3 * x5 - W1 * x7 + 1024) >> 11;

    // 组合结果
    block[off] = b0 + c0;
    block[off + 1] = b1 + c1;
    block[off + 2] = b2 + c2;
    block[off + 3] = b3 + c3;
    block[off + 4] = b3 - c3;
    block[off + 5] = b2 - c2;
    block[off + 6] = b1 - c1;
    block[off + 7] = b0 - c0;
}

/// 8 点一维 IDCT 列变换 (带精确 rounding)
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

    // 偶数部分
    let a0 = (W2 * x2 + W6 * x6 + 1024) >> 11;
    let a1 = (W6 * x2 - W2 * x6 + 1024) >> 11;
    let a2 = (x0 + x4) << 1;
    let a3 = (x0 - x4) << 1;

    let b0 = a2 + a0;
    let b1 = a3 + a1;
    let b2 = a3 - a1;
    let b3 = a2 - a0;

    // 奇数部分
    let c0 = (W1 * x1 + W3 * x3 + W5 * x5 + W7 * x7 + 1024) >> 11;
    let c1 = (W3 * x1 - W7 * x3 - W1 * x5 - W5 * x7 + 1024) >> 11;
    let c2 = (W5 * x1 - W1 * x3 + W7 * x5 + W3 * x7 + 1024) >> 11;
    let c3 = (W7 * x1 - W5 * x3 + W3 * x5 - W1 * x7 + 1024) >> 11;

    // 组合并输出 (带 rounding)
    block[col] = (b0 + c0 + 32) >> 6;
    block[col + 8] = (b1 + c1 + 32) >> 6;
    block[col + 16] = (b2 + c2 + 32) >> 6;
    block[col + 24] = (b3 + c3 + 32) >> 6;
    block[col + 32] = (b3 - c3 + 32) >> 6;
    block[col + 40] = (b2 - c2 + 32) >> 6;
    block[col + 48] = (b1 - c1 + 32) >> 6;
    block[col + 56] = (b0 - c0 + 32) >> 6;
}

/// 完整 8x8 IDCT (行+列)
pub(super) fn idct_8x8(block: &mut [i32; 64]) {
    for row in 0..8 {
        idct_row(block, row);
    }
    for col in 0..8 {
        idct_col(block, col);
    }
}
