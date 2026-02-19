//! MP3 解码精度对比测试.
//!
//! 当前阶段输出 Tao 与 FFmpeg 的误差统计, 用于持续收敛.
//! 手动执行: cargo test --test mp3_module_compare -- --nocapture --ignored

use std::fmt::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::vlc::{BitOrder, Codebook, CodebookBuilder, Entry16x16};
use symphonia::core::io::{BitReaderLtr, MediaSourceStream, ReadBitsLtr};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};
use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::decoders::mp3::debug as mp3_debug;
use tao::codec::frame::Frame;
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};
use tracing::info;

#[allow(dead_code)]
#[path = "../crates/tao-codec/src/decoders/mp3/bitreader.rs"]
mod bitreader;
#[allow(dead_code)]
#[path = "../crates/tao-codec/src/decoders/mp3/huffman_explicit_tables.rs"]
mod huffman_explicit_tables;

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.raw", tag, pid, seq)
}

fn init_test_tracing() {
    let _ = env_logger::builder().is_test(true).try_init();
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}

fn decode_mp3_with_tao(path: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path)?;
    let mut demuxer = format_registry.open_input(&mut io, Some(path))?;
    demuxer.open(&mut io)?;

    let stream = demuxer
        .streams()
        .iter()
        .find(|s| s.codec_id == CodecId::Mp3)
        .ok_or("未找到 MP3 音频流")?
        .clone();

    let (sample_rate, channel_layout) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => (a.sample_rate, a.channel_layout),
        _ => (44100, ChannelLayout::STEREO),
    };

    let params = CodecParameters {
        codec_id: CodecId::Mp3,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format: SampleFormat::F32,
            frame_size: 1152,
        }),
    };

    let mut decoder = codec_registry.create_decoder(CodecId::Mp3)?;
    decoder.open(&params)?;

    let mut out = Vec::<f32>::new();
    let mut actual_sr = sample_rate;
    let mut actual_ch = channel_layout.channels;

    let mut demux_eof = false;
    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index != stream.index {
                        continue;
                    }
                    decoder.send_packet(&pkt)?;
                }
                Err(TaoError::Eof) => {
                    decoder.send_packet(&Packet::empty())?;
                    demux_eof = true;
                }
                Err(e) => return Err(format!("读取包失败: {}", e).into()),
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    actual_sr = af.sample_rate;
                    actual_ch = af.channel_layout.channels;
                    if !af.data.is_empty() {
                        out.extend(
                            af.data[0]
                                .chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        );
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => {
                    if demux_eof {
                        return Ok((actual_sr, actual_ch, out));
                    }
                    break;
                }
                Err(TaoError::Eof) => return Ok((actual_sr, actual_ch, out)),
                Err(e) => return Err(format!("取帧失败: {}", e).into()),
            }
        }
    }
}

fn read_mp3_gapless_info(path: &str) -> Result<(u32, u32, u64), Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut io = IoContext::open_read(path)?;
    let mut demuxer = format_registry.open_input(&mut io, Some(path))?;
    demuxer.open(&mut io)?;

    let stream = demuxer
        .streams()
        .iter()
        .find(|s| s.codec_id == CodecId::Mp3)
        .ok_or("未找到 MP3 音频流")?;

    let extra = &stream.extra_data;
    if extra.len() >= 16 {
        let delay = u32::from_le_bytes(extra[0..4].try_into().unwrap_or([0; 4]));
        let padding = u32::from_le_bytes(extra[4..8].try_into().unwrap_or([0; 4]));
        let valid_total = u64::from_le_bytes(extra[8..16].try_into().unwrap_or([0; 8]));
        Ok((delay, padding, valid_total))
    } else {
        Ok((0, 0, 0))
    }
}

fn build_gapless_extra_data(delay: u32, padding: u32, valid_total: u64) -> Vec<u8> {
    if delay == 0 && padding == 0 && valid_total == 0 {
        return Vec::new();
    }

    let mut buf = [0u8; 16];
    buf[0..4].copy_from_slice(&delay.to_le_bytes());
    buf[4..8].copy_from_slice(&padding.to_le_bytes());
    buf[8..16].copy_from_slice(&valid_total.to_le_bytes());
    buf.to_vec()
}

fn decode_mp3_with_ffmpeg(path: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let tmp = make_ffmpeg_tmp_path("mp3_cmp");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
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

    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,channels",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()?;
    let probe_s = String::from_utf8_lossy(&probe.stdout);
    let parts: Vec<&str> = probe_s.trim().split(',').collect();
    let sr = parts
        .first()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(44100);
    let ch = parts
        .get(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2);

    let raw = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    let pcm = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok((sr, ch, pcm))
}

fn trim_gapless(mut pcm: Vec<f32>, channels: u32, delay: u32, valid_total: u64) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let per_ch_len = pcm.len() / ch;
    let skip = delay.min(per_ch_len as u32) as usize;
    let usable = per_ch_len.saturating_sub(skip);
    let keep = if valid_total > 0 {
        usable.min(valid_total as usize)
    } else {
        usable
    };

    let start = skip * ch;
    let end = start + keep * ch;
    if start >= pcm.len() {
        return Vec::new();
    }
    let end = end.min(pcm.len());
    pcm.copy_within(start..end, 0);
    pcm.truncate(end - start);
    pcm
}

const HUFFMAN_LINBITS: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 6, 8, 10, 13, 4, 5, 6, 7, 8, 9, 11,
    13,
];

const COUNT1A_BITS: [u8; 16] = [1, 4, 4, 5, 4, 6, 5, 6, 4, 5, 5, 6, 5, 6, 6, 6];
const COUNT1A_CODES: [u32; 16] = [1, 5, 4, 5, 6, 5, 4, 4, 7, 3, 6, 0, 7, 2, 3, 1];
const COUNT1B_BITS: [u8; 16] = [4; 16];
const COUNT1B_CODES: [u32; 16] = [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];

struct RefCodebooks {
    big: Vec<Option<Codebook<Entry16x16>>>,
    count1_a: Codebook<Entry16x16>,
    count1_b: Codebook<Entry16x16>,
}

fn build_codebook(codes: &'static [u32], lens: &'static [u8], wrap: usize) -> Codebook<Entry16x16> {
    let values: Vec<u16> = (0..codes.len())
        .map(|i| (((i / wrap) << 4) | (i % wrap)) as u16)
        .collect();
    let mut builder = CodebookBuilder::new(BitOrder::Verbatim);
    builder.bits_per_read(8);
    builder
        .make(codes, lens, &values)
        .expect("构建 Huffman 参考码表失败")
}

fn explicit_codebook(table_id: u8) -> Option<(&'static [u32], &'static [u8], usize)> {
    use huffman_explicit_tables as t;
    match table_id {
        1 => Some((&t::MPEG_CODES_1, &t::MPEG_BITS_1, 2)),
        2 => Some((&t::MPEG_CODES_2, &t::MPEG_BITS_2, 3)),
        3 => Some((&t::MPEG_CODES_3, &t::MPEG_BITS_3, 3)),
        5 => Some((&t::MPEG_CODES_5, &t::MPEG_BITS_5, 4)),
        6 => Some((&t::MPEG_CODES_6, &t::MPEG_BITS_6, 4)),
        7 => Some((&t::MPEG_CODES_7, &t::MPEG_BITS_7, 6)),
        8 => Some((&t::MPEG_CODES_8, &t::MPEG_BITS_8, 6)),
        9 => Some((&t::MPEG_CODES_9, &t::MPEG_BITS_9, 6)),
        10 => Some((&t::MPEG_CODES_10, &t::MPEG_BITS_10, 8)),
        11 => Some((&t::MPEG_CODES_11, &t::MPEG_BITS_11, 8)),
        12 => Some((&t::MPEG_CODES_12, &t::MPEG_BITS_12, 8)),
        13 => Some((&t::MPEG_CODES_13, &t::MPEG_BITS_13, 16)),
        15 => Some((&t::MPEG_CODES_15, &t::MPEG_BITS_15, 16)),
        16..=23 => Some((&t::MPEG_CODES_16, &t::MPEG_BITS_16, 16)),
        24..=31 => Some((&t::MPEG_CODES_24, &t::MPEG_BITS_24, 16)),
        _ => None,
    }
}

