# tao-probe 与 ffprobe 7.1.3 兼容执行计划（接口先行）

## 背景与目标
- 背景: 当前 `bins/tao-probe` 仅具备少量参数和自定义输出, 与 `ffprobe 7.1.3-0+deb13u1` 差距较大。
- 目标: 先完成接口层 100% 接入（参数可识别、命令分发可达、输出 writer 架构齐备）, 对核心暂缺能力统一返回 `Function not implemented`。
- 范围: Linux 基线, 双入口 `tao-probe`/`ffprobe`, 输出风格与错误语义向 ffprobe 对齐。

## 分步任务与预期产出
1. 建立新架构
- 产出: `run(argv, invocation_name)` 薄入口, `cli/core/model/writer/compat` 目录与基础模块。

2. 参数接口与解析器
- 产出: `ffprobe_7_1_3_options.rs` 全量主参数规格 + AVOption 名称接口清单, 自定义 parser 支持 `-opt value`、`-opt=value`、重复参数、输入前后混排。

3. 命令分发与执行模型
- 产出: `CommandPlan` 归一化模型, 区分全局命令与探测命令, 旧参数别名映射到 ffprobe 语义。

4. 统一输出模型与 7 种 writer
- 产出: section/value/schema 模型, `default/compact/csv/flat/ini/json/xml` 写出器与 `-of` 解析。

5. 未实现白名单机制
- 产出: `compat/unimplemented.rs` 白名单, 所有缺失点附中文 `TODO(ffprobe-compat)` 注释, 统一返回 `Function not implemented`。

6. 核心接口扩展
- 产出: `tao-format::Demuxer` 默认扩展接口（chapters/programs/stream_groups/format_long_name/start_time/bit_rate）;
  `tao-codec::Packet` side-data 接口壳（默认空）。

7. 测试与验证
- 产出: `tests/ffprobe_compat_pipeline.rs` 覆盖参数解析、writer、错误、白名单; 执行 `fmt/clippy/check/test/doc` 门禁。

## 依赖与前置条件
- 系统已安装 `ffprobe 7.1.3-0+deb13u1`（用于对拍和错误文案基线采样）。
- 本仓库 `.codegraph/` 可用（查询需显式 `projectPath`）。

## 验收标准
- 主参数接口与 AVOption 名称接口可识别, 无已纳入范围的未知参数漏网。
- `tao-probe` 与 `ffprobe` 两个入口可运行且共享同一逻辑。
- 支持项输出稳定, 未实现项统一 `Function not implemented` + 退出码 1。
- 新增/改动代码可通过编译与测试门禁。

## 进度标记
- [x] 读取基线计划与确认差距
- [x] 建立本执行计划文件
- [x] 搭建 `tao-probe` 新架构与双入口
- [x] 完成参数规格表与 parser
- [x] 完成 `CommandPlan` + 命令分发
- [x] 完成模型与 7 writer
- [x] 接入未实现白名单与 TODO 注释
- [x] 扩展 Demuxer/Packet 接口
- [x] 新增兼容测试
- [ ] 执行门禁并修复（受工作区既有问题影响, 见本轮执行记录）
