//! AAC 编解码 + ADTS 封装器 + MP3 封装器 集成测试.
//!
//! 测试完整的 AAC 管线:
//! PCM 原始数据 → AAC Encoder → ADTS Muxer → ADTS 数据 (内存)
//! → AAC Demuxer → AAC Decoder → PCM 数据
//!
//! 以及 MP3 封装器的基本功能验证.

use tao::codec::{
    CodecId, CodecParameters, CodecRegistry,
    codec_parameters::{AudioCodecParams, CodecParamsType},
    frame::{AudioFrame, Frame},
};
use tao::core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError};
use tao::format::{
    FormatId, FormatRegistry, IoContext,
    io::MemoryBackend,
    stream::{AudioStreamParams, Stream, StreamParams},
};

// ============================================================
// 辅助函数
// ============================================================

/// 生成 F32 交错静音帧数据
fn generate_silence_f32(nb_samples: u32, channels: u32) -> Vec<u8> {
    vec![0u8; nb_samples as usize * channels as usize * 4]
}

/// 生成 F32 交错正弦波帧数据
fn generate_sine_f32(sample_rate: u32, freq: f64, nb_samples: u32, channels: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(nb_samples as usize * channels as usize * 4);
    for i in 0..nb_samples {
        let t = i as f64 / sample_rate as f64;
        let value = (t * freq * 2.0 * std::f64::consts::PI).sin() as f32;
        for _ in 0..channels {
            buf.extend_from_slice(&value.to_le_bytes());
        }
    }
    buf
}

/// 创建 AAC 音频流描述
fn make_aac_stream(sample_rate: u32, channels: u32) -> Stream {
    Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id: CodecId::Aac,
        time_base: Rational::new(1, sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::F32,
            bit_rate: 0,
            frame_size: 1024,
        }),
        metadata: Vec::new(),
    }
}

/// 创建 MP3 音频流描述
fn make_mp3_stream(sample_rate: u32, channels: u32) -> Stream {
    Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id: CodecId::Mp3,
        time_base: Rational::new(1, sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::F32,
            bit_rate: 128000,
            frame_size: 1152,
        }),
        metadata: Vec::new(),
    }
}

fn init_registries() -> (CodecRegistry, FormatRegistry) {
    let mut codec_reg = CodecRegistry::new();
    tao::codec::register_all(&mut codec_reg);

    let mut format_reg = FormatRegistry::new();
    tao::format::register_all(&mut format_reg);

    (codec_reg, format_reg)
}

// ============================================================
// AAC 编码器测试
// ============================================================

#[test]
fn test_aac_encoder_registration() {
    let (codec_reg, _) = init_registries();
    let encoder = codec_reg.create_encoder(CodecId::Aac);
    assert!(encoder.is_ok(), "AAC 编码器应已注册");
    assert_eq!(encoder.unwrap().codec_id(), CodecId::Aac);
}

#[test]
fn test_aac_decoder_registration() {
    let (codec_reg, _) = init_registries();
    let decoder = codec_reg.create_decoder(CodecId::Aac);
    assert!(decoder.is_ok(), "AAC 解码器应已注册");
    assert_eq!(decoder.unwrap().codec_id(), CodecId::Aac);
}

#[test]
fn test_aac_adts_muxer_registration() {
    let (_, format_reg) = init_registries();
    let muxer = format_reg.create_muxer(FormatId::AacAdts);
    assert!(muxer.is_ok(), "AAC ADTS 封装器应已注册");
}

#[test]
fn test_mp3_muxer_registration() {
    let (_, format_reg) = init_registries();
    let muxer = format_reg.create_muxer(FormatId::Mp3Container);
    assert!(muxer.is_ok(), "MP3 封装器应已注册");
}

