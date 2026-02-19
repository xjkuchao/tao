//! MP4 封装器集成测试.
//!
//! 验证 MP4 封装器的完整流程:
//! 1. 写入 ftyp + mdat + moov
//! 2. 通过 MP4 解封装器往返验证
//! 3. 注册表集成

use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat};
use tao_format::demuxers::mp4::Mp4Demuxer;
use tao_format::io::{IoContext, MemoryBackend};
use tao_format::muxers::mp4::Mp4Muxer;
use tao_format::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

// ========================
// 辅助函数
// ========================

fn make_video_stream(width: u32, height: u32, timescale: i32) -> Stream {
    Stream {
        index: 0,
        media_type: MediaType::Video,
        codec_id: CodecId::H264,
        time_base: Rational::new(1, timescale),
        duration: -1,
        start_time: 0,
        nb_frames: 0,
        extra_data: vec![
            // 最小 avcC: version=1, profile=66, compat=0, level=30,
            // lengthSize=4, numSPS=1, spsLen=4, sps, numPPS=1, ppsLen=2, pps
            0x01, 0x42, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1E, 0x01, 0x00,
            0x02, 0x68, 0xCE,
        ],
        params: StreamParams::Video(VideoStreamParams {
            width,
            height,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: Rational::new(30, 1),
            sample_aspect_ratio: Rational::new(1, 1),
            bit_rate: 0,
        }),
        metadata: Vec::new(),
    }
}

fn make_audio_stream(sample_rate: u32, channels: u32) -> Stream {
    Stream {
        index: 1,
        media_type: MediaType::Audio,
        codec_id: CodecId::Aac,
        time_base: Rational::new(1, sample_rate as i32),
        duration: -1,
        start_time: 0,
        nb_frames: 0,
        extra_data: vec![0x12, 0x10], // AAC-LC, 44100Hz, stereo
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::F32,
            bit_rate: 128000,
            frame_size: 1024,
        }),
        metadata: Vec::new(),
    }
}

/// 封装并返回 seek-到开头的 IoContext (用于后续解封装)
fn mux_to_io(streams: &[Stream], packets: &[Packet]) -> IoContext {
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));

    let mut muxer = Mp4Muxer::create().unwrap();
    muxer.write_header(&mut io, streams).unwrap();

    for pkt in packets {
        muxer.write_packet(&mut io, pkt).unwrap();
    }

    muxer.write_trailer(&mut io).unwrap();

    // seek 到开头供后续解封装读取
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    io
}

// ========================
// 测试
// ========================

#[test]
fn test_registry_contains_mp4_muxer() {
    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let muxers = registry.list_muxers();
    assert!(
        muxers.iter().any(|m| m.1 == "mp4"),
        "注册表应包含 mp4 封装器",
    );
}

#[test]
fn test_video_only_mux_demux_roundtrip() {
    let video_stream = make_video_stream(640, 480, 90000);
    let streams = vec![video_stream];

    let num_frames = 5;
    let frame_delta = 3000i64; // 90000/30 = 3000
    let mut packets = Vec::new();
    for i in 0..num_frames {
        let mut pkt = Packet::from_data(vec![0xAA + (i as u8); 200 + i * 50]);
        pkt.stream_index = 0;
        pkt.pts = (i as i64) * frame_delta;
        pkt.dts = (i as i64) * frame_delta;
        pkt.duration = frame_delta;
        pkt.is_keyframe = i == 0;
        pkt.time_base = Rational::new(1, 90000);
        packets.push(pkt);
    }

    let mut io = mux_to_io(&streams, &packets);

    // 使用解封装器读取
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams().to_vec();
    assert_eq!(streams.len(), 1, "应有 1 个视频轨道");
    assert_eq!(streams[0].media_type, MediaType::Video);
    assert_eq!(streams[0].codec_id, CodecId::H264);

    // 验证视频参数
    if let StreamParams::Video(ref v) = streams[0].params {
        assert_eq!(v.width, 640);
        assert_eq!(v.height, 480);
    } else {
        panic!("应为视频流参数");
    }

    // 读取所有包并验证数据
    for i in 0..num_frames {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.stream_index, 0);
        let expected_size = 200 + i * 50;
        assert_eq!(pkt.data.len(), expected_size, "帧 {} 大小不匹配", i);

        // 验证数据内容
        let expected_byte = 0xAA + (i as u8);
        assert!(
            pkt.data.iter().all(|&b| b == expected_byte),
            "帧 {} 数据不匹配",
            i,
        );
    }

    // EOF
    assert!(demuxer.read_packet(&mut io).is_err());
}

#[test]
fn test_audio_only_mux_demux_roundtrip() {
    let mut audio_stream = make_audio_stream(44100, 2);
    audio_stream.index = 0; // 唯一流, 索引为 0

    let num_frames = 10usize;
    let frame_delta = 1024i64;
    let mut packets = Vec::new();
    for i in 0..num_frames {
        let mut pkt = Packet::from_data(vec![(i + 1) as u8; 128]);
        pkt.stream_index = 0;
        pkt.pts = (i as i64) * frame_delta;
        pkt.dts = (i as i64) * frame_delta;
        pkt.duration = frame_delta;
        pkt.is_keyframe = true;
        pkt.time_base = Rational::new(1, 44100);
        packets.push(pkt);
    }

    let mut io = mux_to_io(&[audio_stream], &packets);

    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams().to_vec();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].media_type, MediaType::Audio);
    assert_eq!(streams[0].codec_id, CodecId::Aac);

    for i in 0..num_frames {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.data.len(), 128);
        assert_eq!(pkt.pts, (i as i64) * frame_delta);

        let expected_byte = (i + 1) as u8;
        assert!(
            pkt.data.iter().all(|&b| b == expected_byte),
            "帧 {} 数据不匹配",
            i,
        );
    }

    assert!(demuxer.read_packet(&mut io).is_err());
}