fn build_ref_codebooks() -> RefCodebooks {
    let mut big: Vec<Option<Codebook<Entry16x16>>> = Vec::with_capacity(32);
    for _ in 0..32 {
        big.push(None);
    }
    for table_id in 1..=31u8 {
        if let Some((codes, lens, wrap)) = explicit_codebook(table_id) {
            big[table_id as usize] = Some(build_codebook(codes, lens, wrap));
        }
    }
    let count1_a = build_codebook(&COUNT1A_CODES, &COUNT1A_BITS, 16);
    let count1_b = build_codebook(&COUNT1B_CODES, &COUNT1B_BITS, 16);
    RefCodebooks {
        big,
        count1_a,
        count1_b,
    }
}

fn get_ref_codebooks() -> &'static RefCodebooks {
    static STORE: std::sync::OnceLock<RefCodebooks> = std::sync::OnceLock::new();
    STORE.get_or_init(build_ref_codebooks)
}

fn map_big_codebook(table_id: u8) -> Option<u8> {
    match table_id {
        0 | 4 | 14 => None,
        1..=15 => Some(table_id),
        16..=23 => Some(16),
        24..=31 => Some(24),
        _ => None,
    }
}

fn decode_huffman_reference(snap: &mp3_debug::FrameSnapshot) -> Result<[i32; 576], String> {
    let mut out = [0i32; 576];
    let codebooks = get_ref_codebooks();
    if snap.main_data.is_empty() {
        return Ok(out);
    }

    let mut br = BitReaderLtr::new(&snap.main_data);
    let part2_3_begin = snap.part2_3_begin as u32;
    let part2_bits = snap.part2_bits;
    let part3_bits = snap.part2_3_length.saturating_sub(part2_bits);

    br.ignore_bits(part2_3_begin + part2_bits)
        .map_err(|e| format!("跳过 part2 失败: {}", e))?;

    let mut bits_read = 0u32;
    let mut i = 0usize;
    let big_values_len = snap.big_values.min(576);
    let regions = [
        snap.region1_start.min(big_values_len),
        snap.region2_start.min(big_values_len),
        big_values_len.min(576),
    ];

    for (region_idx, region_end) in regions.iter().enumerate() {
        let table_id = snap.table_select[region_idx];
        let Some(book_id) = map_big_codebook(table_id) else {
            while i < *region_end {
                out[i] = 0;
                i += 1;
                if i < *region_end {
                    out[i] = 0;
                    i += 1;
                }
            }
            continue;
        };
        let linbits = HUFFMAN_LINBITS[table_id as usize] as u32;
        let codebook = codebooks
            .big
            .get(book_id as usize)
            .and_then(|v| v.as_ref())
            .ok_or_else(|| "缺少 Huffman 码表".to_string())?;

        while i < *region_end && bits_read < part3_bits {
            let (value, code_len) = br
                .read_codebook(codebook)
                .map_err(|e| format!("Huffman 解码失败: {}", e))?;
            bits_read += code_len;

            let mut x = (value >> 4) as i32;
            let mut y = (value & 0x0f) as i32;

            if x > 0 {
                if x == 15 && linbits > 0 {
                    x += br
                        .read_bits_leq32(linbits)
                        .map_err(|e| format!("linbits 读取失败: {}", e))?
                        as i32;
                    bits_read += linbits;
                }
                let sign = br.read_bit().map_err(|e| e.to_string())?;
                bits_read += 1;
                if sign != 0 {
                    x = -x;
                }
            }
            if y > 0 {
                if y == 15 && linbits > 0 {
                    y += br
                        .read_bits_leq32(linbits)
                        .map_err(|e| format!("linbits 读取失败: {}", e))?
                        as i32;
                    bits_read += linbits;
                }
                let sign = br.read_bit().map_err(|e| e.to_string())?;
                bits_read += 1;
                if sign != 0 {
                    y = -y;
                }
            }

            if i < 576 {
                out[i] = x;
                i += 1;
            }
            if i < 576 {
                out[i] = y;
                i += 1;
            }
        }
    }

    let count1_codebook = if snap.count1_table == 33 {
        &codebooks.count1_b
    } else {
        &codebooks.count1_a
    };

    while i <= 572 && bits_read < part3_bits {
        let (value, code_len) = br
            .read_codebook(count1_codebook)
            .map_err(|e| format!("Count1 解码失败: {}", e))?;
        bits_read += code_len;
        let num_ones = (value & 0x0f).count_ones();
        let mut signs = if num_ones > 0 {
            br.read_bits_leq32(num_ones)
                .map_err(|e| format!("Count1 符号读取失败: {}", e))?
        } else {
            0
        };
        bits_read += num_ones;

        let mut v = 0;
        let mut w = 0;
        let mut x = 0;
        let mut y = 0;

        if value & 0x1 != 0 {
            y = if (signs & 1) != 0 { -1 } else { 1 };
            signs >>= 1;
        }
        if value & 0x2 != 0 {
            x = if (signs & 1) != 0 { -1 } else { 1 };
            signs >>= 1;
        }
        if value & 0x4 != 0 {
            w = if (signs & 1) != 0 { -1 } else { 1 };
            signs >>= 1;
        }
        if value & 0x8 != 0 {
            v = if (signs & 1) != 0 { -1 } else { 1 };
        }

        out[i] = v;
        out[i + 1] = w;
        out[i + 2] = x;
        out[i + 3] = y;
        i += 4;
    }

    if bits_read < part3_bits {
        br.ignore_bits(part3_bits - bits_read)
            .map_err(|e| format!("跳过填充位失败: {}", e))?;
    }

    Ok(out)
}

fn compare_huffman_reference(path: &str, snapshots: &[mp3_debug::FrameSnapshot]) {
    if std::env::var("TAO_MP3_DEBUG_HUFFMAN_REF").is_err() {
        return;
    }
    if snapshots.is_empty() {
        info!("[{}] Huffman参考: 未收集到快照", path);
        return;
    }

    let mut total = 0usize;
    let mut mismatch = 0usize;
    let mut first = None;

    for snap in snapshots {
        let ref_is = match decode_huffman_reference(snap) {
            Ok(v) => v,
            Err(err) => {
                info!(
                    "[{}] Huffman参考: frame={}, gr={}, ch={}, 参考解码失败: {}",
                    path, snap.frame_index, snap.gr, snap.ch, err
                );
                continue;
            }
        };

        for i in 0..576 {
            total += 1;
            if ref_is[i] != snap.is_samples[i] {
                mismatch += 1;
                if first.is_none() {
                    first = Some((
                        snap.frame_index,
                        snap.gr,
                        snap.ch,
                        i,
                        snap.is_samples[i],
                        ref_is[i],
                    ));
                }
            }
        }
    }

    if let Some((frame, gr, ch, idx, actual, reference)) = first {
        let info = snapshots
            .iter()
            .find(|s| s.frame_index == frame && s.gr == gr && s.ch == ch);
        let (t0, t1, t2) = if let Some(s) = info {
            (s.table_select[0], s.table_select[1], s.table_select[2])
        } else {
            (0, 0, 0)
        };
        let pair_base = idx.saturating_sub(idx % 2);
        let (tao0, tao1, ref0, ref1) = if let Some(s) = info {
            let r = decode_huffman_reference(s).unwrap_or([0i32; 576]);
            (
                s.is_samples[pair_base],
                s.is_samples[(pair_base + 1).min(575)],
                r[pair_base],
                r[(pair_base + 1).min(575)],
            )
        } else {
            (actual, 0, reference, 0)
        };
        info!(
            "[{}] Huffman参考: 样本={}, 不一致={}, 首个不一致=frame={}, gr={}, ch={}, idx={}, tao={}, ref={}, pair=({},{})->({},{}) table_select=[{},{},{}]",
            path,
            total,
            mismatch,
            frame,
            gr,
            ch,
            idx,
            actual,
            reference,
            tao0,
            tao1,
            ref0,
            ref1,
            t0,
            t1,
            t2
        );

        if let Some(snap) = info {
            if let Some(detail) = trace_huffman_mismatch_detail(snap, idx) {
                info!("[{}] Huffman参考: {}", path, detail);
            }
        }
    } else {
        info!("[{}] Huffman参考: 样本={}, 不一致=0", path, total);
    }
}

