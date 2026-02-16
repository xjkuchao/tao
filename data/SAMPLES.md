# Tao 项目测试样本下载计划

> 基于 https://samples.ffmpeg.org/ 官方样本库

## 1. 核心优先级样本 (必须下载)

根据项目当前支持的编解码器和容器格式，优先下载以下样本:

### 1.1 视频编解码器样本

#### H.264
- `./archive/video/h264/mov+h264+aac++bbc_1080p.mov` (1080p, AAC 音频)
- `./HDTV/Channel9_HD.ts` (MPEG-TS 容器, 720p)
- `./mov/h264_sample.mov` (QuickTime 容器)

#### H.265 (HEVC)
- `./A-codecs/hevc/*.mp4` (HEVC 测试样本)

#### MPEG4 Part 2
- `./V-codecs/MPEG4/mpeg4_sample.avi` (AVI 容器)
- `./mov/mp4/mpeg4_test.mp4` (MP4 容器)

#### Theora
- `./ogg/Theora/theora.ogg` (Ogg 容器)
- `./ogg/Theora/theora-a4_v6-k250-s0.ogg`

### 1.2 音频编解码器样本

#### AAC
- `./A-codecs/ATRAC3/sample.m4a` (ADTS 格式)
- `./A-codecs/aac-he/he_sample.aac` (HE-AAC)

#### MP3
- `./A-codecs/MP3/mp3-2.5/8khz.mp3` (MPEG 2.5 Layer III)
- `./A-codecs/MP3/VBR/lame.mp3` (VBR)

#### FLAC
- `./A-codecs/flac/*.flac` (无损压缩)

#### Vorbis
- `./flac/Yesterday.flac` (用于对比测试)
- `./A-codecs/vorbis/oggvorbis_sample.ogg`

#### PCM
- `./A-codecs/wavpcm/8khz-16bit-mono.wav` (8kHz, 16bit, Mono)
- `./A-codecs/wavpcm/test-96.wav` (96kHz 高码率)

### 1.3 容器格式样本

#### MP4
- `./mov/mp4/mp4_test.mp4` (标准 MP4)
- `./mov/editlist/mov_edl_kf_fix_1.mp4` (editlist 测试)

#### Matroska (MKV)
- `./Matroska/haruhi.mkv` (H.264 + AAC)
- `./Matroska/H264+EAC3.mkv` (H.264 + EAC3)

#### WebM
- (从 MKV 样本测试，因为 WebM 是 MKV 的子集)

#### AVI
- `./avi/02-audio-streams.avi` (双音轨)
- `./V-codecs/MPEG4/mpeg4_avi.avi`

#### FLV
- `./FLV/flash8/artifacts-vp6.flv` (VP6 视频)
- `./FLV/flash_screen/screen.flv` (屏幕录制)

#### MPEG-TS
- `./HDTV/channel9hdtv_ac3.ts` (AC3 音频)
- `./MPEG2/mpegts-klv/Day Flight.mpg`

#### Ogg
- `./ogg/Theora/theora.ogg` (Theora + Vorbis)
- `./ogg/Vorbis/test6.ogg` (纯音频)

#### WAV
- `./A-codecs/wavpcm/8khz-16bit-mono.wav`
- `./A-codecs/wavpcm/test-96.wav`

#### AIFF
- `./AIFF/dragon.aif`
- `./AIFF/M1F1-float32C-AFsp.aif`

## 2. 扩展测试样本 (可选)

### 2.1 边界情况测试
- `./fuzz/` (模糊测试样本)
- `./MPEG2/broken-ntsc.mpg` (损坏文件测试)

### 2.2 特殊格式
- `./game-formats/bink/*.bik` (游戏格式)
- `./real/` (Real Media 格式, 用于兼容性测试)

### 2.3 高级功能
- `./Matroska/chapters/*.mkv` (章节支持)
- `./sub/` (字幕支持)
- `./stereo3d/` (3D 视频)

## 3. 样本文件大小建议

| 类别 | 建议大小 | 说明 |
|------|---------|------|
| 单元测试 | < 1MB | 快速验证基本功能 |
| 集成测试 | 1-10MB | 完整编解码流程测试 |
| 性能测试 | 10-100MB | 性能基准测试 |
| 压力测试 | > 100MB | 大文件处理测试 |

## 4. 下载优先级

1. **P0 (必须)**: 核心编解码器 + 容器格式的基础样本 (< 50MB 总计)
2. **P1 (推荐)**: 边界情况和多码率样本 (< 200MB 总计)
3. **P2 (可选)**: 大文件和特殊格式样本 (< 1GB 总计)

## 5. 存储结构

```
data/samples/
├── video/          # 视频样本
│   ├── h264/
│   ├── h265/
│   ├── mpeg4/
│   └── theora/
├── audio/          # 音频样本
│   ├── aac/
│   ├── mp3/
│   ├── flac/
│   ├── vorbis/
│   └── pcm/
└── container/      # 容器格式样本
    ├── mp4/
    ├── mkv/
    ├── avi/
    ├── flv/
    ├── mpegts/
    ├── ogg/
    ├── wav/
    └── aiff/
```

## 6. 注意事项

- 样本文件仅用于测试，不应提交到 Git 仓库 (除非 < 1MB)
- 大文件使用 Git LFS 或在 CI/CD 中即时下载
- 保留原始 URL 信息，便于重新下载和验证
- 定期检查 FFmpeg 官方样本库更新
