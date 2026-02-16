//! GMC (全局运动补偿 / S-VOP) 支持
//!
//! 实现 sprite_trajectory VLC 解码和基于仿射变换的全局运动预测.
//! 当前支持:
//! - 0 个 warping 点: 静止场景 (等价于零 MV)
//! - 1 个 warping 点: 平移模式 (全局平移 MV)
//! - 2 个 warping 点: 仿射变换 (平移+旋转+缩放, 4自由度)
//! - 3 个 warping 点: 透视变换 (完整 6 自由度)

use log::debug;

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::types::MotionVector;
use crate::frame::VideoFrame;

/// GMC 变换参数
#[derive(Debug, Clone, Default)]
pub(in crate::decoders) struct GmcParameters {
    /// 全局 MV (1-point GMC 平移模式)
    pub global_mv: MotionVector,
    /// warping 点位移 (最多 3 个)
    pub warping_points: [(i32, i32); 3],
    /// 实际使用的 warping 点数
    pub num_points: u8,
    /// GMC 是否激活
    #[allow(dead_code)]
    pub active: bool,
    /// 仿射变换矩阵 (2/3 点 GMC 使用)
    /// [a, b, c, d, e, f] 表示变换: x' = (a*x + b*y + c) / den, y' = (d*x + e*y + f) / den
    pub transform: [i32; 6],
    /// 变换分母 (定点数缩放因子)
    pub transform_den: i32,
}

impl Mpeg4Decoder {
    /// 解码 sprite trajectory (warping 点位移)
    ///
    /// 每个分量使用 "长度 + 数据" 的 VLC 编码:
    /// - 长度码: 连续 '0' 后跟 '1' (计数 '0' 的个数)
    /// - 数据: 长度码个 bits, MSB=1 为正, MSB=0 为负
    pub(super) fn parse_sprite_trajectory(&mut self, reader: &mut BitReader) -> GmcParameters {
        let num_warping_points = self
            .vol_info
            .as_ref()
            .map(|v| v.sprite_warping_points)
            .unwrap_or(0);

        let mut params = GmcParameters {
            num_points: num_warping_points,
            active: true,
            ..Default::default()
        };

        // 读取 warping 点位移
        for i in 0..num_warping_points as usize {
            if i >= 3 {
                break;
            }
            let dx = decode_sprite_trajectory_component(reader).unwrap_or(0);
            reader.read_bit(); // marker bit
            let dy = decode_sprite_trajectory_component(reader).unwrap_or(0);
            reader.read_bit(); // marker bit
            params.warping_points[i] = (dx, dy);
        }

        // 根据 warping 点数计算变换矩阵
        match num_warping_points {
            0 => {
                // 静止 sprite, 无变换
                params.global_mv = MotionVector { x: 0, y: 0 };
            }
            1 => {
                // 1-point GMC: 纯平移
                let (dx, dy) = params.warping_points[0];
                params.global_mv = MotionVector {
                    x: dx as i16,
                    y: dy as i16,
                };
                debug!("GMC 1-point: 平移 MV=({}, {})", dx, dy);
            }
            2 | 3 => {
                // 2/3-point GMC: 仿射/透视变换
                self.compute_sprite_transform(&mut params);
            }
            _ => {}
        }

        params
    }

