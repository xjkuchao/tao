//! MPEG-4 Part 2 解码集成测试
//!
//! 本测试验证 MPEG-4 Part 2 (ISO/IEC 14496-2) 解码器的完整流水：
//! - 容器解复用 (MP4/MKV/AVI/TS)
//! - VOL/VOP 头部解析
//! - I/P/B 帧解码
//! - field_dct / alternate scan 正确性
//! - data_partitioned / RVLC 兼容性
//! - 与 FFmpeg 参考输出对比

mod ffmpeg_compare;

#[cfg(test)]
mod tests {
    use crate::ffmpeg_compare::FfmpegComparer;
    use tao_codec::codec_id::CodecId;
    use tao_codec::codec_parameters::{CodecParameters, CodecParamsType, VideoCodecParams};
    use tao_codec::decoder::Decoder;
    use tao_codec::packet::Packet;
    use tao_core::PixelFormat;
    use tao_core::Rational;

    /// 创建 MPEG4 Part 2 解码器实例
    fn create_mpeg4_decoder() -> Box<dyn Decoder> {
        use tao_codec::decoders::mpeg4::Mpeg4Decoder;
        Mpeg4Decoder::create().expect("创建 MPEG4 解码器失败")
    }

    /// MPEG4 Part 2 解码器创建与基本打开测试
    #[test]
    fn test_mpeg4part2_decoder_create() {
        use tao_codec::decoders::mpeg4::Mpeg4Decoder;
        let decoder = Mpeg4Decoder::create();
        assert!(decoder.is_ok(), "应能创建 MPEG4 Part 2 解码器");
    }

