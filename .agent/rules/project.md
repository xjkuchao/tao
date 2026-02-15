# 项目概述与架构

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
│   ├── tao-codec/          # 编解码器框架 (对标 libavcodec)
│   ├── tao-format/         # 容器格式框架 (对标 libavformat)
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
