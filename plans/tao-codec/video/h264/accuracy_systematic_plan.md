# H264 解码器精度系统化功能对比与迭代修复计划 (v2)

## 概述

对 Tao H264 解码器按功能模块进行系统化拆分, 逐子功能点与 FFmpeg/OpenH264/VLC/H.264 规范 (ITU-T H.264 第 7/8/9 章) 对比, 发现差异即修复并验证, 循环迭代直至精度达标。

## 精度目标

- 首要目标: `data/1.mp4` (已 100% bit-exact, 守护不回归) 和 `data/2.mp4` (当前 78.7%, 目标 100%)。
- 次要目标: C1-C3, E1-E9, X1-X4 全部达到阶段 A (PSNR >= 50dB, 精度 >= 99%, max_err <= 2)。
- 最终目标: 全样本 bit-exact (精度 = 100%, max_err = 0)。

## 当前精度基线 (实测)

```text
┌──────────────────────┬──────────┬────────────────┬─────────────┬────────────────────┐
│ 样本                 │ 精度     │ 熵编码         │ 关键特征    │ 状态               │
├──────────────────────┼──────────┼────────────────┼─────────────┼────────────────────┤
│ data/1.mp4           │ 100.000% │ CABAC High     │ 1080p       │ ✅ bit-exact       │
│ data/2.mp4           │ 78.724%  │ CABAC Main+B帧 │ B帧级联误差 │ ❌ 首要修复目标    │
│ C1                   │ 14.307%  │ CAVLC Baseline │ 720p        │ ❌ CAVLC系统性问题 │
│ C2                   │ 99.999%  │ CABAC Main     │ 1080p B帧   │ ✅ 近bit-exact     │
│ X1                   │ 100%     │ CABAC I-only   │ 纯帧内      │ ✅ bit-exact       │
│ X2                   │ 91.324%  │ CABAC High     │ B帧         │ ❌ B帧误差         │
│ X3                   │ 99.703%  │ CABAC High     │ P-only      │ ⚠️ P帧微漂移       │
│ CAVLC组(C1/E1/E7/E9) │ 10-22%   │ CAVLC          │ 多种        │ ❌ CAVLC独立根因   │
└──────────────────────┴──────────┴────────────────┴─────────────┴────────────────────┘
```

## 核心执行流程

1. 第一步: 功能大块划分 (8 大模块, 约 50 个子功能点)。
2. 第二步: 对每个子功能点 (按优先级顺序)。
    - 仔细对比 Tao 实现 vs FFmpeg/OpenH264/VLC/H.264 规范。
    - 如无差异: 标记 "已确认正确", 进入下一个子功能点。
    - 如有差异: 进入修复流程。
3. 第三步: 修复问题, 运行精度对比测试。
    - 快速验证: `data/1.mp4` + `data/2.mp4` (3 帧)。
    - 标准验证: 10 帧。
    - 完整验证: 67 帧/299 帧。
4. 第四步: 判断是否有效修复 (见判定规则)。
    - 有效: 提交代码, 进入下一个子功能点。
5. 第五步: 无效修复。
    - 恢复代码, 进入下一个子功能点。
6. 第六步: 移至下一个子功能点, 回到第二步。

全部子功能点循环完成后:

- 精度达标: 结束。
- 精度未达标: 重新拆分功能块 (更细粒度), 再次循环。

## 有效修复判定规则 (三条规则, 按优先级)

- 规则 1.1 (逻辑正确性优先): 对比 H.264 规范或 FFmpeg/OpenH264/VLC 实现, 反复确认是 Tao 实现错误的, 即使精度大幅下降也视为有效修复, 因为这说明修复了一个真实 bug, 只是暴露了更多下游问题。
- 规则 1.2 (精度指标辅助): 精度大幅提升 -> 有效; 精度大幅下降且无法从逻辑上确认正确性 -> 无效。
- 规则 1.3 (连锁修复感知): 某些修复需配合其他修复点才能生效 (例如修复了 CABAC 上下文但帧间预测也有 bug), 应从更大范围判断逻辑正确性, 必要时临时保留修复并在后续修复中验证联动效果。

