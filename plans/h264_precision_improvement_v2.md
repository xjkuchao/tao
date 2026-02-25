# H.264 精度提升计划 v2 - P帧质量修复

## 背景与现状

**当前精度基线**（data/1_h264.mp4）：
- 100帧测试: **19.11%** precision
- 300帧测试: **16.99%** precision
- I帧精度: **100%**（完美）
- P帧精度: **~16-19%**（严重降审）
- B帧精度: **3-30%**（连锁降级）

**问题特征**:
- I帧解码完美，说明基础的DCT、量化、反量化、滤波都正确
- P帧首帧（帧1）就开始不匹配，说明第一次运动补偿即出错
- 精度从帧1: ~25% 持续降低到末帧: ~5%，表明参考帧质量逐帧恶化
- 这是典型的"参考帧污染"问题

## 已验证无关因素（锁定为正确实现）

### ✅ Stride处理机制（已验证）
- **测试内容**: 完整添加RefPlanes/ReferencePicture stride字段，修改所有MC调用
- **结果**: P299从18.11%回归到17.14%，说明不是根因
- **结论**: 每帧固定stride实现是正确的，不是问题

### ✅ Chroma坐标计算（已验证）
- **位置**: `macroblock_inter_mv.rs:336` - `c_src_y = c_dst_y + floor_div(mv_y_qpel, 8)`
- **验证**: 该行使用mv_y_qpel（不是mv_x_qpel），与FFmpeg一致
- **关联验证**: blend_inter_block等其他MC函数也都正确
- **结论**: QPEL到像素坐标映射无误

### ✅ DPB B帧处理（已修复）
- **历史**: R-BFRAME-REORDER commit已修复B帧在DPB中的释放顺序
- **验证**: X2样本（B帧）精度从低升至85.95%，证明修复有效
- **结论**: DPB实现已正确

## 真正的根因分析

### 可疑点 #1: MV预测（suspect.mvpred）

**代码位置**: `macroblock_inter_mv.rs:50-150` （predict_mv_l0_partition函数）

**为什么怀疑**:
1. P帧从帧1就开始不匹配→说明MV计算或应用即出错
2. 精度逐帧恶化→参考帧质量都被污染了
3. 如果MV预测错误，所有后续帧都会使用错误的参考→连锁反应

**需要检查**:
```rust
fn predict_mv_l0_partition(
    ctx: &mut MacroblockContext,
    x: usize, y: usize,
    w: u32, h: u32,
    ref_idx: u8,
) -> Mv
```

关键验证点:
- MV预测选择A/B/C候选时，ref_idx匹配逻辑是否正确？
- 零MV特殊处理是否正确？
- MV中值计算的舍入方向是否符合标准？

**对比参考**: FFmpeg `h264.c` 中的 `map_pred_16x16_ref()` 和 `mv_pred()` 函数

---

### 可疑点 #2: L0/L1参考列表构建（suspect.reflist）

**代码位置**: `output.rs:240+` （collect_default_reference_list_l0函数）

**为什么怀疑**:
1. 参考列表顺序错误→MV预测候选选错→后续帧都使用错误参考
2. Ref_idx 到参考帧的映射错误→MC采样错误帧

**需要检查**:
```rust
pub fn collect_default_reference_list_l0(dpb: &Dpb, ...) -> Vec<u32>
```

关键验证点:
- 短期参考和长期参考的排序规则？
- 最大参考帧数限制（MaxNumRefFrames）的应用？
- POC排序是否正确？

**对比参考**: FFmpeg `h264_refs.c` 中的 `ff_h264_build_ref_list()` 函数

---

### 可疑点 #3: 参考帧缓冲初始化(suspect.refbuf)

**代码位置**: `output.rs:206-250` （reference_to_planes函数）

**为什么怀疑**:
1. 如果参考帧指针指向错误的DPB项→MC采样错误数据
2. stride或尺寸不匹配→随机访问越界或采样错位

**需要检查**:
```rust
fn reference_to_planes(...) -> Result<RefPlanes, TaoError>
```

关键验证点:
- ref_idx到DPB索引的映射是否正确？
- 参考帧的YUV平面指针是否有效？
- stride是否与实际帧宽度匹配？
- POC匹配逻辑是否正确？

---

### 可疑点 #4: 运动补偿参数（suspect.mcparams）

**代码位置**: `macroblock_inter_mv.rs:272-342` （apply_inter_block函数）

**为什么怀疑**:
1. 分区宽高参数（part_w, part_h）错误→MC采样范围错误
2. 帧内/帧间分量混淆→采样错误数据
3. 边界处理（src_w, src_h）不匹配实际帧尺寸

**需要检查**:
```rust
fn apply_inter_block(
    output: &mut [u8],
    ref_frame: &[u8],
    width: u32,
    src_x: i32,
    src_y: i32,
    dst_x: u32, dst_y: u32,
    part_w: u32, part_h: u32,
    qpel_x: u16, qpel_y: u16,
    ref_stride: usize,
    stride: usize,
) -> Result<(), TaoError>
```

