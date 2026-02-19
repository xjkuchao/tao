//! MP4/MOV 解封装器集成测试.
//!
//! 通过构造完整的 MP4 文件 (ftyp + moov + mdat) 来验证解封装器的
//! Box 解析、流发现、采样表索引和数据读取的完整流程.

use tao_codec::CodecId;
use tao_core::{MediaType, Rational};
use tao_format::demuxers::mp4::{Mp4Demuxer, Mp4Probe};
use tao_format::io::{IoContext, MemoryBackend};
use tao_format::probe::{FormatProbe, SCORE_EXTENSION, SCORE_MAX};

// ========================
// 辅助函数: 构造 MP4 Box
// ========================

/// 构造一个普通 box
fn build_box(tag: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let size = (8 + content.len()) as u32;
    let mut data = Vec::with_capacity(size as usize);
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(tag);
    data.extend_from_slice(content);
    data
}

/// 构造一个 FullBox (version + flags + content)
fn build_fullbox(tag: &[u8; 4], version: u8, flags: u32, content: &[u8]) -> Vec<u8> {
    let mut full = vec![
        version,
        ((flags >> 16) & 0xFF) as u8,
        ((flags >> 8) & 0xFF) as u8,
        (flags & 0xFF) as u8,
    ];
    full.extend_from_slice(content);
    build_box(tag, &full)
}

/// 构造 ftyp box
fn build_ftyp() -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(b"isom"); // major brand
    content.extend_from_slice(&0u32.to_be_bytes()); // minor version
    content.extend_from_slice(b"isom"); // compatible brand
    content.extend_from_slice(b"mp41"); // compatible brand
    build_box(b"ftyp", &content)
}

/// 构造 mvhd box (version 0)
fn build_mvhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    content.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    content.extend_from_slice(&timescale.to_be_bytes());
    content.extend_from_slice(&duration.to_be_bytes());
    content.extend_from_slice(&0x00010000u32.to_be_bytes()); // rate (1.0)
    content.extend_from_slice(&0x0100u16.to_be_bytes()); // volume (1.0)
    content.extend_from_slice(&[0u8; 10]); // reserved
    // 矩阵 (identity)
    content.extend_from_slice(&0x00010000u32.to_be_bytes());
    content.extend_from_slice(&[0u8; 12]);
    content.extend_from_slice(&0x00010000u32.to_be_bytes());
    content.extend_from_slice(&[0u8; 12]);
    content.extend_from_slice(&0x40000000u32.to_be_bytes());
    // pre_defined
    content.extend_from_slice(&[0u8; 24]);
    // next_track_id
    content.extend_from_slice(&2u32.to_be_bytes());
    build_fullbox(b"mvhd", 0, 0, &content)
}

/// 构造 tkhd box (version 0)
fn build_tkhd(track_id: u32, duration: u32, width: u32, height: u32) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_be_bytes()); // creation
    content.extend_from_slice(&0u32.to_be_bytes()); // modification
    content.extend_from_slice(&track_id.to_be_bytes());
    content.extend_from_slice(&0u32.to_be_bytes()); // reserved
    content.extend_from_slice(&duration.to_be_bytes());
    content.extend_from_slice(&[0u8; 8]); // reserved
    content.extend_from_slice(&0u16.to_be_bytes()); // layer
    content.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
    content.extend_from_slice(&0u16.to_be_bytes()); // volume (视频=0)
    content.extend_from_slice(&0u16.to_be_bytes()); // reserved
    // 矩阵 (identity)
    content.extend_from_slice(&0x00010000u32.to_be_bytes());
    content.extend_from_slice(&[0u8; 12]);
    content.extend_from_slice(&0x00010000u32.to_be_bytes());
    content.extend_from_slice(&[0u8; 12]);
    content.extend_from_slice(&0x40000000u32.to_be_bytes());
    // 宽高 (16.16 定点数)
    content.extend_from_slice(&(width << 16).to_be_bytes());
    content.extend_from_slice(&(height << 16).to_be_bytes());
    build_fullbox(b"tkhd", 0, 3, &content) // flags=3 (track_enabled | track_in_movie)
}

/// 构造 mdhd box (version 0)
fn build_mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_be_bytes()); // creation
    content.extend_from_slice(&0u32.to_be_bytes()); // modification
    content.extend_from_slice(&timescale.to_be_bytes());
    content.extend_from_slice(&duration.to_be_bytes());
    content.extend_from_slice(&0u16.to_be_bytes()); // language (undetermined)
    content.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    build_fullbox(b"mdhd", 0, 0, &content)
}

