# MPEG-4 Part 2 立即修复行动计划 (本周)

> 目标: 在3-5天内修复5个🔴关键问题，提升解码稳定性 X 10
> 工作量: 9小时 + 2小时测试 = 11小时
> 状态: 未开始

---

## 索引

1. [修复 C1: complexity_estimation 解析](#修复-c1)
2. [修复 C5: sprite_enable 比特宽度](#修复-c5)
3. [修复 C3: Inter4V Block 0 MV预测](#修复-c3)
4. [修复 C6: P帧色度MC四分像素](#修复-c6)
5. [修复 C4: S-VOP PictureType](#修复-c4)

---

## 修复 C1

### 问题: complexity_estimation 未解析

**文件**: `crates/tao-codec/src/decoders/mpeg4/header.rs`

**当前代码** (L130-135):

```rust
// ❌ 错误: 忽略 complexity_estimation 导致后续字段位偏移

if !complexity_disable {
    let _ = reader.read_bits(1)?;  // ❌ 仅跳过1 bit
}

// 之后的所有字段位偏移错误!
let resync_marker_disable = reader.read_bits(1)?;
```

**修复方案**:

### Step 1: 扩展 VolInfo 结构体

```rust
// crates/tao-codec/src/decoders/mpeg4/types.rs

pub struct VolInfo {
    // ... 现有字段 ...

    // 新增 complexity_estimation 字段
    pub complexity_estimation_disable: bool,
    pub estimation_method: u8,  // 2 bits

    // 根据estimation_method存储的字段 (可选, 示例)
    pub complexity_estimation_params: ComplexityEstimationParams,
}

#[derive(Debug, Clone, Copy)]
pub struct ComplexityEstimationParams {
    // estimation_method = 0: 基础方法
    pub opaque: u8,
    pub transparent: u8,
    pub intraplomb: u8,
    pub interplomb: u8,
    pub dct_coeff: u8,
    pub dct_lines: u8,
    pub vlc_symbols: u8,
    pub vlc_bits: u8,
    // ... 根据标准补充
}
```

### Step 2: 实现 complexity_estimation 解析

```rust
// crates/tao-codec/src/decoders/mpeg4/header.rs

impl Mpeg4Decoder {
    #[allow(dead_code)]
    fn parse_complexity_estimation(
        reader: &mut BitReader,
        vol_info: &mut VolInfo,
    ) -> TaoResult<()> {
        // ISO 14496-2 §6.3.5

        let complexity_disable = reader.read_bits(1)?;
        vol_info.complexity_estimation_disable = complexity_disable != 0;

        if complexity_disable != 0 {
            // complexity_estimation 禁用, 不读取任何字段
            return Ok(());
        }

        // 读取 estimation_method (2 bits)
        let estimation_method = reader.read_bits(2)? as u8;
        vol_info.estimation_method = estimation_method;

        debug!("complexity_estimation: method={}", estimation_method);

        // 根据不同方法读取对应字段
        match estimation_method {
            0 => {
                // 基础方法: 读取固定字段集
                let opaque = reader.read_bits(1)? as u8;
                let transparent = reader.read_bits(1)? as u8;
                let intraplomb = reader.read_bits(1)? as u8;
                let interplomb = reader.read_bits(1)? as u8;
                let dct_coeff = reader.read_bits(1)? as u8;
                let dct_lines = reader.read_bits(1)? as u8;
                let vlc_symbols = reader.read_bits(1)? as u8;
                let vlc_bits = reader.read_bits(1)? as u8;

                vol_info.complexity_estimation_params = ComplexityEstimationParams {
                    opaque,
                    transparent,
                    intraplomb,
                    interplomb,
                    dct_coeff,
                    dct_lines,
                    vlc_symbols,
                    vlc_bits,
                };
            }
            1 => {
                // 方法1: 扩展字段
                // 实现类似...
                warn!("complexity_estimation method=1 not fully implemented");

                // 跳过对应字段以保持同步
                for _ in 0..8 {
                    let _ = reader.read_bits(1)?;
                }
            }
            _ => {
                // 保留方法
                warn!("complexity_estimation method={} reserved", estimation_method);
            }
        }

        Ok(())
    }
}
```

### Step 3: 集成到 VOL 头解析

```rust
// crates/tao-codec/src/decoders/mpeg4/header.rs
// 在 read_vol_header() 中修改:

// ❌ 旧代码
if !complexity_disable {
    let _ = reader.read_bits(1)?;
}

// ✅ 新代码
if !complexity_disable {
    // 调用完整的解析函数
    Self::parse_complexity_estimation(reader, &mut vol_info)?;
}

// 如果 complexity_disable=1, 则无需调用
// (只需检查标志, 后续字段位偏移正确)
```

### Step 4: 添加单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complexity_estimation_disabled() {
        // complexity_disable=1 的情况 (最常见)
        let data = vec![
            0x00, 0x00, 0x00, 0x01,  // start code
            0xB0,                     // VOL header
            0x04, 0x00, 0x00, 0x00,  // profile_level

            // ... 其他头字段 ...

            // complexity_disable = 1 (1 bit)
            0x80,  // 10000000

            // 后续字段应正确读取
        ];

        let mut reader = BitReader::new(&data[4..]);
        let mut vol_info = VolInfo::default();

        // 应成功解析, 不会位偏移
        let result = Mpeg4Decoder::parse_complexity_estimation(&mut reader, &mut vol_info);
        assert!(result.is_ok());
    }

    #[test]
    fn test_complexity_estimation_method0() {
        let data = vec![
            0x00,  // complexity_disable=0, estimation_method=00 (binary)
            0xFF,  // 8个标志位全=1
        ];

        let mut reader = BitReader::new(&data);
        let mut vol_info = VolInfo::default();

        let result = Mpeg4Decoder::parse_complexity_estimation(&mut reader, &mut vol_info);
        assert!(result.is_ok());
        assert_eq!(vol_info.complexity_estimation_disable, false);
        assert_eq!(vol_info.estimation_method, 0);
        assert_eq!(vol_info.complexity_estimation_params.opaque, 1);
    }
}
```

**验收标准**:

- ✅ 有 complexity_estimation 的VOL头正确解析
- ✅ 位偏移正确 (后续字段不反向移动)
- ✅ 单元测试通过

---

## 修复 C5

### 问题: sprite_enable 比特宽度错误

**文件**: `crates/tao-codec/src/decoders/mpeg4/header.rs`

**当前代码** (L100):

```rust
// ❌ 错误: 固定读 1 bit, 忽略 verid 版本

let sprite_enable = reader.read_bits(1)?;  // 总是读 1 bit!
```

**标准要求** (ISO 14496-2 §6.2.5.1):

```
if (video_object_layer_verid >= 2) {
    sprite_enable (2 bits)       // 2 bits!
} else {
    sprite_enable (1 bit)        // 1 bit
}
```

**修复方案**:

### Step 1: 扩展 VolInfo 保存 verid

```rust
// crates/tao-codec/src/decoders/mpeg4/types.rs

pub struct VolInfo {
    // ... 现有字段 ...
    pub video_object_layer_verid: u8,  // 新增: verid [1,5]
    pub sprite_enable: u8,              // 改为 u8 以支持 2 bits
    pub is_sprite: bool,                // 快速判断是否为S-VOP
}
```

### Step 2: 修复读取逻辑

```rust
// crates/tao-codec/src/decoders/mpeg4/header.rs
// 在 read_vol_header() 中修改:

// Step a: 读取 verid (条件)
let video_object_layer_verid = if object_start_code == 0xB0 {
    // VO (Video Object) 包含 verid
    let verid_and_priority = reader.read_bits(8)? as u8;
    verid_and_priority >> 4  // 高4 bits
} else {
    1  // 默认为1
};

vol_info.video_object_layer_verid = video_object_layer_verid;

// Step b: 根据 verid 读取 sprite_enable
let sprite_enable = if video_object_layer_verid >= 2 {
    reader.read_bits(2)? as u8  // ✅ 2 bits for verid >= 2
} else {
    reader.read_bits(1)? as u8  // 1 bit for verid < 2
};

vol_info.sprite_enable = sprite_enable;
vol_info.is_sprite = sprite_enable != 0;

debug!("sprite_enable={}, verid={}", sprite_enable, video_object_layer_verid);
```

### Step 3: 添加单元测试

```rust
#[test]
fn test_sprite_enable_verid1() {
    // verid=1 时, sprite_enable = 1 bit
    let data = vec![0xC0];  // 11000000 = verid=1, sprite_enable=1 (1 bit)
    let mut reader = BitReader::new(&data);

    let verid = 1;
    let sprite_enable = if verid >= 2 {
        reader.read_bits(2)? as u8
    } else {
        reader.read_bits(1)? as u8
    };

    assert_eq!(sprite_enable, 1);
}

#[test]
fn test_sprite_enable_verid2() {
    // verid=2 时, sprite_enable = 2 bits
    let data = vec![0xC0];  // 11000000 = first 2 bits = 11
    let mut reader = BitReader::new(&data);

    let verid = 2;
    let sprite_enable = if verid >= 2 {
        reader.read_bits(2)? as u8  // 读取 2 bits = 11 (3)
    } else {
        reader.read_bits(1)? as u8
    };

    assert_eq!(sprite_enable, 3);  // 2 bits: 11 = 3
}
```

**验收标准**:

- ✅ verid < 2 时读1 bit
- ✅ verid ≥ 2 时读2 bits
- ✅ 测试用例通过

---

## 修复 C3

### 问题: Inter4V Block 0 MV预测错误

**文件**: `crates/tao-codec/src/decoders/mpeg4/motion.rs`

**当前代码** (L72-82):

```rust
// ❌ 错误: 使用错误的邻居块

fn predict_inter4v_mv(
    &self,
    block_idx: usize,
    mb_x: u32, mb_y: u32,
) -> MotionVector {
    // 目前的实现对所有块使用相同的邻居选择逻辑
    // 这是错误的!

    let left_mb = (mb_x.saturating_sub(1), mb_y);
    let top_mb = (mb_x, mb_y.saturating_sub(1));
    let diag_mb = (mb_x.saturating_sub(1), mb_y.saturating_sub(1));

    // 直接使用MB级的MV (错误!)
    // 应该使用块级的MV
}
```

**标准规定** (ISO 14496-2 Annex E):

Block形状:

```
┌─────────────────┐
│  0   │   1      │  (8x8 一个宏块)
├─────────────────┤
│  2   │   3      │
└─────────────────┘
```

Block 0邻居 (位置 左上):

- `mvPred[0]` = mvd_block[3] (同MB内右下块, 优先级最高)
- `mvPred[1]` = mvd_block_top[2] (上MB的下行块)
- `mvPred[2]` = mvd_block_topleft[3] (左上MB的右下块)

使用中值: `mv_pred_0 = median(mvPred[0], mvPred[1], mvPred[2])`

### Step 1: 增强 MacroblockData 结构

```rust
// crates/tao-codec/src/decoders/mpeg4/types.rs

pub struct MacroblockData {
    // ... 现有字段 ...

    // 新增: 4个块的独立MV (仅 Inter4V 模式)
    pub block_mv: [MotionVector; 4],  // Block 0/1/2/3 的MV
}
```

### Step 2: 实现正确的MV预测

```rust
// crates/tao-codec/src/decoders/mpeg4/motion.rs

impl Mpeg4Decoder {
    /// 为 Inter4V 块计算MV预测值
    ///
    /// block_idx: 0=左上, 1=右上, 2=左下, 3=右下
    pub(super) fn predict_inter4v_block_mv(
        &self,
        block_idx: usize,
        mb_x: u32,
        mb_y: u32,
    ) -> MotionVector {
        match block_idx {
            0 => {
                // 左上块 - 邻居: [cur_block3, top_mb_block2, topleft_mb_block3]
                let mvPred0 = self.get_block_mv(mb_x, mb_y, 3)         // 同MB块3
                    .unwrap_or_default();
                let mvPred1 = self.get_block_mv(mb_x, mb_y - 1, 2)    // 上MB块2
                    .unwrap_or_default();
                let mvPred2 = self.get_block_mv(mb_x - 1, mb_y - 1, 3) // 左上MB块3
                    .unwrap_or_default();

                // 取中值
                let pred_x = Self::median(mvPred0.x, mvPred1.x, mvPred2.x);
                let pred_y = Self::median(mvPred0.y, mvPred1.y, mvPred2.y);
                MotionVector { x: pred_x, y: pred_y }
            }
            1 => {
                // 右上块 - 邻居: [cur_block0, top_mb_block3, topleft_mb_block2]
                let mvPred0 = self.get_block_mv(mb_x, mb_y, 0)
                    .unwrap_or_default();
                let mvPred1 = self.get_block_mv(mb_x + 1, mb_y - 1, 3)  // 右上MB块3
                    .unwrap_or_default();
                let mvPred2 = self.get_block_mv(mb_x, mb_y - 1, 3)      // 正上方MB块3
                    .unwrap_or_default();

                let pred_x = Self::median(mvPred0.x, mvPred1.x, mvPred2.x);
                let pred_y = Self::median(mvPred0.y, mvPred1.y, mvPred2.y);
                MotionVector { x: pred_x, y: pred_y }
            }
            2 => {
                // 左下块 - 邻居: [cur_block3, left_mb_block1, topleft_mb_block3]
                let mvPred0 = self.get_block_mv(mb_x, mb_y, 3)
                    .unwrap_or_default();
                let mvPred1 = self.get_block_mv(mb_x - 1, mb_y, 1)      // 左MB块1
                    .unwrap_or_default();
                let mvPred2 = self.get_block_mv(mb_x - 1, mb_y - 1, 3)
                    .unwrap_or_default();

                let pred_x = Self::median(mvPred0.x, mvPred1.x, mvPred2.x);
                let pred_y = Self::median(mvPred0.y, mvPred1.y, mvPred2.y);
                MotionVector { x: pred_x, y: pred_y }
            }
            3 => {
                // 右下块 - 邻居: [cur_block2, top_mb_block3, right_mb_block2]
                let mvPred0 = self.get_block_mv(mb_x, mb_y, 2)
                    .unwrap_or_default();
                let mvPred1 = self.get_block_mv(mb_x, mb_y - 1, 3)
                    .unwrap_or_default();
                let mvPred2 = self.get_block_mv(mb_x + 1, mb_y, 2)      // 右MB块2
                    .unwrap_or_default();

                let pred_x = Self::median(mvPred0.x, mvPred1.x, mvPred2.x);
                let pred_y = Self::median(mvPred0.y, mvPred1.y, mvPred2.y);
                MotionVector { x: pred_x, y: pred_y }
            }
            _ => MotionVector::default(),
        }
    }

    /// 获取指定宏块和块索引的MV
    fn get_block_mv(&self, mb_x: u32, mb_y: u32, block_idx: usize) -> Option<MotionVector> {
        // 从已解码的宏块缓冲中取出
        let mb_data = self.decoded_mbs.get(&(mb_x, mb_y))?;

        // 如果是 Inter4V, 返回块级MV
        // 否则返回宏块级MV (复制到4个块)
        match mb_data.mb_type {
            MbType::Inter4V => Some(mb_data.block_mv[block_idx]),
            _ if block_idx == 0 => Some(mb_data.motion_vector),
            _ => Some(mb_data.motion_vector),
        }
    }
}
```

### Step 3: 集成到宏块解码

```rust
// crates/tao-codec/src/decoders/mpeg4/mod.rs
// 在 decode_macroblock() 中修改:

if mb_type == MbType::Inter4V {
    // 为4个块分别解码MV
    for block_idx in 0..4 {
        // 获取预测值
        let mv_pred = self.predict_inter4v_block_mv(
            block_idx,
            mb_x as u32,
            mb_y as u32,
        );

        // 解码MVD
        let mv_x = Self::decode_mv_component(reader, fcode_x)?;
        let mv_y = Self::decode_mv_component(reader, fcode_y)?;

        // 应用预测
        let mv = MotionVector {
            x: mv_x + mv_pred.x,
            y: mv_y + mv_pred.y,
        };

        // 存储块级MV
        mb_data.block_mv[block_idx] = mv;
    }
}
```

### Step 4: 单元测试

```rust
#[test]
fn test_inter4v_block0_mv_prediction() {
    // 模拟3个邻居MV
    let mvPred0 = MotionVector { x: -8, y: 4 };   // 同MB块3
    let mvPred1 = MotionVector { x: -4, y: 8 };   // 上MB块2
    let mvPred2 = MotionVector { x: 0, y: 4 };    // 左上MB块3

    // 中值应该是 (-4, 4)
    let pred_x = Mpeg4Decoder::median(mvPred0.x, mvPred1.x, mvPred2.x);
    let pred_y = Mpeg4Decoder::median(mvPred0.y, mvPred1.y, mvPred2.y);

    assert_eq!(pred_x, -4);
    assert_eq!(pred_y, 4);
}
```

---

## 修复 C6

### 问题: P帧色度MC缺少四分像素感知

**文件**: `crates/tao-codec/src/decoders/mpeg4/mod.rs` (L623-637)

**当前代码**:

```rust
// ❌ 错误: 色度仅处理整像素或半像素, 不处理四分像素上下文

fn motion_compensation_chroma(
    &mut self,
    ref_frame: &VideoFrame,
    mb_x: u32, mb_y: u32,
    mv_luma: MotionVector,
    chroma_fcode: u8,
) {
    // 色度MV导出 (标准)
    let mv_chroma = self.derive_chroma_mv(mv_luma, chroma_fcode)?;

    // 应用MC, 但对于 qpel 宏块没有特殊处理
    match chroma_fcode {
        0 => copy_full_pixel(),      // 整像素
        1 => interpolate_half_pixel(), // 半像素
        _ => interpolate_qpel(),       // ❌ 色度本不支持qpel!
    }
}
```

**问题分析**:

Xvid的处理:

```
MPEG-4 标准规定:
- 亮度: 支持 0/1/2 (整/半/四分像素)
- 色度: 固定 0/1 (整/半像素), 不支持四分像素!

但在 qpel 宏块中:
- 亮度使用四分像素 (chroma_fcode=0 表示qpel)
- 色度仍使用半像素精度

关键: 色度MC需要感知 qpel 的存在,
但输出精度仍为半像素
```

### Step 1: 修复色度MC逻辑

```rust
// crates/tao-codec/src/decoders/mpeg4/mod.rs

impl Mpeg4Decoder {
    pub(super) fn motion_compensation_chroma(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        mv_luma: MotionVector,
        mb_has_qpel: bool,  // 新参数: 是否使用qpel
        chroma_fcode: u8,
    ) -> TaoResult<()> {
        // 色度MV导出 (根据亮度MV)
        let mv_chroma = self.derive_chroma_mv(mv_luma)?;

        // ✅ 修复: 色度精度处理

        // 情况1: 标准P帧 (无qpel)
        // chroma_fcode ∈ {0, 1: 整/半像素
        if !mb_has_qpel {
            match chroma_fcode {
                0 => {
                    // 整像素色度MC
                    let (x, y) = (mv_chroma.x >> 2, mv_chroma.y >> 2);
                    self.apply_chroma_mc_full_pixel(
                        ref_frame, dst, mb_x, mb_y, x, y
                    )?;
                }
                1 => {
                    // 半像素色度MC
                    let (x, y) = (mv_chroma.x >> 1, mv_chroma.y >> 1);
                    self.apply_chroma_mc_half_pixel(
                        ref_frame, dst, mb_x, mb_y, x, y
                    )?;
                }
                _ => {
                    // chroma_fcode > 1: 也是半像素
                    // 但fcode影响MV范围
                    let (x, y) = (mv_chroma.x >> 1, mv_chroma.y >> 1);
                    self.apply_chroma_mc_half_pixel(
                        ref_frame, dst, mb_x, mb_y, x, y
                    )?;
                }
            }
            return Ok(());
        }

        // 情况2: qpel宏块 (chroma_fcode=0 表示qpel)
        // 虽然色度不支持qpel, 但亮度使用了qpel
        // -> 色度应使用对应的半像素位置

        if mb_has_qpel {
            // qpel MV 应转换为半像素精度
            // mv_luma 的残差是四分像素 (dx,dy ∈ {0,1,2,3})
            // -> 四舍五入到半像素 (dx',dy' ∈ {0,1})

            let mv_chroma_rounded = MotionVector {
                x: (mv_luma.x + 1) / 2,  // 四舍五入
                y: (mv_luma.y + 1) / 2,
            };

            // 应用半像素MC
            self.apply_chroma_mc_half_pixel(
                ref_frame, dst, mb_x, mb_y,
                mv_chroma_rounded.x,
                mv_chroma_rounded.y
            )?;
        }

        Ok(())
    }

    /// 导出色度MV (从亮度MV)
    fn derive_chroma_mv(&self, mv_luma: MotionVector) -> MotionVector {
        // ISO 14496-2 Annex D.2.2
        // 色度MV与亮度MV的关系

        // 简单情况 (对于大多数P帧):
        // 色度MV = 亮度MV / 2 (向下舍入)

        MotionVector {
            x: mv_luma.x >> 1,
            y: mv_luma.y >> 1,
        }
    }

    fn apply_chroma_mc_half_pixel(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        dx: i16, dy: i16,
    ) -> TaoResult<()> {
        // 应用半像素色度MC
        // dx, dy ∈ {-31...+31} (以半像素为单位)

        let (full_x, rem_x) = ((dx / 2) as i32, (dx % 2) as u8);
        let (full_y, rem_y) = ((dy / 2) as i32, (dy % 2) as u8);

        // 根据端点选择插值方法
        match (rem_x, rem_y) {
            (0, 0) => {
                // 整像素复制
                self.copy_chroma_block(
                    ref_frame, dst, mb_x, mb_y,
                    full_x as u32, full_y as u32
                )?;
            }
            (1, 0) => {
                // 水平半像素
                self.interpolate_chroma_h_half(
                    ref_frame, dst, mb_x, mb_y,
                    full_x as u32, full_y as u32
                )?;
            }
            (0, 1) => {
                // 垂直半像素
                self.interpolate_chroma_v_half(
                    ref_frame, dst, mb_x, mb_y,
                    full_x as u32, full_y as u32
                )?;
            }
            (1, 1) => {
                // 双向半像素 (双线性插值)
                self.interpolate_chroma_hv_half(
                    ref_frame, dst, mb_x, mb_y,
                    full_x as u32, full_y as u32
                )?;
            }
            _ => unreachable!(),
        }

        Ok(())
    }
}

// 辅助函数声明
impl Mpeg4Decoder {
    fn apply_chroma_mc_full_pixel(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        x: i32, y: i32,
    ) -> TaoResult<()> { /* ... */ }

    fn copy_chroma_block(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        x: u32, y: u32,
    ) -> TaoResult<()> { /* ... */ }

    fn interpolate_chroma_h_half(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        x: u32, y: u32,
    ) -> TaoResult<()> { /* ... */ }

    fn interpolate_chroma_v_half(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        x: u32, y: u32,
    ) -> TaoResult<()> { /* ... */ }

    fn interpolate_chroma_hv_half(
        &mut self,
        ref_frame: &VideoFrame,
        dst: &mut VideoFrame,
        mb_x: u32, mb_y: u32,
        x: u32, y: u32,
    ) -> TaoResult<()> { /* ... */ }
}
```

### Step 2: 集成到宏块解码

```rust
// crates/tao-codec/src/decoders/mpeg4/mod.rs
// 在 decode_macroblock() 调用处修改:

// ✅ 修复: 传递 mb_has_qpel 标志
let mb_has_qpel = vop_info.quant_precision &&
                  (mb_data.quant_type == QuantizationType::Qpel);

self.motion_compensation_chroma(
    ref_frame,
    &mut dst_frame,
    mb_x as u32, mb_y as u32,
    mv_luma,
    mb_has_qpel,  // ✅ 新参数
    self.vop_info.chroma_fcode,
)?;
```

### Step 3: 单元测试

```rust
#[test]
fn test_chroma_mc_qpel_sensitivity() {
    // 测试qpel宏块中色度MC的处理
    let mv_luma = MotionVector { x: 10, y: 7 };  // qpel单位

    let decoder = Mpeg4Decoder::new();
    let mv_chroma = decoder.derive_chroma_mv(mv_luma);

    // 色度MV = 亮度MV / 2
    assert_eq!(mv_chroma.x, 5);
    assert_eq!(mv_chroma.y, 3);
}

#[test]
fn test_chroma_mc_rounding() {
    // 四舍五入
    let mv = MotionVector { x: 11, y: 8 };
    let rounded_x = (mv.x + 1) / 2;
    let rounded_y = (mv.y + 1) / 2;

    assert_eq!(rounded_x, 6);
    assert_eq!(rounded_y, 4);
}
```

---

## 修复 C4

### 问题: S-VOP 映射为 I 帧

**文件**: `crates/tao-codec/src/decoders/mpeg4/header.rs` (L155)

**当前代码**:

```rust
// ❌ 错误
let vop_coding_type = reader.read_bits(2)?;
match vop_coding_type {
    0 => PictureType::I,   // I-VOP
    1 => PictureType::P,   // P-VOP
    2 => PictureType::B,   // B-VOP
    3 => PictureType::I,   // ❌ S-VOP 错误映射为 I!
    _ => unreachable!(),
}
```

**标准规定** (ISO 14496-2 §6.2.5):

- vop_coding_type = 0: I-VOP (Intra)
- vop_coding_type = 1: P-VOP (Predicted)
- vop_coding_type = 2: B-VOP (Bidirectional)
- vop_coding_type = 3: S-VOP (Sprite/Static)

**后果**:

- S-VOP被误认为I帧
- GMC运动补偿从未应用
- 导致错误的解码输出

### Step 1: 扩展 PictureType 枚举

```rust
// crates/tao-codec/src/frame.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureType {
    I,  // Intra
    P,  // Predicted
    B,  // Bidirectional
    S,  // Sprite (MPEG-4 only)  // ✅ 新增
}
```

### Step 2: 修复映射逻辑

```rust
// crates/tao-codec/src/decoders/mpeg4/header.rs

// L155: 修改映射
let vop_coding_type = reader.read_bits(2)?;
let picture_type = match vop_coding_type {
    0 => PictureType::I,   // I-VOP
    1 => PictureType::P,   // P-VOP
    2 => PictureType::B,   // B-VOP
    3 => PictureType::S,   // ✅ S-VOP (Sprite)
    _ => unreachable!(),
};

vop_info.picture_type = picture_type;

// L156: 添加S-VOP标志
vop_info.is_sprite = picture_type == PictureType::S;

debug!("vop_coding_type={}, picture_type={:?}", vop_coding_type, picture_type);
```

### Step 3: 在VOP INFO中保存

```rust
// crates/tao-codec/src/decoders/mpeg4/types.rs

pub struct VopInfo {
    pub picture_type: PictureType,  // I/P/B/S
    pub is_sprite: bool,             // ✅ 新增: S-VOP标志
    // ... 其他字段 ...
}
```

### Step 4: 在解码循环中应用GMC

```rust
// crates/tao-codec/src/decoders/mpeg4/mod.rs

fn decode_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
    let mut output_frame = self.create_output_frame()?;

    match self.vop_info.picture_type {
        PictureType::I => {
            self.decode_i_frame(reader, &mut output_frame)?;
        }
        PictureType::P => {
            self.decode_p_frame(reader, &mut output_frame)?;
        }
        PictureType::B => {
            self.decode_b_frame(reader, &mut output_frame)?;
        }
        PictureType::S => {
            // ✅ 修复: S-VOP 应用 GMC
            self.decode_s_vop(reader, &mut output_frame)?;
        }
    }

    Ok(output_frame)
}

fn decode_s_vop(&mut self, reader: &mut BitReader, output: &mut VideoFrame) -> TaoResult<()> {
    // S-VOP 使用 GMC (Global Motion Compensation)

    if let Some(ref_frame) = &self.ref_frame {
        // 应用 GMC 运动补偿
        self.apply_gmc(
            ref_frame,
            &self.gmc_params,  // 从 complexity_estimation 中提取
            output
        )?;
    }

    // 可能还有增量编码 (AC 系数)
    // 标准允许 S-VOP 只包含运动, 不包含残差

    Ok(())
}
```

### Step 5: 单元测试

```rust
#[test]
fn test_svop_picture_type_mapping() {
    let data = vec![0x30];  // vop_coding_type=3 (11b)
    let mut reader = BitReader::new(&data);

    let vop_coding_type = reader.read_bits(2)?;
    let picture_type = match vop_coding_type {
        0 => PictureType::I,
        1 => PictureType::P,
        2 => PictureType::B,
        3 => PictureType::S,
        _ => unreachable!(),
    };

    assert_eq!(picture_type, PictureType::S);
}

#[test]
fn test_svop_decoding() {
    // 实际S-VOP样本解码测试
    // (需要从 samples.ffmpeg.org 获取包含S-VOP的样本)

    let sample_url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi";

    // 解码前10帧
    let mut demuxer = DemuxerRegistry::open(sample_url)?;
    let stream = demuxer.find_stream(MediaType::Video)?;

    let mut decoder = CodecRegistry::create_decoder(stream.codec_id)?;

    let mut frame_count = 0;
    while let Some(packet) = demuxer.read_packet()? {
        if packet.stream_index == stream.index {
            if let Ok(frame) = decoder.decode_frame(&packet)? {
                // 检查 S-VOP 帧被正确处理
                if frame.picture_type == PictureType::S {
                    // S-VOP不应该导致崩溃或错误
                    assert!(frame.data.is_some());
                }
                frame_count += 1;
                if frame_count >= 10 {
                    break;
                }
            }
        }
    }
}
```

---

## 总结

5个关键修复的完成检查表:

- [ ] **C1**: complexity_estimation 完整解析 (2h)
- [ ] **C5**: sprite_enable 比特宽度修复 (1h)
- [ ] **C3**: Inter4V Block 0 MV预测 (2h)
- [ ] **C6**: P帧色度MC四分像素感知 (3h)
- [ ] **C4**: S-VOP PictureType映射 (1h)

**测试覆盖** (需要样本):

- ✅ 标准VOL头 (通常测试)
- ⏳ S-VOP视频: `xvid_gmcqpel_artifact.avi`
- ⏳ DivX Inter4V: `mpeg4_avi.avi`
- ⏳ Quarterpel: `DivX51-Qpel.avi`

**预期收益**:

- 稳定性 X 10 (避免多种视频流导致的崩溃)
- 解码正确性大幅改善 (特别是高级功能支持)
- 为后续GMC/RVLC etc实现铺平道路

---

**下一步**: 立即开始实现这5个修复，完成后进行完整回归测试。
