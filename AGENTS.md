# 项目开发规范

> 本文件是项目唯一的规范来源, 所有 AI 工具 (Cursor, Copilot, Codex, Claude Code 等) 均以此为准.

---

## 1. 项目概述

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架, 目标是全功能复刻 FFmpeg. 项目提供三种使用方式:

1. **Rust 库**: 其他 Rust 项目可通过 `tao` crate 直接调用
2. **C FFI (DLL/SO)**: 通过 `tao-ffi` crate 导出 C 兼容接口, 供 C/C++ 等语言调用
3. **命令行工具**: `tao` (对标 ffmpeg) 和 `tao-probe` (对标 ffprobe) 可执行文件

## 2. 项目结构

本项目采用 Cargo Workspace 多 crate 架构:

```
tao/
├── Cargo.toml              # Workspace 根配置 + 门面库 (re-export 所有子 crate)
├── RULES.md                # 本规范文件
├── .rustfmt.toml           # 代码格式化配置
├── .gitignore
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
│   └── tao-probe/          # tao-probe 探测工具 (对标 ffprobe)
├── tests/                  # 集成测试
└── examples/               # 使用示例
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

- 日志使用 `log` crate 的标准宏 (`error!`, `warn!`, `info!`, `debug!`, `trace!`).
- 库 crate (tao-core, tao-codec 等) 只使用 `log` 宏, 不初始化日志后端.
- 可执行文件 (tao-cli, tao-probe) 负责初始化日志后端 (使用 `env_logger`).
- 日志内容使用中文, 关键操作必须有日志记录:
  - `info!`: 打开文件, 识别格式, 开始/完成转码
  - `debug!`: 流信息, 编解码器参数, 数据包细节
  - `warn!`: 可恢复错误, 损坏但可跳过的数据
  - `error!`: 致命错误, 无法继续处理

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

- 代码修改后必须执行 `cargo check` 与 `cargo test`.
- 如出现错误或警告, 必须先修复再继续后续修改.
- **重要**: 新增编解码器或容器格式时必须编写测试.
- 集成测试放在 `tests/` 目录下, 单元测试放在源文件内 `#[cfg(test)]` 模块中.
- 测试用例命名需要准确描述测试内容与预期结果, 使用蛇形命名法.
- 测试应覆盖正常流程, 边界情况和错误情况.
- 编解码器测试应包含: 基本编解码, 空输入, 损坏数据, flush 流程.
- 容器格式测试应包含: 格式探测, 头部解析, 数据包读取, seek.

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
