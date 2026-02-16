# 项目开发规范

> 本文件是项目唯一的规范来源, 所有 AI 工具 (Cursor, Copilot, Codex, Claude Code 等) 均以此为准.

---

## 1. 项目概述

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架, 目标是全功能复刻 FFmpeg. 项目提供三种使用方式:

1. **Rust 库**: 其他 Rust 项目可通过 `tao` crate 直接调用
2. **C FFI (DLL/SO)**: 通过 `tao-ffi` crate 导出 C 兼容接口, 供 C/C++ 等语言调用
3. **命令行工具**: `tao` (对标 ffmpeg), `tao-probe` (对标 ffprobe) 和 `tao-play` (对标 ffplay) 可执行文件

## 2. 项目结构

本项目采用 Cargo Workspace 多 crate 架构:

```
tao/
├── Cargo.toml              # Workspace 根配置 + 门面库 (re-export 所有子 crate)
├── AGENTS.md               # 本规范文件 (AI 开发规范)
├── .rustfmt.toml           # 代码格式化配置
├── .gitignore
├── plans/                  # AI 执行计划文件存放目录
├── src/
│   └── lib.rs              # 门面库入口, 重导出所有子 crate
├── crates/                 # 库 crate (对标 FFmpeg 各子库)
│   ├── tao-core/           # 核心类型与工具 (对标 libavutil)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs          # 统一错误类型
│   │       ├── rational.rs       # 有理数 (时间基/帧率/宽高比)
│   │       ├── media_type.rs     # 媒体类型 (Video/Audio/Subtitle)
│   │       ├── pixel_format.rs   # 像素格式 (YUV420P, RGB24 等)
│   │       ├── sample_format.rs  # 音频采样格式 (S16, F32 等)
│   │       ├── channel_layout.rs # 声道布局 (Mono/Stereo/5.1 等)
│   │       ├── timestamp.rs      # 时间戳与时间基转换
│   │       └── color/            # 色彩空间/范围/原色/传递特性
│   ├── tao-codec/          # 编解码器框架 (对标 libavcodec)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── codec_id.rs       # 编解码器标识符
│   │       ├── decoder.rs        # 解码器 trait
│   │       ├── encoder.rs        # 编码器 trait
│   │       ├── frame.rs          # 解码帧 (VideoFrame/AudioFrame)
│   │       ├── packet.rs         # 压缩数据包
│   │       └── registry.rs       # 编解码器注册表
│   ├── tao-format/         # 容器格式框架 (对标 libavformat)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── format_id.rs      # 格式标识符
│   │       ├── demuxer.rs        # 解封装器 trait
│   │       ├── muxer.rs          # 封装器 trait
│   │       ├── stream.rs         # 流信息
│   │       ├── io.rs             # I/O 抽象层
│   │       ├── probe.rs          # 格式探测
│   │       └── registry.rs       # 格式注册表
│   ├── tao-filter/         # 滤镜框架 (对标 libavfilter)
│   ├── tao-scale/          # 图像缩放 (对标 libswscale)
│   ├── tao-resample/       # 音频重采样 (对标 libswresample)
│   └── tao-ffi/            # C FFI 导出层 (cdylib + staticlib)
├── bins/                   # 可执行文件 crate
│   ├── tao-cli/            # tao 命令行工具 (对标 ffmpeg)
│   ├── tao-probe/          # tao-probe 探测工具 (对标 ffprobe)
│   └── tao-play/           # 播放器 (对标 ffplay)
├── tests/                  # 集成测试
├── examples/               # 使用示例 (crate 调用示例)
├── samples/                # 测试样本清单 (SAMPLE_URLS.md, SAMPLES.md)
└── data/                   # 临时文件目录 (不提交到 Git)
```

### 2.1 crate 依赖关系

