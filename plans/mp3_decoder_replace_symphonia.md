# MP3 自研解码器替换 Symphonia 工作计划

## 背景与现状

### 当前架构

`tao-codec` 的 MP3 解码器 (`crates/tao-codec/src/decoders/mp3/mod.rs`) 目前采用 **双路径架构**:

1. **symphonia 路径 (当前生效)**: `open()` 时初始化 `SymMpaDecoder`, `decode_one_frame()` 中若 `sym_decoder` 存在则直接使用 symphonia 解码, 输出稳定。
2. **自研路径 (已完整但未启用)**: 完整实现了 MP3 Layer III 的所有解码阶段:
    - 帧头解析 (header.rs)
    - 副作用信息解析 (side_info.rs)
    - Bit Reservoir 管理 (mod.rs)
    - Huffman 解码 (huffman.rs) — 快速查找表, 全部 32 张表
    - 比例因子解码 (mod.rs)
    - 反量化 (requantize.rs) — 长块/短块/混合块
    - 立体声处理 (stereo.rs) — MS Stereo + Intensity Stereo
    - 重排序 (reorder.rs)
    - 抗混叠 (alias.rs)
    - IMDCT (imdct.rs) — 18点/6点, 窗口加权, 重叠相加
    - 多相合成滤波器 (synthesis.rs) — DCT32 + 512点窗口
    - 频率反转 (synthesis.rs)

### 已知问题

- 测试 `tests/mp3_pcm_dump.rs` 中自研解码器与 ffmpeg/symphonia 的对比误差较大。
- 误差来源尚未定位 — 可能涉及多个子模块。
- 故已临时接入 symphonia 作为 backend 保证项目稳定运行。

### 外部依赖

- `symphonia-core = "0.5.5"` (在 tao-codec/Cargo.toml)
- `symphonia-bundle-mp3 = "0.5.5"` (在 tao-codec/Cargo.toml)
- `symphonia = "0.5.5"` (在根 Cargo.toml, dev-dependencies, 用于测试)

## 目标

1. **最终目标**: 彻底移除 symphonia 依赖, 100% 使用自研 MP3 解码器。
2. **质量对标**: 解码输出 100% 对标 FFmpeg (libmpg123/libmp3float)。
3. **性能对标**: 解码速度不低于 symphonia, 目标接近 FFmpeg。
4. **稳定性第一**: 每一步替换都必须可验证、可回退, 不得引入回归。

## 对标参考: FFmpeg MP3 解码器

FFmpeg 的 `libavcodec/mpegaudiodec_template.c` 是权威参考实现, 关键点:

| 模块          | FFmpeg 实现特点                       |
| ------------- | ------------------------------------- |
| 反量化        | f64 精度累积, POW43 查表 8192 项      |
| IMDCT         | 优化的 Lee DCT 9点/36点, 内联窗口函数 |
| 合成滤波器    | 32点 DCT + 512点窗口, FIFO 滑窗       |
| Huffman       | VLC 查表, 无回溯                      |
| 立体声        | IS ratio table, MS 处理在频域         |
| Bit Reservoir | 标准 main_data_begin 管理             |

## 核心策略: 双路径逐模块替换

```
┌─────────────────────────────────────────────────────────────┐
│                    decode_one_frame()                        │
│                                                             │
│  if cfg!(feature = "mp3-native") {                         │
│      自研路径 ───────>  逐模块替换  ───────> 最终唯一路径    │
│  } else {                                                   │
│      symphonia 路径 ──> 参考对比 ────────> 最终移除          │
│  }                                                          │
└─────────────────────────────────────────────────────────────┘
```

通过 Cargo feature flag `mp3-native` 控制路径切换,
每个阶段仅替换一个子模块并通过测试验证, 确保渐进式、可回退。

---

## 阶段 0: 基础设施建设（测试框架与诊断工具） - [Completed]

**目标**: 建立精确的逐模块诊断与对比框架, 为后续每一步替换提供验证基础。

### 任务

- [x] **0.1** 在 `tao-codec/Cargo.toml` 添加 feature flag:
    - `symphonia-backend` (default): 使用 symphonia 解码路径
    - `mp3-native`: 使用自研解码路径
    - `mod.rs` 中用 `#[cfg(feature = "symphonia-backend")]` 守护 symphonia 代码

