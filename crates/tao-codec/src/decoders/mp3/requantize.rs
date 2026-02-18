//! MP3 反量化 (Requantization)
//!
//! 将 Huffman 解码的整数 (is) 转换为浮点频谱值 (xr)
//! xr = sign(is) * |is|^(4/3) * 2^(exp/4)

use super::data::GranuleContext;
use super::header::MpegVersion;
use super::side_info::Granule;
use super::tables::{PRETAB, SFB_WIDTH_LONG, SFB_WIDTH_SHORT, samplerate_index};
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

/// |is|^(4/3) 计算
#[inline]
fn pow43(val: i32, table: &[f32]) -> f32 {
    let abs_val = val.unsigned_abs() as usize;
    if abs_val < POW43_TABLE_SIZE {
        table[abs_val]
    } else {
        (abs_val as f32).powf(4.0 / 3.0)
    }
}

/// 反量化处理
pub fn requantize(
    granule: &Granule,
    ctx: &mut GranuleContext,
    version: MpegVersion,
    sample_rate: u32,
) -> TaoResult<()> {
    let pow43_table = get_pow43_table();
    let scalefac_scale: f64 = if granule.scalefac_scale { 1.0 } else { 0.5 };
    let global_gain = granule.global_gain as f64;

    if version == MpegVersion::Mpeg1 {
        requantize_mpeg1(
            granule,
            ctx,
            pow43_table,
            global_gain,
            scalefac_scale,
            sample_rate,
        );
    }
    // TODO: MPEG-2/2.5 反量化

    Ok(())
}

fn requantize_mpeg1(
    granule: &Granule,
    ctx: &mut GranuleContext,
    pow43_table: &[f32],
    global_gain: f64,
    scalefac_scale: f64,
    sample_rate: u32,
) {
    let is = &ctx.is;
    let xr = &mut ctx.xr;
    let scalefac = &ctx.scalefac;

    let sr_idx = samplerate_index(sample_rate);
    let sfb_width_long = &SFB_WIDTH_LONG[sr_idx];
    let sfb_width_short = &SFB_WIDTH_SHORT[sr_idx];

    if granule.windows_switching_flag && granule.block_type == 2 {
        if granule.mixed_block_flag {
            // Mixed blocks: 前 8 个长块 SFB + 短块 SFB 3-11
            requantize_mixed(
                is,
                xr,
                scalefac,
                granule,
                pow43_table,
                global_gain,
                scalefac_scale,
                sfb_width_long,
                sfb_width_short,
            );
        } else {
            // 纯短块
            requantize_short(
                is,
                xr,
                scalefac,
                granule,
                pow43_table,
                global_gain,
                scalefac_scale,
                sfb_width_short,
            );
        }
    } else {
        // 长块
        requantize_long(
            is,
            xr,
            scalefac,
            granule,
            pow43_table,
            global_gain,
            scalefac_scale,
            sfb_width_long,
        );
    }
}

/// 长块反量化
#[allow(clippy::too_many_arguments)]
fn requantize_long(
    is: &[i32; 576],
    xr: &mut [f32; 576],
    scalefac: &[u8; 40],
    granule: &Granule,
    pow43_table: &[f32],
    global_gain: f64,
    scalefac_scale: f64,
    sfb_width: &[usize; 22],
) {
    let preflag = if granule.preflag { 1.0f64 } else { 0.0 };
    let mut idx = 0;

    for sfb in 0..22 {
        let width = sfb_width[sfb];
        let sf = scalefac[sfb] as f64;
        let pretab_val = if sfb < PRETAB.len() {
            PRETAB[sfb] as f64
        } else {
            0.0
        };

        let term = (sf + preflag * pretab_val) * scalefac_scale;
        let exp = global_gain - 210.0 - 4.0 * term;
        let multiplier = 2.0f64.powf(0.25 * exp) as f32;

        for _ in 0..width {
            if idx >= 576 {
                break;
            }
            let val = is[idx];
            if val != 0 {
                let p = pow43(val, pow43_table);
                xr[idx] = if val > 0 {
                    p * multiplier
                } else {
                    -p * multiplier
                };
            } else {
                xr[idx] = 0.0;
            }
            idx += 1;
        }
    }
}

