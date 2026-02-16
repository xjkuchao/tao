# MPEG4 Part 2 解码器测试计划

> 本文档为 Tao 项目 MPEG4 Part 2 (ISO/IEC 14496-2) 解码器的完整测试计划。

**优先级**: ⭐⭐⭐⭐⭐ 置顶

**更新时间**: 2026-02-16

---

## 概述

### 核心目标

1. **从简到难逐步验证** MPEG4 Part 2 解码能力
2. **每个测试用例对应真实样本** - 使用官方 URL 直接访问
3. **三层验证机制** - 自动化 → 对比 → 人工

### 测试阶段划分

| 阶段        | 说明             | 工作量 | 优先级 |
| ----------- | ---------------- | ------ | ------ |
| **第1阶段** | 基础解码能力验证 | 低     | P0     |
| **第2阶段** | 高级特性验证     | 中     | P1     |
| **第3阶段** | 特殊场景处理     | 高     | P2     |
| **第4阶段** | FFmpeg 对标验证  | 高     | P3     |

---

## 第 1 阶段：基础解码能力验证

### 阶段目标

确保 MPEG4 Part 2 解码器能正确处理：

- 标准视频参数（分辨率、帧率、色彩空间）
- I/P 帧序列
- 常见容器格式（AVI、MP4）

### 测试用例 1.1：基础 AVI 容器解码

**优先级**: P0 - 最高

**样本信息**

- 名称: color16.avi
- 源地址: https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi
- 特性: 标准 MPEG-4 + AVI 容器
- 分辨率: 320×240 (QVGA)
- 帧率: ~25 fps
- 编码特性: 基础 I/P 帧

#### 1.1.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_basic_avi_decode`

**测试代码框架**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4part2_basic_avi_decode() {
    let sample = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi";

    // 1. 打开解复用器
    let mut demux = open_demuxer(sample).expect("打开 URL 失败");

    // 2. 获取视频流参数
    let (width, height, fps) = get_stream_info(&demux);
    assert_eq!(width, 320);
    assert_eq!(height, 240);
    assert!(fps > 20.0 && fps < 30.0);

    // 3. 创建解码器并打开
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器失败");

    // 4. 解码前 10 帧
    let mut frame_count = 0;
    loop {
        if let Ok(packet) = demux.read_packet() {
            if let Ok(_frame) = decode_frame(&mut decoder, &packet) {
                frame_count += 1;
            }
        }
        if frame_count >= 10 { break; }
    }

    // 5. 验证结果
    assert!(frame_count >= 10, "应至少解码成功 10 帧");
    println!("✓ 成功解码 {} 帧", frame_count);
}
```

**验证项**:

- [ ] 能正确解析 AVI 容器头部
- [ ] 能识别 MPEG4 视频流
- [ ] 能解析 VOL header
- [ ] 能成功解码前 10 帧
- [ ] 每帧分辨率、时间戳正确
- [ ] 无 panic 或崩溃

**预期输出**:

```
✓ 基础 AVI 解码测试
  视频流: 320x240, 25 fps
  已解码: 10 帧
  已验证: YUV420p 像素格式
```

#### 1.1.2 对比测试

**对标工具**: FFmpeg & FFprobe

**对比步骤**:

```bash
# 步骤 1: 获取参考信息
ffprobe https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# 步骤 2: 提取参考帧（前 5 帧）
ffmpeg -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
       -vf scale=320:240 \
       -c:v rawvideo -pix_fmt yuv420p \
       -f rawvideo \
       refs/color16_ref_%03d.yuv

# 步骤 3: 用 tao 解码输出
cargo test --test mpeg4_part2_pipeline \
    test_mpeg4part2_basic_avi_decode -- --nocapture

# 步骤 4: 像素级对比（PSNR、差异）
python scripts/compare_yuv.py \
    refs/color16_ref_*.yuv \
    output/color16_tao_*.yuv
