# 日志规范

## 基本原则

- 使用 `tracing` crate: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- 后端使用 `tracing-subscriber` 和 `tracing-appender`
- 库 crate 只使用 `tracing` 宏, 不初始化日志后端
- 可执行文件 (tao-cli, tao-probe, tao-play) 负责初始化日志系统
- 日志内容使用中文

## 日志级别体系

### 命令行级别映射

| verbosity | 参数 | 过滤规则 | 用途 |
|-----------|------|----------|------|
| 0 (默认) | 无 | `info` | 关键生命周期事件 |
| 1 | `-v` | `debug` | 内部状态/决策 |
| 2 | `-vv` | tao crate=`trace`, 第三方=`info` | 项目详细数据流 |
| 3+ | `-vvv` | 全局 `trace` | 含第三方依赖的完整追踪 |

- Console 和 File 统一使用相同过滤级别
- `TAO_LOG` 环境变量可覆盖命令行级别: `TAO_LOG="tao_codec::decoders=trace,info"`

### 日志级别使用规范 (强制)

#### `error!` - 不可恢复的致命错误

程序无法继续运行或丧失核心功能. 极低频率, 通常伴随程序退出.

```rust
error!("打开文件失败: {}", path);
```

#### `warn!` - 可恢复的异常/降级

出现问题但程序可以继续运行. 低频率, 每个异常场景出现一次.

```rust
warn!("VSync 渲染器创建失败, 回退到无 VSync");
```

#### `info!` - 关键生命周期事件 (默认可见)

程序启动/关闭, 资源打开/释放, 用户操作结果. 低到中频率. info 日志应能完整描述 "发生了什么".

```rust
info!("正在打开: {}", path);
info!("视频尺寸 {}x{}", width, height);
```

#### `debug!` - 内部状态/决策 (需 `-v`)

开发调试信息. 中频率, 可能每秒数条. 帮助开发者理解 "为什么这样做".

```rust
debug!("选择解码器: codec_id={:?}, 参数={:?}", id, params);
```

#### `trace!` - 高频热路径数据 (需 `-vv`)

每帧/每包/每次回调级别的详细数据. 极高频率. 热路径中避免复杂格式化.

```rust
trace!("解码视频帧: PTS={}, size={}x{}", pts, w, h);
```

### 禁止事项

- **禁止** 在热路径 (每帧/每包) 中使用 `info!` 或更高级别
- **禁止** 在 `error!`/`warn!` 中记录正常流程 (如 EOF 不是错误)
- **禁止** 日志消息中包含大量二进制数据或超长字符串
- **禁止** 在性能关键路径中使用复杂的 `format!` 表达式作为日志参数

## 日志文件管理

- 日志存放在 `logs/` 目录, 所有 `*.log` 文件被 `.gitignore` 忽略
- 命名格式: `{file_prefix}.{YYYY-MM-DD}.log`
- 文件前缀: tao-cli → `tao-cli`, tao-probe → `tao-probe`, tao-play → `tao-play`
- 自动按日期切换, 可配置保留天数 (默认 30 天)

## AI 调试规范

1. **优先读取 `logs/` 目录下日志文件, 而非 console 输出**
2. 日志文件位于 `logs/{file_prefix}.{YYYY-MM-DD}.log`
3. 测试前清理历史日志: `rm -f logs/*.log` (Linux) 或 `Remove-Item logs/*.log` (Windows)
4. 需要更多细节时使用 `-v` / `-vv` / `-vvv` 运行
5. 可通过 `TAO_LOG` 环境变量精细控制:
   ```bash
   TAO_LOG="tao_codec=trace,info" cargo run --package tao-play -- file.avi
   ```
