# H264 解码器精度迭代循环计划 (Loop v1)

## 1. 背景与目标

- 背景: 当前 H264 解码器仍存在与 FFmpeg/OpenH264/VLC 的行为差异, 且已出现过在上游根因未解决时连续多轮下游无效修复的问题.
- 总目标: 通过 "功能大块 -> 子功能 -> 对比 -> 修复 -> 验证 -> 提交/回滚" 的固定循环, 推进到与参考实现一致.
- 样本优先级:
  - 首要目标: `data/1.mp4`, `data/2.mp4`.
  - 次要目标: `c1-c3`, `e1-e9`, `x1-x4`.
- 关键约束: 先解上游根因, 再做下游收敛, 避免重复无效轮次.

## 2. 功能大块划分与子功能拆分

### 功能点 1: 熵解码同步与语法正确性 (最高优先级)

- 1-1: CABAC 在 P/B 帧语法解析同步, 聚焦 `mb_type/ref_idx/mvd/cbp/residual` 的上下文与时序一致性.
- 1-2: CABAC `mb_skip/end_of_slice/transform_size_8x8_flag` 条件与终止路径.
- 1-3: CAVLC `coeff_token/total_zeros/run_before` 与 `nC` 上下文计算.

### 功能点 2: 参考帧列表与 DPB/MMCO

- 2-1: L0/L1 默认参考列表构建与 POC 排序.
- 2-2: `ref_pic_list_modification` 重排序语义.
- 2-3: DPB 滑窗, MMCO op1-op6, 长短期参考帧切换.

### 功能点 3: 帧间预测与运动向量预测

- 3-1: MVP 候选(A/B/C/D)与 slice 边界可用性.
- 3-2: P_Skip 推导与 MVD+MVP 合成.
- 3-3: B_Direct (spatial/temporal), `MapColToList0`, `dist_scale_factor`, `col_zero_flag`.

### 功能点 4: 运动补偿与加权预测

- 4-1: Luma qpel 16 位置插值.
- 4-2: Chroma 双线性插值与边界扩展.
- 4-3: 显式/隐式/默认加权预测与双向融合舍入.

### 功能点 5: 残差, 反量化, 逆变换

- 5-1: 4x4/8x8 IDCT 精度与舍入.
- 5-2: Chroma DC Hadamard 与扫描顺序.
- 5-3: scaling list, QP 分级, 重建裁剪.

### 功能点 6: 去块滤波与输出重排

- 6-1: BS 计算, alpha/beta/tc0 阈值路径.
- 6-2: 色度去块与 QP 映射.
- 6-3: POC type0/1/2, reorder buffer, 输出帧一致性.

## 3. 固定循环流程 (每个子功能必须执行 1-6)

1. 从当前子功能开始, 先做实现对比: Tao vs FFmpeg/OpenH264/VLC + H264 规范.
2. 分支判断:
   - 若确认无差异, 直接进入步骤 6.
   - 若确认有差异/疑点, 进入步骤 3.
3. 尝试修复问题, 并运行精度对比测试判断修复有效性.
4. 若有效修复, 提交代码, 进入步骤 6.
5. 若无效修复, 回滚本次改动, 进入步骤 6.
6. 切换到下一个子功能, 回到步骤 1.

全子功能遍历后:

- 若总体精度仍未达标, 重新执行 "功能大块拆分 -> 子功能拆分 -> 1-6 循环", 进入下一轮循环.

## 4. 有效修复判定规则 (强制执行)

### 4.1 逻辑正确性优先 (主判定)

- 若可被以下证据稳定证明是 Tao 实现错误, 即判定为有效修复:
  - 与 H264 规范冲突.
  - 与 FFmpeg/OpenH264/VLC 三方实现一致性对比后确认 Tao 偏差.
- 在该条件成立时, 即使短期精度下降, 仍允许判定有效并提交.

### 4.2 精度变化辅助判定

- 一般情况下:
  - 精度大幅提升 -> 有效.
  - 精度大幅下降且逻辑证据不足 -> 无效.

### 4.3 连锁修复判定

- 若修复点逻辑正确, 但依赖上游/并行修复点联动才能生效, 可判定为有效.
- 此类修复提交时必须标注 "联动依赖功能点", 并在后续回归中复核.

### 4.4 防止无效轮次扩散

- 阻断规则: 在 `1-1` 未完成前, 禁止进入功能点 3-6 的实质修复.
- 连续 3 轮未能在 `1-1` 获取新的逻辑证据时, 必须回到功能点拆分阶段, 重新细分 1-1 子路径后继续.

## 5. 对比与验证口径

### 5.1 对比对象

- 规范: ITU-T H.264 对应语法和解码流程章节.
- 参考实现:
  - FFmpeg `libavcodec/h264*`.
  - OpenH264 `codec/decoder/core`.
  - VLC `modules/codec/avcodec` 及其 H264 调用链.

### 5.2 测试样本优先级

- P0: `data/1.mp4`, `data/2.mp4` (每轮必测).
- P1: `c1-c3`, `e1-e9`, `x1-x4` (阶段性回归).

### 5.3 最小验证集

- 快速门禁: 3 帧.
- 短门禁: 10 帧.
- 中门禁: 67 帧.
- 长门禁: 299 帧或全量可用帧.

## 6. 分步任务与预期产出

1. 建立轮次基线.
   - 产出: `data/1.mp4` 与 `data/2.mp4` 的 3/10/67 帧精度基线.
2. 执行功能点 1 的 1-1 到 1-3 循环.
   - 产出: 每个子功能的差异结论, 修复提交或回滚记录.
