//! 音量调节滤镜.
//!
//! 对标 FFmpeg 的 `volume` 滤镜, 支持线性倍数和 dB 两种方式指定增益.

use tao_codec::frame::{AudioFrame, Frame};
use tao_core::{SampleFormat, TaoError, TaoResult};

use crate::Filter;

/// 音量调节滤镜
pub struct VolumeFilter {
    /// 增益系数 (线性, 1.0 = 不变)
    gain: f64,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl VolumeFilter {
    /// 使用线性增益创建 (1.0 = 不变, 2.0 = 加倍, 0.5 = 减半)
    pub fn new(gain: f64) -> Self {
        Self { gain, output: None }
    }

    /// 使用 dB 增益创建 (0 = 不变, 6 约 加倍, -6 约 减半)
    pub fn from_db(db: f64) -> Self {
        Self::new(10.0_f64.powf(db / 20.0))
    }

    /// 对音频帧应用增益
    fn apply_gain(&self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let mut out = frame.clone();

        match frame.sample_format {
            SampleFormat::F32 | SampleFormat::F32p => {
                for plane in &mut out.data {
                    let samples: &mut [f32] = cast_slice_mut(plane);
                    for s in samples.iter_mut() {
                        *s = (*s as f64 * self.gain) as f32;
                    }
                }
            }
            SampleFormat::S16 | SampleFormat::S16p => {
                for plane in &mut out.data {
                    let samples: &mut [i16] = cast_slice_mut(plane);
                    for s in samples.iter_mut() {
                        let v = (*s as f64 * self.gain).round();
                        *s = v.clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                    }
                }
            }
            SampleFormat::S32 | SampleFormat::S32p => {
                for plane in &mut out.data {
                    let samples: &mut [i32] = cast_slice_mut(plane);
                    for s in samples.iter_mut() {
                        let v = (*s as f64 * self.gain).round();
                        *s = v.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                    }
                }
            }
            SampleFormat::F64 | SampleFormat::F64p => {
                for plane in &mut out.data {
                    let samples: &mut [f64] = cast_slice_mut(plane);
                    for s in samples.iter_mut() {
                        *s *= self.gain;
                    }
                }
            }
            _ => {
                return Err(TaoError::Unsupported(format!(
                    "volume 滤镜不支持采样格式 {:?}",
                    frame.sample_format,
                )));
            }
        }

        Ok(out)
    }
}

impl Filter for VolumeFilter {
    fn name(&self) -> &str {
        "volume"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Audio(af) => {
                let result = self.apply_gain(af)?;
                self.output = Some(Frame::Audio(result));
                Ok(())
            }
            Frame::Video(_) => Err(TaoError::InvalidArgument("volume 滤镜仅支持音频帧".into())),
        }
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        self.output.take().ok_or(TaoError::NeedMoreData)
    }

    fn flush(&mut self) -> TaoResult<()> {
        self.output = None;
        Ok(())
    }
}

/// 将字节切片转换为类型切片 (可变)
fn cast_slice_mut<T: Copy + 'static>(bytes: &mut Vec<u8>) -> &mut [T] {
    let len = bytes.len() / std::mem::size_of::<T>();
    unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut T, len) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao_core::{ChannelLayout, Rational};

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
    fn test_volume_加倍() {
        let mut filter = VolumeFilter::new(2.0);
        let input = make_f32_frame(&[0.1, 0.2, -0.3, 0.5]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.2).abs() < 0.001);
        assert!((samples[1] - 0.4).abs() < 0.001);
        assert!((samples[2] - (-0.6)).abs() < 0.001);
        assert!((samples[3] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_volume_减半() {
        let mut filter = VolumeFilter::new(0.5);
        let input = make_f32_frame(&[1.0, -1.0, 0.5]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn test_volume_from_db() {
        let filter = VolumeFilter::from_db(6.0);
        assert!((filter.gain - 1.995).abs() < 0.01);
        let filter = VolumeFilter::from_db(-6.0);
        assert!((filter.gain - 0.501).abs() < 0.01);
        let filter = VolumeFilter::from_db(0.0);
        assert!((filter.gain - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_volume_静音() {
        let mut filter = VolumeFilter::new(0.0);
        let input = make_f32_frame(&[0.5, -0.3, 1.0]);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        for &s in &samples {
            assert_eq!(s, 0.0);
        }
    }

    #[test]
    fn test_volume_视频帧报错() {
        use tao_core::PixelFormat;
        let mut filter = VolumeFilter::new(1.0);
        let vf = Frame::Video(tao_codec::frame::VideoFrame::new(
            320,
            240,
            PixelFormat::Rgb24,
        ));
        assert!(filter.send_frame(&vf).is_err());
    }
}