## 功能模块大块划分 (8 大模块)

### 模块 0: 熵解码 (CABAC/CAVLC) - 最高优先级

- 原理: 熵解码是最上游环节, 一旦解析错误, 后续所有数据全部错误。

#### Phase A: CABAC 路径 (影响 `data/2.mp4` 和大部分 CABAC 样本)

```text
┌──────────────────────────────┬─────┬──────────────────────────────┬────────────────────────────┬──────────────────────────────────────┐
│ 子功能                       │ ID  │ 对比重点                     │ Tao 文件                   │ FFmpeg 参考                          │
├──────────────────────────────┼─────┼──────────────────────────────┼────────────────────────────┼──────────────────────────────────────┤
│ CABAC ref*idx 解码上下文     │ 0-1 │ ctxIdxInc 计算, 越界处理      │ macroblock_inter.rs        │ h264_cabac.c:decode_cabac_mb_ref     │
│ CABAC mvd 上下文选择 (amvd)  │ 0-2 │ 邻居 MVD 绝对值之和分级       │ macroblock_inter_weight.rs │ h264_cabac.c:decode_cabac_mb_mvd     │
│ CABAC cbp 解码               │ 0-3 │ luma(4bit)+chroma(2bit), 邻居上下文 │ slice_decode.rs      │ h264_cabac.c:decode_cabac_mb_cbp_luma/chroma │
│ CABAC 残差 (sig/last/coeff_abs) │ 0-4 │ ctxBlockCat, 扫描顺序, 上下文映射 │ residual.rs          │ h264_cabac.c:decode_cabac_residual   │
│ CABAC mb_skip / end_of_slice │ 0-5 │ 终止条件, skip 上下文偏移     │ slice_decode.rs            │ h264_cabac.c:decode_cabac_mb_skip    │
│ CABAC mb_type / sub_mb_type  │ 0-6 │ P/B 全类型映射               │ slice_decode.rs            │ h264_cabac.c:decode_cabac_mb_type    │
│ CABAC transform_size_8x8_flag│ 0-7 │ 解析时序 (CBP 前/后)         │ slice_decode.rs            │ h264_cabac.c                         │
└──────────────────────────────┴─────┴──────────────────────────────┴────────────────────────────┴──────────────────────────────────────┘
```

#### Phase B: CAVLC 路径 (影响 C1/E1/E7/E9 等所有 CAVLC 样本)

```text
┌────────────────────────────────────┬──────┬──────────────────────────────┬─────────────┬──────────────────────────────────┐
│ 子功能                             │ ID   │ 对比重点                     │ Tao 文件    │ FFmpeg 参考                      │
├────────────────────────────────────┼──────┼──────────────────────────────┼─────────────┼──────────────────────────────────┤
│ CAVLC coeff_token nC 上下文        │ 0-8  │ nC 计算规则, slice 边界感知   │ cavlc.rs    │ h264_cavlc.c:pred_non_zero_count │
│ CAVLC level/total_zeros/run_before │ 0-9  │ VLC 查表正确性, 边界处理      │ cavlc.rs    │ h264_cavlc.c                     │
│ CAVLC mb_type/ref_idx/mvd          │ 0-10 │ Exp-Golomb 映射, P/B 类型表   │ cavlc_mb.rs │ h264_cavlc.c                     │
│ CAVLC cbp 映射表                   │ 0-11 │ Intra/Inter 两套 VLC 映射     │ cavlc_mb.rs │ h264_cavlc.c                     │
└────────────────────────────────────┴──────┴──────────────────────────────┴─────────────┴──────────────────────────────────┘
```

### 模块 1: 参考帧管理

