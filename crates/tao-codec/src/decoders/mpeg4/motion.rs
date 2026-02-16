//! 运动向量解码, 预测与运动补偿

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::tables::{ROUNDTAB_76, ROUNDTAB_79};
use super::types::MotionVector;
use super::vlc::MVD_VLC;
use crate::frame::VideoFrame;

impl Mpeg4Decoder {
    /// 三值取中 (用于 MV 预测)
    pub(super) fn median(a: i16, b: i16, c: i16) -> i16 {
        if a > b {
            if b > c {
                b
            } else if a > c {
                c
            } else {
                a
            }
        } else if b < c {
            b
        } else if a < c {
            c
        } else {
            a
        }
    }

    /// 解码 MVD (含 f_code 残差和范围包装)
    pub(super) fn decode_mv_component(reader: &mut BitReader, f_code: u8) -> Option<i16> {
        for &(len, code, index) in MVD_VLC {
            let Some(bits) = reader.peek_bits(len) else {
                continue;
            };
            if bits as u16 == code {
                reader.read_bits(len)?;
                if index == 0 {
                    return Some(0);
                }
                let val_base = if index % 2 != 0 {
                    (index as i16 + 1) / 2
                } else {
                    -(index as i16 / 2)
                };
                let r_size = f_code.saturating_sub(1);
                if r_size > 0 {
                    let residual = reader.read_bits(r_size)? as i16;
                    let abs_base = val_base.abs();
                    let new_abs = ((abs_base - 1) << r_size) + residual + 1;
                    return Some(if val_base < 0 { -new_abs } else { new_abs });
                }
                return Some(val_base);
            }
        }
        None
    }

    /// 获取预测 MV (支持 block_k 参数用于 Inter4V)
    pub(super) fn get_pmv(&self, mb_x: u32, mb_y: u32, block_k: usize) -> MotionVector {
        let get_mv = |x: i32, y: i32, k: usize| -> MotionVector {
            if x < 0 || y < 0 || x >= self.mb_stride as i32 || y as u32 >= self.height.div_ceil(16)
            {
                return MotionVector { x: 0, y: 0 };
            }
            if let Some(mvs) = self.mv_cache.get(y as usize * self.mb_stride + x as usize) {
                mvs[k]
            } else {
                MotionVector { x: 0, y: 0 }
            }
        };

        let (mv_a, mv_b, mv_c) = if block_k == 0 || block_k > 3 {
            let a = get_mv(mb_x as i32 - 1, mb_y as i32, 0);
            let b = get_mv(mb_x as i32, mb_y as i32 - 1, 0);
            let c = get_mv(mb_x as i32 + 1, mb_y as i32 - 1, 0);
            (a, b, c)
        } else {
            match block_k {
                0 => {
                    // ✅ 修复 C3: Block 0 (左上) 的邻居正确取值
                    // Block 0邻居: [prev_block, top_block, diag_block]
                    let a = get_mv(mb_x as i32 - 1, mb_y as i32, 1); // 左MB的block 1
                    let b = get_mv(mb_x as i32, mb_y as i32 - 1, 2); // 上MB的block 2
                    let c = get_mv(mb_x as i32 - 1, mb_y as i32 - 1, 3); // ✅ 修复: 左上MB的block 3 (不是右上!)
                    (a, b, c)
                }
                1 => {
                    let a = get_mv(mb_x as i32, mb_y as i32, 0);
                    let b = get_mv(mb_x as i32, mb_y as i32 - 1, 3);
                    let c = get_mv(mb_x as i32 + 1, mb_y as i32 - 1, 2);
                    (a, b, c)
                }
                2 => {
                    let a = get_mv(mb_x as i32 - 1, mb_y as i32, 3);
                    let b = get_mv(mb_x as i32, mb_y as i32, 0);
                    let c = get_mv(mb_x as i32, mb_y as i32, 1);
                    (a, b, c)
                }
                3 => {
                    let a = get_mv(mb_x as i32, mb_y as i32, 2);
                    let b = get_mv(mb_x as i32, mb_y as i32, 0);
                    let c = get_mv(mb_x as i32, mb_y as i32, 1);
                    (a, b, c)
                }
                _ => (
                    MotionVector::default(),
                    MotionVector::default(),
                    MotionVector::default(),
                ),
            }
        };

        MotionVector {
            x: Self::median(mv_a.x, mv_b.x, mv_c.x),
            y: Self::median(mv_a.y, mv_b.y, mv_c.y),
        }
    }

