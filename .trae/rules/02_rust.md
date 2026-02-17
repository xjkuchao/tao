# Rust 开发规范

## Rust 编码规范

- 明确类型，严禁随意 `unwrap()`/`expect()`（除常量初始化）  
- 统一错误类型：`TaoError` / `TaoResult`；内部特定错误用 `thiserror`  
- trait 对象使用 `Box<dyn Trait>`；泛型用于内部实现；跨 crate 接口用 trait 对象  
- 所有核心 trait 要求 `Send`；FFI 禁止 panic，必要时 `catch_unwind`  
- FFI 的 `unsafe` 块必须加 `// SAFETY:` 注释说明安全前提  
- 使用 `rustfmt`；行宽 100；缩进 4 空格  
- 重要枚举使用 `#[non_exhaustive]`；枚举变体 PascalCase

---

## 错误处理规范

- 覆盖场景：Io/Codec/Format/Eof/NeedMoreData/InvalidData/NotImplemented  
- 编解码/格式遇损坏数据返回 `InvalidData`，不得 panic  
- 未实现功能返回 `NotImplemented`，不使用 `todo!()`  
- 错误信息必须中文且包含上下文

---

## 开发规则（编解码器/容器格式/FFI）

- 编解码器：在 `tao-codec/src/` 建独立子模块；实现 `Decoder/Encoder`；注册到 `CodecRegistry`；配套单测  
- 容器格式：在 `tao-format/src/` 建独立子模块；实现 `Demuxer/Muxer`；实现 `FormatProbe`；注册到 `FormatRegistry`  
- FFI：签名向后兼容；新增导出函数同步 C 头文件；指针参数检查 null；`#[no_mangle]` + `extern "C"`；前缀 `tao_`