#[test]
fn test_av_mux_demux_roundtrip() {
    let video_stream = make_video_stream(1280, 720, 90000);
    let mut audio_stream = make_audio_stream(48000, 2);
    audio_stream.index = 1;
    let streams = vec![video_stream, audio_stream];

    let mut packets = Vec::new();

    // 3 个视频帧
    for i in 0..3 {
        let mut pkt = Packet::from_data(vec![0xBB; 500]);
        pkt.stream_index = 0;
        pkt.pts = (i as i64) * 3000;
        pkt.dts = (i as i64) * 3000;
        pkt.duration = 3000;
        pkt.is_keyframe = i == 0;
        pkt.time_base = Rational::new(1, 90000);
        packets.push(pkt);
    }

    // 5 个音频帧
    for i in 0..5 {
        let mut pkt = Packet::from_data(vec![0xCC; 100]);
        pkt.stream_index = 1;
        pkt.pts = (i as i64) * 1024;
        pkt.dts = (i as i64) * 1024;
        pkt.duration = 1024;
        pkt.is_keyframe = true;
        pkt.time_base = Rational::new(1, 48000);
        packets.push(pkt);
    }

    let mut io = mux_to_io(&streams, &packets);

    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let demux_streams = demuxer.streams().to_vec();
    assert_eq!(demux_streams.len(), 2, "应有 2 个轨道");

    // 验证流类型
    let video_stream_idx = demux_streams
        .iter()
        .position(|s| s.media_type == MediaType::Video)
        .expect("应有视频轨道");
    let audio_stream_idx = demux_streams
        .iter()
        .position(|s| s.media_type == MediaType::Audio)
        .expect("应有音频轨道");

    assert_eq!(demux_streams[video_stream_idx].codec_id, CodecId::H264);
    assert_eq!(demux_streams[audio_stream_idx].codec_id, CodecId::Aac);

    // 读取全部 8 个包 (3 video + 5 audio)
    let mut video_count = 0;
    let mut audio_count = 0;
    for _ in 0..8 {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        if pkt.stream_index == demux_streams[video_stream_idx].index {
            assert_eq!(pkt.data.len(), 500);
            video_count += 1;
        } else if pkt.stream_index == demux_streams[audio_stream_idx].index {
            assert_eq!(pkt.data.len(), 100);
            audio_count += 1;
        }
    }

    assert_eq!(video_count, 3, "应有 3 个视频包");
    assert_eq!(audio_count, 5, "应有 5 个音频包");
}

#[test]
fn test_keyframe_flag_roundtrip() {
    let video_stream = make_video_stream(320, 240, 30000);
    let streams = vec![video_stream];

    // 创建 10 帧, 每 5 帧一个关键帧
    let mut packets = Vec::new();
    for i in 0..10 {
        let mut pkt = Packet::from_data(vec![0xDD; 100]);
        pkt.stream_index = 0;
        pkt.pts = (i as i64) * 1001;
        pkt.dts = (i as i64) * 1001;
        pkt.duration = 1001;
        pkt.is_keyframe = i % 5 == 0;
        pkt.time_base = Rational::new(1, 30000);
        packets.push(pkt);
    }

    let mut io = mux_to_io(&streams, &packets);

    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    for i in 0..10 {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        let expected_kf = i % 5 == 0;
        assert_eq!(
            pkt.is_keyframe, expected_kf,
            "帧 {} 关键帧标记不匹配 (期望={}, 实际={})",
            i, expected_kf, pkt.is_keyframe,
        );
    }
}

#[test]
fn test_avcc_extra_data_roundtrip() {
    let video_stream = make_video_stream(640, 480, 90000);
    let expected_extra = video_stream.extra_data.clone();
    let streams = vec![video_stream];

    let mut pkt = Packet::from_data(vec![0xFF; 100]);
    pkt.stream_index = 0;
    pkt.pts = 0;
    pkt.dts = 0;
    pkt.duration = 3000;
    pkt.is_keyframe = true;
    pkt.time_base = Rational::new(1, 90000);

    let mut io = mux_to_io(&streams, &[pkt]);

    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let demux_streams = demuxer.streams();
    assert_eq!(
        demux_streams[0].extra_data, expected_extra,
        "avcC extra_data 应完整往返",
    );
}

#[test]
fn test_esds_aac_extra_data_roundtrip() {
    let mut audio_stream = make_audio_stream(44100, 2);
    audio_stream.index = 0;
    let expected_extra = vec![0x12, 0x10]; // AAC-LC 44100Hz stereo

    let mut pkt = Packet::from_data(vec![0xEE; 64]);
    pkt.stream_index = 0;
    pkt.pts = 0;
    pkt.dts = 0;
    pkt.duration = 1024;
    pkt.is_keyframe = true;
    pkt.time_base = Rational::new(1, 44100);

    let mut io = mux_to_io(&[audio_stream], &[pkt]);

    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let demux_streams = demuxer.streams();
    assert_eq!(demux_streams[0].codec_id, CodecId::Aac);

    // 解封装器会将整个 esds box 内容 (含 version+flags + ES_Descriptor) 存为 extra_data
    // 验证 AudioSpecificConfig 字节 [0x12, 0x10] 包含在 extra_data 中
    let extra = &demux_streams[0].extra_data;
    assert!(!extra.is_empty(), "esds extra_data 不应为空",);
    assert!(
        extra
            .windows(expected_extra.len())
            .any(|w| w == expected_extra.as_slice()),
        "esds extra_data 应包含 AudioSpecificConfig {:?}, 实际={:?}",
        expected_extra,
        extra,
    );
}
