# 日志规范

- 日志使用 `log` crate 的标准宏 (`error!`, `warn!`, `info!`, `debug!`, `trace!`).
- 库 crate (tao-core, tao-codec 等) 只使用 `log` 宏, 不初始化日志后端.
- 可执行文件 (tao-cli, tao-probe) 负责初始化日志后端 (使用 `env_logger`).
- 日志内容使用中文, 关键操作必须有日志记录:
    - `info!`: 打开文件, 识别格式, 开始/完成转码
    - `debug!`: 流信息, 编解码器参数, 数据包细节
    - `warn!`: 可恢复错误, 损坏但可跳过的数据
    - `error!`: 致命错误, 无法继续处理
