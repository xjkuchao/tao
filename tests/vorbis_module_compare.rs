//! Vorbis 解码精度对比测试.
//!
//! 当前阶段输出 Tao 与 FFmpeg 的误差统计, 用于持续收敛。

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::frame::Frame;
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.raw", tag, pid, seq)
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

fn compare_pcm(a: &[f32], b: &[f32]) -> (usize, f64, f64) {
    let n = a.len().min(b.len());
    if n == 0 {
        return (0, 0.0, f64::INFINITY);
    }
    let mut mse = 0.0f64;
    let mut max_err = 0.0f64;
    for i in 0..n {
        let d = (a[i] - b[i]) as f64;
        let ad = d.abs();
        max_err = max_err.max(ad);
        mse += d * d;
    }
    mse /= n as f64;
    let psnr = if mse > 0.0 {
        20.0 * (1.0 / mse.sqrt()).log10()
    } else {
        f64::INFINITY
    };
    (n, max_err, psnr)
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (tao_sr, tao_ch, tao_pcm) = decode_vorbis_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_vorbis_with_ffmpeg(path)?;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    let (n, max_err, psnr) = compare_pcm(&tao_pcm, &ff_pcm);
    println!(
        "[{}] 样本={}, Tao={}, FFmpeg={}, max_err={:.6}, psnr={:.2}dB",
        path,
        n,
        tao_pcm.len(),
        ff_pcm.len(),
        max_err,
        psnr
    );

    assert!(n > 0, "无可比较样本");
    Ok(())
}

#[test]
fn test_vorbis_compare_data1() {
    run_compare("data/1.ogg").expect("data/1.ogg 对比失败");
}

#[test]
fn test_vorbis_compare_data2() {
    run_compare("data/2.ogg").expect("data/2.ogg 对比失败");
}
