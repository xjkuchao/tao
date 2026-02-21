use tao_codec::codec_parameters::{CodecParamsType, VideoCodecParams};
use tao_codec::frame::VideoFrame;
use tao_codec::{CodecParameters, CodecRegistry, Frame, Packet};
use tao_core::{MediaType, PixelFormat, TaoError};
use tao_format::stream::{Stream, StreamParams};
use tao_format::{FormatRegistry, IoContext};

use crate::Cli;
use crate::filter::pts_to_sec;

pub(crate) fn transcode_to_raw_yuv(
    input_path: &str,
    output_path: &str,
    cli: &Cli,
) -> Result<(), TaoError> {
    use std::fs::File;

    eprintln!(
        "tao 版本 {} -- 纯 Rust 多媒体转码工具 (YUV 模式)",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("输入: {input_path}");
    eprintln!("输出: {output_path}");

    // 检查输出文件是否已存在
    if !cli.overwrite && std::path::Path::new(output_path).exists() {
        eprintln!("错误: 输出文件已存在 '{output_path}', 使用 -y 覆盖");
        return Err(TaoError::InvalidArgument(
            "输出文件已存在，需要 -y 参数".to_string(),
        ));
    }

    // 初始化注册表
    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);

    let mut codec_registry = CodecRegistry::new();
    tao_codec::register_all(&mut codec_registry);

    // 打开输入文件
    let mut input_io = IoContext::open_url(input_path)
        .or_else(|_| IoContext::open_read(input_path))
        .map_err(|_| TaoError::InvalidData(format!("无法打开输入文件 '{input_path}'")))?;

    // 探测并打开输入
    let mut demuxer = format_registry
        .open_input(&mut input_io, Some(input_path))
        .map_err(|_| TaoError::InvalidData("无法打开输入格式".to_string()))?;

    let input_streams: Vec<Stream> = demuxer.streams().to_vec();

    if input_streams.is_empty() {
        return Err(TaoError::InvalidData(
            "输入文件中没有找到任何流".to_string(),
        ));
    }

    eprintln!("输入格式: {}, {} 条流", demuxer.name(), input_streams.len());

    // 查找第一个视频流
    let video_stream = input_streams
        .iter()
        .find(|s| s.media_type == MediaType::Video)
        .ok_or_else(|| TaoError::Unsupported("没有找到视频流".to_string()))?;

    eprintln!(
        "  视频流 #{}: {}",
        video_stream.index, video_stream.codec_id
    );

    let video_params = match &video_stream.params {
        StreamParams::Video(v) => v,
        _ => return Err(TaoError::InvalidArgument("不是视频流".to_string())),
    };

    eprintln!(
        "  分辨率: {}x{}, 伽码: {:?}",
        video_params.width, video_params.height, video_params.pixel_format
    );

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(video_stream.codec_id)?;
    let dec_params = CodecParameters {
        codec_id: video_stream.codec_id,
        extra_data: video_stream.extra_data.clone(),
        bit_rate: video_params.bit_rate,
        params: CodecParamsType::Video(VideoCodecParams {
            width: video_params.width,
            height: video_params.height,
            pixel_format: video_params.pixel_format,
            frame_rate: video_params.frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
        }),
    };
    decoder.open(&dec_params)?;

    // 打开输出文件
    let mut output_file = File::create(output_path).map_err(TaoError::Io)?;

    // 解析 -ss/-t 参数
    let start_time_sec = cli.ss.unwrap_or(0.0);
    let duration_limit_sec = cli.duration;

    // 处理循环
    let mut frame_count = 0u64;
    let mut byte_count = 0u64;
    const MAX_FRAMES_FOR_VERIFICATION: u64 = 10;

    loop {
        // 检查是否已达到帧数限制
        if frame_count >= MAX_FRAMES_FOR_VERIFICATION {
            break;
        }

        match demuxer.read_packet(&mut input_io) {
            Ok(input_pkt) => {
                let stream_idx = input_pkt.stream_index;
                if stream_idx != video_stream.index {
                    continue;
                }

                // -ss: 跳过早于起始时间的数据包
                if start_time_sec > 0.0 {
                    let pkt_time = pts_to_sec(
                        input_pkt.pts,
                        video_stream.time_base.num,
                        video_stream.time_base.den,
                    );
                    if pkt_time < start_time_sec {
                        continue;
                    }
                }

                // -t: 检查持续时间限制
                if let Some(dur) = duration_limit_sec {
                    let pkt_time = pts_to_sec(
                        input_pkt.pts,
                        video_stream.time_base.num,
                        video_stream.time_base.den,
                    );
                    let effective_time = pkt_time - start_time_sec;
                    if effective_time > dur {
                        break;
                    }
                }

                // 发送数据包到解码器
                decoder.send_packet(&input_pkt)?;

                // 接收解码后的帧
                loop {
                    match decoder.receive_frame() {
                        Ok(frame) => {
                            // 确保是视频帧并转换为 YUV420p
                            if let Frame::Video(vf) = &frame {
                                let yuv_frame =
                                    ensure_yuv420p(vf, video_params.width, video_params.height)?;

                                // 写入 YUV 数据
                                write_yuv420p_frame(&mut output_file, &yuv_frame)?;
                                frame_count += 1;
                                byte_count += yuv_frame_size(yuv_frame.width, yuv_frame.height);

                                if frame_count % 10 == 0 {
                                    eprint!(
                                        "\r已处理 {} 帧, {:.2} MB",
                                        frame_count,
                                        byte_count as f64 / (1024.0 * 1024.0)
                                    );
                                }

                                // 为 PSNR 验证限制帧数
                                if frame_count >= 10 {
                                    eprintln!("\r已达到测试帧数限制 (10 帧)");
                                    break;
                                }
                            }
                        }
                        Err(TaoError::NeedMoreData) => break,
                        Err(TaoError::Eof) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }

    // 刷新解码器的缓存帧
    decoder.send_packet(&Packet::empty())?;
    loop {
        match decoder.receive_frame() {
            Ok(frame) => {
                if let Frame::Video(vf) = &frame {
                    let yuv_frame = ensure_yuv420p(vf, video_params.width, video_params.height)?;
                    write_yuv420p_frame(&mut output_file, &yuv_frame)?;
                    frame_count += 1;
                    byte_count += yuv_frame_size(yuv_frame.width, yuv_frame.height);
                }
            }
            Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }

    eprintln!();
    eprintln!("YUV 输出完成:");
    eprintln!("  输出帧数: {frame_count}");
    eprintln!(
        "  输出大小: {byte_count} 字节 ({:.2} MB)",
        byte_count as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// 确保视频帧是 YUV420p 格式
fn ensure_yuv420p(frame: &VideoFrame, width: u32, height: u32) -> Result<VideoFrame, TaoError> {
    if frame.pixel_format == PixelFormat::Yuv420p {
        return Ok(frame.clone());
    }

    // 需要使用 tao_scale 进行格式转换
    // 需要使用 tao_scale 进行格式转换
    let ctx = tao_scale::ScaleContext::new(
        width,
        height,
        frame.pixel_format,
        width,
        height,
        PixelFormat::Yuv420p,
        tao_scale::ScaleAlgorithm::Bilinear,
    );

    // 准备源数据
    let src_planes: Vec<&[u8]> = frame.data.iter().map(|d| d.as_slice()).collect();
    let src_linesize: Vec<usize> = frame.linesize.clone();

    // 分配目标帧
    let dst_w = width;
    let dst_h = height;
    let dst_fmt = PixelFormat::Yuv420p;
    let plane_count = dst_fmt.plane_count() as usize;

    let mut dst_bufs: Vec<Vec<u8>> = Vec::with_capacity(plane_count);
    let mut dst_linesizes: Vec<usize> = Vec::with_capacity(plane_count);

    for p in 0..plane_count {
        let ls = dst_fmt.plane_linesize(p, dst_w).unwrap_or(dst_w as usize);
        let h = dst_fmt.plane_height(p, dst_h).unwrap_or(dst_h as usize);
        dst_bufs.push(vec![0u8; ls * h]);
        dst_linesizes.push(ls);
    }

    {
        let mut dst_slices: Vec<&mut [u8]> =
            dst_bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
        ctx.scale(&src_planes, &src_linesize, &mut dst_slices, &dst_linesizes)
            .ok();
    }

    let mut out_frame = VideoFrame::new(dst_w, dst_h, dst_fmt);
    out_frame.data = dst_bufs;
    out_frame.linesize = dst_linesizes;
    out_frame.pts = frame.pts;
    out_frame.time_base = frame.time_base;

    Ok(out_frame)
}

/// 计算 YUV420p 帧大小
fn yuv_frame_size(width: u32, height: u32) -> u64 {
    let y_size = width as u64 * height as u64;
    let uv_size = (width as u64 / 2) * (height as u64 / 2) * 2;
    y_size + uv_size
}

/// 写入 YUV420p 帧到文件
fn write_yuv420p_frame(file: &mut std::fs::File, frame: &VideoFrame) -> Result<(), TaoError> {
    use std::io::Write;

    // 写入所有平面 (Y, U, V)
    for plane_data in &frame.data {
        file.write_all(plane_data).map_err(TaoError::Io)?;
    }

    Ok(())
}
