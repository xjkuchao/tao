/// IMDCT 输出时域样本.
#[derive(Debug, Clone)]
pub(crate) struct TimeDomainBlock {
    pub(crate) channels: Vec<Vec<f32>>,
}

use super::residue::ResidueSpectrum;

/// 将 residue 频谱执行 IMDCT 并应用 Vorbis 窗函数.
pub(crate) fn imdct_from_residue(
    residue: &ResidueSpectrum,
    blocksize: usize,
    window: &[f32],
) -> TimeDomainBlock {
    if blocksize == 0 {
        return TimeDomainBlock {
            channels: vec![Vec::new(); residue.channels.len()],
        };
    }

    let n = blocksize;
    let n2 = n / 2;
    let pi = std::f32::consts::PI;
    let mut channels_td = Vec::with_capacity(residue.channels.len());

    for spectrum in &residue.channels {
        if spectrum.iter().all(|&x| x == 0.0) {
            channels_td.push(vec![0.0; n]);
            continue;
        }

        let mut td = vec![0.0f32; n];
        let scale = 1.0f32 / n as f32;
        for (m, out) in td.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for k in 0..n2 {
                let x = spectrum.get(k).copied().unwrap_or(0.0);
                let angle = pi / n as f32 * (m as f32 + 0.5 + n2 as f32 / 2.0) * (k as f32 + 0.5);
                acc += x * angle.cos();
            }
            *out = acc * scale;
        }

        for (m, out) in td.iter_mut().enumerate() {
            *out *= window.get(m).copied().unwrap_or(1.0);
        }
        channels_td.push(td);
    }

    TimeDomainBlock {
        channels: channels_td,
    }
}

pub(crate) fn build_vorbis_window(
    n: usize,
    short_n: usize,
    is_long_block: bool,
    prev_window_flag: bool,
    next_window_flag: bool,
) -> Vec<f32> {
    let mut window = vec![0.0f32; n];
    if n == 0 {
        return window;
    }
    if !is_long_block {
        fill_window_segment(&mut window, 0, n / 2, n / 2);
        fill_window_segment(&mut window, n / 2, n, n / 2);
        return window;
    }

    let left_start;
    let left_end;
    let left_len;
    if prev_window_flag {
        left_start = 0;
        left_end = n / 2;
        left_len = n / 2;
    } else {
        left_start = (n / 4).saturating_sub(short_n / 4);
        left_end = left_start + short_n / 2;
        left_len = short_n / 2;
    }

    let right_start;
    let right_end;
    let right_len;
    if next_window_flag {
        right_start = n / 2;
        right_end = n;
        right_len = n / 2;
    } else {
        right_start = n.saturating_sub(n / 4).saturating_sub(short_n / 4);
        right_end = right_start + short_n / 2;
        right_len = short_n / 2;
    }

    fill_window_segment(&mut window, left_start, left_end, left_len);
    for w in window
        .iter_mut()
        .take(right_start.min(n))
        .skip(left_end.min(n))
    {
        *w = 1.0;
    }
    fill_window_segment(&mut window, right_start, right_end, right_len);
    window
}

fn fill_window_segment(window: &mut [f32], start: usize, end: usize, len: usize) {
    if len == 0 {
        return;
    }
    let pi = std::f32::consts::PI;
    for i in start..end.min(window.len()) {
        let x = (i - start) as f32 + 0.5;
        let angle = x / len as f32 * (pi / 2.0);
        let inner = angle.sin();
        window[i] = (0.5 * pi * inner * inner).sin();
    }
}

/// 将当前块与上一块 overlap 区域拼接成输出块.
pub(crate) fn overlap_add(
    td: &TimeDomainBlock,
    overlap: &mut [Vec<f32>],
    left_start: usize,
    right_start: usize,
    right_end: usize,
) -> TimeDomainBlock {
    let mut out = vec![Vec::<f32>::new(); td.channels.len()];
    for (ch, src) in td.channels.iter().enumerate() {
        let mut mixed = src.clone();
        if let Some(prev) = overlap.get(ch) {
            let n = prev.len().min(mixed.len().saturating_sub(left_start));
            for i in 0..n {
                mixed[left_start + i] += prev[i];
            }
        }

        if let Some(slot) = overlap.get_mut(ch) {
            slot.clear();
            let start = right_start.min(mixed.len());
            let end = right_end.min(mixed.len());
            if start < end {
                slot.extend_from_slice(&mixed[start..end]);
            }
        }

        let produced = right_start.saturating_sub(left_start);
        if produced == 0 || left_start >= mixed.len() {
            continue;
        }
        let end = (left_start + produced).min(mixed.len());
        out[ch].extend_from_slice(&mixed[left_start..end]);
    }
    TimeDomainBlock { channels: out }
}
