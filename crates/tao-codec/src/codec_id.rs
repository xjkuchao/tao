//! 编解码器标识符.
//!
//! 对标 FFmpeg 的 `AVCodecID`, 为每种编解码算法分配唯一标识.

use std::fmt;
use tao_core::MediaType;

/// 编解码器标识符
///
/// 唯一标识一种编解码算法, 与容器格式无关.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CodecId {
    /// 未知编解码器
    None,

    // ========================
    // 视频编解码器
    // ========================
    /// H.264 / AVC / MPEG-4 Part 10
    H264,
    /// H.265 / HEVC / MPEG-H Part 2
    H265,
    /// VP8
    Vp8,
    /// VP9
    Vp9,
    /// AV1 (Alliance for Open Media)
    Av1,
    /// MPEG-1 Video
    Mpeg1Video,
    /// MPEG-2 Video
    Mpeg2Video,
    /// MPEG-4 Part 2 (ASP)
    Mpeg4,
    /// Theora
    Theora,
    /// Motion JPEG
    Mjpeg,
    /// PNG (无损)
    Png,
    /// Raw 视频 (未压缩)
    RawVideo,

    // ========================
    // 音频编解码器
    // ========================
    /// AAC (Advanced Audio Coding)
    Aac,
    /// MP3 (MPEG Audio Layer III)
    Mp3,
    /// MP2 (MPEG Audio Layer II)
    Mp2,
    /// Opus
    Opus,
    /// Vorbis
    Vorbis,
    /// FLAC (Free Lossless Audio Codec)
    Flac,
    /// Apple Lossless (ALAC)
    Alac,
    /// PCM 有符号 16 位小端
    PcmS16le,
    /// PCM 有符号 16 位大端
    PcmS16be,
    /// PCM 有符号 24 位小端
    PcmS24le,
    /// PCM 有符号 32 位小端
    PcmS32le,
    /// PCM 32 位浮点小端
    PcmF32le,
    /// PCM 无符号 8 位
    PcmU8,
    /// AC-3 (Dolby Digital)
    Ac3,
    /// E-AC-3 (Dolby Digital Plus)
    Eac3,
    /// DTS (Digital Theater Systems)
    Dts,

    // ========================
    // 字幕编解码器
    // ========================
    /// SubRip / SRT
    Srt,
    /// ASS / SSA
    Ass,
    /// WebVTT
    Webvtt,
    /// DVD 位图字幕
    DvdSubtitle,
    /// HDMV PGS 字幕
    HdmvPgsSubtitle,
}

impl CodecId {
    /// 获取编解码器对应的媒体类型
    pub const fn media_type(&self) -> MediaType {
        match self {
            Self::None => MediaType::Data,

            // 视频
            Self::H264
            | Self::H265
            | Self::Vp8
            | Self::Vp9
            | Self::Av1
            | Self::Mpeg1Video
            | Self::Mpeg2Video
            | Self::Mpeg4
            | Self::Theora
            | Self::Mjpeg
            | Self::Png
            | Self::RawVideo => MediaType::Video,

            // 音频
            Self::Aac
            | Self::Mp3
            | Self::Mp2
            | Self::Opus
            | Self::Vorbis
            | Self::Flac
            | Self::Alac
            | Self::PcmS16le
            | Self::PcmS16be
            | Self::PcmS24le
            | Self::PcmS32le
            | Self::PcmF32le
            | Self::PcmU8
            | Self::Ac3
            | Self::Eac3
            | Self::Dts => MediaType::Audio,

            // 字幕
            Self::Srt | Self::Ass | Self::Webvtt | Self::DvdSubtitle | Self::HdmvPgsSubtitle => {
                MediaType::Subtitle
            }
        }
    }

    /// 获取编解码器的人类可读名称
    pub const fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::H264 => "h264",
            Self::H265 => "hevc",
            Self::Vp8 => "vp8",
            Self::Vp9 => "vp9",
            Self::Av1 => "av1",
            Self::Mpeg1Video => "mpeg1video",
            Self::Mpeg2Video => "mpeg2video",
            Self::Mpeg4 => "mpeg4",
            Self::Theora => "theora",
            Self::Mjpeg => "mjpeg",
            Self::Png => "png",
            Self::RawVideo => "rawvideo",
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Mp2 => "mp2",
            Self::Opus => "opus",
            Self::Vorbis => "vorbis",
            Self::Flac => "flac",
            Self::Alac => "alac",
            Self::PcmS16le => "pcm_s16le",
            Self::PcmS16be => "pcm_s16be",
            Self::PcmS24le => "pcm_s24le",
            Self::PcmS32le => "pcm_s32le",
            Self::PcmF32le => "pcm_f32le",
            Self::PcmU8 => "pcm_u8",
            Self::Ac3 => "ac3",
            Self::Eac3 => "eac3",
            Self::Dts => "dts",
            Self::Srt => "srt",
            Self::Ass => "ass",
            Self::Webvtt => "webvtt",
            Self::DvdSubtitle => "dvd_subtitle",
            Self::HdmvPgsSubtitle => "hdmv_pgs_subtitle",
        }
    }
}

impl fmt::Display for CodecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}