    /// 计算 sprite 仿射/透视变换矩阵 (2/3 点 GMC)
    ///
    /// 基于 warping 点和 sprite 参考点推导变换参数.
    /// 参考: ISO/IEC 14496-2:2004 Section 7.5.1.4
    fn compute_sprite_transform(&self, params: &mut GmcParameters) {
        let width = self.width as i32;
        let height = self.height as i32;
        let num_points = params.num_points as usize;

        // sprite 参考点 (虚拟坐标)
        // 标准定义: 左上 (0,0), 右上 (w-1,0), 左下 (0,h-1)
        let virt_ref: &[(i32, i32)] = &[(0, 0), (width - 1, 0), (0, height - 1)];

        // 实际 warping 点 = 参考点 + 位移
        let mut real: [(i32, i32); 3] = [(0, 0); 3];
        for i in 0..num_points.min(3) {
            let (dx, dy) = params.warping_points[i];
            real[i] = (virt_ref[i].0 + dx, virt_ref[i].1 + dy);
        }

        // 使用定点数计算 (16 位小数精度)
        let shift = 16;
        let den = 1i32 << shift;

        if num_points == 2 {
            // 2-point GMC: 仿射变换 (4 自由度: 平移 + 旋转 + 等比缩放)
            // 使用前 2 个点推导变换矩阵

            // 计算第一个点的平移分量
            let (dx0, dy0) = params.warping_points[0];
            let (dx1, dy1) = params.warping_points[1];

            // 简化的 2-point 仿射变换:
            // x' = x + (dx0 * (w-x) + dx1 * x) / w
            // y' = y + (dy0 * (h-y) + dy1 * y) / h
            // 转换为标准仿射形式: x' = a*x + b*y + c

            let wx = width - 1;

            // x 方向系数: a = 1 + (dx1 - dx0) / wx
            params.transform[0] = den + ((dx1 - dx0) * den / wx.max(1));
            // x 方向无 y 分量: b = 0
            params.transform[1] = 0;
            // x 方向偏移: c = dx0
            params.transform[2] = dx0 * den;

            // y 方向系数: d = 0
            params.transform[3] = 0;
            // y 方向 y 系数: e = 1 + (dy1 - dy0) / wx
            params.transform[4] = den + ((dy1 - dy0) * den / wx.max(1));
            // y 方向偏移: f = dy0
            params.transform[5] = dy0 * den;

            debug!(
                "GMC 2-point 仿射: [{}, {}, {}, {}, {}, {}] / {}",
                params.transform[0],
                params.transform[1],
                params.transform[2],
                params.transform[3],
                params.transform[4],
                params.transform[5],
                den
            );
        } else if num_points == 3 {
            // 3-point GMC: 完整透视变换 (6 自由度)
            // 求解线性方程组得到变换矩阵
            // 基于 3 个对应点: (virt_ref[i] -> real[i])

            // 虚拟参考点坐标
            let (x0, y0) = virt_ref[0];
            let (x1, y1) = virt_ref[1];
            let (x2, y2) = virt_ref[2];

            // 实际 warping 点坐标
            let (rx0, ry0) = real[0];
            let (rx1, ry1) = real[1];
            let (rx2, ry2) = real[2];

            // 构造方程组求解 [a, b, c, d, e, f]
            // rx0 = a*x0 + b*y0 + c, ry0 = d*x0 + e*y0 + f
            // rx1 = a*x1 + b*y1 + c, ry1 = d*x1 + e*y1 + f
            // rx2 = a*x2 + b*y2 + c, ry2 = d*x2 + e*y2 + f

            // 使用 Cramer 法则求解 (简化版)
            let det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if det.abs() < 1 {
                // 退化情况,使用 1-point 平移
                let (dx, dy) = params.warping_points[0];
                params.transform[0] = den;
                params.transform[1] = 0;
                params.transform[2] = dx * den;
                params.transform[3] = 0;
                params.transform[4] = den;
                params.transform[5] = dy * den;
                debug!("GMC 3-point 退化为平移");
            } else {
                // 求解 a, b, c (x 方向)
                params.transform[0] =
                    ((rx1 - rx0) * (y2 - y0) - (rx2 - rx0) * (y1 - y0)) * den / det;
                params.transform[1] =
                    ((x1 - x0) * (rx2 - rx0) - (x2 - x0) * (rx1 - rx0)) * den / det;
                params.transform[2] =
                    rx0 * den - params.transform[0] * x0 - params.transform[1] * y0;

                // 求解 d, e, f (y 方向)
                params.transform[3] =
                    ((ry1 - ry0) * (y2 - y0) - (ry2 - ry0) * (y1 - y0)) * den / det;
                params.transform[4] =
                    ((x1 - x0) * (ry2 - ry0) - (x2 - x0) * (ry1 - ry0)) * den / det;
                params.transform[5] =
                    ry0 * den - params.transform[3] * x0 - params.transform[4] * y0;

                debug!(
                    "GMC 3-point 透视: [{}, {}, {}, {}, {}, {}] / {}",
                    params.transform[0],
                    params.transform[1],
                    params.transform[2],
                    params.transform[3],
                    params.transform[4],
                    params.transform[5],
                    den
                );
            }
        }

        params.transform_den = den;
    }

