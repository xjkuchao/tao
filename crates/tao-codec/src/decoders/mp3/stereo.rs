//! MP3 立体声处理 (Stereo Processing)
//!
//! 处理 Joint Stereo 的 MS 与 Intensity Stereo 逻辑.
//! 规则参考 ISO/IEC 11172-3 的描述, 并与 FFmpeg/Symphonia 行为对齐.

use super::data::GranuleContext;
use super::header::{ChannelMode, Mp3Header, MpegVersion};
use super::side_info::Granule;
use super::tables::{SFB_WIDTH_LONG, SFB_WIDTH_SHORT, build_sfb_long_bounds, samplerate_index};
use std::f32::consts::FRAC_1_SQRT_2;

/// MPEG-1 强度立体声比例表
/// is_pos=0..6, is_pos==7 为无效.
#[allow(clippy::excessive_precision)]
const IS_RATIOS_MPEG1: [(f32, f32); 7] = [
    (0.000000000, 1.000000000),
    (0.211324865, 0.788675135),
    (0.366025404, 0.633974596),
    (0.500000000, 0.500000000),
    (0.633974596, 0.366025404),
    (0.788675135, 0.211324865),
    (1.000000000, 0.000000000),
];

const IS_POS_INVALID_MPEG1: u8 = 7;
const IS_POS_INVALID_MPEG2: u8 = 64;

fn process_mid_side(l: &mut [f32], r: &mut [f32]) {
    let scale = if std::env::var("TAO_MP3_MS_SCALE_HALF").is_ok() {
        0.5f32
    } else {
        FRAC_1_SQRT_2
    };
    for (l_val, r_val) in l.iter_mut().zip(r.iter_mut()) {
        let m = *l_val;
        let s = *r_val;
        *l_val = (m + s) * scale;
        *r_val = (m - s) * scale;
    }
}

fn process_intensity_band(
    intensity_pos: u8,
    ms_stereo: bool,
    version: MpegVersion,
    mpeg2_sh: u8,
    l: &mut [f32],
    r: &mut [f32],
) {
    match version {
        MpegVersion::Mpeg1 => {
            if intensity_pos < IS_POS_INVALID_MPEG1 {
                let (kl, kr) = IS_RATIOS_MPEG1[intensity_pos as usize];
                for (l_val, r_val) in l.iter_mut().zip(r.iter_mut()) {
                    let v = *l_val;
                    *l_val = v * kl;
                    *r_val = v * kr;
                }
            } else if ms_stereo {
                process_mid_side(l, r);
            }
        }
        _ => {
            if intensity_pos < IS_POS_INVALID_MPEG2 {
                // MPEG-2/2.5 intensity stereo 比例:
                // kr = 2^(-(((is_pos + 1) >> 1) << mpeg2_sh) / 4)
                let exp_q2 = (((u32::from(intensity_pos) + 1) >> 1) << mpeg2_sh) as f32;
                let mut kl = 1.0f32;
                let mut kr = 2.0f32.powf(-exp_q2 * 0.25);
                if (intensity_pos & 1) != 0 {
                    kl = kr;
                    kr = 1.0;
                }
                let s = if ms_stereo { 2.0f32.sqrt() } else { 1.0 };
                let kl = kl * s;
                let kr = kr * s;
                for (l_val, r_val) in l.iter_mut().zip(r.iter_mut()) {
                    let v = *l_val;
                    *l_val = v * kl;
                    *r_val = v * kr;
                }
            } else if ms_stereo {
                process_mid_side(l, r);
            }
        }
    }
}

fn is_zero_band(samples: &[f32]) -> bool {
    !samples.iter().any(|&v| v != 0.0)
}

fn short_intensity_pos(scalefac: &[u8; 40], sfb: usize, win: usize, mixed: bool) -> u8 {
    if mixed {
        if sfb < 3 {
            return scalefac[sfb];
        }
        if sfb >= 12 {
            let idx = 32 + win;
            return scalefac[idx.min(39)];
        }
        let idx = 8 + (sfb - 3) * 3 + win;
        scalefac[idx.min(39)]
    } else {
        if sfb >= 12 {
            let idx = 33 + win;
            return scalefac[idx.min(39)];
        }
        let idx = sfb * 3 + win;
        scalefac[idx.min(39)]
    }
}

fn mixed_long_end(widths: &[usize; 22]) -> usize {
    let mut acc = 0usize;
    for (i, w) in widths.iter().enumerate() {
        acc += *w;
        if acc >= 36 {
            return i + 1;
        }
    }
    8
}

fn process_intensity_long_block(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    ms_stereo: bool,
    version: MpegVersion,
    mpeg2_sh: u8,
    sample_rate: u32,
    max_bound: usize,
) -> usize {
    let bounds = build_sfb_long_bounds(sample_rate);
    let mut is_pos = [0u8; 22];
    is_pos.copy_from_slice(&r_data.scalefac[..22]);
    is_pos[21] = is_pos[20];

    let mut bound = max_bound;
    let rzero = r_data.rzero.min(576);

    for sfb in (0..22).rev() {
        let start = bounds[sfb];
        let end = bounds[sfb + 1];

        let zero = if start >= rzero {
            true
        } else {
            is_zero_band(&r_data.xr[start..end])
        };

        if zero {
            process_intensity_band(
                is_pos[sfb],
                ms_stereo,
                version,
                mpeg2_sh,
                &mut l_data.xr[start..end],
                &mut r_data.xr[start..end],
            );
            bound = start;
        } else {
            break;
        }
    }

    bound
}

