//! MP3 解码器诊断工具模块
//!
//! 提供各阶段中间数据的导出、快照和对比功能,
//! 仅在测试编译时启用 (cfg(test))
#![allow(clippy::needless_range_loop)]

use super::data::{IsSpectrum, XrSpectrum};
use super::side_info::Granule;
use std::sync::{Mutex, OnceLock};

/// 解码管线单帧快照
/// 记录各阶段的中间数据, 用于逐模块精度对比
#[derive(Clone)]
pub struct FrameSnapshot {
    /// 帧索引
    pub frame_index: usize,
    /// Granule 索引 (0 或 1)
    pub gr: usize,
    /// Channel 索引 (0 或 1)
    pub ch: usize,
    /// Huffman 解码后的整数样本 (is[576])
    pub is_samples: IsSpectrum,
    /// 比例因子 (scalefac)
    pub scalefac: [u8; 40],
    /// 全局增益
    pub global_gain: u32,
    /// scalefac_compress
    pub scalefac_compress: u32,
    /// scalefac_scale
    pub scalefac_scale: bool,
    /// preflag
    pub preflag: bool,
    /// subblock_gain
    pub subblock_gain: [u8; 3],
    /// table_select
    pub table_select: [u8; 3],
    /// part2_3_length
    pub part2_3_length: u32,
    /// part2 起始位偏移 (相对于 main_data)
    pub part2_3_begin: usize,
    /// part2 实际读取位数
    pub part2_bits: u32,
    /// 声道模式
    pub channel_mode: u8,
    /// Joint Stereo 扩展位
    pub mode_extension: u8,
    /// block_type
    pub block_type: u8,
    /// mixed_block_flag
    pub mixed_block_flag: bool,
    /// windows_switching_flag
    pub windows_switching_flag: bool,
    /// region1_start (样本索引)
    pub region1_start: usize,
    /// region2_start (样本索引)
    pub region2_start: usize,
    /// big_values (样本数)
    pub big_values: usize,
    /// count1_table
    pub count1_table: u8,
    /// 反量化后的频谱 (xr[576])
    pub xr_after_requantize: XrSpectrum,
    /// 立体声处理后的频谱
    pub xr_after_stereo: XrSpectrum,
    /// 重排序后的频谱
    pub xr_after_reorder: XrSpectrum,
    /// 抗混叠后的频谱
    pub xr_after_alias: XrSpectrum,
    /// IMDCT 时域输出
    pub imdct_output: [f32; 576],
    /// 参考 IMDCT 输出
    pub ref_imdct_output: [f32; 576],
    /// IMDCT 前 overlap 缓冲
    pub overlap_before: [[f32; 18]; 32],
    /// 合成滤波器上下文 v_front
    pub synth_v_front: usize,
    /// 合成滤波器上下文 v_vec
    pub synth_v_vec: [[f32; 64]; 16],
    /// 频率反转后
    pub after_freq_inversion: [f32; 576],
    /// 合成滤波器输出 (PCM)
    pub pcm_output: Vec<f32>,
    /// 参考合成滤波器输出 (PCM)
    pub ref_pcm_output: Vec<f32>,
    /// rzero 值
    pub rzero: usize,
    /// main_data 原始字节 (用于参考 Huffman 解码)
    pub main_data: Vec<u8>,
}

impl Default for FrameSnapshot {
    fn default() -> Self {
        Self {
            frame_index: 0,
            gr: 0,
            ch: 0,
            is_samples: [0; 576],
            scalefac: [0; 40],
            global_gain: 0,
            scalefac_compress: 0,
            scalefac_scale: false,
            preflag: false,
            subblock_gain: [0; 3],
            table_select: [0; 3],
            part2_3_length: 0,
            part2_3_begin: 0,
            part2_bits: 0,
            channel_mode: 0,
            mode_extension: 0,
            block_type: 0,
            mixed_block_flag: false,
            windows_switching_flag: false,
            region1_start: 0,
            region2_start: 0,
            big_values: 0,
            count1_table: 0,
            xr_after_requantize: [0.0; 576],
            xr_after_stereo: [0.0; 576],
            xr_after_reorder: [0.0; 576],
            xr_after_alias: [0.0; 576],
            imdct_output: [0.0; 576],
            ref_imdct_output: [0.0; 576],
            overlap_before: [[0.0; 18]; 32],
            synth_v_front: 0,
            synth_v_vec: [[0.0; 64]; 16],
            after_freq_inversion: [0.0; 576],
            pcm_output: Vec::new(),
            ref_pcm_output: Vec::new(),
            rzero: 0,
            main_data: Vec::new(),
        }
    }
}

