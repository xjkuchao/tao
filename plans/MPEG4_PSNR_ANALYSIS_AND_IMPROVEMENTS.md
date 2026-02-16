# MPEG4 Part 2 解码器 PSNR 验证分析与改进方案

**生成时间**: 2026-02-16 17:29:22
**验证工具**: scripts/verify_mpeg4_psnr.py  
**参考标准**: FFmpeg libavcodec  

---

## 一、验证结果总结

### 1.1 定量指标

| 测试项 | 平均 PSNR Y | 阈值 | 状态 | 问题等级 |
|--------|------------|------|------|---------|
| Test 1: 基础 AVI (color16.avi) | 3.73 dB | 38.0 dB | ❌ FAIL | 严重 |
| Test 2: B 帧 (b-frames.avi) | 1.99 dB | 32.0 dB | ❌ FAIL | 严重 |
| Test 3: Quarterpel (Qpel.avi) | 15.55 dB | 32.0 dB | ❌ FAIL | 中等 |

### 1.2 PSNR 质量评级

```
PSNR 分数评级标准:
- > 40 dB: 极好 (肉眼无差异)
- 35-40 dB: 很好 (高质量)
- 30-35 dB: 可接受 (轻微可见人工制品)
- 25-30 dB: 一般 (明显可见差异)
- 20-25 dB: 较差 (严重失真)
- < 20 dB: 很差 (严重损坏)

当前结果评级:
- Test 1: 3.73 dB → 【很差】严重损坏，几乎完全错误
- Test 2: 1.99 dB → 【极差】完全错误，只有噪音
- Test 3: 15.55 dB → 【很差】严重失真
```

---

## 二、问题根本原因分析

### 2.1 Test 1 和 Test 2: 极低 PSNR (< 4 dB)

**现象**: 
- Y 平面 PSNR 仅 1.99-3.73 dB (接近全噪音)
- 解码输出文件大小异常:
  - Test 1: 1.07 MB (应为 3.75 MB)
  - Test 2: 并生成了 126.1 MB 的解码输出 (异常超大)

**根本原因分析**:
1. **YUV 格式转换失败**: 解码器输出可能不是标准 YUV420p 格式
2. **内存不足或越界**: 解码循环可能在处理大量数据时出错
3. **时间戳问题**: 可能只解码了部分帧而非完整视频
4. **像素值范围错误**: 可能输出值不在 [0, 255] 范围内

**证据**:
- Test 2 输出 126.1 MB (超出预期 4.9 MB 的 25 倍)
- Test 1 输出 1.07 MB (仅预期的 28%)
- PSNR 值接近最小值 (0 dB 是完全错误的理论下界)

**建议检查**:
1. ✅ `ensure_yuv420p()` 函数是否正确处理像素格式
2. ✅ `write_yuv420p_frame()` 是否正确处理所有平面数据
3. ✅ YUV420p 平面大小计算是否正确
4. ✅ 解码循环中的帧数限制是否生效 (当前没有 -t 参数)

### 2.2 Test 3: 中等-低 PSNR (15.55 dB)

**现象**:
- Y 平面 PSNR 15.55 dB (严重失真但非完全错误)
- 某些帧质量相对较好 (Frame 6: 30.40 dB, Frame 3: 19.87 dB)
- 文件大小正常 (1.10 MB ≈ 预期 3.74 MB / 3.4)

**根本原因分析**:
1. **Quarterpel 精度不足**: Quarterpel 运动补偿精度可能不足
2. **DCT 逆变换误差累积**: 长 GOP 中误差可能累积
3. **分部编码处理问题**: 如果视频有分部编码,可能处理错误
4. **量化参数处理**: 可能在量化反演步骤出错

**特征**:
- 帧间方差较大 (最低 10.71 dB, 最高 30.40 dB)
- 暗示某些编码模式处理正确,某些不正确
- 可能与视频内容 (I/P/B 帧分布) 相关

---

## 三、改进优先级方案

### Phase 1: 紧急 (今日)

#### 1.1 修复 YUV 输出问题【优先级: P0】

