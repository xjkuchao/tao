# AGENTS 规则文件

本文件内容由 `.cursor/rules` 下规则文件原文拼接生成, 不做语义改写.

---

## .cursor/rules/00_index.mdc

---
description: Tao 项目开发规范索引
alwaysApply: true
---

# Tao 项目开发规范

> 本项目的开发规范已模块化为多个专注的规则文件, 所有 AI 工具 (Cursor, Copilot, Codex, Claude Code 等) 均需遵循.

## 规则文件索引

规则文件按主题组织在 `.cursor/rules/` 目录下:

### 核心规范

- **01_project_overview.mdc** - 项目概述、结构和核心架构
- **02_language.mdc** - 语言规范（必须使用中文）
- **03_code_organization.mdc** - 代码组织、执行计划和文件管理

### Rust 开发规范

- **04_rust_coding.mdc** - Rust 编码规范、类型安全、并发和格式化
- **05_error_handling.mdc** - 错误处理规范
- **06_development_rules.mdc** - 开发规则（编解码器、容器格式、FFI）

### 质量与规范

- **07_logging.mdc** - 日志规范
- **08_security.mdc** - 安全规范
- **09_commits.mdc** - 代码提交规范
- **10_code_quality.mdc** - 代码质量要求

### 测试规范

- **11_testing.mdc** - 测试规范和用例开发流程
- **14_manual_testing.mdc** - 手动播放测试规范
- **15_data_management.mdc** - 测试文件和临时文件管理

### 其他规范

- **12_comments.mdc** - 注释规范
- **13_performance.mdc** - 性能优化

## 使用说明

- Cursor 会自动加载 `.cursor/rules/` 下的规则文件
- 带有 `alwaysApply: true` 的规则始终生效
- 其他规则根据文件类型和上下文自动应用
- 规则内容在 `.cursor/rules/` 与 `.github/` 中维护

## 规范更新

当需要更新规范时:

1. 同时更新对应的 `.cursor/rules/*.mdc` 文件和 `.github/` 规则文件
2. 确保两者保持一致
3. 提交时说明规范变更内容

---

## .cursor/rules/01_project_overview.mdc

---
description: 项目概述、结构和核心架构
globs:
    - "**/*.rs"
    - "**/*.toml"
    - "**/*.md"
---

# 项目概述、结构和核心架构

## 1. 项目概述

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架, 目标是全功能复刻 FFmpeg. 项目提供三种使用方式:

1. **Rust 库**: 其他 Rust 项目可通过 `tao` crate 直接调用
2. **C FFI (DLL/SO)**: 通过 `tao-ffi` crate 导出 C 兼容接口, 供 C/C++ 等语言调用
3. **命令行工具**: `tao` (对标 ffmpeg), `tao-probe` (对标 ffprobe) 和 `tao-play` (对标 ffplay) 可执行文件

## 2. 项目结构

本项目采用 Cargo Workspace 多 crate 架构:

```
tao/
├── Cargo.toml              # Workspace 根配置 + 门面库
├── .cursor/rules/          # 模块化规则文件
├── plans/                  # AI 执行计划文件
├── samples/                # 测试样本清单
├── data/                   # 临时文件目录 (不提交)
├── crates/                 # 库 crate
│   ├── tao-core/           # 核心类型与工具 (对标 libavutil)
│   ├── tao-codec/          # 编解码器框架 (对标 libavcodec)
│   ├── tao-format/         # 容器格式框架 (对标 libavformat)
│   ├── tao-filter/         # 滤镜框架 (对标 libavfilter)
│   ├── tao-scale/          # 图像缩放 (对标 libswscale)
│   ├── tao-resample/       # 音频重采样 (对标 libswresample)
│   └── tao-ffi/            # C FFI 导出层
├── bins/                   # 可执行文件 crate
│   ├── tao-cli/            # tao 命令行工具 (对标 ffmpeg)
│   ├── tao-probe/          # tao-probe 探测工具 (对标 ffprobe)
│   └── tao-play/           # 播放器 (对标 ffplay)
├── tests/                  # 集成测试
└── examples/               # 使用示例 (crate 调用示例)
```

### crate 依赖关系

```
tao-core (无外部依赖, 最底层)
  ↑
tao-codec, tao-format, tao-filter, tao-scale, tao-resample
  ↑
tao-ffi, tao-cli, tao-probe, tao-play
  ↑
tao (门面库, re-export 所有库 crate)
```

## 3. 核心架构

### 编解码管线

```
[输入文件] → Demuxer → Packet → Decoder → Frame → [Filter] → Encoder → Packet → Muxer → [输出文件]
```

### 注册表模式

编解码器和容器格式均采用注册表 (Registry) 模式:

- 各编解码器/格式通过工厂函数注册到全局注册表
- 运行时按 CodecId/FormatId 查找并创建实例
- 支持按优先级选择多个同 ID 的实现

### I/O 抽象

IoContext 提供统一的读写接口, 支持多种后端:

- 文件 (FileBackend)
- 内存缓冲区 (MemoryBackend, 待实现)
- 网络流 (NetworkBackend, 待实现)

### FFI 导出

tao-ffi crate 编译为 cdylib + staticlib:

- 所有导出函数以 `tao_` 前缀命名
- 使用 `#[no_mangle]` 和 `extern "C"` 确保 ABI 兼容
- 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 释放

---

## .cursor/rules/02_language.mdc

---
description: 语言规范 - 项目全部使用中文
alwaysApply: true
---

# 语言规范

## 基本要求

- **重要**: 项目全部使用中文, 包括:
    - 代码注释
    - 控制台日志
    - 错误信息
    - AI 上下文输出
    - 文档内容
- 所有开发过程中的交流和文档必须使用中文
- 标点符号使用英文标点

## 命名规范

- 代码标识符（变量、函数、类型等）使用英文，遵循 Rust 命名惯例
- 注释、文档字符串、错误消息等使用中文
- 提交信息使用中文

---

## .cursor/rules/03_code_organization.mdc

---
description: 代码组织、执行计划管理和文件管理规范
globs:
    - "**/*.rs"
    - "**/*.md"