/// 每帧侧信息摘要 (用于定位短块/混合块相关误差)
#[derive(Debug, Clone, Copy, Default)]
pub struct GranuleInfo {
    /// block_type (0=长块, 1=Start, 2=短块, 3=Stop)
    pub block_type: u8,
    /// 是否为 mixed block
    pub mixed_block_flag: bool,
    /// 是否启用 windows_switching_flag
    pub windows_switching_flag: bool,
    /// part2_3_length
    pub part2_3_length: u32,
    /// big_values
    pub big_values: u32,
    /// count1table_select
    pub count1table_select: bool,
}

/// 每帧统计信息
#[derive(Debug, Clone)]
pub struct FrameInfo {
    /// 帧索引 (解码器内部计数)
    pub frame_index: u32,
    /// main_data_begin (字节)
    pub main_data_begin: u32,
    /// 复用不足的字节数 (underflow)
    pub underflow_bytes: u32,
    /// 声道数
    pub channels: u32,
    /// granule 数
    pub granules: u32,
    /// [granule][channel] 侧信息摘要
    pub info: [[GranuleInfo; 2]; 2],
}

static FRAME_INFO_STORE: OnceLock<Mutex<Vec<FrameInfo>>> = OnceLock::new();
static PART2_INFO_STORE: OnceLock<Mutex<Vec<Part2Info>>> = OnceLock::new();
static SNAPSHOT_STORE: OnceLock<Mutex<Vec<FrameSnapshot>>> = OnceLock::new();
static HUFFMAN_ERR_STORE: OnceLock<Mutex<Vec<HuffmanErrorInfo>>> = OnceLock::new();

/// 记录帧信息 (仅在显式开启环境变量时调用)
pub fn record_frame_info(info: FrameInfo) {
    if std::env::var("TAO_MP3_DEBUG_FRAME_INFO").is_err() {
        return;
    }
    let store = FRAME_INFO_STORE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = store.lock() {
        guard.push(info);
    }
}

/// 取走已记录的帧信息
pub fn take_frame_infos() -> Vec<FrameInfo> {
    let store = FRAME_INFO_STORE.get_or_init(|| Mutex::new(Vec::new()));
    match store.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Vec::new(),
    }
}

/// Part2(Scalefactor) 读取统计信息
#[derive(Debug, Clone)]
pub struct Part2Info {
    pub frame_index: u32,
    pub gr: u8,
    pub ch: u8,
    pub part2_bits: u32,
    pub part2_3_length: u32,
    pub block_type: u8,
    pub mixed_block_flag: bool,
    pub windows_switching_flag: bool,
    pub scalefac_compress: u32,
    pub slen1: u8,
    pub slen2: u8,
    pub scfsi: [u8; 4],
}

/// 记录 Part2 统计信息
pub fn record_part2_info(info: Part2Info) {
    if std::env::var("TAO_MP3_DEBUG_PART2").is_err() {
        return;
    }
    let store = PART2_INFO_STORE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = store.lock() {
        guard.push(info);
    }
}

/// 取走已记录的 Part2 统计信息
pub fn take_part2_infos() -> Vec<Part2Info> {
    let store = PART2_INFO_STORE.get_or_init(|| Mutex::new(Vec::new()));
    match store.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Vec::new(),
    }
}

/// Huffman 解码异常信息
#[derive(Debug, Clone)]
pub struct HuffmanErrorInfo {
    pub frame_index: u32,
    pub gr: u8,
    pub ch: u8,
    pub stage: &'static str,
    pub bit_offset: usize,
    pub end_bit: usize,
}

/// 记录 Huffman 解码异常
pub fn record_huffman_error(info: HuffmanErrorInfo) {
    if std::env::var("TAO_MP3_DEBUG_HUFFMAN_ERR").is_err() {
        return;
    }
    let store = HUFFMAN_ERR_STORE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = store.lock() {
        guard.push(info);
    }
}

