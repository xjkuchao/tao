// MPEG-4 Part 2 高级特性测试
// 测试 GMC、Data Partitioning、Quarterpel 等高级功能

#[cfg(feature = "http")]
use tao_codec::{CodecParameters, CodecParamsType, CodecRegistry, VideoCodecParams};
#[cfg(feature = "http")]
use tao_core::MediaType;
#[cfg(feature = "http")]
use tao_format::{FormatRegistry, IoContext, stream::StreamParams};

/// 辅助函数: 打开网络样本并解码指定帧数
#[cfg(feature = "http")]
fn decode_network_sample(url: &str, max_frames: usize, test_name: &str) -> Result<usize, String> {
    // 创建并注册所有格式和编解码器
    let mut format_reg = FormatRegistry::new();
    tao_format::register_all(&mut format_reg);

    let mut codec_reg = CodecRegistry::new();
    tao_codec::register_all(&mut codec_reg);

    // 打开网络URL
    let mut io = IoContext::open_url(url).map_err(|e| format!("打开URL失败: {}", e))?;

    // 探测格式并打开解封装器
    let mut demuxer = format_reg
        .open_input(&mut io, None)
        .map_err(|e| format!("打开解封装器失败: {}", e))?;

    // 查找视频流
    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| matches!(s.media_type, MediaType::Video))
        .ok_or_else(|| "未找到视频流".to_string())?;

    let stream = &demuxer.streams()[video_stream_index];

    // 构造 CodecParameters
    let codec_params = match &stream.params {
        StreamParams::Video(v) => CodecParameters {
            codec_id: stream.codec_id,
            extra_data: stream.extra_data.clone(),
            bit_rate: v.bit_rate,
            params: CodecParamsType::Video(VideoCodecParams {
                width: v.width,
                height: v.height,
                pixel_format: v.pixel_format,
                frame_rate: v.frame_rate,
                sample_aspect_ratio: v.sample_aspect_ratio,
            }),
        },
        _ => return Err("不是视频流".to_string()),
    };

    // 创建解码器
    let mut decoder = codec_reg
        .create_decoder(stream.codec_id)
        .map_err(|e| format!("创建解码器失败: {}", e))?;

    decoder
        .open(&codec_params)
        .map_err(|e| format!("打开解码器失败: {}", e))?;

    let mut frame_count = 0;

    // 读取并解码数据包
    loop {
        match demuxer.read_packet(&mut io) {
            Ok(packet) => {
                if packet.stream_index != video_stream_index {
                    continue;
                }

                if decoder.send_packet(&packet).is_ok() {
                    loop {
                        match decoder.receive_frame() {
                            Ok(frame) => {
                                frame_count += 1;
                                let (width, height) = match frame {
                                    tao_codec::Frame::Video(v) => (v.width, v.height),
                                    _ => (0, 0),
                                };
                                println!(
                                    "[{}] 解码帧 #{}, 尺寸: {}x{}",
                                    test_name, frame_count, width, height
                                );

                                if frame_count >= max_frames {
                                    return Ok(frame_count);
                                }
                            }
                            Err(tao_core::TaoError::NeedMoreData) => break,
                            Err(_) => break,
                        }
                    }
                }
            }
            Err(tao_core::TaoError::Eof) => break,
            Err(_) => break,
        }
    }

    Ok(frame_count)
}

/// 测试 GMC (Global Motion Compensation) + Quarterpel
#[test]
#[ignore] // 需要网络访问和 http feature
#[cfg(feature = "http")]
fn test_gmc_quarterpel_xvid() {
    let url =
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi";

    let frame_count = decode_network_sample(url, 20, "GMC+QPel").expect("GMC+QPel 样本解码失败");

    assert!(
        frame_count >= 10,
        "GMC 样本至少应解码 10 帧，实际: {}",
        frame_count
    );
    println!("✅ GMC + Quarterpel 测试通过，解码 {} 帧", frame_count);
}

/// 测试 Data Partitioning 模式
#[test]
#[ignore] // 需要网络访问和 http feature
#[cfg(feature = "http")]
fn test_data_partitioning() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/m4v+mpeg4+++ErrDec_mpeg4datapart-64_qcif.m4v";

    let frame_count =
        decode_network_sample(url, 15, "DataPart").expect("Data Partitioning 样本解码失败");

    assert!(
        frame_count >= 5,
        "Data Partitioning 样本至少应解码 5 帧，实际: {}",
        frame_count
    );
    println!("✅ Data Partitioning 测试通过，解码 {} 帧", frame_count);
}

/// 测试 Quarterpel 运动补偿 (DivX 5.01)
#[test]
#[ignore] // 需要网络访问和 http feature
#[cfg(feature = "http")]
fn test_quarterpel_divx501() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi";

    let frame_count = decode_network_sample(url, 20, "QPel-DivX").expect("DivX QPel 样本解码失败");

    assert!(
        frame_count >= 10,
        "Quarterpel 样本至少应解码 10 帧，实际: {}",
        frame_count
    );
    println!(
        "✅ Quarterpel (DivX 5.01) 测试通过，解码 {} 帧",
        frame_count
    );
}

/// 测试 Quarterpel + B 帧组合
#[test]
#[ignore] // 需要网络访问和 http feature
#[cfg(feature = "http")]
fn test_quarterpel_bframes() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++dx502_b_qpel.avi";

    let frame_count = decode_network_sample(url, 20, "QPel+B").expect("QPel+B 帧样本解码失败");

    assert!(
        frame_count >= 10,
        "QPel+B 帧样本至少应解码 10 帧，实际: {}",
        frame_count
    );
    println!("✅ Quarterpel + B 帧测试通过，解码 {} 帧", frame_count);
}

/// 测试 Data Partitioning Bug 边界情况
#[test]
#[ignore] // 需要网络访问和 http feature
#[cfg(feature = "http")]
fn test_data_partitioning_bug() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi";

    // 此样本可能包含错误，应优雅处理而不是 panic
    let frame_count = decode_network_sample(url, 10, "DataPart-Bug").unwrap_or(0);

    // 即使有错误，至少应该解码一些帧
    assert!(
        frame_count >= 3,
        "Data Partition Bug 样本至少应解码 3 帧，实际: {}",
        frame_count
    );
    println!(
        "✅ Data Partitioning Bug 测试通过，解码 {} 帧（可能存在错误）",
        frame_count
    );
}

// 如果没有 http feature，提供一个占位测试提醒用户
#[test]
#[cfg(not(feature = "http"))]
fn test_advanced_features_require_http() {
    println!("⚠️  MPEG-4 高级特性测试需要启用 'http' feature");
    println!("   请使用以下命令运行:");
    println!("   cargo test --test mpeg4_advanced_features --features http -- --include-ignored");
}
