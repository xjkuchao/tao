# MP3 解码器实现工作计划

## 目标

移除外部依赖 `puremp3`，在 `tao-codec` 中从零实现纯 Rust 的 MP3 (MPEG-1 Layer III) 解码器。

## 阶段规划

### 第一阶段：基础架构与帧解析 (Phase 1: Infrastructure & Parsing)

**目标**: 能够正确识别 MP3 帧，解析头部信息和副作用信息 (Side Information)，并提取主数据 (Main Data)。

1.  **BitReader 实现**:
    -   确保有高效的位级读取器 (BitReader)，支持按位读取、无符号整数解析。
2.  **帧头解析 (Frame Header)**:
    -   识别同步字 (Sync Word: 0xFFE0/0xFFF0)。
    -   解析 Version (MPEG-1/2/2.5), Layer (III), CRC, Bitrate, Samplerate, Padding, Private bit。
    -   解析 Channel Mode (Stereo, Joint Stereo, Dual Channel, Single Channel)。
3.  **副作用信息解析 (Side Information)**:
    -   解析 `main_data_begin` (比特储备库指针)。
    -   解析 `scfsi` (Scale Factor Selection Information)。
    -   解析 Granule 信息 (part2_3_length, big_values, global_gain, scalefac_compress, windows switching 等)。
4.  **主数据提取 (Main Data Extraction)**:
    -   实现 "比特储备库" (Bit Reservoir) 机制，正确组装跨帧的主数据。

### 第二阶段：Huffman 解码与反量化 (Phase 2: Huffman & Dequantization) - [Completed]

**目标**: 将比特流解码为频谱系数 (Spectral Coefficients)。

1.  **Huffman 解码**: [x]
    -   实现 32 张标准 Huffman 表。
    -   解析 `big_values` (大值区)、`count1` (小值区) 和 `rzero` (零值区)。
2.  **比例因子解码 (Scalefactors)**: [x]
    -   根据 `scalefac_compress` 和 `slen` 表解析比例因子。
    -   处理长块 (Long blocks) 和短块 (Short blocks) 的不同解析逻辑。
3.  **反量化 (Requantization)**: [x]
    -   实现 $sample = sign * |huffman|^{4/3} * 2^{(gain - scalefac)/4}$ 公式。
4.  **重排序 (Reordering)**: [x]
    -   对短块 (Short blocks) 进行频谱系数重排序。

### 第三阶段：立体声处理与抗混叠 (Phase 3: Stereo & Alias Reduction) - [Completed]

**目标**: 处理声道间的相关性并消除混叠伪影。

1.  **立体声处理 (Joint Stereo)**: [x]
    -   实现 **MS Stereo** (Middle/Side): $L = (M+S)/\sqrt{2}, R = (M-S)/\sqrt{2}$。
    -   实现 **Intensity Stereo**: 仅在高频部分共享能量，保留相位差异。
2.  **抗混叠 (Alias Reduction)**: [x]
    -   在长块处理中，对子带边界进行“蝴蝶”运算 (Butterfly operation) 以消除混叠。

### 第四阶段：IMDCT 与 频率反转 (Phase 4: IMDCT & Frequency Inversion) - [Completed]

**目标**: 将频谱数据转换为时域 PCM 样本。

1.  **IMDCT (逆修正离散余弦变换)**: [x]
    -   实现 18 点 (Long block) 和 6 点 (Short block) IMDCT。
    -   应用窗口函数 (Sine Window / Kaiser-Bessel Derived Window)。
    -   处理重叠相加 (Overlap-Add)。
2.  **多相合成滤波器组 (Polyphase Synthesis Filterbank)**: [x]
    -   将 32 个子带的样本合成为最终的 PCM 输出。
    -   包含频率反转 (Frequency Inversion) 和合成窗口 (Synthesis Windowing)。

### 第五阶段：集成与验证 (Phase 5: Integration & Verification) - [Completed]

**目标**: 整合所有模块，通过测试并优化性能。

1.  **集成**: [x]
    -   将上述流程串联在 `Mp3Decoder::send_packet` 和 `receive_frame` 中。
2.  **验证**: [x]
    -   使用标准测试文件 (如 `color16.avi` 中的音频，或其他 MP3 样本)。
    -   与 FFmpeg 或 minimp3 的解码结果进行逐采样对比 (PSNR 测试)。
3.  **优化**: [x]
    -   查表优化 (如 Huffman 表、DCT 系数)。
    -   SIMD 优化 (可选，后期进行)。

## 执行顺序建议

建议按照 **Phase 1 -> Phase 2 -> Phase 4 -> Phase 3** 的顺序执行。
(立体声处理可以稍后加入，先确保单声道能出声，或者直接处理双声道但不开启 Joint Stereo)。

## 资源参考

-   ISO/IEC 11172-3 (MPEG-1 Audio)
-   The anatomy of the MP3 format (http://blog.bjrn.se/2008/10/lets-build-mp3-decoder.html)
-   minimp3 (C library reference)
