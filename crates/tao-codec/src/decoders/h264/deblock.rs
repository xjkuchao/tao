//! H.264 去块滤波(基础实现).
//!
//! 当前实现提供最小可用的边界平滑能力:
//! - 亮度按 4x4 边界处理
//! - 色度按 2x2 边界处理
//! - 仅做轻量双边平滑, 为后续规范化滤波预留入口

/// 对 YUV420 帧执行基础去块滤波.
pub(super) fn apply_simple_deblock_yuv420(
    y: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
    stride_y: usize,
    stride_c: usize,
    width: usize,
    height: usize,
) {
    if width == 0 || height == 0 {
        return;
    }
    apply_simple_deblock_plane(y, stride_y, width, height, 4, 12);
    apply_simple_deblock_plane(u, stride_c, width / 2, height / 2, 2, 12);
    apply_simple_deblock_plane(v, stride_c, width / 2, height / 2, 2, 12);
}

fn apply_simple_deblock_plane(
    plane: &mut [u8],
    stride: usize,
    width: usize,
    height: usize,
    boundary_step: usize,
    threshold: u8,
) {
    if width < 2 || height < 2 || stride == 0 || boundary_step == 0 {
        return;
    }

    // 垂直边界
    let mut x = boundary_step;
    while x < width {
        for y in 0..height {
            let li = y * stride + (x - 1);
            let ri = y * stride + x;
            if li >= plane.len() || ri >= plane.len() {
                continue;
            }
            smooth_pair(plane, li, ri, threshold);
        }
        x += boundary_step;
    }

    // 水平边界
    let mut y = boundary_step;
    while y < height {
        for x in 0..width {
            let ti = (y - 1) * stride + x;
            let bi = y * stride + x;
            if ti >= plane.len() || bi >= plane.len() {
                continue;
            }
            smooth_pair(plane, ti, bi, threshold);
        }
        y += boundary_step;
    }
}

fn smooth_pair(plane: &mut [u8], a_idx: usize, b_idx: usize, threshold: u8) {
    let a = plane[a_idx];
    let b = plane[b_idx];
    let diff = a.abs_diff(b);
    if diff > threshold {
        return;
    }
    let avg = ((u16::from(a) + u16::from(b) + 1) >> 1) as u8;
    plane[a_idx] = ((u16::from(a) * 3 + u16::from(avg) + 2) >> 2) as u8;
    plane[b_idx] = ((u16::from(b) * 3 + u16::from(avg) + 2) >> 2) as u8;
}

#[cfg(test)]
mod tests {
    use super::{apply_simple_deblock_plane, apply_simple_deblock_yuv420};

    #[test]
    fn test_apply_simple_deblock_plane_smooth_small_edge() {
        let width = 8usize;
        let height = 4usize;
        let stride = width;
        let mut plane = vec![40u8; width * height];
        for y in 0..height {
            plane[y * stride + 4] = 48;
        }

        apply_simple_deblock_plane(&mut plane, stride, width, height, 4, 12);

        for y in 0..height {
            let left = plane[y * stride + 3];
            let right = plane[y * stride + 4];
            assert!(left > 40, "左侧边界应被平滑提升");
            assert!(right < 48, "右侧边界应被平滑回拉");
        }
    }

    #[test]
    fn test_apply_simple_deblock_plane_keep_large_edge() {
        let width = 8usize;
        let height = 4usize;
        let stride = width;
        let mut plane = vec![10u8; width * height];
        for y in 0..height {
            plane[y * stride + 4] = 90;
        }

        apply_simple_deblock_plane(&mut plane, stride, width, height, 4, 12);

        for y in 0..height {
            assert_eq!(plane[y * stride + 3], 10, "大边界差异不应被平滑");
            assert_eq!(plane[y * stride + 4], 90, "大边界差异不应被平滑");
        }
    }

    #[test]
    fn test_apply_simple_deblock_yuv420_basic() {
        let width = 16usize;
        let height = 16usize;
        let stride_y = width;
        let stride_c = width / 2;
        let mut y = vec![32u8; stride_y * height];
        let mut u = vec![64u8; stride_c * (height / 2)];
        let mut v = vec![96u8; stride_c * (height / 2)];
        y[7] = 40;
        u[3] = 72;
        v[3] = 88;

        apply_simple_deblock_yuv420(&mut y, &mut u, &mut v, stride_y, stride_c, width, height);

        assert_ne!(y[7], 40, "亮度平面应发生平滑");
        assert_ne!(u[3], 72, "U 平面应发生平滑");
        assert_ne!(v[3], 88, "V 平面应发生平滑");
    }
}