- [x] **0.2** 创建逐模块对比测试 `tests/mp3_module_compare.rs`:
    - 帧级别: 自研 vs FFmpeg, 逐帧对比 PCM 输出, 计算 PSNR/MSE/最大误差。
    - 全局精度汇总报告。
    - 多文件批量对比 (summary test)。

- [x] **0.3** 创建 `crates/tao-codec/src/decoders/mp3/debug.rs` 模块:
    - `FrameSnapshot` 结构体: 记录各阶段中间数据。
    - `CompareResult`: 精度对比结果 (max_abs_error, mean_abs_error, mse, psnr_db)。
    - `compare_f32_samples()`, `compare_i32_samples()`: 对比函数。
    - `acceptance` 模块: 验收标准常量。

- [x] **0.4** 使用已有测试样本:
    - `data/1.mp3` — 384 帧, 44100Hz, 2ch (Joint Stereo)
    - `data/2.mp3` — 1150 帧, 44100Hz, 2ch

- [x] **0.5** 定义验收标准常量 (在 `debug::acceptance` 中):
    - `MAX_SAMPLE_ERROR = 1e-4`
    - `MAX_FRAME_AVG_ERROR = 1e-5`
    - `MIN_PSNR_DB = 80.0`

### 基线精度数据 (2026-02-18)

| 文件       | PSNR  | 最大误差 | 平均误差 | 通过帧率   |
| ---------- | ----- | -------- | -------- | ---------- |
| data/1.mp3 | 6.9dB | 1.70     | 0.361    | 0/382 (0%) |
| data/2.mp3 | 6.4dB | 1.91     | 0.369    | 未测       |

> 结论: 误差量级为 O(1), 属于逻辑性错误而非精度问题。

### 产出

- Feature flag 机制可用, 默认使用 symphonia, 加 `--features mp3-native --no-default-features` 使用自研。
- 逐模块对比测试框架就绪。
- 基线精度数据已记录。

### 验收标准

- ✅ `cargo test` 默认 (symphonia 路径) 全部通过。
- ✅ `cargo check --features mp3-native --no-default-features` 编译通过。
- ✅ `cargo test --test mp3_module_compare` 可运行, 输出精度报告。
- ✅ 诊断输出清晰展示每帧每模块的误差。

---

## 阶段 1: 定位与修复反量化 (Requantization)

**目标**: 确保 `requantize.rs` 的输出与 FFmpeg 精确一致。

### 分析

反量化是频域系数 → 浮点值的第一步, 误差会被后续所有模块放大。
当前实现已使用 f64 精度计算指数和 POW43 查表, 但需要逐项对标 FFmpeg:

### 任务

- [ ] **1.1** 对标 FFmpeg 的 `mpegaudiodec_template.c` 反量化公式:
    - 长块: `xr = sign * pow43[|is|] * 2^((global_gain - 210 - sf_mult * (scalefac + preflag * pretab)) / 4)`
    - 短块: `xr = sign * pow43[|is|] * 2^((global_gain - 210 - 8 * subblock_gain - sf_mult * scalefac) / 4)`
    - 验证 `sf_mult` 的定义 (scalefac_scale ? 1.0 : 0.5, 当前实现正确)。

- [ ] **1.2** 验证 POW43 查表精度:
    - 当前使用 `f32::powf(4.0/3.0)`, FFmpeg 使用预计算双精度表。
    - 对比 8192 项表的精度差异。
    - 若差异显著, 改为 `f64` 精度预计算后截断到 `f32`。

- [ ] **1.3** 验证 PRETAB 表:
    - 对照 ISO 11172-3 Table B.6 确认 21 项值。

- [ ] **1.4** 验证 SFB_WIDTH 表:
    - 对照 ISO 标准确认所有采样率的 SFB 宽度 (44100/48000/32000Hz)。
    - 长块 22 bands, 短块 13 bands。

- [ ] **1.5** 比例因子解码对标:
    - 验证 MPEG1 长块 scfsi 处理逻辑。
    - 验证短块 scalefactor 解码 (12 bands \* 3 windows)。
    - 验证混合块 scalefactor 解码 (8 长块 + 短块)。

