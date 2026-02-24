# H264 解码器诊断日志

本文件归档 `decoder_dev.md` 开发过程中的详细诊断实验记录, 供后续定位问题时参考.

## 当前基线(10 帧, 2026-02-22, 精度收敛阶段)

| 样本 | 分辨率    | Profile              | 熵编码 | 精度       | PSNR     | max_err | 状态         |
| ---- | --------- | -------------------- | ------ | ---------- | -------- | ------- | ------------ |
| C1   | 1280x720  | Constrained Baseline | CAVLC  | 10.57%     | 19.08dB  | 249     | 通过         |
| C2   | 1920x1080 | Main                 | CAVLC  | **99.999%** | **79.82dB** | 20   | **近 bit-exact** |
| C3   | 704x480   | High                 | CABAC  | 33.61%     | 12.77dB  | 252     | 通过         |
| E1   | 352x200   | Baseline             | CAVLC  | 20.62%     | 25.33dB  | 252     | 通过         |
| E2   | 1280x720  | Main                 | CAVLC  | 44.99%     | 21.21dB  | 237     | 通过         |
| E3   | 640x352   | Main                 | CABAC  | **99.996%** | **73.58dB** | 44   | **近 bit-exact** |
| E4   | 480x204   | Main                 | CAVLC  | 19.58%     | 19.69dB  | 230     | 通过         |
| E5   | 1920x1088 | Main                 | CABAC  | 26.20%     | 20.08dB  | 211     | 通过         |
| E6   | 1920x1080 | High                 | CABAC  | 25.79%     | 8.75dB   | 239     | 通过         |
| E7   | 1920x1080 | High                 | CAVLC  | 6.77%      | 15.97dB  | 247     | 通过         |
| E8   | 352x288   | High                 | CABAC  | 25.44%     | 20.51dB  | 227     | 通过         |
| E9   | 352x200   | Baseline             | CAVLC  | 16.54%     | 20.16dB  | 181     | 通过         |
| X1   | 352x288   | High                 | CABAC  | 81.02%     | 26.62dB  | 131     | 通过         |
| X2   | 352x288   | High                 | CABAC  | 43.97%     | 13.09dB  | 247     | 通过         |
| X3   | 352x288   | High                 | CABAC  | 42.48%     | 15.16dB  | 247     | 通过         |
| X4   | 352x288   | High                 | CABAC  | 7.34%      | 10.45dB  | 253     | 通过         |

- 通过: 16/16, 失败: 0/16 (阈值 1.00%)
- C2 首帧达到 100% bit-exact (PSNR=inf), 10 帧 99.999%
- E3 帧 0-8 bit-exact, 仅帧 9 有微小偏差 (max_err=44)

### 2026-02-24 轮转执行记录(按 `run_accuracy_round.sh`)

- 轮次脚本: `plans/tao-codec/video/h264/run_accuracy_round.sh`
- 输出日志目录: `data/h264_round_logs/`
- 轮次汇总表: `plans/tao-codec/video/h264/round_journal.md`

R1 (B_Direct temporal 映射一致性, 建立基线):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.745599%, 首错帧=1
- G2(67帧): 14.639176%, 首错帧=1
- G3(299帧): 16.551891%, 首错帧=1
- 判定: 作为当前 best score 基线, 后续轮次对比以该 tuple 为准.

R2 (Temporal direct 比例缩放与回退语义):
- 分数与 R1 完全一致, 无提升.
- 判定: 未形成明确提升, 按流程进入下一轮.

R3 (Spatial direct 邻居与 col_zero 条件):
- 分数与 R1 完全一致, 无提升.
- 判定: 未形成明确提升, 按流程进入下一轮.

R4 (col_zero 基于运动可用性判定):
- G0/G1/G2/G3 与 R1 一致.
- 判定: 未形成明确提升.

