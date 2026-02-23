# H264 解码器精度 100% 提升计划 (第3轮更新)

**目标文件**: `data/1_h264.mp4` (1920×1080, Main/High profile, CABAC, 多参考 P/B slices)
**对比基准**: ffmpeg 逐帧像素输出
**最后更新**: 2026-02-24 (Session 3)

---

## 当前精度状态

| 测试范围            | Y精度      | U精度  | V精度  | 总体精度  |
| ------------------- | ---------- | ------ | ------ | --------- |
| 首帧 (frame 0, I帧) | **77.20%** | 99.90% | 99.80% | ~84.7%    |
| 全片 (299帧)        | -          | -      | -      | **84.7%** |
| 首帧 max_err        | 13         | 小     | 小     | -         |

关键观测:

- U/V 精度 >99.9%, 问题几乎完全在 Y 通道
- 首帧是纯 I 帧, Y=77.2% → I 帧内预测/变换/残差路径存在大量错误
- 关闭 deblock (TAO_SKIP_DEBLOCK=1): Y=66.15%, U=93.58%, V=91.77%
- 开启 deblock: Y=77.2%, U=99.9%, V=99.8% → deblock **有益**, 已排除为 bug 源

---

## 历次修复清单

### ✅ Session 1: 运动预测语法链路 (BUG-1..6)

#### CABAC 字节对齐位解析 bug

- **文件**: `crates/tao-codec/src/decoders/h264/cabac.rs`
- **状态**: 已修复 ✓

#### BUG-1 [CRITICAL]: P_Skip MV 推导 AND→OR 逻辑错误

- **文件**: `macroblock_inter.rs` — `predict_p_skip_mv`
- **修复**: 任一邻居不可用 OR 满足零条件即返回 (0,0)，由 AND 改为 OR 逻辑
- **附带**: zero-check 从 MB-level 改为 4x4-level (`l0_motion_candidate_4x4`)
- **状态**: 已修复 ✓

#### BUG-2/6 [HIGH]: MV 中值预测候选级联 unwrap_or 错误

- **文件**: `macroblock_inter_mv.rs` — `predict_mv_l0_partition` / `predict_mv_l1_partition`
- **修复**: 不可用候选统一 → `(0,0)`；仅 A 可用时直接返回 A
- **参考**: ffmpeg `h264_mvpred.h:226-277`
- **状态**: 已修复 ✓

#### BUG-3 [MEDIUM]: Spatial Direct 无邻居时错误回退到 Temporal Direct

- **文件**: `macroblock_inter.rs:468-481`
- **修复**: 无空间邻居时设 ref=0, mv=(0,0)，不递归 temporal
- **规范**: H.264 spec 8.4.1.2.2
- **状态**: 已修复 ✓

#### BUG-4 [MEDIUM]: MapColToList0 重建 DPB 而非 POC 匹配

- **文件**: `macroblock_inter.rs:402-421` + `output.rs`
- **修复**: `ReferencePicture` 新增 `ref_l0_poc: Vec<i32>`，用 POC 匹配
- **参考**: ffmpeg `h264_direct.c:82-137`
- **状态**: 已修复 ✓

#### BUG-5 [LOW-MEDIUM]: B slice 16x8/8x16 缺少 L1 方向性 MV 预测

- **文件**: `macroblock_inter_mv.rs`
- **修复**: 新增 `predict_mv_l1_16x8` / `predict_mv_l1_8x16`，接入 B slice 路径
- **状态**: 已修复 ✓

### ✅ Session 2: IDCT 变换顺序修复

#### IDCT 4x4 pass 顺序修复

- **问题**: 错误的"列→行→列"顺序
- **修复**: 改为正确的"行→列"两 pass
- **验证**: 代数等效 ffmpeg `h264_idct_dc_add`
- **状态**: 已修复 ✓

#### IDCT 8x8 pass 顺序修复

- **同上**, 8x8 版本
- **状态**: 已修复 ✓

### ✅ Session 3: I_8x8 topright 修复

#### I_8x8 block (1,1) has_topright 错误

- **文件**: `macroblock_intra.rs`
- **修复**: `(1, 1) => mb_right_avail` 改为 `(1, 1) => false`
- **依据**: ffmpeg `topright_samples_available = 0xEEEA` (bit 2 = 0)
- **精度影响**: 零 (Y 仍为 77.198688%)
- **结论**: 修复正确但不是 Y=22.8% 错误的根本原因
- **状态**: 已修复 ✓

