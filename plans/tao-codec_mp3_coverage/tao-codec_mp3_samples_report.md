# tao-codec MP3 样本批量对比报告

字段说明:
- 状态: 成功或失败.
- 失败原因: 仅失败时填写.
- 样本数差异: Tao样本数-FFmpeg样本数.

| 序号 | URL | 状态 | 失败原因 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | https://samples.ffmpeg.org/A-codecs/MP3/01%20-%20Charity%20Case.mp3 | 成功 |  | 16972032 | 16972032 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 2 | https://samples.ffmpeg.org/A-codecs/MP3/ascii.mp3 | 成功 |  | 1396224 | 1398528 | -2304 | 0.000001 | 142.23 | 100.00 |  |
| 3 | https://samples.ffmpeg.org/A-codecs/MP3/Boot%20to%20the%20Head.MP3 | 成功 |  | 24047424 | 23795712 | 251712 | 12031.053010 | -31.36 | 1.27 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 4 | https://samples.ffmpeg.org/A-codecs/MP3/broken-first-frame.mp3 | 成功 |  | 373248 | 375552 | -2304 | 0.000000 | 163.89 | 100.00 |  |
| 5 | https://samples.ffmpeg.org/A-codecs/MP3/Die%20Jodelschule.mp3 | 失败 | MP3 对比失败: CodecNotFound("未找到 mp2 的解码器") | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 6 | https://samples.ffmpeg.org/A-codecs/MP3/Ed_Rush_-_Sabotage.mp3 | 成功 |  | 38787840 | 38787840 | 0 | 0.000004 | 132.88 | 100.00 |  |
| 7 | https://samples.ffmpeg.org/A-codecs/MP3/Enrique.mp3 | 成功 |  | 17756928 | 17756928 | 0 | 0.000004 | 133.54 | 100.00 |  |
| 8 | https://samples.ffmpeg.org/A-codecs/MP3/jpg_in_mp3.mp3 | 成功 |  | 19354608 | 19354608 | 0 | 0.000003 | 137.84 | 100.00 |  |  |  |  |  |  |  |
| 9 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_misidentified_2.mp3 | 成功 |  | 35647488 | 35647488 | 0 | 0.000004 | 133.43 | 100.00 |  |
| 10 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_misidentified.mp3 | 成功 |  | 19335168 | 19335168 | 0 | 0.000004 | 133.92 | 100.00 |  |
| 11 | https://samples.ffmpeg.org/A-codecs/MP3/mp3_with_embedded_albumart.mp3 | 成功 |  | 1347840 | 1350144 | -2304 | 0.000003 | 135.50 | 100.00 |  |  |  |  |  |  |  |
| 12 | https://samples.ffmpeg.org/A-codecs/MP3-pro/18%20Daft%20Punk%20-%20Harder%2C%20Better%2C%20Faster%2C%20Stronger.mp3 | 成功 |  | 9948672 | 9948672 | 0 | 2.160265 | 26.05 | 93.20 | 14.68 | 50.00 |  |
| 13 | https://samples.ffmpeg.org/A-codecs/MP3-pro/27%20MC%20Solaar%20-%20Rmi.mp3 | 成功 |  | 11472768 | 11472768 | 0 | 2.298207 | 27.77 | 96.77 |  |
| 14 | https://samples.ffmpeg.org/A-codecs/MP3-pro/scooter-wicked-02-imraving.mp3 | 成功 |  | 9191808 | 9191808 | 0 | 1.878174 | 25.84 | 93.75 |  |
| 15 | https://samples.ffmpeg.org/A-codecs/MP3/SegvMPlayer0.90.mp3 | 成功 |  | 388224 | 389376 | -1152 | 33708.218981 | -54.81 | 0.00 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 16 | https://samples.ffmpeg.org/A-codecs/MP3/Silent_Light.mp3 | 成功 |  | 23238144 | 23238144 | 0 | 0.000003 | 136.53 | 100.00 |  |
| 17 | https://samples.ffmpeg.org/A-codecs/MP3/%5Buran97_034%5D_02_dq_-_take_that.mp3 | 成功 |  | 20701440 | 20701440 | 0 | 0.000004 | 133.10 | 100.00 |  |
| 18 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz128kbps.mp3 | 成功 |  | 4870656 | 4870656 | 0 | 0.000001 | 138.03 | 100.00 |  |
| 19 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz32kbps.mp3 | 成功 |  | 4870656 | 4870656 | 0 | 0.000001 | 139.02 | 100.00 |  |
| 20 | https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz64kbps.mp3 | 成功 |  | 4870656 | 4870656 | 0 | 0.000001 | 138.15 | 100.00 |  |
| 21 | https://samples.ffmpeg.org/A-codecs/suite/MP3/bboys16.mp3 | 成功 |  | 597888 | 597888 | 0 | 0.273869 | 32.03 | 96.31 |  |
| 22 | https://samples.ffmpeg.org/A-codecs/suite/MP3/idtaggedcassidyhotel.mp3 | 成功 |  | 2122092 | 2122092 | 0 | 0.000002 | 139.36 | 100.00 |  |
| 23 | https://samples.ffmpeg.org/A-codecs/suite/MP3/piano2.mp3 | 成功 |  | 3679488 | 3679488 | 0 | 0.000002 | 147.56 | 100.00 |  |
| 24 | https://samples.ffmpeg.org/A-codecs/suite/MP3/piano.mp3 | 成功 |  | 3849984 | 3849984 | 0 | 0.000001 | 143.26 | 100.00 |  |
| 25 | https://samples.ffmpeg.org/A-codecs/suite/MP3/sample.VBR.32.64.44100Hz.Joint.mp3 | 成功 |  | 391680 | 390622 | 1058 | 0.708415 | 13.76 | 35.22 |  |
| 26 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track1.mp3 | 成功 |  | 1416960 | 1416960 | 0 | 1.236315 | 40.09 | 94.53 |  |
| 27 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track2.mp3 | 成功 |  | 1416960 | 1416960 | 0 | 0.338184 | 38.62 | 95.65 |  |
| 28 | https://samples.ffmpeg.org/A-codecs/suite/MP3/track3.mp3 | 成功 |  | 1416960 | 1416960 | 0 | 0.492340 | 37.99 | 98.22 |  |
| 29 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000073.mp3 | 成功 |  | 12637440 | 12637440 | 0 | 0.000002 | 141.68 | 100.00 |  |
| 30 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000091.mp3 | 成功 |  | 18196992 | 18196992 | 0 | 0.000001 | 143.12 | 100.00 |  |
| 31 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B00000127.mp3 | 成功 |  | 16743168 | 16743168 | 0 | 0.000002 | 139.39 | 100.00 |  |
| 32 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 | 成功 |  | 20877528 | 20877528 | 0 | 0.000003 | 133.99 | 100.00 |  |
| 33 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 | 成功 |  | 21657600 | 21657600 | 0 | 0.000004 | 137.07 | 100.00 |  |
| 34 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 | 成功 |  | 23081472 | 23081472 | 0 | 0.000002 | 140.90 | 100.00 |  |
| 35 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 | 成功 |  | 938880 | 938880 | 0 | 8.163809 | 21.29 | 89.96 |  |
| 36 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 | 成功 |  | 36417024 | 36417024 | 0 | 0.000003 | 138.49 | 100.00 |  |
| 37 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 | 成功 |  | 1456128 | 1458432 | -2304 | 0.000004 | 132.34 | 100.00 |  |
| 38 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bigla.mp3 | 成功 |  | 81571589 | 81571589 | 0 | 0.000001 | 151.80 | 100.00 |  |
| 39 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 | 成功 |  | 29776320 | 29776320 | 0 | 34.987791 | 20.97 | 22.92 |  |
| 40 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 | 成功 |  | 21605184 | 21605184 | 0 | 2.109698 | 23.50 | 36.15 |  |
| 41 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 | 成功 |  | 19888128 | 19888128 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 42 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 | 成功 |  | 357966 | 357966 | 0 | 119.071285 | -6.16 | 0.09 |  |
| 43 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 | 成功 |  | 720570 | 720570 | 0 | 0.000001 | 148.09 | 100.00 |  |
| 44 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 | 成功 |  | 116250 | 116250 | 0 | 66.804523 | -4.87 | 0.12 |  |
| 45 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 | 成功 |  | 22060800 | 22060800 | 0 | 0.000004 | 136.64 | 100.00 |  |
| 46 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.307092 | 33.53 | 95.12 |  |
| 47 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.297041 | 38.52 | 98.40 |  |
| 48 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.07 | 100.00 |  |
| 49 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.12 | 100.00 |  |
| 50 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.19 | 100.00 |  |
| 51 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 | 成功 |  | 6500736 | 6500736 | 0 | 1.695027 | 29.82 | 98.28 |  |
| 52 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 | 成功 |  | 3753216 | 3755520 | -2304 | 0.000003 | 134.45 | 100.00 |  |
| 53 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 | 成功 |  | 14602752 | 14602752 | 0 | 0.000001 | 142.02 | 100.00 |  |
| 54 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 | 成功 |  | 28251648 | 28251648 | 0 | 0.000004 | 133.09 | 100.00 |  |
| 55 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 | 成功 |  | 21685248 | 21685248 | 0 | 0.000004 | 132.89 | 100.00 |  |
| 56 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2Btakethat.mp3 | 成功 |  | 563328 | 564480 | -1152 | 0.283824 | 30.78 | 38.71 |  |
| 57 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 | 成功 |  | 18432 | 18432 | 0 | 0.149729 | 43.56 | 94.93 |  |
| 58 | https://samples.ffmpeg.org/archive/all/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 | 成功 |  | 14208768 | 14208768 | 0 | 0.000001 | 148.75 | 100.00 |  |
| 59 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 | 成功 |  | 12637440 | 12637440 | 0 | 0.000002 | 141.68 | 100.00 |  |
| 60 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 | 成功 |  | 18196992 | 18196992 | 0 | 0.000001 | 143.12 | 100.00 |  |
| 61 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 | 成功 |  | 16743168 | 16743168 | 0 | 0.000002 | 139.39 | 100.00 |  |
| 62 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 | 成功 |  | 20877528 | 20877528 | 0 | 0.000003 | 133.99 | 100.00 |  |
| 63 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 | 成功 |  | 21657600 | 21657600 | 0 | 0.000004 | 137.07 | 100.00 |  |
| 64 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 | 成功 |  | 23081472 | 23081472 | 0 | 0.000002 | 140.90 | 100.00 |  |
| 65 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 | 成功 |  | 938880 | 938880 | 0 | 8.163809 | 21.29 | 89.96 |  |
| 66 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 | 成功 |  | 36417024 | 36417024 | 0 | 0.000003 | 138.49 | 100.00 |  |
| 67 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 | 成功 |  | 1456128 | 1458432 | -2304 | 0.000004 | 132.34 | 100.00 |  |
| 68 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 | 成功 |  | 81571589 | 81571589 | 0 | 0.000001 | 151.80 | 100.00 |  |
| 69 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 | 成功 |  | 29776320 | 29776320 | 0 | 34.987791 | 20.97 | 22.92 |  |
| 70 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 | 成功 |  | 21605184 | 21605184 | 0 | 2.109698 | 23.50 | 36.15 |  |
| 71 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 | 成功 |  | 19888128 | 19888128 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 72 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 | 成功 |  | 357966 | 357966 | 0 | 119.071285 | -6.16 | 0.09 |  |
| 73 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 | 成功 |  | 720570 | 720570 | 0 | 0.000001 | 148.09 | 100.00 |  |
| 74 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 | 成功 |  | 116250 | 116250 | 0 | 66.804523 | -4.87 | 0.12 |  |
| 75 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 | 成功 |  | 22060800 | 22060800 | 0 | 0.000004 | 136.64 | 100.00 |  |
| 76 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.307092 | 33.53 | 95.12 |  |
| 77 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.297041 | 38.52 | 98.40 |  |
| 78 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.07 | 100.00 |  |
| 79 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.12 | 100.00 |  |
| 80 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.19 | 100.00 |  |
| 81 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 | 成功 |  | 6500736 | 6500736 | 0 | 1.695027 | 29.82 | 98.28 |  |
| 82 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 | 成功 |  | 3753216 | 3755520 | -2304 | 0.000003 | 134.45 | 100.00 |  |
| 83 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 | 成功 |  | 14602752 | 14602752 | 0 | 0.000001 | 142.02 | 100.00 |  |
| 84 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 | 成功 |  | 28251648 | 28251648 | 0 | 0.000004 | 133.09 | 100.00 |  |
| 85 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 | 成功 |  | 21685248 | 21685248 | 0 | 0.000004 | 132.89 | 100.00 |  |
| 86 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 | 成功 |  | 563328 | 564480 | -1152 | 0.283824 | 30.78 | 38.71 |  |
| 87 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 | 成功 |  | 18432 | 18432 | 0 | 0.149729 | 43.56 | 94.93 |  |
| 88 | https://samples.ffmpeg.org/archive/audio/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 | 成功 |  | 14208768 | 14208768 | 0 | 0.000001 | 148.75 | 100.00 |  |
| 89 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 | 成功 |  | 12637440 | 12637440 | 0 | 0.000002 | 141.68 | 100.00 |  |
| 90 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 | 成功 |  | 18196992 | 18196992 | 0 | 0.000001 | 143.12 | 100.00 |  |
| 91 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 | 成功 |  | 16743168 | 16743168 | 0 | 0.000002 | 139.39 | 100.00 |  |
| 92 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 | 成功 |  | 20877528 | 20877528 | 0 | 0.000003 | 133.99 | 100.00 |  |
| 93 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 | 成功 |  | 21657600 | 21657600 | 0 | 0.000004 | 137.07 | 100.00 |  |
| 94 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 | 成功 |  | 23081472 | 23081472 | 0 | 0.000002 | 140.90 | 100.00 |  |
| 95 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 | 成功 |  | 938880 | 938880 | 0 | 8.163809 | 21.29 | 89.96 |  |
| 96 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 | 成功 |  | 36417024 | 36417024 | 0 | 0.000003 | 138.49 | 100.00 |  |
| 97 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 | 成功 |  | 1456128 | 1458432 | -2304 | 0.000004 | 132.34 | 100.00 |  |
| 98 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 | 成功 |  | 81571589 | 81571589 | 0 | 0.000001 | 151.80 | 100.00 |  |
| 99 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 | 成功 |  | 29776320 | 29776320 | 0 | 34.987791 | 20.97 | 22.92 |  |
| 100 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 | 成功 |  | 21605184 | 21605184 | 0 | 2.109698 | 23.50 | 36.15 |  |
| 101 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 | 成功 |  | 19888128 | 19888128 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 102 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 | 成功 |  | 357966 | 357966 | 0 | 119.071285 | -6.16 | 0.09 |  |
| 103 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 | 成功 |  | 720570 | 720570 | 0 | 0.000001 | 148.09 | 100.00 |  |
| 104 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 | 成功 |  | 116250 | 116250 | 0 | 66.804523 | -4.87 | 0.12 |  |
| 105 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 | 成功 |  | 22060800 | 22060800 | 0 | 0.000004 | 136.64 | 100.00 |  |
| 106 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.307092 | 33.53 | 95.12 |  |
| 107 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.297041 | 38.52 | 98.40 |  |
| 108 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.07 | 100.00 |  |
| 109 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.12 | 100.00 |  |
| 110 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.19 | 100.00 |  |
| 111 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 | 成功 |  | 6500736 | 6500736 | 0 | 1.695027 | 29.82 | 98.28 |  |
| 112 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 | 成功 |  | 3753216 | 3755520 | -2304 | 0.000003 | 134.45 | 100.00 |  |
| 113 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 | 成功 |  | 14602752 | 14602752 | 0 | 0.000001 | 142.02 | 100.00 |  |
| 114 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 | 成功 |  | 28251648 | 28251648 | 0 | 0.000004 | 133.09 | 100.00 |  |
| 115 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 | 成功 |  | 21685248 | 21685248 | 0 | 0.000004 | 132.89 | 100.00 |  |
| 116 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 | 成功 |  | 563328 | 564480 | -1152 | 0.283824 | 30.78 | 38.71 |  |
| 117 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 | 成功 |  | 18432 | 18432 | 0 | 0.149729 | 43.56 | 94.93 |  |
| 118 | https://samples.ffmpeg.org/archive/container/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 | 成功 |  | 14208768 | 14208768 | 0 | 0.000001 | 148.75 | 100.00 |  |
| 119 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000073.mp3 | 成功 |  | 12637440 | 12637440 | 0 | 0.000002 | 141.68 | 100.00 |  |
| 120 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000091.mp3 | 成功 |  | 18196992 | 18196992 | 0 | 0.000001 | 143.12 | 100.00 |  |
| 121 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B00000127.mp3 | 成功 |  | 16743168 | 16743168 | 0 | 0.000002 | 139.39 | 100.00 |  |
| 122 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2B07-smash_mouth-aint_no_mystery-apc.mp3 | 成功 |  | 20877528 | 20877528 | 0 | 0.000003 | 133.99 | 100.00 |  |
| 123 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BAvril%20Lavigne%20-%20Complicated.mp3 | 成功 |  | 21657600 | 21657600 | 0 | 0.000004 | 137.07 | 100.00 |  |
| 124 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BChrono_Trigger_Temporal_Distortion_OC_ReMix.mp3 | 成功 |  | 23081472 | 23081472 | 0 | 0.000002 | 140.90 | 100.00 |  |
| 125 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bcould_not_find_codec_params.mp3 | 成功 |  | 938880 | 938880 | 0 | 8.163809 | 21.29 | 89.96 |  |
| 126 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BEagles-Hotel_Californa.mp3 | 成功 |  | 36417024 | 36417024 | 0 | 0.000003 | 138.49 | 100.00 |  |
| 127 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bffmpeg_bad_header.mp3 | 成功 |  | 1456128 | 1458432 | -2304 | 0.000004 | 132.34 | 100.00 |  |
| 128 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bigla.mp3 | 成功 |  | 81571589 | 81571589 | 0 | 0.000001 | 151.80 | 100.00 |  |
| 129 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_final.mp3 | 成功 |  | 29776320 | 29776320 | 0 | 34.987791 | 20.97 | 22.92 |  |
| 130 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3_bug_original.mp3 | 成功 |  | 21605184 | 21605184 | 0 | 2.109698 | 23.50 | 36.15 |  |
| 131 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3could_not_find_codec_parameters.mp3 | 成功 |  | 19888128 | 19888128 | 0 | 0.000004 | 135.34 | 100.00 |  |
| 132 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch24.160.mp3 | 成功 |  | 357966 | 357966 | 0 | 119.071285 | -6.16 | 0.09 |  |
| 133 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch48.320.mp3 | 成功 |  | 720570 | 720570 | 0 | 0.000001 | 148.09 | 100.00 |  |
| 134 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3glitch8.64.mp3 | 成功 |  | 116250 | 116250 | 0 | 66.804523 | -4.87 | 0.12 |  |
| 135 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3hang_after_few_seconds.mp3 | 成功 |  | 22060800 | 22060800 | 0 | 0.000004 | 136.64 | 100.00 |  |
| 136 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR40kbps_%28minCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.307092 | 33.53 | 95.12 |  |
| 137 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_CBR96kbps_%28maxCBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.297041 | 38.52 | 98.40 |  |
| 138 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR50-60kbps_%28minVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.07 | 100.00 |  |
| 139 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR65-85kbps.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.12 | 100.00 |  |
| 140 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3pro_VBR95-150kbps_%28maxVBR%29.mp3 | 成功 |  | 22051584 | 22051584 | 0 | 0.000001 | 141.19 | 100.00 |  |
| 141 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Bmp3seek_does_not_work.mp3 | 成功 |  | 6500736 | 6500736 | 0 | 1.695027 | 29.82 | 98.28 |  |
| 142 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Boskar-20021226-mp3mpegps.mp3 | 成功 |  | 3753216 | 3755520 | -2304 | 0.000003 | 134.45 | 100.00 |  |
| 143 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BPeqGesto.mp3 | 成功 |  | 14602752 | 14602752 | 0 | 0.000001 | 142.02 | 100.00 |  |
| 144 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2003%20-%20Unafraid%20%28Paul%20Oakenfold%20Mix%29.mp3 | 成功 |  | 28251648 | 28251648 | 0 | 0.000004 | 133.09 | 100.00 |  |
| 145 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BSwordfish%20-%2015%20-%20Get%20Out%20Of%20My%20Life%20Now.mp3 | 成功 |  | 21685248 | 21685248 | 0 | 0.000004 | 132.89 | 100.00 |  |
| 146 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2Btakethat.mp3 | 成功 |  | 563328 | 564480 | -1152 | 0.283824 | 30.78 | 38.71 |  |
| 147 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BtooSmallFinal.mp3 | 成功 |  | 18432 | 18432 | 0 | 0.149729 | 43.56 | 94.93 |  |
| 148 | https://samples.ffmpeg.org/archive/extension/mp3/mp3%2B%2Bmp3%2B%2BtooSmallOrig.mp3 | 成功 |  | 14208768 | 14208768 | 0 | 0.000001 | 148.75 | 100.00 |  |
| 149 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-073.mp3 | 成功 |  | 12637440 | 12637440 | 0 | 0.000002 | 141.68 | 100.00 |  |
| 150 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-091.mp3 | 成功 |  | 18196992 | 18196992 | 0 | 0.000001 | 143.12 | 100.00 |  |
| 151 | https://samples.ffmpeg.org/ffmpeg-bugs/id3v1_tag_inside_last_frame/id3v1_tag_inside_last_frame-127.mp3 | 成功 |  | 16743168 | 16743168 | 0 | 0.000002 | 139.39 | 100.00 |  |
| 152 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1044/j.mp3 | 成功 |  | 11520 | 13824 | -2304 | 0.324870 | 22.07 | 24.58 |  |
| 153 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/09940204-8808-11de-883e-000423b32792.mp3 | 成功 |  | 42722212 | 42722212 | 0 | 1.977055 | 8.73 | 32.60 |  |
| 154 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/3659eb8c-80f6-11de-883e-000423b32792.mp3 | 成功 |  | 41013000 | 41013000 | 0 | 2.072027 | 6.93 | 34.50 |  |  |  |  |  |  |  |
| 155 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/6c92a34e-8cd9-11de-a52d-000423b32792.mp3 | 成功 |  | 37926000 | 37926000 | 0 | 0.000003 | 135.94 | 100.00 |  |
| 156 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/a3bcfb10-85dd-11de-883e-000423b32792.mp3 | 成功 |  | 39044126 | 39044126 | 0 | 0.000005 | 133.01 | 100.00 |  |
| 157 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/af2eb840-715f-11de-883e-000423b32792.mp3 | 成功 |  | 39969792 | 39969792 | 0 | 0.000004 | 136.01 | 100.00 |  |
| 158 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/b5e90f5c-7059-11de-883e-000423b32792.mp3 | 成功 |  | 22740480 | 22742784 | -2304 | 0.000003 | 135.80 | 100.00 |  |  |  |  |  |  |  |
| 159 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/e0796ece-8bc5-11de-a52d-000423b32792.mp3 | 成功 |  | 43340304 | 16128 | 43324176 | 0.159135 | 31.51 | 50.08 |  |
| 160 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/e6fe582c-8d5a-11de-a52d-000423b32792.mp3 | 成功 |  | 37757848 | 122112 | 37635736 | 1.288243 | 8.91 | 44.08 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 161 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/ea08c0cc-63dc-11de-883e-000423b32792.mp3 | 成功 |  | 42921648 | 42921648 | 0 | 0.000010 | 135.63 | 100.00 |  |  |  |  |  |  |  |
| 162 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1331/fe339fd6-6c83-11de-883e-000423b32792.mp3 | 成功 |  | 40656000 | 64512 | 40591488 | 0.950691 | 21.13 | 50.00 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 163 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1379/ashort.mp3 | 成功 |  | 96763486 | 96763486 | 0 | 0.000008 | 139.58 | 100.00 |  |
| 164 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1379_full/full_audio.mp3 | 成功 |  | 239544576 | 239544576 | 0 | 0.000003 | 144.49 | 100.00 |  |
| 165 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue445/22050.mp3 | 成功 |  | 4785408 | 4785408 | 0 | 0.945175 | 43.47 | 33.94 |  |
| 166 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue445/22050q.mp3 | 成功 |  | 4785408 | 4785408 | 0 | 6.600145 | 34.84 | 13.28 |  |
| 167 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1524/Have%20Yourself%20a%20Merry%20Little%20Christmas.mp3 | 成功 |  | 16062408 | 16062408 | 0 | 1.523345 | 14.93 | 32.39 |  |
| 168 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2377/small-sample-128-and-lossless-mp3HD.mp3 | 成功 |  | 12172032 | 12172032 | 0 | 0.000001 | 143.31 | 100.00 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 169 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2904/multiple_apics.mp3 | 成功 |  | 884958 | 887040 | -2082 | 1.331414 | 13.24 | 35.07 |  |  |  |  |  |  |  |
| 170 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2931/1.mp3 | 成功 |  | 21431808 | 21431808 | 0 | 0.000004 | 132.89 | 100.00 |  |  |  |  |  |  |  |
| 171 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2931/Purity.mp3 | 成功 |  | 7061760 | 7061760 | 0 | 0.000002 | 138.47 | 100.00 |  |  |  |  |  |  |  |
| 172 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3095/bug3095-test-CBR.mp3 | 成功 |  | 50498534 | 50498534 | 0 | 0.000008 | 132.87 | 100.00 |  |
| 173 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3095/bug3095-test-VBR4.mp3 | 成功 |  | 50498534 | 50498534 | 0 | 0.000008 | 132.59 | 100.00 |  |
| 174 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3327/issue3327_2.mp3 | 成功 |  | 25712640 | 25712640 | 0 | 0.000006 | 132.69 | 100.00 |  |  |  |  |  |  |  |
| 175 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3327/sample.mp3 | 成功 |  | 1617408 | 1619712 | -2304 | 0.000002 | 141.27 | 100.00 |  |  |  |  |  |  |  |
| 176 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3844/tuu_gekisinn.mp3 | 成功 |  | 26424576 | 26424576 | 0 | 0.000004 | 132.10 | 100.00 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 177 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3937/05._Du_hast.mp3 | 成功 |  | 20802192 | 20802192 | 0 | 0.000003 | 133.97 | 100.00 | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
| 178 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket4003/mp3_demuxer_EOI.mp3 | 成功 |  | 18770688 | 18770688 | 0 | 0.000004 | 134.33 | 100.00 |  |
| 179 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket5741/defect_mp3.mp3 | 成功 |  | 20759176 | 20759176 | 0 | 0.000005 | 135.39 | 100.00 |  |  |  |  |  |  |  |
| 180 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket6532/test.mp3 | 成功 |  | 27174528 | 27177984 | -3456 | 8.567530 | 13.42 | 32.94 |  |  |  |  |  |  |  |
| 181 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket7879/test.mp3 | 成功 |  | 20251864 | 20251864 | 0 | 0.000005 | 132.63 | 100.00 |  |
| 182 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket8511/OSC053.mp3 | 成功 |  | 490480128 | 490480128 | 0 | 0.000004 | 141.33 | 100.00 |  |  |  |  |  |  |  |
| 183 | https://samples.ffmpeg.org/karaoke/cgs.mp3 | 成功 |  | 16906752 | 16906752 | 0 | 0.000005 | 135.06 | 100.00 |  |
| 184 | https://samples.ffmpeg.org/karaoke/SC8932-15%20Gorillaz%20-%20Feel%20Good%20Inc%20%28Radio%20Version%29.mp3 | 成功 |  | 19761408 | 19761408 | 0 | 0.000003 | 139.12 | 100.00 |  |
| 185 | https://samples.ffmpeg.org/ogg/flac-in-ogg/yukina_lands_of_neverending_demo.ogg.mp3 | 失败 | MP3 对比失败: InvalidData("无效的 FLAC 同步码: 0x2100") | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` | note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace | error: test failed, to rerun pass `--test mp3_module_compare` |  |  |  |  |  |  |  |
