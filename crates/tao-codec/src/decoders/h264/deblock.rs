//! H.264 去块滤波(参数化实现).
//!
//! 当前实现基于 `slice_qp + alpha/beta offset` 与边界强度(`bs`)执行最小规范化滤波:
//! - 亮度按 4x4 边界处理, 色度按 2x2 边界处理.
//! - 宏块边界按 `intra/cbp` 估算强弱(`bs=4/2`), 非宏块边界走弱滤波(`bs=1`).
//! - 弱滤波使用 `tc0` 约束, 强滤波使用更强的 `p0/q0` 更新.

/// 去块滤波输入参数.
#[derive(Clone, Copy, Debug)]
pub(super) struct DeblockSliceParams<'a> {
    pub(super) stride_y: usize,
    pub(super) stride_c: usize,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) slice_qp: i32,
    pub(super) alpha_offset_div2: i32,
    pub(super) beta_offset_div2: i32,
    pub(super) mb_width: usize,
    pub(super) mb_height: usize,
    pub(super) mb_types: Option<&'a [u8]>,
    pub(super) mb_cbp: Option<&'a [u8]>,
}

/// 对 YUV420 帧执行带 slice 参数的去块滤波.
pub(super) fn apply_deblock_yuv420_with_slice_params(
    y: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
    params: DeblockSliceParams<'_>,
) {
    let DeblockSliceParams {
        stride_y,
        stride_c,
        width,
        height,
        slice_qp,
        alpha_offset_div2,
        beta_offset_div2,
        mb_width,
        mb_height,
        mb_types,
        mb_cbp,
    } = params;
    if width == 0 || height == 0 {
        return;
    }
    let alpha_idx = alpha_index(slice_qp, alpha_offset_div2);
    let alpha = alpha_threshold(slice_qp, alpha_offset_div2);
    let beta = beta_threshold(slice_qp, beta_offset_div2);
    if alpha == 0 || beta == 0 {
        return;
    }

    let mb_ctx = mb_types.and_then(|types| {
        mb_cbp.and_then(|cbp| {
            let need = mb_width.saturating_mul(mb_height);
            if need == 0 || types.len() < need || cbp.len() < need {
                None
            } else {
                Some(DeblockMbContext {
                    mb_width,
                    mb_height,
                    mb_types: types,
                    mb_cbp: cbp,
                })
            }
        })
    });

    apply_adaptive_deblock_plane(
        y,
        stride_y,
        width,
        height,
        4,
        16,
        alpha_idx,
        alpha,
        beta,
        mb_ctx.as_ref(),
    );
    apply_adaptive_deblock_plane(
        u,
        stride_c,
        width / 2,
        height / 2,
        2,
        8,
        alpha_idx,
        alpha,
        beta,
        mb_ctx.as_ref(),
    );
    apply_adaptive_deblock_plane(
        v,
        stride_c,
        width / 2,
        height / 2,
        2,
        8,
        alpha_idx,
        alpha,
        beta,
        mb_ctx.as_ref(),
    );
}

#[derive(Clone, Copy)]
struct DeblockMbContext<'a> {
    mb_width: usize,
    mb_height: usize,
    mb_types: &'a [u8],
    mb_cbp: &'a [u8],
}

#[allow(clippy::too_many_arguments)]
fn apply_adaptive_deblock_plane(
    plane: &mut [u8],
    stride: usize,
    width: usize,
    height: usize,
    boundary_step: usize,
    mb_step: usize,
    alpha_idx: usize,
    alpha: u8,
    beta: u8,
    mb_ctx: Option<&DeblockMbContext<'_>>,
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
                let bs = boundary_strength_vertical(x, y, mb_step, mb_ctx);
                filter_edge_with_bs(plane, p1, p0, q0, q1, alpha_idx, alpha, beta, bs);
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
                let bs = boundary_strength_horizontal(x, y, mb_step, mb_ctx);
                filter_edge_with_bs(plane, p1, p0, q0, q1, alpha_idx, alpha, beta, bs);
            }
        }
        y += boundary_step;
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_edge_with_bs(
    plane: &mut [u8],
    p1_idx: usize,
    p0_idx: usize,
    q0_idx: usize,
    q1_idx: usize,
    alpha_idx: usize,
    alpha: u8,
    beta: u8,
    bs: u8,
) {
    if bs == 0 {
        return;
    }
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

    let tc = if bs >= 4 {
        (i32::from(alpha) / 4 + 4).max(2)
    } else {
        let tc0 = i32::from(tc0_threshold(alpha_idx, bs));
        if tc0 == 0 {
            return;
        }
        tc0 + if (p1 - p0).abs() < i32::from(beta) {
            1
        } else {
            0
        } + if (q1 - q0).abs() < i32::from(beta) {
            1
        } else {
            0
        }
    };
    let mut delta = ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3;
    delta = delta.clamp(-tc, tc);

    plane[p0_idx] = (p0 + delta).clamp(0, 255) as u8;
    plane[q0_idx] = (q0 - delta).clamp(0, 255) as u8;

    if bs >= 4 {
        let p1_delta = (delta / 2).clamp(-tc, tc);
        let q1_delta = (delta / 2).clamp(-tc, tc);
        plane[p1_idx] = (p1 + p1_delta).clamp(0, 255) as u8;
        plane[q1_idx] = (q1 - q1_delta).clamp(0, 255) as u8;
    }
}

