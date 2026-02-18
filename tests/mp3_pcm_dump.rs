//! MP3 解码器 PCM 导出与对比测试
//!
//! 功能:
//! 1. 使用 tao MP3 解码器解码 MP3 文件, 导出 PCM 数据
//! 2. 使用 ffmpeg / symphonia 解码同一文件, 导出参考 PCM 数据
//! 3. 逐帧对比两者输出, 报告差异
//!
//! 测试文件: data/1.mp3, data/2.mp3

use std::path::Path;
use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use minimp3::{Decoder as MiniDecoder, Error as MiniError};

/// 使用 tao 解码 MP3 文件, 返回 (采样率, 通道数, PCM f32 样本)
fn decode_mp3_with_tao(path: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    // 初始化注册表
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    // 打开文件
    let mut io = IoContext::open_read(path)?;

    // 自动探测格式并打开
    let mut demuxer = format_registry.open_input(&mut io, Some(path))?;
    demuxer.open(&mut io)?;

    // 获取流信息
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

    // 创建 MP3 解码器
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
    let mut frames_decoded = 0;
    let mut actual_sr = sample_rate;
    let mut actual_ch = channels;

    // 解码循环
    loop {
        // 读取数据包
        let packet = demuxer.read_packet(&mut io);
        match packet {
            Ok(pkt) => {
                decoder.send_packet(&pkt)?;
            }
            Err(TaoError::Eof) => {
                // 发送 flush
                decoder.flush();
                break;
            }
            Err(e) => {
                eprintln!("读包错误: {}", e);
                break;
            }
        }

        // 接收解码帧
        loop {
            match decoder.receive_frame() {
                Ok(frame) => {
                    if let tao::codec::frame::Frame::Audio(audio) = &frame {
                        actual_sr = audio.sample_rate;
                        actual_ch = audio.channel_layout.channels;

                        // 提取 F32 PCM 数据
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
                Err(e) => {
                    eprintln!("解码错误 (帧 {}): {}", frames_decoded, e);
                    break;
                }
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

    eprintln!(
        "tao 解码完成: {} 帧, {} 样本, {}Hz {}ch",
        frames_decoded,
        all_pcm.len(),
        actual_sr,
        actual_ch,
    );

    Ok((actual_sr, actual_ch, all_pcm))
}

/// 使用 ffmpeg 解码 MP3 文件, 返回 (采样率, 通道数, PCM f32 样本)
fn decode_mp3_with_ffmpeg(path: &str) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    use std::process::Command;

    let output_path = format!("{}.ffmpeg.raw", path);

    // ffmpeg -i input.mp3 -f f32le -acodec pcm_f32le output.raw
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            path,
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

    // 获取 ffmpeg 输出的采样率和通道数
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
            path,
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

    // 读取 PCM 数据
    let raw_bytes = std::fs::read(&output_path)?;
    let samples: Vec<f32> = raw_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    // 清理临时文件
    let _ = std::fs::remove_file(&output_path);

    eprintln!(
        "ffmpeg 解码完成: {} 样本, {}Hz {}ch",
        samples.len(),
        sample_rate,
        channels,
    );

    Ok((sample_rate, channels, samples))
}

/// 逐帧对比两个 PCM 数据
/// 返回 (匹配帧数, 总帧数, 最大误差, 平均误差)
fn compare_pcm(
    tao_pcm: &[f32],
    ffmpeg_pcm: &[f32],
    channels: u32,
    samples_per_frame: usize,
) -> (usize, usize, f32, f32) {
    let frame_size = samples_per_frame * channels as usize;
    let tao_frames = tao_pcm.len() / frame_size;
    let ffmpeg_frames = ffmpeg_pcm.len() / frame_size;
    let total_frames = tao_frames.min(ffmpeg_frames);

    let mut matched = 0;
    let mut max_err = 0.0f32;
    let mut sum_err = 0.0f64;
    let mut sample_count = 0usize;

    // 阈值: 允许的最大单样本误差
    let threshold = 0.01f32;

    for frame in 0..total_frames {
        let start = frame * frame_size;
        let end = start + frame_size;

        let tao_slice = &tao_pcm[start..end.min(tao_pcm.len())];
        let ffmpeg_slice = &ffmpeg_pcm[start..end.min(ffmpeg_pcm.len())];

        let mut frame_max_err = 0.0f32;
        for (t, f) in tao_slice.iter().zip(ffmpeg_slice.iter()) {
            let err = (t - f).abs();
            frame_max_err = frame_max_err.max(err);
            max_err = max_err.max(err);
            sum_err += err as f64;
            sample_count += 1;
        }

        if frame_max_err < threshold {
            matched += 1;
        } else if frame < 10 || frame % 100 == 0 {
            eprintln!(
                "帧 {}: 最大误差 {:.6}, 首差异位置 {}",
                frame,
                frame_max_err,
                tao_slice
                    .iter()
                    .zip(ffmpeg_slice.iter())
                    .position(|(t, f)| (t - f).abs() >= threshold)
                    .unwrap_or(0),
            );
        }
    }

    let avg_err = if sample_count > 0 {
        (sum_err / sample_count as f64) as f32
    } else {
        0.0
    };

    (matched, total_frames, max_err, avg_err)
}

/// 使用 symphonia 解码 MP3 文件, 返回 (采样率, 通道数, PCM f32 交织样本)
fn decode_mp3_with_symphonia(
    path: &str,
) -> Result<(u32, u32, Vec<f32>), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    let track = format.default_track().unwrap().clone();
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u32)
        .unwrap_or(2);

    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut all_pcm: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let dur = decoded.capacity();
                let mut sample_buf = SampleBuffer::<f32>::new(dur as u64, spec);
                sample_buf.copy_interleaved_ref(decoded);
                all_pcm.extend_from_slice(sample_buf.samples());
            }
            Err(_) => continue,
        }
    }

    eprintln!(
        "symphonia 解码完成: {} 样本, {}Hz {}ch",
        all_pcm.len(),
        sample_rate,
        channels
    );
    Ok((sample_rate, channels, all_pcm))
}

