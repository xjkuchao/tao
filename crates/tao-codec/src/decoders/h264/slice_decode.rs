use super::*;

// ============================================================
// Slice 解码
// ============================================================

impl H264Decoder {
    #[inline]
    fn qp_for_mb_store(&self, mb_idx: usize, cur_qp: i32) -> i32 {
        if self.mb_types.get(mb_idx).copied() == Some(25) {
            0
        } else {
            cur_qp
        }
    }

    /// 解码一个 VCL NAL (slice)
    pub(super) fn decode_slice(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();

        match self.parse_slice_header(&rbsp, nalu) {
            Ok(mut header) => {
                if header.redundant_pic_cnt > 0 {
                    tracing::debug!(
                        "H264: 跳过冗余 slice, redundant_pic_cnt={}, frame_num={}, pps_id={}",
                        header.redundant_pic_cnt,
                        header.frame_num,
                        header.pps_id
                    );
                    return;
                }

                let prev_frame_num = self.last_frame_num;
                self.last_slice_type = header.slice_type;
                self.last_nal_ref_idc = header.nal_ref_idc;
                self.last_slice_qp = header.slice_qp;
                self.last_disable_deblocking_filter_idc = header.disable_deblocking_filter_idc;
                self.last_slice_alpha_c0_offset_div2 = header.slice_alpha_c0_offset_div2;
                self.last_slice_beta_offset_div2 = header.slice_beta_offset_div2;
                let prev_frame_num_for_poc =
                    self.fill_frame_num_gaps_if_needed(&header, prev_frame_num);
                self.last_poc = self.compute_slice_poc(&header, prev_frame_num_for_poc);
                self.last_frame_num = header.frame_num;
                self.last_dec_ref_pic_marking = std::mem::take(&mut header.dec_ref_pic_marking);
                self.decode_slice_data(&rbsp, &header);
            }
            Err(err) => {
                self.record_malformed_nal_drop("slice_header_parse", &err);
            }
        }
    }

    fn slice_sps_by_header(&self, header: &SliceHeader) -> Option<&Sps> {
        let pps = self.pps_map.get(&header.pps_id).or({
            if self.pps_map.is_empty() {
                self.pps.as_ref()
            } else {
                None
            }
        })?;
        self.sps_map.get(&pps.sps_id).or({
            if self.sps_map.is_empty() {
                self.sps.as_ref()
            } else {
                None
            }
        })
    }

    pub(super) fn fill_frame_num_gaps_if_needed(
        &mut self,
        header: &SliceHeader,
        prev_frame_num: u32,
    ) -> u32 {
        let gaps_allowed = self
            .slice_sps_by_header(header)
            .map(|sps| sps.gaps_in_frame_num_value_allowed_flag)
            .unwrap_or(false);
        if header.is_idr || !gaps_allowed {
            return prev_frame_num;
        }

        let max_frame_num = self.max_frame_num_modulo();
        if max_frame_num == 0 {
            return prev_frame_num;
        }
        let mut next_frame_num = (prev_frame_num + 1) % max_frame_num;
        if next_frame_num == header.frame_num {
            return prev_frame_num;
        }

        let mut inserted = 0usize;
        while next_frame_num != header.frame_num {
            let non_existing_poc = self.last_poc + ((inserted as i32) + 1) * 2;
            self.push_non_existing_short_term_reference(next_frame_num, non_existing_poc);
            inserted += 1;
            if inserted > max_frame_num as usize {
                break;
            }
            next_frame_num = (next_frame_num + 1) % max_frame_num;
        }
        if inserted == 0 {
            return prev_frame_num;
        }
        if header.frame_num == 0 {
            max_frame_num - 1
        } else {
            header.frame_num - 1
        }
    }

    /// 解码 slice 数据 (MB 循环)
    pub(super) fn decode_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        if let Err(err) = self.activate_parameter_sets(header.pps_id) {
            self.record_malformed_nal_drop("slice_activate_parameter_sets", &err);
            return;
        }
        let entropy_coding_mode = match &self.pps {
            Some(p) => p.entropy_coding_mode,
            None => return,
        };

        if entropy_coding_mode != 1 {
            self.decode_cavlc_slice_data(rbsp, header);
            return;
        }

        let cabac_start_byte = header.cabac_start_byte;
        if cabac_start_byte >= rbsp.len() {
            let msg = format!(
                "H264: CABAC 起始字节越界, cabac_start_byte={}, rbsp_len={}",
                cabac_start_byte,
                rbsp.len()
            );
            self.record_malformed_nal_drop("slice_cabac_start_oob", &msg);
            return;
        }

        let cabac_data = &rbsp[cabac_start_byte..];
        let mut cabac = CabacDecoder::new(cabac_data);

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        let mut ctxs = if is_i {
            init_contexts_i_slice(header.slice_qp)
        } else {
            init_contexts_pb_slice(header.slice_qp, header.cabac_init_idc)
        };

        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;
        if first >= total_mbs {
            let msg = format!(
                "H264: first_mb 越界, first_mb={}, total_mbs={}",
                first, total_mbs
            );
            self.record_malformed_nal_drop("slice_first_mb_oob", &msg);
            return;
        }

        if is_i {
            self.decode_i_slice_mbs(
                &mut cabac,
                &mut ctxs,
                first,
                total_mbs,
                header.slice_qp,
                header.first_mb,
            );
            return;
        }

        if header.slice_type == 0 || header.slice_type == 3 {
            let ref_l0_list = self.build_reference_list_l0_with_mod(
                header.num_ref_idx_l0,
                &header.ref_pic_list_mod_l0,
                header.frame_num,
            );
            self.last_ref_l0_poc = ref_l0_list.iter().map(|rp| rp.poc).collect();
            self.last_ref_l1_poc.clear();
            self.decode_p_slice_mbs(
                &mut cabac,
                &mut ctxs,
                first,
                total_mbs,
                header.slice_qp,
                header.first_mb,
                header.num_ref_idx_l0,
                &header.l0_weights,
                header.luma_log2_weight_denom,
                header.chroma_log2_weight_denom,
                &ref_l0_list,
            );
            return;
        }

