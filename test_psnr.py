#!/usr/bin/env python3
"""简化的 PSNR 验证脚本"""

import subprocess

def extract_yuv_ffmpeg(url, output_file, num_frames=10):
    """使用 FFmpeg 提取 YUV"""
    cmd = [
        "ffmpeg", "-i", url,
        "-pix_fmt", "yuv420p",
        "-vframes", str(num_frames),
        "-f", "rawvideo",
        output_file,
        "-y"
    ]
    try:
        result = subprocess.run(cmd, stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL, timeout=60)
        return result.returncode == 0
    except:
        return False

def extract_yuv_tao(url, output_file):
    """使用 tao 提取 YUV"""
    cmd = [
        ".\\target\\release\\tao.exe", "-i", url,
        "--output-raw", output_file,
        "-y"
    ]
    try:
        result = subprocess.run(cmd, stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL, timeout=60)
        return result.returncode == 0
    except:
        return False

def main():
    url = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi"
    ffmpeg_yuv = "data/ffmpeg_ref.yuv"
    tao_yuv = "data/test1.yuv"
    
    w, h = 712, 368
    frame_size = w * h * 3 // 2  # YUV420p
    expected_size = frame_size * 10
    
    print("提取 10 帧 YUV 数据...")
    
    # FFmpeg reference
    print("  FFmpeg...", end="", flush=True)
    if not extract_yuv_ffmpeg(url, ffmpeg_yuv):
        print(" 失败")
        return
    print(" 成功")
    
    # Tao
    print("  Tao...", end="", flush=True)
    if not extract_yuv_tao(url, tao_yuv):
        print(" 失败")
        return
    print(" 成功")
    
    # 验证文件
    print("\n文件大小:", end="")
    with open(ffmpeg_yuv, "rb") as f:
        ffmpeg_size = len(f.read())
    with open(tao_yuv, "rb") as f:
        tao_size = len(f.read())
    print(f" FFmpeg={ffmpeg_size}, Tao={tao_size} (预期 {expected_size})")
    
    if ffmpeg_size == tao_size:
        print("✓ 文件大小匹配")
    else:
        print("✗ 文件大小不匹配")
    
    # 比较前32字节
    print("\n前 32 字节比较:")
    with open(ffmpeg_yuv, "rb") as f:
        ffmpeg_head = f.read(32)
    with open(tao_yuv, "rb") as f:
        tao_head = f.read(32)
    
    ffmpeg_hex = " ".join(f"{b:02x}" for b in ffmpeg_head)
    tao_hex = " ".join(f"{b:02x}" for b in tao_head)
    
    print(f"  FFmpeg: {ffmpeg_hex}")
    print(f"  Tao:    {tao_hex}")
    
    # 计算简单的差异率
    diff_count = sum(1 for i in range(min(len(ffmpeg_head), len(tao_head))) if ffmpeg_head[i] != tao_head[i])
    print(f"  不同字节数: {diff_count}/32")

if __name__ == "__main__":
    main()