```text
┌──────────────────────────────┬─────┬──────────────────────────────┬────────────────────────┬──────────────────────────────────────────┐
│ 子功能                       │ ID  │ 对比重点                     │ Tao 文件               │ FFmpeg 参考                              │
├──────────────────────────────┼─────┼──────────────────────────────┼────────────────────────┼──────────────────────────────────────────┤
│ L0/L1 默认列表构建           │ 1-1 │ POC 排序规则, 长期/短期分组   │ output.rs              │ h264_refs.c:ff_h264_fill_default_ref_list │
│ 参考列表重排序 (modification)│ 1-2 │ ref_pic_list_modification 语义 │ slice_parse.rs, output.rs │ h264_refs.c:ff_h264_ref_list_modification │
│ MapColToList0 (temporal direct) │ 1-3 │ 共定位帧 POC 映射          │ macroblock_inter.rs    │ h264_direct.c:ff_h264_direct_ref_list_init │
│ DPB 滑动窗口 + MMCO          │ 1-4 │ 淘汰策略, op1-op6            │ output.rs              │ h264_refs.c                              │
│ 长期参考帧                   │ 1-5 │ MMCO op3/op6                 │ output.rs              │ h264_refs.c                              │
└──────────────────────────────┴─────┴──────────────────────────────┴────────────────────────┴──────────────────────────────────────────┘
```

### 模块 2: 帧间预测 / 运动向量预测

```text
┌───────────────────────────────┬─────┬────────────────────────────────────┬─────────────────────────┬────────────────────────────────────────────┐
│ 子功能                        │ ID  │ 对比重点                           │ Tao 文件                │ FFmpeg 参考                                │
├───────────────────────────────┼─────┼────────────────────────────────────┼─────────────────────────┼────────────────────────────────────────────┤
│ MV 中值预测 (A/B/C/D)         │ 2-1 │ 候选选择, 仅 A 可用时直接复用      │ macroblock_inter_mv.rs  │ h264_mvpred.h:pred_motion                  │
│ 邻居可用性 (slice 边界)       │ 2-2 │ same_slice_4x4 判定               │ macroblock_inter.rs     │ h264_mvpred.h                              │
│ P_Skip MV 推导                │ 2-3 │ 零 MV 条件, 邻居取值              │ macroblock_inter.rs     │ h264_mvpred.h:fill_decode_caches           │
│ B_Direct spatial 模式         │ 2-4 │ ref_idx 推导, MV 候选, 零 MV 判定  │ macroblock_inter.rs     │ h264_direct.c:pred_spatial_direct_motion   │
│ B_Direct temporal 模式        │ 2-5 │ dist_scale_factor, MV 缩放         │ macroblock_inter.rs     │ h264_direct.c:pred_temp_direct_motion      │
│ col_zero_flag 判定            │ 2-6 │ 共定位 L0/L1 条件优先级           │ macroblock_inter.rs     │ h264_direct.c                              │
│ 16x8/8x16 方向性 MVP          │ 2-7 │ 上/左/对角邻居特殊规则            │ macroblock_inter_mv.rs  │ h264_mvpred.h:pred_16x8_motion             │
│ B_Direct_8x8 子分区           │ 2-8 │ 每 8x8 独立推导                   │ macroblock_inter.rs     │ h264_direct.c                              │
│ MVD + MVP 合成                │ 2-9 │ 逐分区合成, 存储到 4x4 网格       │ macroblock_inter_mv.rs  │ h264_mvpred.h                              │
└───────────────────────────────┴─────┴────────────────────────────────────┴─────────────────────────┴────────────────────────────────────────────┘
```

### 模块 3: 运动补偿 (插值)

```text
┌─────────────────────────────┬─────┬───────────────────────────────────────┬────────────────────────────┬──────────────────────────┐
│ 子功能                      │ ID  │ 对比重点                              │ Tao 文件                   │ FFmpeg 参考              │
├─────────────────────────────┼─────┼───────────────────────────────────────┼────────────────────────────┼──────────────────────────┤
│ Luma qpel 6-tap (16 种位置) │ 3-1 │ 半像素/四分之一像素插值公式           │ common.rs                  │ h264qpel_template.c      │
│ 加权预测 (显式/隐式/默认)   │ 3-2 │ 公式对齐, 舍入, log2_weight_denom     │ macroblock_inter_weight.rs │ h264_weight_template.c   │
│ Chroma 双线性 (1/8 精度)    │ 3-3 │ 插值公式, 舍入                        │ common.rs                  │ h264chroma_template.c    │
│ 双向融合 (Bi-prediction)    │ 3-4 │ (L0 + L1 + 1) >> 1 舍入规则           │ macroblock_inter_cache.rs  │ h264_mc_template.c       │
│ 边界扩展 (padding)          │ 3-5 │ 参考块越界时像素复制策略              │ common.rs                  │ videodsp_template.c      │
└─────────────────────────────┴─────┴───────────────────────────────────────┴────────────────────────────┴──────────────────────────┘
```

