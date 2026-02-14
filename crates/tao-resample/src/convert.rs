//! 采样格式和声道布局转换.

use tao_core::{SampleFormat, TaoError, TaoResult};

/// 采样格式转换
///
/// 将交错格式的音频数据从一种采样格式转换为另一种.
/// 内部通过 f64 中间格式完成转换, 确保精度.
///
/// # 参数
/// - `input`: 输入数据 (交错格式)
/// - `src_format`: 源采样格式
/// - `dst_format`: 目标采样格式
/// - `nb_samples`: 每声道采样数
/// - `channels`: 声道数
pub fn convert_samples(
    input: &[u8],
    src_format: SampleFormat,
    dst_format: SampleFormat,
    nb_samples: usize,
    channels: usize,
) -> TaoResult<Vec<u8>> {
    let total = nb_samples * channels;
    let src_bps = src_format.bytes_per_sample() as usize;
    let dst_bps = dst_format.bytes_per_sample() as usize;

    if src_bps == 0 || dst_bps == 0 {
        return Err(TaoError::InvalidArgument("无效的采样格式".to_string()));
    }

    let expected_len = total * src_bps;
    if input.len() < expected_len {
        return Err(TaoError::InvalidArgument(format!(
            "数据不足: 期望 {expected_len} 字节, 实际 {} 字节",
            input.len()
        )));
    }

    let mut output = Vec::with_capacity(total * dst_bps);

    for i in 0..total {
        let offset = i * src_bps;
        let sample_f64 = decode_sample(&input[offset..offset + src_bps], src_format)?;
        encode_sample(sample_f64, dst_format, &mut output)?;
    }

    Ok(output)
}

/// 声道混合
///
/// 将交错格式的音频数据从一种声道布局转换为另一种.
///
/// 支持的转换:
/// - 单声道 → 立体声: 复制
/// - 立体声 → 单声道: 取平均
/// - N 声道 → M 声道 (N > M): 取前 M 声道的加权平均
/// - N 声道 → M 声道 (N < M): 复制到前 N 声道, 其余填零
pub fn mix_channels(
    input: &[u8],
    format: SampleFormat,
    nb_samples: usize,
    src_channels: usize,
    dst_channels: usize,
) -> TaoResult<Vec<u8>> {
    if src_channels == dst_channels {
        return Ok(input.to_vec());
    }

    let bps = format.bytes_per_sample() as usize;
    if bps == 0 {
        return Err(TaoError::InvalidArgument("无效的采样格式".to_string()));
    }

    let src_frame_size = src_channels * bps;
    let dst_frame_size = dst_channels * bps;
    let mut output = Vec::with_capacity(nb_samples * dst_frame_size);

    for s in 0..nb_samples {
        let src_offset = s * src_frame_size;

        // 解码所有源声道样本为 f64
        let mut src_samples = Vec::with_capacity(src_channels);
        for ch in 0..src_channels {
            let offset = src_offset + ch * bps;
            let val = decode_sample(&input[offset..offset + bps], format)?;
            src_samples.push(val);
        }

        // 混合到目标声道
        for dst_ch in 0..dst_channels {
            let val = if src_channels == 1 && dst_channels >= 2 {
                // 单声道 → 多声道: 复制到所有声道
                src_samples[0]
            } else if src_channels >= 2 && dst_channels == 1 {
                // 多声道 → 单声道: 所有声道取平均
                let sum: f64 = src_samples.iter().sum();
                sum / src_channels as f64
            } else if dst_ch < src_channels {
                // 对应声道直接映射
                src_samples[dst_ch]
            } else {
                // 额外声道填静音
                0.0
            };

            encode_sample(val, format, &mut output)?;
        }
    }

    Ok(output)
}

/// 将原始字节解码为归一化 f64 样本 (-1.0 ~ 1.0)
fn decode_sample(data: &[u8], format: SampleFormat) -> TaoResult<f64> {
    let base = format.to_interleaved();
    match base {
        SampleFormat::U8 => Ok((data[0] as f64 - 128.0) / 128.0),
        SampleFormat::S16 => {
            let v = i16::from_le_bytes([data[0], data[1]]);
            Ok(v as f64 / 32768.0)
        }
        SampleFormat::S32 => {
            let v = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            Ok(v as f64 / 2_147_483_648.0)
        }
        SampleFormat::F32 => {
            let v = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            Ok(v as f64)
        }
        SampleFormat::F64 => {
            let v = f64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            Ok(v)
        }
        _ => Err(TaoError::Unsupported(format!("不支持的采样格式: {format}"))),
    }
}