/// 使用 minimp3 解码 MP3 文件, 返回 f32 交织样本
fn decode_mp3_with_minimp3(path: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = MiniDecoder::new(file);
    let mut all_pcm: Vec<f32> = Vec::new();
    let mut frames = 0;

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                // minimp3 输出 i16 样本, 转为 f32
                for &sample in &frame.data {
                    all_pcm.push(sample as f32 / 32768.0);
                }
                frames += 1;
            }
            Err(MiniError::Eof) => break,
            Err(e) => return Err(format!("minimp3 错误: {:?}", e).into()),
        }
    }

    eprintln!("minimp3 解码完成: {} 帧, {} 样本", frames, all_pcm.len());
    Ok(all_pcm)
}

/// 将 PCM 数据保存为 raw 文件 (用于调试)
fn save_pcm_raw(pcm: &[f32], path: &str) -> std::io::Result<()> {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(path, bytes)
}

#[test]
fn test_mp3_decode_file1() {
    let path = "data/1.mp3";
    if !Path::new(path).exists() {
        eprintln!("跳过: {} 不存在", path);
        return;
    }

    let result = decode_mp3_with_tao(path);
    match result {
        Ok((sr, ch, pcm)) => {
            eprintln!("成功解码 {}: {}Hz {}ch {} 样本", path, sr, ch, pcm.len());
            assert!(!pcm.is_empty(), "PCM 输出不应为空");
            assert!(sr > 0, "采样率应大于 0");

            // 保存 PCM 用于调试
            let _ = save_pcm_raw(&pcm, "data/1.mp3.tao.raw");

            // 检查 PCM 数据不是全零
            let non_zero = pcm.iter().filter(|&&s| s != 0.0).count();
            eprintln!(
                "非零样本: {} / {} ({:.1}%)",
                non_zero,
                pcm.len(),
                non_zero as f64 / pcm.len() as f64 * 100.0
            );

            // 基本验证: 至少有一些非零样本
            assert!(
                non_zero > pcm.len() / 100,
                "PCM 数据几乎全为零, 非零样本比例过低: {:.1}%",
                non_zero as f64 / pcm.len() as f64 * 100.0,
            );
        }
        Err(e) => {
            panic!("解码 {} 失败: {}", path, e);
        }
    }
}

