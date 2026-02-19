use tao_core::{TaoError, TaoResult};

use super::bitreader::LsbBitReader;
use super::codebook::{CodebookHuffman, decode_codebook_scalar, decode_codebook_vector};
use super::setup::{CouplingStep, MappingConfig, ParsedSetup, ResidueConfig};

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
    huffmans: &[CodebookHuffman],
    do_not_decode: &[bool],
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

        let submap_channels: Vec<usize> = (0..channel_count)
            .filter(|&ch| mapping.channel_mux.get(ch).copied().unwrap_or(0) as usize == submap)
            .collect();
        if submap_channels.is_empty() {
            continue;
        }
        let any_decode = submap_channels
            .iter()
            .any(|&ch| !do_not_decode.get(ch).copied().unwrap_or(true));
        if !any_decode {
            continue;
        }

        decode_one_residue(
            br,
            setup,
            residue,
            huffmans,
            &submap_channels,
            do_not_decode,
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

#[allow(clippy::too_many_arguments)]
fn decode_one_residue(
    br: &mut LsbBitReader<'_>,
    setup: &ParsedSetup,
    residue: &ResidueConfig,
    huffmans: &[CodebookHuffman],
    channels: &[usize],
    do_not_decode: &[bool],
    spectrum: &mut [Vec<f32>],
    n2: usize,
) -> TaoResult<()> {
    let mut begin = (residue.begin as usize).min(n2);
    let mut end = (residue.end as usize).min(n2);
    if end <= begin {
        return Ok(());
    }
    let psize = residue.partition_size as usize;
    if psize == 0 {
        return Ok(());
    }
    let mut partitions = (end - begin) / psize;
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
    if class_dimensions == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis residue classbook dimensions 为 0".into(),
        ));
    }

    let maxpass = residue
        .cascades
        .iter()
        .copied()
        .flat_map(|v| (0..8).filter(move |b| (v & (1 << b)) != 0))
        .max()
        .unwrap_or(0);

    let mut vec_buf = Vec::<f32>::new();
    let mut classifs: Vec<usize>;

    if residue.residue_type == 2 {
        let ch_count = channels.len().max(1);
        let any_decode = channels
            .iter()
            .any(|&ch| !do_not_decode.get(ch).copied().unwrap_or(true));
        if !any_decode {
            return Ok(());
        }

        let interleaved_n2 = n2.saturating_mul(ch_count);
        begin = (residue.begin as usize).min(interleaved_n2);
        end = (residue.end as usize).min(interleaved_n2);
        if end <= begin {
            return Ok(());
        }
        partitions = (end - begin) / psize;
        if partitions == 0 {
            return Ok(());
        }

        let cl_stride = partitions + class_dimensions;
        classifs = vec![0usize; cl_stride];
        let mut interleaved = vec![0.0f32; interleaved_n2];

        for pass in 0..=maxpass {
            let mut partition_count = 0usize;
            let mut voffset = begin;
            while partition_count < partitions {
                if pass == 0 {
                    let sym = match decode_codebook_scalar(br, classbook, classbook_huffman) {
                        Ok(v) => v,
                        Err(TaoError::Eof) => return Ok(()),
                        Err(e) => return Err(e),
                    };
                    let mut tmp = sym as usize;
                    for i in 0..class_dimensions {
                        if partition_count + i < partitions {
                            classifs[partition_count + i] = tmp % class_count;
                        }
                        tmp /= class_count;
                    }
                }
                for _ in 0..class_dimensions {
                    if partition_count >= partitions {
                        break;
                    }
                    let class_id = classifs[partition_count];
                    let cascade = residue.cascades.get(class_id).copied().unwrap_or(0);
                    if (cascade & (1 << pass)) != 0 {
                        let book_idx = residue
                            .books
                            .get(class_id)
                            .and_then(|a| a.get(pass))
                            .copied()
                            .flatten();
                        if let Some(book_idx) = book_idx {
                            let book = setup.codebooks.get(book_idx as usize).ok_or_else(|| {
                                TaoError::InvalidData(
                                    "Vorbis residue second-stage book 越界".into(),
                                )
                            })?;
                            let huffman = huffmans.get(book_idx as usize).ok_or_else(|| {
                                TaoError::InvalidData(
                                    "Vorbis residue second-stage Huffman 越界".into(),
                                )
                            })?;
                            apply_partition_residue_type2(
                                br,
                                residue,
                                book,
                                huffman,
                                &mut interleaved,
                                voffset,
                                psize,
                                interleaved_n2,
                                &mut vec_buf,
                            )?;
                        }
                    }
                    partition_count += 1;
                    voffset = voffset.saturating_add(psize);
                }
            }
        }

        for (ch_pos, &ch_idx) in channels.iter().enumerate() {
            if let Some(dst) = spectrum.get_mut(ch_idx) {
                for s in 0..n2 {
                    let idx = s * ch_count + ch_pos;
                    if idx < interleaved.len() && s < dst.len() {
                        dst[s] += interleaved[idx];
                    }
                }
            }
        }
        return Ok(());
    }

    let ch_count = channels.len();
    let cl_stride = partitions + class_dimensions;
    classifs = vec![0usize; ch_count * cl_stride];

    for pass in 0..=maxpass {
        let mut partition_count = 0usize;
        let mut voffset = begin;
        while partition_count < partitions {
            if pass == 0 {
                for (j, &ch_idx) in channels.iter().enumerate() {
                    if do_not_decode.get(ch_idx).copied().unwrap_or(true) {
                        continue;
                    }
                    let sym = match decode_codebook_scalar(br, classbook, classbook_huffman) {
                        Ok(v) => v,
                        Err(TaoError::Eof) => return Ok(()),
                        Err(e) => return Err(e),
                    };
                    let mut tmp = sym as usize;
                    for i in 0..class_dimensions {
                        if partition_count + i < partitions {
                            classifs[j * cl_stride + partition_count + i] = tmp % class_count;
                        }
                        tmp /= class_count;
                    }
                }
            }

            for _ in 0..class_dimensions {
                if partition_count >= partitions {
                    break;
                }
                for (j, &ch_idx) in channels.iter().enumerate() {
                    if do_not_decode.get(ch_idx).copied().unwrap_or(true) {
                        continue;
                    }
                    let class_id = classifs[j * cl_stride + partition_count];
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
                    if let Some(book_idx) = book_idx {
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
                            ch_idx,
                            spectrum,
                            voffset,
                            psize,
                            n2,
                            &mut vec_buf,
                        )?;
                    }
                }
                partition_count += 1;
                voffset = voffset.saturating_add(psize);
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
    spectrum: &mut [Vec<f32>],
    base: usize,
    psize: usize,
    n2: usize,
    vec_buf: &mut Vec<f32>,
) -> TaoResult<()> {
    let dims = usize::from(book.dimensions.max(1));
    if vec_buf.len() < dims {
        vec_buf.resize(dims, 0.0);
    }
    if book.lookup_type == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis residue codebook lookup_type=0 不支持".into(),
        ));
    }

    match residue.residue_type {
        0 => {
            let step = psize / dims;
            let mut j = 0usize;
            while j < step {
                let got = match decode_codebook_vector(br, book, huffman, vec_buf) {
                    Ok(v) => v,
                    Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                };
                if let Some(dst) = spectrum.get_mut(channel) {
                    for (k, val) in vec_buf.iter().copied().enumerate().take(got) {
                        let idx = base + j + k * step;
                        if idx < n2 && idx < dst.len() {
                            dst[idx] += val;
                        }
                    }
                }
                j += 1;
            }
        }
        1 => {
            let mut pos = 0usize;
            while pos < psize {
                let got = match decode_codebook_vector(br, book, huffman, vec_buf) {
                    Ok(v) => v,
                    Err(TaoError::Eof) => break,
                    Err(e) => return Err(e),
                };
                if let Some(dst) = spectrum.get_mut(channel) {
                    for (k, val) in vec_buf.iter().copied().enumerate().take(got) {
                        let idx = base + pos + k;
                        if idx < n2 && idx < dst.len() {
                            dst[idx] += val;
                        }
                    }
                }
                pos = pos.saturating_add(got.max(1));
            }
        }
        2 => {}
        _ => {
            return Err(TaoError::InvalidData("Vorbis residue_type 非法".into()));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_partition_residue_type2(
    br: &mut LsbBitReader<'_>,
    residue: &ResidueConfig,
    book: &super::setup::CodebookConfig,
    huffman: &CodebookHuffman,
    interleaved: &mut [f32],
    base: usize,
    psize: usize,
    n2: usize,
    vec_buf: &mut Vec<f32>,
) -> TaoResult<()> {
    let dims = usize::from(book.dimensions.max(1));
    if vec_buf.len() < dims {
        vec_buf.resize(dims, 0.0);
    }
    if book.lookup_type == 0 {
        return Err(TaoError::InvalidData(
            "Vorbis residue codebook lookup_type=0 不支持".into(),
        ));
    }
    if residue.residue_type == 0 {
        return Ok(());
    }

    let mut pos = 0usize;
    while pos < psize {
        let got = match decode_codebook_vector(br, book, huffman, vec_buf) {
            Ok(v) => v,
            Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        };
        for (k, val) in vec_buf.iter().copied().enumerate().take(got) {
            let idx = base + pos + k;
            if idx < n2 && idx < interleaved.len() {
                interleaved[idx] += val;
            }
        }
        pos = pos.saturating_add(got.max(1));
    }
    Ok(())
}
