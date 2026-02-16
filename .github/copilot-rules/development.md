# 开发规则 - 编解码器、容器格式和 FFI

> 本文件定义新增编解码器、容器格式和 FFI 导出的开发流程和规范。

---

## 1. 新增编解码器

### 1.1 开发流程

**步骤:**

1. **创建独立子模块**
    - 在 `tao-codec/src/decoders/` 或 `encoders/` 下创建子目录
    - 示例: `decoders/h264/`, `encoders/aac/`

2. **实现 Decoder 或 Encoder trait**
    - 定义结构体,包含解码/编码状态
    - 实现必要的 trait 方法

3. **提供工厂函数**
    - 创建解码器/编码器实例的工厂函数
    - 用于注册到 Registry

4. **注册到 CodecRegistry**
    - 在 `lib.rs` 或专门的注册模块中注册
    - 确保在程序启动时调用注册函数

5. **编写单元测试**
    - 验证基本编解码流程
    - 测试边界情况和错误处理
    - 使用 `samples/SAMPLE_URLS.md` 中的样本 URL

### 1.2 Decoder trait 示例

```rust
// tao-codec/src/decoders/h264/mod.rs

use tao_core::{TaoResult, TaoError};
use crate::{Decoder, Packet, Frame, CodecParameters};

/// H.264 解码器
pub struct H264Decoder {
    // 解码器状态
    width: u32,
    height: u32,
    // ... 其他字段
}

impl H264Decoder {
    /// 创建新的 H.264 解码器
    pub fn new(params: &CodecParameters) -> TaoResult<Self> {
        Ok(Self {
            width: params.width().ok_or(TaoError::InvalidData("缺少宽度参数".into()))?,
            height: params.height().ok_or(TaoError::InvalidData("缺少高度参数".into()))?,
        })
    }
}

impl Decoder for H264Decoder {
    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        // 发送数据包到解码器
        // 实现解码逻辑...
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Option<Frame>> {
        // 从解码器接收帧
        // 实现帧提取逻辑...
        Ok(None)  // 或返回解码的帧
    }

    fn flush(&mut self) -> TaoResult<Vec<Frame>> {
        // 刷新解码器,获取所有剩余帧
        Ok(Vec::new())
    }
}

// 工厂函数
pub fn create_h264_decoder(params: &CodecParameters) -> TaoResult<Box<dyn Decoder>> {
    Ok(Box::new(H264Decoder::new(params)?))
}
```

### 1.3 注册到 CodecRegistry

```rust
// tao-codec/src/lib.rs 或 registry.rs

use crate::decoders::h264;

pub fn register_all_codecs() {
    // 注册 H.264 解码器
    CodecRegistry::register_decoder(
        CodecId::H264,
        h264::create_h264_decoder,
    );

    // 注册其他编解码器...
}
```

### 1.4 编解码器测试要求

- ✅ 基本解码(正常流程)
- ✅ 编码(如果实现了编码器)
- ✅ 空输入处理
- ✅ 损坏数据处理
- ✅ Flush 流程
- ✅ 参数解析(SPS/PPS/VPS 等)

---

## 2. 新增容器格式

### 2.1 开发流程

**步骤:**

1. **创建独立子模块**
    - 在 `tao-format/src/demuxers/` 或 `muxers/` 下创建子目录
    - 示例: `demuxers/mp4/`, `muxers/mkv/`

2. **实现 Demuxer 或 Muxer trait**
    - 定义结构体,包含格式解析状态
    - 实现必要的 trait 方法

3. **实现 FormatProbe trait**
    - 支持自动格式识别
    - 根据文件头部魔数等信息判断格式

4. **提供工厂函数**
    - 创建解封装器/封装器实例的工厂函数

5. **注册到 FormatRegistry**
    - 在 `lib.rs` 或专门的注册模块中注册

6. **编写集成测试**
    - 验证格式探测、头部解析、数据包读取等

### 2.2 Demuxer trait 示例

