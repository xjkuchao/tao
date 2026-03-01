//! AAC 频谱处理: 辅助结构体、量化、立体声、TNS 等.

use std::cell::Cell;
use std::sync::OnceLock;

use tao_core::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

use super::huffman::AacCodebooks;
use super::tables::*;

#[derive(Clone, Copy)]
pub(super) struct IcsInfo {
    pub(super) window_sequence: u32,
    pub(super) window_shape: u8,
    pub(super) max_sfb: usize,
    pub(super) num_swb: usize,
    pub(super) num_window_groups: usize,
    pub(super) window_group_lengths: [usize; 8],
    pub(super) window_group_starts: [usize; 8],
}

#[derive(Default, Clone)]
pub(super) struct IcsBandInfo {
    pub(super) band_types: Vec<u8>,
    pub(super) scale_factors: Vec<i32>,
}

pub(super) struct Section {
    pub(super) group: usize,
    pub(super) sect_cb: u8,
    pub(super) sect_start: usize,
    pub(super) sect_end: usize,
}

#[derive(Clone)]
pub(super) struct TnsData {
    pub(super) num_windows: usize,
    pub(super) n_filt: [u8; 8],
    pub(super) length: [[u8; 4]; 8],
    pub(super) order: [[u8; 4]; 8],
    pub(super) direction: [[bool; 4]; 8],
    pub(super) coef: [[[f32; 20]; 4]; 8],
}

#[derive(Default, Clone)]
pub(super) struct PulseData {
    pub(super) num_pulse: usize,
    pub(super) pos: [usize; 4],
    pub(super) amp: [i32; 4],
}

impl Default for TnsData {
    fn default() -> Self {
        Self {
            num_windows: 0,
            n_filt: [0; 8],
            length: [[0; 4]; 8],
            order: [[0; 4]; 8],
            direction: [[false; 4]; 8],
            coef: [[[0.0; 20]; 4]; 8],
        }
    }
}

/// 解析 section_data
pub(super) fn parse_section_data(br: &mut BitReader, info: &IcsInfo) -> TaoResult<Vec<Section>> {
    let mut sections = Vec::new();
    let is_short = info.window_sequence == 2;
    let sect_bits = if is_short { 3 } else { 5 };
    let sect_esc = if is_short { 7 } else { 31 };

    for group in 0..info.num_window_groups {
        let mut k = 0usize;
        while k < info.max_sfb {
            let sect_cb = br.read_bits(4)? as u8;
            if sect_cb == 12 {
                return Err(TaoError::InvalidData(format!(
                    "AAC section_data 非法: group={}, sfb={}, 遇到无效 codebook=12",
                    group, k
                )));
            }
            let mut sect_end = k;
            loop {
                let incr = br.read_bits(sect_bits)? as usize;
                sect_end = sect_end.checked_add(incr).ok_or_else(|| {
                    TaoError::InvalidData(format!(
                        "AAC section_data 非法: group={}, sfb={}, section 长度溢出",
                        group, k
                    ))
                })?;
                if sect_end > info.max_sfb {
                    // FFmpeg 行为: 截断到 max_sfb 并结束当前组的 section 解析
                    sect_end = info.max_sfb;
                    break;
                }
                if incr != sect_esc {
                    break;
                }
            }
            if sect_end == k {
                return Err(TaoError::InvalidData(format!(
                    "AAC section_data 非法: group={}, sfb={}, section 长度为 0",
                    group, k
                )));
            }
            sections.push(Section {
                group,
                sect_cb,
                sect_start: k,
                sect_end,
            });
            k = sect_end;
        }
    }
    Ok(sections)
}

/// 解析 scale_factor_data (ISO 14496-3, 4.5.2.3.4)
///
/// 区分三种码本类型:
/// - 普通码本 (1-11): Huffman 编码 scale factor delta
/// - 噪声 (13): 第一个频带读 9 位原始值, 后续 Huffman 编码
/// - 强度立体声 (14/15): Huffman 编码 IS position delta
pub(super) fn parse_scale_factor_data(
    br: &mut BitReader,
    sections: &[Section],
    info: &IcsInfo,
    global_gain: i32,
    codebooks: &AacCodebooks,
) -> TaoResult<Vec<i32>> {
    let mut scale_factors = vec![0i32; info.num_window_groups * info.num_swb];
    let mut sf = global_gain;
    let mut is_position = 0i32;
    let mut noise_energy = global_gain - NOISE_OFFSET;
    let mut noise_pcm_flag = true; // 第一个噪声频带标志

    for section in sections {
        let cb = section.sect_cb;
        let end = section.sect_end.min(info.num_swb);
        let group_base = section.group * info.num_swb;
        for sfb in section.sect_start..end {
            let sf_slot = &mut scale_factors[group_base + sfb];
            if cb == 0 {
                // ZERO_HCB: 无 scale factor
                *sf_slot = 0;
            } else if cb == NOISE_HCB {
                // NOISE_HCB (13): 噪声能量
                if noise_pcm_flag {
                    // 第一个噪声频带: 读取 9 位原始值
                    noise_pcm_flag = false;
                    let raw = br.read_bits(9)? as i32;
                    noise_energy = global_gain - NOISE_OFFSET + raw - NOISE_PRE;
                } else {
                    let delta = codebooks.sf_tree.decode(br)? - 60;
                    noise_energy += delta;
                }
                noise_energy = noise_energy.clamp(-100, 155);
                *sf_slot = noise_energy;
            } else if cb == INTENSITY_HCB || cb == INTENSITY_HCB2 {
                // INTENSITY_HCB (15) / INTENSITY_HCB2 (14): IS position
                let delta = codebooks.sf_tree.decode(br)? - 60;
                is_position += delta;
                is_position = is_position.clamp(-155, 100);
                *sf_slot = is_position;
            } else {
                // 普通频谱码本: scale factor
                let delta = codebooks.sf_tree.decode(br)? - 60;
                sf += delta;
                *sf_slot = sf.clamp(0, 255);
            }
        }
    }
    Ok(scale_factors)
}

