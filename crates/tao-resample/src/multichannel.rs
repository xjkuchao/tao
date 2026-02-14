//! 多声道混合 (5.1 / 7.1).
//!
//! 提供 5.1/7.1 与立体声之间的下混和上混.
//!
//! 声道顺序 (交错格式):
//! - 5.1: L, R, C, LFE, Ls, Rs (6ch)
//! - 7.1: L, R, C, LFE, Ls, Rs, Lsr, Rsr (8ch)

/// 5.1 下混到立体声的 C/Ls/Rs 系数 (1/sqrt(2) ≈ 0.707)
const DOWNMIX_COEF: f32 = 0.707_106_77;

/// 7.1 侧声道下混系数
const DOWNMIX_SIDE_COEF: f32 = 0.5;

/// 立体声上混到 5.1 的 LFE 衰减
const UPMIX_LFE_COEF: f32 = 0.1;

/// 立体声上混到 5.1 的环绕声道系数
const UPMIX_SURROUND_COEF: f32 = 0.5;

/// 5.1 (6ch) 下混到立体声
///
/// 公式: L' = L + 0.707*C + 0.707*Ls, R' = R + 0.707*C + 0.707*Rs
///
/// # 参数
/// - `input`: 交错 6ch f32 数据, 长度 = nb_samples * 6
/// - `output`: 交错 2ch f32 输出, 长度 = nb_samples * 2
/// - `nb_samples`: 每声道采样数
pub fn downmix_51_to_stereo_f32(input: &[f32], output: &mut [f32], nb_samples: u32) {
    let n = nb_samples as usize;
    let required_in = n * 6;
    let required_out = n * 2;

    if input.len() < required_in || output.len() < required_out {
        return;
    }

    for i in 0..n {
        let off_in = i * 6;
        let l = input[off_in];
        let r = input[off_in + 1];
        let c = input[off_in + 2];
        let _lfe = input[off_in + 3];
        let ls = input[off_in + 4];
        let rs = input[off_in + 5];

        let out_l = l + DOWNMIX_COEF * c + DOWNMIX_COEF * ls;
        let out_r = r + DOWNMIX_COEF * c + DOWNMIX_COEF * rs;

        let off_out = i * 2;
        output[off_out] = out_l;
        output[off_out + 1] = out_r;
    }
}

/// 7.1 (8ch) 下混到立体声
///
/// 公式: L' = L + 0.707*C + 0.707*Ls + 0.5*Lsr, R' = R + 0.707*C + 0.707*Rs + 0.5*Rsr
///
/// # 参数
/// - `input`: 交错 8ch f32 数据, 长度 = nb_samples * 8
/// - `output`: 交错 2ch f32 输出, 长度 = nb_samples * 2
/// - `nb_samples`: 每声道采样数
pub fn downmix_71_to_stereo_f32(input: &[f32], output: &mut [f32], nb_samples: u32) {
    let n = nb_samples as usize;
    let required_in = n * 8;
    let required_out = n * 2;

    if input.len() < required_in || output.len() < required_out {
        return;
    }

    for i in 0..n {
        let off_in = i * 8;
        let l = input[off_in];
        let r = input[off_in + 1];
        let c = input[off_in + 2];
        let _lfe = input[off_in + 3];
        let ls = input[off_in + 4];
        let rs = input[off_in + 5];
        let lsr = input[off_in + 6];
        let rsr = input[off_in + 7];

        let out_l = l + DOWNMIX_COEF * c + DOWNMIX_COEF * ls + DOWNMIX_SIDE_COEF * lsr;
        let out_r = r + DOWNMIX_COEF * c + DOWNMIX_COEF * rs + DOWNMIX_SIDE_COEF * rsr;

        let off_out = i * 2;
        output[off_out] = out_l;
        output[off_out + 1] = out_r;
    }
}

