# 性能优化和日志规范

> 本文件定义性能优化原则和日志系统规范。

---

## 1. 性能优化

### 1.1 内存管理

**原则:**

- ✅ 避免不必要的内存分配,优先使用引用和借用
- ✅ 帧缓冲区应支持复用,避免每帧都重新分配内存
- ✅ 大块数据使用 `bytes::Bytes` 实现零拷贝传递

**示例:**

```rust
// ✅ 正确: 复用缓冲区
pub struct VideoDecoder {
    frame_buffer: Vec<u8>,  // 复用的帧缓冲区
}

impl VideoDecoder {
    pub fn decode_frame(&mut self, packet: &Packet) -> TaoResult<&[u8]> {
        // 复用现有缓冲区,避免每次分配
        self.frame_buffer.clear();
        self.frame_buffer.resize(self.frame_size, 0);

        // 解码到缓冲区
        self.decode_into(&mut self.frame_buffer, packet)?;

        Ok(&self.frame_buffer)
    }
}

// ❌ 错误: 每次都分配新缓冲区
pub fn decode_frame(&mut self, packet: &Packet) -> TaoResult<Vec<u8>> {
    let mut buffer = vec![0u8; self.frame_size];  // 每次都分配!
    self.decode_into(&mut buffer, packet)?;
    Ok(buffer)
}
```

**零拷贝传递:**

```rust
use bytes::Bytes;

// ✅ 使用 Bytes 实现零拷贝
pub struct Packet {
    data: Bytes,  // 引用计数,共享数据
    pts: i64,
}

impl Packet {
    pub fn new(data: Bytes, pts: i64) -> Self {
        Self { data, pts }
    }

    // 零拷贝获取数据切片
    pub fn slice(&self, range: std::ops::Range<usize>) -> Bytes {
        self.data.slice(range)  // 不复制数据,只增加引用计数
    }
}
```

### 1.2 数据处理

**原则:**

- ✅ 大量数据处理使用**迭代器**而非收集到 `Vec` 后再遍历
- ✅ 像素格式转换和编解码热路径应尽量避免分支预测失败
- ✅ 考虑使用 SIMD 指令优化关键路径(通过 `std::arch` 或 `packed_simd`)

**使用迭代器:**

```rust
// ✅ 正确: 使用迭代器,惰性求值
pub fn find_keyframes(packets: &[Packet]) -> impl Iterator<Item = &Packet> + '_ {
    packets.iter()
        .filter(|p| p.is_keyframe())
        .take(10)  // 只处理前 10 个关键帧
}

// ❌ 错误: 先收集再处理,分配不必要的 Vec
pub fn find_keyframes(packets: &[Packet]) -> Vec<&Packet> {
    let keyframes: Vec<_> = packets.iter()
        .filter(|p| p.is_keyframe())
        .collect();  // 分配 Vec
    keyframes.into_iter().take(10).collect()  // 又分配一次
}
```

**SIMD 优化(高级):**

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

// 使用 SIMD 加速像素格式转换
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn convert_yuv_to_rgb_simd(yuv: &[u8], rgb: &mut [u8]) {
    // 使用 AVX2 指令加速转换
    // 注意: 需要详细的 SAFETY 注释
    // ...
}
```

### 1.3 性能测试

- ✅ 使用 `benches/` 目录进行基准测试
- ✅ 使用 `cargo bench` 运行基准测试
- ✅ 关键路径优化前后进行对比测试

**基准测试示例:**

```rust
// benches/codec_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tao_codec::{H264Decoder, Packet};

fn bench_h264_decode(c: &mut Criterion) {
    let packet = Packet::new(/* 测试数据 */);
    let mut decoder = H264Decoder::new(&params).unwrap();

    c.bench_function("h264_decode", |b| {
        b.iter(|| {
            decoder.decode(black_box(&packet))
        });
    });
}