### 模块 4: 残差 / 变换 / 量化

```text
┌────────────────────────────┬─────┬──────────────────────────────────────┬─────────────┬────────────────────────────────────────────┐
│ 子功能                     │ ID  │ 对比重点                             │ Tao 文件    │ FFmpeg 参考                                │
├────────────────────────────┼─────┼──────────────────────────────────────┼─────────────┼────────────────────────────────────────────┤
│ 4x4 IDCT 精度              │ 4-1 │ 变换矩阵, 舍入, 裁剪                 │ residual.rs │ h264idct_template.c:ff_h264_idct_add       │
│ 8x8 IDCT 精度              │ 4-2 │ High profile 8x8 变换矩阵            │ residual.rs │ h264idct_template.c:ff_h264_idct8_add      │
│ Chroma DC Hadamard         │ 4-3 │ 2x2 逆 Hadamard + 舍入偏移           │ residual.rs │ h264idct_template.c                         │
│ 反量化 + 缩放矩阵          │ 4-4 │ flat/custom scaling list, QP 分级     │ residual.rs │ h264_idct_template.c                        │
│ 残差裁剪 (0-255)           │ 4-5 │ 重建像素溢出处理                     │ residual.rs │ -                                          │
│ 扫描顺序 (zig-zag / 8x8)   │ 4-6 │ 4x4/8x8 扫描表                       │ residual.rs │ h264_scan.h                                │
└────────────────────────────┴─────┴──────────────────────────────────────┴─────────────┴────────────────────────────────────────────┘
```

### 模块 5: 去块滤波

```text
┌──────────────────┬─────┬────────────────────────────┬────────────┬───────────────────────────┐
│ 子功能           │ ID  │ 对比重点                   │ Tao 文件   │ FFmpeg 参考               │
├──────────────────┼─────┼────────────────────────────┼────────────┼───────────────────────────┤
│ BS 计算 (4x4 级) │ 5-1 │ 帧内=4, ref/mv 比较规则    │ deblock.rs │ h264_deblock.c            │
│ 边缘 QP 平均     │ 5-2 │ 相邻宏块 QP 取平均         │ deblock.rs │ h264_deblock.c            │
│ 弱滤波 (bs=1-3)  │ 5-3 │ alpha/beta/tc0 阈值表      │ deblock.rs │ h264_deblock_template.c   │
│ 强滤波 (bs=4)    │ 5-4 │ 亮度 4 像素/色度 2 像素    │ deblock.rs │ h264_deblock_template.c   │
│ 色度去块         │ 5-5 │ Chroma QP 映射, 色度阈值   │ deblock.rs │ h264_deblock_template.c   │
└──────────────────┴─────┴────────────────────────────┴────────────┴───────────────────────────┘
```

### 模块 6: 帧内预测 (已基本正确, 验证为主)

```text
┌──────────────────────────────┬─────┬──────────────────────────────────────┬──────────────────────┬─────────────────────┐
│ 子功能                       │ ID  │ 对比重点                             │ Tao 文件             │ FFmpeg 参考         │
├──────────────────────────────┼─────┼──────────────────────────────────────┼──────────────────────┼─────────────────────┤
│ I_4x4 九种模式               │ 6-1 │ 边界像素取值, 预测公式               │ intra.rs             │ h264pred_template.c │
│ I_8x8 九种模式 + 低通滤波    │ 6-2 │ 低通边界, High profile              │ intra.rs             │ h264pred_template.c │
│ I_16x16 四种模式             │ 6-3 │ DC/Plane 公式                       │ intra.rs             │ h264pred_template.c │
│ 色度预测 四种模式            │ 6-4 │ DC 变体                              │ intra.rs             │ h264pred_template.c │
│ 帧内模式可用性重映射         │ 6-5 │ unavailable 邻居 mode remapping      │ macroblock_intra.rs  │ h264_parse.c        │
└──────────────────────────────┴─────┴──────────────────────────────────────┴──────────────────────┴─────────────────────┘
```

