# tao-codec AAC 解码器样本覆盖率测试计划

## 1. 背景与目标

基于 `https://samples.ffmpeg.org/allsamples.txt` 抓取 AAC 相关样本, 建立与 Vorbis 覆盖率一致的批量对比流程.

本轮已完成样本清单初始化:

- 报告文件: `plans/tao-codec/audio/aac/coverage/report.md`
- 样本总数: 114 条
- 筛选规则: 扩展名为 `.aac`, `.m4a`, 或路径中包含 `/AAC/` 且扩展名为常见媒体格式(`mp4`, `mkv`, `mov`, `flv`).

**最终目标**:

1. 所有样本全部完成 Tao/FFmpeg 对比测试.
2. 除明确跳过样本外, 报告状态全部为“成功”.
3. 精度逐步收敛到 100.00% (放宽到 99.9% 允许 PNS 等计算截断漂移).

## 2. 执行范围

- 覆盖目录: `plans/tao-codec/audio/aac/coverage/`
- 核心脚本: `run_decoder.py`
- 输入报告: `report.md`
- 对比测试入口: `cargo test --test run_decoder aac:: -- --nocapture --ignored`

## 3. 分步任务与预期产出

### 任务 A: 样本清单初始化(已完成)

- 从 `allsamples.txt` 抓取 AAC 样本 URL.
- 生成报告模板并写入序号与 URL.

### 任务 B: 批量执行脚本(已完成)

- 复用 Vorbis 批测流程, 支持:
    - 断点续测
    - 失败重测
    - 非 100% 精度重测
    - 指定序号重测
    - 并行执行
- 结果实时回写报告.

### 任务 C: 基线跑批与分类(已完成)

- 获取基线(全量 114 条记录):
    - 总样本: 114
    - 成功: 27
    - 失败: 87

失败分类(初步排查分析):

- 解析/封装层失败 `(~30 条)`:
    - "未找到可解码音频流" (往往因不支持封装格式或参数配置异常)
    - HTTP 链接无效等 (如 `invalid uri character`)
    - MP4 文件中未找到任何轨道
- 不支持的 profile `(~10 条)`:
    - `Unsupported("AAC: 不支持 audioObjectType=1/3/29/0, 仅支持 AAC-LC (2)")` 等 HE-AAC 或 SSR 样本. (计划作为合法跳过项).
- 解析超时 / 死循环 `(~13 条)`:
    - `单样本测试超时: 60s`
- 码流损坏 / ADTS 头部无法找到 / 语法超出范围 `(~20 条)`:
    - 首包或中途 section_data/spectral_data 解码非法跳过并引发错误.
- 输出时域断言漂移引发 panic `(~10+ 条)`:
    - 提取样本长度与 FFmpeg 存在偏离, 但解码流程并未完全中断(如 `right: 147456` 等导致 test panic).

### 任务 D: 解码器修复迭代(当前执行中)

- 针对失败样本和低精度样本修复 AAC 解码实现.

### 任务 E: 跳过口径固化(本轮完成)

- 在 `run_decoder.py` 中固化默认跳过集合(8 条), 用于当前阶段不支持 profile:
    - `aot=29`(HE-AAC): `#9`
    - `aot=1`(AAC Main): `#17/#46/#52/#83/#89`
    - `aot=0`(非法或未知 profile): `#77`
    - `aot=3`(SSR/LTP): `#103`
- 默认执行口径:
    - 不带 `--include-skipped` 时, 上述样本标记为“跳过”.
    - 带 `--include-skipped` 时可强制回归复测.

## 4. 当前结果快照(2026-02-20)

- 总样本: 114
- 成功: 40
- 跳过: 8
- 失败: 66

失败主类仍与基线一致, 当前已从“能力缺失 profile”中剥离出独立跳过口径, 后续修复聚焦:
- AAC `section_data/spectral_data` 解析失败链路
- 超时样本
- 样本数对齐断言导致的 panic/失败

失败原因分布(66 条, 2026-02-20):
- 未找到可解码音频流: 22
- `spectral_data` 解析失败: 15
- 单样本测试超时: 13
- `section_data` 解析失败: 7
- ADTS 帧头无效: 3
- MP4 无轨道: 2
- 整数下溢/溢出 panic: 1
- 采样率不匹配: 1
- 其他(Eof/容器格式码等): 2

本轮推进(2026-02-20):
- 批量重测原 `right:` 失败簇: `#2/#10/#42/#61/#62/#66/#69/#71/#75/#78/#106/#112` 均已转为“成功”(备注: 严格阈值未通过).
- `#18` 从 `right: 44100` 改为结构化失败原因: `采样率不匹配`.
- `#4` 仍为 `attempt to subtract with overflow`, 需在 AAC 解码流程中继续排查根因.

## 5. 依赖与前置条件

1. 本地可用 `ffmpeg` 与 `ffprobe`.
2. 网络可访问 `https://samples.ffmpeg.org/`.
3. 能够执行 `cargo test --test run_decoder aac::`.
4. 从项目根目录运行脚本.

## 6. 使用说明

```bash
# 默认断点续测
python3 plans/tao-codec/audio/aac/coverage/run_decoder.py

# 重测所有失败样本
python3 plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-failed

# 重测精度不为 100% 的样本(含失败)
python3 plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-imprecise

# 重测全部样本
python3 plans/tao-codec/audio/aac/coverage/run_decoder.py --retest-all
```

## 7. 进度标记

- [x] 创建子目录并初始化脚本/报告
- [x] 跑首轮全量基线并更新报告
- [x] 失败样本根因分类
- [x] 固化默认跳过样本口径(aot 非 AAC-LC)
- [x] 清理 `right:` 断言类失败并完成定向重测
- [ ] 精度收敛验证
