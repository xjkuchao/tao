# H264 调试清理与性能收敛计划

## 1. 背景与目标

- 背景: 当前 H264 相关路径累计了大量调试期日志、环境变量开关与临时分支, 在热路径中引入了额外开销, 且影响代码可维护性.
- 目标:
  - 清理调试阶段新增的日志与临时代码.
  - 修复可确认的热路径低效逻辑.
  - 保持 `data/1.mp4`、`data/2.mp4` 与次级样本集(C1-C3/E1-E9/X1-X4)精度不滑落, 维持 `100%`.

## 2. 分步任务与预期产出

1. 盘点调试代码与性能热点.
   - 产出: 调试开关/日志清单与热路径低效点清单.
2. 清理调试日志与临时代码.
   - 产出: 删除或收敛 `eprintln!/trace env var` 相关分支, 保留必要的稳定诊断接口.
3. 热路径性能修正.
   - 产出: 减少 MB/子块级重复 `std::env::var` 读取、消除不必要分支/分配.
4. 回归验证.
   - 产出:
     - `h264::test_h264_compare_sample_1` 通过且 100%.
     - `h264::test_h264_compare_sample_2` 通过且 100%.
     - `h264::test_h264_accuracy_all` 16/16 通过且全部 100%.

## 3. 依赖与前置条件

- 分支: `h264`.
- CodeGraph 可用且已初始化.
- 本地可执行 `ffmpeg/ffprobe`.

## 4. 验收标准