    /// 解码完整 MV (预测 + 差分 + 范围包装)
    pub(super) fn decode_motion_vector(
        &self,
        reader: &mut BitReader,
        mb_x: u32,
        mb_y: u32,
        block_k: usize,
    ) -> Option<MotionVector> {
        let pred = self.get_pmv(mb_x, mb_y, block_k);
        let mvd_x = Self::decode_mv_component(reader, self.f_code_forward)?;
        let mvd_y = Self::decode_mv_component(reader, self.f_code_forward)?;

        let mut mv_x = pred.x + mvd_x;
        let mut mv_y = pred.y + mvd_y;

        // MV 范围包装
        let scale_fac = 1i16 << (self.f_code_forward.saturating_sub(1));
        let high = 32 * scale_fac - 1;
        let low = -32 * scale_fac;
        let range = 64 * scale_fac;

        if mv_x < low {
            mv_x += range;
        } else if mv_x > high {
            mv_x -= range;
        }
        if mv_y < low {
            mv_y += range;
        } else if mv_y > high {
            mv_y -= range;
        }

        Some(MotionVector { x: mv_x, y: mv_y })
    }

    /// 从参考帧获取一个像素 (含边缘扩展)
    pub(super) fn get_ref_pixel(ref_frame: &VideoFrame, plane: usize, x: isize, y: isize) -> u8 {
        let width = ref_frame.linesize[plane] as isize;
        let height = if plane == 0 {
            ref_frame.height as isize
        } else {
            (ref_frame.height / 2) as isize
        };
        let cx = x.clamp(0, width - 1) as usize;
        let cy = y.clamp(0, height - 1) as usize;
        ref_frame.data[plane][cy * width as usize + cx]
    }

    /// 运动补偿: 半像素精度
    pub(super) fn motion_compensation(
        ref_frame: &VideoFrame,
        plane: usize,
        base_x: isize,
        base_y: isize,
        mv_x: i16,
        mv_y: i16,
        rounding: u8,
    ) -> u8 {
        let full_x = (mv_x >> 1) as isize;
        let full_y = (mv_y >> 1) as isize;
        let half_x = (mv_x & 1) != 0;
        let half_y = (mv_y & 1) != 0;

        let sx = base_x + full_x;
        let sy = base_y + full_y;

        if !half_x && !half_y {
            Self::get_ref_pixel(ref_frame, plane, sx, sy)
        } else {
            let p00 = Self::get_ref_pixel(ref_frame, plane, sx, sy) as u16;
            let p01 = Self::get_ref_pixel(ref_frame, plane, sx + 1, sy) as u16;
            let p10 = Self::get_ref_pixel(ref_frame, plane, sx, sy + 1) as u16;
            let p11 = Self::get_ref_pixel(ref_frame, plane, sx + 1, sy + 1) as u16;
            let r = rounding as u16;

            if half_x && !half_y {
                ((p00 + p01 + 1 - r) >> 1) as u8
            } else if !half_x && half_y {
                ((p00 + p10 + 1 - r) >> 1) as u8
            } else {
                ((p00 + p01 + p10 + p11 + 2 - r) >> 2) as u8
            }
        }
    }

    /// Chroma MV 推导 (1MV 模式)
    pub(super) fn chroma_mv_1mv(luma_mv: MotionVector) -> MotionVector {
        MotionVector {
            x: (luma_mv.x >> 1) + ROUNDTAB_79[(luma_mv.x & 3) as usize],
            y: (luma_mv.y >> 1) + ROUNDTAB_79[(luma_mv.y & 3) as usize],
        }
    }

    /// Chroma MV 推导 (4MV 模式)
    pub(super) fn chroma_mv_4mv(mvs: &[MotionVector; 4]) -> MotionVector {
        let sum_x = mvs[0].x as i32 + mvs[1].x as i32 + mvs[2].x as i32 + mvs[3].x as i32;
        let sum_y = mvs[0].y as i32 + mvs[1].y as i32 + mvs[2].y as i32 + mvs[3].y as i32;
        MotionVector {
            x: (sum_x >> 3) as i16 + ROUNDTAB_76[(sum_x & 0xf) as usize],
            y: (sum_y >> 3) as i16 + ROUNDTAB_76[(sum_y & 0xf) as usize],
        }
    }

    /// MV 合法性验证
    pub(super) fn validate_vector(&self, mv: &mut MotionVector, mb_x: u32, mb_y: u32) {
        let shift = 5;
        let x_high = ((self.mb_stride as i16 - mb_x as i16) << shift) - 1;
        let x_low = -((mb_x as i16 + 1) << shift);
        let mb_h = self.height.div_ceil(16) as i16;
        let y_high = ((mb_h - mb_y as i16) << shift) - 1;
        let y_low = -((mb_y as i16 + 1) << shift);

        mv.x = mv.x.clamp(x_low, x_high);
        mv.y = mv.y.clamp(y_low, y_high);
    }

    // ========================================================================
    // Quarter-Pixel 运动补偿 (6-tap FIR 滤波器)
    // ========================================================================

    /// 水平 6-tap FIR 半像素滤波器: h = [-1, 5, 20, 20, 5, -1] / 32
    fn qpel_h_filter(ref_frame: &VideoFrame, plane: usize, x: isize, y: isize) -> i32 {
        let get = |xx: isize| -> i32 { Self::get_ref_pixel(ref_frame, plane, xx, y) as i32 };
        let val = -get(x - 2) + 5 * get(x - 1) + 20 * get(x) + 20 * get(x + 1) + 5 * get(x + 2)
            - get(x + 3);
        ((val + 16) >> 5).clamp(0, 255)
    }

