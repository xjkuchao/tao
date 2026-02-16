# MPEG4 Part 2 解码器 PSNR 验证工作流程

> 本文档详细说明如何执行 PSNR 质量验证，包括自动化工具使用和手动测试步骤

**更新**: 2026-02-16  
**版本**: 1.0

---

## 📋 工作流程总览

```
准备环境
  ↓
生成 FFmpeg 基线
  ↓
运行 Tao 解码
  ↓
计算 PSNR 指标
  ↓
对比质量阈值
  ↓
生成验证报告
  ↓
分析和改进
```

---

## 第一步：环境准备

### 系统要求检查

```bash
# 检查 FFmpeg 是否已安装
ffmpeg -version

# 检查 Python 版本 (需要 3.6+)
python3 --version

# 检查 ffprobe (FFmpeg 的探测工具)
ffprobe -version
```

### 安装依赖

**Windows:**

```bash
# 使用 Chocolatey
choco install ffmpeg python

# 或手动下载
# https://ffmpeg.org/download.html
# https://www.python.org/downloads/
```

**macOS:**

```bash
# 使用 Homebrew
brew install ffmpeg python@3.11
```

**Linux (Ubuntu/Debian):**

```bash
sudo apt-get update
sudo apt-get install ffmpeg python3 python3-dev
```

### 创建工作目录

```bash
# 进入项目根目录
cd /path/to/tao

# 创建必要目录
mkdir -p data/ffmpeg_baselines data/mpeg4_decode_output plans scripts

# 查看目录结构
tree -L 2 data/
```

---

## 第二步：生成 FFmpeg 参考基线

### 方式 1: 使用自动化脚本 (推荐)

```bash
# 运行 Python 验证脚本
python3 scripts/verify_mpeg4_psnr.py

# 输出示例:
# [2026-02-16 14:30:15] INFO  MPEG4 Part 2 解码器 PSNR 验证工具
# [2026-02-16 14:30:16] INFO  检查环境...
# [2026-02-16 14:30:16] INFO  ✓ FFmpeg 可用
# ...
```

### 方式 2: 手动生成各样本基线

```bash
# 创建输出目录
mkdir -p data/ffmpeg_baselines

# 样本 1: 基础 AVI
ffmpeg -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  -pix_fmt yuv420p -f rawvideo \
  -vframes 10 \
  data/ffmpeg_baselines/ref_color16.yuv

# 样本 2: B 帧
ffmpeg -i https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi \
  -pix_fmt yuv420p -f rawvideo \
  -vframes 10 \
  data/ffmpeg_baselines/ref_b_frames.yuv

# 样本 3: Quarterpel
ffmpeg -i https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi \
  -pix_fmt yuv420p -f rawvideo \
  -vframes 10 \
  data/ffmpeg_baselines/ref_qpel.yuv

# 验证生成
ls -lh data/ffmpeg_baselines/
```

**预期输出:**

```
-rw-r--r-- 1 user group  1.8M Feb 16 14:32 ref_color16.yuv      # 312x240x10 帧
-rw-r--r-- 1 user group  5.2M Feb 16 14:35 ref_b_frames.yuv     # 720x480x10 帧
-rw-r--r-- 1 user group  1.2M Feb 16 14:38 ref_qpel.yuv         # 320x240x10 帧
```

### 验证基线文件

```bash
# 查看文件大小和统计
for file in data/ffmpeg_baselines/ref_*.yuv; do
  size=$(stat -f%z "$file" 2>/dev/null || stat -c%s "$file")
  frames=$((size / (312 * 240 * 3 / 2)))  # 以 color16 为例
  echo "$file: $(($size / 1024 / 1024)) MB, ~$frames 帧estimated"
done

# 验证 YUV420p 格式
ffprobe -f rawvideo -pix_fmt yuv420p -s:v 312x240 -show_format \
  data/ffmpeg_baselines/ref_color16.yuv
```

---

## 第三步：运行 Tao 解码

### 当前状态