```rust
// bin/tao-cli/src/main.rs 中的改进:

// 1. 确保格式转换到 YUV420p
fn ensure_yuv420p(frame: &VideoFrame) -> Result<VideoFrame, TaoError> {
    if frame.pixel_format == PixelFormat::Yuv420p {
        Ok(frame.clone())
    } else {
        // 使用 tao_scale 进行格式转换
        let ctx = tao_scale::ScaleContext::new(
            frame.width,
            frame.height,
            frame.pixel_format,
            frame.width,
            frame.height,
            PixelFormat::Yuv420p,  // 目标格式
            tao_scale::ScaleAlgorithm::Bilinear,
        )?;
        
        // 执行转换...
        // 返回 YUV420p 帧
    }
}

// 2. 验证输出文件大小
expected_size = width * height * 1.5 * 10  // YUV420p * 10 frames
actual_size = file.len()
if abs(actual_size - expected_size) > 0.1 * expected_size {
    log_warning("文件大小异常");
}

// 3. 添加帧数限制
if frame_count >= 10 {
    break;  // 仅输出前 10 帧
}
```

**预期改进**:
- Test 1: 3.73 dB → 35-40 dB
- Test 2: 1.99 dB → 30-35 dB

#### 1.2 添加调试输出【优先级: P0】

```python
# scripts/verify_mpeg4_psnr.py 中添加:

def generate_tao_decode(test_case):
    # ... 执行解码后:
    
    # 验证输出文件
    exp_size = test_case['width'] * test_case['height'] * 1.5 * test_case['frames']
    actual_size = os.path.getsize(output_file)
    
    log(f"  期望大小: {exp_size / 1e6:.1f} MB")
    log(f"  实际大小: {actual_size / 1e6:.1f} MB")
    log(f"  比例: {actual_size / exp_size:.2f}x")
    
    if actual_size < exp_size * 0.5:
        log("  ⚠️ 警告: 输出文件过小,可能只解码了部分帧", "WARN")
    elif actual_size > exp_size * 2.0:
        log("  ⚠️ 警告: 输出文件过大,可能包含重复数据", "WARN")
```

### Phase 2: 主要改进 (明日)

#### 2.1 格式转换实现【优先级: P1】

在 `bin/tao-cli/src/main.rs` 中:
- 实现真正的 YUV420p 转换 (而非跳过转换)
- 使用 `tao_scale` 进行像素格式转换
- 添加详细日志记录转换步骤

#### 2.2 性能优化【优先级: P2】

- 减少内存分配 (复用缓冲区)
- 批量处理帧 (避免频繁切换上下文)
- 添加进度指示器

### Phase 3: 深度优化 (周内)

#### 3.1 Quarterpel 精度改进【优先级: P1】

在 `crates/tao-codec/src/decoders/mpeg4/` 中:
- 检查 Quarterpel 运动补偿的精度 (确保使用 8x8 精度)
- 验证滤波系数是否正确
- 对比 FFmpeg 的实现

#### 3.2 编码特性支持【优先级: P1】

检查以下特性的支持情况:
- [ ] Data Partitioning
- [ ] Resync Markers  
- [ ] RVLC (Reversible VLC)
- [ ] Short Header Mode

#### 3.3 单元测试强化【优先级: P2】

逐个编码特性进行单独测试:
```rust
#[test]
fn test_mpeg4_quarterpel_precision() {
    // 验证 Quarterpel 精度±0.5 像素
}

#[test]
fn test_mpeg4_dct_inverse_transform() {
    // 验证 DCT 逆变换精度
}

#[test]
fn test_mpeg4_motion_compensation() {
    // 验证运动补偿精度
}
```

---

## 四、快速诊断工具

### 4.1 检查解码是否真实执行

```bash
# 方法 1: 检查输出文件大小
ls -lh data/mpeg4_decode_output/tao_*.yuv

# 预期:
# - color16.avi: 1.1 MB (312*240*1.5*10 = 1.1 MB) ✓
# - b-frames: 70+ MB (720*480*1.5*260 frames approx) - 需检查帧数
# - Qpel: 1.1 MB (320*240*1.5*10 = 1.1 MB) ✓
```

### 4.2 检查 FFmpeg 基线是否正确

