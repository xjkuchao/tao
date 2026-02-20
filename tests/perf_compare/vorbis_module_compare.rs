//! Vorbis 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) cargo test --test vorbis_module_compare -- --nocapture --ignored test_vorbis_compare -- data/1.ogg
//! 2) TAO_VORBIS_COMPARE_INPUT=data/1.ogg cargo test --test vorbis_module_compare -- --nocapture --ignored test_vorbis_compare
//! 3) TAO_VORBIS_COMPARE_INPUT=https://samples.ffmpeg.org/A-codecs/vorbis/ogg/vorbis_test.ogg cargo test --test vorbis_module_compare -- --nocapture --ignored test_vorbis_compare

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::frame::{AudioFrame, Frame};
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};
use tao::resample::ResampleContext;
use tracing::info;

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_tmp_path(tag: &str, ext: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.{}", tag, pid, seq, ext)
}

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    make_tmp_path(tag, "raw")
}

fn init_test_tracing() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}

fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

fn parse_vorbis_header_type(packet: &[u8]) -> Option<u8> {
    if packet.len() < 7 || &packet[1..7] != b"vorbis" {
        return None;
    }
    match packet[0] {
        1 if packet.len() >= 30 => Some(1),
        3 | 5 => Some(packet[0]),
        _ => None,
    }
}

fn parse_vorbis_ident_info(packet: &[u8]) -> Option<(u32, u32)> {
    if packet.len() < 16 || packet[0] != 1 || &packet[1..7] != b"vorbis" {
        return None;
    }
    let channels = u32::from(packet[11]);
    let sample_rate = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
    if channels == 0 || sample_rate == 0 {
        return None;
    }
    Some((sample_rate, channels))
}

