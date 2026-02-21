# H264 功能点矩阵

## 1. 说明

- 本矩阵用于驱动 `decoder_dev.md` 的 P1-P6 阶段。
- 强制门禁: `P1-P6` 任一项未完成时, 禁止执行精度收敛阶段(`P7`)相关对比结论。
- 状态定义:
    - `未实现`: 代码路径不存在或不可用。
    - `部分`: 已有代码, 但行为不完整或仅覆盖子集。
    - `完成`: 功能逻辑已实现。
    - `已验证`: 完成且有对应自测/集成测试覆盖。

## 2. 功能矩阵

| 分类 | 功能点 | 状态 | 代码位置 | 最小验证方式 |
| --- | --- | --- | --- | --- |
| NAL/输入 | AVCC NAL 拆分 | 部分 | `crates/tao-codec/src/parsers/h264/nal.rs` | 单元测试 + 样本解码 |
| NAL/输入 | AnnexB NAL 拆分 | 部分 | `crates/tao-codec/src/parsers/h264/nal.rs` | 单元测试 + 样本解码 |
| NAL/输入 | AVCC/AnnexB 双入口自动识别 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | `tests/h264_functional_pipeline.rs` |
| 参数集 | SPS 解析 | 部分 | `crates/tao-codec/src/parsers/h264/sps.rs` | SPS 单元测试(含基础 + 深度失败用例) |
| 参数集 | PPS 解析 | 部分 | `crates/tao-codec/src/decoders/h264/parameter_sets.rs` | PPS 单元测试 + 样本读取无崩溃 |
| 参数集 | PPS 扩展(transform_8x8_mode/scaling list) | 部分 | `crates/tao-codec/src/decoders/h264/parameter_sets.rs` | PPS 扩展单元测试 + 样本自测 |
| 参数集 | 参数集变更重建上下文 | 已验证 | `crates/tao-codec/src/decoders/h264/mod.rs` | 单元测试 + `handle_pps/handle_sps` 切换集成单测 |
| Slice | I/P/B slice header 关键字段 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | `test_h264_functional_picture_type_stats_sample1` + `parse_slice_header` 边界/状态单测 + `ref_pic_list_modification` 单测 |
| Slice | 多 slice 同帧拼装 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | `test_h264_functional_sample*_frames` |
| 熵解码 | CABAC 基础引擎 | 部分 | `crates/tao-codec/src/decoders/h264/cabac.rs` | CABAC 单元测试 |
| 熵解码 | CABAC I-slice 路径 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | I 帧样本自测 |
| 熵解码 | CABAC P/B-slice 路径 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | P/B 样本自测 |
| 熵解码 | CAVLC 路径 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | CAVLC 最小路径单测 + 样本自测 |
| 帧内重建 | I_NxN(4x4/8x8) 预测 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | I 帧像素级对比 |
| 帧内重建 | I_16x16 预测 + 残差 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | I 帧像素级对比 |
| 帧间重建 | P 帧运动向量解析 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | P 帧功能自测 |
| 帧间重建 | B 帧运动向量解析 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | B 帧功能自测 |
| 帧间重建 | 运动补偿(整数/半像素/四分之一像素) | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | 运动样本自测 + qpel 单元测试 |
| 输出 | DPB 管理(短期/长期) | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | MMCO/长期参考帧单测 + GOP 样本自测 |
| 输出 | POC 计算与显示重排 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | POC type1/type2 单测 + B 帧顺序自测 |
| 后处理 | 去块滤波 | 部分 | `crates/tao-codec/src/decoders/h264/{deblock.rs,mod.rs}` | 去块单元测试 + `disable_deblocking_filter_idc` 状态单测 + 样本功能自测 |
| 稳定性 | 损坏数据容错与边界保护 | 部分 | `crates/tao-codec/src/decoders/h264/mod.rs` | 异常包测试 |
| 验收 | 两个样本连续帧无中断 | 部分 | `tests/h264_functional_pipeline.rs` | `sample1>=299`, `sample2>=300` |

## 3. 当前阻塞点

1. P/B 帧已接入最小 CABAC + 运动补偿 + 残差路径, 但仍是近似实现:
   - `P_L0_16x16/16x8/8x16/P_8x8` 已接入最小语法消费与整数像素补偿.
   - `B-slice` 已接入 `mb_skip_flag/mb_type/B_8x8 sub_mb_type` 与基础残差路径, 但 `list1/双向` 仍是近似占位重建.
   - `ref_pic_list_modification(list0/list1)` 已接入解析与重排, 且 `P-slice` 已按 `ref_idx_l0` 选择 `L0` 参考帧; `B-slice` 的 `list1/双向` 仍待完整收敛.
   - 亮度运动补偿已接入 H264 `6-tap + qpel` 采样, 色度仍为双线性近似, 尚需补齐规范边界/权重细节。
   - DPB/POC 已接入基础路径, 但完整语义仍缺失(见阻塞点 3)。
