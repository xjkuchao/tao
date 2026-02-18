//! MP3 IMDCT (Inverse Modified Discrete Cosine Transform)
//!
//! 实现 18点 (Long Block) 和 6点 (Short Block) IMDCT, 窗口加权, 重叠相加.

use super::side_info::Granule;
use std::f64::consts::PI;
use std::sync::OnceLock;

/// IMDCT 窗口表 (36 点)
/// 包含 4 种窗口类型: 0=Normal, 1=Start, 2=Short(占位), 3=Stop
/// 使用 f64 精度计算角度后转为 f32 存储
static IMDCT_WINDOWS: OnceLock<[[f32; 36]; 4]> = OnceLock::new();

#[allow(clippy::needless_range_loop)]
fn get_imdct_windows() -> &'static [[f32; 36]; 4] {
    IMDCT_WINDOWS.get_or_init(|| {
        let mut windows = [[0.0f32; 36]; 4];

        for i in 0..36 {
            windows[0][i] = (PI / 36.0 * (i as f64 + 0.5)).sin() as f32;
        }

        for i in 0..18 {
            windows[1][i] = (PI / 36.0 * (i as f64 + 0.5)).sin() as f32;
        }
        for i in 18..24 {
            windows[1][i] = 1.0;
        }
        for i in 24..30 {
            windows[1][i] = (PI / 12.0 * ((i - 18) as f64 + 0.5)).sin() as f32;
        }
        for i in 30..36 {
            windows[1][i] = 0.0;
        }

        for i in 0..6 {
            windows[3][i] = 0.0;
        }
        for i in 6..12 {
            windows[3][i] = (PI / 12.0 * ((i - 6) as f64 + 0.5)).sin() as f32;
        }
        for i in 12..18 {
            windows[3][i] = 1.0;
        }
        for i in 18..36 {
            windows[3][i] = (PI / 36.0 * (i as f64 + 0.5)).sin() as f32;
        }

        windows
    })
}

/// Short Block Window (12 points)
/// 使用 f64 精度计算角度后转为 f32 存储
static SHORT_WINDOW: OnceLock<[f32; 12]> = OnceLock::new();
fn get_short_window() -> &'static [f32; 12] {
    SHORT_WINDOW.get_or_init(|| {
        let mut w = [0.0f32; 12];
        for (i, val) in w.iter_mut().enumerate() {
            *val = (PI / 12.0 * (i as f64 + 0.5)).sin() as f32;
        }
        w
    })
}

/// 18 点 IMDCT 余弦表 (预计算, f64 精度角度)
/// cos(π/(2*36) * (2i + 1 + 18) * (2k + 1)) = cos(π/72 * (2i + 19) * (2k + 1))
static IMDCT18_COS: OnceLock<[[f32; 18]; 36]> = OnceLock::new();
#[allow(clippy::needless_range_loop)]
fn get_imdct18_cos() -> &'static [[f32; 18]; 36] {
    IMDCT18_COS.get_or_init(|| {
        let mut table = [[0.0f32; 18]; 36];
        for (i, row) in table.iter_mut().enumerate() {
            for (k, val) in row.iter_mut().enumerate() {
                *val = (PI / 72.0 * (2.0 * i as f64 + 19.0) * (2.0 * k as f64 + 1.0)).cos() as f32;
            }
        }
        table
    })
}

/// 6 点 IMDCT 余弦表 (预计算, f64 精度角度)
/// cos(π/(2*12) * (2i + 1 + 6) * (2k + 1)) = cos(π/24 * (2i + 7) * (2k + 1))
#[cfg(test)]
static IMDCT6_COS: OnceLock<[[f32; 6]; 12]> = OnceLock::new();
#[cfg(test)]
fn get_imdct6_cos() -> &'static [[f32; 6]; 12] {
    IMDCT6_COS.get_or_init(|| {
        let mut table = [[0.0f32; 6]; 12];
        for (i, row) in table.iter_mut().enumerate() {
            for (k, val) in row.iter_mut().enumerate() {
                *val = (PI / 24.0 * (2.0 * i as f64 + 7.0) * (2.0 * k as f64 + 1.0)).cos() as f32;
            }
        }
        table
    })
}

