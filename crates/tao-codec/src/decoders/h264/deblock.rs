//! H.264 去块滤波(参数化基础实现).
//!
//! 当前实现基于 `slice_qp + alpha/beta offset` 自适应决定是否平滑边界:
//! - 亮度按 4x4 边界处理
//! - 色度按 2x2 边界处理
//! - 采用轻量 `p1/p0/q0/q1` 门限与 delta 裁剪, 便于后续扩展到完整强弱滤波

/// 去块滤波输入参数.
#[derive(Clone, Copy, Debug)]
pub(super) struct DeblockSliceParams {
    pub(super) stride_y: usize,
    pub(super) stride_c: usize,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) slice_qp: i32,
    pub(super) alpha_offset_div2: i32,
    pub(super) beta_offset_div2: i32,
}

/// 对 YUV420 帧执行带 slice 参数的去块滤波.
pub(super) fn apply_deblock_yuv420_with_slice_params(
    y: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
    params: DeblockSliceParams,
) {
    let DeblockSliceParams {
        stride_y,
        stride_c,
        width,
        height,
        slice_qp,
        alpha_offset_div2,
        beta_offset_div2,
    } = params;
    if width == 0 || height == 0 {
        return;
    }
    let alpha = alpha_threshold(slice_qp, alpha_offset_div2);
    let beta = beta_threshold(slice_qp, beta_offset_div2);
    if alpha == 0 || beta == 0 {
        return;
    }

    apply_adaptive_deblock_plane(y, stride_y, width, height, 4, alpha, beta);
    apply_adaptive_deblock_plane(u, stride_c, width / 2, height / 2, 2, alpha, beta);
    apply_adaptive_deblock_plane(v, stride_c, width / 2, height / 2, 2, alpha, beta);
}

fn apply_adaptive_deblock_plane(
    plane: &mut [u8],
    stride: usize,
    width: usize,
    height: usize,
    boundary_step: usize,
    alpha: u8,
    beta: u8,
) {
    if width < 3 || height < 3 || stride == 0 || boundary_step == 0 {
        return;
    }

    // 垂直边界
    let mut x = boundary_step;
    while x < width.saturating_sub(1) {
        if x >= 2 {
            for y in 0..height {
                let p1 = y * stride + (x - 2);
                let p0 = y * stride + (x - 1);
                let q0 = y * stride + x;
                let q1 = y * stride + (x + 1);
                if p1 >= plane.len() || p0 >= plane.len() || q0 >= plane.len() || q1 >= plane.len()
                {
                    continue;
                }
                filter_edge(plane, p1, p0, q0, q1, alpha, beta);
            }
        }
        x += boundary_step;
    }

    // 水平边界
    let mut y = boundary_step;
    while y < height.saturating_sub(1) {
        if y >= 2 {
            for x in 0..width {
                let p1 = (y - 2) * stride + x;
                let p0 = (y - 1) * stride + x;
                let q0 = y * stride + x;
                let q1 = (y + 1) * stride + x;
                if p1 >= plane.len() || p0 >= plane.len() || q0 >= plane.len() || q1 >= plane.len()
                {
                    continue;
                }
                filter_edge(plane, p1, p0, q0, q1, alpha, beta);
            }
        }
        y += boundary_step;
    }
}

fn filter_edge(
    plane: &mut [u8],
    p1_idx: usize,
    p0_idx: usize,
    q0_idx: usize,
    q1_idx: usize,
    alpha: u8,
    beta: u8,
) {
    let p1 = i32::from(plane[p1_idx]);
    let p0 = i32::from(plane[p0_idx]);
    let q0 = i32::from(plane[q0_idx]);
    let q1 = i32::from(plane[q1_idx]);

    if (p0 - q0).abs() >= i32::from(alpha) {
        return;
    }
    if (p1 - p0).abs() >= i32::from(beta) || (q1 - q0).abs() >= i32::from(beta) {
        return;
    }

    let tc = (i32::from(alpha) / 4 + 2).max(1);
    let mut delta = ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3;
    delta = delta.clamp(-tc, tc);

    plane[p0_idx] = (p0 + delta).clamp(0, 255) as u8;
    plane[q0_idx] = (q0 - delta).clamp(0, 255) as u8;
}

fn alpha_threshold(slice_qp: i32, alpha_offset_div2: i32) -> u8 {
    let idx = (slice_qp + alpha_offset_div2 * 2).clamp(0, 51) as usize;
    ALPHA_TABLE[idx]
}

fn beta_threshold(slice_qp: i32, beta_offset_div2: i32) -> u8 {
    let idx = (slice_qp + beta_offset_div2 * 2).clamp(0, 51) as usize;
    BETA_TABLE[idx]
}

#[rustfmt::skip]
const ALPHA_TABLE: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20, 22, 25, 28,
    32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182,
    203, 226, 255, 255,
];

#[rustfmt::skip]
const BETA_TABLE: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16,
    17, 17, 18, 18,
];