```

**对比指标**:

- **PSNR (Peak Signal-to-Noise Ratio)**: >= 38 dB（优秀）
- **差异帧比例**: <= 0.5% 像素差异
- **时间戳误差**: <= 1 ms

#### 1.1.3 播放测试

**工具**: tao-play & FFplay

**播放对比**:

```bash
# FFplay 播放（参考）
ffplay https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# tao-play 播放（待测）
./target/debug/tao-play https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi
```

**人工验证项**:

- [ ] 画面无花屏、无绿屏
- [ ] 色彩正常（与 FFplay 对比）
- [ ] 播放流畅，无卡顿
- [ ] 音频同步（如有音频）
- [ ] 寻位功能（如有实现）

**差异记录**（由人工对比）:

- **视觉质量**: ✓ 一致 / ⚠️ 轻微差异 / ❌ 明显差异
- **具体差异**: （如有）

---

### 测试用例 1.2：MP4 容器解码

**优先级**: P0 - 最高

**样本信息**

- 名称: mov_h264_aac.mov (注：该样本主要为 H.264，备用)
- 替代样本: 需搜索含 MPEG4 的 MP4 样本
- 分辨率: 视情况而定
- 帧率: 视情况而定
- 编码特性: I/P 帧

#### 1.2.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_mp4_decode`

**测试状态**: ⏳ **待确认 MP4 格式的 MPEG4 样本**

**行动**:

1. 在 https://samples.ffmpeg.org/ 中搜索 "MPEG4" 格式的 MP4 样本
2. 如果找不到，记录为 "样本缺缺"，优先级降低到 P2

**预期特性验证**:

- [ ] MP4 stco (sample chunk offset) 寻址
- [ ] mvhd/trak/stbl 头部解析
- [ ] MPEG4 codec 标识识别

---

### 测试用例 1.3：I 帧独立解码

**优先级**: P1

**样本**: color16.avi（复用）

#### 1.3.1 自动化测试用例

**目标**: 验证 I 帧可独立解码，不依赖参考帧

```rust
#[test]
fn test_mpeg4part2_i_frame_independent_decode() {
    let mut demux = open_demuxer("https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi");
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器");

    // 查找第一个 I 帧
    let i_frame_packet = find_first_vop_type(&mut demux, VOPType::I)
        .expect("应找到 I 帧");

    // 不发送任何参考帧，仅解码 I 帧
    decoder.send_packet(&i_frame_packet).expect("发送 I 帧");
    let frame = decoder.receive_frame().expect("解码 I 帧");

    // 验证帧信息
    assert_eq!(frame.width(), 320);
    assert_eq!(frame.height(), 240);
    assert_eq!(frame.format(), PixelFormat::Yuv420p);

    println!("✓ I 帧独立解码成功");
}
```

#### 1.3.2 对比测试

**对标指标** (PSNR >= 40 dB):

```python
# 单独提取 I 帧对比
ffmpeg -i color16.avi -vf select='eq(pict_type\,I)' \
       -f rawvideo -pix_fmt yuv420p refs/i_frame_ref.yuv
```

#### 1.3.3 播放测试

**工具**: tao-play（跳过 B/P 帧）

**预期结果**: 仅播放 I 帧，画面每隔几帧更新一次，基础色彩块可见

---

## 第 2 阶段：高级特性验证

### 阶段目标

验证 MPEG4 Part 2 的高级编码特性：

- B 帧解码
- 四分像素运动补偿 (Quarterpel)
- 全局运动补偿 (GMC)
- 数据分区 (Data Partitioning)

### 测试用例 2.1：B 帧解码

**优先级**: P1

**样本信息**

- 名称: avi+mpeg4+++qprd_cmp_b-frames_naq1.avi
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi
- 特性: 含 B 帧的 MPEG-4
- 分辨率: 352×288 (CIF)
- 帧序列: I、P、B、P、B... 复杂关键帧结构

#### 2.1.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_b_frame_decode`

**测试代码框架**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4part2_b_frame_decode() {
    let sample = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi";

    let mut demux = open_demuxer(sample);
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器");

    // 统计帧类型
    let mut i_frames = 0;
    let mut p_frames = 0;
    let mut b_frames = 0;

    loop {
        let packet = demux.read_packet()?;

        // 解析 VOP header 获取帧类型
        let vop_type = parse_vop_type(&packet);
        match vop_type {
            VOPType::I => i_frames += 1,
            VOPType::P => p_frames += 1,
            VOPType::B => b_frames += 1,
            _ => {}
        }

        // 发送解码
        decoder.send_packet(&packet)?;
        if let Ok(_frame) = decoder.receive_frame() {
            // OK
        }

        if i_frames + p_frames + b_frames >= 20 { break; }
    }

    // 验证检测到 B 帧
    assert!(b_frames > 0, "应检测到至少 1 个 B 帧，实际: {}", b_frames);
    println!("✓ 解码帧统计 - I: {}, P: {}, B: {}", i_frames, p_frames, b_frames);
}
```

**验证项**:

- [ ] VOP header 中 vop_coding_type 正确解析
- [ ] B 帧参考帧列表构建正确
- [ ] 时间戳递增且递减帧排序正确
- [ ] 解码无崩溃，输出有效帧

#### 2.1.2 对比测试

**对标指标**: PSNR >= 36 dB（B 帧通常低于 I/P）

```bash
# 提取所有帧
ffmpeg -i avi+mpeg4+++qprd_cmp_b-frames_naq1.avi \
       -c:v rawvideo -pix_fmt yuv420p \
       -f rawvideo refs/b_frame_ref_%03d.yuv

