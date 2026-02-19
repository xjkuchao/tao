//! 转码管线集成测试.
//!
//! 测试完整的 demux → decode → resample → encode → mux 管线.
//! 包括 PCM 格式转换、声道转换、采样率转换等场景.

use tao::codec::{
    CodecId, CodecParameters, CodecRegistry, Encoder, Frame, Packet,
    codec_parameters::{AudioCodecParams, CodecParamsType},
    frame::AudioFrame,
};
use tao::core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError};
use tao::format::{
    FormatId, FormatRegistry, IoContext,
    io::MemoryBackend,
    stream::{AudioStreamParams, Stream, StreamParams},
};
use tao_resample::ResampleContext;

/// 生成正弦波 PCM S16LE 数据
fn generate_sine_s16(sample_rate: u32, freq: f64, duration_sec: f64, channels: u32) -> Vec<u8> {
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

/// 封装 PCM 数据为 WAV (内存)
fn mux_wav_in_memory(
    format_registry: &FormatRegistry,
    pcm_data: &[u8],
    codec_id: CodecId,
    sample_rate: u32,
    channels: u32,
) -> IoContext {
    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));
    let stream = Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id,
        time_base: Rational::new(1, sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: codec_id_to_sample_format(codec_id),
            bit_rate: 0,
            frame_size: 0,
        }),
        metadata: Vec::new(),
    };
    muxer.write_header(&mut io, &[stream]).unwrap();
    let pkt = Packet::from_data(pcm_data.to_vec());
    muxer.write_packet(&mut io, &pkt).unwrap();
    muxer.write_trailer(&mut io).unwrap();

    // Seek 回起始位置
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    io
}

fn codec_id_to_sample_format(codec_id: CodecId) -> SampleFormat {
    match codec_id {
        CodecId::PcmU8 => SampleFormat::U8,
        CodecId::PcmS16le | CodecId::PcmS16be => SampleFormat::S16,
        CodecId::PcmS24le | CodecId::PcmS32le => SampleFormat::S32,
        CodecId::PcmF32le => SampleFormat::F32,
        _ => SampleFormat::None,
    }
}

