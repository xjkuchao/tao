#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
调试 color16.avi CBPY 解码问题
对比 FFmpeg 和 Tao 的解码结果
"""

import os
import sys
import subprocess
import tempfile
from pathlib import Path


def download_sample(url, output_path):
    """下载测试样本"""
    import urllib.request

    print(f"下载样本: {url}")
    urllib.request.urlretrieve(url, output_path)
    print(f"✓ 下载完成: {output_path}")


def run_ffmpeg_decode(input_file, output_yuv):
    """使用 FFmpeg 解码生成 YUV420p 基准"""
    cmd = [
        "ffmpeg",
        "-y",
        "-i",
        input_file,
        "-vframes",
        "1",  # 只解码第一帧
        "-pix_fmt",
        "yuv420p",
        "-f",
        "rawvideo",
        output_yuv,
    ]
    print(f"运行 FFmpeg: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"FFmpeg 错误: {result.stderr}")
        return False
    print(f"✓ FFmpeg 解码成功: {output_yuv}")
    return True


def run_tao_decode(input_file, output_yuv):
    """使用 Tao 解码生成 YUV420p"""
    tao_cli = Path(__file__).parent.parent / "target" / "release" / "tao"

    # 先构建 tao-cli
    print("构建 tao-cli...")
    subprocess.run(
        ["cargo", "build", "--release", "-p", "tao-cli"],
        cwd=Path(__file__).parent.parent,
        check=True,
    )

    cmd = [
        str(tao_cli),
        "-i",
        input_file,
        "-vframes",
        "1",  # 只解码第一帧
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        output_yuv,
    ]
    print(f"运行 Tao: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    print(result.stdout)
    print(result.stderr)

    if result.returncode != 0:
        print(f"Tao 错误: {result.stderr}")
        return False
    print(f"✓ Tao 解码成功: {output_yuv}")
    return True


def analyze_yuv_diff(ffmpeg_yuv, tao_yuv, width, height):
    """分析两个 YUV 文件的差异"""
    with open(ffmpeg_yuv, "rb") as f1, open(tao_yuv, "rb") as f2:
        data1 = f1.read()
        data2 = f2.read()

    if len(data1) != len(data2):
        print(f"❌ 文件大小不匹配: FFmpeg={len(data1)}, Tao={len(data2)}")
        return

    # YUV420p: Y 平面是 width*height, U/V 各是 (width/2)*(height/2)
    y_size = width * height
    uv_size = (width // 2) * (height // 2)

    # 分析 Y 平面
    diff_count_y = sum(1 for i in range(y_size) if data1[i] != data2[i])

    # 分析 U 平面
    u_offset = y_size
    diff_count_u = sum(
        1 for i in range(uv_size) if data1[u_offset + i] != data2[u_offset + i]
    )

    # 分析 V 平面
    v_offset = y_size + uv_size
    diff_count_v = sum(
        1 for i in range(uv_size) if data1[v_offset + i] != data2[v_offset + i]
    )

    print(f"\n=== YUV 差异分析 ===")
    print(f"Y 平面差异: {diff_count_y}/{y_size} ({diff_count_y * 100.0 / y_size:.2f}%)")
    print(
        f"U 平面差异: {diff_count_u}/{uv_size} ({diff_count_u * 100.0 / uv_size:.2f}%)"
    )
    print(
        f"V 平面差异: {diff_count_v}/{uv_size} ({diff_count_v * 100.0 / uv_size:.2f}%)"
    )

    # 显示前 10 个差异
    print(f"\n前 10 个 Y 平面差异:")
    count = 0
    for i in range(y_size):
        if data1[i] != data2[i]:
            y = i // width
            x = i % width
            print(
                f"  位置 ({x:3d}, {y:3d}): FFmpeg={data1[i]:3d}, Tao={data2[i]:3d}, diff={int(data1[i]) - int(data2[i]):4d}"
            )
            count += 1
            if count >= 10:
                break

    # 检查是否完全匹配
    if data1 == data2:
        print("\n✅ 两个文件完全匹配!")
    else:
        print(f"\n❌ 两个文件不匹配")


def analyze_ffmpeg_bitstream(input_file):
    """使用 FFmpeg 的调试模式分析位流"""
    cmd = [
        "ffmpeg",
        "-debug",
        "mb_type",
        "-i",
        input_file,
        "-vframes",
        "1",
        "-f",
        "null",
        "-",
    ]
    print(f"\n运行 FFmpeg 调试模式...")
    result = subprocess.run(cmd, capture_output=True, text=True)

    # 保存调试输出
    debug_file = "/tmp/ffmpeg_debug.txt"
    with open(debug_file, "w") as f:
        f.write(result.stderr)
    print(f"✓ FFmpeg 调试输出保存到: {debug_file}")

    # 提取关键信息
    lines = result.stderr.split("\n")
    for line in lines[:50]:  # 只显示前 50 行
        if (
            "cbpy" in line.lower()
            or "cbpc" in line.lower()
            or "mb_type" in line.lower()
        ):
            print(line)


def main():
    sample_url = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi"

    with tempfile.TemporaryDirectory() as tmpdir:
        tmpdir = Path(tmpdir)

        # 下载样本
        sample_file = tmpdir / "color16.avi"
        download_sample(sample_url, str(sample_file))

        # FFmpeg 解码
        ffmpeg_yuv = tmpdir / "ffmpeg_frame0.yuv"
        if not run_ffmpeg_decode(str(sample_file), str(ffmpeg_yuv)):
            print("❌ FFmpeg 解码失败")
            return

        # Tao 解码
        tao_yuv = tmpdir / "tao_frame0.yuv"
        if not run_tao_decode(str(sample_file), str(tao_yuv)):
            print("❌ Tao 解码失败")
            # 继续分析，即使 Tao 失败也可能生成了部分数据

        # 对比结果
        width, height = 320, 240  # color16.avi 的分辨率
        if ffmpeg_yuv.exists() and tao_yuv.exists():
            analyze_yuv_diff(str(ffmpeg_yuv), str(tao_yuv), width, height)

        # FFmpeg 位流分析
        analyze_ffmpeg_bitstream(str(sample_file))


if __name__ == "__main__":
    main()