    /// MPEG4 Part 2 解码器打开测试
    #[test]
    fn test_mpeg4part2_decoder_open() {
        let mut decoder = create_mpeg4_decoder();

        let params = CodecParameters {
            codec_id: CodecId::Mpeg4,
            bit_rate: 0,
            extra_data: vec![],
            params: CodecParamsType::Video(VideoCodecParams {
                width: 640,
                height: 480,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(30, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };

        let result = decoder.open(&params);
        assert!(result.is_ok(), "应能打开解码器");
    }

    /// MPEG4 Part 2 空包处理 (flush 信号)
    #[test]
    fn test_mpeg4part2_empty_packet() {
        let mut decoder = create_mpeg4_decoder();

        let params = CodecParameters {
            codec_id: CodecId::Mpeg4,
            bit_rate: 0,
            extra_data: vec![],
            params: CodecParamsType::Video(VideoCodecParams {
                width: 320,
                height: 240,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };
        decoder.open(&params).expect("打开解码器失败");

        let empty_packet = Packet::empty();
        let result = decoder.send_packet(&empty_packet);
        assert!(result.is_ok(), "空包应被安全处理 (flush 信号)");

        let frame = decoder.receive_frame();
        assert!(frame.is_err(), "flush 后不应返回帧");
    }

    /// 无效数据安全处理测试
    #[test]
    fn test_mpeg4part2_invalid_data() {
        let mut decoder = create_mpeg4_decoder();

        let params = CodecParameters {
            codec_id: CodecId::Mpeg4,
            bit_rate: 0,
            extra_data: vec![],
            params: CodecParamsType::Video(VideoCodecParams {
                width: 320,
                height: 240,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };
        decoder.open(&params).expect("打开解码器失败");

        // 无效的 VOP 起始码
        let invalid_packet = Packet::from_data(vec![0x00, 0x00, 0x01, 0x00]);
        let result = decoder.send_packet(&invalid_packet);
        // 应该安全处理，不崩溃
        let _ = result;
    }

    /// 多帧连续解码测试骨架
    ///
    /// 当存在样本文件时，此测试应读取真实 MPEG4 Part 2 帧流
    /// 并验证基本解码通过，无 panic。
    #[test]
    #[ignore]
    fn test_mpeg4part2_multi_frame_decode() {
        // TODO: 添加样本文件 path，验证多帧解码
        // - 正常流 (complete VOP headers, valid coefficients)
        // - 边界流 (minimal dimensions, special MB modes)
        // - 损坏流 (truncated packets, bit flips)
    }

    /// field_dct / alternate scan 正确性验证测试骨架
    ///
    /// 当存在隔行扫描样本时，应验证 DCT 系数扫描顺序正确应用。
    #[test]
    #[ignore]
    fn test_mpeg4part2_field_dct_alternate_scan() {
        // TODO: 需要采集或生成隔行扫描 + alternate_vertical_scan 的 MPEG4 Part 2 样本
        // 验证 block.rs 中 scan_table 的正确传递与使用
    }

    /// data_partitioned / RVLC 兼容性测试骨架
    ///
    /// 当存在数据分区 + RVLC 样本时，应验证警告输出并测试降级解码。
    #[test]
    #[ignore]
    fn test_mpeg4part2_data_partitioned_rvlc() {
        // TODO: 需要采集或生成带 data_partitioned + reversible_vlc 的 ASP 样本
        // 验证:
        // 1. VOL 中 data_partitioned/reversible_vlc 标志正确解析
        // 2. send_packet 中发出适当警告
        // 3. 解码不崩溃（尽管使用占位 RVLC 实现）
    }

    /// FFmpeg 对比测试框架骨架
    ///
    /// 建立与参考 FFmpeg 实现对比的基础：
    /// - 逐帧输出对比 (Y/U/V 平面像素级)
    /// - PSNR/差异统计
    /// - 支持多容器格式 (MP4/MKV/AVI/TS)
    #[test]
    #[ignore]
    fn test_mpeg4part2_vs_ffmpeg_reference() {
        println!("FFmpeg 对比测试框架演示\n");

        // 1. 检查 FFmpeg 可用性
        if !FfmpegComparer::check_ffmpeg_available() {
            println!("⚠️  FFmpeg 未安装，无法执行对比测试");
            println!("   请安装 FFmpeg: https://ffmpeg.org/download.html");
            return;
        }
        println!("✓ FFmpeg 已可用");

        // 2. 准备测试样本路径
        let sample_file = "data/samples/video/mpeg4_test.mp4";

        println!("\n待执行步骤:");
        println!("  1. 从 https://samples.ffmpeg.org/ 下载 MPEG4 Part 2 样本");
        println!("     保存路径: {}", sample_file);
        println!("  2. 使用 FFmpeg 生成参考输出");
        println!("  3. 使用 tao-codec 解码相同文件");
        println!("  4. 逐帧对比像素差异 (Y/U/V 平面)");
        println!("  5. 报告 PSNR 和差异统计");
        println!("  6. 验证解码质量 (PSNR >= 30 dB 为优秀)");

        println!("\n使用示例代码段:");
        println!("  // 创建对比器");
        println!("  let comparer = FfmpegComparer::new(sample_file, output_dir)?;");
        println!();
        println!("  // 获取媒体信息");
        println!("  let (w, h, fps) = comparer.get_video_info()?;");
        println!("  println!(\"视频分辨率: {{}}x{{}}, 帧率: {{:.2}} fps\", w, h, fps);");
        println!();
        println!("  // 生成 FFmpeg 参考输出");
        println!("  let ref_file = comparer.generate_reference_frames(5)?;");
        println!();
        println!("  // 用 tao 解码");
        println!("  let mut decoder = create_mpeg4_decoder();");
        println!("  decoder.open(&params)?;");
        println!();
        println!("  // 逐帧读取并对比");
        println!("  for frame_idx in 0..5 {{");
        println!("      let tao_frame = decoder.receive_frame()?;");
        println!("      let ref_frame = read_yuv420p_frame(&ref_file, frame_idx, w, h)?;");
        println!("      let diff = FrameDiff::compare(&tao_frame, &ref_frame, w, h)?;");
        println!("      println!(\"Frame {{}}: {{}}\", frame_idx, diff.summary());");
        println!("      assert!(diff.is_acceptable(), \"质量不达标\");");
        println!("  }}");

        println!("\n相关资源:");
        println!("  - 官方样本库: https://samples.ffmpeg.org/");
        println!("  - 样本清单: data/SAMPLES_MPEG4.md");
        println!("  - 下载脚本: data/download_samples.ps1");
        println!("  - 对比工具: tests/ffmpeg_compare.rs");
    }

    /// 各容器格式支持验证测试骨架
    #[test]
    #[ignore]
    fn test_mpeg4part2_containers_mp4_mkv_avi_ts() {
        // TODO: 为各容器创建最小测试样本，验证:
        // - MP4: 标准容器，应完整支持
        // - MKV: Matroska，V_MPEG4/ISO/ASP
        // - AVI: 含 packed B-frames (XVID/DIVX)，需兼容性覆盖
        // - TS: MPEG-TS，含 PES 封装
    }

    /// 错误恢复与统计测试骨架
    #[test]
    #[ignore]
    fn test_mpeg4part2_error_recovery_stats() {
        // TODO: 需要:
        // 1. 可控损坏流生成工具 (bit-flip injection, resync marker detection)
        // 2. 验证 resync marker 检测与帧级降级
        // 3. 统计可恢复帧比例、错误隐藏效果
    }
}
