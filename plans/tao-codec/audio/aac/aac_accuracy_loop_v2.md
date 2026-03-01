# AAC 解码器精度提升计划 (Loop v2 — 结构化重写优先)

## 1. 目标

- **P0**: `data/1.m4a` 精度 ≥ 99.99% (当前 99.883833%), `data/2.m4a` 保持 ≥ 99.999% (当前 99.999998%)
- **P1**: `plans/tao-codec/audio/aac/coverage/report.md` 114 条样本整体改善

## 2. 策略

放弃纯参数探针路径 (已在 v1 的 60 轮中穷尽), 转向**结构化重写 + 逐功能对比修复循环**.

## 3. 功能大块划分

### 功能点 0: 代码清理与基线建立

- **0-1**: 清除所有残留环境变量探针 (`TAO_AAC_TNS_REVERSE_DIR_PROBE`, `TAO_AAC_LS_BOUNDARY_SHIFT`, `TAO_AAC_LS_INDEX_SHIFT`, `TAO_AAC_LSTOP_BOUNDARY_SHIFT`, `TAO_AAC_LSTOP_INDEX_SHIFT`, `read_window_shift` 函数), 恢复为固定常量路径
- **0-2**: 建立干净代码基线, 实测 P0 双样本指标并记录到 `aac_accuracy_round_log.md`

### 功能点 1: IMDCT 变换精度 (最高优先级)

当前 Tao 使用 O(N²) 朴素 IMDCT (1024 次余弦求和/样本), FFmpeg 使用 FFT-based split-radix IMDCT. 数值稳定性差异是分布式残差的最可能根因.

- **1-1**: 实现 N/4 点复数 FFT 基础设施 (radix-2 蝶形运算)
- **1-2**: 实现基于 FFT 的 IMDCT-1024 (pre-twiddle → N/4 FFT → post-twiddle)
- **1-3**: 实现基于 FFT 的 IMDCT-128 (同构 short 版本)
- **1-4**: 数值一致性验证: 新旧 IMDCT 输出误差 < 1e-6, P0 指标变化评估

### 功能点 2: 窗函数与加窗路径

- **2-1**: KBD 窗构建函数与 FFmpeg `ff_kbd_window_init` 的数值逐点对比
- **2-2**: Sine 窗构建函数精度验证
- **2-3**: `apply_aac_long_window` 中 LONG_START/LONG_STOP 过渡区窗口索引与 FFmpeg 逐行对比
- **2-4**: `synthesize_short_windows` 窗口乘法与 FFmpeg `vector_fmul_window` 逐行对比

### 功能点 3: Overlap-Add 状态机重写 (核心结构变更)

当前 Tao 采用"统一保存 windowed[1024..2048]"的简化方案; FFmpeg 按 window_sequence 分支管理 saved buffer 并使用 pairwise overlap.

- **3-1**: 逐行对比 FFmpeg `imdct_and_windowing` 的 ONLY_LONG 路径, 确认 saved buffer 语义
- **3-2**: 逐行对比 FFmpeg LONG_START 路径: IMDCT → 加窗 → 输出前半 → saved 更新方式
- **3-3**: 逐行对比 FFmpeg LONG_STOP 路径: saved 拼接 → IMDCT 加窗 → 输出
- **3-4**: 逐行对比 FFmpeg EIGHT_SHORT_SEQUENCE 路径: pairwise overlap + saved 管理
- **3-5**: 实现 FFmpeg 同构 overlap-add 状态机 (在 `decode_raw_frame` 中替换现有逻辑)
- **3-6**: P0 回归验证与指标记录

### 功能点 4: 频谱重建工具链

- **4-1**: Huffman 解码 + 反量化 (`inverse_quantize`) 与 FFmpeg `decode_spectrum_and_dequant` 逐行对比
- **4-2**: PNS 噪声重建 (`NOISE_HCB` 路径) 与 FFmpeg `apply_channel_coupling` / PNS 路径对比
- **4-3**: MS Stereo (`apply_ms_stereo`) 与 FFmpeg `apply_mid_side_stereo` 对比
- **4-4**: Intensity Stereo (`apply_intensity_stereo`) 与 FFmpeg `apply_intensity_stereo` 对比
- **4-5**: Pulse Tool (`apply_pulse_data`) 与 FFmpeg `apply_pulse` 对比
- **4-6**: TNS 滤波 (`apply_tns_data` + `compute_tns_lpc`) 与 FFmpeg `apply_tns` 对比

### 功能点 5: 语法解析与声道管理

- **5-1**: `raw_data_block` 解析入口: SCE/CPE/LFE/FIL/DSE/PCE 与 FFmpeg 对比
- **5-2**: `ics_info` 解析 (window_sequence, max_sfb, grouping) 与 FFmpeg 对比
- **5-3**: common_window CPE 工具链顺序: MS → IS → TNS 时序与 FFmpeg 对比
- **5-4**: 声道映射与输出交错顺序验证
- **5-5**: 首包裁剪 / 尾包 duration 截断语义验证

### 功能点 6: 覆盖率回归与鲁棒性

- **6-1**: P1 全量回归 (114 条样本)
- **6-2**: 失败样本分类与修复 (如 audioObjectType=1 等)
- **6-3**: 损坏流容错路径验证

## 4. 循环流程 (每个子功能)

