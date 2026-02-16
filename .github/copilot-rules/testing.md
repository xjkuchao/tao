# 测试规范和数据管理

> 本文件定义测试规范、测试用例开发流程和数据文件管理规则。

---

## 1. 测试基本要求

### 1.1 强制要求

- ✅ 代码修改后**必须执行** `cargo check` 与 `cargo test`
- ✅ 如出现错误或警告,**必须先修复**再继续后续修改
- ✅ **重要**: 新增编解码器或容器格式时**必须编写测试**
- ✅ 集成测试放在 `tests/` 目录下
- ✅ 单元测试放在源文件内 `#[cfg(test)]` 模块中
- ✅ 测试用例命名需要准确描述测试内容与预期结果,使用蛇形命名法
- ✅ 测试应覆盖**正常流程、边界情况和错误情况**

### 1.2 测试数据原则

- ✅ **所有样本使用 HTTPS URL 直接访问**,不下载到本地
- ✅ 样本来源: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
- ✅ 样本 URL 维护在 `samples/SAMPLE_URLS.md` 中
- ✅ **临时文件放在 `data/` 目录**,永不提交到 Git
- ✅ **日志文件放在 `logs/` 目录**,永不提交到 Git

---

## 2. 测试用例开发流程

### 步骤 1: 确定测试需求

明确需要测试的场景:

- **正常流程**: 标准输入输出,基本功能验证
- **边界情况**: 空输入,极限参数,特殊格式
- **错误处理**: 损坏数据,不支持的参数,资源不足

### 步骤 2: 查找测试样本

**重要**: 所有样本使用 **HTTPS URL** 直接访问,无需下载到本地

**查找流程:**

1. **查找样本**: 在 `samples/SAMPLE_URLS.md` 中查找适用的样本 URL
2. **样本来源**: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
3. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息

    ```bash
    ffprobe https://samples.ffmpeg.org/HDTV/Channel9_HD.ts
    ```

4. **如果没有合适样本**:
    - 访问 https://samples.ffmpeg.org/ 浏览完整样本库
    - 或查看 https://samples.ffmpeg.org/allsamples.txt
    - 使用 `ffprobe <URL>` 验证
    - 添加到 `samples/SAMPLE_URLS.md` 并提交:

    ```bash
    git add samples/SAMPLE_URLS.md
    git commit -m "docs: 添加 XXX 编解码器测试样本 URL"
    ```

### 步骤 3: 编写测试用例

**测试用例模板:**

```rust
// tests/h264_decode_pipeline.rs

use tao_format::demuxer::DemuxerRegistry;
use tao_codec::decoder::DecoderRegistry;
use tao_core::MediaType;

#[test]
fn test_h264_decode_basic() {
    // 1. 从 samples/SAMPLE_URLS.md 获取样本 URL
    let sample_url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    // 2. 直接使用 URL 打开解封装器,无需下载文件
    let mut demuxer = DemuxerRegistry::open(sample_url)
        .expect("应该成功打开解封装器");

    // 3. 查找视频流
    let video_stream = demuxer.streams()
        .iter()
        .find(|s| s.media_type() == MediaType::Video)
        .expect("应该找到视频流");

    // 4. 创建解码器
    let mut decoder = DecoderRegistry::create_decoder(
        video_stream.codec_id(),
        video_stream.codec_params(),
    ).expect("应该成功创建解码器");

    // 5. 解码前几帧验证功能
    // 重要: 只解码 5-10 帧就可以验证功能,避免测试耗时过长
    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet().expect("读取数据包应该成功") {
        if packet.stream_index() == video_stream.index() {
            // 发送数据包到解码器
            decoder.send_packet(&packet).expect("发送数据包应该成功");

            // 接收解码帧
            while let Some(frame) = decoder.receive_frame().expect("接收帧应该成功") {
                frame_count += 1;

                // 验证帧属性
                assert!(frame.width() > 0, "帧宽度应大于 0");
                assert!(frame.height() > 0, "帧高度应大于 0");

                if frame_count >= 10 { break; }  // 限制帧数
            }

            if frame_count >= 10 { break; }
        }
    }

    assert!(frame_count >= 10, "应该解码出至少 10 帧");
}

#[test]
fn test_h264_decode_invalid_data() {
    // 边界测试: 测试损坏数据处理
    use tao_codec::Packet;

    // 构造无效数据
    let corrupted_data = vec![0xFF; 100];
    let packet = Packet::new(corrupted_data, 0);

    // 创建解码器
    let mut params = CodecParameters::new();
    params.set_codec_id(CodecId::H264);
    params.set_width(1920);
    params.set_height(1080);

    let mut decoder = DecoderRegistry::create_decoder(CodecId::H264, &params)
        .expect("应该成功创建解码器");

    // 尝试解码损坏数据
    // 应该返回错误,而不是 panic
    match decoder.send_packet(&packet) {
        Err(TaoError::InvalidData(_)) => {
            // 正确处理损坏数据
        }
        Err(e) => {
            // 其他错误也可接受
            println!("解码器返回错误: {:?}", e);
        }
        Ok(_) => {
            // 某些解码器可能忽略损坏数据,也是可接受的行为
        }
    }
}
```