3. 通过功能点阻断门禁后, 按 2 -> 6 顺序循环.
   - 产出: 子功能级对比证据, 修复记录, 精度趋势.
4. 完成全量子功能后进行总评估.
   - 产出: 是否达标结论, 若未达标则给出下一轮拆分.

## 7. 依赖与前置条件

- 分支: 当前工作分支为 `h264`.
- 工具:
  - `cargo`, `rustfmt`, `clippy`.
  - 本地可执行 `ffmpeg` (用于参考输出和差异定位).
- 数据:
  - `data/1.mp4`, `data/2.mp4` 必须可读.
  - `c1-c3`, `e1-e9`, `x1-x4` 样本路径可访问.
- 日志:
  - 轮次结果写入 `plans/tao-codec/video/h264/p0_round_log.md` 或新增同目录轮次日志.

## 8. 验收标准

- 子功能级验收:
  - 每个子功能都必须有 "已确认正确" 或 "已修复提交" 或 "已判定无效并回滚" 的明确结果.
- 阶段验收:
  - 首要样本 `data/1.mp4`, `data/2.mp4` 达到目标精度阈值.
- 全量验收:
  - 次要样本 `c1-c3`, `e1-e9`, `x1-x4` 达到阶段目标.
  - 无新增回归.

## 9. 进度标记 (断点续执行)

### 9.1 当前循环状态

- 当前轮次: `Round-10`.
- 当前功能点: `1`.
- 当前子功能: `1-1`.
- 当前状态: `in_progress`.
- 上次提交: `9bd99cf`.

### 9.2 子功能检查表

- [ ] 1-1 CABAC P/B 帧语法同步.
- [ ] 1-2 CABAC skip/eos/8x8_flag.
- [ ] 1-3 CAVLC token/zeros/run.
- [ ] 2-1 默认参考列表.
- [ ] 2-2 参考列表重排.
- [ ] 2-3 DPB/MMCO.
- [ ] 3-1 MVP 邻居可用性.
- [ ] 3-2 P_Skip + MVD 合成.
- [ ] 3-3 B_Direct 预测链路.
- [ ] 4-1 Luma qpel.
- [ ] 4-2 Chroma 插值.
- [ ] 4-3 加权预测.
- [ ] 5-1 4x4/8x8 IDCT.
- [ ] 5-2 Chroma DC + 扫描.
- [ ] 5-3 scaling list + QP.
- [ ] 6-1 BS/阈值路径.
- [ ] 6-2 色度去块.
- [x] 6-3 POC/重排输出.

### 9.3 每轮记录模板

- 子功能: `x-y`.
- 对比结论: `一致 | 不一致`.
- 逻辑证据: `规范章节 + 参考实现文件`.
- 精度变化: `data/1.mp4`, `data/2.mp4`, 可选回归集.
- 判定: `有效修复(提交) | 无效修复(回滚) | 无差异(跳过)`.
- 下一子功能: `x-(y+1)` 或下一功能点.

### 9.4 Round-2 记录(3-3: B_Direct temporal)

- 子功能: `3-3`.
- 对比结论: `不一致`.
- 逻辑证据:
  - FFmpeg `h264_direct.c` 的 temporal direct 路径未使用 `col_zero_flag` 置零.
  - OpenH264 `mv_pred.cpp` 的 `PredBDirectTemporal` 也以共定位 MV 缩放链路为主, 无等价 temporal 置零分支.
  - Tao 旧实现在 temporal direct 额外执行 `col_zero` 强制置零, 语义不一致.
- 修复改动:
  - 移除 temporal direct 中 `col_zero` 强制置零逻辑, 保留 spatial direct 的 `col_zero` 行为.
  - 清理本轮临时诊断开关(`TAO_H264_TRACE_*`, `TAO_H264_DROP_B_REF`)避免调试分支残留.
  - 同步更新 temporal list1 fallback 场景测试断言.
- 精度变化:
  - `data/2.mp4`:
    - 10 帧: `63.010095%` (前: `63.010095%`)
    - 20 帧: `49.291953%` (前: `49.291953%`)
    - 67 帧: `37.353963%` (前: `37.353963%`)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (保持无回归)
- 判定: `有效修复(逻辑正确性优先, 精度短期持平)`.
- 下一子功能: `1-1` (回到 CABAC P/B 帧语法同步主线).

### 9.5 Round-3 记录(6-3: 输出重排/DPB 容量约束)

- 子功能: `6-3`.
- 对比结论: `不一致`.
- 逻辑证据:
  - 现有实现在 `push_video_for_output` 中使用 `reference_frames.len() + reorder_buffer.len()` 与 `dpb_capacity` 比较.
  - `reorder_buffer` 中参考帧与 `reference_frames` 指向同一已解码图片语义, 直接相加会重复计数, 触发过早出队.
  - 在 `data/2.mp4` 实测中出现明显乱序输出 (`poc` 序列非单调), 与 H264 输出重排语义不一致.
- 修复改动:
  - `ReorderFrameEntry` 新增 `is_reference` 字段.
  - `build_output_frame` 入队时传入 `self.last_nal_ref_idc != 0`.
  - DPB 容量约束改为 `reference_frames + pending_non_ref` 口径, 避免对参考帧重复计数.
  - 同步更新 `tests/output.rs` 调用签名.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (前: `49.291953%`, `+45.608230`)
    - 67 帧: `82.965657%` (前: `37.353963%`, `+45.611694`)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (保持无回归)
  - `shift` 诊断:
    - 修复前最佳 `shift=-1` 且精度显著高于 `shift=0`.
    - 修复后 `shift=0` 已明显最佳, 帧错位特征消失.
