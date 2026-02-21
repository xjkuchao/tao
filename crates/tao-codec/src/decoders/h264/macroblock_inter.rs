use super::*;

impl H264Decoder {
    fn direct_neighbor_mv_for_list(&self, mb_idx: usize, list1: bool) -> Option<(i32, i32)> {
        if list1 {
            if self.ref_idx_l1.get(mb_idx).copied().unwrap_or(-1) < 0 {
                return None;
            }
            Some((
                self.mv_l1_x.get(mb_idx).copied().unwrap_or(0) as i32,
                self.mv_l1_y.get(mb_idx).copied().unwrap_or(0) as i32,
            ))
        } else {
            if self.ref_idx_l0.get(mb_idx).copied().unwrap_or(-1) < 0 {
                return None;
            }
            Some((
                self.mv_l0_x.get(mb_idx).copied().unwrap_or(0) as i32,
                self.mv_l0_y.get(mb_idx).copied().unwrap_or(0) as i32,
            ))
        }
    }

    fn is_zero_direct_neighbor_mb(&self, mb_idx: usize) -> bool {
        let l0_zero = self.ref_idx_l0.get(mb_idx).copied().unwrap_or(-1) == 0
            && self.mv_l0_x.get(mb_idx).copied().unwrap_or(0) == 0
            && self.mv_l0_y.get(mb_idx).copied().unwrap_or(0) == 0;
        let l1_zero = self.ref_idx_l1.get(mb_idx).copied().unwrap_or(-1) == 0
            && self.mv_l1_x.get(mb_idx).copied().unwrap_or(0) == 0
            && self.mv_l1_y.get(mb_idx).copied().unwrap_or(0) == 0;
        l0_zero && l1_zero
    }

    fn spatial_direct_neighbor_mb_indices(
        &self,
        mb_x: usize,
        mb_y: usize,
    ) -> (Option<usize>, Option<usize>, Option<usize>) {
        let left = if mb_x > 0 {
            self.mb_index(mb_x - 1, mb_y)
        } else {
            None
        };
        let top = if mb_y > 0 {
            self.mb_index(mb_x, mb_y - 1)
        } else {
            None
        };
        let diag = if mb_x + 1 < self.mb_width && mb_y > 0 {
            self.mb_index(mb_x + 1, mb_y - 1)
        } else if mb_x > 0 && mb_y > 0 {
            self.mb_index(mb_x - 1, mb_y - 1)
        } else {
            None
        };
        (left, top, diag)
    }

    fn spatial_direct_zero_mv_condition(&self, mb_x: usize, mb_y: usize) -> bool {
        let (left, top, diag) = self.spatial_direct_neighbor_mb_indices(mb_x, mb_y);
        let mut has_neighbor = false;
        for idx in [left, top, diag].into_iter().flatten() {
            has_neighbor = true;
            if !self.is_zero_direct_neighbor_mb(idx) {
                return false;
            }
        }
        has_neighbor
    }

