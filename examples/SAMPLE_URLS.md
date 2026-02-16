# Tao 测试样本 URL 清单

> 所有测试样本均使用 URL 方式访问，不下载到本地

## 样本来源

- **主要来源**: https://samples.ffmpeg.org/
- **样本列表**:
    - https://samples.ffmpeg.org/allsamples.txt (当前样本)
    - https://samples.ffmpeg.org/allsamples-old.txt (历史样本)

## 使用方法

1. **查找样本**: 在本文件中搜索所需编解码器或容器格式
2. **编写测试**: 使用 URL 直接创建 Demuxer/Decoder
3. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息

## 视频编解码器样本

### H.264

| 用途     | URL                                             | 描述                            |
| -------- | ----------------------------------------------- | ------------------------------- |
| 基础解码 | https://samples.ffmpeg.org/HDTV/Channel9_HD.ts  | 720p, H.264 + AC3, MPEG-TS 容器 |
| 高清解码 | https://samples.ffmpeg.org/Matroska/haruhi.mkv  | 1080p, H.264 + AAC, MKV 容器    |
| MP4 容器 | https://samples.ffmpeg.org/mov/mov_h264_aac.mov | 标准 MP4, H.264 + AAC           |

### MPEG-4 Part 2

| 用途     | URL                                                             | 描述                       |
| -------- | --------------------------------------------------------------- | -------------------------- |
| 基础解码 | https://samples.ffmpeg.org/V-codecs/MPEG4/mpeg4_avi.avi         | 标准 MPEG-4, AVI 容器      |
| 数据分区 | https://samples.ffmpeg.org/V-codecs/MPEG4/data_partitioning.avi | Data Partitioning 特性测试 |

### Theora

| 用途     | URL                                                   | 描述                      |
| -------- | ----------------------------------------------------- | ------------------------- |
| 基础解码 | https://samples.ffmpeg.org/ogg/Theora/theora.ogg      | Theora + Vorbis, Ogg 容器 |
| VP3 兼容 | https://samples.ffmpeg.org/ogg/Theora/theora_test.ogg | VP3 派生编码测试          |

## 音频编解码器样本

### AAC

| 用途   | URL                                                     | 描述              |
| ------ | ------------------------------------------------------- | ----------------- |
| AAC-LC | https://samples.ffmpeg.org/A-codecs/AAC/latm_sample.aac | AAC-LC, LATM 格式 |
| HE-AAC | https://samples.ffmpeg.org/A-codecs/AAC/he_sample.aac   | HE-AAC v1         |

### MP3

| 用途 | URL                                                    | 描述          |
| ---- | ------------------------------------------------------ | ------------- |
| CBR  | https://samples.ffmpeg.org/A-codecs/MP3/CBR/sample.mp3 | 恒定码率      |
| VBR  | https://samples.ffmpeg.org/A-codecs/MP3/VBR/lame.mp3   | 可变码率 LAME |

### FLAC

| 用途     | URL                                                     | 描述          |
| -------- | ------------------------------------------------------- | ------------- |
| 无损音频 | https://samples.ffmpeg.org/A-codecs/flac/Yesterday.flac | FLAC 无损编码 |

### Vorbis

| 用途       | URL                                             | 描述             |
| ---------- | ----------------------------------------------- | ---------------- |
| Ogg Vorbis | https://samples.ffmpeg.org/ogg/Vorbis/test6.ogg | 标准 Vorbis 编码 |

### PCM

| 用途       | URL                                                            | 描述         |
| ---------- | -------------------------------------------------------------- | ------------ |
| 8kHz 16bit | https://samples.ffmpeg.org/A-codecs/wavpcm/8khz-16bit-mono.wav | 低采样率测试 |
| 96kHz      | https://samples.ffmpeg.org/A-codecs/wavpcm/test-96.wav         | 高采样率测试 |

## 容器格式样本

### MP4

| 用途     | URL                                                          | 描述          |
| -------- | ------------------------------------------------------------ | ------------- |
| 标准 MP4 | https://samples.ffmpeg.org/mov/mp4/mp4_test.mp4              | H.264 + AAC   |
| Editlist | https://samples.ffmpeg.org/mov/editlist/mov_edl_kf_fix_1.mp4 | Editlist 特性 |

