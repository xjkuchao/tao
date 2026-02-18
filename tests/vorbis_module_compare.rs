//! Vorbis 解码精度对比测试.
//!
//! 当前阶段输出 Tao 与 FFmpeg 的误差统计, 用于持续收敛。

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

fn decode_vorbis_with_tao(path: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
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
        .find(|s| s.codec_id == CodecId::Vorbis)
        .ok_or("未找到 Vorbis 音频流")?
        .clone();

    let (sample_rate, channel_layout) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => (a.sample_rate, a.channel_layout),
        _ => (44100, ChannelLayout::STEREO),
    };

    let params = CodecParameters {
        codec_id: CodecId::Vorbis,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format: SampleFormat::F32,
            frame_size: 0,
        }),
    };

    let mut decoder = codec_registry.create_decoder(CodecId::Vorbis)?;
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
                Err(TaoError::NeedMoreData) => break,
                Err(TaoError::Eof) => return Ok((actual_sr, actual_ch, out)),
                Err(e) => return Err(format!("取帧失败: {}", e).into()),
            }
        }
    }
}

fn decode_vorbis_with_ffmpeg(
    path: &str,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let tmp = make_ffmpeg_tmp_path("vorbis_cmp");
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

fn decode_vorbis_with_lewton(
    path: &str,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut reader = OggStreamReader::new(file)?;
    let sr = reader.ident_hdr.audio_sample_rate;
    let ch = reader.ident_hdr.audio_channels as u32;

    let mut out = Vec::<f32>::new();
    loop {
        match reader.read_dec_packet_itl()? {
            Some(pkt) => {
                out.extend(pkt.into_iter().map(|v| v as f32 / 32768.0));
            }
            None => break,
        }
    }

    Ok((sr, ch, out))
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

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    init_test_tracing();
    let (tao_sr, tao_ch, tao_pcm) = decode_vorbis_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_vorbis_with_ffmpeg(path)?;
    let (lewton_sr, lewton_ch, lewton_pcm) = decode_vorbis_with_lewton(path)?;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");
    assert_eq!(lewton_sr, ff_sr, "Lewton 采样率不匹配");
    assert_eq!(lewton_ch, ff_ch, "Lewton 通道数不匹配");

    let stats_tao = compare_pcm(&ff_pcm, &tao_pcm);
    let stats_lewton = compare_pcm(&ff_pcm, &lewton_pcm);
    info!(
        "[{}] Tao对比样本={}, Lewton对比样本={}, Tao={}, FFmpeg={}, Lewton={}, \
Tao/FFmpeg: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%, \
Lewton/FFmpeg: max_err={:.6}, psnr={:.2}dB, 精度={:.2}%, FFmpeg=100%",
        path,
        stats_tao.n,
        stats_lewton.n,
        tao_pcm.len(),
        ff_pcm.len(),
        lewton_pcm.len(),
        stats_tao.max_err,
        stats_tao.psnr,
        stats_tao.precision_pct,
        stats_lewton.max_err,
        stats_lewton.psnr,
        stats_lewton.precision_pct
    );

    assert!(stats_tao.n > 0, "无可比较样本");
    assert!(stats_lewton.n > 0, "Lewton 无可比较样本");
    Ok(())
}

#[test]
#[ignore]
fn test_vorbis_compare_data1() {
    run_compare("data/1.ogg").expect("data/1.ogg 对比失败");
}

// 仅保留 data/1.ogg 对比, data/2.ogg 暂不在自动测试中执行.
