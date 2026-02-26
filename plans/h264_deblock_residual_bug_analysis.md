# H.264 去块滤波与残差处理 Bug 分析

## 背景与目标

解码器存在 B 帧精度问题: max_err=30, 散布模式. 本文档对 `deblock.rs` 和 `residual.rs`
进行逐项对照 H.264 规范(ITU-T H.264 Section 8.5 / 8.7)和 FFmpeg 参考实现的分析.

---

## 一、去块滤波 (deblock.rs) 问题清单

### Bug 1: 色度 QP 推导顺序错误 (行 390-396)

**严重程度**: 中 — 在相邻宏块 QP 不同时产生像素误差.

**当前代码**:
```rust
let edge_qp = if let Some(offset) = chroma_qp_remap_offset {
    let qpc_p = chroma_qp_from_luma_with_offset(qp_p, offset);
    let qpc_q = chroma_qp_from_luma_with_offset(qp_q, offset);
    (qpc_p + qpc_q + 1) >> 1
} else {
    (qp_p + qp_q + 1) >> 1
};
```

**问题**: 先对每个宏块的亮度 QP 独立映射为色度 QP, 再取平均值.
H.264 规范 8.7.2.1 要求: 先平均亮度 QP, 再映射一次.

**规范公式**:
1. `QPY = (QPp + QPq + 1) >> 1`
2. `qPi = Clip3(0, 51, QPY + chroma_qp_index_offset)`
3. `QPC = QpC_table[qPi]`

色度 QP 映射表在 QP > 30 时为非线性(饱和), 因此两种顺序产生不同结果.
例如: QPp=32, QPq=36, offset=0:
- 规范: QPY=34 → QpC(34)=33
- 当前: QpC(32)=32, QpC(36)=34 → avg=33 (恰好相同)
- QPp=30, QPq=36: 规范: QPY=33 → QpC(33)=32
  当前: QpC(30)=29, QpC(36)=34 → avg=32 (恰好相同)
- QPp=29, QPq=39: 规范: QPY=34 → QpC(34)=33
  当前: QpC(29)=29, QpC(39)=35 → avg=32 (**差 1**)

**注意**: FFmpeg 的 `h264_filter_mb` 也使用了与本代码相同的"先映射后平均"方式.
因此此差异实际对齐 FFmpeg, 但偏离严格规范. 在 QP 变化剧烈的 B 帧边界处可能引入
少量像素误差(典型 1-2).

**修复方案**:
```rust
let edge_qp = if let Some(offset) = chroma_qp_remap_offset {
    let qp_avg = (qp_p + qp_q + 1) >> 1;
    chroma_qp_from_luma_with_offset(qp_avg, offset)
} else {
    (qp_p + qp_q + 1) >> 1
};
```

---

### Bug 2: B 帧 BS 计算的交叉匹配逻辑不完整 (行 883-914)

**严重程度**: 中 — 在特定 B 帧双向/单向预测混合边界可能产生错误 BS.

**当前代码** (`combine_motion_list_mismatch`):
当直接匹配(L0_a↔L0_b, L1_a↔L1_b)不一致时, 只尝试交叉匹配
(L0_a↔L1_b, L1_a↔L0_b). 如果任一列表数据为 None(而非 ref_idx=-1),
函数可能错误地返回 BS=0.

**具体场景**: 当一个块使用单向 L0 预测(L1 为 None)而另一个块使用双向预测:
- `list0 = Some(false)` (L0 匹配)
- `list1 = None` (L1 数据不可用)
- 结果: `list0.unwrap_or(false) || list1.unwrap_or(false)` = false
- 进入 `list0.is_some() || list1.is_some()` = true → 返回 BS=0
- 但规范要求: 参考图像数量不同时应返回 BS=1

**修复方案**: 在 `combine_motion_list_mismatch` 中增加对不对称预测的检测.

---

### 已验证正确的部分

- **Alpha/Beta/TC0 阈值表** (行 974-999): 与 H.264 Table 8-16/8-17 完全一致. ✓
- **强滤波 BS=4 亮度** (行 433-487): 蝶形运算和条件判断与规范 8.7.2.3 一致. ✓
- **强滤波 BS=4 色度** (行 489-493): 仅更新 p0/q0, 符合 4:2:0 规范. ✓
- **弱滤波 p1/q1 更新** (行 530-539): delta_p1 公式和 tc0 截断正确. ✓
- **色度弱滤波 tc = tc0 + 1** (行 498-504): 正确. ✓
- **MB 边界 BS=4 判断** (行 627-628): is_intra_mb 检查正确. ✓
- **8x8 变换模式内部边界 BS=0** (行 676-683): 正确跳过非 8x8 对齐位置. ✓
- **跨 slice 边界禁止滤波** (行 823-837): idc=2 处理正确. ✓
- **idc=1 完全禁止滤波**: 在 output.rs 调用侧已处理(行 818). ✓