fn trace_huffman_mismatch_detail(
    snap: &mp3_debug::FrameSnapshot,
    target_idx: usize,
) -> Option<String> {
    if snap.main_data.is_empty() {
        return None;
    }

    let mut br_ref = BitReaderLtr::new(&snap.main_data);
    let mut br_tao = bitreader::BitReader::new(&snap.main_data);
    let part2_3_begin = snap.part2_3_begin as u32;
    let part2_bits = snap.part2_bits;
    let part3_bits = snap.part2_3_length.saturating_sub(part2_bits);
    let total_skip = part2_3_begin + part2_bits;

    br_ref
        .ignore_bits(total_skip)
        .map_err(|e| format!("跳过 part2 失败: {}", e))
        .ok()?;
    if !br_tao.skip_bits(total_skip as usize) {
        return Some("Tao位流跳过 part2 失败".to_string());
    }

    let codebooks = get_ref_codebooks();
    let mut bits_ref = 0u32;
    let mut i = 0usize;
    let big_values_len = snap.big_values.min(576);
    let regions = [
        snap.region1_start.min(big_values_len),
        snap.region2_start.min(big_values_len),
        big_values_len.min(576),
    ];

    for (region_idx, region_end) in regions.iter().enumerate() {
        let table_id = snap.table_select[region_idx];
        let linbits = HUFFMAN_LINBITS[table_id as usize] as u32;

        let codebook = match map_big_codebook(table_id) {
            Some(book_id) => codebooks.big.get(book_id as usize).and_then(|v| v.as_ref()),
            None => None,
        };

        while i < *region_end && bits_ref < part3_bits {
            let tao_before = br_tao.bit_offset();
            let tao_result = decode_big_values_tao(&mut br_tao, table_id, linbits as u8);
            let tao_after = br_tao.bit_offset();

            let (ref_x, ref_y, ref_code_len, ref_linbits, ref_signs) = if let Some(book) = codebook
            {
                let (value, code_len) = br_ref.read_codebook(book).ok()?;
                bits_ref += code_len;
                let mut x = (value >> 4) as i32;
                let mut y = (value & 0x0f) as i32;
                let mut lin_used = 0u32;
                let mut sign_used = 0u32;

                if x > 0 {
                    if x == 15 && linbits > 0 {
                        let extra = br_ref.read_bits_leq32(linbits).ok()? as i32;
                        x += extra;
                        bits_ref += linbits;
                        lin_used += linbits;
                    }
                    let sign = br_ref.read_bit().ok()? as i32;
                    bits_ref += 1;
                    sign_used += 1;
                    if sign != 0 {
                        x = -x;
                    }
                }
                if y > 0 {
                    if y == 15 && linbits > 0 {
                        let extra = br_ref.read_bits_leq32(linbits).ok()? as i32;
                        y += extra;
                        bits_ref += linbits;
                        lin_used += linbits;
                    }
                    let sign = br_ref.read_bit().ok()? as i32;
                    bits_ref += 1;
                    sign_used += 1;
                    if sign != 0 {
                        y = -y;
                    }
                }
                (x, y, code_len, lin_used, sign_used)
            } else {
                (0, 0, 0, 0, 0)
            };

            let (tao_x, tao_y, tao_code_len, tao_linbits, tao_signs) =
                tao_result.unwrap_or((0, 0, 0, 0, 0));

            if ref_x != tao_x || ref_y != tao_y || i == target_idx || i + 1 == target_idx {
                return Some(format!(
                    "详细: idx={}, table={}, ref=({},{}), tao=({},{}) | ref_bits={} (code={}, linbits={}, sign={}), tao_bits={}..{} (code={}, linbits={}, sign={})",
                    i,
                    table_id,
                    ref_x,
                    ref_y,
                    tao_x,
                    tao_y,
                    bits_ref,
                    ref_code_len,
                    ref_linbits,
                    ref_signs,
                    tao_before,
                    tao_after,
                    tao_code_len,
                    tao_linbits,
                    tao_signs
                ));
            }

            i += 2;
        }
    }

    None
}

fn decode_big_value_tao_vlc(br: &mut bitreader::BitReader, table_id: u8) -> Option<(u8, u8)> {
    if table_id == 0 || table_id == 4 || table_id == 14 {
        return Some((0, 0));
    }

    let (codes, lens, wrap) = explicit_codebook(table_id)?;
    let mut max_len = 0u8;
    for &len in lens {
        if len > max_len {
            max_len = len;
        }
    }

    for len in 1..=max_len {
        let bits = br.peek_bits(len)?;
        for i in 0..codes.len() {
            if lens[i] == len && codes[i] == bits {
                br.skip_bits(len as usize);
                let symbol = (((i / wrap) as u8) << 4) | (i % wrap) as u8;
                return Some((symbol, len));
            }
        }
    }

    None
}

fn decode_big_values_tao(
    br: &mut bitreader::BitReader,
    table_id: u8,
    linbits: u8,
) -> Option<(i32, i32, u8, u32, u32)> {
    let (symbol, code_len) = decode_big_value_tao_vlc(br, table_id)?;
    let mut x = (symbol >> 4) as i32;
    let mut y = (symbol & 0x0f) as i32;
    let mut lin_used = 0u32;
    let mut sign_used = 0u32;

    if table_id > 15 && x == 15 && linbits > 0 {
        x += br.read_bits(linbits).unwrap_or(0) as i32;
        lin_used += linbits as u32;
    }
    if x > 0 {
        if br.read_bool()? {
            x = -x;
        }
        sign_used += 1;
    }
    if table_id > 15 && y == 15 && linbits > 0 {
        y += br.read_bits(linbits).unwrap_or(0) as i32;
        lin_used += linbits as u32;
    }
    if y > 0 {
        if br.read_bool()? {
            y = -y;
        }
        sign_used += 1;
    }

    Some((x, y, code_len, lin_used, sign_used))
}

fn decode_mp3_with_symphonia(
    path: &str,
    gapless: (u32, u32, u64),
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format.default_track().ok_or("未找到音频轨道")?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let mut decoder = get_codecs().make(&codec_params, &DecoderOptions::default())?;

    let mut out = Vec::<f32>::new();
    let mut sample_rate = codec_params.sample_rate.unwrap_or(0);
    let mut channels = codec_params.channels.map(|c| c.count() as u32).unwrap_or(0);
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err("symphonia 解码要求重置".into());
            }
            Err(e) => return Err(format!("symphonia 读取包失败: {}", e).into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::DecodeError(_)) => {
                continue;
            }
            Err(e) => return Err(format!("symphonia 解码失败: {}", e).into()),
        };

        let spec = *decoded.spec();
        if sample_rate == 0 {
            sample_rate = spec.rate;
        }
        if channels == 0 {
            channels = spec.channels.count() as u32;
        }

        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        out.extend_from_slice(buf.samples());
    }

    if sample_rate == 0 {
        sample_rate = 44100;
    }
    if channels == 0 {
        channels = 2;
    }

    let (delay, _padding, valid_total) = gapless;
    let out = trim_gapless(out, channels, delay, valid_total);
    Ok((sample_rate, channels, out))
}

