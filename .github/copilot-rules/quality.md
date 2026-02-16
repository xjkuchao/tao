# 代码质量、提交规范和注释

> 本文件定义代码质量要求、提交规范和注释规范。

---

## 1. 代码质量要求

### 1.1 基本要求

**绝对禁止:**

- ❌ **不允许存在任何编译错误或 Clippy 警告**
- ❌ 未使用的 `use` 导入
- ❌ 未使用的变量、函数、类型定义
- ❌ 硬编码的魔法数字或字符串
- ❌ 重复代码

**必须做到:**

- ✅ 未使用的变量、函数、类型定义必须删除或添加 `_` 前缀标记为故意未使用
- ✅ 避免硬编码的魔法数字或字符串,使用常量或配置项替代
- ✅ 避免重复代码,提取公共逻辑到工具函数或 trait 中
- ✅ 单个函数不超过 **50 行**

### 1.2 代码审查清单

**每次修改后都应该自我审查代码,确保:**

- [ ] 没有未使用的 imports, variables, functions
- [ ] 没有重复代码
- [ ] 没有硬编码的魔法数字或字符串
- [ ] 遵循项目代码风格
- [ ] 所有错误都有适当的处理
- [ ] 复杂逻辑有清晰注释
- [ ] 单个函数不超过 50 行

### 1.3 代码组织

**模块化:**

- ✅ 确保项目模块化,避免单一函数过于复杂
- ✅ 复杂模块应拆分为多个子模块,每个子模块职责单一
- ✅ 公共类型集中定义在对应 crate 中,避免散落

**示例:**

```rust
// ✅ 正确: 使用常量替代魔法数字
const MAX_PACKET_SIZE: usize = 65536;
const H264_NAL_UNIT_TYPE_MASK: u8 = 0x1F;

fn parse_nal_unit(data: &[u8]) -> TaoResult<NalUnit> {
    if data.len() > MAX_PACKET_SIZE {
        return Err(TaoError::InvalidData("数据包过大".into()));
    }

    let nal_type = data[0] & H264_NAL_UNIT_TYPE_MASK;
    // ...
}

// ❌ 错误: 硬编码魔法数字
fn parse_nal_unit(data: &[u8]) -> TaoResult<NalUnit> {
    if data.len() > 65536 {  // 这是什么?
        return Err(TaoError::InvalidData("数据包过大".into()));
    }

    let nal_type = data[0] & 0x1F;  // 这个掩码代表什么?
    // ...
}
```

---

## 2. 代码提交规范

### 2.1 提交信息格式

使用规范的提交信息格式:

- `feat: 功能描述` - 新增功能
- `fix: 问题描述` - 修复 Bug
- `refactor: 重构描述` - 代码重构
- `style: 样式调整` - 代码格式调整
- `chore: 其他描述` - 构建/工具/依赖更新
- `test: 测试描述` - 新增或修改测试
- `docs: 文档描述` - 文档更新

**要求:**

- ✅ 提交信息必须使用中文
- ✅ 简洁明了地描述变更内容
- ✅ 单次提交专注于单一功能或修复

**示例:**

```bash
# ✅ 正确
git commit -m "feat: 实现 H.265 解码器"
git commit -m "fix: 修复 MP4 解封装器 seek 偏移错误"
git commit -m "test: 添加 AAC 编码器测试用例"

# ❌ 错误
git commit -m "update"  # 不清晰
git commit -m "Add H.265 decoder"  # 使用英文
git commit -m "feat: 实现 H.265 解码器, 修复 MP4 bug, 更新文档"  # 混合多个变更
```

### 2.2 提交前检查(强制)

**严格要求**: 提交前必须按以下顺序执行检查,确保全部通过:

```bash
# 1. 代码格式化检查
cargo fmt --check

# 2. Clippy 检查 (任何警告都必须修复)
cargo clippy -- -D warnings

# 3. 编译检查
cargo check

# 4. 测试通过
cargo test
```

**顺序说明:**

