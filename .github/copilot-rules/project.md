# 项目概述、结构和核心架构

> 本文件定义 Tao 项目的整体架构、目录结构和代码组织规范。

---

## 1. 项目概述

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架,目标是全功能复刻 FFmpeg。

### 三种使用方式

1. **Rust 库** - 其他 Rust 项目可通过 `tao` crate 直接调用
2. **C FFI (DLL/SO)** - 通过 `tao-ffi` crate 导出 C 兼容接口,供 C/C++ 等语言调用
3. **命令行工具**:
    - `tao` - 音视频转换工具(对标 ffmpeg)
    - `tao-probe` - 媒体信息探测工具(对标 ffprobe)
    - `tao-play` - 多媒体播放器(对标 ffplay)

---

## 2. 项目结构

本项目采用 **Cargo Workspace 多 crate 架构**:

```
tao/
├── Cargo.toml              # Workspace 根配置 + 门面库
├── .cursor/rules/          # Cursor AI 规则文件(16个 .mdc 文件)
├── .github/                # GitHub Copilot 规则文件
│   ├── copilot-instructions.md  # 主指令文件
│   └── copilot-rules/      # 详细规则模块(本目录)
├── plans/                  # AI 执行计划文件存放目录
├── samples/                # 测试样本清单
│   ├── SAMPLE_URLS.md      # 样本 URL 清单
│   └── SAMPLES.md          # 样本使用规范
├── data/                   # 临时文件目录(不提交 Git)
├── logs/                   # 日志文件目录(不提交 Git)
├── src/
│   ├── lib.rs              # 门面库入口,重导出所有子 crate
│   └── logging/            # 日志模块
│       ├── mod.rs          # 日志初始化和配置
│       └── task.rs         # 日志维护任务
├── crates/                 # 库 crate(对标 FFmpeg 各子库)
│   ├── tao-core/           # 核心类型与工具(对标 libavutil)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs          # 统一错误类型
│   │       ├── rational.rs       # 有理数(时间基/帧率/宽高比)
│   │       ├── media_type.rs     # 媒体类型(Video/Audio/Subtitle)
│   │       ├── pixel_format.rs   # 像素格式(YUV420P, RGB24 等)
│   │       ├── sample_format.rs  # 音频采样格式(S16, F32 等)
│   │       ├── channel_layout.rs # 声道布局(Mono/Stereo/5.1 等)
│   │       ├── timestamp.rs      # 时间戳与时间基转换
│   │       └── color/            # 色彩空间/范围/原色/传递特性
│   ├── tao-codec/          # 编解码器框架(对标 libavcodec)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── codec_id.rs       # 编解码器标识符
│   │       ├── decoder.rs        # 解码器 trait
│   │       ├── encoder.rs        # 编码器 trait
│   │       ├── frame.rs          # 解码帧(VideoFrame/AudioFrame)
│   │       ├── packet.rs         # 压缩数据包
│   │       ├── registry.rs       # 编解码器注册表
│   │       ├── decoders/         # 各种解码器实现
│   │       ├── encoders/         # 各种编码器实现
│   │       └── parsers/          # 比特流解析器
│   ├── tao-format/         # 容器格式框架(对标 libavformat)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── format_id.rs      # 格式标识符
│   │       ├── demuxer.rs        # 解封装器 trait
│   │       ├── muxer.rs          # 封装器 trait
│   │       ├── stream.rs         # 流信息
│   │       ├── io.rs             # I/O 抽象层
│   │       ├── probe.rs          # 格式探测
│   │       ├── registry.rs       # 格式注册表
│   │       ├── demuxers/         # 各种解封装器实现
│   │       └── muxers/           # 各种封装器实现
│   ├── tao-filter/         # 滤镜框架(对标 libavfilter)
│   ├── tao-scale/          # 图像缩放(对标 libswscale)
│   ├── tao-resample/       # 音频重采样(对标 libswresample)
│   └── tao-ffi/            # C FFI 导出层(cdylib + staticlib)
├── bins/                   # 可执行文件 crate
│   ├── tao-cli/            # tao 命令行工具(对标 ffmpeg)
│   ├── tao-probe/          # tao-probe 探测工具(对标 ffprobe)
│   └── tao-play/           # 播放器(对标 ffplay)
├── tests/                  # 集成测试
├── examples/               # 使用示例(crate 调用示例)
└── benches/                # 性能基准测试
```

---

## 3. crate 依赖关系

```
tao-core (无外部依赖,最底层)
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
tao-play (依赖 tao-core, tao-codec, tao-format)
  ↑
tao (门面库,re-export 所有库 crate)
```

### 依赖原则

- **tao-core**: 最底层,不依赖任何其他 tao crate,仅依赖基础库(thiserror, tracing 等)
- **中间层 crate**: 依赖 tao-core 和必要的同层 crate
- **FFI 和可执行文件**: 依赖所需的库 crate
- **tao 门面库**: 重导出所有库 crate,方便外部使用

---

## 4. 核心架构

### 4.1 编解码管线

```
[输入文件] → Demuxer → Packet → Decoder → Frame → Filter → Encoder → Packet → Muxer → [输出文件]
             (解封装)   (压缩包) (解码)     (原始帧) (滤镜)   (编码)   (压缩包) (封装)
```