/// 完整转码流程:
/// 输入 WAV → demux → decode → (resample) → encode → mux → 输出 WAV
fn transcode_wav(
    input_io: &mut IoContext,
    output_codec_id: CodecId,
    target_sample_rate: Option<u32>,
    target_channels: Option<u32>,
) -> IoContext {
    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);

    let mut codec_registry = CodecRegistry::new();
    tao_codec::register_all(&mut codec_registry);

    // 1. 解封装
    let mut demuxer = format_registry
        .open_input(input_io, Some("test.wav"))
        .unwrap();

    let input_stream = &demuxer.streams()[0];
    let audio_params = match &input_stream.params {
        StreamParams::Audio(a) => a.clone(),
        _ => panic!("期望音频流"),
    };

    let src_codec_id = input_stream.codec_id;

    // 2. 创建解码器
    let mut decoder = codec_registry.create_decoder(src_codec_id).unwrap();
    decoder
        .open(&CodecParameters {
            codec_id: src_codec_id,
            extra_data: Vec::new(),
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: audio_params.sample_rate,
                channel_layout: audio_params.channel_layout,
                sample_format: audio_params.sample_format,
                frame_size: 0,
            }),
        })
        .unwrap();

    // 3. 确定输出参数
    let out_sample_rate = target_sample_rate.unwrap_or(audio_params.sample_rate);
    let out_channels = target_channels.unwrap_or(audio_params.channel_layout.channels);
    let out_channel_layout = ChannelLayout::from_channels(out_channels);
    let out_sample_format = codec_id_to_sample_format(output_codec_id);

    // 4. 创建重采样器 (如果需要)
    let resampler = if audio_params.sample_rate != out_sample_rate
        || audio_params.channel_layout.channels != out_channels
        || audio_params.sample_format != out_sample_format
    {
        Some(ResampleContext::new(
            audio_params.sample_rate,
            audio_params.sample_format,
            audio_params.channel_layout,
            out_sample_rate,
            out_sample_format,
            out_channel_layout,
        ))
    } else {
        None
    };

    // 5. 创建编码器
    let mut encoder = codec_registry.create_encoder(output_codec_id).unwrap();
    encoder
        .open(&CodecParameters {
            codec_id: output_codec_id,
            extra_data: Vec::new(),
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: out_sample_rate,
                channel_layout: out_channel_layout,
                sample_format: out_sample_format,
                frame_size: 0,
            }),
        })
        .unwrap();

    // 6. 创建输出
    let out_stream = Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id: output_codec_id,
        time_base: Rational::new(1, out_sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate: out_sample_rate,
            channel_layout: out_channel_layout,
            sample_format: out_sample_format,
            bit_rate: 0,
            frame_size: 0,
        }),
        metadata: Vec::new(),
    };

    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut output_io = IoContext::new(Box::new(backend));
    muxer.write_header(&mut output_io, &[out_stream]).unwrap();

    // 7. 转码循环
    loop {
        match demuxer.read_packet(input_io) {
            Ok(pkt) => {
                decoder.send_packet(&pkt).unwrap();
                loop {
                    match decoder.receive_frame() {
                        Ok(frame) => {
                            let frame_to_encode = if let Some(ref resampler) = resampler {
                                resample_audio_frame(
                                    resampler,
                                    &frame,
                                    out_channels,
                                    out_sample_format,
                                )
                            } else {
                                frame
                            };
                            encoder.send_frame(Some(&frame_to_encode)).unwrap();
                            drain_encoder(&mut *encoder, &mut *muxer, &mut output_io);
                        }
                        Err(TaoError::NeedMoreData) => break,
                        Err(e) => panic!("解码失败: {e}"),
                    }
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读包失败: {e}"),
        }
    }

    // 8. 刷新
    encoder.send_frame(None).unwrap();
    drain_encoder(&mut *encoder, &mut *muxer, &mut output_io);

    muxer.write_trailer(&mut output_io).unwrap();
    output_io.seek(std::io::SeekFrom::Start(0)).unwrap();
    output_io
}

fn drain_encoder(encoder: &mut dyn Encoder, muxer: &mut dyn tao_format::Muxer, io: &mut IoContext) {
    loop {
        match encoder.receive_packet() {
            Ok(pkt) => muxer.write_packet(io, &pkt).unwrap(),
            Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
            Err(e) => panic!("编码失败: {e}"),
        }
    }
}

fn resample_audio_frame(
    resampler: &ResampleContext,
    frame: &Frame,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
) -> Frame {
    match frame {
        Frame::Audio(audio) => {
            let input_data = &audio.data[0];
            let (output_data, nb_out) = resampler.convert(input_data, audio.nb_samples).unwrap();
            let mut out = AudioFrame::new(
                nb_out,
                resampler.dst_sample_rate,
                dst_sample_format,
                ChannelLayout::from_channels(dst_channels),
            );
            out.data[0] = output_data;
            out.pts = audio.pts;
            out.time_base = audio.time_base;
            Frame::Audio(out)
        }
        _ => panic!("期望音频帧"),
    }
}

// ============================================================
// 辅助: 读取 demuxer 中所有 packet 并拼接数据
// ============================================================

fn read_all_packet_data(demuxer: &mut dyn tao_format::Demuxer, io: &mut IoContext) -> Vec<u8> {
    let mut all_data = Vec::new();
    loop {
        match demuxer.read_packet(io) {
            Ok(pkt) => all_data.extend_from_slice(&pkt.data),
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读包失败: {e}"),
        }
    }
    all_data
}

// ============================================================
// 测试用例
// ============================================================

#[test]
fn test_transcode_s16le_to_f32le_same_params() {
    let format_registry = tao::default_format_registry();

    // 生成 S16LE 单声道 WAV
    let pcm = generate_sine_s16(44100, 440.0, 0.1, 1);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 1);

    // 转码: S16LE → F32LE (同采样率、同声道)
    let mut output_io = transcode_wav(&mut input_io, CodecId::PcmF32le, None, None);

    // 验证输出
    let mut fmt_reg = FormatRegistry::new();
    tao_format::register_all(&mut fmt_reg);
    let mut demuxer = fmt_reg.open_input(&mut output_io, Some("out.wav")).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec_id, CodecId::PcmF32le);
    if let StreamParams::Audio(ref a) = streams[0].params {
        assert_eq!(a.sample_rate, 44100);
        assert_eq!(a.channel_layout.channels, 1);
    }

    // 读取全部输出数据 (WAV demuxer 分块读取, 需要合并)
    let all_data = read_all_packet_data(&mut *demuxer, &mut output_io);
    // S16 数据长度 / 2 * 4 = F32 数据长度
    let expected_f32_len = pcm.len() / 2 * 4;
    assert_eq!(all_data.len(), expected_f32_len);
}

