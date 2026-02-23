use super::*;

impl H264Decoder {
    fn left_neighbor_available_units(
        &self,
        x_units: usize,
        y_units: usize,
        units_per_mb: usize,
    ) -> bool {
        if x_units == 0 || units_per_mb == 0 {
            return false;
        }
        if x_units % units_per_mb != 0 {
            return true;
        }
        let mb_x = x_units / units_per_mb;
        let mb_y = y_units / units_per_mb;
        self.left_avail(mb_x, mb_y)
    }

    fn top_neighbor_available_units(
        &self,
        x_units: usize,
        y_units: usize,
        units_per_mb: usize,
    ) -> bool {
        if y_units == 0 || units_per_mb == 0 {
            return false;
        }
        if y_units % units_per_mb != 0 {
            return true;
        }
        let mb_x = x_units / units_per_mb;
        let mb_y = y_units / units_per_mb;
        self.top_avail(mb_x, mb_y)
    }

    fn left_neighbor_available_4x4(&self, x4: usize, y4: usize) -> bool {
        self.left_neighbor_available_units(x4, y4, 4)
    }

    fn top_neighbor_available_4x4(&self, x4: usize, y4: usize) -> bool {
        self.top_neighbor_available_units(x4, y4, 4)
    }

    fn left_neighbor_available_8x8(&self, x8: usize, y8: usize) -> bool {
        self.left_neighbor_available_units(x8, y8, 2)
    }

    fn top_neighbor_available_8x8(&self, x8: usize, y8: usize) -> bool {
        self.top_neighbor_available_units(x8, y8, 2)
    }

    fn left_neighbor_available_chroma2x2(&self, x2: usize, y2: usize) -> bool {
        self.left_neighbor_available_units(x2, y2, 2)
    }

    fn top_neighbor_available_chroma2x2(&self, x2: usize, y2: usize) -> bool {
        self.top_neighbor_available_units(x2, y2, 2)
    }

    pub(super) fn cbf_stride(&self) -> usize {
        self.mb_width * 4
    }

    pub(super) fn cbf_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.cbf_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    pub(super) fn get_luma_cbf(&self, x4: usize, y4: usize) -> bool {
        self.cbf_index(x4, y4)
            .and_then(|idx| self.cbf_luma.get(idx).copied())
            .unwrap_or(false)
    }

    pub(super) fn set_luma_cbf(&mut self, x4: usize, y4: usize, coded: bool) {
        if let Some(idx) = self.cbf_index(x4, y4)
            && let Some(slot) = self.cbf_luma.get_mut(idx)
        {
            *slot = coded;
        }
    }

