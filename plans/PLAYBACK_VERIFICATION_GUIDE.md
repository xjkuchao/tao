# MPEG4 Part 2 解码器播放验证指南

> 本指南用于手动播放验证 tao-play 与 ffplay 的画质对比

## 概述

本文档提供了详细的播放验证流程，用于通过人工对比来评估 tao-codec 的 MPEG4 Part 2 解码质量。

### 验证流程

```
准备工作 → 编译工具 → 测试样本选择 → 并行播放 → 对比评分 → 记录结果
```

---

## 第一步：准备工作

### 系统要求

- **Windows/macOS/Linux** 系统
- **FFmpeg/ffplay**: 官方参考播放器
  ```bash
  # 检查 ffplay 是否安装
  ffplay -version
  ```
- **tao-play**: Tao 多媒体播放器（本项目）

### 安装 FFmpeg

**Windows:**
```bash
# 使用 Chocolatey
choco install ffmpeg

# 或手动下载
# https://ffmpeg.org/download.html
```

**macOS:**
```bash
brew install ffmpeg
```

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install ffmpeg
```

### 验证安装

```bash
# 验证 FFmpeg
ffplay -version | head -1
# 输出: ffplay version X.X ...

# 验证 tao-play
cargo build -p tao-play
# 成功输出: Finished release [optimized] ...
```

---

## 第二步：编译 tao-play

```bash
# 调试模式（快速编译）
cargo build -p tao-play

# 发布模式（最优性能）
cargo build -p tao-play --release
```

输出位置：
- 调试: `target/debug/tao-play` (Windows: `.exe`)
- 发布: `target/release/tao-play` (Windows: `.exe`)

---

## 第三步：选择测试样本

### 推荐测试样本列表

| 优先级 | 样本 | URL | 大小 | 特性 |
|------|------|-----|-----|------|
| ⭐⭐⭐ | color16.avi | https://samples.ffmpeg.org/V-codecs/MPEG4... | 中等 | 基础 |
| ⭐⭐⭐ | B-frame | https://samples.ffmpeg.org/archive/video/mpeg4/... | 中等 | B帧 |
| ⭐⭐ | Quarterpel | https://samples.ffmpeg.org/archive/video/mpeg4/DivX51-Qpel.avi | 中等 | QPel |
| ⭐ | GMC+QPel | https://samples.ffmpeg.org/archive/video/mpeg4/xvid_gmcqpel... | 大 | 高级 |

### 下载样本 (可选)

> 推荐：直接使用 URL，不下载本地

如需下载：
```bash
# 创建样本目录
mkdir -p data/samples

# 下载样本
wget -o data/samples/color16.avi https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# 验证完整性
ffprobe data/samples/color16.avi
```

---

## 第四步：并行播放对比

### 方式 1: 双终端对比 (推荐)

同时打开两个终端窗口，分别运行 tao-play 和 ffplay。

**终端 1 - 运行 ffplay（参考）:**
```bash
ffplay https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# 或本地文件
ffplay data/samples/color16.avi
```

**终端 2 - 运行 tao-play:**
```bash
# 调试模式
./target/debug/tao-play https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# 发布模式
./target/release/tao-play https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi

# 本地文件
./target/release/tao-play data/samples/color16.avi
```

**快捷键（两个播放器通用）:**
- `Space`: 暂停/继续
- `Q / Esc`: 退出
- `F`: 全屏
- `→`: 快进 5 秒
- `←`: 快退 5 秒

### 方式 2: 屏幕分割对比

在支持窗口分割的系统上，将两个窗口并排放置：

**Windows 10+:**
- ffplay 窗口: 按 Win+Left 靠左
- tao-play 窗口: 按 Win+Right 靠右

**macOS:**
- Mission Control 快速调整窗口位置

**Linux (X11):**
```bash
# 使用窗口管理器的平铺功能
# 或手动调整窗口大小和位置
```

### 方式 3: 录屏对比

如果需要详细分析，可以录制播放过程：

```bash
# 录制 ffplay 输出 (使用 FFmpeg)
ffmpeg -video_size 1920x1080 -framerate 30 -f x11grab -i :0.0 ffplay_out.mp4

# 或使用系统录屏
# Windows: Win+G (Game Bar)
# macOS: Cmd+Shift+5
# Linux: gnome-screenshot / kazam
```

---

## 第五步：对比评分

### 评分标准

为每个样本进行以下检查，使用 ✅/⚠️/❌ 标记：

#### 画面质量

| 项目 | ffplay | tao-play | 备注 |
|------|--------|----------|------|
| 清晰度 | | | 是否模糊或有块状物 |
| 色彩还原 | | | 肤色/背景色是否准确 |
| 无绿屏 | | | 完全绿屏 = ❌ |
| 无花纹 | | | 马赛克花纹 = ❌ |
| 边界清晰 | | | 边缘是否有人为痕迹 |

**质量评级:**
- ✅ 优: 与 ffplay 几乎无差异
- ⚠️ 良: 有轻微差异，但可接受
- ❌ 差: 严重差异或播放失败

#### 播放流畅度

| 项目 | ffplay | tao-play | 备注 |
|------|--------|----------|------|
| 帧率稳定 | | | 是否卡顿、掉帧 |
| 同步准确 | | | 音视频是否同步 |
| 进度条准确 | | | 进度显示是否正确 |

#### 编码特性检测

对于包含特殊编码特性的样本，观察是否正确处理：

**B 帧样本:**
- 画面是否有往返跳动
- 是否有「鬼影」（参考帧错误）

**Quarterpel 样本:**
- 运动平滑度 vs FFmpeg
- 是否有块状或锯齿

**GMC 样本:**
- 缩放/旋转变换是否平滑
- 是否有扭曲或撕裂

### 评分表格模板

```markdown
## 样本: color16.avi