        let ref_l0_list = self.build_reference_list_l0_with_mod(
            header.num_ref_idx_l0,
            &header.ref_pic_list_mod_l0,
            header.frame_num,
        );
        self.last_ref_l0_poc = ref_l0_list.iter().map(|rp| rp.poc).collect();
        let mut ref_l1_list = self.build_reference_list_l1_with_mod(
            header.num_ref_idx_l1,
            &header.ref_pic_list_mod_l1,
            header.frame_num,
        );
        self.maybe_swap_b_default_ref_list_l1(
            &ref_l0_list,
            &mut ref_l1_list,
            &header.ref_pic_list_mod_l0,
            &header.ref_pic_list_mod_l1,
            header.num_ref_idx_l1,
        );
        self.last_ref_l1_poc = ref_l1_list.iter().map(|rp| rp.poc).collect();
        self.decode_b_slice_mbs(
            &mut cabac,
            &mut ctxs,
            first,
            total_mbs,
            header.slice_qp,
            header.first_mb,
            header.num_ref_idx_l0,
            header.num_ref_idx_l1,
            header.direct_spatial_mv_pred_flag,
            &header.l0_weights,
            &header.l1_weights,
            header.luma_log2_weight_denom,
            header.chroma_log2_weight_denom,
            &ref_l0_list,
            &ref_l1_list,
        );
    }

    /// CAVLC 回退: 对所有 MB 使用 DC 预测
    pub(super) fn apply_dc_fallback(&mut self) {
        for mb_y in 0..self.mb_height {
            for mb_x in 0..self.mb_width {
                intra::predict_16x16(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16,
                    mb_y * 16,
                    2,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_u,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_v,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
            }
        }
    }

    /// CAVLC 最小路径: 消费 `mb_skip_run/mb_type`, 并执行基础重建.
    pub(super) fn decode_cavlc_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;
        if first >= total_mbs {
            let msg = format!(
                "H264: CAVLC first_mb 越界, first_mb={}, total_mbs={}",
                first, total_mbs
            );
            self.record_malformed_nal_drop("slice_cavlc_first_mb_oob", &msg);
            return;
        }

        let mut br = BitReader::new(rbsp);
        if br.skip_bits(header.data_bit_offset as u32).is_err() {
            let msg = format!(
                "H264: CAVLC data_bit_offset 越界, data_bit_offset={}, rbsp_bits={}",
                header.data_bit_offset,
                rbsp.len().saturating_mul(8)
            );
            self.record_malformed_nal_drop("slice_cavlc_bit_offset_oob", &msg);
            self.apply_dc_fallback();
            return;
        }

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        let is_b = header.slice_type == 1;
        let mut cur_qp = header.slice_qp;
        self.prev_qp_delta_nz = false;
        if is_i {
            for mb_idx in first..total_mbs {
                self.reset_cavlc_block_error();
                self.mark_mb_slice_first_mb(mb_idx, header.first_mb);
                if !has_more_rbsp_data(&mut br) {
                    break;
                }
                let mb_type = read_ue(&mut br).unwrap_or(0) as u32;
                let mb_x = mb_idx % self.mb_width;
                let mb_y = mb_idx / self.mb_width;
                self.decode_cavlc_i_mb(&mut br, mb_x, mb_y, mb_type, &mut cur_qp);
                if self.take_cavlc_block_error() {
                    let err = format!(
                        "H264: CAVLC 残差解码失败后中止 I-slice, mb_idx={}, first_mb={}",
                        mb_idx, header.first_mb
                    );
                    self.record_mb_decode_error(
                        mb_idx,
                        header.first_mb,
                        "slice_cavlc_residual",
                        &err,
                    );
                    break;
                }
                if mb_idx < self.mb_qp.len() {
                    self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
                }
            }
            return;
        }

        let ref_l0_list = self.build_reference_list_l0_with_mod(
            header.num_ref_idx_l0,
            &header.ref_pic_list_mod_l0,
            header.frame_num,
        );
        self.last_ref_l0_poc = ref_l0_list.iter().map(|rp| rp.poc).collect();
        let ref_l1_list = if is_b {
            self.build_reference_list_l1_with_mod(
                header.num_ref_idx_l1,
                &header.ref_pic_list_mod_l1,
                header.frame_num,
            )
        } else {
            Vec::new()
        };
        // CAVLC 测试流按本地语法消费顺序验证 ref_idx, 这里保持默认 L1 列表顺序不交换.
        self.last_ref_l1_poc = ref_l1_list.iter().map(|rp| rp.poc).collect();
        let mut skip_run_left = 0u32;
        let mut pending_non_skip_mb = false;
        let direct_spatial_mv_pred_flag = header.direct_spatial_mv_pred_flag;
        for mb_idx in first..total_mbs {
            self.reset_cavlc_block_error();
            if !pending_non_skip_mb && skip_run_left == 0 {
                if !has_more_rbsp_data(&mut br) {
                    break;
                }
                let Ok(skip_run) = read_ue(&mut br) else {
                    let err = format!(
                        "H264: CAVLC 宏块 skip_run 解码失败, mb_idx={}, first_mb={}",
                        mb_idx, header.first_mb
                    );
                    self.record_mb_decode_error(
                        mb_idx,
                        header.first_mb,
                        "slice_cavlc_mb_skip_run",
                        &err,
                    );
                    break;
                };
                skip_run_left = skip_run;
            }
            let prev_slice_first_mb = self.mb_slice_first_mb[mb_idx];
            self.mark_mb_slice_first_mb(mb_idx, header.first_mb);
            self.set_mb_skip_flag(mb_idx, false);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            // 与 CABAC 路径保持一致: 每个 MB 开始前先清空运动缓存,
            // 避免 CAVLC 路径在分区未完全覆盖时沿用旧帧运动信息.
            self.clear_mb_mvd_cache(mb_x, mb_y);
            self.clear_mb_motion_cache(mb_x, mb_y);
            let saved_slice_first_mb = self.mb_slice_first_mb[mb_idx];
            let left_unknown = mb_x > 0 && self.mb_slice_first_mb[mb_idx - 1] == u32::MAX;
            let top_unknown =
                mb_y > 0 && self.mb_slice_first_mb[mb_idx - self.mb_width] == u32::MAX;
            let relax_unknown_neighbors = is_b && (left_unknown || top_unknown);
            if relax_unknown_neighbors {
                // 仅在局部/断点解码导致邻居 slice 标记缺失时放宽同 slice 判断.
                self.mb_slice_first_mb[mb_idx] = u32::MAX;
            }
            let (b_pred_mv_x, b_pred_mv_y) = if is_b {
                self.predict_mv_l0_16x16(mb_x, mb_y)
            } else {
                (0, 0)
            };
            if skip_run_left > 0 {
                self.set_mb_skip_flag(mb_idx, true);
                self.mb_types[mb_idx] = if is_b { 254 } else { 255 };
                self.mb_cbp[mb_idx] = 0;
                self.clear_cavlc_mb_coeff_state(mb_x, mb_y);
                if is_b {
                    self.set_direct_block_4x4(mb_x * 16, mb_y * 16, 16, 16, true);
                    let mut last_motion = (0i32, 0i32, 0i8);
                    for sub in 0..4usize {
                        let sub_x = (sub & 1) * 8;
                        let sub_y = (sub >> 1) * 8;
                        last_motion = self.apply_b_direct_sub_8x8(
                            mb_x,
                            mb_y,
                            sub_x,
                            sub_y,
                            b_pred_mv_x,
                            b_pred_mv_y,
                            direct_spatial_mv_pred_flag,
                            &header.l0_weights,
                            &header.l1_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                            &ref_l0_list,
                            &ref_l1_list,
                        );
                    }
                    let (mv_x, mv_y, ref_idx) = last_motion;
                    if let Some(slot) = self.mv_l0_x.get_mut(mb_idx) {
                        *slot = mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    }
                    if let Some(slot) = self.mv_l0_y.get_mut(mb_idx) {
                        *slot = mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    }
                    if let Some(slot) = self.ref_idx_l0.get_mut(mb_idx) {
                        *slot = ref_idx;
                    }
                } else {
                    let relax_p_skip_unknown_neighbors = left_unknown || top_unknown;
                    if relax_p_skip_unknown_neighbors {
                        // 仅在局部/断点解码导致邻居 slice 标记缺失时放宽同 slice 判断.
                        let saved_first_mb = self.mb_slice_first_mb[mb_idx];
                        let relaxed_first_mb = if mb_x > 0 {
                            self.mb_slice_first_mb[mb_idx - 1]
                        } else if mb_y > 0 {
                            self.mb_slice_first_mb[mb_idx - self.mb_width]
                        } else {
                            saved_first_mb
                        };
                        self.mb_slice_first_mb[mb_idx] = relaxed_first_mb;
                        self.decode_p_skip_mb(
                            mb_x,
                            mb_y,
                            &ref_l0_list,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.mb_slice_first_mb[mb_idx] = saved_first_mb;
                    } else {
                        self.decode_p_skip_mb(
                            mb_x,
                            mb_y,
                            &ref_l0_list,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                    }
                }
                skip_run_left -= 1;
                if skip_run_left == 0 {
                    // 本次 skip_run 段已结束, 下一宏块直接读取 mb_type.
                    pending_non_skip_mb = true;
                }
                if mb_idx < self.mb_qp.len() {
                    self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
                }
                if relax_unknown_neighbors {
                    self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                }
                continue;
            }
            pending_non_skip_mb = false;
            if !has_more_rbsp_data(&mut br) {
                self.mb_slice_first_mb[mb_idx] = prev_slice_first_mb;
                break;
            }
            let Ok(mb_type) = read_ue(&mut br) else {
                let err = format!(
                    "H264: CAVLC 宏块 mb_type 解码失败, mb_idx={}, first_mb={}",
                    mb_idx, header.first_mb
                );
                self.record_mb_decode_error(mb_idx, header.first_mb, "slice_cavlc_mb_type", &err);
                if relax_unknown_neighbors {
                    self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                }
                break;
            };
            if is_b {
                let is_inter = mb_type <= 22;
                if is_inter {
                    self.mb_types[mb_idx] = 254;
                    self.mb_cbp[mb_idx] = 0;
                    if mb_type == 22 {
                        // B_8x8 语法路径禁止无条件放宽当前 MB 的 slice 标记。
                        // 否则在多 slice 帧里会把跨 slice 邻居误当作可用候选, 污染 MVP。
                        let (b_pred_mv_x, b_pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
                        let mut sub_mb_types = [0u32; 4];
                        for slot in &mut sub_mb_types {
                            *slot = read_ue(&mut br).unwrap_or(0);
                        }
                        let mut use_l0 = [false; 4];
                        let mut use_l1 = [false; 4];
                        let mut sub_part_count = [1usize; 4];
                        for (sub_idx, sub_mb_type) in sub_mb_types.iter().copied().enumerate() {
                            use_l0[sub_idx] =
                                matches!(sub_mb_type, 1 | 3 | 4 | 5 | 8 | 9 | 10 | 12);
                            use_l1[sub_idx] =
                                matches!(sub_mb_type, 2 | 3 | 6 | 7 | 8 | 9 | 11 | 12);
                            sub_part_count[sub_idx] = match sub_mb_type {
                                4..=9 => 2usize,
                                10..=12 => 4usize,
                                _ => 1usize,
                            };
                        }

                        let mut ref_idx_l0 = [0usize; 4];
                        let mut ref_idx_l1 = [0usize; 4];
                        if header.num_ref_idx_l0 > 1 {
                            let max_ref_idx_l0 = header.num_ref_idx_l0.saturating_sub(1);
                            for sub_idx in 0..4usize {
                                if use_l0[sub_idx] {
                                    ref_idx_l0[sub_idx] =
                                        read_te(&mut br, max_ref_idx_l0).unwrap_or(0) as usize;
                                }
                            }
                        }
                        if header.num_ref_idx_l1 > 1 {
                            let max_ref_idx_l1 = header.num_ref_idx_l1.saturating_sub(1);
                            for sub_idx in 0..4usize {
                                if use_l1[sub_idx] {
                                    ref_idx_l1[sub_idx] =
                                        read_te(&mut br, max_ref_idx_l1).unwrap_or(0) as usize;
                                }
                            }
                        }

                        let mut l0_mvd_x = [[0i32; 4]; 4];
                        let mut l0_mvd_y = [[0i32; 4]; 4];
                        let mut l1_mvd_x = [[0i32; 4]; 4];
                        let mut l1_mvd_y = [[0i32; 4]; 4];
                        for sub_idx in 0..4usize {
                            if use_l0[sub_idx] {
                                for part_idx in 0..sub_part_count[sub_idx] {
                                    l0_mvd_x[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                                    l0_mvd_y[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                                }
                            }
                        }
                        for sub_idx in 0..4usize {
                            if use_l1[sub_idx] {
                                for part_idx in 0..sub_part_count[sub_idx] {
                                    l1_mvd_x[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                                    l1_mvd_y[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                                }
                            }
                        }

                        for (sub_idx, sub_mb_type) in sub_mb_types.iter().copied().enumerate() {
                            let sub_x = mb_x * 16 + (sub_idx % 2) * 8;
                            let sub_y = mb_y * 16 + (sub_idx / 2) * 8;
                            let mut l0_motions = [None; 4];
                            let mut l1_motions = [None; 4];
                            for part_idx in 0..sub_part_count[sub_idx] {
                                let (pw, ph) = match sub_mb_type {
                                    4 | 6 | 8 => (8usize, 4usize),
                                    5 | 7 | 9 => (4, 8),
                                    10..=12 => (4, 4),
                                    _ => (8, 8),
                                };
                                let (p_off_x, p_off_y) = match (pw, ph, sub_part_count[sub_idx]) {
                                    (8, 4, _) => (0, part_idx * 4),
                                    (4, 8, _) => (part_idx * 4, 0),
                                    (4, 4, _) => ((part_idx & 1) * 4, (part_idx >> 1) * 4),
                                    _ => (0, 0),
                                };
                                let part_x4 = (sub_idx % 2) * 2 + p_off_x / 4;
                                let part_y4 = (sub_idx / 2) * 2 + p_off_y / 4;
                                let pw4 = pw / 4;
                                if use_l0[sub_idx] {
                                    let l0_ref_i8 = ref_idx_l0[sub_idx].min(i8::MAX as usize) as i8;
                                    let (pred_x, pred_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        part_x4,
                                        part_y4,
                                        pw4.max(1),
                                        l0_ref_i8,
                                    );
                                    let mv_x = pred_x + l0_mvd_x[sub_idx][part_idx];
                                    let mv_y = pred_y + l0_mvd_y[sub_idx][part_idx];
                                    l0_motions[part_idx] = Some(BMotion {
                                        mv_x,
                                        mv_y,
                                        ref_idx: l0_ref_i8,
                                    });
                                    self.set_l0_motion_block_4x4(
                                        sub_x + p_off_x,
                                        sub_y + p_off_y,
                                        pw,
                                        ph,
                                        mv_x,
                                        mv_y,
                                        l0_ref_i8,
                                    );
                                }
                                if use_l1[sub_idx] {
                                    let l1_ref_i8 = ref_idx_l1[sub_idx].min(i8::MAX as usize) as i8;
                                    let (pred_x, pred_y) = self.predict_mv_l1_partition(
                                        mb_x,
                                        mb_y,
                                        part_x4,
                                        part_y4,
                                        pw4.max(1),
                                        l1_ref_i8,
                                    );
                                    let mv_x = pred_x + l1_mvd_x[sub_idx][part_idx];
                                    let mv_y = pred_y + l1_mvd_y[sub_idx][part_idx];
                                    l1_motions[part_idx] = Some(BMotion {
                                        mv_x,
                                        mv_y,
                                        ref_idx: l1_ref_i8,
                                    });
                                    self.set_l1_motion_block_4x4(
                                        sub_x + p_off_x,
                                        sub_y + p_off_y,
                                        pw,
                                        ph,
                                        mv_x,
                                        mv_y,
                                        l1_ref_i8,
                                    );
                                }
                            }
                            if !use_l0[sub_idx] && !use_l1[sub_idx] {
                                let sub_off_x = (sub_idx % 2) * 8;
                                let sub_off_y = (sub_idx / 2) * 8;
                                let _ = self.apply_b_direct_sub_8x8(
                                    mb_x,
                                    mb_y,
                                    sub_off_x,
                                    sub_off_y,
                                    b_pred_mv_x,
                                    b_pred_mv_y,
                                    direct_spatial_mv_pred_flag,
                                    &header.l0_weights,
                                    &header.l1_weights,
                                    header.luma_log2_weight_denom,
                                    header.chroma_log2_weight_denom,
                                    &ref_l0_list,
                                    &ref_l1_list,
                                );
                                continue;
                            }

                            match sub_mb_type {
                                4 | 6 | 8 => {
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[0],
                                        l1_motions[0],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y,
                                        8,
                                        4,
                                    );
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[1],
                                        l1_motions[1],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y + 4,
                                        8,
                                        4,
                                    );
                                }
                                5 | 7 | 9 => {
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[0],
                                        l1_motions[0],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y,
                                        4,
                                        8,
                                    );
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[1],
                                        l1_motions[1],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        8,
                                    );
                                }
                                10..=12 => {
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[0],
                                        l1_motions[0],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y,
                                        4,
                                        4,
                                    );
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[1],
                                        l1_motions[1],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        4,
                                    );
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[2],
                                        l1_motions[2],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y + 4,
                                        4,
                                        4,
                                    );
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[3],
                                        l1_motions[3],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x + 4,
                                        sub_y + 4,
                                        4,
                                        4,
                                    );
                                }
                                _ => {
                                    let _ = self.apply_b_prediction_block(
                                        l0_motions[0],
                                        l1_motions[0],
                                        &header.l0_weights,
                                        &header.l1_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                        &ref_l0_list,
                                        &ref_l1_list,
                                        sub_x,
                                        sub_y,
                                        8,
                                        8,
                                    );
                                }
                            }
                        }
                        // B_8x8 Inter 残差
                        let sub_mb_types_u8 = [
                            sub_mb_types[0].min(u8::MAX as u32) as u8,
                            sub_mb_types[1].min(u8::MAX as u32) as u8,
                            sub_mb_types[2].min(u8::MAX as u32) as u8,
                            sub_mb_types[3].min(u8::MAX as u32) as u8,
                        ];
                        let no_sub_mb_part_size_less_than_8x8_flag =
                            self.b_no_sub_mb_part_size_less_than_8x8(&sub_mb_types_u8);
                        self.decode_cavlc_mb_residual(
                            &mut br,
                            mb_x,
                            mb_y,
                            &mut cur_qp,
                            false,
                            no_sub_mb_part_size_less_than_8x8_flag,
                        );
                        if self.take_cavlc_block_error() {
                            let err = format!(
                                "H264: CAVLC 残差解码失败后中止 B-slice, mb_idx={}, first_mb={}",
                                mb_idx, header.first_mb
                            );
                            self.record_mb_decode_error(
                                mb_idx,
                                header.first_mb,
                                "slice_cavlc_residual",
                                &err,
                            );
                            if relax_unknown_neighbors {
                                self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                            }
                            break;
                        }
                        if mb_idx < self.mb_qp.len() {
                            self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
                        }
                        if relax_unknown_neighbors {
                            self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                        }
                        continue;
                    }

                    if (4..=21).contains(&mb_type) {
                        const MODE_L0: u8 = 0;
                        const MODE_L1: u8 = 1;
                        const MODE_BI: u8 = 2;

                        let (part0_mode, part1_mode) = match mb_type {
                            4 | 5 => (MODE_L0, MODE_L0),
                            6 | 7 => (MODE_L1, MODE_L1),
                            8 | 9 => (MODE_L0, MODE_L1),
                            10 | 11 => (MODE_L1, MODE_L0),
                            12 | 13 => (MODE_L0, MODE_BI),
                            14 | 15 => (MODE_L1, MODE_BI),
                            16 | 17 => (MODE_BI, MODE_L0),
                            18 | 19 => (MODE_BI, MODE_L1),
                            20 | 21 => (MODE_BI, MODE_BI),
                            _ => (MODE_BI, MODE_BI),
                        };
                        let split_16x8 = mb_type & 1 == 0;
                        let part_modes = [part0_mode, part1_mode];
                        let mut part_use_l0 = [false; 2];
                        let mut part_use_l1 = [false; 2];
                        for (part_idx, mode) in part_modes.iter().copied().enumerate() {
                            part_use_l0[part_idx] = mode == MODE_L0 || mode == MODE_BI;
                            part_use_l1[part_idx] = mode == MODE_L1 || mode == MODE_BI;
                        }

                        let mut ref_idx_l0 = [0usize; 2];
                        let mut ref_idx_l1 = [0usize; 2];
                        // 按 H264 语法顺序分组消费: 先全部 ref_idx, 再全部 mvd。
                        if header.num_ref_idx_l0 > 1 {
                            let max_ref_idx_l0 = header.num_ref_idx_l0.saturating_sub(1);
                            for part_idx in 0..2usize {
                                if part_use_l0[part_idx] {
                                    ref_idx_l0[part_idx] =
                                        read_te(&mut br, max_ref_idx_l0).unwrap_or(0) as usize;
                                }
                            }
                        }
                        if header.num_ref_idx_l1 > 1 {
                            let max_ref_idx_l1 = header.num_ref_idx_l1.saturating_sub(1);
                            for part_idx in 0..2usize {
                                if part_use_l1[part_idx] {
                                    ref_idx_l1[part_idx] =
                                        read_te(&mut br, max_ref_idx_l1).unwrap_or(0) as usize;
                                }
                            }
                        }

                        let mut l0_mvd: [(i32, i32); 2] = [(0, 0); 2];
                        let mut l1_mvd: [(i32, i32); 2] = [(0, 0); 2];
                        for part_idx in 0..2usize {
                            if part_use_l0[part_idx] {
                                l0_mvd[part_idx].0 = read_se(&mut br).unwrap_or(0);
                                l0_mvd[part_idx].1 = read_se(&mut br).unwrap_or(0);
                            }
                        }
                        for part_idx in 0..2usize {
                            if part_use_l1[part_idx] {
                                l1_mvd[part_idx].0 = read_se(&mut br).unwrap_or(0);
                                l1_mvd[part_idx].1 = read_se(&mut br).unwrap_or(0);
                            }
                        }

                        let mut l0_motion: [Option<BMotion>; 2] = [None; 2];
                        let mut l1_motion: [Option<BMotion>; 2] = [None; 2];
                        let mut l0_pred = [(0i32, 0i32); 2];
                        let mut l1_pred = [(0i32, 0i32); 2];
                        let mut l0_mv = [(0i32, 0i32); 2];
                        let mut l1_mv = [(0i32, 0i32); 2];
                        for part_idx in 0..2usize {
                            let (part_x4, part_y4, part_w4) = if split_16x8 {
                                (0usize, part_idx * 2, 4usize)
                            } else {
                                (part_idx * 2, 0usize, 2usize)
                            };
                            if part_use_l0[part_idx] {
                                let l0_ref_i8 = ref_idx_l0[part_idx].min(i8::MAX as usize) as i8;
                                let (pred_x, pred_y) = if split_16x8 {
                                    self.predict_mv_l0_16x8(mb_x, mb_y, part_idx, l0_ref_i8)
                                } else {
                                    self.predict_mv_l0_8x16(mb_x, mb_y, part_idx, l0_ref_i8)
                                };
                                l0_pred[part_idx] = (pred_x, pred_y);
                                let mv_x = pred_x + l0_mvd[part_idx].0;
                                let mv_y = pred_y + l0_mvd[part_idx].1;
                                l0_mv[part_idx] = (mv_x, mv_y);
                                l0_motion[part_idx] = Some(BMotion {
                                    mv_x,
                                    mv_y,
                                    ref_idx: l0_ref_i8,
                                });
                                self.set_l0_motion_block_4x4(
                                    mb_x * 16 + part_x4 * 4,
                                    mb_y * 16 + part_y4 * 4,
                                    part_w4 * 4,
                                    if split_16x8 { 8 } else { 16 },
                                    mv_x,
                                    mv_y,
                                    l0_ref_i8,
                                );
                            }
                            if part_use_l1[part_idx] {
                                let l1_ref_i8 = ref_idx_l1[part_idx].min(i8::MAX as usize) as i8;
                                let (pred_x, pred_y) = if split_16x8 {
                                    self.predict_mv_l1_16x8(mb_x, mb_y, part_idx, l1_ref_i8)
                                } else {
                                    self.predict_mv_l1_8x16(mb_x, mb_y, part_idx, l1_ref_i8)
                                };
                                l1_pred[part_idx] = (pred_x, pred_y);
                                let mv_x = pred_x + l1_mvd[part_idx].0;
                                let mv_y = pred_y + l1_mvd[part_idx].1;
                                l1_mv[part_idx] = (mv_x, mv_y);
                                l1_motion[part_idx] = Some(BMotion {
                                    mv_x,
                                    mv_y,
                                    ref_idx: l1_ref_i8,
                                });
                                self.set_l1_motion_block_4x4(
                                    mb_x * 16 + part_x4 * 4,
                                    mb_y * 16 + part_y4 * 4,
                                    part_w4 * 4,
                                    if split_16x8 { 8 } else { 16 },
                                    mv_x,
                                    mv_y,
                                    l1_ref_i8,
                                );
                            }
                        }

                        let part0_l0_motion = l0_motion[0];
                        let part0_l1_motion = l1_motion[0];
                        let part1_l0_motion = l0_motion[1];
                        let part1_l1_motion = l1_motion[1];
                        if split_16x8 {
                            let _ = self.apply_b_prediction_block(
                                part0_l0_motion,
                                part0_l1_motion,
                                &header.l0_weights,
                                &header.l1_weights,
                                header.luma_log2_weight_denom,
                                header.chroma_log2_weight_denom,
                                &ref_l0_list,
                                &ref_l1_list,
                                mb_x * 16,
                                mb_y * 16,
                                16,
                                8,
                            );
                            let _ = self.apply_b_prediction_block(
                                part1_l0_motion,
                                part1_l1_motion,
                                &header.l0_weights,
                                &header.l1_weights,
                                header.luma_log2_weight_denom,
                                header.chroma_log2_weight_denom,
                                &ref_l0_list,
                                &ref_l1_list,
                                mb_x * 16,
                                mb_y * 16 + 8,
                                16,
                                8,
                            );
                        } else {
                            let _ = self.apply_b_prediction_block(
                                part0_l0_motion,
                                part0_l1_motion,
                                &header.l0_weights,
                                &header.l1_weights,
                                header.luma_log2_weight_denom,
                                header.chroma_log2_weight_denom,
                                &ref_l0_list,
                                &ref_l1_list,
                                mb_x * 16,
                                mb_y * 16,
                                8,
                                16,
                            );
                            let _ = self.apply_b_prediction_block(
                                part1_l0_motion,
                                part1_l1_motion,
                                &header.l0_weights,
                                &header.l1_weights,
                                header.luma_log2_weight_denom,
                                header.chroma_log2_weight_denom,
                                &ref_l0_list,
                                &ref_l1_list,
                                mb_x * 16 + 8,
                                mb_y * 16,
                                8,
                                16,
                            );
                        }
                        // B_16x8/B_8x16 Inter 残差
                        self.decode_cavlc_mb_residual(
                            &mut br,
                            mb_x,
                            mb_y,
                            &mut cur_qp,
                            false,
                            true,
                        );
                        if self.take_cavlc_block_error() {
                            let err = format!(
                                "H264: CAVLC 残差解码失败后中止 B-slice, mb_idx={}, first_mb={}",
                                mb_idx, header.first_mb
                            );
                            self.record_mb_decode_error(
                                mb_idx,
                                header.first_mb,
                                "slice_cavlc_residual",
                                &err,
                            );
                            if relax_unknown_neighbors {
                                self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                            }
                            break;
                        }
                        if mb_idx < self.mb_qp.len() {
                            self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
                        }
                        if relax_unknown_neighbors {
                            self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                        }
                        continue;
                    }

                    let mut l0_motion = None;
                    let mut l1_motion = None;
                    let mut applied_direct_by_sub = false;
                    if mb_type == 0 {
                        self.set_direct_block_4x4(mb_x * 16, mb_y * 16, 16, 16, true);
                        let mut last_motion = (0i32, 0i32, 0i8);
                        for sub in 0..4usize {
                            let sub_x = (sub & 1) * 8;
                            let sub_y = (sub >> 1) * 8;
                            last_motion = self.apply_b_direct_sub_8x8(
                                mb_x,
                                mb_y,
                                sub_x,
                                sub_y,
                                b_pred_mv_x,
                                b_pred_mv_y,
                                direct_spatial_mv_pred_flag,
                                &header.l0_weights,
                                &header.l1_weights,
                                header.luma_log2_weight_denom,
                                header.chroma_log2_weight_denom,
                                &ref_l0_list,
                                &ref_l1_list,
                            );
                        }
                        let (mv_x, mv_y, ref_idx) = last_motion;
                        if let Some(slot) = self.mv_l0_x.get_mut(mb_idx) {
                            *slot = mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                        }
                        if let Some(slot) = self.mv_l0_y.get_mut(mb_idx) {
                            *slot = mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                        }
                        if let Some(slot) = self.ref_idx_l0.get_mut(mb_idx) {
                            *slot = ref_idx;
                        }
                        applied_direct_by_sub = true;
                    } else {
                        let use_l0 = mb_type == 1 || mb_type == 3;
                        let use_l1 = mb_type == 2 || mb_type == 3;
                        let mut l0_ref_idx = 0usize;
                        let mut l1_ref_idx = 0usize;
                        if use_l0 && header.num_ref_idx_l0 > 1 {
                            l0_ref_idx = read_te(&mut br, header.num_ref_idx_l0.saturating_sub(1))
                                .unwrap_or(0) as usize;
                        }
                        if use_l1 && header.num_ref_idx_l1 > 1 {
                            l1_ref_idx = read_te(&mut br, header.num_ref_idx_l1.saturating_sub(1))
                                .unwrap_or(0) as usize;
                        }

                        if use_l0 {
                            let l0_ref_i8 = l0_ref_idx.min(i8::MAX as usize) as i8;
                            let (pred_x, pred_y) =
                                self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, l0_ref_i8);
                            let mvd_x = read_se(&mut br).unwrap_or(0);
                            let mvd_y = read_se(&mut br).unwrap_or(0);
                            l0_motion = Some(BMotion {
                                mv_x: pred_x + mvd_x,
                                mv_y: pred_y + mvd_y,
                                ref_idx: l0_ref_i8,
                            });
                        }
                        if use_l1 {
                            let l1_ref_i8 = l1_ref_idx.min(i8::MAX as usize) as i8;
                            let (pred_x, pred_y) =
                                self.predict_mv_l1_partition(mb_x, mb_y, 0, 0, 4, l1_ref_i8);
                            let mvd_x = read_se(&mut br).unwrap_or(0);
                            let mvd_y = read_se(&mut br).unwrap_or(0);
                            l1_motion = Some(BMotion {
                                mv_x: pred_x + mvd_x,
                                mv_y: pred_y + mvd_y,
                                ref_idx: l1_ref_i8,
                            });
                        }
                    }
                    if !applied_direct_by_sub {
                        let _ = self.apply_b_prediction_block(
                            l0_motion,
                            l1_motion,
                            &header.l0_weights,
                            &header.l1_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                            &ref_l0_list,
                            &ref_l1_list,
                            mb_x * 16,
                            mb_y * 16,
                            16,
                            16,
                        );
                    }
                    // B_Direct/B_L0/B_L1/B_Bi 16x16 Inter 残差
                    self.decode_cavlc_mb_residual(&mut br, mb_x, mb_y, &mut cur_qp, false, true);
                } else {
                    // B-slice 中的 Intra MB: mb_type - 23 映射到 I mb_type
                    let i_mb_type = mb_type - 23;
                    self.decode_cavlc_i_mb(&mut br, mb_x, mb_y, i_mb_type, &mut cur_qp);
                }
                if mb_idx < self.mb_qp.len() {
                    self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
                }
                if relax_unknown_neighbors {
                    self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                }
                continue;
            }
            self.mb_cbp[mb_idx] = 0;
            if mb_type >= 5 {
                // P-slice 中的 Intra MB: mb_type - 5 映射到 I mb_type
                let i_mb_type = mb_type - 5;
                self.decode_cavlc_i_mb(&mut br, mb_x, mb_y, i_mb_type, &mut cur_qp);
            } else {
                let saved_first_mb = self.mb_slice_first_mb[mb_idx];
                let left_unknown = mb_x > 0 && self.mb_slice_first_mb[mb_idx - 1] == u32::MAX;
                let top_unknown =
                    mb_y > 0 && self.mb_slice_first_mb[mb_idx - self.mb_width] == u32::MAX;
                let relax_unknown_neighbors = left_unknown || top_unknown;
                if relax_unknown_neighbors {
                    // 仅在局部/断点解码导致邻居 slice 标记缺失时放宽同 slice 判断.
                    self.mb_slice_first_mb[mb_idx] = u32::MAX;
                }
                self.mb_types[mb_idx] = 200u8.saturating_add((mb_type as u8).min(3));
                let base_x = mb_x * 16;
                let base_y = mb_y * 16;
                let max_ref_idx_l0 = header.num_ref_idx_l0.saturating_sub(1);
                let mut final_mv_x = 0i32;
                let mut final_mv_y = 0i32;
                let mut final_ref_idx = 0u32;
                let mut no_sub_mb_part_size_less_than_8x8_flag = true;
                match mb_type {
                    0 => {
                        let mut ref_idx_l0 = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_l0 = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                        }
                        let ref_idx_i8 = ref_idx_l0.min(i8::MAX as u32) as i8;
                        self.set_l0_motion_block_4x4(base_x, base_y, 16, 16, 0, 0, ref_idx_i8);
                        let (pred_mv_x, pred_mv_y) =
                            self.predict_mv_l0_partition(mb_x, mb_y, 0, 0, 4, ref_idx_i8);
                        let mvd_x = read_se(&mut br).unwrap_or(0);
                        let mvd_y = read_se(&mut br).unwrap_or(0);
                        let mv_x = pred_mv_x + mvd_x;
                        let mv_y = pred_mv_y + mvd_y;
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_l0,
                            base_x,
                            base_y,
                            16,
                            16,
                            mv_x,
                            mv_y,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.set_l0_motion_block_4x4(
                            base_x, base_y, 16, 16, mv_x, mv_y, ref_idx_i8,
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx_l0;
                    }
                    1 => {
                        let mut ref_idx_top = 0u32;
                        let mut ref_idx_bottom = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_top = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                            ref_idx_bottom = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                        }
                        let top_ref_idx_i8 = ref_idx_top.min(i8::MAX as u32) as i8;
                        let (pred_mv_x, pred_mv_y) =
                            self.predict_mv_l0_16x8(mb_x, mb_y, 0, top_ref_idx_i8);
                        let mvd_top_x = read_se(&mut br).unwrap_or(0);
                        let mvd_top_y = read_se(&mut br).unwrap_or(0);
                        let mv_top_x = pred_mv_x + mvd_top_x;
                        let mv_top_y = pred_mv_y + mvd_top_y;
                        self.set_l0_motion_block_4x4(
                            base_x,
                            base_y,
                            16,
                            8,
                            mv_top_x,
                            mv_top_y,
                            top_ref_idx_i8,
                        );
                        let bottom_ref_idx_i8 = ref_idx_bottom.min(i8::MAX as u32) as i8;
                        let (pred_bottom_x, pred_bottom_y) =
                            self.predict_mv_l0_16x8(mb_x, mb_y, 1, bottom_ref_idx_i8);
                        let mvd_bottom_x = read_se(&mut br).unwrap_or(0);
                        let mvd_bottom_y = read_se(&mut br).unwrap_or(0);
                        let mv_bottom_x = pred_bottom_x + mvd_bottom_x;
                        let mv_bottom_y = pred_bottom_y + mvd_bottom_y;
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_top,
                            base_x,
                            base_y,
                            16,
                            8,
                            mv_top_x,
                            mv_top_y,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_bottom,
                            base_x,
                            base_y + 8,
                            16,
                            8,
                            mv_bottom_x,
                            mv_bottom_y,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.set_l0_motion_block_4x4(
                            base_x,
                            base_y + 8,
                            16,
                            8,
                            mv_bottom_x,
                            mv_bottom_y,
                            bottom_ref_idx_i8,
                        );
                        final_mv_x = mv_bottom_x;
                        final_mv_y = mv_bottom_y;
                        final_ref_idx = ref_idx_bottom;
                    }
                    2 => {
                        let mut ref_idx_left = 0u32;
                        let mut ref_idx_right = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_left = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                            ref_idx_right = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                        }
                        let left_ref_idx_i8 = ref_idx_left.min(i8::MAX as u32) as i8;
                        let (pred_mv_x, pred_mv_y) =
                            self.predict_mv_l0_8x16(mb_x, mb_y, 0, left_ref_idx_i8);
                        let mvd_left_x = read_se(&mut br).unwrap_or(0);
                        let mvd_left_y = read_se(&mut br).unwrap_or(0);
                        let mv_left_x = pred_mv_x + mvd_left_x;
                        let mv_left_y = pred_mv_y + mvd_left_y;
                        self.set_l0_motion_block_4x4(
                            base_x,
                            base_y,
                            8,
                            16,
                            mv_left_x,
                            mv_left_y,
                            left_ref_idx_i8,
                        );
                        let right_ref_idx_i8 = ref_idx_right.min(i8::MAX as u32) as i8;
                        let (pred_right_x, pred_right_y) =
                            self.predict_mv_l0_8x16(mb_x, mb_y, 1, right_ref_idx_i8);
                        let mvd_right_x = read_se(&mut br).unwrap_or(0);
                        let mvd_right_y = read_se(&mut br).unwrap_or(0);
                        let mv_right_x = pred_right_x + mvd_right_x;
                        let mv_right_y = pred_right_y + mvd_right_y;
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_left,
                            base_x,
                            base_y,
                            8,
                            16,
                            mv_left_x,
                            mv_left_y,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_right,
                            base_x + 8,
                            base_y,
                            8,
                            16,
                            mv_right_x,
                            mv_right_y,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                        self.set_l0_motion_block_4x4(
                            base_x + 8,
                            base_y,
                            8,
                            16,
                            mv_right_x,
                            mv_right_y,
                            right_ref_idx_i8,
                        );
                        final_mv_x = mv_right_x;
                        final_mv_y = mv_right_y;
                        final_ref_idx = ref_idx_right;
                    }
                    3 | 4 => {
                        let mut sub_mb_types = [0u32; 4];
                        for slot in &mut sub_mb_types {
                            *slot = read_ue(&mut br).unwrap_or(0);
                        }
                        no_sub_mb_part_size_less_than_8x8_flag =
                            sub_mb_types.iter().all(|&sub_mb_type| sub_mb_type == 0);

                        let mut sub_ref_idx = [0u32; 4];
                        if mb_type == 3 && header.num_ref_idx_l0 > 1 {
                            for slot in &mut sub_ref_idx {
                                *slot = read_te(&mut br, max_ref_idx_l0).unwrap_or(0);
                            }
                        }
                        // 与 CABAC 路径对齐 ref_cache 预填充时序:
                        // 先写每个 8x8 子分区的 [1]/[8]/[9], 将 [0] 延后到该子分区真正开始解码时写入,
                        // 避免“未来子分区左上 4x4”过早参与当前分区 MVP 邻居候选.
                        for (sub_idx, ref_idx) in sub_ref_idx.iter().copied().enumerate() {
                            let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                            let sub_x = base_x + (sub_idx % 2) * 8;
                            let sub_y = base_y + (sub_idx / 2) * 8;
                            self.set_l0_motion_block_4x4(sub_x, sub_y, 4, 4, 0, 0, -2);
                            self.set_l0_motion_block_4x4(sub_x + 4, sub_y, 4, 4, 0, 0, ref_idx_i8);
                            self.set_l0_motion_block_4x4(sub_x, sub_y + 4, 4, 4, 0, 0, ref_idx_i8);
                            self.set_l0_motion_block_4x4(
                                sub_x + 4,
                                sub_y + 4,
                                4,
                                4,
                                0,
                                0,
                                ref_idx_i8,
                            );
                        }
                        let mut sub_mv_x = [[0i32; 4]; 4];
                        let mut sub_mv_y = [[0i32; 4]; 4];
                        for sub_idx in 0..4usize {
                            let sub_part_count = match sub_mb_types[sub_idx] {
                                1 | 2 => 2usize,
                                3 => 4usize,
                                _ => 1usize,
                            };
                            for part_idx in 0..sub_part_count {
                                sub_mv_x[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                                sub_mv_y[sub_idx][part_idx] = read_se(&mut br).unwrap_or(0);
                            }
                        }

                        for sub_idx in 0..4usize {
                            let sub_x = base_x + (sub_idx % 2) * 8;
                            let sub_y = base_y + (sub_idx / 2) * 8;
                            let sub_part_x4 = (sub_idx % 2) * 2;
                            let sub_part_y4 = (sub_idx / 2) * 2;
                            let ref_idx = sub_ref_idx[sub_idx];
                            let ref_idx_i8 = ref_idx.min(i8::MAX as u32) as i8;
                            self.set_l0_motion_block_4x4(sub_x, sub_y, 4, 4, 0, 0, ref_idx_i8);
                            match sub_mb_types[sub_idx] {
                                0 => {
                                    let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        2,
                                        ref_idx_i8,
                                    );
                                    let mv_x = pred_mv_x + sub_mv_x[sub_idx][0];
                                    let mv_y = pred_mv_y + sub_mv_y[sub_idx][0];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y,
                                        8,
                                        8,
                                        mv_x,
                                        mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x, sub_y, 8, 8, mv_x, mv_y, ref_idx_i8,
                                    );
                                    final_mv_x = mv_x;
                                    final_mv_y = mv_y;
                                    final_ref_idx = ref_idx;
                                }
                                1 => {
                                    let (pred_top_x, pred_top_y) = self.predict_mv_l0_sub_8x4(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        0,
                                        ref_idx_i8,
                                    );
                                    let top_mv_x = pred_top_x + sub_mv_x[sub_idx][0];
                                    let top_mv_y = pred_top_y + sub_mv_y[sub_idx][0];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y,
                                        8,
                                        4,
                                        top_mv_x,
                                        top_mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x, sub_y, 8, 4, top_mv_x, top_mv_y, ref_idx_i8,
                                    );
                                    let (pred_bottom_x, pred_bottom_y) = self
                                        .predict_mv_l0_sub_8x4(
                                            mb_x,
                                            mb_y,
                                            sub_part_x4,
                                            sub_part_y4,
                                            1,
                                            ref_idx_i8,
                                        );
                                    let bottom_mv_x = pred_bottom_x + sub_mv_x[sub_idx][1];
                                    let bottom_mv_y = pred_bottom_y + sub_mv_y[sub_idx][1];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y + 4,
                                        8,
                                        4,
                                        bottom_mv_x,
                                        bottom_mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x,
                                        sub_y + 4,
                                        8,
                                        4,
                                        bottom_mv_x,
                                        bottom_mv_y,
                                        ref_idx_i8,
                                    );
                                    final_mv_x = bottom_mv_x;
                                    final_mv_y = bottom_mv_y;
                                    final_ref_idx = ref_idx;
                                }
                                2 => {
                                    let (pred_left_x, pred_left_y) = self.predict_mv_l0_sub_4x8(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        0,
                                        ref_idx_i8,
                                    );
                                    let left_mv_x = pred_left_x + sub_mv_x[sub_idx][0];
                                    let left_mv_y = pred_left_y + sub_mv_y[sub_idx][0];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y,
                                        4,
                                        8,
                                        left_mv_x,
                                        left_mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x, sub_y, 4, 8, left_mv_x, left_mv_y, ref_idx_i8,
                                    );
                                    let (pred_right_x, pred_right_y) = self.predict_mv_l0_sub_4x8(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let right_mv_x = pred_right_x + sub_mv_x[sub_idx][1];
                                    let right_mv_y = pred_right_y + sub_mv_y[sub_idx][1];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        8,
                                        right_mv_x,
                                        right_mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        8,
                                        right_mv_x,
                                        right_mv_y,
                                        ref_idx_i8,
                                    );
                                    final_mv_x = right_mv_x;
                                    final_mv_y = right_mv_y;
                                    final_ref_idx = ref_idx;
                                }
                                3 => {
                                    let (pred00_x, pred00_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let mv00_x = pred00_x + sub_mv_x[sub_idx][0];
                                    let mv00_y = pred00_y + sub_mv_y[sub_idx][0];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y,
                                        4,
                                        4,
                                        mv00_x,
                                        mv00_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x, sub_y, 4, 4, mv00_x, mv00_y, ref_idx_i8,
                                    );
                                    let (pred10_x, pred10_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4 + 1,
                                        sub_part_y4,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let mv10_x = pred10_x + sub_mv_x[sub_idx][1];
                                    let mv10_y = pred10_y + sub_mv_y[sub_idx][1];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        4,
                                        mv10_x,
                                        mv10_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x + 4,
                                        sub_y,
                                        4,
                                        4,
                                        mv10_x,
                                        mv10_y,
                                        ref_idx_i8,
                                    );
                                    let (pred01_x, pred01_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4 + 1,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let mv01_x = pred01_x + sub_mv_x[sub_idx][2];
                                    let mv01_y = pred01_y + sub_mv_y[sub_idx][2];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y + 4,
                                        4,
                                        4,
                                        mv01_x,
                                        mv01_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x,
                                        sub_y + 4,
                                        4,
                                        4,
                                        mv01_x,
                                        mv01_y,
                                        ref_idx_i8,
                                    );
                                    let (pred11_x, pred11_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4 + 1,
                                        sub_part_y4 + 1,
                                        1,
                                        ref_idx_i8,
                                    );
                                    let mv11_x = pred11_x + sub_mv_x[sub_idx][3];
                                    let mv11_y = pred11_y + sub_mv_y[sub_idx][3];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x + 4,
                                        sub_y + 4,
                                        4,
                                        4,
                                        mv11_x,
                                        mv11_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x + 4,
                                        sub_y + 4,
                                        4,
                                        4,
                                        mv11_x,
                                        mv11_y,
                                        ref_idx_i8,
                                    );
                                    final_mv_x = mv11_x;
                                    final_mv_y = mv11_y;
                                    final_ref_idx = ref_idx;
                                }
                                _ => {
                                    let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_partition(
                                        mb_x,
                                        mb_y,
                                        sub_part_x4,
                                        sub_part_y4,
                                        2,
                                        ref_idx_i8,
                                    );
                                    let mv_x = pred_mv_x + sub_mv_x[sub_idx][0];
                                    let mv_y = pred_mv_y + sub_mv_y[sub_idx][0];
                                    self.apply_inter_block_l0(
                                        &ref_l0_list,
                                        ref_idx,
                                        sub_x,
                                        sub_y,
                                        8,
                                        8,
                                        mv_x,
                                        mv_y,
                                        &header.l0_weights,
                                        header.luma_log2_weight_denom,
                                        header.chroma_log2_weight_denom,
                                    );
                                    self.set_l0_motion_block_4x4(
                                        sub_x, sub_y, 8, 8, mv_x, mv_y, ref_idx_i8,
                                    );
                                    final_mv_x = mv_x;
                                    final_mv_y = mv_y;
                                    final_ref_idx = ref_idx;
                                }
                            }
                        }
                    }
                    _ => {
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            0,
                            base_x,
                            base_y,
                            16,
                            16,
                            0,
                            0,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                    }
                }
                self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                self.ref_idx_l0[mb_idx] = final_ref_idx.min(i8::MAX as u32) as i8;
                // Inter MB 残差解码
                self.decode_cavlc_mb_residual(
                    &mut br,
                    mb_x,
                    mb_y,
                    &mut cur_qp,
                    false,
                    no_sub_mb_part_size_less_than_8x8_flag,
                );
                self.mb_slice_first_mb[mb_idx] = saved_first_mb;
            }
            if self.take_cavlc_block_error() {
                let err = format!(
                    "H264: CAVLC 残差解码失败后中止 slice, mb_idx={}, first_mb={}",
                    mb_idx, header.first_mb
                );
                self.record_mb_decode_error(mb_idx, header.first_mb, "slice_cavlc_residual", &err);
                if relax_unknown_neighbors {
                    self.mb_slice_first_mb[mb_idx] = saved_slice_first_mb;
                }
                break;
            }
            if mb_idx < self.mb_qp.len() {
                self.mb_qp[mb_idx] = self.qp_for_mb_store(mb_idx, cur_qp);
            }
        }
    }
}

/// 判断 RBSP 是否仍有有效语法数据 (排除 rbsp_trailing_bits).
fn has_more_rbsp_data(br: &mut BitReader) -> bool {
    if br.bits_left() == 0 {
        return false;
    }
    let data = br.data();
    let start_bit = br.bits_read();
    let total_bits = data.len().saturating_mul(8);
    if start_bit >= total_bits {
        return false;
    }

    let bit_at = |idx: usize| -> u8 {
        let byte = data[idx / 8];
        (byte >> (7 - (idx % 8))) & 1
    };
    // rbsp_trailing_bits: stop_bit(1) + 全 0 对齐位.
    // 若当前位置不是 1, 或 stop_bit 后仍存在 1, 说明还有有效语法数据.
    if bit_at(start_bit) == 0 {
        return true;
    }
    for idx in (start_bit + 1)..total_bits {
        if bit_at(idx) != 0 {
            return true;
        }
    }
    false
}
