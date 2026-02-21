//! H.264 帧内预测模式实现.
//!
//! 提供 Intra_16x16 亮度预测 (4 种模式) 和色度 8x8 预测 (4 种模式).

/// Intra_16x16 亮度预测: 根据模式分发
pub fn predict_16x16(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    mode: u8,
    has_left: bool,
    has_top: bool,
) {
    match mode {
        0 => predict_16x16_v(plane, stride, x0, y0, has_top),
        1 => predict_16x16_h(plane, stride, x0, y0, has_left),
        2 => predict_16x16_dc(plane, stride, x0, y0, has_left, has_top),
        3 => predict_16x16_plane(plane, stride, x0, y0, has_left, has_top),
        _ => predict_16x16_dc(plane, stride, x0, y0, has_left, has_top),
    }
}

/// 模式 0: 垂直预测 - 复制上方行
fn predict_16x16_v(plane: &mut [u8], stride: usize, x0: usize, y0: usize, has_top: bool) {
    if !has_top || y0 == 0 {
        fill_16x16(plane, stride, x0, y0, 128);
        return;
    }
    for dy in 0..16 {
        for dx in 0..16 {
            plane[(y0 + dy) * stride + x0 + dx] = plane[(y0 - 1) * stride + x0 + dx];
        }
    }
}

/// 模式 1: 水平预测 - 复制左侧列
fn predict_16x16_h(plane: &mut [u8], stride: usize, x0: usize, y0: usize, has_left: bool) {
    if !has_left || x0 == 0 {
        fill_16x16(plane, stride, x0, y0, 128);
        return;
    }
    for dy in 0..16 {
        let left = plane[(y0 + dy) * stride + x0 - 1];
        for dx in 0..16 {
            plane[(y0 + dy) * stride + x0 + dx] = left;
        }
    }
}

/// 模式 2: DC 预测 - 邻居平均值
fn predict_16x16_dc(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) {
    let dc = compute_dc_16x16(plane, stride, x0, y0, has_left, has_top);
    fill_16x16(plane, stride, x0, y0, dc);
}

/// 计算 16x16 块的 DC 值
fn compute_dc_16x16(
    plane: &[u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) -> u8 {
    let top_available = has_top && y0 > 0;
    let left_available = has_left && x0 > 0;

    let sum_top = if top_available {
        (0..16)
            .map(|dx| plane[(y0 - 1) * stride + x0 + dx] as u32)
            .sum::<u32>()
    } else {
        0
    };
    let sum_left = if left_available {
        (0..16)
            .map(|dy| plane[(y0 + dy) * stride + x0 - 1] as u32)
            .sum::<u32>()
    } else {
        0
    };

    if top_available && left_available {
        ((sum_top + sum_left + 16) >> 5) as u8
    } else if top_available {
        ((sum_top + 8) >> 4) as u8
    } else if left_available {
        ((sum_left + 8) >> 4) as u8
    } else {
        128
    }
}

/// 模式 3: 平面预测 (Plane prediction)
fn predict_16x16_plane(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) {
    if !has_left || !has_top || x0 == 0 || y0 == 0 {
        predict_16x16_dc(plane, stride, x0, y0, has_left, has_top);
        return;
    }
    let (a, b, c) = compute_plane_params(plane, stride, x0, y0);
    apply_plane_prediction(plane, stride, x0, y0, a, b, c);
}

/// 计算平面预测的 a, b, c 参数
fn compute_plane_params(plane: &[u8], stride: usize, x0: usize, y0: usize) -> (i32, i32, i32) {
    let p = |x: usize, y: usize| -> i32 { plane[y * stride + x] as i32 };

    // 水平梯度
    let mut h_val = 0i32;
    for i in 0..8 {
        let x_plus = i as i32 + 1;
        h_val += x_plus * (p(x0 + 8 + i, y0 - 1) - p(x0 + 6 - i, y0 - 1));
    }
    // 垂直梯度
    let mut v_val = 0i32;
    for i in 0..8 {
        let y_plus = i as i32 + 1;
        v_val += y_plus * (p(x0 - 1, y0 + 8 + i) - p(x0 - 1, y0 + 6 - i));
    }

    let b = (5 * h_val + 32) >> 6;
    let c = (5 * v_val + 32) >> 6;
    let a = 16 * (p(x0 + 15, y0 - 1) + p(x0 - 1, y0 + 15));
    (a, b, c)
}

/// 应用平面预测结果到块
fn apply_plane_prediction(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    a: i32,
    b: i32,
    c: i32,
) {
    for dy in 0..16i32 {
        for dx in 0..16i32 {
            let val = (a + b * (dx - 7) + c * (dy - 7) + 16) >> 5;
            plane[(y0 + dy as usize) * stride + x0 + dx as usize] = val.clamp(0, 255) as u8;
        }
    }
}

/// 色度 DC 预测 (8x8 块)
pub fn predict_chroma_dc(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) {
    let dc = compute_dc_8x8(plane, stride, x0, y0, has_left, has_top);
    fill_block(plane, stride, x0, y0, 8, 8, dc);
}

/// 色度 8x8 预测分发.
///
/// 模式:
/// - 0: DC
/// - 1: Horizontal
/// - 2: Vertical
/// - 3: Plane
pub fn predict_chroma_8x8(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    mode: u8,
    has_left: bool,
    has_top: bool,
) {
    match mode {
        0 => predict_chroma_dc(plane, stride, x0, y0, has_left, has_top),
        1 => predict_chroma_horizontal(plane, stride, x0, y0, has_left),
        2 => predict_chroma_vertical(plane, stride, x0, y0, has_top),
        3 => predict_chroma_plane(plane, stride, x0, y0, has_left, has_top),
        _ => predict_chroma_dc(plane, stride, x0, y0, has_left, has_top),
    }
}

/// 计算 8x8 块的 DC 值
fn compute_dc_8x8(
    plane: &[u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) -> u8 {
    let mut sum = 0u32;
    let mut count = 0u32;

    if has_top && y0 > 0 {
        for dx in 0..8 {
            sum += plane[(y0 - 1) * stride + x0 + dx] as u32;
            count += 1;
        }
    }
    if has_left && x0 > 0 {
        for dy in 0..8 {
            sum += plane[(y0 + dy) * stride + x0 - 1] as u32;
            count += 1;
        }
    }
    if count > 0 { (sum / count) as u8 } else { 128 }
}

/// 色度模式 1: Horizontal.
fn predict_chroma_horizontal(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
) {
    if !has_left || x0 == 0 {
        fill_block(plane, stride, x0, y0, 8, 8, 128);
        return;
    }
    for dy in 0..8 {
        let left = plane[(y0 + dy) * stride + x0 - 1];
        for dx in 0..8 {
            plane[(y0 + dy) * stride + x0 + dx] = left;
        }
    }
}

/// 色度模式 2: Vertical.
fn predict_chroma_vertical(plane: &mut [u8], stride: usize, x0: usize, y0: usize, has_top: bool) {
    if !has_top || y0 == 0 {
        fill_block(plane, stride, x0, y0, 8, 8, 128);
        return;
    }
    for dy in 0..8 {
        for dx in 0..8 {
            plane[(y0 + dy) * stride + x0 + dx] = plane[(y0 - 1) * stride + x0 + dx];
        }
    }
}

/// 色度模式 3: Plane.
fn predict_chroma_plane(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    has_left: bool,
    has_top: bool,
) {
    if !has_left || !has_top || x0 == 0 || y0 == 0 {
        predict_chroma_dc(plane, stride, x0, y0, has_left, has_top);
        return;
    }

    let p = |x: usize, y: usize| -> i32 { plane[y * stride + x] as i32 };

    let mut h_val = 0i32;
    for i in 0..4 {
        let w = i as i32 + 1;
        h_val += w * (p(x0 + 4 + i, y0 - 1) - p(x0 + 2 - i, y0 - 1));
    }
    let mut v_val = 0i32;
    for i in 0..4 {
        let w = i as i32 + 1;
        v_val += w * (p(x0 - 1, y0 + 4 + i) - p(x0 - 1, y0 + 2 - i));
    }

    let a = 16 * (p(x0 - 1, y0 + 7) + p(x0 + 7, y0 - 1));
    let b = (17 * h_val + 16) >> 5;
    let c = (17 * v_val + 16) >> 5;

    for dy in 0..8i32 {
        for dx in 0..8i32 {
            let val = (a + b * (dx - 3) + c * (dy - 3) + 16) >> 5;
            plane[(y0 + dy as usize) * stride + x0 + dx as usize] = val.clamp(0, 255) as u8;
        }
    }
}

/// 4x4 块预测分发函数 (9 种模式)
pub fn predict_4x4(plane: &mut [u8], stride: usize, x0: usize, y0: usize, mode: u8) {
    match mode {
        0 => predict_4x4_vertical(plane, stride, x0, y0),
        1 => predict_4x4_horizontal(plane, stride, x0, y0),
        2 => predict_4x4_dc(plane, stride, x0, y0),
        3 => predict_4x4_diagonal_down_left(plane, stride, x0, y0),
        4 => predict_4x4_diagonal_down_right(plane, stride, x0, y0),
        5 => predict_4x4_vertical_right(plane, stride, x0, y0),
        6 => predict_4x4_horizontal_down(plane, stride, x0, y0),
        7 => predict_4x4_vertical_left(plane, stride, x0, y0),
        8 => predict_4x4_horizontal_up(plane, stride, x0, y0),
        _ => predict_4x4_dc(plane, stride, x0, y0),
    }
}

/// 模式 0: 竖直 (Vertical) - 复制上方行
fn predict_4x4_vertical(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }
    for dy in 0..4 {
        for dx in 0..4 {
            let src_idx = (y0 - 1) * stride + x0 + dx;
            let dst_idx = (y0 + dy) * stride + x0 + dx;
            if src_idx < plane.len() && dst_idx < plane.len() {
                plane[dst_idx] = plane[src_idx];
            }
        }
    }
}

/// 模式 1: 水平 (Horizontal) - 复制左侧列
fn predict_4x4_horizontal(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if x0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }
    for dy in 0..4 {
        let left_val = plane[(y0 + dy) * stride + x0 - 1];
        for dx in 0..4 {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = left_val;
            }
        }
    }
}

