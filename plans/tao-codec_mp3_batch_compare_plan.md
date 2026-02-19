# tao-codec MP3 样本批量对比计划

## 1. 背景与目标
- 需要对 `https://samples.ffmpeg.org/allsamples.txt` 中全部 MP3 样本进行批量精度对比。
- 基准为 FFmpeg 输出, 对比 Tao 解码结果。
- 目标输出: 每个样本的成功/失败状态与差异指标, 支持断点续运行。

## 2. 依赖与前置条件
- 必须安装: `ffmpeg`, `ffprobe`。
- 需要网络访问 `samples.ffmpeg.org`。
- 若使用 URL 直读, Tao 侧需启用 `tao-format` 的 `http` feature。
- 若未启用 `http` feature, 需先下载样本到 `data/` 后再执行本计划。

## 3. 计划步骤与预期产出

### P0 生成 URL 清单
- 产出: URL 已整合到 `plans/tao-codec_mp3_coverage/tao-codec_mp3_samples_report.md` 的 URL 列, 独立清单文件已删除。
- 内容: 由 `allsamples.txt` 提取的全部 MP3 完整 URL 列表(共 185 条)。

### P1 建立结果记录表
- 产出文件: `plans/tao-codec_mp3_coverage/tao-codec_mp3_samples_report.md`
- 表头字段(建议):
  - `序号`
  - `URL`
  - `状态(成功/失败)`
  - `失败原因`
  - `Tao样本数`
  - `FFmpeg样本数`
  - `样本数差异(Tao-FFmpeg)`
  - `max_err`
  - `psnr(dB)`
  - `精度(%)`
  - `备注`
- 断点规则:
  - 每个样本处理后立即写入对应行。
  - 已有 `状态` 的样本跳过, 仅处理空状态的样本。

### P2 执行方式与脚本
- 执行方式:
  - 优先使用 `tests/perf_compare/mp3_module_compare.rs`:
    - `TAO_MP3_COMPARE_INPUT=<URL或本地路径> cargo test --test mp3_module_compare -- --nocapture --ignored`
- 若未启用 `http` feature:
  - 先将 URL 下载到 `data/mp3_samples/`。
  - 然后对本地文件路径执行同样的测试命令。
- 可选产出:
  - `plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py`
  - 功能: 读取 URL 清单, 逐个执行对比, 自动写入结果表。

### P3 批量执行与记录
- 顺序执行 URL 清单, 逐条运行:
  - 成功: 记录 Tao/FFmpeg 样本数、样本数差异、max_err、psnr、精度。
  - 失败: 记录失败原因(如 `http feature` 未启用、`ffmpeg` 解码失败、`Tao` 解码错误等)。
- 严格按断点规则跳过已完成条目。

### P4 汇总与验收
- 汇总统计:
  - 总样本数、成功数、失败数、失败原因分类。
  - 精度分布(可按区间统计)。
- 验收:
  - `plans/tao-codec_mp3_coverage/tao-codec_mp3_samples_report.md` 覆盖全部 185 条样本。

## 4. 验收标准
- 全部 URL 已写入结果表, 结果表包含所有样本记录, 且支持断点续运行。
- 成功样本记录完整差异指标; 失败样本记录具体原因。

## 5. 进度标记
- [x] P0 生成 URL 清单
- [x] P1 建立结果记录表
- [x] P2 执行方式与脚本
- [ ] P3 批量执行与记录
- [ ] P4 汇总与验收
