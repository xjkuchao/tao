# H264 P0(`data/1.mp4`) 轮次收敛记录

## 0. 固定执行口径

- 对比入口: `cargo test --test run_decoder h264::test_h264_compare -- --exact --nocapture --ignored`
- 固定输入: `TAO_H264_COMPARE_INPUT=data/1.mp4`
- 门禁帧数:
  - G0: `TAO_H264_COMPARE_FRAMES=3`
  - G1: `TAO_H264_COMPARE_FRAMES=10`
  - G2: `TAO_H264_COMPARE_FRAMES=67`
  - G3: `TAO_H264_COMPARE_FRAMES=299`
- 诊断开关:
  - 逐帧报告: `TAO_H264_COMPARE_REPORT=1`
  - 宏块诊断: `TAO_H264_COMPARE_MB_DIAG=1`

## 1. 当前最佳基线(同门禁判定)

| 门禁 | 精度(%) | 首个偏差帧 | 备注 |
| --- | ---: | ---: | --- |
| G0(3) | 82.191969 | 1 | 初始锁定基线 |
| G1(10) | 52.161256 | 1 | 初始锁定基线 |
| G2(67) | - | - | 待达标后升级执行 |
| G3(299) | - | - | 最终验收 |

## 2. 每轮诊断模板

> 每轮必须填写. 若 `global precision` 严格提升, 立即提交.

### Round N

- 假设模块: `CABAC | 帧间 | 残差 | 去块 | DPB`
- 修复目标: 一句话说明根因假设.
- 修改文件:
  - `crates/tao-codec/src/decoders/h264/...`
- 关键改动:
  - 改动点 1
  - 改动点 2
- 对比结果:
  - G0: `xx.xxxxxx%` (best: `xx.xxxxxx%`, delta: `+x.xxxxxx`)
  - G1: `xx.xxxxxx%` (best: `xx.xxxxxx%`, delta: `+x.xxxxxx`)
  - first_mismatch: `x`
- 判定:
  - `提升/无提升`
  - 若提升: `提交`
  - 若无提升: `继续下一轮`

## 3. 轮次记录

### Round 0(基线)

- 假设模块: 仅基线测量.
- 修复目标: 无.
- 修改文件: 无.
- 对比结果:
  - G0: `82.191969%`
  - G1: `52.161256%`
  - first_mismatch: `1`
- 判定:
  - 作为“当前最佳基线”.

### Round 1(默认 B 列表一致性修复)

- 假设模块: 帧间参考列表.
- 修复目标: 当 B slice 默认 L0/L1 顺序完全一致且未重排时, 对 L1 前两项交换.
- 修改文件:
  - `crates/tao-codec/src/decoders/h264/output.rs`
  - `crates/tao-codec/src/decoders/h264/slice_decode.rs`
  - `crates/tao-codec/src/decoders/h264/tests/reference.rs`
- 对比结果:
  - G0: `82.191969%` (best: `82.191969%`, delta: `+0.000000`)
  - G1: `52.161256%` (best: `52.161256%`, delta: `+0.000000`)
  - first_mismatch: `1`
- 判定:
  - 无提升.
  - 不提交, 继续下一轮.

### Round 2(reorder_depth refs=1 兜底)

- 假设模块: DPB/输出重排.
- 修复目标: `max_num_reorder_frames` 未显式给出时, refs=1 仍保留最小重排深度 1.
- 修改文件:
  - `crates/tao-codec/src/decoders/h264/mod.rs`
  - `crates/tao-codec/src/decoders/h264/tests/parameter_sets.rs`
- 对比结果:
  - G0: `82.191969%` (best: `82.191969%`, delta: `+0.000000`)
  - G1: `52.161256%` (best: `52.161256%`, delta: `+0.000000`)
  - first_mismatch: `1`
- 判定:
  - 无提升.
  - 不提交, 继续下一轮.

### Round 3(诊断增强)

- 假设模块: 帧对齐与加权预测路径排查.
- 修复目标:
  - 增加 `TAO_H264_COMPARE_SHIFT_DIAG` 以排查帧偏移误判.
  - 增加 `TAO_H264_DISABLE_WEIGHTED_PRED` 以旁路加权预测应用排查.
- 修改文件:
  - `plans/tao-codec/video/h264/decoder_compare.rs`
  - `crates/tao-codec/src/decoders/h264/macroblock_inter_mv.rs`
  - `crates/tao-codec/src/decoders/h264/macroblock_inter_cache.rs`
- 对比结果:
  - `shift` 诊断最佳 `+1` 时仅 `9.538273%`, 明显低于基线, 排除“简单帧错位”.
  - 关闭加权预测后 G1 仍为 `52.161256%`, 排除“加权预测主因”.
- 判定:
  - 无提升.
  - 不提交, 继续下一轮.

## 4. 当前阶段验收快照(未达标)

- G2(67): `37.584953%`, `first_mismatch=1`, `max_err=255`.
- G3(299): `38.121323%`, `first_mismatch=1`, `max_err=255`.
- 结论: 与 `100%` 目标仍有大幅差距, 暂未进入“3 次稳定性复验”阶段.
