//! tao - 多媒体转码命令行工具
//!
//! 对标 FFmpeg 的 ffmpeg 命令行工具, 提供音视频转码、格式转换等功能.

use clap::Parser;
use std::process;

use tao_codec::{
    CodecId, CodecParameters, CodecRegistry, Decoder, Encoder, Frame, Packet,
    codec_parameters::{AudioCodecParams, CodecParamsType, VideoCodecParams},
    frame::AudioFrame,
};
use tao_core::{ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat, TaoError};
use tao_filter::FilterGraph;
use tao_format::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};
use tao_format::{FormatId, FormatRegistry, IoContext, Muxer};
use tao_resample::ResampleContext;

/// Tao 多媒体转码工具
#[derive(Parser, Debug)]
#[command(name = "tao", version, about = "纯 Rust 多媒体转码工具")]
struct Cli {
    /// 输入文件路径
    #[arg(short, long)]
    input: Option<String>,

    /// 输出文件路径
    #[arg(short, long)]
    output: Option<String>,

    /// 音频编解码器 ("copy" 表示直接复制, 或编解码器名如 "pcm_s16le")
    #[arg(short = 'c', long = "acodec")]
    acodec: Option<String>,

    /// 视频编解码器 ("copy" 表示直接复制, 或编解码器名如 "rawvideo")
    #[arg(long = "vcodec")]
    vcodec: Option<String>,

    /// 目标采样率 (Hz)
    #[arg(long)]
    ar: Option<u32>,

    /// 目标声道数
    #[arg(long)]
    ac: Option<u32>,

    /// 目标视频分辨率 (如 "1280x720")
    #[arg(short = 's', long = "size")]
    size: Option<String>,

    /// 目标帧率 (如 "25" 或 "30000/1001")
    #[arg(short = 'r', long = "rate")]
    rate: Option<String>,

    /// 视频滤镜链 (如 "crop=640:480:0:0,pad=800:600:80:60")
    #[arg(long = "vf")]
    vf: Option<String>,

    /// 音频滤镜链 (如 "volume=0.5,fade=in:0:3")
    #[arg(long = "af")]
    af: Option<String>,

    /// 持续时间限制 (秒)
    #[arg(short = 't', long = "duration")]
    duration: Option<f64>,

    /// 起始时间偏移 (秒)
    #[arg(long = "ss")]
    ss: Option<f64>,

    /// 覆盖输出文件
    #[arg(short = 'y', long)]
    overwrite: bool,

    /// 显示版本和编译信息
    #[arg(long)]
    build_info: bool,
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    if cli.build_info {
        print_build_info();
        return;
    }

    if cli.input.is_none() {
        print_banner();
        return;
    }

    let input_path = cli.input.as_ref().unwrap();

    if cli.output.is_none() {
        eprintln!("错误: 必须指定输出文件 (-o <输出文件>)");
        process::exit(1);
    }
    let output_path = cli.output.as_ref().unwrap();

    // 检查输出文件是否已存在
    if !cli.overwrite && std::path::Path::new(output_path).exists() {
        eprintln!("错误: 输出文件已存在 '{output_path}', 使用 -y 覆盖");
        process::exit(1);
    }

    eprintln!(
        "tao 版本 {} -- 纯 Rust 多媒体转码工具",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("输入: {input_path}");
    eprintln!("输出: {output_path}");

    // 解析目标分辨率
    let target_size = cli.size.as_deref().and_then(parse_size);
    // 解析目标帧率
    let target_rate = cli.rate.as_deref().and_then(parse_rate);
    // 解析 -ss/-t
    let start_time_sec = cli.ss.unwrap_or(0.0);
    let duration_limit_sec = cli.duration;

    // 初始化注册表
    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);

    let mut codec_registry = CodecRegistry::new();
    tao_codec::register_all(&mut codec_registry);

    // 打开输入文件
    let mut input_io = match IoContext::open_read(input_path) {
        Ok(io) => io,
        Err(e) => {
            eprintln!("错误: 无法打开输入文件 '{input_path}': {e}");
            process::exit(1);
        }
    };

    // 探测并打开输入
    let mut demuxer = match format_registry.open_input(&mut input_io, Some(input_path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("错误: 无法打开输入格式: {e}");
            process::exit(1);
        }
    };

    let input_streams: Vec<Stream> = demuxer.streams().to_vec();

    if input_streams.is_empty() {
        eprintln!("错误: 输入文件中没有找到任何流");
        process::exit(1);
    }

