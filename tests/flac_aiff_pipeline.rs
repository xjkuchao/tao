//! FLAC 解码 + AIFF 容器集成测试.
//!
//! 测试:
//! 1. AIFF 封装/解封装往返
//! 2. FLAC 容器探测 + demux + decode 管线
//! 3. FLAC 帧的 Constant/Verbatim/Fixed 子帧解码

use tao::codec::{
    CodecId, CodecParameters, CodecRegistry,
    codec_parameters::{AudioCodecParams, CodecParamsType},
};
use tao::core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError};
use tao::format::{
    FormatId, FormatRegistry, IoContext,
    io::MemoryBackend,
    stream::{AudioStreamParams, Stream, StreamParams},
};

// ============================================================
// AIFF 集成测试
// ============================================================

/// 生成大端 S16 PCM 正弦波数据
fn generate_sine_s16be(sample_rate: u32, freq: f64, duration_sec: f64) -> Vec<u8> {
    let total_samples = (sample_rate as f64 * duration_sec) as usize;
    let mut buf = Vec::with_capacity(total_samples * 2);
    for i in 0..total_samples {
        let t = i as f64 / sample_rate as f64;
        let value = (t * freq * 2.0 * std::f64::consts::PI).sin();
        let sample = (value * 32767.0) as i16;
        buf.extend_from_slice(&sample.to_be_bytes()); // 大端!
    }
    buf
}

/// 封装 PCM 数据为 AIFF (内存)
fn mux_aiff_in_memory(
    format_registry: &FormatRegistry,
    pcm_data: &[u8],
    sample_rate: u32,
    channels: u32,
) -> IoContext {
    let mut muxer = format_registry.create_muxer(FormatId::Aiff).unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));
    let stream = Stream {
        index: 0,
        media_type: MediaType::Audio,
        codec_id: CodecId::PcmS16be,
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
    };

    muxer.write_header(&mut io, &[stream]).unwrap();

    let pkt = tao::codec::Packet::from_data(bytes::Bytes::from(pcm_data.to_vec()));
    muxer.write_packet(&mut io, &pkt).unwrap();
    muxer.write_trailer(&mut io).unwrap();

    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    io
}

#[test]
fn test_aiff_mux_demux_roundtrip() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };

    // 生成 0.1 秒 44100 Hz 单声道大端 PCM
    let pcm_data = generate_sine_s16be(44100, 440.0, 0.1);
    let mut io = mux_aiff_in_memory(&format_registry, &pcm_data, 44100, 1);

    // 解封装
    let mut demuxer = format_registry.create_demuxer(FormatId::Aiff).unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec_id, CodecId::PcmS16be);
    assert_eq!(streams[0].media_type, MediaType::Audio);

    if let StreamParams::Audio(a) = &streams[0].params {
        assert_eq!(a.sample_rate, 44100);
        assert_eq!(a.channel_layout.channels, 1);
    } else {
        panic!("期望音频流参数");
    }

    // 读取所有 packet 数据
    let mut all_data = Vec::new();
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => all_data.extend_from_slice(&pkt.data),
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读取 packet 出错: {}", e),
        }
    }

    assert_eq!(all_data.len(), pcm_data.len());
    assert_eq!(all_data, pcm_data);
}

