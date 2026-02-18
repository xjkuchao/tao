use tao_core::{TaoError, TaoResult};

use super::bitreader::LsbBitReader;
use super::codebook::{CodebookHuffman, decode_codebook_scalar, decode_codebook_vector};
use super::floor::FloorCurves;
use super::setup::{CouplingStep, MappingConfig, ParsedSetup, ResidueConfig};

const RESIDUE_VECTOR_GAIN: f32 = 0.0005;

/// residue 解码阶段输出的频谱占位数据.
#[derive(Debug, Clone)]
pub(crate) struct ResidueSpectrum {
    pub(crate) channels: Vec<Vec<f32>>,
}

/// 当前阶段的 residue 近似解码:
/// - 基于 setup/mapping 按位流消费 codebook 符号
/// - 将符号值映射为近似频谱增量, 以打通完整链路
pub(crate) fn decode_residue_approx(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    mapping: &MappingConfig,
    floor: &FloorCurves,
    huffmans: &[CodebookHuffman],
    channel_count: usize,
    blocksize: usize,
) -> TaoResult<ResidueSpectrum> {
    if setup.residue_count == 0 {
        return Err(tao_core::TaoError::InvalidData(
            "Vorbis residue_count 非法".into(),
        ));
    }
    let n2 = blocksize / 2;
    let mut out = ResidueSpectrum {
        channels: vec![vec![0.0; n2]; channel_count],
    };

    let submaps = mapping.submap_residue.len();
    for submap in 0..submaps {
        let residue_idx = mapping.submap_residue[submap] as usize;
        let residue = setup
            .residues
            .get(residue_idx)
            .ok_or_else(|| TaoError::InvalidData("Vorbis residue 索引越界".into()))?;

        let active_channels: Vec<usize> = (0..channel_count)
            .filter(|&ch| {
                mapping.channel_mux.get(ch).copied().unwrap_or(0) as usize == submap
                    && floor.nonzero.get(ch).copied().unwrap_or(false)
            })
            .collect();
        if active_channels.is_empty() {
            continue;
        }

        decode_one_residue(
            br,
            setup,
            residue,
            huffmans,
            &active_channels,
            &mut out.channels,
            n2,
        )?;
    }

    Ok(out)
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

fn decode_one_residue(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    residue: &ResidueConfig,
    huffmans: &[CodebookHuffman],
    channels: &[usize],
    spectrum: &mut [Vec<f32>],
    n2: usize,
) -> TaoResult<()> {
    let begin = (residue.begin as usize).min(n2);
    let end = (residue.end as usize).min(n2);
    if end <= begin {
        return Ok(());
    }
    let psize = residue.partition_size as usize;
    if psize == 0 {
        return Ok(());
    }
    let partitions = (end - begin) / psize;
    if partitions == 0 {
        return Ok(());
    }
    let classbook_idx = residue.classbook as usize;
    let classbook = setup
        .codebooks
        .get(classbook_idx)
        .ok_or_else(|| TaoError::InvalidData("Vorbis residue classbook 越界".into()))?;
    let classbook_huffman = huffmans
        .get(classbook_idx)
        .ok_or_else(|| TaoError::InvalidData("Vorbis residue classbook Huffman 越界".into()))?;
    let class_dimensions = usize::from(classbook.dimensions.max(1));
    let class_count = residue.classifications.max(1) as usize;

    if residue.residue_type == 2 {
        let mut class_vec = vec![0usize; partitions];
        let mut p = 0usize;
        while p < partitions {
            let sym = match decode_codebook_scalar(br, classbook, classbook_huffman) {
                Ok(v) => v,
                Err(TaoError::Eof) => return Ok(()),
                Err(e) => return Err(e),
            };
            let mut tmp = sym as usize;
            let fill = class_dimensions.min(partitions - p);
            for i in 0..fill {
                class_vec[p + i] = tmp % class_count;
                tmp /= class_count;
            }
            p += fill;
        }

        for pass in 0..8usize {
            for (part, class_id_ref) in class_vec.iter().enumerate().take(partitions) {
                let class_id = *class_id_ref;
                let cascade = residue.cascades.get(class_id).copied().unwrap_or(0);
                if (cascade & (1 << pass)) == 0 {
                    continue;
                }
                let book_idx = residue
                    .books
                    .get(class_id)
                    .and_then(|a| a.get(pass))
                    .copied()
                    .flatten();
                let Some(book_idx) = book_idx else {
                    continue;
                };
                let book = setup.codebooks.get(book_idx as usize).ok_or_else(|| {
                    TaoError::InvalidData("Vorbis residue second-stage book 越界".into())
                })?;
                let huffman = huffmans.get(book_idx as usize).ok_or_else(|| {
                    TaoError::InvalidData("Vorbis residue second-stage Huffman 越界".into())
                })?;
                apply_partition_residue(
                    br,
                    residue,
                    book,
                    huffman,
                    channels[0],
                    channels,
                    spectrum,
                    begin + part * psize,
                    psize,
                    n2,
                )?;
            }
        }
        return Ok(());
    }

    for &ch in channels {
        let mut class_vec = vec![0usize; partitions];
        let mut p = 0usize;
        while p < partitions {
            let sym = match decode_codebook_scalar(br, classbook, classbook_huffman) {
                Ok(v) => v,
                Err(TaoError::Eof) => return Ok(()),
                Err(e) => return Err(e),
            };
            let mut tmp = sym as usize;
            let fill = class_dimensions.min(partitions - p);
            for i in 0..fill {
                class_vec[p + i] = tmp % class_count;
                tmp /= class_count;
            }
            p += fill;
        }

        for pass in 0..8usize {
            for (part, class_id_ref) in class_vec.iter().enumerate().take(partitions) {
                let class_id = *class_id_ref;
                let cascade = residue.cascades.get(class_id).copied().unwrap_or(0);
                if (cascade & (1 << pass)) == 0 {
                    continue;
                }
                let book_idx = residue
                    .books
                    .get(class_id)
                    .and_then(|a| a.get(pass))
                    .copied()
                    .flatten();
                let Some(book_idx) = book_idx else {
                    continue;
                };
                let book = setup.codebooks.get(book_idx as usize).ok_or_else(|| {
                    TaoError::InvalidData("Vorbis residue second-stage book 越界".into())
                })?;
                let huffman = huffmans.get(book_idx as usize).ok_or_else(|| {
                    TaoError::InvalidData("Vorbis residue second-stage Huffman 越界".into())
                })?;
                apply_partition_residue(
                    br,
                    residue,
                    book,
                    huffman,
                    ch,
                    channels,
                    spectrum,
                    begin + part * psize,
                    psize,
                    n2,
                )?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_partition_residue(
    br: &mut LsbBitReader<'_>,
    residue: &ResidueConfig,
    book: &super::setup::CodebookConfig,
    huffman: &CodebookHuffman,
    channel: usize,
    active_channels: &[usize],
    spectrum: &mut [Vec<f32>],
    base: usize,
    psize: usize,
    n2: usize,
) -> TaoResult<()> {
    let dims = usize::from(book.dimensions.max(1));
    let mut vec_buf = vec![0.0f32; dims];

    match residue.residue_type {
        0 => {
            let step = (psize / dims).max(1);
            let mut j = 0usize;
            while j < step {
                let got = match decode_codebook_vector(br, book, huffman, &mut vec_buf) {
                    Ok(v) => v,
                    Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                };
                if let Some(dst) = spectrum.get_mut(channel) {
                    for (k, val) in vec_buf.iter().copied().enumerate().take(got) {
                        let idx = base + j + k * step;
                        if idx < n2 && idx < dst.len() {
                            dst[idx] += val * RESIDUE_VECTOR_GAIN;
                        }
                    }
                }
                j += 1;
            }
        }
        1 => {
            let mut pos = 0usize;
            while pos < psize {
                let got = match decode_codebook_vector(br, book, huffman, &mut vec_buf) {
                    Ok(v) => v,
                    Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                };
                if let Some(dst) = spectrum.get_mut(channel) {
                    for (k, val) in vec_buf.iter().copied().enumerate().take(got) {
                        let idx = base + pos + k;
                        if idx < n2 && idx < dst.len() {
                            dst[idx] += val * RESIDUE_VECTOR_GAIN;
                        }
                    }
                }
                pos = pos.saturating_add(got.max(1));
            }
        }
        2 => {
            let ch_count = active_channels.len().max(1);
            let mut pos = 0usize;
            let mut flat = 0usize;
            while pos < psize {
                let got = match decode_codebook_vector(br, book, huffman, &mut vec_buf) {
                    Ok(v) => v,
                    Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                };
                for val in vec_buf.iter().copied().take(got) {
                    let ch_off = flat % ch_count;
                    let sample_off = pos + flat / ch_count;
                    let dst_ch = active_channels[ch_off];
                    if let Some(dst) = spectrum.get_mut(dst_ch) {
                        let idx = base + sample_off;
                        if idx < n2 && idx < dst.len() {
                            dst[idx] += val * RESIDUE_VECTOR_GAIN;
                        }
                    }
                    flat = flat.saturating_add(1);
                }
                pos = pos.saturating_add(flat / ch_count);
                flat %= ch_count;
            }
        }
        _ => {
            return Err(TaoError::InvalidData("Vorbis residue_type 非法".into()));
        }
    }
    Ok(())
}
