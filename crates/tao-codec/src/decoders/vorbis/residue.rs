use tao_core::TaoResult;

use super::setup::{CouplingStep, ParsedSetup};

/// residue 解码阶段输出的频谱占位数据.
#[derive(Debug, Clone)]
pub(crate) struct ResidueSpectrum {
    pub(crate) channels: Vec<Vec<f32>>,
}

/// 当前阶段先返回全零频谱占位, 后续替换为真实 residue 解码.
pub(crate) fn decode_residue_placeholder(
    setup: &ParsedSetup,
    channel_count: usize,
    blocksize: usize,
) -> TaoResult<ResidueSpectrum> {
    if setup.residue_count == 0 {
        return Err(tao_core::TaoError::InvalidData(
            "Vorbis residue_count 非法".into(),
        ));
    }
    Ok(ResidueSpectrum {
        channels: vec![vec![0.0; blocksize / 2]; channel_count],
    })
}

/// 对 residue 频谱执行 Vorbis channel coupling 反变换。
pub(crate) fn apply_coupling_inverse(
    spectrum: &mut ResidueSpectrum,
    coupling_steps: &[CouplingStep],
) -> TaoResult<()> {
    for step in coupling_steps.iter().rev() {
        let m_ch = usize::from(step.magnitude);
        let a_ch = usize::from(step.angle);
        if m_ch >= spectrum.channels.len() || a_ch >= spectrum.channels.len() {
            return Err(tao_core::TaoError::InvalidData(
                "Vorbis coupling 声道索引越界".into(),
            ));
        }

        let len = spectrum.channels[m_ch]
            .len()
            .min(spectrum.channels[a_ch].len());
        for i in 0..len {
            let m = spectrum.channels[m_ch][i];
            let a = spectrum.channels[a_ch][i];
            if m > 0.0 {
                if a > 0.0 {
                    spectrum.channels[m_ch][i] = m;
                    spectrum.channels[a_ch][i] = m - a;
                } else {
                    spectrum.channels[a_ch][i] = m;
                    spectrum.channels[m_ch][i] = m + a;
                }
            } else if a > 0.0 {
                spectrum.channels[m_ch][i] = m;
                spectrum.channels[a_ch][i] = m + a;
            } else {
                spectrum.channels[a_ch][i] = m;
                spectrum.channels[m_ch][i] = m - a;
            }
        }
    }
    Ok(())
}