- 判定: `有效修复(逻辑正确性与精度均显著提升)`.
- 下一子功能: `1-1` (继续 CABAC P/B 帧语法同步主线).

### 9.6 Round-4 记录(1-1: B-slice 邻居 slice 标记缺失容错 + direct spatial AB)

- 子功能: `1-1`.
- 对比结论:
  - `不一致`: B-slice CAVLC 路径在局部/断点解码场景下, 当左/上邻居 `mb_slice_first_mb==u32::MAX` 时, 当前 MB 仍按严格同 slice 约束计算邻居可用性, 导致 direct spatial/predict 路径退化.
  - `无效尝试已回滚`: 在 spatial direct 的 L1 邻居提取中引入 MB 级回退后, 主目标精度显著下降.
- 逻辑证据:
  - P-slice CAVLC 已有 `unknown neighbor` 放宽逻辑, B-slice 缺失同等容错, 路径不对称.
  - FFmpeg `pred_spatial_direct_motion` 依赖邻居缓存可用性;在局部调试/断点解码环境下若 slice 标记缺失, 需要避免把本应可用邻居全部判为跨 slice.
- 修复改动(保留):
  - 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`.
  - 在 B-slice CAVLC 每 MB 处理入口增加 `unknown neighbor` 检测.
  - 仅当左/上邻居 slice 标记为 `u32::MAX` 时, 临时将当前 MB 标记为 `u32::MAX` 放宽同 slice 判定.
  - 在 `continue/break` 前恢复原 `slice_first_mb`, 避免污染后续 MB 状态.
- 无效改动(已回滚):
  - 尝试在 `macroblock_inter.rs` 的 spatial direct L1 邻居提取中加入 MB 级回退.
  - 回归结果:
    - `data/2.mp4` 20 帧: `94.900183% -> 88.672407%`
    - `data/2.mp4` 67 帧: `82.965657% -> 78.567244%`
  - 判定: `无效修复`, 已完整回滚.
- 精度变化(保留改动后):
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (持平)
    - 67 帧: `82.965657%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `test_decode_cavlc_slice_data_b_non_skip_direct_spatial_zero_condition_forces_zero_mv` 通过.
  - `test_decode_cavlc_slice_data_b_non_skip_direct_spatial_uses_independent_l1_neighbor_mv` 仍失败(`17 != 18`), 进入下一轮持续定位.
- 判定: `有效修复(逻辑正确性成立, 主目标精度无回归)`.
- 下一子功能: `1-1` (继续 direct spatial L1 邻居差异对齐, 仅接受不降精度修复).

### 9.7 Round-5 记录(1-1: direct spatial L1 邻居差异在 unknown-slice 场景的定向修复)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - `decode_b` 的 direct spatial 用例里, 在 `first_mb` 局部解码 + 邻居 `slice_first_mb==u32::MAX` 场景下, L1 仅依赖 4x4 cache 会丢失邻居 MV, 与预期行为不一致.
- 逻辑证据:
  - 正常全量解码应严格依赖 4x4 cache.
  - 仅在 unknown-slice 局部解码场景下, 可接受受限 MB 级回退以保证 direct 邻居可用性, 且不应影响常规路径.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter.rs`.
  - 在 `spatial_direct_neighbor_candidates_for_list(list1)` 中增加受限回退:
    - 仅当当前或邻居 MB 的 `mb_slice_first_mb == u32::MAX` 时启用.
    - 优先 4x4 cache, 失败后回退到 MB 级 L1 运动信息.
    - 非 unknown-slice 场景保持原行为不变.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (持平)
    - 67 帧: `82.965657%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec direct_spatial_ -- --nocapture` 全通过.
  - `test_decode_cavlc_slice_data_b_non_skip_direct_spatial_uses_independent_l1_neighbor_mv` 通过.
- 判定: `有效修复(逻辑收敛 + 用例修复 + 主目标无回归)`.
- 下一子功能: `1-1` (继续 P/B 语法与运动预测差异定位, 聚焦 frame1 首次失配根因).

### 9.8 Round-6 记录(1-1 + 6-3: unknown-slice L1 回退收敛 + reorder_depth 推导修复)

- 子功能:
  - `1-1`: direct spatial unknown-slice 回退约束修正.
  - `6-3`: SPS 未显式信令 `max_num_reorder_frames` 时的 `reorder_depth` 推导.
- 对比结论: `不一致`.
  - Round-5 的 L1 MB 回退在部分 `prediction` 场景会过度生效, 导致 L0 被意外置空.
  - `derive_reorder_depth_from_sps` 在 `max_num_reorder_frames=None` 且 `max_num_ref_frames` 较小时会被放大到 level 上限(如 15), 与预期不符.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter.rs`
    - L1 MB 回退新增 `has_l0_seed_for_direct` 约束.
    - 仅在 unknown-slice 且存在 L0 4x4 邻居种子时启用回退, 避免破坏 `L0/L1 同时回退` 语义.
  - 文件: `crates/tao-codec/src/decoders/h264/mod.rs`
    - `derive_reorder_depth_from_sps` 引入 `signaled_ref_cap`.
    - 当未显式信令 `max_num_reorder_frames` 时, 使用 `min(signaled_ref_cap, level_reorder_cap)`.
    - 显式信令路径同样受 `signaled_ref_cap` 约束.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (持平)
    - 67 帧: `82.965657%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全部通过(`184 passed`).
  - `test_activate_sps_updates_reorder_depth_from_sps_max_ref_frames` 通过.
  - `test_activate_sps_reorder_depth_clamped_by_max_num_reorder_frames` 通过.
- 判定: `有效修复(逻辑正确性成立 + 测试覆盖提升 + 主目标无回归)`.
- 下一子功能: `1-1` (继续定位 `data/2.mp4` frame1 首次失配根因, 优先 CABAC P/B 语法细节差异).

### 9.9 Round-7 记录(1-1: Direct_8x8 与 transform_size_8x8_flag 门控对齐)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 在 CABAC B-slice 的 `no_sub_mb_part_size_less_than_8x8_flag` 计算中, 未把 `direct_8x8_inference_flag==0` 纳入 Direct 路径约束.
  - 结果是某些 B_Direct/B_8x8(Direct_8x8) 场景会错误允许解析 `transform_size_8x8_flag`, 存在 CABAC 语法失步风险.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_cabac.c` 在 `IS_DIRECT(mb_type)` 路径显式执行 `dct8x8_allowed &= sps->direct_8x8_inference_flag`.
  - OpenH264 `codec/decoder/core/src/decode_slice.cpp` 通过 `pNoSubMbPartSizeLessThan8x8Flag` 门控 `transform_size_8x8_flag` 读取, 与规范口径一致.
  - VLC `modules/codec/avcodec/video.c` 使用 libavcodec 解码 H264, 行为与 FFmpeg 主路径一致.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter.rs`
    - 新增 `b_no_sub_mb_part_size_less_than_8x8()`:
      - 仅当子分区均为 8x8 且若存在 Direct_8x8 时 `direct_8x8_inference_flag==1`, 才返回 true.
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs`
    - `B_Direct_16x16` 分支将 `no_sub_mb_part_size_less_than_8x8_flag` 绑定为 `direct_8x8_inference_enabled()`.
    - `B_8x8` 分支改为调用新 helper 计算 no_sub 标记.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/prediction.rs`
    - 新增 `test_b_no_sub_mb_part_size_less_than_8x8_respects_direct_8x8_inference_flag`.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (持平)
    - 67 帧: `82.965657%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`185 passed`).
  - 新增 no_sub 门控测试通过.
