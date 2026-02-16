# Tao 项目测试样本自动下载脚本 v3
# 基于 https://samples.ffmpeg.org/ 官方样本库

# 启用严格模式
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# 样本库基础 URL
$BASE_URL = "https://samples.ffmpeg.org"

# 创建目录结构
function New-SampleDirectory {
    param([string]$Path)
    if (-not (Test-Path $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
        Write-Host "✓ 创建目录: $Path" -ForegroundColor Green
    }
}

# 下载单个文件
function Download-Sample {
    param(
        [string]$Url,
        [string]$OutputPath,
        [switch]$Force
    )
    
    if ((Test-Path $OutputPath) -and -not $Force) {
        Write-Host "⊘ 跳过 (已存在): $OutputPath" -ForegroundColor Yellow
        return $true
    }
    
    try {
        Write-Host "⬇ 下载: $Url" -ForegroundColor Cyan
        $fullUrl = "$BASE_URL/$Url"
        Invoke-WebRequest -Uri $fullUrl -OutFile $OutputPath -UseBasicParsing
        
        $size = (Get-Item $OutputPath).Length
        $sizeKB = [math]::Round($size / 1KB, 1)
        Write-Host "✓ 完成: $OutputPath ($sizeKB KB)" -ForegroundColor Green
        return $true
    }
    catch {
        Write-Host "✗ 失败: $Url - $($_.Exception.Message)" -ForegroundColor Red
        if (Test-Path $OutputPath) {
            Remove-Item $OutputPath -Force
        }
        return $false
    }
}

# 样本定义 (P0 优先级 - 必须下载)
# 基于实际存在的文件路径
$P0_SAMPLES = @{
    # H.264 视频样本
    "video/h264/channel9_hd.ts" = "HDTV/channel9hdtv_ac3.ts"
    "video/h264/h264_mkv.mkv" = "Matroska/haruhi.mkv"
    "video/h264/h264_mp4.mp4" = "mov/mp4/Fraunhofer__a_driving_force_in_innovation__small.mp4"
    
    # MPEG4 Part 2 样本 (使用已验证路径)
    "video/mpeg4/mpeg4_avi.avi" = "avi/2-audio-streams.avi"
    
    # Theora 样本
    "video/theora/theora.ogg" = "ogg/Theora/theora.ogg"
    "video/theora/theora_test.ogg" = "V-codecs/Theora/ED-small-sample-file.ogg"
    
    # AAC 音频样本 (使用实际存在的路径)
    "audio/aac/aac_in_mov.mov" = "mov/aacaudio.mov"
    
    # MP3 音频样本 (使用 MP3 container)
    "audio/mp3/mp3_sample.mp3" = "A-codecs/MP3/CBR/sample.mp3"
    
    # FLAC 音频样本
    "audio/flac/flac_yesterday.flac" = "flac/Yesterday.flac"
    
    # Vorbis 音频样本
    "audio/vorbis/vorbis_test.ogg" = "ogg/Vorbis/test6.ogg"
    "audio/vorbis/vorbis_coyote.ogg" = "ogg/Vorbis/coyote.ogg"
    
    # PCM/WAV 样本
    "audio/pcm/wav_8khz_16bit_mono.wav" = "A-codecs/wavpcm/8khz-16bit-mono.wav"
    "audio/pcm/wav_96khz.wav" = "A-codecs/wavpcm/test-96.wav"
    
    # MP4 容器样本
    "container/mp4/mp4_h264.mp4" = "mov/mp4/Fraunhofer__a_driving_force_in_innovation__small.mp4"
    
    # MKV 容器样本
    "container/mkv/mkv_h264.mkv" = "Matroska/haruhi.mkv"
    
    # AVI 容器样本
    "container/avi/avi_dual_audio.avi" = "avi/2-audio-streams.avi"
    
    # FLV 容器样本
    "container/flv/flv_vp6.flv" = "FLV/flash8/artifacts-vp6.flv"
    
    # MPEG-TS 容器样本
    "container/mpegts/ts_h264_ac3.ts" = "HDTV/channel9hdtv_ac3.ts"
    
    # Ogg 容器样本
    "container/ogg/ogg_theora_vorbis.ogg" = "ogg/Theora/theora.ogg"
    "container/ogg/ogg_vorbis_only.ogg" = "ogg/Vorbis/test6.ogg"
    
    # AIFF 样本
    "container/aiff/aiff_dragon.aif" = "AIFF/dragon.aif"
    
    # WAV 样本
    "container/wav/wav_8khz_mono.wav" = "A-codecs/wavpcm/8khz-16bit-mono.wav"
}

# P1 样本 (推荐下载 - 更多格式和边界测试)
$P1_SAMPLES = @{
    # H.265/HEVC 样本 (注: FFmpeg 样本库中 HEVC 样本较少)
    # 暂时留空，待找到可用样本后补充
    
    # 边界情况测试
    "test/broken_ntsc.mpg" = "MPEG2/broken-ntsc.mpg"
    
    # 高码率测试
    "test/mkv_h264_eac3.mkv" = "Matroska/H264+EAC3.mkv"
    
    # 更多 MP3 样本
    "audio/mp3/mp3_cbr.mp3" = "A-codecs/MP3/CBR/sample.mp3"
    
    # 更多 FLAC 样本  
    "audio/flac/flac_16bit_44khz.flac" = "A-codecs/flac/luckynight.flac"
    
    # AC3 音频样本
    "audio/ac3/ac3_5.1.ac3" = "A-codecs/AC3/Canyon-5.1-48khz-448kbit.ac3"
    
    # MPEG1 视频样本
    "video/mpeg1/zelda_commercial.mpeg" = "MPEG1/zelda first commercial.mpeg"
    
    # MPEG2 视频样本
    "video/mpeg2/dvd_sample.mpeg" = "MPEG2/dvd.mpeg"
}

# P2 样本 (可选下载 - 特殊格式和大文件)
$P2_SAMPLES = @{
    # VP8/VP9 样本 (WebM)
    "video/vp8/vp8_sample.webm" = "V-codecs/VP8/vp8_sample.webm"
    
    # 游戏格式 (作为特殊测试)
    "special/bink_sample.bik" = "game-formats/bink/logo_collective.bik"
    
    # 大文件测试 (> 100MB)
    # 暂时留空，按需添加
}

# 主下载函数
function Start-Download {
    param(
        [hashtable]$Samples,
        [string]$Priority,
        [switch]$Force
    )
    
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Cyan
    Write-Host " 下载 $Priority 优先级样本" -ForegroundColor Cyan
    Write-Host "============================================" -ForegroundColor Cyan
    Write-Host ""
    
    $success = 0
    $failed = 0
    $skipped = 0
    
    foreach ($entry in $Samples.GetEnumerator()) {
        $localPath = ".\samples\$($entry.Key)"
        $remotePath = $entry.Value
        
        # 创建目标目录
        $directory = Split-Path $localPath -Parent
        New-SampleDirectory -Path $directory
        
        # 下载文件
        $result = Download-Sample -Url $remotePath -OutputPath $localPath -Force:$Force
        if ($result) {
            $success++
        } else {
            $failed++
        }
        
        Start-Sleep -Milliseconds 200  # 避免请求过快
    }
    
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Cyan
    Write-Host " $Priority 下载完成" -ForegroundColor Cyan
    Write-Host " 成功: $success | 失败: $failed" -ForegroundColor Cyan
    Write-Host "============================================" -ForegroundColor Cyan
    Write-Host ""
}

# 主程序
function Main {
    param(
        [string]$Priority = "P0",
        [switch]$Force,
        [switch]$All
    )
    
    Write-Host ""
    Write-Host "╔══════════════════════════════════════════╗" -ForegroundColor Magenta
    Write-Host "║  Tao 项目测试样本自动下载工具 v3.0     ║" -ForegroundColor Magenta
    Write-Host "║  基于 FFmpeg 官方样本库                 ║" -ForegroundColor Magenta
    Write-Host "╚══════════════════════════════════════════╝" -ForegroundColor Magenta
    Write-Host ""
    
    # 创建根目录
    $samplesRoot = ".\samples"
    New-SampleDirectory -Path $samplesRoot
    
    # 根据参数决定下载哪些样本
    if ($All) {
        Start-Download -Samples $P0_SAMPLES -Priority "P0" -Force:$Force
        Start-Download -Samples $P1_SAMPLES -Priority "P1" -Force:$Force
        Start-Download -Samples $P2_SAMPLES -Priority "P2" -Force:$Force
    }
    elseif ($Priority -eq "P0") {
        Start-Download -Samples $P0_SAMPLES -Priority "P0" -Force:$Force
    }
    elseif ($Priority -eq "P1") {
        Start-Download -Samples $P1_SAMPLES -Priority "P1" -Force:$Force
    }
    elseif ($Priority -eq "P2") {
        Start-Download -Samples $P2_SAMPLES -Priority "P2" -Force:$Force
    }
    else {
        Write-Host "未知优先级: $Priority" -ForegroundColor Red
        Write-Host "支持的优先级: P0, P1, P2" -ForegroundColor Yellow
        Write-Host "使用 -All 下载所有样本" -ForegroundColor Yellow
        return
    }
    
    Write-Host ""
    Write-Host "✓ 所有下载任务完成!" -ForegroundColor Green
    Write-Host ""
    Write-Host "使用说明:" -ForegroundColor Cyan
    Write-Host "  .\download_samples_v3.ps1 -Priority P0     # 仅下载 P0 (必须)" -ForegroundColor Gray
    Write-Host "  .\download_samples_v3.ps1 -Priority P1     # 仅下载 P1 (推荐)" -ForegroundColor Gray
    Write-Host "  .\download_samples_v3.ps1 -All             # 下载所有优先级" -ForegroundColor Gray
    Write-Host "  .\download_samples_v3.ps1 -Force           # 强制重新下载" -ForegroundColor Gray
    Write-Host ""
}

# 执行主程序
Main @args
