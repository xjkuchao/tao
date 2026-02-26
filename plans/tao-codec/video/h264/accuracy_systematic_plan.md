# H264 解码器精度收敛 -- 系统化功能对比与修复循环计划

## 概述

对 Tao H264 解码器按功能模块进行系统化拆分, 逐子功能点与 FFmpeg/OpenH264/VLC 实现对比, 发现差异即修复并验证, 循环迭代直至精度达标。

首要目标: `data/1.mp4` 和 `data/2.mp4` 达到阶段 A (PSNR≥50dB, 精度≥99%, max_err≤2)。
次要目标: C1-C3, E1-E9, X1-X4 全部达标。

## 当前精度基线 (2026-02-25 更新)

| 样本 | 精度 | 状态 | 熵编码 |
|------|------|------|--------|
| data/1.mp4 | **100.000%** (299帧) | **bit-exact** | CABAC High |
| data/2.mp4 | 57.01% (10帧) | 待修复 | CABAC Main + B帧 |
| C2 | 99.998% | 近 bit-exact | CABAC Main |
| E3 | 99.984% | 近 bit-exact | CABAC Main |
| X1 | 100% | bit-exact | CABAC I-only |
| C1/E1/E7/E9 | 10-22% | 严重偏差 | CAVLC |
| 其余 | 8-88% | 偏差 | 混合 |

## 关键观察

- data/1.mp4 (High CABAC) **100% bit-exact (299帧)** → High profile CABAC 路径完全正确
- X1 (I-only) 100% bit-exact → 帧内预测路径已正确
- C2/E3 (Main CABAC) 近 bit-exact → Main profile CABAC 基本正确
- data/2.mp4 (Main CABAC + B帧) Frame 0 bit-exact, Frame 1 (P) 99.56%, B帧大幅偏差
  - **确认**: P帧误差在 pre-deblock 重建阶段即已存在(非去块问题)
  - **确认**: Frame 0 (I) 无去块时 Tao 与 FFmpeg 0 diff → 参考帧完全一致
  - **确认**: Frame 1 (P) 无去块时仍有 8698 像素差异 (max_err=30) → 解码语法差异
  - **首个误差 MB**: MB(109,9) = P_L0_L0_8x16 (p_mb_type=2), pixel (1752,149), max_err=1
  - 误差从 MB(109,9) 开始逐渐放大 (CABAC 上下文漂移), 到 MB(30,66) 时 max_err=30
  - 已排除: 加权预测, qpel 插值, 去块 QP, CBP 上下文, CABAC 初始化表, 参考帧裁剪
  - **待排查**: P_L0_L0_8x16 路径的 CABAC 语法解码是否有微小偏差
- 所有 CAVLC 样本精度极低 → CAVLC 路径有独立的系统性问题

## 功能模块大块划分 (8 大模块)

### 模块 0: 熵解码 (CABAC/CAVLC) — 最高优先级

> 原理: 熵解码是最上游的环节, 一旦解析错误, 后续所有数据全部错误。过去 40+ 轮无效修复的教训已证明必须优先确保熵解码正确。

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| CABAC ref_idx 解码上下文 | 0-1 | 关键 | 上下文增量计算 (ctxIdxInc), 越界处理 | `macroblock_inter.rs` | `h264_cabac.c:decode_cabac_mb_ref` |
| CABAC mvd 上下文选择 (amvd) | 0-2 | 高 | 邻居 MVD 绝对值之和的分级, ctx 偏移 | `macroblock_inter_weight.rs` | `h264_cabac.c:decode_cabac_mb_mvd` |
| CABAC cbp 解码 | 0-3 | 高 | luma(4bit)+chroma(2bit) 邻居上下文 | `slice_decode.rs` | `h264_cabac.c:decode_cabac_mb_cbp_luma/chroma` |
| CABAC 残差 (sig/last/coeff_abs) | 0-4 | 高 | ctxBlockCat, 扫描顺序, 上下文映射 | `residual.rs` | `h264_cabac.c:decode_cabac_residual` |
| CABAC mb_skip / end_of_slice | 0-5 | 高 | 终止条件, skip 上下文偏移 | `slice_decode.rs` | `h264_cabac.c:decode_cabac_mb_skip` |
| CABAC mb_type / sub_mb_type | 0-6 | 中 | P/B 全类型映射, 上下文索引 | `slice_decode.rs` | `h264_cabac.c:decode_cabac_mb_type` |
| CABAC transform_size_8x8_flag | 0-7 | 中 | 解析时序(CBP 前/后), 条件门控 | `slice_decode.rs` | `h264_cabac.c` |
| CAVLC coeff_token nC 上下文 | 0-8 | 高 | nC 计算规则, slice 边界感知 | `cavlc.rs` | `h264_cavlc.c:pred_non_zero_count` |
| CAVLC level/total_zeros/run_before | 0-9 | 高 | VLC 查表正确性, 边界处理 | `cavlc.rs` | `h264_cavlc.c` |
| CAVLC mb_type/ref_idx/mvd | 0-10 | 高 | Exp-Golomb 映射, P/B 类型表 | `cavlc_mb.rs` | `h264_cavlc.c` |
| CAVLC cbp 映射表 | 0-11 | 中 | Intra/Inter 两套 VLC 映射 | `cavlc_mb.rs` | `h264_cavlc.c` |

