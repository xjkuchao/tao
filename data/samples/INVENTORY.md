# Tao 项目测试样本清单

> 最后更新: 2026-02-16
> 样本来源: https://samples.ffmpeg.org/

## 已下载样本统计

### P0 优先级样本 (核心功能测试)

#### 视频样本

| 文件路径 | 编解码器 | 容器格式 | 用途 |
|---------|---------|---------|------|
| `video/h264/h264_mp4.mp4` | H.264 | MP4 | 基础 H.264 解码测试 |
| `video/h264/h264_mkv.mkv` | H.264 | MKV | Matroska 容器测试 |
| `video/h264/channel9_hd.ts` | H.264 | MPEG-TS | 传输流测试, 720p HD |
| `video/mpeg4/mpeg4_avi.avi` | MPEG4 Part 2 | AVI | MPEG4 解码测试 |
| `video/theora/theora.ogg` | Theora | Ogg | Theora 解码测试 |
| `video/theora/theora_test.ogg` | Theora | Ogg | Theora 小样本测试 |

#### 音频样本

| 文件路径 | 编解码器 | 容器格式 | 用途 |
|---------|---------|---------|------|
| `audio/aac/aac_in_mov.mov` | AAC | MOV | AAC 解码测试 |
| `audio/flac/flac_yesterday.flac` | FLAC | FLAC | 无损音频测试 |
| `audio/vorbis/vorbis_test.ogg` | Vorbis | Ogg | Vorbis 解码测试 |
| `audio/vorbis/vorbis_coyote.ogg` | Vorbis | Ogg | Vorbis 长样本测试 |
| `audio/pcm/wav_8khz_16bit_mono.wav` | PCM S16LE | WAV | 基础 WAV 测试 |
| `audio/pcm/wav_96khz.wav` | PCM S16LE | WAV | 高码率 WAV 测试 |

#### 容器格式样本

| 文件路径 | 主要编解码器 | 用途 |
|---------|------------|------|
| `container/mp4/mp4_h264.mp4` | H.264 + AAC | MP4 封装/解封装 |
| `container/mkv/mkv_h264.mkv` | H.264 + AAC | MKV 封装/解封装 |
| `container/avi/avi_dual_audio.avi` | MPEG4 + PCM | AVI 多音轨测试 |
| `container/flv/flv_vp6.flv` | VP6 + MP3 | FLV 解封装测试 |
| `container/mpegts/ts_h264_ac3.ts` | H.264 + AC3 | MPEG-TS 传输流 |
| `container/ogg/ogg_theora_vorbis.ogg` | Theora + Vorbis | Ogg 解封装测试 |
| `container/ogg/ogg_vorbis_only.ogg` | Vorbis | Ogg 纯音频测试 |
| `container/aiff/aiff_dragon.aif` | PCM | AIFF 解封装测试 |
| `container/wav/wav_8khz_mono.wav` | PCM | WAV 解封装测试 |

### P1 优先级样本 (扩展功能测试)

| 文件路径 | 编解码器 | 用途 |
|---------|---------|------|
| `audio/ac3/ac3_5.1.ac3` | AC3 5.1 | 多声道音频测试 |
| `video/mpeg1/zelda_commercial.mpeg` | MPEG1 | MPEG1 视频测试 |
| `video/mpeg2/dvd_sample.mpeg` | MPEG2 | MPEG2 视频测试 |
| `test/mkv_h264_eac3.mkv` | H.264 + EAC3 | 增强 AC3 测试 |
| `test/broken_ntsc.mpg` | MPEG2 | 损坏文件容错测试 |

## 样本总大小

- **P0 样本**: 约 160 MB
- **P1 样本**: 约 100 MB
- **总计**: 约 260 MB

## 使用建议

### 单元测试
使用小样本文件 (< 5MB):
- `audio/vorbis/vorbis_test.ogg` (51 KB)
- `audio/pcm/wav_8khz_16bit_mono.wav` (186 KB)
- `container/aiff/aiff_dragon.aif` (129 KB)

### 集成测试
使用中等大小样本 (5-20MB):
- `video/theora/theora_test.ogg` (10 MB)
- `audio/flac/flac_yesterday.flac` (11 MB)
- `video/theora/theora.ogg` (15 MB)

### 性能测试
使用大样本文件 (> 20MB):
- `video/h264/channel9_hd.ts` (27 MB)
- `container/mkv/mkv_h264.mkv` (31 MB)
- `test/broken_ntsc.mpg` (42 MB)

## 已知问题

以下样本在下载时出现 404 错误 (不影响核心功能测试):
- `A-codecs/MP3/ID3V2/sample.mp3`
- `A-codecs/MP3/CBR/sample.mp3`
- `A-codecs/flac/luckynight.flac`

这些样本将在后续根据需要使用其他等效样本替代。

## 样本更新

如需重新下载或更新样本:

```powershell
# 下载 P0 样本
.\download_samples_v3.ps1 -Priority P0

# 下载 P1 样本
.\download_samples_v3.ps1 -Priority P1

# 下载所有样本
.\download_samples_v3.ps1 -All

# 强制重新下载
.\download_samples_v3.ps1 -All -Force
```

## 参考资料

- FFmpeg 官方样本库: https://samples.ffmpeg.org/
- 样本列表 (旧): https://samples.ffmpeg.org/allsamples-old.txt
- 样本列表 (新): https://samples.ffmpeg.org/allsamples.txt
