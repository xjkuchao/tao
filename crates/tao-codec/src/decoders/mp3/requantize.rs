//! MP3 反量化 (Requantization)
//!
//! 将 Huffman 解码的整数 (is) 转换为浮点频谱值 (xr)
//! xr = sign(is) * |is|^(4/3) * 2^(exp/4)

use super::data::GranuleContext;
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

/// 反量化处理 (MPEG-1 和 MPEG-2/2.5 使用相同的反量化公式, ISO 13818-3 §2.4.3.4)
pub fn requantize(granule: &Granule, ctx: &mut GranuleContext, sample_rate: u32) -> TaoResult<()> {
    let pow43_table = get_pow43_table();
    let mut scalefac_scale: f64 = if granule.scalefac_scale { 1.0 } else { 0.5 };
    if let Ok(force) = std::env::var("TAO_MP3_FORCE_SCALEFAC_SCALE") {
        scalefac_scale = if force.trim() == "1" { 1.0 } else { 0.5 };
    }
    let global_gain = granule.global_gain as f64;

    requantize_mpeg1(
        granule,
        ctx,
        pow43_table,
        global_gain,
        scalefac_scale,
        sample_rate,
    );

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
    let preflag = if std::env::var("TAO_MP3_DISABLE_PRETAB").is_ok() {
        false
    } else {
        granule.preflag
    };

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
                preflag,
                sfb_width_long,
                sfb_width_short,
                sr_idx,
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
            preflag,
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
    _granule: &Granule,
    pow43_table: &[f32],
    global_gain: f64,
    scalefac_scale: f64,
    preflag: bool,
    sfb_width: &[usize; 22],
) {
    let preflag = if preflag { 1.0f64 } else { 0.0 };
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
    preflag: bool,
    sfb_width_long: &[usize; 22],
    sfb_width_short: &[usize; 13],
    sr_idx: usize,
) {
    let preflag = if preflag { 1.0f64 } else { 0.0 };
    let mut idx = 0;

    // MPEG-1 mixed block 前 8 个 long SFB, MPEG-2/2.5 前 6 个 long SFB.
    let long_sfb_count = if sr_idx >= 5 { 8 } else { 6 };
    for sfb in 0..long_sfb_count {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn next_u32(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed
    }

    fn gen_i32(seed: &mut u32, min: i32, max: i32) -> i32 {
        let r = next_u32(seed) as i32 & 0x7fff_ffff;
        min + (r % (max - min + 1))
    }

    fn gen_u8(seed: &mut u32, max: u8) -> u8 {
        (next_u32(seed) % (max as u32 + 1)) as u8
    }

    fn build_short_bounds(widths: &[usize; 13]) -> [usize; 40] {
        let mut out = [0usize; 40];
        let mut idx = 0usize;
        let mut acc = 0usize;
        out[idx] = acc;
        idx += 1;
        for &w in widths {
            for _ in 0..3 {
                acc += w;
                out[idx] = acc;
                idx += 1;
            }
        }
        out
    }

    fn build_mixed_bounds(long_widths: &[usize; 22], short_widths: &[usize; 13]) -> Vec<usize> {
        // MPEG-1 44.1/48/32kHz 的 mixed 切分点都是前 8 个长带.
        let mut out = Vec::with_capacity(39);
        let mut acc = 0usize;
        out.push(acc);

        for &w in long_widths.iter().take(8) {
            acc += w;
            out.push(acc);
        }

        for &w in short_widths.iter().skip(3) {
            for _ in 0..3 {
                acc += w;
                out.push(acc);
            }
        }

        out
    }

    fn reference_requantize_mpeg1(
        granule: &Granule,
        ctx: &GranuleContext,
        sample_rate: u32,
    ) -> [f32; 576] {
        let sr_idx = samplerate_index(sample_rate);
        let long_widths = &SFB_WIDTH_LONG[sr_idx];
        let short_widths = &SFB_WIDTH_SHORT[sr_idx];
        let long_bounds = super::super::tables::build_sfb_long_bounds(sample_rate);
        let short_bounds = build_short_bounds(short_widths);
        let mixed_bounds = build_mixed_bounds(long_widths, short_widths);

        let mut out = [0.0f32; 576];
        let gain = granule.global_gain as i32 - 210;
        let sf_shift = if granule.scalefac_scale { 2 } else { 1 };
        let pre = if granule.preflag { PRETAB } else { [0u8; 22] };

        let mut base = [0.0f32; 576];
        let pow43_table = get_pow43_table();
        for (i, b) in base.iter_mut().enumerate() {
            let v = ctx.is[i];
            if v != 0 {
                let p = if v > 0 {
                    pow43(v, pow43_table)
                } else {
                    -pow43(v, pow43_table)
                };
                *b = p;
            }
        }

        let is_short = granule.windows_switching_flag && granule.block_type == 2;
        if !is_short {
            for sfb in 0..22 {
                let start = long_bounds[sfb];
                let end = long_bounds[sfb + 1];
                let b = ((ctx.scalefac[sfb] + pre[sfb]) as i32) << sf_shift;
                let mul = 2.0f64.powf(0.25 * (gain - b) as f64) as f32;
                for i in start..end {
                    out[i] = base[i] * mul;
                }
            }
            return out;
        }

        if granule.mixed_block_flag {
            // long part: bands 0..8
            for sfb in 0..8 {
                let start = mixed_bounds[sfb];
                let end = mixed_bounds[sfb + 1];
                let b = ((ctx.scalefac[sfb] + pre[sfb]) as i32) << sf_shift;
                let mul = 2.0f64.powf(0.25 * (gain - b) as f64) as f32;
                for i in start..end {
                    out[i] = base[i] * mul;
                }
            }

            // short part: mixed_bounds[8..]
            let a = [
                gain - 8 * granule.subblock_gain[0] as i32,
                gain - 8 * granule.subblock_gain[1] as i32,
                gain - 8 * granule.subblock_gain[2] as i32,
            ];
            for seg in 8..(mixed_bounds.len() - 1) {
                let start = mixed_bounds[seg];
                let end = mixed_bounds[seg + 1];

                // mixed 的短块段从 seg=8 开始, 需要先减去 long 段数量后再映射 window.
                let short_seg = seg - 8;
                let window = short_seg % 3;
                let short_sfb = 3 + short_seg / 3;
                let b = if short_sfb < 12 {
                    (ctx.scalefac[8 + (short_sfb - 3) * 3 + window] as i32) << sf_shift
                } else {
                    0
                };
                let mul = 2.0f64.powf(0.25 * (a[window] - b) as f64) as f32;
                for i in start..end {
                    out[i] = base[i] * mul;
                }
            }
            return out;
        }

        let a = [
            gain - 8 * granule.subblock_gain[0] as i32,
            gain - 8 * granule.subblock_gain[1] as i32,
            gain - 8 * granule.subblock_gain[2] as i32,
        ];
        for sfb in 0..39 {
            let start = short_bounds[sfb];
            let end = short_bounds[sfb + 1];
            let b = (ctx.scalefac[sfb] as i32) << sf_shift;
            let mul = 2.0f64.powf(0.25 * (a[sfb % 3] - b) as f64) as f32;
            for i in start..end {
                out[i] = base[i] * mul;
            }
        }

        out
    }

    #[test]
    fn test_requantize_mixed_short_index_mapping() {
        // mixed block 的短块部分: sfb=3..11, 每个 sfb 有 3 个 window, 共 27 段.
        // scalefac 索引应从 8 开始连续递增: 8..34.
        let mut wrong_window_count = 0usize;

        for seg in 0..27usize {
            let sfb = 3 + seg / 3;
            let window = seg % 3;

            let expected_sf_index = 8 + seg;
            let impl_sf_index = 8 + (sfb - 3) * 3 + window;

            assert_eq!(
                impl_sf_index, expected_sf_index,
                "mixed short scalefac 索引错误: seg={}, sfb={}, window={}",
                seg, sfb, window
            );

            // 错误写法示例: 直接对全局索引取模(例如 idx%3 或 sfb_global%3),
            // 会导致 window 映射整体错位.
            let wrong_window = expected_sf_index % 3;
            if wrong_window != window {
                wrong_window_count += 1;
            }
        }

        assert_eq!(wrong_window_count, 27, "专项测试未覆盖到 window 错位风险");
    }

    #[test]
    fn test_requantize_matches_reference_long_short_mixed() {
        let mut seed = 0x1234_5678u32;
        for case in 0..120 {
            let mut granule = Granule::default();
            granule.scalefac_scale = (next_u32(&mut seed) & 1) != 0;
            granule.preflag = (next_u32(&mut seed) & 1) != 0;
            granule.global_gain = gen_u8(&mut seed, 255) as u32;

            match case % 3 {
                0 => {
                    granule.windows_switching_flag = false;
                    granule.block_type = 0;
                    granule.mixed_block_flag = false;
                }
                1 => {
                    granule.windows_switching_flag = true;
                    granule.block_type = 2;
                    granule.mixed_block_flag = false;
                    granule.subblock_gain = [
                        gen_u8(&mut seed, 7),
                        gen_u8(&mut seed, 7),
                        gen_u8(&mut seed, 7),
                    ];
                }
                _ => {
                    granule.windows_switching_flag = true;
                    granule.block_type = 2;
                    granule.mixed_block_flag = true;
                    granule.subblock_gain = [
                        gen_u8(&mut seed, 7),
                        gen_u8(&mut seed, 7),
                        gen_u8(&mut seed, 7),
                    ];
                }
            }

            let mut ctx = GranuleContext::default();
            for sf in &mut ctx.scalefac {
                *sf = gen_u8(&mut seed, 15);
            }
            if granule.block_type == 2 {
                if granule.mixed_block_flag {
                    for sf in ctx.scalefac.iter_mut().skip(35) {
                        *sf = 0;
                    }
                } else {
                    for sf in ctx.scalefac.iter_mut().skip(36) {
                        *sf = 0;
                    }
                }
            }

            for v in &mut ctx.is {
                let r = next_u32(&mut seed) % 100;
                *v = if r < 45 {
                    0
                } else {
                    gen_i32(&mut seed, -200, 200)
                };
            }

            let mut actual = ctx.clone();
            requantize(&granule, &mut actual, 44100).unwrap();
            let expected = reference_requantize_mpeg1(&granule, &ctx, 44100);

            let mut max_err = 0.0f32;
            let mut max_idx = 0usize;
            for (i, (a, e)) in actual.xr.iter().zip(expected.iter()).enumerate() {
                let err = (a - e).abs();
                if err > max_err {
                    max_err = err;
                    max_idx = i;
                }
            }

            assert!(
                max_err < 1e-5,
                "case={} 反量化与参考不一致: max_err={:.6e} @{}, block_type={}, mixed={}, gg={}, sf_scale={}, sbg={:?}",
                case,
                max_err,
                max_idx,
                granule.block_type,
                granule.mixed_block_flag,
                granule.global_gain,
                granule.scalefac_scale,
                granule.subblock_gain
            );
        }
    }
}
