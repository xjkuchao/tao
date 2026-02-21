pub(super) fn imdct(spectrum: &[f32]) -> Vec<f32> {
    let n = spectrum.len();
    let n2 = 2 * n;
    let mut output = vec![0.0f32; n2];

    if spectrum.iter().all(|&s| s == 0.0) {
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
    window_shape: u8,
    short_sine_window: &[f32],
    short_kbd_window: &[f32],
) -> Vec<f32> {
    let mut output = vec![0.0f32; 2048];
    let short_window = pick_window(window_shape, short_sine_window, short_kbd_window);
    for win in 0..8 {
        let begin = win * 128;
        let end = begin + 128;
        if end > spectrum.len() {
            break;
        }
        let td = imdct_128(&spectrum[begin..end]);
        let write_start = 448 + win * 128;
        for (i, &sample) in td.iter().enumerate() {
            let idx = write_start + i;
            if idx < output.len() {
                output[idx] += sample * short_window[i];
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

    let half = len / 2;
    let mut proto = vec![0.0f64; half];
    let mut cum = vec![0.0f64; half];
    let half_f = half as f64;

    for (i, slot) in proto.iter_mut().enumerate() {
        let x = (2.0 * i as f64) / half_f - 1.0;
        let arg = alpha * std::f64::consts::PI * (1.0 - x * x).max(0.0).sqrt();
        *slot = bessel_i0(arg);
    }

    let mut running = 0.0f64;
    for (i, &v) in proto.iter().enumerate() {
        running += v;
        cum[i] = running;
    }
    let denom = cum[half - 1].max(f64::EPSILON);

    let mut window = vec![0.0f32; len];
    for i in 0..half {
        let w = (cum[i] / denom).sqrt() as f32;
        window[i] = w;
        window[len - 1 - i] = w;
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
