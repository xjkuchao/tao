//! 整数 IDCT (基于 FFmpeg simple_idct 实现)
//!
//! 8-bit 精度, W 常量按 2^14 缩放, 兼容 IEEE 1180.
//! 参考: FFmpeg libavcodec/simple_idct_template.c (Michael Niedermayer)

/// W 常量: cos(i*π/16) * √2 * 2^14
const W1: i64 = 22725;
const W2: i64 = 21407;
const W3: i64 = 19266;
const W4: i64 = 16383;
const W5: i64 = 12873;
const W6: i64 = 8867;
const W7: i64 = 4520;

const ROW_SHIFT: u32 = 11;
const COL_SHIFT: u32 = 20;
const DC_SHIFT: u32 = 3;

/// 8 点一维 IDCT 行变换
fn idct_row(block: &mut [i32; 64], row: usize) {
    let off = row * 8;
    let x0 = block[off] as i64;
    let x1 = block[off + 1] as i64;
    let x2 = block[off + 2] as i64;
    let x3 = block[off + 3] as i64;
    let x4 = block[off + 4] as i64;
    let x5 = block[off + 5] as i64;
    let x6 = block[off + 6] as i64;
    let x7 = block[off + 7] as i64;

    // 快速检查: 如果 AC 系数全零, 只用 DC
    if x1 == 0 && x2 == 0 && x3 == 0 && x4 == 0 && x5 == 0 && x6 == 0 && x7 == 0 {
        let val = (x0 << DC_SHIFT) as i32;
        for i in 0..8 {
            block[off + i] = val;
        }
        return;
    }

    let round = 1i64 << (ROW_SHIFT - 1);

    // 偶数部分: 使用 W4 处理 x0, x4
    let mut a0 = W4 * x0 + round;
    let mut a1 = a0;
    let mut a2 = a0;
    let mut a3 = a0;

    a0 += W2 * x2;
    a1 += W6 * x2;
    a2 -= W6 * x2;
    a3 -= W2 * x2;

    // x4, x6 贡献
    if x4 != 0 || x6 != 0 {
        a0 += W4 * x4 + W6 * x6;
        a1 += -W4 * x4 - W2 * x6;
        a2 += -W4 * x4 + W2 * x6;
        a3 += W4 * x4 - W6 * x6;
    }

    // 奇数部分
    let mut b0 = W1 * x1 + W3 * x3;
    let mut b1 = W3 * x1 - W7 * x3;
    let mut b2 = W5 * x1 - W1 * x3;
    let mut b3 = W7 * x1 - W5 * x3;

    if x5 != 0 || x7 != 0 {
        b0 += W5 * x5 + W7 * x7;
        b1 += -W1 * x5 - W5 * x7;
        b2 += W7 * x5 + W3 * x7;
        b3 += W3 * x5 - W1 * x7;
    }

    // 组合并右移 ROW_SHIFT
    block[off] = ((a0 + b0) >> ROW_SHIFT) as i32;
    block[off + 1] = ((a1 + b1) >> ROW_SHIFT) as i32;
    block[off + 2] = ((a2 + b2) >> ROW_SHIFT) as i32;
    block[off + 3] = ((a3 + b3) >> ROW_SHIFT) as i32;
    block[off + 4] = ((a3 - b3) >> ROW_SHIFT) as i32;
    block[off + 5] = ((a2 - b2) >> ROW_SHIFT) as i32;
    block[off + 6] = ((a1 - b1) >> ROW_SHIFT) as i32;
    block[off + 7] = ((a0 - b0) >> ROW_SHIFT) as i32;
}

/// 8 点一维 IDCT 列变换
fn idct_col(block: &mut [i32; 64], col: usize) {
    let x0 = block[col] as i64;
    let x1 = block[col + 8] as i64;
    let x2 = block[col + 16] as i64;
    let x3 = block[col + 24] as i64;
    let x4 = block[col + 32] as i64;
    let x5 = block[col + 40] as i64;
    let x6 = block[col + 48] as i64;
    let x7 = block[col + 56] as i64;

    // 快速检查: 如果 AC 系数全零, 只用 DC
    if x1 == 0 && x2 == 0 && x3 == 0 && x4 == 0 && x5 == 0 && x6 == 0 && x7 == 0 {
        // COL_SHIFT = 20, W4 = 16383, round = (1 << 19) / 16383 ≈ 32
        let val = ((x0 * W4 + (1i64 << (COL_SHIFT - 1))) >> COL_SHIFT) as i32;
        for i in 0..8 {
            block[col + i * 8] = val;
        }
        return;
    }

    // 列变换的 rounding: (1 << (COL_SHIFT-1)) / W4 折入 W4 * x0
    let col_round = 1i64 << (COL_SHIFT - 1);

    // 偶数部分
    let mut a0 = W4 * x0 + col_round;
    let mut a1 = a0;
    let mut a2 = a0;
    let mut a3 = a0;

    a0 += W2 * x2;
    a1 += W6 * x2;
    a2 -= W6 * x2;
    a3 -= W2 * x2;

    if x4 != 0 {
        a0 += W4 * x4;
        a1 -= W4 * x4;
        a2 -= W4 * x4;
        a3 += W4 * x4;
    }

    if x6 != 0 {
        a0 += W6 * x6;
        a1 -= W2 * x6;
        a2 += W2 * x6;
        a3 -= W6 * x6;
    }

    // 奇数部分
    let mut b0 = W1 * x1;
    let mut b1 = W3 * x1;
    let mut b2 = W5 * x1;
    let mut b3 = W7 * x1;

    b0 += W3 * x3;
    b1 -= W7 * x3;
    b2 -= W1 * x3;
    b3 -= W5 * x3;

    if x5 != 0 {
        b0 += W5 * x5;
        b1 -= W1 * x5;
        b2 += W7 * x5;
        b3 += W3 * x5;
    }

    if x7 != 0 {
        b0 += W7 * x7;
        b1 -= W5 * x7;
        b2 += W3 * x7;
        b3 -= W1 * x7;
    }

    // 组合并右移 COL_SHIFT
    block[col] = ((a0 + b0) >> COL_SHIFT) as i32;
    block[col + 8] = ((a1 + b1) >> COL_SHIFT) as i32;
    block[col + 16] = ((a2 + b2) >> COL_SHIFT) as i32;
    block[col + 24] = ((a3 + b3) >> COL_SHIFT) as i32;
    block[col + 32] = ((a3 - b3) >> COL_SHIFT) as i32;
    block[col + 40] = ((a2 - b2) >> COL_SHIFT) as i32;
    block[col + 48] = ((a1 - b1) >> COL_SHIFT) as i32;
    block[col + 56] = ((a0 - b0) >> COL_SHIFT) as i32;
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
