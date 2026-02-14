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

/// 4x4 子块 DC 预测 (用于 I_4x4 简化实现)
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

/// 将残差值加到平面上 (用于 DC 残差叠加)
pub fn add_residual_to_block(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    residual: i32,
) {
    for dy in 0..h {
        for dx in 0..w {
            let idx = (y0 + dy) * stride + x0 + dx;
            if idx < plane.len() {
                let val = plane[idx] as i32 + residual;
                plane[idx] = val.clamp(0, 255) as u8;
            }
        }
    }
}