/// 取走已记录的 Huffman 解码异常
pub fn take_huffman_errors() -> Vec<HuffmanErrorInfo> {
    let store = HUFFMAN_ERR_STORE.get_or_init(|| Mutex::new(Vec::new()));
    match store.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Vec::new(),
    }
}

/// 是否启用帧快照记录
pub fn snapshot_enabled() -> bool {
    std::env::var("TAO_MP3_SNAPSHOT").is_ok()
}

fn parse_env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
}

/// 判断是否应记录指定 frame/gr/ch
pub fn should_record_snapshot(frame_index: usize, gr: usize, ch: usize) -> bool {
    if !snapshot_enabled() {
        return false;
    }
    if let Some(target) = parse_env_usize("TAO_MP3_SNAPSHOT_FRAME") {
        if target != frame_index {
            return false;
        }
    }
    if let Some(target) = parse_env_usize("TAO_MP3_SNAPSHOT_GR") {
        if target != gr {
            return false;
        }
    }
    if let Some(target) = parse_env_usize("TAO_MP3_SNAPSHOT_CH") {
        if target != ch {
            return false;
        }
    }
    true
}

/// 记录快照 (遵循环境变量过滤与数量上限)
pub fn record_snapshot(snapshot: FrameSnapshot) {
    if !snapshot_enabled() {
        return;
    }
    if !should_record_snapshot(snapshot.frame_index, snapshot.gr, snapshot.ch) {
        return;
    }
    let max_count = parse_env_usize("TAO_MP3_SNAPSHOT_MAX").unwrap_or(8);
    let store = SNAPSHOT_STORE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = store.lock() {
        if guard.len() < max_count {
            guard.push(snapshot);
        }
    }
}

/// 取走已记录的快照
pub fn take_snapshots() -> Vec<FrameSnapshot> {
    let store = SNAPSHOT_STORE.get_or_init(|| Mutex::new(Vec::new()));
    match store.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Vec::new(),
    }
}

/// 精度对比结果
#[derive(Debug, Clone)]
pub struct CompareResult {
    /// 阶段名称
    pub stage: String,
    /// 最大绝对误差
    pub max_abs_error: f64,
    /// 平均绝对误差
    pub mean_abs_error: f64,
    /// 均方误差
    pub mse: f64,
    /// PSNR (dB), 以 1.0 为满量程
    pub psnr_db: f64,
    /// 最大误差所在索引
    pub max_error_index: usize,
    /// 非零样本数 (参考)
    pub nonzero_count: usize,
    /// 总样本数
    pub total_samples: usize,
}

impl std::fmt::Display for CompareResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] 样本: {}/{}, 最大误差: {:.2e} (@{}), 平均: {:.2e}, MSE: {:.2e}, PSNR: {:.1}dB",
            self.stage,
            self.nonzero_count,
            self.total_samples,
            self.max_abs_error,
            self.max_error_index,
            self.mean_abs_error,
            self.mse,
            self.psnr_db,
        )
    }
}

/// 对比两组 f32 样本, 计算精度指标
pub fn compare_f32_samples(stage: &str, actual: &[f32], expected: &[f32]) -> CompareResult {
    let total = actual.len().min(expected.len());
    let mut max_abs_error: f64 = 0.0;
    let mut sum_abs_error: f64 = 0.0;
    let mut sum_sq_error: f64 = 0.0;
    let mut max_error_index = 0usize;
    let mut nonzero_count = 0usize;

    for i in 0..total {
        let a = actual[i] as f64;
        let e = expected[i] as f64;
        let err = (a - e).abs();

        if e.abs() > 1e-10 {
            nonzero_count += 1;
        }

        sum_abs_error += err;
        sum_sq_error += err * err;

        if err > max_abs_error {
            max_abs_error = err;
            max_error_index = i;
        }
    }

    let mean_abs_error = if total > 0 {
        sum_abs_error / total as f64
    } else {
        0.0
    };
    let mse = if total > 0 {
        sum_sq_error / total as f64
    } else {
        0.0
    };

    // PSNR: 以 1.0 为满量程 (PCM float 归一化范围 [-1, 1])
    let psnr_db = if mse > 0.0 {
        10.0 * (1.0f64 / mse).log10()
    } else {
        f64::INFINITY
    };

    CompareResult {
        stage: stage.to_string(),
        max_abs_error,
        mean_abs_error,
        mse,
        psnr_db,
        max_error_index,
        nonzero_count,
        total_samples: total,
    }
}

