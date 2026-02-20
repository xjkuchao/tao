# tao-codec Vorbis 样本批量对比报告

来源: `https://samples.ffmpeg.org/allsamples.txt` (抓取日期: 2026-02-19).

筛选规则: 路径包含 `vorbis`(不区分大小写), 且扩展名属于 `.ogg/.ogm/.mkv/.avi/.mp4/.nut`.

字段说明:
- 状态: 成功、失败或跳过.
- 失败原因: 仅失败时填写.
- 样本数差异: Tao样本数-FFmpeg样本数.

| 序号 | URL | 状态 | 失败原因 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | https://samples.ffmpeg.org/A-codecs/vorbis/ffvorbis_crash.ogm | 成功 |  | 659072 | 659072 | 0 | 0.000000596 | 145.45 | 100.00 |  |
| 2 | https://samples.ffmpeg.org/A-codecs/vorbis/floor_type_0/01_Duran_Duran_Planet_Earth.ogg | 成功 |  | 3517312 | 3517312 | 0 | 0.000009477 | 135.77 | 100.00 |  |
| 3 | https://samples.ffmpeg.org/A-codecs/vorbis/floor_type_0/vorbis_floor_type_0.ogg | 成功 |  | 12525572 | 12525572 | 0 | 0.000007527 | 138.68 | 100.00 |  |
| 4 | https://samples.ffmpeg.org/A-codecs/vorbis/Lumme-Badloop.ogg | 成功 |  | 35725824 | 35725824 | 0 | 0.000001252 | 144.98 | 100.00 |  |
| 5 | https://samples.ffmpeg.org/A-codecs/vorbis/vorbis3plus_sample.avi | 成功 |  | 2680320 | 2680320 | 0 | 0.000001162 | 150.51 | 100.00 |  |
| 6 | https://samples.ffmpeg.org/A-codecs/vorbis/vp8OggVorbis1.avi | 成功 |  | 2641600 | 2641600 | 0 | 0.000000194 | 158.96 | 100.00 |  |
| 7 | https://samples.ffmpeg.org/archive/all/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 8 | https://samples.ffmpeg.org/archive/all/matroska+h264+vorbis++COLORS.mkv | 成功 |  | 416384 | 416384 | 0 | 0.000001371 | 139.66 | 100.00 |  |
| 9 | https://samples.ffmpeg.org/archive/all/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 10 | https://samples.ffmpeg.org/archive/all/ogg+mpeg4+vorbis++crash_foobar.ogg | 成功 |  | 161152 | 161152 | 0 | 0.000000086 | 168.08 | 100.00 |  |
| 11 | https://samples.ffmpeg.org/archive/all/ogg++vorbis++vocal2_prob_converting.ogg | 成功 |  | 1855360 | 1855360 | 0 | 0.000001192 | 140.47 | 100.00 |  |
| 12 | https://samples.ffmpeg.org/archive/audio/vorbis/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 13 | https://samples.ffmpeg.org/archive/audio/vorbis/matroska+h264+vorbis++COLORS.mkv | 成功 |  | 416384 | 416384 | 0 | 0.000001371 | 139.66 | 100.00 |  |
| 14 | https://samples.ffmpeg.org/archive/audio/vorbis/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 15 | https://samples.ffmpeg.org/archive/audio/vorbis/ogg+mpeg4+vorbis++crash_foobar.ogg | 成功 |  | 161152 | 161152 | 0 | 0.000000086 | 168.08 | 100.00 |  |
| 16 | https://samples.ffmpeg.org/archive/audio/vorbis/ogg++vorbis++vocal2_prob_converting.ogg | 成功 |  | 1855360 | 1855360 | 0 | 0.000001192 | 140.47 | 100.00 |  |
| 17 | https://samples.ffmpeg.org/archive/container/matroska/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 18 | https://samples.ffmpeg.org/archive/container/matroska/matroska+h264+vorbis++COLORS.mkv | 成功 |  | 416384 | 416384 | 0 | 0.000001371 | 139.66 | 100.00 |  |
| 19 | https://samples.ffmpeg.org/archive/container/ogg/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 20 | https://samples.ffmpeg.org/archive/container/ogg/ogg+mpeg4+vorbis++crash_foobar.ogg | 成功 |  | 161152 | 161152 | 0 | 0.000000086 | 168.08 | 100.00 |  |
| 21 | https://samples.ffmpeg.org/archive/container/ogg/ogg++vorbis++vocal2_prob_converting.ogg | 成功 |  | 1855360 | 1855360 | 0 | 0.000001192 | 140.47 | 100.00 |  |
| 22 | https://samples.ffmpeg.org/archive/extension/mkv/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 23 | https://samples.ffmpeg.org/archive/extension/mkv/matroska+h264+vorbis++COLORS.mkv | 成功 |  | 416384 | 416384 | 0 | 0.000001371 | 139.66 | 100.00 |  |
| 24 | https://samples.ffmpeg.org/archive/extension/ogg/ogg+mpeg4+vorbis++crash_foobar.ogg | 成功 |  | 161152 | 161152 | 0 | 0.000000086 | 168.08 | 100.00 |  |
| 25 | https://samples.ffmpeg.org/archive/extension/ogg/ogg++vorbis++vocal2_prob_converting.ogg | 成功 |  | 1855360 | 1855360 | 0 | 0.000001192 | 140.47 | 100.00 |  |
| 26 | https://samples.ffmpeg.org/archive/extension/ogm/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 27 | https://samples.ffmpeg.org/archive/subtitles/0x0000/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 28 | https://samples.ffmpeg.org/archive/subtitles/0x0000/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 29 | https://samples.ffmpeg.org/archive/video/h264/matroska+h264+vorbis+0x0000+sws10_screenshot_failure.mkv | 成功 |  | 2760320 | 2760320 | 0 | 0.000000253 | 160.66 | 100.00 |  |
| 30 | https://samples.ffmpeg.org/archive/video/h264/matroska+h264+vorbis++COLORS.mkv | 成功 |  | 416384 | 416384 | 0 | 0.000001371 | 139.66 | 100.00 |  |
| 31 | https://samples.ffmpeg.org/archive/video/mpeg4/ogg+mpeg4+vorbis+0x0000+ogm_remux.ogm | 成功 |  | 1278592 | 1278592 | 0 | 0.000000536 | 153.00 | 100.00 |  |
| 32 | https://samples.ffmpeg.org/archive/video/mpeg4/ogg+mpeg4+vorbis++crash_foobar.ogg | 成功 |  | 161152 | 161152 | 0 | 0.000000086 | 168.08 | 100.00 |  |
| 33 | https://samples.ffmpeg.org/avi/ogg/Coyote.Ugly.Sample.Ogg.Vorbis.avi | 成功 |  | 7342848 | 7342848 | 0 | 0.000001684 | 140.06 | 100.00 |  |
| 34 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue746/746-theora-vorbis-sample.ogg | 成功 |  | 548864 | 548864 | 0 | 0.000000004 | 191.42 | 100.00 |  |
| 35 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2893/vorbis_fails_to_decode.ogg | 成功 |  | 18648192 | 18648192 | 0 | 0.000002146 | 137.97 | 100.00 |  |
| 36 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket8741/dx50_vorbis.ogm | 跳过 | 上游已知问题: FFmpeg trac ticket8741 (dx50_vorbis.ogm), 当前阶段跳过 |  |  |  |  |  |  | 已跳过 |
| 37 | https://samples.ffmpeg.org/Matroska/mewmew/mewmew-vorbis-ssa.mkv | 成功 |  | 5587072 | 5587072 | 0 | 0.000001132 | 147.08 | 100.00 |  |
| 38 | https://samples.ffmpeg.org/Matroska/vorbis-audio-switch.mkv | 成功 |  | 2198912 | 2198912 | 0 | 0.000000507 | 152.06 | 100.00 |  |
| 39 | https://samples.ffmpeg.org/MPEG-4/vorbis-in-mp4/borgcube_vorbis20.mp4 | 成功 |  | 2774912 | 2774912 | 0 | 0.000000000 | inf | 100.00 |  |
| 40 | https://samples.ffmpeg.org/MPEG-4/vorbis-in-mp4/mi2_vorbis51.mp4 | 成功 |  | 26561280 | 26561280 | 0 | 0.000000000 | inf | 100.00 |  |
| 41 | https://samples.ffmpeg.org/nut/mewmew-vorbis-ssa.nut | 成功 |  | 5587072 | 5587072 | 0 | 0.000000000 | inf | 100.00 |  |
| 42 | https://samples.ffmpeg.org/ogg/DShow-OLD/DivX640x480-oggvorbis.avi | 成功 |  | 1306240 | 1306240 | 0 | 0.000000075 | 176.38 | 100.00 |  |
| 43 | https://samples.ffmpeg.org/ogg/Vorbis/1sec.ogg | 成功 |  | 88982 | 88982 | 0 | 0.000000149 | 157.63 | 100.00 |  |
| 44 | https://samples.ffmpeg.org/ogg/Vorbis/coyote.ogg | 成功 |  | 7342848 | 7342848 | 0 | 0.000001684 | 140.06 | 100.00 |  |
| 45 | https://samples.ffmpeg.org/ogg/Vorbis/MetalGearSolid/mgs1-sample1.ogg | 跳过 | 暂时跳过: MetalGearSolid 异常 Ogg 样本, 后续专项处理 |  |  |  |  |  |  | 已跳过 |
| 46 | https://samples.ffmpeg.org/ogg/Vorbis/MetalGearSolid/mgs1-sample2.ogg | 跳过 | 暂时跳过: MetalGearSolid 异常 Ogg 样本, 后续专项处理 |  |  |  |  |  |  | 已跳过 |
| 47 | https://samples.ffmpeg.org/ogg/Vorbis/MetalGearSolid/mgs1-sample3.ogg | 跳过 | 暂时跳过: MetalGearSolid 异常 Ogg 样本, 后续专项处理 |  |  |  |  |  |  | 已跳过 |
| 48 | https://samples.ffmpeg.org/ogg/Vorbis/test6.ogg | 成功 |  | 3050496 | 3050496 | 0 | 0.000000596 | 160.36 | 100.00 |  |