- [ ] **1.6** 编写反量化单元测试:
    - 构造已知输入 (is, scalefac, granule params), 对比 FFmpeg 参考输出。
    - 测试长块/短块/混合块各场景。

### 验收标准

- 反量化输出 xr[576] 与 FFmpeg 参考的逐样本误差 < 1e-5。
- 所有块类型 (长块/短块/混合块) 均通过。

---

## 阶段 2: 定位与修复 Huffman 解码 + Bit Reservoir

**目标**: 确保 Huffman 解码和 Bit Reservoir 管理的正确性。

### 任务

- [ ] **2.1** 验证 Huffman Big Values 解码:
    - 回归测试: 所有 31 张表的编码-解码往返 (已有测试)。
    - 新增: 对实际 MP3 帧的 Huffman 输出与 FFmpeg 参考对比。
    - 验证 linbits 处理。

- [ ] **2.2** 验证 Count1 解码:
    - Table 32/33 的输出正确性。
    - 边界条件: count1 超出 part2_3_length 时的丢弃逻辑。

- [ ] **2.3** 验证 region 边界计算:
    - `region0_count`, `region1_count` → SFB 累积边界。
    - 短块固定边界 (36, 576)。
    - 对比 FFmpeg 的 region 划分。

- [ ] **2.4** 验证 Bit Reservoir 管理:
    - `main_data_begin` 指针的正确解释。
    - 跨帧数据组装的正确性。
    - 储备库大小限制 (当前 512 字节, FFmpeg 使用 BACKSTEP_SIZE = 512)。
    - 不足数据时的处理 (丢帧 vs 填零)。

- [ ] **2.5** 比特消耗审计:
    - 在 Huffman 解码后验证 br.bit_offset() 是否精确匹配 part2_3_length。
    - 添加诊断: 打印每个 granule 的 part2 bits, part3 bits, 总消耗。

### 验收标准

- 对实际 MP3 文件, 逐帧的 Huffman 输出 (is[576]) 与参考完全一致 (整数精确匹配)。
- Bit Reservoir 管理无数据错位。

---

## 阶段 3: 定位与修复立体声处理 + 重排序 + 抗混叠

**目标**: 确保频域处理链的正确性。

### 任务

- [ ] **3.1** 验证 MS Stereo:
    - 公式: `L = (M + S) * 1/√2`, `R = (M - S) * 1/√2`。
    - 处理范围: 仅在 IS bound 之前 (若有 Intensity Stereo)。
    - 对比 FFmpeg 的 `compute_stereo()` 函数。

- [ ] **3.2** 验证 Intensity Stereo:
    - IS ratio 表对照。
    - IS bound 计算: R channel 非零边界 → SFB 对齐。
    - is_pos == 7 的处理 (无效, 回退 MS 或保持)。
    - 长块/短块分别验证。

- [ ] **3.3** 验证处理顺序:
    - FFmpeg 顺序: Huffman → 反量化 → 立体声 → 重排序 → 抗混叠 → IMDCT。
    - 当前自研顺序: 一致, 但需确认 stereo 在 reorder 之前。

- [ ] **3.4** 验证 Reorder:
    - 重排序映射: (band, window) → (subband, window, position)。
    - 混合块: 仅对 sample >= 36 的短块区域重排。
    - 对比 FFmpeg 的 `reorder_block()` 实现。

- [ ] **3.5** 验证 Alias Reduction:
    - 8 对蝴蝶系数 CS/CA 对照 ISO 标准。
    - 混合块: 仅处理前 2 个子带边界。
    - rzero 限制: 避免将零值扩散。

- [ ] **3.6** 频域处理链集成测试:
    - 输入: 已验证正确的反量化 xr。
    - 输出: stereo + reorder + alias 后的 xr。
    - 与 FFmpeg 参考逐样本对比。

### 验收标准

- 频域处理链输出 xr[576] 与 FFmpeg 参考的逐样本误差 < 1e-5。
- Joint Stereo (MS + IS) 场景通过。
- 短块/混合块重排序后数据与参考一致。

---

## 阶段 4: 定位与修复 IMDCT + 合成滤波器

