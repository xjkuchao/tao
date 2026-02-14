//! # Tao (道)
//!
//! 纯 Rust 实现的多媒体处理框架, 对标 FFmpeg.
//!
//! Tao 提供了完整的音视频处理能力:
//! - **编解码**: 视频 (H.264, H.265, VP9, AV1...) 与音频 (AAC, MP3, FLAC, Opus...)
//! - **容器格式**: 封装/解封装 (MP4, MKV, AVI, FLV, TS, WAV...)
//! - **滤镜**: 音视频变换处理
//! - **图像缩放**: 分辨率与像素格式转换
//! - **音频重采样**: 采样率、声道、格式转换
//!
//! # 快速开始
//!
//! ```rust,no_run
//! use tao::core::{PixelFormat, SampleFormat, Rational};
//!
//! // 定义一个 29.97fps 的帧率
//! let frame_rate = Rational::new(30000, 1001);
//! println!("帧率: {frame_rate} ({:.2} fps)", frame_rate.to_f64());
//! ```
//!
//! # Crate 结构
//!
//! | Crate | 功能 |
//! |-------|------|
//! | `tao-core` | 核心类型与工具 |
//! | `tao-codec` | 编解码器框架 |
//! | `tao-format` | 容器格式框架 |
//! | `tao-filter` | 滤镜框架 |
//! | `tao-scale` | 图像缩放 |
//! | `tao-resample` | 音频重采样 |

/// 核心类型与工具 (对标 libavutil)
pub use tao_core as core;

/// 编解码器框架 (对标 libavcodec)
pub use tao_codec as codec;

/// 容器格式框架 (对标 libavformat)
pub use tao_format as format;

/// 滤镜框架 (对标 libavfilter)
pub use tao_filter as filter;

/// 图像缩放与像素格式转换 (对标 libswscale)
pub use tao_scale as scale;

/// 音频重采样 (对标 libswresample)
pub use tao_resample as resample;

/// 获取 Tao 版本号
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 创建已注册所有内置编解码器的注册表
pub fn default_codec_registry() -> tao_codec::CodecRegistry {
    let mut registry = tao_codec::CodecRegistry::new();
    tao_codec::register_all(&mut registry);
    registry
}

/// 创建已注册所有内置容器格式的注册表
pub fn default_format_registry() -> tao_format::FormatRegistry {
    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    registry
}
