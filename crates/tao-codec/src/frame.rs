//! 解码后的帧数据 (Frame).
//!
//! 对标 FFmpeg 的 `AVFrame`, 表示解码后的原始音视频数据.

use tao_core::{
    ChannelLayout, PixelFormat, Rational, SampleFormat,
    color::{ColorRange, ColorSpace},
};

/// 视频帧
///
/// 包含解码后的原始像素数据, 支持多平面存储.
/// 例如 YUV420P 格式有 3 个平面: Y, U, V.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// 各平面的像素数据
    pub data: Vec<Vec<u8>>,
    /// 各平面每行的字节数 (linesize / stride)
    pub linesize: Vec<usize>,
    /// 宽度 (像素)
    pub width: u32,
    /// 高度 (像素)
    pub height: u32,
    /// 像素格式
    pub pixel_format: PixelFormat,
    /// 显示时间戳 (PTS)
    pub pts: i64,
    /// 时间基
    pub time_base: Rational,
    /// 帧时长 (以 time_base 为单位)
    pub duration: i64,
    /// 是否为关键帧
    pub is_keyframe: bool,
    /// 图片类型 (I/P/B 帧)
    pub picture_type: PictureType,
    /// 采样宽高比 (SAR)
    pub sample_aspect_ratio: Rational,
    /// 色彩空间
    pub color_space: ColorSpace,
    /// 色彩范围
    pub color_range: ColorRange,
}

impl VideoFrame {
    /// 创建空的视频帧
    pub fn new(width: u32, height: u32, pixel_format: PixelFormat) -> Self {
        let plane_count = pixel_format.plane_count() as usize;
        Self {
            data: vec![Vec::new(); plane_count],
            linesize: vec![0; plane_count],
            width,
            height,
            pixel_format,
            pts: tao_core::timestamp::NOPTS_VALUE,
            time_base: Rational::UNDEFINED,
            duration: 0,
            is_keyframe: false,
            picture_type: PictureType::None,
            sample_aspect_ratio: Rational::new(1, 1),
            color_space: ColorSpace::default(),
            color_range: ColorRange::default(),
        }
    }
}

/// 音频帧
///
/// 包含解码后的原始音频采样数据.
/// 平面格式: data 中每个 Vec 对应一个声道.
/// 交错格式: data 中只有一个 Vec, 所有声道交替排列.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// 音频采样数据 (平面格式: 每声道一个 Vec; 交错格式: 单个 Vec)
    pub data: Vec<Vec<u8>>,
    /// 本帧包含的采样数 (每声道)
    pub nb_samples: u32,
    /// 采样率 (Hz)
    pub sample_rate: u32,
    /// 采样格式
    pub sample_format: SampleFormat,
    /// 声道布局
    pub channel_layout: ChannelLayout,
    /// 显示时间戳 (PTS)
    pub pts: i64,
    /// 时间基
    pub time_base: Rational,
    /// 帧时长 (以 time_base 为单位)
    pub duration: i64,
}

impl AudioFrame {
    /// 创建空的音频帧
    pub fn new(
        nb_samples: u32,
        sample_rate: u32,
        sample_format: SampleFormat,
        channel_layout: ChannelLayout,
    ) -> Self {
        let plane_count = if sample_format.is_planar() {
            channel_layout.channels as usize
        } else {
            1
        };
        Self {
            data: vec![Vec::new(); plane_count],
            nb_samples,
            sample_rate,
            sample_format,
            channel_layout,
            pts: tao_core::timestamp::NOPTS_VALUE,
            time_base: Rational::UNDEFINED,
            duration: 0,
        }
    }
}

/// 帧 (视频帧或音频帧的统一包装)
#[derive(Debug, Clone)]
pub enum Frame {
    /// 视频帧
    Video(VideoFrame),
    /// 音频帧
    Audio(AudioFrame),
}

/// 图片类型 (I/P/B 帧)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PictureType {
    /// 未指定
    #[default]
    None,
    /// I 帧 (关键帧, 帧内编码)
    I,
    /// P 帧 (前向预测)
    P,
    /// B 帧 (双向预测)
    B,
    /// S 帧 (GMC Sprite)
    S,
    /// SI 帧 (切换 I 帧)
    Si,
    /// SP 帧 (切换 P 帧)
    Sp,
}
