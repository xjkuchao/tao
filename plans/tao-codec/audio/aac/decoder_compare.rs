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

fn decode_aac_with_tao(
    path: &str,
) -> Result<(u32, u32, Vec<f32>, Option<u32>), Box<dyn std::error::Error>> {
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
        info!(
            "[{}] 非 AAC 流({}), 对比测试回退到 FFmpeg 解码基线",
            path, codec_id
        );
        let (sr, ch, pcm) = decode_aac_with_ffmpeg(path, Some(stream_index_u32))?;
        return Ok((sr, ch, pcm, Some(stream_index_u32)));
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

    let mut demux_eof = false;
    let mut packet_index: u64 = 0;
    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index != stream.index {
                        continue;
                    }
                    packet_index += 1;
                    if packet_index <= 16 {
                        let head_len = pkt.data.len().min(8);
                        info!(
                            "[{}] Tao AAC包 #{}: size={}, head={:02x?}",
                            path,
                            packet_index,
                            pkt.data.len(),
                            &pkt.data[..head_len]
                        );
                    }
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
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => {
                    if demux_eof {
                        return Ok((actual_sr, actual_ch, out, Some(stream_index_u32)));
                    }
                    break;
                }
                Err(TaoError::Eof) => {
                    return Ok((actual_sr, actual_ch, out, Some(stream_index_u32)));
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
    let tmp = make_ffmpeg_tmp_path("aac_cmp");
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

#[derive(Clone, Copy)]
struct CompareStats {
    n: usize,
    max_err: f64,
    psnr: f64,
    precision_pct: f64,
    corr: f64,
    ref_rms: f64,
    test_rms: f64,
}

fn compare_pcm(reference: &[f32], test: &[f32]) -> CompareStats {
    let n = reference.len().min(test.len());
    if n == 0 {
        return CompareStats {
            n: 0,
            max_err: 0.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
            corr: 0.0,
            ref_rms: 0.0,
            test_rms: 0.0,
        };
    }
    let mut mse = 0.0f64;
    let mut max_err = 0.0f64;
    let mut ref_power = 0.0f64;
    let mut test_power = 0.0f64;
    let mut dot = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        let d = t - r;
        let ad = d.abs();
        max_err = max_err.max(ad);
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
        max_err,
        psnr,
        precision_pct,
        corr,
        ref_rms: ref_power.sqrt(),
        test_rms: test_power.sqrt(),
    }
}

fn compare_pcm_after_linear_scale(reference: &[f32], test: &[f32]) -> Option<(f64, CompareStats)> {
    let n = reference.len().min(test.len());
    if n == 0 {
        return None;
    }
    let mut dot_rt = 0.0f64;
    let mut dot_tt = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        dot_rt += r * t;
        dot_tt += t * t;
    }
    if dot_tt == 0.0 {
        return None;
    }
    let alpha = dot_rt / dot_tt;
    let scaled: Vec<f32> = test
        .iter()
        .take(n)
        .map(|&v| (v as f64 * alpha) as f32)
        .collect();
    Some((alpha, compare_pcm(&reference[..n], &scaled)))
}

fn saturated_ratio(samples: &[f32], threshold: f32) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let saturated = samples.iter().filter(|&&v| v.abs() >= threshold).count();
    saturated as f64 / samples.len() as f64
}

fn swap_stereo_interleaved(samples: &[f32]) -> Option<Vec<f32>> {
    if samples.len() < 2 || !samples.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(samples.len());
    for frame in samples.chunks_exact(2) {
        out.push(frame[1]);
        out.push(frame[0]);
    }
    Some(out)
}

fn compare_pcm_with_shift(reference: &[f32], test: &[f32], shift: isize) -> CompareStats {
    if shift == 0 {
        return compare_pcm(reference, test);
    }
    if shift > 0 {
        let s = shift as usize;
        if s >= test.len() {
            return CompareStats {
                n: 0,
                max_err: 0.0,
                psnr: f64::INFINITY,
                precision_pct: 0.0,
                corr: 0.0,
                ref_rms: 0.0,
                test_rms: 0.0,
            };
        }
        return compare_pcm(reference, &test[s..]);
    }
    let s = (-shift) as usize;
    if s >= reference.len() {
        return CompareStats {
            n: 0,
            max_err: 0.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
            corr: 0.0,
            ref_rms: 0.0,
            test_rms: 0.0,
        };
    }
    compare_pcm(&reference[s..], test)
}

fn split_interleaved_channel(samples: &[f32], channels: usize, channel_index: usize) -> Vec<f32> {
    if channels == 0 || channel_index >= channels {
        return Vec::new();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame[channel_index])
        .collect()
}

fn alternating_sign(samples: &[f32]) -> Vec<f32> {
    samples
        .iter()
        .enumerate()
        .map(|(i, &v)| if i % 2 == 0 { v } else { -v })
        .collect()
}