/// 构造 hdlr box
fn build_hdlr(handler_type: &[u8; 4]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    content.extend_from_slice(handler_type);
    content.extend_from_slice(&[0u8; 12]); // reserved
    content.push(0); // name (null terminator)
    build_fullbox(b"hdlr", 0, 0, &content)
}

/// 构造视频 stsd box (avc1 条目)
fn build_video_stsd(width: u16, height: u16) -> Vec<u8> {
    let mut entry = Vec::new();
    // 通用 Sample Entry 部分
    entry.extend_from_slice(&[0u8; 6]); // reserved
    entry.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index

    // Visual Sample Entry 部分
    entry.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    entry.extend_from_slice(&0u16.to_be_bytes()); // reserved
    entry.extend_from_slice(&[0u8; 12]); // pre_defined + reserved
    entry.extend_from_slice(&width.to_be_bytes());
    entry.extend_from_slice(&height.to_be_bytes());
    entry.extend_from_slice(&0x00480000u32.to_be_bytes()); // horiz_res (72 dpi)
    entry.extend_from_slice(&0x00480000u32.to_be_bytes()); // vert_res
    entry.extend_from_slice(&0u32.to_be_bytes()); // reserved
    entry.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    entry.extend_from_slice(&[0u8; 32]); // compressor name
    entry.extend_from_slice(&0x0018u16.to_be_bytes()); // depth (24)
    entry.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined

    let entry_box = build_box(b"avc1", &entry);

    let mut stsd_content = Vec::new();
    stsd_content.extend_from_slice(&1u32.to_be_bytes()); // entry_count
    stsd_content.extend_from_slice(&entry_box);
    build_fullbox(b"stsd", 0, 0, &stsd_content)
}

/// 构造音频 stsd box (mp4a 条目)
fn build_audio_stsd(sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut entry = Vec::new();
    // 通用 Sample Entry 部分
    entry.extend_from_slice(&[0u8; 6]); // reserved
    entry.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index

    // Audio Sample Entry 部分
    entry.extend_from_slice(&[0u8; 8]); // reserved
    entry.extend_from_slice(&channels.to_be_bytes());
    entry.extend_from_slice(&16u16.to_be_bytes()); // sample_size
    entry.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
    entry.extend_from_slice(&0u16.to_be_bytes()); // reserved
    entry.extend_from_slice(&(sample_rate << 16).to_be_bytes()); // sample_rate (16.16)

    let entry_box = build_box(b"mp4a", &entry);

    let mut stsd_content = Vec::new();
    stsd_content.extend_from_slice(&1u32.to_be_bytes()); // entry_count
    stsd_content.extend_from_slice(&entry_box);
    build_fullbox(b"stsd", 0, 0, &stsd_content)
}

/// 构造 stts box
fn build_stts(entries: &[(u32, u32)]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (count, delta) in entries {
        content.extend_from_slice(&count.to_be_bytes());
        content.extend_from_slice(&delta.to_be_bytes());
    }
    build_fullbox(b"stts", 0, 0, &content)
}

/// 构造 stsc box
fn build_stsc(entries: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (first_chunk, samples_per_chunk, desc_idx) in entries {
        content.extend_from_slice(&first_chunk.to_be_bytes());
        content.extend_from_slice(&samples_per_chunk.to_be_bytes());
        content.extend_from_slice(&desc_idx.to_be_bytes());
    }
    build_fullbox(b"stsc", 0, 0, &content)
}

/// 构造 stsz box (逐样本大小)
fn build_stsz(sizes: &[u32]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_be_bytes()); // default_sample_size = 0
    content.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
    for size in sizes {
        content.extend_from_slice(&size.to_be_bytes());
    }
    build_fullbox(b"stsz", 0, 0, &content)
}

/// 构造 stsz box (统一大小)
fn build_stsz_uniform(sample_size: u32, count: u32) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&sample_size.to_be_bytes());
    content.extend_from_slice(&count.to_be_bytes());
    build_fullbox(b"stsz", 0, 0, &content)
}

/// 构造 stco box
fn build_stco(offsets: &[u32]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
    for offset in offsets {
        content.extend_from_slice(&offset.to_be_bytes());
    }
    build_fullbox(b"stco", 0, 0, &content)
}