---

# 代码组织与文件管理

## 代码组织原则

- 确保项目模块化, 避免单一函数过于复杂 (建议单个函数不超过 50 行)
- 按功能分类组织代码结构, 遵循现有的 crate 划分和目录结构
- 复杂模块应拆分为多个子模块, 每个子模块职责单一
- 公共类型集中定义在对应 crate 中, 避免散落
- 新增编解码器实现应放在 `tao-codec` 下对应子目录中
- 新增容器格式实现应放在 `tao-format` 下对应子目录中

## 执行计划管理

- **必须**: AI 在制定执行计划时, 必须将计划文件写入 `plans/` 目录
- **命名规范**: 计划文件命名格式为 `{功能模块}_{任务描述}.md`
    - 示例: `h264_decoder_improvement.md`
- **计划内容**: 计划文件应包含:
    - 任务背景和目标
    - 详细执行步骤 (带编号)
    - 每步的预期产出
    - 依赖项和前置条件
    - 验收标准
- **断点续执行**: 计划文件应支持断点续执行, AI 应在计划中标记已完成的步骤
- **跨 AI 协作**: 计划文件应足够详细, 使得不同 AI 工具可以基于同一计划继续执行

## 根目录文件管理

- **严格禁止**: 不允许在项目根目录下随意创建新文件

### 允许的根目录文件

- 项目配置: `Cargo.toml`, `.rustfmt.toml`, `.gitignore`
- 核心文档: `README.md`
- License 文件: `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`

### 文件存放位置

- 执行计划: 必须放在 `plans/` 目录
- 技术文档: 放在 `docs/` 目录 (如有)
- 示例代码: 放在 `examples/` 目录
- 样本清单: 放在 `samples/` 目录 (SAMPLE_URLS.md, SAMPLES.md)
- 测试数据: 放在 `data/` 目录
- 历史遗留文件: 应逐步迁移到对应目录

---

## .cursor/rules/04_rust_coding.mdc

---
description: Rust 编码规范 - 类型安全、并发、格式化
globs:
    - "**/*.rs"
---

# Rust 编码规范

## 类型与安全

- **必须**: 为所有公开函数参数和返回值定义明确的类型
- **禁止**: 随意使用 `unwrap()` / `expect()`, 除非能确保不会 panic (如常量初始化)
- **必须**: 使用 `TaoError` / `TaoResult` 作为统一错误类型
    - crate 内部特定错误使用 `thiserror` 定义
- **推荐**:
    - 使用 `struct` 定义数据结构
    - 使用 `enum` 定义状态与变体
    - 使用 `type` 定义别名
- **推荐**:
    - trait 对象使用 `Box<dyn Trait>`
    - 泛型用于内部实现
    - trait 对象用于跨 crate 接口

## 并发与 FFI

- 所有 trait (Decoder, Encoder, Demuxer, Muxer, Filter) 要求 `Send`, 以支持多线程使用
- FFI 导出函数中禁止 panic
    - 必须使用 `catch_unwind` 包装或确保无 panic 路径
- FFI 函数的 `unsafe` 块必须添加 `// SAFETY:` 注释说明安全前提

## 格式化

- 代码格式化使用 `rustfmt`, 配置见 `.rustfmt.toml`
- 行宽上限 100 字符
- 缩进 4 空格, 不使用 tab

## 枚举设计

- 编解码器 ID, 像素格式, 采样格式等枚举使用 `#[non_exhaustive]`, 以便后续扩展
- 枚举变体命名使用 PascalCase, 与 Rust 惯例一致

---

## .cursor/rules/05_error_handling.mdc

---
description: 错误处理规范
globs:
    - "**/*.rs"
---

# 错误处理规范

## 基本原则

- 所有 I/O 操作 (文件, 网络) 必须处理错误, 禁止吞错
- 使用 `TaoError` 枚举覆盖所有错误场景:
    - `Io` - I/O 错误
    - `Codec` - 编解码错误
    - `Format` - 格式错误
    - `Eof` - 文件结束
    - `NeedMoreData` - 需要更多数据
    - `InvalidData` - 无效数据
    - `NotImplemented` - 未实现的功能

## 特定场景处理

- 编解码器和格式处理中遇到的损坏数据应返回 `TaoError::InvalidData`, 不得 panic
- 未实现的功能返回 `TaoError::NotImplemented`, 不使用 `todo!()` 宏 (会 panic)
- 所有错误信息必须使用中文
- 错误信息应提供足够的上下文信息, 便于调试

---

## .cursor/rules/06_development_rules.mdc

---
description: 开发规则 - 编解码器、容器格式、FFI
globs:
    - "crates/tao-codec/**/*.rs"
    - "crates/tao-format/**/*.rs"
    - "crates/tao-ffi/**/*.rs"
---

# 开发规则

## 新增编解码器

- 每个编解码器在 `tao-codec/src/` 下创建独立子模块
    - 示例: `decoders/h264/`, `encoders/aac/`
- 实现 `Decoder` 或 `Encoder` trait
- 提供工厂函数并注册到 `CodecRegistry`
- 编写单元测试验证基本编解码流程

## 新增容器格式

- 每个容器格式在 `tao-format/src/` 下创建独立子模块
    - 示例: `demuxers/mp4/`, `muxers/wav/`
- 实现 `Demuxer` 或 `Muxer` trait
- 实现 `FormatProbe` trait 以支持自动格式识别
- 提供工厂函数并注册到 `FormatRegistry`

## FFI 规则

- FFI 函数签名变更须向后兼容, 不得删除已发布的导出函数
- 新增导出函数须同步更新 C 头文件 (后续提供自动生成工具)
- 所有指针参数必须检查 null
- 所有导出函数必须使用 `#[no_mangle]` 和 `extern "C"`
- 导出函数以 `tao_` 前缀命名

---

## .cursor/rules/07_logging.mdc

---
description: 日志规范
globs:
    - "**/*.rs"
---

# 日志规范

## 基本原则

