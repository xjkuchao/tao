//! 端到端集成测试: WAV 文件的完整处理管线.
//!
//! 测试流程: 生成 PCM 数据 → 封装为 WAV → 解封装 → 解码 → 验证
//! 以及: 生成 PCM 数据 → 编码 → 封装为 WAV → 解封装 → 解码 → 验证

use tao::codec::{
    CodecId, CodecParameters, Packet,
    codec_parameters::{AudioCodecParams, CodecParamsType},
    frame::{AudioFrame, Frame},
};
use tao::core::{ChannelLayout, Rational, SampleFormat};
use tao::format::{
    FormatId, IoContext,
    io::MemoryBackend,
    stream::{AudioStreamParams, Stream, StreamParams},
};

/// 生成正弦波 PCM S16LE 数据
fn generate_sine_wave_s16(
    sample_rate: u32,
    freq: f64,
    duration_sec: f64,
    channels: u32,
) -> Vec<u8> {
    let total_samples = (sample_rate as f64 * duration_sec) as usize;
    let mut buf = Vec::with_capacity(total_samples * channels as usize * 2);
    for i in 0..total_samples {
        let t = i as f64 / sample_rate as f64;
        let value = (t * freq * 2.0 * std::f64::consts::PI).sin();
        let sample = (value * 32767.0) as i16;
        for _ch in 0..channels {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
    }
    buf
}

/// 辅助: 创建音频流描述
fn make_audio_stream(codec_id: CodecId, sample_rate: u32, channels: u32) -> Stream {
    Stream {
        index: 0,
        media_type: tao::core::MediaType::Audio,
        codec_id,
        time_base: Rational::new(1, sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::S16,
            bit_rate: 0,
            frame_size: 0,
        }),
        metadata: Vec::new(),
    }
}

/// 辅助: 创建音频编解码参数
fn make_audio_params(codec_id: CodecId, sample_rate: u32, channels: u32) -> CodecParameters {
    CodecParameters {
        codec_id,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::None,
            frame_size: 0,
        }),
    }
}

#[test]
fn test_full_pipeline_generate_wav_demux_decode() {
    let sample_rate = 44100u32;
    let channels = 1u32;

    // 1. 生成 PCM 数据 (440Hz 正弦波, 0.1 秒)
    let pcm_data = generate_sine_wave_s16(sample_rate, 440.0, 0.1, channels);
    let original_len = pcm_data.len();

    // 2. 封装为 WAV
    let format_registry = tao::default_format_registry();
    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut io_w = IoContext::new(Box::new(backend));
    let stream = make_audio_stream(CodecId::PcmS16le, sample_rate, channels);
    muxer.write_header(&mut io_w, &[stream]).unwrap();
    let pkt = Packet::from_data(pcm_data.clone());
    muxer.write_packet(&mut io_w, &pkt).unwrap();
    muxer.write_trailer(&mut io_w).unwrap();

    // 3. 解封装
    io_w.seek(std::io::SeekFrom::Start(0)).unwrap();
    let mut demuxer = format_registry.create_demuxer(FormatId::Wav).unwrap();
    demuxer.open(&mut io_w).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec_id, CodecId::PcmS16le);

    // 4. 读取数据包并解码
    let codec_registry = tao::default_codec_registry();
    let mut decoder = codec_registry.create_decoder(CodecId::PcmS16le).unwrap();
    let params = make_audio_params(CodecId::PcmS16le, sample_rate, channels);
    decoder.open(&params).unwrap();

    let mut decoded_bytes = 0usize;
    loop {
        match demuxer.read_packet(&mut io_w) {
            Ok(pkt) => {
                decoder.send_packet(&pkt).unwrap();
                match decoder.receive_frame() {
                    Ok(Frame::Audio(af)) => {
                        decoded_bytes += af.data[0].len();
                        assert_eq!(af.sample_rate, sample_rate);
                        assert_eq!(af.sample_format, SampleFormat::S16);
                    }
                    Ok(_) => panic!("期望音频帧"),
                    Err(e) => panic!("解码失败: {e}"),
                }
            }
            Err(tao::core::TaoError::Eof) => break,
            Err(e) => panic!("读包失败: {e}"),
        }
    }

    // 验证: 解码后的数据总量应该与原始 PCM 数据一致
    assert_eq!(decoded_bytes, original_len);
}