#[test]
fn test_transcode_mono_to_stereo() {
    let format_registry = tao::default_format_registry();

    let pcm = generate_sine_s16(44100, 440.0, 0.05, 1);
    let orig_samples = pcm.len() / 2; // 单声道样本数
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 1);

    // 转码: 单声道 → 立体声
    let mut output_io = transcode_wav(&mut input_io, CodecId::PcmS16le, None, Some(2));

    // 验证
    let mut fmt_reg = FormatRegistry::new();
    tao_format::register_all(&mut fmt_reg);
    let mut demuxer = fmt_reg.open_input(&mut output_io, Some("out.wav")).unwrap();

    let streams = demuxer.streams();
    if let StreamParams::Audio(ref a) = streams[0].params {
        assert_eq!(a.channel_layout.channels, 2);
        assert_eq!(a.sample_rate, 44100);
    }

    let all_data = read_all_packet_data(&mut *demuxer, &mut output_io);
    // 单声道 → 立体声: 数据量翻倍
    assert_eq!(all_data.len(), orig_samples * 2 * 2);
}

#[test]
fn test_transcode_stereo_to_mono() {
    let format_registry = tao::default_format_registry();

    let pcm = generate_sine_s16(44100, 440.0, 0.05, 2);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 2);

    // 转码: 立体声 → 单声道
    let mut output_io = transcode_wav(&mut input_io, CodecId::PcmS16le, None, Some(1));

    let mut fmt_reg = FormatRegistry::new();
    tao_format::register_all(&mut fmt_reg);
    let mut demuxer = fmt_reg.open_input(&mut output_io, Some("out.wav")).unwrap();

    let streams = demuxer.streams();
    if let StreamParams::Audio(ref a) = streams[0].params {
        assert_eq!(a.channel_layout.channels, 1);
    }

    let all_data = read_all_packet_data(&mut *demuxer, &mut output_io);
    // 立体声 → 单声道: 数据量减半
    assert_eq!(all_data.len(), pcm.len() / 2);
}

#[test]
fn test_transcode_resample_44100_to_48000() {
    let format_registry = tao::default_format_registry();

    let pcm = generate_sine_s16(44100, 440.0, 0.1, 1);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 1);

    // 转码: 44100Hz → 48000Hz
    let mut output_io = transcode_wav(&mut input_io, CodecId::PcmS16le, Some(48000), None);

    let mut fmt_reg = FormatRegistry::new();
    tao_format::register_all(&mut fmt_reg);
    let mut demuxer = fmt_reg.open_input(&mut output_io, Some("out.wav")).unwrap();

    let streams = demuxer.streams();
    if let StreamParams::Audio(ref a) = streams[0].params {
        assert_eq!(a.sample_rate, 48000);
    }

    let all_data = read_all_packet_data(&mut *demuxer, &mut output_io);
    let input_samples = pcm.len() / 2;
    let expected_samples = (input_samples as u64 * 48000).div_ceil(44100) as usize;
    let actual_samples = all_data.len() / 2;
    // 分块重采样会有极小的舍入误差 (每块独立向上取整)
    let tolerance = 2; // 最多容许 2 个样本的误差
    assert!(
        actual_samples.abs_diff(expected_samples) <= tolerance,
        "输出样本数 {} 与期望 {} 差距过大",
        actual_samples,
        expected_samples
    );
}

