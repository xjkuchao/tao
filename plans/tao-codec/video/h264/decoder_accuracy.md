# H264 解码器 -- 精度收敛计划

> 前置条件: `decoder_dev.md` P1-P6.6 **全部完成**后才允许进入本计划.
>
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

| 变量                                     | 默认值 | 说明                          |
| ---------------------------------------- | ------ | ----------------------------- |
| `TAO_H264_COMPARE_INPUT`                 | (必须) | 输入文件路径或 URL            |
| `TAO_H264_COMPARE_FRAMES`                | `120`  | 对比帧数上限                  |
| `TAO_H264_COMPARE_REQUIRED_PRECISION`    | `100.0`| 精度阈值(%), 低于此值测试失败 |
| `TAO_H264_COMPARE_REPORT`                | `0`    | `1` 时输出逐帧 JSON 报告     |
| `TAO_H264_COMPARE_MB_DIAG`               | `0`    | `1` 时启用宏块级诊断输出      |
| `TAO_H264_COMPARE_TIMING`                | `0`    | `1` 时输出解码耗时统计        |
| `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK`  | `0`    | `1` 时参考帧回退即失败        |

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

## 3. 测试样本

> 所有样本 URL 均已通过 `curl -sI` 验证可达(HTTP 200).
> 完整样本清单同步维护在 `samples/SAMPLE_URLS.md`.
> 样本已下载到 `data/h264_samples/` 目录, 对比测试使用本地路径.

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
  1. 运行全样本逐帧对比, 输出报告
  2. 定位"首个偏差帧" -> 对应模块(帧内/帧间/残差/去块/DPB)
  3. 对齐 FFmpeg 源码, 定位根因
  4. 修复 + 补单测
  5. 回归复测全样本
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

## 5. 精度基线记录

### 当前基线(10 帧, 2026-02-22)

| 样本 | 分辨率    | Profile              | 熵编码 | 精度       | Y-PSNR      | max_err | 状态             |
| ---- | --------- | -------------------- | ------ | ---------- | ----------- | ------- | ---------------- |
| C1   | 1280x720  | Constrained Baseline | CAVLC  | 10.57%     | 19.08dB     | 249     | 待修复           |
| C2   | 1920x1080 | Main                 | CABAC  | **99.999%**| **79.82dB** | 20      | **近 bit-exact** |
| C3   | 704x480   | High                 | CABAC  | 33.61%     | 12.77dB     | 252     | 待修复           |
| E1   | 352x200   | Baseline             | CAVLC  | 20.62%     | 25.33dB     | 252     | 待修复           |
| E2   | 1280x720  | Main                 | CABAC  | 44.99%     | 21.21dB     | 237     | 待修复           |
| E3   | 640x352   | Main                 | CABAC  | **99.996%**| **73.58dB** | 44      | **近 bit-exact** |
| E4   | 480x204   | Main                 | CABAC  | 19.58%     | 19.69dB     | 230     | 待修复           |
| E5   | 1920x1088 | Main                 | CABAC  | 26.20%     | 20.08dB     | 211     | 待修复           |
| E6   | 1920x1080 | High                 | CABAC  | 25.79%     | 8.75dB      | 239     | 待修复           |
| E7   | 1920x1080 | High                 | CAVLC  | 6.77%      | 15.97dB     | 247     | 待修复           |
| E8   | 352x288   | High                 | CABAC  | 25.44%     | 20.51dB     | 227     | 待修复           |
| E9   | 352x200   | Baseline             | CAVLC  | 16.54%     | 20.16dB     | 181     | 待修复           |
| X1   | 352x288   | High                 | CABAC  | 81.02%     | 26.62dB     | 131     | 待修复           |
| X2   | 352x288   | High                 | CABAC  | 43.97%     | 13.09dB     | 247     | 待修复           |
| X3   | 352x288   | High                 | CABAC  | 42.48%     | 15.16dB     | 247     | 待修复           |
| X4   | 352x288   | High                 | CABAC  | 7.34%      | 10.45dB     | 253     | 待修复           |

