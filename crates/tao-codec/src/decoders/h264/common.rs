use super::*;

// ============================================================
// 工具函数
// ============================================================

pub(super) fn median3(a: i32, b: i32, c: i32) -> i32 {
    let mut vals = [a, b, c];
    vals.sort_unstable();
    vals[1]
}

pub(super) fn floor_div(v: i32, d: i32) -> i32 {
    let mut q = v / d;
    let r = v % d;
    if r != 0 && ((r > 0) != (d > 0)) {
        q -= 1;
    }
    q
}

pub(super) fn mod_floor(v: i32, d: i32) -> i32 {
    let r = v % d;
    if r < 0 { r + d } else { r }
}

pub(super) fn p_l0_weight(weights: &[PredWeightL0], ref_idx: u32) -> Option<&PredWeightL0> {
    usize::try_from(ref_idx)
        .ok()
        .and_then(|idx| weights.get(idx))
}

pub(super) fn p_l1_weight(weights: &[PredWeightL0], ref_idx: u32) -> Option<&PredWeightL0> {
    usize::try_from(ref_idx)
        .ok()
        .and_then(|idx| weights.get(idx))
}

pub(super) fn select_ref_planes(ref_list: &[RefPlanes], ref_idx: i8) -> Option<&RefPlanes> {
    if ref_idx < 0 {
        return None;
    }
    ref_list.get(ref_idx as usize)
}

pub(super) fn apply_weighted_sample(sample: u8, weight: i32, offset: i32, log2_denom: u8) -> u8 {
    let shift = usize::from(log2_denom.min(31));
    let round = if shift > 0 { 1i32 << (shift - 1) } else { 0 };
    let scaled = (sample as i32) * weight;
    let shifted = if shift > 0 {
        (scaled + round) >> shift
    } else {
        scaled
    };
    (shifted + offset).clamp(0, 255) as u8
}

pub(super) fn h264_round_avg_u8(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
}

pub(super) fn sample_clamped(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> u8 {
    let max_x = src_w.saturating_sub(1) as i32;
    let max_y = src_h.saturating_sub(1) as i32;
    let sx = x.clamp(0, max_x) as usize;
    let sy = y.clamp(0, max_y) as usize;
    let idx = sy * stride + sx;
    src.get(idx).copied().unwrap_or(0)
}

pub(super) fn h264_luma_6tap_filter_raw(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
    horizontal: bool,
) -> i32 {
    let get = |off: i32| -> i32 {
        if horizontal {
            i32::from(sample_clamped(src, stride, src_w, src_h, x + off, y))
        } else {
            i32::from(sample_clamped(src, stride, src_w, src_h, x, y + off))
        }
    };
    get(-2) - 5 * get(-1) + 20 * get(0) + 20 * get(1) - 5 * get(2) + get(3)
}

pub(super) fn h264_luma_6tap_round(v: i32) -> i32 {
    ((v + 16) >> 5).clamp(0, 255)
}

pub(super) fn sample_h264_luma_half_h(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    h264_luma_6tap_round(h264_luma_6tap_filter_raw(
        src, stride, src_w, src_h, x, y, true,
    ))
}

pub(super) fn sample_h264_luma_half_v(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    h264_luma_6tap_round(h264_luma_6tap_filter_raw(
        src, stride, src_w, src_h, x, y, false,
    ))
}

pub(super) fn sample_h264_luma_half_hv(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    let h_row =
        |yy: i32| -> i32 { h264_luma_6tap_filter_raw(src, stride, src_w, src_h, x, yy, true) };
    let val = h_row(y - 2) - 5 * h_row(y - 1) + 20 * h_row(y) + 20 * h_row(y + 1)
        - 5 * h_row(y + 2)
        + h_row(y + 3);
    ((val + 512) >> 10).clamp(0, 255)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sample_h264_luma_qpel(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    base_x: i32,
    base_y: i32,
    frac_x: u8,
    frac_y: u8,
) -> u8 {
    let dx = usize::from(frac_x & 3);
    let dy = usize::from(frac_y & 3);
    let f = |ox: i32, oy: i32| -> i32 {
        i32::from(sample_clamped(
            src,
            stride,
            src_w,
            src_h,
            base_x + ox,
            base_y + oy,
        ))
    };
    let h = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_h(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let v = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_v(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let hv = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_hv(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let avg = |a: i32, b: i32| -> i32 { (a + b + 1) >> 1 };

    let val = match (dx, dy) {
        (0, 0) => f(0, 0),
        (1, 0) => avg(f(0, 0), h(0, 0)),
        (2, 0) => h(0, 0),
        (3, 0) => avg(h(0, 0), f(1, 0)),
        (0, 1) => avg(f(0, 0), v(0, 0)),
        (0, 2) => v(0, 0),
        (0, 3) => avg(v(0, 0), f(0, 1)),
        (2, 2) => hv(0, 0),
        (1, 1) => avg(f(0, 0), hv(0, 0)),
        (3, 1) => avg(f(1, 0), hv(0, 0)),
        (1, 3) => avg(f(0, 1), hv(0, 0)),
        (3, 3) => avg(f(1, 1), hv(0, 0)),
        (2, 1) => avg(h(0, 0), hv(0, 0)),
        (2, 3) => avg(hv(0, 0), h(0, 1)),
        (1, 2) => avg(v(0, 0), hv(0, 0)),
        (3, 2) => avg(hv(0, 0), v(1, 0)),
        _ => f(0, 0),
    };
    val.clamp(0, 255) as u8
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sample_bilinear_clamped(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    base_x: i32,
    base_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
) -> u8 {
    if frac_base == 0 {
        return sample_clamped(src, stride, src_w, src_h, base_x, base_y);
    }
    let fx = frac_x.min(frac_base);
    let fy = frac_y.min(frac_base);
    if fx == 0 && fy == 0 {
        return sample_clamped(src, stride, src_w, src_h, base_x, base_y);
    }

    let p00 = i32::from(sample_clamped(src, stride, src_w, src_h, base_x, base_y));
    let p10 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y,
    ));
    let p01 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x,
        base_y + 1,
    ));
    let p11 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y + 1,
    ));

    let fx = i32::from(fx);
    let fy = i32::from(fy);
    let base = i32::from(frac_base);
    let wx0 = base - fx;
    let wy0 = base - fy;
    let den = base * base;
    let sum = p00 * wx0 * wy0 + p10 * fx * wy0 + p01 * wx0 * fy + p11 * fx * fy;
    ((sum + den / 2) / den).clamp(0, 255) as u8
}

