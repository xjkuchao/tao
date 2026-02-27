# H264 解码器 -- 精度收敛计划

> 关联文档:
>
> - 功能开发: `decoder_dev.md`
> - 功能矩阵: `h264_feature_matrix.md`
> - 性能优化: `decoder_perf.md`

## 1. 目标

分两阶段达成与 FFmpeg bit-exact 对标:

### 阶段 A -- 功能正确(里程碑)

- 全部核心+扩展样本达到以下指标:
    - Y-PSNR >= 50dB.
    - U-PSNR >= 50dB.
    - V-PSNR >= 50dB.
    - 像素级精度(完全一致像素占比) >= 99%.
    - 最大单像素误差 <= 2.
- 帧数一致: Tao 与 FFmpeg 输出帧数完全相同.

### 阶段 B -- bit-exact(最终目标)

- 全部核心+扩展样本达到以下指标:
    - 像素精度 = 100%(逐字节完全一致).
    - Y/U/V-PSNR = Infinity.
    - 最大单像素误差 = 0.
- 与 `decoder_compare.rs` 中 `DEFAULT_REQUIRED_PRECISION = 100.0` 一致.

### 回归保护

- 建立精度基线, 任何后续变更不得引入回归.
- CI 门禁阻断精度下降的提交.

## 2. 对比基础设施

### 2.1 对比入口

- [x] 维护 `plans/tao-codec/video/h264/decoder_compare.rs` 作为核心对比工具.
- [x] 通过 `tests/run_decoder.rs` 的 `#[path]` 接入, 可用 `cargo test --test run_decoder` 执行.
- [x] 支持环境变量控制: `TAO_H264_COMPARE_INPUT`(输入文件/URL), `TAO_H264_COMPARE_FRAMES`(帧数),
      `TAO_H264_COMPARE_REQUIRED_PRECISION`(精度阈值).

### 2.2 逐帧统计

- [x] 对比输出: Y/U/V 各平面的 PSNR, 最大误差, 精度百分比.
- [x] 首个偏差帧自动定位: 输出 `first_mismatch_frame` 索引.
- [x] 帧数一致性检查: Tao 与 FFmpeg 解码帧数必须相同, 不一致时报错.

### 2.3 JSON 逐帧报告

- [x] 新增逐帧报告输出: 每帧独立的 Y/U/V PSNR, max_err, precision_pct.
- [x] 通过 `TAO_H264_COMPARE_REPORT=1` 启用, 输出到 `data/h264_compare_reports/`.
- [x] 报告格式: JSON 数组, 每个元素包含 `frame_idx`, `y_psnr`, `u_psnr`, `v_psnr`,
      `y_max_err`, `u_max_err`, `v_max_err`, `y_precision`, `u_precision`, `v_precision`.

### 2.4 CI 精度门禁

- [x] 在 `decoder_compare.rs` 中已新增精度回归测试(C1-C3, E1-E9, X1-X4).
- [ ] 精度下降超阈值时 CI 测试失败.
- [x] 样本已下载到 `data/h264_samples/` 本地目录, 对比使用本地路径避免重复网络请求.

### 2.5 环境变量完整列表

| 变量                                    | 默认值  | 说明                          |
| --------------------------------------- | ------- | ----------------------------- |
| `TAO_H264_COMPARE_INPUT`                | (必须)  | 输入文件路径或 URL            |
| `TAO_H264_COMPARE_FRAMES`               | `120`   | 对比帧数上限                  |
| `TAO_H264_COMPARE_REQUIRED_PRECISION`   | `100.0` | 精度阈值(%), 低于此值测试失败 |
| `TAO_H264_COMPARE_REPORT`               | `0`     | `1` 时输出逐帧 JSON 报告      |
| `TAO_H264_COMPARE_MB_DIAG`              | `0`     | `1` 时启用宏块级诊断输出      |
| `TAO_H264_COMPARE_TIMING`               | `0`     | `1` 时输出解码耗时统计        |
| `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK` | `0`     | `1` 时参考帧回退即失败        |

