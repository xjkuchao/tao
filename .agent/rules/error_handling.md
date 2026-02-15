# 错误处理

- 所有 I/O 操作 (文件, 网络) 必须处理错误, 禁止吞错.
- 使用 `TaoError` 枚举覆盖所有错误场景 (Io, Codec, Format, Eof, NeedMoreData 等).
- 编解码器和格式处理中遇到的损坏数据应返回 `TaoError::InvalidData`, 不得 panic.
- 未实现的功能返回 `TaoError::NotImplemented`, 不使用 `todo!()` 宏 (会 panic).