```
tao-core (无外部依赖, 最底层)
  ↑
tao-codec (依赖 tao-core)
  ↑
tao-format (依赖 tao-core, tao-codec)
tao-filter (依赖 tao-core, tao-codec)
tao-scale (依赖 tao-core)
tao-resample (依赖 tao-core)
  ↑
tao-ffi (依赖所有库 crate)
tao-cli (依赖所有库 crate)
tao-probe (依赖 tao-core, tao-codec, tao-format)
  ↑
tao (门面库, re-export 所有库 crate)
```

## 3. 核心架构

### 3.1 编解码管线

```
[输入文件] → Demuxer → Packet → Decoder → Frame → [Filter] → Encoder → Packet → Muxer → [输出文件]
```

### 3.2 注册表模式

编解码器和容器格式均采用注册表 (Registry) 模式:

- 各编解码器/格式通过工厂函数注册到全局注册表
- 运行时按 CodecId/FormatId 查找并创建实例
- 支持按优先级选择多个同 ID 的实现

### 3.3 I/O 抽象

IoContext 提供统一的读写接口, 支持多种后端:

- 文件 (FileBackend)
- 内存缓冲区 (MemoryBackend, 待实现)
- 网络流 (NetworkBackend, 待实现)

### 3.4 FFI 导出

tao-ffi crate 编译为 cdylib + staticlib:

- 所有导出函数以 `tao_` 前缀命名
- 使用 `#[no_mangle]` 和 `extern "C"` 确保 ABI 兼容
- 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 释放

## 4. 语言规范

- **重要**: 项目全部使用中文, 包括代码注释, 控制台日志, 错误信息, AI 输出等.
- 所有开发过程中的交流和文档必须使用中文.
- 标点符号使用英文标点.

## 5. 代码组织

- 确保项目模块化, 避免单一函数过于复杂 (建议单个函数不超过 50 行).
- 按功能分类组织代码结构, 遵循现有的 crate 划分和目录结构 (见第 2 节).
- 复杂模块应拆分为多个子模块, 每个子模块职责单一.
- 公共类型集中定义在对应 crate 中, 避免散落.
- 新增编解码器实现应放在 `tao-codec` 下对应子目录中.
- 新增容器格式实现应放在 `tao-format` 下对应子目录中.

### 5.1 执行计划管理

- **必须**: AI 在制定执行计划时, 必须将计划文件写入 `plans/` 目录.
- **命名规范**: 计划文件命名格式为 `{功能模块}_{任务描述}.md`, 如 `h264_decoder_improvement.md`.
- **计划内容**: 计划文件应包含:
    - 任务背景和目标
    - 详细执行步骤 (带编号)
    - 每步的预期产出
    - 依赖项和前置条件
    - 验收标准
- **断点续执行**: 计划文件应支持断点续执行, AI 应在计划中标记已完成的步骤.
- **跨 AI 协作**: 计划文件应足够详细, 使得不同 AI 工具可以基于同一计划继续执行.

### 5.2 根目录文件管理

- **严格禁止**: 不允许在项目根目录下随意创建新文件.
- **允许的根目录文件**:
    - 项目配置: `Cargo.toml`, `.rustfmt.toml`, `.gitignore`
    - 核心文档: `README.md`, `AGENTS.md` (本规范文件)
    - License 文件: `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`
- **其他文件的存放位置**:
    - 执行计划: 必须放在 `plans/` 目录
    - 技术文档: 放在 `docs/` 目录 (如有)
    - 示例代码: 放在 `examples/` 目录
    - 样本清单: 放在 `samples/` 目录 (SAMPLE_URLS.md, SAMPLES.md)
    - 测试数据: 放在 `data/` 目录
- **历史遗留文件**: 如果根目录已存在其他文件 (如 `H264_IMPROVEMENT_PLAN.md`), 应逐步迁移到对应目录.

## 6. Rust 编码规范

### 6.1 类型与安全

