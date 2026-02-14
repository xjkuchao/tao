//! 色彩相关类型定义.
//!
//! 对标 FFmpeg 的色彩空间、色彩范围、色彩原色等定义.

mod color_primaries;
mod color_range;
mod color_space;
mod color_transfer;

pub use color_primaries::ColorPrimaries;
pub use color_range::ColorRange;
pub use color_space::ColorSpace;
pub use color_transfer::ColorTransfer;