⚠️ **需要实现**: tao-cli 或 tao-codec 库还需添加支持：

1. 从网络 URL 读取视频
2. 直接输出原始 YUV 数据

### 实现方案

#### 选项 1: 扩展 tao-cli 命令

```bash
# 建议的新命令行参数
tao-cli --input <file_or_url> \
        --output-raw <yuv_file> \
        --format yuv420p \
        --frames 10

# 使用示例
./target/release/tao-cli \
  --input https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  --output-raw data/mpeg4_decode_output/tao_color16.yuv \
  --frames 10
```

#### 选项 2: 使用 tao-codec 库编写测试程序

```rust
// 伪代码: tests/mpeg4_decode_to_yuv.rs
#[test]
fn decode_to_yuv_files() {
    let samples = vec![
        ("https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi", "data/mpeg4_decode_output/tao_color16.yuv"),
        // ... 其他样本
    ];

    for (url, output_file) in samples {
        let mut demuxer = DemuxerRegistry::open(url).unwrap();
        let mut file = File::create(output_file).unwrap();

        let mut frame_count = 0;
        while let Some(frame) = decoder.receive_frame() {
            match frame {
                Frame::Video(vf) => {
                    // 将 YUV 帧写入文件 (YUV420p 格式)
                    file.write_all(&vf.data_y).unwrap();
                    file.write_all(&vf.data_u).unwrap();
                    file.write_all(&vf.data_v).unwrap();
                    frame_count += 1;
                    if frame_count >= 10 { break; }
                }
                _ => {}
            }
        }
    }
}
```

#### 选项 3: 临时使用 FFmpeg 作为参考

```bash
# 使用 FFmpeg 作为 tao 的"完美解码器"进行验证
# 这样可以先验证 PSNR 计算框架是否正确工作

for sample in \
    "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi" \
    "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++qprd_cmp_b-frames_naq1.avi"
do
    filename=$(basename "$sample" | sed 's/\.[^.]*$//')

    # 生成"参考"输出 (实际上就是 FFmpeg)
    ffmpeg -i "$sample" -pix_fmt yuv420p -f rawvideo \
      -vframes 10 \
      "data/mpeg4_decode_output/tao_${filename}.yuv"
done
```

---

## 第四步：计算 PSNR 指标

### 使用 Python 脚本

```bash
# 运行完整验证 (包括所有步骤)
python3 scripts/verify_mpeg4_psnr.py

# 输出示例:
# [2026-02-16 14:40:00] INFO  验证: 1.1 基础 AVI 解码
# [2026-02-16 14:40:05] INFO  计算 PSNR: ref_color16.yuv vs tao_color16.yuv
# [2026-02-16 14:40:06] DEBUG  Frame  0: Y= 38.25 dB, U= 42.15 dB, V= 41.98 dB
# [2026-02-16 14:40:06] DEBUG  Frame  1: Y= 38.18 dB, U= 42.12 dB, V= 41.95 dB
# ...
# [2026-02-16 14:40:10] INFO  ✓ 通过! 平均 PSNR Y: 38.22 dB >= 38.0 dB
```

### 手动计算 (使用 FFmpeg)

```bash
# 方式 1: 使用 FFmpeg 的 PSNR filter
ffmpeg -i ref_color16.yuv -i tao_color16.yuv \
  -lavfi "[0:0][1:0]psnr=stats_file=psnr.log" \
  -f null -

# 查看结果
cat psnr.log
# 输出示例:
# n:0 mse_y=2.45 mse_u=1.82 mse_v=1.95 psnr_y=38.23 psnr_u=42.15 psnr_v=41.98
```

### 手动计算 (使用其他工具)

