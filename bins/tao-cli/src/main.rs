//! tao - 多媒体转码命令行工具
//!
//! 对标 FFmpeg 的 ffmpeg 命令行工具, 提供音视频转码、格式转换等功能.

mod filter;
mod logging;
mod processor;
mod transcode;

use clap::Parser;
use std::process;

use tao_codec::CodecRegistry;
use tao_core::{MediaType, TaoError};
use tao_format::stream::{Stream, StreamParams};
use tao_format::{FormatId, FormatRegistry, IoContext, Muxer};

use filter::{parse_codec_name, parse_filter_chain, parse_rate, parse_size, pts_to_sec};
use processor::{
    StreamProcessor, create_audio_processor, create_video_processor, flush_encoder,
    transcode_packet,
};
use transcode::transcode_to_raw_yuv;

#[derive(Parser, Debug)]
#[command(name = "tao", version, about = "纯 Rust 多媒体转码工具")]
struct Cli {
    /// 输入文件路径
    #[arg(short, long)]
    input: Option<String>,

    /// 输出文件路径
    #[arg(short, long)]
    output: Option<String>,

    /// 输出原始 YUV420p 帧到文件（用于质量验证）
    #[arg(long = "output-raw")]
    output_raw: Option<String>,

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

    /// 日志级别 (-v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let cli = Cli::parse();
    logging::init("tao-cli", cli.verbose);

    if cli.build_info {
        print_build_info();
        return;
    }

    if cli.input.is_none() {
        print_banner();
        return;
    }

    let input_path = cli.input.as_ref().unwrap();

    // 如果指定了 --output-raw, 执行原始 YUV 输出
    if let Some(raw_output_path) = &cli.output_raw {
        if let Err(e) = transcode_to_raw_yuv(input_path, raw_output_path, &cli) {
            eprintln!("错误: {e}");
            process::exit(1);
        }
        return;
    }

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
    let mut input_io = match IoContext::open_url(input_path) {
        Ok(io) => io,
        Err(_) => {
            // 如果作为 URL 打开失败，尝试作为本地文件打开
            match IoContext::open_read(input_path) {
                Ok(io) => io,
                Err(e) => {
                    eprintln!("错误: 无法打开输入文件 '{input_path}': {e}");
                    process::exit(1);
                }
            }
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