/// 构造 stss box (同步采样)
fn build_stss(sync_samples: &[u32]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&(sync_samples.len() as u32).to_be_bytes());
    for s in sync_samples {
        content.extend_from_slice(&s.to_be_bytes());
    }
    build_fullbox(b"stss", 0, 0, &content)
}

// ========================
// 测试
// ========================

/// 构造包含一个视频轨道的完整 MP4
fn build_video_mp4(
    width: u16,
    height: u16,
    timescale: u32,
    sample_sizes: &[u32],
    frame_delta: u32,
) -> Vec<u8> {
    let total_samples = sample_sizes.len() as u32;
    let total_duration = total_samples * frame_delta;

    // 1) 构造 mdat 内容
    let total_mdat_bytes: u32 = sample_sizes.iter().sum();
    let mut mdat_content = Vec::with_capacity(total_mdat_bytes as usize);
    for (i, &size) in sample_sizes.iter().enumerate() {
        // 每个采样用不同的填充字节标识
        mdat_content.extend(std::iter::repeat_n((i & 0xFF) as u8, size as usize));
    }

    // 先构造 moov 以计算偏移
    let ftyp = build_ftyp();

    // stbl 子 box 们
    let stsd = build_video_stsd(width, height);
    let stts = build_stts(&[(total_samples, frame_delta)]);
    let stsc = build_stsc(&[(1, total_samples, 1)]); // 所有采样在一个块中
    let stsz = build_stsz(sample_sizes);
    // stco 偏移需要知道 moov 的大小, 先用占位符
    let stco_placeholder = build_stco(&[0]);
    let stss_box = build_stss(&[1]); // 只有第一帧是关键帧

    let stbl_content = [
        stsd.as_slice(),
        stts.as_slice(),
        stsc.as_slice(),
        stsz.as_slice(),
        stco_placeholder.as_slice(),
        stss_box.as_slice(),
    ]
    .concat();
    let stbl = build_box(b"stbl", &stbl_content);

    let dinf = build_box(
        b"dinf",
        &build_box(b"dref", &{
            let mut d = vec![0, 0, 0, 0]; // version + flags
            d.extend_from_slice(&1u32.to_be_bytes()); // entry_count
            d.extend_from_slice(&build_box(b"url ", &[0, 0, 0, 1])); // self-contained
            d
        }),
    );

    let hdlr = build_hdlr(b"vide");
    let mdhd = build_mdhd(timescale, total_duration);
    let tkhd = build_tkhd(1, total_duration, width as u32, height as u32);
    let mvhd = build_mvhd(timescale, total_duration);

    // 第一次构造 moov (用占位符 stco) 来计算大小
    let minf = build_box(b"minf", &[dinf.clone(), stbl].concat());
    let mdia = build_box(b"mdia", &[mdhd.clone(), hdlr.clone(), minf].concat());
    let trak = build_box(b"trak", &[tkhd.clone(), mdia].concat());
    let moov = build_box(b"moov", &[mvhd.clone(), trak].concat());

    // 计算 mdat 偏移
    let mdat_header_size = 8u32;
    let mdat_data_start = ftyp.len() as u32 + moov.len() as u32 + mdat_header_size;

    // 重新构造 stco 用正确偏移
    let stco_correct = build_stco(&[mdat_data_start]);
    let stbl_correct = [
        stsd.as_slice(),
        stts.as_slice(),
        stsc.as_slice(),
        stsz.as_slice(),
        stco_correct.as_slice(),
        stss_box.as_slice(),
    ]
    .concat();
    let stbl_c = build_box(b"stbl", &stbl_correct);
    let minf_c = build_box(b"minf", &[dinf, stbl_c].concat());
    let mdia_c = build_box(b"mdia", &[mdhd, hdlr, minf_c].concat());
    let trak_c = build_box(b"trak", &[tkhd, mdia_c].concat());
    let moov_c = build_box(b"moov", &[mvhd, trak_c].concat());

    // 验证偏移一致
    assert_eq!(moov.len(), moov_c.len(), "moov 大小不应改变");

    let mdat = build_box(b"mdat", &mdat_content);

    [ftyp, moov_c, mdat].concat()
}