1. **`cargo fmt --check`** - 确认代码格式一致
    - 如果失败,运行 `cargo fmt` 修复格式
2. **`cargo clippy -- -D warnings`** - 修复所有 Clippy 警告
    - `-D warnings` 将警告视为错误,确保 0 警告
3. **`cargo check`** - 确认编译通过
    - 快速检查,不生成可执行文件
4. **`cargo test`** - 确认所有测试通过
    - 包括单元测试和集成测试

### 2.3 0 警告容忍

**绝对禁止:**

- ❌ **任何 Clippy 警告都必须在提交前修复,不允许忽略**
- ❌ 禁止使用 `#[allow(...)]` 来绕过 Clippy 检查,**除非有充分理由并添加详细注释说明**

**示例:**

```rust
// ✅ 正确: 有充分理由使用 #[allow]
#[allow(clippy::too_many_arguments)]  // FFI 函数签名由外部 API 定义,无法简化
pub unsafe extern "C" fn tao_demuxer_open(
    url: *const c_char,
    io_flags: u32,
    options: *const TaoOptions,
    demuxer: *mut *mut TaoDemuxer,
    callback: TaoCallback,
    user_data: *mut c_void,
) -> i32 {
    // FFI 函数实现...
}

// ❌ 错误: 无理由忽略警告
#[allow(clippy::manual_map)]  // 为什么忽略? 应该修复!
fn get_stream_index(streams: &[Stream], media_type: MediaType) -> Option<usize> {
    // ...
}
```

### 2.4 自动提交规则

**要求:**

- ✅ 每完成一轮功能开发,且代码检查全部通过后,**必须自动提交本次修改**
- ✅ 提交范围应仅包含当轮功能涉及的文件
- ✅ 提交信息应准确概括本轮变更内容

**流程:**

```bash
# 1. 完成功能开发
# 2. 运行代码检查
cargo fmt && cargo clippy -- -D warnings && cargo test

# 3. 检查全部通过后,提交修改
git add crates/tao-codec/src/decoders/h265/
git commit -m "feat: 实现 H.265 解码器"
```

---

## 3. 注释规范

### 3.1 基本要求

- ✅ **必须**: 所有注释使用中文
- ✅ 复杂逻辑必须添加注释说明
- ✅ 公开函数和 trait 使用 `///` 文档注释,说明功能、参数、返回值
- ✅ 每个 crate 的 `lib.rs` 使用 `//!` 模块文档注释,说明 crate 用途
- ✅ FFI 导出函数必须同时说明安全性要求(`# Safety` 段落)
- ✅ 特殊处理或 Workaround 必须注释说明原因
- ✅ 临时代码或待优化代码使用 `// TODO:` 标记

### 3.2 文档注释

**公开函数:**

````rust
/// 解码视频数据包
///
/// # 参数
///
/// - `packet` - 待解码的压缩数据包
///
/// # 返回值
///
/// - `Ok(frames)` - 成功解码的帧列表(可能为空)
/// - `Err(e)` - 解码失败,返回错误信息
///
/// # 错误
///
/// - `TaoError::InvalidData` - 数据包格式无效
/// - `TaoError::Codec` - 解码器内部错误
///
/// # 示例
///
/// ```rust
/// let mut decoder = H264Decoder::new(&params)?;
/// let frames = decoder.decode(&packet)?;
/// for frame in frames {
///     println!("解码帧: {}x{}", frame.width(), frame.height());
/// }
/// ```
pub fn decode(&mut self, packet: &Packet) -> TaoResult<Vec<Frame>> {
    // 实现...
}
````

**模块文档:**

````rust
// crates/tao-codec/src/lib.rs

//! # tao-codec
//!
//! 编解码器框架,对标 FFmpeg 的 libavcodec。
//!
//! 提供统一的编解码器接口和注册表,支持:
//!
//! - 视频编解码器: H.264, H.265, VP8, VP9, AV1 等
//! - 音频编解码器: AAC, MP3, FLAC, Opus 等
//! - 编解码器注册表: 动态查找和创建编解码器实例
//!
//! ## 使用示例
//!
//! ```rust
//! use tao_codec::{CodecRegistry, CodecId};
//!
//! // 创建 H.264 解码器
//! let decoder = CodecRegistry::create_decoder(CodecId::H264, &params)?;
//! ```
````

