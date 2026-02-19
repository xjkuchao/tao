//! 淡入淡出滤镜.
//!
//! 对标 FFmpeg 的 `afade` / `fade` 滤镜, 支持音频和视频的淡入/淡出效果.
//!
//! # 参数
//! - `fade_type`: FadeIn (淡入) 或 FadeOut (淡出)
//! - `start_time`: 开始时间 (秒)
//! - `duration`: 淡变时长 (秒)

use tao_codec::frame::{AudioFrame, Frame, VideoFrame};
use tao_core::{SampleFormat, TaoError, TaoResult};

use crate::Filter;

/// 淡变类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FadeType {
    /// 淡入 (从静音/黑色到正常)
    In,
    /// 淡出 (从正常到静音/黑色)
    Out,
}

/// 淡入淡出滤镜
pub struct FadeFilter {
    /// 淡变类型
    fade_type: FadeType,
    /// 开始时间 (秒)
    start_time: f64,
    /// 淡变时长 (秒)
    duration: f64,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl FadeFilter {
    /// 创建淡变滤镜
    pub fn new(fade_type: FadeType, start_time: f64, duration: f64) -> Self {
        Self {
            fade_type,
            start_time,
            duration,
            output: None,
        }
    }

    /// 计算当前时间点的增益因子 (0.0 ~ 1.0)
    fn gain_at(&self, time_sec: f64) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }

        let progress = if time_sec < self.start_time {
            0.0
        } else if time_sec >= self.start_time + self.duration {
            1.0
        } else {
            (time_sec - self.start_time) / self.duration
        };

        match self.fade_type {
            FadeType::In => progress,
            FadeType::Out => 1.0 - progress,
        }
    }

    /// 对音频帧应用淡变
    fn fade_audio(&self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let time_sec = if frame.time_base.den > 0 {
            frame.pts as f64 * frame.time_base.num as f64 / frame.time_base.den as f64
        } else {
            0.0
        };

        let gain = self.gain_at(time_sec);
        let mut out = frame.clone();

        match frame.sample_format {
            SampleFormat::F32 | SampleFormat::F32p => {
                for plane in &mut out.data {
                    let samples = unsafe {
                        std::slice::from_raw_parts_mut(
                            plane.as_mut_ptr() as *mut f32,
                            plane.len() / 4,
                        )
                    };
                    for s in samples.iter_mut() {
                        *s = (*s as f64 * gain) as f32;
                    }
                }
            }
            SampleFormat::S16 | SampleFormat::S16p => {
                for plane in &mut out.data {
                    let samples = unsafe {
                        std::slice::from_raw_parts_mut(
                            plane.as_mut_ptr() as *mut i16,
                            plane.len() / 2,
                        )
                    };
                    for s in samples.iter_mut() {
                        *s = (*s as f64 * gain).round() as i16;
                    }
                }
            }
            _ => {
                return Err(TaoError::Unsupported(format!(
                    "fade: 不支持采样格式 {:?}",
                    frame.sample_format,
                )));
            }
        }

        Ok(out)
    }

    /// 对视频帧应用淡变 (乘以增益因子)
    fn fade_video(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        let time_sec = if frame.time_base.den > 0 {
            frame.pts as f64 * frame.time_base.num as f64 / frame.time_base.den as f64
        } else {
            0.0
        };

        let gain = self.gain_at(time_sec);
        let mut out = frame.clone();

        // 对所有像素数据乘以增益 (淡到黑色)
        for plane in &mut out.data {
            for byte in plane.iter_mut() {
                *byte = (*byte as f64 * gain).round() as u8;
            }
        }

        Ok(out)
    }
}

impl Filter for FadeFilter {
    fn name(&self) -> &str {
        "fade"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        let result = match frame {
            Frame::Audio(af) => Frame::Audio(self.fade_audio(af)?),
            Frame::Video(vf) => Frame::Video(self.fade_video(vf)?),
        };
        self.output = Some(result);
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        self.output.take().ok_or(TaoError::NeedMoreData)
    }

    fn flush(&mut self) -> TaoResult<()> {
        self.output = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao_core::{ChannelLayout, PixelFormat, Rational};

    fn make_f32_frame_at(pts: i64, time_base: Rational, samples: &[f32]) -> Frame {
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
            pts,
            time_base,
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
    fn test_fade_in_mute_at_start() {
        let mut filter = FadeFilter::new(FadeType::In, 0.0, 1.0);
        // pts=0, time_base=1/1 -> time=0s (淡入开始, gain=0)
        let input = make_f32_frame_at(0, Rational::new(1, 1), &[1.0, 0.5, -0.5]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        for &s in &samples {
            assert!(s.abs() < 0.001, "淡入开始时应为静音, 实际={s}");
        }
    }

    #[test]
    fn test_fade_in_normal_after_end() {
        let mut filter = FadeFilter::new(FadeType::In, 0.0, 1.0);
        // pts=2, time_base=1/1 -> time=2s (淡入已结束, gain=1.0)
        let input = make_f32_frame_at(2, Rational::new(1, 1), &[1.0, 0.5]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 1.0).abs() < 0.001);
        assert!((samples[1] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_fade_out_mute_at_end() {
        let mut filter = FadeFilter::new(FadeType::Out, 0.0, 1.0);
        // pts=1, time_base=1/1 -> time=1s (淡出结束, gain=0)
        let input = make_f32_frame_at(1, Rational::new(1, 1), &[1.0, 0.5]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        for &s in &samples {
            assert!(s.abs() < 0.001, "淡出结束时应为静音");
        }
    }

    #[test]
    fn test_fade_video() {
        let mut filter = FadeFilter::new(FadeType::In, 0.0, 1.0);

        let mut vf = VideoFrame::new(2, 2, PixelFormat::Rgb24);
        vf.data = vec![vec![255; 12]]; // 2x2 白色
        vf.linesize = vec![6];
        vf.pts = 0;
        vf.time_base = Rational::new(1, 1);

        filter.send_frame(&Frame::Video(vf)).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(out) = &output {
            // 时间=0, 淡入 gain=0, 所有像素应为 0
            for &b in &out.data[0] {
                assert_eq!(b, 0, "淡入开始时视频应为黑色");
            }
        }
    }

    #[test]
    fn test_fade_middle_value() {
        let mut filter = FadeFilter::new(FadeType::In, 0.0, 2.0);
        // pts=1, time_base=1/1 -> time=1s, progress=0.5, gain=0.5
        let input = make_f32_frame_at(1, Rational::new(1, 1), &[1.0, -1.0]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - (-0.5)).abs() < 0.001);
    }
}
