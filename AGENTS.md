# AGENTS.md

本文件是 Tao 项目的统一 AI 开发规范, 适用于 VSCode, Cursor, Windsurf, Claude Code, Codex 等所有工具。
目标是在不依赖工具特定机制的前提下, 提供一致、可执行、可审计的开发规则。

## 1. 总体原则

- 项目定位: 纯 Rust 多媒体框架, 目标全功能复刻 FFmpeg。
- 重要(强制自研约束): 禁止依赖任何外部多媒体能力库实现核心功能。编解码器、滤镜、缩放、重采样等核心能力必须在 Tao 仓库内自行实现。允许使用通用基础库(如日志、错误处理、容器类型), 但不得用于替代 Tao 的媒体算法实现。
- 架构形态: Cargo Workspace, 多 crate 分层。
- 开发风格: 模块化、可测试、可维护、可回归。
- 规则优先级: 安全与正确性 > 向后兼容 > 性能 > 开发效率。

## 2. 语言规范

- 重要: 项目内容统一使用中文, 包括:
    - 代码注释
    - 控制台日志
    - 错误信息
    - AI 过程输出
    - 文档内容
- 标点使用英文标点。
- 代码标识符(变量/函数/类型等)使用英文, 遵循 Rust 命名惯例。
- 重要: 项目里所有文件夹名称、文件名称, 以及源代码内变量名、函数名、类型名、模块名等标识都只能使用英文。
- 提交信息使用中文。

## 3. 项目结构与职责

```text
tao/
├── Cargo.toml
├── AGENTS.md
├── plans/
├── samples/
├── data/
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

- crate 依赖方向:
    - `tao-core` 为底层。
    - `tao-codec/tao-format/tao-filter/tao-scale/tao-resample` 依赖 `tao-core`。
    - `tao-ffi` 与各 `bins` 位于上层。
- 编解码管线:
    - `输入 -> Demuxer -> Packet -> Decoder -> Frame -> Filter -> Encoder -> Packet -> Muxer -> 输出`。

## 4. 代码组织规则

- 模块化设计, 单一函数建议不超过 50 行。
- 复杂逻辑必须拆分子模块, 每个子模块职责单一。
- 重要: 编写编解码器、解析器、滤镜、muxer、demuxer、scale、resample 等模块时:
    - 若完整实现较简单, 可在单文件内实现。
    - 若实现复杂, 必须建立独立子目录并按功能拆分子文件。
    - 子模块职责必须清晰, 命名与边界可读、可维护。
- 公共类型集中定义, 避免跨处散落。
- 新增编解码器放在 `crates/tao-codec/src/` 对应子目录。
- 新增容器格式放在 `crates/tao-format/src/` 对应子目录。
- 严禁在根目录随意新增文件。

允许的根目录文件类型:

- 项目配置: `Cargo.toml`, `.rustfmt.toml`, `.gitignore`
- 核心文档: `README.md`, `README_CN.md`, `AGENTS.md`, `LESSONS_LEARNED.md`, `LESSONS_LEARNED_CN.md`
- 许可证: `LICENSE*`

文件放置要求:

- 执行计划: `plans/`
- 示例: `examples/`
- 测试: `tests/`
- 样本清单: `samples/`
- 临时产物: `data/`

## 5. 执行计划与协作

- 涉及多步骤或跨模块任务时, 必须在 `plans/` 写计划文件。
- 命名: `{功能模块}_{任务描述}.md`。
- 计划必须包含:
    - 背景与目标
    - 分步任务与预期产出
    - 依赖与前置条件
    - 验收标准
    - 进度标记(支持断点续执行)
- 计划内容需可被其他 AI 工具直接接续执行。

## 6. Rust 编码规范

### 6.1 类型与 API

- 所有公开函数参数和返回值必须显式类型。
- 统一使用 `TaoError` / `TaoResult`。
- crate 内部专用错误推荐 `thiserror`。
- 推荐:
    - `struct` 表示数据
    - `enum` 表示状态/变体
    - `type` 表示别名
- 跨 crate 接口优先 trait 对象(`Box<dyn Trait>`), 内部实现可用泛型。

### 6.2 安全与并发

- `Decoder/Encoder/Demuxer/Muxer/Filter` 等核心 trait 必须满足 `Send`。
- 禁止无依据 `unwrap()/expect()`。
- 禁止 `todo!()` 进入可执行路径。
- FFI 导出函数禁止 panic, 必须 `catch_unwind` 或证明无 panic 路径。
- `unsafe` 代码必须有 `// SAFETY:` 注释。
- FFI 指针参数必须检查 null。

### 6.3 格式化与枚举

- 使用 `rustfmt`, 行宽上限 100, 缩进 4 空格。
- 编解码器 ID、像素格式、采样格式等扩展型枚举使用 `#[non_exhaustive]`。

## 7. 错误处理规范

- 所有 I/O 操作必须显式处理错误, 禁止吞错。
- 损坏数据返回 `TaoError::InvalidData`, 不得 panic。
- 未实现功能返回 `TaoError::NotImplemented`。
- 错误信息必须中文且包含必要上下文。

## 8. 编解码器/格式/FFI 开发规范

### 8.1 新增编解码器

- 在 `tao-codec` 下创建独立模块。
- 实现 `Decoder` 或 `Encoder` trait。
- 提供工厂函数并注册到 `CodecRegistry`。
- 增加基本流程单元测试。

### 8.2 新增容器格式

- 在 `tao-format` 下创建独立模块。
- 实现 `Demuxer` 或 `Muxer` trait。
- 实现 `FormatProbe` 以支持自动探测。
- 提供工厂函数并注册到 `FormatRegistry`。

