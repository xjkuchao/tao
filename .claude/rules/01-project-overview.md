# 项目概述与核心架构

## 项目概述

**Tao (道)** 是纯 Rust 编写的多媒体处理框架, 目标是全功能复刻 FFmpeg. 提供三种使用方式:

1. **Rust 库**: 通过 `tao` crate 直接调用
2. **C FFI (DLL/SO)**: 通过 `tao-ffi` crate 导出 C 兼容接口
3. **命令行工具**: `tao` (对标 ffmpeg), `tao-probe` (对标 ffprobe), `tao-play` (对标 ffplay)

## 项目结构

Cargo Workspace 多 crate 架构:

```
tao/
├── Cargo.toml              # Workspace 根配置 + 门面库
├── plans/                  # AI 执行计划文件
├── samples/                # 测试样本清单 (URL, 不含文件)
├── data/                   # 临时文件目录 (不提交)
├── logs/                   # 日志目录 (不提交)
├── crates/
│   ├── tao-core/           # 核心类型与工具 (对标 libavutil)
│   ├── tao-codec/          # 编解码器框架 (对标 libavcodec)
│   ├── tao-format/         # 容器格式框架 (对标 libavformat)
│   ├── tao-filter/         # 滤镜框架 (对标 libavfilter)
│   ├── tao-scale/          # 图像缩放 (对标 libswscale)
│   ├── tao-resample/       # 音频重采样 (对标 libswresample)
│   └── tao-ffi/            # C FFI 导出层
├── bins/
│   ├── tao-cli/            # tao 命令行工具 (对标 ffmpeg)
│   ├── tao-probe/          # tao-probe 探测工具 (对标 ffprobe)
│   └── tao-play/           # 播放器 (对标 ffplay)
├── tests/                  # 集成测试
└── examples/               # 使用示例
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

## 核心架构

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

IoContext 提供统一读写接口, 支持 FileBackend / MemoryBackend / NetworkBackend.

### FFI 导出

tao-ffi 编译为 cdylib + staticlib:

- 所有导出函数以 `tao_` 前缀命名
- 使用 `#[no_mangle]` 和 `extern "C"` 确保 ABI 兼容
- 由 Tao 分配的内存必须通过对应的 `tao_*_free()` 释放
