# Tao 项目 - GitHub Copilot 开发指令

> 本文件为 GitHub Copilot 提供 Tao 项目的核心开发规范。

---

## 规范来源

本项目的开发规范维护在两个位置:

- **Cursor 规则**: `.cursor/rules/` - 为 Cursor AI 优化的模块化规则文件 (16个文件)
- **Copilot 规则**: `.github/copilot-rules/` - 本规则系统,为 GitHub Copilot 优化 (6个模块文件)

---

## 核心规范速查

### 🌐 语言要求 (最高优先级)

**必须严格遵守:**

- ✅ **所有代码注释必须使用中文**
- ✅ **所有日志输出必须使用中文**
- ✅ **所有错误消息必须使用中文**
- ✅ **AI 上下文输出必须使用中文**
- ✅ **标点符号使用英文标点**
- ✅ 代码标识符(变量/函数/类型)使用英文,遵循 Rust 命名惯例
- ✅ 提交信息使用中文

**示例:**

```rust
// ✅ 正确: 中文注释
/// 解码 H.264 视频帧
fn decode_h264_frame(packet: &Packet) -> TaoResult<Frame> {
    debug!("开始解码 H.264 帧, PTS: {}", packet.pts());
    // 实现解码逻辑...
}

// ❌ 错误: 英文注释
/// Decode H.264 video frame
fn decode_h264_frame(packet: &Packet) -> TaoResult<Frame> {
    debug!("Decoding H.264 frame, PTS: {}", packet.pts());
    // ...
}
```

### 🔨 代码质量 (0 警告容忍)

**提交前强制检查顺序:**

```bash
# 1. 代码格式化检查
cargo fmt --check

# 2. Clippy 检查 (任何警告都必须修复)
cargo clippy -- -D warnings

# 3. 编译检查
cargo check

# 4. 测试通过
cargo test
```

**绝对禁止:**

- ❌ 任何 Clippy 警告
- ❌ 未使用的 imports/variables/functions
- ❌ 使用 `#[allow(...)]` 绕过检查(除非有充分理由并详细注释)
- ❌ 随意使用 `unwrap()` / `expect()` (除非能确保不会 panic)
- ❌ 硬编码的魔法数字或字符串

**代码审查清单:**

- [ ] 没有未使用的 imports, variables, functions
- [ ] 没有重复代码
- [ ] 没有硬编码的魔法数字或字符串
- [ ] 遵循项目代码风格
- [ ] 所有错误都有适当的处理
- [ ] 复杂逻辑有清晰注释
- [ ] 单个函数不超过 50 行

### � MCP 服务器优先级规则 (最高优先级)

**重要: 工具选择顺序**

根据可用的 MCP 服务器改变工具优先级:

#### 1️⃣ rust-mcp-server (Cargo 命令相关)

**如果检测到 rust-mcp-server 可用，必须优先使用:**

在执行以下命令时，优先通过 MCP 服务器而非终端:

- `cargo build` / `cargo check` / `cargo fmt` / `cargo clippy` / `cargo test` / `cargo run`
- `cargo add` / `cargo remove` / `cargo update`
- `cargo doc` / `cargo tree` / `cargo expand`

**优点:**

- 更快的命令响应
- 更完整的错误诊断
- 集成的代码分析反馈
- 自动的构建缓存管理

**使用前检测:**

```
# 检查是否有可用的 rust-mcp-server MCP
如果命令执行时有"cargo: build"等相关 MCP 工具可用，优先调用该工具
```

#### 2️⃣ rust-analyzer-mcp (代码分析相关)

**如果检测到 rust-analyzer-mcp 可用，必须优先使用:**

对于以下代码分析需求，优先通过 MCP 服务器而非终端:

- 查找符号定义 (Go to Definition)
- 查找所有引用 (Find References)
- 获取符号信息和文档
- 代码悬停信息 (Hover)
- 代码完成建议 (Completion)
- 代码操作和快速修复 (Code Actions)
- 诊断信息 (Compiler Diagnostics)
- 代码格式化 (Format Code)

**优点:**

- 精确的代码导航
- 类型检查和错误诊断
- 智能重构建议
- 无需编译的快速分析

**使用前检测:**