R5 (P 链路保留 t8x8 语法消费并使用 4x4 残差回退):
- G0(3帧): 83.205075%, 首错帧=1
- G1(10帧): 46.524601%, 首错帧=1
- G2(67帧): 14.641011%, 首错帧=1
- G3(299帧): 16.801326%, 首错帧=1
- 判定: 相比 R1 形成提升, 触发严格验证; 5 项未全通过, 未提交.

R6 (P+B 链路保留 t8x8 语法消费并使用 4x4 残差回退):
- G0(3帧): 83.205075%, 首错帧=1
- G1(10帧): 46.211330%, 首错帧=1
- G2(67帧): 14.600487%, 首错帧=1
- G3(299帧): 16.812227%, 首错帧=1
- 判定: 相比 R5 继续提升, 触发严格验证; 5 项未全通过, 未提交.

R7 (P+B residual 4x4 回退 + 强制 8x8 语法消费 + temporal direct):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.870997%, 首错帧=1
- G2(67帧): 14.732088%, 首错帧=1
- G3(299帧): 16.816100%, 首错帧=1
- 判定: 相比 R6 明确提升, 触发严格验证; 5 项未全通过, 未提交.
- 当前 best: `P299=16.816100`, `FM=1`, `P67=14.732088`, `P10=46.870997`.

R8 (R7 基础叠加强制 spatial direct):
- 四级门禁分数与 R7 完全一致.
- 判定: 未形成明确提升.

R9 (spatial direct 缺省语义对齐: 单侧不补 ref0 + 无邻居 MV 归零):
- G0(3帧): 84.649648%, 首错帧=1
- G1(10帧): 46.823585%, 首错帧=1
- G2(67帧): 14.680349%, 首错帧=1
- G3(299帧): 16.811616%, 首错帧=1
- 判定: 未提升, 已回滚该实验改动.

R10 (temporal direct 去除 col_zero 归零, 仅保留缩放 MV):
- G0(3帧): 84.728524%, 首错帧=1
- G1(10帧): 46.870155%, 首错帧=1
- G2(67帧): 14.732023%, 首错帧=1
- G3(299帧): 16.815508%, 首错帧=1
- 判定: 未提升, 已回滚该实验改动.

R11 (R7 基础上强制 MB0 使用 8x8 语法路径):
- 四级门禁与 R7 完全一致.
- 判定: 未提升.

R12 (R7 基础上切换旧版 inter transform 上下文):
- 四级门禁与 R7 完全一致.
- 判定: 未提升.

R13 (R7 基础上强制 AMVD=0):
- G0(3帧): 84.344082%, 首错帧=1
- G1(10帧): 46.524907%, 首错帧=1
- G2(67帧): 14.653201%, 首错帧=1
- G3(299帧): 14.300676%, 首错帧=1
- 判定: 明显回退, 保持不采用该开关.

R14 (`map_col_to_list0` 失败回退改为 `col_ref_idx` 合法索引):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.869493%, 首错帧=1
- G2(67帧): 14.732052%, 首错帧=1
- G3(299帧): 16.818193%, 首错帧=1
- 判定: 相比 R7 形成提升, 触发严格验证; 5 项未全通过, 未提交.
- 取证: 在 R7 组合下 `TAO_H264_TRACE_B_DIRECT=1` 统计到 `map_col_to_list0 失败=5238/299帧`, 该回退路径为高频热点.
- 当前 best: `P299=16.818193`, `FM=1`, `P67=14.732052`, `P10=46.869493`.

R15 (`map_col_to_list0` 精确失败时按 POC 最近邻回退):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.870550%, 首错帧=1
- G2(67帧): 14.732038%, 首错帧=1
- G3(299帧): 16.817706%, 首错帧=1
- 判定: 未提升, 已回滚最近邻回退策略.

R16 (`map_col_to_list0` 失败回退改为 `col_ref_idx % cur_l0_len`):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.870756%, 首错帧=1
- G2(67帧): 14.732162%, 首错帧=1
- G3(299帧): 16.817642%, 首错帧=1
- 判定: 未提升(P67/P10 上升但 P299 回落), 已回滚取模回退策略.