### 2.6 对比执行方式

```bash
# 运行全部样本批量对比
cargo test --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored

# 对比单个样本(手动指定路径)
TAO_H264_COMPARE_INPUT=data/h264_samples/c1_cavlc_baseline_720p.mp4 \
  cargo test --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 控制帧数与精度阈值
TAO_H264_COMPARE_FRAMES=30 \
TAO_H264_COMPARE_REQUIRED_PRECISION=99.0 \
TAO_H264_COMPARE_INPUT=data/h264_samples/c3_high_8x8.mkv \
  cargo test --test run_decoder h264::test_h264_compare -- --nocapture --ignored

# 启用逐帧报告
TAO_H264_COMPARE_REPORT=1 \
TAO_H264_COMPARE_INPUT=data/h264_samples/c1_cavlc_baseline_720p.mp4 \
  cargo test --test run_decoder h264::test_h264_compare -- --nocapture --ignored
```

### 2.7 自动轮转相关环境变量

| 变量                       | 默认值                                                                               | 说明                                 |
| -------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------ |
| `TAO_AUTO_TARGET_SAMPLE`   | `data/2.mp4`                                                                         | 自动轮转目标样本                     |
| `TAO_AUTO_KEEP_SAMPLE`     | `data/1.mp4`                                                                         | 自动轮转守护样本                     |
| `TAO_AUTO_TARGET_REQUIRED` | `100.0`                                                                              | 目标样本精度阈值(%)                  |
| `TAO_AUTO_KEEP_REQUIRED`   | `100.0`                                                                              | 守护样本精度阈值(%)                  |
| `TAO_AUTO_MAX_ROUNDS`      | `50`                                                                                 | 最大轮次, 防止无限循环               |
| `TAO_AUTO_STABLE_ROUNDS`   | `3`                                                                                  | 双样本连续达标后停止的稳定轮次数     |
| `TAO_AUTO_SKIP_STRICT`     | `0`                                                                                  | `1` 时跳过严格 5 项验证              |
| `TAO_AUTO_COMMIT_PATHS`    | `crates/tao-codec/src/decoders/h264 plans/tao-codec/video/h264 tests/run_decoder.rs` | 自动提交的白名单路径                 |
| `TAO_AUTO_COMMIT_TYPE`     | `fix`                                                                                | 自动提交信息前缀(`fix/refactor/...`) |

## 3. 测试样本

> 所有样本 URL 均已通过 `curl -sI` 验证可达(HTTP 200).
> 完整样本清单同步维护在 `samples/SAMPLE_URLS.md`.
> 样本已下载到 `data/h264_samples/` 目录, 对比测试使用本地路径.

**重要** 样本优先级调整:

- P0 首要样本: `data/1.mp4`(最高优先级, 必须先达到阶段 A 精度门槛, 再推进全样本收敛).
- P1 次要样本: `data/2.mp4`(最高优先级, 在确保P0 样本进度100%条件下, 推进本样本收敛).
- 回归样本集: `C1-C3`, `E1-E9`, `X1-X4` 保持现有覆盖与门禁, 作为首要样本达标后的批量回归集合.

### 3.1 核心样本

| 编号 | 本地路径                                       | URL                                                                            | Profile              | 分辨率    | 容器 | 覆盖特征                  |
| ---- | ---------------------------------------------- | ------------------------------------------------------------------------------ | -------------------- | --------- | ---- | ------------------------- |
| C1   | `data/h264_samples/c1_cavlc_baseline_720p.mp4` | `https://samples.ffmpeg.org/HDTV/Xacti-elst/MP4-AVC-SanyoXactiHD700-elst.mp4`  | Constrained Baseline | 1280x720  | MP4  | CAVLC, 无 B 帧, Level 3.1 |
| C2   | `data/h264_samples/c2_main_cabac_1080p.mov`    | `https://samples.ffmpeg.org/archive/container/mov/mov+h264+aac++bbc_1080p.mov` | Main                 | 1920x1080 | MOV  | CABAC, B 帧, Level 4.0    |
| C3   | `data/h264_samples/c3_high_8x8.mkv`            | `https://samples.ffmpeg.org/Matroska/haruhi.mkv`                               | High                 | 704x480   | MKV  | CABAC, 8x8 变换, B 帧     |