### 模块 7: 输出 / DPB / POC (验证为主)

```text
┌─────────────────────────────┬─────┬────────────────────────────┬─────────────────────┬────────────────┐
│ 子功能                      │ ID  │ 对比重点                   │ Tao 文件            │ FFmpeg 参考    │
├─────────────────────────────┼─────┼────────────────────────────┼─────────────────────┼────────────────┤
│ POC 计算 (type 0/1/2)       │ 7-1 │ 公式对齐                   │ output.rs           │ h264_slice.c   │
│ 显示重排 (reorder buffer)   │ 7-2 │ reorder_depth, 输出时机    │ output.rs           │ h264_picture.c │
│ 帧数一致性                  │ 7-3 │ flush 后帧数对齐           │ output.rs, mod.rs   │ -              │
└─────────────────────────────┴─────┴────────────────────────────┴─────────────────────┴────────────────┘
```

## 推荐执行阶段

### Phase 1: CABAC 帧间路径 (目标: `data/2.mp4` 提升)

- 模块 0 Phase A (0-1 -> 0-7)。
- 模块 2 (2-1 -> 2-9)。
- 模块 1 (1-1 -> 1-3)。
- 模块 3 (3-1 -> 3-5)。
- 验证样本: `data/2.mp4` + 守护 `data/1.mp4`。

### Phase 2: CAVLC 独立修复 (目标: C1/E1/E7/E9 提升)

- 模块 0 Phase B (0-8 -> 0-11)。
- 验证样本: C1 + E1 + 守护 `data/1.mp4`。

### Phase 3: 残差/变换/去块精度微调 (目标: 全样本收敛)

- 模块 4 (4-1 -> 4-6)。
- 模块 5 (5-1 -> 5-5)。
- 验证样本: 全样本批量对比。

### Phase 4: 验证性扫描 (目标: 确认无遗漏)

- 模块 6 (6-1 -> 6-5)。
- 模块 7 (7-1 -> 7-3)。
- 验证样本: 全样本批量对比。

## 子功能点对比方法

对每个子功能点 (如 ID 2-4 "B_Direct spatial 模式"), 执行以下对比步骤:

1. 读 Tao 源码: 理解当前实现逻辑, 标注关键决策点。
2. 读 FFmpeg 对应函数: 逐行对比差异 (如 `h264_direct.c:pred_spatial_direct_motion`)。
3. 读 H.264 规范: 确认哪个实现符合规范 (ITU-T H.264 Section 7/8/9)。
4. 辅参 OpenH264/VLC: 作为第三方验证。
5. 定位偏差: 记录具体代码行和差异描述。
6. 修复并测试: 按执行流程操作。

## 精度测试命令

```bash
# 快速验证 (3帧)
TAO_H264_COMPARE_INPUT=data/2.mp4 TAO_H264_COMPARE_FRAMES=3 TAO_H264_COMPARE_REQUIRED_PRECISION=0.1 \
cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 标准验证 (10帧)
TAO_H264_COMPARE_INPUT=data/2.mp4 TAO_H264_COMPARE_FRAMES=10 TAO_H264_COMPARE_REQUIRED_PRECISION=0.1 \
cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 守护样本 (data/1.mp4 必须保持 100%)
TAO_H264_COMPARE_INPUT=data/1.mp4 TAO_H264_COMPARE_FRAMES=10 \
cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 全样本批量对比
cargo test --release --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored
```

## 子功能点状态跟踪

每个子功能点使用以下状态之一:

- 未开始: 尚未进行对比。
- 对比中: 正在阅读代码对比差异。
- 已确认正确: 对比后确认 Tao 实现与规范/FFmpeg 一致。
- 问题修复中: 发现差异, 正在修复。
- 已提交: 修复已通过验证并提交。
- 已回退 (需联动): 修复回退, 标记需配合其他修复点联动。
- 跳过 (上游阻塞): 因上游子功能未修复而暂跳。

## 重新拆分策略

当所有 ~50 个子功能点循环一遍后, 如精度仍未达标:

