//! 编解码器参数.
//!
//! 对标 FFmpeg 的 `AVCodecParameters`, 描述编解码器的配置参数.

use tao_core::{ChannelLayout, PixelFormat, Rational, SampleFormat};

use crate::codec_id::CodecId;

/// 编解码器参数
///
/// 传递给编解码器的配置信息, 通常从容器格式中提取.
#[derive(Debug, Clone)]
pub struct CodecParameters {
    /// 编解码器标识
    pub codec_id: CodecId,
    /// 额外数据 (如 SPS/PPS, DecoderSpecificInfo 等)
    pub extra_data: Vec<u8>,
    /// 码率 (bits/s)
    pub bit_rate: u64,
    /// 媒体类型特定参数
    pub params: CodecParamsType,
}

/// 媒体类型特定参数
#[derive(Debug, Clone)]
pub enum CodecParamsType {
    /// 视频参数
    Video(VideoCodecParams),
    /// 音频参数
    Audio(AudioCodecParams),
    /// 无特定参数
    None,
}

/// 视频编解码器参数
#[derive(Debug, Clone)]
pub struct VideoCodecParams {
    /// 宽度 (像素)
    pub width: u32,
    /// 高度 (像素)
    pub height: u32,
    /// 像素格式
    pub pixel_format: PixelFormat,
    /// 帧率
    pub frame_rate: Rational,
    /// 采样宽高比 (SAR)
    pub sample_aspect_ratio: Rational,
}

/// 音频编解码器参数
#[derive(Debug, Clone)]
pub struct AudioCodecParams {
    /// 采样率 (Hz)
    pub sample_rate: u32,
    /// 声道布局
    pub channel_layout: ChannelLayout,
    /// 采样格式
    pub sample_format: SampleFormat,
    /// 每帧采样数 (0 表示可变)
    pub frame_size: u32,
}

impl CodecParameters {
    /// 获取视频参数 (如果是视频流)
    pub fn video(&self) -> Option<&VideoCodecParams> {
        match &self.params {
            CodecParamsType::Video(v) => Some(v),
            _ => None,
        }
    }

    /// 获取音频参数 (如果是音频流)
    pub fn audio(&self) -> Option<&AudioCodecParams> {
        match &self.params {
            CodecParamsType::Audio(a) => Some(a),
            _ => None,
        }
    }
}
