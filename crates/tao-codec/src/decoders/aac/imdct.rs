#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn from_polar(radius: f64, angle: f64) -> Self {
        Self {
            re: radius * angle.cos(),
            im: radius * angle.sin(),
        }
    }

    fn add(self, rhs: Self) -> Self {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }

    fn sub(self, rhs: Self) -> Self {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }

    fn mul(self, rhs: Self) -> Self {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

fn fft_in_place(data: &mut [Complex]) {
    let n = data.len();
    assert!(n.is_power_of_two(), "FFT 长度必须是 2 的幂");
    if n <= 1 {
        return;
    }

    // 位逆序重排.
    let mut j = 0usize;
    for i in 1..(n - 1) {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }

    // Radix-2 DIT 蝶形.
    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        let w_step = Complex::from_polar(1.0, angle);
        for start in (0..n).step_by(len) {
            let mut w = Complex { re: 1.0, im: 0.0 };
            let half = len / 2;
            for i in 0..half {
                let u = data[start + i];
                let v = w.mul(data[start + i + half]);
                data[start + i] = u.add(v);
                data[start + i + half] = u.sub(v);
                w = w.mul(w_step);
            }
        }
        len <<= 1;
    }
}

fn imdct_reference(spectrum: &[f32]) -> Vec<f32> {
    let n = spectrum.len();
    let n2 = 2 * n;
    let mut output = vec![0.0f32; n2];
    if n == 0 || spectrum.iter().all(|&s| s == 0.0) {
        return output;
    }

    let scale = 2.0 / n as f64;
    let half_n = n as f64 / 2.0;

    for (i, out_sample) in output.iter_mut().enumerate() {
        let mut sum = 0.0f64;
        let n_plus_half = i as f64 + 0.5 + half_n;
        for (k, &spec_val) in spectrum.iter().enumerate() {
            if spec_val == 0.0 {
                continue;
            }
            let k_plus_half = k as f64 + 0.5;
            let angle = std::f64::consts::PI / n as f64 * n_plus_half * k_plus_half;
            sum += spec_val as f64 * angle.cos();
        }
        *out_sample = (sum * scale) as f32;
    }
    output
}

fn imdct_fft(spectrum: &[f32]) -> Vec<f32> {
    let n = spectrum.len();
    let n2 = 2 * n;
    let mut output = vec![0.0f32; n2];
    if n == 0 || spectrum.iter().all(|&s| s == 0.0) {
        return output;
    }

    // 通过 2N 点复数 FFT 计算 DCT-IV, 再映射为 IMDCT.
    // DCT-IV 恒等式:
    // S(m) = Re{ exp(-j*pi*m/(2N)) * FFT_2N(y)[m] }
    // y[n] = X[n] * exp(-j*pi*(2n+1)/(4N)), y[n>=N]=0
    // IMDCT: x[i] = 2/N * S(i + N/2)
    let n_f64 = n as f64;
    let mut y = vec![Complex::default(); n2];
    for (k, &val) in spectrum.iter().enumerate() {
        let angle = -std::f64::consts::PI * (2.0 * k as f64 + 1.0) / (4.0 * n_f64);
        y[k] = Complex::from_polar(val as f64, angle);
    }
    fft_in_place(&mut y);

    let scale = 2.0 / n_f64;
    let half_n = n / 2;
    for i in 0..n2 {
        let mut m = i + half_n;
        let mut sign = 1.0f64;
        if m >= n2 {
            m -= n2;
            sign = -1.0;
        }
        let angle = -std::f64::consts::PI * m as f64 / (2.0 * n_f64);
        let twiddle = Complex::from_polar(1.0, angle);
        let value = twiddle.mul(y[m]).re;
        output[i] = (sign * scale * value) as f32;
    }

    output
}

pub(super) fn imdct(spectrum: &[f32]) -> Vec<f32> {
    if spectrum.len().is_power_of_two() {
        imdct_fft(spectrum)
    } else {
        imdct_reference(spectrum)
    }
}