/// 对比两组 i32 样本 (Huffman 输出), 统计不匹配数
pub fn compare_i32_samples(stage: &str, actual: &[i32], expected: &[i32]) -> CompareResult {
    let total = actual.len().min(expected.len());
    let mut max_abs_error: f64 = 0.0;
    let mut sum_abs_error: f64 = 0.0;
    let mut max_error_index = 0usize;
    let mut mismatches = 0usize;

    for i in 0..total {
        let err = (actual[i] as f64 - expected[i] as f64).abs();
        if err > 0.0 {
            mismatches += 1;
        }
        sum_abs_error += err;
        if err > max_abs_error {
            max_abs_error = err;
            max_error_index = i;
        }
    }

    let mean_abs_error = if total > 0 {
        sum_abs_error / total as f64
    } else {
        0.0
    };
    let mse = mean_abs_error * mean_abs_error;
    let psnr_db = if mse > 0.0 {
        10.0 * (1.0f64 / mse).log10()
    } else {
        f64::INFINITY
    };

    CompareResult {
        stage: stage.to_string(),
        max_abs_error,
        mean_abs_error,
        mse,
        psnr_db,
        max_error_index,
        nonzero_count: mismatches,
        total_samples: total,
    }
}

/// 验收标准常量
pub mod acceptance {
    /// 单样本最大允许误差 (对标 FFmpeg, 针对归一化 [-1, 1] 的 PCM)
    pub const MAX_SAMPLE_ERROR: f32 = 1e-4;
    /// 帧级平均误差上限
    pub const MAX_FRAME_AVG_ERROR: f32 = 1e-5;
    /// PSNR 下限 (dB)
    pub const MIN_PSNR_DB: f32 = 80.0;
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

fn pow43(val: i32) -> f32 {
    let abs_val = val.abs() as f32;
    abs_val.powf(4.0 / 3.0)
}

pub fn reference_requantize_mpeg1(snapshot: &FrameSnapshot, sample_rate: u32) -> [f32; 576] {
    use super::tables::{
        PRETAB, SFB_WIDTH_LONG, SFB_WIDTH_SHORT, build_sfb_long_bounds, samplerate_index,
    };

    let sr_idx = samplerate_index(sample_rate);
    let long_widths = &SFB_WIDTH_LONG[sr_idx];
    let short_widths = &SFB_WIDTH_SHORT[sr_idx];
    let long_bounds = build_sfb_long_bounds(sample_rate);
    let short_bounds = build_short_bounds(short_widths);
    let mixed_bounds = build_mixed_bounds(long_widths, short_widths);

    let gain = snapshot.global_gain as i32 - 210;
    let sf_shift = if snapshot.scalefac_scale { 2 } else { 1 };
    let pre = if snapshot.preflag { PRETAB } else { [0u8; 22] };

    let mut base = [0.0f32; 576];
    for (i, b) in base.iter_mut().enumerate() {
        let v = snapshot.is_samples[i];
        if v != 0 {
            let p = pow43(v);
            *b = if v > 0 { p } else { -p };
        }
    }

    let mut out = [0.0f32; 576];
    let is_short = snapshot.windows_switching_flag && snapshot.block_type == 2;

    if !is_short {
        for sfb in 0..22 {
            let start = long_bounds[sfb];
            let end = long_bounds[sfb + 1];
            let b = i32::from((snapshot.scalefac[sfb] + pre[sfb]) << sf_shift);
            let mul = f64::powf(2.0, 0.25 * f64::from(gain - b)) as f32;
            for i in start..end {
                out[i] = base[i] * mul;
            }
        }
        return out;
    }

    if snapshot.mixed_block_flag {
        for sfb in 0..8 {
            let start = mixed_bounds[sfb];
            let end = mixed_bounds[sfb + 1];
            let b = i32::from((snapshot.scalefac[sfb] + pre[sfb]) << sf_shift);
            let mul = f64::powf(2.0, 0.25 * f64::from(gain - b)) as f32;
            for i in start..end {
                out[i] = base[i] * mul;
            }
        }

        let a = [
            gain - 8 * snapshot.subblock_gain[0] as i32,
            gain - 8 * snapshot.subblock_gain[1] as i32,
            gain - 8 * snapshot.subblock_gain[2] as i32,
        ];

        for seg in 8..(mixed_bounds.len() - 1) {
            let start = mixed_bounds[seg];
            let end = mixed_bounds[seg + 1];

            let short_seg = seg - 8;
            let window = short_seg % 3;
            let short_sfb = 3 + short_seg / 3;
            let b = if short_sfb < 12 {
                i32::from(snapshot.scalefac[8 + (short_sfb - 3) * 3 + window] << sf_shift)
            } else {
                0
            };
            let mul = f64::powf(2.0, 0.25 * f64::from(a[window] - b)) as f32;
            for i in start..end {
                out[i] = base[i] * mul;
            }
        }
        return out;
    }

    let a = [
        gain - 8 * snapshot.subblock_gain[0] as i32,
        gain - 8 * snapshot.subblock_gain[1] as i32,
        gain - 8 * snapshot.subblock_gain[2] as i32,
    ];
    for sfb in 0..39 {
        let start = short_bounds[sfb];
        let end = short_bounds[sfb + 1];
        let b = i32::from(snapshot.scalefac[sfb] << sf_shift);
        let mul = f64::powf(2.0, 0.25 * f64::from(a[sfb % 3] - b)) as f32;
        for i in start..end {
            out[i] = base[i] * mul;
        }
    }

    out
}

fn build_imdct_windows() -> [[f32; 36]; 4] {
    use std::f64::consts::PI;
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

    for i in 0..12 {
        windows[2][i] = (PI / 12.0 * (i as f64 + 0.5)).sin() as f32;
    }

    windows
}

fn imdct18_direct(input: &[f32; 18]) -> [f32; 36] {
    use std::f64::consts::PI;
    let mut output = [0.0f32; 36];
    for i in 0..36 {
        let mut sum = 0.0f64;
        for k in 0..18 {
            let angle = PI / 72.0 * (2.0 * i as f64 + 19.0) * (2.0 * k as f64 + 1.0);
            sum += input[k] as f64 * angle.cos();
        }
        output[i] = sum as f32;
    }
    output
}

fn imdct12_direct(input: &[f32; 6]) -> [f32; 12] {
    use std::f64::consts::PI;
    let mut output = [0.0f32; 12];
    for i in 0..12 {
        let mut sum = 0.0f64;
        for k in 0..6 {
            let angle = PI / 24.0 * (2.0 * i as f64 + 7.0) * (2.0 * k as f64 + 1.0);
            sum += input[k] as f64 * angle.cos();
        }
        output[i] = sum as f32;
    }
    output
}

pub fn reference_imdct(snapshot: &FrameSnapshot) -> [f32; 576] {
    let windows = build_imdct_windows();
    let short_win = &windows[2][0..12];
    let mut overlap = snapshot.overlap_before;
    let mut output = [0.0f32; 576];
    let block_type = if snapshot.windows_switching_flag {
        snapshot.block_type
    } else {
        0
    };
    let imdct_scale = std::env::var("TAO_MP3_IMDCT_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0);

    let sb_limit = snapshot.rzero.div_ceil(18).min(32);
    let sb_split = if snapshot.windows_switching_flag && block_type == 2 {
        if snapshot.mixed_block_flag { 2 } else { 0 }
    } else {
        32
    };

    let sb_long_end = sb_split.min(sb_limit);
    for sb in 0..sb_long_end {
        let sb_idx = sb * 18;
        let input_chunk: &[f32; 18] = (&snapshot.xr_after_alias[sb_idx..sb_idx + 18])
            .try_into()
            .unwrap();
        let mut raw_out = imdct18_direct(input_chunk);

        let mut win_idx = if snapshot.windows_switching_flag
            && block_type == 2
            && snapshot.mixed_block_flag
            && sb < 2
        {
            0
        } else {
            block_type as usize
        };
        if win_idx > 3 {
            win_idx = 0;
        }
        if std::env::var("TAO_MP3_FORCE_NORMAL_TRANSITION_WIN").is_ok_and(|v| v == "1")
            && (win_idx == 1 || win_idx == 3)
        {
            win_idx = 0;
        }
        let win = &windows[win_idx];
        for i in 0..36 {
            raw_out[i] *= win[i] * imdct_scale;
        }

        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }

    let sb_short_begin = sb_split.min(sb_limit);
    for sb in sb_short_begin..sb_limit {
        let sb_idx = sb * 18;
        let input_chunk: &[f32; 18] = (&snapshot.xr_after_alias[sb_idx..sb_idx + 18])
            .try_into()
            .unwrap();
        let mut raw_out = [0.0f32; 36];

        for w in 0..3 {
            let mut x = [0.0f32; 6];
            for i in 0..6 {
                x[i] = input_chunk[w + 3 * i];
            }
            let y = imdct12_direct(&x);
            for i in 0..12 {
                raw_out[6 + 6 * w + i] += y[i] * short_win[i];
            }
        }

        for i in 0..36 {
            raw_out[i] *= imdct_scale;
        }
        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }

    for sb in sb_limit..32 {
        let base = sb * 18;
        output[base..base + 18].copy_from_slice(&overlap[sb]);
    }

    output
}

fn reference_imdct_from_granule(
    granule: &Granule,
    xr: &[f32; 576],
    rzero: usize,
    overlap: &mut [[f32; 18]; 32],
) -> [f32; 576] {
    let windows = build_imdct_windows();
    let short_win = &windows[2][0..12];
    let mut output = [0.0f32; 576];
    let block_type = if granule.windows_switching_flag {
        granule.block_type
    } else {
        0
    };
    let imdct_scale = std::env::var("TAO_MP3_IMDCT_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0);

    let sb_limit = rzero.div_ceil(18).min(32);
    let sb_split = if granule.windows_switching_flag && block_type == 2 {
        if granule.mixed_block_flag { 2 } else { 0 }
    } else {
        32
    };

    let sb_long_end = sb_split.min(sb_limit);
    for sb in 0..sb_long_end {
        let sb_idx = sb * 18;
        let input_chunk: &[f32; 18] = (&xr[sb_idx..sb_idx + 18]).try_into().unwrap();
        let mut raw_out = imdct18_direct(input_chunk);

        let mut win_idx = if granule.windows_switching_flag
            && block_type == 2
            && granule.mixed_block_flag
            && sb < 2
        {
            0
        } else {
            block_type as usize
        };
        if win_idx > 3 {
            win_idx = 0;
        }
        if std::env::var("TAO_MP3_FORCE_NORMAL_TRANSITION_WIN").is_ok_and(|v| v == "1")
            && (win_idx == 1 || win_idx == 3)
        {
            win_idx = 0;
        }
        let win = &windows[win_idx];
        for i in 0..36 {
            raw_out[i] *= win[i] * imdct_scale;
        }

        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }

    let sb_short_begin = sb_split.min(sb_limit);
    for sb in sb_short_begin..sb_limit {
        let sb_idx = sb * 18;
        let input_chunk: &[f32; 18] = (&xr[sb_idx..sb_idx + 18]).try_into().unwrap();
        let mut raw_out = [0.0f32; 36];

        for w in 0..3 {
            let mut x = [0.0f32; 6];
            for i in 0..6 {
                x[i] = input_chunk[w + 3 * i];
            }
            let y = imdct12_direct(&x);
            for i in 0..12 {
                raw_out[6 + 6 * w + i] += y[i] * short_win[i];
            }
        }

        for i in 0..36 {
            raw_out[i] *= imdct_scale;
        }
        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }

    for sb in sb_limit..32 {
        let base = sb * 18;
        output[base..base + 18].copy_from_slice(&overlap[sb]);
        overlap[sb].fill(0.0);
    }

    output
}

fn reference_matrixing(samples: &[f32; 32]) -> [f64; 64] {
    use std::f64::consts::PI;
    let mut v = [0.0f64; 64];
    for i in 0..64 {
        let mut sum = 0.0f64;
        for k in 0..32 {
            let angle = PI / 64.0 * (16.0 + i as f64) * (2.0 * k as f64 + 1.0);
            sum += samples[k] as f64 * angle.cos();
        }
        v[i] = sum;
    }
    v
}

#[derive(Clone)]
struct ReferenceSynthContext {
    v_vec: [[f64; 64]; 16],
    v_front: usize,
}

impl ReferenceSynthContext {
    fn from_snapshot(snapshot: &FrameSnapshot) -> Self {
        let mut v_vec = [[0.0f64; 64]; 16];
        for (dst, src) in v_vec.iter_mut().zip(snapshot.synth_v_vec.iter()) {
            for (d, s) in dst.iter_mut().zip(src.iter()) {
                *d = *s as f64;
            }
        }
        Self {
            v_vec,
            v_front: snapshot.synth_v_front & 0xf,
        }
    }
}

impl Default for ReferenceSynthContext {
    fn default() -> Self {
        Self {
            v_vec: [[0.0f64; 64]; 16],
            v_front: 0,
        }
    }
}

fn reference_synthesis_step(ctx: &mut ReferenceSynthContext, new_samples: &[f32; 32]) -> [f32; 32] {
    let v = reference_matrixing(new_samples);
    ctx.v_vec[ctx.v_front] = v;

    let mut pcm = [0.0f32; 32];
    for j in 0..8 {
        let v_start = ctx.v_front + (j << 1);
        let v0 = &ctx.v_vec[v_start & 0xf][0..32];
        let v1 = &ctx.v_vec[(v_start + 1) & 0xf][32..64];
        let k = j << 6;

        for i in 0..32 {
            let mut sum = pcm[i] as f64;
            sum += v0[i] * f64::from(super::synthesis::SYNTH_WINDOW[k + i]);
            sum += v1[i] * f64::from(super::synthesis::SYNTH_WINDOW[k + 32 + i]);
            pcm[i] = sum as f32;
        }
    }

    ctx.v_front = (ctx.v_front + 15) & 0xf;
    pcm
}

pub fn reference_synthesis_pcm(snapshot: &FrameSnapshot) -> [f32; 576] {
    let mut ctx = ReferenceSynthContext::from_snapshot(snapshot);
    let mut pcm = [0.0f32; 576];

    for k in 0..18 {
        let mut subband = [0.0f32; 32];
        for sb in 0..32 {
            subband[sb] = snapshot.after_freq_inversion[sb * 18 + k];
        }
        let out = reference_synthesis_step(&mut ctx, &subband);
        pcm[k * 32..k * 32 + 32].copy_from_slice(&out);
    }

    pcm
}

#[derive(Clone)]
struct ReferencePipeline {
    overlap: [[[f32; 18]; 32]; 2],
    synth: [ReferenceSynthContext; 2],
}

impl Default for ReferencePipeline {
    fn default() -> Self {
        Self {
            overlap: [[[0.0; 18]; 32]; 2],
            synth: [
                ReferenceSynthContext::default(),
                ReferenceSynthContext::default(),
            ],
        }
    }
}

#[derive(Clone)]
pub struct ReferenceOutputs {
    pub imdct_output: [f32; 576],
    pub pcm_output: [f32; 576],
}

static REF_PIPELINE: OnceLock<Mutex<ReferencePipeline>> = OnceLock::new();

pub fn reference_pipeline_enabled() -> bool {
    std::env::var("TAO_MP3_DEBUG_REF_PIPELINE").is_ok()
}

pub fn reset_reference_pipeline() {
    if !reference_pipeline_enabled() {
        return;
    }
    let store = REF_PIPELINE.get_or_init(|| Mutex::new(ReferencePipeline::default()));
    if let Ok(mut guard) = store.lock() {
        *guard = ReferencePipeline::default();
    }
}

pub fn reference_pipeline_step(
    granule: &Granule,
    xr: &[f32; 576],
    rzero: usize,
    ch: usize,
) -> Option<ReferenceOutputs> {
    if !reference_pipeline_enabled() {
        return None;
    }
    let store = REF_PIPELINE.get_or_init(|| Mutex::new(ReferencePipeline::default()));
    let mut guard = store.lock().ok()?;
    let ch_idx = ch.min(1);
    let imdct_output = reference_imdct_from_granule(granule, xr, rzero, &mut guard.overlap[ch_idx]);

    let mut freq = imdct_output;
    if std::env::var("TAO_MP3_DISABLE_FREQ_INV").is_err() {
        super::synthesis::frequency_inversion(&mut freq);
    }

    let mut pcm = [0.0f32; 576];
    for k in 0..18 {
        let mut subband = [0.0f32; 32];
        for sb in 0..32 {
            subband[sb] = freq[sb * 18 + k];
        }
        let out = reference_synthesis_step(&mut guard.synth[ch_idx], &subband);
        pcm[k * 32..k * 32 + 32].copy_from_slice(&out);
    }

    Some(ReferenceOutputs {
        imdct_output,
        pcm_output: pcm,
    })
}