#[allow(clippy::too_many_arguments)]
fn process_intensity_short_block(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    granule: &Granule,
    ms_stereo: bool,
    version: MpegVersion,
    mpeg2_sh: u8,
    sample_rate: u32,
    max_bound: usize,
) -> usize {
    let sr_idx = samplerate_index(sample_rate);
    let short_width = &SFB_WIDTH_SHORT[sr_idx];
    let long_width = &SFB_WIDTH_LONG[sr_idx];

    let mixed = granule.mixed_block_flag;
    let short_start = if mixed { 3 } else { 0 };
    let long_end = if mixed { mixed_long_end(long_width) } else { 0 };

    let short_region_start = if mixed {
        long_width.iter().take(long_end).sum()
    } else {
        0
    };

    let mut sfb_starts = Vec::with_capacity(13);
    let mut acc = short_region_start;
    for (sfb, width) in short_width.iter().enumerate().take(13).skip(short_start) {
        sfb_starts.push((sfb, acc));
        acc += width * 3;
    }

    let mut window_is_zero = [true; 3];
    let mut bound = max_bound;
    let mut found_bound = false;
    let rzero = r_data.rzero.min(576);

    for (sfb, start) in sfb_starts.iter().rev().copied() {
        let width = short_width[sfb];
        let s0 = start;
        let s1 = s0 + width;
        let s2 = s1 + width;
        let s3 = s2 + width;

        let windows = [(2, s2, s3), (1, s1, s2), (0, s0, s1)];
        for (win, w_start, w_end) in windows {
            let zero = if w_start >= rzero {
                true
            } else {
                is_zero_band(&r_data.xr[w_start..w_end])
            };

            window_is_zero[win] = window_is_zero[win] && zero;

            if window_is_zero[win] {
                let is_pos = short_intensity_pos(&r_data.scalefac, sfb, win, mixed);
                process_intensity_band(
                    is_pos,
                    ms_stereo,
                    version,
                    mpeg2_sh,
                    &mut l_data.xr[w_start..w_end],
                    &mut r_data.xr[w_start..w_end],
                );
            } else if ms_stereo {
                process_mid_side(
                    &mut l_data.xr[w_start..w_end],
                    &mut r_data.xr[w_start..w_end],
                );
            }
        }

        bound = s0;
        found_bound = !window_is_zero[0] && !window_is_zero[1] && !window_is_zero[2];
        if found_bound {
            break;
        }
    }

    if !found_bound && mixed {
        let long_bounds = build_sfb_long_bounds(sample_rate);
        for sfb in (0..long_end).rev() {
            let start = long_bounds[sfb];
            let end = long_bounds[sfb + 1];
            let zero = if start >= rzero {
                true
            } else {
                is_zero_band(&r_data.xr[start..end])
            };

            if zero {
                let is_pos = r_data.scalefac[sfb];
                process_intensity_band(
                    is_pos,
                    ms_stereo,
                    version,
                    mpeg2_sh,
                    &mut l_data.xr[start..end],
                    &mut r_data.xr[start..end],
                );
                bound = start;
            } else {
                break;
            }
        }
    }

    bound
}

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
    let mut ms_stereo = (mode_ext & 0x2) != 0;
    if std::env::var("TAO_MP3_DISABLE_MS").is_ok() {
        ms_stereo = false;
    }

    if !intensity_stereo && !ms_stereo {
        return;
    }

    let (l_data, r_data) = {
        let row = &mut granule_data[gr];
        let (l_slice, r_slice) = row.split_at_mut(1);
        (&mut l_slice[0], &mut r_slice[0])
    };

    let l_gr = &granules[gr][0];

    let end = l_data.rzero.max(r_data.rzero).min(576);

    let intensity_enabled = intensity_stereo;
    let mpeg2_sh = (granules[gr][1].scalefac_compress & 1) as u8;

    let is_bound = if intensity_enabled {
        if l_gr.windows_switching_flag && l_gr.block_type == 2 {
            process_intensity_short_block(
                l_data,
                r_data,
                l_gr,
                ms_stereo,
                header.version,
                mpeg2_sh,
                sample_rate,
                end,
            )
        } else {
            process_intensity_long_block(
                l_data,
                r_data,
                ms_stereo,
                header.version,
                mpeg2_sh,
                sample_rate,
                end,
            )
        }
    } else {
        end
    };

    if ms_stereo && is_bound > 0 {
        let bound = is_bound.min(576);
        process_mid_side(&mut l_data.xr[..bound], &mut r_data.xr[..bound]);
    }

    if intensity_enabled || ms_stereo {
        l_data.rzero = end;
        r_data.rzero = end;
    }
}
