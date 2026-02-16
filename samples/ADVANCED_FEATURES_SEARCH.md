# MPEG-4 Part 2 高级特性测试样本搜索报告

> 日期: 2026-02-16
> 搜索范围: https://samples.ffmpeg.org/
> 目标: 完善 MPEG-4 Part 2 解码器的高级特性（GMC、Data Partitioning、RVLC、交错扫描）

---

## 🎯 搜索目标

根据当前测试清单与实现目标，我们需要找到以下特性的测试样本：

1. ✅ **GMC (Global Motion Compensation)** - 2/3 点精灵轨迹变换
2. ✅ **Data Partitioning** - 视频分区模式
3. ❌ **RVLC (Reversible VLC)** - 可逆变长编码
4. ❌ **Interlaced (交错扫描)** - 场预测 + 场 DCT

---

## ✅ 成功找到的样本

### 1. GMC + Quarterpel 组合测试

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi
- **大小**: 2.8 MB
- **描述**: Xvid 编码，包含 GMC 和 Quarterpel 运动补偿
- **优先级**: ⭐⭐⭐ **最高**
- **用途**:
    - 测试 GMC 1/2/3 点精灵轨迹变换
    - 验证 Quarterpel 运动补偿精度
    - 检测 GMC artifact 边界情况
- **测试文件**: `tests/mpeg4_advanced_features.rs::test_gmc_quarterpel_xvid()`

### 2. Data Partitioning 主样本

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/m4v+mpeg4+++ErrDec_mpeg4datapart-64_qcif.m4v
- **大小**: 287 KB
- **格式**: M4V (MPEG-4 Elementary Stream)
- **描述**: 专用 Data Partitioning 测试样本，含错误恢复测试
- **优先级**: ⭐⭐⭐ **最高**
- **用途**:
    - 验证 Data Partitioning 分区标记解析
    - 测试分区模式下的错误处理
    - 对比 FFmpeg 的分区解码逻辑
- **测试文件**: `tests/mpeg4_advanced_features.rs::test_data_partitioning()`

### 3. Data Partitioning Bug 样本

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi
- **大小**: 180 KB
- **描述**: Data Partitioning 边界情况 bug 样本
- **优先级**: ⭐⭐ 高
- **用途**:
    - 测试异常分区数据的处理
    - 验证解码器稳健性
    - 确保不会 panic 或崩溃
- **测试文件**: `tests/mpeg4_advanced_features.rs::test_data_partitioning_bug()`

### 4. Quarterpel 系列样本

#### DivX 5.01 Quarterpel

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi
- **大小**: 4.2 MB
- **描述**: DivX 5.01 编码，标准 Quarterpel 测试
- **测试文件**: `tests/mpeg4_advanced_features.rs::test_quarterpel_divx501()`

#### DivX 5.02 B 帧 + Quarterpel

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++dx502_b_qpel.avi
- **大小**: 4.5 MB
- **描述**: DivX 5.02 B 帧 + Quarterpel 组合
- **优先级**: ⭐⭐⭐ 最高（测试组合特性）
- **用途**:
    - 验证 B 帧 + Quarterpel 路径
    - 测试 DPB (Decoded Picture Buffer) + QPel
    - 确保帧重排序正确
- **测试文件**: `tests/mpeg4_advanced_features.rs::test_quarterpel_bframes()`

#### 通用 Quarterpel + B 帧

- **URL**: https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+mp3++qpel-bframes.avi
- **大小**: 667 KB
- **描述**: 轻量级 QPel + B 帧测试样本

---

## ❌ 未找到的样本

### 1. RVLC (Reversible Variable Length Codes)

**搜索结果**: 整个 FFmpeg 样本库未发现包含 RVLC 的 MPEG-4 样本

**原因分析**:

- RVLC 是 MPEG-4 ASP 的可选特性，主要用于错误恢复
- 实际编码器（Xvid/DivX/FFmpeg）很少启用此特性
- 标准测试向量可能存在于 MPEG 官方参考软件中

**解决方案**:

1. 查找 MPEG-4 官方测试向量 (ISO conformance streams)
2. 使用 MPEG-4 参考软件自行生成 RVLC 样本
3. **临时搁置**: 保留 RVLC 解析框架，待找到样本后完善