    /// 垂直 6-tap FIR 半像素滤波器
    fn qpel_v_filter(ref_frame: &VideoFrame, plane: usize, x: isize, y: isize) -> i32 {
        let get = |yy: isize| -> i32 { Self::get_ref_pixel(ref_frame, plane, x, yy) as i32 };
        let val = -get(y - 2) + 5 * get(y - 1) + 20 * get(y) + 20 * get(y + 1) + 5 * get(y + 2)
            - get(y + 3);
        ((val + 16) >> 5).clamp(0, 255)
    }

    /// 对角线 6-tap FIR 半像素滤波器 (先水平后垂直)
    fn qpel_hv_filter(ref_frame: &VideoFrame, plane: usize, x: isize, y: isize) -> i32 {
        let h_row = |yy: isize| -> i32 {
            let get = |xx: isize| -> i32 { Self::get_ref_pixel(ref_frame, plane, xx, yy) as i32 };
            -get(x - 2) + 5 * get(x - 1) + 20 * get(x) + 20 * get(x + 1) + 5 * get(x + 2)
                - get(x + 3)
        };
        let val =
            -h_row(y - 2) + 5 * h_row(y - 1) + 20 * h_row(y) + 20 * h_row(y + 1) + 5 * h_row(y + 2)
                - h_row(y + 3);
        ((val + 512) >> 10).clamp(0, 255)
    }

    /// Quarter-Pixel 运动补偿 (单像素)
    ///
    /// MV 使用四分之一像素精度: MV=4 表示偏移 1 个整像素.
    /// 对 16 种 (dx, dy) 组合使用 6-tap FIR 和双线性插值.
    pub(super) fn qpel_motion_compensation(
        ref_frame: &VideoFrame,
        plane: usize,
        base_x: isize,
        base_y: isize,
        mv_x: i16,
        mv_y: i16,
    ) -> u8 {
        let ix = (mv_x >> 2) as isize;
        let iy = (mv_y >> 2) as isize;
        let dx = ((mv_x & 3) + 4) as usize % 4;
        let dy = ((mv_y & 3) + 4) as usize % 4;

        let sx = base_x + ix;
        let sy = base_y + iy;

        let f = |ox: isize, oy: isize| -> i32 {
            Self::get_ref_pixel(ref_frame, plane, sx + ox, sy + oy) as i32
        };
        let h = |ox: isize, oy: isize| -> i32 {
            Self::qpel_h_filter(ref_frame, plane, sx + ox, sy + oy)
        };
        let v = |ox: isize, oy: isize| -> i32 {
            Self::qpel_v_filter(ref_frame, plane, sx + ox, sy + oy)
        };
        let hv = |ox: isize, oy: isize| -> i32 {
            Self::qpel_hv_filter(ref_frame, plane, sx + ox, sy + oy)
        };

        let avg = |a: i32, b: i32| -> i32 { (a + b + 1) >> 1 };

        let result = match (dx, dy) {
            (0, 0) => f(0, 0),
            (1, 0) => avg(f(0, 0), h(0, 0)),
            (2, 0) => h(0, 0),
            (3, 0) => avg(h(0, 0), f(1, 0)),
            (0, 1) => avg(f(0, 0), v(0, 0)),
            (0, 2) => v(0, 0),
            (0, 3) => avg(v(0, 0), f(0, 1)),
            (2, 2) => hv(0, 0),
            (1, 1) => avg(f(0, 0), hv(0, 0)),
            (3, 1) => avg(f(1, 0), hv(0, 0)),
            (1, 3) => avg(f(0, 1), hv(0, 0)),
            (3, 3) => avg(f(1, 1), hv(0, 0)),
            (2, 1) => avg(h(0, 0), hv(0, 0)),
            (2, 3) => avg(hv(0, 0), h(0, 1)),
            (1, 2) => avg(v(0, 0), hv(0, 0)),
            (3, 2) => avg(hv(0, 0), v(1, 0)),
            _ => f(0, 0),
        };

        result.clamp(0, 255) as u8
    }

    /// 通用运动补偿入口 (根据 quarterpel 标志选择半像素或四分之一像素 MC)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn motion_compensate(
        ref_frame: &VideoFrame,
        plane: usize,
        base_x: isize,
        base_y: isize,
        mv_x: i16,
        mv_y: i16,
        rounding: u8,
        quarterpel: bool,
    ) -> u8 {
        if quarterpel {
            Self::qpel_motion_compensation(ref_frame, plane, base_x, base_y, mv_x, mv_y)
        } else {
            Self::motion_compensation(ref_frame, plane, base_x, base_y, mv_x, mv_y, rounding)
        }
    }
}
