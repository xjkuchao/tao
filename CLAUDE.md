# CLAUDE.md — Tao 项目 Claude Code 规则

> 本文档为 AGENTS.md 的浓缩版，供要求精简上下文的工具使用，如有模糊之处请以 AGENTS.md 为准

## 语言规范

- 所有输出(注释、日志、错误信息、过程说明、文档)统一使用**中文**.
- 标点使用英文标点.
- 代码标识符(变量/函数/类型)使用英文, 遵循 Rust 命名惯例.
- **重要**: 项目内所有文件夹和文件命名必须全英文, 禁止使用中文或其他非 ASCII 字符.
- 提交信息使用中文.

## 项目定位

纯 Rust 多媒体框架, 目标全功能复刻 FFmpeg. Cargo Workspace, 多 crate 分层.

规则优先级: 安全与正确性 > 向后兼容 > 性能 > 开发效率.

**禁止依赖任何外部多媒体能力库实现核心功能.** 编解码器、滤镜、缩放、重采样等核心算法必须在本仓库内自行实现.

## 项目结构

```
tao/
├── Cargo.toml
├── AGENTS.md / CLAUDE.md
├── plans/          # 执行计划
├── samples/        # 样本清单
├── data/           # 临时产物(不入库)
├── crates/
│   ├── tao-core/
│   ├── tao-codec/
│   ├── tao-format/
│   ├── tao-filter/
│   ├── tao-scale/
│   ├── tao-resample/
│   └── tao-ffi/
├── bins/
│   ├── tao-cli/
│   ├── tao-probe/
│   └── tao-play/
├── tests/
└── examples/
```

- crate 依赖方向: `tao-core` 为底层, `tao-ffi` 与各 `bins` 位于上层.
- 编解码管线: `输入 -> Demuxer -> Packet -> Decoder -> Frame -> Filter -> Encoder -> Packet -> Muxer -> 输出`.
- 严禁在根目录随意新增文件(仅允许 `Cargo.toml`、`.rustfmt.toml`、`.gitignore`、`README.md`、`AGENTS.md`、`CLAUDE.md`、`LICENSE*`).

## 执行计划

涉及多步骤或跨模块任务时, 必须在 `plans/` 写计划文件, 命名为 `{功能模块}_{任务描述}.md`.

计划必须包含: 背景与目标、分步任务与预期产出、依赖与前置条件、验收标准、进度标记.

## Rust 编码规范

- 公开函数参数和返回值必须显式类型.
- 统一使用 `TaoError` / `TaoResult`, 内部错误推荐 `thiserror`.
- 跨 crate 接口优先 trait 对象(`Box<dyn Trait>`), 内部实现可用泛型.
- `Decoder/Encoder/Demuxer/Muxer/Filter` 等核心 trait 必须满足 `Send`.
- 禁止无依据 `unwrap()/expect()`.
- 禁止 `todo!()` 进入可执行路径.
- `unsafe` 代码必须有 `// SAFETY:` 注释; FFI 指针参数必须检查 null.
- FFI 导出函数禁止 panic, 必须 `catch_unwind` 或证明无 panic 路径.
- 使用 `rustfmt`, 行宽上限 100, 缩进 4 空格.
- 扩展型枚举(编解码器 ID、像素格式等)使用 `#[non_exhaustive]`.
- 函数建议不超过 50 行; 复杂模块必须拆分子文件, 职责单一.

## 错误处理

- 所有 I/O 操作必须显式处理错误, 禁止吞错.
- 损坏数据返回 `TaoError::InvalidData`, 不得 panic.
- 未实现功能返回 `TaoError::NotImplemented`.
- 错误信息必须中文且包含必要上下文.

## 日志规范

- 统一使用 `tracing`; 库 crate 只记录日志, 不初始化后端.
- 日志内容必须中文. 日志文件命名为 `{prefix}.{YYYY-MM-DD}.log`.
- `error!`: 不可恢复错误 | `warn!`: 可恢复异常 | `info!`: 关键生命周期 | `debug!`: 内部状态 | `trace!`: 高频热路径.
- 禁止在热路径使用 `info!` 及以上级别.
- 禁止将正常流程(如 EOF)记为 `error!/warn!`.

## 测试规范

- 测试文件命名: `tests/{feature}_pipeline.rs`.
- 测试函数命名: `test_{component}_{scenario}`.
- 每个断言包含失败说明; 复杂逻辑分步注释.
- 样本优先使用 HTTPS URL 直接访问, 来源: `https://samples.ffmpeg.org/`.
- 样本清单维护在 `samples/SAMPLE_URLS.md`; 新增样本必须更新清单.
- 功能验证通常只需解码前 5-10 帧; 大型测试使用 `#[ignore]`.
- 禁止完整播放全片验证; 默认播放 10 秒, 最多 30 秒.
- Windows 播放测试超时保护: `Start-Process` + `Start-Sleep` + `TASKKILL /F /IM tao-play.exe /T`.
- Unix(Linux/macOS) 播放测试必须带超时保护: `timeout 15|30 ...`.

## 临时文件管理

- 临时文件统一放入 `data/`, 命名建议 `tmp_{feature}_{pid}.{ext}`.
- `data/` 内容不提交 Git; 测试结束后主动清理临时产物.

## 注释规范

- 所有注释使用中文.
- 公开 API 使用 `///`; crate 级说明使用 `//!`.
- FFI 函数写明 `# Safety`; 临时方案明确 `TODO:` 与后续方向.

## 代码质量

- 单次变更必须完整、可编译、可测试.
- 禁止保留调试代码(临时 `println!/eprintln!`、调试分支、未使用导入).
- 禁止引入与任务无关改动; 禁止重复实现已有能力.
- 优先修复根因, 避免表层补丁.

## 性能规范

- 减少不必要分配, 优先借用与复用.
- 热路径避免多余分支与格式化开销.
- 大数据传递优先零拷贝(如 `bytes::Bytes`).
- 性能优化需可测量: `benches/` + `cargo bench`.

## 安全规范

- 禁止硬编码密钥、令牌、密码、证书.
- 所有外部输入(文件/网络/FFI)必须做边界检查.
- 处理不可信媒体数据时避免 panic/越界/未定义行为.

## 提交流程

提交前按顺序执行:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo check`
4. `cargo test`

提交信息格式: `feat|fix|refactor|style|chore|test|docs: 中文简洁描述`.

提交范围仅包含本轮任务相关文件; 每完成一轮可验证功能变更及时提交.

## 新增编解码器/格式

**编解码器**: 在 `tao-codec` 下建立独立模块, 实现 `Decoder`/`Encoder` trait, 注册到 `CodecRegistry`, 增加单元测试.

**容器格式**: 在 `tao-format` 下建立独立模块, 实现 `Demuxer`/`Muxer` trait 和 `FormatProbe`, 注册到 `FormatRegistry`.

**FFI**: 导出函数必须 `#[no_mangle]` + `extern "C"` + `tao_` 前缀; 不得删除已发布导出函数; 新增导出需同步更新 C 头文件.

## 冲突处理

优先级: 安全与稳定性 > 向后兼容性 > 架构一致性 > 性能 > 开发效率.

无法自动判定时, 在变更说明中记录取舍依据.
