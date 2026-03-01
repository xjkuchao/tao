//! AAC 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare -- data/1.m4a
//! 2) TAO_AAC_COMPARE_INPUT=data/1.m4a cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare
//! 3) TAO_AAC_COMPARE_INPUT=https://samples.ffmpeg.org/A-codecs/AAC/ct_faac-adts_stereo.aac cargo test --test aac_module_compare -- --nocapture --ignored test_aac_compare

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::frame::{AudioFrame, Frame};
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const DEFAULT_COMPARE_SECONDS: u32 = 10;
type DecodeResult = Result<(u32, u32, Vec<f32>, Option<u32>, usize), Box<dyn std::error::Error>>;

fn compare_seconds_limit() -> u32 {
    std::env::var("TAO_AAC_COMPARE_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_COMPARE_SECONDS)
}

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.raw", tag, pid, seq)
}

fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
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

fn append_audio_frame_to_f32(
    af: &AudioFrame,
    nominal_bits: u32,
    out: &mut Vec<f32>,
) -> Result<(), Box<dyn std::error::Error>> {
    if af.sample_format.is_planar() {
        return Err("当前对比脚本不支持平面音频输出".into());
    }

    let data = af.data.first().ok_or("音频帧缺少主数据平面, 无法对比")?;

    match af.sample_format {
        SampleFormat::U8 => {
            out.extend(data.iter().map(|&v| (f32::from(v) - 128.0) / 128.0));
        }
        SampleFormat::S16 => {
            out.extend(
                data.chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0),
            );
        }
        SampleFormat::S32 => {
            let scale = if (1..32).contains(&nominal_bits) {
                (1u64 << (nominal_bits - 1)) as f32
            } else {
                2147483648.0
            };
            out.extend(
                data.chunks_exact(4)
                    .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / scale),
            );
        }
        SampleFormat::F32 => {
            out.extend(
                data.chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        _ => {
            return Err(format!("当前对比脚本暂不支持采样格式: {}", af.sample_format).into());
        }
    }

    Ok(())
}

fn decode_aac_with_tao(path: &str) -> DecodeResult {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = open_input(path)?;
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
        .find(|s| s.codec_id == CodecId::Aac)
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
    if codec_id != CodecId::Aac {
        println!(
            "[{}] 非 AAC 流({}), 对比测试回退到 FFmpeg 解码基线",
            path, codec_id
        );
        let (sr, ch, pcm) = decode_aac_with_ffmpeg(path, Some(stream_index_u32), None)?;
        return Ok((sr, ch, pcm, Some(stream_index_u32), 0));
    }

    let (sample_rate, channel_layout, sample_format, frame_size) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => (
            a.sample_rate,
            a.channel_layout,
            a.sample_format,
            a.frame_size,
        ),
        _ => (44100, ChannelLayout::STEREO, SampleFormat::F32, 1024),
    };

    let nominal_bits = match sample_format {
        SampleFormat::U8 => 8,
        SampleFormat::S16 => 16,
        _ => 32,
    };

    let params = CodecParameters {
        codec_id,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format,
            frame_size,
        }),
    };

    let mut decoder = codec_registry.create_decoder(codec_id)?;
    decoder.open(&params)?;

    let mut out = Vec::<f32>::new();
    let mut actual_sr = sample_rate;
    let mut actual_ch = channel_layout.channels;
    let max_seconds = compare_seconds_limit();
    let mut max_samples: Option<usize> = None;

    let mut demux_eof = false;
    let mut packet_index: u64 = 0;
    let mut first_audio_packet_seen = false;
    let mut estimated_leading_trim_samples = 0usize;
    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index != stream.index {
                        continue;
                    }
                    if !first_audio_packet_seen {
                        first_audio_packet_seen = true;
                        let has_adts_header = pkt.data.len() >= 2
                            && pkt.data[0] == 0xFF
                            && (pkt.data[1] & 0xF0) == 0xF0;
                        if !has_adts_header
                            && pkt.pts != tao_core::timestamp::NOPTS_VALUE
                            && pkt.pts < 0
                        {
                            estimated_leading_trim_samples = (-pkt.pts) as usize;
                        }
                    }
                    packet_index += 1;
                    decoder.send_packet(&pkt).map_err(|e| {
                        format!(
                            "发送 AAC 包失败: {}, 包序号={}, pos={}, 大小={}",
                            e,
                            packet_index,
                            pkt.pos,
                            pkt.data.len()
                        )
                    })?;
                }
                Err(TaoError::Eof) => {
                    decoder.send_packet(&Packet::empty()).map_err(|e| {
                        format!("发送 AAC 刷新包失败: {}, 已处理包数={}", e, packet_index)
                    })?;
                    demux_eof = true;
                }
                Err(e) => {
                    return Err(
                        format!("读取 AAC 包失败: {}, 已处理包数={}", e, packet_index).into(),
                    );
                }
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    actual_sr = af.sample_rate;
                    actual_ch = af.channel_layout.channels;
                    append_audio_frame_to_f32(&af, nominal_bits, &mut out)?;
                    if max_samples.is_none() && actual_sr > 0 && actual_ch > 0 {
                        max_samples = Some(
                            (actual_sr as usize) * (actual_ch as usize) * (max_seconds as usize),
                        );
                    }
                    if let Some(limit) = max_samples
                        && out.len() >= limit
                    {
                        out.truncate(limit);
                        return Ok((
                            actual_sr,
                            actual_ch,
                            out,
                            Some(stream_index_u32),
                            estimated_leading_trim_samples,
                        ));
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => {
                    if demux_eof {
                        return Ok((
                            actual_sr,
                            actual_ch,
                            out,
                            Some(stream_index_u32),
                            estimated_leading_trim_samples,
                        ));
                    }
                    break;
                }
                Err(TaoError::Eof) => {
                    return Ok((
                        actual_sr,
                        actual_ch,
                        out,
                        Some(stream_index_u32),
                        estimated_leading_trim_samples,
                    ));
                }
                Err(e) => {
                    return Err(
                        format!("接收 AAC 帧失败: {}, 当前包序号={}", e, packet_index).into(),
                    );
                }
            }
        }
    }
}

