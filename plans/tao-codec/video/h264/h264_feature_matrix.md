# H264 功能点矩阵

## 1. 说明

- 本矩阵用于驱动 `decoder_dev.md` 的 P1-P6 阶段。
- 强制门禁: `P1-P6` 任一项未完成时, 禁止执行精度收敛阶段(`P7`)相关对比结论。
- 状态定义:
    - `未实现`: 代码路径不存在或不可用。
    - `部分`: 已有代码, 但行为不完整或仅覆盖子集。
    - `完成`: 功能逻辑已实现。
    - `已验证`: 完成且有对应自测/集成测试覆盖。

## 2. 功能矩阵

| 分类     | 功能点                             | 状态   | 代码位置                                                  | 备注                                          |
| -------- | ---------------------------------- | ------ | --------------------------------------------------------- | --------------------------------------------- |
| NAL/输入 | AVCC NAL 拆分                      | 已验证 | `crates/tao-codec/src/parsers/h264/nal.rs`                | 单元测试 + 样本解码                           |
| NAL/输入 | AnnexB NAL 拆分                    | 已验证 | `crates/tao-codec/src/parsers/h264/nal.rs`                | 单元测试 + 样本解码                           |
| NAL/输入 | AVCC/AnnexB 双入口自动识别         | 已验证 | `crates/tao-codec/src/decoders/h264/mod.rs`               | `tests/h264_functional_pipeline.rs`           |
| 参数集   | SPS 解析                           | 已验证 | `crates/tao-codec/src/parsers/h264/sps.rs`                | 含 VUI/scaling list/max_dec_frame_buffering   |
| 参数集   | PPS 解析                           | 已验证 | `crates/tao-codec/src/decoders/h264/parameter_sets.rs`    | 含 transform_8x8/scaling list                 |
| 参数集   | 参数集变更重建上下文               | 已验证 | `crates/tao-codec/src/decoders/h264/mod.rs`               | handle_pps/handle_sps 切换集成单测            |
| 参数集   | SEI 解析                           | 完成   | `crates/tao-codec/src/decoders/h264/sei.rs`               | recovery_point/pic_timing/buffering_period    |
| Slice    | I/P/B slice header 关键字段        | 已验证 | `crates/tao-codec/src/decoders/h264/slice_parse.rs`       | ref_pic_list_modification/weighted_pred/MMCO  |
| Slice    | 多 slice 同帧拼装                  | 已验证 | `crates/tao-codec/src/decoders/h264/slice_decode.rs`      | 跨包 pending frame 拼帧                       |
| 熵解码   | CABAC 基础引擎                     | 已验证 | `crates/tao-codec/src/decoders/h264/cabac.rs`             | decode_decision/bypass/terminate              |
| 熵解码   | CABAC I-slice 路径                 | 已验证 | `crates/tao-codec/src/decoders/h264/`                     | I_4x4/I_8x8/I_16x16/I_PCM                     |
| 熵解码   | CABAC P/B-slice 路径               | 已验证 | `crates/tao-codec/src/decoders/h264/`                     | mb_type/sub_mb_type/mvd/ref_idx/cbp/residual  |
| 熵解码   | CABAC MVD 上下文 (amvd)            | 完成   | `macroblock_inter_weight.rs`, `macroblock_inter_cache.rs` | mvd_cache + 邻居 MVD 绝对值之和               |
| 熵解码   | CAVLC 路径                         | 已验证 | `crates/tao-codec/src/decoders/h264/cavlc.rs`             | coeff_token/level/total_zeros/run_before      |
| 帧内重建 | I_4x4 预测                         | 完成   | `crates/tao-codec/src/decoders/h264/intra.rs`             | 9 种预测模式                                  |
| 帧内重建 | I_8x8 预测                         | 完成   | `crates/tao-codec/src/decoders/h264/intra.rs`             | 9 种预测模式                                  |
| 帧内重建 | I_16x16 预测 + 残差                | 完成   | `crates/tao-codec/src/decoders/h264/intra.rs`             | 4 种预测模式 + DC/AC Hadamard                 |
| 帧内重建 | Chroma 预测                        | 完成   | `crates/tao-codec/src/decoders/h264/intra.rs`             | 4 种色度预测模式                              |
| 帧间重建 | P 帧运动向量预测                   | 完成   | `macroblock_inter_mv.rs`                                  | median + 16x8/8x16 方向性预测                 |
| 帧间重建 | B 帧运动向量预测                   | 完成   | `macroblock_inter_mv.rs`, `macroblock_inter.rs`           | Spatial/Temporal Direct + col_zero_flag       |
| 帧间重建 | P_Skip/B_Skip                      | 完成   | `macroblock_inter.rs`                                     |                                               |
| 帧间重建 | P_8x8 子分区                       | 完成   | `macroblock_inter_weight.rs`                              | 8x8/8x4/4x8/4x4                               |
| 帧间重建 | B_8x8 子分区                       | 完成   | `macroblock_inter_cache.rs`                               | Direct/L0/L1/Bi 各子类型                      |
| 帧间重建 | 运动补偿(Luma 6-tap Qpel)          | 完成   | `common.rs`                                               | 16 种 qpel 位置                               |
| 帧间重建 | 运动补偿(Chroma 双线性)            | 完成   | `common.rs`                                               | 1/8 精度双线性插值                            |
| 帧间重建 | 加权预测(显式/隐式/默认)           | 完成   | `macroblock_inter_weight.rs`, `macroblock_inter_cache.rs` | weighted_bipred_idc=0/1/2                     |
| 残差     | 4x4 IDCT + 反量化                  | 完成   | `residual.rs`                                             | 含自定义缩放矩阵                              |
| 残差     | 8x8 IDCT + 反量化                  | 完成   | `residual.rs`                                             | 含自定义缩放矩阵                              |
| 残差     | Luma DC Hadamard (I_16x16)         | 完成   | `residual.rs`                                             | 4x4 逆 Hadamard                               |
| 残差     | Chroma DC Hadamard                 | 完成   | `residual.rs`                                             | 2x2 逆 Hadamard                               |
| 残差     | Transform bypass (QP=0)            | 完成   | `residual.rs`                                             | qpprime_y_zero_transform_bypass               |
| 输出     | DPB 管理(短期/长期)                | 完成   | `output.rs`                                               | 滑窗 + MMCO op1-op6                           |
| 输出     | POC 计算 (type 0/1/2)              | 完成   | `output.rs`                                               |                                               |
| 输出     | 显示重排                           | 完成   | `output.rs`                                               | reorder_buffer + Level 查表推导 reorder_depth |
| 输出     | max_dec_frame_buffering            | 完成   | `sps.rs`, `mod.rs`                                        | 存入 SPS 并约束 DPB 容量                      |
| 输出     | gaps_in_frame_num 处理             | 完成   | `output.rs`                                               | 插入非存在短期参考帧                          |
| 后处理   | 去块滤波                           | 完成   | `deblock.rs`                                              | BS 计算 + alpha/beta/tc0 表 + 强/弱滤波       |
| 后处理   | 去块 disable_deblocking_filter_idc | 完成   | `deblock.rs`                                              | idc=0/1/2 三种模式                            |
| 稳定性   | 损坏数据容错与边界保护             | 完成   | `mod.rs`                                                  | malformed NAL 丢弃 + MB 级异常恢复            |
| 验收     | 两个样本连续帧无中断               | 已验证 | `tests/h264_functional_pipeline.rs`                       | sample1>=299, sample2>=300                    |

## 3. 非目标范围 (不影响当前验收)

- MBAFF (宏块级自适应场帧编码): 不支持
- 场编码 (`frame_mbs_only_flag=0`): 不支持
- 高位深 (>8-bit): 不支持
- Profile 144 (旧版 High 4:4:4): 不支持
- 非 4:2:0 色度格式: 不支持

## 4. 下一步: 精度收敛阶段 (P7)

1. 使用 FFmpeg 逐帧像素级对比, 定位并修复剩余偏差。
2. 运行多组样本回归测试, 确认各功能点无退化。
3. 性能优化: 热路径 SIMD 加速, 减少内存分配。