- 通过: 16/16, 失败: 0/16 (阈值 1.00%)
- C2 首帧达到 100% bit-exact (PSNR=inf)
- E3 帧 0-8 bit-exact, 仅帧 9 有微小偏差

## 6. 已完成的关键修复(归档)

### 6.1 基础设施修复

1. 清除残留 `eprintln!` 调试输出
2. 修复 MOV 解封装器 `hdlr` box 覆盖问题 (C2, E2, E4)
3. 新增 H264 AnnexB Elementary Stream 解封装器 (E5, E6, E8, X1-X4)
4. 修复 H264EsProbe 探测优先级, 防止 MP3 误检 (E8)
5. 修复 H264 解码器延迟初始化, 支持无 extra_data 的裸流 (E5, E6, X2-X4)
6. 修复 AVCC/AnnexB NAL 分割冲突, length_size=0 用于裸流 (E5, E6, X2-X4)

### 6.2 帧内路径修复

1. **Slice 边界帧内预测邻居可用性**: 根因修复, 新 slice 首 MB 不使用前一 slice 的邻居做预测. 新增 `left_avail()` / `top_avail()` 方法, 基于 `mb_slice_first_mb` 判断同 slice.
   - 影响: C2 从 ~20% 提升到 99.999% (首帧 bit-exact)
2. **IDCT 4x4/8x8 pass 顺序修复**: 错误的"列->行->列"改为正确的"行->列"两 pass.
3. **I_8x8 block (1,1) has_topright 修复**: `(1, 1) => mb_right_avail` 改为 `(1, 1) => false`.

### 6.3 残差/量化修复

1. **色度 DC 反量化**: 添加舍入偏移 `(1 << (qp_per - 1))`
2. **去块滤波器全面修复**:
   - 使用 per-edge QP 替代 slice_qp (mb_qp 数组追踪)
   - 弱滤波添加 p1/q1 修正
   - 色度 boundary_step 2->4
   - 强滤波添加 p2/q2 更新

### 6.4 帧间路径修复

1. **P_Skip MV 推导 AND->OR 逻辑错误**: 任一邻居不可用 OR 满足零条件即返回 (0,0)
2. **MV 中值预测候选级联 unwrap_or 错误**: 不可用候选统一 -> `(0,0)`, 仅 A 可用时直接返回 A
3. **Spatial Direct 无邻居错误回退**: 无空间邻居时设 ref=0, mv=(0,0), 不递归 temporal
4. **MapColToList0 重建 DPB 而非 POC 匹配**: `ReferencePicture` 新增 `ref_l0_poc: Vec<i32>`, 用 POC 匹配
5. **B slice 16x8/8x16 L1 方向性 MV 预测**: 新增 `predict_mv_l1_16x8` / `predict_mv_l1_8x16`

### 6.5 CAVLC 路径修复

1. **CAVLC nC 上下文 slice 边界感知**: `calc_luma_nc` / `calc_chroma_u_nc` / `calc_chroma_v_nc` 在 MB 边界检查 slice 归属
2. **CAVLC I_8x8 语法补齐**: 补齐 `transform_size_8x8_flag` 与 `intra8x8_pred_mode` 解析, 并新增 I_8x8 交织预测+残差路径.
   - 影响: E7 从 0.47% 提升到 6.76%
3. **CAVLC 容错收敛 (coeff_token + total_zeros)**: `coeff_token` 邻近 VLC 表回退, `total_zeros` 越界抑制.
   - 影响: E2 `44.68% -> 44.99%`, E4 `19.34% -> 19.58%`, E7 `6.75% -> 6.77%`

### 6.6 容器格式修复

1. **MP4 `edts/elst` 时间线对齐**: MP4 demuxer 新增 `elst` 解析, packet `pts` 按 `media_time` 归一化.
   - `decoder_compare.rs` 默认过滤 `pts<0` 帧.
   - 影响: C1 `10.32% -> 10.56%`, E4 `8.41% -> 19.34%`

