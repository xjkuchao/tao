# H264 解码器 -- 精度收敛计划

> 前置条件: `decoder_dev.md` P1-P6.6 **全部完成**后才允许进入本计划.
>
> 关联文档:
> - 功能开发: `decoder_dev.md`
> - 性能优化: `decoder_perf.md`
> - 诊断日志: `diagnosis_log.md`

## 1. 目标

- 与 FFmpeg 逐帧对比, 使全部测试样本达到以下精度指标:
    - Y-PSNR >= 50dB.
    - 像素级精度(完全一致像素占比) >= 99%.
    - 最大单像素误差 <= 2.
- 建立可回归复跑的精度基线, 防止后续优化引入回归.

## 2. 对比基础设施

- [ ] 维护 `plans/tao-codec/video/h264/decoder_compare.rs` 对比入口.
- [ ] 逐帧对比输出: Y/U/V 各平面的 PSNR, 最大误差, 精度百分比.
- [ ] 自动生成 JSON/CSV 报告, 存入 `plans/tao-codec/video/h264/coverage/`.
- [ ] 集成到 CI: 精度回归超阈值时测试失败.

## 3. 测试样本

### 3.1 核心样本

| 样本 | 路径 | Profile | 特征 |
| --- | --- | --- | --- |
| sample1 | `data/1_h264.mp4` | High | 通用测试 |
| sample2 | `data/2_h264.mp4` | High | 通用测试 |

### 3.2 扩展样本(达标后逐步补充)

| 类别 | 覆盖目标 | 来源 |
| --- | --- | --- |
| Baseline + CAVLC | 纯 CAVLC 路径验证 | `https://samples.ffmpeg.org/` |
| Main + CABAC | B 帧密集 + 加权预测 | `https://samples.ffmpeg.org/` |
| High + 8x8 变换 | 8x8 IDCT + 自定义量化矩阵 | `https://samples.ffmpeg.org/` |
| 多参考帧 | MMCO + 长期参考 | `https://samples.ffmpeg.org/` |
| 大分辨率(1080p+) | 宏块数量/DPB 压力 | `https://samples.ffmpeg.org/` |
| POC type 0/1/2 | 各 POC 计算路径 | `https://samples.ffmpeg.org/` |
| I-only | 纯帧内路径 | `https://samples.ffmpeg.org/` |

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

- 逐帧 dump: Tao vs FFmpeg 各宏块的 MV/ref_idx/residual/重建像素.
- 逐宏块对比: 在首个偏差帧中定位首个偏差宏块.
- CABAC 状态 trace: 对比 CABAC 上下文状态与 FFmpeg 的 ctxIdx/state/mps 演进.
- 环境变量开关: 临时隔离模块(如强制 4x4/跳过去块)缩小范围.

### 4.2 常见偏差根因分类

| 类别 | 表现 | 排查方向 |
| --- | --- | --- |
| CABAC 语法失步 | slice 提前结束 / 宏块数不足 | 上下文增量 / 残差块类别 / 扫描顺序 |
| 帧内预测偏差 | I 帧像素系统性偏移 | 预测公式 / 邻居可用性 / 滤波 |
| 帧间预测偏差 | P/B 帧像素偏移 | MV 推导 / Direct 模式 / 加权预测 |
| 残差偏差 | 重建像素高频噪声 | 反量化 / IDCT 精度 / 扫描表 |
| 去块偏差 | 边界伪影 | BS 计算 / alpha/beta/tc0 / 强弱滤波 |
| DPB/POC 偏差 | 错帧/错序 | POC 计算 / 参考列表构建 / MMCO |

## 5. 精度基线记录

> 以下为功能开发阶段的**临时诊断数据**, 仅供参考, 不作为精度结论.

### 当前基线(120 帧, 2026-02-21, 功能未完整)

| 样本 | 精度 |
| --- | --- |
| data/1_h264.mp4 | 1.343662% |
| data/2_h264.mp4 | 1.792586% |
| 平均 | 1.568124% |

### 已知主瓶颈

- 首个 IDR slice CABAC 语法失步(decoded_mbs=102/8160).
- I_8x8 coded_block_flag 上下文建模不完整.
- B-slice list1/双向仍有近似路径.

详细诊断记录见 `diagnosis_log.md`.

## 6. 验收标准

- [ ] 核心双样本: Y-PSNR >= 50dB, 像素精度 >= 99%, 最大误差 <= 2.
- [ ] 扩展样本: 全部达到相同指标.
- [ ] 精度回归 CI 门禁通过.
- [ ] 输出最终精度报告(各样本各帧统计).

## 7. 进度

- [ ] 对比基础设施搭建
- [ ] 核心双样本达标
- [ ] 扩展样本达标
- [ ] CI 精度门禁集成
- [ ] 最终精度报告
