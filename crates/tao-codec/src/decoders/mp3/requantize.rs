//! MP3 反量化 (Requantization)
//!
//! 将 Huffman 解码的整数 (is) 转换为浮点频谱值 (xr)
//! xr = sign(is) * |is|^(4/3) * 2^(exp/4)

use super::data::GranuleContext;
use super::header::MpegVersion;
use super::side_info::Granule;
use super::tables::{PRETAB, SFB_WIDTH_LONG_44, SFB_WIDTH_SHORT_44};
use std::sync::OnceLock;
use tao_core::TaoResult;

/// x^(4/3) 查找表大小
const POW43_TABLE_SIZE: usize = 8192;
static POW43_TABLE: OnceLock<Vec<f32>> = OnceLock::new();

/// 获取 x^(4/3) 表
fn get_pow43_table() -> &'static Vec<f32> {
    POW43_TABLE.get_or_init(|| {
        let mut table = Vec::with_capacity(POW43_TABLE_SIZE);
        for i in 0..POW43_TABLE_SIZE {
            table.push((i as f32).powf(4.0 / 3.0));
        }
        table
    })
}

/// 预加重表 (Pretab) for Long Blocks
/// ISO 11172-3 Table B.6
// const PRETAB: [u8; 22] = ... (moved to tables.rs)

/// 反量化处理
pub fn requantize(
    granule: &Granule,
    ctx: &mut GranuleContext,
    version: MpegVersion,
    sample_rate: u32, // 用于推断 subband 边界? 实际上 subband 边界是固定的 (21 bands)
) -> TaoResult<()> {
    let pow43 = get_pow43_table();
    let scalefac_scale = if granule.scalefac_scale { 1.0 } else { 0.5 };
    let global_gain = granule.global_gain as i32;

    // 预计算 2^(exp/4) 的一部分: 2^((global_gain - 210) / 4)
    // 实际上我们需要对每个 scalefactor band 计算一个 multiplier

    if version == MpegVersion::Mpeg1 {
        requantize_mpeg1(
            granule,
            ctx,
            pow43,
            global_gain,
            scalefac_scale,
            sample_rate,
        );
    } else {
        // MPEG-2 (TODO)
    }

    Ok(())
}

