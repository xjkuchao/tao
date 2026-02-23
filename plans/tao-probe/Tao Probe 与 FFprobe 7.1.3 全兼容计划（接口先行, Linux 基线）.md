# Tao Probe 与 FFprobe 7.1.3 全兼容计划（接口先行, Linux 基线）

## 摘要
目标是把 `tao-probe` 做成 `ffprobe 7.1.3` 的兼容实现, 覆盖参数、输入输出格式、退出码与错误风格。执行策略是“先把全部接口做出来”, 对 Tao 核心暂缺能力统一走 `Function not implemented` 路径, 同时在代码中写中文 `TODO` 注释并维护未实现白名单, 再逐步清零直到达到字节级对拍一致。

## 当前基线差距（已确认）
1. `bins/tao-probe/src/main.rs` 仅支持极少参数（约 6 个）, 与 `ffprobe -h` 主参数集（60+）差距巨大。  
2. 当前输出是自定义中文结构, 与 ffprobe 的 `default/compact/csv/flat/ini/json/xml` 写出规则不兼容。  
3. 当前参数解析基于 clap, 不支持 ffprobe 的单短横长参数风格（如 `-show_streams`）、输入参数前后混排、多别名语法。  
4. 当前启动会初始化文件日志并输出 banner, 行为与 ffprobe 不一致。  

## 关键接口与类型变更（实现前先定版）
1. CLI 入口改为双入口兼容：保留 `tao-probe` 并新增 `ffprobe` 二进制入口（同一代码路径）。  
2. 新增 ffprobe 参数规格表：在 `bins/tao-probe/src/cli/ffprobe_7_1_3_options.rs` 定义全量参数接口（Main options + AVOptions 名称接口）。  
3. 新增自定义 argv 解析器：`bins/tao-probe/src/cli/parser.rs`，支持 `-opt value`、`-opt=value`、多别名、重复参数、输入参数前后混排。  
4. 新增统一执行模型：`bins/tao-probe/src/core/command_plan.rs`，将参数解析结果归一为 `CommandPlan`。  
5. 新增统一输出模型：`bins/tao-probe/src/model/` 下定义 section/value/schema（含字段类型、可选性、默认显示策略）。  
6. 新增多 writer：`bins/tao-probe/src/writer/{default,compact,csv,flat,ini,json,xml}.rs`，严格复刻 ffprobe 写出格式。  
7. 扩展 `crates/tao-format/src/demuxer.rs`：增加默认方法接口（如 `chapters/programs/stream_groups/format_long_name/start_time/bit_rate`），默认空实现，不破坏现有 demuxer。  
8. 扩展 `crates/tao-codec/src/packet.rs`（必要时）：增加 side-data 接口壳（默认空），用于 `-show_packets/-show_data/-show_data_hash` 兼容。  

## 实施步骤（决策完整, 可直接执行）
1. 先创建计划文件 `plans/tao_probe_ffprobe_compatibility_100.md`，写入背景、分步任务、依赖、验收标准、进度标记。  
2. 重构 `bins/tao-probe/src/main.rs` 为薄入口，只保留 `run(argv, invocation_name)` 调度。  
3. 落地全量参数接口表与解析器，先保证“所有参数可识别 + 错误消息风格兼容”。  
4. 实现全局命令分发（`-version/-buildconf/-L/-h/-formats/-codecs/...` 与探测类命令分离）。  
5. 实现探测数据采集管线：输入解析（`-i` 与位置参数）、`-f` 强制格式、`-select_streams`、`-show_entries`、`-read_intervals` 接口。  
6. 实现 section 生产器：`format/streams/packets/frames/programs/stream_groups/chapters/error/program_version/library_versions/pixel_formats`。  
7. 实现 7 种 writer 与 writer 选项解析（含 `-of xxx=...`）。  
8. 实现单位与显示修饰链（`-unit/-prefix/-byte_binary_prefix/-sexagesimal/-pretty/-show_optional_fields/-show_private_data`）。  
9. 实现“未实现白名单”机制：参数接口已接入但核心能力未完成时，统一返回 `Function not implemented`（退出码 1），并记录白名单条目。  
10. 在所有缺失核心点写中文注释：`// TODO(ffprobe-compat): ... 当前返回 Function not implemented ...`。  
11. 实现双入口行为对齐：`ffprobe` 名称下帮助文案、usage、错误前缀严格按 ffprobe 风格；`tao-probe` 入口走同逻辑。  
12. 清理旧参数兼容层：将 `--json/--show-format/--show-streams/--show-packets` 作为隐藏兼容别名映射到 ffprobe 语义。  

## 未实现输出与 TODO 规则（按你已确认的口径）
1. 核心能力缺失时不新增中文业务字段；终端输出采用 ffprobe 风格错误文本，核心关键词为 `Function not implemented`。  
2. 所有未实现点必须有中文 `TODO` 注释，且带具体缺失能力与目标行为。  
3. 白名单文件固定在 `bins/tao-probe/src/compat/unimplemented.rs`，每条包含参数名、缺失原因、关联模块、清零条件。  
4. 兼容门禁分层：支持清单走字节级对拍；白名单项允许“未实现”输出，直到 Tao 核心补齐后移出白名单。  

## 测试与场景（字节级对拍）
1. 新增 `tests/ffprobe_compat_pipeline.rs`：对每条命令比较 `stdout/stderr/exit code` 与本机 `ffprobe 7.1.3`。  
2. 参数解析回归：覆盖单短横长参数、别名、参数值缺失、未知参数、多输入冲突、输入前后混排。  
3. writer 回归：同一 `show_entries` 在 7 种输出格式下字节级对拍。  
4. 错误回归：不存在文件、非法 `-of`、非法 `-show_entries`、非法 hash 算法、非法 stream specifier。  
5. 白名单回归：对白名单参数断言输出 `Function not implemented` 与退出码 1。  
6. 样本策略：单元测试优先代码构造数据；对拍样本使用固定远程 URL（`samples.ffmpeg.org`）并更新 `samples/SAMPLE_URLS.md`。  

## 验收标准
1. 全量参数接口已接入（含 Main options + AVOptions 名称接口），无“未知参数”漏网。  
2. 支持清单命令在 Linux 基线对拍中 `stdout/stderr/exit code` 完全一致。  
3. 未实现清单全部有 TODO 注释和白名单条目，运行时返回统一未实现风格。  
4. `tao-probe` 与 `ffprobe` 双入口均可工作，行为一致。  
5. 最终目标阶段：白名单清零，达到你要求的“各方面 100% 兼容”。  

## 已锁定的默认假设
1. 目标版本：`ffprobe 7.1.3-0+deb13u1`。  
2. 平台基线：Linux 优先。  
3. 输出语言：完全按 ffprobe 英文原样。  
4. 兼容粒度：字节级一致（stdout/stderr/exit code）。  
5. 缺失核心能力处理：先进入未实现白名单，不做伪实现。  
6. 现有脏工作区改动（`crates/tao-codec/src/decoders/h264/{deblock.rs,macroblock_intra.rs,output.rs}`）保留并忽略，不纳入本任务。  