    /// GMC 运动补偿 (单像素)
    ///
    /// 根据 GMC 类型选择对应的变换:
    /// - 1-point: 使用全局 MV 的简单平移
    /// - 2/3-point: 使用仿射/透视变换矩阵
    #[allow(dead_code)] // 将在宏块解码时使用
    pub(super) fn gmc_motion_compensation(
        ref_frame: &VideoFrame,
        plane: usize,
        px: isize,
        py: isize,
        gmc_params: &GmcParameters,
    ) -> u8 {
        match gmc_params.num_points {
            1 => {
                // 1-point GMC: 简单平移
                Self::motion_compensation(
                    ref_frame,
                    plane,
                    px,
                    py,
                    gmc_params.global_mv.x,
                    gmc_params.global_mv.y,
                    0,
                )
            }
            2 | 3 => {
                // 2/3-point GMC: 仿射/透视变换
                let [a, b, c, d, e, f] = gmc_params.transform;
                let den = gmc_params.transform_den;

                if den == 0 {
                    // 退化情况,返回原始像素
                    return Self::get_ref_pixel(ref_frame, plane, px, py);
                }

                // 应用仿射变换: x' = (a*x + b*y + c) / den
                let x = px as i32;
                let y = py as i32;
                let src_x_fixed = (a * x + b * y + c) / den;
                let src_y_fixed = (d * x + e * y + f) / den;

                // 定点数转整数 (sub-pixel 插值)
                let src_x_int = src_x_fixed >> 16;
                let src_y_int = src_y_fixed >> 16;
                let frac_x = (src_x_fixed & 0xFFFF) as u16;
                let frac_y = (src_y_fixed & 0xFFFF) as u16;

                // 双线性插值
                if frac_x == 0 && frac_y == 0 {
                    // 整数位置,直接采样
                    Self::get_ref_pixel(ref_frame, plane, src_x_int as isize, src_y_int as isize)
                } else {
                    // Sub-pixel 插值 (简化为最近邻)
                    // TODO: 实现完整的双线性插值以提高精度
                    let interp_x = if frac_x >= 0x8000 {
                        src_x_int + 1
                    } else {
                        src_x_int
                    };
                    let interp_y = if frac_y >= 0x8000 {
                        src_y_int + 1
                    } else {
                        src_y_int
                    };
                    Self::get_ref_pixel(ref_frame, plane, interp_x as isize, interp_y as isize)
                }
            }
            _ => {
                // 0 个warping点或其他,返回原像素
                Self::get_ref_pixel(ref_frame, plane, px, py)
            }
        }
    }
}

/// 解码 sprite trajectory 分量
///
/// VLC 编码: 先读长度码 (连续 '0' 计数), 再读对应 bits 数的数据.
fn decode_sprite_trajectory_component(reader: &mut BitReader) -> Option<i32> {
    let mut length: u8 = 0;
    while length < 12 {
        match reader.read_bit()? {
            true => break,
            false => length += 1,
        }
    }

    if length >= 12 {
        return None;
    }

    if length == 0 {
        return Some(0);
    }

    let code = reader.read_bits(length)? as i32;

    // MSB=1 为正, MSB=0 为负 (补码)
    if (code >> (length - 1)) & 1 != 0 {
        Some(code)
    } else {
        Some(code - (1 << length) + 1)
    }
}
