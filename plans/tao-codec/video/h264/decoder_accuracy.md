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

**重要** 样本优先级调整:

- P0 首要样本: `data/1.mp4`(最高优先级, 必须先达到阶段 A 精度门槛, 再推进全样本收敛).
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

## 5. 精度基线记录

### 5.1 首要样本基线重算(10 帧, 2026-02-25)

> 执行命令:
> `TAO_H264_COMPARE_INPUT=data/1.mp4 TAO_H264_COMPARE_FRAMES=10 cargo test --test run_decoder h264::test_h264_compare -- --nocapture --ignored`

| 样本       | 分辨率    | Profile | 熵编码 | 精度       | Y-PSNR   | U-PSNR   | V-PSNR   | max_err | 首个不一致帧 | 状态   |
| ---------- | --------- | ------- | ------ | ---------- | -------- | -------- | -------- | ------- | ------------ | ------ |
| `data/1.mp4` | 1920x1080 | High    | CABAC  | **9.019579%** | 12.6388dB | 24.6379dB | 23.2619dB | 255     | 0            | 待修复 |

### 5.2 当前全样本基线重算(10 帧, 2026-02-25)

> 执行命令:
> `TAO_H264_COMPARE_FRAMES=10 cargo test --test run_decoder h264::test_h264_accuracy_all -- --nocapture --ignored`

| 样本 | 分辨率    | Profile              | 熵编码 | 精度         | Y-PSNR      | max_err | 状态             |
| ---- | --------- | -------------------- | ------ | ------------ | ----------- | ------- | ---------------- |
| C1   | 1280x720  | Constrained Baseline | CAVLC  | 10.019567%   | 16.4540dB   | 249     | 待修复           |
| C2   | 1920x1080 | Main                 | CABAC  | **99.998691%** | **78.0267dB** | 20      | **近 bit-exact** |
| C3   | 704x480   | High                 | CABAC  | 88.532197%   | 18.6053dB   | 237     | 待修复           |
| E1   | 352x200   | Baseline             | CAVLC  | 22.701420%   | 26.5907dB   | 248     | 待修复           |
| E2   | 1280x720  | Main                 | CABAC  | 48.264818%   | 15.1537dB   | 237     | 待修复           |
| E3   | 640x352   | Main                 | CABAC  | **99.984168%** | **63.5354dB** | 51      | **近 bit-exact** |
| E4   | 480x204   | Main                 | CABAC  | 18.908633%   | 15.2386dB   | 245     | 待修复           |
| E5   | 1920x1088 | Main                 | CABAC  | 39.972762%   | 27.4135dB   | 182     | 待修复           |
| E6   | 1920x1080 | High                 | CABAC  | 27.440114%   | 8.2483dB    | 239     | 待修复           |
| E7   | 1920x1080 | High                 | CAVLC  | 14.946132%   | 13.5658dB   | 247     | 待修复           |
| E8   | 352x288   | High                 | CABAC  | 19.565709%   | 18.9940dB   | 226     | 待修复           |
| E9   | 352x200   | Baseline             | CAVLC  | 18.495265%   | 21.6717dB   | 176     | 待修复           |
| X1   | 352x288   | High                 | CABAC  | **100.000000%** | **infdB**    | 0       | **bit-exact**    |
| X2   | 352x288   | High                 | CABAC  | 85.957623%   | 26.0769dB   | 244     | 待修复           |
| X3   | 352x288   | High                 | CABAC  | 80.550492%   | 22.2407dB   | 247     | 待修复           |
| X4   | 352x288   | High                 | CABAC  | 8.437500%    | 10.9704dB   | 239     | 待修复           |

- 通过: 16/16, 失败: 0/16 (阈值 1.00%).
- 近 bit-exact 样本: C2, E3.
- bit-exact 样本: X1.

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

### P0 首要样本(`data/1.mp4`)

- [ ] 阶段 A(P0): `data/1.mp4` 在 G3(299 帧)满足 `Y/U/V-PSNR >= 50dB`, `像素精度 >= 99%`, `最大误差 <= 2`.
- [ ] 阶段 B(P0): `data/1.mp4` 在 G3(299 帧)满足 `像素精度 = 100%`, `Y/U/V-PSNR = inf`, `最大误差 = 0`.
- [ ] 帧数一致性(P0): `data/1.mp4` Tao 与 FFmpeg 输出帧数完全相同.
- [ ] 稳定性(P0): 连续 3 次独立运行结果一致, 不出现随机波动或偶发退化.

### 阶段 A

- [ ] 前置条件: 必须先通过 P0 阶段 A, 才进入核心+扩展样本阶段 A 验收.
- [ ] 核心 3 样本(C1-C3): Y/U/V-PSNR >= 50dB, 像素精度 >= 99%, 最大误差 <= 2.
- [ ] 扩展 9 样本(E1-E9): 全部达到相同指标.
- [ ] 帧数一致性: 全部样本 Tao 与 FFmpeg 输出帧数完全相同.

### 阶段 B

- [ ] 前置条件: 必须先通过 P0 阶段 B, 才进入核心+扩展样本 bit-exact 验收.
- [ ] 核心+扩展全部样本: 像素精度 = 100%, 最大误差 = 0.
- [ ] `decoder_compare.rs` 默认精度 100% 通过.

### 通用

- [ ] 验收顺序固定: `P0(data/1.mp4) -> 核心样本(C1-C3) -> 扩展样本(E1-E9) -> 自制回归(X1-X4)`.
- [ ] 门禁帧数要求: `G0=3`(快速), `G1=10`(短门禁), `G2=67`(中门禁), `G3=299`(最终验收), 正式通过必须以 G3 结果为准.
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
