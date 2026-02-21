# H264 解码器 -- 功能开发计划

> 关联文档:
> - 精度收敛: `decoder_accuracy.md`
> - 性能优化: `decoder_perf.md`
> - 诊断日志: `diagnosis_log.md`
> - 功能矩阵: `h264_feature_matrix.md`

## 1. 总则

- 纯自研, 不依赖外部多媒体能力库.
- 对标 FFmpeg `libavcodec/h264*`, 辅参 OpenH264/JM.
- 目标 Profile: Constrained Baseline / Main / High (4:2:0 + 8-bit).
- 本文件仅覆盖 **功能实现**(P0-P6). 精度收敛见 `decoder_accuracy.md`, 性能优化见 `decoder_perf.md`.

## 2. 模块结构

```text
crates/tao-codec/src/decoders/h264/
├── mod.rs               # 状态机与对外接口
├── cabac.rs             # CABAC 引擎
├── cabac_init_ext.rs    # CABAC 扩展初始化表(ctxIdx 460-1011)
├── cabac_init_pb.rs     # CABAC P/B-slice 初始化表
├── cavlc.rs             # CAVLC 残差系数解码(待新建)
├── common.rs            # 通用工具(Exp-Golomb/QP/采样)
├── config.rs            # avcC 配置解析
├── deblock.rs           # 去块滤波
├── direct.rs            # B-slice Direct 模式推导(待新建)
├── intra.rs             # 帧内预测
├── macroblock_inter.rs  # 帧间宏块处理
├── macroblock_intra.rs  # 帧内宏块处理
├── macroblock_state.rs  # 宏块状态管理
├── mv_pred.rs           # 运动向量预测(待新建)
├── output.rs            # DPB/POC/参考帧/输出重排
├── parameter_sets.rs    # SPS/PPS 解析
├── residual.rs          # 残差解码/反量化/IDCT
├── sei.rs               # SEI 消息解析(待新建)
├── slice_decode.rs      # Slice 级解码
├── syntax.rs            # CABAC 语法元素工具
└── tests.rs             # 单元测试
```

## 3. 提交规则(强制)

每完成一个关键变更必须按顺序执行:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo check --workspace --all-targets --all-features`
4. `cargo test --workspace --all-targets --all-features --no-fail-fast`
5. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`

五项通过后立即提交, 中文提交信息, 禁止堆积.

## 4. 里程碑

### P0 基线与计划

- [x] 明确三阶段策略(功能 -> 精度 -> 优化).
- [x] 输出可断点续跑计划.

### P1 功能矩阵

- [x] 新建 `h264_feature_matrix.md`.
- [x] 按 FFmpeg 语义梳理完整功能点并定义完成判据.

### P2 输入链路与参数集

- [x] AnnexB/AVCC 双入口统一.
- [ ] SPS/PPS 完整解析.
    - [x] PPS 解析迁移 + 合法性校验 + 单测.
    - [x] SPS 基础/深度校验 + 单测.
    - [x] SPS 能力门禁(4:2:0 + 8-bit + frame_mbs_only).
    - [x] `handle_sps` 缓存前能力校验 + 单测.
    - [x] avcC 全链路校验(版本/保留位/截断/NAL类型) + 单测.
    - [ ] SPS 自定义量化矩阵解析(`scaling_list` 4x4/8x8) + 单测.
    - [ ] PPS 自定义量化矩阵解析(`scaling_list` 4x4/8x8) + 单测.
- [ ] 参数集变更重建.
    - [x] PPS 变更分级策略(None/RuntimeOnly/Full) + 单测.
    - [x] 参数集一致性保护 + 切换集成测试.
    - [x] SPS 尺寸变更集成测试.
- [ ] SEI 消息基础解析.
    - [ ] 新建 `sei.rs`, 分帧与类型分发.
    - [ ] `recovery_point` / `pic_timing` / `buffering_period` / `user_data_unregistered` 解析.
    - [ ] 未识别 SEI 安全跳过 + `debug!` 日志.
    - [ ] SEI 单测(各类型成功/截断失败).
- 验收: 样本稳定进入 slice 解码, 参数切换无崩溃.

### P3 Slice 语法与熵解码

#### P3.1 Slice Header