/// 解码频谱数据
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_spectral_data(
    br: &mut BitReader,
    spectral: &mut [f32],
    sections: &[Section],
    scale_factors: &[i32],
    codebooks: &AacCodebooks,
    info: &IcsInfo,
    swb_offset: &[usize],
    random_state: &Cell<u32>,
) -> TaoResult<()> {
    let is_short = info.window_sequence == 2;
    for section in sections {
        let cb = section.sect_cb;
        if cb == 0 || cb == INTENSITY_HCB2 || cb == INTENSITY_HCB {
            // ZERO_HCB / INTENSITY: 频谱为 0
            continue;
        }

        for sfb in section.sect_start..section.sect_end {
            let start_idx = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end_idx = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            let sf = scale_factors
                .get(section.group * info.num_swb + sfb)
                .copied()
                .unwrap_or(0);
            let band_width = end_idx.saturating_sub(start_idx);
            let window_group_len = info.window_group_lengths[section.group];
            let group_start = info.window_group_starts[section.group];
            if cb == NOISE_HCB {
                // PNS 噪声重建: 与 FFmpeg 相同的 LCG 随机源 + 频带归一化.
                let target = 2.0f32.powf(0.25 * (sf - 100) as f32);
                for win_in_group in 0..window_group_len {
                    let win = group_start + win_in_group;
                    let win_base = win * 128 + start_idx;
                    let mut energy = 0.0f32;
                    for i in 0..band_width {
                        let idx = win_base + i;
                        if idx < spectral.len() {
                            let noise = lcg_random(random_state);
                            spectral[idx] = noise;
                            energy += noise * noise;
                        }
                    }
                    if energy > 0.0 {
                        let scale = target / energy.sqrt();
                        for i in 0..band_width {
                            let idx = win_base + i;
                            if idx < spectral.len() {
                                spectral[idx] *= scale;
                            }
                        }
                    }
                }
                continue;
            }

            if (1..=11).contains(&cb) {
                let cb_idx = (cb - 1) as usize;
                let spec_cb = codebooks.spectral[cb_idx]
                    .as_ref()
                    .ok_or_else(|| TaoError::Unsupported(format!("AAC: 频谱码本 {cb} 未实现")))?;
                if is_short {
                    // short 窗口需按每个窗单独解码 (与 FFmpeg 行为一致).
                    'short_outer: for win_in_group in 0..window_group_len {
                        let win = group_start + win_in_group;
                        let win_base = win * 128 + start_idx;
                        let mut i = 0usize;
                        while i < band_width {
                            let values = match spec_cb.decode_values(br) {
                                Ok(v) => v,
                                Err(_) => {
                                    // 码流提前结束: 剩余频谱系数保持为 0, 停止解码 (与 FFmpeg 行为一致)
                                    break 'short_outer;
                                }
                            };
                            let count = spec_cb.dim.min(band_width - i);
                            for (j, &v) in values.iter().enumerate().take(count) {
                                let idx = win_base + i + j;
                                if idx < spectral.len() {
                                    spectral[idx] = inverse_quantize(v, sf);
                                }
                            }
                            i += spec_cb.dim;
                        }
                    }
                } else {
                    // long 窗口按单个频带连续解码.
                    let mut i = 0usize;
                    'long_outer: while i < band_width {
                        let values = match spec_cb.decode_values(br) {
                            Ok(v) => v,
                            Err(_) => {
                                // 码流提前结束: 剩余频谱系数保持为 0, 停止解码 (与 FFmpeg 行为一致)
                                break 'long_outer;
                            }
                        };
                        let count = spec_cb.dim.min(band_width - i);
                        for (j, &v) in values.iter().enumerate().take(count) {
                            let idx = start_idx + i + j;
                            if idx < spectral.len() {
                                spectral[idx] = inverse_quantize(v, sf);
                            }
                        }
                        i += spec_cb.dim;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn lcg_random(state: &Cell<u32>) -> f32 {
    let next = state
        .get()
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    state.set(next);
    (next as i32) as f32
}

/// 对 CPE 频谱应用 MS 立体声反变换
pub(super) fn apply_ms_stereo(
    left: &mut [f32],
    right: &mut [f32],
    info: &IcsInfo,
    ms_used: &[bool],
    left_band_types: Option<&[u8]>,
    right_band_types: Option<&[u8]>,
    swb_offset: &[usize],
) {
    let is_short = info.window_sequence == 2;
    for group in 0..info.num_window_groups {
        for sfb in 0..info.max_sfb {
            let mask_idx = group * info.max_sfb + sfb;
            if !ms_used.get(mask_idx).copied().unwrap_or(false) {
                continue;
            }
            if let (Some(left_types), Some(right_types)) = (left_band_types, right_band_types) {
                let left_bt = left_types.get(mask_idx).copied().unwrap_or(0);
                let right_bt = right_types.get(mask_idx).copied().unwrap_or(0);
                if left_bt >= NOISE_HCB || right_bt >= NOISE_HCB {
                    continue;
                }
            }
            let start = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            if end <= start {
                continue;
            }
            if is_short {
                let group_len = info.window_group_lengths[group];
                let group_start = info.window_group_starts[group];
                for win in 0..group_len {
                    let win_base = (group_start + win) * 128;
                    for line in start..end {
                        let idx = win_base + line;
                        if idx >= left.len() || idx >= right.len() {
                            continue;
                        }
                        let l = left[idx];
                        let r = right[idx];
                        left[idx] = l + r;
                        right[idx] = l - r;
                    }
                }
            } else {
                for idx in start..end {
                    if idx >= left.len() || idx >= right.len() {
                        continue;
                    }
                    let l = left[idx];
                    let r = right[idx];
                    left[idx] = l + r;
                    right[idx] = l - r;
                }
            }
        }
    }
}

/// 对 CPE 频谱应用强度立体声 (IS) 重建.
pub(super) fn apply_intensity_stereo(
    left: &mut [f32],
    right: &mut [f32],
    info: &IcsInfo,
    right_band_types: &[u8],
    right_scale_factors: &[i32],
    ms_used: Option<&[bool]>,
    swb_offset: &[usize],
) {
    let is_short = info.window_sequence == 2;
    for group in 0..info.num_window_groups {
        for sfb in 0..info.max_sfb {
            let band_idx = group * info.max_sfb + sfb;
            let sf_idx = group * info.num_swb + sfb;
            let band_type = right_band_types.get(band_idx).copied().unwrap_or(0);
            if band_type != INTENSITY_HCB && band_type != INTENSITY_HCB2 {
                continue;
            }

            // intensity_position 对应 scalefactor 值.
            let is_position = right_scale_factors.get(sf_idx).copied().unwrap_or(0) as f32;
            let mut sign = if band_type == INTENSITY_HCB2 {
                -1.0f32
            } else {
                1.0f32
            };
            if ms_used
                .and_then(|mask| mask.get(band_idx))
                .copied()
                .unwrap_or(false)
            {
                sign = -sign;
            }
            let scale = sign * 0.5f32.powf(0.25 * is_position);

            let start = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            if end <= start {
                continue;
            }

            if is_short {
                let group_len = info.window_group_lengths[group];
                let group_start = info.window_group_starts[group];
                for win in 0..group_len {
                    let win_base = (group_start + win) * 128;
                    for line in start..end {
                        let idx = win_base + line;
                        if idx >= left.len() || idx >= right.len() {
                            continue;
                        }
                        right[idx] = left[idx] * scale;
                    }
                }
            } else {
                for idx in start..end {
                    if idx >= left.len() || idx >= right.len() {
                        continue;
                    }
                    right[idx] = left[idx] * scale;
                }
            }
        }
    }
}

/// 解析 pulse_data (ISO 14496-3 Table 4.7).
pub(super) fn parse_pulse_data(
    br: &mut BitReader,
    info: &IcsInfo,
    swb_offset: &[usize],
) -> TaoResult<PulseData> {
    if info.window_sequence == 2 {
        return Err(TaoError::InvalidData(
            "AAC pulse_data 非法: EIGHT_SHORT_SEQUENCE 不允许 pulse tool".into(),
        ));
    }
    if info.num_swb >= swb_offset.len() {
        return Err(TaoError::InvalidData(format!(
            "AAC pulse_data 非法: num_swb={} 超出 swb_offset 边界={}",
            info.num_swb,
            swb_offset.len()
        )));
    }

    let mut pulse = PulseData {
        num_pulse: br.read_bits(2)? as usize + 1,
        ..Default::default()
    };
    let pulse_swb = br.read_bits(6)? as usize;
    if pulse_swb >= info.num_swb {
        return Err(TaoError::InvalidData(format!(
            "AAC pulse_data 非法: pulse_start_sfb={} 超出 num_swb={}",
            pulse_swb, info.num_swb
        )));
    }

    let max_line = swb_offset[info.num_swb];
    pulse.pos[0] = swb_offset[pulse_swb] + br.read_bits(5)? as usize;
    if pulse.pos[0] >= max_line {
        return Err(TaoError::InvalidData(format!(
            "AAC pulse_data 非法: pulse pos[0]={} 超出频谱上限={}",
            pulse.pos[0], max_line
        )));
    }
    pulse.amp[0] = br.read_bits(4)? as i32;

    for i in 1..pulse.num_pulse {
        pulse.pos[i] = pulse.pos[i - 1] + br.read_bits(5)? as usize;
        if pulse.pos[i] >= max_line {
            return Err(TaoError::InvalidData(format!(
                "AAC pulse_data 非法: pulse pos[{}]={} 超出频谱上限={}",
                i, pulse.pos[i], max_line
            )));
        }
        pulse.amp[i] = br.read_bits(4)? as i32;
    }
    Ok(pulse)
}

/// 对频谱应用 pulse tool 校正.
pub(super) fn apply_pulse_data(
    spectral: &mut [f32],
    pulse: &PulseData,
    sections: &[Section],
    scale_factors: &[i32],
    info: &IcsInfo,
    swb_offset: &[usize],
) {
    if pulse.num_pulse == 0 || info.num_swb == 0 || info.num_swb >= swb_offset.len() {
        return;
    }

    // pulse 仅用于 long block, 这里按首组 SFB 定位 band_type/scalefactor.
    let mut band_types = vec![0u8; info.max_sfb];
    for section in sections {
        if section.group != 0 {
            continue;
        }
        let end = section.sect_end.min(info.max_sfb);
        for sfb in section.sect_start..end {
            band_types[sfb] = section.sect_cb;
        }
    }

    let max_line = swb_offset[info.num_swb];
    let mut sfb_idx = 0usize;
    for i in 0..pulse.num_pulse.min(4) {
        let pos = pulse.pos[i];
        if pos >= spectral.len() || pos >= max_line {
            continue;
        }

        while sfb_idx + 1 < info.num_swb && swb_offset[sfb_idx + 1] <= pos {
            sfb_idx += 1;
        }
        let band_type = band_types.get(sfb_idx).copied().unwrap_or(0);
        if !(1..=11).contains(&band_type) {
            continue;
        }
        let sf = scale_factors.get(sfb_idx).copied().unwrap_or(0);
        let sf_scale = 2.0f32.powf(0.25 * (sf - 120) as f32);
        if !sf_scale.is_finite() || sf_scale == 0.0 {
            continue;
        }

        let co = spectral[pos];
        let amp = pulse.amp[i] as f32;
        let mut q = -amp;
        if co != 0.0 {
            let mut quantized = co / sf_scale;
            let abs_q = quantized.abs();
            if abs_q > 0.0 {
                quantized /= abs_q.sqrt().sqrt();
            } else {
                quantized = 0.0;
            }
            q = quantized + if quantized > 0.0 { -amp } else { amp };
        }
        spectral[pos] = if q == 0.0 {
            0.0
        } else {
            q.signum() * q.abs().powf(4.0 / 3.0) * sf_scale
        };
    }
}

/// 解析 tns_data.
pub(super) fn parse_tns_data(br: &mut BitReader, is_short: bool) -> TaoResult<TnsData> {
    let mut data = TnsData {
        num_windows: if is_short { 8 } else { 1 },
        ..Default::default()
    };
    let max_order = if is_short { 7u32 } else { 12u32 };

    for w in 0..data.num_windows {
        let n_filt = br.read_bits(if is_short { 1 } else { 2 })? as usize;
        if n_filt > 4 {
            return Err(TaoError::InvalidData(format!(
                "AAC TNS 非法: window={}, n_filt={} 超出上限",
                w, n_filt
            )));
        }
        data.n_filt[w] = n_filt as u8;
        if n_filt == 0 {
            continue;
        }

        let coef_res = br.read_bit()? as usize;
        for filt in 0..n_filt {
            data.length[w][filt] = br.read_bits(if is_short { 4 } else { 6 })? as u8;
            let order = br.read_bits(if is_short { 3 } else { 5 })?;
            if order > max_order {
                return Err(TaoError::InvalidData(format!(
                    "AAC TNS 非法: window={}, filter={}, order={} 超出上限={}",
                    w, filt, order, max_order
                )));
            }
            data.order[w][filt] = order as u8;
            if order == 0 {
                continue;
            }

            data.direction[w][filt] = br.read_bit()? != 0;
            let coef_compress = br.read_bit()? as usize;
            let coef_len = coef_res + 3 - coef_compress;
            let map_idx = 2 * coef_compress + coef_res;
            for i in 0..order as usize {
                let q = br.read_bits(coef_len as u32)? as usize;
                data.coef[w][filt][i] = tns_coef_from_index(map_idx, q)?;
            }
        }
    }
    Ok(data)
}

/// 从 TNS 量化索引恢复滤波系数.
pub(super) fn tns_coef_from_index(map_idx: usize, q: usize) -> TaoResult<f32> {
    let val = match map_idx {
        0 => TNS_TMP2_MAP_0_3.get(q).copied(),
        1 => TNS_TMP2_MAP_0_4.get(q).copied(),
        2 => TNS_TMP2_MAP_1_3.get(q).copied(),
        3 => TNS_TMP2_MAP_1_4.get(q).copied(),
        _ => None,
    };
    val.ok_or_else(|| {
        TaoError::InvalidData(format!(
            "AAC TNS 系数索引越界: map_idx={}, q={}",
            map_idx, q
        ))
    })
}

/// 将 TNS 反射系数转换为 LPC 系数.
pub(super) fn compute_tns_lpc(coefs: &[f32]) -> [f32; 20] {
    let mut lpc = [0.0f32; 20];
    if coefs.is_empty() {
        return lpc;
    }
    for i in 0..coefs.len() {
        // 与 FFmpeg/FDK 行为保持一致: 反射系数到 LPC 递推使用负号.
        let r = -coefs[i];
        lpc[i] = r;
        for j in 0..((i + 1) >> 1) {
            let f = lpc[j];
            let b = lpc[i - 1 - j];
            lpc[j] = f + r * b;
            lpc[i - 1 - j] = b + r * f;
        }
    }
    lpc
}

/// 在频域上应用 TNS all-pole 滤波.
pub(super) fn apply_tns_data(
    spectral: &mut [f32],
    tns: &TnsData,
    info: &IcsInfo,
    swb_offset: &[usize],
    tns_max_bands: usize,
) {
    let mmm = tns_max_bands.min(info.max_sfb);
    if mmm == 0 || tns.num_windows == 0 {
        return;
    }

    for w in 0..tns.num_windows {
        let mut bottom = info.num_swb;
        for filt in 0..tns.n_filt[w] as usize {
            let top = bottom;
            bottom = top.saturating_sub(tns.length[w][filt] as usize);
            let order = tns.order[w][filt] as usize;
            if order == 0 {
                continue;
            }

            let start_band = bottom.min(mmm);
            let end_band = top.min(mmm);
            let start = swb_offset[start_band.min(swb_offset.len() - 1)];
            let end = swb_offset[end_band.min(swb_offset.len() - 1)];
            let size = end.saturating_sub(start);
            if size == 0 {
                continue;
            }

            let lpc = compute_tns_lpc(&tns.coef[w][filt][..order]);
            let reverse = tns.direction[w][filt];
            let mut pos = if reverse {
                (w * 128 + end.saturating_sub(1)) as isize
            } else {
                (w * 128 + start) as isize
            };
            let inc = if reverse { -1isize } else { 1isize };

            for m in 0..size {
                let idx = pos as usize;
                if idx >= spectral.len() {
                    break;
                }
                let mut acc = spectral[idx];
                let tap = m.min(order);
                for i in 1..=tap {
                    let src = (pos - (i as isize) * inc) as usize;
                    if src >= spectral.len() {
                        continue;
                    }
                    acc -= spectral[src] * lpc[i - 1];
                }
                spectral[idx] = acc;
                pos += inc;
            }
        }
    }
}

/// 跳过 gain_control_data (ISO 14496-3 Table 4.55).
pub(super) fn skip_gain_control_data(br: &mut BitReader, window_sequence: u32) -> TaoResult<()> {
    // [wd_num, wd_test, aloc_size]
    const GAIN_MODE: [[u8; 3]; 4] = [
        [1, 0, 5], // ONLY_LONG_SEQUENCE
        [2, 1, 2], // LONG_START_SEQUENCE
        [8, 0, 2], // EIGHT_SHORT_SEQUENCE
        [2, 1, 5], // LONG_STOP_SEQUENCE
    ];

    let mode = window_sequence as usize;
    if mode >= GAIN_MODE.len() {
        return Err(TaoError::InvalidData(format!(
            "AAC gain_control_data: 无效窗口序列 {}",
            window_sequence
        )));
    }
    let max_band = br.read_bits(2)? as usize;
    for _band in 0..max_band {
        for wd in 0..GAIN_MODE[mode][0] as usize {
            let adjust_num = br.read_bits(3)? as usize;
            for _ in 0..adjust_num {
                let aloc_size = if wd == 0 && GAIN_MODE[mode][1] != 0 {
                    4u32
                } else {
                    GAIN_MODE[mode][2] as u32
                };
                br.skip_bits(4 + aloc_size)?;
            }
        }
    }
    Ok(())
}

/// 反量化: iq = sign(x) * |x|^(4/3) * 2^(0.25 * (sf - 120))
pub(super) fn inverse_quantize(x: i32, sf: i32) -> f32 {
    if x == 0 {
        return 0.0;
    }
    let sign = if x > 0 { 1.0f32 } else { -1.0f32 };
    let abs_x = x.unsigned_abs() as usize;
    let pow_val = pow43_value(abs_x);
    let scale = inverse_quant_scale(sf);
    sign * pow_val * scale
}

fn pow43_value(abs_x: usize) -> f32 {
    const POW43_TABLE_MAX: usize = 8191;
    static POW43_TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = POW43_TABLE.get_or_init(|| {
        let mut t = vec![0.0f32; POW43_TABLE_MAX + 1];
        for (i, slot) in t.iter_mut().enumerate() {
            *slot = (i as f64).powf(4.0 / 3.0) as f32;
        }
        t
    });
    if abs_x <= POW43_TABLE_MAX {
        table[abs_x]
    } else {
        (abs_x as f64).powf(4.0 / 3.0) as f32
    }
}

fn inverse_quant_scale(sf: i32) -> f32 {
    const SF_MIN: i32 = -256;
    const SF_MAX: i32 = 511;
    static SCALE_TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = SCALE_TABLE.get_or_init(|| {
        let mut t = Vec::with_capacity((SF_MAX - SF_MIN + 1) as usize);
        for sfv in SF_MIN..=SF_MAX {
            t.push((2.0f64).powf(0.25 * (sfv - 120) as f64) as f32);
        }
        t
    });
    if (SF_MIN..=SF_MAX).contains(&sf) {
        table[(sf - SF_MIN) as usize]
    } else {
        (2.0f64).powf(0.25 * (sf - 120) as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IcsInfo, PulseData, Section, TnsData, apply_intensity_stereo, apply_ms_stereo,
        apply_pulse_data, apply_tns_data, compute_tns_lpc, inverse_quantize,
    };
    use crate::decoders::aac::tables::{INTENSITY_HCB, INTENSITY_HCB2, NOISE_HCB};

    fn inverse_quantize_reference(x: i32, sf: i32) -> f32 {
        if x == 0 {
            return 0.0;
        }
        let sign = if x > 0 { 1.0f32 } else { -1.0f32 };
        let abs_x = x.unsigned_abs() as f64;
        let pow_val = abs_x.powf(4.0 / 3.0) as f32;
        let scale = (2.0f64).powf(0.25 * (sf - 120) as f64) as f32;
        sign * pow_val * scale
    }

    #[test]
    fn test_inverse_quantize_matches_reference() {
        let samples = [
            (0, 120),
            (1, 120),
            (-1, 120),
            (7, 98),
            (-15, 140),
            (64, 60),
            (-91, 200),
            (1024, 180),
            (-4096, 96),
            (16384, 256),
        ];
        for (x, sf) in samples {
            let ours = inverse_quantize(x, sf);
            let reference = inverse_quantize_reference(x, sf);
            let err = (ours - reference).abs();
            assert!(
                err < 1e-5,
                "inverse_quantize 偏差超限: x={}, sf={}, ours={:.9}, ref={:.9}, err={:.9}",
                x,
                sf,
                ours,
                reference,
                err
            );
        }
    }

    fn make_long_ics(max_sfb: usize) -> IcsInfo {
        let mut group_lengths = [0usize; 8];
        let mut group_starts = [0usize; 8];
        group_lengths[0] = 1;
        group_starts[0] = 0;
        IcsInfo {
            window_sequence: 0,
            window_shape: 0,
            max_sfb,
            num_swb: max_sfb,
            num_window_groups: 1,
            window_group_lengths: group_lengths,
            window_group_starts: group_starts,
        }
    }

    fn make_short_ics(max_sfb: usize, group0_len: usize, group1_len: usize) -> IcsInfo {
        let mut group_lengths = [0usize; 8];
        let mut group_starts = [0usize; 8];
        group_lengths[0] = group0_len;
        group_lengths[1] = group1_len;
        group_starts[0] = 0;
        group_starts[1] = group0_len;
        IcsInfo {
            window_sequence: 2,
            window_shape: 0,
            max_sfb,
            num_swb: max_sfb,
            num_window_groups: 2,
            window_group_lengths: group_lengths,
            window_group_starts: group_starts,
        }
    }

    fn apply_ms_reference(
        left: &mut [f32],
        right: &mut [f32],
        info: &IcsInfo,
        ms_used: &[bool],
        left_band_types: Option<&[u8]>,
        right_band_types: Option<&[u8]>,
        swb_offset: &[usize],
    ) {
        let is_short = info.window_sequence == 2;
        for group in 0..info.num_window_groups {
            for sfb in 0..info.max_sfb {
                let mask_idx = group * info.max_sfb + sfb;
                if !ms_used.get(mask_idx).copied().unwrap_or(false) {
                    continue;
                }
                if let (Some(left_types), Some(right_types)) = (left_band_types, right_band_types) {
                    let left_bt = left_types.get(mask_idx).copied().unwrap_or(0);
                    let right_bt = right_types.get(mask_idx).copied().unwrap_or(0);
                    if left_bt >= NOISE_HCB || right_bt >= NOISE_HCB {
                        continue;
                    }
                }
                let start = swb_offset[sfb];
                let end = swb_offset[sfb + 1];
                if is_short {
                    let group_len = info.window_group_lengths[group];
                    let group_start = info.window_group_starts[group];
                    for win in 0..group_len {
                        let win_base = (group_start + win) * 128;
                        for line in start..end {
                            let idx = win_base + line;
                            let l = left[idx];
                            let r = right[idx];
                            left[idx] = l + r;
                            right[idx] = l - r;
                        }
                    }
                } else {
                    for idx in start..end {
                        let l = left[idx];
                        let r = right[idx];
                        left[idx] = l + r;
                        right[idx] = l - r;
                    }
                }
            }
        }
    }

    fn apply_is_reference(
        left: &mut [f32],
        right: &mut [f32],
        info: &IcsInfo,
        right_band_types: &[u8],
        right_scale_factors: &[i32],
        ms_used: Option<&[bool]>,
        swb_offset: &[usize],
    ) {
        let is_short = info.window_sequence == 2;
        for group in 0..info.num_window_groups {
            for sfb in 0..info.max_sfb {
                let band_idx = group * info.max_sfb + sfb;
                let sf_idx = group * info.num_swb + sfb;
                let band_type = right_band_types.get(band_idx).copied().unwrap_or(0);
                if band_type != INTENSITY_HCB && band_type != INTENSITY_HCB2 {
                    continue;
                }
                let is_position = right_scale_factors.get(sf_idx).copied().unwrap_or(0) as f32;
                let mut sign = if band_type == INTENSITY_HCB2 {
                    -1.0
                } else {
                    1.0
                };
                if ms_used
                    .and_then(|mask| mask.get(band_idx))
                    .copied()
                    .unwrap_or(false)
                {
                    sign = -sign;
                }
                let scale = sign * 0.5f32.powf(0.25 * is_position);
                let start = swb_offset[sfb];
                let end = swb_offset[sfb + 1];
                if is_short {
                    let group_len = info.window_group_lengths[group];
                    let group_start = info.window_group_starts[group];
                    for win in 0..group_len {
                        let win_base = (group_start + win) * 128;
                        for line in start..end {
                            let idx = win_base + line;
                            right[idx] = left[idx] * scale;
                        }
                    }
                } else {
                    for idx in start..end {
                        right[idx] = left[idx] * scale;
                    }
                }
            }
        }
    }

    fn apply_pulse_reference(
        spectral: &mut [f32],
        pulse: &PulseData,
        sections: &[Section],
        scale_factors: &[i32],
        info: &IcsInfo,
        swb_offset: &[usize],
    ) {
        if pulse.num_pulse == 0 || info.num_swb == 0 || info.num_swb >= swb_offset.len() {
            return;
        }
        let mut band_types = vec![0u8; info.max_sfb];
        for section in sections {
            if section.group != 0 {
                continue;
            }
            let end = section.sect_end.min(info.max_sfb);
            for sfb in section.sect_start..end {
                band_types[sfb] = section.sect_cb;
            }
        }
        let max_line = swb_offset[info.num_swb];
        let mut sfb_idx = 0usize;
        for i in 0..pulse.num_pulse.min(4) {
            let pos = pulse.pos[i];
            if pos >= spectral.len() || pos >= max_line {
                continue;
            }
            while sfb_idx + 1 < info.num_swb && swb_offset[sfb_idx + 1] <= pos {
                sfb_idx += 1;
            }
            let band_type = band_types.get(sfb_idx).copied().unwrap_or(0);
            if !(1..=11).contains(&band_type) {
                continue;
            }
            let sf = scale_factors.get(sfb_idx).copied().unwrap_or(0);
            let sf_scale = 2.0f32.powf(0.25 * (sf - 120) as f32);
            if !sf_scale.is_finite() || sf_scale == 0.0 {
                continue;
            }
            let co = spectral[pos];
            let amp = pulse.amp[i] as f32;
            let mut q = -amp;
            if co != 0.0 {
                let mut quantized = co / sf_scale;
                let abs_q = quantized.abs();
                if abs_q > 0.0 {
                    quantized /= abs_q.sqrt().sqrt();
                } else {
                    quantized = 0.0;
                }
                q = quantized + if quantized > 0.0 { -amp } else { amp };
            }
            spectral[pos] = if q == 0.0 {
                0.0
            } else {
                q.signum() * q.abs().powf(4.0 / 3.0) * sf_scale
            };
        }
    }

    fn compute_tns_lpc_reference(coefs: &[f32]) -> [f32; 20] {
        let mut lpc = [0.0f32; 20];
        if coefs.is_empty() {
            return lpc;
        }
        for i in 0..coefs.len() {
            let r = -coefs[i];
            lpc[i] = r;
            for j in 0..((i + 1) >> 1) {
                let f = lpc[j];
                let b = lpc[i - 1 - j];
                lpc[j] = f + r * b;
                lpc[i - 1 - j] = b + r * f;
            }
        }
        lpc
    }

    fn apply_tns_reference(
        spectral: &mut [f32],
        tns: &TnsData,
        info: &IcsInfo,
        swb_offset: &[usize],
        tns_max_bands: usize,
    ) {
        let mmm = tns_max_bands.min(info.max_sfb);
        if mmm == 0 || tns.num_windows == 0 {
            return;
        }
        for w in 0..tns.num_windows {
            let mut bottom = info.num_swb;
            for filt in 0..tns.n_filt[w] as usize {
                let top = bottom;
                bottom = top.saturating_sub(tns.length[w][filt] as usize);
                let order = tns.order[w][filt] as usize;
                if order == 0 {
                    continue;
                }
                let start_band = bottom.min(mmm);
                let end_band = top.min(mmm);
                let start = swb_offset[start_band.min(swb_offset.len() - 1)];
                let end = swb_offset[end_band.min(swb_offset.len() - 1)];
                let size = end.saturating_sub(start);
                if size == 0 {
                    continue;
                }
                let lpc = compute_tns_lpc_reference(&tns.coef[w][filt][..order]);
                let reverse = tns.direction[w][filt];
                let mut pos = if reverse {
                    (w * 128 + end.saturating_sub(1)) as isize
                } else {
                    (w * 128 + start) as isize
                };
                let inc = if reverse { -1isize } else { 1isize };
                for m in 0..size {
                    let idx = pos as usize;
                    if idx >= spectral.len() {
                        break;
                    }
                    let mut acc = spectral[idx];
                    let tap = m.min(order);
                    for i in 1..=tap {
                        let src = (pos - (i as isize) * inc) as usize;
                        if src >= spectral.len() {
                            continue;
                        }
                        acc -= spectral[src] * lpc[i - 1];
                    }
                    spectral[idx] = acc;
                    pos += inc;
                }
            }
        }
    }

    fn assert_slice_close(lhs: &[f32], rhs: &[f32], tol: f32, tag: &str) {
        assert_eq!(lhs.len(), rhs.len(), "{} 长度不一致", tag);
        let mut max_err = 0.0f32;
        for (&a, &b) in lhs.iter().zip(rhs.iter()) {
            let e = (a - b).abs();
            if e > max_err {
                max_err = e;
            }
        }
        assert!(
            max_err < tol,
            "{} 偏差超限: max_err={:.9}, tol={:.9}",
            tag,
            max_err,
            tol
        );
    }

    #[test]
    fn test_apply_ms_stereo_matches_reference_long() {
        let info = make_long_ics(3);
        let swb_offset = [0usize, 4, 8, 12];
        let mut left = (0..12).map(|v| v as f32 * 0.1 + 0.3).collect::<Vec<_>>();
        let mut right = (0..12).map(|v| 1.7 - v as f32 * 0.05).collect::<Vec<_>>();
        let mut left_ref = left.clone();
        let mut right_ref = right.clone();
        let ms_used = [true, false, true];
        let left_bt = [1u8, 1, NOISE_HCB];
        let right_bt = [1u8, 1, 1];

        apply_ms_stereo(
            &mut left,
            &mut right,
            &info,
            &ms_used,
            Some(&left_bt),
            Some(&right_bt),
            &swb_offset,
        );
        apply_ms_reference(
            &mut left_ref,
            &mut right_ref,
            &info,
            &ms_used,
            Some(&left_bt),
            Some(&right_bt),
            &swb_offset,
        );

        assert_slice_close(&left, &left_ref, 1e-7, "MS long left");
        assert_slice_close(&right, &right_ref, 1e-7, "MS long right");
    }

    #[test]
    fn test_apply_ms_stereo_matches_reference_short_grouped() {
        let info = make_short_ics(2, 3, 5);
        let swb_offset = [0usize, 6, 12];
        let mut left = vec![0.0f32; 8 * 128];
        let mut right = vec![0.0f32; 8 * 128];
        for i in 0..left.len() {
            left[i] = (i as f32 * 0.001).sin();
            right[i] = (i as f32 * 0.002).cos();
        }
        let mut left_ref = left.clone();
        let mut right_ref = right.clone();
        let ms_used = [true, false, false, true];
        let left_bt = [1u8, 1, 1, 1];
        let right_bt = [1u8, 1, 1, 1];

        apply_ms_stereo(
            &mut left,
            &mut right,
            &info,
            &ms_used,
            Some(&left_bt),
            Some(&right_bt),
            &swb_offset,
        );
        apply_ms_reference(
            &mut left_ref,
            &mut right_ref,
            &info,
            &ms_used,
            Some(&left_bt),
            Some(&right_bt),
            &swb_offset,
        );

        assert_slice_close(&left, &left_ref, 1e-7, "MS short left");
        assert_slice_close(&right, &right_ref, 1e-7, "MS short right");
    }

    #[test]
    fn test_apply_intensity_stereo_matches_reference_long() {
        let info = make_long_ics(3);
        let swb_offset = [0usize, 4, 8, 12];
        let mut left = (0..12).map(|v| (v as f32 * 0.3).sin()).collect::<Vec<_>>();
        let mut right = vec![0.0f32; 12];
        let mut left_ref = left.clone();
        let mut right_ref = right.clone();
        let right_bt = [INTENSITY_HCB, 1, INTENSITY_HCB2];
        let right_sf = [8i32, 0, -4];
        let ms_used = [true, false, false];

        apply_intensity_stereo(
            &mut left,
            &mut right,
            &info,
            &right_bt,
            &right_sf,
            Some(&ms_used),
            &swb_offset,
        );
        apply_is_reference(
            &mut left_ref,
            &mut right_ref,
            &info,
            &right_bt,
            &right_sf,
            Some(&ms_used),
            &swb_offset,
        );

        assert_slice_close(&left, &left_ref, 1e-7, "IS long left");
        assert_slice_close(&right, &right_ref, 1e-7, "IS long right");
    }

    #[test]
    fn test_apply_intensity_stereo_matches_reference_short_grouped() {
        let info = make_short_ics(2, 2, 6);
        let swb_offset = [0usize, 8, 16];
        let mut left = vec![0.0f32; 8 * 128];
        let mut right = vec![0.0f32; 8 * 128];
        for i in 0..left.len() {
            left[i] = (i as f32 * 0.004).cos();
            right[i] = (i as f32 * 0.003).sin();
        }
        let mut left_ref = left.clone();
        let mut right_ref = right.clone();
        let right_bt = [INTENSITY_HCB, 1, 1, INTENSITY_HCB2];
        let right_sf = [6i32, 0, 0, -8];
        let ms_used = [false, false, false, true];

        apply_intensity_stereo(
            &mut left,
            &mut right,
            &info,
            &right_bt,
            &right_sf,
            Some(&ms_used),
            &swb_offset,
        );
        apply_is_reference(
            &mut left_ref,
            &mut right_ref,
            &info,
            &right_bt,
            &right_sf,
            Some(&ms_used),
            &swb_offset,
        );

        assert_slice_close(&left, &left_ref, 1e-7, "IS short left");
        assert_slice_close(&right, &right_ref, 1e-7, "IS short right");
    }

    #[test]
    fn test_apply_pulse_data_matches_reference() {
        let info = make_long_ics(4);
        let swb_offset = [0usize, 4, 8, 12, 16];
        let sections = vec![Section {
            group: 0,
            sect_cb: 5,
            sect_start: 0,
            sect_end: 4,
        }];
        let scale_factors = [120i32, 116, 124, 112];
        let pulse = PulseData {
            num_pulse: 2,
            pos: [2, 10, 0, 0],
            amp: [3, 2, 0, 0],
        };
        let mut spectral = (0..16)
            .map(|i| {
                if i % 2 == 0 {
                    0.5 + i as f32 * 0.1
                } else {
                    -0.7 + i as f32 * 0.05
                }
            })
            .collect::<Vec<_>>();
        let mut reference = spectral.clone();

        apply_pulse_data(
            &mut spectral,
            &pulse,
            &sections,
            &scale_factors,
            &info,
            &swb_offset,
        );
        apply_pulse_reference(
            &mut reference,
            &pulse,
            &sections,
            &scale_factors,
            &info,
            &swb_offset,
        );

        assert_slice_close(&spectral, &reference, 1e-6, "pulse");
    }

    #[test]
    fn test_compute_tns_lpc_matches_reference() {
        let coefs = [0.12f32, -0.31, 0.27, -0.08, 0.05];
        let ours = compute_tns_lpc(&coefs);
        let reference = compute_tns_lpc_reference(&coefs);
        assert_slice_close(
            &ours[..coefs.len()],
            &reference[..coefs.len()],
            1e-7,
            "tns_lpc",
        );
    }

    #[test]
    fn test_apply_tns_data_matches_reference() {
        let info = make_long_ics(4);
        let swb_offset = [0usize, 4, 8, 12, 16];
        let mut tns = TnsData {
            num_windows: 1,
            ..TnsData::default()
        };
        tns.n_filt[0] = 1;
        tns.length[0][0] = 4;
        tns.order[0][0] = 3;
        tns.direction[0][0] = false;
        tns.coef[0][0][0] = 0.21;
        tns.coef[0][0][1] = -0.14;
        tns.coef[0][0][2] = 0.09;

        let mut spectral = (0..1024)
            .map(|i| (i as f32 * 0.007).sin() * 0.8 + (i as f32 * 0.003).cos() * 0.2)
            .collect::<Vec<_>>();
        let mut reference = spectral.clone();

        apply_tns_data(&mut spectral, &tns, &info, &swb_offset, 4);
        apply_tns_reference(&mut reference, &tns, &info, &swb_offset, 4);

        assert_slice_close(&spectral, &reference, 1e-6, "tns_apply");
    }
}

// ============================================================
// Decoder trait 实现
// ============================================================
