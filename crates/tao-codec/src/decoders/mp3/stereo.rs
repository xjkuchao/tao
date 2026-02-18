//! MP3 立体声处理 (Stereo Processing)
//!
//! 支持 MS Stereo 和 Intensity Stereo (MPEG-1)
//! 参考 ISO 11172-3 和 minimp3 实现.

use super::data::GranuleContext;
use super::header::{ChannelMode, Mp3Header};
use super::side_info::Granule;
use super::tables::{SFB_WIDTH_LONG, SFB_WIDTH_SHORT, samplerate_index};
use std::f32::consts::FRAC_1_SQRT_2;

/// IS 比率表 (MPEG-1)
/// 基于 ISO 11172-3: is_ratio = tan(is_pos * PI/12)
/// kL = is_ratio / (1 + is_ratio)
/// kR = 1 / (1 + is_ratio)
/// 每对为 (kL, kR), is_pos = 0..6
#[allow(clippy::excessive_precision)]
const IS_RATIOS: [(f32, f32); 7] = [
    (0.000000000, 1.000000000), // is_pos = 0: tan(0) = 0
    (0.211324865, 0.788675135), // is_pos = 1: tan(π/12) ≈ 0.2679
    (0.366025404, 0.633974596), // is_pos = 2: tan(π/6) ≈ 0.5774
    (0.500000000, 0.500000000), // is_pos = 3: tan(π/4) = 1.0
    (0.633974596, 0.366025404), // is_pos = 4: tan(π/3) ≈ 1.7321
    (0.788675135, 0.211324865), // is_pos = 5: tan(5π/12) ≈ 3.7321
    (1.000000000, 0.000000000), // is_pos = 6: tan(π/2) = ∞
];

/// 立体声处理
pub fn process_stereo(
    gr: usize,
    header: &Mp3Header,
    granule_data: &mut [[GranuleContext; 2]; 2],
    granules: &[[Granule; 2]; 2],
    sample_rate: u32,
) {
    // 仅 Joint Stereo 模式需要处理
    if header.mode != ChannelMode::JointStereo {
        return;
    }

    let mode_ext = header.mode_extension;
    let intensity_stereo = (mode_ext & 0x1) != 0;
    let ms_stereo = (mode_ext & 0x2) != 0;

    if !intensity_stereo && !ms_stereo {
        return;
    }

    // Borrow splitting 绕过 borrow checker
    let row = &mut granule_data[gr];
    let (l_slice, r_slice) = row.split_at_mut(1);
    let l_data = &mut l_slice[0];
    let r_data = &mut r_slice[0];

    let l_gr = &granules[gr][0];
    let r_gr = &granules[gr][1];
    let sr_idx = samplerate_index(sample_rate);

    if l_gr.windows_switching_flag && l_gr.block_type == 2 {
        // 短块立体声处理
        process_stereo_short(
            l_data,
            r_data,
            l_gr,
            r_gr,
            ms_stereo,
            intensity_stereo,
            sr_idx,
        );
    } else {
        // 长块立体声处理
        process_stereo_long(
            l_data,
            r_data,
            l_gr,
            r_gr,
            ms_stereo,
            intensity_stereo,
            sr_idx,
        );
    }
}

