# 开发规则 - 编解码器, 容器格式, FFI

## 新增编解码器

- 在 `tao-codec/src/` 下创建独立子模块 (如 `decoders/h264/`, `encoders/aac/`)
- 实现 `Decoder` 或 `Encoder` trait
- 提供工厂函数并注册到 `CodecRegistry`
- 编写单元测试验证基本编解码流程

## 新增容器格式

- 在 `tao-format/src/` 下创建独立子模块 (如 `demuxers/mp4/`, `muxers/wav/`)
- 实现 `Demuxer` 或 `Muxer` trait
- 实现 `FormatProbe` trait 以支持自动格式识别
- 提供工厂函数并注册到 `FormatRegistry`

## FFI 规则

- FFI 函数签名变更须向后兼容, 不得删除已发布的导出函数
- 新增导出函数须同步更新 C 头文件
- 所有指针参数必须检查 null
- 所有导出函数必须使用 `#[no_mangle]` 和 `extern "C"`
- 导出函数以 `tao_` 前缀命名