- 调试临时代码明显减少, 热路径不再频繁读取环境变量.
- 代码可编译并通过:
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo check --workspace --all-targets --all-features`
- 样本精度保持:
  - `data/1.mp4`、`data/2.mp4` 为 `100.000000%`.
  - 次级样本(C1-C3/E1-E9/X1-X4)全量为 `100.000000%`.

## 5. 进度标记

- [x] 建立计划文件.
- [x] 回滚到精度基线并确认全样本 100%.
- [x] 小块 1: `macroblock_inter.rs` 清理 P slice trace/env.
- [x] 小块 2: `macroblock_inter_cache.rs` 清理 B 详情 trace 与残差跳过调试开关.
- [x] 小块 3: `macroblock_inter_mv.rs` 清理 L1 MVP trace.
- [x] 小块 4: `macroblock_inter_mv.rs` 去除 16x8/8x16 方向性 MVP 热路径 env 读取.
- [x] 小块 5: `output.rs` 清理参考列表/输出/负参考 MV trace 日志.
- [x] 小块 6: `output.rs` 清理 `TAO_H264_DISABLE_REF_MOD*` 与 `TAO_H264_DISABLE_MMCO` 调试开关.
- [x] 小块 7: `macroblock_inter_weight.rs` 清理 B slice trace 入口.
- [x] 小块 8: `macroblock_inter_weight.rs` 去除 `decode_p_inter_mb` 的 `TAO_H264_TRACE_P_MB_DETAIL` env 解析.
- [x] 小块 9: `macroblock_inter_weight.rs` 去除 `TAO_H264_SKIP_*_MB` 调试开关路径.
- [x] 小块 10: `macroblock_inter_weight.rs` 去除 `decode_p_inter_mb` 入口处 P 详情日志构建.
- [x] 小块 11: `macroblock_inter_weight.rs` 将 `trace_motion` 收敛为 no-op, 移除该路径日志输出.
- [x] 小块回滚: 尝试移除 `macroblock_inter_mv.rs` 的 `TAO_H264_USE_DIR_SUB_8X4/4X8` 后 `E4` 回退到 95.94%, 已立即回滚该小块并恢复 100%.
- [x] 小块 12: `macroblock_inter_weight.rs` 删除 `decode_p_inter_mb` 中固定关闭的 `trace_this_mb/eprintln!` 分支.
- [x] 小块 13: `macroblock_inter_weight.rs` 删除 `trace_motion` no-op 闭包与所有调用(含临时 `format!`).
- [x] 小块回滚: 尝试将 `predict_mv_l0_sub_8x4` 固化为方向性预测后, `E4` 回退到 `98.009736%`(`max_err=123`), 已立即回滚并恢复 100%.
- [x] 小块 14: `slice_decode.rs` 删除 `TAO_H264_TRACE_SLICE_HDR` 头部日志分支.
- [x] 小块回滚: 尝试将 `predict_mv_l0_sub_4x8` 固化为方向性预测后, `E4` 回退到 `96.324414%`(`max_err=202`), 已立即回滚并恢复 100%.
- [x] 小块 15: `slice_decode.rs` 删除 `trace_cavlc_break` 与 `preview_remaining_bits` 断点日志链路.
- [x] 小块 16: `cavlc.rs` 删除 `TAO_H264_TRACE_CAVLC_START_BIT` 及整段残差块 trace 输出.
- [x] 小块 17: `macroblock_intra.rs` 删除 `TAO_H264_TRACE_CABAC_I_MB` 与 I 宏块详情输出.
- [x] 小块 18: `macroblock_intra.rs` 清理 I4x4 预测 trace 输出与热路径追踪逻辑; 保留 `should_trace_i4x4_block` 空实现和 `read_luma_4x4_block` 供 `cavlc_mb.rs` 编译依赖.
- [x] 小块 19: `cavlc.rs` 删除 `TAO_H264_CAVLC_TRACE_FALLBACK` 日志门控与原子计数器.
- [x] 小块 20: `slice_decode.rs` 将 `trace_cavlc_summary/mb/mb_range/i_stats` 固化为关闭, 去除热路径 env 读取.
- [x] 小块 21: `slice_decode.rs` 将两处 `trace_this_mb` 固化为 `false`, 移除无意义目标匹配计算.
- [x] 小块 22: `slice_decode.rs` 删除 I-slice `trace_this_mb` 日志块及关联局部变量.
- [x] 小块 23: `cavlc_mb.rs` 删除 `decode_cavlc_residual_block_or_zero` 的 `TAO_H264_TRACE_CAVLC_ERRORS` 错误日志链路.
- [x] 小块 24: `cavlc_mb.rs` 清理小块 23 引入的未使用参数告警.
- [x] 小块 25: `cavlc_mb.rs` 删除 `decode_cavlc_luma_dc` 的 `TAO_H264_FORCE_I16_DC_NC0` 调试开关与 trace 日志.
- [x] 小块 26: `cavlc_mb.rs` 删除 `decode_cavlc_mb_residual` 中固定关闭的 `trace_this_mb/trace_pixels` 日志分支.
- [x] 小块 27: `cavlc_mb.rs` 删除已无调用的 `trace_cavlc_mb_pixels_enabled/trace_cavlc_luma_mb_block`.
- [x] 小块 28: `cavlc_mb.rs` 删除 `decode_cavlc_chroma_residual` 全部 `trace_this_mb` 日志输出.
- [x] 小块 29: `cavlc_mb.rs` 删除 `decode_cavlc_i16x16_luma_residual` 中 `bits_before/nc_dbg/raw_dbg/final_block` 等调试变量与日志.
- [x] 小块 30: `cavlc_mb.rs` 删除 `decode_cavlc_i_mb` 的 `trace_this_mb` 日志分支(`H264-CAVLC-I/I16`).
- [x] 小块 31: `cavlc_mb.rs` 删除已无调用的 `trace_cavlc_target_mb`.
- [x] 小块 32: `cavlc_mb.rs` 删除 `decode_cavlc_i_mb` 恒为 false 的 `restore_*` 回写路径与大块临时缓冲拷贝.
- [x] 小块 33: `cavlc_mb.rs` 删除已无调用的 `debug_restore_luma_after_residual/debug_restore_chroma_after_residual`.
- [x] 小块 34: `cavlc_mb.rs` 删除 `decode_cavlc_i4x4_luma_residual` 中 `should_trace_i4x4_block` 相关日志与仅日志变量.
- [x] 小块 35: 删除 `cavlc_mb.rs` 未使用 `mb_idx` 与 `macroblock_intra.rs` 已无调用 `should_trace_i4x4_block`.
- [x] 小块 36: `slice_decode.rs` 删除 B_8x8 残差后的 `H264-CAVLC-MB` trace 块.
- [x] 小块 37: `slice_decode.rs` 删除 B_16x8/B_8x16 分支 `H264-CAVLC-B16` 与后续 `H264-CAVLC-MB` trace.
- [x] 小块 38: `slice_decode.rs` 删除 B/P 共用分支收尾 `H264-CAVLC-MB` 以及 `P16x16` trace.
- [x] 小块 39: `slice_decode.rs` 删除 `P8x8/P8x4/P4x8/P4x4/default` 全部子分区 trace.
- [x] 小块 40: `slice_decode.rs` 删除残差后 `skip/zero/skip_luma/skip_chroma` trace 与循环末尾 `H264-CAVLC-MB` trace.
- [x] 小块 41: `slice_decode.rs` 删除 `B16x16` 仅日志变量(`l0_pred/l1_pred/l*_mvd/l*_mv`)并移除 CAVLC summary 统计与输出链路(`trace_cavlc_summary`, `processed_mbs`, `p_*_cnt`).
- [x] 小块 42: `slice_decode.rs` 删除 `TAO_H264_FORCE_B_L1_REF_MB` / `TAO_H264_FORCE_B_L1_MV_MB` 两个按 MB 解析的调试覆盖路径.
- [x] 小块 43: `slice_decode.rs` 删除按 MB 读取的目标调试覆盖链路(`parse_target_mb/frame` 与 `TAO_H264_SKIP*/ZERO*/FORCE_ZERO*/FORCE_SLICE_QP`), 并固化 `relax_unknown_neighbors` 为默认语义.
- [x] 小块 44: `slice_decode.rs` 删除恒 false 的 `force_zero_* / skip_residual_* / force_slice_qp_*` 调试分支与相关冗余拷贝逻辑.
- [x] 小块 45: `macroblock_intra.rs` 删除 `TAO_H264_DISABLE_I4X4_TR_FIX` 与 `TAO_H264_SKIP_CHROMA_DETAIL` 调试开关分支.
- [x] 小块 46: `mod.rs` 删除 `TAO_H264_FORCE_CONSTRAINED_INTRA_PRED` 热路径 env 覆盖与 `send_packet` 每包刷新调试开销.
- [x] 小块回滚: 尝试将 `common.rs` 与 `macroblock_inter_mv.rs` 调试开关固化后, `E4` 回退到 `95.941585%`(`max_err=202`), 已立即回滚并恢复.
- [x] 小块 47: `mod.rs` 将缺参考/ref_idx 越界/mvd 溢出的 3 个调试失败开关固定为默认关闭.
- [x] 小块 48: `cavlc.rs` 将 coeff_token/total_zeros fallback 的调试 env 开关固定为默认关闭.
- [x] 小块 49: `macroblock_inter_mv.rs` 将 `TAO_H264_USE_DIR_SUB_8X4/4X8` 从热路径 `env::var` 改为解码器级缓存字段.
- [x] 小块 50: `common.rs` 将 `TAO_H264_USE_LUMA_BILINEAR` 从每次采样读取改为 `OnceLock` 一次读取.
- [x] 小块 51: `mod.rs` 删除已固定关闭的 `disable_weighted_pred/skip_deblock` 运行时状态字段, 相关接口改为常量返回.
- [x] 小块 52: 移除 `TAO_H264_USE_DIR_SUB_8X4/4X8` 运行时环境变量通道, 默认行为固定为关闭.
- [x] 小块 53: `tests/decode.rs` 删除 `ScopedEnvVar` 与环境变量依赖, 改为 `#[cfg(test)]` 显式开关 `set_dir_sub_mv_predictor_for_test`.
- [x] 小块 54: `tests/helpers.rs` 同步 `H264Decoder` 新字段初始化.
- [x] 小块 55: `common.rs` 删除最后一个 `TAO_H264_USE_LUMA_BILINEAR` 调试开关, qpel 固定标准路径.
- [x] 小块 56: `macroblock_inter_mv.rs/macroblock_inter_cache.rs/output.rs` 删除 `weighted_pred_disabled/skip_deblock_by_env` 常量 false 相关无效分支.
- [x] 小块 57: `mod.rs` 删除 `weighted_pred_disabled/skip_deblock_by_env` 无效接口, 进一步收敛调试遗留代码.
- [x] 小块 58: `h264` 目录调试环境变量入口清零(`std::env::var/TAO_H264_*` 检索为空).
- [x] 小块 59: `mod.rs` 将 `*_from_env` 命名清理为 `reset_*`, 消除与实现不符的命名遗留.
- [x] 小块 60: 回归验证新增 `tao-codec` 受影响单测 2 条均通过.
- [x] 小块 61: 完成本轮 `h264` 目录调试开关与无效分支收尾清理, 样本精度保持 100%.
- [x] 小块 62: `output.rs` 删除 `apply_ref_pic_list_modifications` 中无效 `let _ = cur_frame_num`.
- [x] 小块 63: `output.rs` 避免每帧克隆 `DecRefPicMarking`, 改为按值字段读取 + 按索引复制 `MmcoOp`.
- [x] 小块 64: `slice_decode.rs` 将 `dec_ref_pic_marking` 从 `clone` 改为 `std::mem::take` 转移, 避免每个 slice 额外分配/复制.
- [x] 小块 65: `mod.rs` 在 `activate_parameter_sets` 中移除 `prev_pps` 整体克隆, 改为借用 `self.pps` 计算 `rebuild_action`.
- [x] 小块 66: `output.rs` 清理 `apply_ref_pic_list_modifications` 中恒等的重复截断分支, 减少无效判断.
- [x] 小块 67: `slice_decode.rs` 删除 CABAC 路径中无效占位变量 `_num_ref_idx_l1`.
- [x] 小块 68: `slice_decode.rs` 删除 `computed_poc` 中间变量并直接写入 `last_poc`.
- [x] 质量门禁全量回归(`fmt/clippy/check/test/doc`).
- [ ] 小块 69+: 继续推进剩余性能收敛.

