# Rust 编码规范、错误处理和安全

> 本文件定义 Rust 语言特定的编码规范、错误处理策略和安全要求。

---

## 1. Rust 编码规范

### 1.1 类型与安全

**必须遵守:**

- ✅ **为所有公开函数参数和返回值定义明确的类型**
- ❌ **禁止**: 随意使用 `unwrap()` / `expect()`
    - 仅在能确保不会 panic 的场景使用(如常量初始化)
    - 其他场景必须使用 `?` 或 `match` 处理错误
- ✅ **必须**: 使用 `TaoError` / `TaoResult` 作为统一错误类型
- ✅ crate 内部特定错误使用 `thiserror` 定义
- ✅ **推荐**: 使用 `struct` 定义数据结构,使用 `enum` 定义状态与变体,使用 `type` 定义别名
- ✅ **推荐**: trait 对象使用 `Box<dyn Trait>`,泛型用于内部实现,trait 对象用于跨 crate 接口

**示例:**

```rust
// ✅ 正确: 使用 ? 处理错误
pub fn decode_frame(&mut self, packet: &Packet) -> TaoResult<Vec<Frame>> {
    let data = packet.data().ok_or(TaoError::InvalidData("数据包为空".into()))?;
    // 解码逻辑...
    Ok(frames)
}

// ❌ 错误: 使用 unwrap
pub fn decode_frame(&mut self, packet: &Packet) -> Vec<Frame> {
    let data = packet.data().unwrap(); // 可能 panic!
    // ...
}

// ✅ 正确: 常量初始化可以使用 expect
const DEFAULT_RATIONAL: Rational = Rational::new(1, 1000).expect("有效的有理数");
```

### 1.2 并发与 FFI

**并发安全:**

- ✅ 所有 trait(`Decoder`, `Encoder`, `Demuxer`, `Muxer`, `Filter`)要求 `Send`,以支持多线程使用
- ✅ 跨线程共享的状态使用 `Arc<Mutex<T>>` 或 `Arc<RwLock<T>>`
- ✅ 优先使用 `std::sync` 而非 `parking_lot`(保持依赖简洁)

**FFI 安全:**

- ❌ FFI 导出函数中**禁止 panic**
- ✅ 必须使用 `catch_unwind` 包装或确保无 panic 路径
- ✅ FFI 函数的 `unsafe` 块必须添加 `// SAFETY:` 注释说明安全前提
- ✅ 所有指针参数必须检查 null

**FFI 示例:**

```rust
/// 创建解码器
///
/// # Safety
///
/// - `codec_params` 必须是有效的非 null 指针
/// - 返回的指针必须通过 `tao_decoder_free()` 释放
#[no_mangle]
pub unsafe extern "C" fn tao_decoder_create(
    codec_id: u32,
    codec_params: *const TaoCodecParameters,
) -> *mut TaoDecoder {
    // SAFETY: 调用者保证 codec_params 是有效指针
    if codec_params.is_null() {
        return std::ptr::null_mut();
    }

    let params = &*codec_params;

    // 捕获 panic,确保不会跨越 FFI 边界
    let result = std::panic::catch_unwind(|| {
        DecoderRegistry::create_decoder(codec_id.into(), params)
    });

    match result {
        Ok(Ok(decoder)) => Box::into_raw(Box::new(TaoDecoder { inner: decoder })),
        _ => std::ptr::null_mut(),
    }
}
```

### 1.3 格式化

- ✅ 代码格式化使用 `rustfmt`,配置见 `.rustfmt.toml`
- ✅ 行宽上限 **100 字符**
- ✅ 缩进 **4 空格**,不使用 tab
- ✅ 提交前必须运行 `cargo fmt --check`

### 1.4 枚举设计

- ✅ 编解码器 ID、像素格式、采样格式等枚举使用 `#[non_exhaustive]`,以便后续扩展
- ✅ 枚举变体命名使用 PascalCase,与 Rust 惯例一致

**示例:**

```rust
/// 编解码器标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]  // 允许未来添加新的编解码器
pub enum CodecId {
    // 视频编解码器
    H264,
    H265,
    Vp8,
    Vp9,
    Av1,

    // 音频编解码器
    Aac,
    Mp3,
    Flac,
    Opus,

    // 未来可能添加更多...
}
```

---

## 2. 错误处理规范

### 2.1 基本原则

- ✅ 所有 I/O 操作(文件、网络)必须处理错误,**禁止吞错**
- ✅ 使用 `TaoError` 枚举覆盖所有错误场景
- ✅ 编解码器和格式处理中遇到的损坏数据应返回 `TaoError::InvalidData`
- ✅ 未实现的功能返回 `TaoError::NotImplemented`,**不使用 `todo!()` 宏**(会 panic)
- ✅ 错误消息必须使用中文

### 2.2 TaoError 枚举

定义在 `tao-core/src/error.rs`:

