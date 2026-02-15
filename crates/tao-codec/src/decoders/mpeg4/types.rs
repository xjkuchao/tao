//! MPEG-4 Part 2 解码器类型定义

use crate::frame::PictureType;

/// 宏块类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MbType {
    Intra,
    IntraQ,
    Inter,
    InterQ,
    Inter4V,
}

/// 运动向量
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::decoders) struct MotionVector {
    pub x: i16,
    pub y: i16,
}

/// DC/AC 预测方向
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::decoders) enum PredictorDirection {
    #[allow(dead_code)]
    None,
    Horizontal,
    Vertical,
}

/// VOL (Video Object Layer) 信息
#[derive(Debug, Clone)]
pub(in crate::decoders) struct VolInfo {
    pub vop_time_increment_resolution: u16,
    #[allow(dead_code)]
    pub fixed_vop_rate: bool,
    #[allow(dead_code)]
    pub data_partitioned: bool,
    /// 量化类型: 0=H.263, 1=MPEG
    pub quant_type: u8,
    /// 是否支持隔行扫描
    pub interlacing: bool,
    /// 是否启用 quarter-pixel
    #[allow(dead_code)]
    pub quarterpel: bool,
    /// sprite 使能 (0=无, 1=static, 2=GMC)
    #[allow(dead_code)]
    pub sprite_enable: u8,
    /// sprite warping 点数
    #[allow(dead_code)]
    pub sprite_warping_points: u8,
    /// 是否禁用 resync marker
    #[allow(dead_code)]
    pub resync_marker_disable: bool,
}

/// VOP (Video Object Plane) 信息
#[derive(Debug)]
pub(super) struct VopInfo {
    pub picture_type: PictureType,
    pub vop_coded: bool,
    #[allow(dead_code)]
    pub vop_rounding_type: u8,
    #[allow(dead_code)]
    pub intra_dc_vlc_thr: u32,
}