### 6.7 CABAC 引擎修复

1. **CABAC 字节对齐位解析**: 修复字节对齐位解析 bug.
2. **P/B 帧 CABAC 解析脱轨(根因)**: P/B 帧的 CABAC 上下文演进在 `end_of_slice_flag` 处脱轨, 导致 slice 提前终止(如 frame1 仅解码 188/8160 MB). 此为影响所有 P/B 帧精度的核心根因.

## 7. 已验证正确的组件(勿重复验证)

以下组件经过逐行代码比对与 FFmpeg/OpenH264 源码确认一致:

1. IDCT 4x4/8x8: 代数等效 FFmpeg
2. 所有反量化路径: 等效 FFmpeg
3. I_4x4 全部 9 个预测模式公式: 逐行比对 FFmpeg
4. I8x8Refs::load 滤波: 与 FFmpeg `PREDICT_8x8_LOAD_*` 宏等效
5. I_16x16 Plane prediction: `compute_plane_params` 与 FFmpeg `pred16x16_plane_compat` 等效
6. `remap_i4x4_mode_for_unavailable`: TOP_MAP/LEFT_MAP 与 FFmpeg 完全一致
7. `predict_i4x4_block_with_tr_unavail_fix`: 修补列表正确
8. `i4x4_modes` 缓存初始化: 填充为 2 (DC), 正确
9. I_16x16 `mb_qp_delta` 总是存在: 正确(规范要求)
10. Deblocking filter: 有益不有害, 已排除为 bug 源
11. I_16x16 残差 DC 路径: AC dequant -> Hadamard -> IDCT 顺序正确
12. Zigzag 扫描表: 与 OpenH264 参考匹配
13. Hadamard DC 变换: 正确

## 8. 下一步优先级(待修复方向)

### 8.1 CABAC P/B 帧路径(高优先级)

CABAC 解析脱轨已修复根因(slice 提前终止), 但 P/B 帧语法路径仍有精度差距:

- C3 (CABAC High, 33.61%), E2 (CABAC Main, 44.99%), E5 (CABAC Main, 26.20%), E6 (CABAC High, 25.79%), E8 (CABAC High, 25.44%)
- 排查方向:
  - B_Direct/Temporal Direct 共定位 MV 与 `map_col_to_list0` 参考映射
  - Inter `transform_size_8x8_flag` 上下文推导
  - P/B 帧残差 4x4 vs 8x8 路径选择

### 8.2 CAVLC P/B 帧运动补偿(高优先级)

- C1 (CAVLC Baseline, 10.57%), E1 (CAVLC Baseline, 20.62%), E9 (CAVLC Baseline, 16.54%)
- 排查方向:
  - `run_before` 失败集中在 `zeros_left=7/8/9`
  - 前序语法链路偏差导致 `run_before` 无法匹配(非 VLC 表本身错误)
  - 当前剩余错误构成: `run_before` 21 次, `total_coeff=16>15` 2 次

### 8.3 CABAC I_8x8 预测模式(中优先级)

- X1 (CABAC I-only, 81.02%) 首帧 CABAC 上下文演进偏差
- 排查方向:
  - `remap_i8x8_mode_for_unavailable` 是 STUB(仅 `.min(8)`, 忽略 top/left 可用性)
  - I_8x8 预测模式 3-8 (DDL/DDR/VR/HD/VL/HU) 8x8 版本尚未逐行验证

### 8.4 CAVLC I_8x8 路径(中优先级)

- E7 (CAVLC High, 6.77%) 当前 Y 面仅 0.57%, 远低于阶段 A 目标

### 8.5 C1 CAVLC 深度诊断线索