/// 长块立体声处理
fn process_stereo_long(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    _l_gr: &Granule,
    _r_gr: &Granule,
    ms_stereo: bool,
    intensity_stereo: bool,
    sr_idx: usize,
) {
    let sfb_width = &SFB_WIDTH_LONG[sr_idx];

    // 查找 R channel 的非零边界 (IS bound)
    let mut r_nonzero = 576;
    while r_nonzero > 0 && r_data.xr[r_nonzero - 1] == 0.0 {
        r_nonzero -= 1;
    }

    // 计算 IS bound (对齐到 SFB 边界)
    let mut is_bound = 576;
    if intensity_stereo {
        let mut offset = 0;
        for &width in sfb_width.iter().take(22) {
            if offset >= r_nonzero {
                is_bound = offset;
                break;
            }
            offset += width;
        }
    }

    // 1. MS Stereo (低频部分, 到 IS bound)
    if ms_stereo {
        let ms_limit = if intensity_stereo { is_bound } else { 576 };
        for i in 0..ms_limit {
            let m = l_data.xr[i];
            let s = r_data.xr[i];
            l_data.xr[i] = (m + s) * FRAC_1_SQRT_2;
            r_data.xr[i] = (m - s) * FRAC_1_SQRT_2;
        }
    }

    // 2. Intensity Stereo (高频部分)
    if intensity_stereo {
        let mut offset = 0;
        for (sfb, &width) in sfb_width.iter().enumerate().take(22) {
            if offset >= is_bound {
                let is_pos = r_data.scalefac[sfb];
                if is_pos < 7 {
                    let (kl, kr) = IS_RATIOS[is_pos as usize];
                    for i in 0..width {
                        let idx = offset + i;
                        if idx < 576 {
                            let val = l_data.xr[idx];
                            l_data.xr[idx] = val * kl;
                            r_data.xr[idx] = val * kr;
                        }
                    }
                } else {
                    // is_pos == 7: intensity 无效.
                    // 若启用 MS, 对该频带执行 MS; 否则保持原值.
                    for i in 0..width {
                        let idx = offset + i;
                        if idx < 576 && ms_stereo {
                            let m = l_data.xr[idx];
                            let s = r_data.xr[idx];
                            l_data.xr[idx] = (m + s) * FRAC_1_SQRT_2;
                            r_data.xr[idx] = (m - s) * FRAC_1_SQRT_2;
                        }
                    }
                }
            }
            offset += width;
        }
    }
}

/// 短块立体声处理
///
/// 注意: stereo 在 reorder 之前执行, 数据仍为 SFB 顺序:
/// [SFB0_W0, SFB0_W1, SFB0_W2, SFB1_W0, SFB1_W1, SFB1_W2, ...]
/// 顺序索引: idx = sfb_base * 3 + win * width + s
fn process_stereo_short(
    l_data: &mut GranuleContext,
    r_data: &mut GranuleContext,
    _l_gr: &Granule,
    _r_gr: &Granule,
    ms_stereo: bool,
    intensity_stereo: bool,
    sr_idx: usize,
) {
    let sfb_width = &SFB_WIDTH_SHORT[sr_idx];

    // 对每个 window 分别处理
    for win in 0..3 {
        // 查找 R channel 在当前 window 的非零边界
        let mut r_nonzero = 0;
        let mut offset = 0;
        for &width in sfb_width.iter().take(13) {
            for s in 0..width {
                let idx = offset * 3 + win * width + s;
                if idx < 576 && r_data.xr[idx] != 0.0 {
                    r_nonzero = offset + width;
                }
            }
            offset += width;
        }

        // 按 SFB 处理
        offset = 0;
        for (sfb, &width) in sfb_width.iter().enumerate().take(13) {
            for s in 0..width {
                let idx = offset * 3 + win * width + s;
                if idx >= 576 {
                    continue;
                }

                if offset < r_nonzero || !intensity_stereo {
                    // MS Stereo 区域
                    if ms_stereo {
                        let m = l_data.xr[idx];
                        let ss = r_data.xr[idx];
                        l_data.xr[idx] = (m + ss) * FRAC_1_SQRT_2;
                        r_data.xr[idx] = (m - ss) * FRAC_1_SQRT_2;
                    }
                } else {
                    // IS 区域
                    let is_pos = r_data.scalefac[sfb * 3 + win];
                    if is_pos < 7 {
                        let (kl, kr) = IS_RATIOS[is_pos as usize];
                        let val = l_data.xr[idx];
                        l_data.xr[idx] = val * kl;
                        r_data.xr[idx] = val * kr;
                    } else if ms_stereo {
                        // is_pos == 7: intensity 无效, 回退到 MS.
                        let m = l_data.xr[idx];
                        let ss = r_data.xr[idx];
                        l_data.xr[idx] = (m + ss) * FRAC_1_SQRT_2;
                        r_data.xr[idx] = (m - ss) * FRAC_1_SQRT_2;
                    }
                }
            }
            offset += width;
        }
    }
}