- 日志使用 `tracing` crate: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- 日志后端使用 `tracing-subscriber` 和 `tracing-appender`
- 库 crate (tao-core, tao-codec 等) 只使用 `tracing` 宏, 不初始化日志后端
- 可执行文件 (tao-cli, tao-probe, tao-play) 负责初始化日志系统
- 日志内容使用中文

## 日志模块位置

- 根 crate 日志模块位于 `src/logging/` (含 tokio 维护任务)
- 各 bin 的轻量日志模块位于 `bins/{name}/src/logging.rs`
  - 不依赖 tokio, 使用 `tracing-appender` 按日滚动
  - 支持 `-v` / `-vv` / `-vvv` 命令行参数和 `TAO_LOG` 环境变量

## 日志级别体系 (重要)

### 命令行级别映射

| verbosity | 参数 | 过滤规则 | 用途 |
|-----------|------|----------|------|
| 0 (默认) | 无 | `info` | 关键生命周期事件 |
| 1 | `-v` | `debug` | 内部状态/决策 |
| 2 | `-vv` | tao crate=`trace`, 第三方=`info` | 项目详细数据流 |
| 3+ | `-vvv` | 全局 `trace` | 含第三方依赖的完整追踪 |

- Console 和 File 统一使用相同的过滤级别
- `TAO_LOG` 环境变量可覆盖命令行级别, 支持精细控制:
  `TAO_LOG="tao_codec::decoders=trace,info"`

### 日志级别使用规范 (强制)

编写代码时, 必须严格按以下标准选择日志级别:

#### `error!` - 不可恢复的致命错误

- 程序无法继续运行或丧失核心功能
- 示例: 文件打开失败, 内存分配失败, 关键依赖不可用
- **频率**: 极低, 通常伴随程序退出或功能完全丧失

```rust
error!("打开文件失败: {}", path);
error!("音频设备初始化失败, 无法播放");
```

#### `warn!` - 可恢复的异常/降级

- 出现问题但程序可以继续运行, 功能可能不完整
- 示例: 数据损坏但可跳过, 回退到次优方案, 性能降级
- **频率**: 低, 每个异常场景出现一次

```rust
warn!("VSync 渲染器创建失败, 回退到无 VSync");
warn!("[Seek] 失败: {}", e);
warn!("音频转换失败, 回退原始数据: {}", e);
```

#### `info!` - 关键生命周期事件 (默认可见)

- 程序启动/关闭, 资源打开/释放, 用户触发的操作结果
- 示例: 文件打开, 格式识别, 播放开始/结束, seek 操作, 按键事件
- **频率**: 低到中, 每个用户操作/重要阶段一条
- **原则**: 用户运行程序后, info 日志应能完整描述 "发生了什么"

```rust
info!("正在打开: {}", path);
info!("视频尺寸 {}x{}", width, height);
info!("[Seek] offset={:+.1}s, 目标={:.3}s", offset, target);
info!("播放结束: 发送 {} 帧, 耗时 {:.1}s", frames, elapsed);
```

#### `debug!` - 内部状态/决策 (需 `-v`)

- 开发调试信息: 内部状态变化, 分支决策, 配置解析结果
- 示例: 解码器参数, 同步决策, 缓冲区状态, 流信息
- **频率**: 中, 可能每秒数条, 但不会每帧都出现
- **原则**: 帮助开发者理解 "为什么这样做"

```rust
debug!("选择解码器: codec_id={:?}, 参数={:?}", id, params);
debug!("音频缓冲区状态: 队列={}, 延迟={}ms", queue_len, latency);
debug!("A/V 同步决策: diff={:.3}s, delay={:.3}s", diff, delay);
```

#### `trace!` - 高频热路径数据 (需 `-vv`)

- 每帧/每包/每次回调级别的详细数据
- 示例: 每个数据包的 PTS/DTS, 每帧解码耗时, 每次音频回调
- **频率**: 极高, 可能每秒数百条
- **原则**: 仅在排查细粒度时序/数据问题时需要
- **注意**: trace 日志本身会影响性能, 热路径中避免复杂的格式化

```rust
trace!("解码视频帧: PTS={}, size={}x{}", pts, w, h);
trace!("音频回调: 填充 {} 样本, PTS={:.3}s", samples, pts);
trace!("demux 包: stream={}, pts={}, size={}", idx, pts, size);
```

### 禁止事项

- **禁止** 在热路径 (每帧/每包) 中使用 `info!` 或更高级别
- **禁止** 在 `error!`/`warn!` 中记录正常流程 (如 EOF 不是错误)
- **禁止** 日志消息中包含大量二进制数据或超长字符串
- **禁止** 在性能关键路径中使用复杂的 `format!` 表达式作为日志参数

## 日志输出规则

### 控制台输出

- 与文件统一级别 (默认 info)
- 输出到 stdout
- 带颜色输出 (ANSI)

### 文件输出

- 与控制台统一级别
- 无颜色输出 (纯文本)
- 支持按日期自动切换日志文件

## 日志文件管理

### 日志目录

- 所有日志文件存放在项目根目录 `logs/` 目录下
- `logs/` 目录在 Git 中只保留 `.gitkeep` 文件
- 所有 `*.log` 文件都被 `.gitignore` 忽略, 不提交到 Git

### 日志文件命名

- 格式: `{file_prefix}.{YYYY-MM-DD}.log`
- 示例: `tao.2026-02-16.log`, `tao-probe.2026-02-16.log`

### 文件前缀规范

- **tao-cli**: 使用 `file_prefix = "tao-cli"`
- **tao-probe**: 使用 `file_prefix = "tao-probe"`
- **tao-play**: 使用 `file_prefix = "tao-play"`

### 日志维护

- 自动按日期切换日志文件 (每日凌晨)
- 可配置历史日志保留天数 (默认 30 天)
- 可配置是否压缩历史日志 (默认开启, 生成 `.gz` 文件)
- 定期清理过期日志 (可配置清理间隔)

## 日志初始化示例

### bin 项目 (tao-cli / tao-play / tao-probe)

```rust
mod logging;

fn main() {
    let args = Args::parse();
    // file_prefix 按项目写死, verbosity 由 -v 参数控制
    logging::init("tao-play", args.verbose);
    log::info!("程序启动");
}
```