- **必须**: 为所有公开函数参数和返回值定义明确的类型.
- **禁止**: 随意使用 `unwrap()` / `expect()`, 除非能确保不会 panic (如常量初始化).
- **必须**: 使用 `TaoError` / `TaoResult` 作为统一错误类型; crate 内部特定错误使用 `thiserror` 定义.
- **推荐**: 使用 `struct` 定义数据结构, 使用 `enum` 定义状态与变体, 使用 `type` 定义别名.
- **推荐**: trait 对象使用 `Box<dyn Trait>`, 泛型用于内部实现, trait 对象用于跨 crate 接口.

### 6.2 并发与 FFI

- 所有 trait (Decoder, Encoder, Demuxer, Muxer, Filter) 要求 `Send`, 以支持多线程使用.
- FFI 导出函数中禁止 panic; 必须使用 `catch_unwind` 包装或确保无 panic 路径.
- FFI 函数的 `unsafe` 块必须添加 `// SAFETY:` 注释说明安全前提.

### 6.3 格式化

- 代码格式化使用 `rustfmt`, 配置见 `.rustfmt.toml`.
- 行宽上限 100 字符, 缩进 4 空格, 不使用 tab.

### 6.4 枚举设计

- 编解码器 ID, 像素格式, 采样格式等枚举使用 `#[non_exhaustive]`, 以便后续扩展.
- 枚举变体命名使用 PascalCase, 与 Rust 惯例一致.

## 7. 错误处理

- 所有 I/O 操作 (文件, 网络) 必须处理错误, 禁止吞错.
- 使用 `TaoError` 枚举覆盖所有错误场景 (Io, Codec, Format, Eof, NeedMoreData 等).
- 编解码器和格式处理中遇到的损坏数据应返回 `TaoError::InvalidData`, 不得 panic.
- 未实现的功能返回 `TaoError::NotImplemented`, 不使用 `todo!()` 宏 (会 panic).

## 8. 开发规则

### 8.1 新增编解码器

- 每个编解码器在 `tao-codec/src/` 下创建独立子模块 (如 `decoders/h264/`, `encoders/aac/`).
- 实现 `Decoder` 或 `Encoder` trait.
- 提供工厂函数并注册到 `CodecRegistry`.
- 编写单元测试验证基本编解码流程.

### 8.2 新增容器格式

- 每个容器格式在 `tao-format/src/` 下创建独立子模块 (如 `demuxers/mp4/`, `muxers/wav/`).
- 实现 `Demuxer` 或 `Muxer` trait.
- 实现 `FormatProbe` trait 以支持自动格式识别.
- 提供工厂函数并注册到 `FormatRegistry`.

### 8.3 FFI 规则

- FFI 函数签名变更须向后兼容, 不得删除已发布的导出函数.
- 新增导出函数须同步更新 C 头文件 (后续提供自动生成工具).
- 所有指针参数必须检查 null.

## 9. 日志规范

### 9.1 基本原则

- 日志使用 `tracing` crate (`error!`, `warn!`, `info!`, `debug!`, `trace!`)
- 日志后端使用 `tracing-subscriber` 和 `tracing-appender`
- 库 crate (tao-core, tao-codec 等) 只使用 `tracing` 宏, 不初始化日志后端
- 可执行文件 (tao-cli, tao-probe, tao-play) 负责初始化日志系统
- 日志内容使用中文

### 9.2 日志模块位置

- 日志初始化模块位于 `src/logging/`
- 包含两个文件:
    - `mod.rs` - 日志初始化和配置
    - `task.rs` - 日志维护任务 (日志切换、清理、压缩)

### 9.3 日志输出规则

**控制台输出**:

- 始终固定为 debug 级别
- 输出到 stdout, 带颜色输出 (ANSI)
- 过滤规则: `debug`

**文件输出**:

- 可通过命令行参数、环境变量等改变日志过滤级别
- 文件级别通过 `LoggingConfig.level` 配置
- 无颜色输出 (纯文本)
- 支持按日期自动切换日志文件
- 支持历史日志压缩和自动清理

### 9.4 日志文件管理

**日志目录**:

