//! # tao-codec
//!
//! Tao 多媒体框架编解码器库, 提供编解码器框架与 Packet/Frame 抽象.
//!
//! 本 crate 对标 FFmpeg 的 libavcodec, 定义了编解码器注册、编解码流程的核心抽象.

pub mod codec_id;
pub mod codec_parameters;
pub mod decoder;
pub mod decoders;
pub mod encoder;
pub mod encoders;
pub mod frame;
pub mod packet;
pub mod registry;

// 重导出常用类型
pub use codec_id::CodecId;
pub use codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType, VideoCodecParams};
pub use decoder::Decoder;
pub use encoder::Encoder;
pub use frame::{AudioFrame, Frame, VideoFrame};
pub use packet::Packet;
pub use registry::CodecRegistry;

/// 注册所有内置编解码器
pub fn register_all(registry: &mut CodecRegistry) {
    decoders::register_all_decoders(registry);
    encoders::register_all_encoders(registry);
}
