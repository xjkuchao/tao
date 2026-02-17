// 测试辅助工具：FFmpeg 集成与对比验证
// 位置: tests/ffmpeg_compare.rs

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// FFmpeg 参考解码器输出管理
#[allow(dead_code)]
pub struct FfmpegComparer {
    /// 输入媒体文件路径
    input_file: PathBuf,
    /// 输出断言文件所在目录
    output_dir: PathBuf,
}

impl FfmpegComparer {
    /// 创建新的 FFmpeg 对比器
    ///
    /// # 参数
    /// - `input_file`: 输入媒体文件路径
    /// - `output_dir`: 输出参考文件的目录
    ///
    /// # 返回
    /// 成功返回 FfmpegComparer，失败返回错误描述
    #[allow(dead_code)]
    pub fn new<P: AsRef<Path>>(input_file: P, output_dir: P) -> Result<Self, String> {
        let input = input_file.as_ref().to_path_buf();
        let output = output_dir.as_ref().to_path_buf();

        if !input.exists() {
            return Err(format!("输入文件不存在: {:?}", input));
        }

        fs::create_dir_all(&output).map_err(|e| format!("无法创建输出目录: {}", e))?;

        Ok(FfmpegComparer {
            input_file: input,
            output_dir: output,
        })
    }

    /// 检查 FFmpeg 是否已安装
    ///
    /// # 返回
    /// true - FFmpeg 可用; false - 不可用或未安装
    pub fn check_ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// 使用 FFmpeg 生成参考输出视频帧
    ///
    /// 将输入视频解码为原始 YUV420p 格式，便于像素级对比。
    ///
    /// # 参数
    /// - `frames`: 要输出的帧数 (0 = 所有帧)
    ///
    /// # 返回
    /// 成功返回参考输出文件路径，失败返回错误
    ///
    /// # 备注
    /// 输出文件名格式: `reference_frames.yuv`
    #[allow(dead_code)]
    pub fn generate_reference_frames(&self, frames: u32) -> Result<PathBuf, String> {
        let output_file = self.output_dir.join("reference_frames.yuv");

        // 检查 FFmpeg 可用性
        if !Self::check_ffmpeg_available() {
            return Err("FFmpeg 未安装或不可用，无法生成参考输出".to_string());
        }

        // 构建 FFmpeg 命令
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y") // Overwrite output file
            .arg("-i")
            .arg(&self.input_file)
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-f")
            .arg("rawvideo");

        // 如果指定帧数，限制输出
        if frames > 0 {
            cmd.arg("-vframes").arg(frames.to_string());
        }

        cmd.arg("-loglevel").arg("error").arg(&output_file);

        // 执行命令
        let output = cmd
            .output()
            .map_err(|e| format!("FFmpeg 执行失败: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("FFmpeg 解码失败: {}", stderr));
        }

        if !output_file.exists() {
            return Err(format!("参考输出文件不存在: {:?}", output_file));
        }

        Ok(output_file)
    }

    /// 生成 FFmpeg 媒体信息日志
    ///
    /// 使用 ffprobe 获取输入文件的详细格式信息。
    ///
    /// # 返回
    /// 成功返回 JSON 格式的媒体信息字符串，失败返回错误
    #[allow(dead_code)]
    pub fn probe_media_info(&self) -> Result<String, String> {
        let output = Command::new("ffprobe")
            .arg("-v")
            .arg("error")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(&self.input_file)
            .output()
            .map_err(|e| format!("ffprobe 执行失败: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ffprobe 失败: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// 获取媒体的基本信息
    ///
    /// 解析 ffprobe 输出，返回分辨率、帧率等关键信息。
    ///
    /// # 返回
    /// 包含视频分辨率和帧率的元组 (width, height, fps) 或错误
    #[allow(dead_code)]
    pub fn get_video_info(&self) -> Result<(u32, u32, f64), String> {
        let json_str = self.probe_media_info()?;

        // 简化解析：使用正则或简单字符串查找
        // 实际项目中应使用 serde_json
        let width = extract_json_value(&json_str, "\"width\"").ok_or("无法找到视频宽度")?;
        let height = extract_json_value(&json_str, "\"height\"").ok_or("无法找到视频高度")?;
        let r_frame_rate =
            extract_json_string(&json_str, "\"r_frame_rate\"").ok_or("无法找到帧率")?;

        // 解析 "30/1" 格式的帧率
        let fps = if let Some(slash_pos) = r_frame_rate.find('/') {
            let (num_str, den_str) = r_frame_rate.split_at(slash_pos);
            let num: f64 = num_str
                .parse()
                .map_err(|_| "无法解析帧率分子".to_string())?;
            let den: f64 = den_str[1..]
                .parse()
                .map_err(|_| "无法解析帧率分母".to_string())?;
            num / den
        } else {
            r_frame_rate
                .parse()
                .map_err(|_| "无法解析帧率".to_string())?
        };

        Ok((width, height, fps))
    }

    /// 获取参考输出文件路径
    #[allow(dead_code)]
    pub fn reference_output_path(&self) -> PathBuf {
        self.output_dir.join("reference_frames.yuv")
    }

    /// 获取输出目录
    #[allow(dead_code)]
    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

/// 从 JSON 字符串中提取整数值 (简易版本)
fn extract_json_value(json: &str, key: &str) -> Option<u32> {
    let start_idx = json.find(key)?;
    let after_colon = json[start_idx + key.len()..].find(':')?;
    let num_start = start_idx + key.len() + after_colon + 1;
    let num_str = &json[num_start..];

    // 查找第一个数字
    let digits: String = num_str.chars().take_while(|c| c.is_ascii_digit()).collect();

    digits.parse().ok()
}

/// 从 JSON 字符串中提取字符串值 (简易版本)
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let start_idx = json.find(key)?;
    let after_colon = json[start_idx + key.len()..].find(':')?;
    let str_start = start_idx + key.len() + after_colon + 1;

    // 查找引号之间的字符串
    let after_quote = json[str_start..].find('"')?;
    let actual_start = str_start + after_quote + 1;
    let end_quote = json[actual_start..].find('"')?;

    Some(json[actual_start..actual_start + end_quote].to_string())
}

/// 像素级差异统计
#[allow(dead_code)]
pub struct FrameDiff {
    /// Y 平面最大差异
    pub max_y_diff: u32,
    /// U 平面最大差异
    pub max_u_diff: u32,
    /// V 平面最大差异
    pub max_v_diff: u32,
    /// Y 平面均方误差 (MSE)
    pub mse_y: f64,
    /// U 平面均方误差
    pub mse_u: f64,
    /// V 平面均方误差
    pub mse_v: f64,
    /// Y 平面 PSNR (分贝)
    pub psnr_y: f64,
    /// U 平面 PSNR
    pub psnr_u: f64,
    /// V 平面 PSNR
    pub psnr_v: f64,
}

impl FrameDiff {
    /// 计算两个 YUV420p 帧之间的像素差异
    ///
    /// # 参数
    /// - `frame1`: 第一个帧数据 (YUV420p 原始格式)
    /// - `frame2`: 第二个帧数据
    /// - `width`, `height`: 视频分辨率
    ///
    /// # 返回
    /// 差异统计信息或错误
    pub fn compare(frame1: &[u8], frame2: &[u8], width: u32, height: u32) -> Result<Self, String> {
        if frame1.len() != frame2.len() {
            return Err(format!(
                "帧大小不匹配: {} vs {}",
                frame1.len(),
                frame2.len()
            ));
        }

        let y_size = (width * height) as usize;
        let uv_size = (width.div_ceil(2) * height.div_ceil(2)) as usize;

        if frame1.len() < y_size + 2 * uv_size {
            return Err(format!(
                "帧数据过小: {} < {}",
                frame1.len(),
                y_size + 2 * uv_size
            ));
        }

        // 提取 Y/U/V 平面
        let y1 = &frame1[..y_size];
        let u1 = &frame1[y_size..y_size + uv_size];
        let v1 = &frame1[y_size + uv_size..y_size + 2 * uv_size];

        let y2 = &frame2[..y_size];
        let u2 = &frame2[y_size..y_size + uv_size];
        let v2 = &frame2[y_size + uv_size..y_size + 2 * uv_size];

        // 计算差异
        let (max_y, mse_y) = Self::compute_diff(y1, y2)?;
        let (max_u, mse_u) = Self::compute_diff(u1, u2)?;
        let (max_v, mse_v) = Self::compute_diff(v1, v2)?;

        // 计算 PSNR (PSNR = 20 * log10(255 / sqrt(MSE)))
        let psnr_y = if mse_y > 0.0 {
            20.0 * (255.0 / mse_y.sqrt()).log10()
        } else {
            f64::INFINITY
        };
        let psnr_u = if mse_u > 0.0 {
            20.0 * (255.0 / mse_u.sqrt()).log10()
        } else {
            f64::INFINITY
        };
        let psnr_v = if mse_v > 0.0 {
            20.0 * (255.0 / mse_v.sqrt()).log10()
        } else {
            f64::INFINITY
        };

        Ok(FrameDiff {
            max_y_diff: max_y,
            max_u_diff: max_u,
            max_v_diff: max_v,
            mse_y,
            mse_u,
            mse_v,
            psnr_y,
            psnr_u,
            psnr_v,
        })
    }

    /// 计算单个平面的最大差异和 MSE
    fn compute_diff(data1: &[u8], data2: &[u8]) -> Result<(u32, f64), String> {
        if data1.len() != data2.len() {
            return Err("数据长度不匹配".to_string());
        }

        let mut max_diff = 0u32;
        let mut sum_sq: f64 = 0.0;

        for (v1, v2) in data1.iter().zip(data2.iter()) {
            let diff = (*v1 as i32 - *v2 as i32).unsigned_abs();
            max_diff = max_diff.max(diff);
            sum_sq += (diff as f64) * (diff as f64);
        }

        let mse = sum_sq / (data1.len() as f64);
        Ok((max_diff, mse))
    }

    /// 格式化差异统计为可读字符串
    #[allow(dead_code)]
    pub fn summary(&self) -> String {
        format!(
            "Frame Diff Summary:\n  
            Max Y: {}, U: {}, V: {}\n  
            MSE Y: {:.2}, U: {:.2}, V: {:.2}\n  
            PSNR Y: {:.2} dB, U: {:.2} dB, V: {:.2} dB",
            self.max_y_diff,
            self.max_u_diff,
            self.max_v_diff,
            self.mse_y,
            self.mse_u,
            self.mse_v,
            self.psnr_y,
            self.psnr_u,
            self.psnr_v
        )
    }

    /// 检查差异是否在可接受范围内
    ///
    /// 默认容差: PSNR >= 30 dB (相当好的质量)
    pub fn is_acceptable(&self) -> bool {
        self.psnr_y >= 30.0 && self.psnr_u >= 30.0 && self.psnr_v >= 30.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffmpeg_available() {
        let available = FfmpegComparer::check_ffmpeg_available();
        println!("FFmpeg available: {}", available);
        // 此测试不断言，因为在 CI 环境中 FFmpeg 可能未装
    }

    #[test]
    fn test_frame_diff_identical_frames() {
        // 创建两个相同的帧数据
        let frame = vec![128u8; 1920 * 1080 + 2 * 960 * 540];
        let diff = FrameDiff::compare(&frame, &frame, 1920, 1080).expect("对比失败");

        assert_eq!(diff.max_y_diff, 0);
        assert_eq!(diff.max_u_diff, 0);
        assert_eq!(diff.max_v_diff, 0);
        assert!(diff.psnr_y.is_infinite());
        assert!(diff.is_acceptable());
    }

    #[test]
    fn test_frame_diff_small_difference() {
        // 创建几乎相同的帧
        let frame1 = vec![128u8; 1920 * 1080 + 2 * 960 * 540];
        let mut frame2 = frame1.clone();
        frame2[0] = 130; // 差异 2

        let diff = FrameDiff::compare(&frame1, &frame2, 1920, 1080).expect("对比失败");
        assert_eq!(diff.max_y_diff, 2);
        println!("Small diff PSNR Y: {:.2} dB", diff.psnr_y);
    }
}