- 所有日志文件存放在项目根目录 `logs/` 目录下
- `logs/` 目录在 Git 中只保留 `.gitkeep` 文件
- 所有 `*.log` 文件都被 `.gitignore` 忽略, 不提交到 Git

**日志文件命名**:

- 格式: `{file_prefix}.{YYYY-MM-DD}.log`
- 示例: `tao.2026-02-16.log`, `tao-probe.2026-02-16.log`

**文件前缀规范**:

- tao-cli: 使用 `file_prefix = "tao"`
- tao-probe: 使用 `file_prefix = "tao-probe"`
- tao-play: 使用 `file_prefix = "tao-play"`

**日志维护**:

- 自动按日期切换日志文件 (每日凌晨)
- 可配置历史日志保留天数 (默认 30 天)
- 可配置是否压缩历史日志 (默认开启, 生成 `.gz` 文件)
- 定期清理过期日志 (可配置清理间隔)

### 9.5 日志级别使用

关键操作必须有日志记录:

- `error!`: 致命错误, 无法继续处理
- `warn!`: 可恢复错误, 损坏但可跳过的数据
- `info!`: 打开文件, 识别格式, 开始/完成转码
- `debug!`: 流信息, 编解码器参数, 数据包细节
- `trace!`: 详细的调试信息, 性能追踪

### 9.6 AI 调试规范

当需要调试代码时:

1. **优先查看日志文件而非控制台输出**
2. 日志文件位于 `logs/{file_prefix}.{date}.log`
3. 调试前可以删除对应的日志文件, 避免历史日志污染
4. 示例: 删除 `logs/tao.2026-02-16.log` 重新运行程序生成新日志
5. 通过日志文件分析程序执行流程和错误原因
6. 减少频繁读取控制台输出, 提高调试效率

## 10. 安全规范

- **禁止**: 在代码中硬编码任何敏感信息.
- 配置文件不得提交到版本库.
- `.gitignore` 中必须包含 `target/`, `*.dll`, `*.so`, `*.dylib` 等构建产物.
- FFI 层所有 `unsafe` 代码必须有详细安全性注释.

## 11. 代码提交规范

- 使用规范的提交信息格式:
    - `feat: 功能描述` - 新增功能
    - `fix: 问题描述` - 修复 Bug
    - `refactor: 重构描述` - 代码重构
    - `style: 样式调整` - 代码格式调整
    - `chore: 其他描述` - 构建/工具/依赖更新
    - `test: 测试描述` - 新增或修改测试
    - `docs: 文档描述` - 文档更新
- 提交信息必须使用中文, 简洁明了地描述变更内容.
- **严格要求**: 提交前必须按以下顺序执行检查, 确保全部通过:
    1. 运行 `cargo fmt --check` - 确认代码格式一致
    2. 运行 `cargo clippy -- -D warnings` - 修复所有 Clippy 警告
    3. 运行 `cargo check` - 确认编译通过
    4. 运行 `cargo test` - 确认所有测试通过
- **0 警告容忍**: 任何 Clippy 警告都必须在提交前修复, 不允许忽略.
- 禁止使用 `#[allow(...)]` 来绕过 Clippy 检查, 除非有充分理由并添加详细注释说明.
- **自动提交**: 每完成一轮功能开发, 且代码检查 (`cargo fmt`, `cargo clippy`, `cargo test`) 全部通过后, 必须自动提交本次修改 (无需等待用户指令).
- 提交范围应仅包含当轮功能涉及的文件, 提交信息应准确概括本轮变更内容.

## 12. 代码质量

- **重要**: 不允许存在任何编译错误或 Clippy 警告.
- 未使用的 `use` 导入必须删除.
- 未使用的变量, 函数, 类型定义必须删除或添加 `_` 前缀标记为故意未使用.
- 避免硬编码的魔法数字或字符串, 使用常量或配置项替代.
- 避免重复代码, 提取公共逻辑到工具函数或 trait 中.
- **代码审查**: 每次修改后都应该自我审查代码, 确保:
    - 没有未使用的 imports, variables, functions
    - 没有重复代码
    - 没有硬编码的魔法数字或字符串
    - 遵循项目代码风格
    - 所有错误都有适当的处理