### 3.2 扩展样本

| 编号 | 本地路径                                         | 覆盖目标                  | URL                                                                                        | Profile  | 分辨率    | 关键特征                                     |
| ---- | ------------------------------------------------ | ------------------------- | ------------------------------------------------------------------------------------------ | -------- | --------- | -------------------------------------------- |
| E1   | `data/h264_samples/e1_baseline_cavlc_lowres.mp4` | Baseline + CAVLC 低分辨率 | `https://samples.ffmpeg.org/A-codecs/Nelly_Moser/h264_NellyMoser.mp4`                      | Baseline | 352x200   | 纯 CAVLC 路径, Level 2.1                     |
| E2   | `data/h264_samples/e2_main_cabac_720p.mov`       | Main + CABAC 720p         | `https://samples.ffmpeg.org/V-codecs/h264/bbc-africa_m720p.mov`                            | Main     | 1280x720  | CABAC + B 帧, Level 3.2                      |
| E3   | `data/h264_samples/e3_main_cabac_midres.mp4`     | Main + CABAC 中分辨率     | `https://samples.ffmpeg.org/V-codecs/h264/cathedral-beta2-400extra-crop-avc.mp4`           | Main     | 640x352   | CABAC + B 帧, Level 4.0, HE-AAC              |
| E4   | `data/h264_samples/e4_main_cabac_lowres.mov`     | Main + CABAC 低分辨率     | `https://samples.ffmpeg.org/archive/container/mov/mov+h264+aac++Demo_FlagOfOurFathers.mov` | Main     | 480x204   | CABAC + B 帧, Level 2.0                      |
| E5   | `data/h264_samples/e5_main_1080p.264`            | Main + 1080p 裸流         | `https://samples.ffmpeg.org/archive/all/h264+h264+++Fish_1080P_16M.264`                    | Main     | 1920x1088 | CABAC, Level 4.0, 高码率, height=1088 需裁剪 |
| E6   | `data/h264_samples/e6_high_1080p.h264`           | High + 1080p 裸流         | `https://samples.ffmpeg.org/archive/all/h264+h264+++ffh264_issue4.h264`                    | High     | 1920x1080 | CABAC + 8x8, Level 4.1, 裸流                 |
| E7   | `data/h264_samples/e7_high_1080p.mp4`            | High + 1080p MP4          | `https://samples.ffmpeg.org/HDTV/xacti_hd2000_dogsample20090207_2a.mp4`                    | High     | 1920x1080 | CAVLC + 8x8, Level 4.2, yuvj420p             |
| E8   | `data/h264_samples/e8_ipcm.h264`                 | IPCM 边界                 | `https://samples.ffmpeg.org/archive/all/h264+h264+++IPCM_decode_error.h264`                | High     | 352x288   | IPCM 宏块解码边界, Level 5.1                 |
| E9   | `data/h264_samples/e9_cavlc_baseline2.mp4`       | CAVLC Baseline 2          | `https://samples.ffmpeg.org/A-codecs/speex/h264_speex.mp4`                                 | Baseline | 352x200   | CAVLC, Level 3.1, 不同音频封装               |

### 3.3 自制定向样本

以下特征在现有公开样本中难以确认覆盖, 使用 FFmpeg 编码自制:

