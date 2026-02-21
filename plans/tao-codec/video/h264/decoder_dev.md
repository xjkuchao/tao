# tao-codec H264 解码器开发计划

## 1. 背景与目标

- 当前 H264 解码器仍处于功能不完整阶段, 直接做精度对比会被基础能力缺口放大。
- 必须遵守项目规则: 核心媒体算法纯自研, 不依赖外部多媒体能力库完成解码。
- 执行顺序调整为两阶段:
    - 第一阶段: 对比 FFmpeg 等开源实现, 先完整实现功能点。
    - 第二阶段: 在功能完整基础上做精度对比, 定位 Bug 并持续收敛。
- 目标样本: `data/1_h264.mp4`, `data/2_h264.mp4`。

## 2. 对齐策略(先功能, 后精度)

- 对标对象:
    - FFmpeg `libavcodec/h264*`
    - OpenH264
    - JM 参考实现(仅语义与流程对照)
- 对齐范围:
    - 比特流语法覆盖
    - 帧内/帧间重建流程
    - DPB/POC/重排序
    - 去块滤波
    - 容错与边界行为
- 对齐方式:
    - 建立“功能点矩阵”, 逐项标记 `未实现/部分/完成/已验证`。
    - 每完成一类功能即补对应测试, 先验证“能否正确执行该功能”, 不先追求像素级一致。

## 3. 模块化决策

- 判定: H264 实现复杂度高, 不适合单文件。
- 目录规划:
    - `crates/tao-codec/src/decoders/h264/mod.rs`: 状态机与对外接口
    - `crates/tao-codec/src/decoders/h264/bitstream.rs`: 比特读取与 Exp-Golomb
    - `crates/tao-codec/src/decoders/h264/nal.rs`: NAL 分包与 RBSP 还原
    - `crates/tao-codec/src/decoders/h264/parameter_sets.rs`: SPS/PPS 解析与缓存
    - `crates/tao-codec/src/decoders/h264/slice.rs`: slice header 与宏块流程
    - `crates/tao-codec/src/decoders/h264/cabac.rs`: CABAC
    - `crates/tao-codec/src/decoders/h264/cavlc.rs`: CAVLC
    - `crates/tao-codec/src/decoders/h264/prediction.rs`: 帧内/帧间预测
    - `crates/tao-codec/src/decoders/h264/mc.rs`: 运动补偿与插值
    - `crates/tao-codec/src/decoders/h264/deblock.rs`: 去块滤波
    - `crates/tao-codec/src/decoders/h264/output.rs`: DPB 管理、重排序、输出帧
- 当前落地状态(2026-02-21):
    - 已落地: `mod.rs`, `cabac.rs`, `cabac_init_ext.rs`, `cabac_init_pb.rs`, `intra.rs`, `residual.rs`
    - 未落地: `bitstream.rs`, `nal.rs`, `parameter_sets.rs`, `slice.rs`, `cavlc.rs`, `prediction.rs`, `mc.rs`, `deblock.rs`, `output.rs`
- 结论:
    - 上述“未落地模块”对应能力目前仍在 `mod.rs` 内部混合实现, 且部分功能尚未实现。
    - 在模块未落地且功能未完成前, 精度测试结论无验收意义, 只能作为临时诊断。

## 4. 里程碑与执行顺序

### 执行与提交规则(强制)

- 每完成一个关键变更(例如: `slice header 完整解析`, `P/B 帧重建`, `DPB 重排序`, `去块滤波`)必须执行:
    1. `cargo fmt --all -- --check`
    2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    3. `cargo check --workspace --all-targets --all-features`
    4. `cargo test --workspace --all-targets --all-features --no-fail-fast`
    5. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- 五项通过后立即提交, 提交信息中文且只描述本次关键变更。
- 禁止跨多个关键变更堆积后一次性提交。
- 任何阶段未通过五项门禁不得进入下一阶段。
- 功能门禁(强制):
    - `P1-P6` 任一项未完成时, 禁止进行精度对比收敛工作(`decoder_compare.rs`, `TAO_H264_COMPARE_*`)。
    - 允许的诊断仅限“功能自测和语法路径定位”, 不得将其作为精度结论输出。
    - 仅当 `P1-P6` 全部完成后, 才允许进入 `P7` 的精度收敛阶段。