#[test]
fn test_aac_encode_silence_frame() {
    let (codec_reg, _) = init_registries();

    let mut encoder = codec_reg.create_encoder(CodecId::Aac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: Vec::new(),
        bit_rate: 128000,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    encoder.open(&params).unwrap();

    // 创建静音帧
    let pcm_data = generate_silence_f32(1024, 2);
    let frame = AudioFrame {
        data: vec![pcm_data],
        nb_samples: 1024,
        sample_rate: 44100,
        channel_layout: ChannelLayout::from_channels(2),
        sample_format: SampleFormat::F32,
        pts: 0,
        time_base: Rational::new(1, 44100),
        duration: 1024,
    };

    encoder.send_frame(Some(&Frame::Audio(frame))).unwrap();
    let packet = encoder.receive_packet().unwrap();

    // 验证 ADTS sync word
    assert!(packet.data.len() >= 7, "数据包应至少包含 ADTS 头");
    assert_eq!(packet.data[0], 0xFF, "ADTS sync byte 0");
    assert_eq!(packet.data[1] & 0xF0, 0xF0, "ADTS sync byte 1");
}

#[test]
fn test_aac_codec_roundtrip_mute() {
    let (codec_reg, _) = init_registries();

    // === 编码 ===
    let mut encoder = codec_reg.create_encoder(CodecId::Aac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: Vec::new(),
        bit_rate: 128000,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    encoder.open(&params).unwrap();

    let pcm_data = generate_silence_f32(1024, 2);
    let frame = AudioFrame {
        data: vec![pcm_data],
        nb_samples: 1024,
        sample_rate: 44100,
        channel_layout: ChannelLayout::from_channels(2),
        sample_format: SampleFormat::F32,
        pts: 0,
        time_base: Rational::new(1, 44100),
        duration: 1024,
    };

    encoder.send_frame(Some(&Frame::Audio(frame))).unwrap();
    let encoded_packet = encoder.receive_packet().unwrap();

    // === 解码 ===
    let mut decoder = codec_reg.create_decoder(CodecId::Aac).unwrap();
    let dec_params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: vec![0x12, 0x10], // AAC-LC, 44100Hz, stereo
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    decoder.open(&dec_params).unwrap();

    decoder.send_packet(&encoded_packet).unwrap();
    let decoded_frame = decoder.receive_frame().unwrap();

    if let Frame::Audio(af) = decoded_frame {
        assert_eq!(af.nb_samples, 1024);
        assert_eq!(af.sample_rate, 44100);
        assert_eq!(af.channel_layout.channels, 2);
        assert_eq!(af.sample_format, SampleFormat::F32);

        // 由于 AAC 是有损编解码, 解码后的静音帧应接近零
        let samples: Vec<f32> = af.data[0]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let max_abs = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        // 对于静音输入, 允许有损编码引入的少量误差
        assert!(
            max_abs < 0.1,
            "静音帧解码后最大绝对值应 < 0.1, 实际={}",
            max_abs
        );
    } else {
        panic!("期望音频帧");
    }
}

#[test]
fn test_aac_adts_mux_write_and_read() {
    let (_, format_reg) = init_registries();

    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));

    let stream = make_aac_stream(44100, 2);
    let mut muxer = format_reg.create_muxer(FormatId::AacAdts).unwrap();

    muxer.write_header(&mut io, &[stream]).unwrap();

    // 写入模拟 AAC 原始帧
    let raw_aac = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
    let pkt = tao::codec::Packet::from_data(bytes::Bytes::from(raw_aac.clone()));
    muxer.write_packet(&mut io, &pkt).unwrap();

    muxer.write_trailer(&mut io).unwrap();

    // 验证输出: seek 回头并读取
    io.seek(std::io::SeekFrom::Start(0)).unwrap();

    // 第一个字节应该是 ADTS sync word
    let data = io.read_bytes(7 + raw_aac.len()).unwrap();
    assert_eq!(data[0], 0xFF, "ADTS sync byte 0");
    assert_eq!(data[1] & 0xF0, 0xF0, "ADTS sync nibble");

    // 原始数据应在 ADTS 头之后
    assert_eq!(&data[7..], &raw_aac[..], "原始 AAC 数据应在 ADTS 头之后");
}