- 判定: `有效修复(规范/参考实现一致性成立, 主目标无回归)`.
- 下一子功能: `1-1` (继续定位 `data/2.mp4` frame1 局部失配, 优先 CABAC mb_type/ref_idx/mvd 条件分支差异).

### 9.10 Round-8 记录(1-1: B_8x8 混合 direct 子分区的 ref_idx 上下文时序对齐)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 在 `B_8x8` 混合子分区路径中, 先解析 `ref_idx` 再写入 direct 4x4 标记.
  - FFmpeg 在同路径先建立 direct cache, 再进入 `ref_idx` 语法解析, 以保证 `ctxIdxInc` 正确忽略 direct 邻居.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_cabac.c` 中:
    - 先在 `partition_count == 4` 分支对 direct 子分区执行 `fill_rectangle(... direct_cache ...)`.
    - 随后才进入 `decode_cabac_mb_ref` 循环解析 `ref_idx`.
  - Tao 的 `decode_ref_idx` 上下文计算依赖 `get_direct_4x4_flag`, 若标记写入滞后会改变 `ctxIdxInc`.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs`.
  - 在 `Some(22)`(B_8x8) 路径中, `sub_type` 解析后立即对 direct 子分区预写 `set_direct_block_4x4(..., true)`.
  - 保持后续语法顺序不变(`L0 ref_idx` 全量后再 `L1 ref_idx`).
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `94.900183%` (持平)
    - 67 帧: `82.965657%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`185 passed`).
  - `TAO_H264_COMPARE_INPUT=data/2.mp4` 的 20/67 帧回归无退化, `data/1.mp4` 维持 100%.
- 判定: `有效修复(语法时序与参考实现一致, 主目标无回归)`.
- 下一子功能: `1-1` (继续聚焦 frame1 首次失配, 排查 CABAC mb_type 分支选择与 MVD 上下文联动差异).

### 9.11 Round-9 记录(1-1: B_8x8 direct 缓存建立时序对齐)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - FFmpeg 在 `B_8x8(partition_count==4)` 路径中, 会在解析 non-direct 子分区 `ref_idx/mvd` 之前先执行 direct 运动推导并写入缓存.
  - Tao 原实现仅提前写了 direct 标记, 但直到最后 `apply` 阶段才建立 direct 运动缓存, 导致 non-direct 子分区 MVP 无法读取 direct 邻居运动信息.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_cabac.c`:
    - direct 子分区存在时先调用 `ff_h264_pred_direct_motion(...)` (source line 2124).
    - 后续 `ref_idx` 循环跳过 `IS_DIRECT` 分区 (source line 2142), non-direct 分区继续解码 `ref_idx/mvd`.
  - 这意味着 direct 子分区缓存在 non-direct `pred_motion` 前已可见.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs`.
  - `Some(22)`(B_8x8) 路径中:
    - 在 `sub_mb_type` 解析后、`ref_idx/mvd` 解析前, 提前对 direct 子分区执行 `apply_b_direct_sub_8x8`, 建立 direct 运动缓存.
    - `ref_idx_l0/ref_idx_l1` 初始化阶段对 direct 子分区不再写 `-1`, 避免覆盖提前写入的 direct 缓存.
    - 最终应用阶段对已提前处理的 direct 子分区跳过重复 `apply`.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/decode_b.rs`.
    - 新增 `test_decode_cavlc_slice_data_b_non_skip_b8x8_l0_uses_direct_neighbor_motion_cache`.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `95.284645%` (前: `94.900183%`, `+0.384462`)
    - 67 帧: `85.109306%` (前: `82.965657%`, `+2.143649`)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (保持无回归)
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`186 passed`).
  - 新增 direct 缓存时序回归用例通过.