/// 短块反量化
#[allow(clippy::too_many_arguments)]
fn requantize_short(
    is: &[i32; 576],
    xr: &mut [f32; 576],
    scalefac: &[u8; 40],
    granule: &Granule,
    pow43_table: &[f32],
    global_gain: f64,
    scalefac_scale: f64,
    sfb_width: &[usize; 13],
) {
    let mut idx = 0;

    for sfb in 0..13 {
        let width = sfb_width[sfb];
        for window in 0..3 {
            let subblock_gain = granule.subblock_gain[window] as f64;
            let sf = if sfb < 12 {
                scalefac[sfb * 3 + window] as f64
            } else {
                0.0
            };

            let exp = global_gain - 210.0 - 8.0 * subblock_gain - 4.0 * scalefac_scale * sf;
            let multiplier = 2.0f64.powf(0.25 * exp) as f32;

            for _ in 0..width {
                if idx >= 576 {
                    break;
                }
                let val = is[idx];
                if val != 0 {
                    let p = pow43(val, pow43_table);
                    xr[idx] = if val > 0 {
                        p * multiplier
                    } else {
                        -p * multiplier
                    };
                } else {
                    xr[idx] = 0.0;
                }
                idx += 1;
            }
        }
    }

    for sample in xr.iter_mut().skip(idx) {
        *sample = 0.0;
    }
}

/// 混合块反量化
/// 前 8 个长块 SFB (大约前 36 个样本) 使用长块公式,
/// 之后的短块 SFB 使用短块公式.
#[allow(clippy::too_many_arguments)]
fn requantize_mixed(
    is: &[i32; 576],
    xr: &mut [f32; 576],
    scalefac: &[u8; 40],
    granule: &Granule,
    pow43_table: &[f32],
    global_gain: f64,
    scalefac_scale: f64,
    sfb_width_long: &[usize; 22],
    sfb_width_short: &[usize; 13],
) {
    let preflag = if granule.preflag { 1.0f64 } else { 0.0 };
    let mut idx = 0;

    for sfb in 0..8 {
        let width = sfb_width_long[sfb];
        let sf = scalefac[sfb] as f64;
        let pretab_val = PRETAB[sfb] as f64;

        let term = (sf + preflag * pretab_val) * scalefac_scale;
        let exp = global_gain - 210.0 - 4.0 * term;
        let multiplier = 2.0f64.powf(0.25 * exp) as f32;

        for _ in 0..width {
            if idx >= 576 {
                break;
            }
            let val = is[idx];
            if val != 0 {
                let p = pow43(val, pow43_table);
                xr[idx] = if val > 0 {
                    p * multiplier
                } else {
                    -p * multiplier
                };
            } else {
                xr[idx] = 0.0;
            }
            idx += 1;
        }
    }

    for sfb in 3..13 {
        let width = sfb_width_short[sfb];
        for window in 0..3 {
            let subblock_gain = granule.subblock_gain[window] as f64;
            let sf = if sfb < 12 {
                scalefac[8 + (sfb - 3) * 3 + window] as f64
            } else {
                0.0
            };

            let exp = global_gain - 210.0 - 8.0 * subblock_gain - 4.0 * scalefac_scale * sf;
            let multiplier = 2.0f64.powf(0.25 * exp) as f32;

            for _ in 0..width {
                if idx >= 576 {
                    break;
                }
                let val = is[idx];
                if val != 0 {
                    let p = pow43(val, pow43_table);
                    xr[idx] = if val > 0 {
                        p * multiplier
                    } else {
                        -p * multiplier
                    };
                } else {
                    xr[idx] = 0.0;
                }
                idx += 1;
            }
        }
    }

    for sample in xr.iter_mut().skip(idx) {
        *sample = 0.0;
    }
}