### 3.3 FFI 安全注释

**所有 FFI 函数必须包含 `# Safety` 段落:**

```rust
/// 释放解码器
///
/// # Safety
///
/// - `decoder` 必须是由 `tao_decoder_create()` 创建的有效指针
/// - 调用后 `decoder` 指针不再有效,不得再次使用
/// - 不得对同一个 `decoder` 指针调用多次
#[no_mangle]
pub unsafe extern "C" fn tao_decoder_free(decoder: *mut TaoDecoder) {
    if !decoder.is_null() {
        // SAFETY: decoder 由 tao_decoder_create 创建,调用者保证指针有效且未被释放
        drop(Box::from_raw(decoder));
    }
}
```

### 3.4 特殊注释

**SAFETY 注释 (unsafe 块):**

```rust
pub fn read_u32_be(&mut self) -> TaoResult<u32> {
    let mut buf = [0u8; 4];
    self.read_exact(&mut buf)?;

    // SAFETY: buf 保证是 4 字节,read_unaligned 是安全的
    Ok(unsafe { u32::from_be(std::ptr::read_unaligned(buf.as_ptr() as *const u32)) })
}
```

**TODO 注释 (待实现功能):**

```rust
pub fn seek(&mut self, timestamp: i64) -> TaoResult<()> {
    // TODO: 实现精确 seek,当前只支持关键帧 seek
    self.seek_to_keyframe(timestamp)
}
```

**WORKAROUND 注释 (临时解决方案):**

```rust
// WORKAROUND: 某些 MP4 文件的 moov box 在文件末尾,
// 需要先读取 mdat box 大小再 seek 到 moov
// 后续可通过预加载完整 moov box 优化
if self.moov_at_end {
    self.io.seek(SeekFrom::End(-8))?;
    let moov_size = self.io.read_u32_be()?;
    self.io.seek(SeekFrom::End(-(moov_size as i64)))?;
}
```

### 3.5 行内注释

**复杂逻辑需要解释:**

```rust
// 解析 NAL 单元类型(取低 5 位)
let nal_type = data[0] & 0x1F;

// H.264 规范: NAL 类型 5 表示 IDR 帧(关键帧)
if nal_type == 5 {
    self.is_keyframe = true;
}

// 跳过 start code (0x00 0x00 0x00 0x01)
let payload = &data[4..];
```

---

## 4. 代码风格

### 4.1 命名规范

- ✅ 结构体、枚举、trait: `PascalCase`
- ✅ 函数、变量、模块: `snake_case`
- ✅ 常量、静态变量: `SCREAMING_SNAKE_CASE`
- ✅ 类型参数: 单个大写字母或 `PascalCase`

**示例:**

```rust
// 结构体
pub struct H264Decoder { ... }

// 枚举
pub enum CodecId { ... }

// trait
pub trait Decoder { ... }

// 函数
pub fn create_decoder() -> TaoResult<Box<dyn Decoder>> { ... }

// 常量
const MAX_BUFFER_SIZE: usize = 1024 * 1024;

// 类型参数
fn convert<T: AsRef<[u8]>>(data: T) -> Vec<u8> { ... }
```

### 4.2 格式化

- ✅ 使用 `cargo fmt` 自动格式化
- ✅ 行宽上限 **100 字符** (配置在 `.rustfmt.toml`)
- ✅ 缩进 **4 空格**,不使用 tab
- ✅ 函数参数过多时,每行一个参数

---

## 总结

代码质量要求**0 警告容忍**,提交前必须通过 `fmt/clippy/check/test` 检查。提交信息使用中文,格式规范。所有注释使用中文,公开接口必须有文档注释,FFI 函数必须有 `# Safety` 说明。代码审查清单确保代码质量,避免常见问题。
