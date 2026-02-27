use super::*;

impl H264Decoder {
    #[allow(clippy::too_many_arguments)]
    fn set_b_motion_cache_block(
        &mut self,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        motion_l0: Option<BMotion>,
        motion_l1: Option<BMotion>,
    ) {
        let (l0_mv_x, l0_mv_y, l0_ref_idx) = motion_l0
            .map(|m| (m.mv_x, m.mv_y, m.ref_idx))
            .unwrap_or((0, 0, -1));
        let (l1_mv_x, l1_mv_y, l1_ref_idx) = motion_l1
            .map(|m| (m.mv_x, m.mv_y, m.ref_idx))
            .unwrap_or((0, 0, -1));
        self.set_l0_motion_block_4x4(dst_x, dst_y, w, h, l0_mv_x, l0_mv_y, l0_ref_idx);
        self.set_l1_motion_block_4x4(dst_x, dst_y, w, h, l1_mv_x, l1_mv_y, l1_ref_idx);
        if let Some(mb_idx) = self.mb_index(dst_x / 16, dst_y / 16) {
            self.mv_l0_x[mb_idx] = l0_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            self.mv_l0_y[mb_idx] = l0_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            self.ref_idx_l0[mb_idx] = l0_ref_idx;
            self.mv_l1_x[mb_idx] = l1_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            self.mv_l1_y[mb_idx] = l1_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            self.ref_idx_l1[mb_idx] = l1_ref_idx;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_b_prediction_block(
        &mut self,
        motion_l0: Option<BMotion>,
        motion_l1: Option<BMotion>,
        l0_weights: &[PredWeightL0],
        l1_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
    ) -> (i32, i32, i8) {
        match (motion_l0, motion_l1) {
            (Some(m0), Some(m1)) => {
                let fallback_l0 = self.zero_reference_planes();
                let fallback_l1 = self.zero_reference_planes();
                let ref_l0 = if let Some(found) = select_ref_planes(ref_l0_list, m0.ref_idx) {
                    found
                } else {
                    self.record_missing_reference_fallback(
                        "apply_b_prediction_block_l0",
                        m0.ref_idx as i32,
                        ref_l0_list.len(),
                    );
                    &fallback_l0
                };
                let ref_l1 = if let Some(found) = select_ref_planes(ref_l1_list, m1.ref_idx) {
                    found
                } else {
                    self.record_missing_reference_fallback(
                        "apply_b_prediction_block_l1",
                        m1.ref_idx as i32,
                        ref_l1_list.len(),
                    );
                    &fallback_l1
                };
                let weighted_bipred_idc = self
                    .pps
                    .as_ref()
                    .map(|p| p.weighted_bipred_idc)
                    .unwrap_or(0);
                let weighted_bipred_idc = if self.weighted_pred_disabled() {
                    0
                } else {
                    weighted_bipred_idc
                };
                if weighted_bipred_idc == 2 {
                    let (w0, w1) = self.implicit_bi_weights(
                        ref_l0.poc,
                        ref_l1.poc,
                        ref_l0.is_long_term,
                        ref_l1.is_long_term,
                    );
                    self.apply_bi_weighted_block(
                        ref_l0.y.as_slice(),
                        ref_l0.u.as_slice(),
                        ref_l0.v.as_slice(),
                        ref_l1.y.as_slice(),
                        ref_l1.u.as_slice(),
                        ref_l1.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m0.mv_x,
                        m0.mv_y,
                        m1.mv_x,
                        m1.mv_y,
                        w0,
                        w1,
                    );
                } else if weighted_bipred_idc == 1 {
                    let default_luma_weight = 1 << luma_log2_weight_denom;
                    let default_chroma_weight = 1 << chroma_log2_weight_denom;
                    let weight_l0 = p_l0_weight(l0_weights, m0.ref_idx.max(0) as u32).copied();
                    let weight_l1 = p_l1_weight(l1_weights, m1.ref_idx.max(0) as u32).copied();
                    let l0 = weight_l0.unwrap_or(PredWeightL0 {
                        luma_weight: default_luma_weight,
                        luma_offset: 0,
                        chroma_weight: [default_chroma_weight; 2],
                        chroma_offset: [0, 0],
                    });
                    let l1 = weight_l1.unwrap_or(PredWeightL0 {
                        luma_weight: default_luma_weight,
                        luma_offset: 0,
                        chroma_weight: [default_chroma_weight; 2],
                        chroma_offset: [0, 0],
                    });
                    self.apply_bi_explicit_weighted_block(
                        ref_l0.y.as_slice(),
                        ref_l0.u.as_slice(),
                        ref_l0.v.as_slice(),
                        ref_l1.y.as_slice(),
                        ref_l1.u.as_slice(),
                        ref_l1.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m0.mv_x,
                        m0.mv_y,
                        m1.mv_x,
                        m1.mv_y,
                        l0.luma_weight,
                        l0.luma_offset,
                        l1.luma_weight,
                        l1.luma_offset,
                        l0.chroma_weight,
                        l0.chroma_offset,
                        l1.chroma_weight,
                        l1.chroma_offset,
                        luma_log2_weight_denom,
                        chroma_log2_weight_denom,
                    );
                } else {
                    self.apply_inter_block(
                        ref_l0.y.as_slice(),
                        ref_l0.u.as_slice(),
                        ref_l0.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m0.mv_x,
                        m0.mv_y,
                        None,
                        0,
                        0,
                    );
                    self.blend_inter_block(
                        ref_l1.y.as_slice(),
                        ref_l1.u.as_slice(),
                        ref_l1.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m1.mv_x,
                        m1.mv_y,
                    );
                }
                self.set_b_motion_cache_block(dst_x, dst_y, w, h, Some(m0), Some(m1));
                (m0.mv_x, m0.mv_y, m0.ref_idx)
            }
            (Some(m0), None) => {
                let fallback_l0 = self.zero_reference_planes();
                let ref_l0 = if let Some(found) = select_ref_planes(ref_l0_list, m0.ref_idx) {
                    found
                } else {
                    self.record_missing_reference_fallback(
                        "apply_b_prediction_block_l0_only",
                        m0.ref_idx as i32,
                        ref_l0_list.len(),
                    );
                    &fallback_l0
                };
                let weighted_bipred_idc = self
                    .pps
                    .as_ref()
                    .map(|p| p.weighted_bipred_idc)
                    .unwrap_or(0);
                let weighted_bipred_idc = if self.weighted_pred_disabled() {
                    0
                } else {
                    weighted_bipred_idc
                };
                let pred_weight = if weighted_bipred_idc == 1 {
                    p_l0_weight(l0_weights, m0.ref_idx.max(0) as u32)
                } else {
                    None
                };
                self.apply_inter_block(
                    ref_l0.y.as_slice(),
                    ref_l0.u.as_slice(),
                    ref_l0.v.as_slice(),
                    dst_x,
                    dst_y,
                    w,
                    h,
                    m0.mv_x,
                    m0.mv_y,
                    pred_weight,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
                self.set_b_motion_cache_block(dst_x, dst_y, w, h, Some(m0), None);
                (m0.mv_x, m0.mv_y, m0.ref_idx)
            }
            (None, Some(m1)) => {
                let fallback_l1 = self.zero_reference_planes();
                let ref_l1 = if let Some(found) = select_ref_planes(ref_l1_list, m1.ref_idx) {
                    found
                } else {
                    self.record_missing_reference_fallback(
                        "apply_b_prediction_block_l1_only",
                        m1.ref_idx as i32,
                        ref_l1_list.len(),
                    );
                    &fallback_l1
                };
                let weighted_bipred_idc = self
                    .pps
                    .as_ref()
                    .map(|p| p.weighted_bipred_idc)
                    .unwrap_or(0);
                let weighted_bipred_idc = if self.weighted_pred_disabled() {
                    0
                } else {
                    weighted_bipred_idc
                };
                let pred_weight = if weighted_bipred_idc == 1 {
                    p_l1_weight(l1_weights, m1.ref_idx.max(0) as u32)
                } else {
                    None
                };
                self.apply_inter_block(
                    ref_l1.y.as_slice(),
                    ref_l1.u.as_slice(),
                    ref_l1.v.as_slice(),
                    dst_x,
                    dst_y,
                    w,
                    h,
                    m1.mv_x,
                    m1.mv_y,
                    pred_weight,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
                self.set_b_motion_cache_block(dst_x, dst_y, w, h, None, Some(m1));
                (m1.mv_x, m1.mv_y, m1.ref_idx)
            }
            (None, None) => {
                self.set_b_motion_cache_block(dst_x, dst_y, w, h, None, None);
                (0, 0, 0)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_b_inter_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        mb_type_idx: Option<u8>,
        direct_spatial_mv_pred_flag: bool,
        cur_qp: &mut i32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        l0_weights: &[PredWeightL0],
        l1_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        self.set_direct_block_4x4(mb_x * 16, mb_y * 16, 16, 16, false);

        let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
        let mut final_mv_x = pred_mv_x;
        let mut final_mv_y = pred_mv_y;
        let mut final_ref_idx = 0i8;
        let mut no_sub_mb_part_size_less_than_8x8_flag = true;

        match mb_type_idx {
            None => {
                self.mb_types[mb_idx] = 254;
                self.set_direct_block_4x4(mb_x * 16, mb_y * 16, 16, 16, true);
                let (motion_l0, motion_l1) = self.build_b_direct_motion(
                    mb_x,
                    mb_y,
                    pred_mv_x,
                    pred_mv_y,
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
                final_mv_x = mv_x;
                final_mv_y = mv_y;
                final_ref_idx = ref_idx;
            }
            Some(22) => {
                self.mb_types[mb_idx] = 222;
                let mut sub_types = [0u8; 4];
                for slot in &mut sub_types {
                    *slot = self.decode_b_sub_mb_type(cabac, ctxs);
                }
                no_sub_mb_part_size_less_than_8x8_flag =
                    sub_types.iter().all(|&sub_type| sub_type <= 3);
                let mut sub_part_w = [8usize; 4];
                let mut sub_part_h = [8usize; 4];
                let mut sub_part_count = [1usize; 4];
                let mut sub_dir = [BPredDir::Direct; 4];
                let mut sub_use_l0 = [false; 4];
                let mut sub_use_l1 = [false; 4];
                for (sub, sub_type) in sub_types.iter().copied().enumerate() {
                    let (part_w, part_h, part_count, dir) = Self::b_sub_mb_info(sub_type);
                    sub_part_w[sub] = part_w;
                    sub_part_h[sub] = part_h;
                    sub_part_count[sub] = part_count;
                    sub_dir[sub] = dir;
                    sub_use_l0[sub] = matches!(dir, BPredDir::L0 | BPredDir::Bi);
                    sub_use_l1[sub] = matches!(dir, BPredDir::L1 | BPredDir::Bi);
                }

                // 语法顺序必须与规范一致: 先完整读取所有 L0 ref_idx, 再读取所有 L1 ref_idx.
                // 若按分区交错读取(L0/L1 同时), 在 L1_L0/L0_L1 等组合会导致 CABAC 失步.
                let mut ref_idx_l0 = [0i8; 4];
                for sub in 0..4usize {
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    if sub_use_l0[sub] {
                        let idx = if num_ref_idx_l0 > 1 {
                            self.decode_ref_idx(
                                cabac,
                                ctxs,
                                num_ref_idx_l0,
                                0,
                                mb_x * 4 + (sub & 1) * 2,
                                mb_y * 4 + (sub >> 1) * 2,
                                true,
                            )
                        } else {
                            0
                        };
                        ref_idx_l0[sub] = idx.min(i8::MAX as u32) as i8;
                        self.set_l0_motion_block_4x4(
                            mb_x * 16 + sx,
                            mb_y * 16 + sy,
                            8,
                            8,
                            0,
                            0,
                            ref_idx_l0[sub],
                        );
                    } else {
                        self.set_l0_motion_block_4x4(
                            mb_x * 16 + sx,
                            mb_y * 16 + sy,
                            8,
                            8,
                            0,
                            0,
                            -1,
                        );
                    }
                }
                let mut ref_idx_l1 = [0i8; 4];
                for sub in 0..4usize {
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    if sub_use_l1[sub] {
                        let idx = if num_ref_idx_l1 > 1 {
                            self.decode_ref_idx(
                                cabac,
                                ctxs,
                                num_ref_idx_l1,
                                1,
                                mb_x * 4 + (sub & 1) * 2,
                                mb_y * 4 + (sub >> 1) * 2,
                                true,
                            )
                        } else {
                            0
                        };
                        ref_idx_l1[sub] = idx.min(i8::MAX as u32) as i8;
                        self.set_l1_motion_block_4x4(
                            mb_x * 16 + sx,
                            mb_y * 16 + sy,
                            8,
                            8,
                            0,
                            0,
                            ref_idx_l1[sub],
                        );
                    } else {
                        self.set_l1_motion_block_4x4(
                            mb_x * 16 + sx,
                            mb_y * 16 + sy,
                            8,
                            8,
                            0,
                            0,
                            -1,
                        );
                    }
                }

                let mut mvd_l0_x = [[0i32; 4]; 4];
                let mut mvd_l0_y = [[0i32; 4]; 4];
                let mut mvd_l1_x = [[0i32; 4]; 4];
                let mut mvd_l1_y = [[0i32; 4]; 4];

                let part_offset =
                    |part_w: usize, part_h: usize, part_count: usize, part: usize| match (
                        part_w, part_h, part_count,
                    ) {
                        (8, 8, _) => (0usize, 0usize),
                        (8, 4, _) => (0usize, part * 4),
                        (4, 8, _) => (part * 4, 0usize),
                        _ => ((part & 1) * 4, (part >> 1) * 4),
                    };

                for sub in 0..4usize {
                    if !sub_use_l0[sub] {
                        continue;
                    }
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    for part in 0..sub_part_count[sub] {
                        let (off_x, off_y) = part_offset(
                            sub_part_w[sub],
                            sub_part_h[sub],
                            sub_part_count[sub],
                            part,
                        );
                        let x4 = mb_x * 4 + (sx + off_x) / 4;
                        let y4 = mb_y * 4 + (sy + off_y) / 4;
                        let (amvd_x, amvd_y) = self.compute_cabac_amvd(x4, y4, 0);
                        let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                        let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                        self.set_mvd_block_4x4(
                            mb_x * 16 + sx + off_x,
                            mb_y * 16 + sy + off_y,
                            sub_part_w[sub],
                            sub_part_h[sub],
                            mvd_x,
                            mvd_y,
                            0,
                        );
                        mvd_l0_x[sub][part] = mvd_x;
                        mvd_l0_y[sub][part] = mvd_y;
                    }
                }
                for sub in 0..4usize {
                    if !sub_use_l1[sub] {
                        continue;
                    }
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    for part in 0..sub_part_count[sub] {
                        let (off_x, off_y) = part_offset(
                            sub_part_w[sub],
                            sub_part_h[sub],
                            sub_part_count[sub],
                            part,
                        );
                        let x4 = mb_x * 4 + (sx + off_x) / 4;
                        let y4 = mb_y * 4 + (sy + off_y) / 4;
                        let (amvd_x, amvd_y) = self.compute_cabac_amvd(x4, y4, 1);
                        // 对齐 FFmpeg decode_cabac_mb_mvd: list1 需要在 list0 基址上 +4.
                        let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 44, amvd_x);
                        let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 51, amvd_y);
                        self.set_mvd_block_4x4(
                            mb_x * 16 + sx + off_x,
                            mb_y * 16 + sy + off_y,
                            sub_part_w[sub],
                            sub_part_h[sub],
                            mvd_x,
                            mvd_y,
                            1,
                        );
                        mvd_l1_x[sub][part] = mvd_x;
                        mvd_l1_y[sub][part] = mvd_y;
                    }
                }

                for sub in 0..4usize {
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    if matches!(sub_dir[sub], BPredDir::Direct) {
                        self.set_direct_block_4x4(mb_x * 16 + sx, mb_y * 16 + sy, 8, 8, true);
                        let (mv_x, mv_y, ref_idx) = self.apply_b_direct_sub_8x8(
                            mb_x,
                            mb_y,
                            sx,
                            sy,
                            pred_mv_x,
                            pred_mv_y,
                            direct_spatial_mv_pred_flag,
                            l0_weights,
                            l1_weights,
                            luma_log2_weight_denom,
                            chroma_log2_weight_denom,
                            ref_l0_list,
                            ref_l1_list,
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx;
                        continue;
                    }

                    for part in 0..sub_part_count[sub] {
                        let (off_x, off_y) = part_offset(
                            sub_part_w[sub],
                            sub_part_h[sub],
                            sub_part_count[sub],
                            part,
                        );
                        let bpx_x = mb_x * 16 + sx + off_x;
                        let bpx_y = mb_y * 16 + sy + off_y;
                        let part_x4 = (sx + off_x) / 4;
                        let part_y4 = (sy + off_y) / 4;
                        let part_w4 = (sub_part_w[sub] / 4).max(1);

                        let motion_l0 = if sub_use_l0[sub] {
                            let (pred_x, pred_y) = self.predict_mv_l0_partition(
                                mb_x,
                                mb_y,
                                part_x4,
                                part_y4,
                                part_w4,
                                ref_idx_l0[sub],
                            );
                            Some(BMotion {
                                mv_x: pred_x + mvd_l0_x[sub][part],
                                mv_y: pred_y + mvd_l0_y[sub][part],
                                ref_idx: ref_idx_l0[sub],
                            })
                        } else {
                            None
                        };
                        let motion_l1 = if sub_use_l1[sub] {
                            let (pred_x, pred_y) = self.predict_mv_l1_partition(
                                mb_x,
                                mb_y,
                                part_x4,
                                part_y4,
                                part_w4,
                                ref_idx_l1[sub],
                            );
                            Some(BMotion {
                                mv_x: pred_x + mvd_l1_x[sub][part],
                                mv_y: pred_y + mvd_l1_y[sub][part],
                                ref_idx: ref_idx_l1[sub],
                            })
                        } else {
                            None
                        };

                        let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                            motion_l0,
                            motion_l1,
                            l0_weights,
                            l1_weights,
                            luma_log2_weight_denom,
                            chroma_log2_weight_denom,
                            ref_l0_list,
                            ref_l1_list,
                            bpx_x,
                            bpx_y,
                            sub_part_w[sub],
                            sub_part_h[sub],
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx;
                    }
                }
            }
            Some(ty) => {
                self.mb_types[mb_idx] = 210u8.saturating_add(ty.min(40));
                if let Some((shape, dir0, dir1)) = Self::b_mb_partition_info(ty) {
                    let part_count = if shape == 0 { 1 } else { 2 };
                    let mut part_dirs = [BPredDir::Direct; 2];
                    let mut part_w = [16usize; 2];
                    let mut part_h = [16usize; 2];
                    let mut part_off_x = [0usize; 2];
                    let mut part_off_y = [0usize; 2];
                    let mut part_x4 = [0usize; 2];
                    let mut part_y4 = [0usize; 2];
                    let mut part_w4 = [4usize; 2];
                    let mut part_use_l0 = [false; 2];
                    let mut part_use_l1 = [false; 2];

                    for part in 0..part_count {
                        let dir = if part == 0 { dir0 } else { dir1 };
                        let (w, h, off_x, off_y) = match shape {
                            0 => (16usize, 16usize, 0usize, 0usize),
                            1 => (16usize, 8usize, 0usize, part * 8),
                            _ => (8usize, 16usize, part * 8, 0usize),
                        };
                        part_dirs[part] = dir;
                        part_w[part] = w;
                        part_h[part] = h;
                        part_off_x[part] = off_x;
                        part_off_y[part] = off_y;
                        part_x4[part] = off_x / 4;
                        part_y4[part] = off_y / 4;
                        part_w4[part] = (w / 4).max(1);
                        part_use_l0[part] = matches!(dir, BPredDir::L0 | BPredDir::Bi);
                        part_use_l1[part] = matches!(dir, BPredDir::L1 | BPredDir::Bi);
                    }

                    let mut ref_idx_l0 = [0i8; 2];
                    for part in 0..part_count {
                        let px = mb_x * 16 + part_off_x[part];
                        let py = mb_y * 16 + part_off_y[part];
                        if part_use_l0[part] {
                            let idx = if num_ref_idx_l0 > 1 {
                                self.decode_ref_idx(
                                    cabac,
                                    ctxs,
                                    num_ref_idx_l0,
                                    0,
                                    mb_x * 4 + part_x4[part],
                                    mb_y * 4 + part_y4[part],
                                    true,
                                )
                            } else {
                                0
                            };
                            ref_idx_l0[part] = idx.min(i8::MAX as u32) as i8;
                            self.set_l0_motion_block_4x4(
                                px,
                                py,
                                part_w[part],
                                part_h[part],
                                0,
                                0,
                                ref_idx_l0[part],
                            );
                        } else {
                            self.set_l0_motion_block_4x4(
                                px,
                                py,
                                part_w[part],
                                part_h[part],
                                0,
                                0,
                                -1,
                            );
                        }
                    }
                    let mut ref_idx_l1 = [0i8; 2];
                    for part in 0..part_count {
                        let px = mb_x * 16 + part_off_x[part];
                        let py = mb_y * 16 + part_off_y[part];
                        if part_use_l1[part] {
                            let idx = if num_ref_idx_l1 > 1 {
                                self.decode_ref_idx(
                                    cabac,
                                    ctxs,
                                    num_ref_idx_l1,
                                    1,
                                    mb_x * 4 + part_x4[part],
                                    mb_y * 4 + part_y4[part],
                                    true,
                                )
                            } else {
                                0
                            };
                            ref_idx_l1[part] = idx.min(i8::MAX as u32) as i8;
                            self.set_l1_motion_block_4x4(
                                px,
                                py,
                                part_w[part],
                                part_h[part],
                                0,
                                0,
                                ref_idx_l1[part],
                            );
                        } else {
                            self.set_l1_motion_block_4x4(
                                px,
                                py,
                                part_w[part],
                                part_h[part],
                                0,
                                0,
                                -1,
                            );
                        }
                    }

                    let mut motion_l0 = [None; 2];
                    let mut motion_l1 = [None; 2];
                    for part in 0..part_count {
                        if part_use_l0[part] {
                            let x4 = mb_x * 4 + part_x4[part];
                            let y4 = mb_y * 4 + part_y4[part];
                            let (pred_x, pred_y) = match shape {
                                1 => self.predict_mv_l0_16x8(mb_x, mb_y, part, ref_idx_l0[part]),
                                2 => self.predict_mv_l0_8x16(mb_x, mb_y, part, ref_idx_l0[part]),
                                _ => self.predict_mv_l0_partition(
                                    mb_x,
                                    mb_y,
                                    part_x4[part],
                                    part_y4[part],
                                    part_w4[part],
                                    ref_idx_l0[part],
                                ),
                            };
                            let (amvd_x, amvd_y) = self.compute_cabac_amvd(x4, y4, 0);
                            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, amvd_x);
                            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, amvd_y);
                            self.set_mvd_block_4x4(
                                mb_x * 16 + part_off_x[part],
                                mb_y * 16 + part_off_y[part],
                                part_w[part],
                                part_h[part],
                                mvd_x,
                                mvd_y,
                                0,
                            );
                            motion_l0[part] = Some(BMotion {
                                mv_x: pred_x + mvd_x,
                                mv_y: pred_y + mvd_y,
                                ref_idx: ref_idx_l0[part],
                            });
                        }
                    }
                    for part in 0..part_count {
                        if part_use_l1[part] {
                            let x4 = mb_x * 4 + part_x4[part];
                            let y4 = mb_y * 4 + part_y4[part];
                            let (pred_x, pred_y) = match shape {
                                1 => self.predict_mv_l1_16x8(mb_x, mb_y, part, ref_idx_l1[part]),
                                2 => self.predict_mv_l1_8x16(mb_x, mb_y, part, ref_idx_l1[part]),
                                _ => self.predict_mv_l1_partition(
                                    mb_x,
                                    mb_y,
                                    part_x4[part],
                                    part_y4[part],
                                    part_w4[part],
                                    ref_idx_l1[part],
                                ),
                            };
                            let (amvd_x, amvd_y) = self.compute_cabac_amvd(x4, y4, 1);
                            // 对齐 FFmpeg decode_cabac_mb_mvd: list1 需要在 list0 基址上 +4.
                            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 44, amvd_x);
                            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 51, amvd_y);
                            self.set_mvd_block_4x4(
                                mb_x * 16 + part_off_x[part],
                                mb_y * 16 + part_off_y[part],
                                part_w[part],
                                part_h[part],
                                mvd_x,
                                mvd_y,
                                1,
                            );
                            motion_l1[part] = Some(BMotion {
                                mv_x: pred_x + mvd_x,
                                mv_y: pred_y + mvd_y,
                                ref_idx: ref_idx_l1[part],
                            });
                        }
                    }

                    for part in 0..part_count {
                        let bpx_x = mb_x * 16 + part_off_x[part];
                        let bpx_y = mb_y * 16 + part_off_y[part];
                        let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                            motion_l0[part],
                            motion_l1[part],
                            l0_weights,
                            l1_weights,
                            luma_log2_weight_denom,
                            chroma_log2_weight_denom,
                            ref_l0_list,
                            ref_l1_list,
                            bpx_x,
                            bpx_y,
                            part_w[part],
                            part_h[part],
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx;
                    }
                }
            }
        }

        self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = final_ref_idx;

        let (luma_cbp, chroma_cbp) =
            self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, false);
        let cbp = luma_cbp | (chroma_cbp << 4);
        self.set_mb_cbp(mb_x, mb_y, cbp);

        // H.264 规范 7.3.5.1: transform_size_8x8_flag 必须在 mb_qp_delta 之前解析
        let parsed_use_8x8 = luma_cbp != 0
            && no_sub_mb_part_size_less_than_8x8_flag
            && self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
            && self.decode_transform_size_8x8_flag_inter(cabac, ctxs, mb_x, mb_y);
        let use_8x8_residual = parsed_use_8x8;
        self.set_transform_8x8_flag(mb_x, mb_y, parsed_use_8x8);

        if cbp != 0 {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        if use_8x8_residual {
            self.decode_i8x8_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, false);
        } else {
            self.decode_inter_4x4_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp);
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, false);
        }
    }

    /// 解码并应用互预测 4x4 残差.
    pub(super) fn decode_inter_4x4_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
    ) {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(false);
        let transform_bypass = self.is_transform_bypass_active(qp);
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }

        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let has_residual_8x8 = luma_cbp & (1 << i8x8) != 0;
            let mut coded_8x8 = false;

            for i_sub in 0..4 {
                let sub_x = i_sub & 1;
                let sub_y = i_sub >> 1;
                let abs_sub_x = x8x8 * 2 + sub_x;
                let abs_sub_y = y8x8 * 2 + sub_y;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;

                if !has_residual_8x8 {
                    self.set_luma_cbf(x4, y4, false);
                    continue;
                }

                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, false);
                let mut raw_coeffs =
                    decode_residual_block(cabac, ctxs, &residual::CAT_LUMA_4X4, cbf_inc);
                let coded = raw_coeffs.iter().any(|&c| c != 0);
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    coded_8x8 = true;
                }
                while raw_coeffs.len() < 16 {
                    raw_coeffs.push(0);
                }

                let mut coeffs_arr = [0i32; 16];
                coeffs_arr.copy_from_slice(&raw_coeffs[..16]);
                if transform_bypass {
                    residual::apply_4x4_bypass_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        mb_x * 16 + abs_sub_x * 4,
                        mb_y * 16 + abs_sub_y * 4,
                        &coeffs_arr,
                    );
                } else {
                    residual::dequant_4x4_ac_with_scaling(&mut coeffs_arr, qp, &luma_scaling_4x4);
                    residual::apply_4x4_ac_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        mb_x * 16 + abs_sub_x * 4,
                        mb_y * 16 + abs_sub_y * 4,
                        &coeffs_arr,
                    );
                }
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }
}
