# tao-codec MP3 样本批量对比报告

字段说明:
- 状态: 成功或失败.
- 失败原因: 仅失败时填写.
- 样本数差异: Tao样本数-FFmpeg样本数.

| 序号 | URL | 状态 | 失败原因 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | https://samples.ffmpeg.org/A-codecs/MP3/01%20-%20Charity%20Case.mp3 | 成功 |  | 16972032 | 16972032 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 2 | https://samples.ffmpeg.org/A-codecs/MP3/ascii.mp3 | 成功 |  | 1392768 | 1398528 | -5760 | 1.361548 | 13.06 | 31.35 |  |
| 3 | https://samples.ffmpeg.org/A-codecs/MP3/Boot%20to%20the%20Head.MP3 | 失败 | MP3 对比失败: "取帧失败: 无效数据: MP3 main_data 偏移无效" | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 4 | https://samples.ffmpeg.org/A-codecs/MP3/broken-first-frame.mp3 | 成功 |  | 369792 | 375552 | -5760 | 0.059361 | 40.45 | 35.06 |  |
| 5 | https://samples.ffmpeg.org/A-codecs/MP3/Die%20Jodelschule.mp3 | 失败 | MP3 对比失败: "未找到 MP3 音频流" | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 6 | https://samples.ffmpeg.org/A-codecs/MP3/Ed_Rush_-_Sabotage.mp3 | 成功 |  | 38784384 | 38787840 | -3456 | 2.647036 | 8.16 | 31.83 |  |
| 7 | https://samples.ffmpeg.org/A-codecs/MP3/Enrique.mp3 | 成功 |  | 17750016 | 17756928 | -6912 | 2.023514 | 8.05 | 32.73 |  |
| 8 | https://samples.ffmpeg.org/A-codecs/MP3/jpg_in_mp3.mp3 | 失败 | MP3 对比失败: InvalidData("MP3: 未找到有效的 MPEG 音频帧") | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 9 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_misidentified_2.mp3 | 成功 |  | 35644032 | 35647488 | -3456 | 2.111423 | 6.73 | 32.68 |  |
| 10 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_misidentified.mp3 | 成功 |  | 19330560 | 19335168 | -4608 | 1.985407 | 6.90 | 32.68 |  |
| 11 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_with_embedded_albumart.mp3 | 失败 | MP3 对比失败: InvalidData("MP3: 未找到有效的 MPEG 音频帧") | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 12 | https://samples.ffmpeg.org/A-codecs/MP3-pro/18%20Daft%20Punk%20-%20Harder%2C%20Better%2C%20Faster%2C%20Stronger.mp3 | 成功 |  | 9945216 | 9948672 | -3456 | 1.324342 | 14.68 | 50.00 |  |
| 13 | https://samples.ffmpeg.org/A-codecs/MP3-pro/27%20MC%20Solaar%20-%20Rmi.mp3 | 成功 |  | 11469312 | 11472768 | -3456 | 1.448923 | 13.00 | 50.00 |  |
| 14 | https://samples.ffmpeg.org/A-codecs/MP3-pro/scooter-wicked-02-imraving.mp3 | 成功 |  | 9189504 | 9191808 | -2304 | 1.234582 | 14.08 | 50.00 |  |
| 15 | https://samples.ffmpeg.org/A-codecs/MP3/SegvMPlayer0.90.mp3 | 失败 | MP3 对比失败: "取帧失败: 无效数据: MP3 part2_3_length 小于 scale factor 长度" | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 16 | https://samples.ffmpeg.org/A-codecs/MP3/Silent_Light.mp3 | 成功 |  | 23234688 | 23238144 | -3456 | 1.888716 | 9.59 | 32.15 |  |
| 17 | https://samples.ffmpeg.org/A-codecs/MP3/%5Buran97_034%5D_02_dq_-_take_that.mp3 | 成功 |  | 20697984 | 20701440 | -3456 | 1.988511 | 9.38 | 40.92 |  |
| 18 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz128kbps.mp3 | 成功 |  | 4867200 | 4870656 | -3456 | 1.070862 | 12.17 | 32.55 |  |
| 19 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz32kbps.mp3 | 成功 |  | 4867200 | 4870656 | -3456 | 1.243593 | 12.27 | 32.56 |  |
| 20 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz64kbps.mp3 | 成功 |  | 4867200 | 4870656 | -3456 | 1.089559 | 12.18 | 32.55 |  |
| 21 | https://samples.ffmpeg.org/A-codecs/suite/MP3/bboys16.mp3 | 成功 |  | 593280 | 597888 | -4608 | 0.982843 | 17.88 | 50.00 |  |
| 22 | https://samples.ffmpeg.org/A-codecs/suite/MP3/idtaggedcassidyhotel.mp3 | 成功 |  | 2122092 | 2122092 | 0 | 0.000002 | 139.36 | 100.00 |  |
| 23 | https://samples.ffmpeg.org/A-codecs/suite/MP3/piano2.mp3 | 成功 |  | 3676032 | 3679488 | -3456 | 1.361788 | 19.01 | 30.27 |  |
| 24 | https://samples.ffmpeg.org/A-codecs/suite/MP3/piano.mp3 | 成功 |  | 3845376 | 3849984 | -4608 | 1.388273 | 15.52 | 32.19 |  |
| 25 | https://samples.ffmpeg.org/A-codecs/suite/MP3/sample.VBR.32.64.44100Hz.Joint.mp3 | 成功 |  | 391680 | 390622 | 1058 | 0.708415 | 13.76 | 35.22 |  |
| 26 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track1.mp3 | 成功 |  | 1414656 | 1416960 | -2304 | 0.217497 | 27.70 | 50.00 |  |
| 27 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track2.mp3 | 成功 |  | 1414656 | 1416960 | -2304 | 0.425154 | 25.19 | 50.00 |  |
| 28 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track3.mp3 | 成功 |  | 1414656 | 1416960 | -2304 | 0.546579 | 20.58 | 50.00 |  |
| 29 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000073.mp3 | 成功 |  | 12633984 | 12637440 | -3456 | 1.539386 | 15.51 | 32.61 |  |
| 30 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000091.mp3 | 成功 |  | 18193536 | 18196992 | -3456 | 1.040156 | 14.46 | 30.85 |  |
| 31 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000127.mp3 | 成功 |  | 16737408 | 16743168 | -5760 | 1.366899 | 14.00 | 32.73 |  |
| 32 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 | 成功 |  | 20877528 | 20877528 | 0 | 0.000003 | 133.99 | 100.00 |  |
| 33 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 | 成功 |  | 21650688 | 21657600 | -6912 | 1.942495 | 10.64 | 32.57 |  |
| 34 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 | 成功 |  | 23078016 | 23081472 | -3456 | 1.503002 | 14.54 | 32.23 |  |
| 35 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 | 成功 |  | 938304 | 938880 | -576 | 1.071937 | 11.77 | 50.00 |  |
| 36 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 | 成功 |  | 36413568 | 36417024 | -3456 | 2.123067 | 11.56 | 32.12 |  |
| 37 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 | 成功 |  | 1452672 | 1458432 | -5760 | 1.953029 | 4.61 | 29.56 |  |
| 38 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bigla.mp3 | 成功 |  | 81571589 | 81571589 | 0 | 0.000001 | 151.80 | 100.00 |  |
| 39 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 | 成功 |  | 29775168 | 29776320 | -1152 | 1.042691 | 26.23 | 50.00 |  |
| 40 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 | 成功 |  | 21604032 | 21605184 | -1152 | 0.957259 | 25.97 | 50.00 |  |
| 41 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 | 成功 |  | 19884672 | 19888128 | -3456 | 2.065250 | 9.46 | 32.81 |  |
| 42 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 | 成功 |  | 357966 | 357966 | 0 | 0.263960 | 24.35 | 50.00 |  |
| 43 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 | 成功 |  | 720570 | 720570 | 0 | 0.000001 | 148.09 | 100.00 |  |
| 44 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 | 成功 |  | 116250 | 116250 | 0 | 0.271499 | 24.41 | 50.00 |  |
| 45 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 | 成功 |  | 22057344 | 22060800 | -3456 | 1.938634 | 9.73 | 31.16 |  |
| 46 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 | 成功 |  | 22049280 | 22051584 | -2304 | 0.937428 | 20.63 | 50.00 |  |
| 47 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 | 成功 |  | 22049280 | 22051584 | -2304 | 0.975655 | 20.63 | 50.00 |  |
| 48 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.937609 | 20.65 | 50.00 |  |
| 49 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.949698 | 20.65 | 50.00 |  |
| 50 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 51 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 |  |  |  |  |  |  |  |  |  |
| 52 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 |  |  |  |  |  |  |  |  |  |
| 53 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 |  |  |  |  |  |  |  |  |  |
| 54 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 |  |  |  |  |  |  |  |  |  |
| 55 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 |  |  |  |  |  |  |  |  |  |
| 56 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Btakethat.mp3 |  |  |  |  |  |  |  |  |  |
| 57 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 |  |  |  |  |  |  |  |  |  |
| 58 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 |  |  |  |  |  |  |  |  |  |
| 59 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 |  |  |  |  |  |  |  |  |  |
| 60 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 |  |  |  |  |  |  |  |  |  |
| 61 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 |  |  |  |  |  |  |  |  |  |
| 62 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 |  |  |  |  |  |  |  |  |  |
| 63 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 |  |  |  |  |  |  |  |  |  |
| 64 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 |  |  |  |  |  |  |  |  |  |
| 65 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 |  |  |  |  |  |  |  |  |  |
| 66 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 |  |  |  |  |  |  |  |  |  |
| 67 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 |  |  |  |  |  |  |  |  |  |
| 68 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 |  |  |  |  |  |  |  |  |  |
| 69 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 |  |  |  |  |  |  |  |  |  |
| 70 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 |  |  |  |  |  |  |  |  |  |
| 71 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 |  |  |  |  |  |  |  |  |  |
| 72 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 |  |  |  |  |  |  |  |  |  |
| 73 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 |  |  |  |  |  |  |  |  |  |
| 74 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 |  |  |  |  |  |  |  |  |  |
| 75 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 |  |  |  |  |  |  |  |  |  |
| 76 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 77 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 78 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 79 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 |  |  |  |  |  |  |  |  |  |
| 80 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 81 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 |  |  |  |  |  |  |  |  |  |
| 82 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 |  |  |  |  |  |  |  |  |  |
| 83 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 |  |  |  |  |  |  |  |  |  |
| 84 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 |  |  |  |  |  |  |  |  |  |
| 85 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 |  |  |  |  |  |  |  |  |  |
| 86 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 |  |  |  |  |  |  |  |  |  |
| 87 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 |  |  |  |  |  |  |  |  |  |
| 88 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 |  |  |  |  |  |  |  |  |  |
| 89 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 |  |  |  |  |  |  |  |  |  |
| 90 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 |  |  |  |  |  |  |  |  |  |
| 91 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 |  |  |  |  |  |  |  |  |  |
| 92 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 |  |  |  |  |  |  |  |  |  |
| 93 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 |  |  |  |  |  |  |  |  |  |
| 94 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 |  |  |  |  |  |  |  |  |  |
| 95 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 |  |  |  |  |  |  |  |  |  |
| 96 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 |  |  |  |  |  |  |  |  |  |
| 97 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 |  |  |  |  |  |  |  |  |  |
| 98 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 |  |  |  |  |  |  |  |  |  |
| 99 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 |  |  |  |  |  |  |  |  |  |
| 100 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 |  |  |  |  |  |  |  |  |  |
| 101 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 |  |  |  |  |  |  |  |  |  |
| 102 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 |  |  |  |  |  |  |  |  |  |
| 103 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 |  |  |  |  |  |  |  |  |  |
| 104 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 |  |  |  |  |  |  |  |  |  |
| 105 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 |  |  |  |  |  |  |  |  |  |
| 106 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 107 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 108 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 109 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 |  |  |  |  |  |  |  |  |  |
| 110 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 111 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 |  |  |  |  |  |  |  |  |  |
| 112 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 |  |  |  |  |  |  |  |  |  |
| 113 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 |  |  |  |  |  |  |  |  |  |
| 114 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 |  |  |  |  |  |  |  |  |  |
| 115 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 |  |  |  |  |  |  |  |  |  |
| 116 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 |  |  |  |  |  |  |  |  |  |
| 117 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 |  |  |  |  |  |  |  |  |  |
| 118 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 |  |  |  |  |  |  |  |  |  |
| 119 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 |  |  |  |  |  |  |  |  |  |
| 120 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 |  |  |  |  |  |  |  |  |  |
| 121 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 |  |  |  |  |  |  |  |  |  |
| 122 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 |  |  |  |  |  |  |  |  |  |
| 123 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 |  |  |  |  |  |  |  |  |  |
| 124 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 |  |  |  |  |  |  |  |  |  |
| 125 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 |  |  |  |  |  |  |  |  |  |
| 126 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 |  |  |  |  |  |  |  |  |  |
| 127 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 |  |  |  |  |  |  |  |  |  |
| 128 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 |  |  |  |  |  |  |  |  |  |
| 129 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 |  |  |  |  |  |  |  |  |  |
| 130 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 |  |  |  |  |  |  |  |  |  |
| 131 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 |  |  |  |  |  |  |  |  |  |
| 132 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 |  |  |  |  |  |  |  |  |  |
| 133 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 |  |  |  |  |  |  |  |  |  |
| 134 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 |  |  |  |  |  |  |  |  |  |
| 135 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 |  |  |  |  |  |  |  |  |  |
| 136 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 137 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 138 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 139 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 |  |  |  |  |  |  |  |  |  |
| 140 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 |  |  |  |  |  |  |  |  |  |
| 141 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 |  |  |  |  |  |  |  |  |  |
| 142 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 |  |  |  |  |  |  |  |  |  |
| 143 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 |  |  |  |  |  |  |  |  |  |
| 144 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 |  |  |  |  |  |  |  |  |  |
| 145 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 |  |  |  |  |  |  |  |  |  |
| 146 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 |  |  |  |  |  |  |  |  |  |
| 147 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 |  |  |  |  |  |  |  |  |  |
| 148 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 |  |  |  |  |  |  |  |  |  |
| 149 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-073.mp3 |  |  |  |  |  |  |  |  |  |
| 150 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-091.mp3 |  |  |  |  |  |  |  |  |  |
| 151 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-127.mp3 |  |  |  |  |  |  |  |  |  |
| 152 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1044/j.mp3 |  |  |  |  |  |  |  |  |  |
| 153 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/09940204-8808-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 154 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/3659eb8c-80f6-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 155 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/6c92a34e-8cd9-11de-a52d-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 156 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/a3bcfb10-85dd-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 157 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/af2eb840-715f-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 158 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/b5e90f5c-7059-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 159 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/e0796ece-8bc5-11de-a52d-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 160 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/e6fe582c-8d5a-11de-a52d-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 161 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/ea08c0cc-63dc-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 162 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/fe339fd6-6c83-11de-883e-000423b32792.mp3 |  |  |  |  |  |  |  |  |  |
| 163 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1379/ashort.mp3 |  |  |  |  |  |  |  |  |  |
| 164 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1379_full/full_audio.mp3 |  |  |  |  |  |  |  |  |  |
| 165 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue445/22050.mp3 |  |  |  |  |  |  |  |  |  |
| 166 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue445/22050q.mp3 |  |  |  |  |  |  |  |  |  |
| 167 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1524/Have%20Yourself%20a%20Merry%20Little%20Christmas.mp3 |  |  |  |  |  |  |  |  |  |
| 168 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2377/small-sample-128-and-lossless-mp3HD.mp3 |  |  |  |  |  |  |  |  |  |
| 169 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2904/multiple_apics.mp3 |  |  |  |  |  |  |  |  |  |
| 170 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2931/1.mp3 |  |  |  |  |  |  |  |  |  |
| 171 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2931/Purity.mp3 |  |  |  |  |  |  |  |  |  |
| 172 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3095/bug3095-test-CBR.mp3 |  |  |  |  |  |  |  |  |  |
| 173 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3095/bug3095-test-VBR4.mp3 |  |  |  |  |  |  |  |  |  |
| 174 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3327/issue3327_2.mp3 |  |  |  |  |  |  |  |  |  |
| 175 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3327/sample.mp3 |  |  |  |  |  |  |  |  |  |
| 176 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3844/tuu_gekisinn.mp3 |  |  |  |  |  |  |  |  |  |
| 177 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3937/05._Du_hast.mp3 |  |  |  |  |  |  |  |  |  |
| 178 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket4003/mp3_demuxer_EOI.mp3 |  |  |  |  |  |  |  |  |  |
| 179 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket5741/defect_mp3.mp3 |  |  |  |  |  |  |  |  |  |
| 180 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket6532/test.mp3 |  |  |  |  |  |  |  |  |  |
| 181 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket7879/test.mp3 |  |  |  |  |  |  |  |  |  |
| 182 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket8511/OSC053.mp3 |  |  |  |  |  |  |  |  |  |
| 183 | https://samples.ffmpeg.org/karaoke/cgs.mp3 |  |  |  |  |  |  |  |  |  |
| 184 | https://samples.ffmpeg.org/karaoke/SC8932-15%20Gorillaz%20-%20Feel%20Good%20Inc%20%28Radio%20Version%29.mp3 |  |  |  |  |  |  |  |  |  |
| 185 | https://samples.ffmpeg.org/ogg/flac-in-ogg/yukina_lands_of_neverending_demo.ogg.mp3 |  |  |  |  |  |  |  |  |  |
