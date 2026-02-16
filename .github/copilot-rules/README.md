# GitHub Copilot 规则文件目录

本目录包含 Tao 项目的模块化开发规范文件，专为 GitHub Copilot 优化。

---

## 📋 规则文件列表

### 📁 [project.md](project.md) - 项目概述和架构

- 项目简介和三种使用方式
- 完整项目结构和目录组织
- crate 依赖关系详解
- 核心架构：编解码管线、注册表模式、I/O 抽象、FFI 导出
- 代码组织规范
- 执行计划管理
- 根目录文件管理规则
- Git 管理规范

### 🦀 [rust.md](rust.md) - Rust 编码规范

- 类型与安全（禁止 unwrap/expect）
- 并发与 FFI（Send trait、catch_unwind、SAFETY 注释）
- 代码格式化（rustfmt、100字符行宽）
- 枚举设计（#[non_exhaustive]）
- 错误处理规范（TaoError、TaoResult）
- 安全规范（敏感信息、FFI 安全、内存安全）

### 🛠️ [development.md](development.md) - 开发规则

- 新增编解码器开发流程（6步法）
- Decoder/Encoder trait 实现示例
- 新增容器格式开发流程
- Demuxer/Muxer trait 实现示例
- FormatProbe 实现
- FFI 导出规则（向后兼容、null 检查、错误处理、内存管理）
- 注册表模式详解
- C 头文件生成

### 🧪 [testing.md](testing.md) - 测试规范

- 测试基本要求和强制规则
- 测试用例开发流程（5步法）
- 测试样本 URL 使用规范
- 测试用例编写标准（文件命名、断言、注释）
- 测试覆盖范围（编解码器、容器格式、滤镜）
- 手动播放测试规范（时长限制、进程终止）
- 测试文件和临时文件管理
- 新增测试样本流程

### ✨ [quality.md](quality.md) - 代码质量和提交规范

- 代码质量要求（0 警告容忍）
- 代码审查清单（7项检查点）
- 代码提交规范（格式、类型）
- 提交前检查流程（fmt → clippy → check → test）
- 0 警告容忍策略
- 自动提交规则
- 注释规范（文档注释、FFI 安全注释、特殊注释）
- 代码风格（命名规范、格式化）

### ⚡ [performance.md](performance.md) - 性能优化和日志规范

- 内存管理（避免分配、缓冲区复用、零拷贝）
- 数据处理（迭代器、SIMD）
- 性能测试（基准测试）
- 日志规范（tracing、日志级别）
- 日志模块位置
- 日志输出规则（控制台、文件）
- 日志文件管理（命名、维护、清理）
- AI 调试规范
- 性能优化清单

---

## 🎯 设计原则

### 1. 模块化设计

每个规则文件专注于特定领域，便于快速查找和加载：

- **project.md** - 项目整体架构和组织
- **rust.md** - Rust 语言特定规范
- **development.md** - 开发流程和实践
- **testing.md** - 测试相关规范
- **quality.md** - 质量控制和提交
- **performance.md** - 性能和日志

### 2. 分类合理

按开发阶段和关注点分类：

```
项目基础 (project.md)
    ↓
Rust 编码 (rust.md)
    ↓
功能开发 (development.md)
    ↓
测试验证 (testing.md)
    ↓
质量保证 (quality.md)
    ↓
性能优化 (performance.md)
```

### 3. 避免重复

- 每个规则只在一个文件中定义
- 通过相对路径交叉引用
- 主指令文件（`copilot-instructions.md`）提供核心规则摘要和索引

### 4. 高效上下文利用

- 主指令文件包含最高频的核心规则（200-300行）
- 详细规则文件按需加载（每个 100-200行）
- 总文件数控制在合理范围（1主文件 + 6详细文件）

---

## 📖 使用说明

### GitHub Copilot 自动加载

- GitHub Copilot 会自动读取 `.github/copilot-instructions.md`
- 详细规则文件通过主文件索引引导加载
- 在编辑特定模块时，Copilot 会参考对应的详细规则文件

### 手动查阅

开发者可以根据需要查阅对应的规则文件：

```bash
# 了解项目结构
cat .github/copilot-rules/project.md

# 查看测试规范
cat .github/copilot-rules/testing.md

# 检查提交规范
cat .github/copilot-rules/quality.md
```

### 规则索引

主指令文件提供了快速参考表，可快速定位具体场景的规则：

| 场景         | 关键规则                      | 详细文档                                                        |
| ------------ | ----------------------------- | --------------------------------------------------------------- |
| 编写代码     | 中文注释、日志、AI 上下文输出 | [copilot-instructions.md](../copilot-instructions.md) §语言要求 |
| 提交代码     | fmt → clippy → test           | [quality.md](quality.md) §提交规范                              |
| 新增编解码器 | 创建模块 → 注册 → 测试        | [development.md](development.md) §编解码器                      |
| 编写测试     | 使用样本 URL                  | [testing.md](testing.md) §测试流程                              |
| FFI 导出     | SAFETY 注释 → null 检查       | [development.md](development.md) §FFI 规则                      |
| 日志记录     | tracing 宏 → 中文消息         | [performance.md](performance.md) §日志规范                      |

---

## 🔄 规范更新和同步

### 两套规范系统

本项目维护两套开发规范：

1. **Cursor 规则** - `.cursor/rules/`（16个 `.mdc` 文件）
    - 作用：为 Cursor AI 优化的模块化规则
    - 特点：YAML front matter、globs 匹配、编号系统
    - 大小：约 858 行（总计）

2. **Copilot 规则** - `.github/`（本规则系统）
    - 作用：为 GitHub Copilot 优化的规则文件
    - 特点：纯 Markdown、分层索引、上下文优化
    - 大小：约 300 行（主文件）+ 约 600 行（详细文件）

### 更新流程

当需要更新规范时：

1. 确定变更范围和影响的规则
2. 同时更新两套规范：
    - `.cursor/rules/*.mdc` - Cursor 版本
    - `.github/copilot-instructions.md` 和 `.github/copilot-rules/*.md` - Copilot 版本
3. 确保两者内容保持一致
4. 提交时说明规范变更内容：

```bash
git add .cursor/rules/ .github/
git commit -m "docs: 更新 XXX 规范，增加 YYY 要求"
```

---

## 📚 相关文档

- **Cursor 规则** - [.cursor/rules/](../../.cursor/rules/)
- **主指令文件** - [copilot-instructions.md](../copilot-instructions.md)
- **样本 URL 清单** - [samples/SAMPLE_URLS.md](../../samples/SAMPLE_URLS.md)
- **样本使用规范** - [samples/SAMPLES.md](../../samples/SAMPLES.md)

---

## ✅ 总结

本规则文件系统为 GitHub Copilot 提供了：

- ✅ **分类存放** - 6个主题模块，职责清晰
- ✅ **技能精准** - 每个文件专注特定领域，规则明确
- ✅ **高效上下文** - 双层架构，核心规则摘要 + 详细规则按需加载
- ✅ **避免重复** - 每个规则只定义一次，交叉引用避免冗余
- ✅ **分类合理** - 按项目→开发→测试→质量→优化的流程组织

**记住核心原则：中文注释，0 警告，先测试再提交！** 🚀
