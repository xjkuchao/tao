//! CAVLC 宏块级语法与残差解码.
//!
//! 提供 coded_block_pattern 映射表、nC 上下文计算、
//! 以及 CAVLC 亮度/色度残差的 MB 级解码接口.

use std::cell::Cell;

use super::*;

// ============================================================
// coded_block_pattern 映射表 (H.264 Table 9-4)
// ============================================================

/// Intra 宏块 CBP 映射: code_num → CBP
const GOLOMB_TO_INTRA_CBP: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];

/// Inter 宏块 CBP 映射: code_num → CBP
const GOLOMB_TO_INTER_CBP: [u8; 48] = [
    0, 16, 1, 2, 4, 8, 32, 3, 5, 10, 12, 15, 47, 7, 11, 13, 14, 6, 9, 31, 35, 37, 42, 44, 33, 34,
    36, 40, 39, 43, 45, 46, 17, 18, 20, 24, 19, 21, 26, 28, 23, 27, 29, 30, 22, 25, 38, 41,
];

/// I_16x16 宏块按 8x8 分组遍历 4x4 子块的顺序 (对齐 FFmpeg scan8).
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

/// I_8x8 预测模式遍历顺序 (左上, 右上, 左下, 右下).
const I8X8_SCAN_ORDER: [(usize, usize); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];

thread_local! {
    static CAVLC_BLOCK_ERROR_FLAG: Cell<bool> = const { Cell::new(false) };
}

// ============================================================
// nC 上下文计算
// ============================================================

impl H264Decoder {
    fn trace_cavlc_target_mb(&self, mb_x: usize, mb_y: usize) -> bool {
        let mb_idx = mb_y
            .checked_mul(self.mb_width)
            .and_then(|base| base.checked_add(mb_x))
            .unwrap_or(usize::MAX);
        std::env::var("TAO_H264_TRACE_CAVLC_MB")
            .ok()
            .and_then(|v| {
                let mut it = v.split(',');
                let frame = it.next()?.parse::<u32>().ok()?;
                let target_mb = it.next()?.parse::<usize>().ok()?;
                Some((frame, target_mb))
            })
            .map(|(frame, target_mb)| self.last_frame_num == frame && mb_idx == target_mb)
            .unwrap_or(false)
    }

    fn env_match_target_mb(&self, key: &str, mb_x: usize, mb_y: usize) -> bool {
        let mb_idx = mb_y
            .checked_mul(self.mb_width)
            .and_then(|base| base.checked_add(mb_x))
            .unwrap_or(usize::MAX);
        std::env::var(key)
            .ok()
            .and_then(|v| {
                let mut it = v.split(',');
                let frame = it.next()?.parse::<u32>().ok()?;
                let target_mb = it.next()?.parse::<usize>().ok()?;
                Some((frame, target_mb))
            })
            .map(|(frame, target_mb)| self.last_frame_num == frame && mb_idx == target_mb)
            .unwrap_or(false)
    }

    fn env_match_target_frame(&self, key: &str) -> bool {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .map(|frame| self.last_frame_num == frame)
            .unwrap_or(false)
    }

    fn debug_restore_luma_after_residual(&self, mb_x: usize, mb_y: usize) -> bool {
        let zero_residual_this_mb = self.env_match_target_mb("TAO_H264_ZERO_RES_MB", mb_x, mb_y)
            || self.env_match_target_frame("TAO_H264_ZERO_RES_FRAME");
        zero_residual_this_mb
            || self.env_match_target_mb("TAO_H264_SKIP_LUMA_MB", mb_x, mb_y)
            || self.env_match_target_frame("TAO_H264_SKIP_LUMA_FRAME")
    }

    fn debug_restore_chroma_after_residual(&self, mb_x: usize, mb_y: usize) -> bool {
        let zero_residual_this_mb = self.env_match_target_mb("TAO_H264_ZERO_RES_MB", mb_x, mb_y)
            || self.env_match_target_frame("TAO_H264_ZERO_RES_FRAME");
        zero_residual_this_mb
            || self.env_match_target_mb("TAO_H264_SKIP_CHROMA_MB", mb_x, mb_y)
            || self.env_match_target_frame("TAO_H264_SKIP_CHROMA_FRAME")
    }

    fn trace_cavlc_mb_pixels_enabled(&self) -> bool {
        std::env::var("TAO_H264_TRACE_CAVLC_MB_PIXELS").as_deref() == Ok("1")
    }