### 8.3 FFI 兼容性

- 不得删除已发布导出函数。
- 新增导出函数需同步更新 C 头文件。
- 导出函数必须 `#[no_mangle]` + `extern "C"` + `tao_` 前缀。

## 9. 日志规范

- 统一使用 `tracing` 记录日志。
- 库 crate 只记录日志, 不初始化日志后端。
- 可执行程序负责初始化日志系统。
- 日志内容必须中文。

### 9.1 级别定义(强制)

- `error!`: 不可恢复错误, 功能无法继续。
- `warn!`: 可恢复异常或降级路径。
- `info!`: 关键生命周期事件(默认可见)。
- `debug!`: 内部状态与决策。
- `trace!`: 高频热路径细节(每帧/每包)。

禁止事项:

- 热路径使用 `info!` 及以上级别。
- 将正常流程(如 EOF)记为 `error!/warn!`。
- 输出超长或大块二进制日志。

### 9.2 输出与文件

- 日志目录: `logs/`(可选 `data/logs/`)。
- 命名: `{prefix}.{YYYY-MM-DD}.log`。
- 前缀:
    - `tao-cli`
    - `tao-probe`
    - `tao-play`
- 调试优先读取日志文件, 不依赖终端滚动输出。

## 10. 测试规范

### 10.1 基本规则

- 测试文件命名: `tests/{feature}_pipeline.rs`。
- 测试函数命名: `test_{component}_{scenario}`。
- 每个断言都应包含失败说明。
- 复杂测试逻辑需要分步注释。

### 10.2 样本与数据源

- 所有样本优先使用 HTTPS URL 直接访问。
- 样本来源: `https://samples.ffmpeg.org/`。
- 样本清单维护在 `samples/SAMPLE_URLS.md`。
- 若新增样本, 必须更新样本清单并提交。
- 重要: 单元测试(`#[cfg(test)]` 模块)和 `tests/` 目录下的集成测试禁止依赖 `data/` 目录中的本地临时文件。`data/` 是临时产物目录, 内容会被清理且不入 Git, 以此为数据源会导致测试因环境差异而失败。
- 测试数据的合法来源(按优先级):
    1. 代码内自构造/填充的测试数据(字节数组、结构体等)。
    2. 固定可达的远程 URL(如 `https://samples.ffmpeg.org/...`), 不随项目变动而失效。
- `plans/` 下的快速原型验证脚本(如 `decoder_compare.rs`)不受此限制, 开发者自行保证所需 `data/` 文件存在。

### 10.3 性能与稳定性

- 功能验证通常只需解码前 5-10 帧。
- 避免长时间测试影响 CI 与迭代速度。
- 大型测试可使用 `#[ignore]` 并说明触发条件。

### 10.4 手动播放测试

- 禁止完整播放全片进行验证。
- 默认播放 10 秒, 最多 30 秒。
- 启动播放必须带超时保护:
    - Linux/macOS: `timeout 15|30 ...`
    - Windows: `Start-Process` + `Start-Sleep` + `TASKKILL /F /IM tao-play.exe /T`
- Windows 禁止使用 `TASKKILL /PID` 作为标准流程。

## 11. 测试文件与临时文件管理

- 临时文件统一放入 `data/`。
- `data/` 下内容不提交 Git。
- 日志目录保留 `.gitkeep`, 日志文件不入库。
- 临时文件命名建议:
    - `tmp_{feature}_{pid}.{ext}`
- 测试结束后应主动清理临时产物。

## 12. 注释规范

- 所有注释使用中文。
- 公开 API 使用 `///` 文档注释。
- crate 级说明使用 `//!`。
- FFI 函数写明 `# Safety`。
- `unsafe` 代码必须有 `// SAFETY:`。
- 临时方案明确 `TODO:` 与后续处理方向。

## 13. 代码质量规范

- 单次变更必须完整、可编译、可测试。
- 禁止保留调试代码:
    - 临时 `println!/eprintln!`
    - 调试分支
    - 未使用变量/函数/导入
- 禁止引入与任务无关改动。
- 禁止重复实现已有能力。
- 优先修复根因, 避免表层补丁。

## 14. 性能规范

- 减少不必要分配, 优先借用与复用。
- 热路径避免多余分支与格式化开销。
- 大数据传递优先零拷贝方案(如 `bytes::Bytes`)。
- 性能优化需可测量, 使用 `benches/` + `cargo bench`。

## 15. 安全规范

- 禁止在代码中硬编码密钥、令牌、密码、证书。
- 禁止提交敏感文件和本地配置。
- 所有外部输入(文件/网络/FFI)必须做边界检查。
- 处理不可信媒体数据时避免 panic/越界/未定义行为。

## 16. 提交流程规范

提交前必须按顺序执行:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo check --workspace --all-targets --all-features`
4. `cargo test --workspace --all-targets --all-features --no-fail-fast`
5. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`

提交信息格式:

- `feat: ...`
- `fix: ...`
- `refactor: ...`
- `style: ...`
- `chore: ...`
- `test: ...`
- `docs: ...`

要求:

- 提交信息必须中文、简洁、准确。
- 提交范围仅包含本轮任务相关文件。
- 每完成一轮可验证功能变更, 应及时提交。

## 17. 冲突处理规则

当规则出现重叠或冲突时按以下优先级执行:

1. 安全与稳定性
2. 向后兼容性
3. 架构一致性
4. 性能
5. 开发效率

若无法自动判定, 在变更说明中记录取舍依据。
