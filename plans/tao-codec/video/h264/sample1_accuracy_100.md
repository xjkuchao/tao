# data/1_h264.mp4 H264 解码精度 100% 专项跟踪

## 1. 背景与目标

- 目标样本: `data/1_h264.mp4`.
- 最终验收: 全片 `299` 帧对比 FFmpeg `100%` 逐字节一致.
- 对标实现: `FFmpeg + OpenH264`.
- 专项门禁: 支持 `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK=1`, 将缺失参考回退视为失败.

## 2. 里程碑与进度

- [x] 建立专项文档与固定里程碑.
- [x] 扩展 DPB 参考快照数据模型 (MB + 4x4, L0 + L1).
- [x] `RefPlanes` 增加 `frame_num` 与 `long_term_frame_idx` 元数据.
- [x] `RefList` 构建取消 `refs.first()` 静默复用.
- [x] 增加缺失参考硬失败门禁 (`TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK`).
- [x] I8x8 预测模式推导改为规范顺序 + 邻居可用性细化.
- [x] B Direct 重构到分区粒度 (`4x4/8x8` 路径统一入口).
- [ ] 对齐 OpenH264 `MapColToList0` 语义到 POC 映射级别.
- [ ] 消除 `build_l0_list_rank_missing` 回退.
- [ ] 逐帧收敛到 `1/67/120/299 = 100%`.

## 3. 当前基线 (2026-02-22, 本轮)

执行命令:

```bash
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=1 TAO_H264_COMPARE_REQUIRED_PRECISION=0 cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=67 TAO_H264_COMPARE_REQUIRED_PRECISION=0 cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=120 TAO_H264_COMPARE_REQUIRED_PRECISION=0 cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=299 TAO_H264_COMPARE_REQUIRED_PRECISION=0 TAO_H264_COMPARE_REPORT=1 cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
```

结果:

| 帧数 | 全局精度 | Y 精度 | U 精度 | V 精度 | max_err |
| --- | --- | --- | --- | --- | --- |
| 1 | 59.443866% | 39.253665% | 99.866898% | 99.781636% | 21 |
| 67 | 12.337902% | 5.013505% | 28.578695% | 25.394693% | 232 |
| 120 | 16.683185% | 7.356836% | 34.862463% | 35.809301% | 252 |
| 299 | 13.911657% | 5.831160% | 28.908107% | 31.237199% | 255 |

报告快照:

- `data/h264_compare_reports/1_h264_1771783852.json`
- `data/h264_compare_reports/1_h264_1771799163.json`

## 4. 专项门禁结果 (本轮)

执行:

```bash
TAO_H264_COMPARE_INPUT=data/1_h264.mp4 TAO_H264_COMPARE_FRAMES=120 TAO_H264_COMPARE_REQUIRED_PRECISION=0 TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK=1 cargo test --test run_decoder h264::test_h264_compare_sample_1 -- --nocapture --ignored
```

结果:

- `120` 帧运行未触发 `missing_reference_fallback` 硬失败.
- 说明本轮路径下未出现“缺失参考回退”直接命中, 但全局精度仍显著低于目标.

结论:

- 当前主矛盾转为 CABAC 语法链路误差, 尤其是多参考 P slice (`num_ref_idx_l0>1`) 下的漂移.

## 5. 本轮关键发现

1. `mb_qp_delta` 上下文链路存在实现偏差: Inter 非 skip 宏块在 `cbp==0` 时不应无条件重置 `prev_qp_delta_nz`.
2. 修正后首个 P slice (`num_ref_idx_l0=1`) 解码宏块数从 `188` 提升到 `660`, 说明 CABAC 链路更接近规范.
3. 但 `num_ref_idx_l0=4` 的 P slice 仍明显早停 (如 `86/21/18/80`), 多参考链路仍是当前瓶颈.

## 6. 下一轮执行项

1. 对 `num_ref_idx_l0>1` 的 P slice 增加 CABAC ref_idx 上下文快照, 与 FFmpeg `decode_cabac_mb_ref` 逐项对齐.
2. 按 FFmpeg `fill_decode_caches` 语义补齐 `left_cbp/top_cbp` 与邻居 motion cache 的一致性路径.
3. 继续推进 `MapColToList0` 的 POC 映射实现, 关闭 Direct 近似分支.
4. 复跑 `1 -> 67 -> 120 -> 299` 门禁并写回增量.
