use super::*;

impl H264Decoder {
    pub(super) fn weighted_pred_disabled_by_env() -> bool {
        std::env::var("TAO_H264_DISABLE_WEIGHTED_PRED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    pub(super) fn l0_motion_candidate_4x4(&self, x4: isize, y4: isize) -> Option<(i32, i32, i8)> {
        if x4 < 0 || y4 < 0 {
            return None;
        }
        let x4 = x4 as usize;
        let y4 = y4 as usize;
        let idx = self.motion_l0_4x4_index(x4, y4)?;
        let ref_idx = *self.ref_idx_l0_4x4.get(idx)?;
        if ref_idx < 0 {
            return None;
        }
        let mv_x = *self.mv_l0_x_4x4.get(idx)? as i32;
        let mv_y = *self.mv_l0_y_4x4.get(idx)? as i32;
        Some((mv_x, mv_y, ref_idx))
    }

    pub(super) fn l1_motion_candidate_4x4(&self, x4: isize, y4: isize) -> Option<(i32, i32, i8)> {
        if x4 < 0 || y4 < 0 {
            return None;
        }
        let x4 = x4 as usize;
        let y4 = y4 as usize;
        let idx = self.motion_l1_4x4_index(x4, y4)?;
        let ref_idx = *self.ref_idx_l1_4x4.get(idx)?;
        if ref_idx < 0 {
            return None;
        }
        let mv_x = *self.mv_l1_x_4x4.get(idx)? as i32;
        let mv_y = *self.mv_l1_y_4x4.get(idx)? as i32;
        Some((mv_x, mv_y, ref_idx))
    }

    pub(super) fn predict_mv_l0_partition(
        &self,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
        part_w4: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4 + part_x4;
        let y4 = mb_y * 4 + part_y4;

        let mut cand_a = self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize);
        if let Some(_) = cand_a
            && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4)
        {
            cand_a = None;
        }
        let mut cand_b = self.l0_motion_candidate_4x4(x4 as isize, y4 as isize - 1);
        if let Some(_) = cand_b
            && !self.same_slice_4x4(x4, y4, x4, y4.saturating_sub(1))
        {
            cand_b = None;
        }
        let mut cand_c = self.l0_motion_candidate_4x4((x4 + part_w4) as isize, y4 as isize - 1);
        if let Some(_) = cand_c
            && !self.same_slice_4x4(x4, y4, x4 + part_w4, y4.saturating_sub(1))
        {
            cand_c = None;
        }
        if cand_c.is_none() {
            let mut cand_d = self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize - 1);
            if let Some(_) = cand_d
                && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4.saturating_sub(1))
            {
                cand_d = None;
            }
            cand_c = cand_d;
        }

        let mut matched = [(0i32, 0i32); 3];
        let mut matched_count = 0usize;
        for cand in [cand_a, cand_b, cand_c].into_iter().flatten() {
            if cand.2 == ref_idx {
                matched[matched_count] = (cand.0, cand.1);
                matched_count += 1;
            }
        }

        if matched_count == 1 {
            return matched[0];
        }

        // 对齐 ffmpeg pred_motion: 不可用邻居使用 (0,0), 非级联默认值.
        let a = cand_a.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        let b = cand_b.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        let c = cand_c.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        // 当匹配邻居数量 >=2 时, 仍应按原始 A/B/C 取中值, 而非仅在匹配集合内取中值.
        if matched_count >= 2 {
            return (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1));
        }
        // spec: 仅 A 可用 (B/C 都不可用) 时直接返回 A.
        if cand_b.is_none() && cand_c.is_none() && cand_a.is_some() {
            return a;
        }
        (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1))
    }

    pub(super) fn predict_mv_l0_16x16(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, 0)
    }

    /// L1 版本的 MV 中值预测, 使用 L1 邻居运动信息.
    pub(super) fn predict_mv_l1_partition(
        &self,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
        part_w4: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4 + part_x4;
        let y4 = mb_y * 4 + part_y4;

        let mut cand_a = self.l1_motion_candidate_4x4(x4 as isize - 1, y4 as isize);
        if let Some(_) = cand_a
            && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4)
        {
            cand_a = None;
        }
        let mut cand_b = self.l1_motion_candidate_4x4(x4 as isize, y4 as isize - 1);
        if let Some(_) = cand_b
            && !self.same_slice_4x4(x4, y4, x4, y4.saturating_sub(1))
        {
            cand_b = None;
        }
        let mut cand_c = self.l1_motion_candidate_4x4((x4 + part_w4) as isize, y4 as isize - 1);
        if let Some(_) = cand_c
            && !self.same_slice_4x4(x4, y4, x4 + part_w4, y4.saturating_sub(1))
        {
            cand_c = None;
        }
        if cand_c.is_none() {
            let mut cand_d = self.l1_motion_candidate_4x4(x4 as isize - 1, y4 as isize - 1);
            if let Some(_) = cand_d
                && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4.saturating_sub(1))
            {
                cand_d = None;
            }
            cand_c = cand_d;
        }

        let mut matched = [(0i32, 0i32); 3];
        let mut matched_count = 0usize;
        for cand in [cand_a, cand_b, cand_c].into_iter().flatten() {
            if cand.2 == ref_idx {
                matched[matched_count] = (cand.0, cand.1);
                matched_count += 1;
            }
        }

        if matched_count == 1 {
            return matched[0];
        }

        // 对齐 ffmpeg pred_motion: 不可用邻居使用 (0,0), 非级联默认值.
        let a = cand_a.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        let b = cand_b.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        let c = cand_c.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        // 当匹配邻居数量 >=2 时, 仍应按原始 A/B/C 取中值, 而非仅在匹配集合内取中值.
        if matched_count >= 2 {
            return (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1));
        }
        // spec: 仅 A 可用 (B/C 都不可用) 时直接返回 A.
        if cand_b.is_none() && cand_c.is_none() && cand_a.is_some() {
            return a;
        }
        (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1))
    }

    /// 16x8 分区的方向性 MV 预测 (对标 FFmpeg pred_16x8_motion).
    ///
    /// - part=0 (上半): 优先使用上邻居 MV (若 ref 匹配)
    /// - part=1 (下半): 优先使用左邻居 MV (若 ref 匹配)
    /// - 不匹配时回退到通用 median 预测
    pub(super) fn predict_mv_l0_16x8(
        &self,
        mb_x: usize,
        mb_y: usize,
        part: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4;
        let y4 = mb_y * 4 + part * 2;
        if part == 0 {
            if let Some((mv_x, mv_y, top_ref)) =
                self.l0_motion_candidate_4x4(x4 as isize, y4 as isize - 1)
            {
                if top_ref == ref_idx && self.same_slice_4x4(x4, y4, x4, y4.saturating_sub(1)) {
                    return (mv_x, mv_y);
                }
            }
        } else if let Some((mv_x, mv_y, left_ref)) =
            self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize)
        {
            if left_ref == ref_idx && self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4) {
                return (mv_x, mv_y);
            }
        }
        self.predict_mv_l0_partition(mb_x, mb_y, 0, part * 2, 4, ref_idx)
    }

    /// 8x16 分区的方向性 MV 预测 (对标 FFmpeg pred_8x16_motion).
    ///
    /// - part=0 (左半): 优先使用左邻居 MV (若 ref 匹配)
    /// - part=1 (右半): 优先使用对角邻居 MV (若 ref 匹配)
    /// - 不匹配时回退到通用 median 预测
    pub(super) fn predict_mv_l0_8x16(
        &self,
        mb_x: usize,
        mb_y: usize,
        part: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4 + part * 2;
        let y4 = mb_y * 4;
        if part == 0 {
            if let Some((mv_x, mv_y, left_ref)) =
                self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize)
            {
                if left_ref == ref_idx && self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4) {
                    return (mv_x, mv_y);
                }
            }
        } else {
            let mut diag =
                self.l0_motion_candidate_4x4((x4 + 2) as isize, y4 as isize - 1);
            if diag.is_some()
                && !self.same_slice_4x4(x4, y4, x4 + 2, y4.saturating_sub(1))
            {
                diag = None;
            }
            if diag.is_none() {
                diag = self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize - 1);
                if diag.is_some()
                    && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4.saturating_sub(1))
                {
                    diag = None;
                }
            }
            if let Some((mv_x, mv_y, diag_ref)) = diag
                && diag_ref == ref_idx
            {
                return (mv_x, mv_y);
            }
        }
        self.predict_mv_l0_partition(mb_x, mb_y, part * 2, 0, 2, ref_idx)
    }

    /// L1 版本的 16x8 方向性 MV 预测 (对标 FFmpeg pred_16x8_motion with list=1).
    pub(super) fn predict_mv_l1_16x8(
        &self,
        mb_x: usize,
        mb_y: usize,
        part: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4;
        let y4 = mb_y * 4 + part * 2;
        if part == 0 {
            if let Some((mv_x, mv_y, top_ref)) =
                self.l1_motion_candidate_4x4(x4 as isize, y4 as isize - 1)
            {
                if top_ref == ref_idx && self.same_slice_4x4(x4, y4, x4, y4.saturating_sub(1)) {
                    return (mv_x, mv_y);
                }
            }
        } else if let Some((mv_x, mv_y, left_ref)) =
            self.l1_motion_candidate_4x4(x4 as isize - 1, y4 as isize)
        {
            if left_ref == ref_idx && self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4) {
                return (mv_x, mv_y);
            }
        }
        self.predict_mv_l1_partition(mb_x, mb_y, 0, part * 2, 4, ref_idx)
    }

    /// L1 版本的 8x16 方向性 MV 预测 (对标 FFmpeg pred_8x16_motion with list=1).
    pub(super) fn predict_mv_l1_8x16(
        &self,
        mb_x: usize,
        mb_y: usize,
        part: usize,
        ref_idx: i8,
    ) -> (i32, i32) {
        let x4 = mb_x * 4 + part * 2;
        let y4 = mb_y * 4;
        if part == 0 {
            if let Some((mv_x, mv_y, left_ref)) =
                self.l1_motion_candidate_4x4(x4 as isize - 1, y4 as isize)
            {
                if left_ref == ref_idx && self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4) {
                    return (mv_x, mv_y);
                }
            }
        } else {
            let mut diag =
                self.l1_motion_candidate_4x4((x4 + 2) as isize, y4 as isize - 1);
            if diag.is_some()
                && !self.same_slice_4x4(x4, y4, x4 + 2, y4.saturating_sub(1))
            {
                diag = None;
            }
            if diag.is_none() {
                diag = self.l1_motion_candidate_4x4(x4 as isize - 1, y4 as isize - 1);
                if diag.is_some()
                    && !self.same_slice_4x4(x4, y4, x4.saturating_sub(1), y4.saturating_sub(1))
                {
                    diag = None;
                }
            }
            if let Some((mv_x, mv_y, diag_ref)) = diag
                && diag_ref == ref_idx
            {
                return (mv_x, mv_y);
            }
        }
        self.predict_mv_l1_partition(mb_x, mb_y, part * 2, 0, 2, ref_idx)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_inter_block(
        &mut self,
        src_y: &[u8],
        src_u: &[u8],
        src_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        pred_weight: Option<&PredWeightL0>,
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let luma_src_x = dst_x as i32 + floor_div(mv_x_qpel, 4);
        let luma_src_y = dst_y as i32 + floor_div(mv_y_qpel, 4);
        let luma_fx = mod_floor(mv_x_qpel, 4) as u8;
        let luma_fy = mod_floor(mv_y_qpel, 4) as u8;

        if let Some(weight) = pred_weight {
            weighted_copy_luma_block_with_h264_qpel(
                src_y,
                self.stride_y,
                &mut self.ref_y,
                self.stride_y,
                luma_src_x,
                luma_src_y,
                luma_fx,
                luma_fy,
                dst_x,
                dst_y,
                w,
                h,
                self.stride_y,
                self.mb_height * 16,
                weight.luma_weight,
                weight.luma_offset,
                luma_log2_weight_denom,
            );
        } else {
            copy_luma_block_with_h264_qpel(
                src_y,
                self.stride_y,
                &mut self.ref_y,
                self.stride_y,
                luma_src_x,
                luma_src_y,
                luma_fx,
                luma_fy,
                dst_x,
                dst_y,
                w,
                h,
                self.stride_y,
                self.mb_height * 16,
            );
        }

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let c_src_x = c_dst_x as i32 + floor_div(mv_x_qpel, 8);
        let c_src_y = c_dst_y as i32 + floor_div(mv_y_qpel, 8);
        let c_fx = mod_floor(mv_x_qpel, 8) as u8;
        let c_fy = mod_floor(mv_y_qpel, 8) as u8;
        if let Some(weight) = pred_weight {
            weighted_copy_block_with_qpel_bilinear(
                src_u,
                self.stride_c,
                &mut self.ref_u,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
                weight.chroma_weight[0],
                weight.chroma_offset[0],
                chroma_log2_weight_denom,
            );
            weighted_copy_block_with_qpel_bilinear(
                src_v,
                self.stride_c,
                &mut self.ref_v,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
                weight.chroma_weight[1],
                weight.chroma_offset[1],
                chroma_log2_weight_denom,
            );
        } else {
            copy_block_with_qpel_bilinear(
                src_u,
                self.stride_c,
                &mut self.ref_u,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
            );
            copy_block_with_qpel_bilinear(
                src_v,
                self.stride_c,
                &mut self.ref_v,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_inter_block_l0(
        &mut self,
        ref_l0_list: &[RefPlanes],
        ref_idx: u32,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
        self.set_l0_motion_block_4x4(dst_x, dst_y, w, h, mv_x_qpel, mv_y_qpel, ref_idx_i8);

        let fallback = self.zero_reference_planes();
        let ref_src = if let Ok(ref_idx_i8) = i8::try_from(ref_idx) {
            if let Some(found) = select_ref_planes(ref_l0_list, ref_idx_i8) {
                found
            } else {
                self.record_missing_reference_fallback(
                    "apply_inter_block_l0",
                    ref_idx_i8 as i32,
                    ref_l0_list.len(),
                );
                &fallback
            }
        } else {
            self.record_missing_reference_fallback(
                "apply_inter_block_l0",
                ref_idx as i32,
                ref_l0_list.len(),
            );
            &fallback
        };
        self.apply_inter_block(
            ref_src.y.as_slice(),
            ref_src.u.as_slice(),
            ref_src.v.as_slice(),
            dst_x,
            dst_y,
            w,
            h,
            mv_x_qpel,
            mv_y_qpel,
            if Self::weighted_pred_disabled_by_env() {
                None
            } else {
                p_l0_weight(l0_weights, ref_idx)
            },
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn blend_inter_block(
        &mut self,
        src_y: &[u8],
        src_u: &[u8],
        src_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
    ) {
        let luma_src_x = dst_x as i32 + floor_div(mv_x_qpel, 4);
        let luma_src_y = dst_y as i32 + floor_div(mv_y_qpel, 4);
        let luma_fx = mod_floor(mv_x_qpel, 4) as u8;
        let luma_fy = mod_floor(mv_y_qpel, 4) as u8;
        blend_luma_block_with_h264_qpel(
            src_y,
            self.stride_y,
            &mut self.ref_y,
            self.stride_y,
            luma_src_x,
            luma_src_y,
            luma_fx,
            luma_fy,
            dst_x,
            dst_y,
            w,
            h,
            self.stride_y,
            self.mb_height * 16,
        );

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let c_src_x = c_dst_x as i32 + floor_div(mv_x_qpel, 8);
        let c_src_y = c_dst_y as i32 + floor_div(mv_y_qpel, 8);
        let c_fx = mod_floor(mv_x_qpel, 8) as u8;
        let c_fy = mod_floor(mv_y_qpel, 8) as u8;
        blend_block_with_qpel_bilinear(
            src_u,
            self.stride_c,
            &mut self.ref_u,
            self.stride_c,
            c_src_x,
            c_src_y,
            c_fx,
            c_fy,
            8,
            c_dst_x,
            c_dst_y,
            cw,
            ch,
            self.stride_c,
            self.mb_height * 8,
        );
        blend_block_with_qpel_bilinear(
            src_v,
            self.stride_c,
            &mut self.ref_v,
            self.stride_c,
            c_src_x,
            c_src_y,
            c_fx,
            c_fy,
            8,
            c_dst_x,
            c_dst_y,
            cw,
            ch,
            self.stride_c,
            self.mb_height * 8,
        );
    }

    pub(super) fn implicit_bi_weights(
        &self,
        ref_l0_poc: i32,
        ref_l1_poc: i32,
        ref_l0_is_long_term: bool,
        ref_l1_is_long_term: bool,
    ) -> (i32, i32) {
        if ref_l0_is_long_term || ref_l1_is_long_term {
            return (32, 32);
        }
        let Some(dist_scale_factor) =
            self.temporal_direct_dist_scale_factor(ref_l0_poc, ref_l1_poc)
        else {
            return (32, 32);
        };
        let w1 = dist_scale_factor >> 2;
        if (-64..=128).contains(&w1) {
            (64 - w1, w1)
        } else {
            (32, 32)
        }
    }

    pub(super) fn temporal_direct_dist_scale_factor(
        &self,
        ref_l0_poc: i32,
        ref_l1_poc: i32,
    ) -> Option<i32> {
        let td = (ref_l1_poc - ref_l0_poc).clamp(-128, 127);
        if td == 0 {
            return None;
        }
        let tb = (self.last_poc - ref_l0_poc).clamp(-128, 127);
        let tx = (16384 + (td.abs() >> 1)) / td;
        Some(((tb * tx + 32) >> 6).clamp(-1024, 1023))
    }

    pub(super) fn scale_temporal_direct_mv_component(
        &self,
        col_mv_qpel: i32,
        dist_scale_factor: i32,
    ) -> i32 {
        ((dist_scale_factor * col_mv_qpel + 128) >> 8).clamp(i16::MIN as i32, i16::MAX as i32)
    }

    pub(super) fn scale_temporal_direct_mv_pair_component(
        &self,
        col_mv_qpel: i32,
        dist_scale_factor: i32,
    ) -> (i32, i32) {
        let mv_l0 = self.scale_temporal_direct_mv_component(col_mv_qpel, dist_scale_factor);
        let mv_l1 = (mv_l0 - col_mv_qpel).clamp(i16::MIN as i32, i16::MAX as i32);
        (mv_l0, mv_l1)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_bi_weighted_block(
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
        w0: i32,
        w1: i32,
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
                    let v = ((w0 * px0 + w1 * px1 + 32) >> 6).clamp(0, 255) as u8;
                    self.ref_y[dst_idx] = v;
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
                    self.ref_u[dst_idx] = ((w0 * u0 + w1 * u1 + 32) >> 6).clamp(0, 255) as u8;
                }
                if dst_idx < self.ref_v.len() {
                    self.ref_v[dst_idx] = ((w0 * v0 + w1 * v1 + 32) >> 6).clamp(0, 255) as u8;
                }
            }
        }
    }
}