| 编号 | 覆盖目标          | 本地路径                                       | 生成命令                                                                                            | 状态   |
| ---- | ----------------- | ---------------------------------------------- | --------------------------------------------------------------------------------------------------- | ------ |
| X1   | I-only 纯帧内     | `data/h264_samples/custom_ionly.264`           | `ffmpeg -f lavfi -i testsrc=d=2:s=352x288:r=25 -c:v libx264 -pix_fmt yuv420p -g 1 -bf 0`            | 已生成 |
| X2   | B 帧覆盖          | `data/h264_samples/custom_poc1.264`            | `ffmpeg -f lavfi -i testsrc=d=2:s=352x288:r=25 -c:v libx264 -pix_fmt yuv420p`                       | 已生成 |
| X3   | P-only 无 B 帧    | `data/h264_samples/custom_poc2.264`            | `ffmpeg -f lavfi -i testsrc=d=2:s=352x288:r=25 -c:v libx264 -pix_fmt yuv420p -bf 0`                 | 已生成 |
| X4   | 多 slice 同帧     | `data/h264_samples/custom_multislice.264`      | `ffmpeg -f lavfi -i testsrc=d=5:s=352x288:r=25 -c:v libx264 -pix_fmt yuv420p -x264-params slices=4` | 已生成 |
| -    | MMCO 长期参考     | `data/h264_samples/custom_mmco_lt.264`         | 需 `long-term-ref-enable` 支持                                                                      | 待生成 |
| -    | gaps_in_frame_num | `data/h264_samples/custom_gap_frame_num.264`   | 需手动构造或从特定样本提取                                                                          | 待生成 |
| -    | 隐式加权预测      | `data/h264_samples/custom_implicit_weight.264` | 需 `weightp=0:weightb=1` 参数                                                                       | 待生成 |

> 自制样本仅供 `plans/` 下快速验证脚本使用, 不纳入正式测试(遵循 AGENTS.md 约束).

### 3.4 排除样本(非目标范围)

以下样本已确认为隔行扫描, 属于非目标范围(MBAFF/场编码不支持), 不纳入精度测试:

| URL                                                                     | 原因                                  |
| ----------------------------------------------------------------------- | ------------------------------------- |
| `https://samples.ffmpeg.org/V-codecs/h264/interlaced_crop.mp4`          | field_order=tt, Main, 640x360, 隔行   |
| `https://samples.ffmpeg.org/archive/all/h264+h264+++harm.h264`          | field_order=tt, Main, 720x480, 隔行   |
| `https://samples.ffmpeg.org/3D/issue1930.h264`                          | field_order=tt, High, 1920x1080, 隔行 |
| `https://samples.ffmpeg.org/archive/all/h264+h264+++ffh264_issue3.h264` | field_order=bb, High, 1920x1080, 隔行 |

## 4. 收敛流程

```text
循环:
  1. 先运行首要样本 `data/1.mp4` 逐帧对比, 输出报告
  2. 定位"首个偏差帧" -> 对应模块(帧内/帧间/残差/去块/DPB)
  3. 对齐 FFmpeg 源码, 定位根因
  4. 修复 + 补单测
  5. 回归复测 `data/1.mp4`, 达到阶段 A 后再复测全样本
  6. 若达标 -> 结束; 否则 -> 继续循环
```

### 4.1 定位方法

- **逐帧 dump**: Tao vs FFmpeg 各宏块的 MV/ref_idx/residual/重建像素.
- **逐宏块对比**: 在首个偏差帧中定位首个偏差宏块.
- **CABAC 状态 trace**: 对比 CABAC 上下文状态与 FFmpeg 的 ctxIdx/state/mps 演进.
- **环境变量开关**: 临时隔离模块(如强制 4x4/跳过去块)缩小范围.

#### FFmpeg 调试模式

```bash
# 使用 FFmpeg trace_headers 验证码流解析
ffmpeg -v trace -i input.mp4 -f null /dev/null 2>&1 | head -200

# 解码输出 rawvideo 用于手动对比
ffmpeg -y -i input.mp4 -pix_fmt yuv420p -vframes 10 -f rawvideo ref.yuv
```

### 4.2 常见偏差根因分类

