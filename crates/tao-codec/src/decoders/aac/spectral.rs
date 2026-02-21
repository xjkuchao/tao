//! AAC 频谱处理: 辅助结构体、量化、立体声、TNS 等.

use std::cell::Cell;

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
                let target = inverse_quantize(1, sf).abs();
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
                    // short 窗口必须按每个窗单独解码, 不能把组内多个窗拼接后一次解码.
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

/// 跳过 pulse_data
pub(super) fn skip_pulse_data(br: &mut BitReader) -> TaoResult<()> {
    let num_pulse = br.read_bits(2)? + 1;
    let _pulse_start_sfb = br.read_bits(6)?;
    for _ in 0..num_pulse {
        let _offset = br.read_bits(5)?;
        let _amp = br.read_bits(4)?;
    }
    Ok(())
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
    lpc[0] = coefs[0];
    for i in 1..coefs.len() {
        let r = coefs[i];
        for j in 0..(i / 2) {
            let tmp_coef = r * lpc[j];
            lpc[j] += r * lpc[i - 1 - j];
            lpc[i - 1 - j] += tmp_coef;
        }
        if i % 2 != 0 {
            let j = i / 2;
            lpc[j] += r * lpc[j];
        }
        lpc[i] = r;
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
            let mut pos = if tns.direction[w][filt] {
                (w * 128 + end.saturating_sub(1)) as isize
            } else {
                (w * 128 + start) as isize
            };
            let inc = if tns.direction[w][filt] {
                -1isize
            } else {
                1isize
            };

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
    let abs_x = x.unsigned_abs() as f32;
    let pow_val = abs_x.powf(4.0 / 3.0);
    let scale = 2.0f32.powf(0.25 * (sf - 120) as f32);
    sign * pow_val * scale
}

// ============================================================
// Decoder trait 实现
// ============================================================