fn f32le_bytes_to_vec(f32le_data: &[u8]) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    if !f32le_data.len().is_multiple_of(4) {
        return Err("Vorbis F32 数据长度不是 4 的整数倍".into());
    }
    Ok(f32le_data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn rescale_samples(value: i64, src_rate: u32, dst_rate: u32) -> i64 {
    if src_rate == 0 || dst_rate == 0 {
        return value;
    }
    let scaled = i128::from(value) * i128::from(dst_rate);
    let denom = i128::from(src_rate);
    let rounded = if scaled >= 0 {
        (scaled + (denom / 2)) / denom
    } else {
        (scaled - (denom / 2)) / denom
    };
    rounded
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
        .try_into()
        .unwrap_or_else(|_| {
            if rounded.is_negative() {
                i64::MIN
            } else {
                i64::MAX
            }
        })
}

struct OutputTimeline {
    last_end_samples: i64,
    pts_offset_samples: i64,
    discont_threshold_samples: i64,
}

impl OutputTimeline {
    fn new() -> Self {
        Self {
            last_end_samples: 0,
            pts_offset_samples: 0,
            discont_threshold_samples: 4096,
        }
    }
}

fn decode_audio_frame_to_target(
    af: &AudioFrame,
    target_sample_rate: u32,
    target_layout: ChannelLayout,
    resample_remainders: &mut BTreeMap<u32, u64>,
) -> Result<(Vec<f32>, usize, i64), Box<dyn std::error::Error>> {
    if af.data.is_empty() || af.nb_samples == 0 {
        return Ok((Vec::new(), 0, tao::core::timestamp::NOPTS_VALUE));
    }
    if af.sample_format != SampleFormat::F32 {
        return Err(format!("Vorbis 对比仅支持 F32 输出, 当前为 {}", af.sample_format).into());
    }
    if !af.data[0].len().is_multiple_of(4) {
        return Err("Vorbis 音频帧数据长度不是 4 的整数倍".into());
    }

    let start_pts_target = if af.pts == tao::core::timestamp::NOPTS_VALUE {
        tao::core::timestamp::NOPTS_VALUE
    } else {
        rescale_samples(af.pts, af.sample_rate, target_sample_rate)
    };

    if af.sample_rate == target_sample_rate && af.channel_layout == target_layout {
        let mut samples = f32le_bytes_to_vec(&af.data[0])?;
        let expected = af.nb_samples as usize * target_layout.channels as usize;
        if samples.len() > expected {
            samples.truncate(expected);
        }
        let nb_samples = samples.len() / target_layout.channels as usize;
        return Ok((samples, nb_samples, start_pts_target));
    }

    let ctx = ResampleContext::new(
        af.sample_rate,
        SampleFormat::F32,
        af.channel_layout,
        target_sample_rate,
        SampleFormat::F32,
        target_layout,
    );
    let (converted, converted_nb_samples) = ctx.convert(&af.data[0], af.nb_samples)?;
    let desired_nb_samples = {
        let src_rate = u64::from(af.sample_rate);
        let dst_rate = u64::from(target_sample_rate);
        let carry = resample_remainders.entry(af.sample_rate).or_insert(0);
        let scaled = u64::from(af.nb_samples)
            .saturating_mul(dst_rate)
            .saturating_add(*carry);
        let desired = (scaled / src_rate) as usize;
        *carry = scaled % src_rate;
        desired
    };
    let usable_nb_samples = desired_nb_samples.min(converted_nb_samples as usize);
    let expected_bytes = usable_nb_samples * target_layout.channels as usize * 4;
    if converted.len() < expected_bytes {
        return Err(format!(
            "Vorbis 重采样输出长度异常: 输出字节={}, 期望至少={}",
            converted.len(),
            expected_bytes
        )
        .into());
    }
    let samples = f32le_bytes_to_vec(&converted[..expected_bytes])?;
    Ok((samples, usable_nb_samples, start_pts_target))
}

fn write_frame_to_timeline(
    frame_samples: &[f32],
    frame_nb_samples: usize,
    channels: usize,
    start_pts_target: i64,
    timeline: &mut OutputTimeline,
    out: &mut Vec<f32>,
) -> Result<(), Box<dyn std::error::Error>> {
    if frame_nb_samples == 0 || frame_samples.is_empty() {
        return Ok(());
    }
    if frame_samples.len() < frame_nb_samples.saturating_mul(channels) {
        return Err("写入时间线失败: 帧数据长度不足".into());
    }

    let mut start = if start_pts_target == tao::core::timestamp::NOPTS_VALUE {
        timeline.last_end_samples
    } else {
        start_pts_target.saturating_add(timeline.pts_offset_samples)
    };

    if start.saturating_add(timeline.discont_threshold_samples) < timeline.last_end_samples {
        let delta = timeline.last_end_samples.saturating_sub(start);
        timeline.pts_offset_samples = timeline.pts_offset_samples.saturating_add(delta);
        start = start.saturating_add(delta);
    }

    let mut skip_samples = 0usize;
    if start < 0 {
        let drop = ((-start) as usize).min(frame_nb_samples);
        skip_samples = skip_samples.saturating_add(drop);
        start = 0;
    }

    if start > timeline.last_end_samples {
        let pad = (start - timeline.last_end_samples) as usize;
        out.resize(out.len() + pad.saturating_mul(channels), 0.0);
        timeline.last_end_samples = start;
    }

    if start < timeline.last_end_samples {
        let overlap = (timeline.last_end_samples - start) as usize;
        let drop = overlap.min(frame_nb_samples.saturating_sub(skip_samples));
        skip_samples = skip_samples.saturating_add(drop);
    }

    if skip_samples >= frame_nb_samples {
        return Ok(());
    }
    let usable_samples = frame_nb_samples - skip_samples;
    let begin = skip_samples.saturating_mul(channels);
    let end = begin + usable_samples.saturating_mul(channels);
    out.extend_from_slice(&frame_samples[begin..end]);
    timeline.last_end_samples = timeline
        .last_end_samples
        .saturating_add(usable_samples as i64);
    Ok(())
}

fn open_input(path: &str) -> Result<IoContext, Box<dyn std::error::Error>> {
    if is_url(path) {
        #[cfg(feature = "http")]
        {
            return Ok(IoContext::open_url(path)?);
        }
        #[cfg(not(feature = "http"))]
        {
            return Err("当前构建未启用 http 特性, 无法读取 URL".into());
        }
    }
    Ok(IoContext::open_read(path)?)
}

fn find_next_oggs_sync(data: &[u8], from: usize) -> Option<usize> {
    if from >= data.len() {
        return None;
    }
    data[from..]
        .windows(4)
        .position(|w| w == b"OggS")
        .map(|pos| from + pos)
}

fn ogg_crc32(data: &[u8]) -> u32 {
    const OGG_CRC_POLY: u32 = 0x04C11DB7;
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ OGG_CRC_POLY;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn is_valid_ogg_page(page: &[u8]) -> bool {
    if page.len() < 27 || &page[..4] != b"OggS" {
        return false;
    }
    if page[4] != 0 {
        return false;
    }
    let header_type = page[5];
    if header_type & !0x07 != 0 {
        return false;
    }
    let read_crc = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
    let mut crc_page = page.to_vec();
    crc_page[22..26].fill(0);
    ogg_crc32(&crc_page) == read_crc
}

fn build_ogg_page(
    packet: &[u8],
    serial: u32,
    sequence: u32,
    header_type: u8,
    granule: i64,
) -> Option<Vec<u8>> {
    let mut lacing = Vec::<u8>::new();
    let mut remaining = packet.len();
    while remaining >= 255 {
        lacing.push(255);
        remaining -= 255;
    }
    lacing.push(remaining as u8);
    if lacing.len() > 255 {
        return None;
    }

    let mut page = Vec::<u8>::with_capacity(27 + lacing.len() + packet.len());
    page.extend_from_slice(b"OggS");
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&(granule as u64).to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes());
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    page.extend_from_slice(packet);
    let crc = ogg_crc32(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    Some(page)
}

fn parse_legacy_vorbis_avi_headers(extra_data: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if extra_data.len() < 12 {
        return None;
    }
    for offset in 0..=extra_data.len().saturating_sub(12) {
        let h0_len = u32::from_le_bytes([
            extra_data[offset],
            extra_data[offset + 1],
            extra_data[offset + 2],
            extra_data[offset + 3],
        ]) as usize;
        let h1_len = u32::from_le_bytes([
            extra_data[offset + 4],
            extra_data[offset + 5],
            extra_data[offset + 6],
            extra_data[offset + 7],
        ]) as usize;
        let h2_len = u32::from_le_bytes([
            extra_data[offset + 8],
            extra_data[offset + 9],
            extra_data[offset + 10],
            extra_data[offset + 11],
        ]) as usize;
        if h0_len == 0 || h1_len == 0 || h2_len == 0 {
            continue;
        }
        let total = match h0_len
            .checked_add(h1_len)
            .and_then(|v| v.checked_add(h2_len))
        {
            Some(v) => v,
            None => continue,
        };
        if offset + 12 + total > extra_data.len() {
            continue;
        }
        let h0_end = offset + 12 + h0_len;
        let h1_end = h0_end + h1_len;
        let h2_end = h1_end + h2_len;
        let h0 = extra_data[offset + 12..h0_end].to_vec();
        let h1 = extra_data[h0_end..h1_end].to_vec();
        let h2 = extra_data[h1_end..h2_end].to_vec();
        if h0.len() < 7 || h1.len() < 7 || h2.len() < 7 {
            continue;
        }
        if h0[0] != 1 || &h0[1..7] != b"vorbis" {
            continue;
        }
        if h1[0] != 3 || &h1[1..7] != b"vorbis" {
            continue;
        }
        if h2[0] != 5 || &h2[1..7] != b"vorbis" {
            continue;
        }
        return Some((h0, h1, h2));
    }
    None
}

fn append_complete_ogg_pages(
    chunk: &[u8],
    out: &mut Vec<u8>,
    selected_serial: &mut Option<u32>,
    last_page_sequence: &mut Option<u32>,
    saw_bos: &mut bool,
    allow_first_page_bootstrap: bool,
) -> (usize, usize) {
    let mut cursor = 0usize;
    let mut page_count = 0usize;
    let mut consumed = 0usize;
    while let Some(sync_pos) = find_next_oggs_sync(chunk, cursor) {
        if sync_pos + 27 > chunk.len() {
            consumed = sync_pos;
            break;
        }
        let segment_count = usize::from(chunk[sync_pos + 26]);
        let seg_table_start = sync_pos + 27;
        let seg_table_end = seg_table_start + segment_count;
        if seg_table_end > chunk.len() {
            consumed = sync_pos;
            break;
        }
        let body_len = chunk[seg_table_start..seg_table_end]
            .iter()
            .map(|&v| usize::from(v))
            .sum::<usize>();
        let page_len = 27 + segment_count + body_len;
        if sync_pos + page_len > chunk.len() {
            consumed = sync_pos;
            break;
        }
        let page = &chunk[sync_pos..sync_pos + page_len];
        if !is_valid_ogg_page(page) {
            cursor = sync_pos.saturating_add(1);
            consumed = consumed.max(cursor);
            continue;
        }
        let page_serial = u32::from_le_bytes([page[14], page[15], page[16], page[17]]);
        let page_sequence = u32::from_le_bytes([page[18], page[19], page[20], page[21]]);
        let header_type = page[5];
        if (header_type & 0x02) != 0 {
            *saw_bos = true;
        }
        if selected_serial.is_none() {
            if (header_type & 0x02) == 0 && !allow_first_page_bootstrap {
                cursor = sync_pos + page_len;
                continue;
            }
            *selected_serial = Some(page_serial);
        }
        if Some(page_serial) != *selected_serial {
            cursor = sync_pos + page_len;
            continue;
        }
        if let Some(last) = *last_page_sequence
            && page_sequence <= last
        {
            cursor = sync_pos + page_len;
            consumed = consumed.max(cursor);
            continue;
        }
        *last_page_sequence = Some(page_sequence);
        out.extend_from_slice(page);
        page_count += 1;
        cursor = sync_pos + page_len;
        consumed = consumed.max(cursor);
    }
    (page_count, consumed)
}

fn try_extract_embedded_ogg_from_avi(
    path: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);

    let mut io = open_input(path)?;
    let mut demuxer = match format_registry.open_input(&mut io, None) {
        Ok(d) => d,
        Err(_) => {
            io.seek(std::io::SeekFrom::Start(0))?;
            format_registry.open_input(&mut io, Some(path))?
        }
    };
    if demuxer.name() != "avi" {
        return Ok(None);
    }

    let audio_stream = match demuxer
        .streams()
        .iter()
        .find(|s| s.media_type == tao::core::MediaType::Audio)
    {
        Some(v) => v.clone(),
        None => return Ok(None),
    };

    let mut extracted = Vec::<u8>::new();
    let mut page_count = 0usize;
    let mut selected_serial = None::<u32>;
    let mut last_page_sequence = None::<u32>;
    let mut saw_bos = false;
    let mut pending = Vec::<u8>::new();
    let legacy_headers = parse_legacy_vorbis_avi_headers(&audio_stream.extra_data);
    let allow_first_page_bootstrap = legacy_headers.is_some();
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                if pkt.stream_index != audio_stream.index {
                    continue;
                }
                pending.extend_from_slice(pkt.data.as_ref());
                let (added, consumed) = append_complete_ogg_pages(
                    &pending,
                    &mut extracted,
                    &mut selected_serial,
                    &mut last_page_sequence,
                    &mut saw_bos,
                    allow_first_page_bootstrap,
                );
                page_count += added;
                if consumed > 0 {
                    pending.drain(0..consumed);
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => return Err(format!("AVI 内嵌 Ogg 提取失败: {}", e).into()),
        }
    }

    if page_count == 0 || extracted.len() < 27 {
        let copied_packets_path = make_tmp_path("vorbis_cmp_avi_packets", "bin");
        let copy_status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-i",
                path,
                "-map",
                "0:a:0",
                "-c",
                "copy",
                "-f",
                "data",
                &copied_packets_path,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(status) = copy_status
            && status.success()
            && let Ok(copied_bytes) = std::fs::read(&copied_packets_path)
        {
            extracted.clear();
            page_count = 0;
            selected_serial = None;
            last_page_sequence = None;
            saw_bos = false;
            let (added, _) = append_complete_ogg_pages(
                &copied_bytes,
                &mut extracted,
                &mut selected_serial,
                &mut last_page_sequence,
                &mut saw_bos,
                allow_first_page_bootstrap,
            );
            page_count += added;
        }
        let _ = std::fs::remove_file(&copied_packets_path);
        if page_count == 0 || extracted.len() < 27 {
            return Ok(None);
        }
    }

    if !saw_bos && let (Some((h0, h1, h2)), Some(serial)) = (legacy_headers, selected_serial) {
        let mut prefixed = Vec::<u8>::new();
        let page0 =
            build_ogg_page(&h0, serial, 0, 0x02, 0).ok_or("构造 Ogg identification 头页失败")?;
        let page1 = build_ogg_page(&h1, serial, 1, 0, 0).ok_or("构造 Ogg comment 头页失败")?;
        let page2 = build_ogg_page(&h2, serial, 2, 0, 0).ok_or("构造 Ogg setup 头页失败")?;
        prefixed.extend_from_slice(&page0);
        prefixed.extend_from_slice(&page1);
        prefixed.extend_from_slice(&page2);
        prefixed.extend_from_slice(&extracted);
        extracted = prefixed;
    }

    let tmp_ogg = make_tmp_path("vorbis_cmp_avi_extract", "ogg");
    std::fs::write(&tmp_ogg, &extracted)?;
    Ok(Some(tmp_ogg))
}