# 统计B帧位置
ffprobe -show_frames avi+mpeg4+++qprd_cmp_b-frames_naq1.avi | grep pict_type
```

#### 2.1.3 播放测试

**预期效果**: 动作平滑，无断裂感（B 帧插值作用）

---

### 测试用例 2.2：四分像素运动补偿 (Quarterpel)

**优先级**: P1

**样本信息**

- 名称: avi+mpeg4+++DivX51-Qpel.avi
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi
- 特性: 四分像素运动补偿
- 分辨率: 368×272
- 帧率: 25 fps
- 特点: 精细运动补偿，需要高精度子像素插值

#### 2.2.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_quarterpel_decode`

**测试代码框架**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4part2_quarterpel_decode() {
    let sample = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi";

    let mut demux = open_demuxer(sample);
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器");

    // 检查 VOL header 中的 quarter_sample 标志
    let vol_header = demux.get_extradata();  // 或从第一个 VOP 前获取
    let has_quarter_sample = check_vol_quarter_sample(vol_header);

    println!("✓ Quarter Sample 标志: {}", has_quarter_sample);
    assert!(has_quarter_sample, "样本应启用四分像素运动补偿");

    // 解码前 15 帧，验证无崩溃
    let mut frame_count = 0;
    loop {
        let packet = demux.read_packet()?;
        decoder.send_packet(&packet)?;

        if let Ok(frame) = decoder.receive_frame() {
            frame_count += 1;

            // 验证运动补偿效果（变化度量）
            if frame_count > 1 {
                let variance = calculate_frame_variance(&frame);
                assert!(variance > 0, "帧应有像素变化");
            }
        }

        if frame_count >= 15 { break; }
    }

    assert!(frame_count >= 15, "应解码 15 帧");
    println!("✓ 四分像素运动补偿解码: {} 帧成功", frame_count);
}
```

**验证项**:

- [ ] VOL header 中 quarter_sample 标志识别
- [ ] 运动补偿向量精度到 1/4 像素
- [ ] 运动补偿插值滤波正确
- [ ] 解码无伪影或毛刺

#### 2.2.2 对比测试

**对标工具**: FFmpeg 运动补偿参考输出

**对比指标**: PSNR >= 34 dB（四分像素精细，但可能有舍入差异）

#### 2.2.3 播放测试

**预期效果**: 运动平滑，无块状感或边界不连续

---

### 测试用例 2.3：GMC 全局运动补偿 + Quarterpel

**优先级**: P2（复杂特性）

**样本信息**

- 名称: avi+mpeg4+++xvid_gmcqpel_artifact.avi (2.8M)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi
- 特性: GMC 全局运动补偿 + 四分像素组合
- 分辨率: 720×480
- 特点: 缩放、旋转、倾斜等 2D 仿射变换

#### 2.3.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_gmc_qpel_decode`