    fn predict_spatial_direct_mv_for_list(
        &self,
        mb_x: usize,
        mb_y: usize,
        list1: bool,
        fallback_mv_x: i32,
        fallback_mv_y: i32,
    ) -> (i32, i32) {
        let (left, top, diag) = self.spatial_direct_neighbor_mb_indices(mb_x, mb_y);
        let cand_a = left.and_then(|idx| self.direct_neighbor_mv_for_list(idx, list1));
        let cand_b = top.and_then(|idx| self.direct_neighbor_mv_for_list(idx, list1));
        let cand_c = diag.and_then(|idx| self.direct_neighbor_mv_for_list(idx, list1));

        let mut matched = [(0i32, 0i32); 3];
        let mut count = 0usize;
        for cand in [cand_a, cand_b, cand_c].into_iter().flatten() {
            matched[count] = cand;
            count += 1;
        }
        if count == 0 {
            return (fallback_mv_x, fallback_mv_y);
        }
        if count == 1 {
            return matched[0];
        }
        let a = matched[0];
        let b = matched[1];
        let c = if count == 3 { matched[2] } else { matched[1] };
        (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1))
    }

    fn find_reference_picture_for_planes(&self, planes: &RefPlanes) -> Option<&ReferencePicture> {
        self.reference_frames.iter().rev().find(|pic| {
            pic.poc == planes.poc && (pic.long_term_frame_idx.is_some() == planes.is_long_term)
        })
    }

    fn temporal_direct_colocated_l0_motion(
        &self,
        mb_x: usize,
        mb_y: usize,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) -> Option<(i32, i32)> {
        let mb_idx = self.mb_index(mb_x, mb_y)?;
        for col_planes in [ref_l1_list.first(), ref_l0_list.first()]
            .into_iter()
            .flatten()
        {
            let Some(col_pic) = self.find_reference_picture_for_planes(col_planes) else {
                continue;
            };
            let Some(&ref_idx) = col_pic.ref_idx_l0.get(mb_idx) else {
                continue;
            };
            if ref_idx < 0 {
                continue;
            }
            let Some(&mv_x) = col_pic.mv_l0_x.get(mb_idx) else {
                continue;
            };
            let Some(&mv_y) = col_pic.mv_l0_y.get(mb_idx) else {
                continue;
            };
            return Some((mv_x as i32, mv_y as i32));
        }
        None
    }

    /// 构建 B-slice Direct 预测的最小运动信息.
    ///
    /// temporal direct 路径按 list1[0] 优先定位共定位宏块并读取其 list0 MV,
    /// 若共定位信息不可用则回退输入预测; spatial direct 路径使用 list0/list1 双向预测.
    ///
    /// spatial direct 最小实现:
    /// - 当左/上邻居都存在且二者均为 list0/list1 的 `ref_idx=0 && mv=(0,0)` 时, 直接输出零 MV.
    /// - 其它情况: L0/L1 均独立使用邻居预测(缺失时回退输入预测 MV).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_b_direct_motion(
        &self,
        mb_x: usize,
        mb_y: usize,
        mv_x: i32,
        mv_y: i32,
        direct_spatial_mv_pred_flag: bool,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) -> (Option<BMotion>, Option<BMotion>) {
        let (direct_l0_mv_x, direct_l0_mv_y, direct_l1_mv_x, direct_l1_mv_y) =
            if direct_spatial_mv_pred_flag && self.spatial_direct_zero_mv_condition(mb_x, mb_y) {
                (0, 0, 0, 0)
            } else if direct_spatial_mv_pred_flag {
                let (l0_mv_x, l0_mv_y) =
                    self.predict_spatial_direct_mv_for_list(mb_x, mb_y, false, mv_x, mv_y);
                let (l1_mv_x, l1_mv_y) =
                    self.predict_spatial_direct_mv_for_list(mb_x, mb_y, true, mv_x, mv_y);
                (l0_mv_x, l0_mv_y, l1_mv_x, l1_mv_y)
            } else {
                // Temporal Direct 最小实现:
                // 先定位共定位宏块并读取其 list0 MV, 缩放仍暂用 dist_scale_factor=256.
                let (col_mv_x, col_mv_y) = self
                    .temporal_direct_colocated_l0_motion(mb_x, mb_y, ref_l0_list, ref_l1_list)
                    .unwrap_or((mv_x, mv_y));
                let (l0_mv_x, _) = self.scale_temporal_direct_mv_pair_component(col_mv_x, 256);
                let (l0_mv_y, _) = self.scale_temporal_direct_mv_pair_component(col_mv_y, 256);
                (l0_mv_x, l0_mv_y, l0_mv_x, l0_mv_y)
            };
        let motion_l0 = Some(BMotion {
            mv_x: direct_l0_mv_x,
            mv_y: direct_l0_mv_y,
            ref_idx: 0,
        });
        let motion_l1 = if direct_spatial_mv_pred_flag {
            Some(BMotion {
                mv_x: direct_l1_mv_x,
                mv_y: direct_l1_mv_y,
                ref_idx: 0,
            })
        } else {
            None
        };
        (motion_l0, motion_l1)
    }

    pub(super) fn direct_8x8_inference_enabled(&self) -> bool {
        self.sps
            .as_ref()
            .map(|sps| sps.direct_8x8_inference_flag)
            .unwrap_or(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_b_direct_sub_8x8(
        &mut self,
        mb_x: usize,
        mb_y: usize,
        sub_x: usize,
        sub_y: usize,
        pred_mv_x: i32,
        pred_mv_y: i32,
        direct_spatial_mv_pred_flag: bool,
        l0_weights: &[PredWeightL0],
        l1_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) -> (i32, i32, i8) {
        if self.direct_8x8_inference_enabled() {
            let (motion_l0, motion_l1) = self.build_b_direct_motion(
                mb_x,
                mb_y,
                pred_mv_x,
                pred_mv_y,
                direct_spatial_mv_pred_flag,
                ref_l0_list,
                ref_l1_list,
            );
            return self.apply_b_prediction_block(
                motion_l0,
                motion_l1,
                l0_weights,
                l1_weights,
                luma_log2_weight_denom,
                chroma_log2_weight_denom,
                ref_l0_list,
                ref_l1_list,
                mb_x * 16 + sub_x,
                mb_y * 16 + sub_y,
                8,
                8,
            );
        }

        let base_part_x4 = sub_x / 4;
        let base_part_y4 = sub_y / 4;
        let mut part_pred_mv_x = [[0i32; 2]; 2];
        let mut part_pred_mv_y = [[0i32; 2]; 2];
        for part_y in 0..2usize {
            for part_x in 0..2usize {
                (
                    part_pred_mv_x[part_y][part_x],
                    part_pred_mv_y[part_y][part_x],
                ) = self.predict_mv_l0_partition(
                    mb_x,
                    mb_y,
                    base_part_x4 + part_x,
                    base_part_y4 + part_y,
                    1,
                    0,
                );
            }
        }
        let mut last_mv = (pred_mv_x, pred_mv_y, 0i8);
        for part_y in 0..2usize {
            for part_x in 0..2usize {
                let (motion_l0, motion_l1) = self.build_b_direct_motion(
                    mb_x,
                    mb_y,
                    part_pred_mv_x[part_y][part_x],
                    part_pred_mv_y[part_y][part_x],
                    direct_spatial_mv_pred_flag,
                    ref_l0_list,
                    ref_l1_list,
                );
                last_mv = self.apply_b_prediction_block(
                    motion_l0,
                    motion_l1,
                    l0_weights,
                    l1_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                    ref_l0_list,
                    ref_l1_list,
                    mb_x * 16 + sub_x + part_x * 4,
                    mb_y * 16 + sub_y + part_y * 4,
                    4,
                    4,
                );
            }
        }
        last_mv
    }

    /// 推导 P_Skip 的 L0 运动向量.
    ///
    /// 规则:
    /// - 当左/上邻居都存在且二者均为 `ref_idx=0 且 mv=(0,0)` 时, 返回零向量.
    /// - 其它情况退化为 `ref_idx=0` 的 16x16 MVP.
    pub(super) fn predict_p_skip_mv(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        let left = if mb_x > 0 {
            self.mb_index(mb_x - 1, mb_y)
        } else {
            None
        };
        let top = if mb_y > 0 {
            self.mb_index(mb_x, mb_y - 1)
        } else {
            None
        };
        if let (Some(left_idx), Some(top_idx)) = (left, top) {
            let left_zero = self.ref_idx_l0.get(left_idx).copied().unwrap_or(-1) == 0
                && self.mv_l0_x.get(left_idx).copied().unwrap_or(0) == 0
                && self.mv_l0_y.get(left_idx).copied().unwrap_or(0) == 0;
            let top_zero = self.ref_idx_l0.get(top_idx).copied().unwrap_or(-1) == 0
                && self.mv_l0_x.get(top_idx).copied().unwrap_or(0) == 0
                && self.mv_l0_y.get(top_idx).copied().unwrap_or(0) == 0;
            if left_zero && top_zero {
                return (0, 0);
            }
        }
        self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_p_skip_mb(
        &mut self,
        mb_x: usize,
        mb_y: usize,
        ref_l0_list: &[RefPlanes],
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.mb_types[mb_idx] = 255;
        self.set_mb_cbp(mb_x, mb_y, 0);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        self.set_chroma_pred_mode(mb_x, mb_y, 0);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        let (pred_x, pred_y) = self.predict_p_skip_mv(mb_x, mb_y);
        self.apply_inter_block_l0(
            ref_l0_list,
            0,
            mb_x * 16,
            mb_y * 16,
            16,
            16,
            pred_x,
            pred_y,
            l0_weights,
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
        );
        self.mv_l0_x[mb_idx] = pred_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = pred_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = 0;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_p_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        slice_first_mb: u32,
        num_ref_idx_l0: u32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
    ) {
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;

        for mb_idx in first..total {
            self.mark_mb_slice_first_mb(mb_idx, slice_first_mb);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_p_mb_skip_flag(cabac, ctxs, mb_x, mb_y);

            if skip {
                self.decode_p_skip_mb(
                    mb_x,
                    mb_y,
                    ref_l0_list,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
            } else if let Some(p_mb_type) = self.decode_p_mb_type(cabac, ctxs, mb_x, mb_y) {
                self.decode_p_inter_mb(
                    cabac,
                    ctxs,
                    mb_x,
                    mb_y,
                    p_mb_type,
                    &mut cur_qp,
                    num_ref_idx_l0,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                    ref_l0_list,
                );
            } else {
                let intra_mb_type = decode_intra_mb_type(
                    cabac,
                    ctxs,
                    17,
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
                    self.decode_i_16x16_mb(cabac, ctxs, mb_x, mb_y, intra_mb_type, &mut cur_qp);
                } else if intra_mb_type == 25 {
                    self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                    self.prev_qp_delta_nz = false;
                }
            }

            if mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                break;
            }
        }
    }

    /// 解码 P-slice 的 mb_skip_flag.
    pub(super) fn decode_p_mb_skip_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left_non_skip = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .unwrap_or(255)
                != 255;
        let top_non_skip = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .unwrap_or(255)
                != 255;
        let ctx = usize::from(left_non_skip) + (usize::from(top_non_skip) << 1);
        cabac.decode_decision(&mut ctxs[11 + ctx]) == 1
    }

    /// 解码 P-slice 的 mb_type.
    ///
    /// 返回值:
    /// - `Some(0..=3)`: 互预测类型.
    /// - `None`: Intra 宏块, 需走 intra_mb_type 语法.
    pub(super) fn decode_p_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        _mb_x: usize,
        _mb_y: usize,
    ) -> Option<u8> {
        if cabac.decode_decision(&mut ctxs[14]) == 0 {
            if cabac.decode_decision(&mut ctxs[15]) == 0 {
                let idx = 3 * cabac.decode_decision(&mut ctxs[16]) as u8;
                return Some(idx);
            }
            let idx = 2 - cabac.decode_decision(&mut ctxs[17]) as u8;
            return Some(idx);
        }
        None
    }

    /// 解码 P_8x8 的 sub_mb_type.
    pub(super) fn decode_p_sub_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
    ) -> u8 {
        if cabac.decode_decision(&mut ctxs[21]) == 1 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[22]) == 0 {
            return 1;
        }
        if cabac.decode_decision(&mut ctxs[23]) == 1 {
            2
        } else {
            3
        }
    }

    /// 解码 B-slice 的 mb_skip_flag.
    pub(super) fn decode_b_mb_skip_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left_non_skip = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254 && t != 255)
                .unwrap_or(false);
        let top_non_skip = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254 && t != 255)
                .unwrap_or(false);
        let ctx = usize::from(left_non_skip) + usize::from(top_non_skip);
        cabac.decode_decision(&mut ctxs[24 + ctx]) == 1
    }

    /// 解码 B-slice 的 mb_type.
    pub(super) fn decode_b_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> BMbType {
        let left_direct = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t == 254)
                .unwrap_or(false);
        let top_direct = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t == 254)
                .unwrap_or(false);

        let mut ctx = 0usize;
        if !left_direct {
            ctx += 1;
        }
        if !top_direct {
            ctx += 1;
        }

        if cabac.decode_decision(&mut ctxs[27 + ctx]) == 0 {
            return BMbType::Direct;
        }
        if cabac.decode_decision(&mut ctxs[30]) == 0 {
            let idx = 1 + cabac.decode_decision(&mut ctxs[32]) as u8;
            return BMbType::Inter(idx);
        }

        let mut bits = (cabac.decode_decision(&mut ctxs[31]) as u8) << 3;
        bits |= (cabac.decode_decision(&mut ctxs[32]) as u8) << 2;
        bits |= (cabac.decode_decision(&mut ctxs[32]) as u8) << 1;
        bits |= cabac.decode_decision(&mut ctxs[32]) as u8;

        if bits < 8 {
            return BMbType::Inter(bits + 3);
        }
        if bits == 13 {
            return BMbType::Intra;
        }
        if bits == 14 {
            return BMbType::Inter(11);
        }
        if bits == 15 {
            return BMbType::Inter(22);
        }

        bits = (bits << 1) | cabac.decode_decision(&mut ctxs[32]) as u8;
        BMbType::Inter(bits - 4)
    }

    /// 解码 B_8x8 的 sub_mb_type.
    pub(super) fn decode_b_sub_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
    ) -> u8 {
        if cabac.decode_decision(&mut ctxs[36]) == 0 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[37]) == 0 {
            return 1 + cabac.decode_decision(&mut ctxs[39]) as u8;
        }
        let mut ty = 3u8;
        if cabac.decode_decision(&mut ctxs[38]) == 1 {
            if cabac.decode_decision(&mut ctxs[39]) == 1 {
                return 11 + cabac.decode_decision(&mut ctxs[39]) as u8;
            }
            ty += 4;
        }
        ty += (cabac.decode_decision(&mut ctxs[39]) as u8) << 1;
        ty += cabac.decode_decision(&mut ctxs[39]) as u8;
        ty
    }

    pub(super) fn b_mb_partition_info(mb_type_idx: u8) -> Option<(u8, BPredDir, BPredDir)> {
        match mb_type_idx {
            1 => Some((0, BPredDir::L0, BPredDir::Direct)),
            2 => Some((0, BPredDir::L1, BPredDir::Direct)),
            3 => Some((0, BPredDir::Bi, BPredDir::Direct)),
            4 => Some((1, BPredDir::L0, BPredDir::L0)),
            5 => Some((2, BPredDir::L0, BPredDir::L0)),
            6 => Some((1, BPredDir::L1, BPredDir::L1)),
            7 => Some((2, BPredDir::L1, BPredDir::L1)),
            8 => Some((1, BPredDir::L0, BPredDir::L1)),
            9 => Some((2, BPredDir::L0, BPredDir::L1)),
            10 => Some((1, BPredDir::L1, BPredDir::L0)),
            11 => Some((2, BPredDir::L1, BPredDir::L0)),
            12 => Some((1, BPredDir::L0, BPredDir::Bi)),
            13 => Some((2, BPredDir::L0, BPredDir::Bi)),
            14 => Some((1, BPredDir::L1, BPredDir::Bi)),
            15 => Some((2, BPredDir::L1, BPredDir::Bi)),
            16 => Some((1, BPredDir::Bi, BPredDir::L0)),
            17 => Some((2, BPredDir::Bi, BPredDir::L0)),
            18 => Some((1, BPredDir::Bi, BPredDir::L1)),
            19 => Some((2, BPredDir::Bi, BPredDir::L1)),
            20 => Some((1, BPredDir::Bi, BPredDir::Bi)),
            21 => Some((2, BPredDir::Bi, BPredDir::Bi)),
            _ => None,
        }
    }

    pub(super) fn b_sub_mb_info(sub_mb_type: u8) -> (usize, usize, usize, BPredDir) {
        match sub_mb_type {
            0 => (8, 8, 1, BPredDir::Direct),
            1 => (8, 8, 1, BPredDir::L0),
            2 => (8, 8, 1, BPredDir::L1),
            3 => (8, 8, 1, BPredDir::Bi),
            4 => (8, 4, 2, BPredDir::L0),
            5 => (4, 8, 2, BPredDir::L0),
            6 => (8, 4, 2, BPredDir::L1),
            7 => (4, 8, 2, BPredDir::L1),
            8 => (8, 4, 2, BPredDir::Bi),
            9 => (4, 8, 2, BPredDir::Bi),
            10 => (4, 4, 4, BPredDir::L0),
            11 => (4, 4, 4, BPredDir::L1),
            12 => (4, 4, 4, BPredDir::Bi),
            _ => (8, 8, 1, BPredDir::Direct),
        }
    }

    pub(super) fn decode_ref_idx_l0(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        num_ref_idx_l0: u32,
    ) -> u32 {
        if num_ref_idx_l0 <= 1 {
            return 0;
        }
        let mut ref_idx = 0u32;
        let mut ctx = 0usize;
        while cabac.decode_decision(&mut ctxs[54 + ctx]) == 1 {
            ref_idx += 1;
            ctx = (ctx >> 2) + 4;
            if ref_idx + 1 >= num_ref_idx_l0 {
                break;
            }
            if ref_idx >= 31 {
                break;
            }
        }
        ref_idx
    }

    pub(super) fn decode_mb_mvd_component(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        ctx_base: usize,
        amvd: i32,
    ) -> i32 {
        let ctx_inc = if amvd > 32 {
            2usize
        } else if amvd > 2 {
            1usize
        } else {
            0usize
        };
        if cabac.decode_decision(&mut ctxs[ctx_base + ctx_inc]) == 0 {
            return 0;
        }

        let mut mvd = 1i32;
        let mut ctx = ctx_base + 3;
        while mvd < 9 && cabac.decode_decision(&mut ctxs[ctx]) == 1 {
            if mvd < 4 {
                ctx += 1;
            }
            mvd += 1;
        }

        if mvd >= 9 {
            let mut k = 3i32;
            while cabac.decode_bypass() == 1 && k < 24 {
                mvd += 1 << k;
                k += 1;
            }
            while k > 0 {
                k -= 1;
                mvd += (cabac.decode_bypass() as i32) << k;
            }
        }

        if cabac.decode_bypass() == 1 {
            -mvd
        } else {
            mvd
        }
    }
}
