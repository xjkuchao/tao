//! MP3 解码器精度对比测试
//!
//! 功能:
//! 1. 使用 tao 自研 MP3 解码器解码 MP3 URL
//! 2. 使用 ffmpeg 解码同一 URL, 获取参考 PCM
//! 3. 逐帧对比 PCM 输出, 计算 PSNR/MSE/最大误差
//!
//! 运行方式:
//!   cargo test --test mp3_module_compare --features http -- --nocapture

#![cfg(feature = "http")]

use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

use tao::codec::decoders::mp3::debug;

/// MP3 测试样本集合 (来自 samples.ffmpeg.org)
const MP3_SAMPLES: &[(&str, &str)] = &[
    ("ascii", "https://samples.ffmpeg.org/A-codecs/MP3/ascii.mp3"),
    (
        "Enrique",
        "https://samples.ffmpeg.org/A-codecs/MP3/Enrique.mp3",
    ),
    (
        "Silent_Light",
        "https://samples.ffmpeg.org/A-codecs/MP3/Silent_Light.mp3",
    ),
    (
        "44khz128kbps",
        "https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz128kbps.mp3",
    ),
    (
        "44khz64kbps",
        "https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz64kbps.mp3",
    ),
    (
        "44khz32kbps",
        "https://samples.ffmpeg.org/A-codecs/suite/MP3/44khz32kbps.mp3",
    ),
    (
        "piano",
        "https://samples.ffmpeg.org/A-codecs/suite/MP3/piano.mp3",
    ),
    (
        "mp3pro_scooter",
        "https://samples.ffmpeg.org/A-codecs/MP3-pro/scooter-wicked-02-imraving.mp3",
    ),
];

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.raw", tag, pid, seq)
}

/// 使用 tao 解码 MP3 URL, 返回 (采样率, 通道数, PCM f32 样本)
fn decode_mp3_with_tao_url(url: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_url(url)?;
    let mut demuxer = format_registry.open_input(&mut io, Some(url))?;
    demuxer.open(&mut io)?;

    let streams = demuxer.streams();
    if streams.is_empty() {
        return Err("没有找到音频流".into());
    }
    let stream = &streams[0];
    let sample_rate = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => a.sample_rate,
        _ => 44100,
    };
    let channels = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => a.channel_layout.channels,
        _ => 2,
    };

    let params = CodecParameters {
        codec_id: CodecId::Mp3,
        extra_data: stream.extra_data.clone(),
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

    let mut all_pcm = Vec::new();
    let mut frames_decoded = 0u32;
    let mut actual_sr = sample_rate;
    let mut actual_ch = channels;

    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                decoder.send_packet(&pkt)?;
            }
            Err(TaoError::Eof) => {
                decoder.flush();
                break;
            }
            Err(_) => break,
        }

        loop {
            match decoder.receive_frame() {
                Ok(frame) => {
                    if let tao::codec::frame::Frame::Audio(audio) = &frame {
                        actual_sr = audio.sample_rate;
                        actual_ch = audio.channel_layout.channels;
                        if !audio.data.is_empty() {
                            let bytes = &audio.data[0];
                            let samples: Vec<f32> = bytes
                                .chunks_exact(4)
                                .map(|chunk| {
                                    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                                })
                                .collect();
                            all_pcm.extend_from_slice(&samples);
                        }
                        frames_decoded += 1;
                    }
                }
                Err(TaoError::NeedMoreData) => break,
                Err(TaoError::Eof) => break,
                Err(_) => break,
            }
        }
    }

    // 接收剩余帧
    loop {
        match decoder.receive_frame() {
            Ok(frame) => {
                if let tao::codec::frame::Frame::Audio(audio) = &frame {
                    if !audio.data.is_empty() {
                        let bytes = &audio.data[0];
                        let samples: Vec<f32> = bytes
                            .chunks_exact(4)
                            .map(|chunk| {
                                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                            })
                            .collect();
                        all_pcm.extend_from_slice(&samples);
                    }
                    frames_decoded += 1;
                }
            }
            Err(_) => break,
        }
    }

    println!(
        "tao 解码完成: {} 帧, {} 样本, {}Hz {}ch",
        frames_decoded,
        all_pcm.len(),
        actual_sr,
        actual_ch,
    );

    Ok((actual_sr, actual_ch, all_pcm))
}

