//! 色彩空间 (YCbCr 矩阵系数).
//!
//! 对标 FFmpeg 的 `AVColorSpace`.

/// YCbCr 色彩空间 (矩阵系数)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorSpace {
    /// 未指定
    #[default]
    Unspecified,
    /// ITU-R BT.709 (高清)
    Bt709,
    /// ITU-R BT.470 BG (PAL/SECAM)
    Bt470bg,
    /// SMPTE 170M (NTSC)
    Smpte170m,
    /// SMPTE 240M
    Smpte240m,
    /// ITU-R BT.2020 非恒定亮度
    Bt2020Ncl,
    /// ITU-R BT.2020 恒定亮度
    Bt2020Cl,
    /// sRGB / IEC 61966-2-1
    Rgb,
}