- [x] 边界校验(cabac_init_idc/slice_qp/deblocking_idc/ref_idx) + 失败单测.
- [x] poc_type1 delta 解析 + deblocking_idc 透传.
- [x] ref_pic_list_modification 解析(list0/list1) + 边界校验 + 单测.
- [x] pred_weight_table 解析(含 B-slice list1) + 单测.
- [x] direct_spatial_mv_pred_flag + redundant_pic_cnt 解析 + 单测.
- [x] `redundant_pic_cnt > 0` 时跳过冗余 slice + 单测.

#### P3.2 CABAC 完整路径

- [ ] I-slice 完整语法.
    - [ ] `I_4x4`: prev/rem pred_mode 全 16 块, 对齐 scan8 顺序.
    - [ ] `I_8x8`: transform_size_8x8_flag + prev/rem pred_mode 全 4 块.
    - [ ] `I_8x8` 完整 coded_block_flag 上下文(cat=5, 4x4 邻居), 移除 `TAO_H264_8X8_SKIP_CBF`.
    - [ ] `I_8x8` 完整 8x8 残差(significant/last/coeff_abs 全 64 系数, 8x8 扫描表).
    - [ ] `I_16x16`: DC(cat=0, Hadamard) + AC(cat=1, 15 系数), 对齐子块遍历顺序.
    - [ ] `I_PCM`: 字节对齐 + 像素读取 + CABAC 重启.
    - [ ] coded_block_pattern: luma(4b) + chroma(2b) + 邻居上下文.
    - [ ] mb_qp_delta: 一元编码(上限 2*QP_MAX) + prev 上下文.
    - [ ] intra_chroma_pred_mode: 截断一元(0-3) + 邻居上下文.
    - [ ] I-slice 全类型 CABAC 单测.
- [ ] P-slice 完整语法.
    - [ ] mb_skip_flag(左/上邻居上下文).
    - [ ] 宏块类型(P_L0_16x16/16x8/8x16/P_8x8/P_8x8ref0 + I).
    - [ ] P_Skip MV 推导(邻居中值预测).
    - [ ] ref_idx_l0(截断一元 + 邻居上下文).
    - [ ] mvd_l0(x/y 独立 + 邻居 MVD abs 上下文).
    - [ ] P_8x8 子类型(P_L0_8x8/8x4/4x8/4x4).
    - [ ] P-slice I 宏块帧内预测路径.
    - [ ] P-slice 帧间残差(CBP + 4x4/8x8 残差).
    - [ ] P-slice 全类型 CABAC 单测.
- [ ] B-slice 完整语法.
    - [ ] mb_skip_flag(B-slice 上下文偏移).
    - [ ] 宏块类型(全部 23 种 B 类型 + I).
    - [ ] B_Skip/B_Direct_16x16 推导(Spatial/Temporal Direct).
    - [ ] ref_idx_l0/l1(双列表).
    - [ ] mvd_l0/l1(双列表).
    - [ ] B_8x8 子类型(全部 13 种含 B_Direct_8x8).
    - [ ] B-slice 帧间残差(CBP + 残差).
    - [ ] B-slice I 宏块路径.
    - [ ] B-slice 全类型 CABAC 单测.
- [ ] end_of_slice_flag 终止逻辑对齐.

#### P3.3 CAVLC 完整路径

- [x] 宏块级语法(mb_skip_run/mb_type/sub_mb_type/ref_idx/mvd).
- [x] P-slice 全类型 + B-slice 全类型.
- [ ] 残差系数解码.
    - [ ] coeff_token(nC 上下文 + VLC 查表, 含 chroma DC 专用表).
    - [ ] trailing_ones_sign_flag.
    - [ ] level 系数码(前缀/后缀 + suffixLength 自适应).
    - [ ] total_zeros(VLC 查表, 含 chroma DC 专用表).
    - [ ] run_before(VLC 查表).
    - [ ] 反扫描重建(Zig-Zag 4x4/8x8).
    - [ ] 反量化 + IDCT 接入.
    - [ ] 残差系数解码单测(DC/AC/8x8).
- [ ] CAVLC coded_block_pattern(Intra/Inter 两套 VLC 映射).
- [ ] CAVLC mb_qp_delta(有符号 Exp-Golomb).
- [ ] CAVLC intra_chroma_pred_mode(无符号 Exp-Golomb).
- [ ] CAVLC transform_size_8x8_flag(单 bit).
- 验收: CABAC/CAVLC 双模式下 I/P/B 均完整重建, 无占位回退.

