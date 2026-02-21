# H264 解码器诊断日志

本文件归档 `decoder_dev.md` 开发过程中的详细诊断实验记录, 供后续定位问题时参考.

## 当前基线(120 帧, 2026-02-21)

- `data/1_h264.mp4`: `1.343662%`
- `data/2_h264.mp4`: `1.792586%`
- 双样本平均精度: `1.568124%`
- 已定位问题: 首个 IDR slice 仍存在宏块级语法失步, `I_8x8` 变换残差路径需要继续对齐规范上下文与语法消费.

## 1 帧最小复现实验

- 默认路径(`TAO_H264_8X8_SKIP_CBF=1`): `data/1_h264.mp4` 首个 IDR slice 提前结束在 `decoded_mbs=102/8160`.
- 严格 8x8 CBF 路径(`TAO_H264_8X8_SKIP_CBF=0`): 同位置为 `decoded_mbs=58/8160`.
- 默认最小复现失步位置: `last_mb=(101, 0)`, `cabac_bits=7402/438360`.
- `ffmpeg trace_headers` 已核对 slice header 位流对齐, `cabac_start_byte=4` 与当前实现一致, 根因不在 slice header 解析.
- 高概率根因在宏块语法路径(CABAC 上下文演进或残差语法消费)而非 NAL 头和 slice header.

## 2026-02-20 诊断记录

- CABAC 引擎已切换为 `low/range/bytestream` 形态并对齐 FFmpeg 的 I_PCM 重启流程, 但首个 IDR 失步点未变化.
- 已验证 `I_PCM restart` 为必要步骤: 跳过重启会提前失步.
- 已验证"仅关闭 terminate break 但持续消费 terminate bin"会让失步帧精度进一步恶化.
- 已验证首个视频包仅包含 `SEI + 1个IDR slice`, 不存在"同帧多 slice 未拼装"导致截断.
- `TAO_H264_IPCM_PTR_ADJUST` 在 `[-20, 20]` 扫描中, `decoded_mbs` 最多仅到 `1621`(偏移 `-16`), 且 120 帧精度反而下降, I_PCM 固定偏移不是根因.
- Chroma DC 上下文改为"真实 DC-CBF"后首帧退化到 `decoded_mbs=90/8160`, 残差/上下文链路尚未满足切换前置条件.
- Chroma AC 改为 `U/V` 交错消费后首帧延后到 `decoded_mbs=375/8160`, 但 120 帧精度下降, 已回滚.
- `TAO_H264_IPCM_RESTART_PTR_ADJUST` 在 `data/1_h264.mp4` 上 `-13` 无稳定收益, 保持默认 `0`.
- 已补齐 8x8 CBF 高位上下文初始化(`ctxIdx 1012..1015`), 开启严格 coded_block_flag 消费后退化到 `decoded_mbs=109~171`, `I_8x8` fallback 仍缺失完整上下文建模.
- 已完成按 `pic_parameter_set_id` 选择 `PPS/SPS` 并在 slice 级激活参数集.

## 2026-02-21 诊断记录

- CABAC 初始化已改为与 FFmpeg `ff_init_cabac_decoder` 同步的指针对齐分支路径, 样本结果无变化.
- `mb_qp_delta` 解码上限已按规范修正为 `2*MAX_QP`, 样本结果无变化.
- 16 组组合扫描(`FORCE_4X4/SKIP_IPCM_CHECK/FORCE_NO_IPCM/IGNORE_TERMINATE`), 默认组合最佳(`avg=1.431478%`).
- 已接入最小 P-slice CABAC 语法路径与整数像素运动补偿.
- 已接入最小 B-slice 路径(参考源暂以 list0 占位近似).
- 本轮改动后 120 帧: `sample1 1.126295% -> 1.271633%`, `sample2 1.742157% -> 1.704838%`, 平均提升到 `1.488236%`.
- 新增 `luma 8x8 coded_block_flag` 严格语法路径(含 8x8 邻居 CBF 缓存与上下文增量): `sample1 1.271633% -> 1.287982%`, `sample2 1.704838% -> 1.716215%`, 平均提升到 `1.502099%`.
- I_PCM 对齐修复: 原实现 `raw_pos=ceil(bit_pos/8)` 导致固定 `+1` 字节偏移, 已按 FFmpeg `ptr=bytestream; if(low&1) ptr--` 语义修复.
- 修复后复扫 `IPCM_PTR_ADJUST/IPCM_RESTART_PTR_ADJUST` 的 `[-2,2]` 区间, `0` 为最优.
- `I_8x8 fallback` 默认调整为 `skip_cbf=true`: `sample1=1.325445%`, `sample2=1.736214%`, 平均 `1.530829%`.
- CABAC 邻居默认上下文修复(不可用邻居按非零处理): IDR 最小复现从 `decoded_mbs=90` 到 `102`, `sample1=1.353898%`, `sample2=1.793304%`, 平均 `1.573601%`.
- 复扫 16 组组合仍默认最优: `avg=1.573601%`; `skip_ipcm_check=1` 推到 `465` 但精度下降.
- `I_16x16` AC 行优先实验: `decoded_mbs` 推到 296 但精度下降, 已回滚.
- 帧间亚像素路径接入: 功能自测通过, 精度略回落到 `avg=1.560688%`.
- 8x8 `coded_block_flag` 上下文增量修正为 4x4 邻居语义, 严格路径仍提前失步.
- CABAC 初始化表 `ctxIdx 1016..1023` 补齐, 对样本无直接变化.
- `pending frame` 拼帧逻辑接入, `POC + decode_order` 输出重排缓存接入.
- 参数集切换后运行时状态重置补强完成.
- 8x8 严格 CBF 路径(8x8 邻居上下文): `decoded_mbs=58` 提升到 `95`, 但 120 帧精度仍低于默认路径.
- 默认路径最终基线: `sample1=1.343662%`, `sample2=1.792586%`, 平均 `1.568124%`.