### 模块 1: 参考帧管理

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| L0/L1 默认列表构建 | 1-1 | 高 | POC 排序规则, 长期/短期分组 | `output.rs` | `h264_refs.c:ff_h264_fill_default_ref_list` |
| 参考列表重排序 (modification) | 1-2 | 高 | ref_pic_list_modification 语义 | `slice_parse.rs`, `output.rs` | `h264_refs.c:ff_h264_ref_list_modification` |
| MapColToList0 (temporal direct) | 1-3 | 高 | 共定位帧 POC 映射 | `macroblock_inter.rs` | `h264_direct.c:ff_h264_direct_ref_list_init` |
| DPB 滑动窗口 + MMCO | 1-4 | 中 | 淘汰策略, op1-op6 | `output.rs` | `h264_refs.c` |
| 长期参考帧 | 1-5 | 低 | MMCO op3/op6 | `output.rs` | `h264_refs.c` |

### 模块 2: 帧间预测 / 运动向量预测

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| MV 中值预测 (A/B/C/D) | 2-1 | 高 | 候选选择, 仅 A 可用时直接复用 | `macroblock_inter_mv.rs` | `h264_mvpred.h:pred_motion` |
| 邻居可用性 (slice 边界) | 2-2 | 高 | same_slice_4x4 判定 | `macroblock_inter.rs` | `h264_mvpred.h` |
| P_Skip MV 推导 | 2-3 | 高 | 零 MV 条件, 邻居取值 | `macroblock_inter.rs` | `h264_mvpred.h:fill_decode_caches` |
| B_Direct spatial 模式 | 2-4 | 高 | ref_idx 推导, MV 候选, 零 MV 判定 | `macroblock_inter.rs` | `h264_direct.c:pred_spatial_direct_motion` |
| B_Direct temporal 模式 | 2-5 | 高 | dist_scale_factor, MV 缩放 | `macroblock_inter.rs` | `h264_direct.c:pred_temp_direct_motion` |
| col_zero_flag 判定 | 2-6 | 中 | 共定位 L0/L1 条件优先级 | `macroblock_inter.rs` | `h264_direct.c` |
| 16x8/8x16 方向性 MVP | 2-7 | 中 | 上/左/对角邻居特殊规则 | `macroblock_inter_mv.rs` | `h264_mvpred.h:pred_16x8_motion` |
| B_Direct_8x8 子分区 | 2-8 | 中 | 每 8x8 独立推导 | `macroblock_inter.rs` | `h264_direct.c` |
| MVD + MVP 合成 | 2-9 | 中 | 逐分区合成, 存储到 4x4 网格 | `macroblock_inter_mv.rs` | `h264_mvpred.h` |

### 模块 3: 运动补偿 (插值)

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| Luma qpel 6-tap (16 种位置) | 3-1 | 高 | 各半像素/四分之一像素位置的插值公式 | `common.rs` | `h264qpel_template.c` |
| 加权预测 (显式/隐式/默认) | 3-2 | 高 | 公式对齐, 舍入, log2_weight_denom | `macroblock_inter_weight.rs` | `h264_weight_template.c` |
| Chroma 双线性 (1/8 精度) | 3-3 | 中 | 插值公式, 舍入 | `common.rs` | `h264chroma_template.c` |
| 双向融合 (Bi-prediction) | 3-4 | 中 | `(L0 + L1 + 1) >> 1` 舍入规则 | `macroblock_inter_cache.rs` | `h264_mc_template.c` |
| 边界扩展 (padding) | 3-5 | 中 | 参考块越界时的像素复制策略 | `common.rs` | `videodsp_template.c` |