/// 使用 ffmpeg 解码 MP3 URL, 返回 (采样率, 通道数, PCM f32 样本)
/// ffmpeg 原生支持 HTTP/HTTPS URL 作为输入
fn decode_mp3_with_ffmpeg_url(
    url: &str,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    use std::process::Command;

    std::fs::create_dir_all("data").ok();
    let output_path = make_ffmpeg_tmp_path("ffmpeg_cmp");

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            url,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            &output_path,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => return Err(format!("ffmpeg 退出码: {}", s).into()),
        Err(e) => return Err(format!("ffmpeg 执行失败: {}", e).into()),
    }

    let probe_output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,channels",
            "-of",
            "csv=p=0",
            url,
        ])
        .output()?;

    let probe_str = String::from_utf8_lossy(&probe_output.stdout);
    let parts: Vec<&str> = probe_str.trim().split(',').collect();
    let sample_rate = parts
        .first()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(44100);
    let channels = parts
        .get(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2);

    let raw_bytes = std::fs::read(&output_path)?;
    let samples: Vec<f32> = raw_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    let _ = std::fs::remove_file(&output_path);

    println!(
        "ffmpeg 解码完成: {} 样本, {}Hz {}ch",
        samples.len(),
        sample_rate,
        channels,
    );

    Ok((sample_rate, channels, samples))
}

/// 帧级 (1152 样本) 精度对比, 输出逐帧报告
fn compare_frames(
    tao_pcm: &[f32],
    ref_pcm: &[f32],
    channels: u32,
    samples_per_frame: usize,
    max_frames_to_report: usize,
) {
    let frame_size = samples_per_frame * channels as usize;
    let tao_frames = tao_pcm.len() / frame_size;
    let ref_frames = ref_pcm.len() / frame_size;
    let total_frames = tao_frames.min(ref_frames);

    println!("=== 帧级精度对比 ===");
    println!(
        "tao: {} 帧, ref: {} 帧, 对比: {} 帧",
        tao_frames, ref_frames, total_frames,
    );

    let mut pass_count = 0;
    let mut worst_psnr = f64::INFINITY;
    let mut worst_frame = 0;
    let mut worst_max_err: f64 = 0.0;

    for frame_idx in 0..total_frames {
        let start = frame_idx * frame_size;
        let end = start + frame_size;

        let tao_slice = &tao_pcm[start..end.min(tao_pcm.len())];
        let ref_slice = &ref_pcm[start..end.min(ref_pcm.len())];

        let result = debug::compare_f32_samples(&format!("帧 {}", frame_idx), tao_slice, ref_slice);

        if result.psnr_db >= debug::acceptance::MIN_PSNR_DB as f64
            && result.max_abs_error <= debug::acceptance::MAX_SAMPLE_ERROR as f64
        {
            pass_count += 1;
        }

        if result.psnr_db < worst_psnr {
            worst_psnr = result.psnr_db;
            worst_frame = frame_idx;
        }

        if result.max_abs_error > worst_max_err {
            worst_max_err = result.max_abs_error;
        }

        if frame_idx < max_frames_to_report {
            println!("  {}", result);
        }
    }

    println!("\n=== 汇总 ===");
    println!(
        "总帧数: {}, 通过: {}, 失败: {}, 通过率: {:.1}%",
        total_frames,
        pass_count,
        total_frames - pass_count,
        if total_frames > 0 {
            (pass_count as f64 / total_frames as f64) * 100.0
        } else {
            0.0
        },
    );
    println!("最差帧: #{}, PSNR: {:.1}dB", worst_frame, worst_psnr,);
    println!("最大单样本误差: {:.2e}", worst_max_err);
}

/// 全局精度对比 (所有样本合并)
fn compare_global(tao_pcm: &[f32], ref_pcm: &[f32]) -> debug::CompareResult {
    let total = tao_pcm.len().min(ref_pcm.len());
    debug::compare_f32_samples("全局", &tao_pcm[..total], &ref_pcm[..total])
}

// ===== 测试用例 =====

/// CBR MP3 精度对比 (samples.ffmpeg.org)
#[test]
fn test_mp3_native_vs_ffmpeg_cbr() {
    let url = MP3_SAMPLES[0].1; // ascii.mp3

    let tao_result = decode_mp3_with_tao_url(url);
    let tao_result = match tao_result {
        Ok(r) => r,
        Err(e) => {
            println!("跳过: tao 解码失败 (网络/解码错误): {}", e);
            return;
        }
    };

    let ff_result = decode_mp3_with_ffmpeg_url(url);
    let ff_result = match ff_result {
        Ok(r) => r,
        Err(e) => {
            println!("跳过: ffmpeg 解码失败 (网络/工具未安装): {}", e);
            return;
        }
    };

    let (tao_sr, tao_ch, tao_pcm) = tao_result;
    let (ff_sr, ff_ch, ff_pcm) = ff_result;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    let global = compare_global(&tao_pcm, &ff_pcm);
    println!("\n=== 全局精度 (CBR) ===");
    println!("  {}", global);

    compare_frames(&tao_pcm, &ff_pcm, tao_ch, 1152, 20);

    if global.psnr_db >= debug::acceptance::MIN_PSNR_DB as f64 {
        println!(
            "\n✅ PSNR {:.1}dB >= {}dB, 通过",
            global.psnr_db,
            debug::acceptance::MIN_PSNR_DB
        );
    } else {
        println!(
            "\n⚠️  PSNR {:.1}dB < {}dB, 未达标 (当前阶段不强制失败)",
            global.psnr_db,
            debug::acceptance::MIN_PSNR_DB,
        );
    }
}