```rust
use thiserror::Error;

/// Tao 统一错误类型
#[derive(Debug, Error)]
pub enum TaoError {
    /// I/O 错误
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    /// 编解码器错误
    #[error("编解码器错误: {0}")]
    Codec(String),

    /// 格式错误
    #[error("格式错误: {0}")]
    Format(String),

    /// 无效数据
    #[error("无效数据: {0}")]
    InvalidData(String),

    /// 文件结束
    #[error("文件结束")]
    Eof,

    /// 需要更多数据
    #[error("需要更多数据")]
    NeedMoreData,

    /// 不支持的特性
    #[error("不支持的特性: {0}")]
    NotSupported(String),

    /// 未实现的功能
    #[error("未实现的功能: {0}")]
    NotImplemented(String),

    /// 其他错误
    #[error("{0}")]
    Other(String),
}

/// Tao 统一结果类型
pub type TaoResult<T> = Result<T, TaoError>;
```

### 2.3 错误处理最佳实践

**场景 1: I/O 操作**

```rust
// ✅ 正确: 使用 ? 传播错误
pub fn read_header(&mut self) -> TaoResult<()> {
    let magic = self.io.read_u32_be()?;
    if magic != 0x1A45DFA3 {
        return Err(TaoError::Format("无效的 Matroska 魔数".into()));
    }
    Ok(())
}
```

**场景 2: 损坏数据**

```rust
// ✅ 正确: 返回 InvalidData
pub fn parse_sps(&self, data: &[u8]) -> TaoResult<Sps> {
    if data.len() < 4 {
        return Err(TaoError::InvalidData("SPS 数据过短".into()));
    }
    // 解析逻辑...
}
```

**场景 3: 未实现功能**

```rust
// ✅ 正确: 返回 NotImplemented
pub fn seek(&mut self, timestamp: i64) -> TaoResult<()> {
    Err(TaoError::NotImplemented("FLV 格式暂不支持 seek".into()))
}

// ❌ 错误: 使用 todo! 会 panic
pub fn seek(&mut self, timestamp: i64) -> TaoResult<()> {
    todo!("实现 seek")  // 运行时会 panic!
}
```

**场景 4: 需要更多数据(流式解码)**

```rust
pub fn decode(&mut self, packet: &Packet) -> TaoResult<Vec<Frame>> {
    self.buffer.extend_from_slice(packet.data());

    if self.buffer.len() < self.required_size {
        return Err(TaoError::NeedMoreData);  // 需要更多数据
    }

    // 解码逻辑...
    Ok(frames)
}
```

---

## 3. 安全规范

### 3.1 敏感信息管理

- ❌ **禁止**: 在代码中硬编码任何敏感信息
- ❌ 配置文件不得提交到版本库
- ✅ `.gitignore` 中必须包含:
    - `target/` - 构建产物
    - `*.dll`, `*.so`, `*.dylib` - 动态库
    - `data/` - 临时文件(仅保留 `.gitkeep`)
    - `logs/` - 日志文件(仅保留 `.gitkeep`)

### 3.2 FFI 安全

- ✅ FFI 层所有 `unsafe` 代码必须有详细安全性注释
- ✅ 使用 `// SAFETY:` 注释说明为何该 `unsafe` 块是安全的
- ✅ 所有指针参数必须检查 null
- ✅ 由 Tao 分配的内存必须提供对应的 `tao_*_free()` 函数
- ✅ 跨 FFI 边界的所有函数必须使用 `catch_unwind` 防止 panic

**SAFETY 注释示例:**

```rust
#[no_mangle]
pub unsafe extern "C" fn tao_packet_data(packet: *const TaoPacket) -> *const u8 {
    // SAFETY: 调用者保证 packet 在此函数调用期间有效且非 null
    if packet.is_null() {
        return std::ptr::null();
    }

    let packet = &*packet;
    packet.inner.data().as_ptr()
}
```

### 3.3 内存安全

- ✅ 避免不必要的 `unsafe` 代码
- ✅ 优先使用安全的 Rust 抽象
- ✅ 必须使用 `unsafe` 时,缩小 `unsafe` 块的范围
- ✅ 使用 `#[deny(unsafe_op_in_unsafe_fn)]` 强制要求 `unsafe` 块

**示例:**

```rust
#![deny(unsafe_op_in_unsafe_fn)]

pub unsafe fn read_unaligned_u32(ptr: *const u8) -> u32 {
    // SAFETY: 调用者保证 ptr 指向至少 4 字节的有效内存
    unsafe {
        std::ptr::read_unaligned(ptr as *const u32)
    }
}
```

---

## 总结

Rust 编码规范强调**类型安全、错误处理和内存安全**。通过统一的 `TaoError` 类型、严格的 FFI 安全要求和清晰的 `unsafe` 代码注释,确保项目的健壮性和可维护性。始终记住:**禁止 panic,处理所有错误,中文错误消息**。
