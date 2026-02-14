//! 色彩范围定义.
//!
//! 对标 FFmpeg 的 `AVColorRange`.

/// 色彩范围
///
/// 决定像素值的有效范围:
/// - Limited: Y 16-235, Cb/Cr 16-240 (8 位) - 广播标准
/// - Full: Y/Cb/Cr 0-255 (8 位) - JPEG/PC 标准
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorRange {
    /// 未指定
    #[default]
    Unspecified,
    /// 有限范围 (广播/TV) Y 16-235
    Limited,
    /// 完整范围 (JPEG/PC) Y 0-255
    Full,
}
