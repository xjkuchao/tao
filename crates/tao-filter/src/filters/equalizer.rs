//! 音频均衡器滤镜.
//!
//! 使用级联双二阶 (biquad) 滤波器实现参数均衡器.

use tao_codec::frame::{AudioFrame, Frame};
use tao_core::{SampleFormat, TaoError, TaoResult};

use crate::Filter;

/// 双二阶滤波器频段
struct BiquadBand {
    /// 中心频率 (Hz)
    frequency: f64,
    /// 增益 (dB)
    gain_db: f64,
    /// Q 因子 (带宽)
    q: f64,
    /// 双二阶系数
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    /// 每声道状态 (x[n-1], x[n-2], y[n-1], y[n-2])
    state: Vec<[f64; 4]>,
    /// 计算系数时使用的采样率
    sample_rate: u32,
}

impl BiquadBand {
    /// 创建新的双二阶频段 (峰形 EQ)
    ///
    /// 使用峰形 EQ 公式计算系数:
    /// A = 10^(gain_db/40)
    /// w0 = 2*pi*frequency/sample_rate
    /// alpha = sin(w0) / (2*Q)
    /// b0 = 1 + alpha*A, b1 = -2*cos(w0), b2 = 1 - alpha*A
    /// a0 = 1 + alpha/A, a1 = -2*cos(w0), a2 = 1 - alpha/A
    /// 然后除以 a0 归一化
    fn new(frequency: f64, gain_db: f64, q: f64, sample_rate: u32, channels: usize) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * frequency / sample_rate as f64;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();

        let mut b0 = 1.0 + alpha * a;
        let mut b1 = -2.0 * cos_w0;
        let mut b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let mut a1 = -2.0 * cos_w0;
        let mut a2 = 1.0 - alpha / a;

        b0 /= a0;
        b1 /= a0;
        b2 /= a0;
        a1 /= a0;
        a2 /= a0;

        let state = vec![[0.0; 4]; channels];

        Self {
            frequency,
            gain_db,
            q,
            b0,
            b1,
            b2,
            a1,
            a2,
            state,
            sample_rate,
        }
    }

    /// 确保有足够的声道状态
    fn ensure_channels(&mut self, channels: usize) {
        if self.state.len() < channels {
            self.state.resize(channels, [0.0; 4]);
        }
    }

    /// 处理单个采样 (Direct Form I: y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2])
    fn process_sample(&mut self, channel: usize, input: f64) -> f64 {
        self.ensure_channels(channel + 1);
        let s = &mut self.state[channel];
        let (x1, x2, y1, y2) = (s[0], s[1], s[2], s[3]);

        let output = self.b0 * input + self.b1 * x1 + self.b2 * x2 - self.a1 * y1 - self.a2 * y2;

        s[0] = input;
        s[1] = x1;
        s[2] = output;
        s[3] = y1;

        output
    }
}

/// 均衡器滤镜
pub struct EqualizerFilter {
    bands: Vec<BiquadBand>,
    output: Option<Frame>,
}

impl EqualizerFilter {
    /// 创建空均衡器 (直通)
    pub fn new() -> Self {
        Self {
            bands: Vec::new(),
            output: None,
        }
    }

    /// 添加参数 EQ 频段
    pub fn add_band(&mut self, frequency: f64, gain_db: f64, q: f64) -> &mut Self {
        // 采样率和声道数在首次处理时确定
        self.bands
            .push(BiquadBand::new(frequency, gain_db, q, 44100, 1));
        self
    }

    /// 处理 F32 音频
    fn process_f32(&mut self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let mut out = frame.clone();
        let channels = frame.channel_layout.channels as usize;
        let sample_rate = frame.sample_rate;

        // 更新各频段的采样率和声道数
        for band in &mut self.bands {
            if band.sample_rate != sample_rate {
                *band =
                    BiquadBand::new(band.frequency, band.gain_db, band.q, sample_rate, channels);
            } else {
                band.ensure_channels(channels);
            }
        }

        let is_planar = frame.sample_format.is_planar();

        if is_planar {
            for (ch, plane) in out.data.iter_mut().enumerate() {
                let samples: &mut [f32] = cast_slice_mut(plane);
                for s in samples.iter_mut() {
                    let mut v = *s as f64;
                    for band in &mut self.bands {
                        v = band.process_sample(ch, v);
                    }
                    *s = v as f32;
                }
            }
        } else {
            let samples: &mut [f32] = cast_slice_mut(&mut out.data[0]);
            let n_channels = channels;
            for (i, s) in samples.iter_mut().enumerate() {
                let ch = i % n_channels;
                let mut v = *s as f64;
                for band in &mut self.bands {
                    v = band.process_sample(ch, v);
                }
                *s = v as f32;
            }
        }

        Ok(out)
    }