    eprintln!("输入格式: {}, {} 条流", demuxer.name(), input_streams.len());

    // 确定输出格式
    let output_format = match FormatId::from_filename(output_path) {
        Some(f) => f,
        None => {
            eprintln!("错误: 无法从输出文件名确定格式: '{output_path}'");
            process::exit(1);
        }
    };

    eprintln!("输出格式: {output_format}");

    // 确定每条流的处理方式
    let is_audio_copy = cli.acodec.as_deref() == Some("copy");
    let is_video_copy = cli.vcodec.as_deref() == Some("copy");

    let target_audio_codec = if is_audio_copy {
        None
    } else {
        cli.acodec.as_deref().map(parse_codec_name)
    };

    let target_video_codec = if is_video_copy {
        None
    } else {
        cli.vcodec.as_deref().map(parse_codec_name)
    };

    // 解析视频/音频滤镜链
    let video_filters = cli.vf.as_deref().map(parse_filter_chain);
    let audio_filters = cli.af.as_deref().map(parse_filter_chain);

    // 为每条流准备编解码器
    let mut stream_processors: Vec<Option<StreamProcessor>> = Vec::new();
    let mut output_streams: Vec<Stream> = Vec::new();
    let mut stream_copy_flags: Vec<bool> = Vec::new();

    for stream in &input_streams {
        match stream.media_type {
            MediaType::Audio => {
                if is_audio_copy {
                    output_streams.push(stream.clone());
                    stream_processors.push(None);
                    stream_copy_flags.push(true);
                    eprintln!("  流 #{}: 音频 -> 直接复制", stream.index);
                } else {
                    let out_codec_id = target_audio_codec.unwrap_or(stream.codec_id);
                    let processor = create_audio_processor(
                        stream,
                        out_codec_id,
                        &codec_registry,
                        cli.ar,
                        cli.ac,
                        &audio_filters,
                    );
                    match processor {
                        Ok((proc, out_stream)) => {
                            eprintln!(
                                "  流 #{}: 音频 {} -> {}",
                                stream.index, stream.codec_id, out_codec_id
                            );
                            output_streams.push(out_stream);
                            stream_processors.push(Some(proc));
                            stream_copy_flags.push(false);
                        }
                        Err(e) => {
                            eprintln!("错误: 无法创建流 #{} 的编解码器: {e}", stream.index);
                            process::exit(1);
                        }
                    }
                }
            }
            MediaType::Video => {
                if is_video_copy {
                    output_streams.push(stream.clone());
                    stream_processors.push(None);
                    stream_copy_flags.push(true);
                    eprintln!("  流 #{}: 视频 -> 直接复制", stream.index);
                } else if cli.vcodec.is_some()
                    || target_size.is_some()
                    || target_rate.is_some()
                    || video_filters.is_some()
                {
                    let out_codec_id = target_video_codec.unwrap_or(stream.codec_id);
                    let processor = create_video_processor(
                        stream,
                        out_codec_id,
                        &codec_registry,
                        target_size,
                        target_rate,
                        &video_filters,
                    );
                    match processor {
                        Ok((proc, out_stream)) => {
                            eprintln!(
                                "  流 #{}: 视频 {} -> {} ({}x{})",
                                stream.index,
                                stream.codec_id,
                                out_codec_id,
                                if let StreamParams::Video(v) = &out_stream.params {
                                    v.width
                                } else {
                                    0
                                },
                                if let StreamParams::Video(v) = &out_stream.params {
                                    v.height
                                } else {
                                    0
                                }
                            );
                            output_streams.push(out_stream);
                            stream_processors.push(Some(proc));
                            stream_copy_flags.push(false);
                        }
                        Err(e) => {
                            eprintln!("错误: 无法创建流 #{} 的视频编解码器: {e}", stream.index);
                            process::exit(1);
                        }
                    }
                } else {
                    // 没有指定 -vcodec 且无视频处理参数, 跳过视频流
                    eprintln!("  流 #{}: 视频 -> 跳过 (未指定 --vcodec)", stream.index);
                    stream_processors.push(None);
                    stream_copy_flags.push(false);
                }
            }
            _ => {
                eprintln!(
                    "  流 #{}: {} -> 跳过 (暂不支持)",
                    stream.index, stream.media_type
                );
                stream_processors.push(None);
                stream_copy_flags.push(false);
            }
        }
    }