R17 (R14 基线 + temporal direct 去除 col_zero 归零):
- G0(3帧): 84.728524%, 首错帧=1
- G1(10帧): 46.868660%, 首错帧=1
- G2(67帧): 14.732031%, 首错帧=1
- G3(299帧): 16.817662%, 首错帧=1
- 判定: 未提升, 已回滚该组合改动.

R18 (temporal direct 在 list1 无运动时回退尝试 list0 共定位):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.866946%, 首错帧=1
- G2(67帧): 14.732999%, 首错帧=1
- G3(299帧): 16.809317%, 首错帧=1
- 判定: 未提升且明显回退, 已回滚该改动.

R19 (`map_col_to_list0` 失败回退: 越界统一回退 0, 合法索引保持 col_ref_idx):
- G0(3帧): 84.731181%, 首错帧=1
- G1(10帧): 46.870997%, 首错帧=1
- G2(67帧): 14.732211%, 首错帧=1
- G3(299帧): 16.818375%, 首错帧=1
- 判定: 相比 R14 形成提升, 触发严格验证; 5 项未全通过, 未提交.
- 当前 best: `P299=16.818375`, `FM=1`, `P67=14.732211`, `P10=46.870997`.

R20 (temporal direct 共定位无运动时改为零 MV 兜底):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.745599%, 首错帧=1
- G2(67帧): 14.639176%, 首错帧=1
- G3(299帧): 16.551891%, 首错帧=1
- 判定: 明显回退, 已回滚该改动.

R21 (`map_col_to_list0` 失败回退在 `col_list==1` 时固定为 0):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.745599%, 首错帧=1
- G2(67帧): 14.639176%, 首错帧=1
- G3(299帧): 16.551891%, 首错帧=1
- 判定: 明显回退, 已回滚该改动.

R22 (`map_col_to_list0` POC 未命中时补充 DPB 重建匹配):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.745599%, 首错帧=1
- G2(67帧): 14.639176%, 首错帧=1
- G3(299帧): 16.551891%, 首错帧=1
- 判定: 明显回退, 已回滚该改动.

R23 (spatial direct 去除单侧缺失强补 ref0):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.745599%, 首错帧=1
- G2(67帧): 14.639176%, 首错帧=1
- G3(299帧): 16.551891%, 首错帧=1
- 判定: 未提升, 已回滚该改动.

R23 取证补充:
- `TAO_H264_TRACE_B_DIRECT=1` 下 `map_col_to_list0 失败` 计数为 0, 当前代码路径该 fallback 未触发.
- 方向探针(299 帧): default=`16.551891`, force temporal=`16.550508`, force spatial=`16.551891`.
- 结论: 下一轮应优先排查非 `map_col_to_list0` 的 direct/B 链路.

R24 (`decode_ref_idx` OOB 后语义裁剪到合法上界):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.748367%, 首错帧=1
- G2(67帧): 14.638814%, 首错帧=1
- G3(299帧): 16.568465%, 首错帧=1
- 判定: 相对当前可复现基线(16.551891)有小幅提升, 但未超过本轮框架 best(R19), 已回滚.

R24 取证补充:
- `TAO_H264_TRACE_REF_IDX_OOB=1` 下, 299 帧 `ref_idx` OOB 共 `1408` 次.
- `TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK=1` 首次命中:
  - `scene=build_l0_list_padded, ref_idx=3, list_len=3, 包序号=6`.
- `TAO_H264_TRACE_REF_LIST=1` 下 `missing_fallbacks` 计数到 `1861`, 说明参考回退链路触发频繁.

R25 (`decode_ref_idx` OOB 后回落到 ref_idx=0):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.575138%, 首错帧=1
- G2(67帧): 14.561774%, 首错帧=1
- G3(299帧): 16.659376%, 首错帧=1
- 判定: 相对当前可复现基线(16.551891)有提升, 但未超过本轮框架 best(R19), 已回滚.
- 结论: `ref_idx` OOB 的后处理策略对精度有显著影响, 下一轮继续围绕 OOB 语义做更细粒度约束.

