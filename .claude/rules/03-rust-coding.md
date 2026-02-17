# Rust 编码规范

## 类型与安全

- **必须**: 为所有公开函数参数和返回值定义明确的类型
- **禁止**: 随意使用 `unwrap()` / `expect()`, 除非能确保不会 panic (如常量初始化)
- **必须**: 使用 `TaoError` / `TaoResult` 作为统一错误类型
  - crate 内部特定错误使用 `thiserror` 定义
- trait 对象使用 `Box<dyn Trait>`, 泛型用于内部实现, trait 对象用于跨 crate 接口

## 并发与 FFI

- 所有 trait (Decoder, Encoder, Demuxer, Muxer, Filter) 要求 `Send`
- FFI 导出函数中禁止 panic, 必须使用 `catch_unwind` 包装或确保无 panic 路径
- FFI 函数的 `unsafe` 块必须添加 `// SAFETY:` 注释

## 格式化

- 使用 `rustfmt`, 配置见 `.rustfmt.toml`
- 行宽上限 100 字符
- 缩进 4 空格, 不使用 tab

## 枚举设计

- 编解码器 ID, 像素格式, 采样格式等枚举使用 `#[non_exhaustive]`
- 枚举变体使用 PascalCase