/// 模式 2: DC - 邻居平均值
pub fn predict_4x4_dc(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    let top_available = y0 > 0
        && (0..4).all(|dx| {
            let idx = (y0 - 1) * stride + x0 + dx;
            idx < plane.len()
        });
    let left_available = x0 > 0
        && (0..4).all(|dy| {
            let idx = (y0 + dy) * stride + x0 - 1;
            idx < plane.len()
        });

    let sum_top = if top_available {
        (0..4)
            .map(|dx| plane[(y0 - 1) * stride + x0 + dx] as u32)
            .sum::<u32>()
    } else {
        0
    };
    let sum_left = if left_available {
        (0..4)
            .map(|dy| plane[(y0 + dy) * stride + x0 - 1] as u32)
            .sum::<u32>()
    } else {
        0
    };

    let dc = if top_available && left_available {
        ((sum_top + sum_left + 4) >> 3) as u8
    } else if top_available {
        ((sum_top + 2) >> 2) as u8
    } else if left_available {
        ((sum_left + 2) >> 2) as u8
    } else {
        128
    };
    fill_block(plane, stride, x0, y0, 4, 4, dc);
}

/// 模式 3: 对角线向下左 (Diagonal Down-Left)
fn predict_4x4_diagonal_down_left(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    // 该模式仅使用上方与右上参考样本.
    let mut top = [128u8; 8];
    let mut last = 128u8;
    for (i, item) in top.iter_mut().enumerate() {
        let col = x0 + i;
        let idx = (y0 - 1) * stride + col;
        if col < stride && idx < plane.len() {
            last = plane[idx];
            *item = last;
        } else {
            // 越界时沿用最后一个可用样本.
            *item = last;
        }
    }

    for dy in 0..4 {
        for dx in 0..4 {
            let base = dx + dy;
            let s0 = top[base.min(7)] as u32;
            let s1 = top[(base + 1).min(7)] as u32;
            let s2 = top[(base + 2).min(7)] as u32;
            let val = ((s0 + 2 * s1 + s2 + 2) / 4) as u8;
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 4: 对角线向下右 (Diagonal Down-Right)
fn predict_4x4_diagonal_down_right(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if x0 == 0 || y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let top_left_idx = (y0 - 1) * stride + x0 - 1;
    if top_left_idx >= plane.len() {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let x = plane[top_left_idx];
    let mut top = [x; 5];
    let mut left = [x; 5];

    for i in 0..4 {
        let col = x0 + i;
        let top_idx = (y0 - 1) * stride + col;
        if col < stride && top_idx < plane.len() {
            top[i + 1] = plane[top_idx];
        } else {
            top[i + 1] = top[i];
        }
    }
    for i in 0..4 {
        let left_idx = (y0 + i) * stride + x0 - 1;
        if left_idx < plane.len() {
            left[i + 1] = plane[left_idx];
        } else {
            left[i + 1] = left[i];
        }
    }

    let filt = |a: u8, b: u8, c: u8| -> u8 { ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8 };

    for dy in 0..4 {
        for dx in 0..4 {
            let val = if dx > dy {
                let k = dx - dy;
                filt(top[k - 1], top[k], top[k + 1])
            } else if dx == dy {
                filt(left[1], top[0], top[1])
            } else {
                let k = dy - dx;
                filt(left[k + 1], left[k], left[k - 1])
            };
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 5: 竖直-右 (Vertical-Right)
fn predict_4x4_vertical_right(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if x0 == 0 || y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let top_left_idx = (y0 - 1) * stride + x0 - 1;
    if top_left_idx >= plane.len() {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let x = plane[top_left_idx];
    let mut top = [x; 5];
    let mut left = [x; 5];

    for i in 0..4 {
        let col = x0 + i;
        let top_idx = (y0 - 1) * stride + col;
        if col < stride && top_idx < plane.len() {
            top[i + 1] = plane[top_idx];
        } else {
            top[i + 1] = top[i];
        }
    }
    for i in 0..4 {
        let left_idx = (y0 + i) * stride + x0 - 1;
        if left_idx < plane.len() {
            left[i + 1] = plane[left_idx];
        } else {
            left[i + 1] = left[i];
        }
    }

    let avg2 = |a: u8, b: u8| -> u8 { (a as u32 + b as u32).div_ceil(2) as u8 };
    let avg3 = |a: u8, b: u8, c: u8| -> u8 { ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8 };

    let p00 = avg3(x, top[1], top[2]);
    let p01 = avg2(top[1], top[2]);
    let p02 = avg2(top[2], top[3]);
    let p03 = avg2(top[3], top[4]);
    let p10 = avg3(left[1], x, top[1]);
    let p20 = avg3(x, left[1], left[2]);
    let p30 = avg2(left[1], left[2]);

    let preds = [
        [p00, p01, p02, p03],
        [p10, p00, p01, p02],
        [p20, p10, p00, p01],
        [p30, p20, p10, p00],
    ];

    for (dy, row) in preds.iter().enumerate() {
        for (dx, &val) in row.iter().enumerate() {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 6: 水平-下 (Horizontal-Down)
fn predict_4x4_horizontal_down(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if x0 == 0 || y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let top_left_idx = (y0 - 1) * stride + x0 - 1;
    if top_left_idx >= plane.len() {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let x = plane[top_left_idx];
    let mut top = [x; 5];
    let mut left = [x; 5];

    for i in 0..4 {
        let col = x0 + i;
        let top_idx = (y0 - 1) * stride + col;
        if col < stride && top_idx < plane.len() {
            top[i + 1] = plane[top_idx];
        } else {
            top[i + 1] = top[i];
        }
    }
    for i in 0..4 {
        let left_idx = (y0 + i) * stride + x0 - 1;
        if left_idx < plane.len() {
            left[i + 1] = plane[left_idx];
        } else {
            left[i + 1] = left[i];
        }
    }

    let avg2 = |a: u8, b: u8| -> u8 { (a as u32 + b as u32).div_ceil(2) as u8 };
    let avg3 = |a: u8, b: u8, c: u8| -> u8 { ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8 };

    let q00 = avg3(x, left[1], left[2]);
    let q10 = avg2(left[1], left[2]);
    let q20 = avg2(left[2], left[3]);
    let q30 = avg2(left[3], left[4]);
    let q01 = avg3(top[1], x, left[1]);
    let q02 = avg3(x, top[1], top[2]);
    let q03 = avg2(top[1], top[2]);

    let preds = [
        [q00, q10, q20, q30],
        [q01, q00, q10, q20],
        [q02, q01, q00, q10],
        [q03, q02, q01, q00],
    ];

    for (dy, row) in preds.iter().enumerate() {
        for (dx, &val) in row.iter().enumerate() {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 7: 竖直-左 (Vertical-Left)
fn predict_4x4_vertical_left(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if y0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let mut top = [128u8; 8];
    let mut last = 128u8;
    for (i, item) in top.iter_mut().enumerate() {
        let col = x0 + i;
        let idx = (y0 - 1) * stride + col;
        if col < stride && idx < plane.len() {
            last = plane[idx];
            *item = last;
        } else {
            *item = last;
        }
    }

    let avg2 = |a: u8, b: u8| -> u8 { (a as u32 + b as u32).div_ceil(2) as u8 };
    let avg3 = |a: u8, b: u8, c: u8| -> u8 { ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8 };
    let avg2_at = |i: usize| -> u8 { avg2(top[i], top[i + 1]) };
    let avg3_at = |i: usize| -> u8 { avg3(top[i], top[i + 1], top[i + 2]) };

    let preds = [
        [avg2_at(0), avg2_at(1), avg2_at(2), avg2_at(3)],
        [avg3_at(0), avg3_at(1), avg3_at(2), avg3_at(3)],
        [avg2_at(1), avg2_at(2), avg2_at(3), avg2_at(4)],
        [avg3_at(1), avg3_at(2), avg3_at(3), avg3_at(4)],
    ];

    for (dy, row) in preds.iter().enumerate() {
        for (dx, &val) in row.iter().enumerate() {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 8: 水平-上 (Horizontal-Up)
fn predict_4x4_horizontal_up(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    if x0 == 0 {
        fill_block(plane, stride, x0, y0, 4, 4, 128);
        return;
    }

    let mut left = [128u8; 5];
    let mut last = 128u8;
    for (i, item) in left.iter_mut().enumerate().take(4) {
        let idx = (y0 + i) * stride + x0 - 1;
        if idx < plane.len() {
            last = plane[idx];
            *item = last;
        } else {
            *item = last;
        }
    }
    let ext_idx = (y0 + 4) * stride + x0 - 1;
    left[4] = if ext_idx < plane.len() {
        plane[ext_idx]
    } else {
        left[3]
    };

    let avg2 = |a: u8, b: u8| -> u8 { (a as u32 + b as u32).div_ceil(2) as u8 };
    let avg3 = |a: u8, b: u8, c: u8| -> u8 { ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8 };

    for dy in 0..4 {
        for dx in 0..4 {
            let z = dx + 2 * dy;
            let val = match z {
                0 | 2 | 4 | 6 => {
                    let i = z / 2;
                    avg2(left[i], left[i + 1])
                }
                1 | 3 | 5 => {
                    let i = (z - 1) / 2;
                    avg3(left[i], left[i + 1], left[i + 2])
                }
                7 => ((left[3] as u32 + 3 * left[4] as u32 + 2) >> 2) as u8,
                _ => left[4],
            };
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 用单一值填充 16x16 块
fn fill_16x16(plane: &mut [u8], stride: usize, x0: usize, y0: usize, val: u8) {
    fill_block(plane, stride, x0, y0, 16, 16, val);
}

/// 用单一值填充矩形块
pub fn fill_block(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    val: u8,
) {
    for dy in 0..h {
        let start = (y0 + dy) * stride + x0;
        let end = start + w;
        if end <= plane.len() {
            plane[start..end].fill(val);
        }
    }
}

// ============================================================
// Intra 8x8 亮度预测 (H.264 规范 8.3.2, 9 种模式)
// ============================================================
//
// 与 Intra 4x4 不同, Intra 8x8 的边界参考样本须先经过低通滤波处理.
// 滤波后的参考样本命名为 p'[-1,-1..7] (上方/右上) 与 p'[-1,0..7] (左侧),
// 对应 FFmpeg `PREDICT_8x8_LOAD_*` 宏.
//
// 参数约定:
//   plane       -- 图像平面像素缓冲
//   stride      -- 行步长
//   x0, y0      -- 8x8 块左上角像素坐标
//   has_left    -- 左邻居是否可用
//   has_top     -- 上邻居是否可用
//   has_topleft -- 左上角像素是否可用
//   has_topright-- 右上邻居是否可用(列宽超出宏块右边界则不可用)

/// 读取原始像素, 越界时夹紧到边界
#[inline(always)]
fn px(plane: &[u8], stride: usize, x: i64, y: i64) -> u8 {
    let xi = x.max(0) as usize;
    let yi = y.max(0) as usize;
    let idx = yi * stride + xi;
    if idx < plane.len() { plane[idx] } else { 128 }
}

/// Intra 8x8 低通滤波参考样本结构体
///
/// 按规范 8.3.2.2.2 对左侧(l0..l7)、上方(t0..t7)、右上(t8..t15)、左上角(lt)做低通滤波.
struct I8x8Refs {
    lt: i32,
    t: [i32; 16],
    l: [i32; 8],
}

/// Intra 8x8 预测的邻居可用性标志
pub struct I8x8Avail {
    pub has_left: bool,
    pub has_top: bool,
    pub has_topleft: bool,
    pub has_topright: bool,
}

impl I8x8Refs {
    /// 收集并滤波 8x8 块的边界参考样本.
    ///
    /// 滤波公式 (对应规范与 FFmpeg `PREDICT_8x8_LOAD_*` 宏):
    ///   - 内部点: (p[i-1] + 2*p[i] + p[i+1] + 2) >> 2
    ///   - 端点用相邻点或 has_topleft/topright 控制
    fn load(plane: &[u8], stride: usize, x0: usize, y0: usize, avail: &I8x8Avail) -> Self {
        let has_left = avail.has_left;
        let has_top = avail.has_top;
        let has_topleft = avail.has_topleft;
        let has_topright = avail.has_topright;
        let x = x0 as i64;
        let y = y0 as i64;

        // 读取原始参考样本 (未滤波)
        let raw_tl = if has_topleft && x > 0 && y > 0 {
            px(plane, stride, x - 1, y - 1)
        } else if has_top && y > 0 {
            px(plane, stride, x, y - 1)
        } else if has_left && x > 0 {
            px(plane, stride, x - 1, y)
        } else {
            128
        } as i32;

        // 上方参考 (含右上)
        let mut raw_t = [128i32; 16];
        if has_top && y > 0 {
            for (i, item) in raw_t[..8].iter_mut().enumerate() {
                *item = px(plane, stride, x + i as i64, y - 1) as i32;
            }
        }
        if has_topright && y > 0 {
            for (i, item) in raw_t[8..].iter_mut().enumerate() {
                *item = px(plane, stride, x + (i + 8) as i64, y - 1) as i32;
            }
        } else {
            // 右上不可用时复制 t7
            let t7 = raw_t[7];
            for item in raw_t[8..].iter_mut() {
                *item = t7;
            }
        }

        // 左侧参考
        let mut raw_l = [128i32; 8];
        if has_left && x > 0 {
            for (i, item) in raw_l.iter_mut().enumerate() {
                *item = px(plane, stride, x - 1, y + i as i64) as i32;
            }
        }

        // 滤波
        // lt = (l[0] + 2*tl + t[0] + 2) >> 2  (左上角)
        let lt = if has_topleft && has_left && has_top {
            (raw_l[0] + 2 * raw_tl + raw_t[0] + 2) >> 2
        } else {
            raw_tl
        };

        // l[0]: has_topleft 时用 tl, 否则复制 l[0]
        let mut l = [0i32; 8];
        if has_left {
            l[0] = if has_topleft {
                (raw_tl + 2 * raw_l[0] + raw_l[1] + 2) >> 2
            } else {
                (raw_l[0] + 2 * raw_l[0] + raw_l[1] + 2) >> 2
            };
            for i in 1..7 {
                l[i] = (raw_l[i - 1] + 2 * raw_l[i] + raw_l[i + 1] + 2) >> 2;
            }
            l[7] = (raw_l[6] + 3 * raw_l[7] + 2) >> 2;
        } else {
            l.fill(128);
        }

        // t[0]: has_topleft 时用 tl, 否则复制 t[0]
        let mut t = [0i32; 16];
        if has_top {
            t[0] = if has_topleft {
                (raw_tl + 2 * raw_t[0] + raw_t[1] + 2) >> 2
            } else {
                (raw_t[0] + 2 * raw_t[0] + raw_t[1] + 2) >> 2
            };
            for i in 1..7 {
                t[i] = (raw_t[i - 1] + 2 * raw_t[i] + raw_t[i + 1] + 2) >> 2;
            }
            // t[7]: 有右上时用 t8, 否则复制 t7
            t[7] = if has_topright {
                (raw_t[6] + 2 * raw_t[7] + raw_t[8] + 2) >> 2
            } else {
                (raw_t[6] + 3 * raw_t[7] + 2) >> 2
            };
            // t[8..15] (右上参考)
            if has_topright {
                t[8] = (raw_t[7] + 2 * raw_t[8] + raw_t[9] + 2) >> 2;
                for i in 9..15 {
                    t[i] = (raw_t[i - 1] + 2 * raw_t[i] + raw_t[i + 1] + 2) >> 2;
                }
                t[15] = (raw_t[14] + 3 * raw_t[15] + 2) >> 2;
            } else {
                let t7 = t[7];
                for item in t[8..].iter_mut() {
                    *item = t7;
                }
            }
        } else {
            t.fill(128);
        }

        I8x8Refs { lt, t, l }
    }
}

/// Intra 8x8 预测入口 -- 根据模式分发
///
/// 模式编号与 Intra 4x4 相同: 0=V, 1=H, 2=DC, 3=DDL, 4=DDR, 5=VR, 6=HD, 7=VL, 8=HU.
pub fn predict_8x8(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    mode: u8,
    avail: &I8x8Avail,
) {
    let r = I8x8Refs::load(plane, stride, x0, y0, avail);
    match mode {
        0 => predict_8x8_vertical(plane, stride, x0, y0, &r, avail.has_top),
        1 => predict_8x8_horizontal(plane, stride, x0, y0, &r, avail.has_left),
        2 => predict_8x8_dc(plane, stride, x0, y0, &r, avail.has_left, avail.has_top),
        3 => predict_8x8_down_left(plane, stride, x0, y0, &r),
        4 => predict_8x8_down_right(plane, stride, x0, y0, &r),
        5 => predict_8x8_vertical_right(plane, stride, x0, y0, &r),
        6 => predict_8x8_horizontal_down(plane, stride, x0, y0, &r),
        7 => predict_8x8_vertical_left(plane, stride, x0, y0, &r),
        8 => predict_8x8_horizontal_up(plane, stride, x0, y0, &r),
        _ => predict_8x8_dc(plane, stride, x0, y0, &r, avail.has_left, avail.has_top),
    }
}

#[inline(always)]
fn set8(plane: &mut [u8], stride: usize, x0: usize, y0: usize, dx: usize, dy: usize, v: i32) {
    let idx = (y0 + dy) * stride + x0 + dx;
    if idx < plane.len() {
        plane[idx] = v.clamp(0, 255) as u8;
    }
}

/// 模式 0: 垂直 (Vertical)
fn predict_8x8_vertical(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    r: &I8x8Refs,
    has_top: bool,
) {
    if !has_top {
        fill_block(plane, stride, x0, y0, 8, 8, 128);
        return;
    }
    for dy in 0..8 {
        for dx in 0..8 {
            set8(plane, stride, x0, y0, dx, dy, r.t[dx]);
        }
    }
}

/// 模式 1: 水平 (Horizontal)
fn predict_8x8_horizontal(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    r: &I8x8Refs,
    has_left: bool,
) {
    if !has_left {
        fill_block(plane, stride, x0, y0, 8, 8, 128);
        return;
    }
    for dy in 0..8 {
        let v = r.l[dy];
        for dx in 0..8 {
            set8(plane, stride, x0, y0, dx, dy, v);
        }
    }
}

/// 模式 2: DC
fn predict_8x8_dc(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    r: &I8x8Refs,
    has_left: bool,
    has_top: bool,
) {
    let dc = if has_left && has_top {
        let sum: i32 = r.l.iter().sum::<i32>() + r.t[..8].iter().sum::<i32>();
        (sum + 8) >> 4
    } else if has_top {
        let sum: i32 = r.t[..8].iter().sum();
        (sum + 4) >> 3
    } else if has_left {
        let sum: i32 = r.l.iter().sum();
        (sum + 4) >> 3
    } else {
        128
    };
    fill_block(plane, stride, x0, y0, 8, 8, dc.clamp(0, 255) as u8);
}

/// 模式 3: 对角线向下左 (Diagonal Down-Left)
fn predict_8x8_down_left(plane: &mut [u8], stride: usize, x0: usize, y0: usize, r: &I8x8Refs) {
    // SRC(x,y) = t[x+y], SRC(x,y) using filtered top/topright
    let t = &r.t;
    // 对应 FFmpeg pred8x8l_down_left
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 0, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(1, 0, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(0, 1, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(2, 0, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(1, 1, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(0, 2, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(3, 0, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(2, 1, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(1, 2, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(0, 3, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(4, 0, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(3, 1, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(2, 2, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(1, 3, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(0, 4, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(5, 0, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(4, 1, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(3, 2, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(2, 3, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(1, 4, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(0, 5, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(6, 0, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(5, 1, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(4, 2, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(3, 3, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(2, 4, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(1, 5, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(0, 6, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(7, 0, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(6, 1, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(5, 2, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(4, 3, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(3, 4, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(2, 5, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(1, 6, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(0, 7, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(7, 1, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(6, 2, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(5, 3, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(4, 4, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(3, 5, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(2, 6, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(1, 7, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(7, 2, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(6, 3, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(5, 4, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(4, 5, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(3, 6, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(2, 7, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(7, 3, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
    s!(6, 4, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
    s!(5, 5, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
    s!(4, 6, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
    s!(3, 7, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
    s!(7, 4, (t[11] + 2 * t[12] + t[13] + 2) >> 2);
    s!(6, 5, (t[11] + 2 * t[12] + t[13] + 2) >> 2);
    s!(5, 6, (t[11] + 2 * t[12] + t[13] + 2) >> 2);
    s!(4, 7, (t[11] + 2 * t[12] + t[13] + 2) >> 2);
    s!(7, 5, (t[12] + 2 * t[13] + t[14] + 2) >> 2);
    s!(6, 6, (t[12] + 2 * t[13] + t[14] + 2) >> 2);
    s!(5, 7, (t[12] + 2 * t[13] + t[14] + 2) >> 2);
    s!(7, 6, (t[13] + 2 * t[14] + t[15] + 2) >> 2);
    s!(6, 7, (t[13] + 2 * t[14] + t[15] + 2) >> 2);
    s!(7, 7, (t[14] + 3 * t[15] + 2) >> 2);
}

/// 模式 4: 对角线向下右 (Diagonal Down-Right)
fn predict_8x8_down_right(plane: &mut [u8], stride: usize, x0: usize, y0: usize, r: &I8x8Refs) {
    let t = &r.t;
    let l = &r.l;
    let lt = r.lt;
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 7, (l[6] + 2 * l[5] + l[4] + 2) >> 2); // SRC(0,7)
    // 按 FFmpeg pred8x8l_down_right 逐行赋值
    s!(0, 6, (l[5] + 2 * l[4] + l[3] + 2) >> 2);
    s!(1, 7, (l[5] + 2 * l[4] + l[3] + 2) >> 2);
    s!(0, 5, (l[4] + 2 * l[3] + l[2] + 2) >> 2);
    s!(1, 6, (l[4] + 2 * l[3] + l[2] + 2) >> 2);
    s!(2, 7, (l[4] + 2 * l[3] + l[2] + 2) >> 2);
    s!(0, 4, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(1, 5, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(2, 6, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(3, 7, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(0, 3, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(1, 4, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(2, 5, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(3, 6, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(4, 7, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(0, 2, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(1, 3, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(2, 4, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(3, 5, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(4, 6, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(5, 7, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(0, 1, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(1, 2, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(2, 3, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(3, 4, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(4, 5, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(5, 6, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(6, 7, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(0, 0, (l[0] + 2 * lt + t[0] + 2) >> 2); // SRC(0,0) = SRC(1,1) = ... (主对角线同一公式)
    // 主对角线: (l0 + 2*lt + t0 + 2) >> 2
    s!(0, 0, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(1, 1, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(2, 2, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(3, 3, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(4, 4, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(5, 5, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(6, 6, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(7, 7, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(1, 0, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(2, 1, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(3, 2, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(4, 3, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(5, 4, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(6, 5, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(7, 6, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(2, 0, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(3, 1, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(4, 2, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(5, 3, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(6, 4, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(7, 5, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(3, 0, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(4, 1, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(5, 2, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(6, 3, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(7, 4, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(4, 0, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(5, 1, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(6, 2, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(7, 3, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(5, 0, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(6, 1, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(7, 2, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(6, 0, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(7, 1, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(7, 0, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
}

/// 模式 5: 垂直右 (Vertical-Right)
fn predict_8x8_vertical_right(plane: &mut [u8], stride: usize, x0: usize, y0: usize, r: &I8x8Refs) {
    let t = &r.t;
    let l = &r.l;
    let lt = r.lt;
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 6, (l[4] + 2 * l[3] + l[2] + 2) >> 2);
    s!(0, 7, (l[5] + 2 * l[4] + l[3] + 2) >> 2);
    s!(0, 4, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(1, 6, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(0, 5, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(1, 7, (l[3] + 2 * l[2] + l[1] + 2) >> 2);
    s!(0, 2, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(1, 4, (l[0] + 2 * lt + t[0] + 2) >> 2); // 注意: 此处使用带滤波的公式
    // 实际上 Vertical-Right 中左侧用的是 l[1]+2*l[0]+lt 等
    s!(0, 2, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(1, 4, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(2, 6, (l[1] + 2 * l[0] + lt + 2) >> 2);
    s!(0, 3, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(1, 5, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(2, 7, (l[2] + 2 * l[1] + l[0] + 2) >> 2);
    s!(0, 1, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(1, 3, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(2, 5, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(3, 7, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(0, 0, (lt + t[0] + 1) >> 1);
    s!(1, 2, (lt + t[0] + 1) >> 1);
    s!(2, 4, (lt + t[0] + 1) >> 1);
    s!(3, 6, (lt + t[0] + 1) >> 1);
    s!(1, 1, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(2, 3, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(3, 5, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(4, 7, (lt + 2 * t[0] + t[1] + 2) >> 2);
    s!(1, 0, (t[0] + t[1] + 1) >> 1);
    s!(2, 2, (t[0] + t[1] + 1) >> 1);
    s!(3, 4, (t[0] + t[1] + 1) >> 1);
    s!(4, 6, (t[0] + t[1] + 1) >> 1);
    s!(2, 1, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(3, 3, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(4, 5, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(5, 7, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(2, 0, (t[1] + t[2] + 1) >> 1);
    s!(3, 2, (t[1] + t[2] + 1) >> 1);
    s!(4, 4, (t[1] + t[2] + 1) >> 1);
    s!(5, 6, (t[1] + t[2] + 1) >> 1);
    s!(3, 1, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(4, 3, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(5, 5, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(6, 7, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(3, 0, (t[2] + t[3] + 1) >> 1);
    s!(4, 2, (t[2] + t[3] + 1) >> 1);
    s!(5, 4, (t[2] + t[3] + 1) >> 1);
    s!(6, 6, (t[2] + t[3] + 1) >> 1);
    s!(4, 1, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(5, 3, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(6, 5, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(7, 7, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(4, 0, (t[3] + t[4] + 1) >> 1);
    s!(5, 2, (t[3] + t[4] + 1) >> 1);
    s!(6, 4, (t[3] + t[4] + 1) >> 1);
    s!(7, 6, (t[3] + t[4] + 1) >> 1);
    s!(5, 1, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(6, 3, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(7, 5, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(5, 0, (t[4] + t[5] + 1) >> 1);
    s!(6, 2, (t[4] + t[5] + 1) >> 1);
    s!(7, 4, (t[4] + t[5] + 1) >> 1);
    s!(6, 1, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(7, 3, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(6, 0, (t[5] + t[6] + 1) >> 1);
    s!(7, 2, (t[5] + t[6] + 1) >> 1);
    s!(7, 1, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(7, 0, (t[6] + t[7] + 1) >> 1);
}

/// 模式 6: 水平向下 (Horizontal-Down)
fn predict_8x8_horizontal_down(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    r: &I8x8Refs,
) {
    let t = &r.t;
    let l = &r.l;
    let lt = r.lt;
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 7, (l[6] + l[7] + 1) >> 1);
    s!(1, 7, (l[5] + 2 * l[6] + l[7] + 2) >> 2);
    s!(0, 6, (l[5] + l[6] + 1) >> 1);
    s!(2, 7, (l[5] + l[6] + 1) >> 1);
    s!(1, 6, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(3, 7, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(0, 5, (l[4] + l[5] + 1) >> 1);
    s!(2, 6, (l[4] + l[5] + 1) >> 1);
    s!(4, 7, (l[4] + l[5] + 1) >> 1);
    s!(1, 5, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(3, 6, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(5, 7, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(0, 4, (l[3] + l[4] + 1) >> 1);
    s!(2, 5, (l[3] + l[4] + 1) >> 1);
    s!(4, 6, (l[3] + l[4] + 1) >> 1);
    s!(6, 7, (l[3] + l[4] + 1) >> 1);
    s!(1, 4, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(3, 5, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(5, 6, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(7, 7, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(0, 3, (l[2] + l[3] + 1) >> 1);
    s!(2, 4, (l[2] + l[3] + 1) >> 1);
    s!(4, 5, (l[2] + l[3] + 1) >> 1);
    s!(6, 6, (l[2] + l[3] + 1) >> 1);
    s!(1, 3, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(3, 4, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(5, 5, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(7, 6, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(0, 2, (l[1] + l[2] + 1) >> 1);
    s!(2, 3, (l[1] + l[2] + 1) >> 1);
    s!(4, 4, (l[1] + l[2] + 1) >> 1);
    s!(6, 5, (l[1] + l[2] + 1) >> 1);
    s!(1, 2, (l[0] + 2 * l[1] + l[2] + 2) >> 2);
    s!(3, 3, (l[0] + 2 * l[1] + l[2] + 2) >> 2);
    s!(5, 4, (l[0] + 2 * l[1] + l[2] + 2) >> 2);
    s!(7, 5, (l[0] + 2 * l[1] + l[2] + 2) >> 2);
    s!(0, 1, (l[0] + l[1] + 1) >> 1);
    s!(2, 2, (l[0] + l[1] + 1) >> 1);
    s!(4, 3, (l[0] + l[1] + 1) >> 1);
    s!(6, 4, (l[0] + l[1] + 1) >> 1);
    s!(1, 1, (lt + 2 * l[0] + l[1] + 2) >> 2);
    s!(3, 2, (lt + 2 * l[0] + l[1] + 2) >> 2);
    s!(5, 3, (lt + 2 * l[0] + l[1] + 2) >> 2);
    s!(7, 4, (lt + 2 * l[0] + l[1] + 2) >> 2);
    s!(0, 0, (lt + l[0] + 1) >> 1);
    s!(2, 1, (lt + l[0] + 1) >> 1);
    s!(4, 2, (lt + l[0] + 1) >> 1);
    s!(6, 3, (lt + l[0] + 1) >> 1);
    s!(1, 0, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(3, 1, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(5, 2, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(7, 3, (l[0] + 2 * lt + t[0] + 2) >> 2);
    s!(2, 0, (t[1] + 2 * t[0] + lt + 2) >> 2);
    s!(4, 1, (t[1] + 2 * t[0] + lt + 2) >> 2);
    s!(6, 2, (t[1] + 2 * t[0] + lt + 2) >> 2);
    s!(3, 0, (t[2] + 2 * t[1] + t[0] + 2) >> 2);
    s!(5, 1, (t[2] + 2 * t[1] + t[0] + 2) >> 2);
    s!(7, 2, (t[2] + 2 * t[1] + t[0] + 2) >> 2);
    s!(4, 0, (t[3] + 2 * t[2] + t[1] + 2) >> 2);
    s!(6, 1, (t[3] + 2 * t[2] + t[1] + 2) >> 2);
    s!(5, 0, (t[4] + 2 * t[3] + t[2] + 2) >> 2);
    s!(7, 1, (t[4] + 2 * t[3] + t[2] + 2) >> 2);
    s!(6, 0, (t[5] + 2 * t[4] + t[3] + 2) >> 2);
    s!(7, 0, (t[6] + 2 * t[5] + t[4] + 2) >> 2);
}

/// 模式 7: 垂直左 (Vertical-Left)
fn predict_8x8_vertical_left(plane: &mut [u8], stride: usize, x0: usize, y0: usize, r: &I8x8Refs) {
    let t = &r.t;
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 0, (t[0] + t[1] + 1) >> 1);
    s!(0, 1, (t[0] + 2 * t[1] + t[2] + 2) >> 2);
    s!(1, 0, (t[1] + t[2] + 1) >> 1);
    s!(0, 2, (t[1] + t[2] + 1) >> 1);
    s!(1, 1, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(0, 3, (t[1] + 2 * t[2] + t[3] + 2) >> 2);
    s!(2, 0, (t[2] + t[3] + 1) >> 1);
    s!(1, 2, (t[2] + t[3] + 1) >> 1);
    s!(0, 4, (t[2] + t[3] + 1) >> 1);
    s!(2, 1, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(1, 3, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(0, 5, (t[2] + 2 * t[3] + t[4] + 2) >> 2);
    s!(3, 0, (t[3] + t[4] + 1) >> 1);
    s!(2, 2, (t[3] + t[4] + 1) >> 1);
    s!(1, 4, (t[3] + t[4] + 1) >> 1);
    s!(0, 6, (t[3] + t[4] + 1) >> 1);
    s!(3, 1, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(2, 3, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(1, 5, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(0, 7, (t[3] + 2 * t[4] + t[5] + 2) >> 2);
    s!(4, 0, (t[4] + t[5] + 1) >> 1);
    s!(3, 2, (t[4] + t[5] + 1) >> 1);
    s!(2, 4, (t[4] + t[5] + 1) >> 1);
    s!(1, 6, (t[4] + t[5] + 1) >> 1);
    s!(4, 1, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(3, 3, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(2, 5, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(1, 7, (t[4] + 2 * t[5] + t[6] + 2) >> 2);
    s!(5, 0, (t[5] + t[6] + 1) >> 1);
    s!(4, 2, (t[5] + t[6] + 1) >> 1);
    s!(3, 4, (t[5] + t[6] + 1) >> 1);
    s!(2, 6, (t[5] + t[6] + 1) >> 1);
    s!(5, 1, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(4, 3, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(3, 5, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(2, 7, (t[5] + 2 * t[6] + t[7] + 2) >> 2);
    s!(6, 0, (t[6] + t[7] + 1) >> 1);
    s!(5, 2, (t[6] + t[7] + 1) >> 1);
    s!(4, 4, (t[6] + t[7] + 1) >> 1);
    s!(3, 6, (t[6] + t[7] + 1) >> 1);
    s!(6, 1, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(5, 3, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(4, 5, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(3, 7, (t[6] + 2 * t[7] + t[8] + 2) >> 2);
    s!(7, 0, (t[7] + t[8] + 1) >> 1);
    s!(6, 2, (t[7] + t[8] + 1) >> 1);
    s!(5, 4, (t[7] + t[8] + 1) >> 1);
    s!(4, 6, (t[7] + t[8] + 1) >> 1);
    s!(7, 1, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(6, 3, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(5, 5, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(4, 7, (t[7] + 2 * t[8] + t[9] + 2) >> 2);
    s!(7, 2, (t[8] + t[9] + 1) >> 1);
    s!(6, 4, (t[8] + t[9] + 1) >> 1);
    s!(5, 6, (t[8] + t[9] + 1) >> 1);
    s!(7, 3, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(6, 5, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(5, 7, (t[8] + 2 * t[9] + t[10] + 2) >> 2);
    s!(7, 4, (t[9] + t[10] + 1) >> 1);
    s!(6, 6, (t[9] + t[10] + 1) >> 1);
    s!(7, 5, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(6, 7, (t[9] + 2 * t[10] + t[11] + 2) >> 2);
    s!(7, 6, (t[10] + t[11] + 1) >> 1);
    s!(7, 7, (t[10] + 2 * t[11] + t[12] + 2) >> 2);
}

/// 模式 8: 水平向上 (Horizontal-Up)
fn predict_8x8_horizontal_up(plane: &mut [u8], stride: usize, x0: usize, y0: usize, r: &I8x8Refs) {
    let l = &r.l;
    macro_rules! s {
        ($x:expr, $y:expr, $v:expr) => {
            set8(plane, stride, x0, y0, $x, $y, $v)
        };
    }
    s!(0, 0, (l[0] + l[1] + 1) >> 1);
    s!(1, 0, (l[0] + 2 * l[1] + l[2] + 2) >> 2);
    s!(2, 0, (l[1] + l[2] + 1) >> 1);
    s!(0, 1, (l[1] + l[2] + 1) >> 1);
    s!(3, 0, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(1, 1, (l[1] + 2 * l[2] + l[3] + 2) >> 2);
    s!(4, 0, (l[2] + l[3] + 1) >> 1);
    s!(2, 1, (l[2] + l[3] + 1) >> 1);
    s!(0, 2, (l[2] + l[3] + 1) >> 1);
    s!(5, 0, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(3, 1, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(1, 2, (l[2] + 2 * l[3] + l[4] + 2) >> 2);
    s!(6, 0, (l[3] + l[4] + 1) >> 1);
    s!(4, 1, (l[3] + l[4] + 1) >> 1);
    s!(2, 2, (l[3] + l[4] + 1) >> 1);
    s!(0, 3, (l[3] + l[4] + 1) >> 1);
    s!(7, 0, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(5, 1, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(3, 2, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(1, 3, (l[3] + 2 * l[4] + l[5] + 2) >> 2);
    s!(6, 1, (l[4] + l[5] + 1) >> 1);
    s!(4, 2, (l[4] + l[5] + 1) >> 1);
    s!(2, 3, (l[4] + l[5] + 1) >> 1);
    s!(0, 4, (l[4] + l[5] + 1) >> 1);
    s!(7, 1, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(5, 2, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(3, 3, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(1, 4, (l[4] + 2 * l[5] + l[6] + 2) >> 2);
    s!(6, 2, (l[5] + l[6] + 1) >> 1);
    s!(4, 3, (l[5] + l[6] + 1) >> 1);
    s!(2, 4, (l[5] + l[6] + 1) >> 1);
    s!(0, 5, (l[5] + l[6] + 1) >> 1);
    s!(7, 2, (l[5] + 2 * l[6] + l[7] + 2) >> 2);
    s!(5, 3, (l[5] + 2 * l[6] + l[7] + 2) >> 2);
    s!(3, 4, (l[5] + 2 * l[6] + l[7] + 2) >> 2);
    s!(1, 5, (l[5] + 2 * l[6] + l[7] + 2) >> 2);
    s!(6, 3, (l[6] + l[7] + 1) >> 1);
    s!(4, 4, (l[6] + l[7] + 1) >> 1);
    s!(2, 5, (l[6] + l[7] + 1) >> 1);
    s!(0, 6, (l[6] + l[7] + 1) >> 1);
    s!(7, 3, (l[6] + 3 * l[7] + 2) >> 2);
    s!(5, 4, (l[6] + 3 * l[7] + 2) >> 2);
    s!(3, 5, (l[6] + 3 * l[7] + 2) >> 2);
    s!(1, 6, (l[6] + 3 * l[7] + 2) >> 2);
    s!(6, 4, l[7]);
    s!(4, 5, l[7]);
    s!(2, 6, l[7]);
    s!(0, 7, l[7]);
    s!(7, 4, l[7]);
    s!(5, 5, l[7]);
    s!(3, 6, l[7]);
    s!(1, 7, l[7]);
    s!(6, 5, l[7]);
    s!(4, 6, l[7]);
    s!(2, 7, l[7]);
    s!(7, 5, l[7]);
    s!(5, 6, l[7]);
    s!(3, 7, l[7]);
    s!(6, 6, l[7]);
    s!(4, 7, l[7]);
    s!(7, 6, l[7]);
    s!(5, 7, l[7]);
    s!(6, 7, l[7]);
    s!(7, 7, l[7]);
}

#[cfg(test)]
mod tests {
    use super::{predict_4x4, predict_16x16, predict_chroma_8x8};

    fn read_block_4x4(plane: &[u8], stride: usize, x0: usize, y0: usize) -> [[u8; 4]; 4] {
        let mut block = [[0u8; 4]; 4];
        for dy in 0..4 {
            for dx in 0..4 {
                block[dy][dx] = plane[(y0 + dy) * stride + x0 + dx];
            }
        }
        block
    }

    fn read_block_8x8(plane: &[u8], stride: usize, x0: usize, y0: usize) -> [[u8; 8]; 8] {
        let mut block = [[0u8; 8]; 8];
        for dy in 0..8 {
            for dx in 0..8 {
                block[dy][dx] = plane[(y0 + dy) * stride + x0 + dx];
            }
        }
        block
    }

    fn read_block_16x16(plane: &[u8], stride: usize, x0: usize, y0: usize) -> [[u8; 16]; 16] {
        let mut block = [[0u8; 16]; 16];
        for dy in 0..16 {
            for dx in 0..16 {
                block[dy][dx] = plane[(y0 + dy) * stride + x0 + dx];
            }
        }
        block
    }

    #[test]
    fn test_intra16x16_mode0_without_top_falls_back_to_128() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![99u8; stride * stride];

        predict_16x16(&mut plane, stride, x0, y0, 0, true, false);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[128u8; 16]; 16];

        assert_eq!(got, expect, "16x16 模式0在上参考不可用时应回退为128填充");
    }

    #[test]
    fn test_intra16x16_mode1_without_left_falls_back_to_128() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![77u8; stride * stride];

        predict_16x16(&mut plane, stride, x0, y0, 1, false, true);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[128u8; 16]; 16];

        assert_eq!(got, expect, "16x16 模式1在左参考不可用时应回退为128填充");
    }

    #[test]
    fn test_intra16x16_mode2_dc_both_neighbors_uses_rounding_rule() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * stride];

        for i in 0..16 {
            plane[(y0 - 1) * stride + x0 + i] = (i + 1) as u8;
            plane[(y0 + i) * stride + x0 - 1] = (i + 17) as u8;
        }

        predict_16x16(&mut plane, stride, x0, y0, 2, true, true);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[17u8; 16]; 16];

        assert_eq!(
            got, expect,
            "16x16 模式2在上左都可用时应使用(sum_top+sum_left+16)>>5"
        );
    }

    #[test]
    fn test_intra16x16_mode2_dc_top_only_variant() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * stride];

        for i in 0..16 {
            plane[(y0 - 1) * stride + x0 + i] = (i + 1) as u8;
        }

        predict_16x16(&mut plane, stride, x0, y0, 2, false, true);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[9u8; 16]; 16];

        assert_eq!(got, expect, "16x16 模式2在仅上可用时应使用(sum_top+8)>>4");
    }

    #[test]
    fn test_intra16x16_mode2_dc_left_only_variant() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * stride];

        for i in 0..16 {
            plane[(y0 + i) * stride + x0 - 1] = (i + 17) as u8;
        }

        predict_16x16(&mut plane, stride, x0, y0, 2, true, false);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[25u8; 16]; 16];

        assert_eq!(got, expect, "16x16 模式2在仅左可用时应使用(sum_left+8)>>4");
    }

    #[test]
    fn test_intra16x16_mode2_dc_none_variant_128() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![55u8; stride * stride];

        predict_16x16(&mut plane, stride, x0, y0, 2, false, false);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[128u8; 16]; 16];

        assert_eq!(got, expect, "16x16 模式2在上左都不可用时应使用DC-128变体");
    }

    #[test]
    fn test_intra16x16_mode3_plane_without_top_or_left_falls_back_to_dc_variant() {
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * stride];

        for i in 0..16 {
            plane[(y0 + i) * stride + x0 - 1] = (i + 17) as u8;
        }

        predict_16x16(&mut plane, stride, x0, y0, 3, true, false);
        let got = read_block_16x16(&plane, stride, x0, y0);
        let expect = [[25u8; 16]; 16];

        assert_eq!(
            got, expect,
            "16x16 模式3在缺少上参考时应回退到DC并使用可用邻居变体"
        );
    }

    #[test]
    fn test_intra4x4_mode3_diagonal_down_left_uses_top_and_top_right() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        let top_ref = [10u8, 20, 30, 40, 50, 60, 70, 80];
        for (i, val) in top_ref.iter().enumerate() {
            plane[(y0 - 1) * stride + x0 + i] = *val;
        }
        for row in 0..4 {
            plane[(y0 + row) * stride + x0 - 1] = 200 + row as u8;
        }

        predict_4x4(&mut plane, stride, x0, y0, 3);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [20, 30, 40, 50],
            [30, 40, 50, 60],
            [40, 50, 60, 70],
            [50, 60, 70, 78],
        ];

        assert_eq!(got, expect, "模式3应仅按上方与右上样本进行对角线向下左预测");
    }

    #[test]
    fn test_intra4x4_mode3_without_top_falls_back_to_128() {
        let stride = 16;
        let x0 = 4;
        let y0 = 0;
        let mut plane = vec![7u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 3);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "无上方参考时模式3应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode4_diagonal_down_right_matches_spec_mapping() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[(y0 - 1) * stride + x0 - 1] = 10;
        plane[(y0 - 1) * stride + x0] = 20;
        plane[(y0 - 1) * stride + x0 + 1] = 30;
        plane[(y0 - 1) * stride + x0 + 2] = 40;
        plane[(y0 - 1) * stride + x0 + 3] = 50;
        plane[y0 * stride + x0 - 1] = 60;
        plane[(y0 + 1) * stride + x0 - 1] = 70;
        plane[(y0 + 2) * stride + x0 - 1] = 80;
        plane[(y0 + 3) * stride + x0 - 1] = 90;

        predict_4x4(&mut plane, stride, x0, y0, 4);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [25, 20, 30, 40],
            [50, 25, 20, 30],
            [70, 50, 25, 20],
            [80, 70, 50, 25],
        ];

        assert_eq!(
            got, expect,
            "模式4应按规范使用左/左上/上样本的对角线向下右映射"
        );
    }

    #[test]
    fn test_intra4x4_mode4_without_left_or_top_falls_back_to_128() {
        let stride = 16;
        let x0 = 0;
        let y0 = 4;
        let mut plane = vec![9u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 4);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式4在缺少左或上参考时应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode5_vertical_right_matches_spec_mapping() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[(y0 - 1) * stride + x0 - 1] = 10;
        plane[(y0 - 1) * stride + x0] = 20;
        plane[(y0 - 1) * stride + x0 + 1] = 30;
        plane[(y0 - 1) * stride + x0 + 2] = 40;
        plane[(y0 - 1) * stride + x0 + 3] = 50;
        plane[y0 * stride + x0 - 1] = 60;
        plane[(y0 + 1) * stride + x0 - 1] = 70;
        plane[(y0 + 2) * stride + x0 - 1] = 80;
        plane[(y0 + 3) * stride + x0 - 1] = 90;

        predict_4x4(&mut plane, stride, x0, y0, 5);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [20, 25, 35, 45],
            [25, 20, 25, 35],
            [50, 25, 20, 25],
            [65, 50, 25, 20],
        ];

        assert_eq!(got, expect, "模式5应按规范进行竖直-右预测映射");
    }

    #[test]
    fn test_intra4x4_mode5_without_left_or_top_falls_back_to_128() {
        let stride = 16;
        let x0 = 0;
        let y0 = 4;
        let mut plane = vec![11u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 5);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式5在缺少左或上参考时应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode6_horizontal_down_matches_spec_mapping() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[(y0 - 1) * stride + x0 - 1] = 10;
        plane[(y0 - 1) * stride + x0] = 20;
        plane[(y0 - 1) * stride + x0 + 1] = 30;
        plane[(y0 - 1) * stride + x0 + 2] = 40;
        plane[(y0 - 1) * stride + x0 + 3] = 50;
        plane[y0 * stride + x0 - 1] = 60;
        plane[(y0 + 1) * stride + x0 - 1] = 70;
        plane[(y0 + 2) * stride + x0 - 1] = 80;
        plane[(y0 + 3) * stride + x0 - 1] = 90;

        predict_4x4(&mut plane, stride, x0, y0, 6);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [50, 65, 75, 85],
            [25, 50, 65, 75],
            [20, 25, 50, 65],
            [25, 20, 25, 50],
        ];

        assert_eq!(got, expect, "模式6应按规范进行水平-下预测映射");
    }

    #[test]
    fn test_intra4x4_mode6_without_left_or_top_falls_back_to_128() {
        let stride = 16;
        let x0 = 0;
        let y0 = 4;
        let mut plane = vec![13u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 6);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式6在缺少左或上参考时应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode7_vertical_left_matches_spec_mapping() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        let top_ref = [20u8, 30, 40, 50, 60, 70, 80, 90];
        for (i, val) in top_ref.iter().enumerate() {
            plane[(y0 - 1) * stride + x0 + i] = *val;
        }
        for row in 0..4 {
            plane[(y0 + row) * stride + x0 - 1] = 200 + row as u8;
        }

        predict_4x4(&mut plane, stride, x0, y0, 7);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [25, 35, 45, 55],
            [30, 40, 50, 60],
            [35, 45, 55, 65],
            [40, 50, 60, 70],
        ];

        assert_eq!(
            got, expect,
            "模式7应按规范使用上方与右上样本进行竖直-左预测"
        );
    }

    #[test]
    fn test_intra4x4_mode7_without_top_falls_back_to_128() {
        let stride = 16;
        let x0 = 4;
        let y0 = 0;
        let mut plane = vec![15u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 7);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式7在缺少上方参考时应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode8_horizontal_up_matches_spec_mapping() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[y0 * stride + x0 - 1] = 20;
        plane[(y0 + 1) * stride + x0 - 1] = 30;
        plane[(y0 + 2) * stride + x0 - 1] = 40;
        plane[(y0 + 3) * stride + x0 - 1] = 50;
        plane[(y0 + 4) * stride + x0 - 1] = 60;
        for i in 0..8 {
            plane[(y0 - 1) * stride + x0 + i] = 200 + i as u8;
        }

        predict_4x4(&mut plane, stride, x0, y0, 8);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [
            [25, 30, 35, 40],
            [35, 40, 45, 50],
            [45, 50, 55, 58],
            [55, 58, 60, 60],
        ];

        assert_eq!(got, expect, "模式8应按规范使用左样本进行水平-上预测");
    }

    #[test]
    fn test_intra4x4_mode8_without_left_falls_back_to_128() {
        let stride = 16;
        let x0 = 0;
        let y0 = 4;
        let mut plane = vec![17u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 8);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式8在缺少左参考时应回退为128填充");
    }

    #[test]
    fn test_intra4x4_mode2_dc_both_neighbors_uses_rounding_rule() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        let top = [1u8, 2, 3, 4];
        for (i, val) in top.iter().enumerate() {
            plane[(y0 - 1) * stride + x0 + i] = *val;
        }
        let left = [5u8, 6, 7, 8];
        for (i, val) in left.iter().enumerate() {
            plane[(y0 + i) * stride + x0 - 1] = *val;
        }

        predict_4x4(&mut plane, stride, x0, y0, 2);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[5u8; 4]; 4];

        assert_eq!(
            got, expect,
            "模式2在上左都可用时应使用(sum_top+sum_left+4)>>3"
        );
    }

    #[test]
    fn test_intra4x4_mode2_dc_left_only_variant() {
        let stride = 16;
        let x0 = 4;
        let y0 = 0;
        let mut plane = vec![0u8; stride * stride];

        let left = [1u8, 2, 3, 4];
        for (i, val) in left.iter().enumerate() {
            plane[(y0 + i) * stride + x0 - 1] = *val;
        }

        predict_4x4(&mut plane, stride, x0, y0, 2);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[3u8; 4]; 4];

        assert_eq!(
            got, expect,
            "模式2在仅左可用时应使用Left-DC变体(sum_left+2)>>2"
        );
    }

    #[test]
    fn test_intra4x4_mode2_dc_top_only_variant() {
        let stride = 16;
        let x0 = 0;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        let top = [1u8, 2, 3, 4];
        for (i, val) in top.iter().enumerate() {
            plane[(y0 - 1) * stride + x0 + i] = *val;
        }

        predict_4x4(&mut plane, stride, x0, y0, 2);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[3u8; 4]; 4];

        assert_eq!(
            got, expect,
            "模式2在仅上可用时应使用Top-DC变体(sum_top+2)>>2"
        );
    }

    #[test]
    fn test_intra4x4_mode2_dc_none_variant_128() {
        let stride = 16;
        let x0 = 0;
        let y0 = 0;
        let mut plane = vec![99u8; stride * stride];

        predict_4x4(&mut plane, stride, x0, y0, 2);
        let got = read_block_4x4(&plane, stride, x0, y0);
        let expect = [[128u8; 4]; 4];

        assert_eq!(got, expect, "模式2在上左都不可用时应使用DC-128变体");
    }

    #[test]
    fn test_chroma_mode1_horizontal_uses_left_samples() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        for row in 0..8 {
            plane[(y0 + row) * stride + x0 - 1] = 10 + row as u8;
        }
        for col in 0..8 {
            plane[(y0 - 1) * stride + x0 + col] = 200 + col as u8;
        }

        predict_chroma_8x8(&mut plane, stride, x0, y0, 1, true, true);
        let got = read_block_8x8(&plane, stride, x0, y0);
        for (row, row_vals) in got.iter().enumerate() {
            let expect = 10 + row as u8;
            for (col, val) in row_vals.iter().enumerate() {
                assert_eq!(*val, expect, "mode1 第({}, {})像素应复制左侧样本", row, col);
            }
        }
    }

    #[test]
    fn test_chroma_mode2_vertical_uses_top_samples() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        for col in 0..8 {
            plane[(y0 - 1) * stride + x0 + col] = 20 + col as u8;
        }
        for row in 0..8 {
            plane[(y0 + row) * stride + x0 - 1] = 180 + row as u8;
        }

        predict_chroma_8x8(&mut plane, stride, x0, y0, 2, true, true);
        let got = read_block_8x8(&plane, stride, x0, y0);
        for (row, row_vals) in got.iter().enumerate() {
            for (col, val) in row_vals.iter().enumerate() {
                let expect = 20 + col as u8;
                assert_eq!(*val, expect, "mode2 第({}, {})像素应复制上方样本", row, col);
            }
        }
    }

    #[test]
    fn test_chroma_mode3_plane_matches_expected_gradient_points() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[(y0 - 1) * stride + x0 - 1] = 5;
        let top_ref = [20u8, 30, 40, 50, 60, 70, 80, 90];
        for (i, val) in top_ref.iter().enumerate() {
            plane[(y0 - 1) * stride + x0 + i] = *val;
        }
        let left_ref = [15u8, 25, 35, 45, 55, 65, 75, 85];
        for (i, val) in left_ref.iter().enumerate() {
            plane[(y0 + i) * stride + x0 - 1] = *val;
        }

        predict_chroma_8x8(&mut plane, stride, x0, y0, 3, true, true);
        let got = read_block_8x8(&plane, stride, x0, y0);

        assert_eq!(got[0][0], 27, "mode3 左上角像素应符合 plane 公式");
        assert_eq!(got[0][7], 99, "mode3 右上角像素应符合 plane 公式");
        assert_eq!(got[7][0], 97, "mode3 左下角像素应符合 plane 公式");
        assert_eq!(got[7][7], 169, "mode3 右下角像素应符合 plane 公式");
    }

    #[test]
    fn test_chroma_predict_8x8_dispatch_covers_all_modes() {
        let stride = 16;
        let x0 = 4;
        let y0 = 4;
        let mut plane = vec![0u8; stride * stride];

        plane[(y0 - 1) * stride + x0 - 1] = 10;
        for i in 0..8 {
            plane[(y0 - 1) * stride + x0 + i] = 20 + i as u8;
            plane[(y0 + i) * stride + x0 - 1] = 40 + i as u8;
        }

        let mut mode0 = plane.clone();
        let mut mode1 = plane.clone();
        let mut mode2 = plane.clone();
        let mut mode3 = plane.clone();

        predict_chroma_8x8(&mut mode0, stride, x0, y0, 0, true, true);
        predict_chroma_8x8(&mut mode1, stride, x0, y0, 1, true, true);
        predict_chroma_8x8(&mut mode2, stride, x0, y0, 2, true, true);
        predict_chroma_8x8(&mut mode3, stride, x0, y0, 3, true, true);

        let b0 = read_block_8x8(&mode0, stride, x0, y0);
        let b1 = read_block_8x8(&mode1, stride, x0, y0);
        let b2 = read_block_8x8(&mode2, stride, x0, y0);
        let b3 = read_block_8x8(&mode3, stride, x0, y0);

        assert_eq!(b0[0][0], 33, "mode0 首像素应为 DC 预测结果");
        assert_eq!(b1[0][0], 40, "mode1 首像素应来自左样本");
        assert_eq!(b2[0][0], 20, "mode2 首像素应来自上样本");
        assert_eq!(b3[0][0], 23, "mode3 首像素应来自 plane 预测结果");
    }

    // ===== Intra 8x8 预测测试 =====

    /// 辅助: 读取 8x8 块内容
    fn read_block_8x8_intra(plane: &[u8], stride: usize, x0: usize, y0: usize) -> [[u8; 8]; 8] {
        let mut out = [[0u8; 8]; 8];
        for dy in 0..8 {
            for dx in 0..8 {
                out[dy][dx] = plane[(y0 + dy) * stride + x0 + dx];
            }
        }
        out
    }

    #[test]
    fn test_intra8x8_mode2_dc_no_neighbors_gives_128() {
        // 模式 2 (DC): 无上无左时填充 128
        let stride = 32;
        let x0 = 0;
        let y0 = 0;
        let mut plane = vec![200u8; stride * 32];

        super::predict_8x8(
            &mut plane,
            stride,
            x0,
            y0,
            2,
            &super::I8x8Avail {
                has_left: false,
                has_top: false,
                has_topleft: false,
                has_topright: false,
            },
        );
        let got = read_block_8x8_intra(&plane, stride, x0, y0);
        for row in &got {
            assert_eq!(row, &[128u8; 8], "无邻居时 DC 预测应填充 128");
        }
    }

    #[test]
    fn test_intra8x8_mode2_dc_top_only() {
        // 模式 2 (DC): 仅上方可用, DC = (sum_top + 4) >> 3
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * 32];
        // 设置上方 8 个参考像素均为 16 (无滤波时 DC = (8*16+4)>>3 = 16)
        // 经过低通滤波后值不变 (相邻值相同)
        for dx in 0..8 {
            plane[(y0 - 1) * stride + x0 + dx] = 16;
        }
        // 需要左上角为 16 (has_topleft=true 情况下)
        plane[(y0 - 1) * stride + x0 - 1] = 16;

        super::predict_8x8(
            &mut plane,
            stride,
            x0,
            y0,
            2,
            &super::I8x8Avail {
                has_left: false,
                has_top: true,
                has_topleft: true,
                has_topright: false,
            },
        );
        let got = read_block_8x8_intra(&plane, stride, x0, y0);
        // 滤波后所有 t[] = 16, DC = (8*16+4)>>3 = 16
        for row in &got {
            for &v in row {
                assert_eq!(v, 16, "DC 预测: 仅上方可用时应填充上方均值");
            }
        }
    }

    #[test]
    fn test_intra8x8_mode0_vertical_copies_top() {
        // 模式 0 (垂直): 各列从上方样本复制
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * 32];
        let top_row: [u8; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        for dx in 0..10 {
            plane[(y0 - 1) * stride + x0 + dx - 1] = top_row[dx];
        }

        super::predict_8x8(
            &mut plane,
            stride,
            x0,
            y0,
            0,
            &super::I8x8Avail {
                has_left: false,
                has_top: true,
                has_topleft: true,
                has_topright: true,
            },
        );
        let got = read_block_8x8_intra(&plane, stride, x0, y0);

        // 验证每列的值应来自上方 (经过滤波)
        // 至少首行所有列值应相同 (来自 top)
        let first_row = got[0];
        for row in got[1..].iter() {
            assert_eq!(*row, first_row, "垂直预测: 每行应与第一行相同");
        }
    }

    #[test]
    fn test_intra8x8_mode1_horizontal_copies_left() {
        // 模式 1 (水平): 各行从左侧样本复制
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        let mut plane = vec![0u8; stride * 32];
        for dy in 0..9 {
            plane[(y0 + dy - 1) * stride + x0 - 1] = (dy * 10) as u8;
        }

        super::predict_8x8(
            &mut plane,
            stride,
            x0,
            y0,
            1,
            &super::I8x8Avail {
                has_left: true,
                has_top: false,
                has_topleft: false,
                has_topright: false,
            },
        );
        let got = read_block_8x8_intra(&plane, stride, x0, y0);

        // 水平预测: 每行所有像素相同 (来自左侧)
        for row in &got {
            let v = row[0];
            for &p in row {
                assert_eq!(p, v, "水平预测: 每行内所有像素应相同");
            }
        }
    }

    #[test]
    fn test_intra8x8_no_top_falls_back_for_mode0() {
        // 模式 0 (垂直): 无上方时回退为 128
        let stride = 16;
        let x0 = 0;
        let y0 = 0;
        let mut plane = vec![99u8; stride * 16];

        super::predict_8x8(
            &mut plane,
            stride,
            x0,
            y0,
            0,
            &super::I8x8Avail {
                has_left: false,
                has_top: false,
                has_topleft: false,
                has_topright: false,
            },
        );
        let got = read_block_8x8_intra(&plane, stride, x0, y0);
        for row in &got {
            assert_eq!(row, &[128u8; 8], "无上方邻居时垂直预测应回退为 128");
        }
    }

    #[test]
    fn test_intra8x8_dispatch_covers_all_modes() {
        // 覆盖测试: 所有 9 种模式均可调用, 无崩溃
        let stride = 32;
        let x0 = 8;
        let y0 = 8;
        for mode in 0u8..9 {
            let mut plane = vec![128u8; stride * 32];
            // 设置充分的边界参考
            for dx in 0..16 {
                plane[(y0 - 1) * stride + x0 + dx] = 100 + dx as u8;
            }
            for dy in 0..8 {
                plane[(y0 + dy) * stride + x0 - 1] = 50 + dy as u8;
            }
            plane[(y0 - 1) * stride + x0 - 1] = 80;
            super::predict_8x8(
                &mut plane,
                stride,
                x0,
                y0,
                mode,
                &super::I8x8Avail {
                    has_left: true,
                    has_top: true,
                    has_topleft: true,
                    has_topright: true,
                },
            );
            // 仅验证不崩溃且输出范围合法
            let got = read_block_8x8_intra(&plane, stride, x0, y0);
            for row in &got {
                // u8 值始终在 [0,255] 范围内, 此处仅验证函数正常运行不崩溃
                let _ = row;
            }
        }
    }
}