fn decode_vorbis_with_tao(
    path: &str,
) -> Result<(u32, u32, Vec<f32>, Option<u32>), Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = open_input(path)?;
    // 先按内容探测, 失败后回退扩展名辅助探测.
    let mut demuxer = match format_registry.open_input(&mut io, None) {
        Ok(d) => d,
        Err(_) => {
            io.seek(std::io::SeekFrom::Start(0))?;
            format_registry.open_input(&mut io, Some(path))?
        }
    };

    let stream = demuxer
        .streams()
        .iter()
        .find(|s| s.codec_id == CodecId::Vorbis)
        .or_else(|| {
            demuxer
                .streams()
                .iter()
                .find(|s| s.media_type == tao::core::MediaType::Audio)
        })
        .ok_or("未找到可解码音频流")?
        .clone();
    let stream_index_u32 =
        u32::try_from(stream.index).map_err(|_| "流索引超出 u32 范围, 无法用于 ffmpeg 映射")?;
    let codec_id = stream.codec_id;
    if codec_id != CodecId::Vorbis {
        info!(
            "[{}] 非 Vorbis 流({}), 对比测试回退到 FFmpeg 解码基线",
            path, codec_id
        );
        let (sr, ch, pcm) = decode_vorbis_with_ffmpeg(path, Some(stream_index_u32))?;
        return Ok((sr, ch, pcm, Some(stream_index_u32)));
    }

    let (sample_rate, channel_layout) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => (a.sample_rate, a.channel_layout),
        _ => (44100, ChannelLayout::STEREO),
    };

    let base_params = CodecParameters {
        codec_id,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format: SampleFormat::F32,
            frame_size: 0,
        }),
    };

    let mut decoder = codec_registry.create_decoder(codec_id)?;
    decoder.open(&base_params)?;

    let mut out = Vec::<f32>::new();
    let actual_sr = sample_rate;
    let actual_ch = channel_layout.channels;
    let mut seen_audio_payload = false;
    let mut pending_restart_ident = None::<(Vec<u8>, u32, u32)>;
    let mut pending_restart_comment = None::<Vec<u8>>;
    let mut resample_remainders = BTreeMap::<u32, u64>::new();
    let mut output_timeline = OutputTimeline::new();
    let mut audio_packets_since_header = 0usize;
    let same_params_restart_packet_threshold =
        std::env::var("TAO_VORBIS_SAME_RESTART_PACKET_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1000);
    let debug_pts = std::env::var("TAO_VORBIS_DEBUG_PTS")
        .map(|v| v == "1")
        .unwrap_or(false);
    let mut last_frame_end_pts_target = None::<i64>;
    let mut frame_samples_by_rate = BTreeMap::<u32, u64>::new();

    let mut demux_eof = false;
    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index != stream.index {
                        continue;
                    }
                    let packet_data = pkt.data.as_ref();
                    let header_type = parse_vorbis_header_type(packet_data);
                    if debug_pts && seen_audio_payload && header_type.is_some() {
                        info!(
                            "[{}] 中途头包: type={:?}, pkt_pts={}, tb={}/{}",
                            path, header_type, pkt.pts, pkt.time_base.num, pkt.time_base.den
                        );
                    }
                    if seen_audio_payload {
                        if pending_restart_ident.is_none() && header_type == Some(1) {
                            if let Some((restart_sr, restart_ch)) =
                                parse_vorbis_ident_info(packet_data)
                            {
                                // 仅在参数发生变化时重建解码器, 同参数头链按损坏头包忽略.
                                if restart_sr != sample_rate
                                    || restart_ch != channel_layout.channels
                                    || audio_packets_since_header
                                        >= same_params_restart_packet_threshold
                                {
                                    pending_restart_ident =
                                        Some((packet_data.to_vec(), restart_sr, restart_ch));
                                    pending_restart_comment = None;
                                    continue;
                                } else {
                                    let mut header_pkt = Packet::from_data(packet_data.to_vec());
                                    header_pkt.stream_index = pkt.stream_index;
                                    header_pkt.time_base = pkt.time_base;
                                    header_pkt.pos = pkt.pos;
                                    header_pkt.pts = tao::core::timestamp::NOPTS_VALUE;
                                    header_pkt.dts = tao::core::timestamp::NOPTS_VALUE;
                                    decoder.send_packet(&header_pkt).map_err(|e| {
                                        format!("发送同参数 identification 头失败: {}", e)
                                    })?;
                                    continue;
                                }
                            }
                        }
                        if pending_restart_ident.is_some() {
                            if header_type == Some(3) && pending_restart_comment.is_none() {
                                pending_restart_comment = Some(packet_data.to_vec());
                                continue;
                            }
                            if header_type == Some(5) && pending_restart_comment.is_some() {
                                decoder
                                    .send_packet(&Packet::empty())
                                    .map_err(|e| format!("重置前发送 flush 包失败: {}", e))?;
                                loop {
                                    match decoder.receive_frame() {
                                        Ok(Frame::Audio(af)) => {
                                            let (samples, nb_samples, start_pts_target) =
                                                decode_audio_frame_to_target(
                                                    &af,
                                                    sample_rate,
                                                    channel_layout,
                                                    &mut resample_remainders,
                                                )?;
                                            write_frame_to_timeline(
                                                &samples,
                                                nb_samples,
                                                channel_layout.channels as usize,
                                                start_pts_target,
                                                &mut output_timeline,
                                                &mut out,
                                            )?;
                                            last_frame_end_pts_target =
                                                Some(output_timeline.last_end_samples);
                                        }
                                        Ok(_) => {}
                                        Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => {
                                            break;
                                        }
                                        Err(e) => {
                                            return Err(format!("重置前取帧失败: {}", e).into());
                                        }
                                    }
                                }
                                let (ident, restart_sr, restart_ch) = pending_restart_ident
                                    .take()
                                    .ok_or("重置缺少 identification 头")?;
                                let comment = pending_restart_comment
                                    .take()
                                    .ok_or("重置缺少 comment 头")?;

                                decoder = codec_registry.create_decoder(codec_id)?;
                                let mut restart_params = base_params.clone();
                                restart_params.extra_data = ident.clone();
                                if let CodecParamsType::Audio(audio) = &mut restart_params.params {
                                    audio.sample_rate = restart_sr;
                                    audio.channel_layout = ChannelLayout::from_channels(restart_ch);
                                }
                                decoder.open(&restart_params)?;

                                let mut comment_pkt = Packet::from_data(comment);
                                comment_pkt.stream_index = stream.index;
                                comment_pkt.time_base = pkt.time_base;
                                decoder
                                    .send_packet(&comment_pkt)
                                    .map_err(|e| format!("发送重置 comment 头失败: {}", e))?;
                                let mut setup_pkt = Packet::from_data(packet_data.to_vec());
                                setup_pkt.stream_index = stream.index;
                                setup_pkt.time_base = pkt.time_base;
                                decoder
                                    .send_packet(&setup_pkt)
                                    .map_err(|e| format!("发送重置 setup 头失败: {}", e))?;

                                seen_audio_payload = false;
                                audio_packets_since_header = 0;
                                continue;
                            }
                            // 中途头链不完整或格式不合法, 放弃本次重置候选并继续常规路径.
                            pending_restart_ident = None;
                            pending_restart_comment = None;
                        } else if matches!(header_type, Some(3) | Some(5)) {
                            // 中途孤立 comment/setup 头直接跳过, 避免污染音频路径.
                            continue;
                        }
                    }
                    if header_type.is_none() {
                        seen_audio_payload = true;
                        audio_packets_since_header = audio_packets_since_header.saturating_add(1);
                    }
                    decoder
                        .send_packet(&pkt)
                        .map_err(|e| format!("发送音频包失败: {}", e))?;
                }
                Err(TaoError::Eof) => {
                    decoder
                        .send_packet(&Packet::empty())
                        .map_err(|e| format!("发送 flush 包失败: {}", e))?;
                    demux_eof = true;
                }
                Err(e) => return Err(format!("读取包失败: {}", e).into()),
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    *frame_samples_by_rate.entry(af.sample_rate).or_default() +=
                        u64::from(af.nb_samples);
                    let (samples, nb_samples, start_pts_target) = decode_audio_frame_to_target(
                        &af,
                        sample_rate,
                        channel_layout,
                        &mut resample_remainders,
                    )?;
                    let corrected_start = if start_pts_target == tao::core::timestamp::NOPTS_VALUE {
                        output_timeline.last_end_samples
                    } else {
                        start_pts_target.saturating_add(output_timeline.pts_offset_samples)
                    };
                    if debug_pts
                        && let Some(last_end) = last_frame_end_pts_target
                        && corrected_start != last_end
                    {
                        info!(
                            "[{}] PTS不连续: last_end={}, cur_start={}, raw_start={}, sr={}, ch={}",
                            path,
                            last_end,
                            corrected_start,
                            start_pts_target,
                            af.sample_rate,
                            af.channel_layout.channels
                        );
                    }
                    write_frame_to_timeline(
                        &samples,
                        nb_samples,
                        channel_layout.channels as usize,
                        start_pts_target,
                        &mut output_timeline,
                        &mut out,
                    )?;
                    last_frame_end_pts_target = Some(output_timeline.last_end_samples);
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => {
                    if demux_eof {
                        if debug_pts {
                            info!(
                                "[{}] Tao原始帧采样率分布: {:?}",
                                path, frame_samples_by_rate
                            );
                        }
                        return Ok((actual_sr, actual_ch, out, Some(stream_index_u32)));
                    }
                    break;
                }
                Err(TaoError::Eof) => {
                    if debug_pts {
                        info!(
                            "[{}] Tao原始帧采样率分布: {:?}",
                            path, frame_samples_by_rate
                        );
                    }
                    return Ok((actual_sr, actual_ch, out, Some(stream_index_u32)));
                }
                Err(e) => return Err(format!("取帧失败: {}", e).into()),
            }
        }
    }
}

