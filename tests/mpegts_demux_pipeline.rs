//! MPEG-TS 解封装器集成测试

use tao_core::{MediaType, Rational, TaoError};
use tao_format::io::{IoContext, MemoryBackend};

// ============================================================
// 辅助函数: 构建 TS 包
// ============================================================

const TS_PACKET_SIZE: usize = 188;
const TS_SYNC_BYTE: u8 = 0x47;

/// 构造普通 TS 包 (无 adaptation field)
fn build_ts_packet(pid: u16, pusi: bool, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    pkt[0] = TS_SYNC_BYTE;
    pkt[1] = (if pusi { 0x40 } else { 0x00 }) | ((pid >> 8) as u8 & 0x1F);
    pkt[2] = pid as u8;
    pkt[3] = 0x10; // AFC=01, CC=0
    let n = payload.len().min(TS_PACKET_SIZE - 4);
    pkt[4..4 + n].copy_from_slice(&payload[..n]);
    pkt
}

/// 构造带 adaptation field 的 TS 包
fn build_ts_packet_with_af(
    pid: u16,
    pusi: bool,
    random_access: bool,
    payload: &[u8],
) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    pkt[0] = TS_SYNC_BYTE;
    pkt[1] = (if pusi { 0x40 } else { 0x00 }) | ((pid >> 8) as u8 & 0x1F);
    pkt[2] = pid as u8;
    pkt[3] = 0x30; // AFC=11
    let af_flags = if random_access { 0x40 } else { 0x00 };
    let payload_space = TS_PACKET_SIZE - 4 - 2;
    let n = payload.len().min(payload_space);
    let stuffing = payload_space - n;
    pkt[4] = (1 + stuffing) as u8;
    pkt[5] = af_flags;
    for i in 0..stuffing {
        pkt[6 + i] = 0xFF;
    }
    let start = 6 + stuffing;
    pkt[start..start + n].copy_from_slice(&payload[..n]);
    pkt
}

/// 编码 33-bit PTS 到 5 字节
fn encode_pts(pts: u64) -> [u8; 5] {
    [
        0x21 | ((((pts >> 30) as u8) & 0x07) << 1),
        (pts >> 22) as u8,
        0x01 | ((((pts >> 15) as u8) & 0x7F) << 1),
        (pts >> 7) as u8,
        0x01 | (((pts as u8) & 0x7F) << 1),
    ]
}

/// 构造 PES 包
fn build_pes(stream_id: u8, pts: Option<u64>, data: &[u8]) -> Vec<u8> {
    let mut pes = Vec::new();
    pes.extend_from_slice(&[0x00, 0x00, 0x01]);
    pes.push(stream_id);
    let has_pts = pts.is_some();
    let hdr_ext = if has_pts { 5 } else { 0 };
    let pes_len = 3 + hdr_ext + data.len();
    pes.push((pes_len >> 8) as u8);
    pes.push(pes_len as u8);
    pes.push(0x80); // marker
    pes.push(if has_pts { 0x80 } else { 0x00 });
    pes.push(hdr_ext as u8);
    if let Some(v) = pts {
        pes.extend_from_slice(&encode_pts(v));
    }
    pes.extend_from_slice(data);
    pes
}

/// 构造 PAT
fn build_pat(pmt_pid: u16) -> [u8; TS_PACKET_SIZE] {
    let mut s = Vec::new();
    s.push(0x00); // pointer
    s.push(0x00); // table_id
    let len: u16 = 13;
    s.push(0xB0 | ((len >> 8) as u8 & 0x0F));
    s.push(len as u8);
    s.extend_from_slice(&[0x00, 0x01]); // ts_id
    s.push(0xC1);
    s.push(0x00);
    s.push(0x00);
    s.push(0x00);
    s.push(0x01); // program_number=1
    s.push(0xE0 | ((pmt_pid >> 8) as u8 & 0x1F));
    s.push(pmt_pid as u8);
    s.extend_from_slice(&[0x00; 4]); // CRC
    build_ts_packet(0x0000, true, &s)
}

