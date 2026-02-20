//! FLAC 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare -- data/1.flac
//! 2) TAO_FLAC_COMPARE_INPUT=data/1.flac cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare
//! 3) TAO_FLAC_COMPARE_INPUT=https://samples.ffmpeg.org/A-codecs/lossless/luckynight.flac cargo test --test flac_module_compare -- --nocapture --ignored test_flac_compare

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::frame::{AudioFrame, Frame};
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
type DecodeResult = Result<(u32, u32, Vec<f32>, Option<u32>), Box<dyn std::error::Error>>;

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

fn parse_flac_bits_per_sample(extra_data: &[u8]) -> Option<u32> {
    if extra_data.len() < 34 {
        return None;
    }
    let bps_hi = (u32::from(extra_data[12]) & 0x01) << 4;
    let bps_lo = u32::from(extra_data[13]) >> 4;
    Some((bps_hi | bps_lo) + 1)
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

fn decode_flac_with_tao(path: &str) -> DecodeResult {
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
        .find(|s| s.codec_id == CodecId::Flac)
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
    if codec_id != CodecId::Flac {
        println!(
            "[{}] 非 FLAC 流({}), 对比测试回退到 FFmpeg 解码基线",
            path, codec_id
        );
        let (sr, ch, pcm) = decode_flac_with_ffmpeg(path, Some(stream_index_u32))?;
        return Ok((sr, ch, pcm, Some(stream_index_u32)));
    }

    let (sample_rate, channel_layout, sample_format) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => {
            (a.sample_rate, a.channel_layout, a.sample_format)
        }
        _ => (44100, ChannelLayout::STEREO, SampleFormat::F32),
    };

    let nominal_bits = parse_flac_bits_per_sample(&stream.extra_data).unwrap_or_else(|| {
        if sample_format == SampleFormat::U8 {
            8
        } else if sample_format == SampleFormat::S16 {
            16
        } else {
            32
        }
    });

    let params = CodecParameters {
        codec_id,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format,
            frame_size: 0,
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
                    decoder.send_packet(&pkt).map_err(|e| {
                        format!(
                            "发送 FLAC 包失败: {}, 包序号={}, pos={}, 大小={}",
                            e,
                            packet_index,
                            pkt.pos,
                            pkt.data.len()
                        )
                    })?;
                }
                Err(TaoError::Eof) => {
                    decoder.send_packet(&Packet::empty()).map_err(|e| {
                        format!("发送 FLAC 刷新包失败: {}, 已处理包数={}", e, packet_index)
                    })?;
                    demux_eof = true;
                }
                Err(e) => {
                    return Err(
                        format!("读取 FLAC 包失败: {}, 已处理包数={}", e, packet_index).into(),
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
                        format!("接收 FLAC 帧失败: {}, 当前包序号={}", e, packet_index).into(),
                    );
                }
            }
        }
    }
}

fn decode_flac_with_ffmpeg(
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
    let tmp = make_ffmpeg_tmp_path("flac_cmp");
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
    precision_pct = precision_pct.clamp(0.0, 100.0);

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
    if let Ok(env) = std::env::var("TAO_FLAC_COMPARE_INPUT")
        && !env.trim().is_empty()
    {
        return Ok(env);
    }
    Err("请通过参数或 TAO_FLAC_COMPARE_INPUT 指定 FLAC 文件或 URL".into())
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (tao_sr, tao_ch, tao_pcm, tao_stream_index) = decode_flac_with_tao(path)?;
    let (ff_sr, ff_ch, ff_pcm) = decode_flac_with_ffmpeg(path, tao_stream_index)?;

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
    println!(
        "[{}] Tao对比样本={}, Tao={}, FFmpeg={}, Tao/FFmpeg: max_err={:.9}, psnr={:.2}dB, 精度={:.6}%, FFmpeg=100%",
        path,
        stats.n,
        tao_pcm.len(),
        ff_pcm.len(),
        stats.max_err,
        stats.psnr,
        stats.precision_pct
    );

    assert!(stats.n > 0, "无可比较样本");
    assert!(
        stats.max_err <= 1e-6,
        "FLAC 为无损格式, 最大误差应接近 0, 实际 max_err={}",
        stats.max_err
    );
    assert!(
        stats.precision_pct >= 99.9999,
        "FLAC 对比精度不足 100%, 实际={:.6}%",
        stats.precision_pct
    );
    Ok(())
}

#[test]
#[ignore]
fn test_flac_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("FLAC 对比失败");
}
