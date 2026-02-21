use super::*;

// ============================================================
// Slice 解码
// ============================================================

impl H264Decoder {
    /// 解码一个 VCL NAL (slice)
    pub(super) fn decode_slice(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();

        if let Ok(header) = self.parse_slice_header(&rbsp, nalu) {
            let prev_frame_num = self.last_frame_num;
            self.last_slice_type = header.slice_type;
            self.last_nal_ref_idc = header.nal_ref_idc;
            self.last_slice_qp = header.slice_qp;
            self.last_disable_deblocking_filter_idc = header.disable_deblocking_filter_idc;
            self.last_slice_alpha_c0_offset_div2 = header.slice_alpha_c0_offset_div2;
            self.last_slice_beta_offset_div2 = header.slice_beta_offset_div2;
            self.last_poc = self.compute_slice_poc(&header, prev_frame_num);
            self.last_frame_num = header.frame_num;
            self.last_dec_ref_pic_marking = header.dec_ref_pic_marking.clone();
            self.decode_slice_data(&rbsp, &header);
        }
    }

    /// 解析 slice header, 返回 CABAC 数据起始位置
    pub(super) fn parse_slice_header(&self, rbsp: &[u8], nalu: &NalUnit) -> TaoResult<SliceHeader> {
        let mut br = BitReader::new(rbsp);

        let first_mb = read_ue(&mut br)?;
        let slice_type = read_ue(&mut br)? % 5;
        let pps_id = read_ue(&mut br)?;
        let pps = self
            .pps_map
            .get(&pps_id)
            .or({
                if self.pps_map.is_empty() {
                    self.pps.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 PPS id={}", pps_id)))?;
        let sps = self
            .sps_map
            .get(&pps.sps_id)
            .or({
                if self.sps_map.is_empty() {
                    self.sps.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 SPS id={}", pps.sps_id)))?;

        // frame_num
        let frame_num = br.read_bits(sps.log2_max_frame_num)?;

        let mut field_pic = false;
        if !sps.frame_mbs_only {
            field_pic = br.read_bit()? == 1;
            if field_pic {
                let _bottom_field_flag = br.read_bit()?;
            }
        }

        // IDR 特有字段
        if nalu.nal_type == NalUnitType::SliceIdr {
            let _idr_pic_id = read_ue(&mut br)?;
        }

        // pic_order_cnt
        let mut pic_order_cnt_lsb = None;
        let mut delta_poc_bottom = 0i32;
        let mut delta_poc_0 = 0i32;
        let mut delta_poc_1 = 0i32;
        if sps.poc_type == 0 {
            let poc_lsb = br.read_bits(sps.log2_max_poc_lsb)?;
            pic_order_cnt_lsb = Some(poc_lsb);
            if pps.pic_order_present && !field_pic {
                delta_poc_bottom = read_se(&mut br)?;
            }
        } else if sps.poc_type == 1 && !sps.delta_pic_order_always_zero_flag {
            delta_poc_0 = read_se(&mut br)?;
            if pps.pic_order_present && !field_pic {
                delta_poc_1 = read_se(&mut br)?;
            }
        }

        // 参考索引数量
        if pps.redundant_pic_cnt_present {
            let _redundant_pic_cnt = read_ue(&mut br)?;
        }
        let mut num_ref_idx_l0 = pps.num_ref_idx_l0_default_active;
        let mut num_ref_idx_l1 = pps.num_ref_idx_l1_default_active;

        let is_b = slice_type == 1;
        let is_i = slice_type == 2 || slice_type == 4;
        if !is_i {
            if is_b {
                let _direct_spatial_mv_pred_flag = br.read_bit()?;
            }
            let override_refs = br.read_bit()? == 1;
            if override_refs {
                num_ref_idx_l0 = read_ue(&mut br)? + 1;
                if is_b {
                    num_ref_idx_l1 = read_ue(&mut br)? + 1;
                }
            }
            if num_ref_idx_l0 == 0 || num_ref_idx_l0 > 32 {
                return Err(TaoError::InvalidData(format!(
                    "H264: num_ref_idx_l0_active_minus1 非法, value={}",
                    num_ref_idx_l0.saturating_sub(1)
                )));
            }
            if is_b && (num_ref_idx_l1 == 0 || num_ref_idx_l1 > 32) {
                return Err(TaoError::InvalidData(format!(
                    "H264: num_ref_idx_l1_active_minus1 非法, value={}",
                    num_ref_idx_l1.saturating_sub(1)
                )));
            }
        }

        let (ref_pic_list_mod_l0, ref_pic_list_mod_l1) =
            self.parse_ref_pic_list_mod(&mut br, slice_type, num_ref_idx_l0, num_ref_idx_l1)?;
        let (luma_log2_weight_denom, chroma_log2_weight_denom, l0_weights, l1_weights) = self
            .parse_pred_weight_table(
                &mut br,
                sps,
                pps,
                slice_type,
                num_ref_idx_l0,
                num_ref_idx_l1,
            )?;
        let dec_ref_pic_marking = self.parse_dec_ref_pic_marking(&mut br, nalu)?;

        // CABAC init
        let mut cabac_init_idc = 0u8;
        if pps.entropy_coding_mode == 1 && !is_i {
            let cabac_init_idc_raw = read_ue(&mut br)?;
            if cabac_init_idc_raw > 2 {
                return Err(TaoError::InvalidData(format!(
                    "H264: cabac_init_idc 非法, value={}",
                    cabac_init_idc_raw
                )));
            }
            cabac_init_idc = cabac_init_idc_raw as u8;
        }

        // slice_qp_delta
        let qp_delta = read_se(&mut br)?;
        let slice_qp = pps.pic_init_qp + qp_delta;
        if !(0..=51).contains(&slice_qp) {
            return Err(TaoError::InvalidData(format!(
                "H264: slice_qp 超出范围, slice_qp={}",
                slice_qp
            )));
        }

        // 跳过去块效应滤波器参数
        let mut disable_deblocking_filter_idc = 0u32;
        let mut slice_alpha_c0_offset_div2 = 0i32;
        let mut slice_beta_offset_div2 = 0i32;
        if pps.deblocking_filter_control {
            let disable = read_ue(&mut br)?;
            if disable > 2 {
                return Err(TaoError::InvalidData(format!(
                    "H264: disable_deblocking_filter_idc 非法, value={}",
                    disable
                )));
            }
            disable_deblocking_filter_idc = disable;
            if disable != 1 {
                let alpha = read_se(&mut br)?;
                let beta = read_se(&mut br)?;
                if !(-6..=6).contains(&alpha) {
                    return Err(TaoError::InvalidData(format!(
                        "H264: slice_alpha_c0_offset_div2 超出范围, value={}",
                        alpha
                    )));
                }
                if !(-6..=6).contains(&beta) {
                    return Err(TaoError::InvalidData(format!(
                        "H264: slice_beta_offset_div2 超出范围, value={}",
                        beta
                    )));
                }
                slice_alpha_c0_offset_div2 = alpha;
                slice_beta_offset_div2 = beta;
            }
        }

        let mut data_bit_offset = br.bits_read();
        if pps.entropy_coding_mode == 1 {
            while br.bits_read() & 7 != 0 {
                let _cabac_alignment_one_bit = br.read_bit()?;
            }
            data_bit_offset = br.bits_read();
        }
        let cabac_start = br.byte_position();

        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if debug_mb {
            eprintln!(
                "[H264][SliceHeader] first_mb={}, pps_id={}, slice_type={}, frame_num={}, slice_qp={}, cabac_init_idc={}, cabac_start_byte={}, idr={}",
                first_mb,
                pps_id,
                slice_type,
                frame_num,
                slice_qp,
                cabac_init_idc,
                cabac_start,
                nalu.nal_type == NalUnitType::SliceIdr
            );
        }

        Ok(SliceHeader {
            first_mb,
            pps_id,
            slice_type,
            frame_num,
            slice_qp,
            cabac_init_idc,
            num_ref_idx_l0,
            num_ref_idx_l1,
            ref_pic_list_mod_l0,
            ref_pic_list_mod_l1,
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
            l0_weights,
            l1_weights,
            data_bit_offset,
            cabac_start_byte: cabac_start,
            nal_ref_idc: nalu.ref_idc,
            is_idr: nalu.nal_type == NalUnitType::SliceIdr,
            pic_order_cnt_lsb,
            delta_poc_bottom,
            delta_poc_0,
            delta_poc_1,
            disable_deblocking_filter_idc,
            slice_alpha_c0_offset_div2,
            slice_beta_offset_div2,
            dec_ref_pic_marking,
        })
    }

    pub(super) fn compute_slice_poc(&mut self, header: &SliceHeader, prev_frame_num: u32) -> i32 {
        let Some(sps) = self.sps.as_ref() else {
            return header.frame_num as i32;
        };

        if header.is_idr {
            self.prev_ref_poc_msb = 0;
            self.prev_ref_poc_lsb = 0;
            self.prev_frame_num_offset_type1 = 0;
            self.prev_frame_num_offset_type2 = 0;
        }

        match sps.poc_type {
            0 => {
                let Some(poc_lsb_u32) = header.pic_order_cnt_lsb else {
                    return header.frame_num as i32;
                };
                let max_poc_lsb = 1i32 << sps.log2_max_poc_lsb.min(30);
                let poc_lsb = poc_lsb_u32 as i32;

                let mut poc_msb = self.prev_ref_poc_msb;
                if !header.is_idr {
                    if poc_lsb < self.prev_ref_poc_lsb
                        && (self.prev_ref_poc_lsb - poc_lsb) >= (max_poc_lsb / 2)
                    {
                        poc_msb += max_poc_lsb;
                    } else if poc_lsb > self.prev_ref_poc_lsb
                        && (poc_lsb - self.prev_ref_poc_lsb) > (max_poc_lsb / 2)
                    {
                        poc_msb -= max_poc_lsb;
                    }
                }

                let poc = poc_msb + poc_lsb + header.delta_poc_bottom;
                if header.nal_ref_idc != 0 {
                    self.prev_ref_poc_msb = poc_msb;
                    self.prev_ref_poc_lsb = poc_lsb;
                }
                poc
            }
            1 => {
                let max_frame_num = self.max_frame_num_modulo() as i32;
                if max_frame_num <= 0 {
                    return header.frame_num as i32;
                }
                let frame_num = header.frame_num as i32;
                let prev_num = prev_frame_num as i32;
                let mut frame_num_offset = if header.is_idr {
                    0
                } else {
                    self.prev_frame_num_offset_type1
                };
                if !header.is_idr && prev_num > frame_num {
                    frame_num_offset += max_frame_num;
                }

                let mut abs_frame_num = if sps.max_num_ref_frames == 0 {
                    0
                } else {
                    frame_num_offset + frame_num
                };
                if header.nal_ref_idc == 0 && abs_frame_num > 0 {
                    abs_frame_num -= 1;
                }

                let mut expected_poc = 0i32;
                if abs_frame_num > 0 && !sps.offset_for_ref_frame.is_empty() {
                    let cycle_len = sps.offset_for_ref_frame.len() as i32;
                    let expected_delta_per_cycle: i32 = sps.offset_for_ref_frame.iter().sum();
                    let pic_order_cnt_cycle_cnt = (abs_frame_num - 1) / cycle_len;
                    let frame_num_in_cycle = (abs_frame_num - 1) % cycle_len;
                    expected_poc = pic_order_cnt_cycle_cnt * expected_delta_per_cycle;
                    for i in 0..=frame_num_in_cycle {
                        expected_poc += sps.offset_for_ref_frame[i as usize];
                    }
                }
                if header.nal_ref_idc == 0 {
                    expected_poc += sps.offset_for_non_ref_pic;
                }

                let top = expected_poc + header.delta_poc_0;
                let bottom = top + sps.offset_for_top_to_bottom_field + header.delta_poc_1;
                if header.nal_ref_idc != 0 {
                    self.prev_frame_num_offset_type1 = frame_num_offset;
                }
                top.min(bottom)
            }
            2 => {
                let max_frame_num = self.max_frame_num_modulo() as i32;
                if max_frame_num <= 0 {
                    return header.frame_num as i32;
                }
                let mut frame_num_offset = if header.is_idr {
                    0
                } else {
                    self.prev_frame_num_offset_type2
                };
                let frame_num = header.frame_num as i32;
                let prev_num = prev_frame_num as i32;
                if !header.is_idr && prev_num > frame_num {
                    frame_num_offset += max_frame_num;
                }

                let mut poc = 2 * (frame_num_offset + frame_num);
                if header.nal_ref_idc == 0 {
                    poc -= 1;
                }
                self.prev_frame_num_offset_type2 = frame_num_offset;
                poc
            }
            _ => header.frame_num as i32,
        }
    }

    /// 解析参考图像列表修改语法
    pub(super) fn parse_ref_pic_list_mod(
        &self,
        br: &mut BitReader,
        slice_type: u32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
    ) -> TaoResult<(Vec<RefPicListMod>, Vec<RefPicListMod>)> {
        let mut mods_l0 = Vec::new();
        let mut mods_l1 = Vec::new();
        if slice_type == 2 || slice_type == 4 {
            return Ok((mods_l0, mods_l1));
        }

        let reorder_l0 = br.read_bit()?;
        if reorder_l0 == 1 && num_ref_idx_l0 > 0 {
            mods_l0 = self.parse_single_ref_pic_list_mod(br)?;
        }

        if slice_type == 1 {
            let reorder_l1 = br.read_bit()?;
            if reorder_l1 == 1 && num_ref_idx_l1 > 0 {
                mods_l1 = self.parse_single_ref_pic_list_mod(br)?;
            }
        }
        Ok((mods_l0, mods_l1))
    }

    pub(super) fn parse_single_ref_pic_list_mod(
        &self,
        br: &mut BitReader,
    ) -> TaoResult<Vec<RefPicListMod>> {
        let mut mods = Vec::new();
        loop {
            let op = read_ue(br)?;
            match op {
                0 => {
                    let abs_diff_pic_num_minus1 = read_ue(br)?;
                    mods.push(RefPicListMod::ShortTermSub {
                        abs_diff_pic_num_minus1,
                    });
                }
                1 => {
                    let abs_diff_pic_num_minus1 = read_ue(br)?;
                    mods.push(RefPicListMod::ShortTermAdd {
                        abs_diff_pic_num_minus1,
                    });
                }
                2 => {
                    let long_term_pic_num = read_ue(br)?;
                    mods.push(RefPicListMod::LongTerm { long_term_pic_num });
                }
                3 => break,
                _ => {
                    return Err(TaoError::InvalidData(format!(
                        "H264: ref_pic_list_modification_idc 非法, value={}",
                        op
                    )));
                }
            }
            if mods.len() > 96 {
                return Err(TaoError::InvalidData(
                    "H264: ref_pic_list_modification 项数过多".into(),
                ));
            }
        }
        Ok(mods)
    }

    /// 解析并(按需)返回 list0 加权预测参数.
    pub(super) fn parse_pred_weight_table(
        &self,
        br: &mut BitReader,
        sps: &Sps,
        pps: &Pps,
        slice_type: u32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
    ) -> TaoResult<(u8, u8, Vec<PredWeightL0>, Vec<PredWeightL0>)> {
        let use_weight_l0 = pps.weighted_pred && (slice_type == 0 || slice_type == 3);
        let use_weight_l1 = pps.weighted_bipred_idc == 1 && slice_type == 1;
        if !use_weight_l0 && !use_weight_l1 {
            return Ok((0, 0, Vec::new(), Vec::new()));
        }

        let luma_log2_weight_denom_raw = read_ue(br)?;
        if luma_log2_weight_denom_raw > 7 {
            return Err(TaoError::InvalidData(format!(
                "H264: luma_log2_weight_denom 非法, value={}",
                luma_log2_weight_denom_raw
            )));
        }
        let luma_log2_weight_denom = luma_log2_weight_denom_raw as u8;
        let mut chroma_present = false;
        let mut chroma_log2_weight_denom = 0u8;
        if sps.chroma_format_idc != 0 {
            chroma_present = true;
            let chroma_log2_weight_denom_raw = read_ue(br)?;
            if chroma_log2_weight_denom_raw > 7 {
                return Err(TaoError::InvalidData(format!(
                    "H264: chroma_log2_weight_denom 非法, value={}",
                    chroma_log2_weight_denom_raw
                )));
            }
            chroma_log2_weight_denom = chroma_log2_weight_denom_raw as u8;
        }

        let need_parse_l0 = use_weight_l0 || use_weight_l1;
        let mut l0_weights = Vec::new();
        if need_parse_l0 {
            for _ in 0..num_ref_idx_l0 {
                let mut w = PredWeightL0 {
                    luma_weight: 1 << luma_log2_weight_denom,
                    luma_offset: 0,
                    chroma_weight: [1 << chroma_log2_weight_denom; 2],
                    chroma_offset: [0, 0],
                };
                let luma_weight_flag = br.read_bit()?;
                if luma_weight_flag == 1 {
                    w.luma_weight = read_se(br)?;
                    w.luma_offset = read_se(br)?;
                    if !(-128..=127).contains(&w.luma_weight) {
                        return Err(TaoError::InvalidData(format!(
                            "H264: luma_weight_l0 超出范围, value={}",
                            w.luma_weight
                        )));
                    }
                    if !(-128..=127).contains(&w.luma_offset) {
                        return Err(TaoError::InvalidData(format!(
                            "H264: luma_offset_l0 超出范围, value={}",
                            w.luma_offset
                        )));
                    }
                }
                if chroma_present {
                    let chroma_weight_flag = br.read_bit()?;
                    if chroma_weight_flag == 1 {
                        for c in 0..2 {
                            w.chroma_weight[c] = read_se(br)?;
                            w.chroma_offset[c] = read_se(br)?;
                            if !(-128..=127).contains(&w.chroma_weight[c]) {
                                return Err(TaoError::InvalidData(format!(
                                    "H264: chroma_weight_l0[{}] 超出范围, value={}",
                                    c, w.chroma_weight[c]
                                )));
                            }
                            if !(-128..=127).contains(&w.chroma_offset[c]) {
                                return Err(TaoError::InvalidData(format!(
                                    "H264: chroma_offset_l0[{}] 超出范围, value={}",
                                    c, w.chroma_offset[c]
                                )));
                            }
                        }
                    }
                }
                l0_weights.push(w);
            }
        }
        let mut l1_weights = Vec::new();
        if use_weight_l1 {
            for _ in 0..num_ref_idx_l1 {
                let mut w = PredWeightL0 {
                    luma_weight: 1 << luma_log2_weight_denom,
                    luma_offset: 0,
                    chroma_weight: [1 << chroma_log2_weight_denom; 2],
                    chroma_offset: [0, 0],
                };
                let luma_weight_flag = br.read_bit()?;
                if luma_weight_flag == 1 {
                    w.luma_weight = read_se(br)?;
                    w.luma_offset = read_se(br)?;
                    if !(-128..=127).contains(&w.luma_weight) {
                        return Err(TaoError::InvalidData(format!(
                            "H264: luma_weight_l1 超出范围, value={}",
                            w.luma_weight
                        )));
                    }
                    if !(-128..=127).contains(&w.luma_offset) {
                        return Err(TaoError::InvalidData(format!(
                            "H264: luma_offset_l1 超出范围, value={}",
                            w.luma_offset
                        )));
                    }
                }
                if chroma_present {
                    let chroma_weight_flag = br.read_bit()?;
                    if chroma_weight_flag == 1 {
                        for c in 0..2 {
                            w.chroma_weight[c] = read_se(br)?;
                            w.chroma_offset[c] = read_se(br)?;
                            if !(-128..=127).contains(&w.chroma_weight[c]) {
                                return Err(TaoError::InvalidData(format!(
                                    "H264: chroma_weight_l1[{}] 超出范围, value={}",
                                    c, w.chroma_weight[c]
                                )));
                            }
                            if !(-128..=127).contains(&w.chroma_offset[c]) {
                                return Err(TaoError::InvalidData(format!(
                                    "H264: chroma_offset_l1[{}] 超出范围, value={}",
                                    c, w.chroma_offset[c]
                                )));
                            }
                        }
                    }
                }
                l1_weights.push(w);
            }
        }
        Ok((
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
            l0_weights,
            l1_weights,
        ))
    }

    /// 解析 dec_ref_pic_marking 语法.
    pub(super) fn parse_dec_ref_pic_marking(
        &self,
        br: &mut BitReader,
        nalu: &NalUnit,
    ) -> TaoResult<DecRefPicMarking> {
        let mut marking = DecRefPicMarking::default();
        if nalu.nal_type == NalUnitType::SliceIdr {
            marking.is_idr = true;
            marking.no_output_of_prior_pics = br.read_bit()? == 1;
            marking.long_term_reference_flag = br.read_bit()? == 1;
            return Ok(marking);
        }
        if nalu.ref_idc == 0 {
            return Ok(marking);
        }

        marking.adaptive = br.read_bit()? == 1;
        if !marking.adaptive {
            return Ok(marking);
        }

        loop {
            let op = read_ue(br)?;
            match op {
                0 => break,
                1 => {
                    let difference = read_ue(br)?;
                    marking.ops.push(MmcoOp::ForgetShort {
                        difference_of_pic_nums_minus1: difference,
                    });
                }
                2 => {
                    let long_term_pic_num = read_ue(br)?;
                    marking.ops.push(MmcoOp::ForgetLong { long_term_pic_num });
                }
                3 => {
                    let difference = read_ue(br)?;
                    let long_term_frame_idx = read_ue(br)?;
                    marking.ops.push(MmcoOp::ConvertShortToLong {
                        difference_of_pic_nums_minus1: difference,
                        long_term_frame_idx,
                    });
                }
                4 => {
                    let max_long_term_frame_idx_plus1 = read_ue(br)?;
                    marking.ops.push(MmcoOp::TrimLong {
                        max_long_term_frame_idx_plus1,
                    });
                }
                5 => marking.ops.push(MmcoOp::ClearAll),
                6 => {
                    let long_term_frame_idx = read_ue(br)?;
                    marking.ops.push(MmcoOp::MarkCurrentLong {
                        long_term_frame_idx,
                    });
                }
                _ => {
                    return Err(TaoError::InvalidData(format!(
                        "H264: MMCO op 非法, op={}",
                        op
                    )));
                }
            }
        }
        Ok(marking)
    }

    /// 解码 slice 数据 (MB 循环)
    pub(super) fn decode_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        if self.activate_parameter_sets(header.pps_id).is_err() {
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
        let _num_ref_idx_l1 = header.num_ref_idx_l1;

        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;

        if is_i {
            self.decode_i_slice_mbs(&mut cabac, &mut ctxs, first, total_mbs, header.slice_qp);
            return;
        }

        if header.slice_type == 0 || header.slice_type == 3 {
            let ref_l0_list = self.build_reference_list_l0_with_mod(
                header.num_ref_idx_l0,
                &header.ref_pic_list_mod_l0,
                header.frame_num,
            );
            self.decode_p_slice_mbs(
                &mut cabac,
                &mut ctxs,
                first,
                total_mbs,
                header.slice_qp,
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
        let ref_l1_list = self.build_reference_list_l1_with_mod(
            header.num_ref_idx_l1,
            &header.ref_pic_list_mod_l1,
            header.frame_num,
        );
        self.decode_b_slice_mbs(
            &mut cabac,
            &mut ctxs,
            first,
            total_mbs,
            header.slice_qp,
            header.num_ref_idx_l0,
            header.num_ref_idx_l1,
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

    pub(super) fn copy_macroblock_from_planes(
        &mut self,
        mb_x: usize,
        mb_y: usize,
        ref_src: &RefPlanes,
    ) {
        let y_base_x = mb_x * 16;
        let y_base_y = mb_y * 16;
        for y in 0..16usize {
            let dst_y = y_base_y + y;
            for x in 0..16usize {
                let dst_x = y_base_x + x;
                let dst_idx = dst_y * self.stride_y + dst_x;
                if dst_idx >= self.ref_y.len() {
                    continue;
                }
                let src_idx = dst_y * self.stride_y + dst_x;
                self.ref_y[dst_idx] = *ref_src.y.get(src_idx).unwrap_or(&128);
            }
        }

        let c_base_x = mb_x * 8;
        let c_base_y = mb_y * 8;
        for y in 0..8usize {
            let dst_y = c_base_y + y;
            for x in 0..8usize {
                let dst_x = c_base_x + x;
                let dst_idx = dst_y * self.stride_c + dst_x;
                if dst_idx >= self.ref_u.len() || dst_idx >= self.ref_v.len() {
                    continue;
                }
                self.ref_u[dst_idx] = *ref_src.u.get(dst_idx).unwrap_or(&128);
                self.ref_v[dst_idx] = *ref_src.v.get(dst_idx).unwrap_or(&128);
            }
        }
    }

    /// CAVLC 最小路径: 消费 `mb_skip_run/mb_type`, 并执行基础重建.
    pub(super) fn decode_cavlc_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;
        if first >= total_mbs {
            return;
        }

        let mut br = BitReader::new(rbsp);
        if br.skip_bits(header.data_bit_offset as u32).is_err() {
            self.apply_dc_fallback();
            return;
        }

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        let is_b = header.slice_type == 1;
        if is_i {
            for mb_idx in first..total_mbs {
                let _mb_type = read_ue(&mut br).unwrap_or(0);
                self.mb_types[mb_idx] = 1;
                self.mb_cbp[mb_idx] = 0;
                let mb_x = mb_idx % self.mb_width;
                let mb_y = mb_idx / self.mb_width;
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
            return;
        }

        let ref_l0_list = self.build_reference_list_l0_with_mod(
            header.num_ref_idx_l0,
            &header.ref_pic_list_mod_l0,
            header.frame_num,
        );
        let ref_l1_list = if is_b {
            self.build_reference_list_l1_with_mod(
                header.num_ref_idx_l1,
                &header.ref_pic_list_mod_l1,
                header.frame_num,
            )
        } else {
            Vec::new()
        };
        let ref_l0 = ref_l0_list
            .first()
            .cloned()
            .unwrap_or_else(|| self.zero_reference_planes());
        let mut skip_run_left = 0u32;
        for mb_idx in first..total_mbs {
            if skip_run_left == 0 {
                skip_run_left = read_ue(&mut br).unwrap_or(0);
            }
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            if skip_run_left > 0 {
                self.mb_types[mb_idx] = if is_b { 254 } else { 255 };
                self.mb_cbp[mb_idx] = 0;
                if is_b {
                    let _ = self.apply_b_prediction_block(
                        Some(BMotion {
                            mv_x: 0,
                            mv_y: 0,
                            ref_idx: 0,
                        }),
                        Some(BMotion {
                            mv_x: 0,
                            mv_y: 0,
                            ref_idx: 0,
                        }),
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
                } else {
                    self.copy_macroblock_from_planes(mb_x, mb_y, &ref_l0);
                }
                skip_run_left -= 1;
                continue;
            }
            let mb_type = read_ue(&mut br).unwrap_or(0);
            if is_b {
                let is_inter = mb_type <= 22;
                if is_inter {
                    self.mb_types[mb_idx] = 254;
                    self.mb_cbp[mb_idx] = 0;
                    let mut l0_ref_idx = 0usize;
                    let mut l1_ref_idx = 0usize;
                    if mb_type == 1 && header.num_ref_idx_l0 > 1 {
                        l0_ref_idx = read_ue(&mut br).unwrap_or(0) as usize;
                    }
                    if mb_type == 2 && header.num_ref_idx_l1 > 1 {
                        l1_ref_idx = read_ue(&mut br).unwrap_or(0) as usize;
                    }
                    let mut l0_motion = Some(BMotion {
                        mv_x: 0,
                        mv_y: 0,
                        ref_idx: l0_ref_idx.min(i8::MAX as usize) as i8,
                    });
                    let mut l1_motion = Some(BMotion {
                        mv_x: 0,
                        mv_y: 0,
                        ref_idx: l1_ref_idx.min(i8::MAX as usize) as i8,
                    });
                    if mb_type == 1 {
                        l1_motion = None;
                    } else if mb_type == 2 {
                        l0_motion = None;
                    }
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
                } else {
                    self.mb_types[mb_idx] = 1;
                    self.mb_cbp[mb_idx] = 0;
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
                continue;
            }
            self.mb_cbp[mb_idx] = 0;
            if mb_type >= 5 {
                self.mb_types[mb_idx] = 1;
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
            } else {
                self.mb_types[mb_idx] = 200u8.saturating_add((mb_type as u8).min(3));
                let base_x = mb_x * 16;
                let base_y = mb_y * 16;
                match mb_type {
                    0 => {
                        let mut ref_idx_l0 = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_l0 = read_ue(&mut br).unwrap_or(0);
                        }
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_l0,
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
                    1 => {
                        let mut ref_idx_top = 0u32;
                        let mut ref_idx_bottom = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_top = read_ue(&mut br).unwrap_or(0);
                            ref_idx_bottom = read_ue(&mut br).unwrap_or(0);
                        }
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_top,
                            base_x,
                            base_y,
                            16,
                            8,
                            0,
                            0,
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
                            0,
                            0,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                    }
                    2 => {
                        let mut ref_idx_left = 0u32;
                        let mut ref_idx_right = 0u32;
                        if header.num_ref_idx_l0 > 1 {
                            ref_idx_left = read_ue(&mut br).unwrap_or(0);
                            ref_idx_right = read_ue(&mut br).unwrap_or(0);
                        }
                        self.apply_inter_block_l0(
                            &ref_l0_list,
                            ref_idx_left,
                            base_x,
                            base_y,
                            8,
                            16,
                            0,
                            0,
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
                            0,
                            0,
                            &header.l0_weights,
                            header.luma_log2_weight_denom,
                            header.chroma_log2_weight_denom,
                        );
                    }
                    _ => {
                        self.copy_macroblock_from_planes(mb_x, mb_y, &ref_l0);
                    }
                }
            }
        }
    }
}