**流程说明:**

1. **Demuxer(解封装器)** - 从容器格式中提取压缩数据包(Packet)
2. **Decoder(解码器)** - 将压缩数据包解码为原始帧(Frame)
3. **Filter(滤镜)** - 对原始帧进行处理(缩放/裁剪/调色等)
4. **Encoder(编码器)** - 将原始帧编码为压缩数据包
5. **Muxer(封装器)** - 将压缩数据包写入容器格式

### 4.2 注册表模式

编解码器和容器格式均采用**注册表(Registry)模式**:

**核心思想:**

- 各编解码器/格式通过工厂函数注册到全局注册表
- 运行时按 `CodecId` / `FormatId` 查找并创建实例
- 支持按优先级选择多个同 ID 的实现

**示例:**

```rust
// 注册 H.264 解码器
pub fn register_h264_decoder() {
    CodecRegistry::register(
        CodecId::H264,
        DecoderFactory::new(|params| Ok(Box::new(H264Decoder::new(params)?))),
    );
}

// 运行时创建解码器
let decoder = CodecRegistry::create_decoder(CodecId::H264, &params)?;
```

### 4.3 I/O 抽象

`IoContext` 提供统一的读写接口,支持多种后端:

- **FileBackend** - 文件读写(已实现)
- **MemoryBackend** - 内存缓冲区(待实现)
- **NetworkBackend** - 网络流(HTTP/HTTPS/RTMP 等,待实现)

**优势:**

- 解封装器和封装器无需关心数据来源
- 支持流式处理,无需完整下载
- 便于测试(使用内存后端)

### 4.4 FFI 导出

`tao-ffi` crate 编译为 `cdylib` + `staticlib`,提供 C 兼容接口:

**规范:**

- 所有导出函数以 `tao_` 前缀命名
- 使用 `#[no_mangle]` 和 `extern "C"` 确保 ABI 兼容
- 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 释放
- 所有指针参数必须检查 null
- 禁止 panic,使用 `catch_unwind` 包装

---

## 5. 代码组织规范

### 5.1 模块化原则

- ✅ 确保项目模块化,避免单一函数过于复杂
- ✅ **建议单个函数不超过 50 行**
- ✅ 按功能分类组织代码结构,遵循现有的 crate 划分
- ✅ 复杂模块应拆分为多个子模块,每个子模块职责单一
- ✅ 公共类型集中定义在对应 crate 中,避免散落
- ✅ 新增编解码器实现应放在 `tao-codec` 下对应子目录中
- ✅ 新增容器格式实现应放在 `tao-format` 下对应子目录中

### 5.2 执行计划管理

**必须**: AI 在制定执行计划时,必须将计划文件写入 `plans/` 目录。

**命名规范**: 计划文件命名格式为 `{功能模块}_{任务描述}.md`

示例: `h264_decoder_improvement.md`, `mkv_demuxer_implementation.md`

**计划内容**:

- 任务背景和目标
- 详细执行步骤(带编号)
- 每步的预期产出
- 依赖项和前置条件
- 验收标准

**断点续执行**: 计划文件应支持断点续执行,AI 应在计划中标记已完成的步骤。

**跨 AI 协作**: 计划文件应足够详细,使得不同 AI 工具可以基于同一计划继续执行。

### 5.3 根目录文件管理

**严格禁止**: 不允许在项目根目录下随意创建新文件。

**允许的根目录文件**:

- 项目配置: `Cargo.toml`, `.rustfmt.toml`, `.gitignore`
- 核心文档: `README.md`
- License 文件: `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`

**其他文件的存放位置**:

- 执行计划 → `plans/` 目录
- 技术文档 → `docs/` 目录(如有)
- 示例代码 → `examples/` 目录
- 样本清单 → `samples/` 目录(`SAMPLE_URLS.md`, `SAMPLES.md`)
- 测试数据 → `data/` 目录(临时文件,不提交 Git)
- 日志文件 → `logs/` 目录(不提交 Git)

**历史遗留文件**: 如果根目录已存在其他文件,应逐步迁移到对应目录。

---

## 6. Git 管理

### 不提交到 Git 的目录

以下目录在 `.gitignore` 中忽略:

- `target/` - Cargo 构建产物
- `data/` - 临时文件和测试数据(仅保留 `.gitkeep`)
- `logs/` - 运行时日志文件(仅保留 `.gitkeep`)
- `*.dll`, `*.so`, `*.dylib` - 动态库文件
- 编辑器临时文件

### 必须提交的文件

- 所有源代码(`src/`, `crates/`, `bins/`, `tests/`, `examples/`, `benches/`)
- 项目配置(`Cargo.toml`, `.rustfmt.toml`, `.gitignore`)
- 文档(`README.md`, `samples/SAMPLE_URLS.md`, `samples/SAMPLES.md`)
- 规则文件(`.cursor/rules/`, `.github/`)
- 执行计划(`plans/`)
- 目录占位符(`data/.gitkeep`, `logs/.gitkeep`)

---

## 总结

Tao 项目采用清晰的多 crate 架构,每个 crate 职责单一、依赖明确。通过注册表模式实现编解码器和格式的插件化管理,通过 I/O 抽象支持多种数据源。严格的代码组织和文件管理规范确保项目的长期可维护性。