## 13. 测试规范

### 13.1 基本要求

- 代码修改后必须执行 `cargo check` 与 `cargo test`.
- 如出现错误或警告, 必须先修复再继续后续修改.
- **重要**: 新增编解码器或容器格式时必须编写测试.
- 集成测试放在 `tests/` 目录下, 单元测试放在源文件内 `#[cfg(test)]` 模块中.
- 测试用例命名需要准确描述测试内容与预期结果, 使用蛇形命名法.
- 测试应覆盖正常流程, 边界情况和错误情况.
- 编解码器测试应包含: 基本编解码, 空输入, 损坏数据, flush 流程.
- 容器格式测试应包含: 格式探测, 头部解析, 数据包读取, seek.

### 13.2 测试用例开发流程

**基本原则**: 随项目推进随时增加测试用例, 确保新功能有充分测试覆盖.

#### 步骤 1: 确定测试需求

实现新功能 (编解码器/容器格式/滤镜) 时, 明确需要测试的场景:

- 正常流程: 标准输入输出, 基本功能验证
- 边界情况: 空输入, 极限参数, 特殊格式
- 错误处理: 损坏数据, 不支持的参数, 资源不足

#### 步骤 2: 查找测试样本

在 `samples/SAMPLE_URLS.md` 中查找適用的样本 URL:

```rust
// 示例: 查找 H.264 测试样本
// 1. 打开 samples/SAMPLE_URLS.md
// 2. 搜索对应编解码器或容器格式
// 3. 复制合适的样本 URL
```

**如果找到合适样本**: 直接跳到步骤 4
**如果没有合适样本**: 继续步骤 3

#### 步骤 3: 维护样本库 (如需要)

当现有样本不满足测试需求时:

1. **查找新样本**: 访问 https://samples.ffmpeg.org/ 查找合适样本
    - 浏览样本库: https://samples.ffmpeg.org/
    - 查看完整列表: https://samples.ffmpeg.org/allsamples.txt
2. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息和编解码器
3. **添加到清单**: 在 `samples/SAMPLE_URLS.md` 对应章节添加表格行
4. **提交更新**: 提交清单更新到 Git

**流程示例:**

```bash
# 1. 找到新样本 (例如 VP9 编码器)
# 访问 https://samples.ffmpeg.org/ 或查看 allsamples.txt

# 2. 验证样本 (确认编解码器)
ffprobe https://samples.ffmpeg.org/path/to/sample.webm

# 3. 编辑样本清单
vim samples/SAMPLE_URLS.md
# 在对应章节添加表格行: | 用途 | URL | 描述 |

# 4. 提交更改
git add samples/SAMPLE_URLS.md
git commit -m "docs: 添加 XXX 编解码器样本 URL"
```

> **注意**: 所有样本 **仅作为 URL 维护**, 不下载到本地. 详见 [samples/SAMPLES.md](samples/SAMPLES.md).

#### 步骤 4: 编写测试用例

根据样本 URL 编写集成测试或单元测试:

