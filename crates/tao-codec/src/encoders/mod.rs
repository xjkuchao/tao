//! 编码器实现模块.

pub mod flac;
pub mod pcm;
pub mod rawvideo;

use crate::codec_id::CodecId;
use crate::registry::CodecRegistry;

/// 注册所有内置编码器
pub fn register_all_encoders(registry: &mut CodecRegistry) {
    registry.register_encoder(
        CodecId::RawVideo,
        "rawvideo",
        rawvideo::RawVideoEncoder::create,
    );
    registry.register_encoder(CodecId::PcmU8, "pcm_u8", pcm::PcmEncoder::new_u8);
    registry.register_encoder(CodecId::PcmS16le, "pcm_s16le", pcm::PcmEncoder::new_s16le);
    registry.register_encoder(CodecId::PcmS16be, "pcm_s16be", pcm::PcmEncoder::new_s16be);
    registry.register_encoder(CodecId::PcmS24le, "pcm_s24le", pcm::PcmEncoder::new_s24le);
    registry.register_encoder(CodecId::PcmS32le, "pcm_s32le", pcm::PcmEncoder::new_s32le);
    registry.register_encoder(CodecId::PcmF32le, "pcm_f32le", pcm::PcmEncoder::new_f32le);
    registry.register_encoder(CodecId::Flac, "flac", flac::FlacEncoder::create);
}
