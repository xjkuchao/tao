# MPEG-4 Part 2 解码器完善计划 — 100% FFmpeg 对标

> 日期: 2026-02-16
> 状态: 执行中
> 目标: 使 tao 的 MPEG-4 Part 2 解码器输出与 FFmpeg 像素级一致, 兼容性/稳健性/性能达到同等水平

---

## 0. 背景与现状

### 0.1 当前实现概况

解码器位于 `crates/tao-codec/src/decoders/mpeg4/`, 共 12 个文件, ~3,894 行代码.

| 文件         | 行数 | 功能                                      |
| ------------ | ---- | ----------------------------------------- |
| mod.rs       | 1271 | 主解码器, I/P/B 帧解码, 宏块解码循环      |
| vlc.rs       | 755  | VLC 表 (MCBPC/CBPY/DC/MVD/AC), O(1) 查表  |
| bframe.rs    | 372  | B 帧: Direct/Forward/Backward/Interpolate |
| motion.rs    | 344  | MV 解码/预测, 半像素/四分像素运动补偿     |
| header.rs    | 311  | VOL/VOP 头解析                            |
| bitreader.rs | 160  | 比特流读取器                              |
| gmc.rs       | 134  | GMC 精灵轨迹解码                          |
| block.rs     | 130  | 8x8 块 DCT 系数解码, AC/DC 预测           |
| tables.rs    | 120  | 量化矩阵, 扫描表, DC 缩放因子             |
| types.rs     | 118  | 类型定义                                  |
| idct.rs      | 100  | 整数 IDCT (Chen-Wang 算法)                |
| dequant.rs   | 79   | H.263/MPEG 反量化                         |

### 0.2 已实现功能 ✅

- I/P/B 帧解码 (B: Direct/Forward/Backward/Interpolate 模式)
- VOL/VOP 头部解析
- 完整 VLC 解码 (Escape Mode 1/2/3), O(1) 快速查表
- H.263 和 MPEG 两种反量化
- DC 缩放因子 (标准 Table 7-1)
- 半像素/四分像素运动补偿 (6-tap FIR)
- 色度 MV 导出 (含 rounding 表)
- MV 范围 wrapping
- AC/DC 预测
- 交替扫描表
- 失配控制 (MPEG 量化类型)
- 边缘扩展
- Resync marker 检测
- 整数 IDCT (Chen-Wang 定点算法)

### 0.3 参考对比: FFmpeg vs Xvid

| 特性                    | FFmpeg (mpeg4videodec.c) | Xvid (libxvidcore) | tao 当前         |
| ----------------------- | ------------------------ | ------------------ | ---------------- |
| Simple Profile          | ✅ 完整                  | ✅ 完整            | ⚠️ 部分          |
| Advanced Simple Profile | ✅ 完整                  | ✅ 完整            | ⚠️ 部分          |
| B 帧                    | ✅ 完整                  | ✅ 完整            | ⚠️ 有 bug        |
| 四分像素 MC             | ✅ 精确                  | ✅ 精确            | ⚠️ rounding 差异 |
| GMC (S-VOP)             | ✅ 1/2/3 点              | ✅ 1/2/3 点        | ❌ 未集成        |
| 交错扫描 (Interlaced)   | ✅ 场预测+场 DCT         | ✅ 场预测+场 DCT   | ❌ 仅解析        |
| RVLC 后向解码           | ✅ 完整                  | ✅ 完整            | ❌ 前向回退      |
| Data Partitioning       | ✅ 完整                  | ✅ 完整            | ❌ 字节启发式    |
| AC/DC 预测              | ✅ 按标准                | ✅ 按标准          | ❌ 扫描表错误    |
| IDCT 精度               | ✅ IEEE 1180 合规        | ✅ IEEE 1180 合规  | ⚠️ ±1-2 LSB      |
| 反量化裁剪              | ✅ [-2048, 2047]         | ✅ [-2048, 2047]   | ❌ 缺失          |
| complexity_estimation   | ✅ 完整解析              | ✅ 完整解析        | ❌ 未解析        |
| 帧重排序 (B 帧 DPB)     | ✅                       | ✅                 | ❌               |
| 错误恢复                | ✅ resync + slice        | ✅ resync + slice  | ⚠️ 基础          |

---

## 1. 发现的问题 — 按严重性排列

### 1.1 关键 Bug (🔴 会产生可见错误输出)

