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
4. **保持同步**: 与根目录的 `AGENTS.md` 保持内容一致

## 使用说明

- Cursor 会自动加载本目录下的规则文件
- 带有 `alwaysApply: true` 的规则始终生效
- 其他规则根据文件类型和上下文（通过 `globs`）自动应用
- 根目录的 `AGENTS.md` 保留作为完整规范参考

## 维护说明

更新规范时需要：

1. 同时更新对应的 `.cursor/rules/*.mdc` 文件和 `AGENTS.md`
2. 确保两者保持一致
3. 提交时说明规范变更内容

## 文件命名规范

- 使用两位数字前缀（00-15）确保文件顺序
- 使用 `.mdc` 扩展名（Markdown with Cursor rules）
- 文件名使用下划线分隔的英文描述