### 根 crate (含 tokio 维护任务)

```rust
use tao::logging::{init, LoggingConfig};

fn main() -> anyhow::Result<()> {
    init(LoggingConfig {
        level: "info".to_string(),
        directory: "logs".to_string(),
        file_prefix: "tao".to_string(),
        retention_days: 30,
        compress_history: true,
        cleanup_interval_seconds: 3600,
    })?;
    Ok(())
}
```

## AI 调试规范

当需要调试代码时:

1. **优先读取 `logs/` 目录下日志文件, 而非 console 输出**
2. 日志文件位于 `logs/{file_prefix}.{YYYY-MM-DD}.log`
   - tao-play: `logs/tao-play.2026-02-17.log`
   - tao-cli: `logs/tao-cli.2026-02-17.log`
   - tao-probe: `logs/tao-probe.2026-02-17.log`
3. **测试前清理历史日志**, 避免污染:
   ```bash
   rm -f logs/tao-play.*.log   # 清理 tao-play 历史日志
   rm -f logs/*.log             # 清理所有历史日志
   ```
4. 运行程序后直接读取日志文件分析:
   ```bash
   # 读取最新日志
   cat logs/tao-play.$(date +%Y-%m-%d).log
   # 搜索特定关键字
   rg "Seek" logs/tao-play.*.log
   ```
5. 日志文件比 console 输出更高效:
   - 不受终端缓冲区限制
   - 可以用 Read/Grep 工具精准搜索
   - 默认 info 级别, 不含 debug 噪音
   - 需要更多细节时使用 `-v` / `-vv` / `-vvv` 运行
6. 如需调整日志级别:
   ```bash
   # 通过命令行参数
   cargo run --package tao-play -- file.avi          # info (默认)
   cargo run --package tao-play -- -v file.avi       # debug
   cargo run --package tao-play -- -vv file.avi      # trace (tao crate)
   cargo run --package tao-play -- -vvv file.avi     # trace (全局)
   # 通过环境变量 (精细控制)
   TAO_LOG=debug cargo run --package tao-play -- file.avi
   TAO_LOG="tao_codec=trace,info" cargo run --package tao-play -- file.avi
   ```

---

## .cursor/rules/08_security.mdc

---
description: 安全规范
globs:
    - "**/*.rs"
    - "**/*.toml"
    - ".gitignore"
---

# 安全规范

## 敏感信息管理

- **禁止**: 在代码中硬编码任何敏感信息
- 配置文件不得提交到版本库
- 使用环境变量或配置文件管理敏感配置

## 构建产物管理

- `.gitignore` 中必须包含:
    - `target/` - Rust 构建产物
    - `*.dll`, `*.so`, `*.dylib` - 动态库
    - `*.a`, `*.lib` - 静态库
    - 其他临时文件和调试符号

## FFI 安全

- FFI 层所有 `unsafe` 代码必须有详细安全性注释
- 使用 `// SAFETY:` 注释说明为什么这段代码是安全的
- 所有指针参数必须检查 null
- 确保内存所有权和生命周期正确

---

## .cursor/rules/09_commits.mdc

---
description: 代码提交规范
alwaysApply: true
---

# 代码提交规范

## 提交信息格式

使用规范的提交信息格式:

- `feat: 功能描述` - 新增功能
- `fix: 问题描述` - 修复 Bug
- `refactor: 重构描述` - 代码重构
- `style: 样式调整` - 代码格式调整
- `chore: 其他描述` - 构建/工具/依赖更新
- `test: 测试描述` - 新增或修改测试
- `docs: 文档描述` - 文档更新

提交信息必须使用中文, 简洁明了地描述变更内容.

## 提交前检查（强制）

**严格要求**: 提交前必须按以下顺序执行检查, 确保全部通过:

1. `cargo fmt --check` - 确认代码格式一致
2. `cargo clippy -- -D warnings` - 修复所有 Clippy 警告
3. `cargo check` - 确认编译通过
4. `cargo test` - 确认所有测试通过

## 0 警告容忍

- **任何 Clippy 警告都必须在提交前修复, 不允许忽略**
- 禁止使用 `#[allow(...)]` 来绕过 Clippy 检查, 除非有充分理由并添加详细注释说明

## 自动提交规则

- 每完成一轮功能开发, 且代码检查全部通过后, 必须自动提交本次修改
- 提交范围应仅包含当轮功能涉及的文件
- 提交信息应准确概括本轮变更内容

---

## .cursor/rules/10_code_quality.mdc

---
description: 代码质量要求
globs:
    - "**/*.rs"
alwaysApply: true
---

# 代码质量要求

## 基本要求

- **重要**: 不允许存在任何编译错误或 Clippy 警告
- 未使用的 `use` 导入必须删除
- 未使用的变量, 函数, 类型定义必须删除或添加 `_` 前缀标记为故意未使用

## 代码规范

- 避免硬编码的魔法数字或字符串, 使用常量或配置项替代
- 避免重复代码, 提取公共逻辑到工具函数或 trait 中
- 保持函数简洁, 建议单个函数不超过 50 行
- 复杂逻辑应拆分为多个小函数

## 代码审查清单

每次修改后都应该自我审查代码, 确保:

- ✓ 没有未使用的 imports, variables, functions
- ✓ 没有重复代码
- ✓ 没有硬编码的魔法数字或字符串
- ✓ 遵循项目代码风格
- ✓ 所有错误都有适当的处理
- ✓ 所有公开 API 都有文档注释
- ✓ 复杂逻辑有适当的注释说明

---

## .cursor/rules/11_testing.mdc

---
description: 测试规范和用例开发流程
globs:
    - "tests/**/*.rs"
    - "benches/**/*.rs"
---

# 测试规范

## 基本要求

