//! 自研路径 vs Symphonia 路径精确帧级对比
//!
//! 运行:
//!   cargo test --test mp3_native_vs_symphonia --features mp3-native -- --nocapture
//!
//! 此测试同时解码两次:
//! 1. 使用 symphonia (直接调用 symphonia crate)
//! 2. 使用 tao (当 mp3-native feature 启用时, 走自研路径)
//! 然后逐帧对比, 精确定位哪一帧/样本开始出错.

use std::path::Path;
use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

use tao::codec::decoders::mp3::debug;

/// 使用 tao 解码 MP3, 返回每帧的 PCM 数据
fn decode_tao_frames(path: &str) -> Vec<Vec<f32>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path).unwrap();
    let mut demuxer = format_registry.open_input(&mut io, Some(path)).unwrap();
    demuxer.open(&mut io).unwrap();

    let streams = demuxer.streams();
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
    let mut decoder = codec_registry.create_decoder(CodecId::Mp3).unwrap();
    decoder.open(&params).unwrap();

    let mut frames = Vec::new();

    loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                decoder.send_packet(&pkt).unwrap();
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
                        if !audio.data.is_empty() {
                            let bytes = &audio.data[0];
                            let samples: Vec<f32> = bytes
                                .chunks_exact(4)
                                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect();
                            frames.push(samples);
                        }
                    }
                }
                Err(TaoError::NeedMoreData) => break,
                Err(_) => break,
            }
        }
    }

    // drain remaining
    loop {
        match decoder.receive_frame() {
            Ok(frame) => {
                if let tao::codec::frame::Frame::Audio(audio) = &frame {
                    if !audio.data.is_empty() {
                        let bytes = &audio.data[0];
                        let samples: Vec<f32> = bytes
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        frames.push(samples);
                    }
                }
            }
            Err(_) => break,
        }
    }
    frames
}

/// 使用 symphonia 解码 MP3, 返回每帧的 PCM 数据
fn decode_symphonia_frames(path: &str) -> Vec<Vec<f32>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).unwrap();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .unwrap();

    let mut format = probed.format;
    let track = format.default_track().unwrap().clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .unwrap();

    let mut frames = Vec::new();

    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track.id {
                    continue;
                }
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let mut sample_buf =
                            SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                        sample_buf.copy_interleaved_ref(decoded);
                        frames.push(sample_buf.samples().to_vec());
                    }
                    Err(_) => continue,
                }
            }
            Err(_) => break,
        }
    }

    frames
}

#[test]
fn test_native_vs_symphonia_frame_by_frame() {
    let path = "data/1.mp3";
    if !Path::new(path).exists() {
        println!("SKIP: {} not found", path);
        return;
    }

    let tao_frames = decode_tao_frames(path);
    let sym_frames = decode_symphonia_frames(path);

    println!("\n=== tao(自研) vs symphonia 帧级对比 ===");
    println!(
        "tao: {} 帧, symphonia: {} 帧",
        tao_frames.len(),
        sym_frames.len()
    );

    let compare_count = tao_frames.len().min(sym_frames.len()).min(30);

    let mut first_bad_frame = None;
    let mut total_psnr_sum = 0.0f64;
    let mut total_compared = 0;

    for i in 0..compare_count {
        let tao_f = &tao_frames[i];
        let sym_f = &sym_frames[i];

        // 可能帧大小不同 (tao 第一帧可能有 None)
        let len = tao_f.len().min(sym_f.len());
        if len == 0 {
            println!("  帧 {:3}: 跳过 (空帧)", i);
            continue;
        }

        let result =
            debug::compare_f32_samples(&format!("帧 {:3}", i), &tao_f[..len], &sym_f[..len]);

        total_psnr_sum += result.psnr_db;
        total_compared += 1;

        let pass = result.psnr_db >= 60.0 && result.max_abs_error < 0.01;

        if !pass && first_bad_frame.is_none() {
            first_bad_frame = Some(i);
        }

        // 显示前 5 帧, 以及第一个异常帧附近的帧
        let show = i < 5 || !pass || first_bad_frame.map_or(false, |fb| i <= fb + 3);

        if show {
            let status = if pass { "✅" } else { "❌" };
            println!(
                "  {} {} | 样本: {} | 最大误差: {:.2e} | PSNR: {:.1}dB",
                status, result.stage, len, result.max_abs_error, result.psnr_db,
            );

            // 如果有差异, 打印前几个不同的样本
            if !pass {
                let mut diff_count = 0;
                for j in 0..len {
                    let diff = (tao_f[j] - sym_f[j]).abs();
                    if diff > 0.001 {
                        if diff_count < 5 {
                            println!(
                                "    样本 [{}]: tao={:.6}, sym={:.6}, diff={:.6}",
                                j, tao_f[j], sym_f[j], diff,
                            );
                        }
                        diff_count += 1;
                    }
                }
                if diff_count > 5 {
                    println!("    ... 共 {} 个样本差异 > 0.001", diff_count);
                }
            }
        }
    }

    if total_compared > 0 {
        println!("\n--- 统计 ---");
        println!("平均 PSNR: {:.1}dB", total_psnr_sum / total_compared as f64,);
        if let Some(fb) = first_bad_frame {
            println!("第一个异常帧: #{}", fb);
        } else {
            println!("所有帧均通过!");
        }
    }
}
