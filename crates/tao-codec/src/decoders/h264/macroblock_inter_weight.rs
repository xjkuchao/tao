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
        let shift = i32::from(log2_denom).saturating_add(1);
        let round = 1i32 << i32::from(log2_denom);
        let offset = (o0 + o1 + 1) >> 1;
        let weighted = (w0 * p0 + w1 * p1 + round) >> shift;
        (weighted + offset).clamp(0, 255) as u8
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
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);

        let mut final_mv_x = 0i32;
        let mut final_mv_y = 0i32;
        let mut final_ref_idx = 0u32;

        match p_mb_type {
            0 => {
                final_ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                let ref_idx_i8 = final_ref_idx.min(i8::MAX as u32) as i8;
                let (pred_mv_x, pred_mv_y) =
                    self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, ref_idx_i8);
                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                final_mv_x = pred_mv_x + mvd_x;
                final_mv_y = pred_mv_y + mvd_y;
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
                let mut top_ref_idx = 0u32;
                let mut top_mv_x = 0i32;
                let mut top_mv_y = 0i32;
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    let (part_pred_mv_x, part_pred_mv_y) = if part == 0 {
                        self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, ref_idx_i8)
                    } else if ref_idx == top_ref_idx {
                        (top_mv_x, top_mv_y)
                    } else {
                        self.predict_mv_l0_partition(mb_x, mb_y, 0, 2, 4, ref_idx_i8)
                    };
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = part_pred_mv_x + mvd_x;
                    let mv_y = part_pred_mv_y + mvd_y;
                    let y_off = part * 8;
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
                    if part == 0 {
                        top_ref_idx = ref_idx;
                        top_mv_x = mv_x;
                        top_mv_y = mv_y;
                    }
                }
            }
            2 => {
                let mut left_ref_idx = 0u32;
                let mut left_mv_x = 0i32;
                let mut left_mv_y = 0i32;
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                    let (part_pred_mv_x, part_pred_mv_y) = if part == 0 {
                        self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 2, ref_idx_i8)
                    } else if ref_idx == left_ref_idx {
                        (left_mv_x, left_mv_y)
                    } else {
                        self.predict_mv_l0_partition(mb_x, mb_y, 2, 0, 2, ref_idx_i8)
                    };
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = part_pred_mv_x + mvd_x;
                    let mv_y = part_pred_mv_y + mvd_y;
                    let x_off = part * 8;
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
                    if part == 0 {
                        left_ref_idx = ref_idx;
                        left_mv_x = mv_x;
                        left_mv_y = mv_y;
                    }
                }
            }
            _ => {
                for sub in 0..4usize {
                    let sub_type = self.decode_p_sub_mb_type(cabac, ctxs);
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    match sub_type {
                        0 => {
                            let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                            let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                            let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                mb_x,
                                mb_y,
                                sx / 4,
                                sy / 4,
                                2,
                                ref_idx_i8,
                            );
                            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                            let mv_x = pred_mv_x + mvd_x;
                            let mv_y = pred_mv_y + mvd_y;
                            self.apply_inter_block_l0(
                                ref_l0_list,
                                ref_idx,
                                mb_x * 16 + sx,
                                mb_y * 16 + sy,
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
                                let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                                let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                    mb_x,
                                    mb_y,
                                    sx / 4,
                                    sy / 4 + part,
                                    2,
                                    ref_idx_i8,
                                );
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    mb_x * 16 + sx,
                                    mb_y * 16 + sy + part * 4,
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
                                let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                                let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                    mb_x,
                                    mb_y,
                                    sx / 4 + part,
                                    sy / 4,
                                    1,
                                    ref_idx_i8,
                                );
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    mb_x * 16 + sx + part * 4,
                                    mb_y * 16 + sy,
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
                                    let ref_idx =
                                        self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                    let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                                    let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sx / 4 + part_x,
                                        sy / 4 + part_y,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                    let mv_x = pred_mv_x + mvd_x;
                                    let mv_y = pred_mv_y + mvd_y;
                                    self.apply_inter_block_l0(
                                        ref_l0_list,
                                        ref_idx,
                                        mb_x * 16 + sx + part_x * 4,
                                        mb_y * 16 + sy + part_y * 4,
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

        self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = final_ref_idx.min(i8::MAX as u32) as i8;
        self.mb_types[mb_idx] = 200u8.saturating_add(p_mb_type.min(3));

        let (luma_cbp, chroma_cbp) =
            self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, false);
        let cbp = luma_cbp | (chroma_cbp << 4);
        self.set_mb_cbp(mb_x, mb_y, cbp);

        if cbp != 0 {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        let use_8x8 = luma_cbp != 0
            && self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
            && self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);

        if use_8x8 {
            self.decode_i8x8_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, false);
        } else {
            self.decode_inter_4x4_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp);
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, false);
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

        for mb_idx in first..total {
            self.mark_mb_slice_first_mb(mb_idx, slice_first_mb);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_b_mb_skip_flag(cabac, ctxs, mb_x, mb_y);

            if skip {
                self.mb_types[mb_idx] = 254;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
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
            } else {
                match self.decode_b_mb_type(cabac, ctxs, mb_x, mb_y) {
                    BMbType::Intra => {
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
                        }
                    }
                    BMbType::Direct => {
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

            if mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                break;
            }
        }
    }
}
