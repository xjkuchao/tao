//! Matroska/WebM 解封装器集成测试.
//!
//! 在内存中构造符合 Matroska 规范的二进制数据, 测试完整的
//! 探测 → 打开 → 读取流信息 → 读取数据包 流程.

use tao_codec::CodecId;
use tao_core::MediaType;
use tao_format::demuxers::mkv::{MkvDemuxer, MkvProbe, ebml};
use tao_format::io::{IoContext, MemoryBackend};
use tao_format::probe::FormatProbe;
use tao_format::stream::StreamParams;

/// 辅助: 写入 EBML 变长整数 (ID, 不掩码)
fn write_vint_id(buf: &mut Vec<u8>, id: u32) {
    if id <= 0xFF {
        buf.push(id as u8);
    } else if id <= 0xFFFF {
        buf.push((id >> 8) as u8);
        buf.push(id as u8);
    } else if id <= 0xFF_FFFF {
        buf.push((id >> 16) as u8);
        buf.push((id >> 8) as u8);
        buf.push(id as u8);
    } else {
        buf.push((id >> 24) as u8);
        buf.push((id >> 16) as u8);
        buf.push((id >> 8) as u8);
        buf.push(id as u8);
    }
}

/// 辅助: 写入 EBML 变长整数 (大小, 加标记位)
fn write_vint_size(buf: &mut Vec<u8>, size: u64) {
    if size < 0x7F {
        buf.push(0x80 | size as u8);
    } else if size < 0x3FFF {
        buf.push(0x40 | (size >> 8) as u8);
        buf.push(size as u8);
    } else {
        buf.push(0x20 | (size >> 16) as u8);
        buf.push((size >> 8) as u8);
        buf.push(size as u8);
    }
}

/// 辅助: 写入 EBML 元素 (ID + size + content)
fn write_element(buf: &mut Vec<u8>, id: u32, content: &[u8]) {
    write_vint_id(buf, id);
    write_vint_size(buf, content.len() as u64);
    buf.extend_from_slice(content);
}

/// 辅助: 写入 uint 元素
fn write_uint_element(buf: &mut Vec<u8>, id: u32, val: u64) {
    let bytes = if val == 0 {
        vec![0]
    } else {
        let mut b = val.to_be_bytes().to_vec();
        while b.len() > 1 && b[0] == 0 {
            b.remove(0);
        }
        b
    };
    write_element(buf, id, &bytes);
}

/// 辅助: 写入 float 元素 (8 字节)
fn write_float_element(buf: &mut Vec<u8>, id: u32, val: f64) {
    let bytes = val.to_bits().to_be_bytes();
    write_element(buf, id, &bytes);
}

/// 辅助: 写入 string 元素
fn write_string_element(buf: &mut Vec<u8>, id: u32, s: &str) {
    write_element(buf, id, s.as_bytes());
}

