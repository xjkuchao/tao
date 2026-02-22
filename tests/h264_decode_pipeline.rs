//! H.264/AVC 视频解码集成测试
//!
//! 验证 H.264 解码器的基础功能:
//! - SPS/PPS 参数解析
//! - 解码器创建与打开
//! - 空包与无效数据安全处理

#[cfg(test)]
mod tests {
    use tao_codec::codec_id::CodecId;
    use tao_codec::codec_parameters::{CodecParameters, CodecParamsType, VideoCodecParams};
    use tao_codec::decoder::Decoder;
    use tao_codec::packet::Packet;
    use tao_core::PixelFormat;
    use tao_core::Rational;

    /// 创建 H.264 解码器实例
    fn create_h264_decoder() -> Box<dyn Decoder> {
        use tao_codec::decoders::h264::H264Decoder;
        H264Decoder::create().expect("创建 H.264 解码器失败")
    }

    /// H.264 解码器创建测试
    #[test]
    fn test_h264_decoder_create() {
        use tao_codec::decoders::h264::H264Decoder;
        let decoder = H264Decoder::create();
        assert!(decoder.is_ok(), "应能创建 H.264 解码器");
    }

    /// H.264 解码器打开测试
    #[test]
    fn test_h264_decoder_open() {
        let mut decoder = create_h264_decoder();

        // AVCC 格式配置：SPS 和 PPS
        let mut extra_data = Vec::new();
        // AVCC 头: version (1) + profile (1) + profile_compat (1) + level (1)
        extra_data.extend_from_slice(&[0x01, 0x42, 0x00, 0x1E]);
        // length_size_minus_1 (6 bits) + reserved (2 bits) = 0xFF (length_size=4)
        extra_data.push(0xFF);
        // num_sps (5 bits reserved + 3 bits count)
        extra_data.push(0xE1); // 1 SPS
        // SPS 长度 (2 bytes, big-endian)
        extra_data.extend_from_slice(&[0x00, 0x04]);
        // SPS: 0x67 (NAL type 7) + 简单参数
        extra_data.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E]);
        // num_pps
        extra_data.push(0x01); // 1 PPS
        // PPS 长度
        extra_data.extend_from_slice(&[0x00, 0x02]);
        // PPS: 0x68 (NAL type 8) + 简单参数
        extra_data.extend_from_slice(&[0x68, 0xCE]);

        let params = CodecParameters {
            codec_id: CodecId::H264,
            bit_rate: 0,
            extra_data,
            params: CodecParamsType::Video(VideoCodecParams {
                width: 640,
                height: 480,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(30, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };

        let result = decoder.open(&params);
        assert!(result.is_ok(), "应能打开解码器（即使参数有限）");
    }

    /// 空包处理测试 (flush 信号)
    #[test]
    fn test_h264_empty_packet() {
        let mut decoder = create_h264_decoder();

        let params = CodecParameters {
            codec_id: CodecId::H264,
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
    fn test_h264_invalid_data() {
        let mut decoder = create_h264_decoder();

        let params = CodecParameters {
            codec_id: CodecId::H264,
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

        // 无效的 NAL 单元
        let invalid_packet = Packet::from_data(vec![0x00, 0x00, 0x00, 0x01, 0x00]);
        let result = decoder.send_packet(&invalid_packet);
        // 应该安全处理，不崩溃
        let _ = result;
    }

}
