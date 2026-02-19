//! FLAC 编码 + 封装 + 解封装 + 解码 无损往返集成测试.
//!
//! 测试完整的 FLAC 编码管线:
//! PCM 原始数据 → FLAC Encoder → FLAC Muxer → FLAC 文件 (内存)
//! → FLAC Demuxer → FLAC Decoder → PCM 原始数据
//! 验证输入输出完全一致 (无损).

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

/// 生成小端 S16 正弦波 PCM 数据
fn generate_sine_s16le(sample_rate: u32, freq: f64, nb_samples: u32, channels: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(nb_samples as usize * channels as usize * 2);
    for i in 0..nb_samples {
        let t = i as f64 / sample_rate as f64;
        let value = (t * freq * 2.0 * std::f64::consts::PI).sin();
        let sample = (value * 16000.0) as i16;
        for _ in 0..channels {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
    }
    buf
}

/// 生成小端 S16 全零 PCM 数据
fn generate_silence_s16le(nb_samples: u32, channels: u32) -> Vec<u8> {
    vec![0u8; nb_samples as usize * channels as usize * 2]
}

/// 生成小端 S16 递增锯齿波 PCM 数据
fn generate_sawtooth_s16le(nb_samples: u32, channels: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(nb_samples as usize * channels as usize * 2);
    for i in 0..nb_samples {
        let sample = (i as i16).wrapping_mul(100);
        for _ in 0..channels {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
    }
    buf
}

/// FLAC 编码管线: PCM → encode → mux → 返回内存中的 FLAC 数据
fn encode_to_flac_memory(
    codec_registry: &CodecRegistry,
    format_registry: &FormatRegistry,
    pcm_data: &[u8],
    sample_rate: u32,
    channels: u32,
    block_size: u32,
) -> IoContext {
    // 创建编码器
    let mut encoder = codec_registry.create_encoder(CodecId::Flac).unwrap();
    let enc_params = CodecParameters {
        codec_id: CodecId::Flac,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::S16,
            frame_size: block_size,
        }),
    };
    encoder.open(&enc_params).unwrap();

    // 创建封装器
    let mut muxer = format_registry
        .create_muxer(FormatId::FlacContainer)
        .unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));

    let stream = Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id: CodecId::Flac,
        time_base: Rational::new(1, sample_rate as i32),
        duration: -1,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::S16,
            bit_rate: 0,
            frame_size: block_size,
        }),
        metadata: Vec::new(),
    };
    muxer.write_header(&mut io, &[stream]).unwrap();

    // 分块编码
    let bytes_per_sample = 2 * channels as usize;
    let block_bytes = block_size as usize * bytes_per_sample;
    let mut sample_offset = 0u32;

    let mut offset = 0;
    while offset < pcm_data.len() {
        let end = (offset + block_bytes).min(pcm_data.len());
        let chunk = &pcm_data[offset..end];
        let nb = (chunk.len() / bytes_per_sample) as u32;

        let mut af = AudioFrame::new(
            nb,
            sample_rate,
            SampleFormat::S16,
            ChannelLayout::from_channels(channels),
        );
        af.data[0] = chunk.to_vec();
        af.pts = i64::from(sample_offset);
        af.time_base = Rational::new(1, sample_rate as i32);

        encoder.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = encoder.receive_packet().unwrap();
        muxer.write_packet(&mut io, &pkt).unwrap();

        sample_offset += nb;
        offset = end;
    }

    // 刷新
    encoder.send_frame(None).unwrap();
    muxer.write_trailer(&mut io).unwrap();

    // 回到开头以供读取
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    io
}

/// FLAC 解码管线: FLAC 文件 (内存) → demux → decode → 返回 PCM 数据
fn decode_from_flac_memory(
    codec_registry: &CodecRegistry,
    format_registry: &FormatRegistry,
    io: &mut IoContext,
) -> Vec<u8> {
    // 解封装
    let mut demuxer = format_registry
        .create_demuxer(FormatId::FlacContainer)
        .unwrap();
    demuxer.open(io).unwrap();
    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);

    let stream = &streams[0];
    assert_eq!(stream.codec_id, CodecId::Flac);

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(CodecId::Flac).unwrap();
    let audio_params = match &stream.params {
        StreamParams::Audio(a) => a,
        _ => panic!("期望音频流"),
    };
    let dec_params = CodecParameters {
        codec_id: CodecId::Flac,
        extra_data: stream.extra_data.clone(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: audio_params.sample_rate,
            channel_layout: audio_params.channel_layout,
            sample_format: audio_params.sample_format,
            frame_size: audio_params.frame_size,
        }),
    };
    decoder.open(&dec_params).unwrap();

    // 读取所有包并解码
    let mut all_pcm = Vec::new();
    loop {
        let pkt = match demuxer.read_packet(io) {
            Ok(p) => p,
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读取数据包失败: {}", e),
        };

        decoder.send_packet(&pkt).unwrap();
        match decoder.receive_frame() {
            Ok(Frame::Audio(af)) => {
                all_pcm.extend_from_slice(&af.data[0]);
            }
            Ok(_) => panic!("期望音频帧"),
            Err(TaoError::NeedMoreData) => {}
            Err(e) => panic!("解码失败: {}", e),
        }
    }

    all_pcm
}

