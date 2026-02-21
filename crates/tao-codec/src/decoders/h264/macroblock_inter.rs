use super::*;

impl H264Decoder {
    /// 构建 B-slice Direct 预测的最小运动信息.
    ///
    /// 目前 temporal direct 路径先复用 list0 预测, spatial direct 路径使用 list0/list1 双向预测.
    pub(super) fn build_b_direct_motion(
        &self,
        mv_x: i32,
        mv_y: i32,
        direct_spatial_mv_pred_flag: bool,
    ) -> (Option<BMotion>, Option<BMotion>) {
        let motion_l0 = Some(BMotion {
            mv_x,
            mv_y,
            ref_idx: 0,
        });
        let motion_l1 = if direct_spatial_mv_pred_flag {
            Some(BMotion {
                mv_x,
                mv_y,
                ref_idx: 0,
            })
        } else {
            None
        };
        (motion_l0, motion_l1)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_p_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        num_ref_idx_l0: u32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let mut decoded = 0usize;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_p_mb_skip_flag(cabac, ctxs, mb_x, mb_y);
            decoded += 1;

            if skip {
                self.mb_types[mb_idx] = 255;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
                let (pred_x, pred_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
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
                self.mv_l0_x[mb_idx] = pred_x as i16;
                self.mv_l0_y[mb_idx] = pred_y as i16;
                self.ref_idx_l0[mb_idx] = 0;
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
                if debug_mb {
                    eprintln!(
                        "[H264][P-slice] 提前结束: first_mb={}, total_mbs={}, decoded_mbs={}, last_mb=({}, {}), cabac_bits={}/{}",
                        first,
                        total,
                        decoded,
                        mb_x,
                        mb_y,
                        cabac.bit_pos(),
                        cabac.total_bits()
                    );
                }
                break;
            }
        }

        if debug_mb {
            eprintln!(
                "[H264][P-slice] 完成: first_mb={}, total_mbs={}, decoded_mbs={}, cabac_bits={}/{}",
                first,
                total,
                decoded,
                cabac.bit_pos(),
                cabac.total_bits()
            );
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

    pub(super) fn predict_mv_l0_16x16(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        let left = if mb_x > 0 {
            self.mb_index(mb_x - 1, mb_y)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };
        let top = if mb_y > 0 {
            self.mb_index(mb_x, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };
        let top_right = if mb_y > 0 && mb_x + 1 < self.mb_width {
            self.mb_index(mb_x + 1, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else if mb_x > 0 && mb_y > 0 {
            self.mb_index(mb_x - 1, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };

        let a = left.unwrap_or((0, 0));
        let b = top.unwrap_or(a);
        let c = top_right.unwrap_or(b);
        (
            median3(a.0 as i32, b.0 as i32, c.0 as i32),
            median3(a.1 as i32, b.1 as i32, c.1 as i32),
        )
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
        let td = (ref_l1_poc - ref_l0_poc).clamp(-128, 127);
        if td == 0 {
            return (32, 32);
        }
        let tb = (self.last_poc - ref_l0_poc).clamp(-128, 127);
        let tx = (16384 + (td.abs() >> 1)) / td;
        let dist_scale_factor = ((tb * tx + 32) >> 6).clamp(-1024, 1023);
        let w1 = dist_scale_factor >> 2;
        if (-64..=128).contains(&w1) {
            (64 - w1, w1)
        } else {
            (32, 32)
        }
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

        let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
        let mut final_mv_x = pred_mv_x;
        let mut final_mv_y = pred_mv_y;
        let mut final_ref_idx = 0u32;

        match p_mb_type {
            0 => {
                final_ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                final_mv_x += mvd_x;
                final_mv_y += mvd_y;
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
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = pred_mv_x + mvd_x;
                    let mv_y = pred_mv_y + mvd_y;
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
                }
            }
            2 => {
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = pred_mv_x + mvd_x;
                    let mv_y = pred_mv_y + mvd_y;
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
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let mut decoded = 0usize;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_b_mb_skip_flag(cabac, ctxs, mb_x, mb_y);
            decoded += 1;

            if skip {
                self.mb_types[mb_idx] = 254;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
                let (pred_x, pred_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
                let (motion_l0, motion_l1) =
                    self.build_b_direct_motion(pred_x, pred_y, direct_spatial_mv_pred_flag);
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
                if debug_mb {
                    eprintln!(
                        "[H264][B-slice] 提前结束: first_mb={}, total_mbs={}, decoded_mbs={}, last_mb=({}, {}), cabac_bits={}/{}",
                        first,
                        total,
                        decoded,
                        mb_x,
                        mb_y,
                        cabac.bit_pos(),
                        cabac.total_bits()
                    );
                }
                break;
            }
        }

        if debug_mb {
            eprintln!(
                "[H264][B-slice] 完成: first_mb={}, total_mbs={}, decoded_mbs={}, cabac_bits={}/{}",
                first,
                total,
                decoded,
                cabac.bit_pos(),
                cabac.total_bits()
            );
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
                (m1.mv_x, m1.mv_y, m1.ref_idx)
            }
            (None, None) => (0, 0, 0),
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

        let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
        let mut final_mv_x = pred_mv_x;
        let mut final_mv_y = pred_mv_y;
        let mut final_ref_idx = 0i8;

        match mb_type_idx {
            None => {
                self.mb_types[mb_idx] = 254;
                let (motion_l0, motion_l1) =
                    self.build_b_direct_motion(pred_mv_x, pred_mv_y, direct_spatial_mv_pred_flag);
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
                for (sub, sub_type) in sub_types.into_iter().enumerate() {
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    let (part_w, part_h, part_count, dir) = Self::b_sub_mb_info(sub_type);
                    for part in 0..part_count {
                        let (part_off_x, part_off_y) = match (part_w, part_h, part_count) {
                            (8, 8, _) => (0, 0),
                            (8, 4, _) => (0, part * 4),
                            (4, 8, _) => (part * 4, 0),
                            _ => ((part & 1) * 4, (part >> 1) * 4),
                        };
                        let (motion_l0, motion_l1) = self.decode_b_partition_motion(
                            cabac,
                            ctxs,
                            dir,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            pred_mv_x,
                            pred_mv_y,
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
                            mb_x * 16 + sx + part_off_x,
                            mb_y * 16 + sy + part_off_y,
                            part_w,
                            part_h,
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
                    for part in 0..part_count {
                        let dir = if part == 0 { dir0 } else { dir1 };
                        let (part_w, part_h, part_off_x, part_off_y) = match shape {
                            0 => (16usize, 16usize, 0usize, 0usize),
                            1 => (16usize, 8usize, 0usize, part * 8),
                            _ => (8usize, 16usize, part * 8, 0usize),
                        };
                        let (motion_l0, motion_l1) = self.decode_b_partition_motion(
                            cabac,
                            ctxs,
                            dir,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            pred_mv_x,
                            pred_mv_y,
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
                            mb_x * 16 + part_off_x,
                            mb_y * 16 + part_off_y,
                            part_w,
                            part_h,
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
    pub(super) fn decode_b_partition_motion(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        dir: BPredDir,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        pred_mv_x: i32,
        pred_mv_y: i32,
    ) -> (Option<BMotion>, Option<BMotion>) {
        let mut mv_l0_x = pred_mv_x;
        let mut mv_l0_y = pred_mv_y;
        let mut mv_l1_x = pred_mv_x;
        let mut mv_l1_y = pred_mv_y;
        let mut motion_l0 = None;
        let mut motion_l1 = None;

        if matches!(dir, BPredDir::L0 | BPredDir::Bi) {
            let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
            mv_l0_x += mvd_x;
            mv_l0_y += mvd_y;
            motion_l0 = Some(BMotion {
                mv_x: mv_l0_x,
                mv_y: mv_l0_y,
                ref_idx: ref_idx.min(i8::MAX as u32) as i8,
            });
        }
        if matches!(dir, BPredDir::L1 | BPredDir::Bi) {
            let ref_idx_l1 = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l1);
            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
            mv_l1_x += mvd_x;
            mv_l1_y += mvd_y;
            motion_l1 = Some(BMotion {
                mv_x: mv_l1_x,
                mv_y: mv_l1_y,
                ref_idx: ref_idx_l1.min(i8::MAX as u32) as i8,
            });
        }

        if motion_l0.is_none() && motion_l1.is_none() {
            (
                Some(BMotion {
                    mv_x: pred_mv_x,
                    mv_y: pred_mv_y,
                    ref_idx: 0,
                }),
                None,
            )
        } else {
            (motion_l0, motion_l1)
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
                residual::dequant_4x4_ac_with_scaling(&mut coeffs_arr, qp, &luma_scaling_4x4);
                residual::apply_4x4_ac_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16 + abs_sub_x * 4,
                    mb_y * 16 + abs_sub_y * 4,
                    &coeffs_arr,
                );
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }
}