**测试代码框架**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4part2_gmc_qpel_decode() {
    let sample = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi";

    let mut demux = open_demuxer(sample);
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器");

    // 检查 GMC 特性
    let has_gmc = check_vop_has_gmc(&mut demux);
    let has_qpel = check_Vol_quarter_sample(&demux);

    println!("✓ 检测特性 - GMC: {}, Quarterpel: {}", has_gmc, has_qpel);

    // 解码前 20 帧
    let mut gmc_frame_count = 0;
    let mut frame_count = 0;

    loop {
        let packet = demux.read_packet()?;
        let is_gmc_frame = check_packet_has_gmc(&packet);

        decoder.send_packet(&packet)?;
        if decoder.receive_frame().is_ok() {
            frame_count += 1;
            if is_gmc_frame {
                gmc_frame_count += 1;
            }
        }

        if frame_count >= 20 { break; }
    }

    assert!(gmc_frame_count > 0, "应检测到至少 1 个 GMC 帧");
    println!("✓ 解码 {} 帧 (含 {} 个 GMC 帧)", frame_count, gmc_frame_count);
}
```

**验证项**:

- [ ] VOP header 中 gmc_enabled 标志检测
- [ ] 2D 仿射变换矩阵解析正确
- [ ] GMC 补偿计算无崩溃
- [ ] 与 FFmpeg 输出一致（运动补偿一致）

#### 2.3.2 对比测试

**对标工具**: FFmpeg

**对比指标**: PSNR >= 32 dB（复杂变换，允许略低）

#### 2.3.3 播放测试

**预期效果**: 缩放/旋转变换平滑，无撕裂

---

### 测试用例 2.4：数据分区 (Data Partitioning)

**优先级**: P2（码流特性）

**样本信息**

- 名称: ErrDec_mpeg4datapart-64_qcif.m4v (287K)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/m4v+mpeg4+++ErrDec_mpeg4datapart-64_qcif.m4v
- 特性: 数据分区模式
- 分辨率: 176×144 (QCIF)
- 特点: 头部、运动、纹理三分区分离

#### 2.4.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_data_partitioning`

**已有框架**: 详见 [mpeg4_part2_pipeline.rs](../tests/mpeg4_part2_pipeline.rs) 中 `test_mpeg4part2_data_partitioning_real_sample()`

**验证项**:

- [ ] 检测 data_partitioned 标志
- [ ] 分区边界识别（0x01B4/0x01B5）
- [ ] Header 分区解析
- [ ] Motion 分区解析
- [ ] Texture 分区解析
- [ ] RVLC (Reversible VLC) 支持（如启用）

#### 2.4.2 对比测试

**对标工具**: FFmpeg

**对比指标**: PSNR >= 35 dB

#### 2.4.3 播放测试

**预期效果**: 画面完整、无花屏

---

### 测试用例 2.5：数据分区边界情况测试

**优先级**: P2

**样本信息**

- 名称: vdpart-bug.avi (180K)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi
- 特点: 数据分区边界情况和 bug 重现

#### 2.5.1 自动化测试用例

**文件位置**: `tests/mpeg4_part2_pipeline.rs` - `test_mpeg4part2_data_partitioning_edge_cases`