    /// 处理 S16 音频 (转换为 F32 处理后再转回)
    fn process_s16(&mut self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let mut out = frame.clone();
        let channels = frame.channel_layout.channels as usize;
        let sample_rate = frame.sample_rate;

        for band in &mut self.bands {
            if band.sample_rate != sample_rate {
                *band =
                    BiquadBand::new(band.frequency, band.gain_db, band.q, sample_rate, channels);
            } else {
                band.ensure_channels(channels);
            }
        }

        let is_planar = frame.sample_format.is_planar();

        if is_planar {
            for (ch, plane) in out.data.iter_mut().enumerate() {
                let samples: &mut [i16] = cast_slice_mut(plane);
                for s in samples.iter_mut() {
                    let mut v = *s as f64 / i16::MAX as f64;
                    for band in &mut self.bands {
                        v = band.process_sample(ch, v);
                    }
                    *s = (v * i16::MAX as f64)
                        .round()
                        .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                }
            }
        } else {
            let samples: &mut [i16] = cast_slice_mut(&mut out.data[0]);
            let n_channels = channels;
            for (i, s) in samples.iter_mut().enumerate() {
                let ch = i % n_channels;
                let mut v = *s as f64 / i16::MAX as f64;
                for band in &mut self.bands {
                    v = band.process_sample(ch, v);
                }
                *s = (v * i16::MAX as f64)
                    .round()
                    .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            }
        }

        Ok(out)
    }
}

impl Default for EqualizerFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Filter for EqualizerFilter {
    fn name(&self) -> &str {
        "equalizer"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Audio(af) => {
                let result = match af.sample_format {
                    SampleFormat::F32 | SampleFormat::F32p => self.process_f32(af)?,
                    SampleFormat::S16 | SampleFormat::S16p => self.process_s16(af)?,
                    _ => {
                        return Err(TaoError::Unsupported(format!(
                            "equalizer 滤镜不支持采样格式 {:?}",
                            af.sample_format,
                        )));
                    }
                };
                self.output = Some(Frame::Audio(result));
                Ok(())
            }
            Frame::Video(_) => Err(TaoError::InvalidArgument(
                "equalizer 滤镜仅支持音频帧".into(),
            )),
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

    fn make_f32_frame(samples: &[f32], sample_rate: u32) -> Frame {
        let mut data = Vec::with_capacity(samples.len() * 4);
        for &s in samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        Frame::Audio(AudioFrame {
            data: vec![data],
            nb_samples: samples.len() as u32,
            sample_rate,
            sample_format: SampleFormat::F32,
            channel_layout: ChannelLayout::from_channels(1),
            pts: 0,
            time_base: Rational::new(1, sample_rate as i32),
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
    fn test_empty_equalizer() {
        let mut filter = EqualizerFilter::new();
        let input = make_f32_frame(&[0.5, -0.3, 1.0], 44100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - (-0.3)).abs() < 0.001);
        assert!((samples[2] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_single_band_unity_gain() {
        let mut filter = EqualizerFilter::new();
        filter.add_band(1000.0, 0.0, 1.0);
        let input = make_f32_frame(&[0.5, -0.3, 1.0], 44100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert!((samples[0] - 0.5).abs() < 0.01);
        assert!((samples[1] - (-0.3)).abs() < 0.01);
        assert!((samples[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_biquad_coefficients() {
        let band = BiquadBand::new(1000.0, 0.0, 1.0, 44100, 1);
        // 0 dB 增益时 A=1, 归一化后 b0=1, 系数应满足稳定性 (a1, a2 在单位圆内)
        assert!((band.b0 - 1.0).abs() < 0.01, "b0 应接近 1");
        assert!(band.b1 < 0.0, "b1 应为负");
        assert!(
            band.a1.abs() < 2.0 && band.a2.abs() < 2.0,
            "系数应满足稳定性"
        );
    }

    #[test]
    fn test_process_silence() {
        let mut filter = EqualizerFilter::new();
        filter.add_band(1000.0, 6.0, 1.0);
        let input = make_f32_frame(&[0.0, 0.0, 0.0], 44100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        for &s in &samples {
            assert!(s.abs() < 0.0001, "静音通过 EQ 应仍为静音, 得到 {}", s);
        }
    }

    #[test]
    fn test_multi_band() {
        let mut filter = EqualizerFilter::new();
        filter.add_band(100.0, 3.0, 1.0);
        filter.add_band(1000.0, -3.0, 1.0);
        filter.add_band(10000.0, 0.0, 1.0);
        let input = make_f32_frame(&[0.5, -0.3, 1.0], 44100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let samples = extract_f32(&output);
        assert_eq!(samples.len(), 3);
    }
}