fn decode_aac_with_ffmpeg(
    path: &str,
    preferred_stream: Option<u32>,
    target_params: Option<(u32, u32)>,
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
    let (out_sr, out_ch) = target_params.unwrap_or((sr, ch));
    let max_seconds = compare_seconds_limit().to_string();
    let out_sr_s = out_sr.to_string();
    let out_ch_s = out_ch.to_string();
    let tmp = make_ffmpeg_tmp_path("aac_cmp");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
            "-map",
            &map_spec,
            "-t",
            &max_seconds,
            "-ar",
            &out_sr_s,
            "-ac",
            &out_ch_s,
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
    Ok((out_sr, out_ch, pcm))
}

#[derive(Clone, Copy)]
struct CompareStats {
    n: usize,
    lag: isize,
    max_err: f64,
    max_err_idx: usize,
    max_err_ref: f64,
    max_err_test: f64,
    psnr: f64,
    precision_pct: f64,
    corr: f64,
    ref_rms: f64,
    test_rms: f64,
}

#[derive(Clone, Copy)]
struct MaxErrorMapping {
    aligned_idx: usize,
    frame_idx: usize,
    channel_idx: usize,
    block_1024: usize,
    block_offset: usize,
    ref_idx: usize,
    test_idx: usize,
}

fn align_by_lag<'a>(reference: &'a [f32], test: &'a [f32], lag: isize) -> (&'a [f32], &'a [f32]) {
    if lag >= 0 {
        let rs = lag as usize;
        let r = if rs < reference.len() {
            &reference[rs..]
        } else {
            &reference[0..0]
        };
        (r, test)
    } else {
        let ts = (-lag) as usize;
        let t = if ts < test.len() {
            &test[ts..]
        } else {
            &test[0..0]
        };
        (reference, t)
    }
}

fn aligned_index_to_original(aligned_idx: usize, lag: isize) -> (usize, usize) {
    if lag >= 0 {
        (aligned_idx + lag as usize, aligned_idx)
    } else {
        (aligned_idx, aligned_idx + (-lag) as usize)
    }
}

fn map_max_error_index(max_err_idx: usize, channels: usize, lag: isize) -> Option<MaxErrorMapping> {
    if channels == 0 {
        return None;
    }
    let frame_idx = max_err_idx / channels;
    let channel_idx = max_err_idx % channels;
    let block_1024 = frame_idx / 1024;
    let block_offset = frame_idx % 1024;
    let (ref_idx, test_idx) = aligned_index_to_original(max_err_idx, lag);
    Some(MaxErrorMapping {
        aligned_idx: max_err_idx,
        frame_idx,
        channel_idx,
        block_1024,
        block_offset,
        ref_idx,
        test_idx,
    })
}

