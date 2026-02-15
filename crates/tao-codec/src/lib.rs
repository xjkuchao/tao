//! # tao-codec
//!
//! Tao 多媒体框架编解码器库, 提供编解码器框架与 Packet/Frame 抽象.
//!
//! 本 crate 对标 FFmpeg 的 libavcodec, 定义了编解码器注册、编解码流程的核心抽象.
//!
//! ## 支持的编解码器
//!
//! - **解码器**: PCM (U8/S16/S24/S32/F32), FLAC, AAC, MP3, Vorbis, RawVideo, H.264 解析器
//! - **编码器**: PCM (多种格式), FLAC, AAC, RawVideo
//!
//! ## 使用示例
//!
//! ```rust
//! use tao_codec::{CodecRegistry, CodecId};
//!
//! let mut reg = CodecRegistry::new();
//! tao_codec::register_all(&mut reg);
//!
//! // 按 CodecId 创建编解码器实例
//! let decoder = reg.create_decoder(CodecId::Flac).unwrap();
//! let encoder = reg.create_encoder(CodecId::PcmS16le).unwrap();
//! ```

pub mod codec_id;
pub mod codec_parameters;
pub mod decoder;
pub mod decoders;
pub mod encoder;
pub mod encoders;
pub mod frame;
pub mod packet;
pub mod parsers;
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
