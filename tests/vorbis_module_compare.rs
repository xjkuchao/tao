//! Vorbis 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) cargo test --test vorbis_module_compare -- --nocapture --ignored -- data/1.ogg
//! 2) TAO_VORBIS_COMPARE_INPUT=data/1.ogg cargo test --test vorbis_module_compare -- --nocapture --ignored
//! 3) TAO_VORBIS_COMPARE_INPUT=https://samples.ffmpeg.org/A-codecs/vorbis/ogg/vorbis_test.ogg cargo test --test vorbis_module_compare -- --nocapture --ignored

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use lewton::inside_ogg::OggStreamReader;
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
    demuxer.open(&mut io)?;

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

    let params = CodecParameters {
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
    decoder.open(&params)?;

    let mut out = Vec::<f32>::new();
    let mut actual_sr = sample_rate;
    let mut actual_ch = channel_layout.channels;
    let mut logged_first_frame = false;

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
                    if !logged_first_frame {
                        info!(
                            "[{}] Tao首帧: format={:?}, planes={}, bytes0={}, samples_per_ch={}, ch={}",
                            path,
                            af.sample_format,
                            af.data.len(),
                            af.data.first().map_or(0, Vec::len),
                            af.nb_samples,
                            af.channel_layout.channels
                        );
                        logged_first_frame = true;
                    }
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
                        return Ok((actual_sr, actual_ch, out, Some(stream_index_u32)));
                    }
                    break;
                }
                Err(TaoError::Eof) => {
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

fn decode_vorbis_with_lewton(
    path: &str,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    if is_url(path) {
        return Err("lewton 对比暂不支持 URL 输入".into());
    }
    let f = std::fs::File::open(path)?;
    let mut rdr = OggStreamReader::new(f)?;
    let sr = rdr.ident_hdr.audio_sample_rate;
    let ch = u32::from(rdr.ident_hdr.audio_channels);
    let channels = ch as usize;
    let mut out = Vec::<f32>::new();
    while let Some(pkt) = rdr.read_dec_packet_itl()? {
        // lewton 输出 i16 交错样本, 转为 f32.
        out.extend(pkt.into_iter().map(|v| v as f32 / 32768.0));
    }
    if channels == 0 {
        return Err("lewton 解码得到 0 声道".into());
    }
    Ok((sr, ch, out))
}

struct CompareStats {
    n: usize,
    max_err: f64,
    psnr: f64,
    precision_pct: f64,
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
            };
        }
        let n = reference.len().min(test.len() - s);
        return compare_pcm(&reference[..n], &test[s..s + n]);
    }

    let s = (-shift) as usize;
    if s >= reference.len() {
        return CompareStats {
            n: 0,
            max_err: 0.0,
            psnr: f64::INFINITY,
            precision_pct: 0.0,
        };
    }
    let n = test.len().min(reference.len() - s);
    compare_pcm(&reference[s..s + n], &test[..n])
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

fn estimate_gain_and_corr(reference: &[f32], test: &[f32]) -> (f64, f64, f64, f64) {
    let n = reference.len().min(test.len());
    if n == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mut rr = 0.0f64;
    let mut tt = 0.0f64;
    let mut rt = 0.0f64;
    for i in 0..n {
        let r = reference[i] as f64;
        let t = test[i] as f64;
        rr += r * r;
        tt += t * t;
        rt += r * t;
    }
    let gain = if tt > 0.0 { rt / tt } else { 0.0 };
    let corr = if rr > 0.0 && tt > 0.0 {
        rt / (rr.sqrt() * tt.sqrt())
    } else {
        0.0
    };
    let rms_r = (rr / n as f64).sqrt();
    let rms_t = (tt / n as f64).sqrt();
    (gain, corr, rms_r, rms_t)
}