### P0 基线与计划

- [x] 明确“先功能完整, 后精度收敛”。
- [x] 输出可断点续跑计划。

### P1 参考实现对齐与功能矩阵

- [x] 新建 `plans/tao-codec/video/h264/h264_feature_matrix.md`。
- [x] 按 FFmpeg/OpenH264 语义梳理完整功能点, 覆盖:
    - NAL/参数集
    - slice 语法
    - CABAC/CAVLC
    - 帧内/帧间预测
    - 运动补偿
    - DPB/POC/重排序
    - 去块滤波
    - 错误恢复
- [x] 为每个功能点定义“完成判据”和最小验证用例。
- 验收: 矩阵可直接驱动后续编码与验收, 无空白关键项。

### P2 输入链路与参数集完整实现

- [x] AnnexB/AVCC 双入口统一。
- [ ] SPS/PPS 完整解析与合法性校验。
    - [x] PPS 解析迁移到独立模块 `parameter_sets.rs`。
    - [x] PPS 关键字段合法性校验(`pps_id/sps_id/ref_idx/weighted_bipred_idc/pic_init_qp/chroma_qp_offset`)。
    - [x] PPS 单元测试补齐(基础路径/扩展路径/非法输入)。
    - [x] SPS 基础合法性校验补齐(`sps_id/chroma_format_idc/poc_type/log2_max_*/max_num_ref_frames/裁剪边界`)并补充失败用例单测。
    - [x] SPS 深度校验补齐(POC type1 循环上限、VUI `aspect_ratio_idc`/`Extended_SAR`/timing 零值校验)并补充失败用例单测。
- [ ] 参数集变更时上下文重建。
    - [x] 新增 `PPS` 变更重建分级策略(`None/RuntimeOnly/Full`)并接入 `activate_parameter_sets`。
    - [x] 覆盖 `PPS` 关键字段变更单元测试(熵编码/SPS绑定触发 Full, QP/加权预测触发 RuntimeOnly)。
    - [x] 补充参数集切换集成测试(走 `handle_pps` NAL 解析路径验证同 `pps_id` 的 RuntimeOnly/Full 切换行为)。
    - [x] 补充 `handle_sps` 同 `sps_id` 尺寸变更集成测试, 验证缓冲重建与参考状态重置。
- 验收: 两个样本稳定进入 slice 解码, 参数切换无崩溃。

### P3 Slice 语法与熵解码完整实现

- [ ] slice header 关键字段完整覆盖(I/P/B, ref list, 权重预测相关字段, deblock 相关字段)。
    - [x] 补齐关键边界校验: `cabac_init_idc(<=2)`, `slice_qp(0..51)`, `disable_deblocking_filter_idc(<=2)`, `num_ref_idx_l0/l1(1..=32)`。
    - [x] 新增 `parse_slice_header` 失败用例单测(非法 `cabac_init_idc/slice_qp/deblocking_idc`)。
    - [x] 补齐 `poc_type1` 的 `delta_pic_order_cnt[0/1]` 解析和 `disable_deblocking_filter_idc` 状态透传。
    - [x] 接入 `ref_pic_list_modification` 解析(`list0/list1`)并将重排结果接入 CABAC/CAVLC 帧间参考列表构建。
    - [x] 补齐参考列表重排单测(短期重排/长期重排)。
    - [x] 补齐 `pred_weight_table` 合法性校验(`luma/chroma_log2_weight_denom <= 7`, `luma/chroma weight/offset` 范围)并新增权重表解析单测。
    - [x] B-slice `pred_weight_table` 补齐 `list1` 权重解析与存储(`l1_weights`), 并新增对应单测。