    pub(super) fn luma_cbf_ctx_inc(&self, x4: usize, y4: usize, intra_defaults: bool) -> usize {
        let left = if self.left_neighbor_available_4x4(x4, y4) {
            usize::from(self.get_luma_cbf(x4 - 1, y4))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_neighbor_available_4x4(x4, y4) {
            usize::from(self.get_luma_cbf(x4, y4 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn luma_8x8_cbf_stride(&self) -> usize {
        self.mb_width * 2
    }

    pub(super) fn luma_8x8_cbf_index(&self, x8: usize, y8: usize) -> Option<usize> {
        let stride = self.luma_8x8_cbf_stride();
        if stride == 0 {
            return None;
        }
        let h8 = self.mb_height * 2;
        if x8 >= stride || y8 >= h8 {
            return None;
        }
        Some(y8 * stride + x8)
    }

    pub(super) fn set_luma_8x8_cbf(&mut self, x8: usize, y8: usize, coded: bool) {
        if let Some(idx) = self.luma_8x8_cbf_index(x8, y8)
            && let Some(slot) = self.cbf_luma_8x8.get_mut(idx)
        {
            *slot = coded;
        }
    }

    pub(super) fn get_luma_8x8_cbf(&self, x8: usize, y8: usize) -> bool {
        self.luma_8x8_cbf_index(x8, y8)
            .and_then(|idx| self.cbf_luma_8x8.get(idx).copied())
            .unwrap_or(false)
    }

    pub(super) fn luma_8x8_cbf_ctx_inc(&self, x8: usize, y8: usize, intra_defaults: bool) -> usize {
        // 8x8 变换块 CBF 采用 8x8 邻居非零状态作为上下文增量.
        let left = if self.left_neighbor_available_8x8(x8, y8) {
            usize::from(self.get_luma_8x8_cbf(x8 - 1, y8))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_neighbor_available_8x8(x8, y8) {
            usize::from(self.get_luma_8x8_cbf(x8, y8 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn chroma_cbf_stride(&self) -> usize {
        self.mb_width * 2
    }

    pub(super) fn chroma_cbf_index(&self, x2: usize, y2: usize) -> Option<usize> {
        let stride = self.chroma_cbf_stride();
        if stride == 0 {
            return None;
        }
        let h2 = self.mb_height * 2;
        if x2 >= stride || y2 >= h2 {
            return None;
        }
        Some(y2 * stride + x2)
    }

    pub(super) fn get_chroma_u_cbf(&self, x2: usize, y2: usize) -> bool {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.cbf_chroma_u.get(idx).copied())
            .unwrap_or(false)
    }

    pub(super) fn set_chroma_u_cbf(&mut self, x2: usize, y2: usize, coded: bool) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.cbf_chroma_u.get_mut(idx)
        {
            *slot = coded;
        }
    }

    pub(super) fn get_chroma_v_cbf(&self, x2: usize, y2: usize) -> bool {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.cbf_chroma_v.get(idx).copied())
            .unwrap_or(false)
    }

    pub(super) fn set_chroma_v_cbf(&mut self, x2: usize, y2: usize, coded: bool) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.cbf_chroma_v.get_mut(idx)
        {
            *slot = coded;
        }
    }

    pub(super) fn chroma_u_cbf_ctx_inc(&self, x2: usize, y2: usize, intra_defaults: bool) -> usize {
        let left = if self.left_neighbor_available_chroma2x2(x2, y2) {
            usize::from(self.get_chroma_u_cbf(x2 - 1, y2))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_neighbor_available_chroma2x2(x2, y2) {
            usize::from(self.get_chroma_u_cbf(x2, y2 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn chroma_v_cbf_ctx_inc(&self, x2: usize, y2: usize, intra_defaults: bool) -> usize {
        let left = if self.left_neighbor_available_chroma2x2(x2, y2) {
            usize::from(self.get_chroma_v_cbf(x2 - 1, y2))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_neighbor_available_chroma2x2(x2, y2) {
            usize::from(self.get_chroma_v_cbf(x2, y2 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn chroma_dc_cbf_ctx_inc(
        &self,
        mb_x: usize,
        mb_y: usize,
        intra_defaults: bool,
    ) -> usize {
        let left = if self.left_avail(mb_x, mb_y) {
            usize::from(
                self.mb_index(mb_x - 1, mb_y)
                    .and_then(|idx| self.cbf_chroma_dc_u.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_avail(mb_x, mb_y) {
            usize::from(
                self.mb_index(mb_x, mb_y - 1)
                    .and_then(|idx| self.cbf_chroma_dc_u.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn set_chroma_dc_u_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_chroma_dc_u.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 6;
                } else {
                    *slot &= !(1u16 << 6);
                }
            }
        }
    }

    pub(super) fn set_luma_dc_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_luma_dc.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 8;
                } else {
                    *slot &= !(1u16 << 8);
                }
            }
        }
    }

    pub(super) fn get_luma_dc_cbf(&self, mb_x: usize, mb_y: usize) -> bool {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.cbf_luma_dc.get(idx).copied())
            .unwrap_or(false)
    }

    pub(super) fn chroma_dc_v_cbf_ctx_inc(
        &self,
        mb_x: usize,
        mb_y: usize,
        intra_defaults: bool,
    ) -> usize {
        let left = if self.left_avail(mb_x, mb_y) {
            usize::from(
                self.mb_index(mb_x - 1, mb_y)
                    .and_then(|idx| self.cbf_chroma_dc_v.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if self.top_avail(mb_x, mb_y) {
            usize::from(
                self.mb_index(mb_x, mb_y - 1)
                    .and_then(|idx| self.cbf_chroma_dc_v.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    pub(super) fn set_chroma_dc_v_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_chroma_dc_v.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 7;
                } else {
                    *slot &= !(1u16 << 7);
                }
            }
        }
    }

    pub(super) fn reset_chroma_cbf_mb(&mut self, mb_x: usize, mb_y: usize) {
        self.set_chroma_dc_u_cbf(mb_x, mb_y, false);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, false);
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, false);
                self.set_chroma_v_cbf(x2, y2, false);
            }
        }
    }

    pub(super) fn reset_luma_8x8_cbf_mb(&mut self, mb_x: usize, mb_y: usize) {
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                self.set_luma_8x8_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, false);
            }
        }
    }

    pub(super) fn motion_l0_4x4_stride(&self) -> usize {
        self.mb_width * 4
    }

    pub(super) fn motion_l0_4x4_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.motion_l0_4x4_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    pub(super) fn set_l0_motion_4x4(
        &mut self,
        x4: usize,
        y4: usize,
        mv_x: i16,
        mv_y: i16,
        ref_idx: i8,
    ) {
        if let Some(idx) = self.motion_l0_4x4_index(x4, y4) {
            if let Some(slot) = self.mv_l0_x_4x4.get_mut(idx) {
                *slot = mv_x;
            }
            if let Some(slot) = self.mv_l0_y_4x4.get_mut(idx) {
                *slot = mv_y;
            }
            if let Some(slot) = self.ref_idx_l0_4x4.get_mut(idx) {
                *slot = ref_idx;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_l0_motion_block_4x4(
        &mut self,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        ref_idx: i8,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let x4_start = dst_x / 4;
        let y4_start = dst_y / 4;
        let x4_end = (dst_x + w).div_ceil(4);
        let y4_end = (dst_y + h).div_ceil(4);
        let mv_x = mv_x_qpel.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let mv_y = mv_y_qpel.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        for y4 in y4_start..y4_end {
            for x4 in x4_start..x4_end {
                self.set_l0_motion_4x4(x4, y4, mv_x, mv_y, ref_idx);
            }
        }
    }

    pub(super) fn motion_l1_4x4_stride(&self) -> usize {
        self.mb_width * 4
    }

    pub(super) fn direct_4x4_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.mb_width * 4;
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    pub(super) fn get_direct_4x4_flag(&self, x4: usize, y4: usize) -> bool {
        self.direct_4x4_index(x4, y4)
            .and_then(|idx| self.direct_4x4_flags.get(idx).copied())
            .unwrap_or(0)
            != 0
    }

    pub(super) fn set_direct_4x4_flag(&mut self, x4: usize, y4: usize, is_direct: bool) {
        if let Some(idx) = self.direct_4x4_index(x4, y4)
            && let Some(slot) = self.direct_4x4_flags.get_mut(idx)
        {
            *slot = u8::from(is_direct);
        }
    }

    pub(super) fn set_direct_block_4x4(
        &mut self,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        is_direct: bool,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let x4_start = dst_x / 4;
        let y4_start = dst_y / 4;
        let x4_end = (dst_x + w).div_ceil(4);
        let y4_end = (dst_y + h).div_ceil(4);
        for y4 in y4_start..y4_end {
            for x4 in x4_start..x4_end {
                self.set_direct_4x4_flag(x4, y4, is_direct);
            }
        }
    }

    pub(super) fn motion_l1_4x4_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.motion_l1_4x4_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    pub(super) fn set_l1_motion_4x4(
        &mut self,
        x4: usize,
        y4: usize,
        mv_x: i16,
        mv_y: i16,
        ref_idx: i8,
    ) {
        if let Some(idx) = self.motion_l1_4x4_index(x4, y4) {
            if let Some(slot) = self.mv_l1_x_4x4.get_mut(idx) {
                *slot = mv_x;
            }
            if let Some(slot) = self.mv_l1_y_4x4.get_mut(idx) {
                *slot = mv_y;
            }
            if let Some(slot) = self.ref_idx_l1_4x4.get_mut(idx) {
                *slot = ref_idx;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_l1_motion_block_4x4(
        &mut self,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        ref_idx: i8,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let x4_start = dst_x / 4;
        let y4_start = dst_y / 4;
        let x4_end = (dst_x + w).div_ceil(4);
        let y4_end = (dst_y + h).div_ceil(4);
        let mv_x = mv_x_qpel.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let mv_y = mv_y_qpel.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        for y4 in y4_start..y4_end {
            for x4 in x4_start..x4_end {
                self.set_l1_motion_4x4(x4, y4, mv_x, mv_y, ref_idx);
            }
        }
    }

    /// 计算 CABAC MVD 上下文: 左/上 4x4 邻居 MVD 绝对值之和.
    pub(super) fn compute_cabac_amvd(&self, x4: usize, y4: usize, list: usize) -> (i32, i32) {
        if std::env::var("TAO_H264_DEBUG_FORCE_AMVD_ZERO").as_deref() == Ok("1") {
            return (0, 0);
        }
        let stride = self.mb_width * 4;
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return (0, 0);
        }
        let (mvd_x_arr, mvd_y_arr) = if list == 0 {
            (&self.mvd_l0_x_4x4, &self.mvd_l0_y_4x4)
        } else {
            (&self.mvd_l1_x_4x4, &self.mvd_l1_y_4x4)
        };
        let left_abs_x = if self.left_neighbor_available_4x4(x4, y4) {
            mvd_x_arr
                .get(y4 * stride + x4 - 1)
                .copied()
                .unwrap_or(0)
                .unsigned_abs() as i32
        } else {
            0
        };
        let left_abs_y = if self.left_neighbor_available_4x4(x4, y4) {
            mvd_y_arr
                .get(y4 * stride + x4 - 1)
                .copied()
                .unwrap_or(0)
                .unsigned_abs() as i32
        } else {
            0
        };
        let top_abs_x = if self.top_neighbor_available_4x4(x4, y4) {
            mvd_x_arr
                .get((y4 - 1) * stride + x4)
                .copied()
                .unwrap_or(0)
                .unsigned_abs() as i32
        } else {
            0
        };
        let top_abs_y = if self.top_neighbor_available_4x4(x4, y4) {
            mvd_y_arr
                .get((y4 - 1) * stride + x4)
                .copied()
                .unwrap_or(0)
                .unsigned_abs() as i32
        } else {
            0
        };
        (left_abs_x + top_abs_x, left_abs_y + top_abs_y)
    }

    /// 将解码后的 MVD 写入 4x4 块级缓存.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_mvd_block_4x4(
        &mut self,
        px_x: usize,
        px_y: usize,
        w: usize,
        h: usize,
        mvd_x: i32,
        mvd_y: i32,
        list: usize,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let x4_start = px_x / 4;
        let y4_start = px_y / 4;
        let x4_end = (px_x + w).div_ceil(4);
        let y4_end = (px_y + h).div_ceil(4);
        let stride = self.mb_width * 4;
        // 与 FFmpeg `decode_cabac_mb_mvd` 对齐: mvd_cache 仅保留 [-70, 70] 截断值用于后续 amvd 上下文.
        let mvd_x_i16 = mvd_x.clamp(-70, 70) as i16;
        let mvd_y_i16 = mvd_y.clamp(-70, 70) as i16;
        let (mvd_x_arr, mvd_y_arr) = if list == 0 {
            (&mut self.mvd_l0_x_4x4, &mut self.mvd_l0_y_4x4)
        } else {
            (&mut self.mvd_l1_x_4x4, &mut self.mvd_l1_y_4x4)
        };
        for y4 in y4_start..y4_end {
            for x4 in x4_start..x4_end {
                let idx = y4 * stride + x4;
                if let Some(slot) = mvd_x_arr.get_mut(idx) {
                    *slot = mvd_x_i16;
                }
                if let Some(slot) = mvd_y_arr.get_mut(idx) {
                    *slot = mvd_y_i16;
                }
            }
        }
    }

    /// 清空一个宏块对应的 MVD 4x4 缓存.
    ///
    /// 作用:
    /// - 避免 skip/intra 宏块沿用旧帧残留 MVD, 污染后续 CABAC `amvd` 上下文.
    pub(super) fn clear_mb_mvd_cache(&mut self, mb_x: usize, mb_y: usize) {
        let px_x = mb_x * 16;
        let px_y = mb_y * 16;
        self.set_mvd_block_4x4(px_x, px_y, 16, 16, 0, 0, 0);
        self.set_mvd_block_4x4(px_x, px_y, 16, 16, 0, 0, 1);
    }

    /// 清空一个宏块对应的运动向量/参考索引缓存.
    ///
    /// 作用:
    /// - Intra 宏块不应携带历史帧的 inter 运动信息.
    /// - 避免后续 `ref_idx`/MV 上下文读取到过期缓存.
    pub(super) fn clear_mb_motion_cache(&mut self, mb_x: usize, mb_y: usize) {
        let px_x = mb_x * 16;
        let px_y = mb_y * 16;
        self.set_l0_motion_block_4x4(px_x, px_y, 16, 16, 0, 0, -1);
        self.set_l1_motion_block_4x4(px_x, px_y, 16, 16, 0, 0, -1);
        self.set_direct_block_4x4(px_x, px_y, 16, 16, false);
        if let Some(mb_idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.mv_l0_x.get_mut(mb_idx) {
                *slot = 0;
            }
            if let Some(slot) = self.mv_l0_y.get_mut(mb_idx) {
                *slot = 0;
            }
            if let Some(slot) = self.ref_idx_l0.get_mut(mb_idx) {
                *slot = -1;
            }
            if let Some(slot) = self.mv_l1_x.get_mut(mb_idx) {
                *slot = 0;
            }
            if let Some(slot) = self.mv_l1_y.get_mut(mb_idx) {
                *slot = 0;
            }
            if let Some(slot) = self.ref_idx_l1.get_mut(mb_idx) {
                *slot = -1;
            }
        }
    }

    pub(super) fn i4x4_mode_stride(&self) -> usize {
        self.mb_width * 4
    }

    pub(super) fn i4x4_mode_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.i4x4_mode_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    pub(super) fn get_i4x4_mode(&self, x4: usize, y4: usize) -> u8 {
        self.i4x4_mode_index(x4, y4)
            .and_then(|idx| self.i4x4_modes.get(idx).copied())
            .unwrap_or(2)
    }

    pub(super) fn set_i4x4_mode(&mut self, x4: usize, y4: usize, mode: u8) {
        if let Some(idx) = self.i4x4_mode_index(x4, y4)
            && let Some(slot) = self.i4x4_modes.get_mut(idx)
        {
            // 对齐 FFmpeg 内部缓存语义: 允许 9/10/11 三个 DC 派生模式进入邻居预测缓存.
            *slot = mode.min(11);
        }
    }

    pub(super) fn mb_index(&self, mb_x: usize, mb_y: usize) -> Option<usize> {
        if mb_x >= self.mb_width || mb_y >= self.mb_height {
            return None;
        }
        Some(mb_y * self.mb_width + mb_x)
    }

    pub(super) fn get_mb_cbp(&self, mb_x: usize, mb_y: usize) -> u8 {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.mb_cbp.get(idx).copied())
            .unwrap_or(0)
    }

    pub(super) fn set_mb_cbp(&mut self, mb_x: usize, mb_y: usize, cbp: u8) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.mb_cbp.get_mut(idx) {
                *slot = cbp;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                *slot = (*slot & !0x003F) | u16::from(cbp & 0x3F);
            }
        }
    }

    pub(super) fn get_chroma_pred_mode(&self, mb_x: usize, mb_y: usize) -> u8 {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.chroma_pred_modes.get(idx).copied())
            .unwrap_or(0)
    }

    pub(super) fn set_chroma_pred_mode(&mut self, mb_x: usize, mb_y: usize, mode: u8) {
        if let Some(idx) = self.mb_index(mb_x, mb_y)
            && let Some(slot) = self.chroma_pred_modes.get_mut(idx)
        {
            *slot = mode.min(3);
        }
    }

    pub(super) fn set_transform_8x8_flag(&mut self, mb_x: usize, mb_y: usize, flag: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y)
            && let Some(slot) = self.transform_8x8_flags.get_mut(idx)
        {
            *slot = u8::from(flag);
        }
    }

    pub(super) fn get_transform_8x8_flag(&self, mb_x: usize, mb_y: usize) -> bool {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.transform_8x8_flags.get(idx).copied())
            .unwrap_or(0)
            != 0
    }

    /// 解码 transform_size_8x8_flag.
    pub(super) fn decode_transform_size_8x8_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left = self.left_avail(mb_x, mb_y) && self.get_transform_8x8_flag(mb_x - 1, mb_y);
        let top = self.top_avail(mb_x, mb_y) && self.get_transform_8x8_flag(mb_x, mb_y - 1);
        let idx = 399usize + usize::from(left) + usize::from(top);
        if idx < ctxs.len() {
            cabac.decode_decision(&mut ctxs[idx]) == 1
        } else {
            cabac.decode_decision(&mut ctxs[68]) == 1
        }
    }

    /// 按 FFmpeg inter 语义解码 transform_size_8x8_flag.
    ///
    /// 邻居上下文:
    /// - unavailable -> 0
    /// - Direct 邻居 -> 0
    /// - 邻居已使用 8x8 变换 -> 0
    /// - 其它 -> 1
    pub(super) fn decode_transform_size_8x8_flag_inter(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let mb_idx = mb_y * self.mb_width + mb_x;
        let trace_mb_detail =
            self.trace_mb_detail_enabled() && self.should_trace_mb_idx(mb_idx, usize::MAX);
        let left_ctx = if self.left_avail(mb_x, mb_y) {
            if let Some(left_mb_idx) = self.mb_index(mb_x - 1, mb_y) {
                let left_is_direct =
                    self.mb_types.get(left_mb_idx).copied().unwrap_or_default() == 254;
                let left_is_8x8dct = self
                    .transform_8x8_flags
                    .get(left_mb_idx)
                    .copied()
                    .unwrap_or(0)
                    != 0;
                usize::from(!left_is_direct && !left_is_8x8dct)
            } else {
                0
            }
        } else {
            0
        };
        let top_ctx = if self.top_avail(mb_x, mb_y) {
            if let Some(top_mb_idx) = self.mb_index(mb_x, mb_y - 1) {
                let top_is_direct =
                    self.mb_types.get(top_mb_idx).copied().unwrap_or_default() == 254;
                let top_is_8x8dct = self
                    .transform_8x8_flags
                    .get(top_mb_idx)
                    .copied()
                    .unwrap_or(0)
                    != 0;
                usize::from(!top_is_direct && !top_is_8x8dct)
            } else {
                0
            }
        } else {
            0
        };
        let idx = 399usize + left_ctx + top_ctx;
        let bits_before = if trace_mb_detail {
            cabac.bits_read()
        } else {
            0
        };
        if idx < ctxs.len() {
            let bin = cabac.decode_decision(&mut ctxs[idx]) == 1;
            if trace_mb_detail {
                let bits_after = cabac.bits_read();
                eprintln!(
                    "[H264_T8X8_CTX] idx={} mb=({}, {}) left_ctx={} top_ctx={} ctx_idx={} bin={} bits_before={} bits_after={} delta={}",
                    mb_idx,
                    mb_x,
                    mb_y,
                    left_ctx,
                    top_ctx,
                    idx,
                    u8::from(bin),
                    bits_before,
                    bits_after,
                    bits_after.saturating_sub(bits_before)
                );
            }
            bin
        } else {
            let bin = cabac.decode_decision(&mut ctxs[68]) == 1;
            if trace_mb_detail {
                let bits_after = cabac.bits_read();
                eprintln!(
                    "[H264_T8X8_CTX] idx={} mb=({}, {}) left_ctx={} top_ctx={} ctx_idx=fallback68 bin={} bits_before={} bits_after={} delta={}",
                    mb_idx,
                    mb_x,
                    mb_y,
                    left_ctx,
                    top_ctx,
                    u8::from(bin),
                    bits_before,
                    bits_after,
                    bits_after.saturating_sub(bits_before)
                );
            }
            bin
        }
    }

    pub(super) fn decode_chroma_pred_mode(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> u8 {
        let mut ctx = 0usize;
        if self.left_avail(mb_x, mb_y) && self.get_chroma_pred_mode(mb_x - 1, mb_y) != 0 {
            ctx += 1;
        }
        if self.top_avail(mb_x, mb_y) && self.get_chroma_pred_mode(mb_x, mb_y - 1) != 0 {
            ctx += 1;
        }
        if cabac.decode_decision(&mut ctxs[64 + ctx]) == 0 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[67]) == 0 {
            return 1;
        }
        if cabac.decode_decision(&mut ctxs[67]) == 0 {
            2
        } else {
            3
        }
    }

    pub(super) fn decode_coded_block_pattern(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        intra_defaults: bool,
    ) -> (u8, u8) {
        let unavailable_cbp = if intra_defaults { 0xcf } else { 0x0f };
        let cbp_a = if self.left_avail(mb_x, mb_y) {
            self.get_mb_cbp(mb_x - 1, mb_y)
        } else {
            unavailable_cbp
        };
        let cbp_b = if self.top_avail(mb_x, mb_y) {
            self.get_mb_cbp(mb_x, mb_y - 1)
        } else {
            unavailable_cbp
        };

        let mut luma_cbp = 0u8;
        let mut ctx = usize::from((cbp_a & 0x02) == 0) + (usize::from((cbp_b & 0x04) == 0) << 1);
        let bit0 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit0;

        ctx = usize::from((luma_cbp & 0x01) == 0) + (usize::from((cbp_b & 0x08) == 0) << 1);
        let bit1 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit1 << 1;

        ctx = usize::from((cbp_a & 0x08) == 0) + (usize::from((luma_cbp & 0x01) == 0) << 1);
        let bit2 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit2 << 2;

        ctx = usize::from((luma_cbp & 0x04) == 0) + (usize::from((luma_cbp & 0x02) == 0) << 1);
        let bit3 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit3 << 3;

        let cbp_a_chroma = (cbp_a >> 4) & 0x03;
        let cbp_b_chroma = (cbp_b >> 4) & 0x03;
        let mut c_ctx = 0usize;
        if cbp_a_chroma > 0 {
            c_ctx += 1;
        }
        if cbp_b_chroma > 0 {
            c_ctx += 2;
        }
        if cabac.decode_decision(&mut ctxs[77 + c_ctx]) == 0 {
            return (luma_cbp, 0);
        }

        let mut c_ctx2 = 4usize;
        if cbp_a_chroma == 2 {
            c_ctx2 += 1;
        }
        if cbp_b_chroma == 2 {
            c_ctx2 += 2;
        }
        let chroma_cbp = 1u8 + cabac.decode_decision(&mut ctxs[77 + c_ctx2]) as u8;
        (luma_cbp, chroma_cbp)
    }
}
