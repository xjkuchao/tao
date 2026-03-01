# 背景与目标

在 `data/h264_samples/e4_main_cabac_lowres.mov` 的 H.264 对比中, Tao 曾出现明显精度回归(`89.765092%`)。  
定位结果指向 CAVLC `P_8x8/P_8x8ref0` 路径中, 子分区解码时 `ref_cache` 写入时序与 CABAC 路径不一致, 导致 MVP 邻居候选被“未来子分区”状态污染。

目标:

- 修复 CAVLC `P_8x8/P_8x8ref0` 子分区 `ref_cache` 写入时序。
- 恢复 `e4` 样本精度到 `100%`。
- 确保 `data/2.mp4` 不回归。

# 分步任务与预期产出

1. 复现与定位
预期产出:
- 复现日志(`e4` 精度 < 100%)。
- 定位到 `slice_decode.rs` CAVLC P_8x8 分支的时序差异。

2. 最小修复
预期产出:
- 在 `crates/tao-codec/src/decoders/h264/slice_decode.rs` 实施最小改动:
  - 先写每个 8x8 子分区 `[1]/[8]/[9]`。
  - `[0]` 先置 `-2(PartNotAvailable)`。
  - 真正解码该子分区前再写 `[0]=ref_idx`。

3. 回归验证
预期产出:
- `e4` 对比 `100%`。
- `data/2.mp4` 对比 `100%`。
- `test_h264_accuracy_e4` 通过。
- 记录 `data/1.mp4` 当前基线状态, 与本修复影响分离。

# 依赖与前置条件

- 本地存在样本:
  - `data/h264_samples/e4_main_cabac_lowres.mov`
  - `data/2.mp4`
  - `data/1.mp4`
- 可执行测试:
  - `cargo test --test run_decoder ... --ignored`

# 验收标准

- `e4` 30 帧对比: `SCORE precision=100.000000`。
- `data/2.mp4` 30 帧对比: `SCORE precision=100.000000`。
- `cargo test --test run_decoder h264::test_h264_accuracy_e4 -- --ignored` 通过。
- 改动范围限定在 CAVLC P_8x8 时序逻辑, 不引入无关修改。

# 进度标记

- [x] 步骤1 复现与定位完成。
- [x] 步骤2 最小修复完成。
- [x] 步骤3 核心回归验证完成(`e4`/`data2`/`accuracy_e4`)。
- [ ] 后续项: 继续单独跟进 `data/1.mp4` 的历史微差(`±1` 级别), 与本修复解耦处理。