```bash
# 方式 2: 使用 ImageMagick (对比静止帧)
# 先将 YUV 转换为 PNG，再计算差异

for frame_num in 0 1 2 3 4; do
  offset=$((frame_num * 312 * 240 * 3 / 2))

  # 提取参考帧
  dd if=data/ffmpeg_baselines/ref_color16.yuv bs=1 skip=$offset count=$((312*240*3/2)) \
    of=/tmp/frame_ref.yuv 2>/dev/null

  # 提取 tao 帧
  dd if=data/mpeg4_decode_output/tao_color16.yuv bs=1 skip=$offset count=$((312*240*3/2)) \
    of=/tmp/frame_tao.yuv 2>/dev/null

  # 转换为图片并对比
  ffmpeg -f rawvideo -pix_fmt yuv420p -s 312x240 -i /tmp/frame_ref.yuv /tmp/frame_ref.png
  ffmpeg -f rawvideo -pix_fmt yuv420p -s 312x240 -i /tmp/frame_tao.yuv /tmp/frame_tao.png

  # 计算差异
  compare -metric RMSE /tmp/frame_ref.png /tmp/frame_tao.png null:
done
```

---

## 第五步：对比质量阈值

### 质量标准

| 指标          | 评级 | 说明               |
| ------------- | ---- | ------------------ |
| PSNR >= 40 dB | 极好 | 基本无可见差异     |
| PSNR 35-40 dB | 很好 | 非常小的可见差异   |
| PSNR 30-35 dB | 好   | 可接受的质量       |
| PSNR 25-30 dB | 一般 | 明显差异，但可接受 |
| PSNR < 25 dB  | 差   | 严重质量下降       |

### 测试用例阈值

| 测试           | 样本             | 预期 PSNR | 理由                   |
| -------------- | ---------------- | --------- | ---------------------- |
| 1.1 基础 AVI   | color16.avi      | >= 38 dB  | 标准编码，应近乎完美   |
| 2.1 B 帧       | b-frames.avi     | >= 32 dB  | 高级特性，允许更大容差 |
| 2.2 Quarterpel | DivX51-Qpel.avi  | >= 32 dB  | 高级特性，允许更大容差 |
| 2.3 GMC+QPel   | xvid_gmcqpel.avi | >= 32 dB  | 复杂特性，允许更大容差 |

### 判断标准

```python
# PSNR 对比逻辑 (伪代码)
def check_quality(psnr_y, psnr_u, psnr_v, threshold):
    if psnr_y >= threshold and psnr_u >= threshold - 2 and psnr_v >= threshold - 2:
        return "PASS"  # 通过
    else:
        return "FAIL"  # 失败
```

---

## 第六步：生成验证报告

### 自动报告生成

上述 Python 脚本会自动生成报告：

```bash
python3 scripts/verify_mpeg4_psnr.py

# 输出文件: plans/MPEG4_PSNR_VERIFICATION_REPORT.md
```

### 报告内容示例

```markdown
# MPEG4 Part 2 解码器 PSNR 验证报告

**验证时间**: 2026-02-16 14:45:30

## 摘要

| 指标     | 结果 |
| -------- | ---- |
| 总测试数 | 3    |
| 通过数   | 3    |
| 失败数   | 0    |
| 通过率   | 100% |

## 详细结果

### Test 1: 1.1 基础 AVI 解码

**状态**: PASSED

**质量阈值**: PSNR Y >= 38.0 dB

**平均 PSNR**:

- Y 平面: 38.22 dB
- U 平面: 42.14 dB
- V 平面: 41.96 dB

### Test 2: 2.1 B 帧解码

**状态**: PASSED

...
```

### 查看报告

```bash
# 查看生成的报告
cat plans/MPEG4_PSNR_VERIFICATION_REPORT.md

# 或在编辑器中打开
code plans/MPEG4_PSNR_VERIFICATION_REPORT.md
```

---

## 第七步：分析和改进

### 如果测试通过 ✅

```bash
# 太棒了！记录成功
git add plans/MPEG4_PSNR_VERIFICATION_REPORT.md
git commit -m "test: MPEG4 Part 2 PSNR 验证通过 (所有样本达标)"

# 进行人工播放验证
# 查看 PLAYBACK_VERIFICATION_GUIDE.md
```

### 如果 PSNR 低于阈值 ❌

#### 步骤 1: 定位问题