### 模块 4: 残差 / 变换 / 量化

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| 4x4 IDCT 精度 | 4-1 | 中 | 变换矩阵, 舍入, 裁剪 | `residual.rs` | `h264idct_template.c:ff_h264_idct_add` |
| 8x8 IDCT 精度 | 4-2 | 中 | High profile, 8x8 变换矩阵 | `residual.rs` | `h264idct_template.c:ff_h264_idct8_add` |
| Chroma DC Hadamard | 4-3 | 中 | 2x2 逆 Hadamard + 舍入偏移 | `residual.rs` | `h264idct_template.c` |
| 反量化 + 缩放矩阵 | 4-4 | 中 | flat/custom scaling list, QP 分级 | `residual.rs` | `h264_idct_template.c` |
| 残差裁剪 (0-255) | 4-5 | 低 | 重建像素溢出处理 | `residual.rs` | - |
| 扫描顺序 (zig-zag / 8x8) | 4-6 | 低 | 4x4/8x8 扫描表 | `residual.rs` | `h264_scan.h` |

### 模块 5: 去块滤波

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| BS 计算 (4x4 级) | 5-1 | 中 | 帧内=4, ref/mv 比较规则 | `deblock.rs` | `h264_deblock.c:ff_h264_filter_mb` |
| 边缘 QP 平均 | 5-2 | 中 | 相邻宏块 QP 取平均 | `deblock.rs` | `h264_deblock.c` |
| 弱滤波 (bs=1-3) | 5-3 | 中 | alpha/beta/tc0 阈值表, p1/q1 修正 | `deblock.rs` | `h264_deblock_template.c` |
| 强滤波 (bs=4) | 5-4 | 中 | 亮度 4 像素/色度 2 像素 | `deblock.rs` | `h264_deblock_template.c` |
| 色度去块 | 5-5 | 低 | Chroma QP 映射, 色度阈值 | `deblock.rs` | `h264_deblock_template.c` |

### 模块 6: 帧内预测 (验证为主, 已证明基本正确)

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| I_4x4 九种模式 | 6-1 | 低 | 边界像素取值, 预测公式 | `intra.rs` | `h264pred_template.c` |
| I_8x8 九种模式 + 低通滤波 | 6-2 | 低 | 低通边界, High profile | `intra.rs` | `h264pred_template.c` |
| I_16x16 四种模式 | 6-3 | 低 | DC/Plane 公式 | `intra.rs` | `h264pred_template.c` |
| 色度预测 四种模式 | 6-4 | 低 | DC 变体 | `intra.rs` | `h264pred_template.c` |
| 帧内预测模式可用性重映射 | 6-5 | 中 | unavailable 邻居时的 mode remapping | `macroblock_intra.rs` | `h264_parse.c` |

### 模块 7: 输出 / DPB / POC (验证为主)

| 子功能 | ID | 优先级 | 对比重点 | Tao 文件 | FFmpeg 参考 |
|--------|-----|--------|---------|----------|-------------|
| POC 计算 (type 0/1/2) | 7-1 | 中 | 公式对齐 | `output.rs` | `h264_slice.c` |
| 显示重排 (reorder buffer) | 7-2 | 中 | reorder_depth, 输出时机 | `output.rs` | `h264_picture.c` |
| 帧数一致性 | 7-3 | 中 | flush 后帧数对齐 | `output.rs`, `mod.rs` | - |

## 执行流程

```text
对每个功能块(按 0→1→2→3→4→5→6→7 顺序):
  对每个子功能(按优先级从高到低):
    第一步: 仔细对比 Tao 实现与 FFmpeg/OpenH264/H.264 规范
    第二步: 判断是否有偏差
      → 无偏差: 标记"已确认正确", 进入下一个子功能
      → 有偏差: 进入第三步
    第三步: 修复问题, 运行精度对比测试
      → data/1.mp4 + data/2.mp4 (G1=10帧快速验证)
      → 如果有明显提升: G2=67帧, G3=299帧完整验证
    第四步: 判断有效性 (见下方判定规则)
      → 有效修复: 严格 5 项验证 → 提交代码 → 进入第六步
      → 无效修复: 恢复代码 → 进入第六步
    第六步: 进入下一个子功能, 重复第一步

全部功能块循环完成后:
  → 精度达标: 结束
  → 精度未达标: 重新拆分功能块, 细化子功能, 再次循环
```

