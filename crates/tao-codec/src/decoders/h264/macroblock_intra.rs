use super::*;

impl H264Decoder {
    pub(super) fn decode_i4x4_pred_modes(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 16] {
        // H.264 规范顺序: 按 8x8 块分组, 每组内按 2x2 子块顺序.
        const I4X4_SCAN_ORDER: [(usize, usize); 16] = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (0, 2),
            (1, 2),
            (0, 3),
            (1, 3),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
        ];

        let mut modes = [2u8; 16];
        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            let left = if x4 > 0 {
                self.get_i4x4_mode(x4 - 1, y4)
            } else {
                2
            };
            let top = if y4 > 0 {
                self.get_i4x4_mode(x4, y4 - 1)
            } else {
                2
            };
            let pred_mode = left.min(top);
            let prev_flag = cabac.decode_decision(&mut ctxs[68]);
            let mode = if prev_flag == 1 {
                pred_mode
            } else {
                let rem = (cabac.decode_decision(&mut ctxs[69])
                    | (cabac.decode_decision(&mut ctxs[69]) << 1)
                    | (cabac.decode_decision(&mut ctxs[69]) << 2)) as u8;
                if rem < pred_mode { rem } else { rem + 1 }
            };
            modes[sub_y * 4 + sub_x] = mode.min(8);
            self.set_i4x4_mode(x4, y4, mode);
        }
        modes
    }

    /// 解码 I_8x8 宏块的 4 个预测模式 (最小可用路径)
    pub(super) fn decode_i8x8_pred_modes(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 4] {
        let mut modes = [2u8; 4];
        for block_y in 0..2 {
            for block_x in 0..2 {
                let x4 = mb_x * 4 + block_x * 2;
                let y4 = mb_y * 4 + block_y * 2;
                let left = if x4 > 0 {
                    self.get_i4x4_mode(x4 - 1, y4)
                } else {
                    2
                };
                let top = if y4 > 0 {
                    self.get_i4x4_mode(x4, y4 - 1)
                } else {
                    2
                };
                let pred_mode = left.min(top);
                let prev_flag = cabac.decode_decision(&mut ctxs[68]);
                let mode = if prev_flag == 1 {
                    pred_mode
                } else {
                    let rem = (cabac.decode_decision(&mut ctxs[69])
                        | (cabac.decode_decision(&mut ctxs[69]) << 1)
                        | (cabac.decode_decision(&mut ctxs[69]) << 2))
                        as u8;
                    if rem < pred_mode { rem } else { rem + 1 }
                }
                .min(8);

                let idx = block_y * 2 + block_x;
                modes[idx] = mode;
                for sub_y in 0..2 {
                    for sub_x in 0..2 {
                        self.set_i4x4_mode(x4 + sub_x, y4 + sub_y, mode);
                    }
                }
            }
        }
        modes
    }

    /// 解码 I-slice 的所有宏块
    pub(super) fn decode_i_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        slice_first_mb: u32,
    ) {
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;

        for mb_idx in first..total {
            self.mark_mb_slice_first_mb(mb_idx, slice_first_mb);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;

            let mb_type = decode_i_mb_type(cabac, ctxs, &self.mb_types, self.mb_width, mb_x, mb_y);
            self.mb_types[mb_idx] = mb_type as u8;

            if mb_type == 0 {
                self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
            } else if mb_type <= 24 {
                self.decode_i_16x16_mb(cabac, ctxs, mb_x, mb_y, mb_type, &mut cur_qp);
            } else if mb_type == 25 {
                self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                self.prev_qp_delta_nz = false;
            }
            if mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                break;
            }
        }
    }

    /// 解码 P-slice 宏块.
    pub(super) fn decode_i_4x4_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        cur_qp: &mut i32,
    ) {
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        // 1. 可选 transform_size_8x8_flag + 预测模式
        let use_8x8 = self
            .pps
            .as_ref()
            .map(|p| p.transform_8x8_mode)
            .unwrap_or(false)
            && self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);
        let pred_modes_4x4 = if use_8x8 {
            [2u8; 16]
        } else {
            self.decode_i4x4_pred_modes(cabac, ctxs, mb_x, mb_y)
        };
        let pred_modes_8x8 = if use_8x8 {
            self.decode_i8x8_pred_modes(cabac, ctxs, mb_x, mb_y)
        } else {
            [2u8; 4]
        };

        // 2. 解码 intra_chroma_pred_mode
        let chroma_mode = self.decode_chroma_pred_mode(cabac, ctxs, mb_x, mb_y);
        self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

        // 3. 解码 coded_block_pattern
        let (luma_cbp, chroma_cbp) = self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, true);
        self.set_mb_cbp(mb_x, mb_y, luma_cbp | (chroma_cbp << 4));

        // 4. mb_qp_delta (仅当 CBP != 0)
        let has_residual = luma_cbp != 0 || chroma_cbp != 0;
        if has_residual {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        // 5. 应用真正的预测 (根据预测模式)
        if use_8x8 {
            for block_y in 0..2 {
                for block_x in 0..2 {
                    let mode = pred_modes_8x8[block_y * 2 + block_x];
                    for sub_y in 0..2 {
                        for sub_x in 0..2 {
                            intra::predict_4x4(
                                &mut self.ref_y,
                                self.stride_y,
                                mb_x * 16 + (block_x * 2 + sub_x) * 4,
                                mb_y * 16 + (block_y * 2 + sub_y) * 4,
                                mode,
                            );
                        }
                    }
                }
            }
        }
        intra::predict_chroma_8x8(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_8x8(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 6. 解码残差并应用
        if use_8x8 {
            self.decode_i8x8_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, true);
        } else {
            self.decode_i4x4_residual(
                cabac,
                ctxs,
                luma_cbp,
                (mb_x, mb_y),
                *cur_qp,
                &pred_modes_4x4,
            );
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, true);
        }
    }

    /// 解码 I_PCM 宏块: 字节对齐后直接读取原始样本
    pub(super) fn decode_i_pcm_mb(&mut self, cabac: &mut CabacDecoder, mb_x: usize, mb_y: usize) {
        cabac.align_to_byte_boundary();
        // I_PCM 按“全部块可用”更新邻居上下文缓存, 避免后续 CABAC 上下文漂移.
        self.set_mb_cbp(mb_x, mb_y, 0x2f);
        self.set_chroma_pred_mode(mb_x, mb_y, 0);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        self.set_luma_dc_cbf(mb_x, mb_y, true);

        let x0 = mb_x * 16;
        let y0 = mb_y * 16;
        for dy in 0..16 {
            for dx in 0..16 {
                let idx = (y0 + dy) * self.stride_y + x0 + dx;
                if idx < self.ref_y.len() {
                    self.ref_y[idx] = cabac.read_raw_byte();
                } else {
                    let _ = cabac.read_raw_byte();
                }
            }
        }
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, true);
                self.set_i4x4_mode(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 2);
            }
        }
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                self.set_luma_8x8_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, true);
            }
        }

        let cx0 = mb_x * 8;
        let cy0 = mb_y * 8;
        for plane in [&mut self.ref_u, &mut self.ref_v] {
            for dy in 0..8 {
                for dx in 0..8 {
                    let idx = (cy0 + dy) * self.stride_c + cx0 + dx;
                    if idx < plane.len() {
                        plane[idx] = cabac.read_raw_byte();
                    } else {
                        let _ = cabac.read_raw_byte();
                    }
                }
            }
        }
        self.set_chroma_dc_u_cbf(mb_x, mb_y, true);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, true);
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, true);
                self.set_chroma_v_cbf(x2, y2, true);
            }
        }
        // I_PCM 后需要重启 CABAC 引擎继续解码后续宏块.
        cabac.restart_engine();
    }

    /// 解码 I_4x4 宏块的残差并应用到预测上
    pub(super) fn decode_i4x4_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_pos: (usize, usize),
        qp: i32,
        pred_modes: &[u8; 16],
    ) {
        let (mb_x, mb_y) = mb_pos;
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        // 先清空当前宏块的 luma CBF 状态.
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }

        // 亮度: 按规范顺序逐块重建, 保证后续块可引用“已重建”邻居样本.
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

                let px = mb_x * 16 + abs_sub_x * 4;
                let py = mb_y * 16 + abs_sub_y * 4;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;

                let mode = pred_modes[abs_sub_y * 4 + abs_sub_x];
                intra::predict_4x4(&mut self.ref_y, self.stride_y, px, py, mode);

                if !has_residual_8x8 {
                    self.set_luma_cbf(x4, y4, false);
                    continue;
                }

                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, true);
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
                        px,
                        py,
                        &coeffs_arr,
                    );
                } else {
                    residual::dequant_4x4_ac_with_scaling(&mut coeffs_arr, qp, &luma_scaling_4x4);
                    residual::apply_4x4_ac_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs_arr,
                    );
                }
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }

    /// I_8x8 残差规范路径: 按 8x8 块执行 CABAC 残差解码 + 8x8 反量化与反变换.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_i8x8_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        intra_defaults: bool,
    ) {
        let luma_scaling_8x8 = self.active_luma_scaling_list_8x8(intra_defaults);
        let transform_bypass = self.is_transform_bypass_active(qp);
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);

        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let x8 = mb_x * 2 + x8x8;
            let y8 = mb_y * 2 + y8x8;
            if luma_cbp & (1 << i8x8) == 0 {
                self.set_luma_8x8_cbf(x8, y8, false);
                continue;
            }
            let x4 = mb_x * 4 + x8x8 * 2;
            let y4 = mb_y * 4 + y8x8 * 2;
            let cbf_inc = self.luma_8x8_cbf_ctx_inc(x8, y8, intra_defaults);
            let raw_coeffs = decode_residual_block(cabac, ctxs, &CAT_LUMA_8X8, cbf_inc);
            let coded = raw_coeffs.iter().any(|&c| c != 0);
            self.set_luma_8x8_cbf(x8, y8, coded);
            // 对齐 FFmpeg: 8x8 变换块会把非零计数写回 2x2 子块缓存.
            // CABAC 上下文只依赖“是否非零”, 因此四个子块统一使用 8x8 的 coded 状态.
            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    self.set_luma_cbf(x4 + sub_x, y4 + sub_y, coded);
                }
            }
            if !coded {
                continue;
            }

            let mut coeffs_scan = [0i32; 64];
            for (idx, coeff) in raw_coeffs.iter().take(64).enumerate() {
                coeffs_scan[idx] = *coeff;
            }

            let px = mb_x * 16 + x8x8 * 8;
            let py = mb_y * 16 + y8x8 * 8;
            if transform_bypass {
                residual::apply_8x8_bypass_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                );
            } else {
                residual::apply_8x8_ac_residual_with_scaling(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                    qp,
                    &luma_scaling_8x8,
                );
            }
        }
    }

    /// 解码 I_16x16 宏块 (预测 + 残差)
    pub(super) fn decode_i_16x16_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        mb_type: u32,
        cur_qp: &mut i32,
    ) {
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        // I_16x16 预测模式: mb_type 后缀 2bit 直接映射到 0..3.
        const I16_PRED_MODE_MAP: [u8; 4] = [0, 1, 2, 3];
        let pred_mode = I16_PRED_MODE_MAP[((mb_type - 1) % 4) as usize];
        let cbp_chroma = ((mb_type - 1) / 4) % 3;
        let cbp_luma_nz = (mb_type - 1) >= 12;
        let cbp_luma = if cbp_luma_nz { 0x0f } else { 0x00 };
        self.set_mb_cbp(mb_x, mb_y, cbp_luma | ((cbp_chroma as u8) << 4));

        // 1. 解码 intra_chroma_pred_mode (消耗 CABAC 比特)
        let chroma_mode = self.decode_chroma_pred_mode(cabac, ctxs, mb_x, mb_y);
        self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

        // 2. mb_qp_delta (I_16x16 始终存在)
        let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
        self.prev_qp_delta_nz = qp_delta != 0;
        *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);

        // 3. 应用亮度预测
        intra::predict_16x16(
            &mut self.ref_y,
            self.stride_y,
            mb_x * 16,
            mb_y * 16,
            pred_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 4. 应用色度预测 (DC)
        intra::predict_chroma_8x8(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_8x8(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 5. 亮度残差 (DC 始终存在, AC 按 mb_type 的 CBP 决定)
        let dc_coeffs = self.decode_luma_dc_coeffs(cabac, ctxs, mb_x, mb_y, *cur_qp);
        self.decode_i16x16_luma_residual(
            cabac,
            ctxs,
            (mb_x, mb_y),
            *cur_qp,
            &dc_coeffs,
            cbp_luma_nz,
        );

        // 6. 色度残差
        if cbp_chroma >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, cbp_chroma >= 2, true);
        }
    }

    /// 解码 I_16x16 亮度 DC 残差
    pub(super) fn decode_luma_dc_coeffs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        slice_qp: i32,
    ) -> [i32; 16] {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(slice_qp);
        // 解码 DC 系数
        let cbf_inc = self.get_dc_cbf_inc(mb_x, mb_y, true);
        let raw_coeffs = decode_residual_block(cabac, ctxs, &CAT_LUMA_DC, cbf_inc);
        self.set_luma_dc_cbf(mb_x, mb_y, raw_coeffs.iter().any(|&c| c != 0));

        // 反扫描 + 反 Hadamard + 反量化
        let mut dc_block = [0i32; 16];
        for (scan_pos, &(row, col)) in residual::ZIGZAG_4X4.iter().enumerate() {
            if let Some(&c) = raw_coeffs.get(scan_pos) {
                dc_block[row * 4 + col] = c;
            }
        }
        if !transform_bypass {
            inverse_hadamard_4x4(&mut dc_block);
            residual::dequant_luma_dc_with_scaling(&mut dc_block, slice_qp, &luma_scaling_4x4);
        }
        dc_block
    }

    /// 解码并应用 I_16x16 的亮度残差
    pub(super) fn decode_i16x16_luma_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_pos: (usize, usize),
        qp: i32,
        dc_coeffs: &[i32; 16],
        has_luma_ac: bool,
    ) {
        let (mb_x, mb_y) = mb_pos;
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        // 对齐 FFmpeg scan8 的 i4x4 索引顺序:
        // i4x4=0..15 对应 8x8 分组遍历, 而非纯行优先遍历.
        const I4X4_SCAN_ORDER: [(usize, usize); 16] = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (0, 2),
            (1, 2),
            (0, 3),
            (1, 3),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
        ];

        // I_16x16 AC 的 CBF 按 4x4 子块追踪.
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        let mut coded_8x8 = [false; 4];

        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let block_idx = sub_y * 4 + sub_x;
            let mut coeffs_scan = [0i32; 16];
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            if has_luma_ac {
                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, true);
                let raw_ac = decode_residual_block(cabac, ctxs, &CAT_LUMA_AC, cbf_inc);
                let coded = raw_ac.iter().any(|&c| c != 0);
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    let idx8 = (sub_y / 2) * 2 + (sub_x / 2);
                    coded_8x8[idx8] = true;
                }
                for (scan, &c) in raw_ac.iter().enumerate().take(15) {
                    coeffs_scan[scan + 1] = c;
                }
            } else {
                self.set_luma_cbf(x4, y4, false);
            }
            coeffs_scan[0] = dc_coeffs[block_idx];

            let px = mb_x * 16 + sub_x * 4;
            let py = mb_y * 16 + sub_y * 4;
            if transform_bypass {
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                );
            } else {
                residual::dequant_4x4_ac_with_scaling(&mut coeffs_scan, qp, &luma_scaling_4x4);
                residual::apply_4x4_ac_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                );
            }
        }

        for (idx8, coded) in coded_8x8.iter().copied().enumerate() {
            let x8 = idx8 & 1;
            let y8 = idx8 >> 1;
            self.set_luma_8x8_cbf(mb_x * 2 + x8, mb_y * 2 + y8, coded);
        }
    }

    /// 解码并应用色度残差
    pub(super) fn decode_chroma_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_pos: (usize, usize),
        slice_qp: i32,
        has_chroma_ac: bool,
        intra_defaults: bool,
    ) {
        let (mb_x, mb_y) = mb_pos;
        let u_scaling_4x4 = self.active_chroma_scaling_list_4x4(intra_defaults, false);
        let v_scaling_4x4 = self.active_chroma_scaling_list_4x4(intra_defaults, true);
        let transform_bypass = self.is_transform_bypass_active(slice_qp);
        // 色度 QP 映射(按 PPS 中的 Cb/Cr 偏移分别计算).
        let (chroma_off_u, chroma_off_v) = self
            .pps
            .as_ref()
            .map(|p| (p.chroma_qp_index_offset, p.second_chroma_qp_index_offset))
            .unwrap_or((0, 0));
        let chroma_qp_u = chroma_qp_from_luma_with_offset(slice_qp, chroma_off_u);
        let chroma_qp_v = chroma_qp_from_luma_with_offset(slice_qp, chroma_off_v);

        // U 通道
        let chroma_dc_cbf_inc_u = self.chroma_dc_cbf_ctx_inc(mb_x, mb_y, intra_defaults);
        let u_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, chroma_dc_cbf_inc_u);
        self.set_chroma_dc_u_cbf(mb_x, mb_y, u_coeffs.iter().any(|&c| c != 0));
        let mut u_dc = [0i32; 4];
        for (i, &c) in u_coeffs.iter().enumerate().take(4) {
            u_dc[i] = c;
        }
        if !transform_bypass {
            inverse_hadamard_2x2(&mut u_dc);
            residual::dequant_chroma_dc_with_scaling(&mut u_dc, chroma_qp_u, &u_scaling_4x4);
        }

        // V 通道
        let chroma_dc_cbf_inc_v = self.chroma_dc_v_cbf_ctx_inc(mb_x, mb_y, intra_defaults);
        let v_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, chroma_dc_cbf_inc_v);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, v_coeffs.iter().any(|&c| c != 0));
        let mut v_dc = [0i32; 4];
        for (i, &c) in v_coeffs.iter().enumerate().take(4) {
            v_dc[i] = c;
        }
        if !transform_bypass {
            inverse_hadamard_2x2(&mut v_dc);
            residual::dequant_chroma_dc_with_scaling(&mut v_dc, chroma_qp_v, &v_scaling_4x4);
        }

        // H.264 语法顺序: 先完整解码 U 的 4 个 AC 块, 再完整解码 V 的 4 个 AC 块.
        let mut u_scans = [[0i32; 16]; 4];
        let mut v_scans = [[0i32; 16]; 4];

        if has_chroma_ac {
            for (block_idx, u_scan) in u_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;

                let cbf_inc_u = self.chroma_u_cbf_ctx_inc(x2, y2, intra_defaults);
                let raw_u_ac = decode_residual_block(cabac, ctxs, &CAT_CHROMA_AC, cbf_inc_u);
                let coded_u = raw_u_ac.iter().any(|&c| c != 0);
                self.set_chroma_u_cbf(x2, y2, coded_u);
                for (scan, &c) in raw_u_ac.iter().enumerate().take(15) {
                    u_scan[scan + 1] = c;
                }
            }
            for (block_idx, v_scan) in v_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;

                let cbf_inc_v = self.chroma_v_cbf_ctx_inc(x2, y2, intra_defaults);
                let raw_v_ac = decode_residual_block(cabac, ctxs, &CAT_CHROMA_AC, cbf_inc_v);
                let coded_v = raw_v_ac.iter().any(|&c| c != 0);
                self.set_chroma_v_cbf(x2, y2, coded_v);
                for (scan, &c) in raw_v_ac.iter().enumerate().take(15) {
                    v_scan[scan + 1] = c;
                }
            }
        } else {
            for block_idx in 0..4usize {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, false);
                self.set_chroma_v_cbf(x2, y2, false);
            }
        }

        // 应用到色度平面: 每个 4x4 子块独立重建 (DC + AC)
        for block_idx in 0..4usize {
            let sub_x = block_idx & 1;
            let sub_y = block_idx >> 1;
            let px = mb_x * 8 + sub_x * 4;
            let py = mb_y * 8 + sub_y * 4;

            let mut u_scan = u_scans[block_idx];
            u_scan[0] = u_dc[block_idx];
            if transform_bypass {
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_u,
                    self.stride_c,
                    px,
                    py,
                    &u_scan,
                );
            } else {
                residual::dequant_4x4_ac_with_scaling(&mut u_scan, chroma_qp_u, &u_scaling_4x4);
                residual::apply_4x4_ac_residual(&mut self.ref_u, self.stride_c, px, py, &u_scan);
            }

            let mut v_scan = v_scans[block_idx];
            v_scan[0] = v_dc[block_idx];
            if transform_bypass {
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_v,
                    self.stride_c,
                    px,
                    py,
                    &v_scan,
                );
            } else {
                residual::dequant_4x4_ac_with_scaling(&mut v_scan, chroma_qp_v, &v_scaling_4x4);
                residual::apply_4x4_ac_residual(&mut self.ref_v, self.stride_c, px, py, &v_scan);
            }
        }
    }

    /// 获取 DC coded_block_flag 的上下文增量
    pub(super) fn get_dc_cbf_inc(&self, mb_x: usize, mb_y: usize, intra_defaults: bool) -> usize {
        let left = if mb_x > 0 {
            usize::from(self.get_luma_dc_cbf(mb_x - 1, mb_y))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if mb_y > 0 {
            usize::from(self.get_luma_dc_cbf(mb_x, mb_y - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }
}