2. CABAC 仍未完整对齐规范(特别是 I_NxN 8x8 残差与部分上下文增量), IDR 首个 slice 仍会在中途提前结束, 造成帧内重建不完整。
   - 当前最小复现: `data/1_h264.mp4` 在首个 IDR slice 出现 `decoded_mbs=102/8160` 提前结束(默认 `TAO_H264_8X8_SKIP_CBF=1`)。
   - 严格 8x8 CBF 路径(`TAO_H264_8X8_SKIP_CBF=0`)下同位置为 `decoded_mbs=58/8160`。
   - `ffmpeg trace_headers` 已确认 slice header 与 CABAC 起始字节对齐正确, 问题集中在宏块层 CABAC 语法消费。
   - `2026-02-20` 已验证 CABAC `low/range/bytestream` 形态与 I_PCM 重启路径改造后, 失步点仍在首个 IDR slice 中段, 说明根因仍在宏块语法消费顺序/上下文演进。
   - 包级排查已确认首包仅 `SEI + 1个IDR slice`, 不存在“同帧多 slice 未拼装”导致的截断。
   - `TAO_H264_IPCM_PTR_ADJUST` 扫描 `[-20,20]` 未发现可稳定提升到完整宏块的偏移, I_PCM 固定偏移假设已排除。
   - I_PCM 对齐已按 FFmpeg `ptr=bytestream; if(low&1) ptr--` 修复, 去除原有 `ceil(bit_pos/8)` 的系统性 `+1` 偏移。
   - Chroma DC/AC 语法顺序与上下文切换的多组实验会进一步提前失步(90~448 宏块), 表明当前问题属于更上层的上下文模型一致性缺口。
   - 已补齐 8x8 CBF 高位上下文初始化(`ctxIdx 1012..1023`)并接入严格 `coded_block_flag` + 8x8 邻居上下文缓存, 当前默认切回 FFmpeg 4:2:0 对齐的 `skip_cbf=true` 路径, 严格模式保留开关用于诊断。
   - `2026-02-21` 进一步核对 CABAC 初始化表后, `I/PB0` 与 FFmpeg 全量 `1024` 上下文参数已一致; 当前样本失步位置无变化, 说明根因不在初始化表残缺。
   - 已对齐 FFmpeg 的 intra 边界上下文默认值(不可用邻居按非零处理, `cbp/cbf` 上下文修复), IDR 最小复现从 `decoded_mbs=90` 提升到 `decoded_mbs=102`。
   - 当前 120 帧基线: `sample1=1.343662%`, `sample2=1.792586%`, 平均 `1.568124%`; 但 IDR 提前结束问题仍未根除。
   - 已接入按 `pic_parameter_set_id` 激活 `PPS/SPS` 的参数集选择路径, 并补齐参数集变更重建策略与 `handle_pps` 切换集成单测。
   - `2026-02-21` 复扫 16 组诊断组合(`FORCE_4X4/SKIP_IPCM_CHECK/FORCE_NO_IPCM/IGNORE_TERMINATE`), 默认组合仍是双样本精度最优(`avg=1.573601%`)。
   - `skip_ipcm_check=1` 虽可把首个 IDR 的 `decoded_mbs` 推到 `465`, 但精度下降, 已判定为错误收敛方向。
   - `I_16x16` AC 子块遍历顺序改为行优先会把 `decoded_mbs` 推到 `296`, 但 1 帧精度下降(`1.252636% -> 1.156346%`), 已回滚。
   - 已将 `cat=5`(8x8 变换)的 CBF 上下文增量改为 4x4 邻居(`scan8[idx]-1/-8`)语义; 在严格 `TAO_H264_8X8_SKIP_CBF=0` 诊断路径下最小复现仍提前到 `decoded_mbs=58`, 说明 8x8 严格语义仍不完整。
3. DPB/POC/重排已接入基础路径, 但仍不完整:
   - 已接入 `POC + decode_order` 输出重排缓存, 并补齐 `POC type1/type2` 基础计算路径。
   - 已接入跨包 `pending frame` 拼帧提交逻辑。
   - 已接入 `dec_ref_pic_marking` 基础 MMCO(`op1-op6`)和长期参考帧管理, 并补齐单元测试。
   - 仍缺 `POC type1` 字段图像完整语义与完整 MMCO 语义(字段图像/完整 picNum 语义), B 帧路径仍存在结构性偏差。
4. CAVLC 已接入最小路径(`mb_skip_run/mb_type` 消费 + 基础重建 + 宏块状态回填), 但残差/块级语法和规范级重建仍未完成。
5. 去块滤波已接入基础平滑实现并完成单测, 但仍未对齐 H264 规范的 alpha/beta/tc0 约束和宏块边界强弱判定。

## 4. 下一步执行顺序

1. 完成 P/B slice 语法解析与最小可用宏块重建。
2. 补齐色度运动补偿与边界语义, 将当前近似路径收敛到规范行为。
3. 补齐 DPB 长期参考/MMCO 与完整重排策略, 再扩展 B 帧路径。
4. 最后补齐去块滤波规范 alpha/beta/tc0 与容错边界路径。
