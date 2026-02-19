//! # tao-resample
//!
//! Tao 多媒体框架音频重采样库.
//!
//! 本 crate 对标 FFmpeg 的 libswresample, 提供:
//! - 采样格式转换 (如 S16 -> F32)
//! - 声道布局转换 (如立体声 -> 单声道)
//! - 采样率转换 (如 44100Hz -> 48000Hz, 线性插值)

mod convert;
mod multichannel;

use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

pub use convert::{convert_samples, mix_channels};
pub use multichannel::{
    downmix_51_to_stereo_f32, downmix_71_to_stereo_f32, upmix_stereo_to_51_f32,
};

/// 重采样上下文
///
/// 配置一次后可多次复用, 用于在不同音频参数之间转换.
pub struct ResampleContext {
    /// 源采样率
    pub src_sample_rate: u32,
    /// 源采样格式
    pub src_sample_format: SampleFormat,
    /// 源声道布局
    pub src_channel_layout: ChannelLayout,
    /// 目标采样率
    pub dst_sample_rate: u32,
    /// 目标采样格式
    pub dst_sample_format: SampleFormat,
    /// 目标声道布局
    pub dst_channel_layout: ChannelLayout,
}

impl ResampleContext {
    /// 创建新的重采样上下文
    pub fn new(
        src_sample_rate: u32,
        src_sample_format: SampleFormat,
        src_channel_layout: ChannelLayout,
        dst_sample_rate: u32,
        dst_sample_format: SampleFormat,
        dst_channel_layout: ChannelLayout,
    ) -> Self {
        Self {
            src_sample_rate,
            src_sample_format,
            src_channel_layout,
            dst_sample_rate,
            dst_sample_format,
            dst_channel_layout,
        }
    }

    /// 是否需要转换 (源和目标参数不同)
    pub fn is_needed(&self) -> bool {
        self.src_sample_rate != self.dst_sample_rate
            || self.src_sample_format != self.dst_sample_format
            || self.src_channel_layout != self.dst_channel_layout
    }

    /// 执行重采样
    ///
    /// 处理交错格式数据. 按以下顺序转换:
    /// 1. 采样格式转换
    /// 2. 声道布局转换
    /// 3. 采样率转换
    ///
    /// # 参数
    /// - `input`: 输入交错格式的原始字节
    /// - `nb_samples`: 输入采样数 (每声道)
    ///
    /// # 返回
    /// 转换后的交错格式字节数据和输出每声道采样数
    pub fn convert(&self, input: &[u8], nb_samples: u32) -> TaoResult<(Vec<u8>, u32)> {
        if !self.is_needed() {
            return Ok((input.to_vec(), nb_samples));
        }

        let src_channels = self.src_channel_layout.channels as usize;
        let dst_channels = self.dst_channel_layout.channels as usize;
        let mut nb = nb_samples;

        // 步骤 1: 采样格式转换
        let mut data = if self.src_sample_format != self.dst_sample_format {
            convert_samples(
                input,
                self.src_sample_format,
                self.dst_sample_format,
                nb as usize,
                src_channels,
            )?
        } else {
            input.to_vec()
        };

        // 当前格式已经是目标格式
        let current_format = self.dst_sample_format;

        // 步骤 2: 声道布局转换
        if self.src_channel_layout != self.dst_channel_layout {
            data = mix_channels(
                &data,
                current_format,
                nb as usize,
                src_channels,
                dst_channels,
            )?;
        }

        // 步骤 3: 采样率转换 (线性插值)
        if self.src_sample_rate != self.dst_sample_rate {
            let (resampled, new_nb) = resample_linear(
                &data,
                current_format,
                nb as usize,
                dst_channels,
                self.src_sample_rate,
                self.dst_sample_rate,
            )?;
            data = resampled;
            nb = new_nb as u32;
        }

        Ok((data, nb))
    }
}