R26 (L0 参考列表长度不足时复用最后有效参考):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.747666%, 首错帧=1
- G2(67帧): 14.636794%, 首错帧=1
- G3(299帧): 16.613570%, 首错帧=1
- 判定: 相对当前可复现基线(16.551891)有提升, 但未超过本轮框架 best(R19), 已回滚.
- 结论: 列表补齐策略会影响中后段帧, 但单独替换补齐策略不足以达成阶段性提升.

R27 (`decode_ref_idx` OOB 分层回退: `active_ref_count<=2` 用上界, 其余回 0):
- G0(3帧): 84.481214%, 首错帧=1
- G1(10帧): 46.622804%, 首错帧=1
- G2(67帧): 14.657667%, 首错帧=1
- G3(299帧): 16.672510%, 首错帧=1
- 判定: 相对当前可复现基线(16.551891)有提升, 但未超过本轮框架 best(R19), 已回滚.
- 结论: OOB 后处理分层策略优于单纯回 0/上界, 但仍不足以跨越当前 best.

严格验证状态汇总(R5/R6/R7/R14/R19):
- `cargo fmt --all -- --check`: 通过.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 失败(现存 warnings/Clippy 规则违规).
- `cargo check --workspace --all-targets --all-features`: 通过.
- `cargo test --workspace --all-targets --all-features --no-fail-fast`: 失败(固定失败目标: `tao-codec --lib`, `tao-format --lib`).
- `RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --all-features --no-deps`: 通过.

2026-02-24 已验证并回滚的新增实验:
- 假设: 在 `TAO_H264_DEBUG_INTER_PARSE_T8X8_USE_4X4=1` 时, `transform_8x8_flag` 写回上下文改为跟随 `use_8x8_residual`.
- 结果: `G3(299帧)` 从 `16.812227` 回退到 `16.480674`.
- 结论: 回滚该改动, 不进入轮次提交.

本轮已验证并回滚的实验性改动(未保留到代码):
- `find_reference_picture_for_planes` 短期图像匹配优先级改为 POC 优先.
- temporal direct 无共定位运动时强制零 MV 回退.
- spatial direct 邻居全不可用时强制零 MV 回退.
- 结果: 对 `1_h264.mp4` 299 帧精度未形成可接受增益, 且中间门禁(P67/P10)出现回落, 已全部回滚.

### 精度收敛阶段已完成的修复

1. **去块滤波器全面修复**:
   - Bug1: 使用 per-edge QP 替代 slice_qp (mb_qp 数组追踪)
   - Bug2: 弱滤波添加 p1/q1 修正
   - Bug3: 色度 boundary_step 2→4
   - Bug5: 强滤波添加 p2/q2 更新
2. **色度 DC 反量化**: 添加舍入偏移 `(1 << (qp_per - 1))`
3. **Slice 边界帧内预测邻居可用性**: 根因修复, 新 slice 首 MB 不使用前一 slice 的邻居做预测.
   - 新增 `left_avail()` / `top_avail()` 方法, 基于 `mb_slice_first_mb` 判断同 slice
   - 影响: C2 从 ~20% 提升到 99.999% (首帧 bit-exact)
4. **CAVLC nC 上下文 slice 边界感知**: `calc_luma_nc` / `calc_chroma_u_nc` / `calc_chroma_v_nc` 在 MB 边界检查 slice 归属
5. **CAVLC I_8x8 语法补齐**: 在 `decode_cavlc_i_mb` 中补齐 `transform_size_8x8_flag` 与 `intra8x8_pred_mode` 解析, 并新增 I_8x8 交织预测+残差路径.
   - 影响: E7 从 0.47% 提升到 6.76%, 从唯一失败变为通过(阈值 1.00%)
6. **MP4 `edts/elst` 时间线对齐 + 对比侧负 PTS 过滤**:
   - MP4 demuxer 新增 `elst` 解析并按 `media_time` 归一化 `pkt.pts`.
   - `decoder_compare` 默认过滤 `pts<0` 帧, 保持参考链不丢失同时对齐 FFmpeg 输出时间线.
   - 影响: C1 `10.32% -> 10.56%`, E4 `8.41% -> 19.34%`.