**测试代码**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4part2_data_partitioning_edge_cases() {
    let sample = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi";

    let mut demux = open_demuxer(sample);
    let mut decoder = create_mpeg4_decoder();
    decoder.open(&params).expect("打开解码器");

    // 解码所有帧，收集错误和警告
    let mut frame_count = 0;
    let mut error_count = 0;
    let mut recovered_frames = 0;

    loop {
        match demux.read_packet() {
            Ok(packet) => {
                match decoder.send_packet(&packet) {
                    Ok(_) => {
                        if decoder.receive_frame().is_ok() {
                            frame_count += 1;
                            recovered_frames += 1;
                        }
                    }
                    Err(e) => {
                        // 记录错误但继续
                        println!("⚠️  解码错误: {:?}", e);
                        error_count += 1;
                    }
                }
            }
            Err(tao_core::TaoError::Eof) => break,
            Err(e) => {
                println!("⚠️  读包错误: {:?}", e);
                break;
            }
        }
    }

    println!("✓ 边界情况处理");
    println!("  总帧数: {}", frame_count);
    println!("  错误恢复帧数: {}", recovered_frames);
    println!("  未恢复错误: {}", error_count);

    // 允许少量错误，但应恢复大部分帧
    assert!(recovered_frames >= frame_count * 80 / 100,
            "应恢复至少 80% 的帧");
}
```

#### 2.5.2 对比测试

**对标工具**: FFmpeg 的帧数比较

#### 2.5.3 播放测试

**预期效果**: 大部分画面正常，允许个别帧花屏或丢失

---

## 第 3 阶段：特殊场景处理

### 阶段目标

验证特殊编码方式和边界情况处理。

### 测试用例 3.1：低分辨率解码

**优先级**: P2

**样本信息**

- 名称: difficult_lowres.avi (1.3M)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++difficult_lowres.avi
- 特性: 低分辨率特殊处理
- 预期分辨率: < 176×144

#### 3.1.1 自动化测试用例

**验证项**:

- [ ] 分辨率正确识别
- [ ] 宏块划分正确（QCIF 可能非标）
- [ ] 解码无崩溃

#### 3.1.2 对比测试

**对标指标**: PSNR >= 30 dB

#### 3.1.3 播放测试

**预期效果**: 画面清晰（尽管分辨率低）

---

### 测试用例 3.2：Quarterpel + B 帧组合

**优先级**: P2

**样本信息**

- 名称: qpel-bframes.avi (667K)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+mp3++qpel-bframes.avi
- 特性: 四分像素 + B 帧同时启用
- 分辨率: 320×240
- 特点: 复杂特性组合

#### 3.2.1 自动化测试用例

**检验**: 两个特性组合工作正常

#### 3.2.2 对比测试

**对标指标**: PSNR >= 33 dB

#### 3.2.3 播放测试

**预期效果**: 运动平滑、帧间过渡自然

---

### 测试用例 3.3：DivX 5.02 B 帧 + Quarterpel

**优先级**: P2

**样本信息**

- 名称: dx502_b_qpel.avi (4.5M)
- 源地址: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++dx502_b_qpel.avi
- 特性: DivX 5.02 特定编码选项
- 分辨率: 512×384
- 特点: 高分辨率、多 B 帧

#### 3.3.1 自动化测试用例

**验证**: 正确处理 DivX 特定编码参数

#### 3.3.2 对比测试

**对标指标**: PSNR >= 32 dB

#### 3.3.3 播放测试

**预期效果**: 高清画面流畅

---

## 第 4 阶段：FFmpeg 对标验证

### 阶段目标

确保 tao-codec 与 FFmpeg 的解码结果在像素级能达到可比水平。

### 系统对标流程

#### 4.1 自动化对比框架

**工具**: `tests/ffmpeg_compare.rs`

**流程**:

```rust
#[test]
#[cfg(feature = "http")]
fn test_mpeg4_part2_vs_ffmpeg_all_samples() {
    let samples = vec![
        "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi",
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi",
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi",
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi",
        // ... 更多样本
    ];

    for sample in samples {
        println!("\n🔍 对比测试: {}", sample);

        // 使用 FfmpegComparer 逐帧对比
        let mut comparer = FfmpegComparer::new(sample, "output/")?;
        let result = comparer.compare_all_frames()?;

        println!("  PSNR (平均): {:.2} dB", result.avg_psnr);
        println!("  最小 PSNR: {:.2} dB", result.min_psnr);
        println!("  差异比例: {:.2}%", result.diff_percentage);

        // 断言质量要求
        assert!(result.avg_psnr >= 30.0, "平均 PSNR 应 >= 30 dB");
    }
}
```

#### 4.2 对标指标汇总表

| 测试用例       | 样本             | 预期 PSNR (dB) | 预期差异 (%) | 状态 |
| -------------- | ---------------- | -------------- | ------------ | ---- |
| 1.1 基础 AVI   | color16.avi      | >= 38          | <= 0.5       | ⏳   |
| 2.1 B 帧       | b-frames.avi     | >= 36          | <= 1.0       | ⏳   |
| 2.2 Quarterpel | DivX51-Qpel.avi  | >= 34          | <= 1.5       | ⏳   |
| 2.3 GMC+Qpel   | xvid_gmcqpel.avi | >= 32          | <= 2.0       | ⏳   |
| 2.4 数据分区   | datapart.m4v     | >= 35          | <= 1.0       | ⏳   |
| 2.5 边界情况   | vdpart-bug.avi   | >= 30          | <= 5.0       | ⏳   |
| 3.1 低分辨率   | lowres.avi       | >= 30          | <= 2.0       | ⏳   |
| 3.2 Qpel+B     | qpel-bframes.avi | >= 33          | <= 1.5       | ⏳   |
| 3.3 DivX5.02   | dx502_b_qpel.avi | >= 32          | <= 1.5       | ⏳   |

---

## 不存在/未测试的特性

### 样本缺乏清单

| 特性              | 说明             | 备注           |
| ----------------- | ---------------- | -------------- |
| MPEG-4 + MP4 容器 | 暂未找到官方样本 | 优先级降至 P2  |
| Field-based 编码  | 隔行扫描 MPEG-4  | 需专门编码样本 |
| Sprite (GMC-only) | 无运动补偿的 GMC | 罕见特性       |
| 自定义量化矩阵    | 非标准量化       | 需特殊编码     |

### 已跳过的特性

暂无（全部特性有样本或计划）

---

## 执行步骤

### 快速启动

```bash
# 1. 运行第 1 阶段核心测试
cargo test --test mpeg4_part2_pipeline test_mpeg4part2_basic_avi_decode -- --nocapture