## 有效修复判定规则

1. **规则 1 (逻辑正确性优先)**: 如对比 H.264 规范或 FFmpeg/OpenH264 实现确认 Tao 有错误, 即使精度短期下降也视为有效修复
2. **规则 2 (精度指标辅助)**: 精度大幅提升 → 有效; 精度大幅下降 → 一般无效
3. **规则 3 (连锁修复)**: 某些修复需配合其他修复点才能生效, 从更大范围判断
4. **规则 4 (避免无根因修复)**: 如果发现上游问题(如 CABAC 漂移), 必须先修复上游再处理下游

## 推荐执行顺序

| 阶段 | 功能点范围 | 目标样本 | 理由 |
|------|-----------|---------|------|
| Phase 1 | 0-1 → 0-7 (CABAC) | data/1.mp4, data/2.mp4 | 最上游, 级联影响最大; C2/E3 已近 bit-exact 说明 Main CABAC 基本正确, 重点排查 High profile 路径差异 |
| Phase 2 | 2-1 → 2-9 (帧间预测) | data/1.mp4, data/2.mp4 | 帧间预测是 P/B 帧重建核心 |
| Phase 3 | 1-1 → 1-3 (参考管理) | data/1.mp4, data/2.mp4 | 参考列表影响所有帧间预测 |
| Phase 4 | 3-1 → 3-5 (运动补偿) | data/1.mp4, data/2.mp4 | 插值精度直接影响像素值 |
| Phase 5 | 0-8 → 0-11 (CAVLC) | C1, E1, E7, E9 | CAVLC 样本全部精度极低, 独立修复 |
| Phase 6 | 4-1 → 4-6 (残差/变换) | 全部 | 精度微调 |
| Phase 7 | 5-1 → 5-5 (去块) | 全部 | 后处理精度 |
| Phase 8 | 6-1 → 7-3 (帧内/输出) | 全部 | 验证性为主 |

## 精度测试命令

```bash
# 快速验证 (3帧)
TAO_H264_COMPARE_INPUT=data/1.mp4 TAO_H264_COMPARE_FRAMES=3 cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 标准验证 (10帧)
TAO_H264_COMPARE_INPUT=data/1.mp4 TAO_H264_COMPARE_FRAMES=10 cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 完整验证 (299帧)
TAO_H264_COMPARE_INPUT=data/1.mp4 TAO_H264_COMPARE_FRAMES=299 cargo test --release --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 自动轮转 (双样本)
plans/tao-codec/video/h264/run_accuracy_autoloop.sh

# 全样本批量对比
cargo test --release --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored
```

## 终止与重启条件

- 当所有子功能点循环一遍后, 如精度仍未达标, 重新审视功能划分, 细化拆分后重新循环
- 如发现新的根因级别问题 (类似之前的 CABAC 脱轨), 立即中断当前循环, 优先修复根因

## 对比方法

对每个子功能点, 对比步骤:
1. **读 Tao 源码**: 理解当前实现逻辑
2. **读 FFmpeg 对应函数**: 逐行对比差异 (参考下方代码位置映射)
3. **读 H.264 规范**: 确认哪个实现符合规范 (ITU-T H.264 Section 7/8/9)
4. **辅参 OpenH264**: 作为第三方验证 (github.com/cisco/openh264)
5. **定位偏差**: 记录具体代码行和差异描述
6. **修复并测试**: 按执行流程操作

## 相关文件

