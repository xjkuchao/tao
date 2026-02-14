//! 色彩传递特性 (Transfer Characteristics / EOTF).
//!
//! 对标 FFmpeg 的 `AVColorTransferCharacteristic`.

/// 色彩传递特性 (伽马/EOTF)
///
/// 定义了线性光和编码值之间的映射关系 (即"伽马曲线").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorTransfer {
    /// 未指定
    #[default]
    Unspecified,
    /// ITU-R BT.709 (高清标准伽马)
    Bt709,
    /// ITU-R BT.470 M (伽马 2.2)
    Gamma22,
    /// ITU-R BT.470 BG (伽马 2.8)
    Gamma28,
    /// SMPTE 170M
    Smpte170m,
    /// SMPTE 240M
    Smpte240m,
    /// 线性传递 (无伽马)
    Linear,
    /// IEC 61966-2-1 (sRGB)
    Srgb,
    /// ITU-R BT.2020 10 位
    Bt202010bit,
    /// ITU-R BT.2020 12 位
    Bt202012bit,
    /// SMPTE ST 2084 (PQ / HDR10)
    SmpteSt2084,
    /// ARIB STD-B67 (HLG / 混合对数伽马)
    AribStdB67,
}