- 代码修改后必须执行 `cargo check` 与 `cargo test`
- 如出现错误或警告, 必须先修复再继续后续修改
- **重要**: 新增编解码器或容器格式时必须编写测试
- 集成测试放在 `tests/` 目录下, 单元测试放在源文件内 `#[cfg(test)]` 模块中
- 测试用例命名需要准确描述测试内容与预期结果, 使用蛇形命名法
- 测试应覆盖正常流程, 边界情况和错误情况
- **所有样本使用 URL 方式访问**，不下载到本地 (详见 [samples/SAMPLE_URLS.md](../../samples/SAMPLE_URLS.md))
- **临时文件放在 `data/` 目录**，永不提交到 Git (详见 [15_data_management.mdc](15_data_management.mdc))

## 测试用例开发流程

### 1. 确定测试需求

明确需要测试的场景:

- 正常流程: 标准输入输出, 基本功能验证
- 边界情况: 空输入, 极限参数, 特殊格式
- 错误处理: 损坏数据, 不支持的参数, 资源不足

### 2. 查找和使用测试样本

**重要**: 所有样本使用 **HTTPS URL** 直接访问，无需下载到本地

1. **查找样本**: 在 `samples/SAMPLE_URLS.md` 中查找適用的样本 URL
2. **样本来源**: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
3. **验证样本**: 使用 `ffprobe <URL>` 验证样本信息
    ```bash
    ffprobe https://samples.ffmpeg.org/HDTV/Channel9_HD.ts
    ```
4. **如果没有合适样本**:
    - 访问 https://samples.ffmpeg.org/ 浏览完整样本库
    - 或查看 https://samples.ffmpeg.org/allsamples.txt
    - 使用 `ffprobe <URL>` 验证
    - 添加到 `samples/SAMPLE_URLS.md` 并提交
    ```bash
    git add samples/SAMPLE_URLS.md
    git commit -m "docs: 添加 XXX 编解码器测试样本 URL"
    ```

### 3. 编写测试用例

```rust
#[test]
fn test_codec_decode_basic() {
    // 1. 从 samples/SAMPLE_URLS.md 获取样本 URL
    let sample_url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    // 2. 直接使用 URL 打开解封装器, 无需下载文件
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

    // 5. 解码前几帧验证功能
    // 重要: 只解码 5-10 帧就可以验证功能, 避免测试耗时过长
    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet().unwrap() {
        if packet.stream_index() == video_stream.index() {
            let frames = decoder.decode(&packet).unwrap();
            frame_count += frames.len();
            if frame_count >= 10 { break; }  // 限制帧数
        }
    }

    assert!(frame_count >= 10, "应该解码出至少 10 帧");
}
```

## 测试用例编写标准

- **文件位置**: `tests/{feature}_pipeline.rs`
- **测试命名**: `test_{component}_{scenario}` 格式
- **断言清晰**: 每个 `assert!` 包含失败消息说明预期行为
- **注释完整**: 复杂逻辑添加注释，建议使用 step-by-step 清晰说明
- **样本来源**: 使用 `samples/SAMPLE_URLS.md` 中的 **HTTPS URL**，无需下载
- **帧数限制**: 只解码前 5-10 帧验证功能，避免测试耗时过长（示例见第 3 节）
- **资源清理**: 临时文件放在 `data/` 目录（详见 [15_data_management.mdc](15_data_management.mdc)）
- **网络访问**: 所有测试假设有网络连接

## 测试覆盖范围

### 编解码器测试

- ✓ 基本解码 (正常流程)
- ✓ 编码 (如果实现了编码器)
- ✓ 空输入处理
- ✓ 损坏数据处理
- ✓ Flush 流程
- ✓ 参数解析 (SPS/PPS/VPS 等)

### 容器格式测试

- ✓ 格式探测 (Probe)
- ✓ 头部解析
- ✓ 数据包读取
- ✓ Seek 操作
- ✓ 多流处理 (音视频同时存在)
- ✓ 损坏文件处理

### 滤镜测试

- ✓ 基本滤镜操作
- ✓ 参数验证
- ✓ 链式滤镜
- ✓ 边界条件 (分辨率, 像素格式)

---

## .cursor/rules/12_comments.mdc

---
description: 注释规范
globs:
    - "**/*.rs"
---

# 注释规范

## 基本要求

- **必须**: 所有注释使用中文
- 复杂逻辑必须添加注释说明
- 公开函数和 trait 使用 `///` 文档注释, 说明功能, 参数, 返回值
- 每个 crate 的 `lib.rs` 使用 `//!` 模块文档注释, 说明 crate 用途

## 特殊注释

- **FFI 导出函数**: 必须说明安全性要求 (`# Safety` 段落)
- **特殊处理**: Workaround 必须注释说明原因
- **临时代码**: 使用 `// TODO:` 标记待优化代码
- **安全代码**: `unsafe` 块使用 `// SAFETY:` 说明安全前提

## 文档注释示例

```rust
/// 解码一个 H.264 视频帧
///
/// # 参数
///
/// * `packet` - 压缩的视频数据包
///
/// # 返回值
///
/// 返回解码后的视频帧, 如果需要更多数据则返回 `None`
///
/// # 错误
///
/// 如果数据包损坏或格式不支持, 返回 `TaoError::InvalidData`
pub fn decode(&mut self, packet: &Packet) -> TaoResult<Option<Frame>> {
    // ...
}
```

---

## .cursor/rules/13_performance.mdc

---
description: 性能优化指南
globs:
    - "crates/tao-codec/**/*.rs"
    - "crates/tao-format/**/*.rs"
    - "crates/tao-scale/**/*.rs"
---

# 性能优化

## 内存管理

- 避免不必要的内存分配, 优先使用引用和借用
- 帧缓冲区应支持复用, 避免每帧都重新分配内存
- 大块数据使用 `bytes::Bytes` 实现零拷贝传递

## 数据处理

- 大量数据处理使用迭代器而非收集到 `Vec` 后再遍历
- 像素格式转换和编解码热路径应尽量避免分支预测失败
- 考虑使用 SIMD 指令优化关键路径 (通过 `std::arch` 或 `packed_simd`)

## 性能测试

- 使用 `benches/` 目录编写基准测试
- 使用 `cargo bench` 测量性能
- 关注热路径的性能表现
- 使用 profiler 工具分析性能瓶颈

---

## .cursor/rules/14_manual_testing.mdc

