//! H.264 去块滤波(参数化实现).
//!
//! 当前实现基于 `slice_qp + alpha/beta offset` 与边界强度(`bs`)执行最小规范化滤波:
//! - 亮度按 4x4 边界处理, 色度按 2x2 边界处理.
//! - 宏块边界按 `intra/cbp/ref_idx/mv` 估算强弱(`bs=4/2/1/0`).
//! - 亮度 4x4 内部边界按 `cbf/ref_idx/mv` 估算强弱(`bs=2/1/0`).
//! - 弱滤波使用 `tc0` 约束, 强滤波使用更强的 `p0/q0` 更新.

use super::common::chroma_qp_from_luma_with_offset;

/// 去块滤波输入参数.
#[derive(Clone, Copy, Debug)]
pub(super) struct DeblockSliceParams<'a> {
    pub(super) stride_y: usize,
    pub(super) stride_c: usize,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) slice_qp: i32,
    pub(super) disable_deblocking_filter_idc: u32,
    pub(super) chroma_qp_index_offset: i32,
    pub(super) second_chroma_qp_index_offset: i32,
    pub(super) alpha_offset_div2: i32,
    pub(super) beta_offset_div2: i32,
    pub(super) mb_width: usize,
    pub(super) mb_height: usize,
    pub(super) mb_types: Option<&'a [u8]>,
    pub(super) mb_cbp: Option<&'a [u8]>,
    pub(super) mb_slice_first_mb: Option<&'a [u32]>,
    pub(super) mv_l0_x: Option<&'a [i16]>,
    pub(super) mv_l0_y: Option<&'a [i16]>,
    pub(super) ref_idx_l0: Option<&'a [i8]>,
    pub(super) mv_l1_x: Option<&'a [i16]>,
    pub(super) mv_l1_y: Option<&'a [i16]>,
    pub(super) ref_idx_l1: Option<&'a [i8]>,
    pub(super) cbf_luma: Option<&'a [bool]>,
    pub(super) mv_l0_x_4x4: Option<&'a [i16]>,
    pub(super) mv_l0_y_4x4: Option<&'a [i16]>,
    pub(super) ref_idx_l0_4x4: Option<&'a [i8]>,
    pub(super) mv_l1_x_4x4: Option<&'a [i16]>,
    pub(super) mv_l1_y_4x4: Option<&'a [i16]>,
    pub(super) ref_idx_l1_4x4: Option<&'a [i8]>,
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
        disable_deblocking_filter_idc,
        chroma_qp_index_offset,
        second_chroma_qp_index_offset,
        alpha_offset_div2,
        beta_offset_div2,
        mb_width,
        mb_height,
        mb_types,
        mb_cbp,
        mb_slice_first_mb,
        mv_l0_x,
        mv_l0_y,
        ref_idx_l0,
        mv_l1_x,
        mv_l1_y,
        ref_idx_l1,
        cbf_luma,
        mv_l0_x_4x4,
        mv_l0_y_4x4,
        ref_idx_l0_4x4,
        mv_l1_x_4x4,
        mv_l1_y_4x4,
        ref_idx_l1_4x4,
    } = params;
    if width == 0 || height == 0 {
        return;
    }
    let luma_alpha_idx = alpha_index(slice_qp, alpha_offset_div2);
    let luma_alpha = alpha_threshold(slice_qp, alpha_offset_div2);
    let luma_beta = beta_threshold(slice_qp, beta_offset_div2);

    let chroma_qp_u = chroma_qp_from_luma_with_offset(slice_qp, chroma_qp_index_offset);
    let chroma_qp_v = chroma_qp_from_luma_with_offset(slice_qp, second_chroma_qp_index_offset);
    let chroma_alpha_idx_u = alpha_index(chroma_qp_u, alpha_offset_div2);
    let chroma_alpha_u = alpha_threshold(chroma_qp_u, alpha_offset_div2);
    let chroma_beta_u = beta_threshold(chroma_qp_u, beta_offset_div2);
    let chroma_alpha_idx_v = alpha_index(chroma_qp_v, alpha_offset_div2);
    let chroma_alpha_v = alpha_threshold(chroma_qp_v, alpha_offset_div2);
    let chroma_beta_v = beta_threshold(chroma_qp_v, beta_offset_div2);

    let mb_ctx = mb_types.and_then(|types| {
        mb_cbp.and_then(|cbp| {
            let need = mb_width.saturating_mul(mb_height);
            if need == 0 || types.len() < need || cbp.len() < need {
                None
            } else {
                let mb_slice_first_mb =
                    mb_slice_first_mb.filter(|slice_map| slice_map.len() >= need);
                Some(DeblockMbContext {
                    mb_width,
                    mb_height,
                    mb_types: types,
                    mb_cbp: cbp,
                    mb_slice_first_mb,
                    disable_cross_slice_boundary_filter: disable_deblocking_filter_idc == 2,
                    mv_l0_x,
                    mv_l0_y,
                    ref_idx_l0,
                    mv_l1_x,
                    mv_l1_y,
                    ref_idx_l1,
                    cbf_luma,
                    mv_l0_x_4x4,
                    mv_l0_y_4x4,
                    ref_idx_l0_4x4,
                    mv_l1_x_4x4,
                    mv_l1_y_4x4,
                    ref_idx_l1_4x4,
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
        luma_alpha_idx,
        luma_alpha,
        luma_beta,
        mb_ctx.as_ref(),
    );
    apply_adaptive_deblock_plane(
        u,
        stride_c,
        width / 2,
        height / 2,
        2,
        8,
        chroma_alpha_idx_u,
        chroma_alpha_u,
        chroma_beta_u,
        mb_ctx.as_ref(),
    );
    apply_adaptive_deblock_plane(
        v,
        stride_c,
        width / 2,
        height / 2,
        2,
        8,
        chroma_alpha_idx_v,
        chroma_alpha_v,
        chroma_beta_v,
        mb_ctx.as_ref(),
    );
}

#[derive(Clone, Copy)]
struct DeblockMbContext<'a> {
    mb_width: usize,
    mb_height: usize,
    mb_types: &'a [u8],
    mb_cbp: &'a [u8],
    mb_slice_first_mb: Option<&'a [u32]>,
    disable_cross_slice_boundary_filter: bool,
    mv_l0_x: Option<&'a [i16]>,
    mv_l0_y: Option<&'a [i16]>,
    ref_idx_l0: Option<&'a [i8]>,
    mv_l1_x: Option<&'a [i16]>,
    mv_l1_y: Option<&'a [i16]>,
    ref_idx_l1: Option<&'a [i8]>,
    cbf_luma: Option<&'a [bool]>,
    mv_l0_x_4x4: Option<&'a [i16]>,
    mv_l0_y_4x4: Option<&'a [i16]>,
    ref_idx_l0_4x4: Option<&'a [i8]>,
    mv_l1_x_4x4: Option<&'a [i16]>,
    mv_l1_y_4x4: Option<&'a [i16]>,
    ref_idx_l1_4x4: Option<&'a [i8]>,
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
    let strong_luma = boundary_step == 4;

    // 垂直边界
    let mut x = boundary_step;
    while x < width.saturating_sub(1) {
        if x >= 2 {
            for y in 0..height {
                let p1 = y * stride + (x - 2);
                let p0 = y * stride + (x - 1);
                let q0 = y * stride + x;
                let q1 = y * stride + (x + 1);
                let p2 = if x >= 3 {
                    Some(y * stride + (x - 3))
                } else {
                    None
                };
                let q2 = if x + 2 < width {
                    Some(y * stride + (x + 2))
                } else {
                    None
                };
                if p1 >= plane.len() || p0 >= plane.len() || q0 >= plane.len() || q1 >= plane.len()
                {
                    continue;
                }
                let bs = boundary_strength_vertical(x, y, mb_step, mb_ctx);
                filter_edge_with_bs(
                    plane,
                    p2,
                    p1,
                    p0,
                    q0,
                    q1,
                    q2,
                    alpha_idx,
                    alpha,
                    beta,
                    bs,
                    strong_luma,
                );
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
                let p2 = if y >= 3 {
                    Some((y - 3) * stride + x)
                } else {
                    None
                };
                let q2 = if y + 2 < height {
                    Some((y + 2) * stride + x)
                } else {
                    None
                };
                if p1 >= plane.len() || p0 >= plane.len() || q0 >= plane.len() || q1 >= plane.len()
                {
                    continue;
                }
                let bs = boundary_strength_horizontal(x, y, mb_step, mb_ctx);
                filter_edge_with_bs(
                    plane,
                    p2,
                    p1,
                    p0,
                    q0,
                    q1,
                    q2,
                    alpha_idx,
                    alpha,
                    beta,
                    bs,
                    strong_luma,
                );
            }
        }
        y += boundary_step;
    }
}

#[allow(clippy::too_many_arguments)]
fn filter_edge_with_bs(
    plane: &mut [u8],
    p2_idx: Option<usize>,
    p1_idx: usize,
    p0_idx: usize,
    q0_idx: usize,
    q1_idx: usize,
    q2_idx: Option<usize>,
    alpha_idx: usize,
    alpha: u8,
    beta: u8,
    bs: u8,
    strong_luma: bool,
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

    if bs >= 4 {
        if strong_luma {
            let alpha_half = (i32::from(alpha) >> 2) + 2;
            if (p0 - q0).abs() < alpha_half {
                let mut new_p0 = ((2 * p1 + p0 + q1 + 2) >> 2).clamp(0, 255);
                let mut new_q0 = ((2 * q1 + q0 + p1 + 2) >> 2).clamp(0, 255);
                if let Some(p2_idx) = p2_idx {
                    let p2 = i32::from(plane[p2_idx]);
                    if (p2 - p0).abs() < i32::from(beta) {
                        new_p0 = ((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3).clamp(0, 255);
                        let new_p1 = ((p2 + p1 + p0 + q0 + 2) >> 2).clamp(0, 255);
                        plane[p1_idx] = new_p1 as u8;
                    }
                }
                if let Some(q2_idx) = q2_idx {
                    let q2 = i32::from(plane[q2_idx]);
                    if (q2 - q0).abs() < i32::from(beta) {
                        new_q0 = ((q2 + 2 * q1 + 2 * q0 + 2 * p0 + p1 + 4) >> 3).clamp(0, 255);
                        let new_q1 = ((q2 + q1 + q0 + p0 + 2) >> 2).clamp(0, 255);
                        plane[q1_idx] = new_q1 as u8;
                    }
                }
                plane[p0_idx] = new_p0 as u8;
                plane[q0_idx] = new_q0 as u8;
                return;
            }
        } else {
            // 色度强滤波仅更新 p0/q0, 保持 2 像素语义.
            let new_p0 = ((2 * p1 + p0 + q1 + 2) >> 2).clamp(0, 255);
            let new_q0 = ((2 * q1 + q0 + p1 + 2) >> 2).clamp(0, 255);
            plane[p0_idx] = new_p0 as u8;
            plane[q0_idx] = new_q0 as u8;
            return;
        }
    }

    let tc0 = i32::from(tc0_threshold(alpha_idx, bs.min(3)));
    if tc0 == 0 {
        return;
    }
    let tc =
        tc0 + if (p1 - p0).abs() < i32::from(beta) {
            1
        } else {
            0
        } + if (q1 - q0).abs() < i32::from(beta) {
            1
        } else {
            0
        };
    let mut delta = ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3;
    delta = delta.clamp(-tc, tc);

    plane[p0_idx] = (p0 + delta).clamp(0, 255) as u8;
    plane[q0_idx] = (q0 - delta).clamp(0, 255) as u8;
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
        return boundary_strength_within_mb_vertical(x, y, mb_step, mb_ctx, mb_x, mb_y);
    }
    let Some(ctx) = mb_ctx else {
        return 2;
    };
    let mb_y = y / mb_step;
    let mb_x_l = (x - 1) / mb_step;
    let mb_x_r = x / mb_step;
    if mb_step == 16 {
        return boundary_strength_between_mb_vertical_4x4(ctx, x, y, mb_x_l, mb_y, mb_x_r);
    }
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
        return boundary_strength_within_mb_horizontal(x, y, mb_step, mb_ctx, mb_x, mb_y);
    }
    let Some(ctx) = mb_ctx else {
        return 2;
    };
    let mb_x = x / mb_step;
    let mb_y_t = (y - 1) / mb_step;
    let mb_y_b = y / mb_step;
    if mb_step == 16 {
        return boundary_strength_between_mb_horizontal_4x4(ctx, x, y, mb_x, mb_y_t, mb_y_b);
    }
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
    if cross_slice_boundary_is_disabled(ctx, i_a, i_b) {
        return 0;
    }
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
    motion_boundary_strength(ctx, i_a, i_b)
}

fn boundary_strength_within_mb_vertical(
    x: usize,
    y: usize,
    mb_step: usize,
    mb_ctx: Option<&DeblockMbContext<'_>>,
    mb_x: usize,
    mb_y: usize,
) -> u8 {
    boundary_strength_within_mb_common(x, y, mb_step, mb_ctx, mb_x, mb_y, true)
}

fn boundary_strength_within_mb_horizontal(
    x: usize,
    y: usize,
    mb_step: usize,
    mb_ctx: Option<&DeblockMbContext<'_>>,
    mb_x: usize,
    mb_y: usize,
) -> u8 {
    boundary_strength_within_mb_common(x, y, mb_step, mb_ctx, mb_x, mb_y, false)
}

fn boundary_strength_within_mb_common(
    x: usize,
    y: usize,
    mb_step: usize,
    mb_ctx: Option<&DeblockMbContext<'_>>,
    mb_x: usize,
    mb_y: usize,
    vertical: bool,
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
    if mb_step == 16 {
        let x4_a;
        let y4_a;
        let x4_b;
        let y4_b;
        if vertical {
            x4_a = (x - 1) / 4;
            y4_a = y / 4;
            x4_b = x / 4;
            y4_b = y / 4;
        } else {
            x4_a = x / 4;
            y4_a = (y - 1) / 4;
            x4_b = x / 4;
            y4_b = y / 4;
        }

        if luma_cbf_non_zero_across_boundary(ctx, x4_a, y4_a, x4_b, y4_b) {
            return 2;
        }
        if let Some(bs) = motion_boundary_strength_4x4(ctx, x4_a, y4_a, x4_b, y4_b) {
            return bs;
        }
        if cbp != 0 {
            return 2;
        }
        return 1;
    }
    if cbp != 0 {
        return 2;
    }
    1
}

fn motion_boundary_strength(ctx: &DeblockMbContext<'_>, idx_a: usize, idx_b: usize) -> u8 {
    combine_motion_list_mismatch(
        list_motion_mismatch(ctx.ref_idx_l0, ctx.mv_l0_x, ctx.mv_l0_y, idx_a, idx_b),
        list_motion_mismatch(ctx.ref_idx_l1, ctx.mv_l1_x, ctx.mv_l1_y, idx_a, idx_b),
    )
    .unwrap_or(1)
}

fn boundary_strength_between_mb_vertical_4x4(
    ctx: &DeblockMbContext<'_>,
    x: usize,
    y: usize,
    mb_x_l: usize,
    mb_y: usize,
    mb_x_r: usize,
) -> u8 {
    let idx_a = mb_index(ctx.mb_width, ctx.mb_height, mb_x_l, mb_y);
    let idx_b = mb_index(ctx.mb_width, ctx.mb_height, mb_x_r, mb_y);
    let (Some(i_a), Some(i_b)) = (idx_a, idx_b) else {
        return 2;
    };
    if cross_slice_boundary_is_disabled(ctx, i_a, i_b) {
        return 0;
    }
    let ty_a = *ctx.mb_types.get(i_a).unwrap_or(&255);
    let ty_b = *ctx.mb_types.get(i_b).unwrap_or(&255);
    if is_intra_mb(ty_a) || is_intra_mb(ty_b) {
        return 4;
    }

    let x4_a = x / 4 - 1;
    let x4_b = x / 4;
    let y4 = y / 4;
    if luma_cbf_non_zero_across_boundary(ctx, x4_a, y4, x4_b, y4) {
        return 2;
    }
    if let Some(bs) = motion_boundary_strength_4x4(ctx, x4_a, y4, x4_b, y4) {
        return bs;
    }

    let cbp_a = *ctx.mb_cbp.get(i_a).unwrap_or(&0);
    let cbp_b = *ctx.mb_cbp.get(i_b).unwrap_or(&0);
    if cbp_a != 0 || cbp_b != 0 {
        return 2;
    }
    motion_boundary_strength(ctx, i_a, i_b)
}

fn boundary_strength_between_mb_horizontal_4x4(
    ctx: &DeblockMbContext<'_>,
    x: usize,
    y: usize,
    mb_x: usize,
    mb_y_t: usize,
    mb_y_b: usize,
) -> u8 {
    let idx_a = mb_index(ctx.mb_width, ctx.mb_height, mb_x, mb_y_t);
    let idx_b = mb_index(ctx.mb_width, ctx.mb_height, mb_x, mb_y_b);
    let (Some(i_a), Some(i_b)) = (idx_a, idx_b) else {
        return 2;
    };
    if cross_slice_boundary_is_disabled(ctx, i_a, i_b) {
        return 0;
    }
    let ty_a = *ctx.mb_types.get(i_a).unwrap_or(&255);
    let ty_b = *ctx.mb_types.get(i_b).unwrap_or(&255);
    if is_intra_mb(ty_a) || is_intra_mb(ty_b) {
        return 4;
    }

    let x4 = x / 4;
    let y4_a = y / 4 - 1;
    let y4_b = y / 4;
    if luma_cbf_non_zero_across_boundary(ctx, x4, y4_a, x4, y4_b) {
        return 2;
    }
    if let Some(bs) = motion_boundary_strength_4x4(ctx, x4, y4_a, x4, y4_b) {
        return bs;
    }

    let cbp_a = *ctx.mb_cbp.get(i_a).unwrap_or(&0);
    let cbp_b = *ctx.mb_cbp.get(i_b).unwrap_or(&0);
    if cbp_a != 0 || cbp_b != 0 {
        return 2;
    }
    motion_boundary_strength(ctx, i_a, i_b)
}

fn luma4x4_index(mb_width: usize, mb_height: usize, x4: usize, y4: usize) -> Option<usize> {
    let stride = mb_width.checked_mul(4)?;
    let h4 = mb_height.checked_mul(4)?;
    if stride == 0 || x4 >= stride || y4 >= h4 {
        return None;
    }
    y4.checked_mul(stride)?.checked_add(x4)
}

fn cross_slice_boundary_is_disabled(
    ctx: &DeblockMbContext<'_>,
    idx_a: usize,
    idx_b: usize,
) -> bool {
    if !ctx.disable_cross_slice_boundary_filter {
        return false;
    }
    let Some(slice_map) = ctx.mb_slice_first_mb else {
        return false;
    };
    let sid_a = slice_map.get(idx_a).copied().unwrap_or(u32::MAX);
    let sid_b = slice_map.get(idx_b).copied().unwrap_or(u32::MAX);
    sid_a != u32::MAX && sid_b != u32::MAX && sid_a != sid_b
}

fn list_motion_mismatch(
    ref_idx: Option<&[i8]>,
    mv_x: Option<&[i16]>,
    mv_y: Option<&[i16]>,
    idx_a: usize,
    idx_b: usize,
) -> Option<bool> {
    let (Some(ref_idx), Some(mv_x), Some(mv_y)) = (ref_idx, mv_x, mv_y) else {
        return None;
    };
    if idx_a >= ref_idx.len() || idx_b >= ref_idx.len() {
        return None;
    }
    if idx_a >= mv_x.len() || idx_b >= mv_x.len() || idx_a >= mv_y.len() || idx_b >= mv_y.len() {
        return None;
    }
    if ref_idx[idx_a] != ref_idx[idx_b] {
        return Some(true);
    }
    if ref_idx[idx_a] < 0 {
        return Some(false);
    }
    let mv_dx = (i32::from(mv_x[idx_a]) - i32::from(mv_x[idx_b])).abs();
    let mv_dy = (i32::from(mv_y[idx_a]) - i32::from(mv_y[idx_b])).abs();
    Some(mv_dx >= 4 || mv_dy >= 4)
}

fn combine_motion_list_mismatch(list0: Option<bool>, list1: Option<bool>) -> Option<u8> {
    if list0.unwrap_or(false) || list1.unwrap_or(false) {
        return Some(1);
    }
    if list0.is_some() || list1.is_some() {
        return Some(0);
    }
    None
}

fn luma_cbf_non_zero_across_boundary(
    ctx: &DeblockMbContext<'_>,
    x4_a: usize,
    y4_a: usize,
    x4_b: usize,
    y4_b: usize,
) -> bool {
    let Some(cbf) = ctx.cbf_luma else {
        return false;
    };
    let idx_a = luma4x4_index(ctx.mb_width, ctx.mb_height, x4_a, y4_a);
    let idx_b = luma4x4_index(ctx.mb_width, ctx.mb_height, x4_b, y4_b);
    let Some(i_a) = idx_a else {
        return false;
    };
    let Some(i_b) = idx_b else {
        return false;
    };
    cbf.get(i_a).copied().unwrap_or(false) || cbf.get(i_b).copied().unwrap_or(false)
}

fn motion_boundary_strength_4x4(
    ctx: &DeblockMbContext<'_>,
    x4_a: usize,
    y4_a: usize,
    x4_b: usize,
    y4_b: usize,
) -> Option<u8> {
    let idx_a = luma4x4_index(ctx.mb_width, ctx.mb_height, x4_a, y4_a)?;
    let idx_b = luma4x4_index(ctx.mb_width, ctx.mb_height, x4_b, y4_b)?;
    combine_motion_list_mismatch(
        list_motion_mismatch(
            ctx.ref_idx_l0_4x4,
            ctx.mv_l0_x_4x4,
            ctx.mv_l0_y_4x4,
            idx_a,
            idx_b,
        ),
        list_motion_mismatch(
            ctx.ref_idx_l1_4x4,
            ctx.mv_l1_x_4x4,
            ctx.mv_l1_y_4x4,
            idx_a,
            idx_b,
        ),
    )
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
        apply_deblock_yuv420_with_slice_params, beta_threshold, boundary_strength_horizontal,
        boundary_strength_vertical, filter_edge_with_bs,
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
                disable_deblocking_filter_idc: 0,
                chroma_qp_index_offset: 0,
                second_chroma_qp_index_offset: 0,
                alpha_offset_div2: 0,
                beta_offset_div2: 0,
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
                mb_slice_first_mb: None,
                mv_l0_x: None,
                mv_l0_y: None,
                ref_idx_l0: None,
                mv_l1_x: None,
                mv_l1_y: None,
                ref_idx_l1: None,
                cbf_luma: None,
                mv_l0_x_4x4: None,
                mv_l0_y_4x4: None,
                ref_idx_l0_4x4: None,
                mv_l1_x_4x4: None,
                mv_l1_y_4x4: None,
                ref_idx_l1_4x4: None,
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
                disable_deblocking_filter_idc: 0,
                chroma_qp_index_offset: 0,
                second_chroma_qp_index_offset: 0,
                alpha_offset_div2: 3,
                beta_offset_div2: 0,
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
                mb_slice_first_mb: None,
                mv_l0_x: None,
                mv_l0_y: None,
                ref_idx_l0: None,
                mv_l1_x: None,
                mv_l1_y: None,
                ref_idx_l1: None,
                cbf_luma: None,
                mv_l0_x_4x4: None,
                mv_l0_y_4x4: None,
                ref_idx_l0_4x4: None,
                mv_l1_x_4x4: None,
                mv_l1_y_4x4: None,
                ref_idx_l1_4x4: None,
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
                disable_deblocking_filter_idc: 0,
                chroma_qp_index_offset: 0,
                second_chroma_qp_index_offset: 0,
                alpha_offset_div2: 0,
                beta_offset_div2: 0,
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
                mb_slice_first_mb: None,
                mv_l0_x: None,
                mv_l0_y: None,
                ref_idx_l0: None,
                mv_l1_x: None,
                mv_l1_y: None,
                ref_idx_l1: None,
                cbf_luma: None,
                mv_l0_x_4x4: None,
                mv_l0_y_4x4: None,
                ref_idx_l0_4x4: None,
                mv_l1_x_4x4: None,
                mv_l1_y_4x4: None,
                ref_idx_l1_4x4: None,
            },
        );

        assert_ne!(y[8], 48, "亮度平面应发生平滑");
        assert_ne!(u[4], 72, "U 平面应发生平滑");
        assert_ne!(v[4], 104, "V 平面应发生平滑");
    }

    #[test]
    fn test_apply_deblock_uses_chroma_qp_mapping_offsets() {
        let width = 16usize;
        let height = 16usize;
        let stride_y = width;
        let stride_c = width / 2;
        let mut y = vec![32u8; stride_y * height];
        let mut u = vec![60u8; stride_c * (height / 2)];
        let mut v = vec![80u8; stride_c * (height / 2)];

        for row in 0..height {
            let base = row * stride_y;
            y[base + 6] = 40;
            y[base + 7] = 40;
            y[base + 8] = 42;
            y[base + 9] = 42;
        }
        for row in 0..(height / 2) {
            let base = row * stride_c;
            u[base + 2] = 60;
            u[base + 3] = 60;
            u[base + 4] = 62;
            u[base + 5] = 62;
            v[base + 2] = 80;
            v[base + 3] = 80;
            v[base + 4] = 82;
            v[base + 5] = 82;
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
                slice_qp: 4,
                disable_deblocking_filter_idc: 0,
                chroma_qp_index_offset: 12,
                second_chroma_qp_index_offset: 12,
                alpha_offset_div2: 0,
                beta_offset_div2: 0,
                mb_width: 0,
                mb_height: 0,
                mb_types: None,
                mb_cbp: None,
                mb_slice_first_mb: None,
                mv_l0_x: None,
                mv_l0_y: None,
                ref_idx_l0: None,
                mv_l1_x: None,
                mv_l1_y: None,
                ref_idx_l1: None,
                cbf_luma: None,
                mv_l0_x_4x4: None,
                mv_l0_y_4x4: None,
                ref_idx_l0_4x4: None,
                mv_l1_x_4x4: None,
                mv_l1_y_4x4: None,
                ref_idx_l1_4x4: None,
            },
        );

        assert_eq!(y[8], 42, "低 luma QP 时亮度阈值为 0, 不应触发去块");
        assert_ne!(u[4], 62, "色度 U 应使用 chroma_qp 映射后阈值触发去块");
        assert_ne!(v[4], 82, "色度 V 应使用 chroma_qp 映射后阈值触发去块");
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
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(16, 0, 16, Some(&ctx));
        assert_eq!(bs, 4, "宏块边界任一侧为帧内宏块时应走强滤波");
    }

    #[test]
    fn test_boundary_strength_vertical_inter_mb_motion_aligned_can_be_zero() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let mv_l0_x = [8i16, 9i16];
        let mv_l0_y = [4i16, 5i16];
        let ref_idx_l0 = [0i8, 0i8];
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(16, 0, 16, Some(&ctx));
        assert_eq!(bs, 0, "同参考且运动向量接近时应允许跳过滤波");
    }

    #[test]
    fn test_boundary_strength_vertical_inter_mb_ref_mismatch_is_non_zero() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let mv_l0_x = [8i16, 8i16];
        let mv_l0_y = [4i16, 4i16];
        let ref_idx_l0 = [0i8, 1i8];
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(16, 0, 16, Some(&ctx));
        assert_eq!(bs, 1, "跨宏块参考索引不一致时应保留弱滤波");
    }

    #[test]
    fn test_boundary_strength_vertical_idc2_cross_slice_boundary_is_zero() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let mv_l0_x = [0i16, 0i16];
        let mv_l0_y = [0i16, 0i16];
        let ref_idx_l0 = [0i8, 1i8];
        let mb_slice_first_mb = [0u32, 1u32];
        let ctx_idc0 = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: Some(&mb_slice_first_mb),
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let ctx_idc2 = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: Some(&mb_slice_first_mb),
            disable_cross_slice_boundary_filter: true,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs_idc0 = boundary_strength_vertical(16, 0, 16, Some(&ctx_idc0));
        let bs_idc2 = boundary_strength_vertical(16, 0, 16, Some(&ctx_idc2));
        assert_eq!(bs_idc0, 1, "idc!=2 时跨 slice 边界应按常规 BS 规则计算");
        assert_eq!(bs_idc2, 0, "idc=2 时跨 slice 宏块边界应禁止去块");
    }

    #[test]
    fn test_boundary_strength_vertical_mb_boundary_follows_4x4_rows() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let cbf_luma = [false; 32];
        let mv_l0_x = [0i16; 2];
        let mv_l0_y = [0i16; 2];
        let ref_idx_l0 = [0i8; 2];
        let mv_l0_x_4x4 = [0i16; 32];
        let mv_l0_y_4x4 = [0i16; 32];
        let mut ref_idx_l0_4x4 = [0i8; 32];
        // y4=0 这一行跨边界参考不一致, y4=2 保持一致.
        ref_idx_l0_4x4[3] = 0;
        ref_idx_l0_4x4[4] = 1;
        ref_idx_l0_4x4[19] = 0;
        ref_idx_l0_4x4[20] = 0;
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };

        let bs_top_row = boundary_strength_vertical(16, 2, 16, Some(&ctx));
        let bs_mid_row = boundary_strength_vertical(16, 10, 16, Some(&ctx));
        assert_eq!(bs_top_row, 1, "跨宏块边界应按对应 4x4 行判定参考差异");
        assert_eq!(bs_mid_row, 0, "未命中差异的 4x4 行应允许 bs=0");
    }

    #[test]
    fn test_boundary_strength_vertical_mb_boundary_4x4_cbf_non_zero_is_two() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let mut cbf_luma = [false; 32];
        cbf_luma[4] = true;
        let mv_l0_x = [0i16; 2];
        let mv_l0_y = [0i16; 2];
        let ref_idx_l0 = [0i8; 2];
        let mv_l0_x_4x4 = [0i16; 32];
        let mv_l0_y_4x4 = [0i16; 32];
        let ref_idx_l0_4x4 = [0i8; 32];
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };

        let bs = boundary_strength_vertical(16, 2, 16, Some(&ctx));
        assert_eq!(bs, 2, "跨宏块边界任一侧 4x4 CBF 非零时应返回 bs=2");
    }

    #[test]
    fn test_boundary_strength_horizontal_mb_boundary_follows_4x4_columns() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let cbf_luma = [false; 32];
        let mv_l0_x = [0i16; 2];
        let mv_l0_y = [0i16; 2];
        let ref_idx_l0 = [0i8; 2];
        let mv_l0_x_4x4 = [0i16; 32];
        let mv_l0_y_4x4 = [0i16; 32];
        let mut ref_idx_l0_4x4 = [0i8; 32];
        // x4=0 这一列跨边界参考不一致, x4=2 保持一致.
        ref_idx_l0_4x4[12] = 0;
        ref_idx_l0_4x4[16] = 1;
        ref_idx_l0_4x4[14] = 0;
        ref_idx_l0_4x4[18] = 0;
        let ctx = DeblockMbContext {
            mb_width: 1,
            mb_height: 2,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };

        let bs_left_col = boundary_strength_horizontal(2, 16, 16, Some(&ctx));
        let bs_right_col = boundary_strength_horizontal(10, 16, 16, Some(&ctx));
        assert_eq!(bs_left_col, 1, "跨宏块水平边界应按对应 4x4 列判定参考差异");
        assert_eq!(bs_right_col, 0, "未命中差异的 4x4 列应允许 bs=0");
    }

    #[test]
    fn test_boundary_strength_vertical_within_mb_cbf_non_zero_is_two() {
        let mb_types = [255u8];
        let mb_cbp = [0u8];
        let mut cbf_luma = vec![false; 16];
        cbf_luma[1] = true;
        let mv_l0_x_4x4 = [0i16; 16];
        let mv_l0_y_4x4 = [0i16; 16];
        let ref_idx_l0_4x4 = [0i8; 16];
        let ctx = DeblockMbContext {
            mb_width: 1,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(4, 2, 16, Some(&ctx));
        assert_eq!(bs, 2, "4x4 内部边界任一侧 cbf!=0 时应返回 bs=2");
    }

    #[test]
    fn test_boundary_strength_vertical_within_mb_ref_or_mv_mismatch_is_one() {
        let mb_types = [255u8];
        let mb_cbp = [0u8];
        let cbf_luma = [false; 16];

        let mv_l0_x_4x4_ref_mismatch = [0i16; 16];
        let mv_l0_y_4x4_ref_mismatch = [0i16; 16];
        let mut ref_idx_l0_4x4_ref_mismatch = [0i8; 16];
        ref_idx_l0_4x4_ref_mismatch[1] = 1;
        let ctx_ref = DeblockMbContext {
            mb_width: 1,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4_ref_mismatch),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4_ref_mismatch),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4_ref_mismatch),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs_ref = boundary_strength_vertical(4, 2, 16, Some(&ctx_ref));
        assert_eq!(bs_ref, 1, "4x4 内部边界 ref_idx 不同应返回 bs=1");

        let mut mv_l0_x_4x4_mv_mismatch = [0i16; 16];
        mv_l0_x_4x4_mv_mismatch[1] = 4;
        let mv_l0_y_4x4_mv_mismatch = [0i16; 16];
        let ref_idx_l0_4x4_mv_mismatch = [0i8; 16];
        let ctx_mv = DeblockMbContext {
            mb_width: 1,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4_mv_mismatch),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4_mv_mismatch),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4_mv_mismatch),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs_mv = boundary_strength_vertical(4, 2, 16, Some(&ctx_mv));
        assert_eq!(bs_mv, 1, "4x4 内部边界 MV 差>=4 时应返回 bs=1");
    }

    #[test]
    fn test_boundary_strength_vertical_inter_mb_list1_ref_mismatch_is_non_zero() {
        let mb_types = [255u8, 255u8];
        let mb_cbp = [0u8, 0u8];
        let mv_l0_x = [8i16, 8i16];
        let mv_l0_y = [4i16, 4i16];
        let ref_idx_l0 = [0i8, 0i8];
        let mv_l1_x = [6i16, 6i16];
        let mv_l1_y = [2i16, 2i16];
        let ref_idx_l1 = [0i8, 1i8];
        let ctx = DeblockMbContext {
            mb_width: 2,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: Some(&mv_l0_x),
            mv_l0_y: Some(&mv_l0_y),
            ref_idx_l0: Some(&ref_idx_l0),
            mv_l1_x: Some(&mv_l1_x),
            mv_l1_y: Some(&mv_l1_y),
            ref_idx_l1: Some(&ref_idx_l1),
            cbf_luma: None,
            mv_l0_x_4x4: None,
            mv_l0_y_4x4: None,
            ref_idx_l0_4x4: None,
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(16, 0, 16, Some(&ctx));
        assert_eq!(bs, 1, "跨宏块 list1 参考索引不一致时应返回 bs=1");
    }

    #[test]
    fn test_boundary_strength_vertical_within_mb_list1_mv_mismatch_is_one() {
        let mb_types = [255u8];
        let mb_cbp = [0u8];
        let cbf_luma = [false; 16];
        let mv_l0_x_4x4 = [0i16; 16];
        let mv_l0_y_4x4 = [0i16; 16];
        let ref_idx_l0_4x4 = [0i8; 16];
        let mut mv_l1_x_4x4 = [0i16; 16];
        let mv_l1_y_4x4 = [0i16; 16];
        let ref_idx_l1_4x4 = [0i8; 16];
        mv_l1_x_4x4[1] = 4;
        let ctx = DeblockMbContext {
            mb_width: 1,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: Some(&mv_l1_x_4x4),
            mv_l1_y_4x4: Some(&mv_l1_y_4x4),
            ref_idx_l1_4x4: Some(&ref_idx_l1_4x4),
        };
        let bs = boundary_strength_vertical(4, 2, 16, Some(&ctx));
        assert_eq!(bs, 1, "4x4 内部边界 list1 MV 差>=4 时应返回 bs=1");
    }

    #[test]
    fn test_boundary_strength_vertical_within_mb_aligned_motion_is_zero() {
        let mb_types = [255u8];
        let mb_cbp = [0u8];
        let cbf_luma = [false; 16];
        let mv_l0_x_4x4 = [0i16; 16];
        let mv_l0_y_4x4 = [0i16; 16];
        let ref_idx_l0_4x4 = [0i8; 16];
        let ctx = DeblockMbContext {
            mb_width: 1,
            mb_height: 1,
            mb_types: &mb_types,
            mb_cbp: &mb_cbp,
            mb_slice_first_mb: None,
            disable_cross_slice_boundary_filter: false,
            mv_l0_x: None,
            mv_l0_y: None,
            ref_idx_l0: None,
            mv_l1_x: None,
            mv_l1_y: None,
            ref_idx_l1: None,
            cbf_luma: Some(&cbf_luma),
            mv_l0_x_4x4: Some(&mv_l0_x_4x4),
            mv_l0_y_4x4: Some(&mv_l0_y_4x4),
            ref_idx_l0_4x4: Some(&ref_idx_l0_4x4),
            mv_l1_x_4x4: None,
            mv_l1_y_4x4: None,
            ref_idx_l1_4x4: None,
        };
        let bs = boundary_strength_vertical(4, 2, 16, Some(&ctx));
        assert_eq!(bs, 0, "4x4 内部边界同参考且 MV 接近时应返回 bs=0");
    }

    #[test]
    fn test_filter_edge_with_bs_uses_tc0_strength() {
        let mut weak = vec![40u8, 40, 48, 48];
        let mut strong = weak.clone();

        filter_edge_with_bs(&mut weak, None, 0, 1, 2, 3, None, 26, 15, 4, 1, true);
        filter_edge_with_bs(&mut strong, None, 0, 1, 2, 3, None, 26, 15, 4, 3, true);

        assert_eq!(weak[1], 40, "弱滤波在 tc0=0 时应保持原样");
        assert_eq!(weak[2], 48, "弱滤波在 tc0=0 时应保持原样");
        assert!(strong[1] > weak[1], "更高 bs 应带来更强的左侧平滑");
        assert!(strong[2] < weak[2], "更高 bs 应带来更强的右侧平滑");
    }

    #[test]
    fn test_filter_edge_with_bs_strong_luma_uses_p2_q2_and_updates_four_pixels() {
        let mut plane = vec![40u8, 40, 45, 47, 48, 48];
        filter_edge_with_bs(
            &mut plane,
            Some(0),
            1,
            2,
            3,
            4,
            Some(5),
            30,
            20,
            10,
            4,
            true,
        );
        assert!(plane[1] != 40, "亮度强滤波应更新 p1");
        assert!(plane[2] != 45, "亮度强滤波应更新 p0");
        assert!(plane[3] != 47, "亮度强滤波应更新 q0");
        assert!(plane[4] != 48, "亮度强滤波应更新 q1");
    }

    #[test]
    fn test_filter_edge_with_bs_strong_chroma_updates_only_two_pixels() {
        let mut plane = vec![40u8, 40, 48, 48];
        filter_edge_with_bs(&mut plane, None, 0, 1, 2, 3, None, 30, 20, 10, 4, false);
        assert_eq!(plane[0], 40, "色度强滤波不应更新 p1");
        assert!(plane[1] != 40, "色度强滤波应更新 p0");
        assert!(plane[2] != 48, "色度强滤波应更新 q0");
        assert_eq!(plane[3], 48, "色度强滤波不应更新 q1");
    }
}