fn decode_vorbis_with_ffmpeg(
    path: &str,
    preferred_stream: Option<u32>,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_type,sample_rate,channels",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()?;
    let probe_s = String::from_utf8_lossy(&probe.stdout);

    let mut selected_idx = None;
    let mut sr = 44_100u32;
    let mut ch = 2u32;
    if let Some(want_idx) = preferred_stream {
        for line in probe_s.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 4 || parts[1] != "audio" {
                continue;
            }
            let idx = parts[0].parse::<u32>().ok();
            let line_sr = parts[2].parse::<u32>().ok();
            let line_ch = parts[3].parse::<u32>().ok();
            if let (Some(idx), Some(line_sr), Some(line_ch)) = (idx, line_sr, line_ch)
                && idx == want_idx
                && line_sr > 0
                && line_ch > 0
            {
                selected_idx = Some(idx);
                sr = line_sr;
                ch = line_ch;
                break;
            }
        }
    }

    if selected_idx.is_none() {
        for line in probe_s.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 4 || parts[1] != "audio" {
                continue;
            }
            let idx = parts[0].parse::<u32>().ok();
            let line_sr = parts[2].parse::<u32>().ok();
            let line_ch = parts[3].parse::<u32>().ok();
            if let (Some(idx), Some(line_sr), Some(line_ch)) = (idx, line_sr, line_ch)
                && line_sr > 0
                && line_ch > 0
            {
                selected_idx = Some(idx);
                sr = line_sr;
                ch = line_ch;
                break;
            }
        }
    }
    let selected_idx = selected_idx.ok_or("ffprobe 未找到有效音频流")?;

    let map_spec = format!("0:{selected_idx}");
    let tmp = make_ffmpeg_tmp_path("vorbis_cmp");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
            "-map",
            &map_spec,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            &tmp,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err("ffmpeg 解码失败".into());
    }

    let raw = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    let pcm = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok((sr, ch, pcm))
}