/// 构造包含一个音频轨道的 MP4
fn build_audio_mp4(
    sample_rate: u32,
    channels: u16,
    timescale: u32,
    frame_size: u32,
    num_frames: u32,
) -> Vec<u8> {
    let total_duration = num_frames * frame_size;

    // 每帧 100 字节
    let frame_byte_size = 100u32;
    let total_bytes = num_frames * frame_byte_size;
    let mut mdat_content = Vec::with_capacity(total_bytes as usize);
    for i in 0..num_frames {
        let pattern = ((i + 1) & 0xFF) as u8;
        mdat_content.extend(std::iter::repeat_n(pattern, frame_byte_size as usize));
    }

    let ftyp = build_ftyp();

    let stsd = build_audio_stsd(sample_rate, channels);
    let stts = build_stts(&[(num_frames, frame_size)]);
    let stsc = build_stsc(&[(1, num_frames, 1)]);
    let stsz = build_stsz_uniform(frame_byte_size, num_frames);
    let stco_placeholder = build_stco(&[0]);

    let stbl_content = [
        stsd.as_slice(),
        stts.as_slice(),
        stsc.as_slice(),
        stsz.as_slice(),
        stco_placeholder.as_slice(),
    ]
    .concat();
    let stbl = build_box(b"stbl", &stbl_content);

    let dinf = build_box(
        b"dinf",
        &build_box(b"dref", &{
            let mut d = vec![0, 0, 0, 0];
            d.extend_from_slice(&1u32.to_be_bytes());
            d.extend_from_slice(&build_box(b"url ", &[0, 0, 0, 1]));
            d
        }),
    );

    let hdlr = build_hdlr(b"soun");
    let mdhd = build_mdhd(timescale, total_duration);
    let tkhd = build_tkhd(1, total_duration, 0, 0);
    let mvhd = build_mvhd(timescale, total_duration);

    // 第一次构造 moov (用占位符 stco) 来计算大小
    let minf = build_box(b"minf", &[dinf.clone(), stbl].concat());
    let mdia = build_box(b"mdia", &[mdhd.clone(), hdlr.clone(), minf].concat());
    let trak = build_box(b"trak", &[tkhd.clone(), mdia].concat());
    let moov = build_box(b"moov", &[mvhd.clone(), trak].concat());

    let mdat_header_size = 8u32;
    let mdat_data_start = ftyp.len() as u32 + moov.len() as u32 + mdat_header_size;

    let stco_correct = build_stco(&[mdat_data_start]);
    let stbl_correct = [
        stsd.as_slice(),
        stts.as_slice(),
        stsc.as_slice(),
        stsz.as_slice(),
        stco_correct.as_slice(),
    ]
    .concat();
    let stbl_c = build_box(b"stbl", &stbl_correct);
    let minf_c = build_box(b"minf", &[dinf, stbl_c].concat());
    let mdia_c = build_box(b"mdia", &[mdhd, hdlr, minf_c].concat());
    let trak_c = build_box(b"trak", &[tkhd, mdia_c].concat());
    let moov_c = build_box(b"moov", &[mvhd, trak_c].concat());

    assert_eq!(moov.len(), moov_c.len());

    let mdat = build_box(b"mdat", &mdat_content);

    [ftyp, moov_c, mdat].concat()
}

// ========================
// 探测测试
// ========================

#[test]
fn test_probe_mp4_ftyp() {
    let probe = Mp4Probe;
    let mp4 = build_ftyp();
    assert_eq!(probe.probe(&mp4, None), Some(SCORE_MAX));
}

#[test]
fn test_probe_mp4_extension() {
    let probe = Mp4Probe;
    assert_eq!(probe.probe(&[], Some("video.mp4")), Some(SCORE_EXTENSION),);
    assert_eq!(probe.probe(&[], Some("audio.m4a")), Some(SCORE_EXTENSION),);
    assert_eq!(probe.probe(&[], Some("movie.mov")), Some(SCORE_EXTENSION),);
    assert!(probe.probe(&[], Some("music.wav")).is_none());
}

// ========================
// 视频轨道测试
// ========================

#[test]
fn test_video_track_basic_info() {
    let sample_sizes = vec![500, 100, 100, 100, 100];
    let mp4 = build_video_mp4(1920, 1080, 24000, &sample_sizes, 1000);

    let backend = MemoryBackend::from_data(mp4);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1, "应该有 1 个视频轨道");

    let stream = &streams[0];
    assert_eq!(stream.media_type, MediaType::Video);
    assert_eq!(stream.codec_id, CodecId::H264);
    assert_eq!(stream.time_base, Rational::new(1, 24000));
    assert_eq!(stream.nb_frames, 5);

    if let tao_format::stream::StreamParams::Video(ref v) = stream.params {
        assert_eq!(v.width, 1920);
        assert_eq!(v.height, 1080);
    } else {
        panic!("应该是视频流参数");
    }
}