criterion_group!(benches, bench_h264_decode);
criterion_main!(benches);
```

---

## 2. 日志规范

### 2.1 基本原则

- ✅ 日志使用 `tracing` crate (`error!`, `warn!`, `info!`, `debug!`, `trace!`)
- ✅ 日志后端使用 `tracing-subscriber` 和 `tracing-appender`
- ✅ **库 crate** (tao-core, tao-codec 等) **只使用 `tracing` 宏**,不初始化日志后端
- ✅ **可执行文件** (tao-cli, tao-probe, tao-play) 负责初始化日志系统
- ✅ **日志内容使用中文**

### 2.2 日志模块位置

日志初始化模块位于 `src/logging/`:

- `mod.rs` - 日志初始化和配置
- `task.rs` - 日志维护任务(日志切换、清理、压缩)

### 2.3 日志输出规则

**控制台输出:**

- ✅ 始终固定为 **debug 级别**
- ✅ 输出到 stdout,带颜色输出(ANSI)
- ✅ 过滤规则: `debug`

**文件输出:**

- ✅ 可通过命令行参数、环境变量等改变日志过滤级别
- ✅ 文件级别通过 `LoggingConfig.level` 配置
- ✅ 无颜色输出(纯文本)
- ✅ 支持按日期自动切换日志文件
- ✅ 支持历史日志压缩和自动清理

### 2.4 日志文件管理

**日志目录:**

- ✅ 所有日志文件存放在项目根目录 `logs/` 目录下
- ✅ `logs/` 目录在 Git 中只保留 `.gitkeep` 文件
- ✅ 所有 `*.log` 文件都被 `.gitignore` 忽略,不提交到 Git

**日志文件命名:**

- 格式: `{file_prefix}.{YYYY-MM-DD}.log`
- 示例: `tao.2026-02-16.log`, `tao-probe.2026-02-16.log`

**文件前缀规范:**

- `tao-cli`: 使用 `file_prefix = "tao"`
- `tao-probe`: 使用 `file_prefix = "tao-probe"`
- `tao-play`: 使用 `file_prefix = "tao-play"`

**日志维护:**

- ✅ 自动按日期切换日志文件(每日凌晨)
- ✅ 可配置历史日志保留天数(默认 30 天)
- ✅ 可配置是否压缩历史日志(默认开启,生成 `.gz` 文件)
- ✅ 定期清理过期日志(可配置清理间隔)

### 2.5 日志级别使用

**使用指南:**

- `error!` - **致命错误**,无法继续处理
- `warn!` - **可恢复错误**,损坏但可跳过的数据
- `info!` - **关键操作**,打开文件、识别格式、开始/完成转码
- `debug!` - **详细信息**,流信息、编解码器参数、数据包细节
- `trace!` - **追踪信息**,详细的调试信息、性能追踪

**示例:**

```rust
use tracing::{error, warn, info, debug, trace};

pub fn open_file(path: &str) -> TaoResult<Demuxer> {
    info!("打开文件: {}", path);  // 关键操作

    let mut io = IoContext::open(path)
        .map_err(|e| {
            error!("无法打开文件: {}, 错误: {}", path, e);  // 致命错误
            e
        })?;

    // 探测格式
    let format_id = probe_format(&mut io)?;
    debug!("检测到格式: {:?}", format_id);  // 详细信息

    // 创建解封装器
    let demuxer = FormatRegistry::create_demuxer(format_id, io)?;

    // 记录流信息
    for stream in demuxer.streams() {
        debug!(
            "流 #{}: {:?}, 编解码器: {:?}, {}x{}",
            stream.index(),
            stream.media_type(),
            stream.codec_id(),
            stream.width().unwrap_or(0),
            stream.height().unwrap_or(0),
        );
    }

    info!("成功打开文件,包含 {} 个流", demuxer.streams().len());

    Ok(demuxer)
}