---
description: 手动播放测试规范
globs:
    - "bins/tao-play/**/*.rs"
alwaysApply: true
---

# 手动播放测试规范

## 播放时长限制

- 手动测试音视频播放时, **禁止完整播放**整个文件
- 默认播放 **前 10 秒** 即可验证功能
- 如有必要 (验证 seek/后段内容) 可增加到 **最多 30 秒**
- 播放结束后必须主动终止播放进程

## 播放超时机制 (强制)

**核心原则**: 启动 `tao-play` 播放进程时, **必须使用 `timeout` 命令包裹**, 防止因逻辑 bug、死锁、解码卡死等问题导致播放进程永远不退出, 阻塞后续执行.

### 超时时间

- 默认超时: **30 秒**
- 短时测试 (仅验证能否启动/前几帧): **15 秒**
- 超时后进程会被强制终止, 不影响后续操作

### Linux/macOS 用法

使用 `timeout` 命令:

```bash
# 正确: 使用 timeout 包裹, 30 秒后自动终止
timeout 30 cargo run --package tao-play -- "https://example.com/video.mp4"

# 正确: 短时测试, 15 秒超时
timeout 15 cargo run --package tao-play -- "https://example.com/video.mp4"

# 错误: 没有超时保护, 可能永远卡住
cargo run --package tao-play -- "https://example.com/video.mp4"
```

### Windows 用法

Windows 下没有原生 `timeout` 命令用于限制进程运行时间, 使用后台启动 + 延时终止:

```powershell
# 启动播放后, 等待 30 秒后强制终止
Start-Process -NoNewWindow cargo -ArgumentList "run","--package","tao-play","--","https://example.com/video.mp4"
Start-Sleep -Seconds 30
TASKKILL /F /IM tao-play.exe /T
```

### 异常处理

- 如果进程在超时前正常退出 (如播放完成或出错), `timeout` 命令会立即返回, 不影响结果
- 如果进程被超时强制终止, 退出码为 124 (Linux), 这是**预期行为**, 不代表测试失败
- 超时终止后应检查日志 (`logs/tao-play.*.log`) 分析是否存在逻辑问题

## 终止播放进程 (Windows)

- Windows 下终止 tao-play 进程时, **必须使用 `TASKKILL /F /IM tao-play.exe /T`**
- **禁止使用 `TASKKILL /F /PID <pid>`** (PID 不可靠)

```powershell
# 正确
TASKKILL /F /IM tao-play.exe /T

# 错误 (PID 不可靠)
TASKKILL /F /PID 12345
```

## 终止播放进程 (Linux/macOS)

- 播放进程通常由 `timeout` 自动终止, 无需手动操作
- 如需手动终止, 使用 `pkill` 或 `killall`:

```bash
# 正确: 按进程名终止
pkill -f tao-play
# 或
killall tao-play

# 错误: 使用 PID (不可靠)
kill -9 12345
```

## 流式播放测试

- `tao-play` 支持 http/https/rtmp 等流式 URL 播放
- **所有测试文件均使用 URL 直接流式播放**, 不下载到本地
- 所有样本 URL 维护在 `samples/SAMPLE_URLS.md` 中

```bash
# 正确: 使用 timeout + URL 进行流式播放测试
timeout 30 cargo run --package tao-play -- "https://samples.ffmpeg.org/flac/Yesterday.flac"

# 查看更多样本 URL
# 请参考 samples/SAMPLE_URLS.md
```

---

## .cursor/rules/15_data_management.mdc

---
description: 测试文件和临时文件管理
globs:
    - "tests/**/*.rs"
    - "data/**"
    - "samples/**"
---

# 测试文件和临时文件管理

> 完整规范参见 [samples/SAMPLES.md](../../samples/SAMPLES.md)

## 核心原则

- **所有样本使用 URL 方式访问，不下载到本地**
- **所有临时文件放在 `data/` 目录下**
- **`data/` 整体被 Git 忽略，永不提交任何文件**
- **所有测试代码应遵循 [samples/SAMPLES.md](../../samples/SAMPLES.md) 规范**

## 目录结构

```
project/
├── tests/
│   ├── {feature}_pipeline.rs         # 集成测试
│   ├── logging_system.rs             # 日志系统测试
│   └── ...
├── benches/
│   └── *.rs                          # 基准测试
├── samples/
│   ├── SAMPLES.md                    # 样本使用规范 (Git 跟踪)
│   └── SAMPLE_URLS.md                # 样本 URL 清单 (Git 跟踪)
├── data/
│   ├── .gitkeep                      # 确保目录在 Git 中
│   ├── tmp_*/                        # 测试产生的临时文件
│   ├── logs/                         # 日志文件（备选放置位置）
│   ├── ffmpeg/                       # 编解码临时输出
│   └── ...                           # 其他临时文件
├── logs/ (可选)
│   ├── .gitkeep                      # 日志目录标记
│   ├── tao.2026-02-16.log            # tao-cli 日志
│   ├── tao-probe.2026-02-16.log      # tao-probe 日志
│   ├── tao-play.2026-02-16.log       # tao-play 日志
│   └── ...
└── .gitignore                        # 忽略 data/ 和 logs/
```

## 文件管理规则

### 测试代码位置

- **单元测试**: 在 `crates/*/src/` 源文件中使用 `#[cfg(test)]` 模块
- **集成测试**: 在 `tests/{feature}_pipeline.rs` 文件中（例如 h264_decode_pipeline.rs）
- **基准测试**: 在 `benches/*.rs` 文件中
- **测试命名**: 使用 `test_{component}_{scenario}` 格式
- 参考示例:
    - `test_h264_decode_basic` (基本功能)
    - `test_h264_decode_corrupted_data` (错误处理)
    - `test_mp4_demux_seek` (格式特性)

### 样本文件 (samples/SAMPLE_URLS.md)

**重要: 所有样本使用 HTTPS URL，无需下载到本地**

