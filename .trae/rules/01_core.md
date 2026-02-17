# 核心规范

## 项目概述、结构和核心架构

**Tao (道)** 是一个用纯 Rust 编写的多媒体处理框架, 目标是全功能复刻 FFmpeg. 项目提供三种使用方式:

1. Rust 库: 其他 Rust 项目可通过 `tao` crate 直接调用  
2. C FFI (DLL/SO): 通过 `tao-ffi` crate 导出 C 兼容接口, 供 C/C++ 等语言调用  
3. 命令行工具: `tao` (对标 ffmpeg), `tao-probe` (对标 ffprobe) 和 `tao-play` (对标 ffplay) 可执行文件

### 项目结构（Cargo Workspace）

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

### 依赖关系

```
tao-core (无外部依赖, 最底层)
  ↑
tao-codec, tao-format, tao-filter, tao-scale, tao-resample
  ↑
tao-ffi, tao-cli, tao-probe, tao-play
  ↑
tao (门面库, re-export 所有库 crate)
```

### 编解码管线

```
[输入文件] → Demuxer → Packet → Decoder → Frame → [Filter] → Encoder → Packet → Muxer → [输出文件]
```

### 注册表模式

- 编解码器/格式通过工厂函数注册到全局注册表  
- 运行时按 CodecId/FormatId 查找并创建实例  
- 支持按优先级选择多个同 ID 的实现

### I/O 抽象

- IoContext 提供统一的读写接口，支持 文件/内存/网络 等后端

### FFI 导出

- `tao-ffi` 编译为 cdylib + staticlib；导出函数以 `tao_` 前缀命名；`#[no_mangle]` + `extern "C"`；由 Tao 分配的内存必须通过对应 `tao_*_free()` 释放

---

## 语言规范（始终生效）

- 项目全部使用中文：代码注释、日志、错误信息、AI 上下文输出、文档内容  
- 开发交流与文档统一中文；标点统一使用英文标点  
- 代码标识符使用英文并遵循 Rust 惯例；提交信息使用中文

---

## 代码组织与文件管理

- 模块化拆分；函数建议不超过 50 行；公共类型集中定义  
- 新增编解码器/容器格式按既有 crates 目录结构放置  
- 执行计划必须写入 `plans/`，命名 `{功能模块}_{任务描述}.md`，内容含背景/步骤/产出/依赖/验收  
- 根目录严格控制文件：配置与核心文档；其他分别放入对应目录（docs/examples/samples/data 等）
