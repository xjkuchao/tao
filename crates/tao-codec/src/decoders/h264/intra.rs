//! H.264 帧内预测模式实现.
//!
//! 提供 Intra_16x16 亮度预测 (4 种模式) 和色度 DC 预测.

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
    let mut sum = 0u32;
    let mut count = 0u32;

    if has_top && y0 > 0 {
        for dx in 0..16 {
            sum += plane[(y0 - 1) * stride + x0 + dx] as u32;
            count += 1;
        }
    }
    if has_left && x0 > 0 {
        for dy in 0..16 {
            sum += plane[(y0 + dy) * stride + x0 - 1] as u32;
            count += 1;
        }
    }
    if count > 0 { (sum / count) as u8 } else { 128 }
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
    let mut sum = 0u32;
    let mut count = 0u32;

    if y0 > 0 {
        for dx in 0..4 {
            let idx = (y0 - 1) * stride + x0 + dx;
            if idx < plane.len() {
                sum += plane[idx] as u32;
                count += 1;
            }
        }
    }
    if x0 > 0 {
        for dy in 0..4 {
            let idx = (y0 + dy) * stride + x0 - 1;
            if idx < plane.len() {
                sum += plane[idx] as u32;
                count += 1;
            }
        }
    }
    let dc = if count > 0 { (sum / count) as u8 } else { 128 };
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
    let mut ref_vals = [128u8; 8];

    if y0 > 0 {
        for (i, ref_val) in ref_vals.iter_mut().enumerate().take(4) {
            let idx = (y0 - 1) * stride + x0 + i;
            if idx < plane.len() {
                *ref_val = plane[idx];
            }
        }
        if x0 + 4 < stride && (y0 - 1) * stride + x0 + 4 < plane.len() {
            ref_vals[4] = plane[(y0 - 1) * stride + x0 + 4];
        }
    }
    if x0 > 0 {
        for i in 0..4 {
            let idx = (y0 + i) * stride + x0 - 1;
            if idx < plane.len() {
                ref_vals[i + 1] = plane[idx];
            }
        }
    }

    for dy in 0..4 {
        for dx in 0..4 {
            let val = if dy % 2 == 0 {
                let a = ref_vals[dx + dy / 2];
                let b = ref_vals[dx + dy / 2 + 1];
                let c = ref_vals.get(dx + dy / 2 + 2).copied().unwrap_or(128);
                ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8
            } else {
                let a = ref_vals[dx + dy.div_ceil(2)];
                let b = ref_vals[dx + dy.div_ceil(2) + 1];
                (a as u32 + b as u32).div_ceil(2) as u8
            };
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                plane[idx] = val;
            }
        }
    }
}

/// 模式 8: 水平-上 (Horizontal-Up)
fn predict_4x4_horizontal_up(plane: &mut [u8], stride: usize, x0: usize, y0: usize) {
    let mut ref_vals = [128u8; 8];

    if x0 > 0 {
        for (i, item) in ref_vals.iter_mut().enumerate().take(4) {
            let idx = (y0 + i) * stride + x0 - 1;
            if idx < plane.len() {
                *item = plane[idx];
            }
        }
        if y0 + 4 < plane.len() / stride {
            let idx = (y0 + 4) * stride + x0 - 1;
            if idx < plane.len() {
                ref_vals[4] = plane[idx];
            }
        }
    }

    for dy in 0..4 {
        for dx in 0..4 {
            let val = if dx < dy {
                if dx + 1 == dy {
                    let a = ref_vals[dy];
                    let b = ref_vals[dy + 1];
                    (a as u32 + b as u32).div_ceil(2) as u8
                } else {
                    ref_vals[dy + 1]
                }
            } else if dx == dy {
                let a = ref_vals[dy];
                let b = ref_vals[dy + 1];
                let c = ref_vals.get(dy + 2).copied().unwrap_or(128);
                ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8
            } else {
                let a = ref_vals[dx - dy];
                let b = ref_vals[dx - dy + 1];
                let c = ref_vals.get(dx - dy + 2).copied().unwrap_or(128);
                ((a as u32 + 2 * b as u32 + c as u32 + 2) / 4) as u8
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

#[cfg(test)]
mod tests {
    use super::predict_4x4;

    fn read_block_4x4(plane: &[u8], stride: usize, x0: usize, y0: usize) -> [[u8; 4]; 4] {
        let mut block = [[0u8; 4]; 4];
        for dy in 0..4 {
            for dx in 0..4 {
                block[dy][dx] = plane[(y0 + dy) * stride + x0 + dx];
            }
        }
        block
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
}
