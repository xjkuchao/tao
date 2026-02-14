//! # tao-format
//!
//! Tao 多媒体框架容器格式库, 提供封装/解封装框架.
//!
//! 本 crate 对标 FFmpeg 的 libavformat, 负责处理多媒体容器格式的读写.

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
