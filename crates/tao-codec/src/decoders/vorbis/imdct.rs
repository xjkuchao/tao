/// IMDCT 输出时域样本.
#[derive(Debug, Clone)]
pub(crate) struct TimeDomainBlock {
    pub(crate) channels: Vec<Vec<f32>>,
}

use super::residue::ResidueSpectrum;

/// 将 residue 频谱执行 IMDCT 并应用 Vorbis 窗函数.
pub(crate) fn imdct_from_residue(residue: &ResidueSpectrum, blocksize: usize) -> TimeDomainBlock {
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
        for (m, out) in td.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for k in 0..n2 {
                let x = spectrum.get(k).copied().unwrap_or(0.0);
                let angle = pi / n2 as f32 * (m as f32 + 0.5 + n2 as f32 / 2.0) * (k as f32 + 0.5);
                acc += x * angle.cos();
            }
            *out = acc;
        }

        for (m, out) in td.iter_mut().enumerate() {
            let t = (m as f32 + 0.5) / n as f32;
            let inner = (pi * t).sin();
            let w = (0.5 * pi * inner * inner).sin();
            *out *= w;
        }
        channels_td.push(td);
    }

    TimeDomainBlock {
        channels: channels_td,
    }
}

/// 将当前块与上一块 overlap 区域拼接成输出块.
pub(crate) fn overlap_add(
    td: &TimeDomainBlock,
    overlap: &mut [Vec<f32>],
    out_samples: usize,
) -> TimeDomainBlock {
    let mut out = vec![Vec::<f32>::new(); td.channels.len()];
    for (ch, src) in td.channels.iter().enumerate() {
        let mut mixed = src.clone();
        if let Some(prev) = overlap.get(ch) {
            let n = prev.len().min(mixed.len());
            for i in 0..n {
                mixed[i] += prev[i];
            }
        }

        let produced = out_samples.min(mixed.len());
        out[ch].extend_from_slice(&mixed[..produced]);

        if let Some(slot) = overlap.get_mut(ch) {
            slot.clear();
            let start = mixed.len() / 2;
            slot.extend_from_slice(&mixed[start..]);
        }
    }
    TimeDomainBlock { channels: out }
}