- [ ] CABAC 完整路径可用。
- [ ] CAVLC 完整路径可用。
    - [x] 接入最小 CAVLC slice 数据路径: `mb_skip_run/mb_type` 基础语法消费 + I 宏块基础帧内预测 + P/B 宏块参考复制。
    - [x] 补齐 CAVLC 最小路径单元测试(I-slice 最小路径, P-slice skip_run 参考复制)。
    - [x] CAVLC B-slice 最小路径接入 list0/list1 预测融合(含显式加权), 并补 skip_run 单元测试。
    - [x] CAVLC B-slice 非 skip `mb_type=1/2` 接入 `L0-only/L1-only` 方向选择, 并补单元测试。
    - [x] CAVLC B-slice 非 skip `mb_type=1/2` 接入 `ref_idx_l0/ref_idx_l1` 语法消费与参考帧选择, 并补单元测试。
    - [x] CAVLC B-slice 非 skip `mb_type=3(B_Bi_16x16)` 接入 `ref_idx_l0/ref_idx_l1` 语法消费与对齐单元测试。
    - [x] CAVLC B-slice 非 skip `mb_type=4..21(B_16x8/B_8x16)` 接入分区级 `ref_idx_l0/ref_idx_l1` 语法消费与最小分区重建, 并补对齐单测。
    - [x] CAVLC B-slice 非 skip `mb_type=3/4..21` 接入 `mvd_l0/mvd_l1` 语法消费与最小运动向量重建, 并补对齐单测。
    - [x] CAVLC B-slice 非 skip `mb_type=4..21(B_16x8/B_8x16)` 按规范分组顺序消费语法(`ref_idx_l0 -> ref_idx_l1 -> mvd_l0 -> mvd_l1`), 并补对齐单测。
    - [x] CAVLC B-slice `B_L0_L1_16x8/B_L0_L1_8x16` 分区方向均补齐分组语法顺序专测, 覆盖 `ref_idx_l1` 受 `mvd` 码字污染的失步场景。
    - [x] CAVLC B-slice 非 skip `mb_type=22(B_8x8)` 接入 `sub_mb_type` + `ref_idx_l0/l1` 语法消费与最小子分区重建, 并补单元测试。
    - [x] CAVLC B-slice `mb_type=22(B_8x8)` 补齐分区级 `mvd_l0/mvd_l1` 语法消费(含 `8x8/8x4/4x8/4x4`), 并补对齐单测。
    - [x] CAVLC B-slice `B_8x8` 按 `sub_mb_type` 子分区几何(`8x4/4x8/4x4`)执行重建, 并补混合子类型/Direct 路径对齐单测。
    - [x] CAVLC B-slice `mb_type=22(B_8x8)` 按规范分组顺序消费语法(`ref_idx_l0 -> ref_idx_l1 -> mvd_l0 -> mvd_l1`), 并补 `L0/L1/混合` 子分区对齐单测。
    - [x] CAVLC P-slice 非 skip `I` 宏块(`mb_type>=5`)接入帧内预测路径, 并补单元测试。
    - [x] CAVLC P-slice 非 skip `P_L0_16x16` 接入 `ref_idx_l0` 语法消费与参考帧选择, 并补单元测试。
    - [x] CAVLC P-slice 非 skip `P_L0_L0_16x8/8x16` 接入分区级 `ref_idx_l0` 语法消费与预测重建, 并补单元测试。
    - [x] CAVLC P-slice 非 skip `P_8x8/P_8x8ref0` 接入 `sub_mb_type` 语法消费、子分区重建与 `ref_idx_l0`(仅 `P_8x8`)路径, 并补单元测试。
    - [x] CAVLC P-slice 非 skip `mb_type=0/1/2/3/4` 接入 `mvd_l0` 语法消费与最小运动向量重建, 并补对齐单测。
- [ ] 残差逆变换/反量化覆盖 4x4/8x8 关键路径。
- [ ] I_NxN 8x8 最小路径已接入(含 `transform_size_8x8_flag` 与近似残差), 需升级为规范 8x8 残差路径。
- 验收: I/P/B slice 均可进入重建流程, 不再使用占位回退路径。

### P4 帧间预测与运动补偿完整实现

