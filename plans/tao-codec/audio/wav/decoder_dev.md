# tao-codec WAV 解码器开发计划

## 1. 背景与目标

- 当前 WAV (PCM) 音频格式可能已存在初步实现, 但缺乏系统化的工作计划与覆盖率对齐环节。
- 必须遵守项目规则: 纯自研实现, 不依赖外部多媒体算法库。
- 目标: 建立完备的解码状态机架构，并在大量 WAV 样本中达到 100.00% 的逐帧对标 FFmpeg 精度。

## 2. 模块化决策

- 判定: WAV 虽然原理简单，但可能涉及复杂的 chunk 处理（`fmt `, `data`, `fact`, `LIST`），支持 PCM 及 ADPCM 等变体。
- 目录规划:
    - `crates/tao-codec/src/decoders/wav/mod.rs`: 解码器状态机与对外接口
    - `crates/tao-codec/src/decoders/wav/demux.rs`: RIFF/WAV chunk 解析
    - `crates/tao-codec/src/decoders/wav/pcm.rs`: 标准 PCM 数据解析与大小端转换
    - 后续附加 `adpcm.rs` 及跨平台特殊编码（如 `G.711` A-law / mu-law 等）

## 3. 里程碑与执行顺序

### 执行与提交规则(强制)

- 每完成一个关键变更必须执行质量门禁: `cargo fmt`, `clippy`, `check`, `test`，且全部通过后方可提交。

### P0 基线与计划

- [x] 明确禁用外部多媒体库。
- [x] 补全可续执行的开发计划文档。

### P1 结构重构与状态机

- [ ] 确保 `wav` 子模块目录架构符合标准拆解。
- [ ] 解析基础 `fmt ` chunk 及校验 `data` chunk。
- [ ] `send_packet/receive_frame/flush` 编解码管线状态机畅通。

### P2 各编码类型详细解析

- [ ] 接入标准线性的 `pcm_s16le`, `pcm_s24le`, `pcm_s32le` 解码。
- [ ] 支持浮点 `pcm_f32le`, `pcm_f64le` 解码模式。
- [ ] 扩展接入 8-bit 的无符号 `pcm_u8`。
- [ ] 容错处理损坏的尾部 chunk 及不标准文件头。

### P3 质量对标与性能调优

- [ ] 避免大块 `memcpy`，优化零拷贝缓冲流。
- [ ] 支持非常规 Channel Layouts。
- [ ] 对齐 FFmpeg 的输出时间戳与样本数推算。

### P4 逐帧对标测试

- [ ] 新增 `tests/perf_compare/compare.rs` 跑跑比对。
- [ ] 产出准确的对比指标（MSE/PSNR）。
- [ ] 全部样本收敛到 `max_err=0.0`, 精度 `100.00%`。

### P5 覆盖率闭环

- [ ] 完成 `coverage` 批量测试计划。
- [ ] 四项门禁全部通过。

## 4. 前置条件

- `ffprobe` / `ffmpeg` 用于建立基准输出。
- 测试样本抓取完毕并归为 `samples/` 相关类别。

## 5. 验收标准

- 样本对比无偏差，无越界读取。