pub fn decode_packet(&mut self, packet: &Packet) -> TaoResult<Vec<Frame>> {
    trace!("解码数据包: size={}, pts={}", packet.size(), packet.pts());

    // 解码逻辑...
    if let Err(e) = self.parse_bitstream(packet.data()) {
        warn!("比特流解析失败: {}, 跳过此数据包", e);  // 可恢复错误
        return Ok(Vec::new());
    }

    let frames = self.do_decode()?;

    trace!("解码完成,输出 {} 帧", frames.len());

    Ok(frames)
}
```

### 2.6 日志初始化(可执行文件)

```rust
// bins/tao-cli/src/main.rs

use tao::logging::{init_logging, LoggingConfig};

fn main() {
    // 初始化日志系统
    let config = LoggingConfig {
        file_prefix: "tao".to_string(),
        level: tracing::Level::DEBUG,
        max_log_days: 30,
        compress_old_logs: true,
    };

    init_logging(config).expect("日志系统初始化失败");

    // 记录启动信息
    info!("Tao 多媒体处理工具 v{}", env!("CARGO_PKG_VERSION"));

    // 主程序逻辑...
}
```

### 2.7 AI 调试规范

当需要调试代码时:

1. **优先查看日志文件而非控制台输出**
2. 日志文件位于 `logs/{file_prefix}.{date}.log`
3. 调试前可以删除对应的日志文件,避免历史日志污染
4. 示例: 删除 `logs/tao.2026-02-16.log` 重新运行程序生成新日志
5. 通过日志文件分析程序执行流程和错误原因
6. 减少频繁读取控制台输出,提高调试效率

**调试示例:**

```powershell
# 1. 删除今天的日志文件
Remove-Item logs\tao.2026-02-16.log -ErrorAction SilentlyContinue

# 2. 运行程序
cargo run --package tao-cli -- input.mp4 output.mkv

# 3. 查看日志文件
Get-Content logs\tao.2026-02-16.log | Select-String "错误|警告"
```

---

## 3. 性能优化清单

在实现编解码器或格式处理时,检查以下优化点:

- [ ] 是否复用了缓冲区,避免频繁分配?
- [ ] 是否使用迭代器而非中间 Vec?
- [ ] 是否避免了不必要的数据复制?
- [ ] 热路径是否优化了分支预测?
- [ ] 是否考虑了 SIMD 加速?(如适用)
- [ ] 是否添加了基准测试?(关键路径)
- [ ] 日志级别是否合理?(避免过度日志影响性能)

---

## 4. 日志最佳实践

### 4.1 日志内容

**应该记录:**

- ✅ 文件打开/关闭
- ✅ 格式探测结果
- ✅ 流信息
- ✅ 编解码器参数
- ✅ 错误和警告
- ✅ 性能关键操作(可使用 `trace!`)

**不应该记录:**

- ❌ 每个数据包的详细信息(除非使用 `trace!` 级别)
- ❌ 大量重复信息
- ❌ 敏感信息(密码、密钥等)
- ❌ 过长的二进制数据

### 4.2 日志格式

**使用结构化日志(可选):**

```rust
use tracing::{info, info_span};

// 创建 span 提供上下文
let _span = info_span!("decode_video", stream_id = 0).entered();

info!("开始解码视频流");
// 后续日志会自动包含 stream_id 上下文
```

**参数化日志:**

```rust
// ✅ 正确: 使用参数化
info!("解码帧: {}x{}, 格式: {:?}", width, height, pixel_format);

// ❌ 错误: 使用字符串拼接
info!(format!("解码帧: {}x{}, 格式: {:?}", width, height, pixel_format));
```

---

## 总结

性能优化关注**内存复用、迭代器使用和 SIMD 加速**。日志系统使用 `tracing` crate,库 crate 只使用宏,可执行文件负责初始化。日志内容使用中文,合理使用日志级别。AI 调试时优先查看日志文件而非控制台输出,提高调试效率。