fn decode_mp3_with_tao_from_symphonia(
    path: &str,
    extra_data: Vec<u8>,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format.default_track().ok_or("未找到音频轨道")?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.map(|c| c.count() as u32).unwrap_or(2);

    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let params = CodecParameters {
        codec_id: CodecId::Mp3,
        extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::F32,
            frame_size: 1152,
        }),
    };

    let mut decoder = codec_registry.create_decoder(CodecId::Mp3)?;
    decoder.open(&params)?;

    let mut out = Vec::<f32>::new();
    let mut actual_sr = sample_rate;
    let mut actual_ch = channels;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err("symphonia 解复用要求重置".into());
            }
            Err(e) => return Err(format!("symphonia 解复用失败: {}", e).into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        decoder.send_packet(&Packet::from_data(packet.data.to_vec()))?;

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    actual_sr = af.sample_rate;
                    actual_ch = af.channel_layout.channels;
                    if !af.data.is_empty() {
                        out.extend(
                            af.data[0]
                                .chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        );
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => break,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(format!("Tao解码失败: {}", e).into()),
            }
        }
    }

    decoder.send_packet(&Packet::empty())?;
    loop {
        match decoder.receive_frame() {
            Ok(Frame::Audio(af)) => {
                actual_sr = af.sample_rate;
                actual_ch = af.channel_layout.channels;
                if !af.data.is_empty() {
                    out.extend(
                        af.data[0]
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                }
            }
            Ok(_) => {}
            Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
            Err(e) => return Err(format!("Tao解码失败: {}", e).into()),
        }
    }

    Ok((actual_sr, actual_ch, out))
}

struct CompareStats {
    n: usize,
    max_err: f64,
    psnr: f64,
    precision_pct: f64,
}

struct AlignDiag {
    offset: i32,
    gain: f64,
    psnr: f64,
    precision_pct: f64,
    samples: usize,
    stride: usize,
}

#[derive(Clone, Copy)]
struct EnvChange {
    key: &'static str,
    value: Option<&'static str>,
}

