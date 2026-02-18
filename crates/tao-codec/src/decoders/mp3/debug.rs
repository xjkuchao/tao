//! MP3 解码器诊断工具模块
//!
//! 提供各阶段中间数据的导出、快照和对比功能,
//! 仅在测试编译时启用 (cfg(test))

use super::data::{IsSpectrum, XrSpectrum};

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
    /// 频率反转后
    pub after_freq_inversion: [f32; 576],
    /// 合成滤波器输出 (PCM)
    pub pcm_output: Vec<f32>,
    /// rzero 值
    pub rzero: usize,
}

impl Default for FrameSnapshot {
    fn default() -> Self {
        Self {
            frame_index: 0,
            gr: 0,
            ch: 0,
            is_samples: [0; 576],
            xr_after_requantize: [0.0; 576],
            xr_after_stereo: [0.0; 576],
            xr_after_reorder: [0.0; 576],
            xr_after_alias: [0.0; 576],
            imdct_output: [0.0; 576],
            after_freq_inversion: [0.0; 576],
            pcm_output: Vec::new(),
            rzero: 0,
        }
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