struct CompareStats {
    n: usize,
    max_err: f64,
    psnr: f64,
    precision_pct: f64,
}

fn compare_pcm(reference: &[f32], test: &[f32]) -> CompareStats {
    let n = reference.len().min(test.len());
    if n == 0 {
        return CompareStats {
            n: 0,
            max_err: 0.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
        };
    }
    let mut mse = 0.0f64;
    let mut max_err = 0.0f64;
    let mut ref_power = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        let d = t - r;
        let ad = d.abs();
        max_err = max_err.max(ad);
        mse += d * d;
        ref_power += r * r;
    }
    mse /= n as f64;
    ref_power /= n as f64;
    let psnr = if mse > 0.0 {
        20.0 * (1.0 / mse.sqrt()).log10()
    } else {
        f64::INFINITY
    };
    let mut precision_pct = if ref_power > 0.0 {
        (ref_power / (ref_power + mse)) * 100.0
    } else if mse == 0.0 {
        100.0
    } else {
        0.0
    };
    if precision_pct.is_nan() {
        precision_pct = 0.0;
    }
    if precision_pct < 0.0 {
        precision_pct = 0.0;
    }
    if precision_pct > 100.0 {
        precision_pct = 100.0;
    }

    CompareStats {
        n,
        max_err,
        psnr,
        precision_pct,
    }
}