/// 线性插值重采样
fn resample_linear(
    input: &[u8],
    format: SampleFormat,
    nb_samples: usize,
    channels: usize,
    src_rate: u32,
    dst_rate: u32,
) -> TaoResult<(Vec<u8>, usize)> {
    let bps = format.bytes_per_sample() as usize;
    if bps == 0 {
        return Err(TaoError::InvalidArgument("无效的采样格式".to_string()));
    }

    // 计算输出采样数
    let out_samples = ((nb_samples as u64) * (dst_rate as u64)).div_ceil(src_rate as u64);
    let out_samples = out_samples as usize;

    // 先转为 f64 样本以便插值
    let samples = bytes_to_f64(input, format, nb_samples * channels)?;
    let mut output = Vec::with_capacity(out_samples * channels);

    let ratio = src_rate as f64 / dst_rate as f64;
    for i in 0..out_samples {
        let src_pos = i as f64 * ratio;
        let idx0 = src_pos.floor() as usize;
        let frac = src_pos - idx0 as f64;
        let idx1 = (idx0 + 1).min(nb_samples - 1);

        for ch in 0..channels {
            let s0 = samples[idx0 * channels + ch];
            let s1 = samples[idx1 * channels + ch];
            let interpolated = s0 + (s1 - s0) * frac;
            output.push(interpolated);
        }
    }

    let result = f64_to_bytes(&output, format)?;
    Ok((result, out_samples))
}

/// 将原始字节转为 f64 样本 (归一化到 -1.0..1.0 范围)
fn bytes_to_f64(data: &[u8], format: SampleFormat, total_samples: usize) -> TaoResult<Vec<f64>> {
    let bps = format.bytes_per_sample() as usize;
    if data.len() < total_samples * bps {
        return Err(TaoError::InvalidArgument("数据不足".to_string()));
    }

    let mut result = Vec::with_capacity(total_samples);
    for i in 0..total_samples {
        let offset = i * bps;
        let sample = match format.to_interleaved() {
            SampleFormat::U8 => (data[offset] as f64 - 128.0) / 128.0,
            SampleFormat::S16 => {
                let v = i16::from_le_bytes([data[offset], data[offset + 1]]);
                v as f64 / 32768.0
            }
            SampleFormat::S32 => {
                let v = i32::from_le_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]);
                v as f64 / 2_147_483_648.0
            }
            SampleFormat::F32 => {
                let v = f32::from_le_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]);
                v as f64
            }
            SampleFormat::F64 => f64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]),
            _ => {
                return Err(TaoError::Unsupported(format!("不支持的采样格式: {format}")));
            }
        };
        result.push(sample);
    }
    Ok(result)
}