#[test]
fn test_aiff_decode_s16be_roundtrip() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };
    let codec_registry = {
        let mut r = CodecRegistry::new();
        tao::codec::register_all(&mut r);
        r
    };

    let pcm_data = generate_sine_s16be(44100, 440.0, 0.01);
    let mut io = mux_aiff_in_memory(&format_registry, &pcm_data, 44100, 1);

    let mut demuxer = format_registry.create_demuxer(FormatId::Aiff).unwrap();
    demuxer.open(&mut io).unwrap();

    // 创建 S16BE 解码器
    let mut decoder = codec_registry.create_decoder(CodecId::PcmS16be).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::PcmS16be,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::MONO,
            sample_format: SampleFormat::S16,
            frame_size: 0,
        }),
    };
    decoder.open(&params).unwrap();

    // 读取所有帧
    let mut decoded_samples: Vec<i16> = Vec::new();
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                decoder.send_packet(&pkt).unwrap();
                match decoder.receive_frame() {
                    Ok(frame) => {
                        if let tao::codec::Frame::Audio(af) = frame {
                            // 解码器输出为 S16LE (小端)
                            for chunk in af.data[0].chunks_exact(2) {
                                decoded_samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                            }
                        }
                    }
                    Err(TaoError::NeedMoreData) => {}
                    Err(e) => panic!("接收帧出错: {}", e),
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读取 packet 出错: {}", e),
        }
    }

    // 原始大端数据转为 i16 样本
    let original_samples: Vec<i16> = pcm_data
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();

    assert_eq!(decoded_samples.len(), original_samples.len());
    assert_eq!(decoded_samples, original_samples);
}

#[test]
fn test_aiff_probe() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };

    let pcm_data = generate_sine_s16be(44100, 440.0, 0.01);
    let mut io = mux_aiff_in_memory(&format_registry, &pcm_data, 44100, 1);

    // 使用 probe_input 自动探测
    let result = format_registry
        .probe_input(&mut io, Some("test.aiff"))
        .unwrap();
    assert_eq!(result.format_id, FormatId::Aiff);
}

// ============================================================
// FLAC 解码集成测试
// ============================================================

/// 构造一个最小的 FLAC 文件 (仅包含 STREAMINFO + 一帧)
fn make_minimal_flac(
    sample_rate: u32,
    channels: u32,
    bps: u32,
    block_size: u32,
    frame_data: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::new();

    // Magic
    buf.extend_from_slice(b"fLaC");

    // STREAMINFO metadata block (is_last=1, type=0, size=34)
    buf.push(0x80); // is_last=1, type=0
    buf.push(0x00);
    buf.push(0x00);
    buf.push(0x22); // size=34

    // STREAMINFO (34 bytes)
    let mut si = [0u8; 34];
    // min/max block size
    let bs = block_size as u16;
    si[0..2].copy_from_slice(&bs.to_be_bytes());
    si[2..4].copy_from_slice(&bs.to_be_bytes());

    // sample_rate (20 bits) + channels-1 (3 bits) + bps-1 (5 bits)
    si[10] = ((sample_rate >> 12) & 0xFF) as u8;
    si[11] = ((sample_rate >> 4) & 0xFF) as u8;
    let sr_low = ((sample_rate & 0x0F) << 4) as u8;
    let ch_bits = (((channels - 1) & 0x07) << 1) as u8;
    let bps_hi = (((bps - 1) >> 4) & 0x01) as u8;
    si[12] = sr_low | ch_bits | bps_hi;
    let bps_lo = (((bps - 1) & 0x0F) << 4) as u8;
    si[13] = bps_lo;

    // total_samples (lower 32 bits)
    let total = block_size;
    si[14..18].copy_from_slice(&total.to_be_bytes());

    buf.extend_from_slice(&si);

    // 帧数据
    buf.extend_from_slice(frame_data);

    buf
}