| #   | 模块      | 位置                  | 问题描述                                                                                     |
| --- | --------- | --------------------- | -------------------------------------------------------------------------------------------- |
| C1  | header.rs | L133                  | `complexity_estimation` 未解析 — `complexity_disable == false` 时后续所有 VOL 字段位偏移错误 |
| C2  | block.rs  | L56-77                | AC 预测使用了错误的扫描位置 — 垂直预测应使用交替水平扫描, 水平预测应使用交替垂直扫描         |
| C3  | motion.rs | L72-82                | Inter4V block 0 MV 预测使用了错误的邻居                                                      |
| C4  | mod.rs    | L958 + header.rs L155 | S-VOP 映射为 I 帧 → GMC 运动补偿从未应用                                                     |
| C5  | header.rs | L100                  | `sprite_enable` 固定读 1 bit — `verid >= 2` 时应读 2 bits                                    |
| C6  | mod.rs    | L623-637              | P 帧色度 MC 缺少四分像素感知                                                                 |
| C7  | bframe.rs | L169                  | Direct 模式色度 MV 使用 1MV 导出 — 应使用 4MV 导出                                           |

### 1.2 中等问题 (🟠 细微质量劣化)

| #   | 模块       | 位置     | 问题描述                                                   |
| --- | ---------- | -------- | ---------------------------------------------------------- |
| M1  | dequant.rs | L30-40   | H.263 反量化后缺少 [-2048, 2047] 裁剪                      |
| M2  | dequant.rs | L54-56   | MPEG 反量化的 mismatch control 仅对 Inter 块执行           |
| M3  | block.rs   | L60-77   | AC 预测值加法后缺少 [-2048, 2047] 裁剪                     |
| M4  | idct.rs    | 整个文件 | 行变换缺少 rounding (+1024), 蝶形结构非标准, ±1-2 LSB 误差 |
| M5  | motion.rs  | L198-270 | qpel MC rounding 行为与标准/FFmpeg 不一致                  |

### 1.3 缺失功能 (🟡 特定流无法播放)

| #   | 模块      | 问题描述                                     |
| --- | --------- | -------------------------------------------- |
| F1  | gmc.rs    | 2/3 点 GMC 仅简化为平移, 无仿射/透视变换     |
| F2  | bframe.rs | 交错场预测 (field_for_top/bot) 未实现        |
| F3  | mod.rs    | B 帧无帧重排序                               |
| F4  | vlc.rs    | RVLC 后向解码未实现                          |
| F5  | mod.rs    | Data Partitioning 分析为字节级启发式         |
| F6  | header.rs | 缺少 `alternate_vertical_scan_flag` VOP 解析 |

---

## 2. 测试样本规划

### 2.1 现有样本

| 样本                  | URL                                                               | 特性                  |
| --------------------- | ----------------------------------------------------------------- | --------------------- |
| mpeg4_avi.avi         | `https://samples.ffmpeg.org/V-codecs/MPEG4/mpeg4_avi.avi`         | 标准 MPEG-4, AVI 容器 |
| data_partitioning.avi | `https://samples.ffmpeg.org/V-codecs/MPEG4/data_partitioning.avi` | Data Partitioning     |

### 2.2 需要新增的样本

每个阶段开始前需从 https://samples.ffmpeg.org/ 查找并验证 (`ffprobe <URL>`) 以下类型的样本:

| 需求            | 搜索路径建议                       | 用途           |
| --------------- | ---------------------------------- | -------------- |
| B 帧 MPEG-4     | `V-codecs/MPEG4/` 或 `mov/` 目录   | B 帧解码正确性 |
| 四分像素 MPEG-4 | `V-codecs/MPEG4/` 目录             | qpel MC 测试   |
| GMC/S-VOP       | `V-codecs/MPEG4/` 或 `DivX/` 目录  | GMC 运动补偿   |
| 交错 MPEG-4     | `V-codecs/MPEG4/` 或 `MPEG4/` 目录 | 场预测+场 DCT  |
| DivX 编码       | `DivX/` 目录                       | DivX 兼容性    |
| Xvid 编码       | `V-codecs/MPEG4/` 或用户生成       | Xvid 兼容性    |
| 多种 profile    | `V-codecs/MPEG4/` 各子目录         | 边界情况覆盖   |

> 每个阶段的第一步操作是: 浏览 https://samples.ffmpeg.org/V-codecs/MPEG4/ 和 https://samples.ffmpeg.org/allsamples.txt 搜索 mpeg4/MPEG4/divx/xvid, 找到合适样本, `ffprobe` 验证后添加到 `samples/SAMPLE_URLS.md`

---

## 3. 执行阶段

### 阶段 1: VOL/VOP 头部解析修复 (🔴 最高优先)

**目标**: 修复头部解析错误, 确保所有后续字段正确读取

**修复项**:

1. **实现 `complexity_estimation` 完整解析** — `crates/tao-codec/src/decoders/mpeg4/header.rs`
    - 参考 ISO 14496-2 §6.3.5 和 FFmpeg `ff_mpeg4_decode_picture_header()` 中 `decode_vol_header()` 的 `complexity_estimation` 分支
    - 解析 `estimation_method`, 根据方法跳过对应字段
    - 存储到 `VolInfo` 中 (即使不使用, 也必须正确跳过比特)