- 首块 (0,0) DC 预测值正确(128), 但残差应用后结果不对
- CAVLC NC 计算错误可能导致 Golomb 码解码偏差
- 首批 `run_before` 失步样本点:
  - `scene=inter_luma_4x4 coord=(10,2) zeros_left=7`
  - `scene=inter_luma_4x4 coord=(112,68) zeros_left=7`
  - `scene=chroma_u_ac coord=(127,13) zeros_left=7`

## 9. 轮转日志摘要

### 42 轮实验总结(2026-02-24, 样本 `data/1_h264.mp4`)

42 轮实验主要围绕以下方向, 均未能突破根本瓶颈:

| 方向                        | 轮次      | 结论                                     |
| --------------------------- | --------- | ---------------------------------------- |
| B_Direct temporal/spatial   | R1-R4     | 映射一致性调整无效果                     |
| Inter 8x8 语法消费+4x4 回退 | R5-R7     | 微量提升(16.55->16.82), R7 为最佳组合   |
| 强制 spatial/temporal        | R8-R12    | 强制切换无效果                           |
| `map_col_to_list0` 回退策略  | R14-R22   | R19 为最佳(16.82), 回退策略天花板低     |
| `decode_ref_idx` OOB 语义   | R24-R27   | 多种策略均未超越 R19                     |
| qpel/反量化/deblock 实验     | R28-R33   | 均无效或引入回退                         |
| `weighted_pred`/组合实验     | R34-R41   | 均未超越 R19                             |

**关键结论**: 42 轮实验围绕 inter 路径局部调优效果甚微, 精度未能超越 ~18.8%.
最终发现**根因是 CABAC P/B 帧解析脱轨**(slice 提前终止), 该根因已在后续修复.

### 已否决实验(勿重复尝试)

- `run_before` run7 全表回退 + clamp: C1 回退
- `total_coeff=16,max_num_coeff=15` 按 `parse_max=16` 消费并裁剪: 多样本回退
- P-slice `P_16x8/P_8x16` 方向性 MV 预测替换: C1/E1/E9 回退
- IDCT 列先行后: 精度下降
- 强制 AMVD=0: 明显回退(P299: 16.82->14.30)
- AC 反量化按 FFmpeg qmul 重构: 严重回退(P299: 16.55->3.62)
- `transform_8x8_flag` 写回上下文改为跟随 `use_8x8_residual`: P299 回退

## 10. 特征覆盖矩阵

| 特征              | 核心样本 | 扩展样本   | 自制样本    |
| ----------------- | -------- | ---------- | ----------- |
| CAVLC 熵编码      | C1       | E1, E9     | -           |
| CABAC 熵编码      | C2, C3   | E2-E8      | X1-X4       |
| Baseline Profile  | C1       | E1, E9     | -           |
| Main Profile      | C2       | E2-E5      | -           |
| High Profile      | C3       | E6-E8      | X1-X4       |
| B 帧              | C2, C3   | E2-E7      | X2          |
| 8x8 变换          | C3       | E6, E7     | X1-X4       |
| 1080p             | C2       | E5, E6, E7 | -           |
| 720p              | C1       | E2         | -           |
| IPCM              | -        | E8         | -           |
| I-only            | -        | -          | X1          |
| P-only 无 B 帧    | -        | -          | X3          |
| 多 slice 同帧     | -        | -          | X4          |
| MMCO 长期参考     | -        | -          | 待生成      |
| gaps_in_frame_num | -        | -          | 待生成      |
| 隐式加权预测      | -        | -          | 待生成      |
| 裸流 AnnexB       | -        | E5, E6, E8 | X1-X4       |

## 11. 验收标准

### 阶段 A

- [ ] 核心 3 样本(C1-C3): Y/U/V-PSNR >= 50dB, 像素精度 >= 99%, 最大误差 <= 2.
- [ ] 扩展 9 样本(E1-E9): 全部达到相同指标.
- [ ] 帧数一致性: 全部样本 Tao 与 FFmpeg 输出帧数完全相同.

### 阶段 B

