// MPEG-4 Part 2 高级特性测试
// 测试 GMC、Data Partitioning、Quarterpel 等高级功能

use tao_codec::decoders::DecoderRegistry;
use tao_format::demuxers::DemuxerRegistry;

/// 测试 GMC (Global Motion Compensation) + Quarterpel
#[test]
#[ignore] // 需要网络访问
fn test_gmc_quarterpel_xvid() {
    let url =
        "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++xvid_gmcqpel_artifact.avi";

    let mut demuxer = DemuxerRegistry::open(url).expect("无法打开 GMC+QPel 样本");

    // 查找视频流
    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建 MPEG-4 解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 20; // 只测试前 20 帧

    while let Some(packet) = demuxer.read_packet().expect("读取 packet 失败") {
        if packet.stream_index != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet).expect("发送 packet 失败");

        while let Some(frame) = decoder.receive_frame().expect("接收 frame 失败") {
            frame_count += 1;
            println!(
                "解码 GMC 帧 #{}, 分辨率: {}x{}",
                frame_count, frame.width, frame.height
            );

            if frame_count >= MAX_FRAMES {
                break;
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

    assert!(
        frame_count >= 10,
        "GMC 样本至少应解码 10 帧，实际: {}",
        frame_count
    );
    println!("✅ GMC + Quarterpel 测试通过，解码 {} 帧", frame_count);
}

/// 测试 Data Partitioning 模式
#[test]
#[ignore] // 需要网络访问
fn test_data_partitioning() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/m4v+mpeg4+++ErrDec_mpeg4datapart-64_qcif.m4v";

    let mut demuxer = DemuxerRegistry::open(url).expect("无法打开 Data Partitioning 样本");

    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建 MPEG-4 解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 15;

    while let Some(packet) = demuxer.read_packet().expect("读取 packet 失败") {
        if packet.stream_index != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet).expect("发送 packet 失败");

        while let Some(frame) = decoder.receive_frame().expect("接收 frame 失败") {
            frame_count += 1;
            println!(
                "解码 Data Partitioning 帧 #{}, 分辨率: {}x{}",
                frame_count, frame.width, frame.height
            );

            if frame_count >= MAX_FRAMES {
                break;
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

    assert!(
        frame_count >= 5,
        "Data Partitioning 样本至少应解码 5 帧，实际: {}",
        frame_count
    );
    println!("✅ Data Partitioning 测试通过，解码 {} 帧", frame_count);
}

/// 测试 Quarterpel 运动补偿 (DivX 5.01)
#[test]
#[ignore] // 需要网络访问
fn test_quarterpel_divx501() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++DivX51-Qpel.avi";

    let mut demuxer = DemuxerRegistry::open(url).expect("无法打开 DivX QPel 样本");

    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建 MPEG-4 解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 20;

    while let Some(packet) = demuxer.read_packet().expect("读取 packet 失败") {
        if packet.stream_index != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet).expect("发送 packet 失败");

        while let Some(frame) = decoder.receive_frame().expect("接收 frame 失败") {
            frame_count += 1;
            println!(
                "解码 Quarterpel 帧 #{}, 分辨率: {}x{}",
                frame_count, frame.width, frame.height
            );

            if frame_count >= MAX_FRAMES {
                break;
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

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
#[ignore] // 需要网络访问
fn test_quarterpel_bframes() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++dx502_b_qpel.avi";

    let mut demuxer = DemuxerRegistry::open(url).expect("无法打开 QPel+B 帧样本");

    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建 MPEG-4 解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 20;

    while let Some(packet) = demuxer.read_packet().expect("读取 packet 失败") {
        if packet.stream_index != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet).expect("发送 packet 失败");

        while let Some(frame) = decoder.receive_frame().expect("接收 frame 失败") {
            frame_count += 1;
            println!(
                "解码 QPel+B 帧 #{}, 分辨率: {}x{}",
                frame_count, frame.width, frame.height
            );

            if frame_count >= MAX_FRAMES {
                break;
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

    assert!(
        frame_count >= 10,
        "QPel+B 帧样本至少应解码 10 帧，实际: {}",
        frame_count
    );
    println!("✅ Quarterpel + B 帧测试通过，解码 {} 帧", frame_count);
}

/// 测试 Data Partitioning Bug 边界情况
#[test]
#[ignore] // 需要网络访问
fn test_data_partitioning_bug() {
    let url = "https://samples.ffmpeg.org/archive/video/mpeg4/avi+mpeg4+++vdpart-bug.avi";

    let mut demuxer = DemuxerRegistry::open(url).expect("无法打开 Data Partition Bug 样本");

    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type.is_video())
        .expect("未找到视频流");

    let stream = &demuxer.streams()[video_stream_index];
    let mut decoder = DecoderRegistry::create_video_decoder(&stream.codec_params)
        .expect("无法创建 MPEG-4 解码器");

    let mut frame_count = 0;
    const MAX_FRAMES: usize = 10;

    // 此样本可能包含错误，应优雅处理而不是 panic
    while let Some(packet) = demuxer.read_packet().unwrap_or(None) {
        if packet.stream_index != video_stream_index {
            continue;
        }

        // 允许部分失败，但不应 panic
        if decoder.send_packet(&packet).is_ok() {
            while let Some(frame) = decoder.receive_frame().unwrap_or(None) {
                frame_count += 1;
                println!(
                    "解码 Data Partition Bug 帧 #{}, 分辨率: {}x{}",
                    frame_count, frame.width, frame.height
                );

                if frame_count >= MAX_FRAMES {
                    break;
                }
            }
        }

        if frame_count >= MAX_FRAMES {
            break;
        }
    }

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
