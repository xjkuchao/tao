# tao-codec MP3 解码器样本覆盖率测试计划

## 1. 背景与目标

基于 `tao-codec_mp3_samples_report.md` 的 185 条样本首轮批量对比结果,
制定 MP3 解码器精度提升与全覆盖达标计划.

**最终目标**: 全部 185 条样本状态为"成功", 精度达到 100.00%.

### 首轮测试结果概览(修复前)

| 类别 | 数量 | 描述 | 处理策略 |
| --- | --- | --- | --- |
| A. 精度 100% | 20 | 成功且精度 = 100.00% | 标注通过, 无需复测 |
| B. 精度 < 100% | 143 | 成功但精度未达标, 根因为样本数差异 | 修复起点帧/末尾帧偏差后复测 |
| C. 测试失败 | 22 | 解码过程发生错误 | 修复解码错误, 使其通过并达到精度 100% |

### P0 修复后测试结果概览(2026-02-19)

修复 `parse_vbr_header` 无条件跳过首帧 bug 后复测结果:

| 类别 | 数量 | 变化 | 说明 |
| --- | --- | --- | --- |
| A. 精度 100% | 97 | +77 | 修复 CBR 首帧被跳过的问题后大幅提升 |
| B. 精度 < 100% | 66 | -77 | 仍有 B 类问题待解决(见下方分析) |
| C. 测试失败 | 22 | 0 | 未变化 |

### 最新复测结果概览(2026-02-19, 本轮修复后)

本轮已完成 MPEG-2/2.5 LSF 比例因子解码、MPEG-2 intensity stereo、探测器误判修复与损坏帧容错增强, 并多轮复测.

| 类别 | 数量 | 说明 |
| --- | --- | --- |
| A. 精度 100% | 122 | 较 P0 后继续提升 |
| B. 精度 < 100% | 59 | 主要为高失真样本与边缘 case |
| C. 测试失败 | 4 | 剩余少量异常样本 |

#### 修复后 B 类问题分类

| 特征 | 数量 | 根因 | 策略 |
| --- | --- | --- | --- |
| diff=0, prec≈50% | 59 | 立体声处理 bug: 一个声道输出错误(前首帧 bug 掩盖了此问题) | 修复联合立体声/强度立体声解码 |
| diff=-1152, prec≈38% | 4 | takethat.mp3 × 4 镜像, 仍少一帧 | 排查末尾帧截断逻辑 |
| diff=+1058, prec≈35% | 1 | sample.VBR.32.64.44100Hz.Joint.mp3, Tao 比 FFmpeg 多输出样本 | 排查 VBR 末尾判断逻辑 |
| diff=-2304, prec≈24% | 1 | issue1044/j.mp3, 少两帧 | 单独排查 |
| diff=43324176, prec≈50% | 1 | 异常大差值, 需单独分析 | 单独排查 |

## 2. 各类问题分析

### B 类: 样本数差异导致精度不足

B 类样本的共同特征: `样本数差异 < 0`(Tao 比 FFmpeg 少),
差值均为 1152 的整数倍(-1152/-2304/-3456/-4608/-5760/-6912 等),
1152 = MP3 单帧每通道样本数.

可能根因(按优先级排查):
1. **Xing/Info/VBRI 标头帧**: Tao 将其跳过, FFmpeg 输出其静音帧内容
2. **首帧对齐差异**: Tao 的 Demuxer 起始位置比 FFmpeg 多跳 N 帧
3. **ID3 标签偏移**: 含大尺寸 ID3v2 标签时, 帧同步起点偏移不一致
4. **末尾帧截断**: Gapless 播放相关的 padding 帧处理差异

### C 类: 测试失败根因分类

| 失败原因 | 数量 | 分析 |
| --- | --- | --- |
| `MP3: 未找到有效的 MPEG 音频帧` | 15 | Demuxer 未能跳过大 ID3 标签/内嵌封面图到达音频帧 |
| `未找到 MP3 音频流` | 2 | 文件格式被误识别或 MP3 流提取逻辑缺失 |
| `TS: 同步字节不匹配` | 2 | 文件被 FormatProbe 误识别为 MPEG-TS |
| `MP3 main_data 偏移无效` | 1 | main_data_begin 回溯超出缓冲区 |
| `MP3 part2_3_length 小于 scale factor 长度` | 1 | 帧长度字段异常, 缺少边界保护 |
| `right: 0`(断言失败) | 1 | FFmpeg 解码输出为空, 属异常样本 |

## 3. 修复路径

### P0 (B 类) 修复首帧/末尾帧样本数偏差 ✅ 部分完成