fn requantize_mpeg1(
    granule: &Granule,
    ctx: &mut GranuleContext,
    pow43: &[f32],
    global_gain: i32,
    scalefac_scale: f32,
    _sample_rate: u32,
) {
    let is = &ctx.is;
    let xr = &mut ctx.xr;
    let scalefac = &ctx.scalefac;

    // Band boundaries for Long blocks (MPEG-1 44.1kHz)
    // TODO: Need tables for 32/48kHz
    // 暂时假设 44.1kHz
    // Table B.8
    // 使用 tables.rs 中的定义
    let sfb_width_long_44 = &SFB_WIDTH_LONG_44;
    // Short blocks boundaries
    let sfb_width_short_44 = &SFB_WIDTH_SHORT_44;

    if granule.windows_switching_flag && granule.block_type == 2 {
        if granule.mixed_block_flag {
            // Mixed blocks
            // TODO
        } else {
            // Short blocks
            let mut idx = 0;
            // 12 bands * 3 windows
            // Band 0: W0, W1, W2
            // Band 1: W0, W1, W2
            // ...
            // Wait, IS data is ordered by band then window?
            // "The short block data is ordered such that... all sample values for a given window... are contiguous?"
            // NO. "The frequency lines are ordered by subband... within a subband, the samples for the three windows are ordered by window."
            // BUT: Huffman decoding produces them in the order of scalefactor bands.
            // For short blocks, scalefactors are transmitted as Band 0 (W0, W1, W2)...
            // And Huffman data follows that structure?
            // ISO 11172-3 2.4.3.4.10: "The frequency lines are processed in groups of 12... 3 short windows."
            // So:
            // for sfb = 0 to 11
            //   for window = 0 to 2
            //     process sfb_width[sfb] samples

            // Note: sfb_width table above is for 44.1kHz.
            // Need correct table selector based on sample rate.
            let widths = &sfb_width_short_44[0..12]; // Use 44.1kHz for now

            for sfb in 0..12 {
                let width = widths[sfb];

                for window in 0..3 {
                    let subblock_gain = granule.subblock_gain[window] as i32;
                    let sf = scalefac[sfb * 3 + window] as i32;

                    // exp = global_gain - 210 - 8*subblock_gain - 2*(scalefac) * multiplier?
                    // Formula: exp = global_gain - 210 - 8*subblock_gain - scalefac * (scale ? 1 : 0.5) * 4 ?
                    // Let's use standard form: 2^(exp/4)
                    // exp_val = global_gain - 210 - 8*subblock_gain
                    // sf_shift = scalefac * scalefac_scale

                    let exp_val = (global_gain - 210 - 8 * subblock_gain) as f32;
                    let val_shift = -(sf as f32) * scalefac_scale;

                    let _factor = 2.0f32.powf(0.25 * (exp_val + 4.0 * val_shift)); // why 4.0?
                    // Spec: 2^(1/4 * (global_gain - 210 - 8*subblock_gain)) * 2^(-scalefac_multiplier * scalefac)
                    // = 2^(1/4 * (...)) * 2^(-s_m * sf * 4/4)
                    // = 2^(1/4 * (global_gain - 210 - 8*subblock_gain - 4*s_m*sf))

                    let common_exp = global_gain as f32
                        - 210.0
                        - 8.0 * subblock_gain as f32
                        - 4.0 * scalefac_scale * (sf as f32);
                    let multiplier = 2.0f32.powf(0.25 * common_exp);

                    for _ in 0..width {
                        if idx >= 576 {
                            break;
                        }
                        let val_is = is[idx];
                        idx += 1;

                        if val_is != 0 {
                            let abs_is = val_is.abs();
                            let pow_val = if (abs_is as usize) < POW43_TABLE_SIZE {
                                pow43[abs_is as usize]
                            } else {
                                (abs_is as f32).powf(4.0 / 3.0)
                            };

                            let val_xr = pow_val * multiplier;
                            xr[idx - 1] = if val_is > 0 { val_xr } else { -val_xr };
                        } else {
                            xr[idx - 1] = 0.0;
                        }
                    }
                }
            }
        }
    } else {
        // Long blocks
        let widths = &sfb_width_long_44; // Use 44.1kHz
        let mut idx = 0;

        for sfb in 0..21 {
            let width = widths[sfb];
            let sf = scalefac[sfb] as i32;
            let preflag = if granule.preflag { 1 } else { 0 };
            let pretab_val = PRETAB[sfb] as i32;

            // exp = global_gain - 210 - (scalefac + preflag*pretab) * scalefac_scale
            // factor = 2^(exp/4)
            // common_exp = global_gain - 210 - 4 * scalefac_scale * (sf + preflag*pretab)

            let term2 = (sf + preflag * pretab_val) as f32 * scalefac_scale;
            let common_exp = global_gain as f32 - 210.0 - 4.0 * term2;
            let multiplier = 2.0f32.powf(0.25 * common_exp);

            for _ in 0..width {
                if idx >= 576 {
                    break;
                }
                let val_is = is[idx];
                idx += 1;

                if val_is != 0 {
                    let abs_is = val_is.abs();
                    let pow_val = if (abs_is as usize) < POW43_TABLE_SIZE {
                        pow43[abs_is as usize]
                    } else {
                        (abs_is as f32).powf(4.0 / 3.0)
                    };

                    let val_xr = pow_val * multiplier;
                    xr[idx - 1] = if val_is > 0 { val_xr } else { -val_xr };
                } else {
                    xr[idx - 1] = 0.0;
                }
            }
        }

        // Zero out remaining
        while idx < 576 {
            xr[idx] = 0.0;
            idx += 1;
        }
    }
}
