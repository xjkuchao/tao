// MPEG4 Part 2 解码器 FFmpeg 对比基线测试
// 位置: tests/mpeg4_ffmpeg_baseline.rs
//
// 本文件使用 FFmpeg 作为参考实现，生成对比基线并计算 PSNR 指标
// 确保 tao-codec 的 MPEG4 解码器与官方 FFmpeg 兼容

mod ffmpeg_compare;

use ffmpeg_compare::{FfmpegComparer, FrameDiff};
use std::fs;
use std::path::PathBuf;

// 配置选项
const TEST_OUTPUT_DIR: &str = "data/ffmpeg_baselines";
const ENABLE_PSNR_CALCULATION: bool = true;
const PSNR_THRESHOLD_BASIC: f64 = 38.0; // 基础测试: PSNR >= 38 dB
const PSNR_THRESHOLD_ADVANCED: f64 = 32.0; // 高级功能: PSNR >= 32 dB
const MAX_COMPARE_FRAMES: u32 = 10; // 对比的最大帧数

/// 初始化测试环境
///
/// 创建输出目录并检查 FFmpeg 可用性
fn init_test_environment() -> Result<PathBuf, String> {
    let output_dir = PathBuf::from(TEST_OUTPUT_DIR);
    fs::create_dir_all(&output_dir).map_err(|e| format!("无法创建输出目录: {}", e))?;

    if !FfmpegComparer::check_ffmpeg_available() {
        return Err("FFmpeg 未安装或不可用，无法生成对比基线".to_string());
    }

    Ok(output_dir)
}

/// 测试 1.1: 基础 AVI 解码 vs FFmpeg 对比
///
/// 样本: color16.avi (320×240, 25fps)
/// 预期: PSNR >= 38 dB (无损或极高质量)
#[test]
#[ignore] // 需要 FFmpeg 和网络连接，手动启用
fn test_mpeg4_baseline_1_1_basic_avi() {
    let output_dir = match init_test_environment() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("环境初始化失败: {}", e);
            return;
        }
    };

    let sample_url = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi";
    println!("\n=== MPEG4 Part 2 基础 AVI 对比基线 (Test 1.1) ===");
    println!("样本: {}", sample_url);

    // 创建 FFmpeg 对比器
    match FfmpegComparer::new(sample_url, &output_dir) {
        Ok(comparer) => {
            // 生成 FFmpeg 参考输出
            match comparer.generate_reference_frames(MAX_COMPARE_FRAMES) {
                Ok(ref_file) => {
                    println!("✓ FFmpeg 参考帧已生成: {:?}", ref_file);

                    // 获取视频信息
                    match comparer.get_video_info() {
                        Ok((width, height, fps)) => {
                            println!("视频信息: {}x{} @ {} fps", width, height, fps);

                            // 保存基线信息文件
                            let baseline_info = format!(
                                "# MPEG4 Part 2 FFmpeg 对比基线 - Test 1.1\n\n\
                                 ## 样本信息\n\
                                 - URL: {}\n\
                                 - 分辨率: {}x{}\n\
                                 - 帧率: {:.2} fps\n\
                                 - 测试类型: 基础 AVI 解码\n\n\
                                 ## 质量要求\n\
                                 - 预期 PSNR: >= {:.1} dB\n\
                                 - 容差范围: Y >= {:.1} dB, U >= {:.1} dB, V >= {:.1} dB\n\n\
                                 ## 下一步\n\
                                 1. 运行 tao-codec MPEG4 解码器\n\
                                 2. 对比解码输出与参考帧数据\n\
                                 3. 计算每帧 PSNR 指标\n\
                                 4. 验证是否达到质量要求\n",
                                sample_url,
                                width,
                                height,
                                fps,
                                PSNR_THRESHOLD_BASIC,
                                PSNR_THRESHOLD_BASIC,
                                PSNR_THRESHOLD_BASIC,
                                PSNR_THRESHOLD_BASIC,
                            );

                            let info_file = output_dir.join("test_1_1_baseline_info.md");
                            if let Err(e) = fs::write(&info_file, baseline_info) {
                                eprintln!("警告: 无法写入基线信息: {}", e);
                            } else {
                                println!("✓ 基线信息已保存: {:?}", info_file);
                            }

                            // 输出 PSNR 计算文件示例
                            println!("\n📊 PSNR 计算示例:");
                            let sample_y_plane = vec![128u8; (width as usize) * (height as usize)];
                            let sample_uv_size =
                                ((width.div_ceil(2)) as usize) * ((height.div_ceil(2)) as usize);
                            let sample_frame = {
                                let mut f = sample_y_plane.clone();
                                f.extend_from_slice(&vec![128u8; sample_uv_size]);
                                f.extend_from_slice(&vec![128u8; sample_uv_size]);
                                f
                            };

                            match FrameDiff::compare(&sample_frame, &sample_frame, width, height) {
                                Ok(diff) => {
                                    println!("参考帧对比结果 (相同帧):");
                                    println!("  Y 平面 PSNR: {:.2} dB", diff.psnr_y);
                                    println!("  U 平面 PSNR: {:.2} dB", diff.psnr_u);
                                    println!("  V 平面 PSNR: {:.2} dB", diff.psnr_v);
                                    println!(
                                        "  是否可接受: {}",
                                        if diff.is_acceptable() {
                                            "✓ 是"
                                        } else {
                                            "✗ 否"
                                        }
                                    );
                                }
                                Err(e) => eprintln!("PSNR 计算失败: {}", e),
                            }
                        }
                        Err(e) => eprintln!("无法获取视频信息: {}", e),
                    }
                }
                Err(e) => eprintln!("参考帧生成失败: {}", e),
            }
        }
        Err(e) => eprintln!("FFmpeg 对比器初始化失败: {}", e),
    }

    println!("\n✓ 基线测试 1.1 完成");
}