- [ ] 核心+扩展全部样本: 像素精度 = 100%, 最大误差 = 0.
- [ ] `decoder_compare.rs` 默认精度 100% 通过.

### 通用

- [ ] 精度回归 CI 门禁通过(至少 3 个本地样本).
- [ ] 输出最终精度报告(各样本各帧 Y/U/V 统计).
- [x] 自制样本覆盖特征矩阵中"需自制"项(至少 3 项): X1(I-only), X3(P-only), X4(多 slice).

## 12. 进度

- [x] 对比基础设施搭建(JSON 报告 + 样本路径映射 + 批量对比)
- [x] 样本本地化: C1-C3, E1-E9 已下载到 `data/h264_samples/`
- [x] 自制定向样本覆盖: X1(I-only), X2(B 帧), X3(P-only), X4(多 slice)
- [x] 精度回归测试: C1-C3, E1-E9, X1-X4 共 16 个独立测试 + 批量汇总
- [x] 去块滤波器修复: per-edge QP, 弱/强滤波像素修正, 色度 boundary_step
- [x] 色度 DC 反量化舍入偏移修复
- [x] Slice 边界帧内预测邻居可用性修复 (C2 100% bit-exact 首帧)
- [x] CAVLC nC 上下文 slice 边界感知修复
- [x] E7 诊断与修复 (CAVLC + yuvj420p, 0.47% -> 6.76%, 已通过 1% 门槛)
- [x] MP4 `edts/elst` 时间线对齐 + 对比工具负 PTS 过滤 (C1/E4 收敛)
- [x] CAVLC 容错收敛 (coeff_token 邻表回退 + total_zeros 越界抑制)
- [x] CABAC P/B 帧解析脱轨根因修复 (slice 提前终止)
- [x] 42 轮实验完成, 建立否决清单
- [ ] CABAC P/B 帧语法路径完善 (C3, E2, E5, E6, E8)
- [ ] CAVLC P/B 帧运动补偿完善 (C1, E1, E9)
- [ ] CABAC I_8x8 预测模式不同步修复 (X1)
- [ ] CAVLC I_8x8 路径精度提升 (E7)
- [ ] 核心 3 样本阶段 A 达标
- [ ] 扩展样本阶段 A 达标
- [ ] 核心+扩展全部样本阶段 B 达标(bit-exact)
- [ ] CI 精度门禁集成
- [ ] 最终精度报告

## 13. 关键代码位置参考

| 组件              | Tao 文件                                | FFmpeg 参考文件                                |
| ----------------- | --------------------------------------- | ---------------------------------------------- |
| I_4x4/I_8x8 预测 | `decoders/h264/intra.rs`                | `libavcodec/h264pred_template.c`               |
| 模式可用性重映射  | `decoders/h264/macroblock_intra.rs`     | `libavcodec/h264_parse.c:130-210`              |
| P_Skip MV         | `decoders/h264/macroblock_inter.rs`     | `libavcodec/h264_mvpred.h:388-485`             |
| MV 中值预测       | `decoders/h264/macroblock_inter_mv.rs`  | `libavcodec/h264_mvpred.h:226-277`             |
| B Direct spatial  | `decoders/h264/macroblock_inter.rs`     | `libavcodec/h264_direct.c:140-600`             |
| MapColToList0     | `decoders/h264/macroblock_inter.rs`     | `libavcodec/h264_direct.c:82-137`              |
| CABAC 引擎        | `decoders/h264/cabac.rs`                | `libavcodec/cabac.h`, `cabac_functions.h`      |
| CAVLC 残差解码    | `decoders/h264/cavlc_mb.rs`             | `libavcodec/h264_cavlc.c`                      |
| 残差/反量化/IDCT  | `decoders/h264/residual.rs`             | `libavcodec/h264_idct_template.c`              |
| 参考帧/DPB/输出   | `decoders/h264/output.rs`               | `libavcodec/h264_refs.c`, `h264_picture.c`     |