关键参数验证:
- src_x, src_y计算是否防止越界？
- part_w, part_h是否与MB/sub-partition大小一致？
- qpel_x, qpel_y是否在[0,3]范围内？
- stride参数计算是否匹配？

---

## 调查方法 - 使用FFmpeg源代码对比

**不采用诊断式修改** - 以免再次引入回归

**策略**: 建立变量对标系统，通过日志打印关键变量，逐段与FFmpeg对比

### 第1阶段: MV预测验证

1. 在 P帧第1个宏块处添加日志:
   ```rust
   eprintln!("MB[0,0] MV predict:");
   eprintln!("  Candidates: A={:?}, B={:?}, C={:?}", mv_a, mv_b, mv_c);
   eprintln!("  Median MV: {:?}", predicted_mv);
   eprintln!("  Frame ref_idx={}, L0[{}]={:?}", ref_idx, ref_idx, ref_frame_info);
   ```

2. 用 FFmpeg decode 同一帧，比较 MV 预测结果

3. 如果 MV 匹配 ✓，进入第2阶段
4. 如果 MV 不匹配 ✗，修复 predict_mv_l0_partition 逻辑

### 第2阶段: 参考列表验证

1. 在集合参考列表时添加日志:
   ```rust
   eprintln!("RefList L0: {:?}", l0_list);
   eprintln!("POC order: short={:?}, long={:?}", short_term_refs, long_term_refs);
   ```

2. 与 FFmpeg 参考列表对比

3. 如果列表正确 ✓，进入第3阶段
4. 如果列表错误 ✗，修复参考列表排序

### 第3阶段: MC参数&应用验证

1. P帧第1个P分区起始处:
   ```rust
   eprintln!("MC params: ref_frame={:?}, mv=({},{})", ref_frame, mv.x, mv.y);
   eprintln!("  src_xy=({},{}), dst_xy=({},{})", src_x, src_y, dst_x, dst_y);
   eprintln!("  ref_frame[0][:8] = {:?}", &ref_frame[..8]);
   eprintln!("  output_before[0][:8] = {:?}", &output_before[..8]);
   ```

2. MC应用后对比输出数据与FFmpeg

3. 定位第一个不匹配的像素位置与值

---

## 执行步骤

### 步骤 1: 建立对比基准
- 获取 data/1_h264.mp4 第 1 个 P 帧 的 FFmpeg 解码结果（参考）
- 获取 Tao 解码结果（当前实现）
- 逐个像素对比，找出第一个不匹配位置

**预期输出**: 第一个错误位置的坐标与值

```bash
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 cargo test --release --test run_decoder h264_compare_sample_1 -- --nocapture --ignored 2>&1 | grep -A 20 "首个不一致帧"
```

### 步骤 2: MV预测调试
- [ ] 修改 predict_mv_l0_partition，添加详细日志
- [ ] 比较first_p_frame（帧1）的MV预测输出
- [ ] 验证与FFmpeg计算结果一致性
- [ ] 如有偏差，修正逻辑

### 步骤 3: 参考列表验证
- [ ] 修改 collect_default_reference_list_l0，打印参考列表
- [ ] 验证POC排序与FFmpeg一致
- [ ] 确认ref_idx映射正确

### 步骤 4: MC参数跟踪
- [ ] 在 apply_inter_block 入口添加参数校验日志
- [ ] 验证 src_xy, dst_xy, stride, part_w/part_h
- [ ] 比对采样点与refr_frame的实际数据

### 步骤 5: 精度验证
- [ ] 修复识别出的根因
- [ ] 执行测试: precision should ≈ 100% or ≥ 90%
- [ ] 逐帧精度应稳定在高水平（≠逐帧下降）

---

## 验收标准

**主要目标**: 将 P 帧精度从 **~17%** 提升到 **≥ 95%**

**分阶段目标**:
1. 识别并修复第一个根因 → 精度 ≥ 50%
2. 识别并修复第二个根因 → 精度 ≥ 80%
3. 完全修复 → 精度 ≥ 95% （保留1-5%误差余地）

**测试覆盖**:
- Primary: 100 frame test on data/1_h264.mp4 (current baseline)
- Regression: All 16 built-in samples (e0-e9, X1-X4)
- Stretch goal: 300 frame test (smooth precision curve)

---

## 附录: 关键代码文件映射

| 功能 | 文件 | 行数 |
|------|------|------|
| MV预测 | macroblock_inter_mv.rs | 50-150 |
| MC应用(单ref) | macroblock_inter_mv.rs | 272-342 |
| MC应用(双ref) | macroblock_inter_mv.rs | 493-520 |
| 参考列表 | output.rs | 240-280 |
| 参考帧平面 | output.rs | 206-250 |
| 缓冲管理 | output.rs | 120-200 |

---

## 时间估算

- 步骤1（基准建立）: 30 min
- 步骤2（MV调试）: 1.5-2 hours
- 步骤3（参考列表）: 1 hour
- 步骤4（MC跟踪）: 1.5 hours
- 步骤5（修复&验证）: 1-2 hours

**总计**: 5.5-7.5 hours (assuming no major issues found)

制定日期: 2026-02-25
预期完成日期: 2026-02-25 或后续