    if output_streams.is_empty() {
        eprintln!("错误: 没有可输出的流");
        process::exit(1);
    }

    // 打开输出文件
    let mut output_io = match IoContext::open_read_write(output_path) {
        Ok(io) => io,
        Err(e) => {
            eprintln!("错误: 无法创建输出文件 '{output_path}': {e}");
            process::exit(1);
        }
    };

    // 创建封装器
    let mut muxer: Box<dyn Muxer> = match format_registry.create_muxer(output_format) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("错误: 无法创建输出格式封装器: {e}");
            process::exit(1);
        }
    };

    // 写入头部
    if let Err(e) = muxer.write_header(&mut output_io, &output_streams) {
        eprintln!("错误: 无法写入输出文件头部: {e}");
        process::exit(1);
    }

    // 处理循环: demux → (decode → filter → scale → encode) → mux
    let mut packet_count = 0u64;
    let mut byte_count = 0u64;

    loop {
        match demuxer.read_packet(&mut input_io) {
            Ok(input_pkt) => {
                let stream_idx = input_pkt.stream_index;
                if stream_idx >= input_streams.len() {
                    continue;
                }

                let in_stream = &input_streams[stream_idx];

                // -ss: 跳过早于起始时间的数据包
                if start_time_sec > 0.0 {
                    let pkt_time = pts_to_sec(
                        input_pkt.pts,
                        in_stream.time_base.num,
                        in_stream.time_base.den,
                    );
                    if pkt_time < start_time_sec {
                        continue;
                    }
                }

                // -t: 检查持续时间限制
                if let Some(dur) = duration_limit_sec {
                    let pkt_time = pts_to_sec(
                        input_pkt.pts,
                        in_stream.time_base.num,
                        in_stream.time_base.den,
                    );
                    let effective_time = pkt_time - start_time_sec;
                    if effective_time > dur {
                        break;
                    }
                }

                // 检查此流是否被输出
                let out_stream_idx = output_streams.iter().position(|s| s.index == stream_idx);
                let out_stream_idx = match out_stream_idx {
                    Some(idx) => idx,
                    None => continue,
                };

                if stream_idx < stream_copy_flags.len() && stream_copy_flags[stream_idx] {
                    // 直接复制路径
                    let mut out_pkt = input_pkt.clone();
                    out_pkt.stream_index = out_stream_idx;
                    if let Err(e) = muxer.write_packet(&mut output_io, &out_pkt) {
                        eprintln!("错误: 写入数据包失败: {e}");
                        process::exit(1);
                    }
                    packet_count += 1;
                    byte_count += out_pkt.size() as u64;
                } else if let Some(ref mut processor) = stream_processors[stream_idx] {
                    // 转码路径
                    match transcode_packet(processor, &input_pkt, out_stream_idx) {
                        Ok(packets) => {
                            for out_pkt in &packets {
                                if let Err(e) = muxer.write_packet(&mut output_io, out_pkt) {
                                    eprintln!("错误: 写入数据包失败: {e}");
                                    process::exit(1);
                                }
                                byte_count += out_pkt.size() as u64;
                            }
                            packet_count += packets.len() as u64;
                        }
                        Err(e) => {
                            eprintln!("错误: 转码失败: {e}");
                            process::exit(1);
                        }
                    }
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => {
                eprintln!("错误: 读取数据包失败: {e}");
                process::exit(1);
            }
        }
    }

    // 刷新编码器缓存
    for (idx, proc_opt) in stream_processors.iter_mut().enumerate() {
        if let Some(processor) = proc_opt {
            let out_stream_idx = output_streams
                .iter()
                .position(|s| s.index == idx)
                .unwrap_or(0);
            match flush_encoder(processor, out_stream_idx) {
                Ok(packets) => {
                    for out_pkt in &packets {
                        if let Err(e) = muxer.write_packet(&mut output_io, out_pkt) {
                            eprintln!("错误: 写入刷新数据包失败: {e}");
                            process::exit(1);
                        }
                        byte_count += out_pkt.size() as u64;
                    }
                    packet_count += packets.len() as u64;
                }
                Err(e) => {
                    eprintln!("警告: 刷新编码器时出错: {e}");
                }
            }
        }
    }

    // 写入尾部
    if let Err(e) = muxer.write_trailer(&mut output_io) {
        eprintln!("错误: 无法写入输出文件尾部: {e}");
        process::exit(1);
    }

    eprintln!();
    eprintln!("转码完成:");
    eprintln!("  输出数据包: {packet_count}");
    eprintln!(
        "  输出大小: {byte_count} 字节 ({:.2} KB)",
        byte_count as f64 / 1024.0
    );
}

// ============================================================
// 流处理器
// ============================================================

/// 流处理器: 解码器 + 滤镜 + 缩放 + 重采样 + 编码器
struct StreamProcessor {
    decoder: Box<dyn Decoder>,
    encoder: Box<dyn Encoder>,
    resampler: Option<ResampleContext>,
    filter_graph: Option<FilterGraph>,
    video_scaler: Option<VideoScaleConfig>,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
}

/// 视频缩放配置
struct VideoScaleConfig {
    dst_width: u32,
    dst_height: u32,
    dst_pixel_format: PixelFormat,
}

// ============================================================
// 转码/刷新
// ============================================================

/// 转码一个数据包
fn transcode_packet(
    proc: &mut StreamProcessor,
    input_pkt: &Packet,
    out_stream_idx: usize,
) -> Result<Vec<Packet>, TaoError> {
    proc.decoder.send_packet(input_pkt)?;

    let mut output_packets = Vec::new();

    loop {
        match proc.decoder.receive_frame() {
            Ok(frame) => {
                // 应用滤镜
                let filtered_frame = if let Some(ref mut graph) = proc.filter_graph {
                    graph.process_frame(&frame)?
                } else {
                    frame
                };

                // 视频缩放
                let scaled_frame = if let Some(ref scale_cfg) = proc.video_scaler {
                    scale_video_frame(&filtered_frame, scale_cfg)?
                } else {
                    filtered_frame
                };

                // 音频重采样
                let frame_to_encode = if let Some(ref resampler) = proc.resampler {
                    resample_frame(
                        resampler,
                        &scaled_frame,
                        proc.dst_channels,
                        proc.dst_sample_format,
                    )?
                } else {
                    scaled_frame
                };

                proc.encoder.send_frame(Some(&frame_to_encode))?;

                loop {
                    match proc.encoder.receive_packet() {
                        Ok(mut pkt) => {
                            pkt.stream_index = out_stream_idx;
                            output_packets.push(pkt);
                        }
                        Err(TaoError::NeedMoreData) => break,
                        Err(TaoError::Eof) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(TaoError::NeedMoreData) => break,
            Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }

    Ok(output_packets)
}

/// 刷新编码器
fn flush_encoder(
    proc: &mut StreamProcessor,
    out_stream_idx: usize,
) -> Result<Vec<Packet>, TaoError> {
    proc.encoder.send_frame(None)?;

    let mut output_packets = Vec::new();
    loop {
        match proc.encoder.receive_packet() {
            Ok(mut pkt) => {
                pkt.stream_index = out_stream_idx;
                output_packets.push(pkt);
            }
            Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(output_packets)
}

// ============================================================
// 视频缩放
// ============================================================

/// 缩放视频帧
fn scale_video_frame(frame: &Frame, config: &VideoScaleConfig) -> Result<Frame, TaoError> {
    use tao_codec::frame::VideoFrame;

    match frame {
        Frame::Video(vf) => {
            if vf.width == config.dst_width
                && vf.height == config.dst_height
                && vf.pixel_format == config.dst_pixel_format
            {
                return Ok(frame.clone());
            }

            let ctx = tao_scale::ScaleContext::new(
                vf.width,
                vf.height,
                vf.pixel_format,
                config.dst_width,
                config.dst_height,
                config.dst_pixel_format,
                tao_scale::ScaleAlgorithm::Bilinear,
            );

            // 准备源数据
            let src_planes: Vec<&[u8]> = vf.data.iter().map(|d| d.as_slice()).collect();
            let src_linesize: Vec<usize> = vf.linesize.clone();

            // 分配目标帧
            let dst_fmt = config.dst_pixel_format;
            let dst_w = config.dst_width;
            let dst_h = config.dst_height;
            let plane_count = dst_fmt.plane_count() as usize;

            let mut dst_bufs: Vec<Vec<u8>> = Vec::with_capacity(plane_count);
            let mut dst_linesizes: Vec<usize> = Vec::with_capacity(plane_count);

            for p in 0..plane_count {
                let ls = dst_fmt
                    .plane_linesize(p, dst_w)
                    .unwrap_or(dst_w as usize * 3);
                let h = dst_fmt.plane_height(p, dst_h).unwrap_or(dst_h as usize);
                dst_bufs.push(vec![0u8; ls * h]);
                dst_linesizes.push(ls);
            }

            {
                let mut dst_slices: Vec<&mut [u8]> =
                    dst_bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
                ctx.scale(&src_planes, &src_linesize, &mut dst_slices, &dst_linesizes)?;
            }

            let mut out_frame = VideoFrame::new(dst_w, dst_h, dst_fmt);
            out_frame.data = dst_bufs;
            out_frame.linesize = dst_linesizes;
            out_frame.pts = vf.pts;
            out_frame.time_base = vf.time_base;

            Ok(Frame::Video(out_frame))
        }
        _ => Ok(frame.clone()),
    }
}

// ============================================================
// 音频重采样
// ============================================================

/// 重采样一帧音频
fn resample_frame(
    resampler: &ResampleContext,
    frame: &Frame,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
) -> Result<Frame, TaoError> {
    match frame {
        Frame::Audio(audio) => {
            let input_data = &audio.data[0];
            let (output_data, nb_out) = resampler.convert(input_data, audio.nb_samples)?;

            let mut out_frame = AudioFrame::new(
                nb_out,
                resampler.dst_sample_rate,
                dst_sample_format,
                ChannelLayout::from_channels(dst_channels),
            );
            out_frame.data[0] = output_data;
            out_frame.pts = audio.pts;
            out_frame.time_base = audio.time_base;
            out_frame.duration = nb_out as i64;

            Ok(Frame::Audio(out_frame))
        }
        _ => Err(TaoError::Unsupported("视频帧重采样尚未实现".to_string())),
    }
}

// ============================================================
// 音频处理器创建
// ============================================================

/// 为音频流创建处理器
fn create_audio_processor(
    input_stream: &Stream,
    output_codec_id: CodecId,
    codec_registry: &CodecRegistry,
    target_sample_rate: Option<u32>,
    target_channels: Option<u32>,
    audio_filters: &Option<Vec<FilterSpec>>,
) -> Result<(StreamProcessor, Stream), TaoError> {
    let audio_params = match &input_stream.params {
        StreamParams::Audio(a) => a,
        _ => {
            return Err(TaoError::InvalidArgument("不是音频流".to_string()));
        }
    };

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(input_stream.codec_id)?;
    let dec_params = CodecParameters {
        codec_id: input_stream.codec_id,
        extra_data: input_stream.extra_data.clone(),
        bit_rate: audio_params.bit_rate,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: audio_params.sample_rate,
            channel_layout: audio_params.channel_layout,
            sample_format: audio_params.sample_format,
            frame_size: audio_params.frame_size,
        }),
    };
    decoder.open(&dec_params)?;

    // 确定输出参数
    let out_sample_rate = target_sample_rate.unwrap_or(audio_params.sample_rate);
    let out_channels = target_channels.unwrap_or(audio_params.channel_layout.channels);
    let out_channel_layout = ChannelLayout::from_channels(out_channels);

    let out_sample_format =
        codec_id_to_sample_format(output_codec_id).unwrap_or(audio_params.sample_format);

    // 创建编码器
    let mut encoder = codec_registry.create_encoder(output_codec_id)?;
    let enc_params = CodecParameters {
        codec_id: output_codec_id,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: out_sample_rate,
            channel_layout: out_channel_layout,
            sample_format: out_sample_format,
            frame_size: 0,
        }),
    };
    encoder.open(&enc_params)?;

    // 判断是否需要重采样
    let need_resample = audio_params.sample_rate != out_sample_rate
        || audio_params.channel_layout.channels != out_channels
        || audio_params.sample_format != out_sample_format;

    let resampler = if need_resample {
        Some(ResampleContext::new(
            audio_params.sample_rate,
            audio_params.sample_format,
            audio_params.channel_layout,
            out_sample_rate,
            out_sample_format,
            out_channel_layout,
        ))
    } else {
        None
    };

    // 创建音频滤镜图
    let filter_graph = build_audio_filter_graph(audio_filters);

    // 构建输出流描述
    let out_stream = Stream {
        index: input_stream.index,
        media_type: MediaType::Audio,
        codec_id: output_codec_id,
        time_base: Rational::new(1, out_sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate: out_sample_rate,
            channel_layout: out_channel_layout,
            sample_format: out_sample_format,
            bit_rate: 0,
            frame_size: 0,
        }),
        metadata: input_stream.metadata.clone(),
    };

    let processor = StreamProcessor {
        decoder,
        encoder,
        resampler,
        filter_graph,
        video_scaler: None,
        dst_channels: out_channels,
        dst_sample_format: out_sample_format,
    };

    Ok((processor, out_stream))
}

// ============================================================
// 视频处理器创建
// ============================================================

/// 为视频流创建处理器
fn create_video_processor(
    input_stream: &Stream,
    output_codec_id: CodecId,
    codec_registry: &CodecRegistry,
    target_size: Option<(u32, u32)>,
    target_rate: Option<Rational>,
    video_filters: &Option<Vec<FilterSpec>>,
) -> Result<(StreamProcessor, Stream), TaoError> {
    let video_params = match &input_stream.params {
        StreamParams::Video(v) => v,
        _ => {
            return Err(TaoError::InvalidArgument("不是视频流".to_string()));
        }
    };

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(input_stream.codec_id)?;
    let dec_params = CodecParameters {
        codec_id: input_stream.codec_id,
        extra_data: input_stream.extra_data.clone(),
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

    // 确定输出参数
    let (out_width, out_height) = target_size.unwrap_or((video_params.width, video_params.height));
    let out_pixel_format = video_params.pixel_format;
    let out_frame_rate = target_rate.unwrap_or(video_params.frame_rate);

    // 创建编码器
    let mut encoder = codec_registry.create_encoder(output_codec_id)?;
    let enc_params = CodecParameters {
        codec_id: output_codec_id,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Video(VideoCodecParams {
            width: out_width,
            height: out_height,
            pixel_format: out_pixel_format,
            frame_rate: out_frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
        }),
    };
    encoder.open(&enc_params)?;

    // 缩放配置
    let needs_scale = out_width != video_params.width || out_height != video_params.height;
    let video_scaler = if needs_scale {
        Some(VideoScaleConfig {
            dst_width: out_width,
            dst_height: out_height,
            dst_pixel_format: out_pixel_format,
        })
    } else {
        None
    };

    // 创建视频滤镜图
    let filter_graph = build_video_filter_graph(video_filters);

    // 构建输出流描述
    let out_stream = Stream {
        index: input_stream.index,
        media_type: MediaType::Video,
        codec_id: output_codec_id,
        time_base: Rational::new(out_frame_rate.den, out_frame_rate.num),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Video(VideoStreamParams {
            width: out_width,
            height: out_height,
            pixel_format: out_pixel_format,
            frame_rate: out_frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
            bit_rate: 0,
        }),
        metadata: input_stream.metadata.clone(),
    };

    let processor = StreamProcessor {
        decoder,
        encoder,
        resampler: None,
        filter_graph,
        video_scaler,
        dst_channels: 0,
        dst_sample_format: SampleFormat::None,
    };

    Ok((processor, out_stream))
}

// ============================================================
// 滤镜解析与构建
// ============================================================

/// 滤镜描述
#[derive(Debug, Clone)]
struct FilterSpec {
    name: String,
    args: Vec<String>,
}

/// 解析滤镜链字符串 (如 "volume=0.5,fade=in:0:3")
fn parse_filter_chain(chain: &str) -> Vec<FilterSpec> {
    chain
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|filter_str| {
            let filter_str = filter_str.trim();
            if let Some(eq_pos) = filter_str.find('=') {
                let name = filter_str[..eq_pos].trim().to_string();
                let args: Vec<String> = filter_str[eq_pos + 1..]
                    .split(':')
                    .map(|s| s.trim().to_string())
                    .collect();
                FilterSpec { name, args }
            } else {
                FilterSpec {
                    name: filter_str.to_string(),
                    args: Vec::new(),
                }
            }
        })
        .collect()
}

/// 构建音频滤镜图
fn build_audio_filter_graph(filters: &Option<Vec<FilterSpec>>) -> Option<FilterGraph> {
    let specs = filters.as_ref()?;
    if specs.is_empty() {
        return None;
    }

    let mut graph = FilterGraph::new();

    for spec in specs {
        match spec.name.as_str() {
            "volume" => {
                let gain: f64 = spec
                    .args
                    .first()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1.0);
                let filter = tao_filter::filters::volume::VolumeFilter::new(gain);
                graph.add_filter(Box::new(filter));
                eprintln!("  [af] volume: gain={gain}");
            }
            "fade" => {
                // fade=in:start_sec:duration_sec 或 fade=out:start_sec:duration_sec
                let fade_type = spec.args.first().map(|s| s.as_str()).unwrap_or("in");
                let start: f64 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let dur: f64 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
                let ft = if fade_type == "out" {
                    tao_filter::filters::fade::FadeType::Out
                } else {
                    tao_filter::filters::fade::FadeType::In
                };
                let filter = tao_filter::filters::fade::FadeFilter::new(ft, start, dur);
                graph.add_filter(Box::new(filter));
                eprintln!("  [af] fade: type={fade_type}, start={start}s, duration={dur}s");
            }
            other => {
                eprintln!("  [af] 未知滤镜: {other}, 跳过");
            }
        }
    }

    if graph.filter_names().is_empty() {
        None
    } else {
        Some(graph)
    }
}

/// 构建视频滤镜图
fn build_video_filter_graph(filters: &Option<Vec<FilterSpec>>) -> Option<FilterGraph> {
    let specs = filters.as_ref()?;
    if specs.is_empty() {
        return None;
    }

    let mut graph = FilterGraph::new();

    for spec in specs {
        match spec.name.as_str() {
            "crop" => {
                // crop=width:height:x:y
                let w: u32 = spec.args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let h: u32 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let x: u32 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: u32 = spec.args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                if w > 0 && h > 0 {
                    let filter = tao_filter::filters::crop::CropFilter::new(x, y, w, h);
                    graph.add_filter(Box::new(filter));
                    eprintln!("  [vf] crop: {w}x{h}+{x}+{y}");
                }
            }
            "pad" => {
                // pad=width:height:x:y:color (color 可选)
                let w: u32 = spec.args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let h: u32 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let x: u32 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: u32 = spec.args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                if w > 0 && h > 0 {
                    let filter = tao_filter::filters::pad::PadFilter::new(w, h, x, y);
                    graph.add_filter(Box::new(filter));
                    eprintln!("  [vf] pad: {w}x{h}+{x}+{y}");
                }
            }
            "fade" => {
                let fade_type = spec.args.first().map(|s| s.as_str()).unwrap_or("in");
                let start: f64 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let dur: f64 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
                let ft = if fade_type == "out" {
                    tao_filter::filters::fade::FadeType::Out
                } else {
                    tao_filter::filters::fade::FadeType::In
                };
                let filter = tao_filter::filters::fade::FadeFilter::new(ft, start, dur);
                graph.add_filter(Box::new(filter));
                eprintln!("  [vf] fade: type={fade_type}, start={start}s, duration={dur}s");
            }
            other => {
                eprintln!("  [vf] 未知滤镜: {other}, 跳过");
            }
        }
    }

    if graph.filter_names().is_empty() {
        None
    } else {
        Some(graph)
    }
}

// ============================================================
// 解析辅助
// ============================================================

/// 解析分辨率字符串 (如 "1280x720")
fn parse_size(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().ok()?;
        let h = parts[1].parse().ok()?;
        Some((w, h))
    } else {
        None
    }
}