#### P3.4 残差逆变换/反量化

- [x] 4x4 IDCT + 反量化.
- [x] 8x8 IDCT + 反量化.
- [x] 4x4 Hadamard(Intra 16x16 DC).
- [x] 2x2 Hadamard(Chroma DC).
- [ ] 自定义量化矩阵接入(SPS/PPS scaling_list 替代 flat 矩阵) + 单测.
- [ ] qpprime_y_zero_transform_bypass(QP=0 跳过变换, 低优先级).

### P4 帧内预测

#### P4.1 Intra 4x4(9 种模式)

- [x] 模式 0-2(V/H/DC 含不可用变体).
- [x] 模式 3: Diagonal Down-Left + 单测.
- [x] 模式 4: Diagonal Down-Right + 单测.
- [x] 模式 5: Vertical-Right + 单测.
- [x] 模式 6: Horizontal-Down + 单测.
- [x] 模式 7: Vertical-Left + 单测.
- [x] 模式 8: Horizontal-Up + 单测.
- [x] DC 变体(Left-DC/Top-DC/DC-128) 边界单测.

#### P4.2 Intra 8x8(9 种模式, High Profile)

- [ ] 低通滤波边界像素(规范 8.3.2.2.2).
- [ ] 模式 0-8(在 8x8 块上操作) + DC 变体.
- [ ] Intra 8x8 全模式单测.

#### P4.3 Intra 16x16(4 种模式)

- [x] 模式 0-3(V/H/DC/Plane).
- [x] 邻居不可用变体确认对齐.

#### P4.4 色度帧内预测(4 种模式)

- [x] 模式 0: DC(含不可用变体).
- [x] 模式 1: Horizontal.
- [x] 模式 2: Vertical.
- [x] 模式 3: Plane.
- [x] 色度全模式单测.

- 验收: I-slice 全部预测模式正确重建, 无硬编码回退.

### P5 帧间预测与运动补偿

#### P5.1 运动向量预测

- [ ] 新建 `mv_pred.rs`.
- [ ] 中值预测(A/B/C, D 替补), 对齐 FFmpeg `pred_motion()`.
- [ ] 16x8/8x16 特殊 MVP(规范 8.4.1.3 directional).
- [ ] P_Skip MV 推导(邻居中值, 非简单复制).
- [ ] mvd + mvp 合成完整 MV(逐分区/子分区).
- [ ] MV 预测单测(16x16/16x8/8x16/8x8/P_Skip).

#### P5.2 B-slice Direct 模式

- [ ] 新建 `direct.rs`.
- [ ] Spatial Direct:
    - [ ] 邻居 MV/ref_idx 收集.
    - [ ] 零 MV 条件判定.
    - [ ] L0/L1 方向独立推导.
    - [ ] 16x16/8x8 粒度(direct_8x8_inference_flag).
    - [ ] Spatial Direct 单测.
- [ ] Temporal Direct:
    - [ ] 共定位 MV 缩放(td/tb).
    - [ ] 共定位宏块定位.
    - [ ] L0/L1 分别缩放.
    - [ ] Temporal Direct 单测.
- [ ] B_Skip/B_Direct_16x16/B_Direct_8x8 接入.

#### P5.3 加权预测

- [x] 显式加权(weighted_pred_flag=1, weighted_bipred_idc=1).
- [x] 隐式加权(weighted_bipred_idc=2): POC 距离推导, 对齐 FFmpeg `implicit_weight()` + 单测.
- [x] 默认加权(无标志): `(L0 + L1 + 1) >> 1`.

#### P5.4 运动补偿插值

- [x] 亮度 6-tap + qpel.
- [x] 色度 1/8 双线性.
- [x] 边界扩展(padding): 参考块越界时边界复制 + 单测.
- [ ] 双向融合舍入对齐.

#### P5.5 多参考帧与 MMCO

- [x] MMCO op1-op6 + 长期参考管理 + 单测.
- [x] MMCO op5(清除全部 + frame_num 重置 + POC 重置).
- [ ] 滑动窗口(超 max_num_ref_frames 自动淘汰, 规范 8.2.5.3).
- [ ] 帧号间隙(gaps_in_frame_num_allowed_flag, 插入"不存在"参考帧).
- [ ] op5 + 滑动窗口 + 帧号间隙单测.