2. **修复 `sprite_enable` 比特宽度** — `header.rs`
    - 在 `VolInfo` 中增加 `video_object_layer_verid` 字段
    - `sprite_enable`: `verid >= 2` 时读 2 bits, 否则读 1 bit

3. **解析 VOP `alternate_vertical_scan_flag`** — `header.rs`
    - 在 P-VOP 和 B-VOP 中读取该标志并写入 `VopInfo`

4. **修复 S-VOP 类型映射** — `header.rs`
    - `vop_coding_type == 3` 映射为 `PictureType::S`
    - 在 `VopInfo` 增加 `is_sprite: bool` 字段

**测试用例 (阶段 1)**:

```
- test_vol_header_parse_basic
- test_vol_header_complexity_estimation
- test_vop_header_svop_detection
- test_vol_header_sprite_enable_verid2
```

**验收标准**: 头部解析测试全部通过, 解码前 5 帧无崩溃

---

### 阶段 2: AC/DC 预测与反量化修复 (🔴 高优先)

**状态**: ✅ 已完成 2026-02-16

**目标**: 修正 DCT 系数域的预测和量化错误

**修复项**:

1. **修复 AC 预测扫描表选择** — `block.rs`
2. **AC 预测后裁剪** — `block.rs`
3. **H.263 反量化后裁剪** — `dequant.rs`
4. **MPEG 反量化 mismatch control 扩展到 Intra 块** — `dequant.rs`

**测试用例 (阶段 2)**:

```
- test_ac_prediction_vertical_direction
- test_ac_prediction_horizontal_direction
- test_ac_prediction_clipping
- test_dequant_h263_clipping
- test_dequant_mpeg_mismatch_intra
- test_mpeg4_decode_basic_pixel_check
```

---

### 阶段 3: IDCT 精度提升 (🟠 中等优先)

**目标**: IDCT 通过 IEEE 1180 合规测试

**修复项**:

- 替换为 IEEE 1180 合规 IDCT (FFmpeg 或 Xvid 方案)

**测试用例 (阶段 3)**:

```
- test_idct_ieee1180_random
- test_idct_ieee1180_sparse
- test_idct_known_values
- test_mpeg4_iframe_psnr_improvement
```

---

### 阶段 4: 运动补偿修复 (🔴 高优先)

**目标**: P 帧运动补偿像素精度与 FFmpeg 一致

**修复项**:

1. 修复 Inter4V block 0 MV 预测
2. P 帧色度 MC 四分像素感知
3. qpel MC rounding 修正
4. MVD 解码优化 (性能)

---

### 阶段 5: B 帧修复与帧重排序 (🔴 高优先)

**目标**: B 帧像素精确, 输出顺序与 FFmpeg 一致

**修复项**:

1. Direct 模式色度 MV 修复
2. Direct 模式 MV 计算验证
3. B 帧帧重排序 (DPB)

---

### 阶段 6: GMC (S-VOP) 实现 (🟡 中等优先)

**目标**: 完整实现 GMC 1/2/3 点运动补偿

---

### 阶段 7: 高级功能完善 (🟡 较低优先)

**目标**: 完善 RVLC, Data Partitioning, 交错扫描等高级特性

---

### 阶段 8: 性能优化与 100% 对标验证 (🟢 最终阶段)

**目标**: 达到 FFmpeg 同等性能, 100% 像素级一致

---

## 4. 依赖关系图

```
阶段 1 (头部解析)
  |
  |--> 阶段 2 (AC/DC 预测 + 反量化)
  |      |
  |      |--> 阶段 3 (IDCT 精度)
  |             |
  |             |--> 阶段 4 (运动补偿) --> 阶段 5 (B 帧) --> 阶段 8 (性能+验证)
  |                                                              ^
  |--> 阶段 6 (GMC) --------------------------------------------|
                                                                 ^
       阶段 7 (高级功能) --------------------------------------|
```

---

## 9. 进度追踪

| 阶段                   | 状态      | 完成日期 | 备注 |
| ---------------------- | --------- | -------- | ---- |
| 阶段 1: 头部解析       | ✅ 已完成 | 2026-02-16 | VOL/VOP 头解析修复与测试补充 |
| 阶段 2: AC/DC + 反量化 | ⬜ 待执行 | —        | —    |
| 阶段 3: IDCT           | ⬜ 待执行 | —        | —    |
| 阶段 4: 运动补偿       | ⬜ 待执行 | —        | —    |
| 阶段 5: B 帧           | ⬜ 待执行 | —        | —    |
| 阶段 6: GMC            | ⬜ 待执行 | —        | —    |
| 阶段 7: 高级功能       | ⬜ 待执行 | —        | —    |
| 阶段 8: 性能 + 验证    | ⬜ 待执行 | —        | —    |