---

## 二、残差处理 (residual.rs) 问题清单

### Bug 3: CABAC 旁路后缀解码位消耗顺序错误 (行 252)

**严重程度**: 低 — 仅在极端大系数(level ≥ 8388623)时触发, 实际视频中几乎不会出现.

**当前代码**:
```rust
while cabac.decode_bypass() == 1 && j < 23 {
    j += 1;
}
```

**问题**: Rust 的 `&&` 短路求值从左到右. 当 `j = 23` 时, 先调用 `decode_bypass()`
消耗一个比特, 然后检查 `j < 23` 失败退出. 这比 FFmpeg 多消耗了一个旁路比特.

FFmpeg 的等效代码:
```c
for(j=0; j<16+7 && get_cabac_bypass(c); j++);
```
C 语言中 `j < 23` 先求值, 当 j=23 时短路不调用 bypass.

**影响**: 当系数绝对值极大时(需要 j=23 的 Exp-Golomb 后缀), 会多消耗一个比特,
导致后续比特流解同步. 但这种系数值在正常视频中不会出现.

**修复方案**:
```rust
while j < 23 && cabac.decode_bypass() == 1 {
    j += 1;
}
```

---

### 已验证正确的部分

- **4x4 IDCT 蝶形运算** (行 400-428): 与规范 8.5.12.1 完全一致, 包括 >>1 移位和
  +32 舍入偏置. ✓
- **8x8 IDCT 蝶形运算** (行 462-530): 奇偶分解、a1-a7/b1-b7 公式与规范 8.5.12.2
  完全一致. ✓
- **4x4 Luma DC 逆 Hadamard** (行 268-295): 行列蝶形变换正确. ✓
- **2x2 Chroma DC 逆 Hadamard** (行 299-308): 4 个输出公式正确. ✓
- **4x4 AC 反量化** (行 358-380): 阈值 qp_per>=4, 舍入 `1<<(shift-1)` 正确. ✓
- **8x8 AC 反量化** (行 433-458): 阈值 qp_per>=6, 舍入正确. ✓
- **Luma DC 反量化** (行 322-335): 与 FFmpeg `luma_dc_dequant_idct` 等效, 阈值
  qp_per>=6 正确(已融合 IDCT >>6 缩放). ✓
- **Chroma DC 反量化** (行 342-355): 阈值 qp_per>=5, 无舍入(纯截断)正确. ✓
- **4x4 Zigzag 扫描** (行 537-554): 与 H.264 Table 8-13 帧扫描完全一致. ✓
- **8x8 Zigzag 扫描** (行 557-561): 正确. ✓
- **LevelScale 表** (行 569-576): 与 H.264 Table 8-14 一致. ✓
- **LevelScale_8x8 表** (行 582-589): 与 H.264 Table 8-16 一致. ✓
- **8x8 反量化位置映射** (行 592-595): V[0]-V[5] 的 8x8 周期模式正确. ✓
- **显著性上下文映射表** (行 598-608): SIG/LAST 偏移正确. ✓
- **系数解码节点状态机** (行 219-222): TRANS_EQ1/TRANS_GT1 与 FFmpeg 一致. ✓
- **像素值最终截断** (行 635, 659, 692, 715): clamp(0, 255) 正确. ✓

---

## 三、与 B 帧测试失败的关系

当前 8 个 B 帧测试失败(decode_b/prediction 模块)的根因位于
`macroblock_inter*.rs`(运动预测逻辑), 而非 deblock.rs 或 residual.rs:
- 多个 off-by-1 误差: B_Direct/B_Skip 运动向量预测未正确使用邻居 MV
- col_zero 分支未命中: temporal direct 的 list1 回退逻辑存在条件判断问题

上述 deblock/residual 的修复可以改善最终像素精度(特别是 Bug 1 在 QP 变化的边界处),
但不会直接修复这 8 个测试失败.

---

## 四、修复优先级

| 优先级 | Bug | 预计影响 |
|--------|-----|----------|
| P1 | Bug 1: 色度 QP 推导顺序 | 在 QP 变化边界处可能引入 1-3 像素误差 |
| P1 | Bug 2: BS 交叉匹配不完整 | 在 B 帧混合预测边界可能错误跳过滤波 |
| P2 | Bug 3: 旁路后缀解码位顺序 | 仅极端情况触发, 实际风险极低 |