/// 立体声上混到 5.1 (6ch)
///
/// 公式: C = 0.5*(L+R), LFE = 0.5*(L+R)*0.1, Ls = L*0.5, Rs = R*0.5
///
/// # 参数
/// - `input`: 交错 2ch f32 数据, 长度 = nb_samples * 2
/// - `output`: 交错 6ch f32 输出, 长度 = nb_samples * 6
/// - `nb_samples`: 每声道采样数
pub fn upmix_stereo_to_51_f32(input: &[f32], output: &mut [f32], nb_samples: u32) {
    let n = nb_samples as usize;
    let required_in = n * 2;
    let required_out = n * 6;

    if input.len() < required_in || output.len() < required_out {
        return;
    }

    for i in 0..n {
        let off_in = i * 2;
        let l = input[off_in];
        let r = input[off_in + 1];

        let c = 0.5 * (l + r);
        let lfe = UPMIX_LFE_COEF * 0.5 * (l + r);
        let ls = UPMIX_SURROUND_COEF * l;
        let rs = UPMIX_SURROUND_COEF * r;

        let off_out = i * 6;
        output[off_out] = l;
        output[off_out + 1] = r;
        output[off_out + 2] = c;
        output[off_out + 3] = lfe;
        output[off_out + 4] = ls;
        output[off_out + 5] = rs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downmix_51_to_stereo() {
        // 6ch 输入, 2ch 输出, 4 个采样
        let input: Vec<f32> = (0..4 * 6).map(|i| i as f32 * 0.1).collect();
        let mut output = vec![0.0f32; 4 * 2];

        downmix_51_to_stereo_f32(&input, &mut output, 4);

        // 第一个采样: L=0, R=0.1, C=0.2, LFE=0.3, Ls=0.4, Rs=0.5
        // out_l = 0 + 0.707*0.2 + 0.707*0.4 = 0 + 0.1414 + 0.2828 ≈ 0.424
        // out_r = 0.1 + 0.707*0.2 + 0.707*0.5 = 0.1 + 0.1414 + 0.3535 ≈ 0.595
        assert!(output[0] > 0.4 && output[0] < 0.45);
        assert!(output[1] > 0.59 && output[1] < 0.6);
        assert_eq!(output.len(), 8);
    }

    #[test]
    fn test_downmix_71_to_stereo() {
        // 8ch 输入, 2ch 输出, 2 个采样
        let input: Vec<f32> = (0..2 * 8).map(|i| i as f32 * 0.1).collect();
        let mut output = vec![0.0f32; 2 * 2];

        downmix_71_to_stereo_f32(&input, &mut output, 2);

        // 第一个采样包含 Lsr(0.6), Rsr(0.7) 的贡献
        assert!(output[0] > 0.4);
        assert!(output[1] > 0.6);
    }

    #[test]
    fn test_upmix_stereo_to_51() {
        // 2ch 输入, 6ch 输出, 3 个采样
        let input = vec![1.0f32, 0.0f32, 0.5f32, 0.5f32, -0.5f32, -0.5f32];
        let mut output = vec![0.0f32; 3 * 6];

        upmix_stereo_to_51_f32(&input, &mut output, 3);

        // 第一个采样: L=1, R=0
        // C = 0.5, LFE = 0.05, Ls = 0.5, Rs = 0
        assert_eq!(output[0], 1.0);
        assert_eq!(output[1], 0.0);
        assert!((output[2] - 0.5).abs() < 0.001);
        assert!((output[3] - 0.05).abs() < 0.001);
        assert!((output[4] - 0.5).abs() < 0.001);
        assert_eq!(output[5], 0.0);

        // 第二个采样: L=0.5, R=0.5 -> C=0.5, LFE=0.05, Ls=0.25, Rs=0.25
        assert!((output[6] - 0.5).abs() < 0.001);
        assert!((output[7] - 0.5).abs() < 0.001);
        assert!((output[8] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_silence_passthrough() {
        // 静音输入 -> 静音输出
        let input = vec![0.0f32; 10 * 6];
        let mut output = vec![1.0f32; 10 * 2]; // 预填非零

        downmix_51_to_stereo_f32(&input, &mut output, 10);

        for &v in &output {
            assert_eq!(v, 0.0, "静音下混应输出静音");
        }

        // 立体声静音上混到 5.1
        let input = vec![0.0f32; 5 * 2];
        let mut output = vec![1.0f32; 5 * 6];

        upmix_stereo_to_51_f32(&input, &mut output, 5);

        for &v in &output {
            assert_eq!(v, 0.0, "静音上混应输出静音");
        }
    }
}
