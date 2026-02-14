//! # tao-filter
//!
//! Tao 多媒体框架滤镜库, 提供音视频滤镜框架.
//!
//! 本 crate 对标 FFmpeg 的 libavfilter, 支持构建滤镜图 (FilterGraph)
//! 对音视频数据进行各种变换处理.
//!
//! ## 支持的滤镜
//!
//! - **音频**: volume (音量), fade (淡入淡出), loudnorm (响度归一化), equalizer (均衡器)
//! - **视频**: crop (裁剪), pad (填充), overlay (叠加), drawtext (文字绘制)
//!
//! ## 使用示例
//!
//! ```rust
//! use tao_filter::FilterGraph;
//! use tao_filter::filters::volume::VolumeFilter;
//!
//! let mut graph = FilterGraph::new();
//! graph.add_filter(Box::new(VolumeFilter::new(2.0)));
//!
//! // 将帧送入滤镜链处理
//! // let output = graph.process_frame(&input_frame).unwrap();
//! ```

pub mod filters;

use tao_codec::frame::Frame;
use tao_core::{TaoError, TaoResult};

/// 滤镜 trait
///
/// 所有音视频滤镜都实现此 trait.
/// 滤镜接收输入帧, 处理后输出帧.
pub trait Filter: Send {
    /// 获取滤镜名称
    fn name(&self) -> &str;

    /// 送入一帧数据
    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()>;

    /// 取出一帧处理后的数据
    fn receive_frame(&mut self) -> TaoResult<Frame>;

    /// 刷新滤镜 (处理剩余缓存数据)
    fn flush(&mut self) -> TaoResult<()>;
}

/// 滤镜图
///
/// 由多个滤镜组成的处理管线, 数据从输入端流经各个滤镜后到达输出端.
///
/// 滤镜图支持线性链: 数据依次通过每个滤镜处理.
pub struct FilterGraph {
    /// 滤镜链中的滤镜列表
    filters: Vec<Box<dyn Filter>>,
}

impl FilterGraph {
    /// 创建空的滤镜图
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// 添加滤镜到图中
    pub fn add_filter(&mut self, filter: Box<dyn Filter>) {
        self.filters.push(filter);
    }

    /// 获取滤镜数量
    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }

    /// 将帧送入滤镜链, 依次流过每个滤镜, 返回最终输出帧.
    ///
    /// 帧从第一个滤镜开始, 每个滤镜的输出作为下一个滤镜的输入.
    /// 如果滤镜链为空, 则直接返回输入帧 (透传).
    pub fn process_frame(&mut self, frame: &Frame) -> TaoResult<Frame> {
        if self.filters.is_empty() {
            return Ok(frame.clone());
        }

        let mut current = frame.clone();
        for filter in &mut self.filters {
            filter.send_frame(&current)?;
            current = filter.receive_frame()?;
        }
        Ok(current)
    }

    /// 刷新所有滤镜, 获取剩余缓存帧.
    ///
    /// 对于有缓冲的滤镜 (如 atempo), 需要在流结束时调用此方法.
    /// 返回所有剩余帧的列表.
    pub fn flush_all(&mut self) -> TaoResult<Vec<Frame>> {
        let mut remaining = Vec::new();
        for filter in &mut self.filters {
            filter.flush()?;
            // 尝试取出刷新产生的帧
            loop {
                match filter.receive_frame() {
                    Ok(frame) => remaining.push(frame),
                    Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(remaining)
    }

    /// 获取滤镜名称列表 (调试用)
    pub fn filter_names(&self) -> Vec<&str> {
        self.filters.iter().map(|f| f.name()).collect()
    }
}

impl Default for FilterGraph {
    fn default() -> Self {
        Self::new()
    }
}

// 便捷重导出
pub use filters::crop::CropFilter;
pub use filters::drawtext::DrawtextFilter;
pub use filters::equalizer::EqualizerFilter;
pub use filters::fade::{FadeFilter, FadeType};
pub use filters::loudnorm::LoudnormFilter;
pub use filters::overlay::OverlayFilter;
pub use filters::pad::{PadColor, PadFilter};
pub use filters::volume::VolumeFilter;

#[cfg(test)]
mod tests {
    use super::*;
    use tao_codec::frame::AudioFrame;
    use tao_core::{ChannelLayout, Rational, SampleFormat};

    fn make_f32_frame(samples: &[f32]) -> Frame {
        let mut data = Vec::with_capacity(samples.len() * 4);
        for &s in samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        Frame::Audio(AudioFrame {
            data: vec![data],
            nb_samples: samples.len() as u32,
            sample_rate: 44100,
            sample_format: SampleFormat::F32,
            channel_layout: ChannelLayout::from_channels(1),
            pts: 0,
            time_base: Rational::new(1, 44100),
            duration: samples.len() as i64,
        })
    }

    fn extract_f32(frame: &Frame) -> Vec<f32> {
        if let Frame::Audio(af) = frame {
            af.data[0]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        } else {
            panic!("期望音频帧");
        }
    }

    #[test]
    fn test_滤镜图_空链透传() {
        let mut graph = FilterGraph::new();
        let input = make_f32_frame(&[0.5, -0.5]);
        let output = graph.process_frame(&input).unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_滤镜图_单个滤镜() {
        let mut graph = FilterGraph::new();
        graph.add_filter(Box::new(VolumeFilter::new(2.0)));
        let input = make_f32_frame(&[0.25, -0.25]);
        let output = graph.process_frame(&input).unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn test_滤镜图_链式处理() {
        let mut graph = FilterGraph::new();
        graph.add_filter(Box::new(VolumeFilter::new(2.0)));
        graph.add_filter(Box::new(VolumeFilter::new(0.5)));
        // 2x * 0.5x = 1x, 应不变
        let input = make_f32_frame(&[0.3, -0.7]);
        let output = graph.process_frame(&input).unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.3).abs() < 0.001);
        assert!((samples[1] - (-0.7)).abs() < 0.001);
    }

    #[test]
    fn test_滤镜图_名称列表() {
        let mut graph = FilterGraph::new();
        graph.add_filter(Box::new(VolumeFilter::new(1.0)));
        graph.add_filter(Box::new(VolumeFilter::from_db(0.0)));
        let names = graph.filter_names();
        assert_eq!(names, vec!["volume", "volume"]);
    }

    #[test]
    fn test_滤镜图_刷新() {
        let mut graph = FilterGraph::new();
        graph.add_filter(Box::new(VolumeFilter::new(1.0)));
        let remaining = graph.flush_all().unwrap();
        assert!(remaining.is_empty());
    }
}
