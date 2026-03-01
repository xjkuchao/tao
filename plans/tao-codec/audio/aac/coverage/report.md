# AAC 解码器覆盖率测试报告

| 序号 | URL | 状态 | 失败原因 | Tao样本数 | FFmpeg样本数 | 样本数差异 | max_err | psnr(dB) | 精度(%) | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | https://samples.ffmpeg.org/A-codecs/AAC/2_aac_error_while_decoding_stream.aac | 成功 |  | 882000 | 882000 | 0 | 1.599771619 | 18.34 | 33.90 | 严格阈值未通过 |
| 2 | https://samples.ffmpeg.org/A-codecs/AAC/8_Channel_ID.m4a | 成功 |  | 3840000 | 3840000 | 0 | 0.507836133 | 36.04 | 34.45 | 严格阈值未通过 |
| 3 | https://samples.ffmpeg.org/A-codecs/AAC/Bandit.mp4 | 成功 |  | 240000 | 240000 | 0 | 0.000094605 | 126.01 | 100.00 |  |
| 4 | https://samples.ffmpeg.org/A-codecs/AAC/Music%20Station%20Super%20Live%20-%202011.12.23.mkv | 成功 |  | 960000 | 960448 | -448 | 0.732727826 | 22.06 | 49.04 | 严格阈值未通过 |
| 5 | https://samples.ffmpeg.org/A-codecs/AAC/aacPlusDecoderCheckPackage_v2.1/bitstreams/File1.aac | 成功 |  | 173056 | 173056 | 0 | 0.362938724 | 25.84 | 76.05 | 严格阈值未通过 |
| 6 | https://samples.ffmpeg.org/A-codecs/AAC/aacPlusDecoderCheckPackage_v2.1/bitstreams/File2.mp4 | 成功 |  | 173056 | 173056 | 0 | 0.363778077 | 25.78 | 75.81 | 严格阈值未通过 |
| 7 | https://samples.ffmpeg.org/A-codecs/AAC/aacPlusDecoderCheckPackage_v2.1/bitstreams/File3.mp4 | 成功 |  | 173056 | 173056 | 0 | 0.363778077 | 25.78 | 75.81 | 严格阈值未通过 |
| 8 | https://samples.ffmpeg.org/A-codecs/AAC/aacPlusDecoderCheckPackage_v2.1/bitstreams/File4.mp4 | 成功 |  | 173056 | 173056 | 0 | 0.363778077 | 25.78 | 75.81 | 严格阈值未通过 |
| 9 | https://samples.ffmpeg.org/A-codecs/AAC/aacPlusDecoderCheckPackage_v2.1/bitstreams/File5.mp4 | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=29, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 10 | https://samples.ffmpeg.org/A-codecs/AAC/channel_model/bad_concat.mp4 | 成功 |  | 441000 | 439296 | 1704 | 1.753169000 | 5.98 | 53.35 | 严格阈值未通过 |
| 11 | https://samples.ffmpeg.org/A-codecs/AAC/channel_model/elem_id0.mp4 | 成功 |  | 1548288 | 1548288 | 0 | 0.000147820 | 112.90 | 100.00 |  |
| 12 | https://samples.ffmpeg.org/A-codecs/AAC/channel_model/lfe_is_sce.mp4 | 成功 |  | 1548288 | 1548288 | 0 | 0.000147820 | 112.90 | 100.00 |  |
| 13 | https://samples.ffmpeg.org/A-codecs/AAC/ct_faac-adts.aac | 成功 |  | 882000 | 880640 | 1360 | 0.000150323 | 89.24 | 100.00 |  |
| 14 | https://samples.ffmpeg.org/A-codecs/AAC/ct_faac.mp4 | 成功 |  | 882000 | 882000 | 0 | 0.000150323 | 89.25 | 100.00 |  |
| 15 | https://samples.ffmpeg.org/A-codecs/AAC/ct_nero-heaac.mp4 | 成功 |  | 441000 | 444584 | -3584 | 0.096137241 | 41.36 | 99.88 | 严格阈值未通过 |
| 16 | https://samples.ffmpeg.org/A-codecs/AAC/faad2-fail.mkv | 成功 |  | 1167360 | 1167360 | 0 | 0.000000000 | inf | 100.00 |  |
| 17 | https://samples.ffmpeg.org/A-codecs/AAC/hulu-aac-main.flv | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=1, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 18 | https://samples.ffmpeg.org/A-codecs/AAC/mdct_error.flv | 成功 |  | 960000 | 800640 | 159360 | 0.534971878 | 23.52 | 32.45 | 严格阈值未通过 |
| 19 | https://samples.ffmpeg.org/A-codecs/AAC/ssr/Dailymotion_-_Los_Sucesos_de_Bagua_-_un_v_deo_de_Creaci_n.mp4 | 成功 |  | 882000 | 880640 | 1360 | 0.000000000 | inf | 100.00 |  |
| 20 | https://samples.ffmpeg.org/A-codecs/AAC/zodiac_audio.mov | 成功 |  | 2880000 | 2880000 | 0 | 0.890620679 | 26.43 | 32.10 | 严格阈值未通过 |
| 21 | https://samples.ffmpeg.org/A-codecs/AAC/zx.eva.renewal.01.divx511.mkv | 成功 |  | 2646282 | 2646282 | 0 | 0.000000000 | inf | 100.00 |  |
| 22 | https://samples.ffmpeg.org/A-codecs/lossless/ALAC/old_midi_stuff.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 23 | https://samples.ffmpeg.org/A-codecs/lossless/ALAC/quicktime-newcodec-applelosslessaudiocodec.m4a | 成功 |  | 868352 | 868352 | 0 | 0.000000000 | inf | 100.00 |  |
| 24 | https://samples.ffmpeg.org/A-codecs/lossless/ALAC/snoop_try.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 25 | https://samples.ffmpeg.org/A-codecs/lossless/luckynight.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 26 | https://samples.ffmpeg.org/A-codecs/suite/AAC+/36kbps_st_48kHz_eaac+_adts.aac | 成功 |  | 240000 | 240000 | 0 | 0.521270394 | 18.92 | 92.84 | 严格阈值未通过 |
| 27 | https://samples.ffmpeg.org/A-codecs/suite/AAC+/48kbps_st_44kHz_aac+_adts.aac | 成功 |  | 441000 | 441000 | 0 | 0.124883674 | 46.11 | 99.73 | 严格阈值未通过 |
| 28 | https://samples.ffmpeg.org/A-codecs/suite/AAC+/WishI-48kSBR.aac | 成功 |  | 441000 | 441000 | 0 | 0.235520873 | 30.74 | 99.06 | 严格阈值未通过 |
| 29 | https://samples.ffmpeg.org/A-codecs/suite/AAC+/WishI-48kSBRPS.aac | 成功 |  | 220500 | 220500 | 0 | 0.459623337 | 20.17 | 93.78 | 严格阈值未通过 |
| 30 | https://samples.ffmpeg.org/A-codecs/suite/AAC/Audio%20AAC/cooki-e2-a32sxx.mp4 | 成功 |  | 220500 | 220500 | 0 | 0.000095308 | 97.78 | 100.00 |  |
| 31 | https://samples.ffmpeg.org/A-codecs/suite/AAC/Audio%20AAC/cooki-e2-a64sxx.mp4 | 成功 |  | 220500 | 220500 | 0 | 0.000094652 | 97.61 | 100.00 |  |
| 32 | https://samples.ffmpeg.org/A-codecs/suite/AAC/Audio%20AAC/sample.PCM.16bit.8000Hz.Mono.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 33 | https://samples.ffmpeg.org/A-codecs/suite/AAC/Frula_Show_-_Gde_si_do_sad.aac | 失败 | AAC 对比失败: InvalidData("MP4 文件中未找到任何轨道") |  |  |  |  |  |  |  |
| 34 | https://samples.ffmpeg.org/A-codecs/suite/AAC/MPEG%20AAC/cooki-e2-m64s25-a32sxx.mp4 | 成功 |  | 220500 | 220500 | 0 | 0.000092149 | 97.78 | 100.00 |  |
| 35 | https://samples.ffmpeg.org/A-codecs/suite/AAC/MPEG%20AAC/xtrem-e2-m064q15-a16sxx.mp4 | 成功 |  | 110250 | 110250 | 0 | 0.000130653 | 94.18 | 100.00 |  |
| 36 | https://samples.ffmpeg.org/A-codecs/suite/AAC/aactestfile.aac | 成功 |  | 320000 | 319488 | 512 | 0.000102043 | 98.34 | 100.00 |  |
| 37 | https://samples.ffmpeg.org/A-codecs/suite/AAC/asimovis.aac | 失败 | AAC 对比失败: Eof |  |  |  |  |  |  |  |
| 38 | https://samples.ffmpeg.org/A-codecs/suite/AAC/particle20041116matrixd3_04_the_golden_gator_reprise.aac | 成功 |  | 882000 | 882000 | 0 | 0.000014991 | 111.21 | 100.00 |  |
| 39 | https://samples.ffmpeg.org/A-codecs/suite/AAC/sample-aac.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 40 | https://samples.ffmpeg.org/A-codecs/suite/AAC/sample-pcm-16bit-8khz-mono-aac.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 41 | https://samples.ffmpeg.org/A-codecs/suite/MP4A/MGPDEMOMP4.m4a | 成功 |  | 882000 | 882000 | 0 | 0.216793239 | 47.59 | 99.94 |  |
| 42 | https://samples.ffmpeg.org/A-codecs/suite/MP4A/motr_aac.m4a | 成功 |  | 441000 | 441000 | 0 | 0.000046670 | 110.48 | 100.00 |  |
| 43 | https://samples.ffmpeg.org/MPEG-4/218-adam-kessel/file_doesnt_work.m4a | 成功 |  | 882000 | 882000 | 0 | 1.224235862 | 12.65 | 34.93 | 严格阈值未通过 |
| 44 | https://samples.ffmpeg.org/MPEG-4/218-adam-kessel/file_works.m4a | 成功 |  | 882000 | 882000 | 0 | 1.224235862 | 12.65 | 34.93 | 严格阈值未通过 |
| 45 | https://samples.ffmpeg.org/archive/all/aac++aac++clip.faac.aac | 成功 |  | 960000 | 958464 | 1536 | 0.000051409 | 101.42 | 100.00 |  |
| 46 | https://samples.ffmpeg.org/archive/all/aac++aac++clip.menc.aac | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=1, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 47 | https://samples.ffmpeg.org/archive/all/aac++aac++uninit_condition_test.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 48 | https://samples.ffmpeg.org/archive/all/mov++alac++failes.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 49 | https://samples.ffmpeg.org/archive/all/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 50 | https://samples.ffmpeg.org/archive/all/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 51 | https://samples.ffmpeg.org/archive/audio/aac/aac++aac++clip.faac.aac | 成功 |  | 960000 | 958464 | 1536 | 0.000051409 | 101.42 | 100.00 |  |
| 52 | https://samples.ffmpeg.org/archive/audio/aac/aac++aac++clip.menc.aac | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=1, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 53 | https://samples.ffmpeg.org/archive/audio/aac/aac++aac++uninit_condition_test.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 54 | https://samples.ffmpeg.org/archive/audio/aac/avi+mpeg4+aac++black_smearing_with_ppc_altivec.mp4 | 失败 | AAC 对比失败: Unsupported("不支持的音频格式码: 0x00FF") |  |  |  |  |  |  |  |
| 55 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++Demo_FlagOfOurFathers.mov | 成功 |  | 652222 | 652222 | 0 | 0.000036150 | 101.41 | 100.00 |  |
| 56 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++H264memleak.mp4 | 成功 |  | 960000 | 958464 | 1536 | 0.000111759 | 92.96 | 100.00 |  |
| 57 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++bbc_1080p.mov | 成功 |  | 960000 | 960000 | 0 | 0.709571302 | 16.65 | 26.03 | 严格阈值未通过 |
| 58 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++devil_may_cry.mp4 | 成功 |  | 2880000 | 2880000 | 0 | 0.341924747 | 29.11 | 37.21 | 严格阈值未通过 |
| 59 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++eric.flv | 成功 |  | 882000 | 882000 | 0 | 0.000078321 | 105.05 | 100.00 |  |
| 60 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++itune_export.mov | 成功 |  | 135157 | 135157 | 0 | 0.007671233 | 71.05 | 100.00 |  |
| 61 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++mp42mp2garbled_sample.mp4 | 成功 |  | 960000 | 960000 | 0 | 0.000151753 | 91.43 | 100.00 |  |
| 62 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++mp4box_frag.mp4 | 成功 |  | 0 | 889168 | -889168 | 0.000000000 | inf | 0.00 | 严格阈值未通过 |
| 63 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++rb_07_mar_15_hd.mov | 成功 |  | 441000 | 441000 | 0 | 0.000162601 | 92.42 | 100.00 |  |
| 64 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++seekhang.mp4 | 成功 |  | 882000 | 880640 | 1360 | 0.000011504 | 119.58 | 100.00 |  |
| 65 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac++testcase2.flv | 成功 |  | 882000 | 882000 | 0 | 0.000118256 | 96.25 | 100.00 |  |
| 66 | https://samples.ffmpeg.org/archive/audio/aac/mov+h264+aac-sac3+mp4s+ETERNAL_cut.mp4 | 成功 |  | 2880000 | 2880000 | 0 | 0.000119090 | 106.86 | 100.00 |  |
| 67 | https://samples.ffmpeg.org/archive/audio/aac/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 68 | https://samples.ffmpeg.org/archive/audio/aac/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 69 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++15fps30000fps.mp4 | 成功 |  | 147456 | 147456 | 0 | 0.000022382 | 124.26 | 100.00 |  |
| 70 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++29fps1000fps.mp4 | 成功 |  | 480000 | 480000 | 0 | 0.000041753 | 113.18 | 100.00 |  |
| 71 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++AmigaTribute.mp4 | 成功 |  | 882000 | 882000 | 0 | 0.751981646 | 19.02 | 30.32 | 严格阈值未通过 |
| 72 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++aac_decoding_error.mp4 | 成功 |  | 220500 | 220500 | 0 | 0.209884934 | 46.82 | 99.92 |  |
| 73 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++broken_file.mp4 | 成功 |  | 2880000 | 2875392 | 4608 | 0.000004947 | 139.90 | 100.00 |  |
| 74 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++ffmpegx_wrong_ar.mp4 | 成功 |  | 960000 | 958464 | 1536 | 0.434516855 | 35.33 | 33.27 | 严格阈值未通过 |
| 75 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++framerate.mp4 | 成功 |  | 139264 | 140288 | -1024 | 0.000133634 | 96.04 | 100.00 |  |
| 76 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++mp4_noise_audio.mp4 | 成功 |  | 2880000 | 2880000 | 0 | 0.025522919 | 67.43 | 98.61 | 严格阈值未通过 |
| 77 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++trutter1_problem.mp4 | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=0, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 78 | https://samples.ffmpeg.org/archive/audio/aac/mov+mpeg4+aac++vfr.mp4 | 成功 |  | 147456 | 147456 | 0 | 0.000022382 | 124.26 | 100.00 |  |
| 79 | https://samples.ffmpeg.org/archive/audio/aac/mov+svq3+aac++animatrix_2_program_640-sample.mov | 失败 | AAC 对比失败: "未找到可解码音频流" |  |  |  |  |  |  |  |
| 80 | https://samples.ffmpeg.org/archive/audio/aac/mov+svq3+aac++t_starcraft2_teasecinv2_h264.mov | 成功 |  | 960000 | 960000 | 0 | 0.000127017 | 90.41 | 100.00 |  |
| 81 | https://samples.ffmpeg.org/archive/audio/alac/mov++alac++failes.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 82 | https://samples.ffmpeg.org/archive/container/aac/aac++aac++clip.faac.aac | 成功 |  | 960000 | 958464 | 1536 | 0.000051409 | 101.42 | 100.00 |  |
| 83 | https://samples.ffmpeg.org/archive/container/aac/aac++aac++clip.menc.aac | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=1, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 84 | https://samples.ffmpeg.org/archive/container/aac/aac++aac++uninit_condition_test.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 85 | https://samples.ffmpeg.org/archive/container/mov/mov++alac++failes.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 86 | https://samples.ffmpeg.org/archive/container/mov/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 87 | https://samples.ffmpeg.org/archive/container/mov/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 88 | https://samples.ffmpeg.org/archive/extension/aac/aac++aac++clip.faac.aac | 成功 |  | 960000 | 958464 | 1536 | 0.000051409 | 101.42 | 100.00 |  |
| 89 | https://samples.ffmpeg.org/archive/extension/aac/aac++aac++clip.menc.aac | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=1, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 90 | https://samples.ffmpeg.org/archive/extension/aac/aac++aac++uninit_condition_test.aac | 成功 |  | 35840 | 35840 | 0 | 0.083198296 | 49.44 | 99.95 |  |
| 91 | https://samples.ffmpeg.org/archive/extension/m4a/mov++alac++failes.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 92 | https://samples.ffmpeg.org/archive/extension/m4a/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 93 | https://samples.ffmpeg.org/archive/extension/m4a/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 94 | https://samples.ffmpeg.org/archive/subtitles/text/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 95 | https://samples.ffmpeg.org/archive/subtitles/text/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 96 | https://samples.ffmpeg.org/archive/subtitles/tx3g/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 97 | https://samples.ffmpeg.org/archive/subtitles/tx3g/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 98 | https://samples.ffmpeg.org/archive/video/mjpeg/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag-2.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 99 | https://samples.ffmpeg.org/archive/video/mjpeg/mov+mjpeg+aac+text-tx3g+jfb_podcast_stung_1-2-libicover-e000000-jpg-mktag.m4a | 成功 |  | 882000 | 882000 | 0 | 0.780053556 | 21.14 | 33.18 | 严格阈值未通过 |
| 100 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1254/lol-pce.m4a | 成功 |  | 661504 | 235520 | 425984 | 0.566354036 | 24.83 | 34.37 | 严格阈值未通过 |
| 101 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue1295/out0.m4a | 成功 |  | 292864 | 283648 | 9216 | 0.860311698 | 22.02 | 37.38 | 严格阈值未通过 |
| 102 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue2481/LTP2.aac | 失败 | AAC 对比失败: "读取 AAC 包失败: 无效数据: AAC: 无效的 ADTS 帧头部, 已处理包数=1" |  |  |  |  |  |  |  |
| 103 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue2481/LTP6.aac | 失败 | AAC 对比失败: Unsupported("AAC: 不支持 audioObjectType=3, 仅支持 AAC-LC (2)") |  |  |  |  |  |  |  |
| 104 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue483/aacchannel/aac-channel-conf.aac | 成功 |  | 441000 | 441000 | 0 | 0.000123084 | 100.56 | 100.00 |  |
| 105 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue662/neesa_wife_whoopie_1041754.aac | 成功 |  | 441000 | 440320 | 680 | 1.101099223 | 23.65 | 34.32 | 严格阈值未通过 |
| 106 | https://samples.ffmpeg.org/ffmpeg-bugs/roundup/issue853/aac_decode_failure.m4a | 成功 |  | 960000 | 960000 | 0 | 0.000136018 | 96.38 | 100.00 |  |
| 107 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1559/Nic%20Chagall%20-%20Get%20The%20Kicks%20Podcast%20001.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000133216 | 94.16 | 100.00 |  |
| 108 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1693/ssr_not_implemented_warning.aac | 成功 |  | 524288 | 493568 | 30720 | 0.518087827 | 27.19 | 47.92 | 严格阈值未通过 |
| 109 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1694/a.aac | 成功 |  | 220500 | 220500 | 0 | 0.204271436 | 37.36 | 91.78 | 严格阈值未通过 |
| 110 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket1730/FFMpeg_Bug_1730_crash_demuxing_m4a.m4a | 失败 | AAC 对比失败: "未找到可解码音频流" |  |  |  |  |  |  |  |
| 111 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2458/trac_8309_raw.aac | 失败 | AAC 对比失败: "读取 AAC 包失败: 无效数据: AAC: 无效的 ADTS 帧头部, 已处理包数=1" |  |  |  |  |  |  |  |
| 112 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket3312/ref.aac | 成功 |  | 299008 | 282624 | 16384 | 0.466102794 | 25.34 | 32.73 | 严格阈值未通过 |
| 113 | https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket5513/alac_20bit.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000000000 | inf | 100.00 |  |
| 114 | https://samples.ffmpeg.org/mov/audio_with_still.m4a | 成功 |  | 882000 | 882000 | 0 | 0.000131428 | 95.59 | 100.00 |  |