## 6. 本轮结果

- 回滚后基线验证:
  - `cargo test --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored` 通过.
  - 16/16 样本全部 `100.000000%`(含 `E4`).
- 分块清理策略:
  - 每完成一小块清理, 立即执行:
    - `h264::test_h264_accuracy_all`
    - `h264::test_h264_compare_sample_1`(`data/1.mp4`)
    - `h264::test_h264_compare_sample_2`(`data/2.mp4`)
  - 若出现任一样本精度回退, 立即回滚该小块并修复后重试.
- 当前状态:
  - 已完成 63 个小块清理, 并按块执行 16 样本 + `data/1.mp4` + `data/2.mp4` 回归.
  - 发生 4 次小块回退(E4=95.94%, E4=98.009736%, E4=96.324414%, E4=95.941585%), 均已按流程即时回滚并恢复.
  - 当前最新回归结果:
    - 16/16 样本全部 `100.000000%`.
    - `data/1.mp4` 为 `100.000000%`.
    - `data/2.mp4` 为 `100.000000%`.
  - 小块 62/63 的增量 16+2 验证均保持 `100.000000%`.
  - 小块 64 的增量 16+2 验证保持 `100.000000%`.
  - 小块 65 的增量 16+2 验证保持 `100.000000%`.
  - 小块 66 的增量 16+2 验证保持 `100.000000%`.
  - 小块 67 的增量 16+2 验证保持 `100.000000%`.
  - 小块 68 的增量 16+2 验证保持 `100.000000%`.
  - 提交前全量门禁已通过:
    - `cargo fmt --all -- --check`
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    - `cargo check --workspace --all-targets --all-features`
    - `cargo test --workspace --all-targets --all-features --no-fail-fast`
    - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
  - 新增受影响单测已通过:
    - `test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_8x4_part1_prefers_left_neighbor`
    - `test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_4x8_part1_prefers_diagonal_neighbor`
  - `crates/tao-codec/src/decoders/h264` 下已无 `std::env::var` / `TAO_H264_*` / `TAO_SKIP_DEBLOCK` 调试开关残留.
