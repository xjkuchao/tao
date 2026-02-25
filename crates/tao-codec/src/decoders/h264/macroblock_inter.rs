use super::*;

impl H264Decoder {
    fn ref_planes_matches_picture(planes: &RefPlanes, pic: &ReferencePicture) -> bool {
        let pic_is_long_term = pic.long_term_frame_idx.is_some();
        if planes.is_long_term != pic_is_long_term {
            return false;
        }
        if planes.is_long_term {
            return planes.long_term_frame_idx == pic.long_term_frame_idx;
        }
        planes.poc == pic.poc
    }

    fn same_reference_picture_identity(a: &ReferencePicture, b: &ReferencePicture) -> bool {
        if a.long_term_frame_idx.is_some() || b.long_term_frame_idx.is_some() {
            return a.long_term_frame_idx == b.long_term_frame_idx;
        }
        a.frame_num == b.frame_num && a.poc == b.poc
    }

    fn frame_num_backward_distance_for(&self, cur_frame_num: u32, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        let cur = cur_frame_num % max;
        let target = frame_num % max;
        let dist = (cur + max - target) % max;
        if dist == 0 { max } else { dist }
    }

    fn frame_num_forward_distance_for(&self, cur_frame_num: u32, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        let cur = cur_frame_num % max;
        let target = frame_num % max;
        let dist = (target + max - cur) % max;
        if dist == 0 { max } else { dist }
    }

    fn picture_uses_list1_motion(pic: &ReferencePicture) -> bool {
        pic.ref_idx_l1.iter().any(|&v| v >= 0) || pic.ref_idx_l1_4x4.iter().any(|&v| v >= 0)
    }

    fn collect_default_reference_list_l0_for_colocated_picture(
        &self,
        col_pic: &ReferencePicture,
    ) -> Vec<&ReferencePicture> {
        let is_b_like = Self::picture_uses_list1_motion(col_pic);
        if is_b_like {
            let cur_poc = col_pic.poc;
            let short_refs: Vec<&ReferencePicture> = self
                .reference_frames
                .iter()
                .filter(|pic| {
                    pic.long_term_frame_idx.is_none()
                        && !Self::same_reference_picture_identity(pic, col_pic)
                })
                .collect();
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
            let mut long_refs: Vec<&ReferencePicture> = self
                .reference_frames
                .iter()
                .filter(|pic| {
                    pic.long_term_frame_idx.is_some()
                        && !Self::same_reference_picture_identity(pic, col_pic)
                })
                .collect();
            long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
            refs.extend(long_refs);
            return refs;
        }

        let cur_frame_num = col_pic.frame_num;
        let mut short_refs: Vec<&ReferencePicture> = self
            .reference_frames
            .iter()
            .filter(|pic| {
                pic.long_term_frame_idx.is_none()
                    && !Self::same_reference_picture_identity(pic, col_pic)
            })
            .collect();
        short_refs.sort_by_key(|pic| {
            (
                self.frame_num_backward_distance_for(cur_frame_num, pic.frame_num),
                self.frame_num_forward_distance_for(cur_frame_num, pic.frame_num),
            )
        });
        let mut refs = short_refs;
        let mut long_refs: Vec<&ReferencePicture> = self
            .reference_frames
            .iter()
            .filter(|pic| {
                pic.long_term_frame_idx.is_some()
                    && !Self::same_reference_picture_identity(pic, col_pic)
            })
            .collect();
        long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
        refs.extend(long_refs);
        refs
    }

    fn find_reference_picture_for_planes(&self, planes: &RefPlanes) -> Option<&ReferencePicture> {
        if planes.is_long_term {
            if let Some(long_idx) = planes.long_term_frame_idx
                && let Some(found) = self
                    .reference_frames
                    .iter()
                    .rev()
                    .find(|pic| pic.long_term_frame_idx == Some(long_idx))
            {
                return Some(found);
            }
        } else if let Some(found) = self
            .reference_frames
            .iter()
            .rev()
            .find(|pic| pic.long_term_frame_idx.is_none() && pic.frame_num == planes.frame_num)
        {
            return Some(found);
        }
        self.reference_frames.iter().rev().find(|pic| {
            pic.poc == planes.poc && (pic.long_term_frame_idx.is_some() == planes.is_long_term)
        })
    }