/// 构造包含视频+音频的 MKV 文件
fn build_mkv_with_clusters(doc_type: &str, num_clusters: usize) -> Vec<u8> {
    let mut data = Vec::new();

    // EBML Header
    let mut ebml_content = Vec::new();
    write_string_element(&mut ebml_content, ebml::EBML_DOC_TYPE, doc_type);
    write_element(&mut data, ebml::EBML_HEADER, &ebml_content);

    // Segment (unknown size)
    write_vint_id(&mut data, ebml::SEGMENT);
    // 8-byte unknown size
    data.push(0x01);
    data.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

    // Segment Info
    let mut info = Vec::new();
    write_uint_element(&mut info, ebml::INFO_TIMESCALE, 1_000_000);
    write_float_element(&mut info, ebml::INFO_DURATION, 10_000.0); // 10s
    write_string_element(&mut info, ebml::INFO_MUXING_APP, "tao-test");
    write_element(&mut data, ebml::SEGMENT_INFO, &info);

    // Tracks: 1 video (H.264), 1 audio (AAC)
    let mut tracks = Vec::new();
    {
        let mut vt = Vec::new();
        write_uint_element(&mut vt, ebml::TRACK_NUMBER, 1);
        write_uint_element(&mut vt, ebml::TRACK_TYPE, 1);
        write_string_element(&mut vt, ebml::TRACK_CODEC_ID, "V_MPEG4/ISO/AVC");
        // default_duration: 33333333 ns = ~30fps
        write_uint_element(&mut vt, ebml::TRACK_DEFAULT_DURATION, 33_333_333);
        // codec private (dummy SPS/PPS)
        write_element(
            &mut vt,
            ebml::TRACK_CODEC_PRIVATE,
            &[0x01, 0x64, 0x00, 0x1F],
        );
        let mut vs = Vec::new();
        write_uint_element(&mut vs, ebml::VIDEO_PIXEL_WIDTH, 1920);
        write_uint_element(&mut vs, ebml::VIDEO_PIXEL_HEIGHT, 1080);
        write_element(&mut vt, ebml::VIDEO_SETTINGS, &vs);
        write_element(&mut tracks, ebml::TRACK_ENTRY, &vt);
    }
    {
        let mut at = Vec::new();
        write_uint_element(&mut at, ebml::TRACK_NUMBER, 2);
        write_uint_element(&mut at, ebml::TRACK_TYPE, 2);
        write_string_element(&mut at, ebml::TRACK_CODEC_ID, "A_AAC");
        let mut als = Vec::new();
        let sr_bytes = (44100.0f64).to_bits().to_be_bytes();
        write_element(&mut als, ebml::AUDIO_SAMPLING_FREQ, &sr_bytes);
        write_uint_element(&mut als, ebml::AUDIO_CHANNELS, 2);
        write_uint_element(&mut als, ebml::AUDIO_BIT_DEPTH, 16);
        write_element(&mut at, ebml::AUDIO_SETTINGS, &als);
        write_element(&mut tracks, ebml::TRACK_ENTRY, &at);
    }
    write_element(&mut data, ebml::TRACKS, &tracks);

    // Clusters
    for i in 0..num_clusters {
        let mut cluster = Vec::new();
        let cluster_ts = i as u64 * 1000; // 每个 Cluster 间隔 1000ms
        write_uint_element(&mut cluster, ebml::CLUSTER_TIMESTAMP, cluster_ts);

        // 视频帧 (keyframe on first, non-keyframe on others)
        {
            let mut block = vec![
                0x81, // track number = 1
                0x00, 0x00, // relative timestamp = 0
            ];
            if i == 0 {
                block.push(0x80);
            } else {
                block.push(0x00);
            } // keyframe flag
            // 帧数据: 标识簇索引
            block.extend_from_slice(&[0xF0 | (i as u8), 0x00, 0x11, 0x22]);
            write_element(&mut cluster, ebml::SIMPLE_BLOCK, &block);
        }

        // 音频帧 (always keyframe)
        {
            let mut block = vec![
                0x82, // track number = 2
                0x00, 0x00, // relative timestamp = 0
                0x80, // keyframe
            ];
            block.extend_from_slice(&[0xA0 | (i as u8), 0xBB, 0xCC]);
            write_element(&mut cluster, ebml::SIMPLE_BLOCK, &block);
        }

        write_element(&mut data, ebml::CLUSTER, &cluster);
    }

    data
}

#[test]
fn test_探测_matroska() {
    let probe = MkvProbe;
    let mkv = build_mkv_with_clusters("matroska", 1);
    assert_eq!(
        probe.probe(&mkv[..32], Some("test.mkv")),
        Some(tao_format::probe::SCORE_MAX)
    );
}

#[test]
fn test_探测_webm() {
    let probe = MkvProbe;
    assert!(probe.probe(&[], Some("video.webm")).is_some());
}