fn compare_pcm_core(reference: &[f32], test: &[f32], lag: isize) -> CompareStats {
    let n = reference.len().min(test.len());
    if n == 0 {
        return CompareStats {
            n: 0,
            lag,
            max_err: 0.0,
            max_err_idx: 0,
            max_err_ref: 0.0,
            max_err_test: 0.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
            corr: 0.0,
            ref_rms: 0.0,
            test_rms: 0.0,
        };
    }
    let mut mse = 0.0f64;
    let mut max_err = 0.0f64;
    let mut max_err_idx = 0usize;
    let mut max_err_ref = 0.0f64;
    let mut max_err_test = 0.0f64;
    let mut ref_power = 0.0f64;
    let mut test_power = 0.0f64;
    let mut dot = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        let d = t - r;
        let ad = d.abs();
        if ad > max_err {
            max_err = ad;
            max_err_idx = i;
            max_err_ref = r;
            max_err_test = t;
        }
        mse += d * d;
        ref_power += r * r;
        test_power += t * t;
        dot += r * t;
    }
    mse /= n as f64;
    ref_power /= n as f64;
    test_power /= n as f64;
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
    precision_pct = precision_pct.clamp(0.0, 100.0);
    let corr = if ref_power > 0.0 && test_power > 0.0 {
        (dot / n as f64) / (ref_power.sqrt() * test_power.sqrt())
    } else {
        0.0
    };

    CompareStats {
        n,
        lag,
        max_err,
        max_err_idx,
        max_err_ref,
        max_err_test,
        psnr,
        precision_pct,
        corr,
        ref_rms: ref_power.sqrt(),
        test_rms: test_power.sqrt(),
    }
}

fn find_best_lag(reference: &[f32], test: &[f32], max_lag: usize, probe_len: usize) -> isize {
    let mut best_lag = 0isize;
    let mut best_mse = f64::INFINITY;
    let max_lag = max_lag as isize;
    for lag in -max_lag..=max_lag {
        let (ref_start, test_start) = if lag >= 0 {
            (lag as usize, 0usize)
        } else {
            (0usize, (-lag) as usize)
        };
        if ref_start >= reference.len() || test_start >= test.len() {
            continue;
        }
        let n = probe_len
            .min(reference.len() - ref_start)
            .min(test.len() - test_start);
        if n == 0 {
            continue;
        }
        let mut mse = 0.0f64;
        for i in 0..n {
            let d = test[test_start + i] as f64 - reference[ref_start + i] as f64;
            mse += d * d;
        }
        mse /= n as f64;
        if mse < best_mse {
            best_mse = mse;
            best_lag = lag;
        }
    }
    best_lag
}

fn compare_pcm(reference: &[f32], test: &[f32]) -> CompareStats {
    if reference.is_empty() || test.is_empty() {
        return compare_pcm_core(reference, test, 0);
    }
    let lag = find_best_lag(reference, test, 4096, 32768);
    let (reference, test) = align_by_lag(reference, test, lag);
    compare_pcm_core(reference, test, lag)
}

fn fit_linear_adjustment(
    reference: &[f32],
    test: &[f32],
    lag: isize,
) -> Option<(f64, f64, f64, f64)> {
    let (reference, test) = align_by_lag(reference, test, lag);
    let n = reference.len().min(test.len());
    if n == 0 {
        return None;
    }
    let mut sum_t = 0.0f64;
    let mut sum_r = 0.0f64;
    let mut sum_tt = 0.0f64;
    let mut sum_tr = 0.0f64;
    let mut ref_power = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        sum_t += t;
        sum_r += r;
        sum_tt += t * t;
        sum_tr += t * r;
        ref_power += r * r;
    }
    let n_f = n as f64;
    let denom = n_f * sum_tt - sum_t * sum_t;
    if denom.abs() <= f64::EPSILON {
        return None;
    }
    let gain = (n_f * sum_tr - sum_t * sum_r) / denom;
    let bias = (sum_r - gain * sum_t) / n_f;

    let mut mse = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        let d = gain * t + bias - r;
        mse += d * d;
    }
    mse /= n_f;
    let ref_power = ref_power / n_f;
    let precision_pct = if ref_power > 0.0 {
        (ref_power / (ref_power + mse)) * 100.0
    } else if mse == 0.0 {
        100.0
    } else {
        0.0
    };
    Some((gain, bias, mse, precision_pct.clamp(0.0, 100.0)))
}

