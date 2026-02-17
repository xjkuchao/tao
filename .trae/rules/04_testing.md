# 测试与性能

## 测试规范

- 修改后必须执行 `cargo check` 与 `cargo test`；新增编解码器或容器格式必须编写测试  
- 集成测试在 `tests/`；单元测试在源文件 `#[cfg(test)]` 模块  
- 所有样本使用 HTTPS URL 直接访问，不下载到本地；临时文件在 `data/` 并被 Git 忽略  
- 覆盖正常/边界/错误场景；示例用例与命名规则；帧数限制 5-10 帧加快测试

---

## 手动播放测试规范

### 播放时长限制

- 手动测试音视频播放时, **禁止完整播放**整个文件
- 默认播放 **前 10 秒** 即可验证功能
- 如有必要 (验证 seek/后段内容) 可增加到 **最多 30 秒**
- 播放结束后必须主动终止播放进程

### 播放超时机制 (强制)

**核心原则**: 启动 `tao-play` 播放进程时, **必须使用 `timeout` 命令包裹**, 防止因逻辑 bug、死锁、解码卡死等问题导致播放进程永远不退出, 阻塞后续执行.

#### 超时时间

- 默认超时: **30 秒**
- 短时测试 (仅验证能否启动/前几帧): **15 秒**
- 超时后进程会被强制终止, 不影响后续操作

#### Linux/macOS 用法

使用 `timeout` 命令:

```bash
# 正确: 使用 timeout 包裹, 30 秒后自动终止
timeout 30 cargo run --package tao-play -- "https://example.com/video.mp4"

# 正确: 短时测试, 15 秒超时
timeout 15 cargo run --package tao-play -- "https://example.com/video.mp4"

# 错误: 没有超时保护, 可能永远卡住
cargo run --package tao-play -- "https://example.com/video.mp4"
```

#### Windows 用法

Windows 下没有原生 `timeout` 命令用于限制进程运行时间, 使用后台启动 + 延时终止:

```powershell
# 启动播放后, 等待 30 秒后强制终止
Start-Process -NoNewWindow cargo -ArgumentList "run","--package","tao-play","--","https://example.com/video.mp4"
Start-Sleep -Seconds 30
TASKKILL /F /IM tao-play.exe /T
```

#### 异常处理

- 如果进程在超时前正常退出 (如播放完成或出错), `timeout` 命令会立即返回, 不影响结果
- 如果进程被超时强制终止, 退出码为 124 (Linux), 这是**预期行为**, 不代表测试失败
- 超时终止后应检查日志 (`logs/tao-play.*.log`) 分析是否存在逻辑问题

### 终止播放进程 (Windows)

- Windows 下终止 tao-play 进程时, **必须使用 `TASKKILL /F /IM tao-play.exe /T`**
- **禁止使用 `TASKKILL /F /PID <pid>`** (PID 不可靠)

```powershell
# 正确
TASKKILL /F /IM tao-play.exe /T

# 错误 (PID 不可靠)
TASKKILL /F /PID 12345
```

### 终止播放进程 (Linux/macOS)

- 播放进程通常由 `timeout` 自动终止, 无需手动操作
- 如需手动终止, 使用 `pkill` 或 `killall`:

```bash
# 正确: 按进程名终止
pkill -f tao-play
# 或
killall tao-play

# 错误: 使用 PID (不可靠)
kill -9 12345
```

### 流式播放测试

- `tao-play` 支持 http/https/rtmp 等流式 URL 播放
- **所有测试文件均使用 URL 直接流式播放**, 不下载到本地
- 所有样本 URL 维护在 `samples/SAMPLE_URLS.md` 中

```bash
# 正确: 使用 timeout + URL 进行流式播放测试
timeout 30 cargo run --package tao-play -- "https://samples.ffmpeg.org/flac/Yesterday.flac"

# 查看更多样本 URL
# 请参考 samples/SAMPLE_URLS.md
```

---

## 测试文件与临时文件管理

- 核心原则：样本用 URL；临时文件放 `data/` 且被 Git 忽略；遵循 `samples/SAMPLES.md`
- 目录结构与文件位置约定（tests/benches/samples/data/logs 等）
- 测试命名规范；样本来源/清单与用法；临时文件的创建/命名/清理（RAII 示例）
- 日志保存位置与前缀规范；AI 调试指南
- 清理 `data/` 的跨平台示例；网络/清理/磁盘空间/安全注意事项

---

## 性能优化指南

- 内存：避免不必要分配；缓冲区复用；零拷贝传递（`bytes::Bytes`）  
- 数据处理：迭代器优先；热路径避免分支失败；考虑 SIMD  
- 性能测试：`benches/` + `cargo bench`；使用 profiler 分析瓶颈
