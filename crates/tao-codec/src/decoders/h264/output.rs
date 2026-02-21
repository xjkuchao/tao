use super::*;

// ============================================================
// 输出帧构建
// ============================================================

impl H264Decoder {
    pub(super) fn record_missing_reference_fallback(
        &mut self,
        scene: &str,
        ref_idx: i32,
        list_len: usize,
    ) {
        self.missing_reference_fallbacks = self.missing_reference_fallbacks.saturating_add(1);
        if self.missing_reference_fallbacks <= 8 {
            warn!(
                "H264: 缺失参考帧, scene={}, ref_idx={}, list_len={}, 使用零参考回退",
                scene, ref_idx, list_len
            );
        } else if self.missing_reference_fallbacks == 9 {
            warn!("H264: 缺失参考帧回退日志过多, 后续同类日志省略");
        }
    }

    pub(super) fn zero_reference_planes(&self) -> RefPlanes {
        RefPlanes {
            y: vec![128u8; self.ref_y.len()],
            u: vec![128u8; self.ref_u.len()],
            v: vec![128u8; self.ref_v.len()],
            poc: self.last_poc,
            is_long_term: false,
        }
    }

    pub(super) fn max_frame_num_modulo(&self) -> u32 {
        let shift = self
            .sps
            .as_ref()
            .map(|s| s.log2_max_frame_num)
            .unwrap_or(4)
            .min(31);
        1u32 << shift
    }

    pub(super) fn frame_num_backward_distance(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        let cur = self.last_frame_num % max;
        let target = frame_num % max;
        let dist = (cur + max - target) % max;
        if dist == 0 { max } else { dist }
    }

    pub(super) fn frame_num_forward_distance(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        let cur = self.last_frame_num % max;
        let target = frame_num % max;
        let dist = (target + max - cur) % max;
        if dist == 0 { max } else { dist }
    }

    pub(super) fn pic_num_subtract(&self, pic_num: u32, sub: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        (pic_num + max - (sub % max)) % max
    }

    pub(super) fn pic_num_from_frame_num(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        frame_num % max
    }

    pub(super) fn short_term_references(&self) -> Vec<&ReferencePicture> {
        self.reference_frames
            .iter()
            .filter(|pic| pic.long_term_frame_idx.is_none())
            .collect()
    }

    pub(super) fn long_term_references(&self) -> Vec<&ReferencePicture> {
        self.reference_frames
            .iter()
            .filter(|pic| pic.long_term_frame_idx.is_some())
            .collect()
    }

    pub(super) fn reference_to_planes(pic: &ReferencePicture) -> RefPlanes {
        RefPlanes {
            y: pic.y.clone(),
            u: pic.u.clone(),
            v: pic.v.clone(),
            poc: pic.poc,
            is_long_term: pic.long_term_frame_idx.is_some(),
        }
    }