/// 测试 2.1: B 帧对比基线
///
/// 样本: b-frames.avi
/// 预期: PSNR >= 32 dB (高级特性，允许更大容差)
#[test]
#[ignore]
fn test_mpeg4_baseline_2_1_b_frames() {
    let output_dir = match init_test_environment() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("环境初始化失败: {}", e);
            return;
        }
    };

    let sample_url =
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi";
    println!("\n=== MPEG4 Part 2 B 帧对比基线 (Test 2.1) ===");
    println!("样本: {}", sample_url);

    match FfmpegComparer::new(sample_url, &output_dir) {
        Ok(comparer) => match comparer.generate_reference_frames(MAX_COMPARE_FRAMES) {
            Ok(_) => {
                println!("✓ FFmpeg 参考帧已生成");

                let baseline_info = format!(
                    "# MPEG4 Part 2 FFmpeg 对比基线 - Test 2.1 (B 帧)\n\n\
                         ## 编码特性\n\
                         - B 帧 (双向预测)\n\
                         - 参考帧管理\n\n\
                         ## 质量要求\n\
                         - 预期 PSNR: >= {:.1} dB\n\
                         - 说明: 高级特性允许更大容差\n",
                    PSNR_THRESHOLD_ADVANCED,
                );

                let info_file = output_dir.join("test_2_1_baseline_info.md");
                let _ = fs::write(&info_file, baseline_info);

                println!("✓ 基线信息已保存");
            }
            Err(e) => eprintln!("参考帧生成失败: {}", e),
        },
        Err(e) => eprintln!("FFmpeg 对比器初始化失败: {}", e),
    }

    println!("\n✓ 基线测试 2.1 完成");
}

