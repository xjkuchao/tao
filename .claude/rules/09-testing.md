# 测试规范

## 基本要求

- 代码修改后必须执行 `cargo check` 与 `cargo test`
- 新增编解码器或容器格式时必须编写测试
- 集成测试: `tests/` 目录, 单元测试: 源文件内 `#[cfg(test)]` 模块
- 测试命名: `test_{component}_{scenario}`, 蛇形命名法
- 覆盖: 正常流程, 边界情况, 错误情况

## 样本使用规范

- **所有样本使用 HTTPS URL 直接访问, 不下载到本地**
- 样本来源: https://samples.ffmpeg.org/
- 样本清单: `samples/SAMPLE_URLS.md`
- 验证样本: `ffprobe <URL>`
- 如无合适样本, 访问 https://samples.ffmpeg.org/ 浏览, 添加到 `samples/SAMPLE_URLS.md` 并提交

## 测试用例编写标准

- 文件位置: `tests/{feature}_pipeline.rs`
- 每个 `assert!` 包含中文失败消息
- 只解码前 5-10 帧验证功能, 避免耗时过长
- 临时文件放在 `data/` 目录, 测试结束后清理
- 所有测试假设有网络连接

## 测试覆盖范围

### 编解码器

- 基本解码, 编码 (如实现), 空输入, 损坏数据, Flush 流程, 参数解析

### 容器格式

- 格式探测, 头部解析, 数据包读取, Seek 操作, 多流处理, 损坏文件

### 滤镜

- 基本操作, 参数验证, 链式滤镜, 边界条件

## 手动播放测试

- **禁止完整播放**, 默认播放前 10 秒, 最多 30 秒
- **必须使用超时保护**, 防止进程卡死:

### Windows

```powershell
Start-Process -NoNewWindow cargo -ArgumentList "run","--package","tao-play","--","<URL>"
Start-Sleep -Seconds 30
TASKKILL /F /IM tao-play.exe /T
```

### Linux/macOS

```bash
timeout 30 cargo run --package tao-play -- "<URL>"
```

- 终止进程: Windows 用 `TASKKILL /F /IM tao-play.exe /T`, Linux 用 `pkill -f tao-play`
- **禁止使用 PID 终止** (不可靠)
- 超时终止后检查 `logs/tao-play.*.log` 分析问题

## 临时文件管理

- 所有临时文件放在 `data/` 目录, 永不提交到 Git
- 命名: `tmp_` + 标识信息 + 扩展名 (使用进程 ID 避免并发冲突)
- 推荐使用 RAII 模式自动清理临时文件
