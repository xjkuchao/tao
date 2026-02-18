# tao-codec Vorbis 解码器开发计划

## 1. 背景与目标
- 当前 Vorbis 仍未完成音频主链路解码。
- 必须遵守项目规则: 纯自研实现, 不依赖外部多媒体算法库。
- 目标: 对 `data/1.ogg` 与 `data/2.ogg` 完成可播放解码与逐帧对标 FFmpeg。

## 2. 模块化决策(按新规则)
- 判定: Vorbis 完整实现复杂度高(头包解析、codebook/floor/residue/mapping、IMDCT、overlap、耦合反变换), 不适合单文件。
- 决策: 采用独立子目录实现。
- 目录规划:
  - `crates/tao-codec/src/decoders/vorbis/mod.rs`: 解码器状态机与对外接口
  - `crates/tao-codec/src/decoders/vorbis/headers.rs`: identification/comment 头包解析
  - `crates/tao-codec/src/decoders/vorbis/setup.rs`: setup 语法解析与结构校验
  - `crates/tao-codec/src/decoders/vorbis/bitreader.rs`: Vorbis LSB 比特读取器
  - 后续按需要新增 `imdct.rs` `floor.rs` `residue.rs` `mapping.rs` `synthesis.rs`

## 3. 里程碑与执行顺序

### 执行与提交规则(强制)
- 每完成一个关键变更(如: 头包解析修正、setup 严格解析修正、IMDCT 接入、floor/residue 完成、对比测试落地), 必须立即执行:
  1. `cargo fmt --check`
  2. `cargo clippy -- -D warnings`
  3. `cargo check`
  4. `cargo test`
- 四项通过后, 立即提交该关键变更代码, 提交信息使用中文并准确描述本次关键点。
- 禁止将多个关键变更长期堆积后一次性提交, 以确保可回滚、可审计、可断点续做。
- 若某关键变更阶段无法通过四项门禁, 不进入下一关键变更, 先修复到通过再提交。

### P0 基线与计划
- [x] 明确禁用外部多媒体库。
- [x] 输出可续执行计划。

### P1 结构重构与状态机
- [x] `vorbis` 从单文件迁移到子目录模块。
- [x] `send_packet/receive_frame/flush` 状态机可工作。
- [x] 三头包入口接通 (`identification/comment/setup`)。
- 验收: 可推进到音频包阶段。

### P2 setup 解析基础设施
- [x] 实现 codebook/floor/residue/mapping/mode 解析框架。
- [x] setup 保留结构化 mode->mapping/mux/coupling 信息, 为后续 floor/residue 真正解码做准备。
- [x] setup 保留 codebook/floor/residue 细粒度配置并增加运行时一致性校验。
- [x] 新增 codebook Huffman 基础模块并接入 setup 运行时校验。
- [x] 样本 `data/1.ogg` `data/2.ogg` 可通过 setup 阶段并进入音频包。
- [x] 收敛 setup 解析中的降级路径, 完成严格解析闭环。
- 验收: 去除降级后仍稳定通过样本。

### P3 音频包解码主链路(进行中)
- [x] 模式切换、块长推进、PTS 递增与基础帧输出队列接通。
- [x] 主链路按模块拆分并接通调用关系:
  - `floor.rs`
  - `residue.rs`
  - `imdct.rs`
  - `synthesis.rs`