- **样本来源**: https://samples.ffmpeg.org/ (FFmpeg 官方测试样本库)
- **样本清单**: `samples/SAMPLE_URLS.md` 记录所有测试样本的 URL 和用途
- **测试用例**: 直接使用 URL 创建 Demuxer/Decoder，无需下载文件
- **样本大小**: 从 KB 级别小文件到 MB 级别大文件，均可直接通过 URL 访问
- **包含类别**:
    - 视频编解码器: H.264, H.265, MPEG-4 Part 2, Theora, VP8, VP9, AV1 等
    - 音频编解码器: AAC, MP3, FLAC, Vorbis, Opus, PCM, ALAC 等
    - 容器格式: MP4, MKV, AVI, FLV, MPEG-TS, Ogg, WAV, AIFF 等

### 临时文件 (data/)

- 项目运行时生成的所有临时文件都应放在 `data/` 目录下
- 用途包括:
    - 编解码过程中的中间文件
    - 测试输出的视频/音频文件
    - 日志文件（可选）
    - 调试数据和性能分析信息
- **永不提交到 Git** (整体被 `.gitignore` 忽略)

## 样本 URL 使用规范

### 基本原则

1. **所有样本使用 HTTPS URL** 直接访问
2. **无需下载到本地** - 直接通过 URL 创建 Demuxer/Decoder
3. **URL 来源**: https://samples.ffmpeg.org/ 官方测试样本库
4. **验证方式**: 使用 `ffprobe <URL>` 检查编解码器和格式信息
5. **版本管理**: 所有有效 URL 记录在 `samples/SAMPLE_URLS.md` 中

### 查找样本

```bash
# 方法 1: 在 samples/SAMPLE_URLS.md 中搜索
grep -i "h264" samples/SAMPLE_URLS.md

# 方法 2: 浏览官方样本库
https://samples.ffmpeg.org/

# 方法 3: 查看完整样本列表
https://samples.ffmpeg.org/allsamples.txt
https://samples.ffmpeg.org/allsamples-old.txt
```

### 使用示例

```rust
use tao_format::demuxer::DemuxerRegistry;
use tao_codec::decoder::DecoderRegistry;
use tao_core::MediaType;

#[test]
fn test_h264_decode_from_url() {
    // 1. 从 samples/SAMPLE_URLS.md 查找合适的 URL
    let sample_url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    // 2. 直接使用 URL 创建 Demuxer
    let mut demuxer = DemuxerRegistry::open(sample_url)
        .expect("打开网络样本文件失败");

    // 3. 查找视频流
    let video_stream = demuxer.streams()
        .iter()
        .find(|s| s.media_type() == MediaType::Video)
        .expect("应该有视频流");

    let mut decoder = DecoderRegistry::create_decoder(
        video_stream.codec_id(),
        video_stream.codec_params(),
    ).expect("创建解码器失败");

    // 4. 只解码前几帧 (5-10 帧) 验证功能
    // 重要: 限制帧数可以加快测试执行速度
    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet()
        .expect("读取数据包失败") {
        if packet.stream_index() == video_stream.index() {
            let frames = decoder.decode(&packet)
                .expect("解码失败");
            frame_count += frames.len();
            if frame_count >= 10 {
                break;  // 解码足够的帧后停止
            }
        }
    }

    assert!(
        frame_count >= 10,
        "应该至少解码 10 帧，实际解码 {} 帧",
        frame_count
    );
}
```

### 验证样本

```bash
# 使用 ffprobe 验证样本编解码器和格式信息
ffprobe https://samples.ffmpeg.org/HDTV/Channel9_HD.ts

# 输出示例:
# Input #0, mpegts, from 'https://samples.ffmpeg.org/HDTV/Channel9_HD.ts':
#   Duration: 00:05:00.00, start: 17.900000, bitrate: 4500 kb/s
#     Stream #0:0[0x100]: Video: h264 (Main), yuv420p, 1280x720 [SAR 1:1 DAR 16:9], 29.97 fps
#     Stream #0:1[0x101]: Audio: ac3, 48000 Hz, 5.1(side), fltp, 448 kb/s

# 验证支持的编解码器
ffprobe -show_entries stream=codec_name https://samples.ffmpeg.org/HDTV/Channel9_HD.ts
```

## 新增测试样本

### 查找样本

1. **浏览官方库**: 访问 https://samples.ffmpeg.org/
2. **搜索样本**: 使用浏览器搜索或查看完整列表
3. **过滤条件**:
    - 编解码器: H.264, AAC, FLAC 等
    - 容器格式: MP4, MKV, TS 等
    - 分辨率: 480p, 720p, 1080p
    - 采样率: 44.1kHz, 48kHz, 96kHz

### 验证样本

```bash
# 验证编解码器支持
ffprobe https://samples.ffmpeg.org/path/to/sample.mp4

# 验证特定属性
ffprobe -show_entries stream=codec_name,width,height,sample_rate \
        https://samples.ffmpeg.org/path/to/sample.mp4

# 检查是否支持 seek
ffprobe -show_entries stream=duration \
        https://samples.ffmpeg.org/path/to/sample.mp4
```

### 添加到清单

**编辑 `samples/SAMPLE_URLS.md`**:

1. 在对应编解码器/容器格式章节添加表格行
2. 格式: `| 用途描述 | URL | 详细说明 |`
3. 示例:
    ```markdown
    | H.265 解码 | https://samples.ffmpeg.org/path/to/hevc.mp4 | 1080p HEVC + AAC, MP4 容器 |
    ```

**提交更新**:

```bash
git add samples/SAMPLE_URLS.md samples/SAMPLES.md
git commit -m "docs: 添加 XXXX 编解码器测试样本 URL

- 新增 H.265 样本: 1080p HEVC + AAC, MP4 容器
- 新增 FLAC 样本: 96kHz 无损音频
- 来源: ffmpeg.org 官方测试样本库"
```

### 维护检查

- **定期检查**: 每季度验证 URL 是否有效
- **更新失效 URL**: 从官方库查找替代样本
- **版本同步**: 保持 `samples/SAMPLE_URLS.md` 的更新

## 临时文件管理

### 创建习惯