```rust
// tests/mpeg4_pipeline.rs
use tao_format::demuxer::DemuxerRegistry;
use tao_codec::decoder::DecoderRegistry;
use tao_core::MediaType;

#[test]
fn test_mpeg4_part2_decode_basic() {
    // 1. 从 samples/SAMPLE_URLS.md 复制样本 URL
    let sample_url = "https://samples.ffmpeg.org/V-codecs/MPEG4/mpeg4_avi.avi";

    // 2. 打开解封装器
    let mut demuxer = DemuxerRegistry::open(sample_url).unwrap();

    // 3. 查找视频流
    let video_stream = demuxer.streams()
        .iter()
        .find(|s| s.media_type() == MediaType::Video)
        .unwrap();

    // 4. 创建解码器
    let mut decoder = DecoderRegistry::create_decoder(
        video_stream.codec_id(),
        video_stream.codec_params(),
    ).unwrap();

    // 5. 解码前几帧
    let mut packet_count = 0;
    while let Some(packet) = demuxer.read_packet().unwrap() {
        if packet.stream_index() == video_stream.index() {
            let frames = decoder.decode(&packet).unwrap();
            assert!(!frames.is_empty(), "应该解码出至少一帧");
            packet_count += 1;
            if packet_count > 10 { break; }  // 仅测试前几帧
        }
    }

    assert!(packet_count > 0, "应该读取到数据包");
}

// 边界测试 (可选)
#[test]
fn test_mpeg4_decode_invalid_data() {
    // 测试错误处理: 构造无效数据
    let mut params = CodecParameters::new();
    params.set_codec_id(CodecId::Mpeg4Part2);
    params.set_width(320);
    params.set_height(240);

    let mut decoder = Mpeg4Decoder::new(&params).unwrap();

    // 尝试解码损坏的数据包
    let corrupted_data = vec![0xFF; 100];
    let packet = Packet::new(corrupted_data, /* timestamp_ms */ 0);

    // 应该返回错误，不是 panic
    match decoder.send_packet(&packet) {
        Err(e) => {
            // 正确处理错误
            assert!(matches!(e, TaoError::InvalidData | TaoError::Eof));
        }
        Ok(_) => {
            // 可能成功解码空帧，也是可以接受的
        }
    }
}
```

**关键要点**:

- 所有样本 URL 来自 `samples/SAMPLE_URLS.md`
- 直接使用 URL 创建 Demuxer/Decoder，无需下载
- 测试结束自动释放网络连接，无需手动清理
- 大文件测试只解码前几帧，避免耗时过长

#### 步骤 5: 执行测试

```bash
# 运行特定测试
cargo test --test vp9_pipeline

# 运行所有测试
cargo test

# 安静模式 (只显示失败)
cargo test --quiet
```

#### 步骤 6: 提交测试代码

测试通过后提交代码:

```bash
git add tests/vp9_pipeline.rs
git commit -m "test: 添加 VP9 解码器测试用例"
```

### 13.3 测试用例编写标准

- **文件位置**: 所有测试放在 `tests/` 目录, 命名为 `{feature}_pipeline.rs`
- **测试命名**: 使用 `test_{component}_{scenario}` 格式, 如 `test_h264_decode_basic`, `test_mp4_demux_seek`
- **断言清晰**: 每个 `assert!` 都应包含失败消息, 说明预期行为
- **注释完整**: 复杂测试逻辑添加注释说明测试目的和步骤
- **样本地址**: 使用 `SAMPLE_URLS.md` 中的 HTTPS URL, 或 `data/tmp/` 中的临时文件
- **资源清理**: 临时文件必须在 `data/tmp/` 目录, 测试结束后清理

### 13.4 测试覆盖范围

每个编解码器/容器格式至少包含以下测试:

**编解码器测试**:

- ✓ 基本解码 (正常流程)
- ✓ 编码 (如果实现了编码器)
- ✓ 空输入处理
- ✓ 损坏数据处理
- ✓ Flush 流程
- ✓ 参数解析 (SPS/PPS/VPS 等)

**容器格式测试**:

- ✓ 格式探测 (Probe)
- ✓ 头部解析
- ✓ 数据包读取
- ✓ Seek 操作
- ✓ 多流处理 (音视频同时存在)
- ✓ 损坏文件处理

**滤镜测试**:

- ✓ 基本滤镜操作
- ✓ 参数验证
- ✓ 链式滤镜
- ✓ 边界条件 (分辨率, 像素格式)

## 14. 注释规范