```rust
// tao-format/src/demuxers/mp4/mod.rs

use tao_core::{TaoResult, TaoError};
use crate::{Demuxer, Stream, Packet, IoContext};

/// MP4 解封装器
pub struct Mp4Demuxer {
    io: IoContext,
    streams: Vec<Stream>,
    // ... 其他字段
}

impl Mp4Demuxer {
    /// 打开 MP4 文件
    pub fn open(mut io: IoContext) -> TaoResult<Self> {
        // 读取文件头
        let ftyp = read_ftyp(&mut io)?;

        // 解析 moov box
        let moov = read_moov(&mut io)?;

        // 提取流信息
        let streams = extract_streams(&moov)?;

        Ok(Self { io, streams })
    }
}

impl Demuxer for Mp4Demuxer {
    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self) -> TaoResult<Option<Packet>> {
        // 读取下一个数据包
        // 实现读取逻辑...
        Ok(None)  // 或返回读取的数据包
    }

    fn seek(&mut self, stream_index: usize, timestamp: i64) -> TaoResult<()> {
        // 实现 seek 功能
        Err(TaoError::NotImplemented("MP4 seek 功能待实现".into()))
    }
}

// 工厂函数
pub fn create_mp4_demuxer(io: IoContext) -> TaoResult<Box<dyn Demuxer>> {
    Ok(Box::new(Mp4Demuxer::open(io)?))
}
```

### 2.3 FormatProbe 实现

```rust
use crate::{FormatProbe, IoContext};

pub struct Mp4Probe;

impl FormatProbe for Mp4Probe {
    fn probe(&self, io: &mut IoContext) -> TaoResult<u32> {
        // 读取文件头部
        let mut buf = [0u8; 12];
        io.peek(&mut buf)?;

        // 检查 ftyp box 魔数
        if &buf[4..8] == b"ftyp" {
            // 检查 brand
            let brand = &buf[8..12];
            match brand {
                b"isom" | b"mp41" | b"mp42" | b"avc1" | b"iso2" => {
                    Ok(100)  // 高置信度
                }
                _ => Ok(50)  // 可能是 MP4
            }
        } else {
            Ok(0)  // 不是 MP4
        }
    }
}
```

### 2.4 注册到 FormatRegistry

```rust
// tao-format/src/lib.rs 或 registry.rs

use crate::demuxers::mp4;

pub fn register_all_formats() {
    // 注册 MP4 解封装器
    FormatRegistry::register_demuxer(
        FormatId::Mp4,
        Box::new(mp4::Mp4Probe),
        mp4::create_mp4_demuxer,
    );

    // 注册其他格式...
}
```

### 2.5 容器格式测试要求

- ✅ 格式探测(Probe)
- ✅ 头部解析
- ✅ 数据包读取
- ✅ Seek 操作
- ✅ 多流处理(音视频同时存在)
- ✅ 损坏文件处理

---

## 3. FFI 导出规则

### 3.1 基本要求

- ✅ **向后兼容**: FFI 函数签名变更须向后兼容,**不得删除**已发布的导出函数
- ✅ **null 检查**: 所有指针参数必须检查 null
- ✅ **安全注释**: 所有 `unsafe` 块必须有 `// SAFETY:` 注释
- ✅ **禁止 panic**: 使用 `catch_unwind` 包装或确保无 panic 路径
- ✅ **内存管理**: 由 Tao 分配的内存必须提供对应的 `*_free()` 函数
- ✅ **错误处理**: 返回错误码或 null 指针,不跨越 FFI 边界传播 panic

### 3.2 FFI 函数命名规范

- 所有导出函数以 `tao_` 前缀命名
- 使用 `#[no_mangle]` 保持符号名称
- 使用 `extern "C"` 指定 C 调用约定

**示例:**

```rust
#[no_mangle]
pub extern "C" fn tao_decoder_create(...) -> *mut TaoDecoder { ... }

#[no_mangle]
pub unsafe extern "C" fn tao_decoder_decode(...) -> i32 { ... }

#[no_mangle]
pub unsafe extern "C" fn tao_decoder_free(decoder: *mut TaoDecoder) { ... }
```

### 3.3 FFI 错误处理

**错误码约定:**

```rust
/// FFI 错误码
#[repr(C)]
pub enum TaoErrorCode {
    /// 成功
    Success = 0,
    /// 无效参数
    InvalidArgument = -1,
    /// I/O 错误
    IoError = -2,
    /// 编解码器错误
    CodecError = -3,
    /// 内存分配失败
    OutOfMemory = -4,
    /// 未实现的功能
    NotImplemented = -100,
}
```

**使用示例:**