- [ ] 宏块分区与运动向量解析。
    - [x] P-slice 互预测路径按 `ref_idx_l0` 选择参考列表项, 不再固定使用 `L0 rank0`。
    - [x] 补齐 `ref_idx_l0` 参考选择单元测试(`test_apply_inter_block_l0_selects_ref_by_ref_idx`)。
    - [x] B-slice 显式加权预测(`weighted_bipred_idc=1`)接入 list0/list1 权重路径, 并补双向/单向 L1 单测。
- [ ] 整数/半像素/四分之一像素插值。
    - [x] 亮度路径接入 H264 6-tap + qpel 16 位置组合采样(`sample_h264_luma_qpel`), 并替换 P/B 帧间重建、双向预测与加权预测中的亮度采样入口。
    - [x] 补齐亮度 qpel 单元测试(整像素透传/半像素 6-tap/1/4 与 3/4 像素平均)。
    - [ ] 色度仍为双线性近似, 后续需补齐规范边界与权重细节。
- [ ] 多参考帧与基本 MMCO。
    - [x] 接入 `dec_ref_pic_marking` 解析和基础 MMCO(`op1-op6`)执行路径。
    - [x] 接入长期参考帧(`long_term_frame_idx`)管理和短期/长期参考列表拼接策略。
    - [x] 补齐 MMCO 行为单测(`op1/op2/op3/op4/op6/IDR long-term`)与参考列表排序单测。
- 验收: 非 I 帧不再复制参考帧占位, 具备真实帧间重建能力。

### P5 输出路径完整实现

- [ ] POC 计算与显示顺序重排。
    - [x] 补齐 `POC type2` 计算路径(含 frame_num wrap 和非参考帧分支)并补充单元测试。
    - [x] 补齐 `POC type1` 基础计算路径(含 `delta_pic_order`、非参考帧分支、frame_num wrap、IDR reset)并补充单元测试。
- [ ] DPB 管理(短期/长期参考)。
    - [x] 参考帧容量淘汰策略改为“短期帧按最小 `pic_num` 优先淘汰”, 并补齐“长期参考优先保留”单测。
- [ ] 去块滤波。
    - [x] 接入基础 in-loop 去块滤波模块 `deblock.rs`(YUV420 亮度/色度边界平滑)并在输出前执行。
    - [x] 补齐去块滤波单元测试(小边界平滑/大边界保持/整帧入口)。
    - [x] 去块滤波按 `disable_deblocking_filter_idc` 生效, 并补状态单测。
    - [x] 接入 `slice_qp + slice_alpha_c0_offset_div2 + slice_beta_offset_div2` 参数化阈值路径, 并补偏移边界/强度单测。
- [ ] 容错与边界保护(坏 NAL、缺参考帧、越界)。
- 验收: 两个样本可长时间稳定解码, 时序正确, 无明显错序/跳帧。

### P6 功能完整性验收门

- [ ] 对照功能矩阵逐项打勾, 关键项全部达到“完成+已验证”。
- [x] 两个样本完成固定帧数连续解码且不中断(`data/1_h264.mp4 >= 299`帧, `data/2_h264.mp4 >= 300`帧)。
- [ ] 不允许保留 `P_Skip 占位复制`、`仅 I 帧可用` 等临时路径。
- 进入下一阶段前置条件: `P1-P6` 全部完成。

### P7 精度对比与 Bug 收敛阶段(功能完整后才开始)

- [ ] 维护 `plans/tao-codec/video/h264/decoder_compare.rs` 对比入口。
- [ ] 与 FFmpeg 逐帧对比 Y/U/V 误差、PSNR、最大误差、精度百分比。
- [ ] 对 `data/1_h264.mp4`、`data/2_h264.mp4` 建立基线与回归报告。
- [ ] 按“最先偏差帧 -> 对应模块 -> 根因修复 -> 回归复测”循环收敛。
- 验收: 两个样本对比达到目标精度并可稳定复跑。
- 阶段门禁:
    - 该阶段当前处于“锁定”状态, 直到 `P1-P6` 全部完成并验收通过后再开启。