### 核心解码器源码
- `crates/tao-codec/src/decoders/h264/cabac.rs` — CABAC 引擎
- `crates/tao-codec/src/decoders/h264/cabac_init_pb.rs` — CABAC P/B 初始化表
- `crates/tao-codec/src/decoders/h264/cabac_init_ext.rs` — CABAC 扩展初始化表
- `crates/tao-codec/src/decoders/h264/cavlc.rs` — CAVLC 引擎
- `crates/tao-codec/src/decoders/h264/cavlc_mb.rs` — CAVLC 宏块语法
- `crates/tao-codec/src/decoders/h264/syntax.rs` — CABAC 语法元素
- `crates/tao-codec/src/decoders/h264/intra.rs` — 帧内预测
- `crates/tao-codec/src/decoders/h264/macroblock_intra.rs` — 帧内宏块
- `crates/tao-codec/src/decoders/h264/macroblock_inter.rs` — 帧间宏块 + Direct 模式
- `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs` — MV 预测
- `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs` — 帧间缓存
- `crates/tao-codec/src/decoders/h264/macroblock_inter_weight.rs` — 加权预测
- `crates/tao-codec/src/decoders/h264/common.rs` — 运动补偿/插值
- `crates/tao-codec/src/decoders/h264/residual.rs` — 残差/IDCT/反量化
- `crates/tao-codec/src/decoders/h264/deblock.rs` — 去块滤波
- `crates/tao-codec/src/decoders/h264/output.rs` — DPB/POC/输出重排
- `crates/tao-codec/src/decoders/h264/slice_decode.rs` — Slice 级解码
- `crates/tao-codec/src/decoders/h264/slice_parse.rs` — Slice Header 解析
- `crates/tao-codec/src/decoders/h264/parameter_sets.rs` — SPS/PPS
- `crates/tao-codec/src/decoders/h264/mod.rs` — 解码器主状态机
- `crates/tao-codec/src/decoders/h264/macroblock_state.rs` — 宏块状态
- `crates/tao-codec/src/decoders/h264/sei.rs` — SEI 消息
- `crates/tao-codec/src/decoders/h264/config.rs` — avcC 配置

### 测试基础设施
- `plans/tao-codec/video/h264/decoder_compare.rs` — 精度对比核心工具
- `plans/tao-codec/video/h264/run_accuracy_round.sh` — 单轮精度测试脚本
- `plans/tao-codec/video/h264/run_accuracy_autoloop.sh` — 自动轮转脚本
- `plans/tao-codec/video/h264/round_journal.md` — 轮转日志
- `tests/run_decoder.rs` — 测试入口

### 参考文档
- `plans/tao-codec/video/h264/decoder_accuracy.md` — 精度收敛计划
- `plans/tao-codec/video/h264/decoder_dev.md` — 功能开发计划
- `plans/tao-codec/video/h264/h264_feature_matrix.md` — 功能矩阵

### FFmpeg 源码参考映射

| 组件 | Tao 文件 | FFmpeg 参考文件 |
|------|----------|----------------|
| CABAC 引擎 | `cabac.rs` | `libavcodec/cabac.h`, `cabac_functions.h` |
| CABAC 语法 | `syntax.rs`, `slice_decode.rs` | `libavcodec/h264_cabac.c` |
| CAVLC 残差 | `cavlc.rs` | `libavcodec/h264_cavlc.c` |
| CAVLC 宏块 | `cavlc_mb.rs` | `libavcodec/h264_cavlc.c` |
| I 预测 | `intra.rs` | `libavcodec/h264pred_template.c` |
| 模式重映射 | `macroblock_intra.rs` | `libavcodec/h264_parse.c:130-210` |
| P_Skip MV | `macroblock_inter.rs` | `libavcodec/h264_mvpred.h:388-485` |
| MV 中值 | `macroblock_inter_mv.rs` | `libavcodec/h264_mvpred.h:226-277` |
| B Direct spatial | `macroblock_inter.rs` | `libavcodec/h264_direct.c:140-600` |
| B Direct temporal | `macroblock_inter.rs` | `libavcodec/h264_direct.c` |
| MapColToList0 | `macroblock_inter.rs` | `libavcodec/h264_direct.c:82-137` |
| 加权预测 | `macroblock_inter_weight.rs` | `libavcodec/h264_weight_template.c` |
| Luma qpel | `common.rs` | `libavcodec/h264qpel_template.c` |
| Chroma bilinear | `common.rs` | `libavcodec/h264chroma_template.c` |
| 残差/IDCT | `residual.rs` | `libavcodec/h264_idct_template.c` |
| 去块滤波 | `deblock.rs` | `libavcodec/h264_deblock.c` |
| 参考帧/DPB | `output.rs` | `libavcodec/h264_refs.c`, `h264_picture.c` |
| POC 计算 | `output.rs` | `libavcodec/h264_slice.c` |

## 备注

- 每个子功能对比时, 应同时参考 ITU-T H.264 规范原文 (特别是第 7/8/9 章)
- 对比时优先使用 FFmpeg 作为参考实现, OpenH264/VLC 作为辅助验证
- 该计划是迭代性的: 第一轮循环完成后如未达标, 需要细化拆分后重新循环
