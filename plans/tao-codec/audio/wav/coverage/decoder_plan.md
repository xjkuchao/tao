# tao-codec WAV 解码器样本覆盖率测试计划

## 1. 背景与目标

抓取并分析 `https://samples.ffmpeg.org/allsamples.txt` 中的 WAV 样本，建立一套系统性的批量对比验证流程，对齐之前 Vorbis 和 MP3 的实践标准。

**最终目标**:

1. 完成 WAV 代表性样本爬取并生成 `report.md`。
2. 批量进行单样本比对，对比基础为 FFmpeg 输出。
3. 把所有的 WAV (尤其是不同位深、大小端、交错方式的 PCM) 解码精度收敛到 100.00%。

## 2. 执行范围

- 目录: `plans/tao-codec/audio/wav/coverage/`
- 脚本: `run.py`
- 报告: `report.md`
- 对比程序: `compare.rs`

## 3. 验收标准

- 所有涵盖的样本皆产出结果。若跳过须带原因。
- 比对实现真正的输出字节到字节（或是 1-to-1 转 F32 / F64 数据）无误差。
- 可以通过脚本进行自动化、断点续测。
