//! 解码器实现模块.

pub mod aac;
pub mod flac;
pub mod h264;
pub mod mp3;
pub mod pcm;
pub mod rawvideo;

use crate::codec_id::CodecId;
use crate::registry::CodecRegistry;

/// 注册所有内置解码器
pub fn register_all_decoders(registry: &mut CodecRegistry) {
    registry.register_decoder(
        CodecId::RawVideo,
        "rawvideo",
        rawvideo::RawVideoDecoder::create,
    );
    registry.register_decoder(CodecId::PcmU8, "pcm_u8", pcm::PcmDecoder::new_u8);
    registry.register_decoder(CodecId::PcmS16le, "pcm_s16le", pcm::PcmDecoder::new_s16le);
    registry.register_decoder(CodecId::PcmS16be, "pcm_s16be", pcm::PcmDecoder::new_s16be);
    registry.register_decoder(CodecId::PcmS24le, "pcm_s24le", pcm::PcmDecoder::new_s24le);
    registry.register_decoder(CodecId::PcmS32le, "pcm_s32le", pcm::PcmDecoder::new_s32le);
    registry.register_decoder(CodecId::PcmF32le, "pcm_f32le", pcm::PcmDecoder::new_f32le);
    registry.register_decoder(CodecId::Flac, "flac", flac::FlacDecoder::create);
    registry.register_decoder(CodecId::Aac, "aac", aac::AacDecoder::create);
    registry.register_decoder(CodecId::Mp3, "mp3", mp3::Mp3Decoder::create);
    registry.register_decoder(CodecId::H264, "h264", h264::H264Decoder::create);
}