```rust
#[no_mangle]
pub unsafe extern "C" fn tao_decoder_send_packet(
    decoder: *mut TaoDecoder,
    packet: *const TaoPacket,
) -> i32 {
    // null 检查
    if decoder.is_null() || packet.is_null() {
        return TaoErrorCode::InvalidArgument as i32;
    }

    // SAFETY: 调用者保证 decoder 和 packet 在此调用期间有效
    let decoder = &mut *decoder;
    let packet = &*packet;

    // 捕获 panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decoder.inner.send_packet(&packet.inner)
    }));

    match result {
        Ok(Ok(())) => TaoErrorCode::Success as i32,
        Ok(Err(e)) => {
            // 将 TaoError 转换为错误码
            match e {
                TaoError::InvalidData(_) => TaoErrorCode::InvalidArgument as i32,
                TaoError::Io(_) => TaoErrorCode::IoError as i32,
                _ => TaoErrorCode::CodecError as i32,
            }
        }
        Err(_) => TaoErrorCode::CodecError as i32,  // panic 被捕获
    }
}
```

### 3.4 内存管理

**原则:**

- ✅ 谁分配,谁释放
- ✅ Tao 分配的内存必须由 Tao 释放
- ✅ 提供对应的 `*_free()` 函数

**示例:**

```rust
/// 创建解码器
#[no_mangle]
pub extern "C" fn tao_decoder_create(...) -> *mut TaoDecoder {
    // 分配内存
    let decoder = Box::new(TaoDecoder { ... });
    Box::into_raw(decoder)  // 转为裸指针,所有权转移给调用者
}

/// 释放解码器
///
/// # Safety
///
/// - `decoder` 必须是由 `tao_decoder_create()` 创建的有效指针
/// - 调用后 `decoder` 指针不再有效,不得再次使用
#[no_mangle]
pub unsafe extern "C" fn tao_decoder_free(decoder: *mut TaoDecoder) {
    if !decoder.is_null() {
        // SAFETY: decoder 由 tao_decoder_create 创建,调用者保证指针有效且未被释放
        drop(Box::from_raw(decoder));  // 重新获取所有权并释放
    }
}
```

### 3.5 C 头文件生成

- 使用 `cbindgen` 自动生成 C 头文件
- 配置文件: `tao-ffi/cbindgen.toml`
- 生成命令: `cbindgen --config cbindgen.toml --crate tao-ffi --output tao.h`

---

## 4. 注册表模式详解

### 4.1 CodecRegistry 实现

```rust
use std::collections::HashMap;
use std::sync::RwLock;

type DecoderFactory = Box<dyn Fn(&CodecParameters) -> TaoResult<Box<dyn Decoder>> + Send + Sync>;

static DECODER_REGISTRY: Lazy<RwLock<HashMap<CodecId, DecoderFactory>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub struct CodecRegistry;

impl CodecRegistry {
    /// 注册解码器
    pub fn register_decoder<F>(codec_id: CodecId, factory: F)
    where
        F: Fn(&CodecParameters) -> TaoResult<Box<dyn Decoder>> + Send + Sync + 'static,
    {
        let mut registry = DECODER_REGISTRY.write().unwrap();
        registry.insert(codec_id, Box::new(factory));
    }

    /// 创建解码器
    pub fn create_decoder(codec_id: CodecId, params: &CodecParameters) -> TaoResult<Box<dyn Decoder>> {
        let registry = DECODER_REGISTRY.read().unwrap();
        let factory = registry.get(&codec_id)
            .ok_or_else(|| TaoError::NotSupported(format!("不支持的编解码器: {:?}", codec_id)))?;

        factory(params)
    }
}
```

### 4.2 优先级支持(可选扩展)

如果需要支持同一 CodecId 的多个实现(如软件解码和硬件加速):

```rust
struct CodecEntry {
    priority: u32,
    factory: DecoderFactory,
}

// 注册时指定优先级
pub fn register_decoder_with_priority(
    codec_id: CodecId,
    priority: u32,
    factory: DecoderFactory,
) {
    // 维护优先级列表
    // 创建时选择最高优先级的实现
}
```

---

## 总结

新增编解码器和容器格式遵循**创建模块 → 实现 trait → 注册 → 测试**的流程。FFI 导出遵循严格的安全规范,确保**无 panic、正确内存管理和向后兼容**。注册表模式提供了灵活的插件化架构,支持运行时动态查找和创建编解码器/格式实例。
