//! B 帧 (双向预测帧) 解码
//!
//! 实现 B-VOP 宏块解码, 包括:
//! - Direct 模式: MV 从共定位 P 帧 MV 按 TRB/TRD 缩放
//! - Forward 模式: 使用前向参考帧 (时间上较早)
//! - Backward 模式: 使用后向参考帧 (时间上较晚)
//! - Interpolate 模式: 使用两个参考帧的加权平均

use log::trace;
use tao_core::TaoResult;

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::block::decode_inter_block_vlc;
use super::idct::idct_8x8;
use super::types::{BframeMbMode, MotionVector};
use super::vlc::{decode_b_mb_type, decode_dbquant, decode_modb};
use crate::frame::{PictureType, VideoFrame};

impl Mpeg4Decoder {
    /// 解码 B 帧
    pub(super) fn decode_b_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::B;
        frame.is_keyframe = false;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);
        trace!(
            "解码 B 帧: {}x{} ({}x{} MB), TRD={}, TRB={}",
            self.width, self.height, mb_w, mb_h, self.time_pp, self.time_bp
        );

        for mb_y in 0..mb_h as u32 {
            for mb_x in 0..mb_w as u32 {
                self.decode_b_macroblock(&mut frame, mb_x, mb_y, reader);
            }
        }
        Ok(frame)
    }

    /// 解码 B 帧宏块
    fn decode_b_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        reader: &mut BitReader,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;

        // 1. MODB
        let (mb_type_present, cbp_present) = decode_modb(reader);

        // 2. 宏块模式
        let mode = if mb_type_present {
            decode_b_mb_type(reader)
        } else {
            BframeMbMode::DirectNoneMv
        };

        // 3. CBP (6 bits)
        let cbp = if cbp_present {
            reader.read_bits(6).unwrap_or(0) as u8
        } else {
            0
        };

        // 4. DBQUANT (当 CBP != 0 时)
        if cbp != 0 {
            let dq = decode_dbquant(reader);
            self.quant = ((self.quant as i32 + dq).clamp(1, 31)) as u8;
        }

        // 4a. 隔行模式: field_dct 和 field_pred (B 帧也需处理)
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        let field_dct = if interlacing && (cbp != 0) {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };
        let field_pred = if interlacing {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };
        if field_pred {
            let _field_for_top = reader.read_bit().unwrap_or(false);
            let _field_for_bot = reader.read_bit().unwrap_or(false);
        }

        // 选择扫描表
        let scan_table = if field_dct {
            &super::tables::ALTERNATE_VERTICAL_SCAN
        } else {
            &super::tables::ZIGZAG_SCAN
        };

        // 5. 运动向量解码
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;
        let (forward_mvs, backward_mvs) = match mode {
            BframeMbMode::Direct | BframeMbMode::DirectNoneMv => {
                let delta_mv = if mode == BframeMbMode::Direct {
                    // Direct 模式: 读取一个 delta MV (f_code=1)
                    let dx = Self::decode_mv_component(reader, 1).unwrap_or(0);
                    let dy = Self::decode_mv_component(reader, 1).unwrap_or(0);
                    MotionVector { x: dx, y: dy }
                } else {
                    MotionVector::default()
                };
                self.compute_direct_mvs(mb_idx, delta_mv)
            }
            BframeMbMode::Forward => {
                // 前向 MV (零预测 + f_code_forward)
                let mv = self.decode_b_motion_vector(reader, self.f_code_forward);
                ([mv; 4], [MotionVector::default(); 4])
            }
            BframeMbMode::Backward => {
                // 后向 MV (零预测 + f_code_backward)
                let mv = self.decode_b_motion_vector(reader, self.f_code_backward);
                ([MotionVector::default(); 4], [mv; 4])
            }
            BframeMbMode::Interpolate => {
                // 双向 MV
                let fwd = self.decode_b_motion_vector(reader, self.f_code_forward);
                let bwd = self.decode_b_motion_vector(reader, self.f_code_backward);
                ([fwd; 4], [bwd; 4])
            }
        };

        // 6. 解码 DCT 块并重建
        let use_forward = matches!(
            mode,
            BframeMbMode::Forward
                | BframeMbMode::Interpolate
                | BframeMbMode::Direct
                | BframeMbMode::DirectNoneMv
        );
        let use_backward = matches!(
            mode,
            BframeMbMode::Backward
                | BframeMbMode::Interpolate
                | BframeMbMode::Direct
                | BframeMbMode::DirectNoneMv
        );

        let quarterpel = self
            .vol_info
            .as_ref()
            .map(|v| v.quarterpel)
            .unwrap_or(false);

        // Y 平面 (4 个 8x8 块)
        for block_idx in 0..4usize {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;
            let coded = cbp & (1 << (5 - block_idx)) != 0;

            let mut block = if coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, false);
            idct_8x8(&mut block);

            let fwd_mv = forward_mvs[block_idx];
            let bwd_mv = backward_mvs[block_idx];

            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 16 + bx as usize * 8 + x) as isize;
                    let py = (mb_y as usize * 16 + by as usize * 8 + y) as isize;
                    if px >= width as isize || py >= height as isize {
                        continue;
                    }
                    let idx = py as usize * width + px as usize;
                    let residual = block[y * 8 + x];
                    let pred = self.b_frame_predict(
                        0,
                        px,
                        py,
                        fwd_mv,
                        bwd_mv,
                        use_forward,
                        use_backward,
                        quarterpel,
                    );
                    frame.data[0][idx] = (pred as i32 + residual).clamp(0, 255) as u8;
                }
            }
        }

        // U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        // Chroma MV: Direct 模式使用 4MV 推导, 其他模式使用 1MV 推导
        let fwd_chroma = if matches!(mode, BframeMbMode::Direct | BframeMbMode::DirectNoneMv) {
            Self::chroma_mv_4mv(&forward_mvs)
        } else {
            Self::chroma_mv_1mv(forward_mvs[0])
        };
        let bwd_chroma = if matches!(mode, BframeMbMode::Direct | BframeMbMode::DirectNoneMv) {
            Self::chroma_mv_4mv(&backward_mvs)
        } else {
            Self::chroma_mv_1mv(backward_mvs[0])
        };

        for plane_idx in 0..2usize {
            let coded = cbp & (1 << (1 - plane_idx)) != 0;

            let mut block = if coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, false);
            idct_8x8(&mut block);

            for v in 0..8 {
                for u in 0..8 {
                    let px = (mb_x as usize * 8 + u) as isize;
                    let py = (mb_y as usize * 8 + v) as isize;
                    if px >= uv_width as isize || py >= uv_height as isize {
                        continue;
                    }
                    let idx = py as usize * uv_width + px as usize;
                    let residual = block[v * 8 + u];
                    let pred = self.b_frame_predict(
                        plane_idx + 1,
                        px,
                        py,
                        fwd_chroma,
                        bwd_chroma,
                        use_forward,
                        use_backward,
                        false, // chroma 不使用 qpel
                    );
                    frame.data[plane_idx + 1][idx] = (pred as i32 + residual).clamp(0, 255) as u8;
                }
            }
        }
    }

    /// 解码 B 帧运动向量 (零预测 + 范围包装)
    fn decode_b_motion_vector(&self, reader: &mut BitReader, f_code: u8) -> MotionVector {
        let dx = Self::decode_mv_component(reader, f_code).unwrap_or(0);
        let dy = Self::decode_mv_component(reader, f_code).unwrap_or(0);

        let mut mv = MotionVector { x: dx, y: dy };

        // 范围包装
        let scale_fac = 1i16 << f_code.saturating_sub(1);
        let high = 32 * scale_fac - 1;
        let low = -32 * scale_fac;
        let range = 64 * scale_fac;

        if mv.x < low {
            mv.x += range;
        } else if mv.x > high {
            mv.x -= range;
        }
        if mv.y < low {
            mv.y += range;
        } else if mv.y > high {
            mv.y -= range;
        }

        mv
    }

    /// 计算 Direct 模式的前向/后向 MV
    ///
    /// 使用共定位 P 帧宏块的 MV 按 TRB/TRD 时间距离比缩放:
    /// - forward_mv = (TRB * co_mv / TRD) + delta_mv
    /// - backward_mv = ((TRB - TRD) * co_mv / TRD) + delta_mv (当 delta != 0 时)
    ///   或 backward_mv = forward_mv - co_mv (当 delta == 0 时)
    pub(super) fn compute_direct_mvs(
        &self,
        mb_idx: usize,
        delta_mv: MotionVector,
    ) -> ([MotionVector; 4], [MotionVector; 4]) {
        let mut forward_mvs = [MotionVector::default(); 4];
        let mut backward_mvs = [MotionVector::default(); 4];

        let trd = self.time_pp.max(1);
        let trb = self.time_bp;

        for k in 0..4 {
            let co_mv = self
                .ref_mv_cache
                .get(mb_idx)
                .map(|mvs| mvs[k])
                .unwrap_or_default();

            let co_x = co_mv.x as i32;
            let co_y = co_mv.y as i32;

            // 前向 MV = (TRB / TRD) * co_mv + delta
            forward_mvs[k].x = ((trb * co_x) / trd) as i16 + delta_mv.x;
            forward_mvs[k].y = ((trb * co_y) / trd) as i16 + delta_mv.y;

            // 后向 MV: 特殊处理 delta == 0 的情况
            if delta_mv.x == 0 {
                backward_mvs[k].x = (((trb - trd) * co_x) / trd) as i16;
            } else {
                backward_mvs[k].x = forward_mvs[k].x - co_mv.x;
            }
            if delta_mv.y == 0 {
                backward_mvs[k].y = (((trb - trd) * co_y) / trd) as i16;
            } else {
                backward_mvs[k].y = forward_mvs[k].y - co_mv.y;
            }
        }

        (forward_mvs, backward_mvs)
    }

    /// B 帧预测值计算
    ///
    /// 根据 use_forward/use_backward 标志从对应参考帧获取预测值.
    /// - 仅前向: 使用 backward_reference (时间上较早的参考帧)
    /// - 仅后向: 使用 reference_frame (时间上较晚的参考帧)
    /// - 双向: 两者的平均值
    #[allow(clippy::too_many_arguments)]
    fn b_frame_predict(
        &self,
        plane: usize,
        px: isize,
        py: isize,
        fwd_mv: MotionVector,
        bwd_mv: MotionVector,
        use_forward: bool,
        use_backward: bool,
        quarterpel: bool,
    ) -> u8 {
        // 前向预测使用较早的参考帧 (backward_reference)
        let fwd_pred = if use_forward {
            self.backward_reference.as_ref().map(|ref_frame| {
                Self::motion_compensate(ref_frame, plane, px, py, fwd_mv.x, fwd_mv.y, 0, quarterpel)
            })
        } else {
            None
        };

        // 后向预测使用较晚的参考帧 (reference_frame)
        let bwd_pred = if use_backward {
            self.reference_frame.as_ref().map(|ref_frame| {
                Self::motion_compensate(ref_frame, plane, px, py, bwd_mv.x, bwd_mv.y, 0, quarterpel)
            })
        } else {
            None
        };

        match (fwd_pred, bwd_pred) {
            (Some(f), Some(b)) => ((f as u16 + b as u16 + 1) >> 1) as u8,
            (Some(f), None) => f,
            (None, Some(b)) => b,
            (None, None) => 128,
        }
    }
}