    fn trace_cavlc_luma_mb_block(&self, mb_x: usize, mb_y: usize, stage: &str) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        let px0 = mb_x * 16;
        let py0 = mb_y * 16;
        eprintln!(
            "[H264-CAVLC-MB-PIX] frame_num={} mb_idx={} stage={} (x={},y={}) Y16x16:",
            self.last_frame_num, mb_idx, stage, mb_x, mb_y
        );
        for dy in 0..16usize {
            let mut row = [0u8; 16];
            for (dx, sample) in row.iter_mut().enumerate() {
                let idx = (py0 + dy) * self.stride_y + (px0 + dx);
                *sample = self.ref_y.get(idx).copied().unwrap_or(0);
            }
            eprintln!("[H264-CAVLC-MB-PIX] stage={} dy{:02} {:?}", stage, dy, row);
        }
    }

    fn write_luma_4x4_block(&mut self, px: usize, py: usize, block: &[u8; 16]) {
        for y in 0..4usize {
            for x in 0..4usize {
                let idx = (py + y) * self.stride_y + (px + x);
                if idx < self.ref_y.len() {
                    self.ref_y[idx] = block[y * 4 + x];
                }
            }
        }
    }

    fn read_luma_8x8_block(&self, px: usize, py: usize) -> [u8; 64] {
        let mut block = [0u8; 64];
        for y in 0..8usize {
            for x in 0..8usize {
                let idx = (py + y) * self.stride_y + (px + x);
                block[y * 8 + x] = self.ref_y.get(idx).copied().unwrap_or(0);
            }
        }
        block
    }

    fn write_luma_8x8_block(&mut self, px: usize, py: usize, block: &[u8; 64]) {
        for y in 0..8usize {
            for x in 0..8usize {
                let idx = (py + y) * self.stride_y + (px + x);
                if idx < self.ref_y.len() {
                    self.ref_y[idx] = block[y * 8 + x];
                }
            }
        }
    }

    pub(super) fn reset_cavlc_block_error(&self) {
        CAVLC_BLOCK_ERROR_FLAG.with(|flag| flag.set(false));
    }

    pub(super) fn take_cavlc_block_error(&self) -> bool {
        CAVLC_BLOCK_ERROR_FLAG.with(|flag| {
            let value = flag.get();
            flag.set(false);
            value
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_cavlc_residual_block_or_zero(
        &self,
        br: &mut BitReader,
        nc: i32,
        max_num_coeff: usize,
        coeffs: &mut [i32],
        scene: &str,
        coord_x: usize,
        coord_y: usize,
    ) -> u8 {
        match cavlc::decode_cavlc_residual_block(br, nc, max_num_coeff, coeffs) {
            Ok(tc) => tc,
            Err(err) => {
                if std::env::var("TAO_H264_TRACE_CAVLC_ERRORS").as_deref() == Ok("1") {
                    let (has_left, has_top, na, nb) = match scene {
                        "inter_luma_4x4" | "i16x16_luma_ac" => {
                            let has_left = coord_x > 0
                                && (coord_x % 4 != 0 || self.left_avail(coord_x / 4, coord_y / 4));
                            let has_top = coord_y > 0
                                && (coord_y % 4 != 0 || self.top_avail(coord_x / 4, coord_y / 4));
                            let na = if has_left {
                                self.get_nz_count_luma(coord_x - 1, coord_y) as i32
                            } else {
                                -1
                            };
                            let nb = if has_top {
                                self.get_nz_count_luma(coord_x, coord_y - 1) as i32
                            } else {
                                -1
                            };
                            (has_left, has_top, na, nb)
                        }
                        "chroma_u_ac" => {
                            let has_left = coord_x > 0
                                && (coord_x % 2 != 0 || self.left_avail(coord_x / 2, coord_y / 2));
                            let has_top = coord_y > 0
                                && (coord_y % 2 != 0 || self.top_avail(coord_x / 2, coord_y / 2));
                            let na = if has_left {
                                self.get_nz_count_chroma_u(coord_x - 1, coord_y) as i32
                            } else {
                                -1
                            };
                            let nb = if has_top {
                                self.get_nz_count_chroma_u(coord_x, coord_y - 1) as i32
                            } else {
                                -1
                            };
                            (has_left, has_top, na, nb)
                        }
                        "chroma_v_ac" => {
                            let has_left = coord_x > 0
                                && (coord_x % 2 != 0 || self.left_avail(coord_x / 2, coord_y / 2));
                            let has_top = coord_y > 0
                                && (coord_y % 2 != 0 || self.top_avail(coord_x / 2, coord_y / 2));
                            let na = if has_left {
                                self.get_nz_count_chroma_v(coord_x - 1, coord_y) as i32
                            } else {
                                -1
                            };
                            let nb = if has_top {
                                self.get_nz_count_chroma_v(coord_x, coord_y - 1) as i32
                            } else {
                                -1
                            };
                            (has_left, has_top, na, nb)
                        }
                        _ => (false, false, -1, -1),
                    };
                    eprintln!(
                        "[H264-CAVLC-ERR] frame_num={} scene={} x={} y={} nc={} max_coeff={} has_left={} has_top={} na={} nb={} bits={} err={}",
                        self.last_frame_num,
                        scene,
                        coord_x,
                        coord_y,
                        nc,
                        max_num_coeff,
                        has_left,
                        has_top,
                        na,
                        nb,
                        br.bits_read(),
                        err
                    );
                }
                CAVLC_BLOCK_ERROR_FLAG.with(|flag| flag.set(true));
                let total_coeff_overflow_i16x16 = scene == "i16x16_luma_ac"
                    && max_num_coeff == 15
                    && err
                        .to_string()
                        .contains("CAVLC total_coeff=16 超过 max_num_coeff=15");
                if total_coeff_overflow_i16x16 { 15 } else { 0 }
            }
        }
    }

    /// 获取 luma 4x4 块的非零系数计数.
    pub(super) fn get_nz_count_luma(&self, x4: usize, y4: usize) -> u8 {
        self.cbf_index(x4, y4)
            .and_then(|idx| self.nz_count_luma.get(idx).copied())
            .unwrap_or(0)
    }

    /// 设置 luma 4x4 块的非零系数计数.
    pub(super) fn set_nz_count_luma(&mut self, x4: usize, y4: usize, count: u8) {
        if let Some(idx) = self.cbf_index(x4, y4)
            && let Some(slot) = self.nz_count_luma.get_mut(idx)
        {
            *slot = count;
        }
    }

    /// 获取 chroma U 4x4 块的非零系数计数.
    pub(super) fn get_nz_count_chroma_u(&self, x2: usize, y2: usize) -> u8 {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.nz_count_chroma_u.get(idx).copied())
            .unwrap_or(0)
    }

    /// 设置 chroma U 4x4 块的非零系数计数.
    pub(super) fn set_nz_count_chroma_u(&mut self, x2: usize, y2: usize, count: u8) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.nz_count_chroma_u.get_mut(idx)
        {
            *slot = count;
        }
    }

    /// 获取 chroma V 4x4 块的非零系数计数.
    pub(super) fn get_nz_count_chroma_v(&self, x2: usize, y2: usize) -> u8 {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.nz_count_chroma_v.get(idx).copied())
            .unwrap_or(0)
    }

    /// 设置 chroma V 4x4 块的非零系数计数.
    pub(super) fn set_nz_count_chroma_v(&mut self, x2: usize, y2: usize, count: u8) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.nz_count_chroma_v.get_mut(idx)
        {
            *slot = count;
        }
    }

    /// 计算 luma 4x4 块的 nC 上下文.
    ///
    /// nC = (nA + nB + 1) >> 1, 其中 nA=左邻块, nB=上邻块.
    /// 仅一方可用时直接取该方, 均不可用时返回 0.
    pub(super) fn calc_luma_nc(&self, x4: usize, y4: usize) -> i32 {
        let has_left = x4 > 0 && (x4 % 4 != 0 || self.left_avail(x4 / 4, y4 / 4));
        let has_top = y4 > 0 && (y4 % 4 != 0 || self.top_avail(x4 / 4, y4 / 4));
        match (has_left, has_top) {
            (true, true) => {
                let na = self.get_nz_count_luma(x4 - 1, y4) as i32;
                let nb = self.get_nz_count_luma(x4, y4 - 1) as i32;
                (na + nb + 1) >> 1
            }
            (true, false) => self.get_nz_count_luma(x4 - 1, y4) as i32,
            (false, true) => self.get_nz_count_luma(x4, y4 - 1) as i32,
            (false, false) => 0,
        }
    }

    /// 计算 chroma U 4x4 块的 nC 上下文.
    pub(super) fn calc_chroma_u_nc(&self, x2: usize, y2: usize) -> i32 {
        let has_left = x2 > 0 && (x2 % 2 != 0 || self.left_avail(x2 / 2, y2 / 2));
        let has_top = y2 > 0 && (y2 % 2 != 0 || self.top_avail(x2 / 2, y2 / 2));
        match (has_left, has_top) {
            (true, true) => {
                let na = self.get_nz_count_chroma_u(x2 - 1, y2) as i32;
                let nb = self.get_nz_count_chroma_u(x2, y2 - 1) as i32;
                (na + nb + 1) >> 1
            }
            (true, false) => self.get_nz_count_chroma_u(x2 - 1, y2) as i32,
            (false, true) => self.get_nz_count_chroma_u(x2, y2 - 1) as i32,
            (false, false) => 0,
        }
    }

    /// 计算 chroma V 4x4 块的 nC 上下文.
    pub(super) fn calc_chroma_v_nc(&self, x2: usize, y2: usize) -> i32 {
        let has_left = x2 > 0 && (x2 % 2 != 0 || self.left_avail(x2 / 2, y2 / 2));
        let has_top = y2 > 0 && (y2 % 2 != 0 || self.top_avail(x2 / 2, y2 / 2));
        match (has_left, has_top) {
            (true, true) => {
                let na = self.get_nz_count_chroma_v(x2 - 1, y2) as i32;
                let nb = self.get_nz_count_chroma_v(x2, y2 - 1) as i32;
                (na + nb + 1) >> 1
            }
            (true, false) => self.get_nz_count_chroma_v(x2 - 1, y2) as i32,
            (false, true) => self.get_nz_count_chroma_v(x2, y2 - 1) as i32,
            (false, false) => 0,
        }
    }

    // ============================================================
    // CAVLC CBP 解码
    // ============================================================

    /// 解码 coded_block_pattern (me(v) 映射).
    ///
    /// 返回 (luma_cbp, chroma_cbp), luma_cbp 为 4 位 (每位对应一个 8x8 块),
    /// chroma_cbp 为 0/1/2.
    pub(super) fn decode_cavlc_cbp(br: &mut BitReader, is_intra: bool) -> (u8, u8) {
        let code_num = read_ue(br).unwrap_or(0) as usize;
        let table = if is_intra {
            &GOLOMB_TO_INTRA_CBP
        } else {
            &GOLOMB_TO_INTER_CBP
        };
        let cbp = if code_num < table.len() {
            table[code_num]
        } else {
            0u8
        };
        let luma_cbp = cbp & 0x0f;
        let chroma_cbp = (cbp >> 4) & 0x03;
        (luma_cbp, chroma_cbp)
    }

    // ============================================================
    // I_4x4 / I_8x8 预测模式 (CAVLC 语法)
    // ============================================================

    /// CAVLC 解码 I_4x4 预测模式 (16 个 4x4 块).
    ///
    /// 每个 4x4 块: prev_intra4x4_pred_mode_flag(1bit),
    /// 若为 0 则 rem_intra4x4_pred_mode(3bits).
    pub(super) fn decode_cavlc_i4x4_pred_modes(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 16] {
        let mut modes = [2u8; 16];
        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            let left = if self.left_neighbor_available_4x4_intra(x4, y4) {
                i16::from(self.get_i4x4_mode(x4 - 1, y4))
            } else {
                -1
            };
            let top = if self.top_neighbor_available_4x4_intra(x4, y4) {
                i16::from(self.get_i4x4_mode(x4, y4 - 1))
            } else {
                -1
            };
            // 对齐 FFmpeg pred_intra_mode: 任一方向不可用时回落 DC(2), 否则取 min(A, B).
            let mpm = if left.min(top) < 0 {
                2u8
            } else {
                left.min(top).clamp(0, 11) as u8
            };

            let prev_flag = br.read_bit().unwrap_or(0);
            let mode = if prev_flag == 1 {
                mpm
            } else {
                let rem = br.read_bits(3).unwrap_or(0) as u8;
                if rem < mpm { rem } else { rem + 1 }
            };
            modes[sub_y * 4 + sub_x] = mode;
            // 同步到全局缓存, 供后续块引用
            self.set_i4x4_mode(x4, y4, mode);
        }
        modes
    }

    /// CAVLC 解码 I_8x8 预测模式 (4 个 8x8 块).
    ///
    /// 每个 8x8 块: prev_intra8x8_pred_mode_flag(1bit),
    /// 若为 0 则 rem_intra8x8_pred_mode(3bits).
    pub(super) fn decode_cavlc_i8x8_pred_modes(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 4] {
        let mut modes = [2u8; 4];
        let mb_left_avail = self.left_avail_intra_pred(mb_x, mb_y);
        let mb_top_avail = self.top_avail_intra_pred(mb_x, mb_y);
        for &(block_x, block_y) in &I8X8_SCAN_ORDER {
            let x4 = mb_x * 4 + block_x * 2;
            let y4 = mb_y * 4 + block_y * 2;
            let left = if block_x > 0 {
                i16::from(modes[block_y * 2 + (block_x - 1)])
            } else if mb_left_avail {
                i16::from(self.get_i4x4_mode(x4 - 1, y4))
            } else {
                -1
            };
            let top = if block_y > 0 {
                i16::from(modes[(block_y - 1) * 2 + block_x])
            } else if mb_top_avail {
                i16::from(self.get_i4x4_mode(x4, y4 - 1))
            } else {
                -1
            };
            // 对齐 FFmpeg pred_intra_mode: 任一方向不可用时回落 DC(2), 否则取 min(A, B).
            let mpm = if left.min(top) < 0 {
                2u8
            } else {
                left.min(top).clamp(0, 11) as u8
            };

            let prev_flag = br.read_bit().unwrap_or(0);
            let mode = if prev_flag == 1 {
                mpm
            } else {
                let rem = br.read_bits(3).unwrap_or(0) as u8;
                if rem < mpm { rem } else { rem + 1 }
            }
            .min(8);

            let idx = block_y * 2 + block_x;
            modes[idx] = mode;

            // I_8x8 模式同步到对应 2x2 个 4x4 子块, 供后续块 MPM 推导.
            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    self.set_i4x4_mode(x4 + sub_x, y4 + sub_y, mode);
                }
            }
        }
        modes
    }

    // ============================================================
    // CAVLC MB 级残差解码
    // ============================================================

    /// CAVLC 解码 I_4x4 宏块的完整 luma 残差.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_cavlc_i4x4_luma_residual(
        &mut self,
        br: &mut BitReader,
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        pred_modes: &[u8; 16],
        restore_luma_after_residual: bool,
    ) {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
                self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 0);
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

                let px = mb_x * 16 + abs_sub_x * 4;
                let py = mb_y * 16 + abs_sub_y * 4;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;
                let mb_idx = mb_y * self.mb_width + mb_x;
                let sub_idx = abs_sub_y * 4 + abs_sub_x;

                let mode = pred_modes[abs_sub_y * 4 + abs_sub_x];
                let trace_i4x4 = self.should_trace_i4x4_block(mb_x, mb_y, abs_sub_x, abs_sub_y);
                self.predict_i4x4_block_with_tr_unavail_fix(
                    mb_x, mb_y, abs_sub_x, abs_sub_y, px, py, mode,
                );
                let pred_block = if restore_luma_after_residual {
                    Some(self.read_luma_4x4_block(px, py))
                } else {
                    None
                };

                if !has_residual_8x8 {
                    self.set_luma_cbf(x4, y4, false);
                    self.set_nz_count_luma(x4, y4, 0);
                    if trace_i4x4 {
                        let final_block = self.read_luma_4x4_block(px, py);
                        eprintln!(
                            "[H264-I4X4-RES] frame_num={} mb_idx={} sub=({},{}#{}) mode={} qp={} bypass={} nc=- tc=0 raw=none used=none final={:?}",
                            self.last_frame_num,
                            mb_idx,
                            abs_sub_x,
                            abs_sub_y,
                            sub_idx,
                            mode,
                            qp,
                            transform_bypass,
                            final_block
                        );
                    }
                    continue;
                }

                let nc = self.calc_luma_nc(x4, y4);
                let has_left = x4 > 0 && (x4 % 4 != 0 || self.left_avail(x4 / 4, y4 / 4));
                let has_top = y4 > 0 && (y4 % 4 != 0 || self.top_avail(x4 / 4, y4 / 4));
                let na = if has_left {
                    self.get_nz_count_luma(x4 - 1, y4) as i32
                } else {
                    -1
                };
                let nb = if has_top {
                    self.get_nz_count_luma(x4, y4 - 1) as i32
                } else {
                    -1
                };
                let mut coeffs = [0i32; 16];
                let bits_before_res = br.bits_read();
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    16,
                    &mut coeffs,
                    "i4x4_luma",
                    x4,
                    y4,
                );
                let bits_after_res = br.bits_read();
                let raw_coeffs = coeffs;
                self.set_nz_count_luma(x4, y4, tc);
                let coded = tc > 0;
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    coded_8x8 = true;
                }

                if transform_bypass {
                    residual::apply_4x4_bypass_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs,
                    );
                    if trace_i4x4 {
                        let final_block = self.read_luma_4x4_block(px, py);
                        eprintln!(
                            "[H264-I4X4-RES] frame_num={} mb_idx={} sub=({},{}#{}) mode={} qp={} bypass={} bits_before={} bits_after={} nc={} has_left={} na={} has_top={} nb={} tc={} raw={:?} used={:?} final={:?}",
                            self.last_frame_num,
                            mb_idx,
                            abs_sub_x,
                            abs_sub_y,
                            sub_idx,
                            mode,
                            qp,
                            transform_bypass,
                            bits_before_res,
                            bits_after_res,
                            nc,
                            has_left,
                            na,
                            has_top,
                            nb,
                            tc,
                            raw_coeffs,
                            raw_coeffs,
                            final_block
                        );
                    }
                    if let Some(pred_block) = pred_block {
                        self.write_luma_4x4_block(px, py, &pred_block);
                    }
                } else {
                    residual::dequant_4x4_ac_with_scaling(&mut coeffs, qp, &luma_scaling_4x4);
                    let used_coeffs = coeffs;
                    residual::apply_4x4_ac_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs,
                    );
                    if trace_i4x4 {
                        let final_block = self.read_luma_4x4_block(px, py);
                        eprintln!(
                            "[H264-I4X4-RES] frame_num={} mb_idx={} sub=({},{}#{}) mode={} qp={} bypass={} bits_before={} bits_after={} nc={} has_left={} na={} has_top={} nb={} tc={} raw={:?} used={:?} final={:?}",
                            self.last_frame_num,
                            mb_idx,
                            abs_sub_x,
                            abs_sub_y,
                            sub_idx,
                            mode,
                            qp,
                            transform_bypass,
                            bits_before_res,
                            bits_after_res,
                            nc,
                            has_left,
                            na,
                            has_top,
                            nb,
                            tc,
                            raw_coeffs,
                            used_coeffs,
                            final_block
                        );
                    }
                    if let Some(pred_block) = pred_block {
                        self.write_luma_4x4_block(px, py, &pred_block);
                    }
                }
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }

    /// CAVLC 解码 I_8x8 预测 + 残差(按 8x8 块交织).
    ///
    /// 每个 8x8 块: 先预测, 再解码并应用该块残差, 保证后续块使用已重建像素.
    #[allow(clippy::too_many_arguments)]
    fn decode_cavlc_i8x8_pred_and_residual(
        &mut self,
        br: &mut BitReader,
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        pred_modes_8x8: &[u8; 4],
        restore_luma_after_residual: bool,
    ) {
        let luma_scaling_8x8 = self.active_luma_scaling_list_8x8(true);
        let transform_bypass = self.is_transform_bypass_active(qp);

        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
                self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 0);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);

        let has_left_mb = self.left_avail_intra_pred(mb_x, mb_y);
        let has_top_mb = self.top_avail_intra_pred(mb_x, mb_y);
        let has_top_right_mb = if mb_x + 1 < self.mb_width {
            self.top_right_avail_intra_pred(mb_x, mb_y)
        } else {
            false
        };

        for i8x8 in 0..4u8 {
            let block_x = (i8x8 & 1) as usize;
            let block_y = (i8x8 >> 1) as usize;
            let x8 = mb_x * 2 + block_x;
            let y8 = mb_y * 2 + block_y;
            let px = mb_x * 16 + block_x * 8;
            let py = mb_y * 16 + block_y * 8;

            let avail = intra::I8x8Avail {
                has_left: if block_x == 0 { has_left_mb } else { true },
                has_top: if block_y == 0 { has_top_mb } else { true },
                has_topleft: match (block_x, block_y) {
                    (0, 0) => has_left_mb && has_top_mb,
                    (1, 0) => has_top_mb,
                    (0, 1) => has_left_mb,
                    _ => true,
                },
                has_topright: match (block_x, block_y) {
                    (0, 0) => has_top_mb,
                    (1, 0) => has_top_right_mb,
                    (0, 1) => true,
                    _ => false,
                },
            };
            intra::predict_8x8(
                &mut self.ref_y,
                self.stride_y,
                px,
                py,
                pred_modes_8x8[i8x8 as usize],
                &avail,
            );
            let pred_block = if restore_luma_after_residual {
                Some(self.read_luma_8x8_block(px, py))
            } else {
                None
            };

            if luma_cbp & (1 << i8x8) == 0 {
                self.set_luma_8x8_cbf(x8, y8, false);
                for sub_y in 0..2 {
                    for sub_x in 0..2 {
                        let x4 = mb_x * 4 + block_x * 2 + sub_x;
                        let y4 = mb_y * 4 + block_y * 2 + sub_y;
                        self.set_luma_cbf(x4, y4, false);
                        self.set_nz_count_luma(x4, y4, 0);
                    }
                }
                continue;
            }

            let mut coeffs_8x8 = [0i32; 64];
            let mut total_nz = 0u8;
            for sub_idx in 0..4usize {
                let sub_x = sub_idx & 1;
                let sub_y = sub_idx >> 1;
                let x4 = mb_x * 4 + block_x * 2 + sub_x;
                let y4 = mb_y * 4 + block_y * 2 + sub_y;
                let nc = self.calc_luma_nc(x4, y4);
                let mut sub_coeffs = [0i32; 16];
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    16,
                    &mut sub_coeffs,
                    "i8x8_luma_sub",
                    x4,
                    y4,
                );
                self.set_nz_count_luma(x4, y4, tc);
                let coded = tc > 0;
                self.set_luma_cbf(x4, y4, coded);
                total_nz = total_nz.saturating_add(tc);
                // CAVLC 8x8: 每个 4x4 子块的第 n 个系数写入全局扫描位 `sub_idx + 4*n`.
                for (coeff_i, &coeff) in sub_coeffs.iter().enumerate() {
                    let scan_pos = sub_idx + coeff_i * 4;
                    coeffs_8x8[scan_pos] = coeff;
                }
            }
            let coded = total_nz > 0;
            self.set_luma_8x8_cbf(x8, y8, coded);

            if !coded {
                continue;
            }
            if transform_bypass {
                residual::apply_8x8_bypass_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_8x8,
                );
            } else {
                residual::apply_8x8_ac_residual_with_scaling(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_8x8,
                    qp,
                    &luma_scaling_8x8,
                );
            }
            if let Some(pred_block) = pred_block {
                self.write_luma_8x8_block(px, py, &pred_block);
            }
        }
    }

    /// CAVLC 解码 I_16x16 亮度 DC 系数.
    pub(super) fn decode_cavlc_luma_dc(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
    ) -> [i32; 16] {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        let trace_mb = self.trace_cavlc_target_mb(mb_x, mb_y);
        let mut nc = self.calc_luma_nc(mb_x * 4, mb_y * 4);
        if std::env::var("TAO_H264_FORCE_I16_DC_NC0").as_deref() == Ok("1") {
            nc = 0;
        }
        let mut dc_scan = [0i32; 16];
        let _tc = self.decode_cavlc_residual_block_or_zero(
            br,
            nc,
            16,
            &mut dc_scan,
            "i16x16_luma_dc",
            mb_x,
            mb_y,
        );
        if trace_mb {
            eprintln!(
                "[H264-CAVLC-I16-DC] frame_num={} mb_idx={} nc={} raw_scan={:?}",
                self.last_frame_num,
                mb_y * self.mb_width + mb_x,
                nc,
                dc_scan
            );
        }
        self.set_luma_dc_cbf(mb_x, mb_y, dc_scan.iter().any(|&c| c != 0));

        let mut dc_block = [0i32; 16];
        for (scan_pos, &(row, col)) in residual::ZIGZAG_4X4.iter().enumerate() {
            if scan_pos < 16 {
                dc_block[row * 4 + col] = dc_scan[scan_pos];
            }
        }
        if !transform_bypass {
            residual::inverse_hadamard_4x4(&mut dc_block);
            residual::dequant_luma_dc_with_scaling(&mut dc_block, qp, &luma_scaling_4x4);
        }
        if trace_mb {
            eprintln!(
                "[H264-CAVLC-I16-DC] frame_num={} mb_idx={} dequant_dc={:?}",
                self.last_frame_num,
                mb_y * self.mb_width + mb_x,
                dc_block
            );
        }
        dc_block
    }

    /// CAVLC 解码并应用 I_16x16 亮度残差 (DC + AC).
    pub(super) fn decode_cavlc_i16x16_luma_residual(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        dc_coeffs: &[i32; 16],
        has_luma_ac: bool,
    ) {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        let trace_mb = self.trace_cavlc_target_mb(mb_x, mb_y);
        let mb_idx = mb_y * self.mb_width + mb_x;

        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
                self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 0);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        let mut coded_8x8 = [false; 4];

        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let block_idx = sub_y * 4 + sub_x;
            let mut coeffs_scan = [0i32; 16];
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            let bits_before = br.bits_read();
            let mut nc_dbg = -1;
            let mut na_dbg = -1;
            let mut nb_dbg = -1;
            let mut tc_dbg = 0u8;
            let mut raw_dbg = [0i32; 16];

            if has_luma_ac {
                let nc = self.calc_luma_nc(x4, y4);
                nc_dbg = nc;
                let has_left = x4 > 0 && (x4 % 4 != 0 || self.left_avail(x4 / 4, y4 / 4));
                let has_top = y4 > 0 && (y4 % 4 != 0 || self.top_avail(x4 / 4, y4 / 4));
                if has_left {
                    na_dbg = self.get_nz_count_luma(x4 - 1, y4) as i32;
                }
                if has_top {
                    nb_dbg = self.get_nz_count_luma(x4, y4 - 1) as i32;
                }
                let mut ac_coeffs = [0i32; 16];
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    15,
                    &mut ac_coeffs,
                    "i16x16_luma_ac",
                    x4,
                    y4,
                );
                tc_dbg = tc;
                self.set_nz_count_luma(x4, y4, tc);
                let coded = tc > 0;
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    let idx8 = (sub_y / 2) * 2 + (sub_x / 2);
                    coded_8x8[idx8] = true;
                }
                coeffs_scan[1..16].copy_from_slice(&ac_coeffs[..15]);
                raw_dbg[1..16].copy_from_slice(&ac_coeffs[..15]);
            } else {
                self.set_luma_cbf(x4, y4, false);
                self.set_nz_count_luma(x4, y4, 0);
            }
            let px = mb_x * 16 + sub_x * 4;
            let py = mb_y * 16 + sub_y * 4;
            if transform_bypass {
                coeffs_scan[0] = dc_coeffs[block_idx];
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                );
            } else {
                // DC 已在 decode_cavlc_luma_dc 中反量化, 仅对 AC 反量化
                residual::dequant_4x4_ac_with_scaling(&mut coeffs_scan, qp, &luma_scaling_4x4);
                coeffs_scan[0] = dc_coeffs[block_idx];
                residual::apply_4x4_ac_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_scan,
                );
            }
            if trace_mb {
                let final_block = self.read_luma_4x4_block(px, py);
                eprintln!(
                    "[H264-CAVLC-I16-BLK] frame_num={} mb_idx={} sub=({},{}#{}) bits_before={} bits_after={} nc={} na={} nb={} tc={} dc={} raw={:?} final={:?}",
                    self.last_frame_num,
                    mb_idx,
                    sub_x,
                    sub_y,
                    block_idx,
                    bits_before,
                    br.bits_read(),
                    nc_dbg,
                    na_dbg,
                    nb_dbg,
                    tc_dbg,
                    dc_coeffs[block_idx],
                    raw_dbg,
                    final_block
                );
            }
        }

        for (idx8, coded) in coded_8x8.iter().copied().enumerate() {
            let x8 = idx8 & 1;
            let y8 = idx8 >> 1;
            self.set_luma_8x8_cbf(mb_x * 2 + x8, mb_y * 2 + y8, coded);
        }
    }

    /// CAVLC 解码 Inter 8x8 变换的 luma 残差.
    ///
    /// 每个 8x8 块由 4 个 4x4 CAVLC 块组成 (每块最多 16 系数),
    /// 按 H.264 规范 8x8 扫描表重建后送入 8x8 IDCT.
    /// 对应 FFmpeg `decode_luma_residual` 中 `IS_8x8DCT(mb_type)` 分支.
    fn decode_cavlc_inter_luma_8x8_residual(
        &mut self,
        br: &mut BitReader,
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
    ) {
        let luma_scaling_8x8 = self.active_luma_scaling_list_8x8(false);
        let transform_bypass = self.is_transform_bypass_active(qp);

        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
                self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 0);
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
                // 更新 4 个 4x4 子块
                for sub_y in 0..2 {
                    for sub_x in 0..2 {
                        let x4 = mb_x * 4 + x8x8 * 2 + sub_x;
                        let y4 = mb_y * 4 + y8x8 * 2 + sub_y;
                        self.set_nz_count_luma(x4, y4, 0);
                        self.set_luma_cbf(x4, y4, false);
                    }
                }
                continue;
            }

            // 解码 4 个 4x4 CAVLC 子块 (共 64 系数)
            let mut coeffs_8x8 = [0i32; 64];
            let mut total_nz = 0u8;
            for sub_idx in 0..4usize {
                let sub_x = sub_idx & 1;
                let sub_y = sub_idx >> 1;
                let x4 = mb_x * 4 + x8x8 * 2 + sub_x;
                let y4 = mb_y * 4 + y8x8 * 2 + sub_y;
                let nc = self.calc_luma_nc(x4, y4);
                let mut sub_coeffs = [0i32; 16];
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    16,
                    &mut sub_coeffs,
                    "inter_luma_8x8_sub",
                    x4,
                    y4,
                );
                self.set_nz_count_luma(x4, y4, tc);
                self.set_luma_cbf(x4, y4, tc > 0);
                total_nz += tc;
                // CAVLC 8x8: 每个 4x4 子块的第 n 个系数写入全局扫描位 `sub_idx + 4*n`.
                for (coeff_i, &coeff) in sub_coeffs.iter().enumerate() {
                    let scan_pos = sub_idx + coeff_i * 4;
                    coeffs_8x8[scan_pos] = coeff;
                }
            }
            let coded = total_nz > 0;
            self.set_luma_8x8_cbf(x8, y8, coded);

            if coded {
                let px = mb_x * 16 + x8x8 * 8;
                let py = mb_y * 16 + y8x8 * 8;
                if transform_bypass {
                    residual::apply_8x8_bypass_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs_8x8,
                    );
                } else {
                    residual::apply_8x8_ac_residual_with_scaling(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs_8x8,
                        qp,
                        &luma_scaling_8x8,
                    );
                }
            }
        }
    }

    /// CAVLC 解码 Inter 宏块的 luma 残差 (I_4x4 式逐块解码, 无预测重建).
    pub(super) fn decode_cavlc_inter_luma_residual(
        &mut self,
        br: &mut BitReader,
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
    ) {
        let luma_scaling_4x4 = self.active_luma_scaling_list_4x4(false);
        let transform_bypass = self.is_transform_bypass_active(qp);

        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
                self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 0);
            }
        }

        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let has_residual = luma_cbp & (1 << i8x8) != 0;
            let mut coded_8x8 = false;

            for i_sub in 0..4 {
                let sub_x = i_sub & 1;
                let sub_y = i_sub >> 1;
                let abs_sub_x = x8x8 * 2 + sub_x;
                let abs_sub_y = y8x8 * 2 + sub_y;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;

                if !has_residual {
                    self.set_luma_cbf(x4, y4, false);
                    self.set_nz_count_luma(x4, y4, 0);
                    continue;
                }

                let nc = self.calc_luma_nc(x4, y4);
                let mut coeffs = [0i32; 16];
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    16,
                    &mut coeffs,
                    "inter_luma_4x4",
                    x4,
                    y4,
                );
                self.set_nz_count_luma(x4, y4, tc);
                let coded = tc > 0;
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    coded_8x8 = true;
                }

                let px = mb_x * 16 + abs_sub_x * 4;
                let py = mb_y * 16 + abs_sub_y * 4;
                if transform_bypass {
                    residual::apply_4x4_bypass_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs,
                    );
                } else {
                    residual::dequant_4x4_ac_with_scaling(&mut coeffs, qp, &luma_scaling_4x4);
                    residual::apply_4x4_ac_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs,
                    );
                }
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }

    /// CAVLC 解码并应用色度残差 (DC + AC).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_cavlc_chroma_residual(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        has_chroma_ac: bool,
        intra_defaults: bool,
    ) {
        let u_scaling_4x4 = self.active_chroma_scaling_list_4x4(intra_defaults, false);
        let v_scaling_4x4 = self.active_chroma_scaling_list_4x4(intra_defaults, true);
        let transform_bypass = self.is_transform_bypass_active(qp);
        let (chroma_off_u, chroma_off_v) = self
            .pps
            .as_ref()
            .map(|p| (p.chroma_qp_index_offset, p.second_chroma_qp_index_offset))
            .unwrap_or((0, 0));
        let trace_this_mb = self.trace_cavlc_target_mb(mb_x, mb_y);
        if trace_this_mb {
            let mb_idx = mb_y * self.mb_width + mb_x;
            eprintln!(
                "[H264-CAVLC-CHROMA] frame_num={} mb_idx={} has_chroma_ac={} qp={} bits_before={}",
                self.last_frame_num,
                mb_idx,
                has_chroma_ac,
                qp,
                br.bits_read()
            );
        }
        let chroma_qp_u = chroma_qp_from_luma_with_offset(qp, chroma_off_u);
        let chroma_qp_v = chroma_qp_from_luma_with_offset(qp, chroma_off_v);

        // Chroma DC: nc=-1 (chroma DC 专用表)
        let mut u_dc_scan = [0i32; 4];
        let _tc_u_dc = self.decode_cavlc_residual_block_or_zero(
            br,
            -1,
            4,
            &mut u_dc_scan,
            "chroma_u_dc",
            mb_x,
            mb_y,
        );
        self.set_chroma_dc_u_cbf(mb_x, mb_y, u_dc_scan.iter().any(|&c| c != 0));

        let mut v_dc_scan = [0i32; 4];
        let _tc_v_dc = self.decode_cavlc_residual_block_or_zero(
            br,
            -1,
            4,
            &mut v_dc_scan,
            "chroma_v_dc",
            mb_x,
            mb_y,
        );
        self.set_chroma_dc_v_cbf(mb_x, mb_y, v_dc_scan.iter().any(|&c| c != 0));
        if trace_this_mb {
            eprintln!(
                "[H264-CAVLC-CHROMA] frame_num={} mb_idx={} after_dc bits={} u_dc={:?} v_dc={:?}",
                self.last_frame_num,
                mb_y * self.mb_width + mb_x,
                br.bits_read(),
                u_dc_scan,
                v_dc_scan
            );
        }

        let mut u_dc = [0i32; 4];
        u_dc.copy_from_slice(&u_dc_scan[..4]);
        if !transform_bypass {
            residual::inverse_hadamard_2x2(&mut u_dc);
            residual::dequant_chroma_dc_with_scaling(&mut u_dc, chroma_qp_u, &u_scaling_4x4);
        }

        let mut v_dc = [0i32; 4];
        v_dc.copy_from_slice(&v_dc_scan[..4]);
        if !transform_bypass {
            residual::inverse_hadamard_2x2(&mut v_dc);
            residual::dequant_chroma_dc_with_scaling(&mut v_dc, chroma_qp_v, &v_scaling_4x4);
        }

        // Chroma AC
        let mut u_scans = [[0i32; 16]; 4];
        let mut v_scans = [[0i32; 16]; 4];

        if has_chroma_ac {
            for (block_idx, u_scan) in u_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                let nc = self.calc_chroma_u_nc(x2, y2);
                let mut ac_coeffs = [0i32; 16];
                if trace_this_mb {
                    eprintln!(
                        "[H264-CAVLC-CHROMA-BLK] frame_num={} mb_idx={} plane=U block_idx={} x2={} y2={} bits_before={} nc={}",
                        self.last_frame_num,
                        mb_y * self.mb_width + mb_x,
                        block_idx,
                        x2,
                        y2,
                        br.bits_read(),
                        nc
                    );
                }
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    15,
                    &mut ac_coeffs,
                    "chroma_u_ac",
                    x2,
                    y2,
                );
                self.set_nz_count_chroma_u(x2, y2, tc);
                self.set_chroma_u_cbf(x2, y2, tc > 0);
                if trace_this_mb {
                    eprintln!(
                        "[H264-CAVLC-CHROMA-BLK] frame_num={} mb_idx={} plane=U block_idx={} bits_after={} tc={} coeffs={:?}",
                        self.last_frame_num,
                        mb_y * self.mb_width + mb_x,
                        block_idx,
                        br.bits_read(),
                        tc,
                        ac_coeffs
                    );
                }
                u_scan[1..16].copy_from_slice(&ac_coeffs[..15]);
            }
            for (block_idx, v_scan) in v_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                let nc = self.calc_chroma_v_nc(x2, y2);
                let mut ac_coeffs = [0i32; 16];
                if trace_this_mb {
                    eprintln!(
                        "[H264-CAVLC-CHROMA-BLK] frame_num={} mb_idx={} plane=V block_idx={} x2={} y2={} bits_before={} nc={}",
                        self.last_frame_num,
                        mb_y * self.mb_width + mb_x,
                        block_idx,
                        x2,
                        y2,
                        br.bits_read(),
                        nc
                    );
                }
                let tc = self.decode_cavlc_residual_block_or_zero(
                    br,
                    nc,
                    15,
                    &mut ac_coeffs,
                    "chroma_v_ac",
                    x2,
                    y2,
                );
                self.set_nz_count_chroma_v(x2, y2, tc);
                self.set_chroma_v_cbf(x2, y2, tc > 0);
                if trace_this_mb {
                    eprintln!(
                        "[H264-CAVLC-CHROMA-BLK] frame_num={} mb_idx={} plane=V block_idx={} bits_after={} tc={} coeffs={:?}",
                        self.last_frame_num,
                        mb_y * self.mb_width + mb_x,
                        block_idx,
                        br.bits_read(),
                        tc,
                        ac_coeffs
                    );
                }
                v_scan[1..16].copy_from_slice(&ac_coeffs[..15]);
            }
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-CHROMA] frame_num={} mb_idx={} after_ac bits={}",
                    self.last_frame_num,
                    mb_y * self.mb_width + mb_x,
                    br.bits_read()
                );
            }
        } else {
            for block_idx in 0..4usize {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, false);
                self.set_chroma_v_cbf(x2, y2, false);
                self.set_nz_count_chroma_u(x2, y2, 0);
                self.set_nz_count_chroma_v(x2, y2, 0);
            }
        }

        for block_idx in 0..4usize {
            let sub_x = block_idx & 1;
            let sub_y = block_idx >> 1;
            let px = mb_x * 8 + sub_x * 4;
            let py = mb_y * 8 + sub_y * 4;

            let mut u_scan = u_scans[block_idx];
            if transform_bypass {
                u_scan[0] = u_dc[block_idx];
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_u,
                    self.stride_c,
                    px,
                    py,
                    &u_scan,
                );
            } else {
                // DC 已反量化, 仅对 AC 反量化
                residual::dequant_4x4_ac_with_scaling(&mut u_scan, chroma_qp_u, &u_scaling_4x4);
                u_scan[0] = u_dc[block_idx];
                residual::apply_4x4_ac_residual(&mut self.ref_u, self.stride_c, px, py, &u_scan);
            }

            let mut v_scan = v_scans[block_idx];
            if transform_bypass {
                v_scan[0] = v_dc[block_idx];
                residual::apply_4x4_bypass_residual(
                    &mut self.ref_v,
                    self.stride_c,
                    px,
                    py,
                    &v_scan,
                );
            } else {
                // DC 已反量化, 仅对 AC 反量化
                residual::dequant_4x4_ac_with_scaling(&mut v_scan, chroma_qp_v, &v_scaling_4x4);
                v_scan[0] = v_dc[block_idx];
                residual::apply_4x4_ac_residual(&mut self.ref_v, self.stride_c, px, py, &v_scan);
            }
        }
    }

    /// CAVLC 解码完整的非 skip 宏块残差 (CBP + qp_delta + luma + chroma).
    ///
    /// 适用于 CAVLC 路径下的 I_4x4、I_16x16 和 Inter 宏块.
    /// 对于 I_16x16, CBP 从 mb_type 导出, 不调用此函数.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_cavlc_mb_residual(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
        cur_qp: &mut i32,
        is_intra: bool,
        no_sub_mb_part_size_less_than_8x8_flag: bool,
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        let trace_this_mb = self.trace_cavlc_target_mb(mb_x, mb_y);
        let trace_pixels = trace_this_mb && self.trace_cavlc_mb_pixels_enabled();
        if trace_this_mb {
            eprintln!(
                "[H264-CAVLC-RES] frame_num={} mb_idx={} bits_before={} is_intra={} no_sub_lt8x8={}",
                self.last_frame_num,
                mb_idx,
                br.bits_read(),
                is_intra,
                no_sub_mb_part_size_less_than_8x8_flag
            );
        }
        if trace_pixels {
            self.trace_cavlc_luma_mb_block(mb_x, mb_y, "before_residual");
        }
        let (luma_cbp, chroma_cbp) = Self::decode_cavlc_cbp(br, is_intra);
        self.set_mb_cbp(mb_x, mb_y, luma_cbp | (chroma_cbp << 4));

        // Inter 宏块: 若 PPS 允许 8x8 变换且 luma_cbp != 0, 读取 transform_size_8x8_flag.
        // 语法顺序必须先于 mb_qp_delta, 否则会导致位流消费错位.
        let use_8x8_raw = if !is_intra && luma_cbp != 0 && no_sub_mb_part_size_less_than_8x8_flag {
            let pps_8x8 = self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false);
            if pps_8x8 {
                br.read_bit().unwrap_or(0) == 1
            } else {
                false
            }
        } else {
            false
        };
        let use_8x8 = use_8x8_raw;
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);

        let has_residual = luma_cbp != 0 || chroma_cbp != 0;
        if has_residual {
            let qp_delta = read_se(br).unwrap_or(0);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        if luma_cbp != 0 {
            if use_8x8 {
                self.decode_cavlc_inter_luma_8x8_residual(br, luma_cbp, mb_x, mb_y, *cur_qp);
            } else {
                self.decode_cavlc_inter_luma_residual(br, luma_cbp, mb_x, mb_y, *cur_qp);
            }
        } else {
            // 清除 luma nz 计数和 cbf
            for sub_y in 0..4 {
                for sub_x in 0..4 {
                    let x4 = mb_x * 4 + sub_x;
                    let y4 = mb_y * 4 + sub_y;
                    self.set_luma_cbf(x4, y4, false);
                    self.set_nz_count_luma(x4, y4, 0);
                }
            }
            self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        }

        if chroma_cbp >= 1 {
            self.decode_cavlc_chroma_residual(br, mb_x, mb_y, *cur_qp, chroma_cbp >= 2, is_intra);
        } else {
            self.reset_chroma_cbf_mb(mb_x, mb_y);
            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    let x2 = mb_x * 2 + sub_x;
                    let y2 = mb_y * 2 + sub_y;
                    self.set_nz_count_chroma_u(x2, y2, 0);
                    self.set_nz_count_chroma_v(x2, y2, 0);
                }
            }
        }
        if trace_this_mb {
            eprintln!(
                "[H264-CAVLC-RES] frame_num={} mb_idx={} bits_after={} luma_cbp={} chroma_cbp={} use_8x8={} cur_qp={}",
                self.last_frame_num,
                mb_idx,
                br.bits_read(),
                luma_cbp,
                chroma_cbp,
                use_8x8,
                *cur_qp
            );
        }
        if trace_pixels {
            self.trace_cavlc_luma_mb_block(mb_x, mb_y, "after_residual");
        }
    }

    // ============================================================
    // CAVLC I 宏块完整解码
    // ============================================================

    /// CAVLC 解码一个 I 宏块 (I-slice 或 P/B-slice 中的 Intra MB).
    ///
    /// `raw_mb_type`: 已读取的 mb_type (I-slice 原始值, 或 P/B-slice 中减去偏移后的值).
    pub(super) fn decode_cavlc_i_mb(
        &mut self,
        br: &mut BitReader,
        mb_x: usize,
        mb_y: usize,
        raw_mb_type: u32,
        cur_qp: &mut i32,
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        // Intra 宏块不应沿用历史 inter 运动信息, 否则会污染后续参考帧与 direct 预测.
        self.clear_mb_motion_cache(mb_x, mb_y);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);

        let has_left = self.left_avail_intra_pred(mb_x, mb_y);
        let has_top = self.top_avail_intra_pred(mb_x, mb_y);
        let trace_this_mb = self.trace_cavlc_target_mb(mb_x, mb_y);
        let restore_luma_after_residual = self.debug_restore_luma_after_residual(mb_x, mb_y);
        let restore_chroma_after_residual = self.debug_restore_chroma_after_residual(mb_x, mb_y);
        if trace_this_mb {
            eprintln!(
                "[H264-CAVLC-I] frame_num={} mb_idx={} raw_mb_type={} bits_before={}",
                self.last_frame_num,
                mb_y * self.mb_width + mb_x,
                raw_mb_type,
                br.bits_read()
            );
            if restore_luma_after_residual {
                eprintln!(
                    "[H264-CAVLC-I] frame_num={} mb_idx={} skip_luma_residual=1",
                    self.last_frame_num, mb_idx
                );
            }
            if restore_chroma_after_residual {
                eprintln!(
                    "[H264-CAVLC-I] frame_num={} mb_idx={} skip_chroma_residual=1",
                    self.last_frame_num, mb_idx
                );
            }
        }

        if raw_mb_type == 0 {
            // I_4x4
            self.mb_types[mb_idx] = 0;
            let use_8x8 = self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
                && br.read_bit().unwrap_or(0) == 1;
            self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);
            let pred_modes_4x4 = if use_8x8 {
                [2u8; 16]
            } else {
                self.decode_cavlc_i4x4_pred_modes(br, mb_x, mb_y)
            };
            let pred_modes_8x8 = if use_8x8 {
                self.decode_cavlc_i8x8_pred_modes(br, mb_x, mb_y)
            } else {
                [2u8; 4]
            };
            let chroma_mode = read_ue(br).unwrap_or(0).min(3) as u8;
            self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

            intra::predict_chroma_8x8(
                &mut self.ref_u,
                self.stride_c,
                mb_x * 8,
                mb_y * 8,
                chroma_mode,
                has_left,
                has_top,
            );
            intra::predict_chroma_8x8(
                &mut self.ref_v,
                self.stride_c,
                mb_x * 8,
                mb_y * 8,
                chroma_mode,
                has_left,
                has_top,
            );
            let mut saved_u = [0u8; 8 * 8];
            let mut saved_v = [0u8; 8 * 8];
            if restore_chroma_after_residual {
                let base_cx = mb_x * 8;
                let base_cy = mb_y * 8;
                for py in 0..8usize {
                    let src_row = (base_cy + py) * self.stride_c + base_cx;
                    let dst_row = py * 8;
                    if src_row + 8 <= self.ref_u.len() && src_row + 8 <= self.ref_v.len() {
                        saved_u[dst_row..dst_row + 8]
                            .copy_from_slice(&self.ref_u[src_row..src_row + 8]);
                        saved_v[dst_row..dst_row + 8]
                            .copy_from_slice(&self.ref_v[src_row..src_row + 8]);
                    }
                }
            }

            let (luma_cbp, chroma_cbp) = Self::decode_cavlc_cbp(br, true);
            self.set_mb_cbp(mb_x, mb_y, luma_cbp | (chroma_cbp << 4));

            let has_residual = luma_cbp != 0 || chroma_cbp != 0;
            if has_residual {
                let qp_delta = read_se(br).unwrap_or(0);
                self.prev_qp_delta_nz = qp_delta != 0;
                *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
            } else {
                self.prev_qp_delta_nz = false;
            }

            if use_8x8 {
                self.decode_cavlc_i8x8_pred_and_residual(
                    br,
                    luma_cbp,
                    mb_x,
                    mb_y,
                    *cur_qp,
                    &pred_modes_8x8,
                    restore_luma_after_residual,
                );
            } else {
                self.decode_cavlc_i4x4_luma_residual(
                    br,
                    luma_cbp,
                    mb_x,
                    mb_y,
                    *cur_qp,
                    &pred_modes_4x4,
                    restore_luma_after_residual,
                );
            }

            if chroma_cbp >= 1 {
                self.decode_cavlc_chroma_residual(br, mb_x, mb_y, *cur_qp, chroma_cbp >= 2, true);
                if restore_chroma_after_residual {
                    let base_cx = mb_x * 8;
                    let base_cy = mb_y * 8;
                    for py in 0..8usize {
                        let dst_row = (base_cy + py) * self.stride_c + base_cx;
                        let src_row = py * 8;
                        if dst_row + 8 <= self.ref_u.len() && dst_row + 8 <= self.ref_v.len() {
                            self.ref_u[dst_row..dst_row + 8]
                                .copy_from_slice(&saved_u[src_row..src_row + 8]);
                            self.ref_v[dst_row..dst_row + 8]
                                .copy_from_slice(&saved_v[src_row..src_row + 8]);
                        }
                    }
                }
            } else {
                self.clear_cavlc_nz_counts_chroma(mb_x, mb_y);
            }
        } else if raw_mb_type <= 24 {
            // I_16x16
            self.mb_types[mb_idx] = raw_mb_type as u8;
            let pred_mode = ((raw_mb_type - 1) % 4) as u8;
            let cbp_chroma = ((raw_mb_type - 1) / 4 % 3) as u8;
            let cbp_luma_nz = raw_mb_type > 12;
            let cbp_luma: u8 = if cbp_luma_nz { 0x0f } else { 0x00 };
            self.set_mb_cbp(mb_x, mb_y, cbp_luma | (cbp_chroma << 4));
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-I16] frame_num={} mb_idx={} pred_mode={} cbp_luma_nz={} cbp_chroma={} bits_after_mb_type={}",
                    self.last_frame_num,
                    mb_idx,
                    pred_mode,
                    cbp_luma_nz,
                    cbp_chroma,
                    br.bits_read()
                );
            }

            let chroma_mode = read_ue(br).unwrap_or(0).min(3) as u8;
            self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

            let qp_delta = read_se(br).unwrap_or(0);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-I16] frame_num={} mb_idx={} chroma_mode={} qp_delta={} cur_qp={} bits_after_qp={}",
                    self.last_frame_num,
                    mb_idx,
                    chroma_mode,
                    qp_delta,
                    *cur_qp,
                    br.bits_read()
                );
            }

            intra::predict_16x16(
                &mut self.ref_y,
                self.stride_y,
                mb_x * 16,
                mb_y * 16,
                pred_mode,
                has_left,
                has_top,
            );
            intra::predict_chroma_8x8(
                &mut self.ref_u,
                self.stride_c,
                mb_x * 8,
                mb_y * 8,
                chroma_mode,
                has_left,
                has_top,
            );
            intra::predict_chroma_8x8(
                &mut self.ref_v,
                self.stride_c,
                mb_x * 8,
                mb_y * 8,
                chroma_mode,
                has_left,
                has_top,
            );
            let mut saved_luma = [0u8; 16 * 16];
            if restore_luma_after_residual {
                let base_x = mb_x * 16;
                let base_y = mb_y * 16;
                for py in 0..16usize {
                    let src_row = (base_y + py) * self.stride_y + base_x;
                    let dst_row = py * 16;
                    if src_row + 16 <= self.ref_y.len() {
                        saved_luma[dst_row..dst_row + 16]
                            .copy_from_slice(&self.ref_y[src_row..src_row + 16]);
                    }
                }
            }
            let mut saved_u = [0u8; 8 * 8];
            let mut saved_v = [0u8; 8 * 8];
            if restore_chroma_after_residual {
                let base_cx = mb_x * 8;
                let base_cy = mb_y * 8;
                for py in 0..8usize {
                    let src_row = (base_cy + py) * self.stride_c + base_cx;
                    let dst_row = py * 8;
                    if src_row + 8 <= self.ref_u.len() && src_row + 8 <= self.ref_v.len() {
                        saved_u[dst_row..dst_row + 8]
                            .copy_from_slice(&self.ref_u[src_row..src_row + 8]);
                        saved_v[dst_row..dst_row + 8]
                            .copy_from_slice(&self.ref_v[src_row..src_row + 8]);
                    }
                }
            }

            let dc_coeffs = self.decode_cavlc_luma_dc(br, mb_x, mb_y, *cur_qp);
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-I16] frame_num={} mb_idx={} after_luma_dc bits={} dc={:?}",
                    self.last_frame_num,
                    mb_idx,
                    br.bits_read(),
                    dc_coeffs
                );
            }
            self.decode_cavlc_i16x16_luma_residual(
                br,
                mb_x,
                mb_y,
                *cur_qp,
                &dc_coeffs,
                cbp_luma_nz,
            );
            if restore_luma_after_residual {
                let base_x = mb_x * 16;
                let base_y = mb_y * 16;
                for py in 0..16usize {
                    let dst_row = (base_y + py) * self.stride_y + base_x;
                    let src_row = py * 16;
                    if dst_row + 16 <= self.ref_y.len() {
                        self.ref_y[dst_row..dst_row + 16]
                            .copy_from_slice(&saved_luma[src_row..src_row + 16]);
                    }
                }
            }
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-I16] frame_num={} mb_idx={} after_luma_ac bits={}",
                    self.last_frame_num,
                    mb_idx,
                    br.bits_read()
                );
            }

            if cbp_chroma >= 1 {
                self.decode_cavlc_chroma_residual(br, mb_x, mb_y, *cur_qp, cbp_chroma >= 2, true);
                if restore_chroma_after_residual {
                    let base_cx = mb_x * 8;
                    let base_cy = mb_y * 8;
                    for py in 0..8usize {
                        let dst_row = (base_cy + py) * self.stride_c + base_cx;
                        let src_row = py * 8;
                        if dst_row + 8 <= self.ref_u.len() && dst_row + 8 <= self.ref_v.len() {
                            self.ref_u[dst_row..dst_row + 8]
                                .copy_from_slice(&saved_u[src_row..src_row + 8]);
                            self.ref_v[dst_row..dst_row + 8]
                                .copy_from_slice(&saved_v[src_row..src_row + 8]);
                        }
                    }
                }
            } else {
                self.clear_cavlc_nz_counts_chroma(mb_x, mb_y);
            }
            if trace_this_mb {
                eprintln!(
                    "[H264-CAVLC-I16] frame_num={} mb_idx={} after_chroma bits={}",
                    self.last_frame_num,
                    mb_idx,
                    br.bits_read()
                );
            }
        } else {
            // I_PCM (mb_type == 25): 字节对齐后读取原始样本
            self.mb_types[mb_idx] = 25;
            self.set_mb_cbp(mb_x, mb_y, 0x2f);
            self.prev_qp_delta_nz = false;
            br.align_to_byte();
            let x0 = mb_x * 16;
            let y0 = mb_y * 16;
            for dy in 0..16 {
                for dx in 0..16 {
                    let idx = (y0 + dy) * self.stride_y + x0 + dx;
                    let val = br.read_bits(8).unwrap_or(128) as u8;
                    if idx < self.ref_y.len() {
                        self.ref_y[idx] = val;
                    }
                }
            }
            let cx0 = mb_x * 8;
            let cy0 = mb_y * 8;
            for dy in 0..8 {
                for dx in 0..8 {
                    let idx = (cy0 + dy) * self.stride_c + cx0 + dx;
                    let val = br.read_bits(8).unwrap_or(128) as u8;
                    if idx < self.ref_u.len() {
                        self.ref_u[idx] = val;
                    }
                }
            }
            for dy in 0..8 {
                for dx in 0..8 {
                    let idx = (cy0 + dy) * self.stride_c + cx0 + dx;
                    let val = br.read_bits(8).unwrap_or(128) as u8;
                    if idx < self.ref_v.len() {
                        self.ref_v[idx] = val;
                    }
                }
            }
            // I_PCM 所有块标记为有内容
            for sub_y in 0..4 {
                for sub_x in 0..4 {
                    self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, true);
                    self.set_nz_count_luma(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 16);
                }
            }
            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    self.set_luma_8x8_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, true);
                    self.set_chroma_u_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, true);
                    self.set_chroma_v_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, true);
                    self.set_nz_count_chroma_u(mb_x * 2 + sub_x, mb_y * 2 + sub_y, 16);
                    self.set_nz_count_chroma_v(mb_x * 2 + sub_x, mb_y * 2 + sub_y, 16);
                }
            }
        }
    }

    /// 仅清除色度 nz 计数 (用于 chroma_cbp == 0 时).
    fn clear_cavlc_nz_counts_chroma(&mut self, mb_x: usize, mb_y: usize) {
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                self.set_nz_count_chroma_u(mb_x * 2 + sub_x, mb_y * 2 + sub_y, 0);
                self.set_nz_count_chroma_v(mb_x * 2 + sub_x, mb_y * 2 + sub_y, 0);
            }
        }
    }

    /// 清空一个宏块的 CAVLC 系数状态 (skip/零残差路径).
    pub(super) fn clear_cavlc_mb_coeff_state(&mut self, mb_x: usize, mb_y: usize) {
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                let x4 = mb_x * 4 + sub_x;
                let y4 = mb_y * 4 + sub_y;
                self.set_luma_cbf(x4, y4, false);
                self.set_nz_count_luma(x4, y4, 0);
            }
        }
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_nz_count_chroma_u(x2, y2, 0);
                self.set_nz_count_chroma_v(x2, y2, 0);
            }
        }
    }
}