fn with_env_changes<T>(
    changes: &[EnvChange],
    f: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut prev = Vec::with_capacity(changes.len());
    for change in changes {
        let old = std::env::var(change.key).ok();
        prev.push((change.key, old));
        match change.value {
            Some(v) => unsafe { std::env::set_var(change.key, v) },
            None => unsafe { std::env::remove_var(change.key) },
        }
    }

    let result = f();

    for (key, old) in prev {
        match old {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    result
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

fn max_abs(samples: &[f32]) -> f32 {
    samples
        .iter()
        .copied()
        .map(|v| v.abs())
        .fold(0.0f32, f32::max)
}

fn deinterleave_channel(pcm: &[f32], channels: u32, ch: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let chs = channels as usize;
    let mut out = Vec::with_capacity(pcm.len() / chs);
    let mut idx = ch;
    while idx < pcm.len() {
        out.push(pcm[idx]);
        idx += chs;
    }
    out
}

fn swap_stereo(pcm: &[f32]) -> Vec<f32> {
    let mut out = pcm.to_vec();
    let mut i = 0usize;
    while i + 1 < out.len() {
        out.swap(i, i + 1);
        i += 2;
    }
    out
}

fn invert_all(pcm: &[f32]) -> Vec<f32> {
    pcm.iter().map(|v| -*v).collect()
}

fn invert_channel(pcm: &[f32], channels: u32, ch: usize) -> Vec<f32> {
    let mut out = pcm.to_vec();
    let chs = channels.max(1) as usize;
    let mut idx = ch;
    while idx < out.len() {
        out[idx] = -out[idx];
        idx += chs;
    }
    out
}

fn ms_to_lr(pcm: &[f32]) -> Vec<f32> {
    let mut out = pcm.to_vec();
    let mut i = 0usize;
    while i + 1 < out.len() {
        let m = out[i];
        let s = out[i + 1];
        out[i] = (m + s) * std::f32::consts::FRAC_1_SQRT_2;
        out[i + 1] = (m - s) * std::f32::consts::FRAC_1_SQRT_2;
        i += 2;
    }
    out
}

fn lr_to_ms(pcm: &[f32]) -> Vec<f32> {
    let mut out = pcm.to_vec();
    let mut i = 0usize;
    while i + 1 < out.len() {
        let l = out[i];
        let r = out[i + 1];
        out[i] = (l + r) * std::f32::consts::FRAC_1_SQRT_2;
        out[i + 1] = (l - r) * std::f32::consts::FRAC_1_SQRT_2;
        i += 2;
    }
    out
}

fn scale_all(pcm: &[f32], gain: f32) -> Vec<f32> {
    pcm.iter().map(|v| v * gain).collect()
}

fn estimate_gain_full(reference: &[f32], test: &[f32]) -> (f64, f64, f64) {
    let n = reference.len().min(test.len());
    if n == 0 {
        return (1.0, f64::INFINITY, 0.0);
    }
    let mut sum_rr = 0.0f64;
    let mut sum_tt = 0.0f64;
    let mut sum_rt = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        sum_rr += r * r;
        sum_tt += t * t;
        sum_rt += r * t;
    }
    let gain = if sum_tt > 0.0 { sum_rt / sum_tt } else { 1.0 };
    let mse = (sum_rr - 2.0 * gain * sum_rt + gain * gain * sum_tt) / n as f64;
    let psnr = if mse > 0.0 {
        20.0 * (1.0 / mse.sqrt()).log10()
    } else {
        f64::INFINITY
    };
    let ref_power = sum_rr / n as f64;
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
    (gain, psnr, precision_pct)
}

fn similarity_stats(reference: &[f32], test: &[f32]) -> (f64, f64) {
    let n = reference.len().min(test.len());
    if n == 0 {
        return (0.0, 0.0);
    }

    let mut sum_rt = 0.0f64;
    let mut sum_rr = 0.0f64;
    let mut sum_tt = 0.0f64;
    let mut sign_mismatch = 0usize;
    let mut sign_total = 0usize;

    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        sum_rt += r * t;
        sum_rr += r * r;
        sum_tt += t * t;

        if r.abs() > 1e-4 {
            sign_total += 1;
            if r.signum() != t.signum() {
                sign_mismatch += 1;
            }
        }
    }

    let corr = if sum_rr > 0.0 && sum_tt > 0.0 {
        sum_rt / (sum_rr.sqrt() * sum_tt.sqrt())
    } else {
        0.0
    };
    let sign_mismatch_pct = if sign_total > 0 {
        (sign_mismatch as f64) * 100.0 / (sign_total as f64)
    } else {
        0.0
    };

    (corr, sign_mismatch_pct)
}

fn ratio_stats(reference: &[f32], test: &[f32]) -> (f64, f64) {
    let n = reference.len().min(test.len());
    if n == 0 {
        return (0.0, 0.0);
    }

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut count = 0usize;

    for i in 0..n {
        let r = reference[i] as f64;
        if r.abs() < 1e-4 {
            continue;
        }
        let t = test[i] as f64;
        let ratio = t / r;
        sum += ratio;
        sum_sq += ratio * ratio;
        count += 1;
    }

    if count == 0 {
        return (0.0, 0.0);
    }

    let mean = sum / count as f64;
    let var = (sum_sq / count as f64) - mean * mean;
    let std = if var > 0.0 { var.sqrt() } else { 0.0 };
    (mean, std)
}

fn summarize_frame_errors(reference: &[f32], test: &[f32], channels: u32) -> String {
    let frame_size = 1152usize * channels as usize;
    if frame_size == 0 {
        return "帧诊断: 无有效帧大小".to_string();
    }
    let total_frames = reference.len().min(test.len()) / frame_size;
    if total_frames == 0 {
        return "帧诊断: 无可比较帧".to_string();
    }

    let mut worst = Vec::<(usize, f64, f64)>::new(); // (frame_idx, mse, max_err)
    let mut high_err = 0usize;
    let mut sum_mse = 0.0f64;

    for frame_idx in 0..total_frames {
        let start = frame_idx * frame_size;
        let end = start + frame_size;
        let mut mse = 0.0f64;
        let mut max_err = 0.0f64;
        for i in start..end {
            let d = (test[i] - reference[i]) as f64;
            let ad = d.abs();
            if ad > max_err {
                max_err = ad;
            }
            mse += d * d;
        }
        mse /= frame_size as f64;
        sum_mse += mse;
        if max_err > 0.1 {
            high_err += 1;
        }
        worst.push((frame_idx, mse, max_err));
    }

    worst.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top = worst.iter().take(5).copied().collect::<Vec<_>>();
    let avg_mse = sum_mse / total_frames as f64;
    let avg_psnr = if avg_mse > 0.0 {
        20.0 * (1.0 / avg_mse.sqrt()).log10()
    } else {
        f64::INFINITY
    };

    let mut lines = Vec::new();
    lines.push(format!(
        "帧诊断: 总帧={}, 高误差帧(>0.1)={}, 平均PSNR={:.2}dB",
        total_frames, high_err, avg_psnr
    ));
    for (idx, mse, max_err) in top {
        let psnr = if mse > 0.0 {
            20.0 * (1.0 / mse.sqrt()).log10()
        } else {
            f64::INFINITY
        };
        lines.push(format!(
            "  最差帧: idx={}, psnr={:.2}dB, max_err={:.4}",
            idx, psnr, max_err
        ));
    }
    lines.join("\n")
}

fn estimate_alignment(reference: &[f32], test: &[f32]) -> AlignDiag {
    let n = reference.len().min(test.len());
    if n == 0 {
        return AlignDiag {
            offset: 0,
            gain: 1.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
            samples: 0,
            stride: 1,
        };
    }

    let sample_count = n.min(200_000);
    let stride = (sample_count / 65_536).max(1);
    let max_offset = 2048i32;

    let mut best_offset = 0i32;
    let mut best_gain = 1.0f64;
    let mut best_mse = f64::INFINITY;
    let mut best_ref_power = 0.0f64;
    let mut best_count = 0usize;

    for offset in -max_offset..=max_offset {
        let mut sum_rr = 0.0f64;
        let mut sum_tt = 0.0f64;
        let mut sum_rt = 0.0f64;
        let mut count = 0usize;

        for idx in (0..sample_count).step_by(stride) {
            let j = idx as i64 + offset as i64;
            if j < 0 || j >= sample_count as i64 {
                continue;
            }
            let r = reference[idx] as f64;
            let t = test[j as usize] as f64;
            sum_rr += r * r;
            sum_tt += t * t;
            sum_rt += r * t;
            count += 1;
        }

        if count == 0 {
            continue;
        }

        let gain = if sum_tt > 0.0 { sum_rt / sum_tt } else { 1.0 };
        let mse = (sum_rr - 2.0 * gain * sum_rt + gain * gain * sum_tt) / count as f64;

        if mse < best_mse {
            best_mse = mse;
            best_offset = offset;
            best_gain = gain;
            best_ref_power = sum_rr / count as f64;
            best_count = count;
        }
    }

    let psnr = if best_mse > 0.0 {
        20.0 * (1.0 / best_mse.sqrt()).log10()
    } else {
        f64::INFINITY
    };
    let mut precision_pct = if best_ref_power > 0.0 {
        (best_ref_power / (best_ref_power + best_mse)) * 100.0
    } else if best_mse == 0.0 {
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

    AlignDiag {
        offset: best_offset,
        gain: best_gain,
        psnr,
        precision_pct,
        samples: best_count,
        stride,
    }
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();
    // SAFETY: 测试中仅用于本进程调试开关, 且在解码前单次设置.
    unsafe {
        std::env::set_var("TAO_MP3_DEBUG_FRAME_INFO", "1");
        std::env::set_var("TAO_MP3_DEBUG_HUFFMAN", "1");
        std::env::set_var("TAO_MP3_DEBUG_PART2", "1");
        std::env::set_var("TAO_MP3_DEBUG_HUFFMAN_ERR", "1");
    }
    let gapless = read_mp3_gapless_info(path)?;
    let gapless_extra = build_gapless_extra_data(gapless.0, gapless.1, gapless.2);
    info!(
        "[{}] Gapless信息: delay={}, padding={}, valid_total={}",
        path, gapless.0, gapless.1, gapless.2
    );
    let (tao_sr, tao_ch, tao_pcm) = decode_mp3_with_tao(path)?;
    let mut tao_sym_pcm = Vec::new();
    if std::env::var("TAO_MP3_DEBUG_TAO_SYM_DEMUX").is_ok() {
        let (_sr, _ch, pcm) = decode_mp3_with_tao_from_symphonia(path, gapless_extra.clone())?;
        tao_sym_pcm = pcm;
    }
    let (ff_sr, ff_ch, ff_pcm) = decode_mp3_with_ffmpeg(path)?;
    let (ref_sr, ref_ch, ref_pcm) = decode_mp3_with_symphonia(path, gapless)?;

    if std::env::var("TAO_MP3_DUMP_PCM").is_ok() {
        let tao_path = make_ffmpeg_tmp_path("tao_pcm");
        let ff_path = make_ffmpeg_tmp_path("ff_pcm");
        let tao_bytes: Vec<u8> = tao_pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        let ff_bytes: Vec<u8> = ff_pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&tao_path, tao_bytes)?;
        std::fs::write(&ff_path, ff_bytes)?;
        info!(
            "[{}] 已输出 PCM: Tao={}, FFmpeg={}",
            path, tao_path, ff_path
        );
    }

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");
    assert_eq!(ref_sr, ff_sr, "symphonia 采样率不匹配");
    assert_eq!(ref_ch, ff_ch, "symphonia 通道数不匹配");

    let stats_tao = compare_pcm(&ff_pcm, &tao_pcm);
    let tao_clamped: Vec<f32> = tao_pcm.iter().map(|v| v.clamp(-1.0, 1.0)).collect();
    let stats_tao_clamped = compare_pcm(&ff_pcm, &tao_clamped);
    let stats_ref = compare_pcm(&ff_pcm, &ref_pcm);
    let stats_tao_ref = compare_pcm(&ref_pcm, &tao_pcm);
    let align = estimate_alignment(&ff_pcm, &tao_pcm);
    let (gain_full, psnr_gain, precision_gain) = estimate_gain_full(&ff_pcm, &tao_pcm);
    let (corr, sign_mismatch_pct) = similarity_stats(&ff_pcm, &tao_pcm);
    let (ratio_mean, ratio_std) = ratio_stats(&ff_pcm, &tao_pcm);
    let max_tao = max_abs(&tao_pcm);
    let max_ff = max_abs(&ff_pcm);
    let max_ref = max_abs(&ref_pcm);
    let frame_summary = summarize_frame_errors(&ff_pcm, &tao_pcm, tao_ch);
    let part2_infos = mp3_debug::take_part2_infos();
    let huff_errors = mp3_debug::take_huffman_errors();
    let snapshots = mp3_debug::take_snapshots();
    compare_huffman_reference(path, &snapshots);

    let mut part2_mismatch = 0u32;
    let mut part2_max_diff = 0i32;
    for info in &part2_infos {
        let expected = if info.windows_switching_flag && info.block_type == 2 {
            if info.mixed_block_flag {
                // 混合块: 8 个长块 + 3 个短块 = 17 个 slen1, 6 个短块 = 18 个 slen2
                info.slen1 as u32 * 17 + info.slen2 as u32 * 18
            } else {
                // 纯短块: 6 个短块 *3 = 18 个 slen1, 6 个短块 *3 = 18 个 slen2
                info.slen1 as u32 * 18 + info.slen2 as u32 * 18
            }
        } else {
            // 长块/Start/End: 4 组 (6,5,5,5)
            let mut bits = 0u32;
            let groups = [6usize, 5, 5, 5];
            for (idx, count) in groups.iter().enumerate() {
                let use_prev = info.gr == 1 && info.scfsi[idx] != 0;
                if use_prev {
                    continue;
                }
                let slen = if idx < 2 { info.slen1 } else { info.slen2 } as u32;
                bits += slen * (*count as u32);
            }
            bits
        };
        let actual = info.part2_bits;
        if expected != actual {
            part2_mismatch += 1;
            let diff = actual as i32 - expected as i32;
            part2_max_diff = part2_max_diff.max(diff.abs());
        }
    }

    let mut ch_stats = Vec::new();
    for ch in 0..tao_ch as usize {
        let ff_ch_pcm = deinterleave_channel(&ff_pcm, tao_ch, ch);
        let tao_ch_pcm = deinterleave_channel(&tao_pcm, tao_ch, ch);
        let stats = compare_pcm(&ff_ch_pcm, &tao_ch_pcm);
        ch_stats.push((ch, stats));
    }
    info!(
        "[{}] Tao对比样本={}, Tao={}, FFmpeg={}, Tao/FFmpeg: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%, FFmpeg=100%",
        path,
        stats_tao.n,
        tao_pcm.len(),
        ff_pcm.len(),
        stats_tao.max_err,
        stats_tao.psnr,
        stats_tao.precision_pct
    );
    info!(
        "[{}] Tao裁剪诊断: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, stats_tao_clamped.max_err, stats_tao_clamped.psnr, stats_tao_clamped.precision_pct
    );
    info!(
        "[{}] symphonia对比样本={}, symphonia={}, FFmpeg={}, symphonia/FFmpeg: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path,
        stats_ref.n,
        ref_pcm.len(),
        ff_pcm.len(),
        stats_ref.max_err,
        stats_ref.psnr,
        stats_ref.precision_pct
    );
    info!(
        "[{}] Tao/symphonia: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, stats_tao_ref.max_err, stats_tao_ref.psnr, stats_tao_ref.precision_pct
    );
    if !tao_sym_pcm.is_empty() {
        let stats_tao_sym = compare_pcm(&ff_pcm, &tao_sym_pcm);
        info!(
            "[{}] Tao(使用symphonia解复用)/FFmpeg: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_tao_sym.max_err, stats_tao_sym.psnr, stats_tao_sym.precision_pct
        );
    }
    info!(
        "[{}] 对齐诊断: offset={}, gain={:.6}, psnr={:.2}dB, 精度={:.2}%, samples={}, stride={}",
        path,
        align.offset,
        align.gain,
        align.psnr,
        align.precision_pct,
        align.samples,
        align.stride
    );
    info!(
        "[{}] 增益诊断: gain_full={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, gain_full, psnr_gain, precision_gain
    );
    info!(
        "[{}] 相似度诊断: 相关系数={:.4}, 符号不一致={:.2}%",
        path, corr, sign_mismatch_pct
    );
    info!(
        "[{}] 比例诊断: mean_ratio={:.4}, std_ratio={:.4}",
        path, ratio_mean, ratio_std
    );
    info!(
        "[{}] 幅度诊断: Tao_max={:.4}, FFmpeg_max={:.4}, symphonia_max={:.4}",
        path, max_tao, max_ff, max_ref
    );
    if !part2_infos.is_empty() {
        let mut overflow = 0usize;
        let mut max_over = 0u32;
        let mut first_over = None;
        for info in &part2_infos {
            if info.part2_bits > info.part2_3_length {
                overflow += 1;
                let over = info.part2_bits - info.part2_3_length;
                if over > max_over {
                    max_over = over;
                    first_over = Some(info.clone());
                }
            }
        }
        info!(
            "[{}] Part2诊断: 记录={}条, 溢出={}条, 最大溢出={}bit",
            path,
            part2_infos.len(),
            overflow,
            max_over
        );
        info!(
            "[{}] Part2一致性: 不匹配={}条, 最大差异={}bit",
            path, part2_mismatch, part2_max_diff
        );
        if let Some(info) = first_over {
            info!(
                "[{}] Part2样例: frame={}, gr={}, ch={}, part2_bits={}, part2_3_length={}, block_type={}, mixed={}, ws={}, sfc={}, slen1={}, slen2={}, scfsi={:?}",
                path,
                info.frame_index,
                info.gr,
                info.ch,
                info.part2_bits,
                info.part2_3_length,
                info.block_type,
                info.mixed_block_flag,
                info.windows_switching_flag,
                info.scalefac_compress,
                info.slen1,
                info.slen2,
                info.scfsi
            );
        }
    }
    if !huff_errors.is_empty() {
        let mut lines = Vec::new();
        for info in huff_errors.iter().take(10) {
            lines.push(format!(
                "f={}, gr={}, ch={}, stage={}, bit_offset={}, end_bit={}",
                info.frame_index, info.gr, info.ch, info.stage, info.bit_offset, info.end_bit
            ));
        }
        info!(
            "[{}] Huffman异常: 总数={}, 样例={}",
            path,
            huff_errors.len(),
            lines.join("; ")
        );
    }
    for line in frame_summary.lines() {
        info!("[{}] {}", path, line);
    }
    for (ch, stats) in ch_stats {
        info!(
            "[{}] 通道{}: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, ch, stats.max_err, stats.psnr, stats.precision_pct
        );
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_DEBUG_REQUANTIZE_REF").is_ok() {
        for snap in &snapshots {
            let xr_ref = mp3_debug::reference_requantize_mpeg1(snap, tao_sr);
            let cmp = mp3_debug::compare_f32_samples(
                "requantize_ref",
                &snap.xr_after_requantize,
                &xr_ref,
            );
            info!(
                "[{}] Requantize对照: frame={}, gr={}, ch={}, {}",
                path, snap.frame_index, snap.gr, snap.ch, cmp
            );
        }
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_DEBUG_MS_REF").is_ok() {
        use std::collections::BTreeMap;
        let mut pairs: BTreeMap<(usize, usize), [Option<&mp3_debug::FrameSnapshot>; 2]> =
            BTreeMap::new();
        for snap in &snapshots {
            if snap.ch < 2 {
                let entry = pairs
                    .entry((snap.frame_index, snap.gr))
                    .or_insert([None, None]);
                entry[snap.ch] = Some(snap);
            }
        }

        for ((frame_idx, gr), pair) in pairs {
            let (Some(left), Some(right)) = (pair[0], pair[1]) else {
                continue;
            };

            let ms_enabled = left.channel_mode == 1 && (left.mode_extension & 0x2) != 0;
            if !ms_enabled {
                continue;
            }

            let scale = if std::env::var("TAO_MP3_MS_SCALE_HALF").is_ok() {
                0.5f32
            } else {
                std::f32::consts::FRAC_1_SQRT_2
            };

            let mut l_ref = left.xr_after_requantize;
            let mut r_ref = right.xr_after_requantize;
            for i in 0..576 {
                let m = l_ref[i];
                let s = r_ref[i];
                l_ref[i] = (m + s) * scale;
                r_ref[i] = (m - s) * scale;
            }

            let cmp_l = mp3_debug::compare_f32_samples("ms_ref_l", &left.xr_after_stereo, &l_ref);
            let cmp_r = mp3_debug::compare_f32_samples("ms_ref_r", &right.xr_after_stereo, &r_ref);
            info!(
                "[{}] MS对照: frame={}, gr={}, L={}, R={}",
                path, frame_idx, gr, cmp_l, cmp_r
            );
        }
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_DEBUG_IMDCT_REF").is_ok() {
        for snap in &snapshots {
            let imdct_ref = mp3_debug::reference_imdct(snap);
            let cmp = mp3_debug::compare_f32_samples("imdct_ref", &snap.imdct_output, &imdct_ref);
            info!(
                "[{}] IMDCT对照: frame={}, gr={}, ch={}, {}",
                path, snap.frame_index, snap.gr, snap.ch, cmp
            );
        }
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_DEBUG_SYNTH_REF").is_ok() {
        for snap in &snapshots {
            let synth_ref = mp3_debug::reference_synthesis_pcm(snap);
            let cmp = mp3_debug::compare_f32_samples("synth_ref", &snap.pcm_output, &synth_ref);
            info!(
                "[{}] 合成滤波器对照: frame={}, gr={}, ch={}, {}",
                path, snap.frame_index, snap.gr, snap.ch, cmp
            );
        }
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_DEBUG_REF_PIPELINE").is_ok() {
        for snap in &snapshots {
            if !snap.ref_pcm_output.is_empty() {
                let cmp_imdct = mp3_debug::compare_f32_samples(
                    "ref_pipeline_imdct",
                    &snap.imdct_output,
                    &snap.ref_imdct_output,
                );
                let cmp_pcm = mp3_debug::compare_f32_samples(
                    "ref_pipeline_pcm",
                    &snap.pcm_output,
                    &snap.ref_pcm_output,
                );
                info!(
                    "[{}] 参考管线对照: frame={}, gr={}, ch={}, IMDCT={}, PCM={}",
                    path, snap.frame_index, snap.gr, snap.ch, cmp_imdct, cmp_pcm
                );
            }
        }
    }

    if !snapshots.is_empty() && std::env::var("TAO_MP3_SNAPSHOT_DUMP").is_ok() {
        for snap in snapshots {
            let dump_path = format!(
                "data/tmp_mp3_snapshot_f{}_g{}_c{}.txt",
                snap.frame_index, snap.gr, snap.ch
            );
            let mut out = String::new();
            let _ = writeln!(
                &mut out,
                "frame={}, gr={}, ch={}, rzero={}, global_gain={}, scalefac_compress={}, scalefac_scale={}, preflag={}, subblock_gain={:?}, table_select={:?}, part2_3_length={}, channel_mode={}, mode_extension={}, block_type={}, mixed={}, ws={}, region1_start={}, region2_start={}, big_values={}, count1_table={}",
                snap.frame_index,
                snap.gr,
                snap.ch,
                snap.rzero,
                snap.global_gain,
                snap.scalefac_compress,
                snap.scalefac_scale,
                snap.preflag,
                snap.subblock_gain,
                snap.table_select,
                snap.part2_3_length,
                snap.channel_mode,
                snap.mode_extension,
                snap.block_type,
                snap.mixed_block_flag,
                snap.windows_switching_flag,
                snap.region1_start,
                snap.region2_start,
                snap.big_values,
                snap.count1_table
            );
            let _ = writeln!(&mut out, "[scalefac]");
            for (idx, v) in snap.scalefac.iter().enumerate() {
                let _ = writeln!(&mut out, "{}:{}", idx, v);
            }
            let _ = writeln!(&mut out, "[is_samples]");
            for v in &snap.is_samples {
                let _ = writeln!(&mut out, "{v}");
            }
            let _ = writeln!(&mut out, "[xr_after_requantize]");
            for v in &snap.xr_after_requantize {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[xr_after_stereo]");
            for v in &snap.xr_after_stereo {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[xr_after_reorder]");
            for v in &snap.xr_after_reorder {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[xr_after_alias]");
            for v in &snap.xr_after_alias {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[imdct_output]");
            for v in &snap.imdct_output {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[after_freq_inversion]");
            for v in &snap.after_freq_inversion {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            let _ = writeln!(&mut out, "[pcm_output]");
            for v in &snap.pcm_output {
                let _ = writeln!(&mut out, "{v:.8}");
            }
            std::fs::write(&dump_path, out)?;
            info!("[{}] 已输出快照: {}", path, dump_path);
        }
    }

    if tao_ch == 2 {
        let swapped = swap_stereo(&tao_pcm);
        let stats_swapped = compare_pcm(&ff_pcm, &swapped);
        info!(
            "[{}] 交换声道诊断: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_swapped.max_err, stats_swapped.psnr, stats_swapped.precision_pct
        );

        let inv_left = invert_channel(&tao_pcm, tao_ch, 0);
        let stats_inv_left = compare_pcm(&ff_pcm, &inv_left);
        info!(
            "[{}] 左声道反相诊断: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_inv_left.max_err, stats_inv_left.psnr, stats_inv_left.precision_pct
        );

        let inv_right = invert_channel(&tao_pcm, tao_ch, 1);
        let stats_inv_right = compare_pcm(&ff_pcm, &inv_right);
        info!(
            "[{}] 右声道反相诊断: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_inv_right.max_err, stats_inv_right.psnr, stats_inv_right.precision_pct
        );
    }

    let inv_all = invert_all(&tao_pcm);
    let stats_inv_all = compare_pcm(&ff_pcm, &inv_all);
    info!(
        "[{}] 全反相诊断: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, stats_inv_all.max_err, stats_inv_all.psnr, stats_inv_all.precision_pct
    );

    if tao_ch == 2 {
        let tao_ms_decode = ms_to_lr(&tao_pcm);
        let stats_ms_decode = compare_pcm(&ff_pcm, &tao_ms_decode);
        info!(
            "[{}] MS解码诊断(将Tao视为M/S): max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_ms_decode.max_err, stats_ms_decode.psnr, stats_ms_decode.precision_pct
        );

        let ff_ms_encode = lr_to_ms(&ff_pcm);
        let stats_ms_encode = compare_pcm(&ff_ms_encode, &tao_pcm);
        info!(
            "[{}] MS编码诊断(将FFmpeg转为M/S): max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, stats_ms_encode.max_err, stats_ms_encode.psnr, stats_ms_encode.precision_pct
        );
    }

    let tao_scale_down = scale_all(&tao_pcm, std::f32::consts::FRAC_1_SQRT_2);
    let stats_scale_down = compare_pcm(&ff_pcm, &tao_scale_down);
    info!(
        "[{}] 缩放诊断(乘1/√2): max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, stats_scale_down.max_err, stats_scale_down.psnr, stats_scale_down.precision_pct
    );
    let tao_scale_up = scale_all(&tao_pcm, std::f32::consts::SQRT_2);
    let stats_scale_up = compare_pcm(&ff_pcm, &tao_scale_up);
    info!(
        "[{}] 缩放诊断(乘√2): max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
        path, stats_scale_up.max_err, stats_scale_up.psnr, stats_scale_up.precision_pct
    );

    let frame_infos = mp3_debug::take_frame_infos();
    if !frame_infos.is_empty() {
        let mut head_lines = Vec::new();
        for info in frame_infos.iter().take(10) {
            head_lines.push(format!(
                "idx={}, main_data_begin={}, underflow_bytes={}",
                info.frame_index, info.main_data_begin, info.underflow_bytes
            ));
        }
        if !head_lines.is_empty() {
            info!("[{}] MainData头部: {}", path, head_lines.join(", "));
        }

        let delay = gapless.0 as i64;
        let channels = tao_ch.max(1) as usize;
        let total_per_ch = ff_pcm.len() / channels;
        let frame_samples = 1152usize;
        let mut worst_frames: Vec<(i64, f64, f64)> = Vec::new();

        let mut short_count = 0usize;
        let mut long_count = 0usize;
        let mut start_count = 0usize;
        let mut stop_count = 0usize;
        let mut short_psnr = 0.0f64;
        let mut long_psnr = 0.0f64;
        let mut start_psnr = 0.0f64;
        let mut stop_psnr = 0.0f64;
        let mut short_prec = 0.0f64;
        let mut long_prec = 0.0f64;
        let mut start_prec = 0.0f64;
        let mut stop_prec = 0.0f64;
        let mut short_max_err = 0.0f64;
        let mut long_max_err = 0.0f64;
        let mut underflow_frames = 0usize;
        let mut max_main_data_begin = 0u32;
        let mut max_underflow = 0u32;
        let mut mdb_zero_count = 0usize;
        let mut mdb_zero_psnr = 0.0f64;
        let mut mdb_zero_prec = 0.0f64;
        let mut mdb_zero_max_err = 0.0f64;
        let mut mdb_nonzero_count = 0usize;
        let mut mdb_nonzero_psnr = 0.0f64;
        let mut mdb_nonzero_prec = 0.0f64;
        let mut mdb_nonzero_max_err = 0.0f64;

        for info in &frame_infos {
            if info.underflow_bytes > 0 {
                underflow_frames += 1;
                max_underflow = max_underflow.max(info.underflow_bytes);
            }
            max_main_data_begin = max_main_data_begin.max(info.main_data_begin);

            let frame_idx = info.frame_index as i64;
            let start_ch = frame_idx * frame_samples as i64 - delay;
            let end_ch = start_ch + frame_samples as i64;
            if start_ch < 0 || end_ch as usize > total_per_ch {
                continue;
            }
            let start = start_ch as usize * channels;
            let end = end_ch as usize * channels;

            let stats = compare_pcm(&ff_pcm[start..end], &tao_pcm[start..end]);
            worst_frames.push((frame_idx, stats.psnr, stats.max_err));

            if info.main_data_begin == 0 {
                mdb_zero_count += 1;
                mdb_zero_psnr += stats.psnr;
                mdb_zero_prec += stats.precision_pct;
                mdb_zero_max_err = mdb_zero_max_err.max(stats.max_err);
            } else {
                mdb_nonzero_count += 1;
                mdb_nonzero_psnr += stats.psnr;
                mdb_nonzero_prec += stats.precision_pct;
                mdb_nonzero_max_err = mdb_nonzero_max_err.max(stats.max_err);
            }
            let mut has_short = false;
            let mut has_start = false;
            let mut has_stop = false;
            for gr in 0..info.granules.min(2) as usize {
                for ch in 0..info.channels.min(2) as usize {
                    match info.info[gr][ch].block_type {
                        2 => {
                            has_short = true;
                        }
                        1 => {
                            has_start = true;
                        }
                        3 => {
                            has_stop = true;
                        }
                        _ => {}
                    }
                }
            }

            if has_short {
                short_count += 1;
                short_psnr += stats.psnr;
                short_prec += stats.precision_pct;
                short_max_err = short_max_err.max(stats.max_err);
            } else {
                long_count += 1;
                long_psnr += stats.psnr;
                long_prec += stats.precision_pct;
                long_max_err = long_max_err.max(stats.max_err);
            }

            if has_start {
                start_count += 1;
                start_psnr += stats.psnr;
                start_prec += stats.precision_pct;
            }
            if has_stop {
                stop_count += 1;
                stop_psnr += stats.psnr;
                stop_prec += stats.precision_pct;
            }
        }

        if short_count > 0 {
            short_psnr /= short_count as f64;
            short_prec /= short_count as f64;
        }
        if long_count > 0 {
            long_psnr /= long_count as f64;
            long_prec /= long_count as f64;
        }
        if start_count > 0 {
            start_psnr /= start_count as f64;
            start_prec /= start_count as f64;
        }
        if stop_count > 0 {
            stop_psnr /= stop_count as f64;
            stop_prec /= stop_count as f64;
        }
        if mdb_zero_count > 0 {
            mdb_zero_psnr /= mdb_zero_count as f64;
            mdb_zero_prec /= mdb_zero_count as f64;
        }
        if mdb_nonzero_count > 0 {
            mdb_nonzero_psnr /= mdb_nonzero_count as f64;
            mdb_nonzero_prec /= mdb_nonzero_count as f64;
        }

        info!(
            "[{}] 帧类型统计: short_frames={}, long_frames={}, short_psnr={:.2}dB, long_psnr={:.2}dB, short_prec={:.2}%, long_prec={:.2}%, short_max_err={:.4}, long_max_err={:.4}, underflow_frames={}",
            path,
            short_count,
            long_count,
            short_psnr,
            long_psnr,
            short_prec,
            long_prec,
            short_max_err,
            long_max_err,
            underflow_frames
        );
        info!(
            "[{}] MainData统计: max_main_data_begin={}, max_underflow_bytes={}",
            path, max_main_data_begin, max_underflow
        );
        info!(
            "[{}] MainData分组: mdb=0 帧数={}, psnr={:.2}dB, 精度={:.2}%, max_err={:.4}; mdb>0 帧数={}, psnr={:.2}dB, 精度={:.2}%, max_err={:.4}",
            path,
            mdb_zero_count,
            mdb_zero_psnr,
            mdb_zero_prec,
            mdb_zero_max_err,
            mdb_nonzero_count,
            mdb_nonzero_psnr,
            mdb_nonzero_prec,
            mdb_nonzero_max_err
        );
        info!(
            "[{}] 起止帧统计: start_frames={}, stop_frames={}, start_psnr={:.2}dB, stop_psnr={:.2}dB, start_prec={:.2}%, stop_prec={:.2}%",
            path, start_count, stop_count, start_psnr, stop_psnr, start_prec, stop_prec
        );

        if !worst_frames.is_empty() {
            worst_frames.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (idx, psnr, max_err) in worst_frames.iter().take(5) {
                if let Some(info) = frame_infos.iter().find(|f| f.frame_index as i64 == *idx) {
                    info!(
                        "[{}] 最差帧详情: idx={}, psnr={:.2}dB, max_err={:.4}, main_data_begin={}, underflow_bytes={}",
                        path, idx, psnr, max_err, info.main_data_begin, info.underflow_bytes
                    );
                    for gr in 0..info.granules.min(2) as usize {
                        for ch in 0..info.channels.min(2) as usize {
                            let gi = info.info[gr][ch];
                            info!(
                                "[{}]   gr={}, ch={}, block_type={}, mixed={}, ws={}, part2_3_length={}, big_values={}, count1_table={}",
                                path,
                                gr,
                                ch,
                                gi.block_type,
                                gi.mixed_block_flag,
                                gi.windows_switching_flag,
                                gi.part2_3_length,
                                gi.big_values,
                                gi.count1table_select
                            );
                        }
                    }
                }
            }
        }
    }

    // 清理调试开关, 避免变体解码产生额外诊断数据.
    unsafe {
        std::env::remove_var("TAO_MP3_DEBUG_FRAME_INFO");
        std::env::remove_var("TAO_MP3_DEBUG_HUFFMAN");
        std::env::remove_var("TAO_MP3_DEBUG_PART2");
    }

    let variants = [
        (
            "禁用抗混叠",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_ALIAS",
                value: Some("1"),
            }],
        ),
        (
            "禁用频率反转",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_FREQ_INV",
                value: Some("1"),
            }],
        ),
        (
            "禁用重排序",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_REORDER",
                value: Some("1"),
            }],
        ),
        (
            "禁用MS立体声",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_MS",
                value: Some("1"),
            }],
        ),
        (
            "禁用强度立体声",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_INTENSITY",
                value: Some("1"),
            }],
        ),
        (
            "禁用Pretab",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_PRETAB",
                value: Some("1"),
            }],
        ),
        (
            "强制普通过渡窗",
            vec![EnvChange {
                key: "TAO_MP3_FORCE_NORMAL_TRANSITION_WIN",
                value: Some("1"),
            }],
        ),
        (
            "Count1符号LSB顺序",
            vec![EnvChange {
                key: "TAO_MP3_COUNT1_SIGN_LSB",
                value: Some("1"),
            }],
        ),
        (
            "禁用Count1区",
            vec![EnvChange {
                key: "TAO_MP3_DISABLE_COUNT1",
                value: Some("1"),
            }],
        ),
        (
            "MS缩放使用0.5",
            vec![EnvChange {
                key: "TAO_MP3_MS_SCALE_HALF",
                value: Some("1"),
            }],
        ),
    ];

    for (name, changes) in variants {
        let (v_sr, v_ch, v_pcm) = with_env_changes(&changes, || decode_mp3_with_tao(path))?;
        assert_eq!(v_sr, ff_sr, "变体采样率不匹配");
        assert_eq!(v_ch, ff_ch, "变体通道数不匹配");
        let v_stats = compare_pcm(&ff_pcm, &v_pcm);
        info!(
            "[{}] 变体诊断({}): max_err={:.6}, psnr={:.2}dB, 精度={:.2}%",
            path, name, v_stats.max_err, v_stats.psnr, v_stats.precision_pct
        );
    }

    assert!(stats_tao.n > 0, "无可比较样本");
    Ok(())
}

#[test]
#[ignore]
fn test_mp3_compare_data1() {
    run_compare("data/1.mp3").expect("data/1.mp3 对比失败");
}