#[test]
fn test_aac_full_pipeline_encode_mux_demux_decode() {
    let (codec_reg, format_reg) = init_registries();

    // === 1. 编码 ===
    let mut encoder = codec_reg.create_encoder(CodecId::Aac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: Vec::new(),
        bit_rate: 128000,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    encoder.open(&params).unwrap();

    // 编码两帧静音
    let mut encoded_packets = Vec::new();
    for i in 0..2 {
        let pcm_data = generate_silence_f32(1024, 2);
        let frame = AudioFrame {
            data: vec![pcm_data],
            nb_samples: 1024,
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            pts: i * 1024,
            time_base: Rational::new(1, 44100),
            duration: 1024,
        };
        encoder.send_frame(Some(&Frame::Audio(frame))).unwrap();
        let pkt = encoder.receive_packet().unwrap();
        encoded_packets.push(pkt);
    }

    // === 2. 封装 ===
    let backend = MemoryBackend::new();
    let mut io_write = IoContext::new(Box::new(backend));

    let stream = make_aac_stream(44100, 2);
    let mut muxer = format_reg.create_muxer(FormatId::AacAdts).unwrap();
    muxer.write_header(&mut io_write, &[stream]).unwrap();

    for pkt in &encoded_packets {
        muxer.write_packet(&mut io_write, pkt).unwrap();
    }
    muxer.write_trailer(&mut io_write).unwrap();

    // === 3. 解封装 ===
    io_write.seek(std::io::SeekFrom::Start(0)).unwrap();

    let mut demuxer = format_reg
        .open_input(&mut io_write, Some("test.aac"))
        .unwrap();
    let streams = demuxer.streams();
    assert!(!streams.is_empty(), "ADTS 应解析出至少一个流");

    let first_stream = &streams[0];
    assert_eq!(first_stream.codec_id, CodecId::Aac);

    // 读取数据包
    let mut demuxed_packets = Vec::new();
    loop {
        match demuxer.read_packet(&mut io_write) {
            Ok(pkt) => demuxed_packets.push(pkt),
            Err(TaoError::Eof) => break,
            Err(e) => {
                // ADTS demuxer 可能在最后返回其他错误
                eprintln!("解封装错误: {}", e);
                break;
            }
        }
    }
    assert!(!demuxed_packets.is_empty(), "应能从 ADTS 流中读到数据包");

    // === 4. 解码 ===
    let mut decoder = codec_reg.create_decoder(CodecId::Aac).unwrap();
    let dec_params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: vec![0x12, 0x10],
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    decoder.open(&dec_params).unwrap();

    let mut decoded_count = 0;
    for pkt in &demuxed_packets {
        if decoder.send_packet(pkt).is_ok() {
            while let Ok(frame) = decoder.receive_frame() {
                if let Frame::Audio(af) = frame {
                    assert_eq!(af.nb_samples, 1024);
                    assert_eq!(af.sample_rate, 44100);
                    decoded_count += 1;
                }
            }
        }
    }
    assert!(decoded_count > 0, "应成功解码至少一帧");
}

// ============================================================
// MP3 封装器测试
// ============================================================

#[test]
fn test_mp3_mux_passthrough() {
    let (_, format_reg) = init_registries();

    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));

    let stream = make_mp3_stream(44100, 2);
    let mut muxer = format_reg.create_muxer(FormatId::Mp3Container).unwrap();

    muxer.write_header(&mut io, &[stream]).unwrap();

    // 写入模拟 MP3 帧 (sync word 0xFF 0xFB)
    let mp3_frame = vec![0xFF, 0xFB, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00];
    let pkt = tao::codec::Packet::from_data(bytes::Bytes::from(mp3_frame.clone()));
    muxer.write_packet(&mut io, &pkt).unwrap();

    muxer.write_trailer(&mut io).unwrap();

    // 验证: MP3 裸流封装器直接透传数据
    let written_bytes = io.position().unwrap();
    assert!(written_bytes >= mp3_frame.len() as u64, "应至少写入帧数据");
}

#[test]
fn test_aac_encode_sine_wave() {
    let (codec_reg, _) = init_registries();

    let mut encoder = codec_reg.create_encoder(CodecId::Aac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: Vec::new(),
        bit_rate: 128000,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(1),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    };
    encoder.open(&params).unwrap();

    // 编码 440Hz 正弦波
    let pcm_data = generate_sine_f32(44100, 440.0, 1024, 1);
    let frame = AudioFrame {
        data: vec![pcm_data],
        nb_samples: 1024,
        sample_rate: 44100,
        channel_layout: ChannelLayout::from_channels(1),
        sample_format: SampleFormat::F32,
        pts: 0,
        time_base: Rational::new(1, 44100),
        duration: 1024,
    };

    encoder.send_frame(Some(&Frame::Audio(frame))).unwrap();
    let packet = encoder.receive_packet().unwrap();

    // ADTS 头校验
    assert!(packet.data.len() > 7, "编码后数据应大于 ADTS 头长度");
    assert_eq!(packet.data[0], 0xFF);
    assert_eq!(packet.data[1] & 0xF0, 0xF0);

    // 正弦波的编码数据应比静音帧大 (更多非零频谱)
    // 验证帧长度字段
    let frame_len = ((packet.data[3] as usize & 0x03) << 11)
        | ((packet.data[4] as usize) << 3)
        | ((packet.data[5] as usize) >> 5);
    assert_eq!(
        frame_len,
        packet.data.len(),
        "ADTS frame_length 应等于数据包总长"
    );
}