- **必须**: 所有注释使用中文.
- 复杂逻辑必须添加注释说明.
- 公开函数和 trait 使用 `///` 文档注释, 说明功能, 参数, 返回值.
- 每个 crate 的 `lib.rs` 使用 `//!` 模块文档注释, 说明 crate 用途.
- FFI 导出函数必须同时说明安全性要求 (`# Safety` 段落).
- 特殊处理或 Workaround 必须注释说明原因.
- 临时代码或待优化代码使用 `// TODO:` 标记.

## 15. 性能优化

- 避免不必要的内存分配, 优先使用引用和借用.
- 大量数据处理使用迭代器而非收集到 `Vec` 后再遍历.
- 像素格式转换和编解码热路径应尽量避免分支预测失败.
- 考虑使用 SIMD 指令优化关键路径 (通过 `std::arch` 或 `packed_simd`).
- 帧缓冲区应支持复用, 避免每帧都重新分配内存.
- 大块数据使用 `bytes::Bytes` 实现零拷贝传递.

## 16. 手动播放测试规范

### 16.1 播放时长限制

- 手动测试音视频播放时, **禁止完整播放**整个文件.
- 默认播放 **前 10 秒** 即可验证功能, 如有必要 (如需验证 seek/后段内容) 可增加到 **最多 30 秒**.
- 播放结束后必须主动终止播放进程.

### 16.2 终止播放进程 (Windows)

- Windows 下终止 tao-play 进程时, **必须使用 `TASKKILL /F /IM tao-play.exe /T`**.
- **禁止使用 `TASKKILL /F /PID <pid>`**, 因为在 Cursor/shell 环境中通常无法获取到正确的 PID.
- 示例:
    ```powershell
    # 正确
    TASKKILL /F /IM tao-play.exe /T
    # 错误 (PID 不可靠)
    TASKKILL /F /PID 12345
    ```

### 16.3 流式播放测试

- `tao-play` 支持 http/https/rtmp 等流式 URL 播放.
- **所有测试文件均使用 URL 直接流式播放**, 不下载到本地.
- 所有样本 URL 维护在 `samples/SAMPLE_URLS.md` 中.
- 示例:

    ```powershell
    # 正确: 直接使用 URL 进行流式播放
    cargo run --package tao-play -- "https://samples.ffmpeg.org/flac/Yesterday.flac"

    # 查看更多样本 URL
    # 请参考 samples/SAMPLE_URLS.md
    ```

## 17. 测试文件和临时文件管理

> **完整规范**: 参见 [samples/SAMPLES.md](samples/SAMPLES.md) 了解详细的文件管理规则和使用示例

### 17.1 目录结构

- **`tests/`**: 项目根目录下, 包含所有测试相关代码
    - **单元测试**: 在源文件中使用 `#[cfg(test)]` 模块
    - **集成测试**: `tests/` 下的 `{feature}_pipeline.rs` 文件
    - **基准测试**: `benches/` 下的 `*.rs` 文件
- **`examples/`**: 样本使用规范和 URL 清单
    - **`samples/SAMPLES.md`**: 样本使用规范和文件管理规则
    - **`samples/SAMPLE_URLS.md`**: 测试样本 URL 清单 (所有样本使用 URL 访问)
- **`data/`**: 临时文件目录
    - **`data/.gitkeep`**: 确保 data 文件夹始终存在于 Git
    - **`data/tmp/` 和其他**: 所有临时文件 (不提交到 Git)

### 17.2 文件放置规则

- **测试代码文件**: 全部放在 `tests/` 目录下
    - 集成测试: `tests/{feature}_pipeline.rs`
    - 单元测试: 在 `crates/` 各 crate 的源文件中使用 `#[cfg(test)]` 模块
    - 测试命名: `test_{component}_{scenario}` 格式
- **测试样本**: 使用 `samples/SAMPLE_URLS.md` 中的 HTTPS URL
    - 所有样本来源: https://samples.ffmpeg.org/
    - 所有样本使用 URL 标识, 无需本地下载
    - 直接使用 URL 创建 Demuxer/Decoder