#[test]
fn test_mp3_decode_file2() {
    let path = "data/2.mp3";
    if !Path::new(path).exists() {
        eprintln!("跳过: {} 不存在", path);
        return;
    }

    let result = decode_mp3_with_tao(path);
    match result {
        Ok((sr, ch, pcm)) => {
            eprintln!("成功解码 {}: {}Hz {}ch {} 样本", path, sr, ch, pcm.len());
            assert!(!pcm.is_empty(), "PCM 输出不应为空");
            assert!(sr > 0, "采样率应大于 0");

            let _ = save_pcm_raw(&pcm, "data/2.mp3.tao.raw");

            let non_zero = pcm.iter().filter(|&&s| s != 0.0).count();
            eprintln!(
                "非零样本: {} / {} ({:.1}%)",
                non_zero,
                pcm.len(),
                non_zero as f64 / pcm.len() as f64 * 100.0
            );

            assert!(
                non_zero > pcm.len() / 100,
                "PCM 数据几乎全为零, 非零样本比例过低",
            );
        }
        Err(e) => {
            panic!("解码 {} 失败: {}", path, e);
        }
    }
}

#[test]
fn test_mp3_compare_ffmpeg_file1() {
    let path = "data/1.mp3";
    if !Path::new(path).exists() {
        eprintln!("跳过: {} 不存在", path);
        return;
    }

    // 检查 ffmpeg 是否可用
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("跳过: ffmpeg 不可用");
        return;
    }

    let (tao_sr, tao_ch, tao_pcm) = decode_mp3_with_tao(path).expect("tao 解码失败");
    let (ff_sr, ff_ch, ff_pcm) = decode_mp3_with_ffmpeg(path).expect("ffmpeg 解码失败");

    eprintln!("tao:    {}Hz {}ch {} 样本", tao_sr, tao_ch, tao_pcm.len());
    eprintln!("ffmpeg: {}Hz {}ch {} 样本", ff_sr, ff_ch, ff_pcm.len());

    // 基本参数应一致
    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    // 寻找最佳对齐: 使用文件中间50%区域, 搜索 tao 相对于 ffmpeg 的偏移
    let compare_len = 20000.min(ff_pcm.len() / 4);
    let ff_mid = ff_pcm.len() / 2; // 从文件中间开始对比

    // 粗搜索: 步长=2, 范围±10000
    let mut best_offset: i64 = 0;
    let mut best_err = f64::MAX;
    for offset_candidate in (-10000i64..=10000).step_by(2) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..compare_len {
            let ff_idx = ff_mid + i;
            let tao_idx = ff_idx as i64 + offset_candidate;
            if ff_idx < ff_pcm.len() && tao_idx >= 0 && (tao_idx as usize) < tao_pcm.len() {
                sum_err += (tao_pcm[tao_idx as usize] - ff_pcm[ff_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_err {
                best_err = avg;
                best_offset = offset_candidate;
            }
        }
    }
    // 精搜索: 步长=1, 范围 best±10
    let coarse_best = best_offset;
    for offset_candidate in (coarse_best - 10)..=(coarse_best + 10) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..compare_len {
            let ff_idx = ff_mid + i;
            let tao_idx = ff_idx as i64 + offset_candidate;
            if ff_idx < ff_pcm.len() && tao_idx >= 0 && (tao_idx as usize) < tao_pcm.len() {
                sum_err += (tao_pcm[tao_idx as usize] - ff_pcm[ff_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_err {
                best_err = avg;
                best_offset = offset_candidate;
            }
        }
    }
    eprintln!(
        "\n最佳对齐 (交织): tao 偏移 {} 样本, 平均误差: {:.6}",
        best_offset, best_err
    );

    // 分声道对齐搜索: 只看 L 声道 (偶数样本)
    let tao_l: Vec<f32> = tao_pcm.iter().step_by(2).copied().collect();
    let ff_l: Vec<f32> = ff_pcm.iter().step_by(2).copied().collect();
    let tao_r: Vec<f32> = tao_pcm.iter().skip(1).step_by(2).copied().collect();
    let ff_r: Vec<f32> = ff_pcm.iter().skip(1).step_by(2).copied().collect();

    let mono_mid = ff_l.len() / 2;
    let mono_compare = 10000.min(ff_l.len() / 4);

    // L 声道单独对齐
    let mut best_l_offset: i64 = 0;
    let mut best_l_err = f64::MAX;
    for offset_candidate in (-5000i64..=5000).step_by(1) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..mono_compare {
            let ff_idx = mono_mid + i;
            let tao_idx = ff_idx as i64 + offset_candidate;
            if ff_idx < ff_l.len() && tao_idx >= 0 && (tao_idx as usize) < tao_l.len() {
                sum_err += (tao_l[tao_idx as usize] - ff_l[ff_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_l_err {
                best_l_err = avg;
                best_l_offset = offset_candidate;
            }
        }
    }

    // R 声道单独对齐
    let mut best_r_offset: i64 = 0;
    let mut best_r_err = f64::MAX;
    for offset_candidate in (-5000i64..=5000).step_by(1) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..mono_compare {
            let ff_idx = mono_mid + i;
            let tao_idx = ff_idx as i64 + offset_candidate;
            if ff_idx < ff_r.len() && tao_idx >= 0 && (tao_idx as usize) < tao_r.len() {
                sum_err += (tao_r[tao_idx as usize] - ff_r[ff_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_r_err {
                best_r_err = avg;
                best_r_offset = offset_candidate;
            }
        }
    }

    // 尝试交叉: tao_L vs ff_R, tao_R vs ff_L
    let mut best_lr_offset: i64 = 0;
    let mut best_lr_err = f64::MAX;
    for offset_candidate in (-5000i64..=5000).step_by(1) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..mono_compare {
            let ff_idx = mono_mid + i;
            let tao_idx = ff_idx as i64 + offset_candidate;
            if ff_idx < ff_r.len() && tao_idx >= 0 && (tao_idx as usize) < tao_l.len() {
                sum_err += (tao_l[tao_idx as usize] - ff_r[ff_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_lr_err {
                best_lr_err = avg;
                best_lr_offset = offset_candidate;
            }
        }
    }

    eprintln!(
        "L声道对齐: offset={}, avg_err={:.6}",
        best_l_offset, best_l_err
    );
    eprintln!(
        "R声道对齐: offset={}, avg_err={:.6}",
        best_r_offset, best_r_err
    );
    eprintln!(
        "tao_L vs ff_R: offset={}, avg_err={:.6}",
        best_lr_offset, best_lr_err
    );

    // 计算 Pearson 相关系数 (在最佳偏移处)
    let compute_corr = |a: &[f32], b: &[f32], offset: i64| -> f64 {
        let n = mono_compare;
        let mut sum_a = 0.0f64;
        let mut sum_b = 0.0f64;
        let mut sum_ab = 0.0f64;
        let mut sum_a2 = 0.0f64;
        let mut sum_b2 = 0.0f64;
        let mut count = 0usize;
        for i in 0..n {
            let b_idx = mono_mid + i;
            let a_idx = b_idx as i64 + offset;
            if b_idx < b.len() && a_idx >= 0 && (a_idx as usize) < a.len() {
                let va = a[a_idx as usize] as f64;
                let vb = b[b_idx] as f64;
                sum_a += va;
                sum_b += vb;
                sum_ab += va * vb;
                sum_a2 += va * va;
                sum_b2 += vb * vb;
                count += 1;
            }
        }
        let n = count as f64;
        let num = n * sum_ab - sum_a * sum_b;
        let den = ((n * sum_a2 - sum_a * sum_a) * (n * sum_b2 - sum_b * sum_b)).sqrt();
        if den > 1e-10 { num / den } else { 0.0 }
    };

    let corr_l = compute_corr(&tao_l, &ff_l, best_l_offset);
    let corr_r = compute_corr(&tao_r, &ff_r, best_r_offset);
    let corr_lr = compute_corr(&tao_l, &ff_r, best_lr_offset);
    let corr_rl = compute_corr(&tao_r, &ff_l, best_l_offset);
    eprintln!(
        "Pearson 相关系数: L={:.4}, R={:.4}, tao_L/ff_R={:.4}, tao_R/ff_L={:.4}",
        corr_l, corr_r, corr_lr, corr_rl
    );

    // 用最佳偏移进行帧级对比
    let tao_shifted: Vec<f32> = if best_offset >= 0 {
        tao_pcm[best_offset as usize..].to_vec()
    } else {
        let pad = vec![0.0f32; (-best_offset) as usize];
        [pad, tao_pcm.clone()].concat()
    };

    let (matched, total, max_err, avg_err) = compare_pcm(&tao_shifted, &ff_pcm, tao_ch, 1152);

    eprintln!(
        "\n对比结果 (对齐后): {}/{} 帧匹配 ({:.1}%), 最大误差: {:.6}, 平均误差: {:.6}",
        matched,
        total,
        matched as f64 / total.max(1) as f64 * 100.0,
        max_err,
        avg_err,
    );

    // 保存用于手动对比
    let _ = save_pcm_raw(&tao_pcm, "data/1.mp3.tao.raw");
    let _ = save_pcm_raw(&ff_pcm, "data/1.mp3.ffmpeg.raw");
}

#[test]
fn test_mp3_compare_ffmpeg_file2() {
    let path = "data/2.mp3";
    if !Path::new(path).exists() {
        eprintln!("跳过: {} 不存在", path);
        return;
    }

    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("跳过: ffmpeg 不可用");
        return;
    }

    let (tao_sr, tao_ch, tao_pcm) = decode_mp3_with_tao(path).expect("tao 解码失败");
    let (ff_sr, ff_ch, ff_pcm) = decode_mp3_with_ffmpeg(path).expect("ffmpeg 解码失败");

    eprintln!("tao:    {}Hz {}ch {} 样本", tao_sr, tao_ch, tao_pcm.len());
    eprintln!("ffmpeg: {}Hz {}ch {} 样本", ff_sr, ff_ch, ff_pcm.len());

    assert_eq!(tao_sr, ff_sr, "采样率不匹配");
    assert_eq!(tao_ch, ff_ch, "通道数不匹配");

    let (matched, total, max_err, avg_err) = compare_pcm(&tao_pcm, &ff_pcm, tao_ch, 1152);

    eprintln!(
        "对比结果: {}/{} 帧匹配 ({:.1}%), 最大误差: {:.6}, 平均误差: {:.6}",
        matched,
        total,
        matched as f64 / total.max(1) as f64 * 100.0,
        max_err,
        avg_err,
    );

    let _ = save_pcm_raw(&tao_pcm, "data/2.mp3.tao.raw");
    let _ = save_pcm_raw(&ff_pcm, "data/2.mp3.ffmpeg.raw");
}

#[test]
fn test_mp3_compare_symphonia_file1() {
    let path = "data/1.mp3";
    if !Path::new(path).exists() {
        eprintln!("跳过: {} 不存在", path);
        return;
    }

    let (tao_sr, tao_ch, tao_pcm) = decode_mp3_with_tao(path).expect("tao 解码失败");
    let (sym_sr, sym_ch, sym_pcm) = decode_mp3_with_symphonia(path).expect("symphonia 解码失败");

    eprintln!(
        "tao:       {}Hz {}ch {} 样本",
        tao_sr,
        tao_ch,
        tao_pcm.len()
    );
    eprintln!(
        "symphonia: {}Hz {}ch {} 样本",
        sym_sr,
        sym_ch,
        sym_pcm.len()
    );

    assert_eq!(tao_sr, sym_sr, "采样率不匹配");
    assert_eq!(tao_ch, sym_ch, "通道数不匹配");

    // 两个 pure Rust 解码器应该有相同的输出长度 (无编码器延迟补偿)
    // 直接从头对比, 搜索最佳偏移
    let compare_len = 20000.min(sym_pcm.len() / 4);
    let mid = sym_pcm.len() / 2;

    let mut best_offset: i64 = 0;
    let mut best_err = f64::MAX;
    for offset_candidate in (-5000i64..=5000).step_by(1) {
        let mut sum_err = 0.0f64;
        let mut count = 0usize;
        for i in 0..compare_len {
            let ref_idx = mid + i;
            let tao_idx = ref_idx as i64 + offset_candidate;
            if ref_idx < sym_pcm.len() && tao_idx >= 0 && (tao_idx as usize) < tao_pcm.len() {
                sum_err += (tao_pcm[tao_idx as usize] - sym_pcm[ref_idx]).abs() as f64;
                count += 1;
            }
        }
        if count > 0 {
            let avg = sum_err / count as f64;
            if avg < best_err {
                best_err = avg;
                best_offset = offset_candidate;
            }
        }
    }

    eprintln!(
        "\ntao vs symphonia 最佳对齐: offset={}, avg_err={:.6}",
        best_offset, best_err
    );

    // 直接从头对比 (两个都没有编码器延迟补偿, offset 应该是 0)
    eprintln!("\n--- offset=0 前 40 个样本 (两个解码器) ---");
    for i in 0..40.min(tao_pcm.len()).min(sym_pcm.len()) {
        let ch = if i % 2 == 0 { "L" } else { "R" };
        eprintln!(
            "  [{:4}] {} tao={:12.6}  sym={:12.6}  ratio={:8.3}",
            i,
            ch,
            tao_pcm[i],
            sym_pcm[i],
            if sym_pcm[i].abs() > 1e-6 {
                tao_pcm[i] / sym_pcm[i]
            } else {
                f32::NAN
            },
        );
    }

    // 从帧3开始对比 (跳过初始transient帧)
    let frame3_start = 2304 * 2; // 帧3开始位置 (每帧2304交织样本)
    eprintln!("\n--- 帧3 (offset=0) 前 20 个样本 ---");
    for i in 0..20 {
        let idx = frame3_start + i;
        if idx < tao_pcm.len() && idx < sym_pcm.len() {
            let ch = if i % 2 == 0 { "L" } else { "R" };
            eprintln!(
                "  [{:5}] {} tao={:12.6}  sym={:12.6}  ratio={:8.3}",
                idx,
                ch,
                tao_pcm[idx],
                sym_pcm[idx],
                if sym_pcm[idx].abs() > 1e-6 {
                    tao_pcm[idx] / sym_pcm[idx]
                } else {
                    f32::NAN
                },
            );
        }
    }

    // Pearson 相关系数
    let n = compare_len;
    let mut sum_a = 0.0f64;
    let mut sum_b = 0.0f64;
    let mut sum_ab = 0.0f64;
    let mut sum_a2 = 0.0f64;
    let mut sum_b2 = 0.0f64;
    let mut cnt = 0usize;
    for i in 0..n {
        let ref_idx = mid + i;
        let tao_idx = ref_idx as i64 + best_offset;
        if ref_idx < sym_pcm.len() && tao_idx >= 0 && (tao_idx as usize) < tao_pcm.len() {
            let va = tao_pcm[tao_idx as usize] as f64;
            let vb = sym_pcm[ref_idx] as f64;
            sum_a += va;
            sum_b += vb;
            sum_ab += va * vb;
            sum_a2 += va * va;
            sum_b2 += vb * vb;
            cnt += 1;
        }
    }
    let nf = cnt as f64;
    let num = nf * sum_ab - sum_a * sum_b;
    let den = ((nf * sum_a2 - sum_a * sum_a) * (nf * sum_b2 - sum_b * sum_b)).sqrt();
    let corr = if den > 1e-10 { num / den } else { 0.0 };
    eprintln!("Pearson 相关系数 (tao vs symphonia): {:.6}", corr);

    // 打印对齐后前 20 个样本
    eprintln!(
        "\n--- tao vs symphonia 对齐后样本 (mid, offset={}) ---",
        best_offset
    );
    for i in 0..20 {
        let ref_idx = mid + i;
        let tao_idx = (ref_idx as i64 + best_offset) as usize;
        if ref_idx < sym_pcm.len() && tao_idx < tao_pcm.len() {
            eprintln!(
                "  [{:5}] tao={:12.6}  sym={:12.6}  diff={:12.6}",
                ref_idx,
                tao_pcm[tao_idx],
                sym_pcm[ref_idx],
                tao_pcm[tao_idx] - sym_pcm[ref_idx],
            );
        }
    }

    // 帧级对比
    let tao_shifted: Vec<f32> = if best_offset >= 0 {
        tao_pcm[best_offset as usize..].to_vec()
    } else {
        let pad = vec![0.0f32; (-best_offset) as usize];
        [pad, tao_pcm.clone()].concat()
    };
    let (matched, total, max_err, avg_err) = compare_pcm(&tao_shifted, &sym_pcm, tao_ch, 1152);
    eprintln!(
        "\ntao vs symphonia: {}/{} 帧匹配 ({:.1}%), 最大误差: {:.6}, 平均误差: {:.6}",
        matched,
        total,
        matched as f64 / total.max(1) as f64 * 100.0,
        max_err,
        avg_err,
    );

    // --- 诊断: 逐帧 RMS 和比值分析 ---
    let frame_size = 1152 * tao_ch as usize;
    // 多区间 RMS 分析
    let total_frames = tao_pcm.len().min(sym_pcm.len()) / frame_size;
    eprintln!("\n--- 逐帧 RMS 分析 (前 10 帧 + 帧 50/100/200/300) ---");
    let check_frames: Vec<usize> = (0..10).chain([50, 100, 200, 300].iter().copied()).collect();
    for &frame in check_frames.iter().filter(|&&f| f < total_frames) {
        let start = frame * frame_size;
        let end = start + frame_size;
        if end > tao_pcm.len() || end > sym_pcm.len() {
            break;
        }
        let tao_rms: f64 = (tao_pcm[start..end]
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum::<f64>()
            / frame_size as f64)
            .sqrt();
        let sym_rms: f64 = (sym_pcm[start..end]
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum::<f64>()
            / frame_size as f64)
            .sqrt();
        let ratio = if sym_rms > 1e-10 {
            tao_rms / sym_rms
        } else {
            f64::NAN
        };
        eprintln!(
            "  帧{:3}: tao_rms={:.6}  sym_rms={:.6}  ratio={:.4}",
            frame, tao_rms, sym_rms, ratio
        );
    }

    // --- 诊断: 帧 2 的 32 个 PCM 样本详细分析 (一个完整 time slot) ---
    let frame2_start = 2 * frame_size;
    eprintln!("\n--- 帧 2, time slot 0, 全部 32 个 PCM 样本 (L 声道) ---");
    for j in 0..32 {
        let idx = frame2_start + j * 2; // L 声道
        if idx < tao_pcm.len() && idx < sym_pcm.len() {
            let t = tao_pcm[idx];
            let s = sym_pcm[idx];
            let ratio = if s.abs() > 1e-6 { t / s } else { f32::NAN };
            eprintln!(
                "  pcm[{:2}] tao={:12.6}  sym={:12.6}  diff={:12.6}  ratio={:8.3}",
                j,
                t,
                s,
                t - s,
                ratio
            );
        }
    }

    // --- Frame 1 分 granule RMS 分析 ---
    {
        let frame1_start = 1 * frame_size;
        let gr0_len = 1152; // 576 samples × 2 channels, interleaved
        for gr in 0..2 {
            let gr_start = frame1_start + gr * gr0_len;
            let gr_end = gr_start + gr0_len;
            if gr_end <= tao_pcm.len() && gr_end <= sym_pcm.len() {
                let tao_rms: f32 = (tao_pcm[gr_start..gr_end].iter().map(|x| x * x).sum::<f32>()
                    / gr0_len as f32)
                    .sqrt();
                let sym_rms: f32 = (sym_pcm[gr_start..gr_end].iter().map(|x| x * x).sum::<f32>()
                    / gr0_len as f32)
                    .sqrt();
                eprintln!(
                    "Frame1 gr={}: tao_rms={:.6} sym_rms={:.6} ratio={:.3}",
                    gr,
                    tao_rms,
                    sym_rms,
                    if sym_rms > 1e-8 {
                        tao_rms / sym_rms
                    } else {
                        f32::NAN
                    }
                );
            }
        }

        // 每 2 个 time slot 采样一个 PCM (L channel)
        eprintln!("\n--- Frame 1 各 time slot L 通道第一个样本 ---");
        for ts in 0..36 {
            let gr = ts / 18;
            let ts_in_gr = ts % 18;
            let idx = frame1_start + gr * gr0_len + ts_in_gr * 64; // 32 stereo pairs = 64 values per time slot
            if idx < tao_pcm.len() && idx < sym_pcm.len() {
                let diff = tao_pcm[idx] - sym_pcm[idx];
                eprintln!(
                    "  gr={} ts={:2}: tao={:12.6}  sym={:12.6}  diff={:+12.6}  ratio={:10.3}",
                    gr,
                    ts_in_gr,
                    tao_pcm[idx],
                    sym_pcm[idx],
                    diff,
                    if sym_pcm[idx].abs() > 1e-8 {
                        tao_pcm[idx] / sym_pcm[idx]
                    } else {
                        f32::NAN
                    },
                );
            }
        }

        // Frame 2 也做同样分析
        let frame2_start = 2 * frame_size;
        eprintln!("\n--- Frame 2 各 time slot L 通道第一个样本 ---");
        for ts in 0..36 {
            let gr = ts / 18;
            let ts_in_gr = ts % 18;
            let idx = frame2_start + gr * gr0_len + ts_in_gr * 64;
            if idx < tao_pcm.len() && idx < sym_pcm.len() {
                let diff = tao_pcm[idx] - sym_pcm[idx];
                eprintln!(
                    "  gr={} ts={:2}: tao={:12.6}  sym={:12.6}  diff={:+12.6}  ratio={:10.3}",
                    gr,
                    ts_in_gr,
                    tao_pcm[idx],
                    sym_pcm[idx],
                    diff,
                    if sym_pcm[idx].abs() > 1e-8 {
                        tao_pcm[idx] / sym_pcm[idx]
                    } else {
                        f32::NAN
                    },
                );
            }
        }
    }

    // --- 三方对比: minimp3 ---
    if let Ok(mini_pcm) = decode_mp3_with_minimp3(path) {
        eprintln!("\n--- 三方 RMS 对比 (前 10 帧) ---");
        let frame_size_ch = 1152; // 每通道每帧样本数
        let frame_size_interleaved = frame_size_ch * tao_ch as usize;
        for frame in 0..10 {
            let start = frame * frame_size_interleaved;
            let end = start + frame_size_interleaved;
            if end > tao_pcm.len() || end > sym_pcm.len() || end > mini_pcm.len() {
                break;
            }
            let rms = |data: &[f32]| -> f64 {
                (data.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / data.len() as f64)
                    .sqrt()
            };
            let t = rms(&tao_pcm[start..end]);
            let s = rms(&sym_pcm[start..end]);
            let m = rms(&mini_pcm[start..end]);
            eprintln!(
                "  帧{:3}: tao={:.6}  sym={:.6}  mini={:.6}  tao/sym={:.4}  tao/mini={:.4}  sym/mini={:.4}",
                frame,
                t,
                s,
                m,
                if s > 1e-10 { t / s } else { f64::NAN },
                if m > 1e-10 { t / m } else { f64::NAN },
                if m > 1e-10 { s / m } else { f64::NAN },
            );
        }
        // 首帧有数据的详细样本对比
        let f1_start = frame_size_interleaved; // Frame 1
        eprintln!("\n--- Frame 1, 前 20 样本 三方对比 ---");
        for i in 0..20 {
            let idx = f1_start + i;
            if idx < tao_pcm.len() && idx < sym_pcm.len() && idx < mini_pcm.len() {
                let ch = if i % 2 == 0 { "L" } else { "R" };
                eprintln!(
                    "  [{:5}] {} tao={:10.6}  sym={:10.6}  mini={:10.6}",
                    idx, ch, tao_pcm[idx], sym_pcm[idx], mini_pcm[idx],
                );
            }
        }
    } else {
        eprintln!("minimp3 解码失败, 跳过三方对比");
    }

    // --- 诊断: Frame 5 L 通道 per-time-slot ratio 分析 ---
    // 这将揭示误差是均匀增益 (所有 ts 相同 ratio) 还是频率依赖 (不同 ts 不同 ratio)
    {
        let frame5_start = 5 * frame_size;
        let gr0_interleaved = 1152; // 576 stereo pairs
        eprintln!("\n--- Frame 5 L 通道 per-time-slot ratio (gr=0, gr=1) ---");
        for gr in 0..2 {
            let gr_start = frame5_start + gr * gr0_interleaved;
            eprintln!("  gr={}:", gr);
            for ts in 0..18 {
                let mut sum_tao_sq = 0.0f64;
                let mut sum_sym_sq = 0.0f64;
                let mut sum_ratio = 0.0f64;
                let mut ratio_count = 0;
                for j in 0..32 {
                    let idx = gr_start + (ts * 32 + j) * 2; // L 声道
                    if idx < tao_pcm.len() && idx < sym_pcm.len() {
                        let t = tao_pcm[idx] as f64;
                        let s = sym_pcm[idx] as f64;
                        sum_tao_sq += t * t;
                        sum_sym_sq += s * s;
                        if s.abs() > 1e-6 {
                            sum_ratio += t / s;
                            ratio_count += 1;
                        }
                    }
                }
                let rms_ratio = if sum_sym_sq > 1e-10 {
                    (sum_tao_sq / sum_sym_sq).sqrt()
                } else {
                    f64::NAN
                };
                let avg_ratio = if ratio_count > 0 {
                    sum_ratio / ratio_count as f64
                } else {
                    f64::NAN
                };
                eprintln!(
                    "    ts={:2}: rms_ratio={:.4}  avg_sample_ratio={:.4}  (N={})",
                    ts, rms_ratio, avg_ratio, ratio_count
                );
            }
        }
    }

    // --- 诊断: Frame 5 gr=0 前 6 个 PCM 样本 (全部 32 subband) ---
    // 如果 ratio 在不同 subband 间变化很大, 说明是频率域问题
    {
        let frame5_start = 5 * frame_size;
        eprintln!("\n--- Frame 5 gr=0 ts=0 全部 32 PCM 样本 (L 声道) ---");
        for j in 0..32 {
            let idx = frame5_start + j * 2; // L 声道
            if idx < tao_pcm.len() && idx < sym_pcm.len() {
                let t = tao_pcm[idx];
                let s = sym_pcm[idx];
                let ratio = if s.abs() > 1e-6 { t / s } else { f32::NAN };
                eprintln!(
                    "    pcm[{:2}] tao={:12.6}  sym={:12.6}  ratio={:8.4}",
                    j, t, s, ratio
                );
            }
        }
    }
}