```
# 检查是否有可用的 rust-analyzer-mcp MCP
如果命令执行时有"rust-analyzer"相关 MCP 工具可用，优先调用该工具
```

#### 3️⃣ 工具选择决策树

```
需要执行 Cargo 命令?
├─ 是 ──> rust-mcp-server 可用?
│         ├─ 是 ──> 优先使用 MCP 工具 (cargo-build, cargo-clippy 等)
│         └─ 否 ──> 使用终端命令 (run_in_terminal)
└─ 否 ──> 需要代码分析?
           ├─ 是 ──> rust-analyzer-mcp 可用?
           │         ├─ 是 ──> 优先使用 MCP 工具 (rust-analyzer-*)
           │         └─ 否 ──> 使用终端或其他方式
           └─ 否 ──> 正常处理
```

#### 4️⃣ 检测可用 MCP 的方法

在每个任务开始时进行一次检测:

```
// 检测 rust-mcp-server
if (can_use_mcp_tool("cargo-build") || can_use_mcp_tool("cargo-check")) {
    // 使用 MCP 工具执行 cargo 命令
    prefer_mcp_for_cargo_commands = true
}

// 检测 rust-analyzer-mcp
if (can_use_mcp_tool("rust-analyzer-hover") || can_use_mcp_tool("rust-analyzer-completion")) {
    // 使用 MCP 工具执行代码分析
    prefer_mcp_for_code_analysis = true
}
```

**注意:** 这是自动检测，工具框架会在工具调用前进行检查。优先级规则在工具可用时自动应用。

### �📝 提交规范

**提交信息格式:**

- `feat: 功能描述` - 新增功能
- `fix: 问题描述` - 修复 Bug
- `refactor: 重构描述` - 代码重构
- `style: 样式调整` - 代码格式调整
- `chore: 其他描述` - 构建/工具/依赖更新
- `test: 测试描述` - 新增或修改测试
- `docs: 文档描述` - 文档更新

**自动提交规则:**

每完成一轮功能开发,且代码检查(`cargo fmt`, `cargo clippy`, `cargo test`)全部通过后,必须自动提交本次修改。

---

## 项目概述

### 项目简介

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架,目标是全功能复刻 FFmpeg。提供三种使用方式:

1. **Rust 库** - 其他 Rust 项目通过 `tao` crate 调用
2. **C FFI** - 通过 `tao-ffi` crate 导出 C 兼容接口
3. **命令行工具** - `tao`, `tao-probe`, `tao-play` 可执行文件

### 项目结构

```
tao/
├── crates/                 # 库 crate
│   ├── tao-core/           # 核心类型 (对标 libavutil)
│   ├── tao-codec/          # 编解码器 (对标 libavcodec)
│   ├── tao-format/         # 容器格式 (对标 libavformat)
│   ├── tao-filter/         # 滤镜 (对标 libavfilter)
│   ├── tao-scale/          # 图像缩放 (对标 libswscale)
│   ├── tao-resample/       # 音频重采样 (对标 libswresample)
│   └── tao-ffi/            # C FFI 导出
├── bins/                   # 可执行文件
│   ├── tao-cli/            # 命令行工具 (对标 ffmpeg)
│   ├── tao-probe/          # 探测工具 (对标 ffprobe)
│   └── tao-play/           # 播放器 (对标 ffplay)
├── tests/                  # 集成测试
├── plans/                  # AI 执行计划
├── samples/                # 测试样本清单
├── data/                   # 临时文件 (不提交 Git)
└── logs/                   # 日志文件 (不提交 Git)
```

### crate 依赖关系

```
tao-core (最底层,无外部依赖)
  ↑
tao-codec, tao-format, tao-filter, tao-scale, tao-resample
  ↑
tao-ffi, tao-cli, tao-probe, tao-play
  ↑
tao (门面库,re-export 所有库 crate)
```

### 核心架构

**编解码管线:**

```
[输入文件] → Demuxer → Packet → Decoder → Frame → Filter → Encoder → Packet → Muxer → [输出文件]
```

**注册表模式:**

- 编解码器和容器格式采用注册表模式
- 各模块通过工厂函数注册到全局注册表
- 运行时按 ID 查找并创建实例

---

## 开发工作流核心要点

### 新增编解码器