/// 1024 点 IMDCT (输入 1024 频谱系数, 输出 2048 时域样本)
pub(super) fn imdct_1024(spectrum: &[f32]) -> Vec<f32> {
    imdct(spectrum)
}

/// 128 点 IMDCT (输入 128 频谱系数, 输出 256 时域样本)
pub(super) fn imdct_128(spectrum: &[f32]) -> Vec<f32> {
    imdct(spectrum)
}

/// 合成 EIGHT_SHORT_SEQUENCE 的 2048 点窗后信号
pub(super) fn synthesize_short_windows(
    spectrum: &[f32],
    prev_window_shape: u8,
    curr_window_shape: u8,
    short_sine_window: &[f32],
    short_kbd_window: &[f32],
) -> Vec<f32> {
    let mut output = vec![0.0f32; 2048];
    let short_prev = pick_window(prev_window_shape, short_sine_window, short_kbd_window);
    let short_curr = pick_window(curr_window_shape, short_sine_window, short_kbd_window);
    for win in 0..8 {
        let begin = win * 128;
        let end = begin + 128;
        if end > spectrum.len() {
            break;
        }
        let td = imdct_128(&spectrum[begin..end]);
        let short_left = if win == 0 { short_prev } else { short_curr };
        let write_start = 448 + win * 128;
        for (i, &sample) in td.iter().enumerate() {
            let idx = write_start + i;
            if idx < output.len() {
                let w = if i < 128 {
                    short_left[i]
                } else {
                    short_curr[i]
                };
                output[idx] += sample * w;
            }
        }
    }
    output
}

/// 正弦窗函数 (2048 点)
pub(super) fn apply_sine_window(time_domain: &[f32]) -> Vec<f32> {
    let n = time_domain.len();
    let mut windowed = vec![0.0f32; n];
    for i in 0..n {
        let w = (std::f64::consts::PI / n as f64 * (i as f64 + 0.5)).sin();
        windowed[i] = time_domain[i] * w as f32;
    }
    windowed
}

pub(super) struct AacWindowBank<'a> {
    pub(super) long_sine: &'a [f32],
    pub(super) long_kbd: &'a [f32],
    pub(super) short_sine: &'a [f32],
    pub(super) short_kbd: &'a [f32],
}

/// AAC 长块窗口函数 (ONLY_LONG/LONG_START/LONG_STOP)
pub(super) fn apply_aac_long_window(
    time_domain: &[f32],
    window_sequence: u32,
    prev_window_shape: u8,
    curr_window_shape: u8,
    windows: &AacWindowBank<'_>,
) -> Vec<f32> {
    let n = time_domain.len();
    if n != 2048 {
        return apply_sine_window(time_domain);
    }
    let mut windowed = vec![0.0f32; n];
    let long_prev = pick_window(prev_window_shape, windows.long_sine, windows.long_kbd);
    let long_curr = pick_window(curr_window_shape, windows.long_sine, windows.long_kbd);
    let short_prev = pick_window(prev_window_shape, windows.short_sine, windows.short_kbd);
    let short_curr = pick_window(curr_window_shape, windows.short_sine, windows.short_kbd);

    for i in 0..n {
        let w = match window_sequence {
            1 => {
                // LONG_START_SEQUENCE
                if i < 1024 {
                    long_prev[i]
                } else if i < 1472 {
                    1.0
                } else if i < 1600 {
                    // 与尾部零区衔接, 需使用短窗后半段 (从 1 递减到 0).
                    short_curr[128 + (i - 1472)]
                } else {
                    0.0
                }
            }
            3 => {
                // LONG_STOP_SEQUENCE
                if i < 448 {
                    0.0
                } else if i < 576 {
                    short_prev[i - 448]
                } else if i < 1024 {
                    1.0
                } else {
                    long_curr[i]
                }
            }
            _ => {
                // ONLY_LONG_SEQUENCE
                if i < 1024 { long_prev[i] } else { long_curr[i] }
            }
        };
        windowed[i] = time_domain[i] * w;
    }
    windowed
}