/// 12 点 IMDCT 半余弦表 (仅 i=3..8 的 6x6 区域)
static IMDCT12_HALF_COS: OnceLock<[[f32; 6]; 6]> = OnceLock::new();
fn get_imdct12_half_cos() -> &'static [[f32; 6]; 6] {
    IMDCT12_HALF_COS.get_or_init(|| {
        let mut table = [[0.0f32; 6]; 6];
        for (i, row) in table.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                let n = (2 * (i + 3) + (12 / 2) + 1) * (2 * k + 1);
                *v = (PI / 24.0 * n as f64).cos() as f32;
            }
        }
        table
    })
}

/// 执行 IMDCT, Windowing, Overlap-Add
///
/// 输入: xr[576] (频域, 32 subbands * 18 samples)
/// 输出: output[576] (时域, 32 subbands * 18 samples)
/// 状态: overlap[32][18] (每个 channel 独立, 跨 granule/帧保持)
pub fn imdct(
    granule: &Granule,
    xr: &[f32; 576],
    overlap: &mut [[f32; 18]; 32],
    output: &mut [f32; 576],
) {
    let windows = get_imdct_windows();
    let short_win = get_short_window();
    let cos18 = get_imdct18_cos();
    let cos12 = get_imdct12_half_cos();
    let block_type = if granule.windows_switching_flag {
        granule.block_type
    } else {
        0
    };

    for sb in 0..32 {
        let sb_idx = sb * 18;
        let input_chunk = &xr[sb_idx..sb_idx + 18];

        let mut raw_out = [0.0f32; 36];

        // 判断当前子带是否为短块
        let is_short = granule.windows_switching_flag
            && granule.block_type == 2
            && (!granule.mixed_block_flag || sb >= 2);

        if is_short {
            // 短块: 与 symphonia 的 imdct12_win 一致的 3 窗口重叠放置.
            for w in 0..3 {
                for i in 0..3 {
                    let yl = (input_chunk[w] * cos12[i][0])
                        + (input_chunk[3 + w] * cos12[i][1])
                        + (input_chunk[6 + w] * cos12[i][2])
                        + (input_chunk[9 + w] * cos12[i][3])
                        + (input_chunk[12 + w] * cos12[i][4])
                        + (input_chunk[15 + w] * cos12[i][5]);

                    let yr = (input_chunk[w] * cos12[i + 3][0])
                        + (input_chunk[3 + w] * cos12[i + 3][1])
                        + (input_chunk[6 + w] * cos12[i + 3][2])
                        + (input_chunk[9 + w] * cos12[i + 3][3])
                        + (input_chunk[12 + w] * cos12[i + 3][4])
                        + (input_chunk[15 + w] * cos12[i + 3][5]);

                    raw_out[6 + 6 * w + 3 - i - 1] += -yl * short_win[3 - i - 1];
                    raw_out[6 + 6 * w + i + 3] += yl * short_win[i + 3];
                    raw_out[6 + 6 * w + i + 6] += yr * short_win[i + 6];
                    raw_out[6 + 6 * w + 12 - i - 1] += yr * short_win[12 - i - 1];
                }
            }
        } else {
            // 长块: 18 点 IMDCT
            imdct18_fast(input_chunk, &mut raw_out, cos18);

            // 窗口加权
            let win_idx = if granule.windows_switching_flag
                && granule.block_type == 2
                && granule.mixed_block_flag
                && sb < 2
            {
                0 // 混合块的长块部分使用 Normal 窗口
            } else {
                block_type as usize
            };

            let win = &windows[win_idx];
            for (sample, &win_val) in raw_out.iter_mut().zip(win.iter()) {
                *sample *= win_val;
            }
        }

        // Overlap-Add
        // 输出 18 个样本 = raw_out[0..18] + 上次的 overlap
        // 新的 overlap = raw_out[18..36]
        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }
}

