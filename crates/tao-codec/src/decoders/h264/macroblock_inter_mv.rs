use super::*;

impl H264Decoder {
    fn l0_motion_candidate_4x4(&self, x4: isize, y4: isize) -> Option<(i32, i32, i8)> {
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

        let cand_a = self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize);
        let cand_b = self.l0_motion_candidate_4x4(x4 as isize, y4 as isize - 1);
        let cand_c = self
            .l0_motion_candidate_4x4((x4 + part_w4) as isize, y4 as isize - 1)
            .or_else(|| self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize - 1));

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
        if matched_count >= 2 {
            let a = matched[0];
            let b = matched[1];
            let c = if matched_count == 3 {
                matched[2]
            } else {
                matched[1]
            };
            return (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1));
        }

        let a = cand_a.map(|(x, y, _)| (x, y)).unwrap_or((0, 0));
        let b = cand_b.map(|(x, y, _)| (x, y)).unwrap_or(a);
        let c = cand_c.map(|(x, y, _)| (x, y)).unwrap_or(b);
        (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1))
    }

    pub(super) fn predict_mv_l0_16x16(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, 0)
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
            p_l0_weight(l0_weights, ref_idx),
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