fn resolve_input() -> Result<String, Box<dyn std::error::Error>> {
    let mut after_dd = std::env::args().skip_while(|v| v != "--").skip(1);
    if let Some(arg) = after_dd.next() {
        return Ok(arg);
    }
    if let Ok(env) = std::env::var("TAO_VORBIS_COMPARE_INPUT")
        && !env.trim().is_empty()
    {
        return Ok(env);
    }
    Err("请通过参数或 TAO_VORBIS_COMPARE_INPUT 指定 OGG 文件或 URL".into())
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let mut compare_input = path.to_string();
    let mut temp_ogg = None::<String>;
    if let Some(extracted) = try_extract_embedded_ogg_from_avi(path)? {
        info!(
            "[{}] 检测到 AVI 内嵌 Ogg 音频, 对比输入切换为 {}",
            path, extracted
        );
        compare_input = extracted.clone();
        temp_ogg = Some(extracted);
    }

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let (tao_sr, tao_ch, tao_pcm, tao_stream_index) = decode_vorbis_with_tao(&compare_input)?;
        let (ff_sr, ff_ch, ff_pcm) = decode_vorbis_with_ffmpeg(&compare_input, tao_stream_index)?;

        assert_eq!(tao_sr, ff_sr, "采样率不匹配");
        assert_eq!(tao_ch, ff_ch, "通道数不匹配");
        if tao_pcm.len() != ff_pcm.len() {
            let n = tao_pcm.len().min(ff_pcm.len());
            if n > 0 {
                let min_stats = compare_pcm(&ff_pcm[..n], &tao_pcm[..n]);
                info!(
                    "[{}] 截断重叠区精度: n={}, max_err={:.9}, psnr={:.2}dB, 精度={:.6}%",
                    path, n, min_stats.max_err, min_stats.psnr, min_stats.precision_pct
                );
            }
            let (longer_tag, longer, shorter_len) = if tao_pcm.len() > ff_pcm.len() {
                ("tao", &tao_pcm, ff_pcm.len())
            } else {
                ("ffmpeg", &ff_pcm, tao_pcm.len())
            };
            let tail = &longer[shorter_len..];
            let mut tail_max = 0.0f64;
            let mut tail_power = 0.0f64;
            for &v in tail {
                let x = f64::from(v.abs());
                if x > tail_max {
                    tail_max = x;
                }
                tail_power += x * x;
            }
            let tail_rms = if tail.is_empty() {
                0.0
            } else {
                (tail_power / tail.len() as f64).sqrt()
            };
            info!(
                "[{}] 长度差尾段统计: longer={}, diff={}, tail_max={:.9}, tail_rms={:.9}",
                path,
                longer_tag,
                tao_pcm.len() as i64 - ff_pcm.len() as i64,
                tail_max,
                tail_rms
            );
        }
        assert_eq!(
            tao_pcm.len(),
            ff_pcm.len(),
            "样本总数不匹配: Tao={}, FFmpeg={}",
            tao_pcm.len(),
            ff_pcm.len()
        );

        let stats_tao = compare_pcm(&ff_pcm, &tao_pcm);
        info!(
            "[{}] Tao对比样本={}, Tao={}, FFmpeg={}, Tao/FFmpeg: max_err={:.9}, psnr={:.2}dB, 精度={:.6}%, FFmpeg=100%",
            path,
            stats_tao.n,
            tao_pcm.len(),
            ff_pcm.len(),
            stats_tao.max_err,
            stats_tao.psnr,
            stats_tao.precision_pct
        );

        assert!(stats_tao.n > 0, "无可比较样本");
        assert!(
            stats_tao.max_err <= 0.00001,
            "Vorbis 对比最大误差超阈值: max_err={}",
            stats_tao.max_err
        );
        assert!(
            stats_tao.precision_pct >= 99.999,
            "Vorbis 对比精度不足 100% 目标: {:.6}%",
            stats_tao.precision_pct
        );
        Ok(())
    })();

    if let Some(tmp) = temp_ogg {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

#[test]
#[ignore]
fn test_vorbis_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("Vorbis 对比失败");
}