- **临时文件**: 必须放在 `data/` 目录下 (如 `data/tmp/`, `data/ffmpeg/` 等)
    - 运行时生成的临时文件
    - 编解码过程中的中间文件
    - 永不提交到 Git (整体 /data 已被 .gitignore)

### 17.3 测试样本 URL 规范

- **样本源**: 优先使用 https://samples.ffmpeg.org/ 提供的公开测试样本
- **样本类别**: 该库包含多种格式的样本:
    - 视频: H.264, H.265, VP8, VP9, AV1, MPEG4 Part 2, Theora, ProRes 等
    - 音频: AAC, MP3, FLAC, Opus, Vorbis, WAV, ALAC 等
    - 容器: MP4, MKV, WebM, OGG, AVI, TS 等
- **使用方式**:
    - **所有样本使用 URL 方式访问, 不下载到本地**
    - 直接使用 HTTPS URL 创建 Demuxer/Decoder
    - 完整 URL 格式: `https://samples.ffmpeg.org/<category>/<filename>`
    - 示例: `https://samples.ffmpeg.org/HDTV/Channel9_HD.ts`
- **版本管理**:
    - 所有样本 URL 记录在 `samples/SAMPLE_URLS.md` 中
    - 添加新样本时更新清单并提交到 Git
    - URL 失效时从 https://samples.ffmpeg.org/ 查找替代样本

### 17.4 临时文件管理

- **创建**: 所有临时文件必须在 `data/` 目录下创建
- **清理**: 测试结束后必须清理临时文件
- **命名**: 临时文件使用前缀 `tmp_` 或进程 ID 命名
- **权限**: 确保临时文件有适当的读写权限
- **Git**: 永不提交到版本控制

### 17.5 Git 管理

- **`samples/SAMPLE_URLS.md`**: 测试样本 URL 清单, 提交到 Git
- **`samples/SAMPLES.md`**: 样本使用规范文档, 提交到 Git
- **`data/`**: 整体忽略，存放所有临时文件（仅保留 `.gitkeep`）
- **`tests/`**: 所有测试代码, 提交到 Git

### 17.6 代码规范

- **测试文件位置**: 所有测试代码放在根目录 `tests/` 中
- **样本 URL**: 从 `samples/SAMPLE_URLS.md` 复制合适的 HTTPS URL
- **临时文件路径**: 使用相对于项目根目录的路径 `data/...`
- **错误处理**: 文件或 URL 不可访问时提供清晰的错误信息
- **跨平台**: 确保路径处理在 Windows/Linux/macOS 上兼容

### 17.7 新增测试样本

当需要新的测试样本时:

1. **查找样本**: 访问 https://samples.ffmpeg.org/ 浏览或搜索合适样本
2. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息
3. **添加 URL**: 在 `samples/SAMPLE_URLS.md` 对应章节添加 URL 和说明
4. **更新规范**: 参考 [samples/SAMPLES.md](samples/SAMPLES.md) 中的流程
5. **提交更改**: git add samples/SAMPLE_URLS.md && git commit -m "docs: 添加 XXX 样本 URL"

### 17.8 持续维护

随着项目推进, 需要持续维护测试样本和规范:

- **新增编解码器**:
    - 在 `samples/SAMPLE_URLS.md` 中添加样本 URL
    - 在 `tests/{codec}_pipeline.rs` 中编写测试
    - 参考 §13.2 测试用例开发流程
- **新增滤镜**:
    - 在 `samples/SAMPLE_URLS.md` 中添加样本 URL
    - 在 `tests/filter_*.rs` 中编写测试
- **性能测试**:
    - 在 `benches/` 中编写基准测试
    - 在 `samples/SAMPLE_URLS.md` 中记录大文件样本 URL
- **维护检查**:
    - 定期检查 FFmpeg 官方样本库更新 (每季度)
    - 验证 `samples/SAMPLE_URLS.md` 中的 URL 是否有效
    - 更新过期或失效的 URL

详见 [samples/SAMPLES.md](samples/SAMPLES.md) 了解更多资源管理规范。