fn channel_corr_matrix(
    reference: &[f32],
    test: &[f32],
    channels: usize,
    lag: isize,
    max_frames: usize,
) -> Vec<Vec<f64>> {
    if channels == 0 {
        return Vec::new();
    }
    let (reference, test) = align_by_lag(reference, test, lag);
    let frames = (reference.len() / channels)
        .min(test.len() / channels)
        .min(max_frames);
    if frames == 0 {
        return vec![vec![0.0; channels]; channels];
    }

    let mut out = vec![vec![0.0f64; channels]; channels];
    for rch in 0..channels {
        for tch in 0..channels {
            let mut dot = 0.0f64;
            let mut rp = 0.0f64;
            let mut tp = 0.0f64;
            for i in 0..frames {
                let rv = reference[i * channels + rch] as f64;
                let tv = test[i * channels + tch] as f64;
                dot += rv * tv;
                rp += rv * rv;
                tp += tv * tv;
            }
            out[rch][tch] = if rp > 0.0 && tp > 0.0 {
                dot / (rp.sqrt() * tp.sqrt())
            } else {
                0.0
            };
        }
    }
    out
}

fn top_error_frames(
    reference: &[f32],
    test: &[f32],
    channels: usize,
    lag: isize,
    limit: usize,
) -> Vec<(usize, f64, f64)> {
    if channels == 0 {
        return Vec::new();
    }
    let (reference, test) = align_by_lag(reference, test, lag);

    let frames = (reference.len() / channels).min(test.len() / channels);
    if frames == 0 {
        return Vec::new();
    }

    let mut rows = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut mse = 0.0f64;
        let mut max_err = 0.0f64;
        for ch in 0..channels {
            let idx = frame * channels + ch;
            let d = test[idx] as f64 - reference[idx] as f64;
            let ad = d.abs();
            if ad > max_err {
                max_err = ad;
            }
            mse += d * d;
        }
        mse /= channels as f64;
        rows.push((frame, mse, max_err));
    }
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(limit);
    rows
}

fn top_error_frames_by_channel(
    reference: &[f32],
    test: &[f32],
    channels: usize,
    lag: isize,
    limit: usize,
) -> Vec<Vec<(usize, f64, f64)>> {
    if channels == 0 {
        return Vec::new();
    }
    let (reference, test) = align_by_lag(reference, test, lag);
    let frames = (reference.len() / channels).min(test.len() / channels);
    if frames == 0 {
        return vec![Vec::new(); channels];
    }
    let mut out = vec![Vec::new(); channels];
    for ch in 0..channels {
        let mut rows = Vec::with_capacity(frames);
        for frame in 0..frames {
            let idx = frame * channels + ch;
            let d = test[idx] as f64 - reference[idx] as f64;
            rows.push((frame, d * d, d.abs()));
        }
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        rows.truncate(limit);
        out[ch] = rows;
    }
    out
}

fn top_error_blocks_by_channel(
    reference: &[f32],
    test: &[f32],
    channels: usize,
    lag: isize,
    block_frames: usize,
    limit: usize,
) -> Vec<Vec<(usize, usize, usize, f64, f64)>> {
    if channels == 0 || block_frames == 0 {
        return Vec::new();
    }
    let (reference, test) = align_by_lag(reference, test, lag);
    let frames = (reference.len() / channels).min(test.len() / channels);
    if frames == 0 {
        return vec![Vec::new(); channels];
    }
    let blocks = frames.div_ceil(block_frames);
    let mut out = vec![Vec::new(); channels];
    for ch in 0..channels {
        let mut rows = Vec::with_capacity(blocks);
        for block in 0..blocks {
            let start = block * block_frames;
            let end = (start + block_frames).min(frames);
            let mut sum = 0.0f64;
            let mut count = 0usize;
            let mut max_err = 0.0f64;
            for frame in start..end {
                let idx = frame * channels + ch;
                let d = test[idx] as f64 - reference[idx] as f64;
                let ad = d.abs();
                if ad > max_err {
                    max_err = ad;
                }
                sum += d * d;
                count += 1;
            }
            let mse = if count > 0 { sum / count as f64 } else { 0.0 };
            rows.push((block, start, end, mse, max_err));
        }
        rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        rows.truncate(limit);
        out[ch] = rows;
    }
    out
}

