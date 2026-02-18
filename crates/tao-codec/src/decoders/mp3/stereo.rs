//! MP3 立体声处理 (Stereo Processing)
//!
//! 采用 FFmpeg `compute_stereo` 的处理策略：
//! - 长块: 按 SFB 从高到低进行 IS + MS 退化。
//! - 短块: 按 (SFB, 窗口) 从高频到低频，并按窗口分别判断是否退化到 MS。
//! - 对应 `s` 为 MS，`i` 为 Intensity Stereo。

use super::data::GranuleContext;
use super::header::{ChannelMode, Mp3Header};
use super::side_info::Granule;
use super::tables::{SFB_WIDTH_LONG, SFB_WIDTH_SHORT, samplerate_index};
use std::f32::consts::FRAC_1_SQRT_2;

/// IS 比率表 (MPEG-1)
/// 对应 `is_pos = 0..6` 的强度刻度。`is_pos == 7` 视为无效。
#[allow(clippy::excessive_precision)]
const IS_RATIOS: [(f32, f32); 7] = [
    (0.000000000, 1.000000000),
    (0.211324865, 0.788675135),
    (0.366025404, 0.633974596),
    (0.500000000, 0.500000000),
    (0.633974596, 0.366025404),
    (0.788675135, 0.211324865),
    (1.000000000, 0.000000000),
];

/// 立体声处理
pub fn process_stereo(
    gr: usize,
    header: &Mp3Header,
    granule_data: &mut [[GranuleContext; 2]; 2],
    granules: &[[Granule; 2]; 2],
    sample_rate: u32,
) {
    if header.mode != ChannelMode::JointStereo {
        return;
    }

    let mode_ext = header.mode_extension;
    let intensity_stereo =
        (mode_ext & 0x1) != 0 && std::env::var("TAO_MP3_DISABLE_INTENSITY").is_err();
    let ms_stereo = (mode_ext & 0x2) != 0;

    if !intensity_stereo && !ms_stereo {
        return;
    }

    let (l_data, r_data) = {
        let row = &mut granule_data[gr];
        let (l_slice, r_slice) = row.split_at_mut(1);
        (&mut l_slice[0], &mut r_slice[0])
    };

    let l_gr = &granules[gr][0];

    if l_gr.windows_switching_flag && l_gr.block_type == 2 {
        process_stereo_short(
            l_data,
            r_data,
            l_gr,
            ms_stereo,
            intensity_stereo,
            sample_rate,
        );
    } else {
        process_stereo_long(
            l_data,
            r_data,
            l_gr,
            ms_stereo,
            intensity_stereo,
            sample_rate,
        );
    }
}

fn process_stereo_long(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    _granule: &Granule,
    ms_stereo: bool,
    intensity_stereo: bool,
    sample_rate: u32,
) {
    let sfb_width = &SFB_WIDTH_LONG[samplerate_index(sample_rate)];
    let sf_max = 7usize;

    let mut l_start = [0usize; 22];
    let mut acc = 0usize;
    for i in 0..22 {
        l_start[i] = acc;
        acc += sfb_width[i];
    }

    let mut non_zero_found = false;

    if !intensity_stereo {
        if ms_stereo {
            for (l, r) in l_data.xr.iter_mut().zip(r_data.xr.iter_mut()) {
                let m = *l;
                let s = *r;
                *l = (m + s) * FRAC_1_SQRT_2;
                *r = (m - s) * FRAC_1_SQRT_2;
            }
        }
        return;
    }

    for i in (0..22).rev() {
        let len = sfb_width[i];
        let start = l_start[i];

        let mut is_non_zero = false;
        if !non_zero_found {
            for j in 0..len {
                if r_data.xr[start + j] != 0.0 {
                    is_non_zero = true;
                    break;
                }
            }
        }

        let mut do_ms = non_zero_found || is_non_zero;
        let is_pos = if !do_ms {
            let idx = if i == 21 { 20 } else { i };
            r_data.scalefac[idx] as usize
        } else {
            0
        };

        if !do_ms && is_pos >= sf_max {
            do_ms = true;
        }

        if do_ms {
            non_zero_found = true;
            if ms_stereo {
                for j in 0..len {
                    let idx = start + j;
                    let m = l_data.xr[idx];
                    let s = r_data.xr[idx];
                    l_data.xr[idx] = (m + s) * FRAC_1_SQRT_2;
                    r_data.xr[idx] = (m - s) * FRAC_1_SQRT_2;
                }
            }
            continue;
        }

        let (kl, kr) = IS_RATIOS[is_pos];
        for j in 0..len {
            let idx = start + j;
            let m = l_data.xr[idx];
            l_data.xr[idx] = m * kl;
            r_data.xr[idx] = m * kr;
        }
    }
}

fn process_stereo_short(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    granule: &Granule,
    ms_stereo: bool,
    intensity_stereo: bool,
    sample_rate: u32,
) {
    let sr_idx = samplerate_index(sample_rate);
    let long_width = &SFB_WIDTH_LONG[sr_idx];
    let short_width = &SFB_WIDTH_SHORT[sr_idx];

    let (long_end, short_start) = if granule.mixed_block_flag {
        let long_end = if sr_idx <= 2 { 8 } else { 6 };
        (long_end, 3)
    } else {
        (0, 0)
    };

    let short_region_start = if long_end == 0 {
        0
    } else {
        long_width.iter().take(long_end).sum()
    };

    let sf_max = 7usize;
    let mut non_zero_found = [false, false, false];

    let mut short_start_offset = [0usize; 13];
    let mut acc = 0usize;
    for i in 0..13 {
        short_start_offset[i] = acc;
        acc += short_width[i];
    }
    let short_start_skip = if short_start == 0 {
        0
    } else {
        short_start_offset[short_start]
    };

    if !intensity_stereo {
        if ms_stereo {
            for (l, r) in l_data.xr.iter_mut().zip(r_data.xr.iter_mut()) {
                let m = *l;
                let s = *r;
                *l = (m + s) * FRAC_1_SQRT_2;
                *r = (m - s) * FRAC_1_SQRT_2;
            }
        }
        return;
    }

    let mut k = (13 - short_start) * 3 + long_end - 3;
    for i in (short_start..=12).rev() {
        if i != 12 {
            k -= 3;
        }

        let len = short_width[i];
        let band_start = short_region_start + short_start_offset[i] - short_start_skip;

        for win in (0..3).rev() {
            let mut is_non_zero = false;
            let win_start = band_start + win * len;
            for j in 0..len {
                let idx = win_start + j;
                if r_data.xr[idx] != 0.0 {
                    is_non_zero = true;
                    break;
                }
            }

            let mut do_ms = non_zero_found[win] || is_non_zero;

            let is_pos = if !do_ms {
                let pos_idx = k + win;
                r_data.scalefac[pos_idx] as usize
            } else {
                0
            };

            if !do_ms && is_pos >= sf_max {
                do_ms = true;
            }

            if do_ms {
                non_zero_found[win] = true;
                if ms_stereo {
                    for j in 0..len {
                        let idx = win_start + j;
                        let m = l_data.xr[idx];
                        let s = r_data.xr[idx];
                        l_data.xr[idx] = (m + s) * FRAC_1_SQRT_2;
                        r_data.xr[idx] = (m - s) * FRAC_1_SQRT_2;
                    }
                }
            } else {
                let (kl, kr) = IS_RATIOS[is_pos];
                for j in 0..len {
                    let idx = win_start + j;
                    let m = l_data.xr[idx];
                    l_data.xr[idx] = m * kl;
                    r_data.xr[idx] = m * kr;
                }
            }
        }
    }
}
