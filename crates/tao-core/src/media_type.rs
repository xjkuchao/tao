//! 媒体类型定义.
//!
//! 对标 FFmpeg 的 `AVMediaType`.

use std::fmt;

/// 媒体流类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    /// 视频流
    Video,
    /// 音频流
    Audio,
    /// 字幕流
    Subtitle,
    /// 数据流 (如时间码)
    Data,
    /// 附件流 (如封面图片、字体)
    Attachment,
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Video => "视频",
            Self::Audio => "音频",
            Self::Subtitle => "字幕",
            Self::Data => "数据",
            Self::Attachment => "附件",
        };
        write!(f, "{name}")
    }
}
