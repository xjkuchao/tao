//! 封装器实现模块.

pub mod aac;
pub mod aiff;
pub mod avi;
pub mod flac;
pub mod flv;
pub mod mkv;
pub mod mp3;
pub mod mp4;
pub mod mpegts;
pub mod ogg;
pub mod wav;

use crate::format_id::FormatId;
use crate::registry::FormatRegistry;

/// 注册所有内置封装器
pub fn register_all_muxers(registry: &mut FormatRegistry) {
    registry.register_muxer(FormatId::AacAdts, "aac", aac::AacAdtsMuxer::create);
    registry.register_muxer(FormatId::Wav, "wav", wav::WavMuxer::create);
    registry.register_muxer(FormatId::Aiff, "aiff", aiff::AiffMuxer::create);
    registry.register_muxer(FormatId::FlacContainer, "flac", flac::FlacMuxer::create);
    registry.register_muxer(FormatId::Mp4, "mp4", mp4::Mp4Muxer::create);
    registry.register_muxer(FormatId::Matroska, "matroska", mkv::MkvMuxer::create);
    registry.register_muxer(FormatId::Webm, "webm", mkv::MkvMuxer::create_webm);
    registry.register_muxer(FormatId::Ogg, "ogg", ogg::OggMuxer::create);
    registry.register_muxer(FormatId::Flv, "flv", flv::FlvMuxer::create);
    registry.register_muxer(FormatId::Mp3Container, "mp3", mp3::Mp3Muxer::create);
    registry.register_muxer(FormatId::MpegTs, "mpegts", mpegts::MpegTsMuxer::create);
    registry.register_muxer(FormatId::Avi, "avi", avi::AviMuxer::create);
}