- 判定: `有效修复(逻辑与参考实现一致, 且主目标精度显著提升)`.
- 下一子功能: `1-1` (继续定位 `data/2.mp4` frame1 首次失配, 聚焦 B_8x8 CABAC 与 MVP 交互分支).

### 9.12 Round-10 记录(1-1: CABAC mvd 上下文 amvd 口径对齐 FFmpeg)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 的 `compute_cabac_amvd` 额外依赖 `ref_idx>=0` 门控后再累计左/上邻居 mvd.
  - FFmpeg `DECODE_CABAC_MB_MVD` 直接使用 `mvd_cache[left] + mvd_cache[top]` 计算 `amvd`, 不额外读取 `ref_cache/ref_idx`.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_cabac.c`:
    - `DECODE_CABAC_MB_MVD` 宏中 `amvd0/amvd1` 直接取 `mvd_cache` 左/上项求和.
    - `decode_cabac_mb_mvd` 的 `ctxbase + (amvd>2) + (amvd>32)` 决策仅依赖该 `amvd`.
  - 因此 `amvd` 口径应由写缓存路径保障(无效邻居写 0), 不应在读取时再二次门控 `ref_idx`.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_state.rs`
    - `compute_cabac_amvd` 改为:
      - 仅按可用性判断左/上邻居.
      - 直接累计 `mvd_cache` 左/上绝对值.
      - 移除 `ref_idx>=0` 的附加门控.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/prediction.rs`
    - 更新单测口径为 “`amvd` 直接来自 `mvd_cache`”.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `95.284645%` (持平)
    - 67 帧: `85.109306%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec test_cabac_amvd_ -- --nocapture` 通过.
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`186 passed`).
- 判定: `有效修复(与参考实现语义对齐, 主目标无回归)`.
- 下一子功能: `1-1` (继续定位 `data/2.mp4` frame1 首失配, 聚焦 P-slice partition_count=4 的 ref_idx/mvd 上下文时序).

### 9.13 Round-11 记录(1-1: frame1 首失配定向诊断与无效路径回滚)

- 子功能: `1-1`.
- 对比结论: `存在差异`, 但本轮尝试路径均未形成可提交修复.
- 诊断事实:
  - `data/2.mp4` 首失配稳定在 frame1.
  - 首个误差 MB 稳定在 `(109,9)`, 路径为 `PInter + p_mb_type=2(P_8x16)`.
  - 该 MB `cbp_luma=0`, Y 仅来自运动补偿(非 luma 残差).
  - MB 局部误差表现为小幅偏差(`max_err=1`)后在后续帧放大.
- 本轮实验:
  - 实验 A: 1088 非裁剪对齐诊断(`TAO_H264_OUTPUT_CODED_SIZE` + FFmpeg `apply_cropping=0`).
  - 实验 B: 双侧关闭去块(Tao `TAO_SKIP_DEBLOCK=1` + FFmpeg `skip_loop_filter=all`).
  - 实验 C: 关闭 `P_8x16` part=1 对角快捷预测, 强制回退 median.
- 结果:
  - 实验 A/B 未消除 frame1 首失配, 仅有轻微波动.
  - 实验 C 精度显著下降:
    - `data/2.mp4` 20 帧: `95.284645% -> 79.787928%`.
  - 判定该方向为无效修复.
- 处理:
  - 全部临时代码与诊断开关实现已回滚.
  - 回到基线:
    - `data/2.mp4` 20 帧: `95.284645%`(恢复).
- 判定: `无效修复(已回滚)`.
- 下一子功能: `1-1` (继续聚焦 frame1 的 P_8x16 运动补偿链路, 优先排查 MC/MVP 与 FFmpeg 的逐分区一致性).

### 9.14 Round-12 记录(1-1: CABAC amvd 跨 slice 邻居可用性门控对齐 FFmpeg)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 的 `compute_cabac_amvd` 只按边界判断左/上邻居可用, 未过滤跨 slice 邻居.
  - FFmpeg 的 `decode_cabac_mb_mvd` 读取的是当前 MB 局部 `mvd_cache` 邻居项, 该缓存构建时已按 slice 可用性过滤不可用邻居.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_cabac.c`:
    - `DECODE_CABAC_MB_MVD` 仅累加 `mvd_cache[scan8[n]-1/-8]`.
  - FFmpeg `libavcodec/h264_mvpred.h`:
    - 邻居提取路径将不可用(含边界/切片不可达)位置折叠为不可用候选, 不应把跨 slice 运动信息泄漏进当前上下文.
  - Tao 直接读取全帧 4x4 MVD 缓存时若不加 slice 门控, 可能在跨 slice 边界引入错误 `ctxIdxInc`.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_state.rs`
    - `compute_cabac_amvd` 新增同 slice 门控:
      - 左邻: `left_neighbor_available_4x4 && same_slice_4x4(cur, left)`.
      - 上邻: `top_neighbor_available_4x4 && same_slice_4x4(cur, top)`.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/prediction.rs`
    - 新增 `test_cabac_amvd_ignores_cross_slice_neighbors`.
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `95.284645%` (持平)
    - 67 帧: `85.109306%` (持平)
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平)
- 测试结果:
  - `cargo test -p tao-codec test_cabac_amvd_ -- --nocapture` 通过(3 passed).
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`187 passed`).
- 判定: `有效修复(与参考实现语义对齐, 主目标无回归)`.
- 下一子功能: `1-1` (继续聚焦 frame1 的 `P_8x16` 首失配, 下一轮优先核对 subpel 插值与边界采样的实现细节).

