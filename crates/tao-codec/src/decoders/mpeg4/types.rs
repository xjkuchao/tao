//! MPEG-4 Part 2 解码器类型定义

use crate::frame::PictureType;

/// 宏块类型 (I/P-VOP)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MbType {
    Intra,
    IntraQ,
    Inter,
    InterQ,
    Inter4V,
}

/// B 帧宏块模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BframeMbMode {
    /// 直接模式: MV 从共定位 P 帧 MV 按 TRB/TRD 缩放
    Direct,
    /// 前向预测: 使用前向参考帧
    Forward,
    /// 后向预测: 使用后向参考帧
    Backward,
    /// 双向插值: 使用两个参考帧的平均
    Interpolate,
    /// 直接模式无 MV (not_coded 等价)
    DirectNoneMv,
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

/// 宏块信息 (用于参考帧存储和 B 帧 Direct 模式)
#[derive(Debug, Clone, Copy)]
pub(in crate::decoders) struct MacroblockInfo {
    /// 宏块模式: 0=Inter, 1=Intra, 2=Inter4V, 3=InterQ, 4=IntraQ
    #[allow(dead_code)]
    pub mode: u8,
    /// 宏块量化参数
    #[allow(dead_code)]
    pub quant: u8,
    /// 4 个 MV (1MV 模式时 [0] 复制到全部)
    #[allow(dead_code)]
    pub mvs: [MotionVector; 4],
}

impl Default for MacroblockInfo {
    fn default() -> Self {
        Self {
            mode: 0,
            quant: 1,
            mvs: [MotionVector::default(); 4],
        }
    }
}

impl MacroblockInfo {
    /// 模式编码常量
    pub const MODE_INTER: u8 = 0;
    pub const MODE_INTRA: u8 = 1;
    pub const MODE_INTER4V: u8 = 2;
    #[allow(dead_code)]
    pub const MODE_INTERQ: u8 = 3;
    #[allow(dead_code)]
    pub const MODE_INTRAQ: u8 = 4;
    pub const MODE_NOT_CODED: u8 = 5;
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
    pub quarterpel: bool,
    /// sprite 使能 (0=无, 1=static, 2=GMC)
    #[allow(dead_code)]
    pub sprite_enable: u8,
    /// sprite warping 点数
    #[allow(dead_code)]
    pub sprite_warping_points: u8,
    /// 是否禁用 resync marker
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
