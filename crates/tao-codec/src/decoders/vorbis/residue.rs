use tao_core::TaoResult;

use super::setup::ParsedSetup;

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