```rust
use std::path::PathBuf;

// 创建 data 目录
let data_dir = PathBuf::from("data");
std::fs::create_dir_all(&data_dir)?;

// 创建临时文件 (使用进程 ID 或时间戳避免冲突)
let temp_file = data_dir.join(format!(
    "tmp_test_{}_output.bin",
    std::process::id()
));

let mut file = std::fs::File::create(&temp_file)?;

// ... 处理文件 ...

// 测试完成后清理
std::fs::remove_file(&temp_file)?;

Ok(())
```

### 命名规范

- 格式: `tmp_` + 标识信息 + 扩展名
- 示例:
    - `tmp_h264_decode_output_12345.yuv`
    - `tmp_transcode_12345.mp4`
    - `tmp_filter_output_12345.bin`
- **推荐使用进程 ID 避免并发冲突**: `std::process::id()`
- **可选**: 使用时间戳: `std::time::SystemTime::now()`

### 清理规则

- **测试结束后必须清理**: 默认清理(不保留中间文件)
- **或在子目录中组织**: 使用 `data/test_name/` 子目录分组
- **永不提交到 Git**: 整体被 `.gitignore` 忽略
- **异常处理**: 即使发生 panic，也应该在 drop 时清理

```rust
// 推荐使用 RAII 模式自动清理
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str) -> std::io::Result<Self> {
        let path = PathBuf::from("data")
            .join(format!("tmp_{}", name));
        std::fs::File::create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
```

## 日志文件管理

### 日志保存位置

- 优先: `logs/` 目录（在项目根目录）
- 备选: `data/logs/` 子目录（其他临时数据旁边）
- 日志命名: `{prefix}.{YYYY-MM-DD}.log`

### 文件前缀规范

| 工具      | 前缀      | 示例日志文件             |
| --------- | --------- | ------------------------ |
| tao-cli   | tao       | tao.2026-02-16.log       |
| tao-probe | tao-probe | tao-probe.2026-02-16.log |
| tao-play  | tao-play  | tao-play.2026-02-16.log  |

### AI 调试指南

当调试代码时遵循本规范：

1. **优先查看日志文件** 而非控制台输出
    - 日志文件位于 `logs/{prefix}.{date}.log`
    - 或 `data/logs/` 内的日志文件
2. **清理旧日志避免污染**
    - 调试前可删除相关日志文件
    - 重新运行程序生成新日志
3. **快速分析执行流程**
    - 通过日志文件追踪程序执行
    - 识别错误发生的位置
    - 减少频繁读取控制台输出

## 手动清理 data 目录

### Linux / macOS

```bash
# 查看 data 目录大小
du -sh data/

# 清理所有临时文件 (保留 .gitkeep)
rm -rf data/*
git checkout data/.gitkeep

# 或仅清理特定类型文件
rm -f data/*.log data/*.bin data/*.tmp
```

### Windows PowerShell

```powershell
# 查看 data 目录大小
(Get-ChildItem -Path data -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB

# 清理所有临时文件 (保留 .gitkeep)
Remove-Item -Path data/* -Force -Recurse -Exclude ".gitkeep"

# 或仅清理特定类型文件
Remove-Item -Path data/*.log, data/*.bin, data/*.tmp -Force
```

## 注意事项

### 网络连接

- 所有测试假设有 **稳定的网络连接**
- URL 无法访问时，测试会失败或超时
- 网络不稳定的环境建议标记为 `#[ignore]` 并离线测试

### 临时文件清理

- 测试异常终止时，临时文件可能不被清理
- 定期手动清理 `data/` 目录
- 生产环境应定时清理脚本

### 磁盘空间

- 大型编解码测试可能占用大量临时空间
- 监控 `data/` 目录大小
- 必要时使用 `cleanup_test_logs(name)` 之类的函数主动清理

### 安全性

- 不在临时文件中存储敏感信息
- 不提交 API 密钥或认证令牌到 Git
- `.gitignore` 中应包含所有需要忽略的临时文件类型

---

## .cursor/rules/README.md

# Cursor Rules 目录

本目录包含 Tao 项目的模块化开发规范文件，专为 Cursor AI 优化。

## 规则文件列表

### 核心规范

- **00_index.mdc** - 规则索引和说明
- **01_project_overview.mdc** - 项目概述、结构和核心架构
- **02_language.mdc** - 语言规范（必须使用中文）
- **03_code_organization.mdc** - 代码组织、执行计划和文件管理

### Rust 开发规范

- **04_rust_coding.mdc** - Rust 编码规范、类型安全、并发和格式化
- **05_error_handling.mdc** - 错误处理规范
- **06_development_rules.mdc** - 开发规则（编解码器、容器格式、FFI）

### 质量与规范

- **07_logging.mdc** - 日志规范
- **08_security.mdc** - 安全规范
- **09_commits.mdc** - 代码提交规范
- **10_code_quality.mdc** - 代码质量要求

### 测试规范

- **11_testing.mdc** - 测试规范和用例开发流程
- **14_manual_testing.mdc** - 手动播放测试规范
- **15_data_management.mdc** - 测试文件和临时文件管理

### 其他规范

- **12_comments.mdc** - 注释规范
- **13_performance.mdc** - 性能优化

## 设计原则

1. **模块化**: 每个规则文件专注于特定领域，便于快速查找和加载
2. **上下文优化**: 使用 YAML front matter 定义适用范围，减少不必要的上下文加载
3. **简洁明确**: 每个文件内容精简，去除冗余，保留核心规则
4. **保持同步**: 与 `.github/` 规则文件保持内容一致

## 使用说明

- Cursor 会自动加载本目录下的规则文件
- 带有 `alwaysApply: true` 的规则始终生效
- 其他规则根据文件类型和上下文（通过 `globs`）自动应用
- `.github/` 规则文件提供 Copilot 版本的完整参考

## 维护说明

更新规范时需要：

1. 同时更新对应的 `.cursor/rules/*.mdc` 文件和 `.github/` 规则文件
2. 确保两者保持一致
3. 提交时说明规范变更内容

## 文件命名规范

- 使用两位数字前缀（00-15）确保文件顺序
- 使用 `.mdc` 扩展名（Markdown with Cursor rules）
- 文件名使用下划线分隔的英文描述