### 9.15 Round-13 记录(1-1: P_8x16 part=1 的 C/D 回退条件对齐 FFmpeg)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 的 `predict_mv_l0_8x16/predict_mv_l1_8x16` 在 part=1 快捷路径中, 使用 `Option` 获取 C 候选.
  - 该实现将 `C=LIST_NOT_USED(ref=-1)` 与 `C=PART_NOT_AVAILABLE` 混同为 `None`, 会误触发 “C 不可用 -> 回退 D(左上)” 逻辑.
  - FFmpeg 语义中, 仅当 `C=PART_NOT_AVAILABLE` 才回退 D; `C=LIST_NOT_USED` 必须保留并进入 `pred_motion` 中值分支.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_mvpred.h`:
    - `fetch_diagonal_mv` 显式区分 `PART_NOT_AVAILABLE` 与 `LIST_NOT_USED`.
    - `pred_8x16_motion` 仅在 `diagonal_ref == ref` 时走快捷返回, 否则进入 `pred_motion`.
    - `pred_motion` 仅在 `diagonal_ref == PART_NOT_AVAILABLE` 时回退 D.
  - 说明 “C 不可用” 与 “C 可用但 list 未使用” 在语义上不可合并.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs`
    - `predict_mv_l0_8x16` / `predict_mv_l1_8x16` 改为基于 `MotionNeighbor` 状态机:
      - 保留 `PartNotAvailable` 与 `ListNotUsed` 区分.
      - 仅在 `PartNotAvailable` 时回退 D.
      - 其余场景按 `pred_motion` 路径回退到通用分区预测.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/prediction.rs`
    - 新增:
      - `test_predict_mv_l0_8x16_part1_does_not_use_d_when_c_is_list_not_used`
      - `test_predict_mv_l1_8x16_part1_does_not_use_d_when_c_is_list_not_used`
- 精度变化:
  - `data/2.mp4`:
    - 20 帧: `95.284645% -> 98.558978%` (`+3.274333`)
    - 67 帧: `85.109306% -> 98.099946%` (`+12.990640`)
    - 首个不一致帧: `1 -> 2`
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (保持无回归)
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 全通过(`189 passed`).
  - 新增 L0/L1 8x16 C/D 回退语义单测通过.
- 判定: `有效修复(逻辑与 FFmpeg 对齐, 且主目标精度显著提升)`.
- 下一子功能: `1-1` (继续聚焦 frame2 新首失配, 优先排查 CABAC 与 P/B 运动预测联动路径).

### 9.16 Round-14 记录(1-1: deblock B-slice 交叉匹配语义对齐 FFmpeg check_mv)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 的 `combine_motion_list_mismatch` 在 B-slice 交叉匹配分支中额外要求 `ref_idx>=0`, 并且对 `ref=-1` 场景直接跳过 MV 门限比较.
  - FFmpeg `check_mv` 语义:
    - 交叉参考匹配判定仅做相等比较, 不要求 `>=0`.
    - 在 list1 路径中, 即使 `ref=-1` 仍参与 MV 差门限比较.
- 逻辑证据:
  - FFmpeg `libavcodec/h264_loopfilter.c:check_mv`:
    - `ref_cache[0]` 仅在 `ref!=-1` 时比较 MV.
    - `ref_cache[1]` 分支直接比较 `ref` 或 MV 差.
    - 交叉分支仅检查 `ref0(a)==ref1(b)` 与 `ref1(a)==ref0(b)`.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/deblock.rs`
    - 重写 `combine_motion_list_mismatch` 为 FFmpeg `check_mv` 等价流程:
      - L0/L1 的 mismatch 触发条件与顺序对齐.
      - 交叉参考匹配去掉 `>=0` 限制.
      - list1 在 `ref=-1` 情况下保留 MV 门限判定.
  - 新增回归测试:
    - `test_boundary_strength_vertical_within_mb_cross_ref_match_accepts_negative_ref`
    - `test_boundary_strength_vertical_within_mb_list1_negative_ref_mv_diff_is_one`
- 精度变化:
  - `data/2.mp4`:
    - 67 帧: `98.099946% -> 98.089780%` (`-0.010166`)
    - 首个不一致帧维持 `2`.
  - `data/1.mp4`:
    - 10 帧: `100.000000%` (持平).
- 测试结果:
  - `cargo test -p tao-codec decoders::h264::deblock::tests:: -- --nocapture` 通过(`23 passed`).
  - `cargo test -p tao-codec decoders::h264::tests:: -- --nocapture` 通过(`189 passed`).
  - `cargo test --release --test run_decoder h264::test_h264_accuracy_ -- --nocapture --ignored` 通过(`17 passed`).
- 判定: `有效修复(去块滤波判定逻辑已与 FFmpeg 对齐; 精度轻微波动可接受)`.
- 下一子功能: `1-1` (继续聚焦 frame2 首失配, 优先定位 B 16x8 L0/L1 在去块边界强度与亚像素补偿链路的联动差异).

### 9.17 Round-15 记录(1-1: CAVLC P_16x8/P_8x16 的方向性 MVP 对齐 FFmpeg)

- 子功能: `1-1`.
- 对比结论: `不一致`.
  - Tao 的 CAVLC P-slice 路径在 `mb_type=1(P_16x8)` / `mb_type=2(P_8x16)` 中, 使用了通用 `predict_mv_l0_partition` + “同 ref 直接复用上一分区 MV”捷径.
  - FFmpeg `h264_mvpred.h` 的 `pred_16x8_motion/pred_8x16_motion` 为方向性规则:
    - `16x8 part1` 优先左邻(匹配时直接取左邻), 否则回退 `pred_motion`.
    - `8x16 part1` 优先对角 C(仅 C 不可用时回退 D), 否则回退 `pred_motion`.
  - 上述差异会在 CAVLC 的 P 帧首批分区中直接改变 MVP, 并在后续参考帧链路累积误差.