fn deinterleave_channels(data: &[f32], channels: usize) -> Vec<Vec<f32>> {
    if channels == 0 {
        return Vec::new();
    }
    let samples_per_ch = data.len() / channels;
    let mut out = vec![vec![0.0f32; samples_per_ch]; channels];
    for s in 0..samples_per_ch {
        for ch in 0..channels {
            out[ch][s] = data[s * channels + ch];
        }
    }
    out
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
    Ok("data/1.ogg".to_string())
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();

    let (tao_sr, tao_ch, tao_pcm, tao_stream_index) = decode_vorbis_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_vorbis_with_ffmpeg(path, tao_stream_index)?;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    let stats_tao = compare_pcm(&ff_pcm, &tao_pcm);
    let preview = ff_pcm.len().min(8).min(tao_pcm.len());
    if preview > 0 {
        info!(
            "[{}] 首样本预览: ff={:?}, tao={:?}",
            path,
            &ff_pcm[..preview],
            &tao_pcm[..preview]
        );
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
    let (gain, corr, ff_rms, tao_rms) = estimate_gain_and_corr(&ff_pcm, &tao_pcm);
    info!(
        "[{}] 幅度诊断: gain={:.6}, corr={:.6}, ff_rms={:.6}, tao_rms={:.6}",
        path, gain, corr, ff_rms, tao_rms
    );
    if ff_pcm.len() > 200_000 && tao_pcm.len() > 200_000 {
        let start = 100_000usize;
        let len = 100_000usize;
        let ff_mid = &ff_pcm[start..start + len];
        let tao_mid = &tao_pcm[start..start + len];
        let mid = compare_pcm(ff_mid, tao_mid);
        let (_, mid_corr, ff_mid_rms, tao_mid_rms) = estimate_gain_and_corr(ff_mid, tao_mid);
        info!(
            "[{}] 中段诊断: 精度={:.2}%, corr={:.6}, ff_rms={:.6}, tao_rms={:.6}",
            path, mid.precision_pct, mid_corr, ff_mid_rms, tao_mid_rms
        );
    }

    if let Ok((lw_sr, lw_ch, lw_pcm)) = decode_vorbis_with_lewton(path)
        && lw_sr == ff_sr
        && lw_ch == ff_ch
    {
        let lw_vs_ff = compare_pcm(&ff_pcm, &lw_pcm);
        let tao_vs_lw = compare_pcm(&lw_pcm, &tao_pcm);
        info!(
            "[{}] 三方对比: lewton/FFmpeg 精度={:.2}%, Tao/lewton 精度={:.2}%",
            path, lw_vs_ff.precision_pct, tao_vs_lw.precision_pct
        );
    }

    if tao_ch == 2 {
        let ff_chs = deinterleave_channels(&ff_pcm, 2);
        let tao_chs = deinterleave_channels(&tao_pcm, 2);
        if ff_chs.len() == 2 && tao_chs.len() == 2 {
            let ll = compare_pcm(&ff_chs[0], &tao_chs[0]);
            let rr = compare_pcm(&ff_chs[1], &tao_chs[1]);
            let lr = compare_pcm(&ff_chs[0], &tao_chs[1]);
            let rl = compare_pcm(&ff_chs[1], &tao_chs[0]);
            info!(
                "[{}] 声道诊断: L-L={:.2}%, R-R={:.2}%, L-R={:.2}%, R-L={:.2}%, Tao-L/R={:.2}%",
                path,
                ll.precision_pct,
                rr.precision_pct,
                lr.precision_pct,
                rl.precision_pct,
                compare_pcm(&tao_chs[0], &tao_chs[1]).precision_pct
            );
        }
    }

    // 诊断: 固定样本偏移是否导致大误差.
    let mut best_shift = 0isize;
    let mut best_prec = stats_tao.precision_pct;
    for s in [64isize, 128, 256, 512, 1024, 2048, 4096] {
        for sign in [-1isize, 1isize] {
            let shift = sign * s * tao_ch as isize;
            let st = compare_pcm_with_shift(&ff_pcm, &tao_pcm, shift);
            if st.n > 0 && st.precision_pct > best_prec {
                best_prec = st.precision_pct;
                best_shift = shift;
            }
        }
    }
    info!(
        "[{}] 偏移诊断: best_shift={}, best_precision={:.2}%",
        path, best_shift, best_prec
    );

    assert!(stats_tao.n > 0, "无可比较样本");
    Ok(())
}

#[test]
#[ignore]
fn test_vorbis_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("Vorbis 对比失败");
}