#[test]
fn test_视频轨道信息() {
    let mkv = build_mkv_with_clusters("matroska", 1);
    let backend = MemoryBackend::from_data(mkv);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = MkvDemuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 2);

    // 视频流
    let vs = &streams[0];
    assert_eq!(vs.media_type, MediaType::Video);
    assert_eq!(vs.codec_id, CodecId::H264);
    assert_eq!(vs.extra_data, vec![0x01, 0x64, 0x00, 0x1F]);
    if let StreamParams::Video(ref v) = vs.params {
        assert_eq!(v.width, 1920);
        assert_eq!(v.height, 1080);
    } else {
        panic!("视频流参数类型错误");
    }
}

#[test]
fn test_音频轨道信息() {
    let mkv = build_mkv_with_clusters("matroska", 1);
    let backend = MemoryBackend::from_data(mkv);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = MkvDemuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    let als = &streams[1];
    assert_eq!(als.media_type, MediaType::Audio);
    assert_eq!(als.codec_id, CodecId::Aac);
    if let StreamParams::Audio(ref a) = als.params {
        assert_eq!(a.sample_rate, 44100);
        assert_eq!(a.channel_layout.channels, 2);
    } else {
        panic!("音频流参数类型错误");
    }
}

#[test]
fn test_文件时长() {
    let mkv = build_mkv_with_clusters("matroska", 3);
    let backend = MemoryBackend::from_data(mkv);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = MkvDemuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let dur = demuxer.duration().expect("应该有时长");
    assert!((dur - 10.0).abs() < 0.01, "时长应为 10 秒, 实际={dur}");
}

#[test]
fn test_读取多个_cluster_的数据包() {
    let mkv = build_mkv_with_clusters("matroska", 3);
    let backend = MemoryBackend::from_data(mkv);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = MkvDemuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let mut video_packets = 0;
    let mut audio_packets = 0;
    let mut last_video_pts = -1i64;

    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                if pkt.stream_index == 0 {
                    // 视频
                    video_packets += 1;
                    assert!(pkt.pts >= last_video_pts, "视频 PTS 应单调递增");
                    last_video_pts = pkt.pts;
                } else {
                    // 音频
                    audio_packets += 1;
                }
            }
            Err(tao_core::TaoError::Eof) => break,
            Err(e) => panic!("读取数据包失败: {e}"),
        }
    }

    assert_eq!(video_packets, 3, "应该有 3 个视频包 (每个 Cluster 一个)");
    assert_eq!(audio_packets, 3, "应该有 3 个音频包");
}

#[test]
fn test_关键帧标记() {
    let mkv = build_mkv_with_clusters("matroska", 2);
    let backend = MemoryBackend::from_data(mkv);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = MkvDemuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    // Cluster 0: video keyframe, audio keyframe
    let pkt0 = demuxer.read_packet(&mut io).unwrap();
    assert_eq!(pkt0.stream_index, 0);
    assert!(pkt0.is_keyframe, "第一个视频帧应该是关键帧");

    let pkt1 = demuxer.read_packet(&mut io).unwrap();
    assert_eq!(pkt1.stream_index, 1);
    assert!(pkt1.is_keyframe, "音频帧应该是关键帧");

    // Cluster 1: video non-keyframe
    let pkt2 = demuxer.read_packet(&mut io).unwrap();
    assert_eq!(pkt2.stream_index, 0);
    assert!(!pkt2.is_keyframe, "第二个视频帧不应该是关键帧");
}

fn create_registry() -> tao_format::registry::FormatRegistry {
    let mut registry = tao_format::registry::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    registry
}

#[test]
fn test_注册表_包含mkv() {
    let registry = create_registry();
    let demuxers = registry.list_demuxers();
    assert!(
        demuxers.iter().any(|d| d.1 == "matroska"),
        "注册表应该包含 matroska 解封装器"
    );
}

#[test]
fn test_注册表_探测mkv() {
    let registry = create_registry();
    // EBML magic bytes
    let data = [0x1A, 0x45, 0xDF, 0xA3, 0x00, 0x00, 0x00];
    let result = registry.probe(&data, Some("test.mkv"));
    assert!(result.is_some(), "探测 MKV 应该成功");
}
