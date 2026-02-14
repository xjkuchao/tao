//! # tao-core
//!
//! Tao 多媒体框架核心库, 提供基础类型定义、错误处理和工具函数.
//!
//! 本 crate 对标 FFmpeg 的 libavutil, 为整个 Tao 框架提供底层基础设施.

pub mod bitreader;
pub mod bitwriter;
pub mod channel_layout;
pub mod color;
pub mod crc;
pub mod error;
pub mod media_type;
pub mod pixel_format;
pub mod rational;
pub mod sample_format;
pub mod subtitle;
pub mod timestamp;

// 重导出常用类型
pub use channel_layout::ChannelLayout;
pub use error::{TaoError, TaoResult};
pub use media_type::MediaType;
pub use pixel_format::PixelFormat;
pub use rational::Rational;
pub use sample_format::SampleFormat;
pub use timestamp::Timestamp;
