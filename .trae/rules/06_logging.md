# 日志规范

## 基本原则

- 日志使用 `tracing` crate: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- 日志后端使用 `tracing-subscriber` 和 `tracing-appender`
- 库 crate (tao-core, tao-codec 等) 只使用 `tracing` 宏, 不初始化日志后端
- 可执行文件 (tao-cli, tao-probe, tao-play) 负责初始化日志系统
- 日志内容使用中文

## 日志模块位置

- 根 crate 日志模块位于 `src/logging/` (含 tokio 维护任务)
- 各 bin 的轻量日志模块位于 `bins/{name}/src/logging.rs`
  - 不依赖 tokio, 使用 `tracing-appender` 按日滚动
  - 支持 `-v` / `-vv` / `-vvv` 命令行参数和 `TAO_LOG` 环境变量

## 日志级别体系 (重要)

### 命令行级别映射

| verbosity | 参数 | 过滤规则 | 用途 |
|-----------|------|----------|------|
| 0 (默认) | 无 | `info` | 关键生命周期事件 |
| 1 | `-v` | `debug` | 内部状态/决策 |
| 2 | `-vv` | tao crate=`trace`, 第三方=`info` | 项目详细数据流 |
| 3+ | `-vvv` | 全局 `trace` | 含第三方依赖的完整追踪 |

- Console 和 File 统一使用相同的过滤级别
- `TAO_LOG` 环境变量可覆盖命令行级别, 支持精细控制:
  `TAO_LOG="tao_codec::decoders=trace,info"`

### 日志级别使用规范 (强制)

编写代码时, 必须严格按以下标准选择日志级别:

#### `error!` - 不可恢复的致命错误

- 程序无法继续运行或丧失核心功能
- 示例: 文件打开失败, 内存分配失败, 关键依赖不可用
- **频率**: 极低, 通常伴随程序退出或功能完全丧失

```rust
error!("打开文件失败: {}", path);
error!("音频设备初始化失败, 无法播放");
```

#### `warn!` - 可恢复的异常/降级

- 出现问题但程序可以继续运行, 功能可能不完整
- 示例: 数据损坏但可跳过, 回退到次优方案, 性能降级
- **频率**: 低, 每个异常场景出现一次

```rust
warn!("VSync 渲染器创建失败, 回退到无 VSync");
warn!("[Seek] 失败: {}", e);
warn!("音频转换失败, 回退原始数据: {}", e);
```

#### `info!` - 关键生命周期事件 (默认可见)

- 程序启动/关闭, 资源打开/释放, 用户触发的操作结果
- 示例: 文件打开, 格式识别, 播放开始/结束, seek 操作, 按键事件
- **频率**: 低到中, 每个用户操作/重要阶段一条
- **原则**: 用户运行程序后, info 日志应能完整描述 "发生了什么"

```rust
info!("正在打开: {}", path);
info!("视频尺寸 {}x{}", width, height);
info!("[Seek] offset={:+.1}s, 目标={:.3}s", offset, target);
info!("播放结束: 发送 {} 帧, 耗时 {:.1}s", frames, elapsed);
```

#### `debug!` - 内部状态/决策 (需 `-v`)

- 开发调试信息: 内部状态变化, 分支决策, 配置解析结果
- 示例: 解码器参数, 同步决策, 缓冲区状态, 流信息
- **频率**: 中, 可能每秒数条, 但不会每帧都出现
- **原则**: 帮助开发者理解 "为什么这样做"

```rust
debug!("选择解码器: codec_id={:?}, 参数={:?}", id, params);
debug!("音频缓冲区状态: 队列={}, 延迟={}ms", queue_len, latency);
debug!("A/V 同步决策: diff={:.3}s, delay={:.3}s", diff, delay);
```

#### `trace!` - 高频热路径数据 (需 `-vv`)

- 每帧/每包/每次回调级别的详细数据
- 示例: 每个数据包的 PTS/DTS, 每帧解码耗时, 每次音频回调
- **频率**: 极高, 可能每秒数百条
- **原则**: 仅在排查细粒度时序/数据问题时需要
- **注意**: trace 日志本身会影响性能, 热路径中避免复杂的格式化