| 类别           | 表现                        | 排查方向                              |
| -------------- | --------------------------- | ------------------------------------- |
| CABAC 语法失步 | slice 提前结束 / 宏块数不足 | 上下文增量 / 残差块类别 / 扫描顺序    |
| 帧内预测偏差   | I 帧像素系统性偏移          | 预测公式 / 邻居可用性 / 滤波          |
| 帧间预测偏差   | P/B 帧像素偏移              | MV 推导 / Direct 模式 / 加权预测      |
| 残差偏差       | 重建像素高频噪声            | 反量化 / IDCT 精度 / 扫描表           |
| 去块偏差       | 边界伪影                    | BS 计算 / alpha/beta/tc0 / 强弱滤波   |
| DPB/POC 偏差   | 错帧/错序                   | POC 计算 / 参考列表构建 / MMCO        |
| 色彩范围偏差   | 整体亮度偏移                | yuvj420p vs yuv420p, color_range 处理 |

### 4.3 门禁策略

精度收敛使用分级门禁:

- G0: 3 帧快速验证 (定位首帧链路问题)
- G1: 10 帧短门禁 (验证收敛趋势)
- G2: 67 帧中门禁 (覆盖 GOP 结构)
- G3: 299 帧全片验收 (最终精度)
- 每次有明显提升后执行严格 5 项验证与提交流程.

## 10. 特征覆盖矩阵

| 特征              | 核心样本 | 扩展样本   | 自制样本 |
| ----------------- | -------- | ---------- | -------- |
| CAVLC 熵编码      | C1       | E1, E9     | -        |
| CABAC 熵编码      | C2, C3   | E2-E8      | X1-X4    |
| Baseline Profile  | C1       | E1, E9     | -        |
| Main Profile      | C2       | E2-E5      | -        |
| High Profile      | C3       | E6-E8      | X1-X4    |
| B 帧              | C2, C3   | E2-E7      | X2       |
| 8x8 变换          | C3       | E6, E7     | X1-X4    |
| 1080p             | C2       | E5, E6, E7 | -        |
| 720p              | C1       | E2         | -        |
| IPCM              | -        | E8         | -        |
| I-only            | -        | -          | X1       |
| P-only 无 B 帧    | -        | -          | X3       |
| 多 slice 同帧     | -        | -          | X4       |
| MMCO 长期参考     | -        | -          | 待生成   |
| gaps_in_frame_num | -        | -          | 待生成   |
| 隐式加权预测      | -        | -          | 待生成   |
| 裸流 AnnexB       | -        | E5, E6, E8 | X1-X4    |

## 13. 关键代码位置参考

| 组件             | Tao 文件                               | FFmpeg 参考文件                            |
| ---------------- | -------------------------------------- | ------------------------------------------ |
| I_4x4/I_8x8 预测 | `decoders/h264/intra.rs`               | `libavcodec/h264pred_template.c`           |
| 模式可用性重映射 | `decoders/h264/macroblock_intra.rs`    | `libavcodec/h264_parse.c:130-210`          |
| P_Skip MV        | `decoders/h264/macroblock_inter.rs`    | `libavcodec/h264_mvpred.h:388-485`         |
| MV 中值预测      | `decoders/h264/macroblock_inter_mv.rs` | `libavcodec/h264_mvpred.h:226-277`         |
| B Direct spatial | `decoders/h264/macroblock_inter.rs`    | `libavcodec/h264_direct.c:140-600`         |
| MapColToList0    | `decoders/h264/macroblock_inter.rs`    | `libavcodec/h264_direct.c:82-137`          |
| CABAC 引擎       | `decoders/h264/cabac.rs`               | `libavcodec/cabac.h`, `cabac_functions.h`  |
| CAVLC 残差解码   | `decoders/h264/cavlc_mb.rs`            | `libavcodec/h264_cavlc.c`                  |
| 残差/反量化/IDCT | `decoders/h264/residual.rs`            | `libavcodec/h264_idct_template.c`          |
| 参考帧/DPB/输出  | `decoders/h264/output.rs`              | `libavcodec/h264_refs.c`, `h264_picture.c` |
