//! EBU R128 响度归一化滤镜.
//!
//! 实现简化的 EBU R128 响度归一化, 测量音频的积分响度并应用增益以达到目标 LUFS 电平.

use tao_codec::frame::{AudioFrame, Frame};
use tao_core::{SampleFormat, TaoError, TaoResult};

use crate::Filter;

/// EBU R128 响度归一化滤镜
pub struct LoudnormFilter {
    /// 目标积分响度 (默认 -23.0 LUFS, EBU R128 标准)
    target_lufs: f64,
    /// 最大真峰值 (dBTP, 默认 -1.0)
    max_true_peak: f64,
    /// 内部状态: 已测量的响度
    measured_loudness: f64,
    /// 内部状态: 应用的增益
    gain: f64,
    /// 运行累加和 (用于响度测量)
    running_sum: f64,
    /// 运行采样计数
    running_count: u64,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl LoudnormFilter {
    /// 使用指定参数创建
    pub fn new(target_lufs: f64, max_true_peak: f64) -> Self {
        Self {
            target_lufs,
            max_true_peak,
            measured_loudness: f64::NEG_INFINITY,
            gain: 1.0,
            running_sum: 0.0,
            running_count: 0,
            output: None,
        }
    }

    /// 使用简化的 K 加权计算瞬时响度 (LUFS)
    ///
    /// 公式: LUFS = -0.691 + 10 * log10(mean_square)
    fn measure_loudness_f32(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return f64::NEG_INFINITY;
        }
        let mean_square = samples
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum::<f64>()
            / samples.len() as f64;
        const EPSILON: f64 = 1e-10;
        let mean_square = mean_square.max(EPSILON);
        -0.691 + 10.0 * mean_square.log10()
    }

    /// 根据测量响度计算线性增益
    fn calculate_gain(&self, measured_lufs: f64) -> f64 {
        if measured_lufs.is_infinite() || measured_lufs.is_nan() {
            return 1.0;
        }
        let db_gain = self.target_lufs - measured_lufs;
        10.0_f64.powf(db_gain / 20.0)
    }

    /// 将 dBTP 转换为线性峰值限制
    fn peak_limit_linear(&self) -> f64 {
        10.0_f64.powf(self.max_true_peak / 20.0)
    }

    /// 处理 F32 音频帧
    fn process_f32(&mut self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let mut out = frame.clone();

        for plane in &mut out.data {
            let samples: &mut [f32] = cast_slice_mut(plane);

            // 测量响度
            let loudness = Self::measure_loudness_f32(samples);
            self.running_sum += samples
                .iter()
                .map(|&s| (s as f64) * (s as f64))
                .sum::<f64>();
            self.running_count += samples.len() as u64;

            // 使用当前帧响度计算增益 (简化实现)
            let frame_loudness = if self.running_count > 0 {
                let mean_sq = self.running_sum / self.running_count as f64;
                let eps = 1e-10;
                -0.691 + 10.0 * (mean_sq.max(eps)).log10()
            } else {
                loudness
            };
            self.measured_loudness = frame_loudness;
            let gain = self.calculate_gain(frame_loudness);
            self.gain = gain;

            let peak_limit = self.peak_limit_linear();

            for s in samples.iter_mut() {
                let scaled = (*s as f64 * gain) as f32;
                *s = scaled.clamp(-peak_limit as f32, peak_limit as f32);
            }
        }

        Ok(out)
    }