**目标**: 确保时域转换与 PCM 合成的正确性。这是误差放大最敏感的环节。

### 任务

- [ ] **4.1** 验证 18 点 IMDCT:
    - 对照 FFmpeg 的 `imdct36()` 实现。
    - 窗口类型 (Normal/Start/Stop/Short) 的窗口系数。
    - 重叠相加 (Overlap-Add) 逻辑。
    - 精度: 使用 f64 累加, 最终截断 f32 (当前已实现)。

- [ ] **4.2** 验证 6 点 IMDCT (短块):
    - 对照 FFmpeg 的 `imdct12()` / `imdct36_short()` 实现。
    - 短块窗口 (12 点正弦窗)。
    - 3 窗口叠加放置逻辑。
    - 与 symphonia `imdct12_win` 对比。

- [ ] **4.3** 验证合成滤波器 (Polyphase Synthesis Filterbank):
    - DCT-32 实现: 对照 FFmpeg 的 Lee DCT 或者 ISO 标准的矩阵运算。
    - 合成窗口 D[512]: 逐项与 ISO Table B.3 / FFmpeg 对照。
    - FIFO 滑窗 (V-Buffer) 管理: offset 递增, 模 1024 取样。
    - 窗口加权累加: 8 段 × 64 点。

- [ ] **4.4** 验证频率反转:
    - 仅 `f(sb, ts)` 中 sb 和 ts 都为奇数时取反。
    - 确认公式: `if sb % 2 == 1 && ts % 2 == 1 { sample = -sample }`。

- [ ] **4.5** IMDCT + 合成滤波器集成测试:
    - 输入: 已验证正确的频域 xr (经过 stereo + reorder + alias)。
    - 输出: PCM f32 交织样本。
    - 与 FFmpeg 参考逐样本对比。

- [ ] **4.6** 累积误差分析:
    - 逐模块追踪误差传播: 反量化误差 → stereo → reorder → alias → IMDCT → synthesis。
    - 确认误差是否在 synthesis 阶段被放大。

### 验收标准

- 完整解码管线的 PCM 输出与 FFmpeg 参考的逐样本误差 < 1e-4。
- PSNR >= 80 dB。
- 前 100 帧无异常噪声或静音。

---

## 阶段 5: 替换切换 — 启用自研路径

**目标**: 将 `mp3-native` 设为默认, 移除 symphonia 依赖。

### 任务

- [ ] **5.1** 将 `mp3-native` 加入 default features:

    ```toml
    [features]
    default = ["mp3-native"]
    mp3-native = []
    ```

- [ ] **5.2** 全量回归测试:
    - 运行 `tests/mp3_pcm_dump.rs` 全部测试。
    - 运行 `tests/ffmpeg_compare.rs` 验证集成。
    - 运行 `benches/codec_bench.rs` 性能对比。

- [ ] **5.3** 边界情况测试:
    - 损坏的 MP3 文件 (bit-flip, truncated)。
    - 极端比特率 (32kbps ~ 320kbps)。
    - 所有采样率 (44100, 48000, 32000, 22050, 24000, 16000, 11025, 12000, 8000)。
    - 自由比特率 (free format, bitrate_idx = 0)。
    - ID3v2 标签跳过。
    - Xing/LAME 头处理。

- [ ] **5.4** 性能验证:
    - 基准: symphonia 路径解码速度。
    - 目标: 自研路径速度 >= symphonia 的 90%。
    - 使用 `criterion` 基准测试。

- [ ] **5.5** 清理 symphonia 代码路径:
    - 移除 `mod.rs` 中的 symphonia import 和调用代码。
    - 移除 `Mp3Decoder` 中的 `sym_decoder` 字段。
    - 移除 `open()` 中的 symphonia 初始化。
    - 移除 `flush()` 中的 symphonia reset。

- [ ] **5.6** 移除 symphonia 依赖:
    - 从 `crates/tao-codec/Cargo.toml` 移除 `symphonia-core` 和 `symphonia-bundle-mp3`。
    - 从根 `Cargo.toml` 的 `dev-dependencies` 中, symphonia 仅保留用于交叉验证测试 (可选)。

- [ ] **5.7** 移除 feature flag:
    - 自研路径成为唯一路径, 移除 `mp3-native` feature 及条件编译。