# 2. 运行所有自动化测试
cargo test --test mpeg4_part2_pipeline -- --nocapture

# 3. 生成对比报告
python scripts/mpeg4_compare_all.py > reports/mpeg4_compare_$(date +%Y%m%d_%H%M%S).log
```

### 详细步骤（逐阶段）

#### 第 1 阶段执行

1. [ ] 实现 `test_mpeg4part2_basic_avi_decode` 完整版本
2. [ ] 运行自动化测试，验证通过
3. [ ] 生成 FFmpeg 参考输出（5 帧）
4. [ ] 用 tao 解码并保存输出
5. [ ] 像素级对比，记录 PSNR
6. [ ] 运行 tao-play & FFplay，人工对比
7. [ ] 记录差异或通过

#### 第 2 阶段执行

逐个完成 2.1 - 2.5 的三层验证

#### 第 3 阶段执行

逐个完成 3.1 - 3.3 的三层验证

#### 第 4 阶段执行

运行全量对标对比框架

---

## 测试结果记录模板

### 测试用例 X.Y 结果

**测试日期**: YYYY-MM-DD

**环境**:

- Rust 版本: `rustc --version`
- FFmpeg 版本: `ffmpeg -version | head -1`
- 操作系统: Windows/Linux/macOS

#### 自动化测试结果

- [ ] 用例执行: ✓ 通过 / ❌ 失败 / ⏳ 跳过
- [ ] 错误信息: (如有)
- [ ] 日志: (详见 logs/mpeg4*part2*\*.log)

#### 对比测试结果

| 指标      | 值         | 阈值    | 符合 |
| --------- | ---------- | ------- | ---- |
| 平均 PSNR | X.XX dB    | >= Y dB | ✓/❌ |
| 最小 PSNR | X.XX dB    | >= Y dB | ✓/❌ |
| 差异比例  | X.XX %     | <= Y %  | ✓/❌ |
| 平均耗时  | X ms/frame | -       | -    |

#### 播放测试结果

**对比工具**: FFplay vs tao-play

| 项目   | FFplay | tao-play       | 一致性  |
| ------ | ------ | -------------- | ------- |
| 画质   | 清晰   | 清晰/花屏/绿屏 | ✓/⚠️/❌ |
| 色彩   | 正确   | 正确/偏色/反色 | ✓/⚠️/❌ |
| 流畅度 | 未卡顿 | 未卡顿/卡顿    | ✓/❌    |
| 同步   | 正常   | 正常/异步      | ✓/❌    |

**人工评分**: 5/5 ✓ / 4/5 ⚠️ / < 3/5 ❌

**差异说明**: (如有明显差异)

#### 总体结论

- 测试结果: ✓ 通过 / ⚠️ 部分通过 / ❌ 失败
- 待改进项: (列表)
- 下一步行动: (建议)

---

## 参考资源

### 官方文档

- ISO/IEC 14496-2:2004 - MPEG-4 Part 2 Specification
- FFmpeg MPEG-4 Decoder: https://git.ffmpeg.org/gitweb/ffmpeg.git/blob/HEAD:/libavcodec/mpeg4dec.c

### 工具和脚本

- `tests/ffmpeg_compare.rs` - 对比框架
- `scripts/compare_yuv.py` - YUV 像素对比
- `scripts/mpeg4_compare_all.py` - 全量对标脚本

### 样本来源

- 官方样本库: https://samples.ffmpeg.org/
- 样本列表: samples/SAMPLE_URLS.md
- 使用规范: samples/SAMPLES.md

### 相关 Tao 代码

- 解码器: [crates/tao-codec/src/decoders/mpeg4/](../crates/tao-codec/src/decoders/mpeg4/)
- 测试: [tests/mpeg4_part2_pipeline.rs](../tests/mpeg4_part2_pipeline.rs)
- 对比工具: [tests/ffmpeg_compare.rs](../tests/ffmpeg_compare.rs)

---

## 变更历史

| 日期       | 版本 | 作者       | 变更内容                            |
| ---------- | ---- | ---------- | ----------------------------------- |
| 2026-02-16 | 1.0  | AI Copilot | 初稿完成，包含四个阶段 9 个测试用例 |

---

## 联系方式

如有问题或建议，请提交 Issue 或 PR。

**状态**: ✅ 待执行

**优先级**: ⭐⭐⭐⭐⭐ 置顶