**当前状态**:

- ✅ data_partitioned 模式已接入 RVLC AC 解码路径
- ⚠️ RVLC 后向解码（错误恢复）框架存在但未测试

### 2. Interlaced (交错扫描场预测)

**搜索结果**: MPEG-4 目录下未发现明确的 Interlaced 样本

**可能原因**:

- MPEG-4 ASP 交错支持不如 MPEG-2 广泛
- 大部分 MPEG-4 视频为逐行扫描
- 交错特性更常见于广播格式（MPEG-2/H.264）

**替代方案**:

1. MPEG-2 样本库有大量 interlaced 样本，可参考字段解析逻辑
2. 检查某些 DivX/Xvid 编码是否包含 `interlaced` 标志
3. 使用 FFmpeg 编码生成测试样本

**当前状态**:

- ✅ 交错标志解析已实现
- ⚠️ 场预测 (top_field_first/alternate_vertical_scan) 待完善
- ⚠️ 场 DCT (field_dct) 待测试

---

## 📊 样本优先级汇总

| 优先级 | 样本                             | 特性                  | URL 后缀                           | 状态 |
| ------ | -------------------------------- | --------------------- | ---------------------------------- | ---- |
| ⭐⭐⭐ | xvid_gmcqpel_artifact.avi        | GMC + Quarterpel      | `xvid_gmcqpel_artifact.avi`        | ✅   |
| ⭐⭐⭐ | ErrDec_mpeg4datapart-64_qcif.m4v | Data Partitioning     | `ErrDec_mpeg4datapart-64_qcif.m4v` | ✅   |
| ⭐⭐⭐ | dx502_b_qpel.avi                 | B 帧 + Quarterpel     | `dx502_b_qpel.avi`                 | ✅   |
| ⭐⭐⭐ | DivX51-Qpel.avi                  | Quarterpel (标准)     | `DivX51-Qpel.avi`                  | ✅   |
| ⭐⭐   | vdpart-bug.avi                   | Data Partition Bug    | `vdpart-bug.avi`                   | ✅   |
| ⭐⭐   | qpel-bframes.avi                 | QPel + B 帧（轻量级） | `qpel-bframes.avi`                 | ✅   |
| ❌     | RVLC 样本                        | 可逆 VLC              | N/A                                | ❌   |
| ❌     | Interlaced 样本                  | 交错场预测            | N/A                                | ❌   |

---

## 🚀 下一步工作

### 1. 立即执行（高优先级）

#### ✅ 已完成

- [x] 更新 `samples/SAMPLE_URLS.md` 添加高级特性样本链接
- [x] 创建 `tests/mpeg4_advanced_features.rs` 测试文件
- [x] 更新 `samples/ADVANCED_FEATURES_SEARCH.md` 添加搜索结果与进展

#### ⏳ 待执行

- [ ] 运行测试验证样本可用性: `cargo test --test mpeg4_advanced_features -- --include-ignored`
- [ ] 修复测试中发现的解码问题
- [ ] 对比 FFmpeg 输出，计算 PSNR/SSIM

### 2. GMC 完善（阶段 2-A）

- [ ] 下载 `xvid_gmcqpel_artifact.avi` 的描述文件查看详细信息
- [ ] 实现 2/3 点 GMC 精灵轨迹变换
- [ ] 对比 FFmpeg `libavcodec/mpeg4videodec.c` 的 `gmc()` 函数
- [ ] 替换当前的 simplified warping 为标准 affine/perspective 变换
- [ ] 验证与 FFmpeg 像素级一致

### 3. Data Partitioning 完善（阶段 2-B）

- [ ] 实现完整的 partition 标记解析（motion/texture/dc）
- [ ] 测试 `ErrDec_mpeg4datapart-64_qcif.m4v` 的错误恢复路径
- [ ] 对比 FFmpeg 的 `decode_vol_header()` 中 data_partitioning 处理
- [ ] 测试 `vdpart-bug.avi` 确保稳健性

### 4. Quarterpel 精度验证（阶段 2-C）

- [ ] 使用 `DivX51-Qpel.avi` 验证基础 QPel 精度
- [ ] 使用 `dx502_b_qpel.avi` 验证 B 帧 + QPel 组合
- [ ] 检查 rounding 表是否与 FFmpeg 一致
- [ ] PSNR 应 > 40 dB (与 FFmpeg 对比)

