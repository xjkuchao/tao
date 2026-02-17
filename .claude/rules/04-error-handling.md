# 错误处理规范

## 基本原则

- 所有 I/O 操作 (文件, 网络) 必须处理错误, 禁止吞错
- 使用 `TaoError` 枚举覆盖所有错误场景:
  - `Io` - I/O 错误
  - `Codec` - 编解码错误
  - `Format` - 格式错误
  - `Eof` - 文件结束
  - `NeedMoreData` - 需要更多数据
  - `InvalidData` - 无效数据
  - `NotImplemented` - 未实现的功能

## 特定场景

- 编解码器和格式处理中遇到损坏数据: 返回 `TaoError::InvalidData`, **不得 panic**
- 未实现的功能: 返回 `TaoError::NotImplemented`, **禁止使用 `todo!()` 宏** (会 panic)
- 所有错误信息必须使用中文
- 错误信息应提供足够的上下文信息, 便于调试