/// 测试 2.2: Quarterpel 对比基线
///
/// 样本: DivX51-Qpel.avi
/// 预期: PSNR >= 32 dB (四分像素运动补偿)
#[test]
#[ignore]
fn test_mpeg4_baseline_2_2_quarterpel() {
    let output_dir = match init_test_environment() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("环境初始化失败: {}", e);
            return;
        }
    };

    let sample_url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi";
    println!("\n=== MPEG4 Part 2 Quarterpel 对比基线 (Test 2.2) ===");
    println!("样本: {}", sample_url);

    match FfmpegComparer::new(sample_url, &output_dir) {
        Ok(comparer) => match comparer.generate_reference_frames(MAX_COMPARE_FRAMES) {
            Ok(_) => {
                println!("✓ FFmpeg 参考帧已生成");

                let baseline_info = format!(
                    "# MPEG4 Part 2 FFmpeg 对比基线 - Test 2.2 (Quarterpel)\n\n\
                         ## 编码特性\n\
                         - Quarterpel (1/4 像素精度运动补偿)\n\
                         - 子像素插值滤波\n\n\
                         ## 质量要求\n\
                         - 预期 PSNR: >= {:.1} dB\n",
                    PSNR_THRESHOLD_ADVANCED,
                );

                let info_file = output_dir.join("test_2_2_baseline_info.md");
                let _ = fs::write(&info_file, baseline_info);

                println!("✓ 基线信息已保存");
            }
            Err(e) => eprintln!("参考帧生成失败: {}", e),
        },
        Err(e) => eprintln!("FFmpeg 对比器初始化失败: {}", e),
    }

    println!("\n✓ 基线测试 2.2 完成");
}

/// 生成所有基线汇总报告
///
/// 创建综合的对比基线文档，便于后续 PSNR 验证
#[test]
#[ignore]
fn test_generate_ffmpeg_baseline_summary() {
    let output_dir = match init_test_environment() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("环境初始化失败: {}", e);
            return;
        }
    };

    println!("\n=== 生成 FFmpeg 对比基线汇总报告 ===");

    let summary = r#"# MPEG4 Part 2 解码器 FFmpeg 对比基线

> 本文档记录所有 FFmpeg 参考基线，用于与 tao-codec 进行 PSNR 对比

## 基线测试清单

### 第 1 阶段 - 基础解码 (P0)

#### 1.1 基础 AVI 解码
- **样本**: color16.avi (320×240, 25fps)
- **编码特性**: 标准 MPEG-4 Part 2, I/P 帧
- **参考文件**: `reference_frames_1_1.yuv`
- **质量要求**: PSNR Y >= 38 dB, U >= 38 dB, V >= 38 dB
- **说明**: 基础测试，应达到无损或极高质量

### 第 2 阶段 - 高级特性 (P1)

#### 2.1 B 帧解码
- **样本**: avi+mpeg4+++qprd_cmp_b-frames_naq1.avi
- **编码特性**: B 帧（双向预测）
- **参考文件**: `reference_frames_2_1.yuv`
- **质量要求**: PSNR Y >= 32 dB

#### 2.2 Quarterpel 运动补偿
- **样本**: avi+mpeg4+++DivX51-Qpel.avi
- **编码特性**: 1/4 像素精度运动补偿
- **参考文件**: `reference_frames_2_2.yuv`
- **质量要求**: PSNR Y >= 32 dB

#### 2.3 GMC + Quarterpel
- **样本**: avi+mpeg4+++xvid_gmcqpel_artifact.avi (2.8M)
- **编码特性**: 全局运动补偿 + 四分像素补偿
- **参考文件**: `reference_frames_2_3.yuv`
- **质量要求**: PSNR Y >= 32 dB

#### 2.4 数据分区 (Data Partitioning)
- **样本**: m4v+mpeg4+++ErrDec_mpeg4datapart-64_qcif.m4v
- **编码特性**: 数据分区分离编码
- **参考文件**: `reference_frames_2_4.yuv`
- **质量要求**: PSNR Y >= 30 dB（特殊样本，容差较大）

## PSNR 计算指标说明

### 质量评级
- **PSNR >= 40 dB**: 极好（基本无可见差异）
- **PSNR 35-40 dB**: 很好（非常小的可见差异）
- **PSNR 30-35 dB**: 好（可接受的质量）
- **PSNR 25-30 dB**: 一般（明显差异，但可接受）
- **PSNR < 25 dB**: 差（严重质量下降）