```bash
# 验证 FFmpeg 输出
ffprobe data/ffmpeg_baselines/ref_color16.yuv -v error \
  -select_streams v:0 -show_entries stream=width,height,duration

# 对比 tao-codec 输出的前 100 字节
xxd -l 100 data/ffmpeg_baselines/ref_color16.yuv
xxd -l 100 data/mpeg4_decode_output/tao_color16.yuv
```

### 4.3 使用 Python 进行像素级调试

```python
import struct
import numpy as np

def analyze_yuv420(filename, width, height, frame_idx=0):
    """分析单个 YUV420p 帧的像素统计"""
    frame_size = width * height * 1.5
    with open(filename, 'rb') as f:
        f.seek(frame_idx * frame_size)
        y_plane = f.read(width * height)
        u_plane = f.read(width * height // 4)
        v_plane = f.read(width * height // 4)
    
    y_values = struct.unpack('B' * len(y_plane), y_plane)
    print(f"Y 平面统计:")
    print(f"  最小值: {min(y_values)}, 最大值: {max(y_values)}")
    print(f"  平均值: {np.mean(y_values):.0f}")
    print(f"  标准差: {np.std(y_values):.0f}")
    
    # 检查是否为常数 (全固定值表示完全错误)
    unique_values = len(set(y_values))
    print(f"  唯一像素值数: {unique_values} / {len(y_values)}")
    
    if unique_values < 10:
        print("  ⚠️ 错误: 像素值过于单调(可能是全黑或全白)")

# 使用
analyze_yuv420('data/ffmpeg_baselines/ref_color16.yuv', 312, 240, 0)
analyze_yuv420('data/mpeg4_decode_output/tao_color16.yuv', 312, 240, 0)
```

---

## 五、建议行动方案

### 今日 (P0 任务)

1. **修复 `ensure_yuv420p()` 函数**
   - 不要跳过格式转换
   - 实现真正的 YUV420p 转换
   - 文件: `bins/tao-cli/src/main.rs:658`

2. **添加调试验证**
   - 输出文件大小检查
   - 帧数限制在前 10 帧
   - 文件: `scripts/verify_mpeg4_psnr.py`

3. **重新运行验证**
   - `python3 scripts/verify_mpeg4_psnr.py`
   - 期望 Test 1/2 PSNR 提升至 30+ dB
   - 期望 Test 3 PSNR 提升至 25+ dB

### 明日 (P1 任务)

4. **逐个特性调试**
   - 启用详细日志: `RUST_LOG=debug`
   - 对比 FFmpeg 的输出
   - 使用像素级工具诊断

5. **性能优化**
   - 减少内存分配
   - 添加进度指示

### 周内 (P2 任务)

6. **完整性检查**
   - 支持所有 MPEG4 Part 2 特性
   - 编写特性专项测试
   - 性能基准测试

---

## 六、参考资源

### MPEG4 Part 2 标准
- ISO/IEC 14496-2:2004 (最新版本)
- 重点章节: 
  - Section 6.2: Visual Object Coding (VOC)
  - Section 6.3: Motion Compensation Tools
  - Section 6.4: Texture Coding Tools

### FFmpeg MPEG4 解码器
- 源代码: `libavcodec/mpeg4dec.c`
- 关键函数:
  - `mpeg4_decode_block()`: 数据块解码
  - `h263_pred_motion()`: 运动向量预测
  - `mp_decode_mv()`: 运动向量解码

### 测试工具
- FFmpeg: `ffmpeg -i input.avi -f rawvideo -pix_fmt yuv420p output.yuv`
- ImageMagick: `convert -size 312x240 -depth 8 -colorspace YCbCr yuv:ref_frame.yuv - | display`
- Python: `numpy`, `scipy`

---

## 附录: PSNR 计算公式

```
MSE = (1/N) * Σ(y_ref - y_tao)²
PSNR = 20 * log10(MAX_VALUE / sqrt(MSE))  [dB]

其中:
- N = 像素总数
- MAX_VALUE = 255 (8-bit)
- log10 = 以 10 为底的对数

规律:
- PSNR 每增加 6 dB → MSE 减少到 1/4
- 相差 0.5 dB → 很难用肉眼察觉
- 相差 3 dB → 稍微可见区别
- 相差 6+ dB → 明显可见区别
```

---

**文档维护者**: tao 项目开发团队  
**最后更新**: 2026-02-16  
**状态**: 进行中
