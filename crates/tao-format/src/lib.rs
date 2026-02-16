//! # tao-format
//!
//! Tao 多媒体框架容器格式库, 提供封装/解封装框架.
//!
//! 本 crate 对标 FFmpeg 的 libavformat, 负责处理多媒体容器格式的读写.
//!
//! ## 支持的格式
//!
//! - **解封装 (Demuxer)**: WAV, FLAC, MP4, MKV, AVI, FLV, MPEG-TS, Ogg, AIFF, ADTS, MP3, M4V
//! - **封装 (Muxer)**: WAV, FLAC, MP4, MKV, AVI, FLV, MPEG-TS, Ogg, AIFF, ADTS, MP3
//!
//! ## 使用示例
//!
//! ```rust,no_run
//! use tao_format::{FormatRegistry, IoContext};
//!
//! let mut reg = FormatRegistry::new();
//! tao_format::register_all(&mut reg);
//!
//! // 打开文件并探测格式
//! let mut io = IoContext::open_read("input.wav").unwrap();
//! let probe = reg.probe_input(&mut io, None).unwrap();
//! ```

pub mod demuxer;
pub mod demuxers;
pub mod format_id;
pub mod io;
pub mod muxer;
pub mod muxers;
pub mod probe;
pub mod registry;
pub mod stream;

// 重导出常用类型
pub use demuxer::Demuxer;
pub use format_id::FormatId;
pub use io::IoContext;
pub use muxer::Muxer;
pub use probe::ProbeResult;
pub use registry::FormatRegistry;
pub use stream::Stream;

/// 注册所有内置容器格式
pub fn register_all(registry: &mut FormatRegistry) {
    demuxers::register_all_demuxers(registry);
    muxers::register_all_muxers(registry);
}