**基本信息:**
- 分辨率: 312×240
- 帧率: 25 fps
- 编码特性: 基础 I/P 帧
- 容器: AVI

**画面质量评分:**
| 项目 | ffplay | tao-play | 评价 |
|------|--------|----------|------|
| 清晰度 | ✅ | ✅ | 完全匹配 |
| 色彩 | ✅ | ✅ | 完全匹配 |
| 绿屏 | ✅ | ✅ | 无绿屏 |
| 花纹 | ✅ | ✅ | 无花纹 |
| 边界 | ✅ | ✅ | 清晰 |

**流畅度评分:**
| 项目 | ffplay | tao-play | 评价 |
|------|--------|----------|------|
| 帧率稳定 | ✅ | ✅ | 都很流畅 |
| 同步准确 | - | - | 无音频 |

**总体评分:** ✅ 优

**备注:** tao-play 画质与 ffplay 完全一致，流畅度相当。
```

---

## 第六步：记录结果

### 创建验证报告

在 `plans/` 目录创建验证报告文件：

```bash
# 文件名格式: MPEG4_PLAYBACK_VERIFICATION_REPORT.md
```

模板：
```markdown
# MPEG4 Part 2 解码器播放验证报告

**日期**: YYYY-MM-DD  
**验证者**: [你的名字]  
**环境**: Windows/macOS/Linux

## 环境信息

- FFmpeg 版本: `ffplay -version`
- tao-play 编译: [调试/发布]
- 系统: [详细信息]

## 测试结果

### Test 1.1: 基础 AVI (color16.avi)

**画面质量**: ✅ 优
**流畅度**: ✅ 优  
**总体**: ✅ 合格

备注: ...

### Test 2.1: B 帧 (b-frames.avi)

**画面质量**: ⚠️ 良
**流畅度**: ✅ 优  
**总体**: ⚠️ 待改进

备注: ...

## 汇总

- ✅ 通过: 7 项
- ⚠️ 部分通过: 2 项
- ❌ 失败: 0 项

**总体评价**: 大多数样本播放正常，X 项需要改进。

## 提交

```bash
git add plans/MPEG4_PLAYBACK_VERIFICATION_REPORT.md
git commit -m "docs: MPEG4 Part 2 播放验证报告 - 完成"
```
```

---

## 故障排除

### 问题 1: ffplay 无法播放网络 URL

**症状:** `Protocol not whitelisted by whitelist`

**解决:**
```bash
# 使用 -protocol_whitelist 允许 https
ffplay -protocol_whitelist file,http,https,tcp,tls -i https://...

# 或先下载到本地再播放
wget https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi
ffplay color16.avi
```

### 问题 2: tao-play 播放时卡顿

**可能原因:**
- 网络连接慢（从 URL 流下载）
- 解码性能不足
- GPU 不支持

**解决:**
```bash
# 使用本地文件测试
./target/release/tao-play data/samples/color16.avi

# 检查 CPU 使用率
# Windows: 任务管理器 → 处理器选项卡
# macOS: 活动监视器
# Linux: top / htop
```

### 问题 3: 绿屏或无输出

**可能原因:**
- 解码失败
- 输出格式不支持

**调试:**
```bash
# 启用详细日志
RUST_LOG=debug ./target/debug/tao-play data/samples/color16.avi 2>&1 | tee playback.log

# 查看错误信息
grep -i error playback.log
```

### 问题 4: 音视频不同步

**可能原因:**
- 音频/视频解码速率不同
- 时间戳处理有误

**验证:**
```bash
# 使用 ffprobe 检查时间戳
ffprobe -show_frames data/samples/color16.avi | grep pkt_pts | head -20
```

---

## 最佳实践

### ✅ 推荐做法

1. **从基础样本开始** - 先测试简单的 I/P 帧样本
2. **逐步增加复杂度** - 然后测试 B 帧、QPel 等
3. **记录观察** - 详细记录每个差异点
4. **重复验证** - 关键样本多次播放验证
5. **对比对齐** - 暂停并对齐两个播放器，逐帧对比

### ❌ 避免操作

1. ❌ 快速浏览 - 难以发现细微差异
2. ❌ 仅靠记忆对比 - 容易遗忘细节
3. ❌ 单一样本验证 - 可能不具代表性
4. ❌ 高播放速度 - 容易遗漏问题

---

## 自动化验证 (高级)

如果需要自动化逐帧对比，可以使用以下方法：

```bash
# 从两个播放器各输出一帧
ffmpeg -i https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  -vf "select=eq(n\,10)" -vsync vfr ffmpeg_frame_10.png

./target/release/tao-play https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi \
  --dump-frame 10 tao_frame_10.png 2>/dev/null

# 使用 ImageMagick 计算差异
compare ffmpeg_frame_10.png tao_frame_10.png -metric RMSE diff.miff
```

---

## 相关文件

- 📋 [MPEG4 Part 2 测试计划](./MPEG4_Part2_Decoder_Test_Plan.md)
- 📊 [MPEG4 Part 2 执行报告](./MPEG4_Part2_Decoder_Test_Execution_Report.md)
- 🔍 [FFmpeg 对比基线](./FFMPEG_BASELINE_SUMMARY.md) (待生成)
- 🎯 [样本 URL 清单](../samples/SAMPLE_URLS.md)

---

## 反馈与改进

如发现播放验证指南有错误或遗漏，请提交 Issue 或 PR：

```bash
git checkout -b improve/playback-verification-guide
# 编辑 PLAYBACK_VERIFICATION_GUIDE.md
git commit -m "docs: 改进播放验证指南"
git push origin improve/playback-verification-guide
```

---

**最后更新**: 2026-02-16  
**版本**: 1.0  
**维护者**: AI Copilot
