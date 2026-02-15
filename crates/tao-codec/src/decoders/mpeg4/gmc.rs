//! GMC (全局运动补偿 / S-VOP) 支持
//!
//! 实现 sprite_trajectory VLC 解码和基于仿射变换的全局运动预测.
//! 当前支持:
//! - 0 个 warping 点: 静止场景 (等价于零 MV)
//! - 1 个 warping 点: 平移模式 (全局平移 MV)
//! - 2/3 个 warping 点: 仿射/透视变换 (基本框架)

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
    #[allow(dead_code)]
    pub num_points: u8,
    /// GMC 是否激活
    #[allow(dead_code)]
    pub active: bool,
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

        // 1-point GMC: 转换为全局 MV
        if num_warping_points >= 1 {
            let (dx, dy) = params.warping_points[0];
            params.global_mv = MotionVector {
                x: dx as i16,
                y: dy as i16,
            };
        }

        debug!(
            "GMC: {} warping points, global_mv=({}, {})",
            num_warping_points, params.global_mv.x, params.global_mv.y
        );

        params
    }

    /// GMC 运动补偿 (单像素)
    ///
    /// 对于 1-point GMC (平移), 等价于使用全局 MV 的运动补偿.
    /// 对于 2/3-point GMC (仿射/透视), 使用简化的平移近似.
    #[allow(dead_code)]
    pub(super) fn gmc_motion_compensation(
        ref_frame: &VideoFrame,
        plane: usize,
        px: isize,
        py: isize,
        gmc_params: &GmcParameters,
    ) -> u8 {
        // 使用全局 MV 做运动补偿 (1-point 精确, 2/3-point 近似)
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