fn error_bins_by_ref_abs(
    reference: &[f32],
    test: &[f32],
    lag: isize,
    edges: &[f64],
) -> Vec<(String, usize, f64, f64)> {
    let mut sums = vec![0.0f64; edges.len() + 1];
    let mut max_errs = vec![0.0f64; edges.len() + 1];
    let mut counts = vec![0usize; edges.len() + 1];
    let (reference, test) = align_by_lag(reference, test, lag);
    let n = reference.len().min(test.len());
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        let abs_r = r.abs();
        let err = (t - r).abs();
        let mut idx = edges.len();
        for (j, edge) in edges.iter().enumerate() {
            if abs_r <= *edge {
                idx = j;
                break;
            }
        }
        counts[idx] += 1;
        sums[idx] += err * err;
        if err > max_errs[idx] {
            max_errs[idx] = err;
        }
    }
    let mut out = Vec::with_capacity(edges.len() + 1);
    for i in 0..=edges.len() {
        let label = if i == 0 {
            format!("[0,{:.2}]", edges[0])
        } else if i < edges.len() {
            format!("({:.2},{:.2}]", edges[i - 1], edges[i])
        } else {
            format!("(>{:.2})", edges[edges.len() - 1])
        };
        let mse = if counts[i] > 0 {
            sums[i] / counts[i] as f64
        } else {
            0.0
        };
        out.push((label, counts[i], mse, max_errs[i]));
    }
    out
}

fn local_best_lag_by_channel(
    reference: &[f32],
    test: &[f32],
    channels: usize,
    lag: isize,
    channel_idx: usize,
    center_frame: usize,
    window_frames: usize,
    max_lag_frames: usize,
) -> Option<(isize, usize, f64)> {
    if channels == 0 || channel_idx >= channels || window_frames == 0 {
        return None;
    }
    let (reference, test) = align_by_lag(reference, test, lag);
    let frames = (reference.len() / channels).min(test.len() / channels);
    if frames == 0 {
        return None;
    }

    let half = window_frames / 2;
    let start = center_frame.saturating_sub(half);
    let end = (start + window_frames).min(frames);
    if end <= start {
        return None;
    }

    let mut best_lag = 0isize;
    let mut best_mse = f64::INFINITY;
    let mut best_count = 0usize;
    let max_lag = max_lag_frames as isize;
    for local_lag in -max_lag..=max_lag {
        let (ref_start, test_start) = if local_lag >= 0 {
            (start + local_lag as usize, start)
        } else {
            (start, start + (-local_lag) as usize)
        };
        if ref_start >= frames || test_start >= frames {
            continue;
        }
        let count = (end - start)
            .min(frames - ref_start)
            .min(frames - test_start);
        if count == 0 {
            continue;
        }

        let mut mse = 0.0f64;
        for i in 0..count {
            let r_idx = (ref_start + i) * channels + channel_idx;
            let t_idx = (test_start + i) * channels + channel_idx;
            let d = test[t_idx] as f64 - reference[r_idx] as f64;
            mse += d * d;
        }
        mse /= count as f64;
        if mse < best_mse {
            best_mse = mse;
            best_lag = local_lag;
            best_count = count;
        }
    }

    if best_count == 0 {
        None
    } else {
        Some((best_lag, best_count, best_mse))
    }
}

fn print_error_neighborhood(
    path: &str,
    reference: &[f32],
    test: &[f32],
    lag: isize,
    idx: usize,
    radius: usize,
) {
    if reference.is_empty() || test.is_empty() {
        return;
    }
    let (reference, test) = align_by_lag(reference, test, lag);
    let n = reference.len().min(test.len());
    if idx >= n {
        return;
    }
    let start = idx.saturating_sub(radius);
    let end = (idx + radius + 1).min(n);
    println!("[{}] 最大误差邻域[{}..{}):", path, start, end);
    for i in start..end {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        println!(
            "[{}]   idx={}, FFmpeg={:.9}, Tao={:.9}, diff={:.9}",
            path,
            i,
            r,
            t,
            t - r
        );
    }
}

