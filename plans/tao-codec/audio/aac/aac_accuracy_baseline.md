# AAC 精度基线记录

## 1. 基线快照

- 记录时间: 2026-03-01.
- 数据来源:
  - `plans/tao-codec/audio/aac/decoder_dev.md`.
  - `plans/tao-codec/audio/aac/coverage/report.md`.

## 2. P0 基线 (首要目标)

| 样本 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| data/1.m4a | 882000 | 882000 | 0 | 0.476204097 | 38.87 | 99.883833 | 实测: `TAO_AAC_COMPARE_INPUT=data/1.m4a cargo test --test run_decoder -- --nocapture --ignored test_aac_compare`, `lag=0` (Round-61, FFT IMDCT 后持平) |
| data/2.m4a | 960000 | 960000 | 0 | 0.000196695 | 86.95 | 99.999998 | 实测: `TAO_AAC_COMPARE_INPUT=data/2.m4a cargo test --test run_decoder -- --nocapture --ignored test_aac_compare`, `lag=0` (Round-61, FFT IMDCT 后持平) |

## 3. P1 覆盖基线 (次要目标)

| 指标 | 当前值 |
| --- | --- |
| 总样本数 | 114 |
| 成功 | 99 |
| 失败 | 15 |
| 精度 100.00% | 47 |
| 精度 < 100.00% | 52 |

## 4. 失败原因分布 (2026-02-28 报告口径)

| 分类 | 数量 |
| --- | --- |
| Unsupported(audioObjectType=1) | 5 |
| 未找到可解码音频流 | 2 |
| 无效 ADTS 帧头 | 2 |
| MP4 文件中未找到任何轨道 | 1 |
| Unsupported(audioObjectType=29) | 1 |
| Unsupported(audioObjectType=0) | 1 |
| Unsupported(audioObjectType=3) | 1 |
| Unsupported(音频格式码 0x00FF) | 1 |
| Eof | 1 |

## 5. 后续更新规则

- 每轮迭代结束后, 必须更新 P0 与 P1 基线表.
- 若基线来源由实测替代文档快照, 在备注列注明测试命令和 commit.

## 6. 本地样本缓存基线

| 指标 | 当前值 |
| --- | --- |
| 本地缓存目录 | data/aac_samples |
| 已缓存目标数 | 114 |
| 失败下载数 | 0 |
| 缓存体积 | 约 1.2G |