/// 解析帧率字符串 (如 "25" 或 "30000/1001")
fn parse_rate(s: &str) -> Option<Rational> {
    if let Some(slash) = s.find('/') {
        let num: i32 = s[..slash].parse().ok()?;
        let den: i32 = s[slash + 1..].parse().ok()?;
        Some(Rational::new(num, den))
    } else {
        let fps: f64 = s.parse().ok()?;
        if fps > 0.0 {
            Some(Rational::new((fps * 1000.0) as i32, 1000))
        } else {
            None
        }
    }
}

/// PTS 转秒
fn pts_to_sec(pts: i64, num: i32, den: i32) -> f64 {
    if den == 0 {
        return 0.0;
    }
    pts as f64 * num as f64 / den as f64
}

/// 根据 CodecId 获取对应的采样格式
fn codec_id_to_sample_format(codec_id: CodecId) -> Option<SampleFormat> {
    match codec_id {
        CodecId::PcmU8 => Some(SampleFormat::U8),
        CodecId::PcmS16le | CodecId::PcmS16be => Some(SampleFormat::S16),
        CodecId::PcmS24le => Some(SampleFormat::S32),
        CodecId::PcmS32le => Some(SampleFormat::S32),
        CodecId::PcmF32le => Some(SampleFormat::F32),
        CodecId::Aac => Some(SampleFormat::F32),
        CodecId::Flac => Some(SampleFormat::S16),
        _ => None,
    }
}