### Matroska (MKV)

| 用途         | URL                                               | 描述       |
| ------------ | ------------------------------------------------- | ---------- |
| H.264 + AAC  | https://samples.ffmpeg.org/Matroska/haruhi.mkv    | 标准 MKV   |
| H.264 + EAC3 | https://samples.ffmpeg.org/Matroska/H264+EAC3.mkv | 多声道音频 |

### AVI

| 用途   | URL                                                     | 描述         |
| ------ | ------------------------------------------------------- | ------------ |
| 双音轨 | https://samples.ffmpeg.org/avi/02-audio-streams.avi     | 多音轨测试   |
| MPEG-4 | https://samples.ffmpeg.org/V-codecs/MPEG4/mpeg4_avi.avi | MPEG-4 + AVI |

### FLV

| 用途 | URL                                                     | 描述     |
| ---- | ------------------------------------------------------- | -------- |
| VP6  | https://samples.ffmpeg.org/FLV/flash8/artifacts-vp6.flv | VP6 编码 |

### MPEG-TS

| 用途        | URL                                                 | 描述       |
| ----------- | --------------------------------------------------- | ---------- |
| H.264 + AC3 | https://samples.ffmpeg.org/HDTV/channel9hdtv_ac3.ts | 高清传输流 |

### Ogg

| 用途            | URL                                              | 描述        |
| --------------- | ------------------------------------------------ | ----------- |
| Theora + Vorbis | https://samples.ffmpeg.org/ogg/Theora/theora.ogg | 视频 + 音频 |
| 纯音频          | https://samples.ffmpeg.org/ogg/Vorbis/test6.ogg  | 仅 Vorbis   |

### WAV

| 用途     | URL                                                            | 描述            |
| -------- | -------------------------------------------------------------- | --------------- |
| 标准 PCM | https://samples.ffmpeg.org/A-codecs/wavpcm/8khz-16bit-mono.wav | 8kHz 16bit Mono |

### AIFF

| 用途      | URL                                                      | 描述      |
| --------- | -------------------------------------------------------- | --------- |
| AIFF 容器 | https://samples.ffmpeg.org/A-codecs/aiff/aiff_dragon.aif | 标准 AIFF |

## 边界测试样本

| 用途      | URL                                            | 描述             |
| --------- | ---------------------------------------------- | ---------------- |
| 损坏 MPEG | https://samples.ffmpeg.org/mpg/broken_ntsc.mpg | 损坏的 MPEG 文件 |

## 添加新样本流程

1. **查找样本**: 访问 https://samples.ffmpeg.org/ 浏览或搜索
2. **验证样本**: 使用 `ffprobe <URL>` 检查编解码器信息
    ```bash
    ffprobe https://samples.ffmpeg.org/path/to/sample.mp4
    ```
3. **添加到清单**: 在对应章节添加表格行
    ```markdown
    | 用途描述 | https://samples.ffmpeg.org/path/to/sample.ext | 详细说明 |
    ```
4. **提交更新**: 提交清单更新到 Git
    ```bash
    git add data/SAMPLE_URLS.md
    git commit -m "docs: 添加 XXX 编解码器测试样本 URL"
    ```

## 测试编写示例

```rust
#[test]
fn test_h264_decode_from_url() {
    let sample_url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    let mut demuxer = DemuxerRegistry::open(sample_url).unwrap();
    let video_stream = demuxer.streams()
        .iter()
        .find(|s| s.media_type() == MediaType::Video)
        .unwrap();

    let mut decoder = DecoderRegistry::create_decoder(
        video_stream.codec_id(),
        video_stream.codec_params(),
    ).unwrap();

    // 解码前几帧验证功能
    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet().unwrap() {
        if packet.stream_index() == video_stream.index() {
            let frames = decoder.decode(&packet).unwrap();
            frame_count += frames.len();
            if frame_count >= 10 { break; }
        }
    }

    assert!(frame_count >= 10, "应该至少解码 10 帧");
}
```

## 注意事项

- 所有测试使用 URL 直接访问，无需下载
- 测试只解码前几帧 (5-10 帧) 验证功能即可
- URL 失效时，从 https://samples.ffmpeg.org/ 查找替代样本
- 网络访问失败的测试可标记为 `#[ignore]`