/// 根据窗口形状选择窗表.
pub(super) fn pick_window<'a>(
    shape: u8,
    sine_window: &'a [f32],
    kbd_window: &'a [f32],
) -> &'a [f32] {
    if shape == 1 { kbd_window } else { sine_window }
}

/// 构建 sine 窗.
pub(super) fn build_sine_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| (std::f64::consts::PI / len as f64 * (i as f64 + 0.5)).sin() as f32)
        .collect()
}

/// 构建 KBD 窗.
pub(super) fn build_kbd_window(len: usize, alpha: f64) -> Vec<f32> {
    if len < 2 || len % 2 != 0 {
        return build_sine_window(len);
    }

    // 与 FFmpeg ff_kbd_window_init 一致:
    // 先生成 half 点窗口(对应 FFmpeg 的 n), 再镜像为 full 点窗口.
    let half = len / 2;
    let mut half_window = vec![0.0f32; half];
    let mut temp = vec![0.0f64; half / 2 + 1];
    let n = half as f64;
    let alpha2 = 4.0 * (alpha * std::f64::consts::PI / n).powi(2);

    let mut scale = 0.0f64;
    for i in 0..=half / 2 {
        let tmp = i as f64 * (half - i) as f64 * alpha2;
        let v = bessel_i0(tmp.sqrt());
        temp[i] = v;
        scale += if i != 0 && i != half / 2 { 2.0 * v } else { v };
    }
    scale = 1.0 / (scale + 1.0);

    let mut sum = 0.0f64;
    for i in 0..=half / 2 {
        sum += temp[i];
        half_window[i] = (sum * scale).sqrt() as f32;
    }
    for i in (half / 2 + 1)..half {
        sum += temp[half - i];
        half_window[i] = (sum * scale).sqrt() as f32;
    }

    let mut window = vec![0.0f32; len];
    for i in 0..half {
        window[i] = half_window[i];
        window[len - 1 - i] = half_window[i];
    }
    window
}