```rust
trace!("解码视频帧: PTS={}, size={}x{}", pts, w, h);
trace!("音频回调: 填充 {} 样本, PTS={:.3}s", samples, pts);
trace!("demux 包: stream={}, pts={}, size={}", idx, pts, size);
```

### 禁止事项

- **禁止** 在热路径 (每帧/每包) 中使用 `info!` 或更高级别
- **禁止** 在 `error!`/`warn!` 中记录正常流程 (如 EOF 不是错误)
- **禁止** 日志消息中包含大量二进制数据或超长字符串
- **禁止** 在性能关键路径中使用复杂的 `format!` 表达式作为日志参数

## 日志输出规则

### 控制台输出

- 与文件统一级别 (默认 info)
- 输出到 stdout
- 带颜色输出 (ANSI)

### 文件输出

- 与控制台统一级别
- 无颜色输出 (纯文本)
- 支持按日期自动切换日志文件

## 日志文件管理

### 日志目录

- 所有日志文件存放在项目根目录 `logs/` 目录下
- `logs/` 目录在 Git 中只保留 `.gitkeep` 文件
- 所有 `*.log` 文件都被 `.gitignore` 忽略, 不提交到 Git

### 日志文件命名

- 格式: `{file_prefix}.{YYYY-MM-DD}.log`
- 示例: `tao.2026-02-16.log`, `tao-probe.2026-02-16.log`

### 文件前缀规范

- **tao-cli**: 使用 `file_prefix = "tao-cli"`
- **tao-probe**: 使用 `file_prefix = "tao-probe"`
- **tao-play**: 使用 `file_prefix = "tao-play"`

### 日志维护

- 自动按日期切换日志文件 (每日凌晨)
- 可配置历史日志保留天数 (默认 30 天)
- 可配置是否压缩历史日志 (默认开启, 生成 `.gz` 文件)
- 定期清理过期日志 (可配置清理间隔)

## 日志初始化示例

### bin 项目 (tao-cli / tao-play / tao-probe)

```rust
mod logging;

fn main() {
    let args = Args::parse();
    // file_prefix 按项目写死, verbosity 由 -v 参数控制
    logging::init("tao-play", args.verbose);
    log::info!("程序启动");
}
```

### 根 crate (含 tokio 维护任务)

```rust
use tao::logging::{init, LoggingConfig};

fn main() -> anyhow::Result<()> {
    init(LoggingConfig {
        level: "info".to_string(),
        directory: "logs".to_string(),
        file_prefix: "tao".to_string(),
        retention_days: 30,
        compress_history: true,
        cleanup_interval_seconds: 3600,
    })?;
    Ok(())
}
```

## AI 调试规范

当需要调试代码时:

1. **优先读取 `logs/` 目录下日志文件, 而非 console 输出**
2. 日志文件位于 `logs/{file_prefix}.{YYYY-MM-DD}.log`
   - tao-play: `logs/tao-play.2026-02-17.log`
   - tao-cli: `logs/tao-cli.2026-02-17.log`
   - tao-probe: `logs/tao-probe.2026-02-17.log`
3. **测试前清理历史日志**, 避免污染:
   ```bash
   rm -f logs/tao-play.*.log   # 清理 tao-play 历史日志
   rm -f logs/*.log             # 清理所有历史日志
   ```
4. 运行程序后直接读取日志文件分析:
   ```bash
   # 读取最新日志
   cat logs/tao-play.$(date +%Y-%m-%d).log
   # 搜索特定关键字
   rg "Seek" logs/tao-play.*.log
   ```
5. 日志文件比 console 输出更高效:
   - 不受终端缓冲区限制
   - 可以用 Read/Grep 工具精准搜索
   - 默认 info 级别, 不含 debug 噪音
   - 需要更多细节时使用 `-v` / `-vv` / `-vvv` 运行
6. 如需调整日志级别:
   ```bash
   # 通过命令行参数
   cargo run --package tao-play -- file.avi          # info (默认)
   cargo run --package tao-play -- -v file.avi       # debug
   cargo run --package tao-play -- -vv file.avi      # trace (tao crate)
   cargo run --package tao-play -- -vvv file.avi     # trace (全局)
   # 通过环境变量 (精细控制)
   TAO_LOG=debug cargo run --package tao-play -- file.avi
   TAO_LOG="tao_codec=trace,info" cargo run --package tao-play -- file.avi
   ```