#[cfg(test)]
mod tests {
    use super::{
        DeblockSliceParams, alpha_threshold, apply_adaptive_deblock_plane,
        apply_deblock_yuv420_with_slice_params,
    };

    #[test]
    fn test_apply_adaptive_deblock_plane_smooth_small_edge() {
        let width = 8usize;
        let height = 6usize;
        let stride = width;
        let mut plane = vec![40u8; width * height];
        for y in 0..height {
            plane[y * stride + 3] = 40;
            plane[y * stride + 4] = 48;
            plane[y * stride + 5] = 48;
        }

        apply_adaptive_deblock_plane(&mut plane, stride, width, height, 4, 15, 4);

        for y in 0..height {
            let left = plane[y * stride + 3];
            let right = plane[y * stride + 4];
            assert!(left > 40, "左侧边界应被平滑提升");
            assert!(right < 48, "右侧边界应被平滑回拉");
        }
    }

    #[test]
    fn test_apply_adaptive_deblock_plane_keep_large_edge() {
        let width = 8usize;
        let height = 6usize;
        let stride = width;
        let mut plane = vec![10u8; width * height];
        for y in 0..height {
            plane[y * stride + 3] = 10;
            plane[y * stride + 4] = 90;
            plane[y * stride + 5] = 90;
        }

        apply_adaptive_deblock_plane(&mut plane, stride, width, height, 4, 15, 4);

        for y in 0..height {
            assert_eq!(plane[y * stride + 3], 10, "大边界差异不应被平滑");
            assert_eq!(plane[y * stride + 4], 90, "大边界差异不应被平滑");
        }
    }

    #[test]
    fn test_apply_deblock_slice_offset_strength() {
        let width = 16usize;
        let height = 8usize;
        let stride = width;
        let mut base = vec![40u8; stride * height];
        for y in 0..height {
            base[y * stride + 7] = 40; // p1
            base[y * stride + 8] = 50; // q0
            base[y * stride + 9] = 50; // q1
        }

        let mut no_offset = base.clone();
        let mut no_offset_u = [0u8; 1];
        let mut no_offset_v = [0u8; 1];
        apply_deblock_yuv420_with_slice_params(
            &mut no_offset,
            &mut no_offset_u,
            &mut no_offset_v,
            DeblockSliceParams {
                stride_y: stride,
                stride_c: 1,
                width,
                height,
                slice_qp: 20,
                alpha_offset_div2: 0,
                beta_offset_div2: 0,
            },
        );

        let mut stronger = base.clone();
        let mut stronger_u = [0u8; 1];
        let mut stronger_v = [0u8; 1];
        apply_deblock_yuv420_with_slice_params(
            &mut stronger,
            &mut stronger_u,
            &mut stronger_v,
            DeblockSliceParams {
                stride_y: stride,
                stride_c: 1,
                width,
                height,
                slice_qp: 20,
                alpha_offset_div2: 3,
                beta_offset_div2: 0,
            },
        );

        let idx = 8;
        assert_eq!(no_offset[idx], 50, "默认 offset 下该边界不应被滤波");
        assert!(stronger[idx] < 50, "提高 alpha offset 后应触发更强滤波");
    }

    #[test]
    fn test_alpha_threshold_clamp() {
        assert_eq!(alpha_threshold(26, 0), 15, "QP26 alpha 阈值应为 15");
        assert_eq!(alpha_threshold(1, -6), 0, "负偏移应被裁剪到 0");
        assert_eq!(alpha_threshold(51, 6), 255, "正偏移应被裁剪到 51");
    }

    #[test]
    fn test_apply_deblock_yuv420_default_params_basic() {
        let width = 16usize;
        let height = 16usize;
        let stride_y = width;
        let stride_c = width / 2;
        let mut y = vec![32u8; stride_y * height];
        let mut u = vec![64u8; stride_c * (height / 2)];
        let mut v = vec![96u8; stride_c * (height / 2)];

        // 构造可通过 alpha/beta 门限的边界样本.
        for row in 0..height {
            let base = row * stride_y;
            y[base + 6] = 40;
            y[base + 7] = 40;
            y[base + 8] = 48;
            y[base + 9] = 48;
        }
        for row in 0..(height / 2) {
            let base = row * stride_c;
            u[base + 2] = 64;
            u[base + 3] = 64;
            u[base + 4] = 72;
            u[base + 5] = 72;
            v[base + 2] = 96;
            v[base + 3] = 96;
            v[base + 4] = 104;
            v[base + 5] = 104;
        }

        apply_deblock_yuv420_with_slice_params(
            &mut y,
            &mut u,
            &mut v,
            DeblockSliceParams {
                stride_y,
                stride_c,
                width,
                height,
                slice_qp: 26,
                alpha_offset_div2: 0,
                beta_offset_div2: 0,
            },
        );

        assert_ne!(y[8], 48, "亮度平面应发生平滑");
        assert_ne!(u[4], 72, "U 平面应发生平滑");
        assert_ne!(v[4], 104, "V 平面应发生平滑");
    }
}