**已修复(2026-02-19)**: `parse_vbr_header` 返回类型由 `TaoResult<(Option<u64>, u32, u32)>`
改为 `TaoResult<Option<(Option<u64>, u32, u32)>>`, 未找到 Xing/VBRI 时返回 `Ok(None)`.
原 `if let Ok(...)` 始终匹配, 导致 CBR 文件首个音频帧被无条件跳过.
修复后 A 类从 20 条提升至 97 条(+77).

**待处理**: 修复后暴露出 B 类剩余问题, 主要为立体声解码 bug(见下方 P0b).

### P0b (B 类) 修复立体声解码 bug (59 条, diff=0, prec≈50%)

- 59 条样本 diff=0(样本数已对齐), 但精度仅约 50%, 说明一个声道解码结果错误
- 精度公式: `ref_power / (ref_power + mse) * 100`, 精度=50% 等价于 MSE ≈ ref_power
- 根因: 联合立体声(Joint Stereo)或强度立体声(Intensity Stereo)的 M/S 矩阵变换或声道重建逻辑有误
- 此 bug 在首帧 bug 修复前被样本数差异所掩盖
- 修复方向: 检查 `tao-codec/src/decoders/mp3/` 中 stereo 解码部分(mid-side/intensity stereo)

### P1 (C 类) 修复 `未找到有效的 MPEG 音频帧`(15 条)

- 排查这些文件的实际内容(多数含内嵌封面图、大 ID3v2 标签)
- 加强 MP3 Demuxer 的帧同步跳跃逻辑, 增大扫描窗口
- 参考: FFmpeg `mp3dec.c` 的 `ff_mp3_search_sync_word` 容错策略

### P2 (C 类) 修复 `未找到 MP3 音频流`(2 条)

- 下载对应样本, 用 `ffprobe` 确认实际容器格式
- 排查 Tao FormatProbe 识别优先级, 修复 MP3 流提取逻辑

### P3 (C 类) 修复 `TS: 同步字节不匹配`(2 条)

- ticket3844/tuu_gekisinn.mp3 和 ticket6532/test.mp3 被误识别为 MPEG-TS
- 改进 FormatProbe 评分: 提高 MPEG-TS 识别门限, 降低误识别率

### P4 (C 类) 修复剩余边缘 Case(3 条)

- `main_data 偏移无效`: 修复 bitstream 回溯缓冲区管理
- `part2_3_length 异常`: 增加帧头字段合法性检查并容错
- `right: 0` 断言: 确认是样本本身异常还是 Tao 解码逻辑问题

## 4. 工具使用说明

所有命令均从**项目根目录**执行.

```bash
# 默认断点续测(跳过已有结果的记录, 只处理未测试的)
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py

# 重新测试所有精度不为 100% 的记录(含失败, B + C 类)
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py --retest-imprecise

# 只重新测试失败的记录(C 类)
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py --retest-failed

# 重新测试全部 185 条记录
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py --retest-all

# 只测试指定序号(可多个, 如 3, 5, 8)
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py --index 3 5 8

# 组合使用: 只复测指定序号中失败的
python plans/tao-codec_mp3_coverage/run_mp3_samples_compare.py --retest-failed --index 3 5 8
```

## 5. 未来扩展: 编码器覆盖率

MP3 编码器开发完成后, 在本目录下新建:
- `tao-codec_mp3_encoder_samples_urls.txt` — 编码测试输入样本清单
- `tao-codec_mp3_encoder_samples_report.md` — 编码器测试结果报告
- `run_mp3_encoder_samples_compare.py` — 编码器批量对比脚本

编码器测试策略: 对 PCM 样本执行 Tao 编码 -> FFmpeg 解码 -> 与参考输出对比精度.

## 6. 验收标准

- 全部 185 条样本状态为"成功", 精度 = 100.00%.
- 报告中无"失败"记录.
- 所有修复均有对应单元测试覆盖.

## 7. 进度标记

- [x] P0 修复首帧偏差(CBR 无条件跳帧 bug) -> A 类 20→97 条(+77)
- [ ] P0b 修复立体声解码 bug -> 59 条 diff=0 prec≈50% 样本达到精度 100%
- [ ] P0b 修复立体声解码 bug -> 已从大批 50% 提升, 仍有非 100% 样本待继续收敛
- [ ] P1 修复 `未找到有效的 MPEG 音频帧`(15 条)
- [ ] P2 修复 `未找到 MP3 音频流`(2 条)
- [ ] P3 修复 TS 误识别(2 条)
- [ ] P4 修复其余边缘 Case(3 条)
- [ ] 全部 185 条样本精度达到 100%, 覆盖率测试完成
