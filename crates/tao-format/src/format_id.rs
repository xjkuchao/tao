//! 容器格式标识符.
//!
//! 对标 FFmpeg 的输入/输出格式名称.

use std::fmt;

/// 容器格式标识符
///
/// 标识一种多媒体容器格式, 如 MP4, MKV, AVI 等.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatId {
    // ========================
    // 视频容器
    // ========================
    /// MPEG-4 Part 14 (MP4)
    Mp4,
    /// Matroska (MKV)
    Matroska,
    /// WebM (Matroska 子集)
    Webm,
    /// Audio Video Interleave (AVI)
    Avi,
    /// Flash Video (FLV)
    Flv,
    /// MPEG Transport Stream (TS)
    MpegTs,
    /// MPEG Program Stream (PS)
    MpegPs,
    /// Material eXchange Format (MXF)
    Mxf,
    /// 3GPP
    ThreeGp,
    /// Ogg
    Ogg,
    /// ASF / WMV / WMA
    Asf,

    // ========================
    // 纯音频容器
    // ========================
    /// WAV (RIFF WAVE)
    Wav,
    /// FLAC 原生容器
    FlacContainer,
    /// MP3 原生容器 (裸 MPEG Audio)
    Mp3Container,
    /// AAC 原生容器 (ADTS)
    AacAdts,
    /// AIFF
    Aiff,

    // ========================
    // 图片序列
    // ========================
    /// 图片序列 (PNG/JPEG/BMP 等)
    ImageSequence,

    // ========================
    // Raw 格式
    // ========================
    /// Raw 视频
    RawVideo,
    /// Raw 音频 (PCM)
    RawAudio,

    // ========================
    // Elementary Stream 格式
    // ========================
    /// MPEG-4 Part 2 Elementary Stream (M4V)
    Mpeg4Es,
}

impl FormatId {
    /// 获取格式的人类可读名称
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "matroska",
            Self::Webm => "webm",
            Self::Avi => "avi",
            Self::Flv => "flv",
            Self::MpegTs => "mpegts",
            Self::MpegPs => "mpeg",
            Self::Mxf => "mxf",
            Self::ThreeGp => "3gp",
            Self::Ogg => "ogg",
            Self::Asf => "asf",
            Self::Wav => "wav",
            Self::FlacContainer => "flac",
            Self::Mp3Container => "mp3",
            Self::AacAdts => "aac",
            Self::Aiff => "aiff",
            Self::ImageSequence => "image2",
            Self::RawVideo => "rawvideo",
            Self::RawAudio => "rawaudio",
            Self::Mpeg4Es => "m4v",
        }
    }

    /// 获取格式常用的文件扩展名
    pub const fn extensions(&self) -> &'static [&'static str] {
        match self {
            Self::Mp4 => &["mp4", "m4a", "m4v", "mov"],
            Self::Matroska => &["mkv", "mka"],
            Self::Webm => &["webm"],
            Self::Avi => &["avi"],
            Self::Flv => &["flv"],
            Self::MpegTs => &["ts", "m2ts", "mts"],
            Self::MpegPs => &["mpg", "mpeg", "vob"],
            Self::Mxf => &["mxf"],
            Self::ThreeGp => &["3gp", "3g2"],
            Self::Ogg => &["ogg", "ogv", "oga", "ogx"],
            Self::Asf => &["asf", "wmv", "wma"],
            Self::Wav => &["wav"],
            Self::FlacContainer => &["flac"],
            Self::Mp3Container => &["mp3"],
            Self::AacAdts => &["aac"],
            Self::Aiff => &["aiff", "aif"],
            Self::ImageSequence => &["png", "jpg", "jpeg", "bmp"],
            Self::RawVideo => &["yuv", "rgb"],
            Self::RawAudio => &["pcm", "raw"],
            Self::Mpeg4Es => &["m4v"],
        }
    }
}

impl FormatId {
    /// 所有已知格式标识的列表
    pub const ALL: &[FormatId] = &[
        Self::Mp4,
        Self::Matroska,
        Self::Webm,
        Self::Avi,
        Self::Flv,
        Self::MpegTs,
        Self::MpegPs,
        Self::Mxf,
        Self::ThreeGp,
        Self::Ogg,
        Self::Asf,
        Self::Wav,
        Self::FlacContainer,
        Self::Mp3Container,
        Self::AacAdts,
        Self::Aiff,
        Self::ImageSequence,
        Self::RawVideo,
        Self::RawAudio,
        Self::Mpeg4Es,
    ];

    /// 根据文件扩展名猜测格式
    ///
    /// # 参数
    /// - `ext`: 文件扩展名 (不含 `.`, 如 "mp4", "wav")
    pub fn from_extension(ext: &str) -> Option<FormatId> {
        let ext_lower = ext.to_lowercase();
        Self::ALL
            .iter()
            .find(|id| id.extensions().contains(&ext_lower.as_str()))
            .copied()
    }

    /// 从文件路径猜测格式
    pub fn from_filename(filename: &str) -> Option<FormatId> {
        let ext = filename.rsplit('.').next()?;
        Self::from_extension(ext)
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}