### 验收标准

- `cargo test` 全部通过, 无 symphonia 依赖。
- 解码输出与 FFmpeg 参考的 PSNR >= 80 dB。
- 性能不低于 symphonia 的 90%。
- `cargo clippy` 无警告。
- `cargo fmt --check` 通过。

---

## 阶段 6: 性能优化与 MPEG-2/2.5 支持

**目标**: 进一步优化性能, 补全 MPEG-2/2.5 支持, 全面对标 FFmpeg。

### 任务

- [ ] **6.1** MPEG-2/2.5 反量化:
    - 实现 LSF (Low Sampling Frequency) 特有的 scalefactor 解码。
    - 实现 MPEG-2 的 IS stereo (不同的 IS ratio 表)。

- [ ] **6.2** MPEG-2/2.5 测试:
    - 补充低采样率测试样本。
    - 与 FFmpeg 对比验证。

- [ ] **6.3** 性能热点优化:
    - IMDCT: 实现 Lee/Pribyl 9点快速 DCT 算法 (替代朴素矩阵乘法)。
    - Synthesis: 优化 DCT-32 的蝴蝶运算 (当前已使用 Lee 算法, 验证是否最优)。
    - Huffman: 评估是否需要更大的 peek 窗口 (当前 10 bit)。
    - POW43: 评估更大查表 (16384 项) 是否能减少 fallback 计算。

- [ ] **6.4** SIMD 优化 (可选):
    - 合成滤波器的窗口加权 (8 × 64 点乘加, 适合 SIMD)。
    - IMDCT 的余弦变换。
    - 使用 `std::arch` 或 `packed_simd2`。

- [ ] **6.5** 编写性能基准:
    - 解码吞吐量 (MB/s)。
    - 单帧解码延迟 (μs)。
    - 与 ffmpeg (libmpg123), symphonia, minimp3 的对比。

### 验收标准

- MPEG-1/2/2.5 Layer III 全部支持。
- 所有采样率和比特率组合通过测试。
- 解码吞吐接近或超过 FFmpeg。
- 无 SIMD 时性能不低于 symphonia。

---

## 执行顺序与时间估算

| 阶段 | 内容                     | 预估工时 | 依赖      |
| ---- | ------------------------ | -------- | --------- |
| 0    | 基础设施建设             | 1-2 天   | 无        |
| 1    | 反量化修复               | 1-2 天   | 阶段 0    |
| 2    | Huffman + Bit Reservoir  | 1-2 天   | 阶段 0    |
| 3    | 立体声 + 重排序 + 抗混叠 | 2-3 天   | 阶段 1, 2 |
| 4    | IMDCT + 合成滤波器       | 2-3 天   | 阶段 3    |
| 5    | 替换切换                 | 1 天     | 阶段 4    |
| 6    | MPEG-2/2.5 + 性能优化    | 3-5 天   | 阶段 5    |

**总计: 约 11-18 天**

> 注: 阶段 1 和 2 可并行进行; 阶段 6 可根据优先级延后。

## 关键原则

1. **稳定性第一**: 每个阶段完成后, 默认路径 (symphonia) 不受影响。仅当整条管线全部验证通过后才切换默认路径。

2. **可回退**: 通过 feature flag 随时可以切回 symphonia 路径。

3. **精确对标**: 以 FFmpeg 的 PCM 输出为唯一参考基准, 不以 symphonia 或 minimp3 为标准 (它们本身可能有微小差异)。

4. **逐模块验证**: 先确保底层模块 (反量化, Huffman) 精确, 再验证上层模块 (stereo, IMDCT, synthesis), 避免误差叠加导致难以定位。

5. **自动化验证**: 所有验收标准都必须有对应的自动化测试, 不依赖人工判断。

## 资源参考

- ISO/IEC 11172-3 (MPEG-1 Audio Layer III 标准)
- FFmpeg `libavcodec/mpegaudiodec_template.c` (权威参考实现)
- minimp3 C 库 (轻量参考)
- symphonia-bundle-mp3 Rust 库 (当前 backend)
- [The Anatomy of the MP3 Format](http://blog.bjrn.se/2008/10/lets-build-mp3-decoder.html)
