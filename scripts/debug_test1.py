#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# 调试 Test 1 MPEG4 的 YUV 输出问题

import os
import sys
import subprocess
import struct
from pathlib import Path

# 处理 Windows 编码问题
if sys.platform == "win32":
    import io

    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8")


def download_avi(url, output_path):
    """下载 AVI 文件"""
    print(f"下载: {url}")
    try:
        import urllib.request

        urllib.request.urlretrieve(url, output_path)
        size_mb = os.path.getsize(output_path) / (1024 * 1024)
        print(f"✓ 下载完成: {size_mb:.1f} MB")
        return True
    except Exception as e:
        print(f"✗ 下载失败: {e}")
        return False


def run_ffmpeg_baseline(avi_path, output_yuv):
    """使用 FFmpeg 生成基线"""
    print(f"\n运行 FFmpeg 基线生成")
    cmd = [
        "ffmpeg",
        "-i",
        avi_path,
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-vframes",
        "10",  # 只取前 10 帧
        "-loglevel",
        "error",
        "-y",
        output_yuv,
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        if result.returncode == 0:
            size_mb = os.path.getsize(output_yuv) / (1024 * 1024)
            print(f"✓ FFmpeg 基线完成: {size_mb:.1f} MB")
            return True
        else:
            print(f"✗ FFmpeg 失败: {result.stderr}")
            return False
    except Exception as e:
        print(f"✗ 异常: {e}")
        return False


def run_tao_decode(avi_path, output_yuv):
    """运行 tao-cli 解码"""
    print(f"\n运行 tao-cli 解码")
    cmd = [
        "cargo",
        "run",
        "--release",
        "-p",
        "tao-cli",
        "--bin",
        "tao",
        "--",
        "-i",
        avi_path,
        "--output-raw",
        output_yuv,
        "-y",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, timeout=120)
        # 处理中文编码
        stderr_text = result.stderr.decode("utf-8", errors="ignore")

        if result.returncode == 0:
            # 查找关键日志
            for line in stderr_text.split("\n"):
                if "已处理" in line or "完成" in line or "帧数" in line:
                    print(f"  {line.strip()}")

            size_mb = os.path.getsize(output_yuv) / (1024 * 1024)
            actual_frames = os.path.getsize(output_yuv) / (312 * 240 * 1.5)

            print(f"✓ tao-cli 完成: {size_mb:.1f} MB ({actual_frames:.0f} 帧)")
            return True
        else:
            print(f"✗ tao-cli 失败:")
            print(stderr_text[-500:])
            return False
    except Exception as e:
        print(f"✗ 异常: {e}")
        return False


def analyze_yuv_frame(filename, width, height, frame_idx=0):
    """分析 YUV420p 帧的像素统计"""
    frame_size = int(width * height * 1.5)
    print(f"\n分析第 {frame_idx} 帧 (大小: {frame_size} 字节)")

    try:
        with open(filename, "rb") as f:
            f.seek(frame_idx * frame_size)
            y_plane = f.read(width * height)
            u_plane = f.read(width * height // 4)
            v_plane = f.read(width * height // 4)

        if len(y_plane) < width * height:
            print(f"✗ 文件过短，无法读取完整帧")
            return False

        y_values = list(y_plane)
        u_values = list(u_plane)
        v_values = list(v_plane)

        import statistics

        print(f"  Y 平面 ({len(y_values)} 字节):")
        print(f"    最小/最大: {min(y_values):3d}/{max(y_values):3d}")
        print(f"    平均值: {statistics.mean(y_values):.0f}")
        print(
            f"    标准差: {statistics.stdev(y_values) if len(y_values) > 1 else 0:.0f}"
        )
        print(f"    唯一值数: {len(set(y_values))}")

        print(f"  U 平面 ({len(u_values)} 字节):")
        print(f"    最小/最大: {min(u_values):3d}/{max(u_values):3d}")
        print(f"    平均值: {statistics.mean(u_values):.0f}")

        print(f"  V 平面 ({len(v_values)} 字节):")
        print(f"    最小/最大: {min(v_values):3d}/{max(v_values):3d}")
        print(f"    平均值: {statistics.mean(v_values):.0f}")

        # 检查异常
        issues = []
        if len(set(y_values)) < 5:
            issues.append("Y 平面值太单调 (可能全黑或全白)")
        if max(y_values) <= 16:
            issues.append("Y 平面值范围太小 (<= 16)")
        if min(y_values) >= 240:
            issues.append("Y 平面值范围太大 (>= 240)")

        if issues:
            print(f"  ⚠️ 警告:")
            for issue in issues:
                print(f"    - {issue}")
            return False
        else:
            print(f"  ✓ 框架看起来正常")
            return True

    except Exception as e:
        print(f"✗ 分析失败: {e}")
        return False


def main():
    # 配置
    work_dir = "data/debug_test1"
    avi_url = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi"

    os.makedirs(work_dir, exist_ok=True)

    avi_path = os.path.join(work_dir, "color16.avi")
    ffmpeg_yuv = os.path.join(work_dir, "ref_color16.yuv")
    tao_yuv = os.path.join(work_dir, "tao_color16.yuv")

    print("=" * 70)
    print("MPEG4 Part 2 Test 1 调试: color16.avi")
    print("=" * 70)

    # 1. 下载
    if not os.path.exists(avi_path):
        if not download_avi(avi_url, avi_path):
            sys.exit(1)
    else:
        print(f"✓ 输入文件存在: {avi_path}")

    # 2. FFmpeg 基线
    if not os.path.exists(ffmpeg_yuv):
        if not run_ffmpeg_baseline(avi_path, ffmpeg_yuv):
            sys.exit(1)
    else:
        size_mb = os.path.getsize(ffmpeg_yuv) / (1024 * 1024)
        print(f"✓ FFmpeg 基线已存在: {size_mb:.1f} MB")

    # 3. tao-cli 解码
    if os.path.exists(tao_yuv):
        os.remove(tao_yuv)

    if not run_tao_decode(avi_path, tao_yuv):
        sys.exit(1)

    # 4. 对比文件大小
    ffmpeg_size = os.path.getsize(ffmpeg_yuv)
    tao_size = os.path.getsize(tao_yuv)
    print(f"\n文件大小对比:")
    print(f"  FFmpeg: {ffmpeg_size:10d} 字节 ({ffmpeg_size / 1e6:.1f} MB)")
    print(f"  tao:    {tao_size:10d} 字节 ({tao_size / 1e6:.1f} MB)")
    print(f"  比例:   {tao_size / ffmpeg_size:.2f}x")

    if abs(tao_size - ffmpeg_size) > ffmpeg_size * 0.1:
        print(f"  ⚠️ 文件大小差异超过 10%")

    # 5. 分析帧数据
    print(f"\n=" * 70)
    print("FFmpeg 基线帧分析:")
    print("=" * 70)
    analyze_yuv_frame(ffmpeg_yuv, 312, 240, 0)

    print(f"\n=" * 70)
    print("tao-codec 输出帧分析:")
    print("=" * 70)
    analyze_yuv_frame(tao_yuv, 312, 240, 0)

    # 6. 逐字节对比前 256 字节
    print(f"\n=" * 70)
    print("前 256 字节对比:")
    print("=" * 70)

    with open(ffmpeg_yuv, "rb") as f:
        ffmpeg_head = f.read(256)
    with open(tao_yuv, "rb") as f:
        tao_head = f.read(256)

    diff_count = sum(1 for a, b in zip(ffmpeg_head, tao_head) if a != b)
    print(f"  不同的字节数: {diff_count} / {len(ffmpeg_head)}")

    if diff_count < 10:
        print(f"  ✓ 前 256 字节基本一致")
    else:
        print(f"  ⚠️ 数据差异较大")
        print(
            f"\n  FFmpeg 前 32 字节: {' '.join(f'{b:02x}' for b in ffmpeg_head[:32])}"
        )
        print(f"  tao    前 32 字节: {' '.join(f'{b:02x}' for b in tao_head[:32])}")


if __name__ == "__main__":
    main()
