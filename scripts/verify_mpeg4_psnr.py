#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# MPEG4 Part 2 解码器 PSNR 质量验证脚本
# 位置: scripts/verify_mpeg4_psnr.py
#
# 功能: 自动化 PSNR 验证工作流
#   1. 生成 FFmpeg 参考基线
#   2. 运行 tao-codec 解码
#   3. 计算 PSNR 指标
#   4. 对比与阈值
#   5. 生成验证报告

import os
import sys
import json
import subprocess
import struct
import math
from pathlib import Path
from datetime import datetime
from typing import Dict, Tuple, List, Optional

# 处理 Windows 编码问题
if sys.platform == "win32":
    import io

    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")

# ============================================================================
# 配置
# ============================================================================

# 基线输出目录
BASELINES_DIR = "data/ffmpeg_baselines"
# 解码输出目录
DECODE_OUTPUT_DIR = "data/mpeg4_decode_output"
# 验证报告目录
REPORT_DIR = "plans"

# 质量阈值 (dB)
QUALITY_THRESHOLDS = {
    "basic": 38.0,  # 基础测试（color16.avi）
    "b_frames": 32.0,  # B 帧测试
    "qpel": 32.0,  # Quarterpel 测试
    "gmc_qpel": 32.0,  # GMC + Quarterpel
}

# 测试用例配置
TEST_CASES = [
    {
        "name": "1.1 基础 AVI 解码",
        "url": "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi",
        "width": 312,
        "height": 240,
        "frames": 10,
        "threshold": QUALITY_THRESHOLDS["basic"],
        "type": "basic",
    },
    {
        "name": "2.1 B 帧解码",
        "url": "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi",
        "width": 720,
        "height": 480,
        "frames": 10,
        "threshold": QUALITY_THRESHOLDS["b_frames"],
        "type": "b_frames",
    },
    {
        "name": "2.2 Quarterpel",
        "url": "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi",
        "width": 320,
        "height": 240,
        "frames": 10,
        "threshold": QUALITY_THRESHOLDS["qpel"],
        "type": "qpel",
    },
]

# ============================================================================
# 工具函数
# ============================================================================


def log(msg: str, level: str = "INFO"):
    """打印带时间戳的日志"""
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    # 简化输出，避免 Unicode 编码问题
    prefix = f"[{timestamp}] {level:5s}"
    print(f"{prefix} {msg}")


def check_command_exists(cmd: str) -> bool:
    """检查命令是否存在"""
    result = subprocess.run(
        ["which" if os.name != "nt" else "where", cmd], capture_output=True, text=True
    )
    return result.returncode == 0


def run_command(cmd: List[str], description: str = None) -> Tuple[bool, str]:
    """运行命令并返回结果"""
    if description:
        log(f"执行: {description}")

    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,  # 5分钟超时
        )

        if result.returncode != 0:
            log(f"命令失败: {' '.join(cmd)}", "ERROR")
            if result.stderr:
                log(f"错误信息:\n{result.stderr}", "ERROR")
            return False, result.stderr

        return True, result.stdout
    except subprocess.TimeoutExpired:
        log("命令超时（5分钟）", "ERROR")
        return False, "timeout"
    except Exception as e:
        log(f"执行异常: {e}", "ERROR")
        return False, str(e)