### 5. RVLC 和交错扫描（低优先级）

#### RVLC

- [ ] 搜索 MPEG-4 官方测试向量（ISO/IEC conformance streams）
- [ ] 联系 FFmpeg 社区询问 RVLC 样本来源
- [ ] 考虑使用参考软件生成测试样本
- [ ] 完成后实现后向解码路径与错误恢复同步

#### 交错扫描

- [ ] 尝试用 FFmpeg 生成交错 MPEG-4 样本: `ffmpeg -i input.mp4 -flags +ildct -c:v mpeg4 output.m4v`
- [ ] 实现 `top_field_first` / `alternate_vertical_scan` 处理
- [ ] 实现 `field_dct` 场 DCT 变换
- [ ] 参考 MPEG-2 的交错处理逻辑

---

## 📝 测试用例开发规范

### 使用样本 URL 的标准模板

```rust
#[test]
#[ignore] // 需要网络访问
fn test_feature_name() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/<sample>.avi";

    let mut demuxer = DemuxerRegistry::open(url)
        .expect("无法打开样本");

    let video_stream_index = demuxer.streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 20; // 只测试前 20 帧

    while let Some(packet) = demuxer.read_packet().expect("读取失败") {
        if packet.stream_index != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet).expect("发送失败");

        while let Some(frame) = decoder.receive_frame().expect("接收失败") {
            frame_count += 1;
            println!("解码帧 #{}, 分辨率: {}x{}",
                     frame_count, frame.width, frame.height);

            if frame_count >= MAX_FRAMES {
                break;
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

    assert!(frame_count >= 10, "至少应解码 10 帧");
    println!("✅ 测试通过，解码 {} 帧", frame_count);
}
```

### 运行测试

```bash
# 运行所有高级特性测试（需要网络）
cargo test --test mpeg4_advanced_features -- --include-ignored

# 单独运行 GMC 测试
cargo test --test mpeg4_advanced_features test_gmc_quarterpel_xvid -- --include-ignored

# 单独运行 Data Partitioning 测试
cargo test --test mpeg4_advanced_features test_data_partitioning -- --include-ignored
```

---

## 🔗 相关资源

### 文档链接

- **样本清单**: [samples/SAMPLE_URLS.md](../samples/SAMPLE_URLS.md)
- **完善计划**: [plans/mpeg4_part2_decoder_perfection.md](../plans/mpeg4_part2_decoder_perfection.md)
- **样本说明**: [samples/SAMPLES.md](../samples/SAMPLES.md)

### FFmpeg 参考代码

- **MPEG-4 解码器**: `libavcodec/mpeg4videodec.c`
- **GMC 实现**: `libavcodec/mpeg4videodec.c::gmc()`
- **Data Partitioning**: `libavcodec/mpeg4videodec.c::decode_vol_header()`

### 样本来源

- **主库**: https://samples.ffmpeg.org/
- **MPEG-4 目录**: https://samples.ffmpeg.org/archive/video/mpeg4/
- **样本列表**: https://samples.ffmpeg.org/allsamples.txt

---

## ✅ 总结

### 成功找到 6 个高级特性样本

- ✅ **3 个 Quarterpel 样本** (DivX 5.01/5.02, B 帧组合)
- ✅ **2 个 Data Partitioning 样本** (标准测试 + Bug 样本)
- ✅ **1 个 GMC + Quarterpel 组合样本** (Xvid)

### 待解决的 2 个特性

- ❌ **RVLC**: 需要从 MPEG 官方或参考软件获取
- ❌ **Interlaced**: 需要自行生成或从 DivX 样本中寻找

### 后续工作优先级

1. **立即**: 运行测试验证样本可用性
2. **高优**: GMC 2/3 点实现，Data Partitioning 完善
3. **中优**: Quarterpel 精度对比，B 帧 + QPel 组合测试
4. **低优**: RVLC 样本收集，Interlaced 样本生成

---

**报告完成日期**: 2026-02-16  
**搜索耗时**: 约 10 分钟  
**样本总大小**: ~15 MB  
**测试文件**: `tests/mpeg4_advanced_features.rs`