- [x] overlap 状态与拼接接口接入主流程 (当前为占位实现)。
- [x] Ogg granule 语义修正与解码端时长对账(仅页尾完整包携带 granule, 其余回退块长推进)。
- [x] 接入 channel coupling 反变换流程(当前 residue 仍为占位频谱)。
- [x] 修正 long-window 包头标志位消费并接入基于 mapping 的 floor 上下文映射。
- [x] 接入 floor1 音频包解析与曲线重建(当前幅度映射为近似实现)。
- [x] 接入 residue 近似解码(按 bitstream/codebook 消费并生成频谱占位增量)。
- [x] 接入 codebook lookup 参数解析与向量恢复接口(含 Vorbis float32 unpack)。
- [x] 接入 residue type0/1/2 的向量写入主流程(当前仍需继续做精确对齐与幅度收敛)。
- [x] 为 residue 向量注入增加临时增益归一化, 避免当前阶段幅度失真导致对比不可用。
- [x] 修复 residue type2 误按声道重复解码问题, 改为按声道组单次解码避免位流错位。
- [x] 修正 IMDCT 角度公式分母错误(`N/2 -> N`), 收敛频域到时域变换比例。
- [x] 接入 long-block 在 short 邻接场景的 Vorbis 窗形选择逻辑(使用 prev/next window flag)。
- [x] 重构 residue2 扁平索引推进逻辑(`flat_idx`), 清理跨向量边界的样本偏移风险。
- [x] 将 codebook Huffman 构建前移到 setup 阶段并缓存复用, 移除每音频包重复构建。
- [x] 对 residue 向量增益进行样本扫描调优, 将 `RESIDUE_VECTOR_GAIN` 收敛到 `0.00024`。
- [x] 去除 residue 分区解码热路径中的重复临时分配, 改为向量缓冲复用。
- [x] floor1 邻点搜索改为严格不等关系(`<`/`>`), 避免相等点参与预测。
- [x] 窗函数改为解码器侧缓存复用, 避免每音频包重复构建 IMDCT 窗数组。
- [x] 接入 flush 阶段尾样本排空逻辑(基于 granule 与 next_pts 对账)。
- [x] 复用 residue 分类向量缓冲(`class_vec`), 减少每声道重复分配。
- [x] 对齐 FFmpeg/lewton: residue classword 分类拆分改为反向填充顺序。
- [x] 对齐 FFmpeg/lewton: 在 residue 前加入 no_residue 反向传播, 并修正 type2 子映射声道集合语义。
- [x] 基于新语义重新扫描 residue 增益并收敛到 `RESIDUE_VECTOR_GAIN=0.00018`。
- [x] 对齐 FFmpeg: 首个长块按 `prev_window_flag` 初始化 `previous_window` 对应块长。
- [x] 对齐 lewton: 包输出样本数改为 `right_win_start-left_win_start` 公式, 替换 `(prev+curr)/4` 近似。
- [x] 对齐 lewton/FFmpeg: floor1 曲线改为 `render_line + inverse_db_table` 方案并保留完整查表精度。
- [x] 修正首包解码语义: 首包参与解码但不输出, 仅初始化 overlap 状态。
- [x] 对比测试驱动改为 EOF 送空包 + drain, 避免遗漏尾帧。
- [x] residue 解码改为按 codebook 向量写入并去除临时增益。
- [x] IMDCT 缩放因子调整为 `1/N` 并改进 overlap 产出区间。
- [x] flush 尾样本输出加入能量阈值, 用于抑制极小尾噪声。
- [ ] 窗口、IMDCT、重叠相加。
- [ ] floor1 恢复、residue 解码、耦合反变换。
- [ ] 输出 `Frame::Audio(F32 interleaved)` + PTS/duration/time_base。
- 验收: `tao-play` 可播放 `data/1.ogg` `data/2.ogg`。

### P4 逐帧对标测试
- [x] 新增 `tests/vorbis_module_compare.rs`。
- [x] 与 FFmpeg 比较 MSE/PSNR/最大误差并输出报告。
- [x] 接入 Lewton 对比并输出精度百分比。
- [ ] 建立并满足误差阈值。
- 当前基线:
  - `data/1.ogg`: PSNR 约 `18.76dB`, max_err 约 `1.621305`, 精度 约 `45.90%`
  - `data/2.ogg`: PSNR 约 `13.02dB`, max_err 约 `6.791577`, 精度 约 `46.96%`
  - Lewton/FFmpeg: PSNR 约 `95.08dB`, max_err 约 `0.000031`, 精度 约 `100.00%`
  - 样本长度: `data/1.ogg` Tao=FFmpeg=`881996`; `data/2.ogg` Tao=FFmpeg=`2646000`
- 验收: 两个样本对比测试通过。

### P5 质量门禁与交付
- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo check`
- [x] `cargo test`
- [ ] 总结偏差与剩余事项。

## 4. 前置条件
- 输入样本: `data/1.ogg`, `data/2.ogg`。
- 本地工具: `ffmpeg`, `ffprobe` (仅用于对比, 不参与解码实现)。

## 5. 验收标准
- Vorbis 全链路为 Tao 自研实现。
- 两个样本可稳定解码并逐帧对标 FFmpeg。
- 四项门禁全部通过。

## 6. 进度标记
- [x] P0
- [x] P1
- [x] P2
- [ ] P3
- [ ] P4
- [x] P5
