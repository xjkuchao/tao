# 背景与目标

- 背景: `main` 合并 `h264` 后, `cargo clippy --workspace --all-targets --all-features -- -D warnings` 在 `tao-play` 失败, 报错 `use of undeclared type Path`。
- 目标: 修复 `tao-play` 编译告警/错误点, 通过仓库要求的 5 项检查, 完成提交并推送主干。

# 分步任务与预期产出

1. 定位与修复编译问题。
- 任务: 在 `bins/tao-play/src/gui.rs` 补充 `Path` 导入, 且不引入无关改动。
- 产出: 最小化代码修复, `clippy` 不再报该错误。

2. 执行完整质量检查链路。
- 任务: 依次执行 `fmt -> clippy -> check -> test -> doc`。
- 产出: 五项命令全部通过, 无 warnings/error。

3. 提交与推送。
- 任务: 仅提交本轮相关文件, 使用中文提交信息, 推送到 `origin/main`。
- 产出: 远端主干包含修复提交, 本地与远端同步。

# 依赖与前置条件

- Rust stable 工具链可用。
- 当前分支为 `main`, 且具备远端推送权限。
- 工作区无与本轮任务冲突的未提交更改。

# 验收标准

- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过。
- `cargo check --workspace --all-targets --all-features` 通过。
- `cargo test --workspace --all-targets --all-features --no-fail-fast` 通过。
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` 通过。
- 修复提交已推送到 `origin/main`。

# 进度标记

- [x] 步骤 1: 定位与修复编译问题。
- [x] 步骤 2: 执行完整质量检查链路。
- [ ] 步骤 3: 提交与推送。