/// 第一类修正贝塞尔函数 I0.
pub(super) fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let half = x * 0.5;
    let mut k = 1.0f64;
    loop {
        term *= (half * half) / (k * k);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
        k += 1.0;
        if k > 50.0 {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::{
        AacWindowBank, apply_aac_long_window, bessel_i0, build_kbd_window, build_sine_window,
        imdct_fft, imdct_reference, synthesize_short_windows,
    };

    fn make_pseudo_random_signal(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let v = ((state >> 32) as u32) as f64 / u32::MAX as f64;
            out.push((v * 2.0 - 1.0) as f32);
        }
        out
    }

    fn assert_close(a: &[f32], b: &[f32], tol: f32) {
        assert_eq!(a.len(), b.len(), "长度不一致");
        let mut max_err = 0.0f32;
        for (&lhs, &rhs) in a.iter().zip(b.iter()) {
            let err = (lhs - rhs).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(
            max_err < tol,
            "FFT IMDCT 与参考实现误差超限: max_err={:.9}, tol={:.9}",
            max_err,
            tol
        );
    }

    #[test]
    fn test_imdct_fft_matches_reference_1024() {
        let spectrum = make_pseudo_random_signal(1024, 0x1234_5678_9abc_def0);
        let fft_out = imdct_fft(&spectrum);
        let ref_out = imdct_reference(&spectrum);
        assert_close(&fft_out, &ref_out, 1e-5);
    }

    #[test]
    fn test_imdct_fft_matches_reference_128() {
        let spectrum = make_pseudo_random_signal(128, 0x0fed_cba9_8765_4321);
        let fft_out = imdct_fft(&spectrum);
        let ref_out = imdct_reference(&spectrum);
        assert_close(&fft_out, &ref_out, 1e-5);
    }

    fn bessel_i0_reference(x: f64) -> f64 {
        let mut sum = 1.0f64;
        let mut term = 1.0f64;
        let half = x * 0.5;
        let mut k = 1.0f64;
        loop {
            term *= (half * half) / (k * k);
            sum += term;
            if term < 1e-15 * sum {
                break;
            }
            k += 1.0;
            if k > 200.0 {
                break;
            }
        }
        sum
    }

    fn build_kbd_window_ffmpeg_reference(len: usize, alpha: f64) -> Vec<f32> {
        let half = len / 2;
        let mut half_window = vec![0.0f32; half];
        let mut temp = vec![0.0f64; half / 2 + 1];
        let n = half as f64;
        let alpha2 = 4.0 * (alpha * std::f64::consts::PI / n).powi(2);

        let mut scale = 0.0f64;
        for i in 0..=half / 2 {
            let tmp = i as f64 * (half - i) as f64 * alpha2;
            let v = bessel_i0_reference(tmp.sqrt());
            temp[i] = v;
            scale += if i != 0 && i != half / 2 { 2.0 * v } else { v };
        }
        scale = 1.0 / (scale + 1.0);

        let mut sum = 0.0f64;
        for i in 0..=half / 2 {
            sum += temp[i];
            half_window[i] = (sum * scale).sqrt() as f32;
        }
        for i in (half / 2 + 1)..half {
            sum += temp[half - i];
            half_window[i] = (sum * scale).sqrt() as f32;
        }

        let mut window = vec![0.0f32; len];
        for i in 0..half {
            window[i] = half_window[i];
            window[len - 1 - i] = half_window[i];
        }
        window
    }

    #[test]
    fn test_kbd_window_matches_ffmpeg_reference_2048() {
        let ours = build_kbd_window(2048, 4.0);
        let reference = build_kbd_window_ffmpeg_reference(2048, 4.0);
        assert_close(&ours, &reference, 1e-6);
    }

    #[test]
    fn test_kbd_window_matches_ffmpeg_reference_256() {
        let ours = build_kbd_window(256, 6.0);
        let reference = build_kbd_window_ffmpeg_reference(256, 6.0);
        assert_close(&ours, &reference, 1e-6);
    }

    #[test]
    fn test_sine_window_precision() {
        let win = build_sine_window(2048);
        for (i, &v) in win.iter().enumerate() {
            let reference = (std::f64::consts::PI / 2048.0 * (i as f64 + 0.5)).sin() as f32;
            let err = (v - reference).abs();
            assert!(err < 1e-7, "sine 窗点值误差超限: idx={}, err={:.9}", i, err);
        }
    }

    #[test]
    fn test_bessel_i0_precision_anchor_points() {
        // 核心锚点用于防止 KBD 窗回归.
        let points = [0.0f64, 0.1, 1.0, 3.5, 6.0];
        for &x in &points {
            let ours = bessel_i0(x);
            let reference = bessel_i0_reference(x);
            let err = (ours - reference).abs();
            assert!(
                err < 1e-12,
                "bessel_i0 精度回归: x={:.3}, ours={:.15}, ref={:.15}, err={:.3e}",
                x,
                ours,
                reference,
                err
            );
        }
    }

    fn build_window_bank() -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
        let long_sine = build_sine_window(2048);
        let long_kbd = build_kbd_window(2048, 4.0);
        let short_sine = build_sine_window(256);
        let short_kbd = build_kbd_window(256, 6.0);
        (long_sine, long_kbd, short_sine, short_kbd)
    }

    fn apply_aac_long_window_reference(
        time_domain: &[f32],
        window_sequence: u32,
        prev_window_shape: u8,
        curr_window_shape: u8,
        long_sine: &[f32],
        long_kbd: &[f32],
        short_sine: &[f32],
        short_kbd: &[f32],
    ) -> Vec<f32> {
        let long_prev = if prev_window_shape == 1 {
            long_kbd
        } else {
            long_sine
        };
        let long_curr = if curr_window_shape == 1 {
            long_kbd
        } else {
            long_sine
        };
        let short_prev = if prev_window_shape == 1 {
            short_kbd
        } else {
            short_sine
        };
        let short_curr = if curr_window_shape == 1 {
            short_kbd
        } else {
            short_sine
        };

        let mut out = vec![0.0f32; 2048];
        for i in 0..2048 {
            let w = match window_sequence {
                1 => {
                    if i < 1024 {
                        long_prev[i]
                    } else if i < 1472 {
                        1.0
                    } else if i < 1600 {
                        short_curr[128 + (i - 1472)]
                    } else {
                        0.0
                    }
                }
                3 => {
                    if i < 448 {
                        0.0
                    } else if i < 576 {
                        short_prev[i - 448]
                    } else if i < 1024 {
                        1.0
                    } else {
                        long_curr[i]
                    }
                }
                _ => {
                    if i < 1024 {
                        long_prev[i]
                    } else {
                        long_curr[i]
                    }
                }
            };
            out[i] = time_domain[i] * w;
        }
        out
    }

    fn synthesize_short_windows_reference(
        spectrum: &[f32],
        prev_window_shape: u8,
        curr_window_shape: u8,
        short_sine: &[f32],
        short_kbd: &[f32],
    ) -> Vec<f32> {
        let short_prev = if prev_window_shape == 1 {
            short_kbd
        } else {
            short_sine
        };
        let short_curr = if curr_window_shape == 1 {
            short_kbd
        } else {
            short_sine
        };

        let mut out = vec![0.0f32; 2048];
        for win in 0..8 {
            let begin = win * 128;
            let end = begin + 128;
            let td = imdct_reference(&spectrum[begin..end]);
            let short_left = if win == 0 { short_prev } else { short_curr };
            let write_start = 448 + win * 128;
            for (i, &sample) in td.iter().enumerate() {
                let idx = write_start + i;
                if idx < out.len() {
                    let w = if i < 128 {
                        short_left[i]
                    } else {
                        short_curr[i]
                    };
                    out[idx] += sample * w;
                }
            }
        }
        out
    }

    #[test]
    fn test_apply_aac_long_window_matches_reference_long_start() {
        let (long_sine, long_kbd, short_sine, short_kbd) = build_window_bank();
        let time_domain = make_pseudo_random_signal(2048, 0xaaaa_bbbb_cccc_dddd);
        let windows = AacWindowBank {
            long_sine: &long_sine,
            long_kbd: &long_kbd,
            short_sine: &short_sine,
            short_kbd: &short_kbd,
        };
        let ours = apply_aac_long_window(&time_domain, 1, 0, 1, &windows);
        let reference = apply_aac_long_window_reference(
            &time_domain,
            1,
            0,
            1,
            &long_sine,
            &long_kbd,
            &short_sine,
            &short_kbd,
        );
        assert_close(&ours, &reference, 1e-7);
    }

    #[test]
    fn test_apply_aac_long_window_matches_reference_long_stop() {
        let (long_sine, long_kbd, short_sine, short_kbd) = build_window_bank();
        let time_domain = make_pseudo_random_signal(2048, 0x1111_2222_3333_4444);
        let windows = AacWindowBank {
            long_sine: &long_sine,
            long_kbd: &long_kbd,
            short_sine: &short_sine,
            short_kbd: &short_kbd,
        };
        let ours = apply_aac_long_window(&time_domain, 3, 1, 0, &windows);
        let reference = apply_aac_long_window_reference(
            &time_domain,
            3,
            1,
            0,
            &long_sine,
            &long_kbd,
            &short_sine,
            &short_kbd,
        );
        assert_close(&ours, &reference, 1e-7);
    }

    #[test]
    fn test_synthesize_short_windows_matches_reference() {
        let (_, _, short_sine, short_kbd) = build_window_bank();
        let spectrum = make_pseudo_random_signal(1024, 0x9999_8888_7777_6666);
        let ours = synthesize_short_windows(&spectrum, 0, 1, &short_sine, &short_kbd);
        let reference =
            synthesize_short_windows_reference(&spectrum, 0, 1, &short_sine, &short_kbd);
        assert_close(&ours, &reference, 1e-5);
    }
}
