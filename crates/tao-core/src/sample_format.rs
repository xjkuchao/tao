//! 音频采样格式定义.
//!
//! 对标 FFmpeg 的 `AVSampleFormat`.

use std::fmt;

/// 音频采样格式
///
/// 定义了单个音频采样点的数据类型和排列方式.
/// - 交错 (Interleaved): 所有声道的采样点交替排列, 如 LRLRLR...
/// - 平面 (Planar): 每个声道独立存储, 如 LLL...RRR...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SampleFormat {
    /// 未指定
    None,

    // ========================
    // 交错格式 (Interleaved)
    // ========================
    /// 无符号 8 位整数, 交错
    U8,
    /// 有符号 16 位整数, 交错
    S16,
    /// 有符号 32 位整数, 交错
    S32,
    /// 32 位浮点, 交错
    F32,
    /// 64 位浮点, 交错
    F64,

    // ========================
    // 平面格式 (Planar)
    // ========================
    /// 无符号 8 位整数, 平面
    U8p,
    /// 有符号 16 位整数, 平面
    S16p,
    /// 有符号 32 位整数, 平面
    S32p,
    /// 32 位浮点, 平面
    F32p,
    /// 64 位浮点, 平面
    F64p,
}

impl SampleFormat {
    /// 每个采样点占用的字节数
    pub const fn bytes_per_sample(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::U8 | Self::U8p => 1,
            Self::S16 | Self::S16p => 2,
            Self::S32 | Self::S32p | Self::F32 | Self::F32p => 4,
            Self::F64 | Self::F64p => 8,
        }
    }

    /// 是否为平面格式
    pub const fn is_planar(&self) -> bool {
        matches!(
            self,
            Self::U8p | Self::S16p | Self::S32p | Self::F32p | Self::F64p
        )
    }

    /// 获取对应的平面格式
    pub const fn to_planar(&self) -> Self {
        match self {
            Self::U8 => Self::U8p,
            Self::S16 => Self::S16p,
            Self::S32 => Self::S32p,
            Self::F32 => Self::F32p,
            Self::F64 => Self::F64p,
            other => *other,
        }
    }

    /// 获取对应的交错格式
    pub const fn to_interleaved(&self) -> Self {
        match self {
            Self::U8p => Self::U8,
            Self::S16p => Self::S16,
            Self::S32p => Self::S32,
            Self::F32p => Self::F32,
            Self::F64p => Self::F64,
            other => *other,
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::None => "none",
            Self::U8 => "u8",
            Self::S16 => "s16",
            Self::S32 => "s32",
            Self::F32 => "flt",
            Self::F64 => "dbl",
            Self::U8p => "u8p",
            Self::S16p => "s16p",
            Self::S32p => "s32p",
            Self::F32p => "fltp",
            Self::F64p => "dblp",
        };
        write!(f, "{name}")
    }
}