/// 解析编解码器名称为 CodecId
fn parse_codec_name(name: &str) -> CodecId {
    match name.to_lowercase().as_str() {
        "pcm_u8" => CodecId::PcmU8,
        "pcm_s16le" => CodecId::PcmS16le,
        "pcm_s16be" => CodecId::PcmS16be,
        "pcm_s24le" => CodecId::PcmS24le,
        "pcm_s32le" => CodecId::PcmS32le,
        "pcm_f32le" => CodecId::PcmF32le,
        "rawvideo" => CodecId::RawVideo,
        "aac" => CodecId::Aac,
        "flac" => CodecId::Flac,
        "mp3" => CodecId::Mp3,
        other => {
            eprintln!("警告: 未知编解码器 '{other}', 使用默认");
            CodecId::PcmS16le
        }
    }
}

// ============================================================
// UI
// ============================================================

/// 打印版本横幅
fn print_banner() {
    println!(
        "tao 版本 {} -- 纯 Rust 多媒体转码工具",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("用法: tao -i <输入文件> -o <输出文件> [选项]");
    println!();
    println!("选项:");
    println!("  -i <文件>           输入文件路径");
    println!("  -o <文件>           输出文件路径");
    println!("  -c <编解码器>       音频编解码器 (copy/pcm_s16le/pcm_f32le/aac/flac/...)");
    println!("  --vcodec <编解码器> 视频编解码器 (copy/rawvideo/...)");
    println!("  --ar <频率>         目标采样率 (Hz)");
    println!("  --ac <声道数>       目标声道数");
    println!("  -s <宽x高>          目标视频分辨率 (如 1280x720)");
    println!("  -r <帧率>           目标帧率 (如 25 或 30000/1001)");
    println!("  --vf <滤镜链>       视频滤镜 (如 crop=640:480:0:0,pad=800:600:80:60)");
    println!("  --af <滤镜链>       音频滤镜 (如 volume=0.5,fade=in:0:3)");
    println!("  -t <秒>             持续时间限制");
    println!("  --ss <秒>           起始时间偏移");
    println!("  -y                  覆盖输出文件");
    println!("  --build-info        显示构建信息");
    println!();
    println!("示例:");
    println!("  tao -i input.wav -o output.wav -c pcm_f32le         转换 PCM 格式");
    println!("  tao -i input.wav -o output.wav -c copy               直接复制");
    println!("  tao -i input.wav -o output.wav --ar 48000            重采样到 48kHz");
    println!("  tao -i input.wav -o output.wav --ac 1                转为单声道");
    println!("  tao -i input.mkv -o output.mkv --vcodec rawvideo     视频转码");
    println!("  tao -i input.mkv -o output.mkv --vcodec copy         视频直接复制");
    println!("  tao -i input.mkv -o output.mkv -s 640x480            视频缩放");
    println!("  tao -i input.wav -o output.wav --af volume=0.5       音量调节");
    println!("  tao -i input.mkv -o output.mkv --vf crop=640:480:0:0 视频裁剪");
    println!("  tao -i input.wav -o output.wav --ss 10 -t 30         截取 10s-40s");
    println!();
    println!("使用 --help 查看完整用法.");
}

/// 打印构建信息
fn print_build_info() {
    println!("tao 版本 {}", env!("CARGO_PKG_VERSION"));
    println!("  构建目标: {}", std::env::consts::ARCH);
    println!("  操作系统: {}", std::env::consts::OS);
    println!("  编译器: rustc");
    println!();
    println!("已注册编解码器:");
    let mut codec_registry = CodecRegistry::new();
    tao_codec::register_all(&mut codec_registry);
    let decoders = codec_registry.list_decoders();
    let encoders = codec_registry.list_encoders();
    println!("  解码器 ({}):", decoders.len());
    for (id, name) in &decoders {
        println!("    {name} ({id})");
    }
    println!("  编码器 ({}):", encoders.len());
    for (id, name) in &encoders {
        println!("    {name} ({id})");
    }
    println!();
    println!("已注册容器格式:");
    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);
    let demuxers = format_registry.list_demuxers();
    let muxers = format_registry.list_muxers();
    println!("  解封装器 ({}):", demuxers.len());
    for (id, name) in &demuxers {
        println!("    {name} ({id})");
    }
    println!("  封装器 ({}):", muxers.len());
    for (id, name) in &muxers {
        println!("    {name} ({id})");
    }
}
