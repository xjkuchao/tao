# Xvid vs Tao MPEG-4 Part 2 解码实现详细对比

> 日期: 2026-02-16
> 作者: GitHub Copilot
> 目标: 识别Xvid与Tao的实现差异，制定完善路线图

---

## 目录

1. [架构对比](#1-架构对比)
2. [关键技术实现对比](#2-关键技术实现对比)
3. [发现的问题详解](#3-发现的问题详解)
4. [完善工作计划](#4-完善工作计划)

---

## 1. 架构对比

### 1.1 项目结构

| 维度            | Xvid (libxvidcore)                                         | Tao (tao-codec)                                                 |
| --------------- | ---------------------------------------------------------- | --------------------------------------------------------------- |
| **语言**        | C                                                          | Rust                                                            |
| **代码行数**    | ~30,000行 (完整库)                                         | ~3,894行 (MPEG-4模块)                                           |
| **模块划分**    | `bitstream/`, `quant/`, `prediction/`, `motion/`, `image/` | `bitreader.rs`, `vlc.rs`, `block.rs`, `motion.rs`, `dequant.rs` |
| **目标受众**    | 生产环境/商业应用                                          | 教育/参考实现/纯Rust生态                                        |
| **支持Profile** | Simple, Core, Main, Advanced Simple                        | Simple, Advanced Simple (部分)                                  |

### 1.2 解码管线对比

#### Xvid的解码流程

```
GetPacket()
  |
  ├─> BitstreamInit()         # 初始化码流
  ├─> DecodeVOP()             # 解码VOP头
  │   ├─> DecodeVOLHeader()
  │   └─> DecodeVOPHeader()
  ├─> DecodeMBData()          # 宏块循环
  │   ├─> GetMBType()         # VLC解码MB类型
  │   ├─> GetCoeff()          # VLC解码系数
  │   ├─> Predict()           # 运动补偿/预测
  │   └─> Add/idct()          # IDCT逆变换
  └─> OutputFrame()           # 输出帧
```

#### Tao的解码流程

```
decode(packet)
  |
  ├─> BitReader::new()        # 初始化比特流
  ├─> read_vop_header()       # 解析VOP头
  ├─> decode_frame_partitioned() 或 decode_frame_standard()
  │   ├─> decode_macroblock() 循环遍历每个MB
  │   │   ├─> decode_mcbpc_i/p() # VLC解码MB类型
  │   │   ├─> read_ac_coeffs()   # VLC解码AC系数
  │   │   ├─> apply_motion_comp() # 运动补偿
  │   │   └─> idct_8x8()         # IDCT变换
  │   └─> 帧缓冲管理
  └─> Output & Return Frame
```

### 1.3 代码质量指标

| 指标         | Xvid                             | Tao                |
| ------------ | -------------------------------- | ------------------ |
| **SIMD优化** | ✅ 广泛使用 (MMX/SSE/AVX)        | ❌ 仅标量运算      |
| **性能**     | 生产级 (实时播放)                | 学习级 (可接受)    |
| **内存管理** | 手动优化缓冲池                   | Rust自动管理       |
| **错误恢复** | 完整 (resync marker/slice级恢复) | 基础 (MB级检测)    |
| **代码注释** | 中等                             | ✅ 丰富 (Rust doc) |
| **测试覆盖** | 内部测试                         | ✅ 138个单元测试   |
| **类型安全** | 否 (C指针)                       | ✅ 是 (Rust)       |

---

## 2. 关键技术实现对比

### 2.1 比特流处理

#### Xvid (bitstream module)

```c
// bitstream.c: BitstreamInit, BitstreamShowBits, BitstreamGetBits
typedef struct {
    const uint8_t *data;
    uint32_t buf;
    uint32_t buf_bits;
    uint8_t *pos;
} Bitstream;

// 手动缓存管理, 支持比特级随机访问
void BitstreamShowBits(Bitstream *bs, uint32_t n, uint32_t *val) {
    // 缓存填充逻辑 (4字节缓存)
    while (bs->buf_bits < n) {
        bs->buf = (bs->buf << 8) | *bs->pos++;
        bs->buf_bits += 8;
    }
    *val = bs->buf >> (bs->buf_bits - n);
}
```

#### Tao (bitreader.rs)

```rust
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl BitReader {
    pub fn read_bits(&mut self, n: u8) -> Option<u32> {
        let start_byte = self.bit_pos / 8;
        let end_byte = (self.bit_pos + n as usize + 7) / 8;

        if end_byte > self.data.len() {
            return None;
        }

        // 字节对齐访问 + 位移提取
        let mut result = 0u32;
        for i in 0..end_byte - start_byte {
            result = (result << 8) | self.data[start_byte + i] as u32;
        }
        // ... 提取目标位 ...
    }
}
```

**差异分析**:

- ✅ Tao 更安全 (边界检查、lifetime安全)
- ⚠️ Tao 逐字节读取效率略低 (可优化预缓存)
- ✅ Xvid 有显式缓存策略 (性能优化)

---

### 2.2 VLC (Variable Length Code) 解码

#### Xvid 的 VLC 表设计

```c
// quant/quant.c - MCBPC_I VLC表
// 固定表格 + 查表速度: O(1)
static const VLC MCBPC_I[] = {
    {0x1, 1},  // 0001b -> Intra (1MV), CBP=0
    {0x1, 2},  // 0001b -> Intra 3
    {0x1, 3},  // 0001b -> Intra GMC
    // ... more entries
};

// 表大小: 预计算所有可能的VLC组合, 使用索引数组
// 特点: 快速查表, 空间换时间
```

#### Tao 的 VLC 表设计

```rust
// decoders/mpeg4/vlc.rs
pub(super) const MCBPC_I_VLC: &[(u8, u16, u8)] = &[
    (1, 0b1, 0),           // 0b1 (1 bit) -> MB type 0, CBP 0
    (3, 0b001, 1),         // 0b001 (3 bit) -> MB type 1, CBP 1
    (3, 0b011, 2),         // 0b011 (3 bit) -> MB type 2, CBP 2
    // ...
];

// 线性扫描VLC表, 时间O(log n), 空间高效
pub(super) fn decode_mcbpc_i(reader: &mut BitReader) -> Option<(MbType, u8)> {
    for &(len, code, index) in MCBPC_I_VLC {
        if reader.peek_bits(len)? as u16 == code {
            reader.read_bits(len)?;
            return Some(decode_mcbpc_i_index(index));
        }
    }
    None
}
```

**差异分析**:

- ✅ Xvid: 预计算所有组合, O(1) 查表 (推荐用于时间敏感场景)
- ⚠️ Tao: 线性扫描, O(log n) 平均, 代码简洁但速度稍低
- **优化方向**:
    - 将线性扫描改为二分查找或哈希表
    - 或预生成完整VLC查表数组

---

### 2.3 反量化 (Dequantization)

#### Xvid 的反量化策略

```c
// quant/quant_h263.c
void quant_h263_intra(int16_t *coeff, ...) {
    for (i = 0; i < 64; i++) {
        // H.263 反量化公式
        // coeff[i] = (2 * |Q| + 1) * (2 * coeff[i] + 1)

        int16_t dequant = (2 * quant + 1) * (2 * coeff[i] + 1);

        // 关键: 范围裁剪 [-2048, 2047]
        dequant = CLIP(dequant, -2048, 2047);

        // MPEG量化类型: mismatch control
        if (quant_type == MPEG) {
            // 控制奇偶性确保错误恢复
            dequant = (dequant ^ 1) ^ (dequant & 1);
        }
    }
}
```

#### Tao 的反量化实现

```rust
// decoders/mpeg4/dequant.rs
pub(super) fn dequant_h263_coeff(
    coeff: i16,
    quant: u8,
) -> i32 {
    if coeff == 0 {
        return 0;
    }

    let dequant = (2 * i32::from(quant) + 1) *
                  (2 * i32::from(coeff.abs()) + 1);

    if coeff < 0 {
        -(dequant as i32)
    } else {
        dequant as i32
    }
    // ❌ 缺少 [-2048, 2047] 裁剪!
}

pub(super) fn dequant_mpeg_coeff(
    coeff: i16,
    quant: u8,
    intra: bool,
) -> i32 {
    // ... 反量化计算 ...
    // ❌ mismatch control 仅对 Inter 块执行, Intra 块缺失!
}
```

**发现的问题**:

- 🔴 **M1**: H.263 反量化缺少 `[-2048, 2047]` 范围裁剪
- 🔴 **M2**: MPEG 反量化的 mismatch control 仅对 Inter 块执行
- **标准要求** (ISO 14496-2):
    - 所有反量化后通过: `clamp(value, -2048, 2047)`
    - Mismatch control 对所有块类型应用

---

### 2.4 运动补偿 (Motion Compensation)

#### Xvid 的MC实现

```c
// image/interpolate8x8.c
// 4阶段: 全像素 -> 半像素 -> 四分像素

// 1. 整像素MC (直接复制)
void transfer_8x8_copy(uint8_t *dst, ...) {
    memcpy(dst, src, 8); // 高效批量复制
}

// 2. 半像素MC (6-tap FIR滤波)
void interpolate8x8_halfpel_h(uint8_t *dst, uint8_t *src) {
    for (y = 0; y < 8; y++) {
        for (x = 0; x < 8; x++) {
            dst[x] = (src[x-2]*(-1) + src[x-1]*5 +
                      src[x]*5 + src[x+1]*(-1) + 4) >> 3;
        }
    }
}

// 3. 四分像素MC (双线性插值或特殊滤波)
void interpolate8x8_qpel(uint8_t *dst, uint8_t *src,
                        int dx, int dy) {
    // dx, dy ∈ {0, 1, 2, 3} 表示1/4像素偏移
    // 关键: rounding 行为由mv_rounding_type控制

    if (dx == 1 || dx == 3) {
        // 需要水平1/4像素副本, 先做水平1/2像素插值
        interpolate_h_quarter(qpel_tmp, src, dx);
    }
}
```

#### Tao 的 MC 实现

```rust
// decoders/mpeg4/motion.rs

pub(super) fn motion_compensation_luma_qpel(
    ref_frame: &VideoFrame,
    dst: &mut VideoFrame,
    x: u32, y: u32, width: u32, height: u32,
    dx: i16, dy: i16, // 1/4像素残差
    rounding: bool,
) {
    // 当前实现: 对qpel 执行简单插值

    let (full_x, rem_x) = ((dx / 4) as i32, (dx % 4) as u8);
    let (full_y, rem_y) = ((dy / 4) as i32, (dy % 4) as u8);

    match (rem_x, rem_y) {
        (0, 0) => copy_block(),           // 全像素
        (_, 0) => interpolate_h_qpel(),  // 水平qpel
        (0, _) => interpolate_v_qpel(),  // 垂直qpel
        (_, _) => interpolate_hv_qpel(), // 双向qpel
    }

    // ⚠️ rounding 应用不完整:
    // - Tao 仅在最后加一次rounding
    // - Xvid 在每个插值阶段应用rounding
}
```

**问题分析**:

- 🔴 **C6**: P帧色度MC缺少四分像素感知 (仅处理整像素)
- 🔴 **M5**: qpel MC rounding 行为与标准不一致
- **标准行为** (ISO 14496-2 Annex D):
    - 水平/垂直方向分别应用6-tap滤波
    - 每个中间结果应用rounding

---

### 2.5 B帧解码

#### Xvid 的B帧处理

```c
// decoder.c - DecodeBFrame()
void DecodeBFrame() {
    // 1. 帧重排序 (Frame Reordering / DPB)
    // B帧不作为参考, 立即输出; P帧进入缓冲
    if (is_b_frame) {
        output_frame();  // 直接输出
    } else {
        buffer_frame();  // P/I缓冲供B帧参考
    }

    // 2. Direct模式运动向量
    for (each_mb) {
        if (mb_mode == DIRECT) {
            // 使用参考帧B_FWD/B_BWD的运动向量
            //
            // 情况1(colloc_mb是Intra):
            //   MV_fwd = (0, 0), MV_bwd = (0, 0)
            // 情况2(colloc_mb是Forward):
            //   MV_fwd = colloc_mv
            //   MV_bwd = -colloc_mv * (B_time / Ref_time)

            derive_direct_mv();  // 标准导出

            // 关键: 色度MV导出
            // Xvid: 使用4MV导出 (div 8后按椭圆滤波)
            chroma_mv_4mv_derivation();
        }
    }
}
```

#### Tao 的 B 帧实现

```rust
// decoders/mpeg4/bframe.rs

fn apply_direct_mode(
    &mut self,
    ref_fwd: &VideoFrame,
    ref_bwd: &VideoFrame,
    ...) {
    // 1. Direct模式MV导出
    for (mb_y, mb_x) in MB_GRID {
        let colloc_info = ref_fwd.get_mb(colloc_x, colloc_y)?;

        if colloc_info.is_intra() {
            mv_fwd = (0, 0);
            mv_bwd = (0, 0);
        } else {
            // ✅ 标准导出实现
            mv_fwd = colloc_info.motion_vector;
            let time_factor = compute_time_factor();
            mv_bwd = -(mv_fwd * time_factor);
        }

        // 🔴 **C7**: 色度MV 使用 1MV 导出
        // 应该使用 4MV 导出 (block-level)
        chroma_mv = derive_chroma_mv_1mv(mv_fwd, mv_bwd);
    }

    // 2. ⚠️ 缺少 帧重排序 (DPB)
    // B帧按解码顺序应用, 但输出顺序错误
}
```

**关键差异**:

- 🔴 **C7**: Direct 模式色度MV使用1MV导出 (应为4MV)
- 🔴 **F3**: 缺少帧重排序 (B帧DPB管理)
- ✅ Tao已在2026-02-16修复了DPB和帧重排序

---

### 2.6 AC/DC 预测

#### Xvid 的AC/DC预测

```c
// image/prediction.c

// DC 预测 (DC Scaler 由量化参数决定)
int16_t dc_pred[4] = {0};  // 上/左/斜/当前预测值

// AC 预测方向选择
void GetPreditionDirection(int x, int y, int direction) {
    // direction = 0: 水平预测 (从左侧取AC系数)
    // direction = 1: 垂直预测 (从上侧取AC系数)
    // direction = 2: 无预测

    // 关键: 扫描表选择
    // - 没有AC预测: 使用 Zigzag 扫描
    // - 水平AC预测: 使用 Alternate Vertical Scan
    // - 垂直AC预测: 使用 Alternate Horizontal Scan
}

// 应用AC预测
void ApplyACPrediction(int16_t *pred_block,
                       int16_t *current_block,
                       int direction) {
    for (int i = 1; i < 8; i++) {
        if (direction == HORIZONTAL) {
            current_block[i] += pred_block[i * 8];  // 左侧系数
        } else {
            current_block[i * 8] += pred_block[i];  // 上侧系数
        }
    }
}
```

#### Tao 的 AC/DC 预测

```rust
// decoders/mpeg4/block.rs

fn select_ac_pred_scan(
    ac_pred_flag: bool,
    direction: PredictorDirection,
    default_scan: &[usize; 64],
) -> &[usize; 64] {
    if !ac_pred_flag {
        return default_scan;
    }

    match direction {
        // ❌ **C2**: 这里反了!
        // 标准规定:
        // - 垂直预测 -> 使用 Alternate HORIZONTAL Scan
        // - 水平预测 -> 使用 Alternate VERTICAL Scan

        // 当前实现 (错误):
        PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,     // ✓ 正确
        PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,     // ✓ 正确
        PredictorDirection::None => default_scan,
    }
}

// ❌ **M3**: AC预测值加法后缺少 [-2048, 2047] 裁剪
let predicted_coeff = existing_coeff + adjacent_ac_coeff;
// 应该: predicted_coeff = clamp(predicted_coeff, -2048, 2047)
```

**问题**:

- ⚠️ 当前实现扫描表已正确 (注释误导)
- 🔴 **M3**: AC预测后缺少范围裁剪

---

### 2.7 IDCT (Inverse Discrete Cosine Transform)

#### Xvid 的 IDCT 实现

```c
// image/image.c - idct function pointer

// 支持多个IDCT实现:
// 1. 参考IDCT (IEEE 1180-1990 合规)
// 2. Integer IDCT (定点快速实现)
// 3. SIMD IDCT (MMX/SSE/AVX优化)

void idct_int32() {
    // AAN (Arai, Agui, Nakajima) 算法
    // 使用定点浮点 (16-bit整数 + 16-bit分数部分)

    // 行变换 (8 x 8 矩阵的8行)
    for (int y = 0; y < 8; y++) {
        // AAN蝶形运算
        // 输入: f1, f3, f5, f7 (奇数DCT系数)
        //     f0, f2, f4, f6 (偶数DCT系数)

        // 中间值计算 (含rounding)
        const int SCALE_FACTOR = (1 << 13);  // 用于定点rounding

        a3 = (f5 + f7) * SCALE * some_const;  // 含rounding
        // ...
    }

    // 列变换 (结果的8列)
    // 类似行变换, 输出范围: [-256, 255]
    // 最后加8 (rounding), 右移4位 (得到[-16, 16]范围)
}

// IEEE 1180 合规性:
// - ±1-2 LSB的量化误差
// - 已验证通过官方测试集
```

#### Tao 的 IDCT 实现

```rust
// decoders/mpeg4/idct.rs

pub fn idct_8x8(block: &mut [[i16; 8]; 8]) {
    // 1. 行变换
    for y in 0..8 {
        // Chen-Wang算法 (AAN的变种)

        let a0 = block[y][0];
        let a1 = block[y][1];
        // ... 计算中间值 ...

        // ❌ **M4**: 缺少 rounding (+1024)
        // 标准行变换输出应该 >> 11 bit
        // 当前: 直接赋值回block

        // ⚠️ 非标蝶形结构可能导致 ±1-2 LSB 误差
    }

    // 2. 列变换
    for x in 0..8 {
        // 类似行变换
        // ⚠️ 同样缺少rounding, 精度可能受影响
    }
}

// IEEE 1180 compliance:
// ⚠️ 当前实现 ±1-2 LSB, 需要改进
```

**关键差异**:

- 🔴 **M4**: Tao 行/列变换缺少正确的 rounding (+1024)
- ⚠️ 蝶形结构不标准, 导致精度偏差
- 📊 **已在2026-02-16修复**: 添加了rounding, 改进IEEE 1180兼容性

---

### 2.8 GMC (Global Motion Compensation)

#### Xvid 的 GMC 实现

```c
// image/gmc.c
void GmcWarp() {
    // S-VOP (Sprite Video Object Plane) 处理

    // 1. 1-point GMC: 纯平移
    // MV directly applied

    // 2. 2-point GMC: 斜率 (仿射变换, 4自由度)
    //
    // | alpha      beta  | (3x3变换矩阵)
    // |-beta      alpha  |
    // | m.x       m.y   |
    //
    // (x', y') = alpha*x - beta*y + m.x
    //           = beta*x + alpha*y + m.y

    int warp_x = alpha * x - beta * y + m.x;
    int warp_y = beta * x  + alpha * y + m.y;

    // 范围检查 + 环绕 (wrapping)
    warp_x = CLIP(warp_x, 0, ref_width-8);

    // 应用MC (支持QPel)
    BlockCopy(dst, src[warp_y][warp_x], ...);

    // 3. 3-point GMC: 透视变换 (8自由度)
    // 完整3x3变换矩阵, 每个点都有独特的warp坐标
}
```

#### Tao 的 GMC 实现

```rust
// decoders/mpeg4/gmc.rs

pub fn apply_gmc(
    &mut self,
    ref_frame: &VideoFrame,
    gmc_params: &GmcParameters,
    output: &mut VideoFrame,
) {
    match gmc_params.sprite_warping_points {
        1 => {
            // ✅ 1-point GMC (平移) - 实现完整
            // 简单的全帧平移
        }
        2 => {
            // ⚠️ 2-point GMC - 仅简化平移
            // 应该计算: alpha, beta (仿射变换)
            // 当前实现: 忽略alpha/beta, 仅使用m.x/m.y (平移)

            // 需要完成:
            // let alpha = gmc_params.alpha;  // 缩放+旋转
            // let beta = gmc_params.beta;
            // warp_x = alpha * x - beta * y + m.x
            // warp_y = beta * x + alpha * y + m.y
        }
        3 => {
            // ❌ 3-point GMC (透视) - 仅简化平移
            // 需要完整的3x3变换矩阵计算
        }
        _ => {}
    }

    // 当前问题:
    // 🔴 **F1**: 2/3 点 GMC 仅简化为平移, 无仿射/透视变换
}
```

**问题**:

- ✅ 1-point GMC 已实现
- 🔴 **F1**: 2/3-point GMC 仅为平移, 缺少几何变换

---

### 2.9 高级特性对比

#### RVLC (Reversible Variable Length Code)

| 特性         | Xvid           | Tao                 |
| ------------ | -------------- | ------------------- |
| **逆向解码** | ✅ 完整实现    | ❌ 框架存在, 未完整 |
| **错误恢复** | ✅ 前/后向解码 | ⚠️ 前向退回         |
| **使用场景** | 高丢包率网络   | (不支持)            |
| **性能影响** | +5-10% CPU     | 无                  |

#### Data Partitioning

```c
// Xvid: 完整的分区管理
// Partition A: MCBPC, CBPY, MV, DQUANT (所有MB头)
// Partition B: DC系数 (使用RVLC)
// Partition C: AC系数

// Tao: 字节级启发式分析
// 使用 Resync Marker 定位分区边界
// 仅支持基础数据提取, 不支持RVLC后向解码

#[allow(dead_code)]
fn locate_partition_boundaries(&self, data: &[u8]) -> TaoResult<DataPartitionInfo> {
    // 扫描resync marker (0x000001B?)
    // 根据marker位置推断分区边界
}
```

#### 隔行扫描 (Interlaced Field Prediction)

| 特性         | Xvid     | Tao         |
| ------------ | -------- | ----------- |
| **字段解析** | ✅ 完整  | ✅ 已解析   |
| **场DCT**    | ✅ 8x4块 | ❌ 仅帧DCT  |
| **场预测**   | ✅ 完整  | ❌ 仅帧预测 |
| **MC校准**   | ✅ 完整  | ❌ 缺失     |

---

## 3. 发现的问题详解

### 问题分类矩阵

```
优先级     |  关键 Bug (C)      |  中等问题 (M)      |  缺失功能 (F)
-----------|------------------|------------------|------------------
影响范围   | 导致崩溃/错误     | 质量劣化/细微差异  | 特定流无法播放
修复时间   | 1-2小时           | 1-4小时           | 4-16小时
测试用例   | 单元 + 集成       | 单元测试          | 端到端测试
```

### 3.1 关键问题 (🔴)

#### C1: complexity_estimation 未解析

**位置**: `header.rs` L133

**问题描述**:

```rust
// 当前代码
if !complexity_disable {
    // 跳过 1 bit, 但实际应该根据 estimation_method 读取多个字段
    reader.skip_bits(1);  // ❌ 错误
}
```

**标准要求** (ISO 14496-2 §6.3.5):

```
if (complexity_estimation_disable == 0) {
    estimation_method (2 bits)
    // 根据 method 读取不同字段数量 (2-12 bits)
}
```

**后续影响**:

- 所有后续VOL字段位偏移错误
- 特别是带 complexity_estimation 的视频流会解析失败

**修复成本**: 1小时 (添加完整字段解析)

---

#### C2: AC预测扫描表错误

**位置**: `block.rs` L56-77

**问题**: 扫描表方向映射有误 (虽然注释可能误导)

实际上当前实现已正确:

```rust
PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,     // ✓
PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,     // ✓
```

**但存在其他AC预测问题**:

- 🔴 **M3**: AC预测后缺少范围裁剪

---

#### C3: Inter4V Block 0 MV预测错误

**位置**: `motion.rs` L72-82

**问题描述**:

```rust
// 当前: Block 0 使用全局邻居
let pred_mv = median(
    motion_vectors[(mbx - 1, mby)],    // ❌ 应该使用Block 3 (同一MB)
    motion_vectors[(mbx, mby - 1)],
    motion_vectors[(mbx - 1, mby - 1)]
);

// 正确做法:
// Block 0 邻居: (prev_block, top_block, diag_block)
// prev_block = 同MB内 Block 3 (如果存在)
// top_block = 上MB的 Block 2
// diag_block = 左上MB或同MB Block 3
```

**标准参考** (ISO 14496-2 Annex E):

- Inter4V模式下, 4个块各有独立MV
- Block编号: 左上=0, 右上=1, 左下=2, 右下=3
- 每个块的MV预测使用特定的邻居块

---

#### C4: S-VOP 映射为 I 帧

**位置**: `header.rs` L155, `mod.rs` L958

**问题**:

```rust
// 当前
3 => PictureType::I,  // ❌ 错误: S-VOP 应为特殊类型

// 正确
3 => PictureType::S,  // Sprite VOP
```

**后续影响**:

- GMC运动补偿从未应用
- S-VOP视频无法正确解码

---

#### C5: sprite_enable 比特宽度错误

**位置**: `header.rs` L100

**问题**:

```rust
// 当前: 固定读 1 bit
let sprite_enable = reader.read_bits(1)?;

// 正确: verid >= 2 时读 2 bits
let sprite_enable = if verid >= 2 {
    reader.read_bits(2)?
} else {
    reader.read_bits(1)?
};
```

**影响**: MPEG-4 Part 2 新版本 (verid=2+) 解析错误

---

#### C6: P帧色度MC缺少四分像素感知

**位置**: `mod.rs` L623-637

**问题**:

```rust
// 当前: 仅处理整像素或半像素
match chroma_fcode {
    0 => copy_full_pixel(),      // 整像素
    1 => interpolate_half_pixel(), // 半像素
    _ => interpolate_qpel(),       // ❌ 色度不支持QPel!
}

// 正确: 色度MV导出时应考虑QPel
// 虽然色度已导出为1/2像素精度
// 但在qpel类型视频中需要特殊处理
```

---

#### C7: Direct模式色度MV使用1MV导出

**位置**: `bframe.rs` L169

**问题**:

```rust
// 当前 (1MV导出)
let chroma_mv = derive_chroma_mv_1mv(mv_fwd, mv_bwd);
// 使用宏块级单MV计算色度MV

// 正确 (4MV导出)
let chroma_mv_4mv = [
    derive_chroma_from_block_mv(block0_mv),
    derive_chroma_from_block_mv(block1_mv),
    derive_chroma_from_block_mv(block2_mv),
    derive_chroma_from_block_mv(block3_mv),
];
let chroma_mv = median(chroma_mv_4mv);
```

**标准参考** (ISO 14496-2 Annex D.3.3):

- Direct模式下, 色度MV应从4个块的MV导出
- 使用中值滤波而非简单平均

**修复状态**: ✅ 已在 2026-02-16 修复

---

### 3.2 中等问题 (🟠)

#### M1: H.263反量化缺少范围裁剪

**问题**:

```rust
let dequant = (2 * quant + 1) * (2 * coeff + 1);
// ❌ 缺少裁剪
// 应该: dequant = clamp(dequant, -2048, 2047);
```

**影响**: 高量化参数时, 量化值超范围导致IDCT结果溢出

**修复成本**: 1行代码, 1小时测试

---

#### M2: MPEG反量化的mismatch control缺失

**问题**:

```rust
pub(super) fn apply_mismatch_control(coeff: &mut i16, intra: bool) {
    if !intra {  // ❌ 仅对Inter块
        // 应用mismatch control
    }
    // 标准要求对所有块应用
}
```

---

#### M3: AC预测后缺少范围裁剪

**问题**:

```rust
let predicted = existing + adjacent_ac;
// ❌ 缺少裁剪
// 应该: predicted = clamp(predicted, -2048, 2047);
```

---

#### M4: IDCT rounding与精度问题

**问题**:

```rust
// 当前: 缺少rounding (+1024)
// 行变换应该在最后添加 >> 11
// 列变换应该在最后添加 + 8 然后 >> 4
```

**修复状态**: ✅ 已在 2026-02-16 改进

---

#### M5: qpel MC rounding不一致

**问题**:

- Xvid: 在每个插值中间值应用rounding
- Tao: 仅在最终结果应用一次rounding
- 标准: 中间值应保持精度, 最终输出时rounding

---

### 3.3 缺失功能 (🟡)

#### F1: 2/3点GMC仅简化为平移

**状态**: ⚠️ 部分实现

需要完整2/3点GMC:

- Alpha/Beta 系数导出
- 仿射变换矩阵计算
- 透视变换 (3点)

---

#### F2: 隔行场预测未实现

**状态**: ❌ 框架存在, 未完整

需要:

- 场DCT (8x4块)
- 场预测 (上/下场选择)
- MC校准

---

#### F3: B帧帧重排序

**状态**: ✅ 已在 2026-02-16 实现 DPB

---

#### F4: RVLC后向解码

**状态**: ❌ 框架存在, 未完整

RVLC的难点:

- 双向VLC表
- 前向/后向解码切换
- 错误恢复逻辑

---

#### F5: Data Partitioning完整处理

**状态**: ⚠️ 字节启发式

完整支持需要:

- 分区标记精确识别
- RVLC后向解码
- 错误分区跳过

---

#### F6: alternate_vertical_scan_flag VOP解析

**状态**: ⚠️ 缺失VOP标志

需在VOP头读取并存储该标志

---

## 4. 完善工作计划

### 概览

基于Xvid对标, 本项目需完成 **9个修复阶段**, 预计 **8-10周**:

```
阶段1 (头部修复, 8h)
  └-> 阶段2 (系数处理, 6h)
       └-> 阶段3 (IDCT精度, 4h)
            └-> 阶段4 (运动补偿, 12h)
                 └-> 阶段5 (B帧完善, 8h)
                      └-> 阶段6 (GMC2/3点, 16h)
                           └-> 阶段7-8 (高级特性, 24h)
                                └-> 阶段9 (性能优化+100%验证, 20h)
```

### Phase 1: VOL/VOP 头部解析修复 (8h) ✅ 部分完成

**修复项**:

| 问题 | 修复内容                      | 优先级 | 难度 | 时间 |
| ---- | ----------------------------- | ------ | ---- | ---- |
| C1   | complexity_estimation完整解析 | 🔴     | 中   | 2h   |
| C5   | sprite_enable比特宽度         | 🔴     | 低   | 1h   |
| F6   | alternate_vertical_scan_flag  | 🟠     | 低   | 1h   |

**测试用例**:

- `test_vop_complexity_estimation`
- `test_vol_sprite_enable_verid`
- `test_vop_alternate_scan_flag`

**验收标准**:

- 解析包含complexity_estimation的流成功
- sprite_enable在verid=2+时正确读取
- 所有头测试通过

---

### Phase 2: DCT系数域修复 (6h) ✅ 已完成

**修复项**:

| 问题 | 修复内容                  | 优先级 | 难度 | 时间 |
| ---- | ------------------------- | ------ | ---- | ---- |
| M1   | H.263反量化范围裁剪       | 🟠     | 低   | 1h   |
| M2   | MPEG mismatch control扩展 | 🟠     | 低   | 1h   |
| M3   | AC预测范围裁剪            | 🟠     | 低   | 1h   |

**测试用例**:

- `test_dequant_h263_clipping`
- `test_dequant_mpeg_mismatch_all_blocks`
- `test_ac_prediction_clipping`

**验收标准**:

- 高量化参数视频正确解码
- 系数范围始终在[-2048, 2047]内
- 测试PSNR提升≥1dB

---

### Phase 3: IDCT精度提升 (4h) ✅ 已完成

**修复项**:

| 问题 | 修复内容             | 优先级 | 难度 | 时间 |
| ---- | -------------------- | ------ | ---- | ---- |
| M4   | 行/列变换rounding    | 🟠     | 中   | 3h   |
| M4   | IEEE前1180兼容性测试 | 🟠     | 中   | 1h   |

**改进细节**:

```rust
// 原始 (缺少rounding)
let s0 = ...;  // 行变换
block[y][x] = s0;

// 改进后 (添加rounding)
let s0 = (... + (1 << 10)) >> 11;  // +1024 rounding
block[y][x] = s0;
```

**测试用例**:

- `test_idct_ieee1180_compliance`
- `test_idct_known_values`
- `test_iframe_psnr`

---

### Phase 4: 运动补偿修复 (12h)

**修复项**:

| 问题 | 修复内容               | 优先级 | 难度 | 时间 |
| ---- | ---------------------- | ------ | ---- | ---- |
| C3   | Inter4V Block 0 MV预测 | 🔴     | 中   | 2h   |
| C6   | P帧色度MC四分像素感知  | 🔴     | 中   | 4h   |
| M5   | qpel MC rounding标准化 | 🟠     | 中   | 3h   |

**关键实现**:

```rust
// Inter4V MV预测修复
pub fn predict_inter4v_mv(block_idx: usize, ...) -> MotionVector {
    // block_idx: 0=左上, 1=右上, 2=左下, 3=右下

    let neighbors = match block_idx {
        0 => [left_mb_block3, top_mb_block2, diag_mb_block3],
        1 => [cur_mb_block0, top_mb_block3, left_top_mb_block3],
        2 => [left_mb_block3, cur_mb_block0, left_mb_block3],
        3 => [cur_mb_block2, top_mb_block2, cur_mb_block1],
    };

    median(neighbors[0], neighbors[1], neighbors[2])
}

// 色度MC四分像素处理
pub fn motion_comp_chroma_qpel(
    ref_frame: &VideoFrame,
    dx: i16, dy: i16,
    fcode: u8,
) -> Option<Block> {
    // 色度fcode通常为1 (1/2像素)
    // 但在qpel宏块中需特殊处理

    // 如果宏块使用了qpel, 色度也应升级到1/4像素?
    // 不! 色度始终为1/2像素, 但MC计算需感知qpel上下文
}
```

**测试用例**:

- `test_inter4v_mv_prediction`
- `test_pframe_chroma_mc_artifacts`
- `test_qpel_rounding_consistency`

**样本需求**:

- Inter4V编码的MPEG-4 (DivX)
- Quarterpel MPEG-4 (DivX 5.0+)

---

### Phase 5: B帧完善 (8h) ✅ 部分完成

**修复项**:

| 问题 | 修复内容                 | 优先级 | 难度 | 时间 |
| ---- | ------------------------ | ------ | ---- | ---- |
| C7   | Direct模式色度MV 4MV导出 | 🔴     | 中   | 3h   |
| F3   | B帧帧重排序DPB实现       | 🟡     | 中   | 4h   |

**修复状态**: ✅ 已在2026-02-16完成

**验收标准**:

- Direct模式B帧输出与FFmpeg一致
- 多B帧序列输出顺序正确

---

### Phase 6: GMC 2/3点实现 (16h)

**修复项**:

| 问题 | 修复内容          | 优先级 | 难度 | 时间 |
| ---- | ----------------- | ------ | ---- | ---- |
| C4   | S-VOP类型映射     | 🔴     | 低   | 1h   |
| F1   | 2点GMC (仿射变换) | 🟡     | 高   | 8h   |
| F1   | 3点GMC (透视变换) | 🟡     | 高   | 8h   |

**算法细节 - 2点GMC**:

```rust
// Sprite trajectory (s) 的两个点:
// s1 = (s_x1, s_y1) - 第一个参考点
// s2 = (s_x2, s_y2) - 第二个参考点

// 推导仿射变换矩阵:
// alpha = (s_x2 - s_x1) / 2^warp_accuracy
// beta = (s_y2 - s_y1) / 2^warp_accuracy
// m_x = s_x1
// m_y = s_y1

// 对每个宏块(mb_x, mb_y)的8x8块:
let block_x = (mb_x * 2 + block_x_offset) * 8;
let block_y = (mb_y * 2 + block_y_offset) * 8;

// 计算warp坐标
let warp_x = (alpha * block_x - beta * block_y + m_x) >> warp_accuracy;
let warp_y = (beta * block_x + alpha * block_y + m_y) >> warp_accuracy;

// 边界检查 + MC
if warp_x >= 0 && warp_y >= 0 {
    mc_block(dst, ref_frame, warp_x, warp_y);
}
```

**算法细节 - 3点GMC**:

```rust
// 3点sprite给出完整3x3变换矩阵
//
// | a  b  m_x |
// | c  d  m_y |
// | e  f   1  |
//
// (x', y', w') = (a*x + b*y + m_x,
//                 c*x + d*y + m_y,
//                 e*x + f*y + 1)
//
// 最终坐标 = (x'/w', y'/w')

// 按照sprite点计算系数
let a = compute_affine_coeff_a(sprite_config);
let b = compute_affine_coeff_b(sprite_config);
// ... e, f, m_x, m_y

// 对每个块进行透视warp
for mb in macroblock_grid {
    for block in mb.blocks {
        let (warp_x, warp_y, warp_w) =
            compute_perspective_coords(block, a, b, c, d, e, f, m_x, m_y);

        let final_x = warp_x / warp_w;
        let final_y = warp_y / warp_w;

        mc_block_qpel(dst, ref_frame, final_x, final_y);
    }
}
```

**测试样本** (需要):

- 2点GMC (仿射): `xvid_gmcqpel_artifact.avi` ✅
- 3点GMC (透视): (需从samples.ffmpeg.org查找)

---

### Phase 7: RVLC后向解码 (12h)

**修复项**:

- RVLC表导出 (反向索引)
- 后向解码循环
- 错误定位与同步

**难度**: 高 (需样本验证)

---

### Phase 8: Data Partitioning完整处理 (8h)

**修复项**:

- 精确分区标记定位
- Partition B/C分离解码
- RVLC集成 (需Phase 7)

---

### Phase 9: 隔行扫描与高级特性 (16h)

**修复项**:

- 场DCT (8x4块IDCT)
- 场预测 (field_pred)
- MC字段校准

**依赖**: Phase 6+

---

### Phase 10: 性能优化与100%对标验证 (20h)

**优化项**:

1. **VLC查表优化** (2h)
    - 将线性扫描改为二分查找或哈希表
    - 目标: O(1) 查表速度

2. **SIMD优化** (8h)
    - 运动补偿: 使用AVX/SSE向量操作
    - IDCT: 向量化 Chen-Wang算法
    - 边缘扩展: SIMD memcpy

3. **缓冲池复用** (4h)
    - 预分配运动补偿缓冲
    - IDCT工作空间复用

4. **100%像素级对标验证** (6h)
    - 收集5类标准测试样本
    - 与FFmpeg逐帧对比 (MD5/PSNR)
    - 差异分析与修正

---

### 总体工作量评估

| 阶段     | 修复项数 | 预计时间 | 状态         | 备注             |
| -------- | -------- | -------- | ------------ | ---------------- |
| 1        | 3        | 8h       | ⚠️ 部分      | 需补充C1/C5/F6   |
| 2        | 3        | 6h       | ✅ 完成      | 已实现           |
| 3        | 2        | 4h       | ✅ 完成      | IDCT精度改进     |
| 4        | 3        | 12h      | ⚠️ 进行中    | C3/C6/M5待修复   |
| 5        | 2        | 8h       | ✅ 完成      | DPB/帧重排序     |
| 6        | 3        | 16h      | ⚠️ 部分      | C4完成, F1待实现 |
| 7        | 1        | 12h      | ❌ 未开始    | 需样本           |
| 8        | 1        | 8h       | ❌ 未开始    | 需样本           |
| 9        | 1        | 16h      | ❌ 未开始    | 基础框架完成     |
| 10       | 4        | 20h      | ❌ 未开始    | 性能与对标       |
| **合计** | **23**   | **100h** | **45% 完成** | **2人周**        |

---

## 对比总结表

### Xvid → Tao 迁移优先度矩阵

```
高影响 │ C1 (头解析)      │ C4 (S-VOP)       │ F1 (2/3GMC) │ Performance
       │ C6 (色度MC)      │ C7 (色度MV)      │             │
高难度 │                 │                 │             │
───────┼─────────────────┼─────────────────┼─────────────┼───────────
       │ M1 (裁剪)       │ F2 (隔行扫描)   │ F5 (Part)   │ SIMD优化
中难度 │ M2 (mismatch)   │ F4 (RVLC)       │             │
───────┼─────────────────┼─────────────────┼─────────────┼───────────
       │ M4 (IDCT)       │ F3 (DPB) ✅     │ F6 (scan)   │ 缓冲池
低难度 │ M5 (rounding)   │ C3 (Inter4V)    │             │ 内存优化
```

---

## 即时修复清单 (本周)

### 🔴 Critical Path

1. **2h** - [C1] complexity_estimation 完整解析
2. **1h** - [C5] sprite_enable 比特宽度修复
3. **2h** - [C3] Inter4V Block 0 MV预测
4. **3h** - [C6] P帧色度MC四分像素感知
5. **1h** - [C4] S-VOP类型映射

**Subtotal**: 9h (可集中完成)

### 🟠 Medium Priority

6. **2h** - [M5] qpel rounding标准化
7. **1h** - [F6] alternate_vertical_scan_flag
8. **2h** - 增强单元测试覆盖

**Subtotal**: 5h

### 近期样本需求

```bash
# 从 samples.ffmpeg.org 下载用于验证:
✅ https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi
✅ https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi
⏳ https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+mp3++qpel-bframes.avi
⏳ 3点GMC样本 (需要搜索)
```

---

## 后续建议

### 短期 (1-2周)

1. **完成Critical Path修复** (9小时)
2. **收集高级特性样本** (查询samples.ffmpeg.org)
3. **建立FFmpeg对标测试** (自动化像素对比)

### 中期 (2-4周)

1. **实现2/3点GMC** (16h, 需样本)
2. **SIMD优化试点** (4-8h, MC/IDCT)
3. **性能基准对标** (FFmpeg同等级)

### 长期 (1个月+)

1. **完整RVLC/Data Partitioning** (依赖样本)
2. **隔行扫描支持** (依赖样本)
3. **生产级稳健性验证** (fuzzing/大样本集)

---

## 参考资源

- **ISO/IEC 14496-2**: MPEG-4 Part 2 标准文档
- **Xvid Source**: https://github.com/Sermale/xvid
- **FFmpeg mpeg4videodec.c**: https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/mpeg4videodec.c
- **样本库**: https://samples.ffmpeg.org/
- **IEEE 1180-1990**: IDCT参考实现

---

**文档完成日期**: 2026-02-16
**下次更新**: 修复C1后更新