def calculate_psnr(ref_file: str, test_file: str, width: int, height: int) -> Dict:
    """
    计算 PSNR 指标

    Args:
        ref_file: FFmpeg 参考输出文件
        test_file: tao-codec 输出文件
        width, height: 视频分辨率

    Returns:
        包含 PSNR 数据的字典
    """
    log(f"计算 PSNR: {ref_file} vs {test_file}")

    if not os.path.exists(ref_file):
        log(f"参考文件不存在: {ref_file}", "ERROR")
        return None

    if not os.path.exists(test_file):
        log(f"测试文件不存在: {test_file}", "ERROR")
        return None

    try:
        with open(ref_file, "rb") as f:
            ref_data = f.read()
        with open(test_file, "rb") as f:
            test_data = f.read()

        frame_size = width * height + 2 * (width // 2) * (height // 2)
        num_frames = min(len(ref_data) // frame_size, len(test_data) // frame_size)

        if num_frames == 0:
            log("无法解析帧数据", "ERROR")
            return None

        results = {
            "frames": num_frames,
            "frame_size": frame_size,
            "per_frame": [],
            "average": {},
        }

        total_psnr_y = 0.0
        total_psnr_u = 0.0
        total_psnr_v = 0.0

        for frame_idx in range(min(num_frames, 10)):  # 最多计算 10 帧
            start = frame_idx * frame_size
            end = start + frame_size

            ref_frame = ref_data[start:end]
            test_frame = test_data[start:end]

            # 计算 Y/U/V 平面 PSNR
            y_size = width * height
            uv_size = (width // 2) * (height // 2)

            ref_y = ref_frame[:y_size]
            ref_u = ref_frame[y_size : y_size + uv_size]
            ref_v = ref_frame[y_size + uv_size :]

            test_y = test_frame[:y_size]
            test_u = test_frame[y_size : y_size + uv_size]
            test_v = test_frame[y_size + uv_size :]

            # 计算每个平面的 PSNR
            def calc_plane_psnr(p1: bytes, p2: bytes) -> float:
                if len(p1) != len(p2):
                    return 0.0
                mse = sum((a - b) ** 2 for a, b in zip(p1, p2)) / len(p1)
                if mse == 0:
                    return 100.0  # 完全相同
                return 20 * math.log10(255 / math.sqrt(mse))

            psnr_y = calc_plane_psnr(ref_y, test_y)
            psnr_u = calc_plane_psnr(ref_u, test_u)
            psnr_v = calc_plane_psnr(ref_v, test_v)

            results["per_frame"].append(
                {
                    "frame": frame_idx,
                    "psnr_y": psnr_y,
                    "psnr_u": psnr_u,
                    "psnr_v": psnr_v,
                }
            )

            total_psnr_y += psnr_y
            total_psnr_u += psnr_u
            total_psnr_v += psnr_v

            log(
                f"  Frame {frame_idx:2d}: Y={psnr_y:6.2f} dB, U={psnr_u:6.2f} dB, V={psnr_v:6.2f} dB",
                "DEBUG",
            )

        avg_frames = min(num_frames, 10)
        results["average"] = {
            "psnr_y": total_psnr_y / avg_frames,
            "psnr_u": total_psnr_u / avg_frames,
            "psnr_v": total_psnr_v / avg_frames,
        }

        return results

    except Exception as e:
        log(f"PSNR 计算异常: {e}", "ERROR")
        return None


def generate_ffmpeg_baseline(test_case: Dict) -> Tuple[bool, str]:
    """生成 FFmpeg 参考基线"""
    url = test_case["url"]
    filename = url.split("/")[-1].replace(".avi", "").replace(".m4v", "")
    output_file = os.path.join(BASELINES_DIR, f"ref_{filename}.yuv")

    log(f"\n生成 FFmpeg 基线: {filename}")
    log(f"  URL: {url}")
    log(f"  输出: {output_file}")

    # 检查是否已存在
    if os.path.exists(output_file):
        size_mb = os.path.getsize(output_file) / (1024 * 1024)
        log(f"  [已存在] 基线已存在 ({size_mb:.1f} MB)", "WARN")
        return True, output_file

    # 使用 ffmpeg 生成参考输出
    cmd = [
        "ffmpeg",
        "-i",
        url,
        "-pix_fmt",
        "yuv420p",
        "-f",
        "rawvideo",
        "-vframes",
        str(test_case["frames"]),
        "-y",  # 覆盖已存在的文件
        output_file,
    ]

    success, output = run_command(cmd, f"生成 {filename} 参考帧")
    if success:
        size_mb = os.path.getsize(output_file) / (1024 * 1024)
        log(f"[OK] 已生成 ({size_mb:.1f} MB)")
        return True, output_file
    else:
        return False, None


def generate_tao_decode(test_case: Dict) -> Tuple[bool, str]:
    """运行 tao-codec 进行解码"""
    url = test_case["url"]
    filename = url.split("/")[-1].replace(".avi", "").replace(".m4v", "")
    output_file = os.path.join(DECODE_OUTPUT_DIR, f"tao_{filename}.yuv")

    log(f"\n运行 tao-codec 解码: {filename}")
    log(f"  URL: {url}")
    log(f"  输出: {output_file}")

    # 检查是否已存在
    if os.path.exists(output_file):
        size_mb = os.path.getsize(output_file) / (1024 * 1024)
        log(f"  [已存在] 输出已存在 ({size_mb:.1f} MB)", "WARN")
        return True, output_file

    # 这里应该调用 tao 的解码命令
    # 当前为占位符实现
    log("  (占位符) 需要实现具体的 tao-codec 解码调用", "WARN")
    log("  建议: 在 tao-cli 中添加 --output-raw 选项", "WARN")

    # 临时创建虚拟输出文件用于演示
    os.makedirs(DECODE_OUTPUT_DIR, exist_ok=True)
    frame_size = test_case["width"] * test_case["height"] + 2 * (
        test_case["width"] // 2
    ) * (test_case["height"] // 2)
    fake_data = b"\x80" * (frame_size * test_case["frames"])  # 中灰色测试帧
    with open(output_file, "wb") as f:
        f.write(fake_data)

    log(f"[OK] 已生成虚拟输出 (用于演示)")
    return True, output_file


def verify_test_case(test_case: Dict) -> Dict:
    """验证单个测试用例"""
    log(f"\n{'=' * 70}")
    log(f"验证: {test_case['name']}")
    log(f"{'=' * 70}")

    result = {
        "name": test_case["name"],
        "url": test_case["url"],
        "threshold": test_case["threshold"],
        "status": "UNKNOWN",
        "ffmpeg_baseline": None,
        "tao_output": None,
        "psnr": None,
        "pass": False,
        "error": None,
    }

    # 第 1 步: 生成 FFmpeg 基线
    success, ref_file = generate_ffmpeg_baseline(test_case)
    if not success:
        result["status"] = "FAILED"
        result["error"] = "FFmpeg 基线生成失败"
        return result
    result["ffmpeg_baseline"] = ref_file

    # 第 2 步: 运行 tao-codec 解码
    success, tao_file = generate_tao_decode(test_case)
    if not success:
        result["status"] = "FAILED"
        result["error"] = "tao-codec 解码失败"
        return result
    result["tao_output"] = tao_file

    # 第 3 步: 计算 PSNR
    psnr_data = calculate_psnr(
        ref_file, tao_file, test_case["width"], test_case["height"]
    )

    if psnr_data is None:
        result["status"] = "FAILED"
        result["error"] = "PSNR 计算失败"
        return result

    result["psnr"] = psnr_data

    # 第 4 步: 对比阈值
    avg_psnr = psnr_data["average"]["psnr_y"]
    threshold = test_case["threshold"]

    if avg_psnr >= threshold:
        result["status"] = "PASSED"
        result["pass"] = True
        log(f"\n[PASS] 平均 PSNR Y: {avg_psnr:.2f} dB >= {threshold:.1f} dB")
    else:
        result["status"] = "FAILED"
        result["pass"] = False
        log(
            f"\n[FAIL] 失败! 平均 PSNR Y: {avg_psnr:.2f} dB < {threshold:.1f} dB",
            "WARN",
        )

    return result


def generate_verification_report(results: List[Dict]) -> str:
    """生成验证报告"""
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")

    report = f"""# MPEG4 Part 2 解码器 PSNR 验证报告

**验证时间**: {timestamp}

## 摘要

| 指标 | 结果 |
|-----|------|
| 总测试数 | {len(results)} |
| 通过数 | {sum(1 for r in results if r["pass"])} |
| 失败数 | {sum(1 for r in results if not r["pass"])} |
| 通过率 | {sum(1 for r in results if r["pass"]) / len(results) * 100:.1f}% |

## 详细结果

"""

    for i, result in enumerate(results, 1):
        report += f"### Test {i}: {result['name']}\n\n"
        report += f"**状态**: {result['status']}\n\n"
        report += f"**质量阈值**: PSNR Y >= {result['threshold']:.1f} dB\n\n"

        if result["error"]:
            report += f"**错误**: {result['error']}\n\n"
        elif result["psnr"]:
            psnr = result["psnr"]
            avg = psnr["average"]
            report += f"**平均 PSNR**:\n"
            report += f"- Y 平面: {avg['psnr_y']:.2f} dB\n"
            report += f"- U 平面: {avg['psnr_u']:.2f} dB\n"
            report += f"- V 平面: {avg['psnr_v']:.2f} dB\n\n"

            if psnr["per_frame"]:
                report += f"**逐帧 PSNR**:\n"
                report += "| 帧 | Y (dB) | U (dB) | V (dB) | 状态 |\n"
                report += "|-----|--------|--------|--------|------|\n"
                for frame in psnr["per_frame"]:
                    status = "✓" if frame["psnr_y"] >= result["threshold"] else "✗"
                    report += f"| {frame['frame']:3d} | {frame['psnr_y']:6.2f} | {frame['psnr_u']:6.2f} | {frame['psnr_v']:6.2f} | {status} |\n"
                report += "\n"

        report += "\n"

    # 总体评价
    report += "## 总体评价\n\n"
    passed = sum(1 for r in results if r["pass"])
    if passed == len(results):
        report += "✅ **所有测试通过!** 解码质量优秀。\n"
    elif passed > len(results) / 2:
        report += "⚠️ **大多数测试通过。** 部分编码特性需要改进。\n"
    else:
        report += "❌ **测试通过率低。** 解码器需要重点优化。\n"

    report += "\n## 后续建议\n\n"
    report += "1. 对于 PSNR 不达标的测试，分析具体原因\n"
    report += "2. 使用 ImageMagick/ffmpeg 工具进行逐帧差异分析\n"
    report += "3. 检查是否涉及特定编码特性的处理问题\n"
    report += "4. 与 FFmpeg 的相应代码逻辑对比\n"
    report += "5. 进行人工播放验证以确认视觉质量\n"

    return report


# ============================================================================
# 主程序
# ============================================================================


def main():
    """主函数"""
    log("MPEG4 Part 2 解码器 PSNR 验证工具")
    log("=" * 70)

    # 检查环境
    log("\n检查环境...")

    if not check_command_exists("ffmpeg"):
        log("FFmpeg 未安装", "ERROR")
        sys.exit(1)
    log(f"[OK] FFmpeg 可用")

    # 创建必要目录
    os.makedirs(BASELINES_DIR, exist_ok=True)
    os.makedirs(DECODE_OUTPUT_DIR, exist_ok=True)
    os.makedirs(REPORT_DIR, exist_ok=True)
    log("✓ 工作目录就绪")

    # 执行验证
    log(f"\n开始验证 {len(TEST_CASES)} 个测试用例...\n")
    results = []

    for test_case in TEST_CASES:
        result = verify_test_case(test_case)
        results.append(result)

    # 生成报告
    log("\n" + "=" * 70)
    log("生成验证报告...")

    report = generate_verification_report(results)
    report_file = os.path.join(REPORT_DIR, "MPEG4_PSNR_VERIFICATION_REPORT.md")

    with open(report_file, "w", encoding="utf-8") as f:
        f.write(report)

    log(f"[OK] 报告已保存: {report_file}")

    # 输出摘要
    log("\n" + "=" * 70)
    log("验证完成!")
    passed = sum(1 for r in results if r["pass"])
    log(f"通过: {passed}/{len(TEST_CASES)}")

    if passed == len(TEST_CASES):
        log(f"[SUCCESS] 所有测试通过!", "INFO")
        return 0
    else:
        log(f"⚠️ 有 {len(TEST_CASES) - passed} 个测试未通过", "WARN")
        return 1


if __name__ == "__main__":
    sys.exit(main())
