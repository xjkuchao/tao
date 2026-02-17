# 代码提交规范

## 提交信息格式

使用 Conventional Commits, **必须使用中文**:

- `feat: 功能描述` - 新增功能
- `fix: 问题描述` - 修复 Bug
- `refactor: 重构描述` - 代码重构
- `style: 样式调整` - 代码格式调整
- `chore: 其他描述` - 构建/工具/依赖更新
- `test: 测试描述` - 新增或修改测试
- `docs: 文档描述` - 文档更新

## 提交前检查 (强制)

必须按顺序全部通过:

1. `cargo fmt --check` - 格式一致
2. `cargo clippy -- -D warnings` - 无 Clippy 警告
3. `cargo check` - 编译通过
4. `cargo test` - 测试通过

## 自动提交规则

- 每完成一轮功能开发且检查全部通过后, 自动提交
- 提交范围仅包含当轮功能涉及的文件
- 提交信息准确概括本轮变更内容