#[test]
fn test_full_pipeline_encode_mux_demux_decode_roundtrip() {
    let sample_rate = 48000u32;
    let channels = 2u32;

    // 1. 生成原始 PCM 数据
    let pcm_data = generate_sine_wave_s16(sample_rate, 1000.0, 0.05, channels);
    let nb_samples = pcm_data.len() / (channels as usize * 2);

    // 2. 构建音频帧
    let mut frame = AudioFrame::new(
        nb_samples as u32,
        sample_rate,
        SampleFormat::S16,
        ChannelLayout::from_channels(channels),
    );
    frame.data[0] = pcm_data.clone();
    frame.pts = 0;
    frame.time_base = Rational::new(1, sample_rate as i32);

    // 3. 编码
    let codec_registry = tao::default_codec_registry();
    let params = make_audio_params(CodecId::PcmS16le, sample_rate, channels);
    let mut encoder = codec_registry.create_encoder(CodecId::PcmS16le).unwrap();
    encoder.open(&params).unwrap();
    encoder.send_frame(Some(&Frame::Audio(frame))).unwrap();
    let encoded_pkt = encoder.receive_packet().unwrap();

    // 4. 封装为 WAV
    let format_registry = tao::default_format_registry();
    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));
    let stream = make_audio_stream(CodecId::PcmS16le, sample_rate, channels);
    muxer.write_header(&mut io, &[stream]).unwrap();
    muxer.write_packet(&mut io, &encoded_pkt).unwrap();
    muxer.write_trailer(&mut io).unwrap();

    // 5. 解封装
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    let mut demuxer = format_registry.create_demuxer(FormatId::Wav).unwrap();
    demuxer.open(&mut io).unwrap();

    // 验证流信息
    let streams = demuxer.streams();
    assert_eq!(streams[0].codec_id, CodecId::PcmS16le);
    if let StreamParams::Audio(a) = &streams[0].params {
        assert_eq!(a.sample_rate, sample_rate);
        assert_eq!(a.channel_layout.channels, channels);
    } else {
        panic!("期望音频流参数");
    }

    // 6. 读取数据包
    let read_pkt = demuxer.read_packet(&mut io).unwrap();

    // 7. 解码
    let mut decoder = codec_registry.create_decoder(CodecId::PcmS16le).unwrap();
    decoder.open(&params).unwrap();
    decoder.send_packet(&read_pkt).unwrap();
    let decoded = decoder.receive_frame().unwrap();

    // 8. 验证往返一致性
    match decoded {
        Frame::Audio(af) => {
            assert_eq!(af.nb_samples, nb_samples as u32);
            assert_eq!(af.data[0], pcm_data, "编码-封装-解封装-解码 往返数据不一致");
        }
        _ => panic!("期望音频帧"),
    }
}

#[test]
fn test_registry_auto_probe_wav() {
    let sample_rate = 44100u32;
    let pcm_data = generate_sine_wave_s16(sample_rate, 440.0, 0.01, 1);

    // 封装
    let format_registry = tao::default_format_registry();
    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));
    let stream = make_audio_stream(CodecId::PcmS16le, sample_rate, 1);
    muxer.write_header(&mut io, &[stream]).unwrap();
    let pkt = Packet::from_data(pcm_data);
    muxer.write_packet(&mut io, &pkt).unwrap();
    muxer.write_trailer(&mut io).unwrap();

    // 探测格式 (读取文件头部, 不超过实际大小)
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    let file_size = io.size().unwrap_or(4096) as usize;
    let probe_size = file_size.min(4096);
    let probe_data = io.read_bytes(probe_size).unwrap();
    let probe_result = format_registry
        .probe(&probe_data, Some("test.wav"))
        .unwrap();
    assert_eq!(probe_result.format_id, FormatId::Wav);
    assert_eq!(probe_result.score, tao::format::probe::SCORE_MAX);
}

#[test]
fn test_wav_not_same_pcm_format() {
    // 测试 U8, S16LE, S32LE, F32LE 格式的 WAV 封装/解封装
    let test_cases: Vec<(CodecId, u32, Vec<u8>)> = vec![
        (
            CodecId::PcmU8,
            8000,
            vec![128, 64, 192, 255, 0, 128, 64, 192],
        ),
        (
            CodecId::PcmS16le,
            44100,
            vec![0x00, 0x01, 0xFF, 0x7F, 0x00, 0x80, 0x01, 0x00],
        ),
        (
            CodecId::PcmS32le,
            96000,
            vec![
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
            ],
        ),
        (CodecId::PcmF32le, 48000, {
            let mut v = Vec::new();
            v.extend_from_slice(&1.0f32.to_le_bytes());
            v.extend_from_slice(&(-1.0f32).to_le_bytes());
            v.extend_from_slice(&0.0f32.to_le_bytes());
            v
        }),
    ];

    let format_registry = tao::default_format_registry();

    for (codec_id, sample_rate, pcm_data) in test_cases {
        // 封装
        let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let stream = make_audio_stream(codec_id, sample_rate, 1);
        muxer.write_header(&mut io, &[stream]).unwrap();
        let pkt = Packet::from_data(pcm_data.clone());
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();

        // 解封装
        io.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut demuxer = format_registry.create_demuxer(FormatId::Wav).unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(
            streams[0].codec_id, codec_id,
            "格式 {} 编解码器不匹配",
            codec_id
        );

        let read_pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(
            &read_pkt.data[..],
            &pcm_data[..],
            "格式 {} 数据不匹配",
            codec_id
        );
    }
}