```
第一步: 仔细对比 Tao 与 FDK-AAC/FFmpeg 源码差异
第二步: 如有差异 → 第三步; 如确认一致 → 第六步
第三步: 修复差异, 运行精度对比测试
第四步: 有效修复 → 提交代码, 进入第六步
第五步: 无效修复 → 恢复代码, 进入第六步
第六步: 切换下一个子功能, 返回第一步
```

全子功能遍历后: 若整体仍未达标, 重新拆分子功能进入下一轮.

## 5. 有效修复判定规则

### 5.1 逻辑正确性优先

- 若对比规范/FFmpeg/FDK 反复确认为 Tao 实现错误, 即使精度暂时下降也接受.
- 需在日志中标注联动依赖子功能.

### 5.2 精度变化辅助判定

- 精度大幅提升 → 有效.
- 精度大幅下降且缺乏逻辑证据 → 无效并回滚.

### 5.3 联动修复判定

- 某些修复需配合后续修复才能生效, 应从更大范围判断.
- 提交和日志中必须标注联动依赖.

## 6. 对比测试命令

```bash
# P0 快速门禁
TAO_AAC_COMPARE_INPUT=data/1.m4a cargo test --test run_decoder -- --nocapture --ignored test_aac_compare
TAO_AAC_COMPARE_INPUT=data/2.m4a cargo test --test run_decoder -- --nocapture --ignored test_aac_compare

# P1 全量回归
python3 plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-imprecise --prefer-local-data --jobs 8
```

## 7. 执行优先级

1. **功能点 0** (清理) → 先决条件
2. **功能点 1** (FFT-based IMDCT) → 预期影响最大, 解决分布式数值精度差异
3. **功能点 3** (Overlap-Add 状态机重写) → 解决窗口过渡结构性偏差
4. **功能点 2** (窗函数) → 配合功能点 3
5. **功能点 4** (频谱工具链) → 精细化对齐
6. **功能点 5** (语法/声道) → 查漏补缺
7. **功能点 6** (覆盖率) → 最终验收

## 8. 相关文件

- `crates/tao-codec/src/decoders/aac/mod.rs` — 主解码器, overlap-add 逻辑
- `crates/tao-codec/src/decoders/aac/imdct.rs` — IMDCT 变换 + 窗函数
- `crates/tao-codec/src/decoders/aac/spectral.rs` — 频谱解码, TNS, MS, IS, PNS, Pulse
- `crates/tao-codec/src/decoders/aac/huffman.rs` — Huffman 码本
- `crates/tao-codec/src/decoders/aac/tables.rs` — 常量表, SFB 边界, TNS 映射表
- `crates/tao-codec/src/decoders/aac/tests.rs` — 单元测试
- `plans/tao-codec/audio/aac/decoder_compare.rs` — P0 对比工具
- `plans/tao-codec/audio/aac/coverage/run_decoder.py` — P1 批量回归脚本
- `plans/tao-codec/audio/aac/aac_accuracy_round_log.md` — 迭代日志
- `plans/tao-codec/audio/aac/aac_accuracy_baseline.md` — 精度基线

## 9. 备注

- 功能点 1 (FFT-based IMDCT) 是本轮与 v1 最大的方向差异. v1 的 60 轮探针均基于朴素 IMDCT 的数值路径, 无法消除 O(N²) 累积浮点误差. FFT 实现可从根本上改善数值精度分布.
- 功能点 3 不能在功能点 1 之前独立做, 否则会重现 Round-55 的不兼容退化. 应先完成 IMDCT 重写, 再重构 overlap-add.
- 自研约束: FFT 和 IMDCT 实现必须在 Tao 仓库内自行实现, 禁止引入外部 FFT 库.

## 10. 进度标记 (断点续执行)

### 10.1 当前循环状态

- 当前轮次: `Round-71`.
- 当前功能点: `3`.
- 当前子功能: `3-2`.
- 当前状态: `in_progress`.

### 10.2 子功能检查表

- [x] 0-1 清除环境变量探针.
- [x] 0-2 干净基线建立.
- [x] 1-1 N/4 点复数 FFT.
- [x] 1-2 FFT-based IMDCT-1024.
- [x] 1-3 FFT-based IMDCT-128.
- [x] 1-4 数值一致性验证.
- [x] 2-1 KBD 窗数值对比.
- [x] 2-2 Sine 窗精度验证.
- [x] 2-3 LONG_START/LONG_STOP 窗口对比.
- [x] 2-4 short 窗口乘法对比.
- [x] 3-1 ONLY_LONG overlap 对比.
- [ ] 3-2 LONG_START overlap 对比.
- [ ] 3-3 LONG_STOP overlap 对比.
- [ ] 3-4 EIGHT_SHORT overlap 对比.
- [ ] 3-5 同构 overlap-add 实现.
- [ ] 3-6 P0 回归验证.
- [x] 4-1 Huffman + 反量化.
- [ ] 4-2 PNS 噪声重建.
- [x] 4-3 MS Stereo.
- [x] 4-4 Intensity Stereo.
- [x] 4-5 Pulse Tool.
- [x] 4-6 TNS 滤波.
- [ ] 5-1 raw_data_block 解析.
- [ ] 5-2 ics_info 解析.
- [ ] 5-3 CPE 工具链顺序.
- [ ] 5-4 声道映射.
- [ ] 5-5 裁剪/截断语义.
- [ ] 6-1 P1 全量回归.
- [ ] 6-2 失败样本修复.
- [ ] 6-3 容错路径验证.