- 验收: 帧间完整重建, Direct/隐式加权/Skip MV 对齐规范, 无占位近似.

### P6 输出与后处理

#### P6.1 POC 与重排

- [x] POC type0/1/2 计算.
- [x] reorder_depth 自适应.
- [ ] 输出重排完整语义(DPB 满/flush 按 POC 升序) + 单测.

#### P6.2 DPB 管理

- [x] 短期/长期淘汰策略.
- [ ] Level 限制 max_dpb_frames(规范 A.3.1) + 单测.
- [ ] max_num_reorder_frames 约束 + 单测.

#### P6.3 去块滤波

- [x] 基础 in-loop + idc 开关 + 参数化阈值 + tc0 约束 + 宏块边界 BS.
- [ ] 4x4 子块内部边界 BS:
    - [ ] cbf!=0 -> bs=2; 不同 ref -> bs=1; MV 差>=4 -> bs=1.
    - [ ] 对齐 FFmpeg 4x4 级 BS.
- [ ] 强滤波(bs=4): 亮度 4 像素 + 色度 2 像素, 对齐规范公式.
- [ ] idc=2: 不跨 slice 边界(first_mb 边界检测).
- [ ] B-slice 双列表 BS(ref_idx_l0/l1 + mv_l0/l1 完整比较).
- [ ] 色度 QP 映射(chroma_qp_from_luma_with_offset).
- [ ] 4x4 内部 BS / 强滤波 / idc=2 单测.

#### P6.4 容错

- [x] MMCO 上限/参数范围 + 缺参考帧回退 + 坏 NAL 拒绝 + slice 边界/first_mb 容错.
- [ ] 宏块解码异常恢复(标记错误 + 跳过后续宏块) + 单测.
- [ ] 帧级错误隐藏(参考帧像素填充缺失区域) + 单测.
- [ ] recovery_point SEI 处理(非 IDR 随机访问点标记).

#### P6.5 多 Slice 拼装

- [x] first_mb==0 新帧边界 + pending frame 拼帧.
- [ ] 同帧多 slice 按 first_mb 偏移拼合 + 单测.

#### P6.6 功能验收门

- [ ] 功能矩阵关键项全部"完成+已验证".
- [x] 双样本连续解码(sample1>=299, sample2>=300).
- [ ] 移除全部 `TAO_H264_*` 临时开关.
- [ ] 移除全部占位/回退路径(P_Skip 复制等).
- [ ] 补充多样本覆盖(CABAC High/CAVLC Baseline/B密集/1080p+), 更新 `samples/SAMPLE_URLS.md`.
- 前置条件: P2-P6.5 全部完成.

## 5. 功能覆盖与非目标

### 目标范围

| 功能 | 说明 |
| --- | --- |
| Profile | Constrained Baseline / Main / High |
| 色度 | 4:2:0 + 8-bit |
| Slice | I/P/B 含 Skip/Direct |
| 熵编码 | CABAC + CAVLC 完整 |
| 帧内 | 4x4(9) + 8x8(9) + 16x16(4) + 色度(4) |
| 帧间 | 全分区 + Direct(Spatial+Temporal) + 隐式/显式加权 |
| 变换 | 4x4/8x8 IDCT + Hadamard + 自定义量化矩阵 |
| 去块 | 完整 BS(4x4 级) + alpha/beta/tc0 |
| DPB | type0/1/2 + MMCO(含 op5) + 滑动窗口 |
| SEI | recovery_point / pic_timing / buffering_period |

### 非目标(与 FFmpeg 一致)

FMO / ASO / SI-SP slice / Data Partitioning / SVC / MVC -- 均不实现.

### 后续扩展

High 10 / High 4:2:2 / High 4:4:4 / Monochrome / MBAFF / PAFF -- 当前不实现, 未来规划.

## 6. 进度

- [x] P0 -- 基线与计划
- [x] P1 -- 功能矩阵
- [ ] P2 -- 输入链路与参数集
- [ ] P3 -- Slice 语法与熵解码
- [ ] P4 -- 帧内预测
- [ ] P5 -- 帧间预测与运动补偿
- [ ] P6 -- 输出与后处理 + 功能验收