---

## Session 3 深度代码验证结论

以下组件经过逐公式代码比对，确认与 ffmpeg/规范一致，**已排除为 bug 源**:

### IDCT 和反量化 (全部验证)

- `idct_4x4`: 行→列 pass，与 ffmpeg 等效 ✓
- `idct_8x8`: 同上 ✓
- 所有反量化路径: 代数等效 ✓
- Zigzag 扫描表: 与 OpenH264 参考匹配 ✓
- Hadamard DC 变换: 正确 ✓

### I_4x4 预测模式 (全部9个)

| Mode | 名称                      | 验证结论                                    |
| ---- | ------------------------- | ------------------------------------------- |
| 0    | Vertical                  | ✓                                           |
| 1    | Horizontal                | ✓                                           |
| 2    | DC                        | ✓ (所有可用性分支)                          |
| 3    | Diagonal Down-Left (DDL)  | ✓                                           |
| 4    | Diagonal Down-Right (DDR) | ✓                                           |
| 5    | Vertical-Right (VR)       | ✓                                           |
| 6    | Horizontal-Down (HD)      | ✓ (加法交换律: `filt(B,A,M) = filt(M,A,B)`) |
| 7    | Vertical-Left (VL)        | ✓                                           |
| 8    | Horizontal-Up (HU)        | ✓                                           |

### I_8x8 参考样本滤波 (`I8x8Refs::load`)

与 ffmpeg `PREDICT_8x8_LOAD_*` 宏逐项验证:

- `lt = (raw_l[0] + 2*raw_tl + raw_t[0] + 2) >> 2` ✓
- `l[0]` 有 topleft: `(raw_tl + 2*raw_l[0] + raw_l[1] + 2) >> 2` ✓
- `l[7] = (raw_l[6] + 3*raw_l[7] + 2) >> 2` ✓
- `t[8..15]` 无 topright: 设为 `raw_t[7]` (原始值，不滤波) ✓

### I_16x16 预测

- **Vertical/Horizontal/DC**: 标准公式 ✓
- **Plane** (`compute_plane_params` + `apply_plane_prediction`):
    - `H = Σk=1..8: k*(p[7+k][-1] - p[7-k][-1])` ✓
    - `V = Σk=1..8: k*(p[-1][7+k] - p[-1][7-k])` ✓
    - `a = 16*(p[15][-1] + p[-1][15])` ✓
    - `b = (5*H+32)>>6`, `c = (5*V+32)>>6` ✓
    - `val = (a + b*(dx-7) + c*(dy-7) + 16) >> 5` ✓ (与 ffmpeg `pred16x16_plane_compat` 完全一致)

### 模式语法处理

- `remap_i4x4_mode_for_unavailable`: TOP_MAP/LEFT_MAP 与 ffmpeg `ff_h264_check_intra4x4_pred_mode` 完全一致 ✓
- `predict_i4x4_block_with_tr_unavail_fix`: 修补列表 `(1,1)|(3,1)|(1,3)|(3,2)|(3,3)` 正确 ✓
- `i4x4_modes` 缓存初始化: 填充为 2 (DC)；I_16x16 MB 不调用 `set_i4x4_mode`，保持 2 ✓
- I_16x16 `decode_qp_delta`: 总是存在 (spec 规定 Intra_16x16 总含 mb_qp_delta) ✓

### 残差应用路径

- I_16x16 DC 路径: AC dequant (coeffs[0]=0) → Hadamard 求解 DC → 写入 coeffs[0] → IDCT ✓
- I_4x4 AC 路径: `apply_4x4_ac_residual` 对全 16 系数 (含 DC at scan pos 0) 正确应用 IDCT ✓

---

## 未解决问题: Y=22.8% 错误的根本原因

### 现状

- 经过 Session 3 系统性代码审查，所有可以通过静态代码比对验证的路径均已确认正确
- 无法通过进一步阅读代码定位剩余问题
- **必须转向实验/运行时追踪方法**

### 仍存在的嫌疑

#### 嫌疑 A: `remap_i8x8_mode_for_unavailable` 是 STUB

```rust
// crates/tao-codec/src/decoders/h264/intra.rs
pub(super) fn remap_i8x8_mode_for_unavailable(mode: u8, _top_available: bool, _left_available: bool) -> u8 {
    mode.min(8)  // BUG: 忽略 top_available/left_available 参数!
}
```

