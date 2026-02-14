//! # Tao - 纯 Rust 多媒体框架
//!
//! Tao 是一个对标 FFmpeg 的纯 Rust 多媒体框架, 提供:
//!
//! - **tao-core**: 核心数据类型和工具 (像素格式, 采样格式, 颜色空间等)
//! - **tao-codec**: 编解码器 (PCM, FLAC, AAC, MP3, RawVideo, H.264 解析器)
//! - **tao-format**: 容器格式 (WAV, FLAC, MP4, MKV, AVI, FLV, TS, Ogg, AIFF, ADTS, MP3)
//! - **tao-filter**: 音视频滤镜 (volume, crop, pad, fade, overlay, drawtext, loudnorm, equalizer)
//! - **tao-scale**: 图像缩放和像素格式转换 (最近邻, 双线性, 双三次, Lanczos, Area)
//! - **tao-resample**: 音频重采样和格式转换
//! - **tao-ffi**: C FFI 导出层
//!
//! ## 使用示例
//!
//! ```rust,no_run
//! use tao::codec::CodecRegistry;
//! use tao::format::FormatRegistry;
//!
//! // 初始化注册表
//! let mut codec_reg = CodecRegistry::new();
//! tao::codec::register_all(&mut codec_reg);
//!
//! let mut format_reg = FormatRegistry::new();
//! tao::format::register_all(&mut format_reg);
//! ```

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