    fn ref_pic_motion_4x4_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.mb_width * 4;
        let height = self.mb_height * 4;
        if x4 >= stride || y4 >= height {
            return None;
        }
        Some(y4 * stride + x4)
    }

    fn ref_pic_l0_motion_at(
        &self,
        pic: &ReferencePicture,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
    ) -> Option<(i32, i32, i8)> {
        let x4 = mb_x * 4 + part_x4;
        let y4 = mb_y * 4 + part_y4;
        if let Some(idx4) = self.ref_pic_motion_4x4_index(x4, y4) {
            let ref_idx = pic.ref_idx_l0_4x4.get(idx4).copied().unwrap_or(-1);
            if ref_idx >= 0 {
                return Some((
                    pic.mv_l0_x_4x4.get(idx4).copied().unwrap_or(0) as i32,
                    pic.mv_l0_y_4x4.get(idx4).copied().unwrap_or(0) as i32,
                    ref_idx,
                ));
            }
        }
        let mb_idx = self.mb_index(mb_x, mb_y)?;
        let ref_idx = pic.ref_idx_l0.get(mb_idx).copied().unwrap_or(-1);
        if ref_idx < 0 {
            return None;
        }
        Some((
            pic.mv_l0_x.get(mb_idx).copied().unwrap_or(0) as i32,
            pic.mv_l0_y.get(mb_idx).copied().unwrap_or(0) as i32,
            ref_idx,
        ))
    }

    fn ref_pic_l1_motion_at(
        &self,
        pic: &ReferencePicture,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
    ) -> Option<(i32, i32, i8)> {
        let x4 = mb_x * 4 + part_x4;
        let y4 = mb_y * 4 + part_y4;
        if let Some(idx4) = self.ref_pic_motion_4x4_index(x4, y4) {
            let ref_idx = pic.ref_idx_l1_4x4.get(idx4).copied().unwrap_or(-1);
            if ref_idx >= 0 {
                return Some((
                    pic.mv_l1_x_4x4.get(idx4).copied().unwrap_or(0) as i32,
                    pic.mv_l1_y_4x4.get(idx4).copied().unwrap_or(0) as i32,
                    ref_idx,
                ));
            }
        }
        let mb_idx = self.mb_index(mb_x, mb_y)?;
        let ref_idx = pic.ref_idx_l1.get(mb_idx).copied().unwrap_or(-1);
        if ref_idx < 0 {
            return None;
        }
        Some((
            pic.mv_l1_x.get(mb_idx).copied().unwrap_or(0) as i32,
            pic.mv_l1_y.get(mb_idx).copied().unwrap_or(0) as i32,
            ref_idx,
        ))
    }

    fn spatial_direct_neighbor_candidates_for_list(
        &self,
        x4: usize,
        y4: usize,
        part_w4: usize,
        list1: bool,
    ) -> [Option<(i32, i32, i8)>; 3] {
        let cand = |cx4: isize, cy4: isize, list1: bool| -> Option<(i32, i32, i8)> {
            if cx4 < 0 || cy4 < 0 {
                return None;
            }
            let cx4_u = cx4 as usize;
            let cy4_u = cy4 as usize;
            let mb_x = cx4_u / 4;
            let mb_y = cy4_u / 4;
            let mb_idx = self.mb_index(mb_x, mb_y);
            if list1 {
                self.l1_motion_candidate_4x4(cx4, cy4).or_else(|| {
                    let idx = mb_idx?;
                    let ref_idx = self.ref_idx_l1.get(idx).copied().unwrap_or(-1);
                    if ref_idx < 0 {
                        return None;
                    }
                    Some((
                        self.mv_l1_x.get(idx).copied().unwrap_or(0) as i32,
                        self.mv_l1_y.get(idx).copied().unwrap_or(0) as i32,
                        ref_idx,
                    ))
                })
            } else {
                self.l0_motion_candidate_4x4(cx4, cy4).or_else(|| {
                    let idx = mb_idx?;
                    let ref_idx = self.ref_idx_l0.get(idx).copied().unwrap_or(-1);
                    if ref_idx < 0 {
                        return None;
                    }
                    Some((
                        self.mv_l0_x.get(idx).copied().unwrap_or(0) as i32,
                        self.mv_l0_y.get(idx).copied().unwrap_or(0) as i32,
                        ref_idx,
                    ))
                })
            }
        };

        if list1 {
            let cand_a = cand(x4 as isize - 1, y4 as isize, true);
            let cand_b = cand(x4 as isize, y4 as isize - 1, true);
            let cand_c = cand((x4 + part_w4) as isize, y4 as isize - 1, true)
                .or_else(|| cand(x4 as isize - 1, y4 as isize - 1, true));
            [cand_a, cand_b, cand_c]
        } else {
            let cand_a = cand(x4 as isize - 1, y4 as isize, false);
            let cand_b = cand(x4 as isize, y4 as isize - 1, false);
            let cand_c = cand((x4 + part_w4) as isize, y4 as isize - 1, false)
                .or_else(|| cand(x4 as isize - 1, y4 as isize - 1, false));
            [cand_a, cand_b, cand_c]
        }
    }

    fn spatial_direct_ref_idx_from_neighbors(cands: &[Option<(i32, i32, i8)>; 3]) -> Option<i8> {
        let mut selected = i8::MAX;
        let mut found = false;
        for cand in cands.iter().flatten() {
            if cand.2 >= 0 {
                selected = selected.min(cand.2);
                found = true;
            }
        }
        if found { Some(selected) } else { None }
    }

    fn spatial_direct_mv_from_neighbors(
        cands: &[Option<(i32, i32, i8)>; 3],
        ref_idx: i8,
        fallback_mv_x: i32,
        fallback_mv_y: i32,
    ) -> (i32, i32) {
        let mut matched = [(0i32, 0i32); 3];
        let mut matched_count = 0usize;
        for cand in cands.iter().flatten() {
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

        let mut all = [(0i32, 0i32); 3];
        let mut all_count = 0usize;
        for cand in cands.iter().flatten() {
            all[all_count] = (cand.0, cand.1);
            all_count += 1;
        }
        if all_count == 0 {
            return (fallback_mv_x, fallback_mv_y);
        }
        if all_count == 1 {
            return all[0];
        }
        let a = all[0];
        let b = all[1];
        let c = if all_count == 3 { all[2] } else { all[1] };
        (median3(a.0, b.0, c.0), median3(a.1, b.1, c.1))
    }

    /// 判断共定位分区是否满足 col_zero_flag 条件 (规范 8.4.1.2.3).
    fn col_zero_flag_for_part(
        &self,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
        ref_l1_list: &[RefPlanes],
    ) -> bool {
        // 默认关闭 col_zero 置零路径, 仅在显式诊断时开启.
        if std::env::var("TAO_H264_ENABLE_COL_ZERO").as_deref() != Ok("1") {
            return false;
        }
        let col_planes = ref_l1_list.first();
        let col_planes = match col_planes {
            Some(p) => p,
            None => return false,
        };
        if col_planes.is_long_term {
            return false;
        }
        let col_pic = match self.find_reference_picture_for_planes(col_planes) {
            Some(p) => p,
            None => return false,
        };
        let mb_idx = match self.mb_index(mb_x, mb_y) {
            Some(idx) => idx,
            None => return false,
        };
        let col_mb_type = col_pic.mb_types.get(mb_idx).copied().unwrap_or(0);
        let col_is_intra = col_mb_type <= 25;
        if col_is_intra {
            return false;
        }
        if let Some((col_mv_x, col_mv_y, col_ref_l0)) =
            self.ref_pic_l0_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4)
        {
            if col_ref_l0 == 0 {
                return col_mv_x.abs() <= 1 && col_mv_y.abs() <= 1;
            }
            if col_ref_l0 >= 0 {
                return false;
            }
        }

        let Some((col_l1_mv_x, col_l1_mv_y, col_ref_l1)) =
            self.ref_pic_l1_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4)
        else {
            return false;
        };
        col_ref_l1 == 0 && col_l1_mv_x.abs() <= 1 && col_l1_mv_y.abs() <= 1
    }

    fn temporal_direct_colocated_l0_motion(
        &self,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) -> Option<(i32, i32, i8, u8, &ReferencePicture)> {
        if let Some(col_planes) = ref_l1_list.first()
            && let Some(col_pic) = self.find_reference_picture_for_planes(col_planes)
        {
            if let Some(motion) = self.ref_pic_l0_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4) {
                return Some((motion.0, motion.1, motion.2, 0, col_pic));
            }
            if let Some((mv_x, mv_y, col_ref_idx)) =
                self.ref_pic_l1_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4)
            {
                return Some((mv_x, mv_y, col_ref_idx, 1, col_pic));
            }
            // 对齐 FFmpeg temporal direct: list1[0] 共定位图像存在时, 不跨图回退到 list0[0].
            return None;
        }

        if let Some(col_planes) = ref_l0_list.first() {
            let col_pic = self.find_reference_picture_for_planes(col_planes)?;
            if let Some(motion) = self.ref_pic_l0_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4) {
                return Some((motion.0, motion.1, motion.2, 0, col_pic));
            }
            if let Some((mv_x, mv_y, col_ref_idx)) =
                self.ref_pic_l1_motion_at(col_pic, mb_x, mb_y, part_x4, part_y4)
            {
                return Some((mv_x, mv_y, col_ref_idx, 1, col_pic));
            }
        }
        None
    }

    fn map_col_to_list0_index_with_col_pic(
        &self,
        col_ref_idx: i8,
        col_list: u8,
        col_pic: &ReferencePicture,
        ref_l0_list: &[RefPlanes],
    ) -> i8 {
        if col_ref_idx < 0 || ref_l0_list.is_empty() {
            return -1;
        }
        let col_ref_poc_table = if col_list == 1 {
            &col_pic.ref_l1_poc
        } else {
            &col_pic.ref_l0_poc
        };
        // BUG-4 修复: 使用共定位图片解码时存储的参考列表 POC 进行匹配,
        // 而非从当前 DPB 重建 (DPB 可能已在 MMCO/新帧进入后发生变化).
        if let Some(&col_ref_poc) = col_ref_poc_table.get(col_ref_idx as usize) {
            if let Some((idx, _)) = ref_l0_list
                .iter()
                .enumerate()
                .find(|(_, planes)| planes.poc == col_ref_poc)
            {
                return idx as i8;
            }
        } else {
            // 回退: 如果存储的 POC 列表不可用, 使用旧的 DPB 重建方法
            let col_l0_list = self.collect_default_reference_list_l0_for_colocated_picture(col_pic);
            if let Some(col_ref_pic) = col_l0_list.get(col_ref_idx as usize).copied()
                && let Some((idx, _)) = ref_l0_list
                    .iter()
                    .enumerate()
                    .find(|(_, planes)| Self::ref_planes_matches_picture(planes, col_ref_pic))
            {
                return idx as i8;
            }
        }
        if ref_l0_list.is_empty() {
            -1
        } else {
            let idx = col_ref_idx.max(0) as usize;
            if idx < ref_l0_list.len() {
                idx as i8
            } else {
                0
            }
        }
    }

    fn clamp_direct_ref_idx(candidate: Option<i8>, list_len: usize) -> Option<i8> {
        if list_len == 0 {
            return None;
        }
        let idx = candidate.unwrap_or(0).max(0) as usize;
        Some(idx.min(list_len - 1) as i8)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_b_direct_motion_for_part(
        &self,
        mb_x: usize,
        mb_y: usize,
        part_x4: usize,
        part_y4: usize,
        part_w4: usize,
        mv_x: i32,
        mv_y: i32,
        direct_spatial_mv_pred_flag: bool,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) -> (Option<BMotion>, Option<BMotion>) {
        let use_spatial = direct_spatial_mv_pred_flag;

        if use_spatial {
            let x4 = mb_x * 4 + part_x4;
            let y4 = mb_y * 4 + part_y4;
            let l0_cands = self.spatial_direct_neighbor_candidates_for_list(x4, y4, part_w4, false);
            let l1_cands = self.spatial_direct_neighbor_candidates_for_list(x4, y4, part_w4, true);

            let mut ref_idx_l0 = Self::spatial_direct_ref_idx_from_neighbors(&l0_cands);
            let mut ref_idx_l1 = Self::spatial_direct_ref_idx_from_neighbors(&l1_cands);
            // H.264 spec 8.4.1.2.2: 当所有空间邻居都不可用时,
            // 设 refIdxL0=0, refIdxL1=0, MV 将由后续 spatial_direct_mv_from_neighbors
            // 返回 fallback (0,0). 不应回退到 temporal direct.
            if ref_idx_l0.is_none() && ref_idx_l1.is_none() {
                if !ref_l0_list.is_empty() {
                    ref_idx_l0 = Some(0);
                }
                if !ref_l1_list.is_empty() {
                    ref_idx_l1 = Some(0);
                }
            }
            if ref_idx_l0.is_none() && !ref_l0_list.is_empty() {
                ref_idx_l0 = Some(0);
            }
            if ref_idx_l1.is_none() && !ref_l1_list.is_empty() {
                ref_idx_l1 = Some(0);
            }
            ref_idx_l0 = Self::clamp_direct_ref_idx(ref_idx_l0, ref_l0_list.len());
            ref_idx_l1 = Self::clamp_direct_ref_idx(ref_idx_l1, ref_l1_list.len());

            let mut motion_l0 = ref_idx_l0.map(|ref_idx| {
                let (mv_l0_x, mv_l0_y) =
                    Self::spatial_direct_mv_from_neighbors(&l0_cands, ref_idx, mv_x, mv_y);
                BMotion {
                    mv_x: mv_l0_x,
                    mv_y: mv_l0_y,
                    ref_idx,
                }
            });
            let mut motion_l1 = ref_idx_l1.map(|ref_idx| {
                let (mv_l1_x, mv_l1_y) =
                    Self::spatial_direct_mv_from_neighbors(&l1_cands, ref_idx, mv_x, mv_y);
                BMotion {
                    mv_x: mv_l1_x,
                    mv_y: mv_l1_y,
                    ref_idx,
                }
            });

            let col_zero = self.col_zero_flag_for_part(mb_x, mb_y, part_x4, part_y4, ref_l1_list);
            if let Some(motion) = motion_l0.as_mut()
                && col_zero
                && motion.ref_idx == 0
            {
                motion.mv_x = 0;
                motion.mv_y = 0;
            }
            if let Some(motion) = motion_l1.as_mut()
                && col_zero
                && motion.ref_idx == 0
            {
                motion.mv_x = 0;
                motion.mv_y = 0;
            }
            return (motion_l0, motion_l1);
        }

        // Temporal Direct: 按规范用 dist_scale_factor 缩放共定位 MV.
        let temporal_col = self
            .temporal_direct_colocated_l0_motion(
                mb_x,
                mb_y,
                part_x4,
                part_y4,
                ref_l0_list,
                ref_l1_list,
            )
            .map(|(mx, my, r, col_list, pic)| (mx, my, r, col_list, Some(pic)))
            .unwrap_or((mv_x, mv_y, 0, 0, None));
        let (col_mv_x, col_mv_y, col_ref_idx, col_list, col_pic_opt) = temporal_col;
        let mut ref_idx_l0 = if let Some(col_pic) = col_pic_opt {
            self.map_col_to_list0_index_with_col_pic(col_ref_idx, col_list, col_pic, ref_l0_list)
        } else if (col_ref_idx as usize) < ref_l0_list.len() {
            col_ref_idx
        } else {
            -1
        };
        if ref_idx_l0 < 0 && !ref_l0_list.is_empty() {
            ref_idx_l0 = 0;
        }
        let ref_idx_l1 = if ref_l1_list.is_empty() { -1 } else { 0 };

        let l0_ref = select_ref_planes(ref_l0_list, ref_idx_l0);
        let l1_ref = select_ref_planes(ref_l1_list, ref_idx_l1);
        let dist_scale_factor = match (l0_ref, l1_ref) {
            (Some(r0), Some(r1)) => self
                .temporal_direct_dist_scale_factor(r0.poc, r1.poc)
                .unwrap_or(256),
            _ => 256,
        };
        let (direct_l0_mv_x, direct_l1_mv_x) =
            self.scale_temporal_direct_mv_pair_component(col_mv_x, dist_scale_factor);
        let (direct_l0_mv_y, direct_l1_mv_y) =
            self.scale_temporal_direct_mv_pair_component(col_mv_y, dist_scale_factor);

        let col_zero = self.col_zero_flag_for_part(mb_x, mb_y, part_x4, part_y4, ref_l1_list);
        let motion_l0 = if ref_idx_l0 >= 0 {
            Some(BMotion {
                mv_x: if col_zero && ref_idx_l0 == 0 {
                    0
                } else {
                    direct_l0_mv_x
                },
                mv_y: if col_zero && ref_idx_l0 == 0 {
                    0
                } else {
                    direct_l0_mv_y
                },
                ref_idx: ref_idx_l0,
            })
        } else {
            None
        };
        let motion_l1 = if ref_idx_l1 >= 0 {
            Some(BMotion {
                mv_x: if col_zero && ref_idx_l1 == 0 {
                    0
                } else {
                    direct_l1_mv_x
                },
                mv_y: if col_zero && ref_idx_l1 == 0 {
                    0
                } else {
                    direct_l1_mv_y
                },
                ref_idx: ref_idx_l1,
            })
        } else {
            None
        };
        (motion_l0, motion_l1)
    }

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
        self.build_b_direct_motion_for_part(
            mb_x,
            mb_y,
            0,
            0,
            4,
            mv_x,
            mv_y,
            direct_spatial_mv_pred_flag,
            ref_l0_list,
            ref_l1_list,
        )
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
        let base_part_x4 = sub_x / 4;
        let base_part_y4 = sub_y / 4;
        if self.direct_8x8_inference_enabled() {
            let (motion_l0, motion_l1) = self.build_b_direct_motion_for_part(
                mb_x,
                mb_y,
                base_part_x4,
                base_part_y4,
                2,
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

        let mut last_mv = (pred_mv_x, pred_mv_y, 0i8);
        for part_y in 0..2usize {
            for part_x in 0..2usize {
                let (part_pred_mv_x, part_pred_mv_y) = self.predict_mv_l0_partition(
                    mb_x,
                    mb_y,
                    base_part_x4 + part_x,
                    base_part_y4 + part_y,
                    1,
                    0,
                );
                let (motion_l0, motion_l1) = self.build_b_direct_motion_for_part(
                    mb_x,
                    mb_y,
                    base_part_x4 + part_x,
                    base_part_y4 + part_y,
                    1,
                    part_pred_mv_x,
                    part_pred_mv_y,
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

    /// 推导 P_Skip 的 L0 运动向量 (H.264 spec 8.4.1.1).
    ///
    /// 规则 (对齐 ffmpeg `pred_pskip_motion`):
    /// - 若 mbAddrA 不存在 (画面左边界): 返回 (0,0).
    /// - 若 mbAddrB 不存在 (画面上边界): 返回 (0,0).
    /// - 若 mbAddrA 使用 L0 且 ref==0 且 mv==(0,0): 返回 (0,0).
    /// - 若 mbAddrB 使用 L0 且 ref==0 且 mv==(0,0): 返回 (0,0).
    /// - 否则: 走 `ref_idx=0` 的 16x16 median 预测.
    ///
    /// 注意: intra 邻居视为 "存在但不使用 L0", 不触发零向量快捷返回.
    pub(super) fn predict_p_skip_mv(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        // spec 8.4.1.1: mbAddrA 不可用 (含 slice 边界) -> (0,0)
        if !self.left_avail(mb_x, mb_y) {
            return (0, 0);
        }
        // spec 8.4.1.1: mbAddrB 不可用 (含 slice 边界) -> (0,0)
        if !self.top_avail(mb_x, mb_y) {
            return (0, 0);
        }
        let x4 = mb_x * 4;
        let y4 = mb_y * 4;
        // 使用 4x4 级别查询, 确保分区化邻居读取正确的边界块.
        // l0_motion_candidate_4x4 对 intra 邻居 (ref_idx<0) 返回 None -> 不触发 zeromv.
        if let Some((mvx, mvy, ref_idx)) = self.l0_motion_candidate_4x4(x4 as isize - 1, y4 as isize)
        {
            if ref_idx == 0 && mvx == 0 && mvy == 0 {
                return (0, 0);
            }
        }
        if let Some((mvx, mvy, ref_idx)) = self.l0_motion_candidate_4x4(x4 as isize, y4 as isize - 1)
        {
            if ref_idx == 0 && mvx == 0 && mvy == 0 {
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
        self.prev_qp_delta_nz = false;
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
            self.set_mb_skip_flag(mb_idx, false);
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            self.clear_mb_mvd_cache(mb_x, mb_y);
            let skip = self.decode_p_mb_skip_flag(cabac, ctxs, mb_x, mb_y);

            if skip {
                self.set_mb_skip_flag(mb_idx, true);
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
                    None,
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
                    cur_qp = 0;
                }
            }

            if mb_idx < self.mb_qp.len() {
                self.mb_qp[mb_idx] = cur_qp;
            }
            if mb_idx + 1 < total {
                let terminate = cabac.decode_terminate() == 1;
                if terminate {
                    break;
                }
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
        let left_non_skip = self.left_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_skip_flags.get(i).copied())
                .unwrap_or(0)
                == 0;
        let top_non_skip = self.top_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_skip_flags.get(i).copied())
                .unwrap_or(0)
                == 0;
        let ctx = usize::from(left_non_skip) + usize::from(top_non_skip);
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
        let left_non_skip = self.left_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_skip_flags.get(i).copied())
                .map(|flag| flag == 0)
                .unwrap_or(false);
        let top_non_skip = self.top_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_skip_flags.get(i).copied())
                .map(|flag| flag == 0)
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
        // 对齐 FFmpeg `decode_cabac_mb_type`:
        // unavailable 或 direct 邻居均不贡献上下文; 仅 available 且 non-direct 时贡献 1.
        let left_non_direct = self.left_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254)
                .unwrap_or(false);
        let top_non_direct = self.top_avail(mb_x, mb_y)
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254)
                .unwrap_or(false);
        let ctx = usize::from(left_non_direct) + usize::from(top_non_direct);

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

    fn ref_idx_ctx_neighbor_bin(
        &self,
        list: usize,
        cur_mb_idx: usize,
        x4: usize,
        y4: usize,
        is_b_slice: bool,
    ) -> usize {
        let stride = self.mb_width * 4;
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return 0;
        }
        let nb_mb_x = x4 / 4;
        let nb_mb_y = y4 / 4;
        let Some(nb_mb_idx) = self.mb_index(nb_mb_x, nb_mb_y) else {
            return 0;
        };
        if nb_mb_idx != cur_mb_idx {
            match (
                self.mb_slice_first_mb.get(cur_mb_idx),
                self.mb_slice_first_mb.get(nb_mb_idx),
            ) {
                (Some(&cur), Some(&nb)) if cur != nb => return 0,
                _ => {}
            }
        }
        let mb_ty = self.mb_types.get(nb_mb_idx).copied().unwrap_or_default();
        if is_b_slice {
            if self.get_direct_4x4_flag(x4, y4) {
                return 0;
            }
            if mb_ty == 254 {
                return 0;
            }
        }
        // For both P and B slices, intra neighbors have implicit ref_idx=0
        if mb_ty <= 25 {
            return 0;
        }
        // P skip also has implicit ref_idx=0
        if mb_ty == 255 {
            return 0;
        }
        let idx4 = y4 * stride + x4;
        let ref_idx = if list == 0 {
            self.ref_idx_l0_4x4.get(idx4).copied().unwrap_or(-1)
        } else {
            self.ref_idx_l1_4x4.get(idx4).copied().unwrap_or(-1)
        };
        usize::from(ref_idx > 0)
    }

    fn ref_idx_ctx_inc(&self, list: usize, x4: usize, y4: usize, is_b_slice: bool) -> usize {
        let Some(cur_mb_idx) = self.mb_index(x4 / 4, y4 / 4) else {
            return 0;
        };
        let left = if x4 > 0 {
            self.ref_idx_ctx_neighbor_bin(list, cur_mb_idx, x4 - 1, y4, is_b_slice)
        } else {
            0
        };
        let top = if y4 > 0 {
            self.ref_idx_ctx_neighbor_bin(list, cur_mb_idx, x4, y4 - 1, is_b_slice)
        } else {
            0
        };
        left + (top << 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_ref_idx(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        num_ref_idx: u32,
        list: usize,
        x4: usize,
        y4: usize,
        is_b_slice: bool,
    ) -> u32 {
        if num_ref_idx <= 1 {
            return 0;
        }
        let mut ref_idx = 0u32;
        let mut ctx = self.ref_idx_ctx_inc(list, x4, y4, is_b_slice);
        while cabac.decode_decision(&mut ctxs[54 + ctx]) == 1 {
            ref_idx += 1;
            ctx = (ctx >> 2) + 4;
            if ref_idx >= 31 {
                break;
            }
        }
        if ref_idx >= num_ref_idx {
            // 越界通常意味着当前语法路径已偏离; 采用 0 号参考可减少后续误差扩散.
            let clipped = 0;
            warn!(
                "H264: CABAC ref_idx 越界, decoded={}, num_ref_idx={}, 已截断为 {}",
                ref_idx, num_ref_idx, clipped
            );
            return clipped;
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
            // 对齐 FFmpeg `decode_cabac_mb_mvd`:
            // while (get_cabac_bypass()) { ...; if (k > 24) return INT_MIN; }
            // 读取 bypass 在前, 命中上界时也会额外消费 1bit.
            while cabac.decode_bypass() == 1 {
                mvd += 1 << k;
                k += 1;
                if k > 24 {
                    // 码流异常时返回 0 作为保底, 避免继续扩展导致失控.
                    return 0;
                }
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