/// 将归一化 f64 样本编码为原始字节
fn encode_sample(value: f64, format: SampleFormat, output: &mut Vec<u8>) -> TaoResult<()> {
    let base = format.to_interleaved();
    match base {
        SampleFormat::U8 => {
            let v = ((value * 128.0) + 128.0).round().clamp(0.0, 255.0) as u8;
            output.push(v);
        }
        SampleFormat::S16 => {
            let v = (value * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            output.extend_from_slice(&v.to_le_bytes());
        }
        SampleFormat::S32 => {
            let v = (value * 2_147_483_648.0)
                .round()
                .clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
            output.extend_from_slice(&v.to_le_bytes());
        }
        SampleFormat::F32 => {
            let v = value as f32;
            output.extend_from_slice(&v.to_le_bytes());
        }
        SampleFormat::F64 => {
            output.extend_from_slice(&value.to_le_bytes());
        }
        _ => {
            return Err(TaoError::Unsupported(format!("不支持的采样格式: {format}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_s16_to_f32() {
        // 32767 (max S16) -> ~0.99997
        let input = 32767i16.to_le_bytes().to_vec();
        let result = convert_samples(&input, SampleFormat::S16, SampleFormat::F32, 1, 1).unwrap();
        let f = f32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        assert!((f - (32767.0_f64 / 32768.0) as f32).abs() < 0.001);
    }

    #[test]
    fn test_convert_f32_to_s16() {
        let input = 1.0f32.to_le_bytes().to_vec();
        let result = convert_samples(&input, SampleFormat::F32, SampleFormat::S16, 1, 1).unwrap();
        let v = i16::from_le_bytes([result[0], result[1]]);
        assert_eq!(v, 32767); // 1.0 * 32768 = 32768, clamped to 32767
    }

    #[test]
    fn test_convert_u8_to_s16() {
        // U8 128 = 0.0 -> S16 0
        let input = vec![128u8];
        let result = convert_samples(&input, SampleFormat::U8, SampleFormat::S16, 1, 1).unwrap();
        let v = i16::from_le_bytes([result[0], result[1]]);
        assert_eq!(v, 0);

        // U8 255 ≈ 0.992 -> S16 ~32512
        let input = vec![255u8];
        let result = convert_samples(&input, SampleFormat::U8, SampleFormat::S16, 1, 1).unwrap();
        let v = i16::from_le_bytes([result[0], result[1]]);
        assert!(v > 32000);
    }

    #[test]
    fn test_convert_s16_to_s32() {
        let input = 16384i16.to_le_bytes().to_vec();
        let result = convert_samples(&input, SampleFormat::S16, SampleFormat::S32, 1, 1).unwrap();
        let v = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        // 16384/32768 * 2^31 ≈ 1073741824
        assert!((v - 1_073_741_824).abs() < 256);
    }

    #[test]
    fn test_mix_mono_to_stereo() {
        let input = 1000i16.to_le_bytes().to_vec();
        let result = mix_channels(&input, SampleFormat::S16, 1, 1, 2).unwrap();
        assert_eq!(result.len(), 4);
        let l = i16::from_le_bytes([result[0], result[1]]);
        let r = i16::from_le_bytes([result[2], result[3]]);
        assert_eq!(l, 1000);
        assert_eq!(r, 1000);
    }

    #[test]
    fn test_mix_stereo_to_mono() {
        let mut input = Vec::new();
        input.extend_from_slice(&2000i16.to_le_bytes());
        input.extend_from_slice(&4000i16.to_le_bytes());
        let result = mix_channels(&input, SampleFormat::S16, 1, 2, 1).unwrap();
        assert_eq!(result.len(), 2);
        let mono = i16::from_le_bytes([result[0], result[1]]);
        // (2000 + 4000) / 2 = 3000
        assert!((mono - 3000).abs() <= 1);
    }
}
