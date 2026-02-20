# tao-codec FLAC 解码器开发计划

## 1. 背景与目标
- 当前 Tao 已有 FLAC 解码基础实现, 但尚未建立针对真实样本的标准化精度对标流程。
- 必须遵守项目规则: 纯自研实现, 不依赖外部多媒体算法库完成核心解码能力。
- 目标: 基于 `data/1.flac` 与 `data/2.flac`, 实现稳定解码并与 FFmpeg 对比达到 `100%` 精度。

## 2. 模块化决策(按规则)
- 判定: FLAC 解码复杂度中等, 当前单文件 `crates/tao-codec/src/decoders/flac.rs` 可维护。
- 决策: 现阶段保持单文件实现, 若后续出现持续复杂增长(子帧类型、残差路径、多位深输出策略耦合), 再拆分子目录:
  - `mod.rs`(状态机与接口)
  - `bitstream.rs`(帧头与子帧头语法)
  - `residual.rs`(Rice 解码)
  - `predict.rs`(Fixed/LPC)
  - `output.rs`(样本格式输出)

## 3. 里程碑与执行顺序

### 执行与提交规则(强制)
- 每完成一个关键变更(如: 对比脚本建立、长度对齐修复、精度收敛修复), 必须立即执行:
  1. `cargo fmt --check`
  2. `cargo clippy -- -D warnings`
  3. `cargo check`
  4. `cargo test`
- 四项通过后, 立即提交关键变更, 提交信息使用中文并准确描述本次关键点。
- 禁止堆积多个关键变更后一次性提交。

### P0 基线与计划
- [x] 制定可续执行开发计划。
- [x] 建立 FLAC 对比测试脚本入口。
- 验收: 计划与脚本均可被后续 AI 直接续做。

### P1 对比测试落地
- [x] 新增 `tests/perf_compare/flac_module_compare.rs`。
- [x] 接入 `Cargo.toml` 的 `[[test]]`。
- [x] 支持参数与环境变量输入:
  - `cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare -- data/1.flac`
  - `TAO_FLAC_COMPARE_INPUT=data/1.flac cargo test --test flac_module_compare -- --nocapture --ignored`
- 验收: 可稳定输出 Tao vs FFmpeg 指标(样本数, max_err, PSNR, 精度)。

### P2 基线采集
- [x] 跑 `data/1.flac` 基线。
- [x] 跑 `data/2.flac` 基线。
- [x] 记录每个样本的:
  - 采样率/声道一致性
  - 样本总数一致性
  - 精度指标
- 当前基线:
  - `data/1.flac`: Tao=FFmpeg=`881998`, max_err=`0.000000000`, psnr=`inf`, 精度=`100.000000%`
  - `data/2.flac`: Tao=FFmpeg=`2646000`, max_err=`0.000000000`, psnr=`inf`, 精度=`100.000000%`
- 验收: 形成可复现实验基线。

### P3 解码精度修复
- [x] 若精度 < 100%, 优先排查:
  - 位深归一化与样本格式转换
  - 子帧 warm-up 与残差拼接
  - 立体声 decorrelation
  - 帧级时序与尾帧排空
- [x] 每轮修复后重复对比测试, 仅在指标收敛时保留改动。
- 已完成关键修复:
  - `crates/tao-format/src/demuxers/flac.rs`: 同步头搜索新增基于前一帧 `CRC-16` 的边界确认, 修复 `data/2.flac` 帧内伪同步导致的截断与解码 `Eof`。
- 验收: `data/1.flac` 与 `data/2.flac` 均达到 `100%` 精度且样本数一致。

### P4 质量门禁与交付
- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo check`
- [x] `cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare -- data/1.flac`
- [x] `cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare -- data/2.flac`
- [x] `cargo test`(全量)
- [x] 更新本计划进度标记与结果总结。
- 验收: 四项门禁全部通过, 可直接进入后续覆盖率计划。

## 4. 前置条件
- 输入样本: `data/1.flac`, `data/2.flac`。
- 对比工具: `ffmpeg`, `ffprobe`(仅用于验证, 不参与解码实现)。

## 5. 验收标准
- FLAC 解码主链路为 Tao 自研实现。
- 两个样本均可稳定解码。
- Tao 与 FFmpeg 对比结果达到 `100%` 精度, 且样本数完全一致。
- 四项质量门禁全部通过。

## 6. 进度标记
- [x] P0
- [x] P1
- [x] P2
- [x] P3
- [x] P4
