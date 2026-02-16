# Tao 项目测试样本下载计划

> 基于 https://samples.ffmpeg.org/ 官方样本库

## 文件大小规则

- **< 50MB**: 下载到 `data/samples/` 目录, 提交到 Git
- **≥ 50MB**: 仅记录 URL 到 `data/samples/INVENTORY.md`, 不下载文件
    - 测试时使用 URL 直接流式访问
    - 测试用例标记为 `#[ignore]` 避免 CI 中频繁下载
    - 手动测试: `cargo test -- --ignored test_name`

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

| 类别     | 建议大小 | 说明               |
| -------- | -------- | ------------------ |
| 单元测试 | < 1MB    | 快速验证基本功能   |
| 集成测试 | 1-10MB   | 完整编解码流程测试 |
| 性能测试 | 10-100MB | 性能基准测试       |
| 压力测试 | > 100MB  | 大文件处理测试     |

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

## 6. 如何添加新样本

### 6.1 添加新编解码器样本

当项目实现新的编解码器时:

1. **查找样本**: 访问 https://samples.ffmpeg.org/ 查找对应编解码器的测试样本
2. **检查大小**: 使用 `curl -I <URL> | Select-String "Content-Length"` 检查文件大小
3. **更新计划**: 在本文件 (`SAMPLES.md`) 中添加新编解码器章节，列出样本 URL
4. **处理样本**:
    - **< 50MB**: 更新 `download_samples.ps1` 并执行下载
    - **≥ 50MB**: 仅在 `INVENTORY.md` 中记录 URL, 标注"仅 URL"
5. **更新清单**: 在 `samples/INVENTORY.md` 中记录样本信息 (本地路径或 URL)
6. **提交到 Git**: 将样本文件 (如有) 和文档更新提交到版本库

### 6.2 添加滤镜测试样本

滤镜测试通常复用已有样本，如需特殊样本:

```powershell
# 创建滤镜测试目录
mkdir -p data/samples/filter/scale
mkdir -p data/samples/filter/crop
mkdir -p data/samples/filter/overlay

# 下载或复制样本
# 记录到 INVENTORY.md
```

### 6.3 添加边界测试样本

边界测试样本放在 `test/unit/` 目录:

```rust
// 在测试代码中引用
const CORRUPTED_FILE: &str = "data/test/unit/corrupted_h264.bin";
const EMPTY_FILE: &str = "data/test/unit/empty.mp4";
const EXTREME_PARAMS: &str = "data/test/unit/extreme_resolution.h264";
```

### 6.4 添加性能测试样本

性能测试样本放在 `test/bench/` 目录:

```rust
// 不同大小的样本用于性能对比
const SMALL_SAMPLE: &str = "data/test/bench/small_480p.mp4";   // < 10MB (本地文件)
const MEDIUM_SAMPLE: &str = "data/test/bench/medium_1080p.mp4"; // 10-50MB (本地文件)

// 大文件使用 URL (≥ 50MB)
const LARGE_SAMPLE_URL: &str = "https://samples.ffmpeg.org/video/4k/large_4k.mp4"; // > 100MB (URL)
```

### 6.5 样本命名规范

- **编解码器样本**: `{codec}_{特性}_{分辨率}.{ext}`
    - 示例: `h264_cabac_1080p.mp4`, `theora_vp3_480p.ogg`
- **容器样本**: `{container}_{codecs}.{ext}`
    - 示例: `mkv_h264_aac.mkv`, `mp4_mpeg4_mp3.mp4`
- **边界测试**: `{类型}_{描述}.{ext}`
    - 示例: `corrupted_missing_sps.h264`, `truncated_middle.mp4`
- **性能测试**: `{size}_{resolution}_{codec}.{ext}`
    - 示例: `small_480p_h264.mp4`, `large_4k_hevc.mkv`

## 7. 维护工作流

```bash
# 1. 实现新编解码器 (例如 VP8)
git checkout -b feat/vp8-decoder

# 2. 更新样本计划
vim data/SAMPLES.md  # 添加 VP8 样本章节

# 3. 更新下载脚本
vim data/download_samples.ps1  # 添加 VP8 样本 URL

# 4. 下载样本
./data/download_samples.ps1 -Priority P0

# 5. 更新清单
vim data/samples/INVENTORY.md  # 记录新样本信息

# 6. 编写测试
vim tests/vp8_pipeline.rs  # 使用新样本测试

# 7. 提交所有更改
git add data/ tests/vp8_pipeline.rs
git commit -m "feat: 添加 VP8 解码器及测试样本"
```

## 8. 注意事项

- **所有样本文件提交到 Git**: 确保 CI/CD 和团队成员测试环境一致
- **保留原始 URL 信息**: 便于重新下载和验证样本来源
- **定期检查更新**: 每季度检查 FFmpeg 官方样本库是否有新样本或更新
- **清理无用样本**: 当编解码器移除或重构时，同步清理相关样本
- **文档同步**: 样本文件、下载脚本、清单文档必须同步更新