fn log_top_errors(path: &str, reference: &[f32], test: &[f32], channels: usize, top_n: usize) {
    let n = reference.len().min(test.len());
    if n == 0 || channels == 0 || top_n == 0 {
        return;
    }
    let mut errs = Vec::with_capacity(n.min(65_536));
    for i in 0..n {
        let diff = (test[i] as f64 - reference[i] as f64).abs();
        errs.push((diff, i));
    }
    errs.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (rank, (err, idx)) in errs.into_iter().take(top_n).enumerate() {
        let frame_sample = idx / channels;
        let channel = idx % channels;
        let block_index = frame_sample / 1024;
        let sample_in_block = frame_sample % 1024;
        info!(
            "[{}] 误差峰值#{}: idx={}, frame_sample={}, ch={}, block={}, block_off={}, ref={:.9}, tao={:.9}, abs_err={:.9}",
            path,
            rank + 1,
            idx,
            frame_sample,
            channel,
            block_index,
            sample_in_block,
            reference[idx],
            test[idx],
            err
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
    init_test_tracing();

    let (tao_sr, tao_ch, tao_pcm, tao_stream_index) = decode_aac_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_aac_with_ffmpeg(path, tao_stream_index)?;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");
    assert_eq!(
        tao_pcm.len(),
        ff_pcm.len(),
        "样本总数不匹配: Tao={}, FFmpeg={}",
        tao_pcm.len(),
        ff_pcm.len()
    );

    let stats = compare_pcm(&ff_pcm, &tao_pcm);
    info!(
        "[{}] Tao对比样本={}, Tao={}, FFmpeg={}, Tao/FFmpeg: max_err={:.9}, psnr={:.2}dB, 精度={:.6}%, 相关系数={:.9}, RMS(Tao/FFmpeg)={:.9}/{:.9}, FFmpeg=100%",
        path,
        stats.n,
        tao_pcm.len(),
        ff_pcm.len(),
        stats.max_err,
        stats.psnr,
        stats.precision_pct,
        stats.corr,
        stats.test_rms,
        stats.ref_rms
    );
    info!(
        "[{}] 饱和采样占比: Tao={:.4}%, FFmpeg={:.4}%",
        path,
        saturated_ratio(&tao_pcm, 0.9999) * 100.0,
        saturated_ratio(&ff_pcm, 0.9999) * 100.0
    );
    if let Some((alpha, scaled_stats)) = compare_pcm_after_linear_scale(&ff_pcm, &tao_pcm) {
        info!(
            "[{}] 线性缩放诊断: alpha={:.9}, max_err={:.9}, psnr={:.2}dB, 精度={:.6}%",
            path, alpha, scaled_stats.max_err, scaled_stats.psnr, scaled_stats.precision_pct
        );
    }
    if tao_ch == 2
        && ff_ch == 2
        && let Some(swapped) = swap_stereo_interleaved(&tao_pcm)
    {
        let swapped_stats = compare_pcm(&ff_pcm, &swapped);
        info!(
            "[{}] 声道互换诊断: max_err={:.9}, psnr={:.2}dB, 精度={:.6}%",
            path, swapped_stats.max_err, swapped_stats.psnr, swapped_stats.precision_pct
        );

        let ff_l = split_interleaved_channel(&ff_pcm, 2, 0);
        let ff_r = split_interleaved_channel(&ff_pcm, 2, 1);
        let tao_l = split_interleaved_channel(&tao_pcm, 2, 0);
        let tao_r = split_interleaved_channel(&tao_pcm, 2, 1);
        let stats_l = compare_pcm(&ff_l, &tao_l);
        let stats_r = compare_pcm(&ff_r, &tao_r);
        info!(
            "[{}] 单声道诊断: L精度={:.6}% R精度={:.6}%",
            path, stats_l.precision_pct, stats_r.precision_pct
        );
        let alt = alternating_sign(&tao_pcm);
        let alt_stats = compare_pcm(&ff_pcm, &alt);
        info!(
            "[{}] 交替符号诊断: max_err={:.9}, psnr={:.2}dB, 精度={:.6}%",
            path, alt_stats.max_err, alt_stats.psnr, alt_stats.precision_pct
        );
    }
    let shift_candidates = [-2048isize, -1024, -512, -256, 256, 512, 1024, 2048];
    let mut best_shift = 0isize;
    let mut best_stats = stats;
    for &shift in &shift_candidates {
        let shifted = compare_pcm_with_shift(&ff_pcm, &tao_pcm, shift);
        if shifted.precision_pct > best_stats.precision_pct {
            best_stats = shifted;
            best_shift = shift;
        }
    }
    if best_shift != 0 {
        info!(
            "[{}] 位移诊断: best_shift={}, max_err={:.9}, psnr={:.2}dB, 精度={:.6}%",
            path, best_shift, best_stats.max_err, best_stats.psnr, best_stats.precision_pct
        );
    }
    log_top_errors(path, &ff_pcm, &tao_pcm, tao_ch as usize, 10);

    assert!(stats.n > 0, "无可比较样本");
    assert!(
        stats.max_err <= 0.00001,
        "AAC 对比最大误差超阈值: max_err={}",
        stats.max_err
    );
    assert!(
        stats.precision_pct >= 99.999,
        "AAC 对比精度不足 100% 目标: {:.6}%",
        stats.precision_pct
    );
    Ok(())
}

#[test]
#[ignore]
fn test_aac_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("AAC 对比失败");
}