1. 分析误差分布: 运行宏块级诊断 (`TAO_H264_COMPARE_MB_DIAG=1`), 定位误差集中区域。
2. 识别高贡献模块: 统计哪些模块的子功能点贡献最大误差。
3. 细化拆分: 对高贡献模块进一步拆分 (例如将 "B_Direct spatial" 拆分为 "ref_idx 推导"/"MV 候选收集"/"零 MV 判定"/"16x16 粒度"/"8x8 粒度" 等)。
4. 重新循环: 用更细粒度的子功能点列表执行新一轮对比修复。

## 相关文件

### 核心解码器源码 (23 个文件)

- `crates/tao-codec/src/decoders/h264/mod.rs` - 解码器主状态机。
- `crates/tao-codec/src/decoders/h264/cabac.rs` - CABAC 引擎。
- `crates/tao-codec/src/decoders/h264/cabac_init_pb.rs` - CABAC P/B 初始化表。
- `crates/tao-codec/src/decoders/h264/cabac_init_ext.rs` - CABAC 扩展初始化表。
- `crates/tao-codec/src/decoders/h264/cavlc.rs` - CAVLC 引擎。
- `crates/tao-codec/src/decoders/h264/cavlc_mb.rs` - CAVLC 宏块语法。
- `crates/tao-codec/src/decoders/h264/syntax.rs` - CABAC 语法元素。
- `crates/tao-codec/src/decoders/h264/intra.rs` - 帧内预测。
- `crates/tao-codec/src/decoders/h264/macroblock_intra.rs` - 帧内宏块。
- `crates/tao-codec/src/decoders/h264/macroblock_inter.rs` - 帧间宏块 + Direct 模式。
- `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs` - MV 预测。
- `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs` - 帧间缓存。
- `crates/tao-codec/src/decoders/h264/macroblock_inter_weight.rs` - 加权预测。
- `crates/tao-codec/src/decoders/h264/macroblock_state.rs` - 宏块状态。
- `crates/tao-codec/src/decoders/h264/common.rs` - 运动补偿/插值。
- `crates/tao-codec/src/decoders/h264/residual.rs` - 残差/IDCT/反量化。
- `crates/tao-codec/src/decoders/h264/deblock.rs` - 去块滤波。
- `crates/tao-codec/src/decoders/h264/output.rs` - DPB/POC/输出重排。
- `crates/tao-codec/src/decoders/h264/slice_decode.rs` - Slice 级解码。
- `crates/tao-codec/src/decoders/h264/slice_parse.rs` - Slice Header 解析。
- `crates/tao-codec/src/decoders/h264/parameter_sets.rs` - SPS/PPS。
- `crates/tao-codec/src/decoders/h264/sei.rs` - SEI 消息。
- `crates/tao-codec/src/decoders/h264/config.rs` - avcC 配置。

### 测试基础设施

- `plans/tao-codec/video/h264/decoder_compare.rs` - 精度对比核心工具。
- `tests/run_decoder.rs` - 测试入口。
- `crates/tao-codec/src/decoders/h264/tests/` - 单元测试 (10 个文件)。

### 参考文档

- `plans/tao-codec/video/h264/accuracy_systematic_plan.md` - 现有精度计划 (将被本计划更新)。
- `plans/tao-codec/video/h264/decoder_accuracy.md` - 精度收敛计划。
- `plans/tao-codec/video/h264/decoder_dev.md` - 功能开发计划。
- `plans/tao-codec/video/h264/h264_feature_matrix.md` - 功能矩阵。

## Notes

- 对比时优先使用 FFmpeg 作为参考实现, OpenH264/VLC 作为辅助验证。
- 每次修复后必须同时验证 `data/1.mp4` 不回归 (守护样本)。
- CAVLC 路径的系统性问题与 CABAC 路径独立, 应分开处理 (Phase 1 vs Phase 2)。
- 发现上游根因问题 (如 CABAC 脱轨) 时立即中断当前循环, 优先修复根因。
- 该计划保存在 `plans/tao-codec/video/h264/accuracy_systematic_plan.md`, 每轮循环结束后更新子功能点状态。