/// H.264 色度 1/8 分数像素双线性采样 (4:2:0).
///
/// `frac_x/frac_y` 取值范围为 `[0, 7]`.
#[allow(clippy::too_many_arguments)]
pub(super) fn sample_h264_chroma_qpel(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    base_x: i32,
    base_y: i32,
    frac_x: u8,
    frac_y: u8,
) -> u8 {
    let fx = i32::from(frac_x & 7);
    let fy = i32::from(frac_y & 7);
    if fx == 0 && fy == 0 {
        return sample_clamped(src, stride, src_w, src_h, base_x, base_y);
    }

    let p00 = i32::from(sample_clamped(src, stride, src_w, src_h, base_x, base_y));
    let p10 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y,
    ));
    let p01 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x,
        base_y + 1,
    ));
    let p11 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y + 1,
    ));

    let wx0 = 8 - fx;
    let wy0 = 8 - fy;
    let sum = p00 * wx0 * wy0 + p10 * fx * wy0 + p01 * wx0 * fy + p11 * fx * fy;
    ((sum + 32) >> 6).clamp(0, 255) as u8
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                dst[dst_idx] = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
                dst[dst_idx] = h264_round_avg_u8(dst[dst_idx], sample);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn weighted_copy_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
    weight: i32,
    offset: i32,
    log2_denom: u8,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
                dst[dst_idx] = apply_weighted_sample(sample, weight, offset, log2_denom);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                dst[dst_idx] = if frac_base == 8 {
                    sample_h264_chroma_qpel(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                    )
                } else {
                    sample_bilinear_clamped(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                        frac_base,
                    )
                };
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = if frac_base == 8 {
                    sample_h264_chroma_qpel(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                    )
                } else {
                    sample_bilinear_clamped(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                        frac_base,
                    )
                };
                dst[dst_idx] = h264_round_avg_u8(dst[dst_idx], sample);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn weighted_copy_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
    weight: i32,
    offset: i32,
    log2_denom: u8,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = if frac_base == 8 {
                    sample_h264_chroma_qpel(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                    )
                } else {
                    sample_bilinear_clamped(
                        src,
                        src_stride,
                        src_w,
                        src_h,
                        src_x + x as i32,
                        src_y + y as i32,
                        frac_x,
                        frac_y,
                        frac_base,
                    )
                };
                dst[dst_idx] = apply_weighted_sample(sample, weight, offset, log2_denom);
            }
        }
    }
}

/// 读取无符号 Exp-Golomb
pub(super) fn read_ue(br: &mut BitReader) -> TaoResult<u32> {
    let mut zeros = 0u32;
    loop {
        let bit = br.read_bit()?;
        if bit == 1 {
            break;
        }
        zeros += 1;
        if zeros > 31 {
            return Err(TaoError::InvalidData("Exp-Golomb 前导零过多".into()));
        }
    }
    if zeros == 0 {
        return Ok(0);
    }
    let suffix = br.read_bits(zeros)?;
    Ok((1 << zeros) - 1 + suffix)
}

/// 读取有符号 Exp-Golomb
pub(super) fn read_se(br: &mut BitReader) -> TaoResult<i32> {
    let code = read_ue(br)?;
    let value = code.div_ceil(2) as i32;
    if code & 1 == 0 { Ok(-value) } else { Ok(value) }
}

/// QP 按 H.264 规则做 0..51 环绕.
pub(super) fn wrap_qp(qp: i64) -> i32 {
    let m = 52i64;
    ((qp % m + m) % m) as i32
}

/// Luma QP → Chroma QP 映射 (H.264 Table 8-15)
pub(super) fn chroma_qp_from_luma_with_offset(qp: i32, offset: i32) -> i32 {
    let qpc = (qp + offset).clamp(0, 51);
    CHROMA_QP_TABLE[qpc as usize]
}

/// 从对齐缓冲区拷贝到紧凑平面
pub(super) fn copy_plane(src: &[u8], src_stride: usize, w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; w * h];
    for y in 0..h {
        let src_off = y * src_stride;
        let dst_off = y * w;
        let copy_len = w.min(src.len().saturating_sub(src_off));
        if copy_len > 0 && dst_off + copy_len <= dst.len() {
            dst[dst_off..dst_off + copy_len].copy_from_slice(&src[src_off..src_off + copy_len]);
        }
    }
    dst
}

/// Chroma QP 映射表 (H.264 Table 8-15)
#[rustfmt::skip]
const CHROMA_QP_TABLE: [i32; 52] = [
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 29, 30,
    31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38,
    39, 39, 39, 39,
];