/// 构造 FLAC 帧 (Constant 子帧, 值为 0)
fn make_constant_frame(block_size: u32, sample_rate: u32, channels: u32, bps: u32) -> Vec<u8> {
    let mut bits: Vec<u8> = Vec::new();
    let mut bit_buf: u64 = 0;
    let mut bit_count = 0u32;

    let flush_byte = |bits: &mut Vec<u8>, buf: &mut u64, count: &mut u32| {
        while *count >= 8 {
            *count -= 8;
            bits.push((*buf >> *count) as u8);
            *buf &= (1u64 << *count) - 1;
        }
    };

    // 同步码 (14 bits) + reserved(1) + blocking(1) = 0xFFF8
    bit_buf = (bit_buf << 14) | 0b11111111111110;
    bit_count += 14;
    bit_buf <<= 1; // reserved = 0
    bit_count += 1;
    bit_buf <<= 1; // blocking = 0 (fixed)
    bit_count += 1;
    flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

    // block_size code (4 bits)
    let bs_code = match block_size {
        192 => 1u32,
        576 => 2,
        1152 => 3,
        2304 => 4,
        4608 => 5,
        256 => 8,
        512 => 9,
        1024 => 10,
        2048 => 11,
        4096 => 12,
        8192 => 13,
        16384 => 14,
        32768 => 15,
        _ => 6,
    };
    bit_buf = (bit_buf << 4) | u64::from(bs_code);
    bit_count += 4;

    // sample rate code (4 bits)
    let sr_code: u32 = match sample_rate {
        88200 => 1,
        176400 => 2,
        192000 => 3,
        8000 => 4,
        16000 => 5,
        22050 => 6,
        24000 => 7,
        32000 => 8,
        44100 => 9,
        48000 => 10,
        96000 => 11,
        _ => 0,
    };
    bit_buf = (bit_buf << 4) | u64::from(sr_code);
    bit_count += 4;
    flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

    // channel assignment (4 bits) = channels - 1
    let ch_code = channels - 1;
    bit_buf = (bit_buf << 4) | u64::from(ch_code);
    bit_count += 4;

    // sample size (3 bits)
    let ss_code: u32 = match bps {
        8 => 1,
        12 => 2,
        16 => 4,
        20 => 5,
        24 => 6,
        32 => 7,
        _ => 0,
    };
    bit_buf = (bit_buf << 3) | u64::from(ss_code);
    bit_count += 3;

    // reserved (1 bit) = 0
    bit_buf <<= 1;
    bit_count += 1;
    flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

    // Frame number (UTF-8 encoded: frame 0 = 0x00)
    bits.push(0x00);

    // Extended block_size (if bs_code == 6)
    if bs_code == 6 {
        bits.push((block_size - 1) as u8);
    }

    // CRC-8
    let crc = tao::core::crc::crc8(&bits);
    bits.push(crc);

    // Subframes: Constant value = 0
    for _ in 0..channels {
        // 子帧头: padding(1)=0 + type(6)=000000(constant) + wasted_flag(1)=0
        bits.push(0x00);
        let value_bytes = bps.div_ceil(8) as usize;
        bits.extend(std::iter::repeat_n(0u8, value_bytes));
    }

    // CRC-16
    let frame_crc = tao::core::crc::crc16(&bits);
    bits.push((frame_crc >> 8) as u8);
    bits.push((frame_crc & 0xFF) as u8);

    bits
}

#[test]
fn test_flac_probe_magic() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };

    let frame_data = make_constant_frame(256, 44100, 1, 16);
    let flac_data = make_minimal_flac(44100, 1, 16, 256, &frame_data);

    let mut io = IoContext::new(Box::new(MemoryBackend::from_data(flac_data)));
    let result = format_registry
        .probe_input(&mut io, Some("test.flac"))
        .unwrap();
    assert_eq!(result.format_id, FormatId::FlacContainer);
}

#[test]
fn test_flac_demux_basic_stream_info() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };

    let frame_data = make_constant_frame(256, 44100, 2, 16);
    let flac_data = make_minimal_flac(44100, 2, 16, 256, &frame_data);

    let mut io = IoContext::new(Box::new(MemoryBackend::from_data(flac_data)));
    let demuxer = format_registry
        .open_input(&mut io, Some("test.flac"))
        .unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec_id, CodecId::Flac);
    assert_eq!(streams[0].media_type, MediaType::Audio);

    if let StreamParams::Audio(a) = &streams[0].params {
        assert_eq!(a.sample_rate, 44100);
        assert_eq!(a.channel_layout.channels, 2);
    } else {
        panic!("期望音频流参数");
    }
}