**预期影响**: 仅边界 MB (~1-2%), 不足以解释 22.8% 错误。
**修复方案**: 使用与 `remap_i4x4_mode_for_unavailable` 相同的 TOP_MAP/LEFT_MAP 逻辑。

#### 嫌疑 B: I_8x8 预测模式 3-8 未逐行验证

- I_4x4 的 9 个模式已全部验证
- I_8x8 用滤波后 8 个参考样本 (`I8x8Refs`), 且操作的是 8x8 块
- modes 3-8 (DDL/DDR/VR/HD/VL/HU) 的 8x8 版本**尚未**逐行与 ffmpeg 代码比对
- 特别是 DDL/DDR 等使用 `t[8]` 样本的模式，当 `has_topright=false` 时的边界处理

#### 嫌疑 C: MB 类型分布不明

- 不清楚 I 帧里有多少 I_4x4 vs I_8x8 vs I_16x16 MB
- 若大部分是 I_8x8 (`transform_size_8x8_flag=1`)，I_8x8 路径问题影响会很大

#### 嫌疑 D: CABAC 上下文累积偏差

- I 帧的 CABAC 上下文依赖重建像素值
- `coded_block_flag`, `significant_coeff_flag` 等上下文类别的初始化/更新有无误差？
- `mb_qp_delta` 的 CABAC 上下文更新路径？

#### 嫌疑 E: QP 计算链路

- `mb_qp_delta` 的解析和累加到 `mb_qp`
- `mb_qp` 如何影响反量化的 `qp_y`/`qp_c`
- 色度 QP 映射 (QP_c = chroma_qp_table[QP_y])

---

## 下一轮调查计划 (第4轮)

### Phase A: 经验性错误定位 [首先执行]

**目标**: 确定错误发生在哪些 MB 类型和帧区域

**方案 A1**: 逐 MB 精度统计
对首帧 Y 通道按 16x16 MB 块统计平均误差，确定错误集中区域：

- 若错误均匀分布 → 可能是系统性错误 (QP? DC偏移?)
- 若错误集中在特定区域 → 可能是特定模式问题

**方案 A2**: I_8x8 vs I_4x4 隔离测试
临时修改代码，强制所有 I_NxN MB 使用 I_4x4 路径 (忽略 `transform_size_8x8_flag`):

- 精度提升 → I_8x8 路径有问题
- 精度不变 → 问题在 I_4x4 路径或共同路径

**诊断命令**:

```bash
# 单帧精度基线
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=1 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=0 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
```

### Phase B: 修复 `remap_i8x8_mode_for_unavailable` stub

**优先级**: 中 (已知 bug，应修复)

```rust
// 正确实现参考 ff_h264_check_intra4x4_pred_mode
pub(super) fn remap_i8x8_mode_for_unavailable(mode: u8, top_available: bool, left_available: bool) -> u8 {
    // 与 remap_i4x4_mode_for_unavailable 相同的 TOP_MAP/LEFT_MAP 逻辑
    // 参考 data/tmp_ffmpeg_src/libavcodec/h264_parse.c:130-210
    let mode = remap_i4x4_mode_for_unavailable(mode, top_available, left_available);
    mode
}
```

验证：

```bash
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=1 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=0 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
```

### Phase C: I_8x8 预测模式逐行验证

对 `intra.rs` 中 I_8x8 预测的 modes 3-8 逐行与 ffmpeg `h264pred_template.c` 比对:

- `pred8x8l_diagonal_down_left`
- `pred8x8l_diagonal_down_right`
- `pred8x8l_vertical_right`
- `pred8x8l_horizontal_down`
- `pred8x8l_vertical_left`
- `pred8x8l_horizontal_up`

重点关注:

- `has_topright=false` 时 `t[8]` 的值
- 边界像素处理

### Phase D: QP 链路追踪

```bash
# 如果 CABAC trace 可用
TAO_H264_SLICE_TRACE=1 TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=1 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=0 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored 2>&1 | grep -i "qp\|delta" | head -100
```

### Phase E: 完整 CABAC trace 对比 [最后手段]

对首帧，在首个出现偏差的 MB 位置启用完整 trace:

```bash
TAO_H264_SLICE_TRACE=1 TAO_H264_SLICE_TRACE_MB=1 \
  TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=1 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=0 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored 2>&1 | head -500
```

---

## 全片验收门禁 (Y 修复后执行)

