# 注释规范

## 基本要求

- **所有注释使用中文**
- 复杂逻辑必须添加注释说明
- 公开函数和 trait 使用 `///` 文档注释, 说明功能, 参数, 返回值
- 每个 crate 的 `lib.rs` 使用 `//!` 模块文档注释, 说明 crate 用途

## 特殊注释

- **FFI 导出函数**: 必须有 `# Safety` 段落说明安全性要求
- **Workaround**: 必须注释说明原因
- **临时代码**: 使用 `// TODO:` 标记
- **unsafe 块**: 使用 `// SAFETY:` 说明安全前提

## 文档注释示例

```rust
/// 解码一个 H.264 视频帧
///
/// # 参数
///
/// * `packet` - 压缩的视频数据包
///
/// # 返回值
///
/// 返回解码后的视频帧, 如果需要更多数据则返回 `None`
///
/// # 错误
///
/// 如果数据包损坏或格式不支持, 返回 `TaoError::InvalidData`
pub fn decode(&mut self, packet: &Packet) -> TaoResult<Option<Frame>> {
    // ...
}
```