### 步骤 4: 执行测试

```bash
# 运行特定测试
cargo test --test h264_decode_pipeline

# 运行所有测试
cargo test

# 安静模式(只显示失败)
cargo test --quiet
```

### 步骤 5: 提交测试代码

测试通过后提交代码:

```bash
git add tests/h264_decode_pipeline.rs
git commit -m "test: 添加 H.264 解码器测试用例"
```

---

## 3. 测试用例编写标准

### 3.1 文件命名和位置

- **文件位置**: `tests/{feature}_pipeline.rs`
- **命名格式**: `{codec/format}_{operation}_pipeline.rs`
- **示例**:
    - `h264_decode_pipeline.rs` - H.264 解码测试
    - `mp4_demux_pipeline.rs` - MP4 解封装测试
    - `aac_encode_pipeline.rs` - AAC 编码测试

### 3.2 测试函数命名

- **格式**: `test_{component}_{scenario}`
- **示例**:
    - `test_h264_decode_basic` - H.264 基本解码
    - `test_h264_decode_invalid_data` - H.264 损坏数据处理
    - `test_mp4_demux_seek` - MP4 seek 操作

### 3.3 断言和错误消息

- ✅ 每个 `assert!` 都应包含失败消息,说明预期行为
- ✅ 使用 `expect()` 而非 `unwrap()`,提供清晰的错误消息

**示例:**

```rust
// ✅ 正确: 包含错误消息
assert!(frame_count >= 10, "应该解码出至少 10 帧,实际: {}", frame_count);
let stream = streams.get(0).expect("应该至少有一个流");

// ❌ 错误: 无错误消息
assert!(frame_count >= 10);
let stream = streams.get(0).unwrap();
```

### 3.4 注释和文档

- ✅ 复杂逻辑添加注释,建议使用 step-by-step 清晰说明
- ✅ 每个测试用例开头说明测试目的
- ✅ 边界测试和错误测试注明具体测试场景

### 3.5 帧数限制

- ✅ **只解码前 5-10 帧验证功能**,避免测试耗时过长
- ✅ 使用 `break` 限制循环次数
- ✅ 大文件测试尤其需要限制

---

## 4. 测试覆盖范围

### 4.1 编解码器测试

- ✅ 基本解码(正常流程)
- ✅ 编码(如果实现了编码器)
- ✅ 空输入处理
- ✅ 损坏数据处理
- ✅ Flush 流程
- ✅ 参数解析(SPS/PPS/VPS 等)

### 4.2 容器格式测试

- ✅ 格式探测(Probe)
- ✅ 头部解析
- ✅ 数据包读取
- ✅ Seek 操作
- ✅ 多流处理(音视频同时存在)
- ✅ 损坏文件处理

### 4.3 滤镜测试

- ✅ 基本滤镜操作
- ✅ 参数验证
- ✅ 链式滤镜
- ✅ 边界条件(分辨率、像素格式)

---

## 5. 手动播放测试规范

### 5.1 播放时长限制

- ✅ 手动测试音视频播放时,**禁止完整播放**整个文件
- ✅ 默认播放 **前 10 秒** 即可验证功能
- ✅ 如有必要(如需验证 seek/后段内容)可增加到 **最多 30 秒**
- ✅ 播放结束后必须主动终止播放进程

### 5.2 终止播放进程(Windows)

- ✅ Windows 下终止 `tao-play` 进程时,**必须使用 `TASKKILL /F /IM tao-play.exe /T`**
- ❌ **禁止使用 `TASKKILL /F /PID <pid>`**(PID 不可靠)

**示例:**

```powershell
# ✅ 正确
TASKKILL /F /IM tao-play.exe /T

# ❌ 错误(PID 不可靠)
TASKKILL /F /PID 12345
```

### 5.3 流式播放测试

- ✅ `tao-play` 支持 http/https/rtmp 等流式 URL 播放
- ✅ **所有测试文件均使用 URL 直接流式播放**,不下载到本地
- ✅ 所有样本 URL 维护在 `samples/SAMPLE_URLS.md` 中

**示例:**

```powershell
# 正确: 直接使用 URL 进行流式播放
cargo run --package tao-play -- "https://samples.ffmpeg.org/flac/Yesterday.flac"

# 查看更多样本 URL
# 请参考 samples/SAMPLE_URLS.md
```