fn resolve_input() -> Result<String, Box<dyn std::error::Error>> {
    let mut after_dd = std::env::args().skip_while(|v| v != "--").skip(1);
    if let Some(arg) = after_dd.next() {
        return Ok(arg);
    }
    if let Ok(env) = std::env::var("TAO_AAC_COMPARE_INPUT")
        && !env.trim().is_empty()
    {
        return Ok(env);
    }
    Err("请通过参数或 TAO_AAC_COMPARE_INPUT 指定 M4A/AAC 文件或 URL".into())
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (tao_sr, tao_ch, tao_pcm, tao_stream_index, tao_leading_trim_samples) =
        decode_aac_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) =
        decode_aac_with_ffmpeg(path, tao_stream_index, Some((tao_sr, tao_ch)))?;

    if tao_sr != ff_sr {
        return Err(format!(
            "AAC 对比失败: 采样率不匹配, Tao={}, FFmpeg={}",
            tao_sr, ff_sr
        )
        .into());
    }
    if tao_ch != ff_ch {
        return Err(format!(
            "AAC 对比失败: 通道数不匹配, Tao={}, FFmpeg={}",
            tao_ch, ff_ch
        )
        .into());
    }
    if tao_pcm.len() != ff_pcm.len() {
        eprintln!(
            "[AAC] 样本总数差异: Tao={}, FFmpeg={}",
            tao_pcm.len(),
            ff_pcm.len()
        );
    }

    let stats = compare_pcm(&ff_pcm, &tao_pcm);
    println!(
        "[{}] Tao对比样本={}, Tao={}, FFmpeg={}, lag={}, Tao/FFmpeg: max_err={:.9}, max_err_idx={}, psnr={:.2}dB, 精度={:.6}%, 相关系数={:.9}, RMS(Tao/FFmpeg)={:.9}/{:.9}, FFmpeg=100%",
        path,
        stats.n,
        tao_pcm.len(),
        ff_pcm.len(),
        stats.lag,
        stats.max_err,
        stats.max_err_idx,
        stats.psnr,
        stats.precision_pct,
        stats.corr,
        stats.test_rms,
        stats.ref_rms
    );
    println!(
        "[{}] 最大误差点: idx={}, FFmpeg={:.9}, Tao={:.9}, diff={:.9}",
        path,
        stats.max_err_idx,
        stats.max_err_ref,
        stats.max_err_test,
        stats.max_err_test - stats.max_err_ref
    );
    if let Some(mapping) = map_max_error_index(stats.max_err_idx, tao_ch as usize, stats.lag) {
        let t = if tao_sr > 0 {
            mapping.frame_idx as f64 / tao_sr as f64
        } else {
            0.0
        };
        println!(
            "[{}] 最大误差映射: aligned_idx={}, ref_idx={}, tao_idx={}, channel={}, frame={}, time={:.3}s, aac_block={}, block_offset={}",
            path,
            mapping.aligned_idx,
            mapping.ref_idx,
            mapping.test_idx,
            mapping.channel_idx,
            mapping.frame_idx,
            t,
            mapping.block_1024,
            mapping.block_offset
        );
        if tao_leading_trim_samples > 0 {
            let raw_frame = mapping.frame_idx + tao_leading_trim_samples;
            let raw_block = raw_frame / 1024;
            let raw_block_offset = raw_frame % 1024;
            let raw_t = if tao_sr > 0 {
                raw_frame as f64 / tao_sr as f64
            } else {
                0.0
            };
            println!(
                "[{}] 原始帧映射(含priming={}): raw_frame={}, raw_time={:.3}s, raw_aac_block={}, raw_block_offset={}",
                path, tao_leading_trim_samples, raw_frame, raw_t, raw_block, raw_block_offset
            );
        }
        if let Some((local_lag, count, local_mse)) = local_best_lag_by_channel(
            &ff_pcm,
            &tao_pcm,
            tao_ch as usize,
            stats.lag,
            mapping.channel_idx,
            mapping.frame_idx,
            2048,
            16,
        ) {
            println!(
                "[{}] 最大误差点局部lag(按通道): ch={}, center_frame={}, window_frames={}, local_lag={}, global_lag={}, combined_lag={}, local_mse={:.9}",
                path,
                mapping.channel_idx,
                mapping.frame_idx,
                count,
                local_lag,
                stats.lag,
                stats.lag + local_lag,
                local_mse
            );
        }
    }
    print_error_neighborhood(path, &ff_pcm, &tao_pcm, stats.lag, stats.max_err_idx, 4);
    if tao_ch == ff_ch && tao_ch > 1 && stats.precision_pct < 99.9 {
        let channels = tao_ch as usize;
        let corr = channel_corr_matrix(&ff_pcm, &tao_pcm, channels, stats.lag, 16_384);
        println!("[{}] 通道相关矩阵(行=FFmpeg, 列=Tao):", path);
        for row in corr {
            let line = row
                .iter()
                .map(|v| format!("{v:.3}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("[{}]   {}", path, line);
        }
        let top_frames = top_error_frames(&ff_pcm, &tao_pcm, channels, stats.lag, 5);
        if !top_frames.is_empty() {
            println!("[{}] 帧级误差 Top5(按 MSE):", path);
            for (frame, mse, max_err) in top_frames {
                let t = frame as f64 / tao_sr as f64;
                if tao_leading_trim_samples > 0 {
                    let raw_frame = frame + tao_leading_trim_samples;
                    let raw_block = raw_frame / 1024;
                    let raw_block_offset = raw_frame % 1024;
                    let raw_t = if tao_sr > 0 {
                        raw_frame as f64 / tao_sr as f64
                    } else {
                        0.0
                    };
                    println!(
                        "[{}]   frame={}, time={:.3}s, raw_frame={}, raw_time={:.3}s, raw_block={}, raw_block_offset={}, mse={:.9}, max_err={:.9}",
                        path, frame, t, raw_frame, raw_t, raw_block, raw_block_offset, mse, max_err
                    );
                } else {
                    println!(
                        "[{}]   frame={}, time={:.3}s, mse={:.9}, max_err={:.9}",
                        path, frame, t, mse, max_err
                    );
                }
            }
        }
        let per_channel = top_error_frames_by_channel(&ff_pcm, &tao_pcm, channels, stats.lag, 3);
        if !per_channel.is_empty() {
            println!("[{}] 按通道帧级误差 Top3:", path);
            for (ch, rows) in per_channel.into_iter().enumerate() {
                for (frame, mse, max_err) in rows {
                    let t = frame as f64 / tao_sr as f64;
                    println!(
                        "[{}]   ch={}, frame={}, time={:.3}s, mse={:.9}, max_err={:.9}",
                        path, ch, frame, t, mse, max_err
                    );
                }
            }
        }
        let block_rows =
            top_error_blocks_by_channel(&ff_pcm, &tao_pcm, channels, stats.lag, 1024, 5);
        if !block_rows.is_empty() {
            println!("[{}] 按通道 1024帧块误差 Top5:", path);
            for (ch, rows) in block_rows.into_iter().enumerate() {
                for (block, start, _end, mse, max_err) in rows {
                    let t = start as f64 / tao_sr as f64;
                    println!(
                        "[{}]   ch={}, block={}, start_frame={}, time={:.3}s, mse={:.9}, max_err={:.9}",
                        path, ch, block, start, t, mse, max_err
                    );
                }
            }
        }

        let bins = error_bins_by_ref_abs(
            &ff_pcm,
            &tao_pcm,
            stats.lag,
            &[0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0],
        );
        if !bins.is_empty() {
            println!("[{}] 按参考幅值分桶误差统计:", path);
            for (label, count, mse, max_err) in bins {
                println!(
                    "[{}]   abs_ref={}, count={}, mse={:.9}, max_err={:.9}",
                    path, label, count, mse, max_err
                );
            }
        }

        if let Some((gain, bias, mse, precision)) =
            fit_linear_adjustment(&ff_pcm, &tao_pcm, stats.lag)
        {
            println!(
                "[{}] 线性校正诊断(test' = gain*test+bias): gain={:.9}, bias={:.9}, mse={:.9}, 估计精度={:.6}%",
                path, gain, bias, mse, precision
            );
        }
    }

    if stats.n == 0 {
        return Err("AAC 对比失败: 无可比较样本".into());
    }
    if !(stats.max_err <= 1.0 || stats.psnr >= 40.0) {
        return Err(format!(
            "AAC 对比失败: 最大误差超阈值且 PSNR 过低, max_err={}, psnr={:.2}dB",
            stats.max_err, stats.psnr
        )
        .into());
    }
    if stats.precision_pct < 99.9 {
        return Err(format!(
            "AAC 对比失败: 精度不足 99.9%, 当前={:.6}%",
            stats.precision_pct
        )
        .into());
    }
    Ok(())
}

#[test]
#[ignore]
fn test_aac_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("AAC 对比失败");
}