    pub(super) fn collect_default_reference_list_l0(&self) -> Vec<&ReferencePicture> {
        if self.last_slice_type == 1 {
            let cur_poc = self.last_poc;
            let short_refs = self.short_term_references();
            let mut before: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc < cur_poc)
                .collect();
            let mut after: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc >= cur_poc)
                .collect();
            before.sort_by_key(|pic| std::cmp::Reverse(pic.poc));
            after.sort_by_key(|pic| pic.poc);
            let mut refs = before;
            refs.extend(after);
            let mut long_refs = self.long_term_references();
            long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
            refs.extend(long_refs);
            return refs;
        }

        let mut short_refs = self.short_term_references();
        short_refs.sort_by_key(|pic| {
            (
                self.frame_num_backward_distance(pic.frame_num),
                self.frame_num_forward_distance(pic.frame_num),
            )
        });
        let mut refs = short_refs;
        let mut long_refs = self.long_term_references();
        long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
        refs.extend(long_refs);
        refs
    }

    pub(super) fn collect_default_reference_list_l1(&self) -> Vec<&ReferencePicture> {
        if self.last_slice_type == 1 {
            let cur_poc = self.last_poc;
            let short_refs = self.short_term_references();
            let mut after: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc > cur_poc)
                .collect();
            let mut before: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc <= cur_poc)
                .collect();
            after.sort_by_key(|pic| pic.poc);
            before.sort_by_key(|pic| std::cmp::Reverse(pic.poc));
            let mut refs = after;
            refs.extend(before);
            let mut long_refs = self.long_term_references();
            long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
            refs.extend(long_refs);
            return refs;
        }

        let mut short_refs = self.short_term_references();
        short_refs.sort_by_key(|pic| {
            (
                self.frame_num_forward_distance(pic.frame_num),
                self.frame_num_backward_distance(pic.frame_num),
            )
        });
        let mut refs = short_refs;
        let mut long_refs = self.long_term_references();
        long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
        refs.extend(long_refs);
        refs
    }

    pub(super) fn short_term_pic_num_from_ref(
        &self,
        ref_frame_num: u32,
        cur_pic_num: i32,
        max_frame_num: i32,
    ) -> i32 {
        let frame_num = (ref_frame_num % (max_frame_num as u32)) as i32;
        if frame_num > cur_pic_num {
            frame_num - max_frame_num
        } else {
            frame_num
        }
    }

    pub(super) fn find_short_term_ref_index_by_pic_num(
        &self,
        refs: &[&ReferencePicture],
        pic_num: i32,
        cur_pic_num: i32,
        max_frame_num: i32,
    ) -> Option<usize> {
        refs.iter().position(|pic| {
            pic.long_term_frame_idx.is_none()
                && self.short_term_pic_num_from_ref(pic.frame_num, cur_pic_num, max_frame_num)
                    == pic_num
        })
    }

    pub(super) fn apply_ref_pic_list_modifications(
        &self,
        refs: &mut Vec<&ReferencePicture>,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) {
        if mods.is_empty() || refs.is_empty() {
            return;
        }
        let max_frame_num = self.max_frame_num_modulo() as i32;
        if max_frame_num <= 0 {
            return;
        }

        let cur_pic_num = cur_frame_num as i32;
        let mut pic_num_pred = cur_pic_num;
        let mut insert_idx = 0usize;

        for &m in mods {
            let target_idx = match m {
                RefPicListMod::ShortTermSub {
                    abs_diff_pic_num_minus1,
                } => {
                    let diff = abs_diff_pic_num_minus1 as i32 + 1;
                    let mut pic_num_no_wrap = pic_num_pred - diff;
                    if pic_num_no_wrap < 0 {
                        pic_num_no_wrap += max_frame_num;
                    }
                    pic_num_pred = pic_num_no_wrap;
                    let pic_num = if pic_num_no_wrap > cur_pic_num {
                        pic_num_no_wrap - max_frame_num
                    } else {
                        pic_num_no_wrap
                    };
                    self.find_short_term_ref_index_by_pic_num(
                        refs.as_slice(),
                        pic_num,
                        cur_pic_num,
                        max_frame_num,
                    )
                }
                RefPicListMod::ShortTermAdd {
                    abs_diff_pic_num_minus1,
                } => {
                    let diff = abs_diff_pic_num_minus1 as i32 + 1;
                    let mut pic_num_no_wrap = pic_num_pred + diff;
                    if pic_num_no_wrap >= max_frame_num {
                        pic_num_no_wrap -= max_frame_num;
                    }
                    pic_num_pred = pic_num_no_wrap;
                    let pic_num = if pic_num_no_wrap > cur_pic_num {
                        pic_num_no_wrap - max_frame_num
                    } else {
                        pic_num_no_wrap
                    };
                    self.find_short_term_ref_index_by_pic_num(
                        refs.as_slice(),
                        pic_num,
                        cur_pic_num,
                        max_frame_num,
                    )
                }
                RefPicListMod::LongTerm { long_term_pic_num } => refs
                    .iter()
                    .position(|pic| pic.long_term_frame_idx == Some(long_term_pic_num)),
            };

            if let Some(src_idx) = target_idx {
                let selected = refs.remove(src_idx);
                let dst_idx = insert_idx.min(refs.len());
                refs.insert(dst_idx, selected);
                insert_idx += 1;
            }
        }
    }

    pub(super) fn build_reference_list_l0_with_mod(
        &mut self,
        count: u32,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) -> Vec<RefPlanes> {
        let target = count.max(1) as usize;
        let mut refs = self.collect_default_reference_list_l0();
        self.apply_ref_pic_list_modifications(&mut refs, mods, cur_frame_num);
        let refs_empty = refs.is_empty();
        let mut out = Vec::with_capacity(target);
        for rank in 0..target {
            if let Some(pic) = refs.get(rank).copied().or_else(|| refs.first().copied()) {
                out.push(Self::reference_to_planes(pic));
            } else {
                out.push(self.zero_reference_planes());
            }
        }
        drop(refs);
        if refs_empty {
            self.record_missing_reference_fallback("build_l0_list_empty", -1, 0);
        }
        out
    }

    pub(super) fn build_reference_list_l1_with_mod(
        &mut self,
        count: u32,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) -> Vec<RefPlanes> {
        let target = count.max(1) as usize;
        let mut refs = self.collect_default_reference_list_l1();
        self.apply_ref_pic_list_modifications(&mut refs, mods, cur_frame_num);
        let refs_empty = refs.is_empty();
        let mut out = Vec::with_capacity(target);
        for rank in 0..target {
            if let Some(pic) = refs.get(rank).copied().or_else(|| refs.first().copied()) {
                out.push(Self::reference_to_planes(pic));
            } else {
                out.push(self.zero_reference_planes());
            }
        }
        drop(refs);
        if refs_empty {
            self.record_missing_reference_fallback("build_l1_list_empty", -1, 0);
        }
        out
    }

    pub(super) fn remove_short_term_by_pic_num(&mut self, pic_num: u32) -> bool {
        let pos = self.reference_frames.iter().rposition(|pic| {
            pic.long_term_frame_idx.is_none()
                && self.pic_num_from_frame_num(pic.frame_num) == pic_num
        });
        if let Some(idx) = pos {
            self.reference_frames.remove(idx);
            return true;
        }
        false
    }

    pub(super) fn remove_long_term_by_idx(&mut self, long_term_frame_idx: u32) -> bool {
        let pos = self
            .reference_frames
            .iter()
            .position(|pic| pic.long_term_frame_idx == Some(long_term_frame_idx));
        if let Some(idx) = pos {
            self.reference_frames.remove(idx);
            return true;
        }
        false
    }

    pub(super) fn trim_long_term_references(&mut self) {
        let Some(max_idx) = self.max_long_term_frame_idx else {
            self.reference_frames
                .retain(|pic| pic.long_term_frame_idx.is_none());
            return;
        };
        self.reference_frames
            .retain(|pic| pic.long_term_frame_idx.is_none_or(|idx| idx <= max_idx));
    }

    fn frame_num_wrap_for_short_term(&self, frame_num: u32, cur_frame_num: u32) -> i32 {
        let max_frame_num = self.max_frame_num_modulo();
        if max_frame_num == 0 {
            return 0;
        }
        let cur = cur_frame_num % max_frame_num;
        let val = frame_num % max_frame_num;
        if val > cur {
            val as i32 - max_frame_num as i32
        } else {
            val as i32
        }
    }

    pub(super) fn remove_short_term_with_lowest_frame_num_wrap_for(
        &mut self,
        cur_frame_num: u32,
    ) -> bool {
        if let Some((idx, _)) = self
            .reference_frames
            .iter()
            .enumerate()
            .filter(|(_, pic)| pic.long_term_frame_idx.is_none())
            .min_by_key(|(_, pic)| self.frame_num_wrap_for_short_term(pic.frame_num, cur_frame_num))
        {
            self.reference_frames.remove(idx);
            return true;
        }
        false
    }

    fn apply_sliding_window_if_needed_for(&mut self, cur_frame_num: u32) {
        if self.reference_frames.len() < self.max_reference_frames {
            return;
        }
        if !self.remove_short_term_with_lowest_frame_num_wrap_for(cur_frame_num) {
            let _ = self.reference_frames.pop_front();
        }
    }

    fn apply_sliding_window_if_needed(&mut self) {
        self.apply_sliding_window_if_needed_for(self.last_frame_num);
    }

    fn enforce_reference_capacity_for(&mut self, cur_frame_num: u32) {
        while self.reference_frames.len() > self.max_reference_frames {
            if !self.remove_short_term_with_lowest_frame_num_wrap_for(cur_frame_num) {
                let _ = self.reference_frames.pop_front();
            }
        }
    }

    pub(super) fn enforce_reference_capacity(&mut self) {
        self.enforce_reference_capacity_for(self.last_frame_num);
    }

    pub(super) fn push_non_existing_short_term_reference(&mut self, frame_num: u32, poc: i32) {
        self.apply_sliding_window_if_needed_for(frame_num);
        self.reference_frames.push_back(ReferencePicture {
            y: vec![128u8; self.ref_y.len()],
            u: vec![128u8; self.ref_u.len()],
            v: vec![128u8; self.ref_v.len()],
            frame_num,
            poc,
            long_term_frame_idx: None,
        });
        self.enforce_reference_capacity_for(frame_num);
    }

    pub(super) fn push_current_reference(&mut self, long_term_frame_idx: Option<u32>) {
        if self.last_nal_ref_idc == 0 {
            return;
        }
        if self.last_slice_type == 1 {
            return;
        }
        self.reference_frames.push_back(ReferencePicture {
            y: self.ref_y.clone(),
            u: self.ref_u.clone(),
            v: self.ref_v.clone(),
            frame_num: self.last_frame_num,
            poc: self.last_poc,
            long_term_frame_idx,
        });
    }

    pub(super) fn store_reference_with_marking(&mut self) {
        if self.last_nal_ref_idc == 0 || self.last_slice_type == 1 {
            return;
        }

        let marking = self.last_dec_ref_pic_marking.clone();
        let mut current_long_term_idx = None;
        let mut has_mmco5 = false;

        if marking.is_idr {
            if marking.no_output_of_prior_pics {
                self.output_queue.clear();
                self.reorder_buffer.clear();
            }
            self.reference_frames.clear();
            if marking.long_term_reference_flag {
                self.max_long_term_frame_idx = Some(0);
                current_long_term_idx = Some(0);
            } else {
                self.max_long_term_frame_idx = None;
            }
        } else if marking.adaptive {
            for op in marking.ops {
                match op {
                    MmcoOp::ForgetShort {
                        difference_of_pic_nums_minus1,
                    } => {
                        let pic_num_x = self.pic_num_subtract(
                            self.pic_num_from_frame_num(self.last_frame_num),
                            difference_of_pic_nums_minus1 + 1,
                        );
                        let _ = self.remove_short_term_by_pic_num(pic_num_x);
                    }
                    MmcoOp::ForgetLong { long_term_pic_num } => {
                        let _ = self.remove_long_term_by_idx(long_term_pic_num);
                    }
                    MmcoOp::ConvertShortToLong {
                        difference_of_pic_nums_minus1,
                        long_term_frame_idx,
                    } => {
                        let pic_num_x = self.pic_num_subtract(
                            self.pic_num_from_frame_num(self.last_frame_num),
                            difference_of_pic_nums_minus1 + 1,
                        );
                        let _ = self.remove_long_term_by_idx(long_term_frame_idx);
                        if let Some(pos) = self.reference_frames.iter().rposition(|pic| {
                            pic.long_term_frame_idx.is_none()
                                && self.pic_num_from_frame_num(pic.frame_num) == pic_num_x
                        }) && let Some(pic) = self.reference_frames.get_mut(pos)
                        {
                            pic.long_term_frame_idx = Some(long_term_frame_idx);
                        }
                    }
                    MmcoOp::TrimLong {
                        max_long_term_frame_idx_plus1,
                    } => {
                        self.max_long_term_frame_idx = max_long_term_frame_idx_plus1.checked_sub(1);
                        self.trim_long_term_references();
                    }
                    MmcoOp::ClearAll => {
                        self.reference_frames.clear();
                        self.max_long_term_frame_idx = None;
                        self.prev_ref_poc_msb = 0;
                        self.prev_ref_poc_lsb = 0;
                        self.prev_frame_num_offset_type1 = 0;
                        self.prev_frame_num_offset_type2 = 0;
                        self.last_frame_num = 0;
                        self.last_poc = 0;
                        has_mmco5 = true;
                    }
                    MmcoOp::MarkCurrentLong {
                        long_term_frame_idx,
                    } => {
                        current_long_term_idx = Some(long_term_frame_idx);
                    }
                }
            }
        } else if current_long_term_idx.is_none() {
            self.apply_sliding_window_if_needed();
        }

        if let Some(idx) = current_long_term_idx {
            if self.max_long_term_frame_idx.is_none_or(|max| idx <= max) {
                let _ = self.remove_long_term_by_idx(idx);
                self.push_current_reference(Some(idx));
            } else {
                self.push_current_reference(None);
            }
        } else {
            self.push_current_reference(None);
        }

        if has_mmco5 {
            if let Some(current) = self.reference_frames.back_mut() {
                current.frame_num = 0;
                current.poc = 0;
            }
        }
        self.enforce_reference_capacity();
    }

    pub(super) fn push_video_for_output(&mut self, vf: VideoFrame, poc: i32) {
        let entry = ReorderFrameEntry {
            frame: vf,
            poc,
            decode_order: self.decode_order_counter,
        };
        self.decode_order_counter = self.decode_order_counter.wrapping_add(1);
        let insert_pos = self.reorder_buffer.partition_point(|cur| {
            cur.poc < entry.poc || (cur.poc == entry.poc && cur.decode_order <= entry.decode_order)
        });
        self.reorder_buffer.insert(insert_pos, entry);

        // 先满足重排深度限制, 再满足 DPB 容量限制.
        while self.reorder_buffer.len() > self.reorder_depth {
            let out = self.reorder_buffer.remove(0);
            self.output_queue.push_back(Frame::Video(out.frame));
        }

        let dpb_capacity = self.max_reference_frames.max(1);
        while !self.reorder_buffer.is_empty()
            && self
                .reference_frames
                .len()
                .saturating_add(self.reorder_buffer.len())
                > dpb_capacity
        {
            let out = self.reorder_buffer.remove(0);
            self.output_queue.push_back(Frame::Video(out.frame));
        }
    }

    pub(super) fn drain_reorder_buffer_to_output(&mut self) {
        while !self.reorder_buffer.is_empty() {
            let out = self.reorder_buffer.remove(0);
            self.output_queue.push_back(Frame::Video(out.frame));
        }
    }

    pub(super) fn build_output_frame(&mut self, pts: i64, time_base: Rational, is_keyframe: bool) {
        let w = self.width as usize;
        let h = self.height as usize;

        if self.last_disable_deblocking_filter_idc != 1 {
            deblock::apply_deblock_yuv420_with_slice_params(
                &mut self.ref_y,
                &mut self.ref_u,
                &mut self.ref_v,
                deblock::DeblockSliceParams {
                    stride_y: self.stride_y,
                    stride_c: self.stride_c,
                    width: w,
                    height: h,
                    slice_qp: self.last_slice_qp,
                    disable_deblocking_filter_idc: self.last_disable_deblocking_filter_idc,
                    alpha_offset_div2: self.last_slice_alpha_c0_offset_div2,
                    beta_offset_div2: self.last_slice_beta_offset_div2,
                    mb_width: self.mb_width,
                    mb_height: self.mb_height,
                    mb_types: Some(&self.mb_types),
                    mb_cbp: Some(&self.mb_cbp),
                    mb_slice_first_mb: Some(&self.mb_slice_first_mb),
                    mv_l0_x: Some(&self.mv_l0_x),
                    mv_l0_y: Some(&self.mv_l0_y),
                    ref_idx_l0: Some(&self.ref_idx_l0),
                    cbf_luma: Some(&self.cbf_luma),
                    mv_l0_x_4x4: Some(&self.mv_l0_x_4x4),
                    mv_l0_y_4x4: Some(&self.mv_l0_y_4x4),
                    ref_idx_l0_4x4: Some(&self.ref_idx_l0_4x4),
                },
            );
        }

        let y_data = copy_plane(&self.ref_y, self.stride_y, w, h);
        let u_data = copy_plane(&self.ref_u, self.stride_c, w / 2, h / 2);
        let v_data = copy_plane(&self.ref_v, self.stride_c, w / 2, h / 2);

        let picture_type = match self.last_slice_type {
            1 => PictureType::B,
            2 | 4 => PictureType::I,
            0 | 3 => PictureType::P,
            _ => {
                if is_keyframe {
                    PictureType::I
                } else {
                    PictureType::P
                }
            }
        };

        let vf = VideoFrame {
            data: vec![y_data, u_data, v_data],
            linesize: vec![w, w / 2, w / 2],
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Yuv420p,
            pts,
            time_base,
            duration: 0,
            is_keyframe,
            picture_type,
            sample_aspect_ratio: Rational::new(1, 1),
            color_space: Default::default(),
            color_range: Default::default(),
        };
        let frame_poc = self.last_poc;
        self.store_reference_with_marking();
        self.push_video_for_output(vf, frame_poc);
    }
}