/// 构造 PMT
fn build_pmt(pmt_pid: u16, entries: &[(u8, u16)]) -> [u8; TS_PACKET_SIZE] {
    let mut s = Vec::new();
    s.push(0x00); // pointer
    s.push(0x02); // table_id
    let sec_len = 9 + entries.len() * 5 + 4;
    s.push(0xB0 | ((sec_len >> 8) as u8 & 0x0F));
    s.push(sec_len as u8);
    s.extend_from_slice(&[0x00, 0x01]); // program_number
    s.push(0xC1);
    s.push(0x00);
    s.push(0x00);
    let pcr_pid = entries.first().map_or(0x1FFF, |e| e.1);
    s.push(0xE0 | ((pcr_pid >> 8) as u8 & 0x1F));
    s.push(pcr_pid as u8);
    s.extend_from_slice(&[0xF0, 0x00]); // program_info_length=0
    for &(st, pid) in entries {
        s.push(st);
        s.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
        s.push(pid as u8);
        s.extend_from_slice(&[0xF0, 0x00]);
    }
    s.extend_from_slice(&[0x00; 4]); // CRC
    build_ts_packet(pmt_pid, true, &s)
}

// ============================================================
// 构造测试用 TS 流
// ============================================================

/// 带有 H.264 + AAC 的最小 TS 流
fn build_test_ts() -> Vec<u8> {
    let pmt_pid: u16 = 0x100;
    let v_pid: u16 = 0x101;
    let a_pid: u16 = 0x102;

    let mut ts = Vec::new();
    ts.extend_from_slice(&build_pat(pmt_pid));
    ts.extend_from_slice(&build_pmt(pmt_pid, &[(0x1B, v_pid), (0x0F, a_pid)]));

    // 视频关键帧 PTS=90000 (1s)
    let v1 = build_pes(0xE0, Some(90000), &[0xDE, 0xAD, 0xBE, 0xEF]);
    ts.extend_from_slice(&build_ts_packet_with_af(v_pid, true, true, &v1));

    // 音频 PTS=90000
    let a1 = build_pes(0xC0, Some(90000), &[0xCA, 0xFE]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a1));

    // 视频 P-frame PTS=93600
    let v2 = build_pes(0xE0, Some(93600), &[0x11, 0x22, 0x33]);
    ts.extend_from_slice(&build_ts_packet_with_af(v_pid, true, false, &v2));

    // 音频 PTS=93600
    let a2 = build_pes(0xC0, Some(93600), &[0x44, 0x55]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a2));

    // 视频 P-frame PTS=97200 (触发前一个包的 flush)
    let v3 = build_pes(0xE0, Some(97200), &[0x66]);
    ts.extend_from_slice(&build_ts_packet_with_af(v_pid, true, false, &v3));

    // 音频 PTS=97200
    let a3 = build_pes(0xC0, Some(97200), &[0x77]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a3));

    ts
}

// ============================================================
// 测试
// ============================================================

#[test]
fn test_registry_contains_mpegts() {
    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let ids: Vec<_> = registry.list_demuxers().iter().map(|d| d.0).collect();
    assert!(
        ids.contains(&tao_format::FormatId::MpegTs),
        "注册表应包含 mpegts 解封装器"
    );
}

#[test]
fn test_probe_mpegts() {
    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let ts = build_test_ts();
    let result = registry.probe(&ts, Some("input.ts"));
    assert!(result.is_some(), "应该能探测到 MPEG-TS");
    let pr = result.unwrap();
    assert_eq!(pr.format_id, tao_format::FormatId::MpegTs);
}

#[test]
fn test_stream_info() {
    let ts = build_test_ts();
    let backend = MemoryBackend::from_data(ts);
    let mut io = IoContext::new(Box::new(backend));

    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry
        .create_demuxer(tao_format::FormatId::MpegTs)
        .unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
    assert_eq!(streams.len(), 2);

    // 视频流
    assert_eq!(streams[0].media_type, MediaType::Video);
    assert_eq!(streams[0].codec_id, tao_codec::CodecId::H264);
    assert_eq!(streams[0].time_base, Rational::new(1, 90000));

    // 音频流
    assert_eq!(streams[1].media_type, MediaType::Audio);
    assert_eq!(streams[1].codec_id, tao_codec::CodecId::Aac);
}