/// 将 f64 样本转回原始字节
fn f64_to_bytes(samples: &[f64], format: SampleFormat) -> TaoResult<Vec<u8>> {
    let bps = format.bytes_per_sample() as usize;
    let mut result = Vec::with_capacity(samples.len() * bps);

    for &s in samples {
        match format.to_interleaved() {
            SampleFormat::U8 => {
                let v = ((s * 128.0) + 128.0).round().clamp(0.0, 255.0) as u8;
                result.push(v);
            }
            SampleFormat::S16 => {
                let v = (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                result.extend_from_slice(&v.to_le_bytes());
            }
            SampleFormat::S32 => {
                let v = (s * 2_147_483_648.0)
                    .round()
                    .clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
                result.extend_from_slice(&v.to_le_bytes());
            }
            SampleFormat::F32 => {
                let v = s as f32;
                result.extend_from_slice(&v.to_le_bytes());
            }
            SampleFormat::F64 => {
                result.extend_from_slice(&s.to_le_bytes());
            }
            _ => {
                return Err(TaoError::Unsupported(format!("不支持的采样格式: {format}")));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_conversion_needed() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::STEREO,
            44100,
            SampleFormat::S16,
            ChannelLayout::STEREO,
        );
        assert!(!ctx.is_needed());

        let data = vec![0u8; 100];
        let (result, nb) = ctx.convert(&data, 25).unwrap();
        assert_eq!(result, data);
        assert_eq!(nb, 25);
    }

    #[test]
    fn test_format_convert_s16_to_f32() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
            44100,
            SampleFormat::F32,
            ChannelLayout::MONO,
        );
        assert!(ctx.is_needed());

        // 0x7FFF = 32767 -> ~1.0
        let input = 32767i16.to_le_bytes().to_vec();
        let (result, nb) = ctx.convert(&input, 1).unwrap();
        assert_eq!(nb, 1);
        assert_eq!(result.len(), 4); // f32 = 4 bytes

        let value = f32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        assert!((value - (32767.0 / 32768.0)).abs() < 0.001);
    }

    #[test]
    fn test_format_convert_f32_to_s16() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::F32,
            ChannelLayout::MONO,
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
        );

        let input_val: f32 = 0.5;
        let input = input_val.to_le_bytes().to_vec();
        let (result, _) = ctx.convert(&input, 1).unwrap();
        assert_eq!(result.len(), 2);

        let value = i16::from_le_bytes([result[0], result[1]]);
        // 0.5 * 32768 = 16384
        assert!((value - 16384).abs() <= 1);
    }

    #[test]
    fn test_channel_convert_mono_to_stereo() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
            44100,
            SampleFormat::S16,
            ChannelLayout::STEREO,
        );

        // 2 个单声道样本
        let mut input = Vec::new();
        input.extend_from_slice(&1000i16.to_le_bytes());
        input.extend_from_slice(&(-2000i16).to_le_bytes());

        let (result, nb) = ctx.convert(&input, 2).unwrap();
        assert_eq!(nb, 2);
        assert_eq!(result.len(), 8); // 2 samples * 2 channels * 2 bytes

        // 第一个样本: L=1000, R=1000
        let l0 = i16::from_le_bytes([result[0], result[1]]);
        let r0 = i16::from_le_bytes([result[2], result[3]]);
        assert_eq!(l0, 1000);
        assert_eq!(r0, 1000);

        // 第二个样本: L=-2000, R=-2000
        let l1 = i16::from_le_bytes([result[4], result[5]]);
        let r1 = i16::from_le_bytes([result[6], result[7]]);
        assert_eq!(l1, -2000);
        assert_eq!(r1, -2000);
    }

    #[test]
    fn test_channel_convert_stereo_to_mono() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::STEREO,
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
        );

        // 1 个立体声样本: L=1000, R=3000
        let mut input = Vec::new();
        input.extend_from_slice(&1000i16.to_le_bytes());
        input.extend_from_slice(&3000i16.to_le_bytes());

        let (result, nb) = ctx.convert(&input, 1).unwrap();
        assert_eq!(nb, 1);
        assert_eq!(result.len(), 2);

        let mono = i16::from_le_bytes([result[0], result[1]]);
        // 平均: (1000 + 3000) / 2 = 2000
        assert!((mono - 2000).abs() <= 1);
    }

    #[test]
    fn test_sample_rate_convert_44100_to_48000() {
        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
            48000,
            SampleFormat::S16,
            ChannelLayout::MONO,
        );

        // 生成 100 个 S16 单声道样本 (简单递增)
        let nb_in = 100;
        let mut input = Vec::with_capacity(nb_in * 2);
        for i in 0..nb_in {
            let v = (i * 100) as i16;
            input.extend_from_slice(&v.to_le_bytes());
        }

        let (result, nb_out) = ctx.convert(&input, nb_in as u32).unwrap();
        // 48000/44100 * 100 ≈ 109 个输出样本
        let expected_out = (nb_in as u64 * 48000).div_ceil(44100) as u32;
        assert_eq!(nb_out, expected_out);
        assert_eq!(result.len(), nb_out as usize * 2);
    }

    #[test]
    fn test_sample_rate_convert_48000_to_44100() {
        let ctx = ResampleContext::new(
            48000,
            SampleFormat::S16,
            ChannelLayout::MONO,
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
        );

        let nb_in = 100;
        let mut input = Vec::with_capacity(nb_in * 2);
        for i in 0..nb_in {
            let v = (i * 100) as i16;
            input.extend_from_slice(&v.to_le_bytes());
        }

        let (result, nb_out) = ctx.convert(&input, nb_in as u32).unwrap();
        // 44100/48000 * 100 ≈ 92
        let expected_out = (nb_in as u64 * 44100).div_ceil(48000) as u32;
        assert_eq!(nb_out, expected_out);
        assert_eq!(result.len(), nb_out as usize * 2);
    }
}
