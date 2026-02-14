//! 音频声道布局定义.
//!
//! 对标 FFmpeg 的 `AVChannelLayout` / `AV_CH_LAYOUT_*`.

use bitflags::bitflags;
use std::fmt;

bitflags! {
    /// 声道位掩码, 每个位代表一个扬声器位置
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ChannelMask: u64 {
        /// 前方左声道
        const FRONT_LEFT            = 1 << 0;
        /// 前方右声道
        const FRONT_RIGHT           = 1 << 1;
        /// 前方中央声道
        const FRONT_CENTER          = 1 << 2;
        /// 低频效果 (LFE / 重低音)
        const LOW_FREQUENCY         = 1 << 3;
        /// 后方左声道
        const BACK_LEFT             = 1 << 4;
        /// 后方右声道
        const BACK_RIGHT            = 1 << 5;
        /// 前方中左声道
        const FRONT_LEFT_OF_CENTER  = 1 << 6;
        /// 前方中右声道
        const FRONT_RIGHT_OF_CENTER = 1 << 7;
        /// 后方中央声道
        const BACK_CENTER           = 1 << 8;
        /// 侧方左声道
        const SIDE_LEFT             = 1 << 9;
        /// 侧方右声道
        const SIDE_RIGHT            = 1 << 10;
        /// 顶部中央
        const TOP_CENTER            = 1 << 11;
        /// 顶部前左
        const TOP_FRONT_LEFT        = 1 << 12;
        /// 顶部前中
        const TOP_FRONT_CENTER      = 1 << 13;
        /// 顶部前右
        const TOP_FRONT_RIGHT       = 1 << 14;
        /// 顶部后左
        const TOP_BACK_LEFT         = 1 << 15;
        /// 顶部后中
        const TOP_BACK_CENTER       = 1 << 16;
        /// 顶部后右
        const TOP_BACK_RIGHT        = 1 << 17;
    }
}

/// 声道布局
///
/// 描述音频流中声道的数量和排列方式.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelLayout {
    /// 声道数量
    pub channels: u32,
    /// 声道位掩码 (标准布局使用)
    pub mask: ChannelMask,
}

impl ChannelLayout {
    /// 单声道
    pub const MONO: Self = Self {
        channels: 1,
        mask: ChannelMask::FRONT_CENTER,
    };

    /// 立体声 (左右)
    pub const STEREO: Self = Self {
        channels: 2,
        mask: ChannelMask::FRONT_LEFT.union(ChannelMask::FRONT_RIGHT),
    };

    /// 5.1 环绕声
    pub const SURROUND_5_1: Self = Self {
        channels: 6,
        mask: ChannelMask::FRONT_LEFT
            .union(ChannelMask::FRONT_RIGHT)
            .union(ChannelMask::FRONT_CENTER)
            .union(ChannelMask::LOW_FREQUENCY)
            .union(ChannelMask::BACK_LEFT)
            .union(ChannelMask::BACK_RIGHT),
    };

    /// 7.1 环绕声
    pub const SURROUND_7_1: Self = Self {
        channels: 8,
        mask: ChannelMask::FRONT_LEFT
            .union(ChannelMask::FRONT_RIGHT)
            .union(ChannelMask::FRONT_CENTER)
            .union(ChannelMask::LOW_FREQUENCY)
            .union(ChannelMask::BACK_LEFT)
            .union(ChannelMask::BACK_RIGHT)
            .union(ChannelMask::SIDE_LEFT)
            .union(ChannelMask::SIDE_RIGHT),
    };

    /// 根据声道数创建默认布局
    pub fn from_channels(channels: u32) -> Self {
        match channels {
            1 => Self::MONO,
            2 => Self::STEREO,
            6 => Self::SURROUND_5_1,
            8 => Self::SURROUND_7_1,
            n => Self {
                channels: n,
                mask: ChannelMask::empty(),
            },
        }
    }
}

impl fmt::Display for ChannelLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MONO => write!(f, "mono"),
            Self::STEREO => write!(f, "stereo"),
            Self::SURROUND_5_1 => write!(f, "5.1"),
            Self::SURROUND_7_1 => write!(f, "7.1"),
            _ => write!(f, "{}ch", self.channels),
        }
    }
}
