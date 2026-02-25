# H264 精度轮转执行日志

| 时间(UTC) | 轮次 | 样本 | 说明 | G3 P299 | 首个不一致帧 | G2 P67 | G1 P10 | 结论 |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| 2026-02-24 20:09:10 | R-WP-I8X8 | `data/1_h264.mp4` | 修复weighted_bi_sample公式+实现remap_i8x8_mode_for_unavailable | 16.989161 | 1 | 13.052769 | 41.620068 | 提升(失败) |
| 2026-02-24 20:46:57 | R-CBF-SKIP | `data/1_h264.mp4` | P_Skip/B_Skip宏块缺少4x4 luma CBF缓存清零导致CABAC上下文污染 | 16.989161 | 1 | 13.052769 | 41.620068 | 未提升 |
| 2026-02-24 20:52:29 | R-TSF-ORDER | `data/1_h264.mp4` | 修复P/B帧transform_size_8x8_flag应在coded_block_pattern之前解析且不以luma_cbp为门控 | 15.033383 | 1 | 14.484375 | 46.608793 | 未提升 |
| 2026-02-24 21:11:01 | R-TSF-SPEC | `data/1_h264.mp4` | 规范对齐: P_16x16/B_16x16在CBP前解析, 其他类型在CBP后解析 | 15.314925 | 1 | 14.502929 | 46.459796 | 未提升 |
| 2026-02-25 04:22:17 | R-MVPRED-FIX | `data/1_h264.mp4` | 修复MV预测match_count>=2时按FFmpeg使用原始A/B/C做median | 18.112247 | 1 | 13.574525 | 44.435545 | 提升(通过) |
| 2026-02-25 05:08:06 | R-BFRAME-REORDER | `data/1_h264.mp4` | 修复 B 帧显示顺序 DPB 释放逻辑, 避免按 POC 乱序输出导致对比失准 | 18.071884 | 1 | 13.621650 | 44.249013 | 未提升 |
| 2026-02-25 06:50:58 | R-PSKIP-SLICE | `data/1_h264.mp4` | 修复P_Skip mv预测在slice边界仍误用邻居的问题, 将邻居不可用判定从画面边界提升为slice感知 | - | 1 | - | 41.620052 | 提升(通过) |
| 2026-02-25 06:56:23 | R-MVPRED-MEDIAN-ABC | `data/1_h264.mp4` | 修复L0/L1分区MV预测在匹配邻居>=2时错误只在匹配集合取中值, 改为按FFmpeg对原始A/B/C取中值 | - | 1 | - | 44.435529 | 提升(通过) |
| 2026-02-25 07:03:10 | R-QPEL-DIAG-MAP | `data/1_h264.mp4` | 修复luma qpel对角位置映射, 对齐H.264/OpenH264的(1,1)/(3,1)/(1,3)/(3,3)取样规则并补充单测 | - | 1 | - | 44.436208 | 提升(通过) |
| 2026-02-25 07:05:24 | R-REFLIST-PAD-LAST | `data/1_h264.mp4` | L0/L1参考列表长度不足时改为复用最后一个有效参考, 避免零参考补位造成运动补偿偏移 | - | 1 | - | 44.447592 | 提升(通过) |
| 2026-02-25 07:07:39 | R-REFIDX-CABAC-CTX58 | `data/1_h264.mp4` | 修复CABAC ref_idx解码上下文: 首bin用54..57, 后续扩展bin固定使用58(对齐FFmpeg) | - | 1 | - | 44.804363 | 提升(通过) |
| 2026-02-25 07:12:00 | R-REFIDX-CABAC-CTX58-REVERT | `data/1_h264.mp4` | 复核FFmpeg源码后回滚: decode_cabac_mb_ref应持续使用54+ctx并迭代ctx, 非固定58 | - | 1 | - | 44.447592 | 回滚(通过) |
| 2026-02-25 07:15:11 | R-REFIDX-CLIP-ON-OVERFLOW | `data/1_h264.mp4` | 当CABAC解出ref_idx>=num_ref_idx时截断到num_ref_idx-1(保持比特消费不变), 消除apply_inter_block_l0缺失参考回退 | - | 1 | - | 44.480684 | 提升(通过) |
| 2026-02-25 07:22:07 | R-REFIDX-CLIP-TO-ZERO | `data/1_h264.mp4` | 将ref_idx越界截断策略从“截到末索引”改为“截到0号参考”, 抑制错误参考扩散并降低中后段色度峰值误差 | - | 1 | - | 44.631195 | 提升(通过) |

#### 本轮(2026-02-25 14:+++) 诊断总结: Stride修复试验

**假设**: MC读reference frame时使用了self.stride_y而非reference frame的stride
**修复范围**: RefPlanes+ReferencePicture添加stride字段, 5个文件修改, 12个签名改动  
**测试结果**: P299: 18.112% → 17.144% (**REGRESSION**)
**结论**: Stride不是根本问题, 修复已完整回滚

**已验证正确的地方**:
- c_src_y = c_dst_y + floor_div(mv_y_qpel, 8) ✓ (正确使用mv_y, 非mv_x)
- QPEL对角线采样 ✓ (已对齐FFmpeg)
- DPB B-frame排序 ✓ (R-BFRAME-REORDER)

**P-frame质量问题(42.7%)仍需排查**:
- 🔍 MV预测逻辑(predict_mv_l0_partition)对标FFmpeg
- 🔍 L0 reference list构建(collect_default_reference_list_l0)
- 🔍 Reference frame buffer是否正确初始化和填充
- 🔍 Weighted prediction参数

