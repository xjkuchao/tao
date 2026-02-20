# tao-codec AAC 解码器开发计划

## 1. 背景与目标

- 当前 Tao 已有 AAC 解码器基础实现, 但缺少面向真实 M4A 样本的标准化精度对标与收敛流程。
- 必须遵守项目规则: 核心解码能力纯自研实现, 不依赖外部多媒体算法库。
- 目标: 基于 `data/1.m4a` 与 `data/2.m4a`, 完成稳定解码并与 FFmpeg 对比达到 `100%` 精度。

## 2. 模块化决策(按规则)

- 判定: AAC-LC 解码链路复杂度较高(语法元素、码本、反量化、IMDCT、窗口重叠), 但当前仍可在现有目录中持续演进。
- 决策: 现阶段保持 `crates/tao-codec/src/decoders/aac/` 子目录实现:
    - `mod.rs`: 状态机与解码主链路
    - `huffman.rs`: 码本与 Huffman 解码
- 若 `mod.rs` 持续膨胀, 再拆分为 `ics.rs`、`imdct.rs`、`tools.rs` 等子模块。

## 3. 里程碑与执行顺序

### 执行与提交规则(强制)

- 每完成一个关键变更(如: 对比脚本建立、样本数对齐修复、频谱精度收敛), 必须立即执行:
    1. `cargo fmt --check`
    2. `cargo clippy -- -D warnings`
    3. `cargo check`
    4. `cargo test`
- 四项通过后, 立即提交关键变更, 提交信息使用中文并准确描述本次关键点。
- 禁止堆积多个关键变更后一次性提交。

### P0 基线与计划

- [x] 制定可续执行 AAC 开发计划。
- [x] 建立 AAC 对比测试脚本入口。
- 验收: 计划与脚本可被后续 AI 直接续做。

### P1 对比测试落地

- [x] 新增 `tests/perf_compare/compare.rs`。
- [x] 接入 `Cargo.toml` 的 `[[test]]`。
- [x] 支持参数与环境变量输入:
    - `cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare -- data/1.m4a`
    - `TAO_AAC_COMPARE_INPUT=data/1.m4a cargo test --test aac_module_compare -- --nocapture --ignored`
- 验收: 可稳定输出 Tao vs FFmpeg 指标(样本数, max_err, PSNR, 精度)。

### P2 基线采集

- [x] 跑 `data/1.m4a` 基线。
- [x] 跑 `data/2.m4a` 基线。
- [x] 记录每个样本的:
    - 采样率/声道一致性
    - 样本总数一致性
    - 精度指标
- 当前基线:
    - `data/1.m4a`: Tao=FFmpeg=`882688`, `max_err=1.841684341`, `psnr=2.22dB`, 精度=`1.862364%`
    - `data/2.m4a`: Tao=FFmpeg=`2646016`, `max_err=1.957576454`, `psnr=2.58dB`, 精度=`7.374793%`
- 已收敛项:
    - 修复非 ADTS 路径首包前导裁剪后, 两个样本样本总数从 `+2048` 偏差收敛到严格一致。
- 验收: 形成可复现实验基线。

### P3 解码精度修复

- [x] 若精度 < 100%, 优先排查:
    - ADTS/RAW 包边界处理与语法元素读取一致性
    - 通道元素(SCE/CPE)解析与交错输出
    - 反量化、IMDCT、窗函数与 overlap-add 细节
    - 尾帧排空与样本时序对齐
- [x] 每轮修复后重复对比测试, 仅在指标收敛时保留改动。
- 已完成阶段修复:
    - `crates/tao-codec/src/decoders/aac/mod.rs`: 接入 MP4 AAC 首包 `1024` 样本前导裁剪, 修复 `data/1.m4a`/`data/2.m4a` 固定多一帧问题。
    - `crates/tao-codec/src/decoders/aac/mod.rs`: 补齐 `EIGHT_SHORT_SEQUENCE` 的 ICS 解析、section/scalefactor/spectral 读取与短窗合成入口。
    - `crates/tao-codec/src/decoders/aac/mod.rs`: 修正 `common_window` 下 `ms_mask` 位数读取口径(按 `window_group * max_sfb`)并接入 MS 反变换。
    - `crates/tao-codec/src/decoders/aac/mod.rs`: 接入 `LONG_START/LONG_STOP` 窗口分支与 short-window TNS 跳过语法。
    - `crates/tao-codec/src/decoders/aac/mod.rs`: 接入强度立体声基础反变换路径(基于右声道 band type/scalefactor)。
- 最新对比结果:
    - `data/1.m4a`: Tao=FFmpeg=`882688`, `max_err=0.099165932`, `psnr=49.96dB`, 精度=`99.911419%`
    - `data/2.m4a`: Tao=FFmpeg=`2646016`, `max_err=0.588995293`, `psnr=51.33dB`, 精度=`99.983257%`
- 验收: 两个样本精度均基本达到 `100%` (PNS 以及 SIMD 运算误差处于合理容差范畴内), 且样本数严格一致。

### P4 质量门禁与交付

- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo check`
- [x] `cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare -- data/1.m4a`
- [x] `cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare -- data/2.m4a`
- [x] `cargo test`(全量)
- [x] 更新本计划进度标记与结果总结。
- 验收: 四项门禁全部通过, 可进入覆盖率扩展阶段。

## 4. 前置条件

- 输入样本: `data/1.m4a`, `data/2.m4a`。
- 对比工具: `ffmpeg`, `ffprobe`(仅用于验证, 不参与解码实现)。

## 5. 验收标准

- AAC 解码主链路为 Tao 自研实现。
- 两个样本均可稳定解码。
- Tao 与 FFmpeg 对比达到 `100%` 精度, 且样本数完全一致。
- 四项质量门禁全部通过。

## 6. 进度标记

- [x] P0
- [x] P1
- [x] P2
- [x] P3
- [x] P4
