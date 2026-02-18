//! MP3 解码精度对比测试.
//!
//! 当前阶段输出 Tao 与 FFmpeg 的误差统计, 用于持续收敛.
//! 手动执行: cargo test --test mp3_module_compare -- --nocapture --ignored

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};
use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::frame::Frame;
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};
use tracing::info;

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.raw", tag, pid, seq)
}

fn init_test_tracing() {
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

fn decode_mp3_with_symphonia(
    path: &str,
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

    Ok((sample_rate, channels, out))
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
    let (tao_sr, tao_ch, tao_pcm) = decode_mp3_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_mp3_with_ffmpeg(path)?;
    let (ref_sr, ref_ch, ref_pcm) = decode_mp3_with_symphonia(path)?;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");
    assert_eq!(ref_sr, ff_sr, "symphonia 采样率不匹配");
    assert_eq!(ref_ch, ff_ch, "symphonia 通道数不匹配");

    let stats_tao = compare_pcm(&ff_pcm, &tao_pcm);
    let stats_ref = compare_pcm(&ff_pcm, &ref_pcm);
    let stats_tao_ref = compare_pcm(&ref_pcm, &tao_pcm);
    let align = estimate_alignment(&ff_pcm, &tao_pcm);
    let (gain_full, psnr_gain, precision_gain) = estimate_gain_full(&ff_pcm, &tao_pcm);
    let max_tao = max_abs(&tao_pcm);
    let max_ff = max_abs(&ff_pcm);
    let max_ref = max_abs(&ref_pcm);
    let frame_summary = summarize_frame_errors(&ff_pcm, &tao_pcm, tao_ch);
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
        "[{}] 幅度诊断: Tao_max={:.4}, FFmpeg_max={:.4}, symphonia_max={:.4}",
        path, max_tao, max_ff, max_ref
    );
    for line in frame_summary.lines() {
        info!("[{}] {}", path, line);
    }

    assert!(stats_tao.n > 0, "无可比较样本");
    Ok(())
}

#[test]
#[ignore]
fn test_mp3_compare_data1() {
    run_compare("data/1.mp3").expect("data/1.mp3 对比失败");
}