---

## 6. 测试文件和临时文件管理

### 6.1 目录结构

- **`tests/`** - 项目根目录下,包含所有集成测试代码
- **`samples/`** - 样本使用规范和 URL 清单
    - `samples/SAMPLES.md` - 样本使用规范和文件管理规则
    - `samples/SAMPLE_URLS.md` - 测试样本 URL 清单(所有样本使用 URL 访问)
- **`data/`** - 临时文件目录
    - `data/.gitkeep` - 确保 data 文件夹始终存在于 Git
    - `data/tmp/` 和其他 - 所有临时文件(不提交到 Git)
- **`logs/`** - 日志文件目录
    - `logs/.gitkeep` - 确保 logs 文件夹始终存在于 Git
    - `logs/*.log` - 所有日志文件(不提交到 Git)

### 6.2 文件放置规则

**测试代码文件**: 全部放在 `tests/` 目录下

- 集成测试: `tests/{feature}_pipeline.rs`
- 单元测试: 在 `crates/` 各 crate 的源文件中使用 `#[cfg(test)]` 模块

**测试样本**: 使用 `samples/SAMPLE_URLS.md` 中的 HTTPS URL

- 所有样本来源: https://samples.ffmpeg.org/
- 所有样本使用 URL 标识,无需本地下载
- 直接使用 URL 创建 Demuxer/Decoder

**临时文件**: 必须放在 `data/` 目录下

- 运行时生成的临时文件
- 编解码过程中的中间文件
- 永不提交到 Git(整体 `/data/` 已被 `.gitignore` 忽略)

**日志文件**: 必须放在 `logs/` 目录下

- 所有日志文件存放在 `logs/` 目录
- 文件命名: `{file_prefix}.{YYYY-MM-DD}.log`
- 永不提交到 Git(整体 `/logs/` 已被 `.gitignore` 忽略)

### 6.3 测试样本 URL 规范

**样本源**: 优先使用 https://samples.ffmpeg.org/ 提供的公开测试样本

**样本类别**:

- 视频: H.264, H.265, VP8, VP9, AV1, MPEG4 Part 2, Theora, ProRes 等
- 音频: AAC, MP3, FLAC, Opus, Vorbis, WAV, ALAC 等
- 容器: MP4, MKV, WebM, OGG, AVI, TS 等

**使用方式**:

- ✅ **所有样本使用 URL 方式访问,不下载到本地**
- ✅ 直接使用 HTTPS URL 创建 Demuxer/Decoder
- ✅ 完整 URL 格式: `https://samples.ffmpeg.org/<category>/<filename>`
- ✅ 示例: `https://samples.ffmpeg.org/HDTV/Channel9_HD.ts`

**版本管理**:

- ✅ 所有样本 URL 记录在 `samples/SAMPLE_URLS.md` 中
- ✅ 添加新样本时更新清单并提交到 Git
- ✅ URL 失效时从 https://samples.ffmpeg.org/ 查找替代样本

### 6.4 临时文件管理

**创建**: 所有临时文件必须在 `data/` 目录下创建

```rust
use std::path::PathBuf;

let tmp_dir = PathBuf::from("data/tmp");
std::fs::create_dir_all(&tmp_dir).expect("创建临时目录失败");

let tmp_file = tmp_dir.join("output.mp4");
```

**清理**: 测试结束后必须清理临时文件

```rust
#[test]
fn test_with_temp_file() {
    let tmp_file = PathBuf::from("data/tmp/test_output.mp4");

    // 测试逻辑...

    // 清理临时文件
    if tmp_file.exists() {
        std::fs::remove_file(&tmp_file).ok();
    }
}
```

**命名**: 临时文件使用前缀 `tmp_` 或进程 ID 命名

**Git**: 永不提交临时文件到版本控制

### 6.5 日志文件管理

详见 [performance.md](performance.md) §日志规范

---

## 7. 新增测试样本流程

当需要新的测试样本时:

1. **查找样本**: 访问 https://samples.ffmpeg.org/ 浏览或搜索合适样本
2. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息
3. **添加 URL**: 在 `samples/SAMPLE_URLS.md` 对应章节添加 URL 和说明
4. **提交更改**:

    ```bash
    git add samples/SAMPLE_URLS.md
    git commit -m "docs: 添加 XXX 样本 URL"
    ```

---

## 总结

测试规范强调**使用 URL 访问样本、限制测试帧数、临时文件放 data/ 目录**。所有集成测试放在 `tests/` 目录,测试用例命名清晰,断言包含错误消息。手动播放测试限制时长,Windows 下使用 `TASKKILL /IM` 终止进程。严格遵守文件管理规则,确保项目整洁。
