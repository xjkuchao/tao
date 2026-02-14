//! 解封装器实现模块.

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

/// 注册所有内置解封装器
pub fn register_all_demuxers(registry: &mut FormatRegistry) {
    registry.register_demuxer(FormatId::Wav, "wav", wav::WavDemuxer::create);
    registry.register_probe(Box::new(wav::WavProbe));

    registry.register_demuxer(FormatId::Aiff, "aiff", aiff::AiffDemuxer::create);
    registry.register_probe(Box::new(aiff::AiffProbe));

    registry.register_demuxer(FormatId::FlacContainer, "flac", flac::FlacDemuxer::create);
    registry.register_probe(Box::new(flac::FlacProbe));

    registry.register_demuxer(FormatId::Ogg, "ogg", ogg::OggDemuxer::create);
    registry.register_probe(Box::new(ogg::OggProbe));

    registry.register_demuxer(FormatId::Mp4, "mp4", mp4::Mp4Demuxer::create);
    registry.register_probe(Box::new(mp4::Mp4Probe));

    registry.register_demuxer(FormatId::Mp3Container, "mp3", mp3::Mp3Demuxer::create);
    registry.register_probe(Box::new(mp3::Mp3Probe));

    registry.register_demuxer(FormatId::Matroska, "matroska", mkv::MkvDemuxer::create);
    registry.register_probe(Box::new(mkv::MkvProbe));

    registry.register_demuxer(FormatId::AacAdts, "aac", aac::AacDemuxer::create);
    registry.register_probe(Box::new(aac::AacProbe));

    registry.register_demuxer(FormatId::Flv, "flv", flv::FlvDemuxer::create);
    registry.register_probe(Box::new(flv::FlvProbe));

    registry.register_demuxer(FormatId::MpegTs, "mpegts", mpegts::TsDemuxer::create);
    registry.register_probe(Box::new(mpegts::TsProbe));

    registry.register_demuxer(FormatId::Avi, "avi", avi::AviDemuxer::create);
    registry.register_probe(Box::new(avi::AviProbe));
}
