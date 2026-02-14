//! Matroska/WebM 封装器集成测试.
//!
//! 测试 MKV 封装 → 解封装往返.

use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat};
use tao_format::format_id::FormatId;
use tao_format::io::{IoContext, MemoryBackend};
use tao_format::registry::FormatRegistry;
use tao_format::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

fn make_video_stream() -> Stream {
    Stream {
        index: 0,
        media_type: MediaType::Video,
        codec_id: CodecId::H264,
        time_base: Rational::new(1, 1000),
        duration: -1,
        start_time: 0,
        nb_frames: 0,
        extra_data: vec![0x01, 0x42, 0x00, 0x1E, 0xFF, 0xE1],
        params: StreamParams::Video(VideoStreamParams {
            width: 320,
            height: 240,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: Rational::new(30, 1),
            sample_aspect_ratio: Rational::new(1, 1),
            bit_rate: 0,
        }),
        metadata: Vec::new(),
    }
}

fn make_audio_stream() -> Stream {
    Stream {
        index: 1,
        media_type: MediaType::Audio,
        codec_id: CodecId::Aac,
        time_base: Rational::new(1, 1000),
        duration: -1,
        start_time: 0,
        nb_frames: 0,
        extra_data: vec![0x12, 0x10],
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::S16,
            bit_rate: 128000,
            frame_size: 1024,
        }),
        metadata: Vec::new(),
    }
}

fn mux_packets(streams: &[Stream], packets: &[Packet]) -> IoContext {
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);

    let mut muxer = registry.create_muxer(FormatId::Matroska).unwrap();
    let backend = MemoryBackend::new();
    let mut io = IoContext::new(Box::new(backend));

    muxer.write_header(&mut io, streams).unwrap();
    for pkt in packets {
        muxer.write_packet(&mut io, pkt).unwrap();
    }
    muxer.write_trailer(&mut io).unwrap();

    // Seek 回开头
    io.seek(std::io::SeekFrom::Start(0)).unwrap();
    io
}

#[test]
fn test_注册表_包含mkv封装器() {
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);
    assert!(registry.create_muxer(FormatId::Matroska).is_ok());
    assert!(registry.create_muxer(FormatId::Webm).is_ok());
}

#[test]
fn test_仅视频_封装() {
    let stream = make_video_stream();
    let mut packets = Vec::new();
    for i in 0..10 {
        let mut pkt = Packet::from_data(vec![0xAA; 100 + i * 10]);
        pkt.stream_index = 0;
        pkt.pts = i as i64 * 33;
        pkt.dts = i as i64 * 33;
        pkt.is_keyframe = i == 0;
        packets.push(pkt);
    }

    let mut io = mux_packets(&[stream], &packets);
    let pos = io.position().unwrap();
    // 应该已 seek 回开头
    assert_eq!(pos, 0);
}

#[test]
fn test_仅视频_封装解封装往返() {
    let stream = make_video_stream();

    let data_pattern = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42];
    let mut pkt = Packet::from_data(data_pattern.clone());
    pkt.stream_index = 0;
    pkt.pts = 0;
    pkt.dts = 0;
    pkt.is_keyframe = true;

    let mut io = mux_packets(&[stream], &[pkt]);

    // 解封装
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);

    let mut demuxer = registry.create_demuxer(FormatId::Matroska).unwrap();
    demuxer.open(&mut io).unwrap();
    let streams = demuxer.streams();

    assert!(!streams.is_empty(), "应有至少一个流");

    // 读取数据包
    let result = demuxer.read_packet(&mut io);
    assert!(result.is_ok(), "应能读取数据包");
    let read_pkt = result.unwrap();
    assert_eq!(
        read_pkt.data.as_ref(),
        data_pattern.as_slice(),
        "数据应一致"
    );
}

#[test]
fn test_音视频_封装() {
    let vs = make_video_stream();
    let aus = make_audio_stream();

    let mut packets = Vec::new();

    // 视频包
    for i in 0..5 {
        let mut pkt = Packet::from_data(vec![0xBB; 200]);
        pkt.stream_index = 0;
        pkt.pts = i * 33;
        pkt.dts = i * 33;
        pkt.is_keyframe = i == 0;
        packets.push(pkt);
    }

    // 音频包
    for i in 0..8 {
        let mut pkt = Packet::from_data(vec![0xCC; 50]);
        pkt.stream_index = 1;
        pkt.pts = i * 23;
        pkt.dts = i * 23;
        pkt.is_keyframe = true;
        packets.push(pkt);
    }

    let mut io = mux_packets(&[vs, aus], &packets);
    let pos = io.position().unwrap();
    assert_eq!(pos, 0);
}

#[test]
fn test_关键帧标记() {
    let stream = make_video_stream();
    let mut packets = Vec::new();

    // 关键帧
    let mut kf = Packet::from_data(vec![0xFF; 100]);
    kf.stream_index = 0;
    kf.pts = 0;
    kf.dts = 0;
    kf.is_keyframe = true;
    packets.push(kf);

    // 非关键帧
    let mut nkf = Packet::from_data(vec![0xEE; 80]);
    nkf.stream_index = 0;
    nkf.pts = 33;
    nkf.dts = 33;
    nkf.is_keyframe = false;
    packets.push(nkf);

    let mut io = mux_packets(&[stream], &packets);

    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry.create_demuxer(FormatId::Matroska).unwrap();
    demuxer.open(&mut io).unwrap();

    let pkt1 = demuxer.read_packet(&mut io).unwrap();
    assert!(pkt1.is_keyframe, "第一个包应为关键帧");

    let pkt2 = demuxer.read_packet(&mut io).unwrap();
    assert!(!pkt2.is_keyframe, "第二个包应为非关键帧");
}