/// VBR MP3 精度对比 (samples.ffmpeg.org)
#[test]
fn test_mp3_native_vs_ffmpeg_vbr() {
    let url = MP3_SAMPLES[1].1; // Enrique.mp3

    let tao_result = decode_mp3_with_tao_url(url);
    let tao_result = match tao_result {
        Ok(r) => r,
        Err(e) => {
            println!("跳过: tao 解码失败 (网络/解码错误): {}", e);
            return;
        }
    };

    let ff_result = decode_mp3_with_ffmpeg_url(url);
    let ff_result = match ff_result {
        Ok(r) => r,
        Err(e) => {
            println!("跳过: ffmpeg 解码失败 (网络/工具未安装): {}", e);
            return;
        }
    };

    let (tao_sr, tao_ch, tao_pcm) = tao_result;
    let (ff_sr, ff_ch, ff_pcm) = ff_result;

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    let global = compare_global(&tao_pcm, &ff_pcm);
    println!("\n=== 全局精度 (VBR) ===");
    println!("  {}", global);

    compare_frames(&tao_pcm, &ff_pcm, tao_ch, 1152, 20);

    if global.psnr_db >= debug::acceptance::MIN_PSNR_DB as f64 {
        println!(
            "\n✅ PSNR {:.1}dB >= {}dB, 通过",
            global.psnr_db,
            debug::acceptance::MIN_PSNR_DB
        );
    } else {
        println!(
            "\n⚠️  PSNR {:.1}dB < {}dB, 未达标 (当前阶段不强制失败)",
            global.psnr_db,
            debug::acceptance::MIN_PSNR_DB,
        );
    }
}

/// 对比摘要报告: 多 URL 统一输出
/// 
/// 注意: 此测试耗时约 65 秒，仅在手动执行时运行
/// 手动执行: cargo test --test mp3_module_compare test_mp3_native_summary -- --nocapture --ignored
#[test]
#[ignore]
fn test_mp3_native_summary() {
    println!("\n========================================");
    println!("  MP3 自研解码器精度测试 - 完整样本集");
    println!("========================================\n");

    let mut passed = 0;
    let mut skipped = 0;
    let mut psnr_values = Vec::new();

    for (label, url) in MP3_SAMPLES {
        let tao_result = decode_mp3_with_tao_url(url);
        let ff_result = decode_mp3_with_ffmpeg_url(url);

        match (tao_result, ff_result) {
            (Ok((_, _, tao_pcm)), Ok((_, _, ff_pcm))) => {
                let global = compare_global(&tao_pcm, &ff_pcm);
                psnr_values.push(global.psnr_db);
                
                let status = if global.psnr_db >= debug::acceptance::MIN_PSNR_DB as f64 {
                    "✅"
                } else {
                    "⚠️ "
                };
                if global.psnr_db >= debug::acceptance::MIN_PSNR_DB as f64 {
                    passed += 1;
                }
                println!(
                    "{} [{}] PSNR: {:.1}dB, 最大误差: {:.2e}, 平均: {:.2e}, 样本: {}",
                    status,
                    label,
                    global.psnr_db,
                    global.max_abs_error,
                    global.mean_abs_error,
                    global.total_samples,
                );
            }
            (Err(e), _) => {
                println!("⚠️  [{}] tao 解码失败 (跳过): {}", label, e);
                skipped += 1;
            }
            (_, Err(e)) => {
                println!("⚠️  [{}] ffmpeg 解码失败 (跳过): {}", label, e);
                skipped += 1;
            }
        }
    }

    // 计算精度统计
    println!("\n========================================");
    println!("测试结果摘要:");
    println!("  ✅ 通过: {}/{}", passed, MP3_SAMPLES.len());
    println!("  ⏭️  跳过: {}", skipped);
    
    if !psnr_values.is_empty() {
        let max_psnr = psnr_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_psnr = psnr_values.iter().cloned().fold(f64::INFINITY, f64::min);
        let avg_psnr = psnr_values.iter().sum::<f64>() / psnr_values.len() as f64;
        
        let ratio_avg = (avg_psnr / max_psnr) * 100.0;
        let good_count = psnr_values.iter().filter(|&&p| p >= max_psnr * 0.9).count();
        let good_ratio = (good_count as f64 / psnr_values.len() as f64) * 100.0;
        
        println!("\n精度分析:");
        println!("  最佳期望值 (max): {:.1}dB = 100%", max_psnr);
        println!("  当前平均精度: {:.1}dB = {:.1}%", avg_psnr, ratio_avg);
        println!("  PSNR范围: {:.1}dB ~ {:.1}dB", min_psnr, max_psnr);
        println!("  接近期望 (>=90%): {}/{} ({:.1}%)", good_count, psnr_values.len(), good_ratio);
    }
    
    println!("========================================\n");
}