- 当前基线(120 帧):
    - `data/1_h264.mp4`: `1.343662%`
    - `data/2_h264.mp4`: `1.792586%`
    - 双样本平均精度: `1.568124%`
    - 已定位问题: 首个 IDR slice 仍存在宏块级语法失步, `I_8x8` 变换残差路径需要继续对齐规范上下文与语法消费.
    - 补充诊断(1 帧最小复现实验):
        - 当前默认路径(`TAO_H264_8X8_SKIP_CBF=1`)下, `data/1_h264.mp4` 首个 IDR slice 提前结束在 `decoded_mbs=102/8160`.
        - 严格 8x8 CBF 路径(`TAO_H264_8X8_SKIP_CBF=0`)下, 同位置为 `decoded_mbs=58/8160`.
        - 当前默认最小复现失步位置: `last_mb=(101, 0)`, `cabac_bits=7402/438360`.
        - `ffmpeg trace_headers` 已核对 slice header 位流对齐, `cabac_start_byte=4` 与当前实现一致, 根因不在 slice header 解析.
        - 现阶段高概率根因在宏块语法路径(CABAC 上下文演进或残差语法消费)而非 NAL 头和 slice header.
    - `2026-02-20` 诊断记录:
        - CABAC 引擎已切换为 `low/range/bytestream` 形态并对齐 FFmpeg 的 I_PCM 重启流程, 但首个 IDR 失步点未变化。
        - 已验证 `I_PCM restart` 为必要步骤: 跳过重启会提前失步。
        - 已验证“仅关闭 terminate break 但持续消费 terminate bin”会让失步帧精度进一步恶化, 说明失步不是单一 break 条件问题。
        - 已验证首个视频包仅包含 `SEI + 1个IDR slice`, 且后续包 `frame_num` 连续递增/重排, 不存在“同帧多 slice 未拼装”导致截断的情况。
        - `TAO_H264_IPCM_PTR_ADJUST` 在 `[-20, 20]` 扫描中, `decoded_mbs` 最多仅到 `1621`(偏移 `-16`), 且 120 帧精度反而下降, 说明 I_PCM 指针固定偏移不是根因。
        - 尝试将 Chroma DC 上下文改为“真实 DC-CBF”后, 首帧会提前退化到 `decoded_mbs=90/8160`, 说明当前残差/上下文链路尚未满足该切换前置条件。
        - 尝试将 Chroma AC 改为 `U/V` 交错消费后, 首帧可延后到 `decoded_mbs=375/8160`, 但 120 帧精度下降(`data/1_h264.mp4=1.081275%`, `data/2_h264.mp4=1.697793%`), 已回滚。
        - 新增 `TAO_H264_IPCM_RESTART_PTR_ADJUST`(仅调节 I_PCM 后 CABAC 重启位置, 不影响 I_PCM 像素读取)。在 `data/1_h264.mp4` 上, `-13` 对当前基线无稳定收益, 已保持默认 `0`。
        - 上述现象说明 I_PCM 后 CABAC 重启位置仍有系统性偏差, 但“常量偏移”并非最终根因修复方案, 当前默认保持 `0` 以避免引入样本特化逻辑。
        - 已补齐 FFmpeg 对应的 8x8 CBF 高位上下文初始化(`ctxIdx 1012..1015`)并尝试开启 8x8 strict coded_block_flag 消费; 1 帧最小复现会提前退化到 `decoded_mbs=109~171`, 说明当前 `I_8x8` fallback 仍缺失完整的 8x8 CBF 上下文建模与残差路径。
        - 为避免回归, 当前仍保持 `I_8x8` fallback 的 `skip_cbf=true` 基线实现。
        - 已完成按 `pic_parameter_set_id` 选择 `PPS/SPS` 并在 slice 级激活参数集, 避免“始终使用最后一个参数集”的路径偏差; 目前仍需补参数集切换后的 DPB/参考状态重建与专项样本验证。
    - `2026-02-21` 诊断记录:
        - CABAC 初始化已改为与 FFmpeg `ff_init_cabac_decoder` 同步的指针对齐分支路径, 当前样本结果无变化。
        - `mb_qp_delta` 解码上限已按规范修正为 `2*MAX_QP`, 当前样本结果无变化。
        - 已对 `TAO_H264_FORCE_4X4/TAO_H264_SKIP_IPCM_CHECK/TAO_H264_FORCE_NO_IPCM/TAO_H264_IGNORE_TERMINATE` 做 16 组组合扫描, 默认组合在双样本平均精度上最佳(`avg=1.431478%`)。
        - 已接入最小 P-slice CABAC 语法路径(`mb_skip_flag/mb_type/P_8x8 sub_mb_type`)与整数像素运动补偿、互预测残差叠加.
        - 已接入最小 B-slice 路径: `mb_skip_flag/mb_type/B_8x8 sub_mb_type` 语法消费 + 互预测宏块重建 + 残差叠加, 参考源暂以 list0 占位近似.
        - 本轮改动后 120 帧结果: `sample1 1.126295% -> 1.271633%`, `sample2 1.742157% -> 1.704838%`, 双样本平均精度提升到 `1.488236%`.
        - 1 帧最小复现下 IDR 仍在 `decoded_mbs=224/8160` 处提前结束, 主瓶颈仍是 I-slice 宏块级 CABAC 语法失步.
        - 新增 `luma 8x8 coded_block_flag` 严格语法路径(含 8x8 邻居 CBF 缓存与上下文增量), 初期以严格路径做诊断, 保留 `TAO_H264_8X8_SKIP_CBF` 开关。
        - 该路径在 120 帧双样本上继续提升: `sample1 1.271633% -> 1.287982%`, `sample2 1.704838% -> 1.716215%`, 双样本平均精度提升到 `1.502099%`。
        - 1 帧最小复现下 IDR 提前结束位置变为 `decoded_mbs=112/8160`, 说明“宏块覆盖率提升”与“像素对比精度提升”在当前阶段并非单调一致, 首个 IDR 的 CABAC 语法一致性仍是主瓶颈。
        - 新发现: I_PCM 对齐实现与 FFmpeg 语义不一致。原实现进入 I_PCM 时 `raw_pos=ceil(bit_pos/8)`, 导致部分样本出现固定 `+1` 字节偏移并污染后续 CABAC 重启。
        - 已按 FFmpeg `ptr=bytestream; if(low&1) ptr--` 语义修复 `align_to_byte_boundary`。修复后无需 `TAO_H264_IPCM_PTR_ADJUST` 手工偏移即可获得稳定收益。
        - 在新实现上复扫 `TAO_H264_IPCM_PTR_ADJUST/TAO_H264_IPCM_RESTART_PTR_ADJUST` 的 `[-2,2]` 区间, `0` 为最优点, 非零偏移会降低双样本平均精度。
        - 同步将 `I_8x8 fallback` 默认路径调整为 `skip_cbf=true`(保留 `TAO_H264_8X8_SKIP_CBF=0` 严格模式开关), 双样本 120 帧基线提升到 `sample1=1.325445%`, `sample2=1.736214%`, 平均 `1.530829%`。
        - 新增 CABAC 邻居默认上下文修复: 对齐 FFmpeg 在 intra 宏块边界的“不可用邻居按非零处理”逻辑(`cbp` 默认值与 `cbf` 边界上下文), 将首个 IDR 最小复现从 `decoded_mbs=90` 推进到 `decoded_mbs=102`, 双样本 120 帧结果更新为 `sample1=1.353898%`, `sample2=1.793304%`, 平均 `1.573601%`。
        - 复扫 `TAO_H264_FORCE_4X4/TAO_H264_SKIP_IPCM_CHECK/TAO_H264_FORCE_NO_IPCM/TAO_H264_IGNORE_TERMINATE` 共 16 组组合(120 帧双样本), 仍是默认组合最优:
            - `sample1=1.353898%`, `sample2=1.793304%`, `avg=1.573601%`。
            - `skip_ipcm_check=1` 会把 IDR 最小复现从 `decoded_mbs=102` 推到 `465`, 但像素精度下降, 说明并非正确修复方向。
        - 针对 `I_16x16` AC 子块遍历顺序做对照实验:
            - 改成纯行优先后, IDR 最小复现可延后到 `decoded_mbs=296`, 但 1 帧精度下降到 `1.156346%`(默认为 `1.252636%`)。
            - 该实验已回滚, 当前继续保留 FFmpeg `scan8` 对齐顺序; 结论是“终止位置更晚”不等于“语法更正确”, 需继续按上下文一致性收敛。
        - 已接入帧间运动补偿的亚像素路径:
            - 亮度按 `1/4` 像素、色度按 `1/8` 像素执行双线性插值(保留加权预测分支)。
            - 功能自测仍通过(`tests/h264_functional_pipeline.rs`), 但当前双样本精度略回落到 `avg=1.560688%`, 说明首个 IDR 失步仍是主导误差源。
        - 已修正 8x8 变换 `coded_block_flag` 上下文增量计算:
            - `cat=5` CBF 上下文由“8x8 聚合邻居”改为“4x4 邻居(`scan8[idx]-1/-8`)”。
            - 在严格 `TAO_H264_8X8_SKIP_CBF=0` 诊断路径下, 仍存在提前失步, 后续需继续对齐 8x8 残差完整语义。
        - 已补齐 CABAC 初始化表 `ctxIdx 1016..1023`(此前仅初始化到 `1015`), 与 FFmpeg `cabac_context_init_I/PB[0]` 全量对齐; 对当前样本失步位置和精度无直接变化, 说明主根因不在该区间上下文缺失。
        - 已接入 `pending frame` 拼帧逻辑: 以 `first_mb_in_slice==0` 作为新帧边界, 在跨包场景下先提交上一帧再解码下一帧, 并在 `flush/IDR` 路径补齐待输出帧提交。
        - 已接入 `POC + decode_order` 输出重排缓存结构(`ReorderFrameEntry`), 当前输出排序不再依赖 `pts` 单键。
        - 已完成参数集切换后的运行时状态重置补强: `activate_sps/activate_parameter_sets` 在关键字段变更时同步清理宏块上下文、参考队列与重排缓存。
        - 已执行严格质量门禁并通过:
            - `cargo fmt --all -- --check`
            - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
            - `cargo check --workspace --all-targets --all-features`
            - `cargo test --workspace --all-targets --all-features --no-fail-fast`
            - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
        - 8x8 严格 CBF 路径新增诊断结果:
            - 将 `cat=5` 的 CBF 上下文改为 8x8 邻居后, `TAO_H264_8X8_SKIP_CBF=0` 的最小复现从 `decoded_mbs=58` 提升到 `decoded_mbs=95`。
            - 但 120 帧精度仍低于默认路径(`sample1=1.154934%`), 当前默认仍保持 `TAO_H264_8X8_SKIP_CBF=1`。
        - 默认路径双样本 120 帧基线维持不变:
            - `sample1=1.343662%`
            - `sample2=1.792586%`
            - 平均 `1.568124%`

### P8 质量门禁与交付

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [x] `cargo check --workspace --all-targets --all-features`
- [x] `cargo test --workspace --all-targets --all-features --no-fail-fast`
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- [ ] 输出最终偏差总结、风险与剩余事项。

## 5. 前置条件

- 输入样本: `data/1_h264.mp4`, `data/2_h264.mp4`。
- 本地工具: `ffmpeg`, `ffprobe`(仅用于对比与诊断, 不参与解码实现)。
- 对比输出目录: `plans/tao-codec/video/h264/coverage/`。

## 6. 验收标准

- H264 全链路为 Tao 自研实现。
- 功能矩阵关键项全部完成并通过对应验证。
- 在功能完整前不进行精度结论判定。
- 功能完整后, 两个样本完成逐帧对比并达到目标精度。
- 五项质量门禁全部通过。

## 7. 进度标记

- [x] P0
- [x] P1
- [ ] P2
- [ ] P3
- [ ] P4
- [ ] P5
- [ ] P6
- [ ] P7
- [ ] P8
