//! 色彩原色定义.
//!
//! 对标 FFmpeg 的 `AVColorPrimaries`.

/// 色彩原色 (色域)
///
/// 定义了 RGB 三原色在 CIE 色度图中的坐标, 决定了颜色的物理范围.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorPrimaries {
    /// 未指定
    #[default]
    Unspecified,
    /// ITU-R BT.709 (sRGB, 高清)
    Bt709,
    /// ITU-R BT.470 M
    Bt470m,
    /// ITU-R BT.470 BG (PAL)
    Bt470bg,
    /// SMPTE 170M (NTSC)
    Smpte170m,
    /// SMPTE 240M
    Smpte240m,
    /// Generic Film
    Film,
    /// ITU-R BT.2020 (超高清/HDR)
    Bt2020,
    /// DCI-P3 (电影院)
    SmpteP3d65,
}