#[test]
fn test_transcode_direct_copy() {
    let format_registry = tao::default_format_registry();
    let codec_registry = tao::default_codec_registry();

    let pcm = generate_sine_s16(44100, 440.0, 0.1, 1);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 1);

    // 直接复制: demux → mux (无解码/编码)
    let mut demuxer = format_registry
        .open_input(&mut input_io, Some("test.wav"))
        .unwrap();

    let input_streams = demuxer.streams().to_vec();

    let mut muxer = format_registry.create_muxer(FormatId::Wav).unwrap();
    let backend = MemoryBackend::new();
    let mut output_io = IoContext::new(Box::new(backend));
    muxer.write_header(&mut output_io, &input_streams).unwrap();

    loop {
        match demuxer.read_packet(&mut input_io) {
            Ok(pkt) => muxer.write_packet(&mut output_io, &pkt).unwrap(),
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读包失败: {e}"),
        }
    }
    muxer.write_trailer(&mut output_io).unwrap();

    // 验证输出与输入一致
    output_io.seek(std::io::SeekFrom::Start(0)).unwrap();
    let mut verify_demuxer = format_registry
        .open_input(&mut output_io, Some("out.wav"))
        .unwrap();

    let out_streams = verify_demuxer.streams();
    assert_eq!(out_streams[0].codec_id, CodecId::PcmS16le);
    if let StreamParams::Audio(ref a) = out_streams[0].params {
        assert_eq!(a.sample_rate, 44100);
    }

    let out_data = read_all_packet_data(&mut *verify_demuxer, &mut output_io);
    assert_eq!(&out_data[..], &pcm[..], "直接复制后数据不一致");

    // 使用 codec_registry 变量避免 unused 警告
    let _ = codec_registry.list_decoders();
}

#[test]
fn test_transcode_combined_format_channel_sample_rate() {
    let format_registry = tao::default_format_registry();

    // 输入: S16LE 44100Hz 立体声
    let pcm = generate_sine_s16(44100, 440.0, 0.05, 2);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 2);

    // 转码: S16LE 44100Hz 立体声 → F32LE 48000Hz 单声道
    let mut output_io = transcode_wav(&mut input_io, CodecId::PcmF32le, Some(48000), Some(1));

    let mut fmt_reg = FormatRegistry::new();
    tao_format::register_all(&mut fmt_reg);
    let mut demuxer = fmt_reg.open_input(&mut output_io, Some("out.wav")).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams[0].codec_id, CodecId::PcmF32le);
    if let StreamParams::Audio(ref a) = streams[0].params {
        assert_eq!(a.sample_rate, 48000);
        assert_eq!(a.channel_layout.channels, 1);
    }

    // 确保数据非空
    let all_data = read_all_packet_data(&mut *demuxer, &mut output_io);
    assert!(!all_data.is_empty());
}

#[test]
fn test_format_id_from_extension() {
    assert_eq!(FormatId::from_extension("wav"), Some(FormatId::Wav));
    assert_eq!(FormatId::from_extension("WAV"), Some(FormatId::Wav));
    assert_eq!(FormatId::from_extension("mp4"), Some(FormatId::Mp4));
    assert_eq!(FormatId::from_extension("mkv"), Some(FormatId::Matroska));
    assert_eq!(
        FormatId::from_extension("flac"),
        Some(FormatId::FlacContainer)
    );
    assert_eq!(FormatId::from_extension("xyz"), None);

    assert_eq!(FormatId::from_filename("output.wav"), Some(FormatId::Wav));
    assert_eq!(FormatId::from_filename("video.mp4"), Some(FormatId::Mp4));
    assert_eq!(FormatId::from_filename("noext"), None);
}

#[test]
fn test_registry_open_input_probe_and_open() {
    let format_registry = tao::default_format_registry();

    let pcm = generate_sine_s16(44100, 440.0, 0.01, 1);
    let mut input_io = mux_wav_in_memory(&format_registry, &pcm, CodecId::PcmS16le, 44100, 1);

    // open_input 应该自动探测 + 解析
    let demuxer = format_registry
        .open_input(&mut input_io, Some("test.wav"))
        .unwrap();

    assert_eq!(demuxer.streams().len(), 1);
    assert_eq!(demuxer.streams()[0].codec_id, CodecId::PcmS16le);
}