```bash
# 67帧门禁
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=67 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=0 TAO_H264_COMPARE_REPORT=1 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored

# 299帧全片验收
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=299 \
  TAO_H264_COMPARE_REQUIRED_PRECISION=100 TAO_H264_COMPARE_REPORT=1 \
  cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored

# 16样本全量回归
cargo test --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored
```

---

## 精度进展记录

| Session    | 首帧 Y | 首帧总体 | 全片 (299帧) | 主要修复                        |
| ---------- | ------ | -------- | ------------ | ------------------------------- |
| 基线       | ~59.4% | ~59.4%   | ~13.9%       | -                               |
| Session 1  | ~77%   | ~77%     | 大幅提升     | BUG-1..6, CABAC 字节对齐        |
| Session 2  | 77.2%  | ~84.7%   | 84.7%        | IDCT 4x4/8x8 pass 顺序          |
| Session 3  | 77.2%  | ~84.7%   | 84.7%        | I_8x8 topright fix (精度无变化) |
| 下一轮目标 | >95%   | >95%     | >95%         | 待 Phase A-E 定位 I 帧 Y 错误源 |

---

## 关键文件清单

| 文件                                                        | 相关内容                                                                             |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `crates/tao-codec/src/decoders/h264/intra.rs`               | I_4x4/I_8x8 所有预测模式, I8x8Refs::load, remap_i8x8_mode_for_unavailable (STUB)     |
| `crates/tao-codec/src/decoders/h264/macroblock_intra.rs`    | MB 解码入口, remap_i4x4_mode_for_unavailable, predict_i4x4_block_with_tr_unavail_fix |
| `crates/tao-codec/src/decoders/h264/residual.rs`            | 反量化, IDCT 应用, I_16x16 DC 路径                                                   |
| `crates/tao-codec/src/decoders/h264/macroblock_inter.rs`    | P_Skip MV, Spatial Direct, MapColToList0                                             |
| `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs` | MV 中值预测, L1 方向性预测                                                           |
| `crates/tao-codec/src/decoders/h264/output.rs`              | ReferencePicture, DPB 管理                                                           |

## 参考源码

| 模块             | ffmpeg 文件                                       | openh264 文件                                                   |
| ---------------- | ------------------------------------------------- | --------------------------------------------------------------- |
| I_4x4/I_8x8 预测 | `tmp_ffmpeg_src/libavcodec/h264pred_template.c`   | -                                                               |
| 模式可用性重映射 | `tmp_ffmpeg_src/libavcodec/h264_parse.c:130-210`  | -                                                               |
| P_Skip MV        | `tmp_ffmpeg_src/libavcodec/h264_mvpred.h:388-485` | `tmp_openh264_src/codec/decoder/core/src/mv_pred.cpp:706-752`   |
| MV 中值预测      | `tmp_ffmpeg_src/libavcodec/h264_mvpred.h:226-277` | `tmp_openh264_src/codec/decoder/core/src/mv_pred.cpp:706-752`   |
| B Direct spatial | `tmp_ffmpeg_src/libavcodec/h264_direct.c:140-600` | `tmp_openh264_src/codec/decoder/core/src/mv_pred.cpp:392-703`   |
| MapColToList0    | `tmp_ffmpeg_src/libavcodec/h264_direct.c:82-137`  | `tmp_openh264_src/codec/decoder/core/src/mv_pred.cpp:1158-1174` |

---

## 附: 已确认不是 Bug 的代码 (勿重复验证)

1. IDCT 4x4/8x8: 代数等效 ffmpeg ✓
2. 所有反量化路径: 等效 ffmpeg ✓
3. I_4x4 全部9个预测模式公式: 逐行比对 ffmpeg ✓
4. I8x8Refs::load 滤波: 与 ffmpeg PREDICT*8x8_LOAD*\* 宏等效 ✓
5. I_16x16 Plane prediction: compute_plane_params 与 ffmpeg pred16x16_plane_compat 等效 ✓
6. remap_i4x4_mode_for_unavailable: TOP_MAP/LEFT_MAP 与 ffmpeg 完全一致 ✓
7. predict_i4x4_block_with_tr_unavail_fix: 修补列表正确 ✓
8. i4x4_modes 缓存初始化: 填充为 2 (DC), 正确 ✓
9. I_16x16 mb_qp_delta 总是存在: 正确 (规范要求) ✓
10. Deblocking filter: 有益不有害, 已排除为 bug 源 ✓
11. I_16x16 残差 DC 路径: AC dequant → Hadamard → IDCT 顺序正确 ✓