1. 在 `tao-codec/src/decoders/` 或 `encoders/` 创建独立子模块
2. 实现 `Decoder` 或 `Encoder` trait
3. 注册到 `CodecRegistry`
4. 编写测试用例(必须,使用 samples/SAMPLE_URLS.md 中的样本 URL)

### 新增容器格式

1. 在 `tao-format/src/demuxers/` 或 `muxers/` 创建独立子模块
2. 实现 `Demuxer` 或 `Muxer` trait
3. 实现 `FormatProbe` trait
4. 注册到 `FormatRegistry`
5. 编写测试用例

### 错误处理

- ✅ 使用 `TaoError` / `TaoResult` 作为统一错误类型
- ✅ I/O 操作必须处理错误,禁止吞错
- ✅ 损坏数据返回 `TaoError::InvalidData`
- ✅ 未实现功能返回 `TaoError::NotImplemented` (不使用 `todo!()`)
- ✅ 错误消息必须使用中文

### 测试样本使用

**重要:所有样本使用 HTTPS URL 直接访问,不下载到本地**

1. 从 `samples/SAMPLE_URLS.md` 查找合适的样本 URL
2. 样本来源: https://samples.ffmpeg.org/
3. 直接使用 URL 创建 Demuxer/Decoder:

```rust
#[test]
fn test_h264_decode_basic() {
    // 从 samples/SAMPLE_URLS.md 获取样本 URL
    let url = "https://samples.ffmpeg.org/HDTV/Channel9_HD.ts";

    // 直接使用 URL,无需下载
    let mut demuxer = DemuxerRegistry::open(url).unwrap();

    // 只解码前 5-10 帧验证功能
    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet().unwrap() {
        // ... 解码逻辑 ...
        frame_count += 1;
        if frame_count >= 10 { break; }
    }

    assert!(frame_count >= 10);
}
```

### 文件管理规则

**严格禁止在项目根目录随意创建新文件**

允许的根目录文件:

- 项目配置: `Cargo.toml`, `.rustfmt.toml`, `.gitignore`
- 核心文档: `README.md`
- License 文件: `LICENSE*`

其他文件存放位置:

- 执行计划 → `plans/` 目录
- 测试代码 → `tests/` 目录
- 样本清单 → `samples/` 目录
- 临时文件 → `data/` 目录(永不提交 Git)
- 日志文件 → `logs/` 目录(永不提交 Git)

---

## 模块化规则索引

详细规则分为 6 个主题模块,位于 `.github/copilot-rules/` 目录:

### 📁 [project.md](copilot-rules/project.md)

- 项目概述和目标
- 完整项目结构
- crate 依赖关系详解
- 核心架构(编解码管线/注册表模式/I/O 抽象/FFI 导出)
- 代码组织规范
- 执行计划管理
- 根目录文件管理规则

### 🦀 [rust.md](copilot-rules/rust.md)

- Rust 编码规范(类型安全/并发/格式化)
- 错误处理规范(`TaoError`/`TaoResult`使用)
- 安全规范(敏感信息/FFI 安全/SAFETY 注释)
- 枚举设计(`#[non_exhaustive]`)

### 🛠️ [development.md](copilot-rules/development.md)

- 新增编解码器开发流程
- 新增容器格式开发流程
- FFI 导出规则(向后兼容/null 检查)
- 注册表模式详解
- 特定场景处理

### 🧪 [testing.md](copilot-rules/testing.md)

- 测试基本要求和覆盖范围
- 测试用例开发流程(6步法)
- 测试样本 URL 使用规范
- 手动播放测试规范
- 测试文件和临时文件管理
- 日志文件管理

### ✨ [quality.md](copilot-rules/quality.md)

- 代码质量要求(0 警告容忍)
- 代码提交规范(格式/检查流程)
- 注释规范(中文/文档注释/特殊注释)
- 代码审查清单

### ⚡ [performance.md](copilot-rules/performance.md)

- 性能优化原则(内存/数据处理/SIMD)
- 日志规范(tracing/日志级别/文件管理)
- AI 调试规范

---

## 快速参考表