#[test]
fn test_read_packets_pts_correct() {
    let ts = build_test_ts();
    let backend = MemoryBackend::from_data(ts);
    let mut io = IoContext::new(Box::new(backend));

    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry
        .create_demuxer(tao_format::FormatId::MpegTs)
        .unwrap();
    demuxer.open(&mut io).unwrap();

    let mut packets = Vec::new();
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => packets.push(pkt),
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读取失败: {e}"),
        }
    }

    // 至少应有 4 个包 (前两轮各 2 个, 在 PUSI 时 flush)
    assert!(
        packets.len() >= 4,
        "至少应有 4 个数据包, 实际={}",
        packets.len()
    );

    // 第一个视频包 PTS=90000
    let first_video = packets.iter().find(|p| p.stream_index == 0).unwrap();
    assert_eq!(first_video.pts, 90000, "第一个视频包 PTS");

    // 第一个音频包 PTS=90000
    let first_audio = packets.iter().find(|p| p.stream_index == 1).unwrap();
    assert_eq!(first_audio.pts, 90000, "第一个音频包 PTS");
}

#[test]
fn test_keyframe_flag() {
    let ts = build_test_ts();
    let backend = MemoryBackend::from_data(ts);
    let mut io = IoContext::new(Box::new(backend));

    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry
        .create_demuxer(tao_format::FormatId::MpegTs)
        .unwrap();
    demuxer.open(&mut io).unwrap();

    let mut video_packets = Vec::new();
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                if pkt.stream_index == 0 {
                    video_packets.push(pkt);
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => panic!("读取失败: {e}"),
        }
    }

    assert!(!video_packets.is_empty(), "应该有视频包");
    assert!(video_packets[0].is_keyframe, "第一个视频包应是关键帧");

    // 后续视频包应该不是关键帧
    if video_packets.len() > 1 {
        assert!(!video_packets[1].is_keyframe, "第二个视频包不应是关键帧");
    }
}

#[test]
fn test_audio_only_ts() {
    let pmt_pid: u16 = 0x100;
    let a_pid: u16 = 0x201;

    let mut ts = Vec::new();
    ts.extend_from_slice(&build_pat(pmt_pid));
    ts.extend_from_slice(&build_pmt(pmt_pid, &[(0x03, a_pid)])); // MP3

    // 两个 MP3 音频 PES
    let a1 = build_pes(0xC0, Some(0), &[0xFF; 20]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a1));
    let a2 = build_pes(0xC0, Some(90000), &[0xAA; 20]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a2));
    // 触发 flush
    let a3 = build_pes(0xC0, Some(180000), &[0xBB; 10]);
    ts.extend_from_slice(&build_ts_packet(a_pid, true, &a3));

    let backend = MemoryBackend::from_data(ts);
    let mut io = IoContext::new(Box::new(backend));

    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry
        .create_demuxer(tao_format::FormatId::MpegTs)
        .unwrap();
    demuxer.open(&mut io).unwrap();

    assert_eq!(demuxer.streams().len(), 1);
    assert_eq!(demuxer.streams()[0].codec_id, tao_codec::CodecId::Mp3);
    assert_eq!(demuxer.streams()[0].media_type, MediaType::Audio);

    let mut count = 0;
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(_) => count += 1,
            Err(TaoError::Eof) => break,
            Err(e) => panic!("{e}"),
        }
    }
    assert!(count >= 2, "应至少有 2 个音频包, 实际={count}");
}

#[test]
fn test_time_base_is_90khz() {
    let ts = build_test_ts();
    let backend = MemoryBackend::from_data(ts);
    let mut io = IoContext::new(Box::new(backend));

    let mut registry = tao_format::FormatRegistry::new();
    tao_format::register_all(&mut registry);
    let mut demuxer = registry
        .create_demuxer(tao_format::FormatId::MpegTs)
        .unwrap();
    demuxer.open(&mut io).unwrap();

    for s in demuxer.streams() {
        assert_eq!(
            s.time_base,
            Rational::new(1, 90000),
            "TS 时间基应为 1/90000"
        );
    }

    let pkt = demuxer.read_packet(&mut io).unwrap();
    assert_eq!(pkt.time_base, Rational::new(1, 90000));
}