#[test]
fn test_flac_full_pipeline_demux_decode_constant() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };
    let codec_registry = {
        let mut r = CodecRegistry::new();
        tao::codec::register_all(&mut r);
        r
    };

    // 构造 FLAC 文件: 256 采样, 44100 Hz, 单声道, 16 位, constant=0
    let frame_data = make_constant_frame(256, 44100, 1, 16);
    let flac_data = make_minimal_flac(44100, 1, 16, 256, &frame_data);

    let mut io = IoContext::new(Box::new(MemoryBackend::from_data(flac_data)));
    let mut demuxer = format_registry
        .open_input(&mut io, Some("test.flac"))
        .unwrap();

    let streams = demuxer.streams();
    let stream = &streams[0];

    // 创建 FLAC 解码器
    let mut decoder = codec_registry.create_decoder(CodecId::Flac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Flac,
        extra_data: stream.extra_data.clone(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::MONO,
            sample_format: SampleFormat::S16,
            frame_size: 256,
        }),
    };
    decoder.open(&params).unwrap();

    // 读取 packet 并解码
    let pkt = demuxer.read_packet(&mut io).unwrap();
    decoder.send_packet(&pkt).unwrap();
    let frame = decoder.receive_frame().unwrap();

    match frame {
        tao::codec::Frame::Audio(af) => {
            assert_eq!(af.nb_samples, 256);
            assert_eq!(af.sample_format, SampleFormat::S16);
            assert_eq!(af.channel_layout.channels, 1);
            // 所有样本应为 0 (constant=0)
            assert!(
                af.data[0].iter().all(|&b| b == 0),
                "constant=0 帧的所有字节应为 0"
            );
        }
        _ => panic!("期望音频帧"),
    }
}

#[test]
fn test_flac_full_pipeline_stereo_constant() {
    let format_registry = {
        let mut r = FormatRegistry::new();
        tao::format::register_all(&mut r);
        r
    };
    let codec_registry = {
        let mut r = CodecRegistry::new();
        tao::codec::register_all(&mut r);
        r
    };

    // 立体声 constant=0
    let frame_data = make_constant_frame(256, 48000, 2, 16);
    let flac_data = make_minimal_flac(48000, 2, 16, 256, &frame_data);

    let mut io = IoContext::new(Box::new(MemoryBackend::from_data(flac_data)));
    let mut demuxer = format_registry
        .open_input(&mut io, Some("test.flac"))
        .unwrap();

    let streams = demuxer.streams();
    let stream = &streams[0];

    let mut decoder = codec_registry.create_decoder(CodecId::Flac).unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Flac,
        extra_data: stream.extra_data.clone(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 48000,
            channel_layout: ChannelLayout::STEREO,
            sample_format: SampleFormat::S16,
            frame_size: 256,
        }),
    };
    decoder.open(&params).unwrap();

    let pkt = demuxer.read_packet(&mut io).unwrap();
    decoder.send_packet(&pkt).unwrap();
    let frame = decoder.receive_frame().unwrap();

    match frame {
        tao::codec::Frame::Audio(af) => {
            assert_eq!(af.nb_samples, 256);
            assert_eq!(af.channel_layout.channels, 2);
            // 立体声交错: 256 samples * 2 channels * 2 bytes = 1024 bytes
            assert_eq!(af.data[0].len(), 256 * 2 * 2);
            assert!(af.data[0].iter().all(|&b| b == 0));
        }
        _ => panic!("期望音频帧"),
    }
}

// ============================================================
// 格式 ID 测试
// ============================================================

#[test]
fn test_format_id_aiff() {
    assert_eq!(FormatId::from_extension("aiff"), Some(FormatId::Aiff));
    assert_eq!(FormatId::from_extension("aif"), Some(FormatId::Aiff));
    assert_eq!(FormatId::from_filename("test.aiff"), Some(FormatId::Aiff));
}

#[test]
fn test_format_id_flac() {
    assert_eq!(
        FormatId::from_extension("flac"),
        Some(FormatId::FlacContainer)
    );
    assert_eq!(
        FormatId::from_filename("music.flac"),
        Some(FormatId::FlacContainer)
    );
}
