//! 流信息定义.
//!
//! 对标 FFmpeg 的 `AVStream`, 描述容器中的一条音视频/字幕流.

use tao_codec::CodecId;
use tao_core::{ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat};

/// 流信息
///
/// 描述容器格式中的一条流 (视频流/音频流/字幕流等).
#[derive(Debug, Clone)]
pub struct Stream {
    /// 流索引 (在容器中的位置, 从 0 开始)
    pub index: usize,
    /// 媒体类型
    pub media_type: MediaType,
    /// 编解码器标识
    pub codec_id: CodecId,
    /// 时间基
    pub time_base: Rational,
    /// 流时长 (以 time_base 为单位, -1 表示未知)
    pub duration: i64,
    /// 起始时间 (以 time_base 为单位)
    pub start_time: i64,
    /// 总帧数 (0 表示未知)
    pub nb_frames: u64,
    /// 编解码器私有数据 (extradata, 如 SPS/PPS)
    pub extra_data: Vec<u8>,
    /// 流特定参数
    pub params: StreamParams,
    /// 元数据 (标题, 语言等)
    pub metadata: Vec<(String, String)>,
}

/// 流特定参数
#[derive(Debug, Clone)]
pub enum StreamParams {
    /// 视频流参数
    Video(VideoStreamParams),
    /// 音频流参数
    Audio(AudioStreamParams),
    /// 字幕流参数
    Subtitle,
    /// 其他
    Other,
}

/// 视频流参数
#[derive(Debug, Clone)]
pub struct VideoStreamParams {
    /// 宽度 (像素)
    pub width: u32,
    /// 高度 (像素)
    pub height: u32,
    /// 像素格式
    pub pixel_format: PixelFormat,
    /// 帧率 (可能是平均帧率)
    pub frame_rate: Rational,
    /// 采样宽高比 (SAR)
    pub sample_aspect_ratio: Rational,
    /// 码率 (bps, 0 表示未知)
    pub bit_rate: u64,
}

/// 音频流参数
#[derive(Debug, Clone)]
pub struct AudioStreamParams {
    /// 采样率 (Hz)
    pub sample_rate: u32,
    /// 声道布局
    pub channel_layout: ChannelLayout,
    /// 采样格式
    pub sample_format: SampleFormat,
    /// 码率 (bps, 0 表示未知)
    pub bit_rate: u64,
    /// 每帧采样数 (如 AAC 为 1024, MP3 为 1152)
    pub frame_size: u32,
}