fn alpha_index(slice_qp: i32, alpha_offset_div2: i32) -> usize {
    (slice_qp + alpha_offset_div2 * 2).clamp(0, 51) as usize
}

fn beta_index(slice_qp: i32, beta_offset_div2: i32) -> usize {
    (slice_qp + beta_offset_div2 * 2).clamp(0, 51) as usize
}

fn alpha_threshold(slice_qp: i32, alpha_offset_div2: i32) -> u8 {
    ALPHA_TABLE[alpha_index(slice_qp, alpha_offset_div2)]
}

fn beta_threshold(slice_qp: i32, beta_offset_div2: i32) -> u8 {
    BETA_TABLE[beta_index(slice_qp, beta_offset_div2)]
}

fn boundary_strength_vertical(
    x: usize,
    y: usize,
    mb_step: usize,
    mb_ctx: Option<&DeblockMbContext<'_>>,
) -> u8 {
    if mb_step == 0 || x == 0 {
        return 0;
    }
    if x % mb_step != 0 {
        let mb_x = x / mb_step;
        let mb_y = y / mb_step;
        return boundary_strength_within_mb(mb_ctx, mb_x, mb_y);
    }
    let Some(ctx) = mb_ctx else {
        return 2;
    };
    let mb_y = y / mb_step;
    let mb_x_l = (x - 1) / mb_step;
    let mb_x_r = x / mb_step;
    boundary_strength_between_mb(ctx, mb_x_l, mb_y, mb_x_r, mb_y)
}

fn boundary_strength_horizontal(
    x: usize,
    y: usize,
    mb_step: usize,
    mb_ctx: Option<&DeblockMbContext<'_>>,
) -> u8 {
    if mb_step == 0 || y == 0 {
        return 0;
    }
    if y % mb_step != 0 {
        let mb_x = x / mb_step;
        let mb_y = y / mb_step;
        return boundary_strength_within_mb(mb_ctx, mb_x, mb_y);
    }
    let Some(ctx) = mb_ctx else {
        return 2;
    };
    let mb_x = x / mb_step;
    let mb_y_t = (y - 1) / mb_step;
    let mb_y_b = y / mb_step;
    boundary_strength_between_mb(ctx, mb_x, mb_y_t, mb_x, mb_y_b)
}

fn boundary_strength_between_mb(
    ctx: &DeblockMbContext<'_>,
    mb_x_a: usize,
    mb_y_a: usize,
    mb_x_b: usize,
    mb_y_b: usize,
) -> u8 {
    let idx_a = mb_index(ctx.mb_width, ctx.mb_height, mb_x_a, mb_y_a);
    let idx_b = mb_index(ctx.mb_width, ctx.mb_height, mb_x_b, mb_y_b);
    let (Some(i_a), Some(i_b)) = (idx_a, idx_b) else {
        return 2;
    };
    let ty_a = *ctx.mb_types.get(i_a).unwrap_or(&255);
    let ty_b = *ctx.mb_types.get(i_b).unwrap_or(&255);
    if is_intra_mb(ty_a) || is_intra_mb(ty_b) {
        return 4;
    }
    let cbp_a = *ctx.mb_cbp.get(i_a).unwrap_or(&0);
    let cbp_b = *ctx.mb_cbp.get(i_b).unwrap_or(&0);
    if cbp_a != 0 || cbp_b != 0 {
        return 2;
    }
    1
}

