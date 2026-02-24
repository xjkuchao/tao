use super::*;

impl H264Decoder {
    fn weighted_bi_sample(
        p0: i32,
        p1: i32,
        w0: i32,
        w1: i32,
        o0: i32,
        o1: i32,
        log2_denom: u8,
    ) -> u8 {
        let shift = i32::from(log2_denom) + 1;
        let offset_raw = (o0 + o1 + 1) >> 1;
        let offset = ((offset_raw + 1) | 1) << i32::from(log2_denom);
        ((w0 * p0 + w1 * p1 + offset) >> shift).clamp(0, 255) as u8
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_bi_explicit_weighted_block(
        &mut self,
        src_l0_y: &[u8],
        src_l0_u: &[u8],
        src_l0_v: &[u8],
        src_l1_y: &[u8],
        src_l1_u: &[u8],
        src_l1_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv0_x_qpel: i32,
        mv0_y_qpel: i32,
        mv1_x_qpel: i32,
        mv1_y_qpel: i32,
        luma_w0: i32,
        luma_o0: i32,
        luma_w1: i32,
        luma_o1: i32,
        chroma_w0: [i32; 2],
        chroma_o0: [i32; 2],
        chroma_w1: [i32; 2],
        chroma_o1: [i32; 2],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let l0_src_x = dst_x as i32 + floor_div(mv0_x_qpel, 4);
        let l0_src_y = dst_y as i32 + floor_div(mv0_y_qpel, 4);
        let l0_fx = mod_floor(mv0_x_qpel, 4) as u8;
        let l0_fy = mod_floor(mv0_y_qpel, 4) as u8;
        let l1_src_x = dst_x as i32 + floor_div(mv1_x_qpel, 4);
        let l1_src_y = dst_y as i32 + floor_div(mv1_y_qpel, 4);
        let l1_fx = mod_floor(mv1_x_qpel, 4) as u8;
        let l1_fy = mod_floor(mv1_y_qpel, 4) as u8;

        for y in 0..h {
            for x in 0..w {
                let px0 = sample_h264_luma_qpel(
                    src_l0_y,
                    self.stride_y,
                    self.stride_y,
                    self.mb_height * 16,
                    l0_src_x + x as i32,
                    l0_src_y + y as i32,
                    l0_fx,
                    l0_fy,
                ) as i32;
                let px1 = sample_h264_luma_qpel(
                    src_l1_y,
                    self.stride_y,
                    self.stride_y,
                    self.mb_height * 16,
                    l1_src_x + x as i32,
                    l1_src_y + y as i32,
                    l1_fx,
                    l1_fy,
                ) as i32;
                let dst_idx = (dst_y + y) * self.stride_y + (dst_x + x);
                if dst_idx < self.ref_y.len() {
                    self.ref_y[dst_idx] = Self::weighted_bi_sample(
                        px0,
                        px1,
                        luma_w0,
                        luma_w1,
                        luma_o0,
                        luma_o1,
                        luma_log2_weight_denom,
                    );
                }
            }
        }

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let l0_c_src_x = c_dst_x as i32 + floor_div(mv0_x_qpel, 8);
        let l0_c_src_y = c_dst_y as i32 + floor_div(mv0_y_qpel, 8);
        let l1_c_src_x = c_dst_x as i32 + floor_div(mv1_x_qpel, 8);
        let l1_c_src_y = c_dst_y as i32 + floor_div(mv1_y_qpel, 8);
        let l0_c_fx = mod_floor(mv0_x_qpel, 8) as u8;
        let l0_c_fy = mod_floor(mv0_y_qpel, 8) as u8;
        let l1_c_fx = mod_floor(mv1_x_qpel, 8) as u8;
        let l1_c_fy = mod_floor(mv1_y_qpel, 8) as u8;

        for y in 0..ch {
            for x in 0..cw {
                let u0 = sample_bilinear_clamped(
                    src_l0_u,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l0_c_src_x + x as i32,
                    l0_c_src_y + y as i32,
                    l0_c_fx,
                    l0_c_fy,
                    8,
                ) as i32;
                let u1 = sample_bilinear_clamped(
                    src_l1_u,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l1_c_src_x + x as i32,
                    l1_c_src_y + y as i32,
                    l1_c_fx,
                    l1_c_fy,
                    8,
                ) as i32;
                let v0 = sample_bilinear_clamped(
                    src_l0_v,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l0_c_src_x + x as i32,
                    l0_c_src_y + y as i32,
                    l0_c_fx,
                    l0_c_fy,
                    8,
                ) as i32;
                let v1 = sample_bilinear_clamped(
                    src_l1_v,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l1_c_src_x + x as i32,
                    l1_c_src_y + y as i32,
                    l1_c_fx,
                    l1_c_fy,
                    8,
                ) as i32;
                let dst_idx = (c_dst_y + y) * self.stride_c + (c_dst_x + x);
                if dst_idx < self.ref_u.len() {
                    self.ref_u[dst_idx] = Self::weighted_bi_sample(
                        u0,
                        u1,
                        chroma_w0[0],
                        chroma_w1[0],
                        chroma_o0[0],
                        chroma_o1[0],
                        chroma_log2_weight_denom,
                    );
                }
                if dst_idx < self.ref_v.len() {
                    self.ref_v[dst_idx] = Self::weighted_bi_sample(
                        v0,
                        v1,
                        chroma_w0[1],
                        chroma_w1[1],
                        chroma_o0[1],
                        chroma_o1[1],
                        chroma_log2_weight_denom,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_p_inter_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        p_mb_type: u8,
        cur_qp: &mut i32,
        num_ref_idx_l0: u32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        let trace_slice_mb = self.trace_slice_mb;
        let trace_mb_limit = self.trace_mb_limit;
        let trace_this_mb = self.should_trace_mb_idx(mb_idx, trace_mb_limit);
        let trace_mb_detail = self.trace_mb_detail_enabled() && trace_this_mb;
        let trace_stage_bits = self.trace_p_stage_bits && trace_this_mb;
        let stage_start_bits = trace_stage_bits.then(|| cabac.bits_read());
        let mut stage_anchor_bits = stage_start_bits;
        let log_stage = |stage: &str, cabac: &CabacDecoder, anchor: &mut Option<usize>| {
            if let Some(prev_bits) = *anchor {
                let now = cabac.bits_read();
                let total_delta = stage_start_bits
                    .map(|start| now.saturating_sub(start))
                    .unwrap_or(0);
                eprintln!(
                    "[H264_P_STAGE_BITS] idx={} mb=({}, {}) p_mb_type={} stage={} bits_before={} bits_after={} delta={} total_delta={}",
                    mb_idx,
                    mb_x,
                    mb_y,
                    p_mb_type,
                    stage,
                    prev_bits,
                    now,
                    now.saturating_sub(prev_bits),
                    total_delta
                );
                *anchor = Some(now);
            }
        };
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);

        let mut final_mv_x = 0i32;
        let mut final_mv_y = 0i32;
        let mut final_ref_idx = 0u32;
        let mut no_sub_mb_part_size_less_than_8x8_flag = true;

        match p_mb_type {
            0 => {
                final_ref_idx =
                    self.decode_ref_idx(cabac, ctxs, num_ref_idx_l0, 0, mb_x * 4, mb_y * 4, false);
                let ref_idx_i8 = final_ref_idx.min(i8::MAX as u32) as i8;
                // 先写入当前分区 ref 缓存, 让同 MB 后续上下文/MV 预测使用规范邻居状态.
                self.set_l0_motion_block_4x4(mb_x * 16, mb_y * 16, 16, 16, 0, 0, ref_idx_i8);
                let (pred_mv_x, pred_mv_y) =
                    self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, ref_idx_i8);
                let (amvd_x, amvd_y) = self.compute_cabac_amvd(mb_x * 4, mb_y * 4, 0);
                let bits_before_mvd = if trace_mb_detail {
                    cabac.bits_read()
                } else {
                    0
                };
                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                self.set_mvd_block_4x4(mb_x * 16, mb_y * 16, 16, 16, mvd_x, mvd_y, 0);
                final_mv_x = pred_mv_x + mvd_x;
                final_mv_y = pred_mv_y + mvd_y;
                if trace_mb_detail {
                    let bits_after_mvd = cabac.bits_read();
                    eprintln!(
                        "[H264_P_MV] idx={} mb=({}, {}) type=16x16 ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                        mb_idx,
                        mb_x,
                        mb_y,
                        final_ref_idx,
                        pred_mv_x,
                        pred_mv_y,
                        amvd_x,
                        amvd_y,
                        mvd_x,
                        mvd_y,
                        final_mv_x,
                        final_mv_y,
                        bits_before_mvd,
                        bits_after_mvd,
                        bits_after_mvd.saturating_sub(bits_before_mvd)
                    );
                }
                self.apply_inter_block_l0(
                    ref_l0_list,
                    final_ref_idx,
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                    final_mv_x,
                    final_mv_y,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
            }
            1 => {
                let mut ref_idx_parts = [0u32; 2];
                for (part, slot) in ref_idx_parts.iter_mut().enumerate() {
                    let ref_idx = if num_ref_idx_l0 > 1 {
                        self.decode_ref_idx(
                            cabac,
                            ctxs,
                            num_ref_idx_l0,
                            0,
                            mb_x * 4,
                            mb_y * 4 + part * 2,
                            false,
                        )
                    } else {
                        0
                    };
                    *slot = ref_idx;
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    self.set_l0_motion_block_4x4(
                        mb_x * 16,
                        mb_y * 16 + part * 8,
                        16,
                        8,
                        0,
                        0,
                        ref_idx_i8,
                    );
                }
                for part in 0..2usize {
                    let ref_idx = ref_idx_parts[part];
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    let (part_pred_mv_x, part_pred_mv_y) =
                        self.predict_mv_l0_16x8(mb_x, mb_y, part, ref_idx_i8);
                    let y_off = part * 8;
                    let (amvd_x, amvd_y) =
                        self.compute_cabac_amvd(mb_x * 4, mb_y * 4 + part * 2, 0);
                    let bits_before_mvd = if trace_mb_detail {
                        cabac.bits_read()
                    } else {
                        0
                    };
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                    self.set_mvd_block_4x4(mb_x * 16, mb_y * 16 + y_off, 16, 8, mvd_x, mvd_y, 0);
                    let mv_x = part_pred_mv_x + mvd_x;
                    let mv_y = part_pred_mv_y + mvd_y;
                    if trace_mb_detail {
                        let bits_after_mvd = cabac.bits_read();
                        eprintln!(
                            "[H264_P_MV] idx={} mb=({}, {}) type=16x8 part={} ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                            mb_idx,
                            mb_x,
                            mb_y,
                            part,
                            ref_idx,
                            part_pred_mv_x,
                            part_pred_mv_y,
                            amvd_x,
                            amvd_y,
                            mvd_x,
                            mvd_y,
                            mv_x,
                            mv_y,
                            bits_before_mvd,
                            bits_after_mvd,
                            bits_after_mvd.saturating_sub(bits_before_mvd)
                        );
                    }
                    self.apply_inter_block_l0(
                        ref_l0_list,
                        ref_idx,
                        mb_x * 16,
                        mb_y * 16 + y_off,
                        16,
                        8,
                        mv_x,
                        mv_y,
                        l0_weights,
                        luma_log2_weight_denom,
                        chroma_log2_weight_denom,
                    );
                    final_mv_x = mv_x;
                    final_mv_y = mv_y;
                    final_ref_idx = ref_idx;
                }
            }
            2 => {
                let mut ref_idx_parts = [0u32; 2];
                for (part, slot) in ref_idx_parts.iter_mut().enumerate() {
                    let ref_idx = if num_ref_idx_l0 > 1 {
                        self.decode_ref_idx(
                            cabac,
                            ctxs,
                            num_ref_idx_l0,
                            0,
                            mb_x * 4 + part * 2,
                            mb_y * 4,
                            false,
                        )
                    } else {
                        0
                    };
                    *slot = ref_idx;
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    self.set_l0_motion_block_4x4(
                        mb_x * 16 + part * 8,
                        mb_y * 16,
                        8,
                        16,
                        0,
                        0,
                        ref_idx_i8,
                    );
                }
                for part in 0..2usize {
                    let ref_idx = ref_idx_parts[part];
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    let (part_pred_mv_x, part_pred_mv_y) =
                        self.predict_mv_l0_8x16(mb_x, mb_y, part, ref_idx_i8);
                    let x_off = part * 8;
                    let (amvd_x, amvd_y) =
                        self.compute_cabac_amvd(mb_x * 4 + part * 2, mb_y * 4, 0);
                    let bits_before_mvd = if trace_mb_detail {
                        cabac.bits_read()
                    } else {
                        0
                    };
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                    self.set_mvd_block_4x4(mb_x * 16 + x_off, mb_y * 16, 8, 16, mvd_x, mvd_y, 0);
                    let mv_x = part_pred_mv_x + mvd_x;
                    let mv_y = part_pred_mv_y + mvd_y;
                    if trace_mb_detail {
                        let bits_after_mvd = cabac.bits_read();
                        eprintln!(
                            "[H264_P_MV] idx={} mb=({}, {}) type=8x16 part={} ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                            mb_idx,
                            mb_x,
                            mb_y,
                            part,
                            ref_idx,
                            part_pred_mv_x,
                            part_pred_mv_y,
                            amvd_x,
                            amvd_y,
                            mvd_x,
                            mvd_y,
                            mv_x,
                            mv_y,
                            bits_before_mvd,
                            bits_after_mvd,
                            bits_after_mvd.saturating_sub(bits_before_mvd)
                        );
                    }
                    self.apply_inter_block_l0(
                        ref_l0_list,
                        ref_idx,
                        mb_x * 16 + x_off,
                        mb_y * 16,
                        8,
                        16,
                        mv_x,
                        mv_y,
                        l0_weights,
                        luma_log2_weight_denom,
                        chroma_log2_weight_denom,
                    );
                    final_mv_x = mv_x;
                    final_mv_y = mv_y;
                    final_ref_idx = ref_idx;
                }
            }
            _ => {
                let mut sub_types = [0u8; 4];
                for sub in 0..4usize {
                    sub_types[sub] = self.decode_p_sub_mb_type(cabac, ctxs);
                }
                if trace_mb_detail {
                    eprintln!(
                        "[H264_P_SUB] idx={} mb=({}, {}) sub_types=[{},{},{},{}]",
                        mb_idx, mb_x, mb_y, sub_types[0], sub_types[1], sub_types[2], sub_types[3]
                    );
                }
                no_sub_mb_part_size_less_than_8x8_flag =
                    sub_types.iter().all(|&sub_type| sub_type == 0);

                let mut ref_idx_sub = [0u32; 4];
                // 逐个子分区即时写回 ref 缓存, 对齐 FFmpeg 的 ref_cache 更新时机.
                for (sub, slot) in ref_idx_sub.iter_mut().enumerate() {
                    let ref_idx = if p_mb_type == 3 && num_ref_idx_l0 > 1 {
                        self.decode_ref_idx(
                            cabac,
                            ctxs,
                            num_ref_idx_l0,
                            0,
                            mb_x * 4 + (sub & 1) * 2,
                            mb_y * 4 + (sub >> 1) * 2,
                            false,
                        )
                    } else {
                        0
                    };
                    *slot = ref_idx;
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    if trace_mb_detail {
                        eprintln!(
                            "[H264_P_SUB_REF] idx={} mb=({}, {}) sub={} sub_type={} ref_idx={}",
                            mb_idx, mb_x, mb_y, sub, sub_types[sub], ref_idx
                        );
                    }
                    self.set_l0_motion_block_4x4(
                        mb_x * 16 + (sub & 1) * 8,
                        mb_y * 16 + (sub >> 1) * 8,
                        8,
                        8,
                        0,
                        0,
                        ref_idx_i8,
                    );
                }

                for sub in 0..4usize {
                    let sub_type = sub_types[sub];
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    let ref_idx = ref_idx_sub[sub];
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    match sub_type {
                        0 => {
                            let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                mb_x,
                                mb_y,
                                sx / 4,
                                sy / 4,
                                2,
                                ref_idx_i8,
                            );
                            let px_x = mb_x * 16 + sx;
                            let px_y = mb_y * 16 + sy;
                            let (amvd_x, amvd_y) = self.compute_cabac_amvd(px_x / 4, px_y / 4, 0);
                            let bits_before_mvd = if trace_mb_detail {
                                cabac.bits_read()
                            } else {
                                0
                            };
                            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                            self.set_mvd_block_4x4(px_x, px_y, 8, 8, mvd_x, mvd_y, 0);
                            let mv_x = pred_mv_x + mvd_x;
                            let mv_y = pred_mv_y + mvd_y;
                            if trace_mb_detail {
                                let bits_after_mvd = cabac.bits_read();
                                eprintln!(
                                    "[H264_P_SUB_MV] idx={} mb=({}, {}) sub={} sub_type=8x8 part=0 ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                                    mb_idx,
                                    mb_x,
                                    mb_y,
                                    sub,
                                    ref_idx,
                                    pred_mv_x,
                                    pred_mv_y,
                                    amvd_x,
                                    amvd_y,
                                    mvd_x,
                                    mvd_y,
                                    mv_x,
                                    mv_y,
                                    bits_before_mvd,
                                    bits_after_mvd,
                                    bits_after_mvd.saturating_sub(bits_before_mvd)
                                );
                            }
                            self.apply_inter_block_l0(
                                ref_l0_list,
                                ref_idx,
                                px_x,
                                px_y,
                                8,
                                8,
                                mv_x,
                                mv_y,
                                l0_weights,
                                luma_log2_weight_denom,
                                chroma_log2_weight_denom,
                            );
                            final_mv_x = mv_x;
                            final_mv_y = mv_y;
                            final_ref_idx = ref_idx;
                        }
                        1 => {
                            for part in 0..2usize {
                                let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                    mb_x,
                                    mb_y,
                                    sx / 4,
                                    sy / 4 + part,
                                    2,
                                    ref_idx_i8,
                                );
                                let px_x = mb_x * 16 + sx;
                                let px_y = mb_y * 16 + sy + part * 4;
                                let (amvd_x, amvd_y) =
                                    self.compute_cabac_amvd(px_x / 4, px_y / 4, 0);
                                let bits_before_mvd = if trace_mb_detail {
                                    cabac.bits_read()
                                } else {
                                    0
                                };
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                                self.set_mvd_block_4x4(px_x, px_y, 8, 4, mvd_x, mvd_y, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                if trace_mb_detail {
                                    let bits_after_mvd = cabac.bits_read();
                                    eprintln!(
                                        "[H264_P_SUB_MV] idx={} mb=({}, {}) sub={} sub_type=8x4 part={} ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                                        mb_idx,
                                        mb_x,
                                        mb_y,
                                        sub,
                                        part,
                                        ref_idx,
                                        pred_mv_x,
                                        pred_mv_y,
                                        amvd_x,
                                        amvd_y,
                                        mvd_x,
                                        mvd_y,
                                        mv_x,
                                        mv_y,
                                        bits_before_mvd,
                                        bits_after_mvd,
                                        bits_after_mvd.saturating_sub(bits_before_mvd)
                                    );
                                }
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    px_x,
                                    px_y,
                                    8,
                                    4,
                                    mv_x,
                                    mv_y,
                                    l0_weights,
                                    luma_log2_weight_denom,
                                    chroma_log2_weight_denom,
                                );
                                final_mv_x = mv_x;
                                final_mv_y = mv_y;
                                final_ref_idx = ref_idx;
                            }
                        }
                        2 => {
                            for part in 0..2usize {
                                let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                    mb_x,
                                    mb_y,
                                    sx / 4 + part,
                                    sy / 4,
                                    1,
                                    ref_idx_i8,
                                );
                                let px_x = mb_x * 16 + sx + part * 4;
                                let px_y = mb_y * 16 + sy;
                                let (amvd_x, amvd_y) =
                                    self.compute_cabac_amvd(px_x / 4, px_y / 4, 0);
                                let bits_before_mvd = if trace_mb_detail {
                                    cabac.bits_read()
                                } else {
                                    0
                                };
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                                self.set_mvd_block_4x4(px_x, px_y, 4, 8, mvd_x, mvd_y, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                if trace_mb_detail {
                                    let bits_after_mvd = cabac.bits_read();
                                    eprintln!(
                                        "[H264_P_SUB_MV] idx={} mb=({}, {}) sub={} sub_type=4x8 part={} ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                                        mb_idx,
                                        mb_x,
                                        mb_y,
                                        sub,
                                        part,
                                        ref_idx,
                                        pred_mv_x,
                                        pred_mv_y,
                                        amvd_x,
                                        amvd_y,
                                        mvd_x,
                                        mvd_y,
                                        mv_x,
                                        mv_y,
                                        bits_before_mvd,
                                        bits_after_mvd,
                                        bits_after_mvd.saturating_sub(bits_before_mvd)
                                    );
                                }
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    px_x,
                                    px_y,
                                    4,
                                    8,
                                    mv_x,
                                    mv_y,
                                    l0_weights,
                                    luma_log2_weight_denom,
                                    chroma_log2_weight_denom,
                                );
                                final_mv_x = mv_x;
                                final_mv_y = mv_y;
                                final_ref_idx = ref_idx;
                            }
                        }
                        _ => {
                            for part_y in 0..2usize {
                                for part_x in 0..2usize {
                                    let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sx / 4 + part_x,
                                        sy / 4 + part_y,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let px_x = mb_x * 16 + sx + part_x * 4;
                                    let px_y = mb_y * 16 + sy + part_y * 4;
                                    let (amvd_x, amvd_y) =
                                        self.compute_cabac_amvd(px_x / 4, px_y / 4, 0);
                                    let bits_before_mvd = if trace_mb_detail {
                                        cabac.bits_read()
                                    } else {
                                        0
                                    };
                                    let mvd_x =
                                        self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                                    let mvd_y =
                                        self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                                    self.set_mvd_block_4x4(px_x, px_y, 4, 4, mvd_x, mvd_y, 0);
                                    let mv_x = pred_mv_x + mvd_x;
                                    let mv_y = pred_mv_y + mvd_y;
                                    if trace_mb_detail {
                                        let bits_after_mvd = cabac.bits_read();
                                        let part = part_y * 2 + part_x;
                                        eprintln!(
                                            "[H264_P_SUB_MV] idx={} mb=({}, {}) sub={} sub_type=4x4 part={} ref_idx={} pred=({}, {}) amvd=({}, {}) mvd=({}, {}) mv=({}, {}) bits_before={} bits_after={} delta={}",
                                            mb_idx,
                                            mb_x,
                                            mb_y,
                                            sub,
                                            part,
                                            ref_idx,
                                            pred_mv_x,
                                            pred_mv_y,
                                            amvd_x,
                                            amvd_y,
                                            mvd_x,
                                            mvd_y,
                                            mv_x,
                                            mv_y,
                                            bits_before_mvd,
                                            bits_after_mvd,
                                            bits_after_mvd.saturating_sub(bits_before_mvd)
                                        );
                                    }
                                    self.apply_inter_block_l0(
                                        ref_l0_list,
                                        ref_idx,
                                        px_x,
                                        px_y,
                                        4,
                                        4,
                                        mv_x,
                                        mv_y,
                                        l0_weights,
                                        luma_log2_weight_denom,
                                        chroma_log2_weight_denom,
                                    );
                                    final_mv_x = mv_x;
                                    final_mv_y = mv_y;
                                    final_ref_idx = ref_idx;
                                }
                            }
                        }
                    }
                }
            }
        }
        log_stage("motion", cabac, &mut stage_anchor_bits);

        self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = final_ref_idx.min(i8::MAX as u32) as i8;
        self.mb_types[mb_idx] = 200u8.saturating_add(p_mb_type.min(3));

        let (luma_cbp, chroma_cbp) =
            self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, false);
        log_stage("cbp", cabac, &mut stage_anchor_bits);
        let cbp = luma_cbp | (chroma_cbp << 4);
        self.set_mb_cbp(mb_x, mb_y, cbp);
        if trace_slice_mb && trace_this_mb {
            eprintln!(
                "[H264_P_CBP] idx={} mb=({}, {}) p_mb_type={} luma_cbp={} chroma_cbp={} cbp={}",
                mb_idx, mb_x, mb_y, p_mb_type, luma_cbp, chroma_cbp, cbp
            );
        }

        if cbp != 0 {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }
        log_stage("qp_delta", cabac, &mut stage_anchor_bits);

        let forced_use_8x8 = self.debug_force_inter_use_8x8;
        let force_mb0_use_8x8 = self.debug_force_inter_mb0_use_8x8;
        let use_old_transform_ctx = self.debug_inter_use_old_transform_ctx;
        let use_8x8 = if let Some(v) = forced_use_8x8 {
            luma_cbp != 0 && v
        } else if force_mb0_use_8x8 && mb_idx == 0 {
            luma_cbp != 0
        } else {
            luma_cbp != 0
                && no_sub_mb_part_size_less_than_8x8_flag
                && self
                    .pps
                    .as_ref()
                    .map(|p| p.transform_8x8_mode)
                    .unwrap_or(false)
                && if use_old_transform_ctx {
                    self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y)
                } else {
                    self.decode_transform_size_8x8_flag_inter(cabac, ctxs, mb_x, mb_y)
                }
        };
        log_stage("transform_size_8x8_flag", cabac, &mut stage_anchor_bits);
        if trace_slice_mb && trace_this_mb {
            let forced_use_8x8_label = match forced_use_8x8 {
                Some(true) => "1",
                Some(false) => "0",
                None => "auto",
            };
            eprintln!(
                "[H264_P_T8X8] idx={} mb=({}, {}) p_mb_type={} use_8x8={} forced={}",
                mb_idx, mb_x, mb_y, p_mb_type, use_8x8, forced_use_8x8_label
            );
        }
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);

        let skip_inter_residual = self.debug_skip_inter_residual;
        let skip_inter_luma_residual = self.debug_skip_inter_luma_residual;
        let skip_inter_chroma_residual = self.debug_skip_inter_chroma_residual;
        let before_luma_bits = if trace_stage_bits {
            Some(cabac.bits_read())
        } else {
            None
        };
        if !skip_inter_residual {
            if !skip_inter_luma_residual {
                if use_8x8 {
                    self.decode_i8x8_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, false);
                } else {
                    self.decode_inter_4x4_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp);
                }
            }
        }
        if let Some(before) = before_luma_bits {
            stage_anchor_bits = Some(before);
            log_stage("luma_residual", cabac, &mut stage_anchor_bits);
        }
        let before_chroma_bits = if trace_stage_bits {
            Some(cabac.bits_read())
        } else {
            None
        };
        if !skip_inter_residual {
            if chroma_cbp >= 1 && !skip_inter_chroma_residual {
                self.decode_chroma_residual(
                    cabac,
                    ctxs,
                    (mb_x, mb_y),
                    *cur_qp,
                    chroma_cbp >= 2,
                    false,
                );
            }
        }
        if let Some(before) = before_chroma_bits {
            stage_anchor_bits = Some(before);
            log_stage("chroma_residual", cabac, &mut stage_anchor_bits);
            log_stage("total", cabac, &mut stage_anchor_bits);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_b_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        slice_first_mb: u32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        direct_spatial_mv_pred_flag: bool,
        l0_weights: &[PredWeightL0],
        l1_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) {
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let trace_slice = self.trace_slice;
        let trace_slice_mb = self.trace_slice_mb;
        let trace_mb_limit = self.trace_mb_limit;
        let ignore_terminate = self.debug_ignore_terminate;
        let mut decoded_mbs = 0usize;
        let mut term_break = false;
        let mut last_mb_idx = first;

        for mb_idx in first..total {
            self.mark_mb_slice_first_mb(mb_idx, slice_first_mb);
            self.set_mb_skip_flag(mb_idx, false);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let trace_this_mb = self.should_trace_mb_idx(mb_idx, trace_mb_limit);
            self.clear_mb_mvd_cache(mb_x, mb_y);
            let skip = self.decode_b_mb_skip_flag(cabac, ctxs, mb_x, mb_y);

            if skip {
                self.set_mb_skip_flag(mb_idx, true);
                if trace_slice_mb && trace_this_mb {
                    eprintln!("[H264_B_MB] idx={} mb=({}, {}) skip=1", mb_idx, mb_x, mb_y);
                }
                self.mb_types[mb_idx] = 254;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
                self.set_direct_block_4x4(mb_x * 16, mb_y * 16, 16, 16, true);
                let (pred_x, pred_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
                let (motion_l0, motion_l1) = self.build_b_direct_motion(
                    mb_x,
                    mb_y,
                    pred_x,
                    pred_y,
                    direct_spatial_mv_pred_flag,
                    ref_l0_list,
                    ref_l1_list,
                );
                let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                    motion_l0,
                    motion_l1,
                    l0_weights,
                    l1_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                    ref_l0_list,
                    ref_l1_list,
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                );
                self.mv_l0_x[mb_idx] = mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                self.mv_l0_y[mb_idx] = mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                self.ref_idx_l0[mb_idx] = ref_idx;
                self.prev_qp_delta_nz = false;
            } else {
                match self.decode_b_mb_type(cabac, ctxs, mb_x, mb_y) {
                    BMbType::Intra => {
                        if trace_slice_mb && trace_this_mb {
                            eprintln!(
                                "[H264_B_MB] idx={} mb=({}, {}) skip=0 mb_type=intra",
                                mb_idx, mb_x, mb_y
                            );
                        }
                        let intra_mb_type = decode_intra_mb_type(
                            cabac,
                            ctxs,
                            32,
                            false,
                            &self.mb_types,
                            self.mb_width,
                            mb_x,
                            mb_y,
                        );
                        self.mb_types[mb_idx] = intra_mb_type as u8;
                        if intra_mb_type == 0 {
                            self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
                        } else if intra_mb_type <= 24 {
                            self.decode_i_16x16_mb(
                                cabac,
                                ctxs,
                                mb_x,
                                mb_y,
                                intra_mb_type,
                                &mut cur_qp,
                            );
                        } else if intra_mb_type == 25 {
                            self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                            self.prev_qp_delta_nz = false;
                            cur_qp = 0;
                        }
                    }
                    BMbType::Direct => {
                        if trace_slice_mb && trace_this_mb {
                            eprintln!(
                                "[H264_B_MB] idx={} mb=({}, {}) skip=0 mb_type=direct",
                                mb_idx, mb_x, mb_y
                            );
                        }
                        self.decode_b_inter_mb(
                            cabac,
                            ctxs,
                            mb_x,
                            mb_y,
                            None,
                            direct_spatial_mv_pred_flag,
                            &mut cur_qp,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            l0_weights,
                            l1_weights,
                            luma_log2_weight_denom,
                            chroma_log2_weight_denom,
                            ref_l0_list,
                            ref_l1_list,
                        );
                    }
                    BMbType::Inter(mb_type_idx) => {
                        if trace_slice_mb && trace_this_mb {
                            eprintln!(
                                "[H264_B_MB] idx={} mb=({}, {}) skip=0 mb_type=inter({})",
                                mb_idx, mb_x, mb_y, mb_type_idx
                            );
                        }
                        self.decode_b_inter_mb(
                            cabac,
                            ctxs,
                            mb_x,
                            mb_y,
                            Some(mb_type_idx),
                            direct_spatial_mv_pred_flag,
                            &mut cur_qp,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            l0_weights,
                            l1_weights,
                            luma_log2_weight_denom,
                            chroma_log2_weight_denom,
                            ref_l0_list,
                            ref_l1_list,
                        );
                    }
                }
            }

            if mb_idx < self.mb_qp.len() {
                self.mb_qp[mb_idx] = cur_qp;
            }
            decoded_mbs += 1;
            last_mb_idx = mb_idx;
            if mb_idx + 1 < total {
                let terminate = cabac.decode_terminate() == 1;
                if terminate {
                    term_break = true;
                    if !ignore_terminate {
                        break;
                    }
                }
            }
        }
        if trace_slice {
            eprintln!(
                "[H264_SLICE_MB] type=B first_mb={} decoded_mbs={} last_mb_idx={} terminate_break={} cabac_bits={}/{}",
                first,
                decoded_mbs,
                last_mb_idx,
                term_break,
                cabac.bits_read(),
                cabac.bits_total()
            );
        }
    }
}
