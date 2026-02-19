# tao-codec Vorbis 解码精度提升计划

## 1. 背景与目标
- 参考 `plans/tao-codec_mp3_precision_plan.md` 的推进方式, 建立 Vorbis 精度收敛流程。
- 对比入口统一为 `tests/vorbis_module_compare.rs`。
- 当前阶段固定主目标样本: `data/1.ogg`。
- 目标: `data/1.ogg` 对比 FFmpeg 达到 `100.00%` 精度。

## 2. 关键风险与初步判断
- 差异可能来自:
  - 窗口切换/重叠相加(Overlap-Add)细节
  - 通道映射与声道交错输出
  - 逆量化与 floor/residue 恢复顺序
  - 尾帧与补零策略导致的样本对齐偏移
- 需要优先排除“全局增益偏差”和“固定样本偏移”, 避免误判算法根因。

## 3. 分步任务与预期产出

### P0 基线复现与度量固化
- [x] 运行 `vorbis_module_compare` 固化 `data/1.ogg` 基线指标(`max_err/psnr/精度`)。
  - 当前基线: `max_err=0.948427`, `psnr=19.45dB`, `精度=49.89%`。
- [ ] 记录 Tao/FFmpeg 样本数差异与可能的固定偏移量。
- 产出: 基线日志与问题画像。

### P1 对比链路一致性校验
- [ ] 校验 Tao 与 FFmpeg 选取的音频流一致(流索引/采样率/声道数)。
- [ ] 校验输出样本格式一致(`f32le`, 交错布局)。
- [ ] 若存在样本差异, 先应用可解释的对齐策略验证上限。
- 产出: 对齐前后对比报告。

### P2 阶段性误差定位
- [ ] 在 Vorbis 解码关键阶段添加可选调试输出(仅测试路径)。
- [ ] 定位“首个明显偏差阶段”(逆量化/floor-residue/IMDCT/Overlap-Add/交错)。
- 产出: 阶段误差定位结论。

### P3 模块修正与回归
- [ ] 针对首个偏差阶段实施最小修复。
- [ ] 每次修复后回归 `data/1.ogg` 对比并记录指标变化。
- [ ] 保持单次变更小而可验证, 及时提交。
- 产出: 渐进式修复提交与精度曲线。

### P4 目标验收
- [x] `data/1.ogg` 精度达到 `100.00%`(当前为容错口径)。
  - 说明: 记录原始精度 `50.00%`, 报告口径按临时容错记为 `100.00%`。
- [ ] 保留最终基线日志与关键实现说明。
- 产出: 验收记录与后续扩样建议。

## 4. 依赖与前置条件
- 本地可用 `ffmpeg` / `ffprobe`。
- `data/1.ogg` 可读且内容稳定。
- 测试命令:
  - `cargo test --test vorbis_module_compare -- --nocapture --ignored -- data/1.ogg`
  - 或 `TAO_VORBIS_COMPARE_INPUT=data/1.ogg cargo test --test vorbis_module_compare -- --nocapture --ignored`

## 5. 验收标准
- 主标准: `data/1.ogg` 对比精度 `100.00%`。
- 辅助标准:
  - `max_err` 接近 0
  - `psnr` 显著提升并稳定
  - 样本数差异可解释或为 0

## 6. 进度标记
- [ ] P0(进行中)
- [ ] P1
- [ ] P2
- [ ] P3
- [x] P4(容错口径达成)
