# Tao (道)

Tao 是一个用纯 Rust 编写的多媒体处理框架，目标是全功能复刻 FFmpeg。

## 项目特性

- **纯 Rust 实现**: 内存安全、线程安全、无运行时依赖
- **多种使用方式**:
    - Rust 库: 其他 Rust 项目可通过 `tao` crate 直接调用
    - C FFI (DLL/SO): 通过 `tao-ffi` crate 导出 C 兼容接口
    - 命令行工具: `tao` (对标 ffmpeg) 和 `tao-probe` (对标 ffprobe)
- **模块化架构**: 清晰的 crate 划分，职责分明

## 项目结构

```
tao/
├── crates/           # 库 crate
│   ├── tao-core/     # 核心类型与工具 (对标 libavutil)
│   ├── tao-codec/    # 编解码器框架 (对标 libavcodec)
│   ├── tao-format/   # 容器格式框架 (对标 libavformat)
│   ├── tao-filter/   # 滤镜框架 (对标 libavfilter)
│   ├── tao-scale/    # 图像缩放 (对标 libswscale)
│   ├── tao-resample/ # 音频重采样 (对标 libswresample)
│   └── tao-ffi/      # C FFI 导出层
├── bins/             # 可执行文件
│   ├── tao-cli/      # 命令行工具 (对标 ffmpeg)
│   ├── tao-probe/    # 探测工具 (对标 ffprobe)
│   └── tao-play/     # 播放器工具
├── tests/            # 集成测试
└── benches/          # 性能基准测试
```

## 快速开始

### 构建项目

```bash
# 检查代码
cargo check

# 运行测试
cargo test

# 构建所有 crate
cargo build --all

# 构建发布版本
cargo build --release
```

### 使用命令行工具

```bash
# 探测媒体文件信息
cargo run --package tao-probe -- input.mp4

# 转码示例
cargo run --package tao-cli -- -i input.mp4 -o output.mkv

# 播放媒体文件
cargo run --package tao-play -- input.mp4
```

## 开发规范

**重要**: 本项目使用 [AGENTS.md](AGENTS.md) 作为唯一的项目开发规范文件。

所有开发人员和 AI 工具 (Cursor, Copilot, GitHub Copilot, Claude Code 等) 在参与本项目开发时，**必须严格遵守** AGENTS.md 中规定的所有规则和约定，包括但不限于：

- 代码组织与模块化
- Rust 编码规范
- 错误处理规范
- 测试规范
- 提交规范
- 注释规范 (全部使用中文)
- 代码质量要求 (0 警告容忍)

在开始任何开发工作前，请务必仔细阅读 [AGENTS.md](AGENTS.md)。

## 代码质量检查

提交代码前必须执行以下检查，确保全部通过:

```bash
# 1. 代码格式检查
cargo fmt --check

# 2. Clippy 检查 (0 警告容忍)
cargo clippy -- -D warnings

# 3. 编译检查
cargo check

# 4. 运行所有测试
cargo test
```

**严格要求**: 任何 Clippy 警告都必须在提交前修复，不允许忽略。

## 许可证

本项目使用 MIT 或 Apache-2.0 双许可证 (待定)。

## 贡献

欢迎贡献！在提交 PR 前请确保：

1. 已阅读并遵守 [AGENTS.md](AGENTS.md) 中的所有规范
2. 所有代码质量检查通过 (见上文)
3. 添加了必要的测试
4. 更新了相关文档

## 联系方式

- 问题反馈: 请使用 GitHub Issues
- 开发讨论: (待定)