fn init_registries() -> (CodecRegistry, FormatRegistry) {
    let codec_registry = tao::default_codec_registry();
    let format_registry = tao::default_format_registry();
    (codec_registry, format_registry)
}

// ============================================================
// 集成测试
// ============================================================

#[test]
fn test_flac_lossless_roundtrip_all_zero_mono() {
    let (codec_reg, format_reg) = init_registries();
    let nb_samples = 512u32;
    let original = generate_silence_s16le(nb_samples, 1);

    let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, 44100, 1, 256);
    let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

    assert_eq!(
        decoded.len(),
        original.len(),
        "解码数据长度不匹配: {} vs {}",
        decoded.len(),
        original.len(),
    );
    assert_eq!(decoded, original, "FLAC 无损往返: 全零 mono 数据不一致");
}

#[test]
fn test_flac_lossless_roundtrip_sine_wave_mono() {
    let (codec_reg, format_reg) = init_registries();
    let nb_samples = 1024u32;
    let original = generate_sine_s16le(44100, 440.0, nb_samples, 1);

    let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, 44100, 1, 256);
    let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

    assert_eq!(decoded.len(), original.len());
    assert_eq!(decoded, original, "FLAC 无损往返: 正弦波 mono 数据不一致");
}

#[test]
fn test_flac_lossless_roundtrip_sine_wave_stereo() {
    let (codec_reg, format_reg) = init_registries();
    let nb_samples = 512u32;
    let original = generate_sine_s16le(44100, 440.0, nb_samples, 2);

    let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, 44100, 2, 256);
    let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

    assert_eq!(decoded.len(), original.len());
    assert_eq!(decoded, original, "FLAC 无损往返: 正弦波 stereo 数据不一致");
}

#[test]
fn test_flac_lossless_roundtrip_sawtooth_wave() {
    let (codec_reg, format_reg) = init_registries();
    let nb_samples = 768u32;
    let original = generate_sawtooth_s16le(nb_samples, 1);

    let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, 48000, 1, 256);
    let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

    assert_eq!(decoded.len(), original.len());
    assert_eq!(decoded, original, "FLAC 无损往返: 锯齿波数据不一致");
}

#[test]
fn test_flac_lossless_roundtrip_large_block() {
    let (codec_reg, format_reg) = init_registries();
    let nb_samples = 4096u32;
    let original = generate_sine_s16le(44100, 1000.0, nb_samples, 1);

    let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, 44100, 1, 4096);
    let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

    assert_eq!(decoded.len(), original.len());
    assert_eq!(decoded, original, "FLAC 无损往返: 大块数据不一致");
}

#[test]
fn test_flac_lossless_roundtrip_multi_sample_rate() {
    let (codec_reg, format_reg) = init_registries();

    for &sr in &[8000u32, 22050, 44100, 48000, 96000] {
        let nb_samples = 256u32;
        let original = generate_sine_s16le(sr, 440.0, nb_samples, 1);

        let mut io = encode_to_flac_memory(&codec_reg, &format_reg, &original, sr, 1, 256);
        let decoded = decode_from_flac_memory(&codec_reg, &format_reg, &mut io);

        assert_eq!(decoded.len(), original.len());
        assert_eq!(decoded, original, "FLAC 无损往返: 采样率 {} 数据不一致", sr,);
    }
}

#[test]
fn test_flac_encoder_registration() {
    let registry = tao::default_codec_registry();
    let encoder = registry.create_encoder(CodecId::Flac);
    assert!(encoder.is_ok(), "FLAC 编码器应在注册表中");
}

#[test]
fn test_flac_muxer_registration() {
    let registry = tao::default_format_registry();
    let muxer = registry.create_muxer(FormatId::FlacContainer);
    assert!(muxer.is_ok(), "FLAC 封装器应在注册表中");
}
