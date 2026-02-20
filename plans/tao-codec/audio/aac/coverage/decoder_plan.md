# tao-codec AAC 解码器样本覆盖率测试计划

## 1. 背景与目标

基于 `https://samples.ffmpeg.org/allsamples.txt` 抓取 AAC 相关样本, 建立与 Vorbis 覆盖率一致的批量对比流程.

本轮已完成样本清单初始化:

- 报告文件: `plans/tao-codec/audio/aac/coverage/report.md`
- 样本总数: 114 条
- 筛选规则: 扩展名为 `.aac`, `.m4a`, 或路径中包含 `/AAC/` 且扩展名为常见媒体格式(`mp4`, `mkv`, `mov`, `flv`).

**最终目标**:

1. 所有样本全部完成 Tao/FFmpeg 对比测试.
2. 报告不再依赖默认跳过, 全部样本进入“成功/失败”真实口径.
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

### 任务 E: 真实口径回归(本轮完成)

- 按最新要求移除 `run_decoder.py` 默认跳过行为:
    - `SKIPPED_SAMPLE_INDEXES = set()`
    - 失败样本不再保留“已跳过”备注.
- 结果口径统一为:
    - `成功`: 已产出可比较指标
    - `失败`: 真实解码/封装/样本能力缺口

### 任务 H: overflow 根因修复(本轮完成)

- 修复 `crates/tao-codec/src/decoders/aac/mod.rs` 中 FIL 扩展长度解析的 `usize` 下溢:
    - 由 `count += read_u8 - 1` 改为 `count += esc_count.saturating_sub(1)`.
- 效果:
    - `#4/#109` 从 `attempt to subtract with overflow` 收敛为结构化对比失败(采样率不匹配), 不再触发 panic.

## 4. 当前结果快照(2026-02-20 最新)

- 总样本: 114
- 成功: 78
- 失败: 36

失败原因分布(36 条, 2026-02-20):
- `Unsupported("AAC: 不支持 audioObjectType=...")`: 8
- `未找到可解码音频流`: 22
- `InvalidData("MP4 文件中未找到任何轨道")`: 2
- `无效 ADTS 帧头`: 2
- `Unsupported("不支持的音频格式码: 0x00FF")`: 1
- `Eof`: 1

精度分布(成功样本 78 条, 2026-02-20):
- 精度 `100.00%`: 35 条
- 精度 `<100.00%`: 43 条

本轮推进(2026-02-20):
- 完整补齐 SFB 边界映射:
    - long/short 覆盖 `96k/64k/48k/32k/24k/16k/8k`.
    - 修复 `22.05k/11.025k/16k/8k` 样本频带切分偏差.
- 多声道输出顺序修正:
    - 新增默认声道配置重排(3/5/6/8 声道).
    - `#11/#12/#73` 已从低精度收敛到 `100.00%`.
- ADTS 头实时同步:
    - 解包时同步 `sample_rate_index/channel_config`, 避免配置漂移.
- 失败行字段清洗:
    - 不再残留“已跳过”备注, 报告仅保留真实失败原因.

当前阻塞(距离“全部 100% 精度”仍有差距):
- 仍有 43 条成功样本精度低于 `100.00%`.
- 最差样本集中在:
    - 非 AAC-LC 能力缺口(HE-AAC/Main/SSR 等).
    - 多声道复杂码流(仍有通道耦合/布局差异).
    - 特殊坏样本/非目标封装导致的对比不一致.

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
- [x] 清理 `right:` 断言类失败并完成定向重测
- [x] 超时样本 180s 定向重测(13 条)
- [x] 修复 FIL 扩展长度 overflow panic
- [x] 移除默认跳过并切换到真实成功/失败口径
- [x] 采样率族群 SFB 映射补齐(96k~8k)
- [x] 多声道默认声道顺序重排(3/5/6/8)
- [ ] 精度收敛到 100.00%(当前: 35/78)