    /// 处理 S16 音频帧 (内部转换为 F32 测量, 应用增益后转回)
    fn process_s16(&mut self, frame: &AudioFrame) -> TaoResult<AudioFrame> {
        let mut out = frame.clone();

        for plane in &mut out.data {
            let samples: &mut [i16] = cast_slice_mut(plane);

            // 转换为 F32 测量响度
            let f32_samples: Vec<f32> = samples
                .iter()
                .map(|&s| s as f32 / i16::MAX as f32)
                .collect();
            let loudness = Self::measure_loudness_f32(&f32_samples);
            self.running_sum += f32_samples
                .iter()
                .map(|&s| (s as f64) * (s as f64))
                .sum::<f64>();
            self.running_count += f32_samples.len() as u64;

            let frame_loudness = if self.running_count > 0 {
                let mean_sq = self.running_sum / self.running_count as f64;
                let eps = 1e-10;
                -0.691 + 10.0 * (mean_sq.max(eps)).log10()
            } else {
                loudness
            };
            self.measured_loudness = frame_loudness;
            let gain = self.calculate_gain(frame_loudness);
            self.gain = gain;

            let peak_limit = self.peak_limit_linear();
            let s16_max = i16::MAX as f64;

            for s in samples.iter_mut() {
                let scaled = *s as f64 * gain;
                let limited = scaled.clamp(-peak_limit * s16_max, peak_limit * s16_max);
                *s = limited.round() as i16;
            }
        }

        Ok(out)
    }
}

impl Default for LoudnormFilter {
    fn default() -> Self {
        Self::new(-23.0, -1.0)
    }
}

impl Filter for LoudnormFilter {
    fn name(&self) -> &str {
        "loudnorm"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Audio(af) => {
                let result = match af.sample_format {
                    SampleFormat::F32 | SampleFormat::F32p => self.process_f32(af)?,
                    SampleFormat::S16 | SampleFormat::S16p => self.process_s16(af)?,
                    _ => {
                        return Err(TaoError::Unsupported(format!(
                            "loudnorm 滤镜不支持采样格式 {:?}",
                            af.sample_format,
                        )));
                    }
                };
                self.output = Some(Frame::Audio(result));
                Ok(())
            }
            Frame::Video(_) => Err(TaoError::InvalidArgument(
                "loudnorm 滤镜仅支持音频帧".into(),
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
    fn test_silence_loudness() {
        let samples = vec![0.0f32; 44100];
        let lufs = LoudnormFilter::measure_loudness_f32(&samples);
        // 静音应有非常低的 LUFS (接近 -inf, 我们使用 epsilon 所以会是一个有限值)
        assert!(lufs < -60.0, "静音响度应很低, 得到 {}", lufs);
    }

    #[test]
    fn test_gain_calculation() {
        let filter = LoudnormFilter::new(-23.0, -1.0);
        let gain = filter.calculate_gain(-33.0);
        // -23 - (-33) = 10 dB, 10^(10/20) ≈ 3.162
        assert!(
            (gain - 3.162).abs() < 0.1,
            "10dB 增益应约 3.16, 得到 {}",
            gain
        );
    }

    #[test]
    fn test_apply_gain_f32() {
        // 创建较安静的 sine 波 (约 -20 dB = 0.1 幅度)
        let mut samples = Vec::with_capacity(4410);
        for i in 0..4410 {
            let t = i as f64 * 440.0 * 2.0 * std::f64::consts::PI / 44100.0;
            samples.push((0.1 * t.sin()) as f32);
        }
        let input = make_f32_frame(&samples, 44100);
        let mut filter = LoudnormFilter::new(-23.0, -1.0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let out_samples = extract_f32(&output);
        // 归一化后幅度应显著增大
        let max_before = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let max_after = out_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max_after > max_before, "归一化后幅度应增大");
    }

    #[test]
    fn test_peak_limiting() {
        // 创建大幅度音频, 归一化后可能超过峰值限制
        let samples: Vec<f32> = (0..1000).map(|_| 0.9).collect();
        let input = make_f32_frame(&samples, 44100);
        let mut filter = LoudnormFilter::new(-23.0, -1.0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        let out_samples = extract_f32(&output);
        let peak_limit = 10.0_f64.powf(-1.0 / 20.0);
        for &s in &out_samples {
            assert!(
                s.abs() <= (peak_limit as f32) + 0.001,
                "样本 {} 超过最大真峰值限制 {}",
                s,
                peak_limit
            );
        }
    }
}