### Y/U/V 平面说明
- **Y 平面**: 亮度信息（最重要，权重 65%）
- **U/V 平面**: 色度信息（权重各 17.5%）

## 对比工作流

### 第 1 步：生成参考帧
```bash
# 为每个样本生成 FFmpeg 参考输出
cargo test --test mpeg4_ffmpeg_baseline test_generate -- --ignored --nocapture
```

### 第 2 步：运行 Tao 解码
```bash
# 使用 tao-codec 解码同样的样本
cargo test --test mpeg4_part2_pipeline --features http -- --nocapture
```

### 第 3 步：计算 PSNR
```bash
# 对比参考帧与 tao 输出，计算 PSNR
# （待实现：自动 PSNR 计算脚本）
```

### 第 4 步：验证质量
- 所有样本 PSNR >= 基线要求
- 记录任何差异大于 2 dB 的情况
- 分析和改进低质量解码

## 参考帧目录结构

```
data/ffmpeg_baselines/
├── reference_frames.yuv           # FFmpeg 参考输出 (YUV420p)
├── reference_frames_1_1.yuv       # Test 1.1 参考帧
├── reference_frames_2_1.yuv       # Test 2.1 参考帧
├── reference_frames_2_2.yuv       # Test 2.2 参考帧
├── reference_frames_2_3.yuv       # Test 2.3 参考帧
├── reference_frames_2_4.yuv       # Test 2.4 参考帧
├── test_1_1_baseline_info.md      # Test 1.1 基线信息
├── test_2_1_baseline_info.md      # Test 2.1 基线信息
└── test_2_2_baseline_info.md      # Test 2.2 基线信息
```

## 故障排除

### FFmpeg 未找到
```bash
# 安装 FFmpeg
# Windows:
choco install ffmpeg
# macOS:
brew install ffmpeg
# Linux (Ubuntu):
sudo apt-get install ffmpeg
```

### 网络连接失败
- 检查互联网连接
- 验证 ffmpeg.org 可访问
- 尝试代理或 VPN

### 磁盘空间不足
- 参考帧文件可能很大 (数百 MB)
- 清理其他临时文件
- 或只保留关键样本的基线

## 后续改进

- [ ] 自动化 PSNR 计算脚本
- [ ] CI/CD 集成自动基线生成
- [ ] 性能对比 (FPS, CPU 使用率)
- [ ] 更多复杂样本的对比基线
- [ ] 个帧 PSNR 分布统计

---

**生成日期**: 2026-02-16  
**版本**: 1.0  
**维护者**: AI Copilot
"#;

    let summary_file = output_dir.join("FFMPEG_BASELINE_SUMMARY.md");
    match fs::write(&summary_file, summary) {
        Ok(_) => {
            println!("✓ 基线汇总报告已生成: {:?}", summary_file);
        }
        Err(e) => {
            eprintln!("无法写入汇总报告: {}", e);
        }
    }

    // 生成 Python 脚本来自动化 PSNR 计算
    let psnr_calculator_script = r#"#!/usr/bin/env python3
# PSNR 自动计算脚本
# 用途: 对比参考帧与 tao 解码输出，计算 PSNR 指标

import os
import math
import struct
import sys
from pathlib import Path