7. **CAVLC 容错收敛 (coeff_token + total_zeros)**:
   - `coeff_token` 在主表失败时回退邻近 VLC 表, 显著减少 `coeff_token` 失败导致的位流停滞.
   - `total_zeros` 在 `max_num_coeff=15` 回退路径裁剪到合法范围, 消除 `scan_pos` 越界型失败.
   - C1 追踪中 `coeff_token` 失败由 6 次降为 0 次, `scan_pos` 越界由 8 次降为 0 次.
   - 当前剩余错误构成(单次 C1 追踪): `run_before` 21 次, `total_coeff=16>15` 2 次, `total_zeros(tc=1,max=16)` 2 次.
   - 影响: E2 `44.68% -> 44.99%`, E4 `19.34% -> 19.58%`, E7 `6.75% -> 6.77%`.

### 2026-02-22 本轮否决实验(已回滚)

- **run_before run7 全表回退 + clamp**:
  - 目标: 缓解 `zeros_left=7..9` 的 `run_before` 失败.
  - 结果: C1 `10.56% -> 9.24%`, 明显回退, 已回滚.
- **`total_coeff=16,max_num_coeff=15` 按 `parse_max=16` 消费并裁剪**:
  - 目标: 保持位流前进, 避免整块置零.
  - 结果: E2 `44.99% -> 44.81%`, E4 `19.58% -> 19.54%`, E7 `6.77% -> 6.77%(微降)`, 已回滚.
- **P-slice `P_16x8/P_8x16` 方向性 MV 预测替换**:
  - 目标: 对齐 FFmpeg `pred_16x8/pred_8x16` 分支.
  - 结果: C1 `10.56% -> 10.40%`, E1/E9 同步回退, 已回滚.

### 2026-02-22 C1 剩余失步样本点(用于下一轮定点修复)

- 首批失败点集中在 `run_before` 且 `zeros_left=7/8/9`, 覆盖 `inter_luma_4x4/chroma_u_ac/i16x16_luma_ac`.
- 代表样本:
  - `scene=inter_luma_4x4 coord=(10,2) bits_read=119 total_coeff=2 trailing_ones=2 total_zeros=7 zeros_left=7`
  - `scene=inter_luma_4x4 coord=(112,68) bits_read=79184 total_coeff=2 trailing_ones=2 total_zeros=7 zeros_left=7`
  - `scene=inter_luma_4x4 coord=(265,160) bits_read=102453 total_coeff=2 trailing_ones=2 total_zeros=9 zeros_left=9`
  - `scene=chroma_u_ac coord=(127,13) bits_read=18006 total_coeff=4 trailing_ones=2 total_zeros=7 zeros_left=7`
  - `scene=i16x16_luma_ac coord=(280,35) bits_read=23819 total_coeff=16(max=15)`
- 结论: 当前更像是"前序语法链路偏差导致 run_before 无法匹配", 不是 run_before VLC 表本身错误.

### 旧基线(10 帧, 2026-02-22, 基础设施阶段)

(C2=33.3%, E3=32.4%, X1=1.6%, E7=0.5% 等, 去块+chroma+slice 修复前)

### 阶段 0 基础设施修复汇总

1. 清除 `slice_decode.rs` 和 `cavlc_mb.rs` 中的 `eprintln!` 调试输出
2. 修复 MOV 解封装器 `hdlr` box 覆盖问题 (C2, E2, E4)
3. 新增 H264 AnnexB Elementary Stream 解封装器 (E5, E6, E8, X1-X4)
4. 修复 H264EsProbe 探测优先级, 防止 MP3 误检 (E8)
5. 修复 H264 解码器延迟初始化, 支持无 extra_data 的裸流 (E5, E6, X2-X4)
6. 修复 AVCC/AnnexB NAL 分割冲突, length_size=0 用于裸流 (E5, E6, X2-X4)

## 旧基线(120 帧, 2026-02-21)

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