#[test]
fn test_video_track_read_packets() {
    let sample_sizes = vec![500, 100, 150, 200, 50];
    let mp4 = build_video_mp4(640, 480, 30000, &sample_sizes, 1001);

    let backend = MemoryBackend::from_data(mp4);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    // 读取所有 5 个包
    for (i, &expected_size) in sample_sizes.iter().enumerate() {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.stream_index, 0);
        assert_eq!(pkt.data.len(), expected_size as usize);
        assert_eq!(pkt.pts, (i as i64) * 1001);

        // 验证数据内容 (每个采样用不同字节填充)
        let expected_byte = (i & 0xFF) as u8;
        assert!(
            pkt.data.iter().all(|&b| b == expected_byte),
            "采样 {} 数据不匹配",
            i,
        );

        // 只有第一帧是关键帧
        if i == 0 {
            assert!(pkt.is_keyframe, "采样 0 应该是关键帧");
        } else {
            assert!(!pkt.is_keyframe, "采样 {} 不应是关键帧", i);
        }
    }

    // 读完后应该返回 EOF
    let eof = demuxer.read_packet(&mut io);
    assert!(eof.is_err());
}

// ========================
// 音频轨道测试
// ========================

#[test]
fn test_audio_track_basic_info() {
    let mp4 = build_audio_mp4(44100, 2, 44100, 1024, 10);

    let backend = MemoryBackend::from_data(mp4);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);

    let stream = &streams[0];
    assert_eq!(stream.media_type, MediaType::Audio);
    assert_eq!(stream.codec_id, CodecId::Aac);
    assert_eq!(stream.nb_frames, 10);

    if let tao_format::stream::StreamParams::Audio(ref a) = stream.params {
        assert_eq!(a.sample_rate, 44100);
        assert_eq!(a.channel_layout.channels, 2);
    } else {
        panic!("应该是音频流参数");
    }
}

#[test]
fn test_audio_track_read_packets() {
    let num_frames = 5u32;
    let frame_byte_size = 100u32;
    let mp4 = build_audio_mp4(48000, 2, 48000, 1024, num_frames);

    let backend = MemoryBackend::from_data(mp4);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    for i in 0..num_frames {
        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.stream_index, 0);
        assert_eq!(pkt.data.len(), frame_byte_size as usize);
        assert_eq!(pkt.pts, (i as i64) * 1024);

        // 无 stss 表示所有帧都是关键帧 (音频通常如此)
        assert!(pkt.is_keyframe);
    }

    let eof = demuxer.read_packet(&mut io);
    assert!(eof.is_err());
}

// ========================
// 时长测试
// ========================

#[test]
fn test_file_duration() {
    // timescale=1000, duration=5000 → 5 秒
    let sample_sizes = vec![100; 5];
    let mp4 = build_video_mp4(320, 240, 1000, &sample_sizes, 1000);

    let backend = MemoryBackend::from_data(mp4);
    let mut io = IoContext::new(Box::new(backend));
    let mut demuxer = Mp4Demuxer::create().unwrap();
    demuxer.open(&mut io).unwrap();

    let duration = demuxer.duration().expect("应该有时长");
    assert!(
        (duration - 5.0).abs() < 0.01,
        "时长应约为 5 秒, 实际={}",
        duration,
    );
}

// ========================
// 注册表集成测试
// ========================

fn create_registry() -> tao_format::FormatRegistry {
    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    registry
}

#[test]
fn test_registry_contains_mp4() {
    let registry = create_registry();
    let demuxers = registry.list_demuxers();
    assert!(
        demuxers.iter().any(|d| d.1 == "mp4"),
        "注册表应包含 mp4 解封装器",
    );
}

#[test]
fn test_registry_probe_mp4() {
    let registry = create_registry();
    let mp4 = build_ftyp();
    let result = registry.probe(&mp4, Some("test.mp4"));
    assert!(result.is_some(), "应该能探测到 MP4 格式");
    let probe_result = result.unwrap();
    assert_eq!(probe_result.format_id, tao_format::format_id::FormatId::Mp4);
}