| 场景         | 关键规则                                      | 详细文档                                                 |
| ------------ | --------------------------------------------- | -------------------------------------------------------- |
| 编写任何代码 | 使用中文注释和日志                            | 本文档 §语言要求                                         |
| 提交代码前   | fmt → clippy → check → test                   | 本文档 §代码质量                                         |
| 新增编解码器 | 创建子模块 → 实现 trait → 注册 → 测试         | [development.md](copilot-rules/development.md)           |
| 新增容器格式 | 创建子模块 → 实现 trait → Probe → 注册 → 测试 | [development.md](copilot-rules/development.md)           |
| 编写测试用例 | 使用样本 URL → 限制测试帧数                   | [testing.md](copilot-rules/testing.md)                   |
| 错误处理     | 使用 TaoError/TaoResult → 中文错误消息        | [rust.md](copilot-rules/rust.md) §错误处理               |
| FFI 导出     | SAFETY 注释 → null 检查 → 向后兼容            | [development.md](copilot-rules/development.md) §FFI 规则 |
| 性能优化     | 避免分配 → 复用缓冲 → SIMD                    | [performance.md](copilot-rules/performance.md)           |
| 添加日志     | 使用 tracing 宏 → 中文消息 → 合适级别         | [performance.md](copilot-rules/performance.md) §日志     |
| 文件管理     | 临时文件放 data/ → 样本用 URL                 | [testing.md](copilot-rules/testing.md) §数据管理         |
| **工具执行** | **检测 MCP → 优先使用 → 回退终端**            | 本文档 §MCP 服务器优先级规则                             |

### MCP 工具映射表

| 任务类型  | Rust-MCP-Server               | Rust-Analyzer-MCP                        | 备注     |
| --------- | ----------------------------- | ---------------------------------------- | -------- |
| 构建/检查 | `cargo-build` / `cargo-check` | -                                        | 优先 MCP |
| 代码审查  | `cargo-clippy`                | -                                        | 优先 MCP |
| 格式化    | `cargo-fmt`                   | `mcp_rust-analyzer_rust_analyzer_format` | MCP only |
| 符号定义  | -                             | `rust-analyzer-goto-definition`          | MCP only |
| 查找引用  | -                             | `rust-analyzer-references`               | MCP only |
| 代码诊断  | -                             | `rust-analyzer-diagnostics`              | MCP only |
| 代码完成  | -                             | `rust-analyzer-completion`               | MCP only |
| 快速修复  | -                             | `rust-analyzer-code-actions`             | MCP only |

---

## 特别提醒

### ⚠️ 常见错误

1. ❌ **使用英文注释或日志** → 必须使用中文
2. ❌ **提交前不运行检查** → 必须执行 fmt/clippy/check/test
3. ❌ **忽略 Clippy 警告** → 必须修复所有警告
4. ❌ **下载测试样本到本地** → 必须使用 URL 直接访问
5. ❌ **在根目录创建文件** → 必须按规范放到对应目录
6. ❌ **使用 `unwrap()` 处理可能失败的操作** → 必须正确处理错误
7. ❌ **硬编码魔法数字** → 使用常量或配置项
8. ❌ **不检测 MCP 服务器可用性** → 必须优先使用 MCP 工具（如果可用）
9. ❌ **混用 MCP 和终端工具** → 同一任务保持工具一致

### ✅ 最佳实践

1. ✅ 单个函数不超过 50 行,复杂逻辑拆分
2. ✅ 所有公开接口使用 `///` 文档注释
3. ✅ 测试只解码前 5-10 帧验证功能
4. ✅ 使用 `TaoError::NotImplemented` 而非 `todo!()`
5. ✅ FFI 函数所有 `unsafe` 块添加 `// SAFETY:` 注释
6. ✅ 枚举类型使用 `#[non_exhaustive]` 便于扩展
7. ✅ 复杂数据处理使用迭代器而非 Vec
8. ✅ 任务开始时检测 MCP 服务器可用性
9. ✅ 优先使用 MCP 工具执行 cargo 和代码分析命令
10. ✅ MCP 工具不可用时优雅回退到终端命令

---

## 获取帮助

- **模块规则**: 查看 `.github/copilot-rules/` 目录下的对应文件
- **样本 URL**: 查看 `samples/SAMPLE_URLS.md`
- **样本使用规范**: 查看 `samples/SAMPLES.md`

---

**祝编码愉快！记住: 中文注释,0 警告,先测试再提交!** 🚀