/// 6 点 IMDCT (使用预计算余弦表, f64 累加)
/// 输入: 6 个频域样本
/// 输出: 12 个时域样本
#[cfg(test)]
fn imdct6_fast(input: &[f32], output: &mut [f32; 12], cos_table: &[[f32; 6]; 12]) {
    for (out, cos_row) in output.iter_mut().zip(cos_table.iter()) {
        let mut sum = 0.0f64;
        for (&inp, &cos_val) in input.iter().zip(cos_row.iter()) {
            sum += inp as f64 * cos_val as f64;
        }
        *out = sum as f32;
    }
}

/// 18 点 IMDCT (使用预计算余弦表, f64 累加)
/// 输入: 18 个频域样本
/// 输出: 36 个时域样本
fn imdct18_fast(input: &[f32], output: &mut [f32; 36], cos_table: &[[f32; 18]; 36]) {
    for (out, cos_row) in output.iter_mut().zip(cos_table.iter()) {
        let mut sum = 0.0f64;
        for (&inp, &cos_val) in input.iter().zip(cos_row.iter()) {
            sum += inp as f64 * cos_val as f64;
        }
        *out = sum as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI as PI64;

    /// f64 精度参考 18 点 IMDCT
    fn imdct18_reference(input: &[f32; 18]) -> [f64; 36] {
        let mut output = [0.0f64; 36];
        for i in 0..36 {
            let mut sum = 0.0f64;
            for k in 0..18 {
                let angle = PI64 / 72.0 * (2.0 * i as f64 + 19.0) * (2.0 * k as f64 + 1.0);
                sum += input[k] as f64 * angle.cos();
            }
            output[i] = sum;
        }
        output
    }

    /// f64 精度参考 6 点 IMDCT
    fn imdct6_reference(input: &[f32; 6]) -> [f64; 12] {
        let mut output = [0.0f64; 12];
        for i in 0..12 {
            let mut sum = 0.0f64;
            for k in 0..6 {
                let angle = PI64 / 24.0 * (2.0 * i as f64 + 7.0) * (2.0 * k as f64 + 1.0);
                sum += input[k] as f64 * angle.cos();
            }
            output[i] = sum;
        }
        output
    }

    #[test]
    fn test_imdct18_accuracy() {
        let input: [f32; 18] = [
            0.5, -0.3, 0.8, -0.1, 0.4, -0.6, 0.2, -0.9, 0.7, -0.4, 0.1, -0.5, 0.3, -0.8, 0.6, -0.2,
            0.9, -0.7,
        ];
        let cos_table = get_imdct18_cos();
        let mut actual = [0.0f32; 36];
        imdct18_fast(&input, &mut actual, cos_table);

        let expected = imdct18_reference(&input);

        let mut max_err = 0.0f64;
        for i in 0..36 {
            let err = (actual[i] as f64 - expected[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }

        eprintln!("=== 18 点 IMDCT 精度测试 ===");
        eprintln!("最大误差: {:.10}", max_err);

        assert!(max_err < 1e-4, "IMDCT18 误差过大: {:.6}", max_err);
    }

    #[test]
    fn test_imdct6_accuracy() {
        let input: [f32; 6] = [0.5, -0.3, 0.8, -0.1, 0.4, -0.6];
        let cos_table = get_imdct6_cos();
        let mut actual = [0.0f32; 12];
        imdct6_fast(&input, &mut actual, cos_table);

        let expected = imdct6_reference(&input);

        let mut max_err = 0.0f64;
        for i in 0..12 {
            let err = (actual[i] as f64 - expected[i]).abs();
            if err > max_err {
                max_err = err;
            }
        }

        eprintln!("=== 6 点 IMDCT 精度测试 ===");
        eprintln!("最大误差: {:.10}", max_err);

        assert!(max_err < 1e-4, "IMDCT6 误差过大: {:.6}", max_err);
    }

    #[test]
    fn test_imdct_full_long_block() {
        // 测试完整的 IMDCT (长块, Normal窗口) + overlap-add
        let mut granule = Granule::default();
        granule.windows_switching_flag = false;
        granule.block_type = 0;

        let mut xr = [0.0f32; 576];
        // 仅在子带 0 放入数据
        for k in 0..18 {
            xr[k] = (k as f32 + 1.0) * 0.1;
        }

        let mut overlap = [[0.0f32; 18]; 32];
        let mut output = [0.0f32; 576];

        // 第一次调用
        imdct(&granule, &xr, &mut overlap, &mut output);

        // 参考: 长块 IMDCT + Normal 窗口
        let input18: [f32; 18] = core::array::from_fn(|k| (k as f32 + 1.0) * 0.1);
        let ref_raw = imdct18_reference(&input18);

        // Normal 窗口
        let windows = get_imdct_windows();
        let win = &windows[0];

        let mut max_err = 0.0f64;
        for i in 0..18 {
            // output[i] = ref_raw[i] * win[i] + overlap_prev[i] (overlap_prev=0)
            let expected = ref_raw[i] * win[i] as f64;
            let err = (output[i] as f64 - expected).abs();
            if err > max_err {
                max_err = err;
            }
        }

        eprintln!("=== IMDCT 完整长块测试 ===");
        eprintln!("最大误差 (output): {:.10}", max_err);

        // 验证 overlap
        let mut max_ovl_err = 0.0f64;
        for i in 0..18 {
            let expected_ovl = ref_raw[18 + i] * win[18 + i] as f64;
            let err = (overlap[0][i] as f64 - expected_ovl).abs();
            if err > max_ovl_err {
                max_ovl_err = err;
            }
        }
        eprintln!("最大误差 (overlap): {:.10}", max_ovl_err);

        assert!(max_err < 1e-4, "IMDCT 长块输出误差过大: {:.6}", max_err);
        assert!(
            max_ovl_err < 1e-4,
            "IMDCT 长块 overlap 误差过大: {:.6}",
            max_ovl_err
        );
    }

    #[test]
    fn test_imdct_short_block() {
        // 测试完整的 IMDCT (纯短块) + overlap-add
        let mut granule = Granule::default();
        granule.windows_switching_flag = true;
        granule.block_type = 2;
        granule.mixed_block_flag = false;

        let mut xr = [0.0f32; 576];
        // 子带 0: 3 个窗口各 6 个样本
        for k in 0..18 {
            xr[k] = (k as f32 + 1.0) * 0.05;
        }

        let mut overlap = [[0.0f32; 18]; 32];
        let mut output = [0.0f32; 576];

        imdct(&granule, &xr, &mut overlap, &mut output);

        // 参考计算
        let short_win = get_short_window();
        let win0: [f32; 6] = core::array::from_fn(|i| xr[i]);
        let win1: [f32; 6] = core::array::from_fn(|i| xr[6 + i]);
        let win2: [f32; 6] = core::array::from_fn(|i| xr[12 + i]);

        let y0 = imdct6_reference(&win0);
        let y1 = imdct6_reference(&win1);
        let y2 = imdct6_reference(&win2);

        // 加窗
        let y0w: Vec<f64> = y0
            .iter()
            .enumerate()
            .map(|(i, &v)| v * short_win[i] as f64)
            .collect();
        let y1w: Vec<f64> = y1
            .iter()
            .enumerate()
            .map(|(i, &v)| v * short_win[i] as f64)
            .collect();
        let y2w: Vec<f64> = y2
            .iter()
            .enumerate()
            .map(|(i, &v)| v * short_win[i] as f64)
            .collect();

        // 重叠放置
        let mut ref_raw = [0.0f64; 36];
        for i in 0..6 {
            ref_raw[6 + i] += y0w[i];
            ref_raw[12 + i] += y0w[6 + i] + y1w[i];
            ref_raw[18 + i] += y1w[6 + i] + y2w[i];
            ref_raw[24 + i] += y2w[6 + i];
        }

        let mut max_err = 0.0f64;
        for i in 0..18 {
            let expected = ref_raw[i]; // overlap=0
            let err = (output[i] as f64 - expected).abs();
            if err > max_err {
                max_err = err;
            }
        }

        eprintln!("=== IMDCT 短块测试 ===");
        eprintln!("最大误差 (output): {:.10}", max_err);
        for i in 0..18 {
            eprintln!(
                "  out[{:2}] tao={:12.8}  ref={:12.8}  diff={:+.8}",
                i,
                output[i],
                ref_raw[i],
                output[i] as f64 - ref_raw[i]
            );
        }

        assert!(max_err < 1e-4, "IMDCT 短块输出误差过大: {:.6}", max_err);
    }
}