```bash
# 逐帧分析差异较大的帧
python3 scripts/analyze_frame_diff.py \
  --ref data/ffmpeg_baselines/ref_color16.yuv \
  --test data/mpeg4_decode_output/tao_color16.yuv \
  --width 312 --height 240 \
  --focus-frame 2  # 检查第 2 帧

# 生成对比图片
ffplay data/ffmpeg_baselines/ref_color16.yuv -f rawvideo -pixel_format yuv420p -s 312x240
# 快速预览参考帧

ffplay data/mpeg4_decode_output/tao_color16.yuv -f rawvideo -pixel_format yuv420p -s 312x240
# 快速预览 tao 输出
```

#### 步骤 2: 分析原因

```bash
# 检查是否涉及特定编码特性
ffprobe -show_frames \
  https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi | \
  head -50

# 查看帧类型、motion vectors 等信息
```

#### 步骤 3: 调试解码器

```bash
# 启用详细日志
RUST_LOG=debug cargo test --test mpeg4_part2_pipeline \
  --features http test_mpeg4part2_1_1_basic_avi -- --nocapture 2>&1 | \
  tee decode_debug.log

# 分析日志中的解码步骤
grep -i "decode\|error\|warn" decode_debug.log
```

#### 步骤 4: 与 FFmpeg 对标

```bash
# 查看 FFmpeg 的编码信息
ffmpeg -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi -t 1 \
  -vf "showinfo" -f null - 2>&1 | head -20

# 对比 tao 的解码逻辑与 FFmpeg 源代码
# 相关代码位置:
# - FFmpeg: libavcodec/mpeg4videodec.c
# - Tao: crates/tao-codec/src/decoders/mpeg4/
```

---

## 故障排除

### 问题 1: FFmpeg 无法从 HTTPS URL 读取

**症状:**

```
Protocol not whitelisted
```

**解决:**

```bash
# 使用 -protocol_whitelist 参数
ffmpeg -protocol_whitelist file,http,https,tcp,tls \
  -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  ...

# 或先下载到本地
wget https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi
ffmpeg -i color16.avi ...
```

### 问题 2: 网络连接慢导致超时

**症状:**

```
timeout
```

**解决:**

```bash
# 增加超时时间
ffmpeg -rtimeout 30000000 \
  -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  ...

# 或提前下载样本
# 参考: PLAYBACK_VERIFICATION_GUIDE.md 的下载步骤
```

### 问题 3: Python 脚本缺少依赖

**症状:**

```
ModuleNotFoundError: No module named 'xxx'
```

**解决:**

```bash
# Python 脚本仅使用标准库，无需额外依赖
# 但可以安装推荐包来增强功能
pip install pillow numpy  # 可选

# 或使用系统 Python
python3 -c "import sys; print(sys.version)"
```

---

## 快速命令参考

```bash
# 完整验证流程
python3 scripts/verify_mpeg4_psnr.py

# 仅生成基线
cd data/ffmpeg_baselines
ffmpeg -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  -pix_fmt yuv420p -f rawvideo -vframes 10 ref_color16.yuv

# 计算单个文件对的 PSNR
ffmpeg -i ref.yuv -i test.yuv \
  -lavfi "[0:0][1:0]psnr" -f null -

# 查看生成的报告
cat plans/MPEG4_PSNR_VERIFICATION_REPORT.md

# 进行人工播放验证
ffplay ref.yuv -f rawvideo -pixel_format yuv420p -s 312x240 -framerate 25
./target/release/tao-play https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi
```

---

## 相关文档

- 📋 [MPEG4 测试计划](./MPEG4_Part2_Decoder_Test_Plan.md)
- 📊 [测试执行报告](./MPEG4_Part2_Decoder_Test_Execution_Report.md)
- 🎯 [播放验证指南](./PLAYBACK_VERIFICATION_GUIDE.md)
- 📌 [样本 URL 清单](../samples/SAMPLE_URLS.md)

---

**维护者**: AI Copilot  
**最后更新**: 2026-02-16