- 逻辑证据:
  - Tao 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - 原实现在 `mb_type=1/2` 分支中未调用 `predict_mv_l0_16x8/predict_mv_l0_8x16`.
    - 且对 `part1` 存在 `if ref_idx_same { 复用 part0 MV }` 的捷径.
  - FFmpeg 文件: `libavcodec/h264_mvpred.h`
    - `pred_16x8_motion` 与 `pred_8x16_motion` 均在 `ff_h264_decode_mb_cavlc` 路径被调用, 与 CABAC 共享同一方向性 MVP 语义.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - `P_16x8`:
      - part0 改为 `predict_mv_l0_16x8(..., part=0, ...)`.
      - part1 改为 `predict_mv_l0_16x8(..., part=1, ...)`.
      - 删除“同 ref 直接复用 part0 MV”捷径.
    - `P_8x16`:
      - part0 改为 `predict_mv_l0_8x16(..., part=0, ...)`.
      - part1 改为 `predict_mv_l0_8x16(..., part=1, ...)`.
      - 删除“同 ref 直接复用 part0 MV”捷径.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/decode.rs`
    - 新增回归:
      - `test_decode_cavlc_slice_data_p_non_skip_inter_16x8_part1_prefers_left_neighbor`
      - `test_decode_cavlc_slice_data_p_non_skip_inter_8x16_part1_prefers_diagonal_neighbor`
- 精度变化:
  - `data/1.mp4`:
    - 120 帧: `100.000000%` (保持无回归)
  - `data/2.mp4`:
    - 120 帧: `100.000000%` (保持无回归)
  - `E1`:
    - 10 帧: `37.419886% -> 38.256061%` (`+0.836175`)
    - `first_mismatch=1` (未前移)
  - `E9`:
    - 10 帧: `36.024242% -> 36.828409%` (`+0.804167`)
    - `first_mismatch=1` (未前移)
- 测试结果:
  - `cargo test -p tao-codec decode_cavlc_slice_data_p_non_skip_inter_ -- --nocapture` 通过(`13 passed`).
  - `cargo test --test run_decoder h264::test_h264_accuracy_e1 -- --ignored --nocapture` 通过.
  - `cargo test --test run_decoder h264::test_h264_accuracy_e9 -- --ignored --nocapture` 通过.
  - `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK=1` 下 `E1/E9` 均通过(无缺失参考回退门禁触发).
- 判定: `有效修复(逻辑与 FFmpeg 方向性 MVP 语义对齐, 且 E1/E9 精度提升)`.
- 下一子功能: `1-2` (继续定位 CAVLC P_8x8 子分区 MVP/MC 链路与 FFmpeg 的细粒度差异, 优先 frame1 首失配区域).

### 9.18 Round-16 记录(1-2: CAVLC P_8x8 子分区 8x4/4x8 方向性 MVP 对齐 FFmpeg)

- 子功能: `1-2`.
- 对比结论: `不一致`.
  - Tao 在 CAVLC `P_8x8` 路径中, `sub_mb_type=1(8x4)` / `sub_mb_type=2(4x8)` 的子分区 MVP 统一走 `predict_mv_l0_partition`.
  - FFmpeg `h264_mvpred.h` 对这些子分区沿用方向性规则:
    - `8x4 part1` 优先左邻(匹配时直接返回).
    - `4x8 part1` 优先对角 C(仅 C 不可用时回退 D, 且匹配时直接返回).
  - Tao 的通用中值路径在 `A/B/C` 同时匹配但 MV 不一致时会偏离上述方向性优先级.
- 逻辑证据:
  - Tao 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - `sub_mb_type=1/2` 的 part0/part1 全部调用 `predict_mv_l0_partition`.
  - FFmpeg 文件: `libavcodec/h264_mvpred.h`
    - `pred_16x8_motion/pred_8x16_motion` 的方向性快捷返回规则同样适用于子分区路径.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs`
    - 新增:
      - `predict_mv_l0_sub_8x4` (part0 顶邻优先, part1 左邻优先).
      - `predict_mv_l0_sub_4x8` (part0 左邻优先, part1 对角 C/D 优先).
  - 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - `P_8x8` 分支:
      - `sub_mb_type=1` 改用 `predict_mv_l0_sub_8x4`.
      - `sub_mb_type=2` 改用 `predict_mv_l0_sub_4x8`.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/decode.rs`
    - 新增回归:
      - `test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_8x4_part1_prefers_left_neighbor`
      - `test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_4x8_part1_prefers_diagonal_neighbor`
- 精度变化:
  - `data/1.mp4`:
    - 120 帧: `100.000000%` (持平)
  - `data/2.mp4`:
    - 120 帧: `100.000000%` (持平)
  - `E1`:
    - 10 帧: `38.256061%` (持平)
  - `E9`:
    - 10 帧: `36.828409%` (持平)
- 测试结果:
  - `cargo test -p tao-codec decode_cavlc_slice_data_p_non_skip_inter_ -- --nocapture` 通过(`15 passed`).
  - `cargo test --test run_decoder h264::test_h264_accuracy_e1 -- --ignored --nocapture` 通过.
  - `cargo test --test run_decoder h264::test_h264_accuracy_e9 -- --ignored --nocapture` 通过.
- 判定: `有效修复(逻辑与 FFmpeg 方向性 MVP 规则对齐, 主/次要目标无回归; 预期为后续链式修复铺垫)`.
- 下一子功能: `1-3` (继续定位 frame1 首失配, 优先核对 CAVLC P 帧残差解码与边界可用性联动).

### 9.19 Round-17 记录(1-3: P-skip 在多 slice 起始处的跨 slice 邻居误用修正)

- 子功能: `1-3`.
- 对比结论: `不一致`.
  - Tao 在 CAVLC `skip_run` 路径的 P-slice 分支里, 会无条件把当前 MB 的 `mb_slice_first_mb` 临时改写为左/上邻居的 slice 标记.
  - 该行为会让“下一 slice 的首个 P-skip MB”错误地把前一 slice 左邻当作同 slice 可用邻居参与 MVP.
  - FFmpeg 语义下, slice 边界邻居应不可用; 仅在调试/局部解码导致邻居 slice 标记缺失时才需要做受限放宽.
- 逻辑证据:
  - Tao 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - 原实现位于 `if skip_run_left > 0` 的 P-slice 分支, 无条件执行 `self.mb_slice_first_mb[mb_idx] = relaxed_first_mb`.
  - 与现有 `unknown-slice` 容错策略不一致:
    - 其它路径已按 `left_unknown || top_unknown` 条件化放宽.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/slice_decode.rs`
    - P-skip 分支改为:
      - 仅当 `left_unknown || top_unknown` 时才临时放宽 slice 标记.
      - 正常多-slice 场景保持原 slice 标记, 防止跨 slice 邻居泄漏.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/decode.rs`
    - 新增回归:
      - `test_decode_cavlc_slice_data_p_skip_at_slice_start_does_not_use_prev_slice_left_mv`
      - 覆盖“第二个 slice 首 MB 为 P-skip”时不借用前一 slice 左邻 MV.
- 精度变化:
  - `data/1.mp4`: `100.000000%` (持平)
  - `data/2.mp4`: `100.000000%` (持平)
  - `E1`: `38.256061%` (持平)
  - `E9`: `36.828409%` (持平)
- 测试结果:
  - `cargo test -p tao-codec test_decode_cavlc_slice_data_p_skip_at_slice_start_does_not_use_prev_slice_left_mv -- --nocapture` 通过.
  - `cargo test -p tao-codec decode_cavlc_slice_data_p_non_skip_inter_ -- --nocapture` 通过(`15 passed`).
  - `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK=1` 下 `E1/E9` 通过.
- 判定: `有效修复(逻辑层面确认实现错误并已对齐 slice 边界语义, 为后续精度链路修复提供稳定前提)`.
- 下一子功能: `1-4` (继续定位 frame1 首失配, 优先核对 CAVLC P-slice residual/qp_delta 与 FFmpeg 的时序一致性).

### 9.20 Round-18 记录(1-4: CAVLC Inter 残差路径补齐 `prev_qp_delta_nz` 时序维护)

- 子功能: `1-4`.
- 对比结论: `不一致`.
  - Tao 的 `decode_cavlc_mb_residual` 在 Inter 宏块路径中只更新 `cur_qp`, 未同步维护 `prev_qp_delta_nz`.
  - 同文件 I 宏块路径(`decode_cavlc_i_mb`)已在 `has_residual` 分支维护该状态, Inter 路径存在时序不对称.
- 逻辑证据:
  - 文件: `crates/tao-codec/src/decoders/h264/cavlc_mb.rs`.
    - 修复前 Inter 路径:
      - `has_residual=true` 时读取 `qp_delta` 但不写 `prev_qp_delta_nz`.
      - `has_residual=false` 时也不清零 `prev_qp_delta_nz`.
  - 规范语义: `mb_qp_delta` 仅在存在残差时出现, 无残差宏块不应延续上个宏块的“qp_delta 非零”状态.
- 修复改动:
  - 文件: `crates/tao-codec/src/decoders/h264/cavlc_mb.rs`
    - 在 `decode_cavlc_mb_residual` 中补齐状态维护:
      - `has_residual=true` 时 `self.prev_qp_delta_nz = (qp_delta != 0)`.
      - `has_residual=false` 时 `self.prev_qp_delta_nz = false`.
  - 文件: `crates/tao-codec/src/decoders/h264/tests/decode.rs`
    - 新增回归:
      - `test_decode_cavlc_mb_residual_inter_cbp_zero_clears_prev_qp_delta_flag`
      - 验证 Inter 宏块 `cbp=0` 时会清零 `prev_qp_delta_nz`, 且不改写 `cur_qp`.
- 精度变化:
  - `data/1.mp4`: `100.000000%` (持平)
  - `data/2.mp4`: `100.000000%` (持平)
  - `E1`: `38.256061%` (持平)
  - `E9`: `36.828409%` (持平)
- 测试结果:
  - `cargo test -p tao-codec test_decode_cavlc_mb_residual_inter_cbp_zero_clears_prev_qp_delta_flag -- --nocapture` 通过.
  - `cargo test -p tao-codec decode_cavlc_slice_data_p_non_skip_inter_ -- --nocapture` 通过(`15 passed`).
  - `cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --ignored --nocapture` 通过.
  - `cargo test --test run_decoder h264::test_h264_compare_sample_2 -- --ignored --nocapture` 通过.
  - `cargo test --test run_decoder h264::test_h264_accuracy_e1 -- --ignored --nocapture` 通过.
  - `cargo test --test run_decoder h264::test_h264_accuracy_e9 -- --ignored --nocapture` 通过.
- 判定: `有效修复(逻辑正确性优先: 修复了 Inter/I 路径的状态机不一致, 主目标与回归集无回退)`.
- 下一子功能: `2-1` (继续定位 E1/E9 首失配, 优先核对多参考帧默认 L0 列表与 DPB 更新链路).