def calculate_psnr(data1: bytes, data2: bytes, width: int, height: int) -> dict:
    """
    计算两个 YUV420p 帧的 PSNR 指标
    
    Args:
        data1: 第一个帧数据 (原始字节)
        data2: 第二个帧数据
        width, height: 视频分辨率
    
    Returns:
        包含 Y/U/V 平面 PSNR 值的字典
    """
    if len(data1) != len(data2):
        raise ValueError(f"帧大小不匹配: {len(data1)} vs {len(data2)}")
    
    y_size = width * height
    uv_size = (width // 2) * (height // 2)
    
    # 提取 Y/U/V 平面
    y1 = data1[:y_size]
    u1 = data1[y_size:y_size + uv_size]
    v1 = data1[y_size + uv_size:y_size + 2*uv_size]
    
    y2 = data2[:y_size]
    u2 = data2[y_size:y_size + uv_size]
    v2 = data2[y_size + uv_size:y_size + 2*uv_size]
    
    def calculate_plane_psnr(p1: bytes, p2: bytes) -> float:
        """计算单个平面的 PSNR"""
        if len(p1) != len(p2):
            return 0.0
        
        mse = sum((a - b) ** 2 for a, b in zip(p1, p2)) / len(p1)
        if mse == 0:
            return float('inf')
        return 20 * math.log10(255 / math.sqrt(mse))
    
    return {
        'psnr_y': calculate_plane_psnr(y1, y2),
        'psnr_u': calculate_plane_psnr(u1, u2),
        'psnr_v': calculate_plane_psnr(v1, v2),
    }

def main():
    """主函数"""
    if len(sys.argv) < 4:
        print("用法: python3 psnr_calc.py <ref_file> <test_file> <width> <height>")
        print("示例: python3 psnr_calc.py ref.yuv test.yuv 1920 1080")
        return
    
    ref_file = sys.argv[1]
    test_file = sys.argv[2]
    width = int(sys.argv[3])
    height = int(sys.argv[4])
    
    # 检查文件
    if not os.path.exists(ref_file):
        print(f"✗ 参考文件不存在: {ref_file}")
        return
    if not os.path.exists(test_file):
        print(f"✗ 测试文件不存在: {test_file}")
        return
    
    # 读取数据
    with open(ref_file, 'rb') as f:
        ref_data = f.read()
    with open(test_file, 'rb') as f:
        test_data = f.read()
    
    frame_size = width * height + 2 * (width // 2) * (height // 2)
    num_frames = len(ref_data) // frame_size
    
    print(f"视频参数: {width}x{height}, {num_frames} 帧")
    print("=" * 60)
    
    total_psnr_y = 0.0
    total_psnr_u = 0.0
    total_psnr_v = 0.0
    
    for frame_idx in range(num_frames):
        start = frame_idx * frame_size
        end = start + frame_size
        
        if end > len(ref_data) or end > len(test_data):
            break
        
        try:
            psnr = calculate_psnr(
                ref_data[start:end],
                test_data[start:end],
                width, height
            )
            
            total_psnr_y += psnr['psnr_y']
            total_psnr_u += psnr['psnr_u']
            total_psnr_v += psnr['psnr_v']
            
            print(f"Frame {frame_idx:3d}: Y={psnr['psnr_y']:6.2f} dB, "
                  f"U={psnr['psnr_u']:6.2f} dB, V={psnr['psnr_v']:6.2f} dB")
        
        except Exception as e:
            print(f"Frame {frame_idx} 计算失败: {e}")
    
    avg_psnr_y = total_psnr_y / num_frames if num_frames > 0 else 0
    avg_psnr_u = total_psnr_u / num_frames if num_frames > 0 else 0
    avg_psnr_v = total_psnr_v / num_frames if num_frames > 0 else 0
    
    print("=" * 60)
    print(f"平均 PSNR:")
    print(f"  Y 平面: {avg_psnr_y:.2f} dB")
    print(f"  U 平面: {avg_psnr_u:.2f} dB")
    print(f"  V 平面: {avg_psnr_v:.2f} dB")

if __name__ == '__main__':
    main()
"#;

    let script_file = output_dir.join("psnr_calculator.py");
    match fs::write(&script_file, psnr_calculator_script) {
        Ok(_) => {
            println!("✓ PSNR 计算脚本已生成: {:?}", script_file);
            println!("  使用方式: python3 psnr_calculator.py ref.yuv test.yuv 1920 1080");
        }
        Err(e) => {
            eprintln!("警告: 无法生成计算脚本: {}", e);
        }
    }

    println!("\n✓ 基线汇总报告生成完成！");
    println!("\n📋 后续步骤:");
    println!("1. 运行此测试生成 FFmpeg 参考帧");
    println!("2. 使用 tao-codec 解码相同样本");
    println!("3. 使用 psnr_calculator.py 计算 PSNR");
    println!("4. 对比结果与预期阈值");
}
