use super::*;

// ============================================================
// CABAC 语法元素解码
// ============================================================

/// 解码 mb_qp_delta (一元编码)
pub(super) fn decode_qp_delta(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    prev_nz: bool,
) -> i32 {
    const MAX_QP: u32 = 51;
    let mut ctx_idx = if prev_nz { 1usize } else { 0 };
    let mut val = 0u32;

    while cabac.decode_decision(&mut ctxs[60 + ctx_idx]) == 1 {
        ctx_idx = 2 + (ctx_idx >> 1);
        val += 1;
        if val > 2 * MAX_QP {
            break;
        }
    }

    match val {
        0 => 0,
        v if v & 1 == 1 => v.div_ceil(2) as i32,
        v => -(v.div_ceil(2) as i32),
    }
}

/// 解码 I-slice 宏块类型
pub(super) fn decode_i_mb_type(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> u32 {
    decode_intra_mb_type(cabac, ctxs, 3, true, mb_types, mb_width, mb_x, mb_y)
}

/// 通用 Intra 宏块类型解码.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_intra_mb_type(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    ctx_base: usize,
    intra_slice: bool,
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> u32 {
    let mut state_base = ctx_base;
    if intra_slice {
        let ctx_inc = compute_mb_type_ctx_inc(mb_types, mb_width, mb_x, mb_y);
        let bin0 = cabac.decode_decision(&mut ctxs[state_base + ctx_inc]);
        if bin0 == 0 {
            return 0;
        }
        state_base += 2;
    } else if cabac.decode_decision(&mut ctxs[state_base]) == 0 {
        return 0;
    }

    if cabac.decode_terminate() == 1 {
        return 25;
    }

    decode_i_16x16_suffix_with_base(cabac, ctxs, state_base, intra_slice)
}

/// 计算 mb_type 前缀的上下文增量
pub(super) fn compute_mb_type_ctx_inc(
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> usize {
    let left_not_i4x4 = if mb_x > 0 {
        mb_types[mb_y * mb_width + mb_x - 1] != 0
    } else {
        false
    };
    let top_not_i4x4 = if mb_y > 0 {
        mb_types[(mb_y - 1) * mb_width + mb_x] != 0
    } else {
        false
    };
    left_not_i4x4 as usize + top_not_i4x4 as usize
}

/// 按上下文基址解码 I_16x16 后缀.
pub(super) fn decode_i_16x16_suffix_with_base(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    state_base: usize,
    intra_slice: bool,
) -> u32 {
    let intra = usize::from(intra_slice);
    let cbp_luma = cabac.decode_decision(&mut ctxs[state_base + 1]);
    let cbp_c0 = cabac.decode_decision(&mut ctxs[state_base + 2]);
    let cbp_chroma = if cbp_c0 == 0 {
        0
    } else {
        let cbp_c1 = cabac.decode_decision(&mut ctxs[state_base + 2 + intra]);
        1 + cbp_c1
    };
    let pm0 = cabac.decode_decision(&mut ctxs[state_base + 3 + intra]);
    let pm1 = cabac.decode_decision(&mut ctxs[state_base + 3 + intra * 2]);
    let pred_mode = pm0 * 2 + pm1;
    1 + pred_mode + 4 * cbp_chroma + 12 * cbp_luma
}