fn boundary_strength_within_mb(
    mb_ctx: Option<&DeblockMbContext<'_>>,
    mb_x: usize,
    mb_y: usize,
) -> u8 {
    let Some(ctx) = mb_ctx else {
        return 2;
    };
    let Some(idx) = mb_index(ctx.mb_width, ctx.mb_height, mb_x, mb_y) else {
        return 2;
    };
    let ty = *ctx.mb_types.get(idx).unwrap_or(&255);
    if is_intra_mb(ty) {
        return 3;
    }
    let cbp = *ctx.mb_cbp.get(idx).unwrap_or(&0);
    if cbp != 0 {
        return 2;
    }
    1
}

fn mb_index(mb_width: usize, mb_height: usize, mb_x: usize, mb_y: usize) -> Option<usize> {
    if mb_x >= mb_width || mb_y >= mb_height {
        return None;
    }
    mb_y.checked_mul(mb_width)?.checked_add(mb_x)
}

fn is_intra_mb(mb_type: u8) -> bool {
    mb_type <= 25
}

fn tc0_threshold(alpha_idx: usize, bs: u8) -> u8 {
    if bs == 0 {
        return 0;
    }
    let bs_idx = (usize::from(bs.min(3))).saturating_sub(1);
    TC0_TABLE[alpha_idx.min(51)][bs_idx]
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

#[rustfmt::skip]
const TC0_TABLE: [[u8; 3]; 52] = [
    [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0],
    [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 1], [0, 0, 1], [0, 0, 1], [0, 0, 1], [0, 0, 1],
    [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 1],
    [0, 1, 1], [0, 1, 1], [0, 1, 1], [0, 1, 2], [0, 1, 2], [0, 1, 2], [0, 1, 2], [0, 1, 3],
    [0, 1, 3], [0, 2, 3], [0, 2, 4], [0, 2, 4], [0, 2, 4], [0, 3, 5], [0, 3, 6], [0, 3, 6],
    [0, 4, 7], [0, 4, 8], [0, 4, 9], [0, 5, 10], [0, 6, 11], [0, 6, 13], [0, 7, 14], [0, 8, 16],
    [0, 9, 18], [0, 10, 20], [0, 11, 23], [0, 13, 25],
];

#[cfg(test)]
mod tests {
    use super::{
        DeblockMbContext, DeblockSliceParams, alpha_threshold, apply_adaptive_deblock_plane,
        apply_deblock_yuv420_with_slice_params, beta_threshold, boundary_strength_vertical,
        filter_edge_with_bs,
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

        apply_adaptive_deblock_plane(&mut plane, stride, width, height, 4, 16, 26, 15, 4, None);

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

        apply_adaptive_deblock_plane(&mut plane, stride, width, height, 4, 16, 26, 15, 4, None);

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
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
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
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
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
        assert_eq!(beta_threshold(26, 0), 6, "QP26 beta 阈值应为 6");
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
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
            },
        );

        assert_ne!(y[8], 48, "亮度平面应发生平滑");
        assert_ne!(u[4], 72, "U 平面应发生平滑");
        assert_ne!(v[4], 104, "V 平面应发生平滑");
    }

    #[test]
    fn test_boundary_strength_vertical_intra_mb_boundary() {
        let mb_types = [1u8, 200u8];
        let mb_cbp = [0u8, 0u8];
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
        };
        let bs = boundary_strength_vertical(16, 0, 16, Some(&ctx));
        assert_eq!(bs, 4, "宏块边界任一侧为帧内宏块时应走强滤波");
    }

    #[test]
    fn test_filter_edge_with_bs_uses_tc0_strength() {
        let mut weak = vec![40u8, 40, 48, 48];
        let mut strong = weak.clone();

        filter_edge_with_bs(&mut weak, 0, 1, 2, 3, 26, 15, 4, 1);
        filter_edge_with_bs(&mut strong, 0, 1, 2, 3, 26, 15, 4, 3);

        assert_eq!(weak[1], 40, "弱滤波在 tc0=0 时应保持原样");
        assert_eq!(weak[2], 48, "弱滤波在 tc0=0 时应保持原样");
        assert!(strong[1] > weak[1], "更高 bs 应带来更强的左侧平滑");
        assert!(strong[2] < weak[2], "更高 bs 应带来更强的右侧平滑");
    }
}
