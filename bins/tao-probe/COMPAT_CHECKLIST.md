# tao-probe 对齐 ffprobe 7.1.3 执行清单

> 目标: Linux 基线 `ffprobe 7.1.3-0+deb13u1` 下 `stdout/stderr/exit code` 字节级一致。

## 公共接口/类型变更

- [x] A01: `parser` 增加原始参数序列字段, 保留 token 顺序与原始写法。
- [x] A02: `ParsedOption` 增加原始拼写元信息, 区分 `-opt value` 与 `-opt=value`。
- [x] A03: `command_plan` 增加有序执行项结构, 执行层按顺序 passthrough。
- [x] A04: `command_plan` 增加可选值三态模型, 保留隐式/显式值状态。
- [ ] A05: `model` 补齐字段元信息, 支持 writer 类型字符串化与可选/私有字段差异。
- [ ] A06: `writer` 增加 `-of` 选项强校验与错误语义对齐。
- [x] A07: `lib.rs` 保持 `run(argv)` 单入口语义。

## P0. 基建与追踪

- [x] P0-01: 新增 `COMPAT_CHECKLIST.md`。
- [x] P0-02: 新增 `compat_command_matrix_full.txt` 并覆盖扩展失败场景。
- [x] P0-03: 升级 `compat_matrix.sh`, 失败时输出 diff 摘要。
- [x] P0-04: 新增 `README_COMPAT.md` 本地流程文档。

## P1. 参数解析与执行语义

- [x] P1-01: 修复 `-show_pixel_formats` 无输入行为。
- [x] P1-02: 修复 `-show_optional_fields` 缺参语义。
- [x] P1-03: 修复 `-show_private_data` 可选值吞参规则。
- [x] P1-04: 修复 `-show_data_hash` 默认值与吞参规则。
- [x] P1-05: 按选项粒度收紧 `-opt=value` 兼容偏差。
- [x] P1-06: passthrough 使用原始顺序参数, 禁止 plan 重排。
- [x] P1-07: 修复 `show_entries` 前置/后置顺序差异。
- [x] P1-08: 统一 parser/build/runtime 错误路径与 banner 行为。

## P2. 输出模型与 writer 对齐

- [ ] P2-01: 对齐 `default` writer。
- [ ] P2-02: 对齐 `compact` writer。
- [ ] P2-03: 对齐 `csv` writer。
- [ ] P2-04: 对齐 `flat` / `ini` writer。
- [ ] P2-05: 对齐 `json` writer。
- [ ] P2-06: 对齐 `xml` writer。

## P3. 探测与过滤语义

- [ ] P3-01: 完整实现 `-show_entries` 语法覆盖。
- [ ] P3-02: 完整实现 `-select_streams` 规则。
- [ ] P3-03: 完整实现 `-read_intervals`。
- [ ] P3-04: 完整实现 `-count_packets` 在筛选/区间下的一致行为。
- [ ] P3-05: 完整实现 `-count_frames`。
- [ ] P3-06: 完整实现 `-find_stream_info`。

## P4. 白名单清零

- [x] W01 devices
- [x] W02 bsfs
- [x] W03 protocols
- [x] W04 filters
- [x] W05 layouts
- [x] W06 sample_fmts
- [x] W07 dispositions
- [x] W08 colors
- [x] W09 show_packets
- [x] W10 show_frames
- [x] W11 show_programs
- [x] W12 show_stream_groups
- [x] W13 show_chapters
- [x] W14 show_error
- [x] W15 show_log
- [x] W16 show_data
- [x] W17 show_data_hash
- [x] W18 read_intervals
- [x] W19 find_stream_info
- [x] W20 count_frames
- [x] W21 sources
- [x] W22 sinks
- [x] W23 show_pixel_formats

## P5. 收口

- [x] P5-01: 清空 `unimplemented` 白名单。
- [x] P5-02: full matrix 0 差异。
- [x] P5-03: 清理无关兼容分支并保持单入口稳定。
- [x] P5-04: 输出 `COMPAT_REPORT.md`。
