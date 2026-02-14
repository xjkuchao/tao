//! tao-probe - 多媒体信息探测工具
//!
//! 对标 FFmpeg 的 ffprobe 命令行工具, 用于分析多媒体文件的详细信息.

use clap::Parser;
use serde::Serialize;
use std::process;

use tao_core::MediaType;
use tao_format::stream::{AudioStreamParams, StreamParams, VideoStreamParams};
use tao_format::{FormatRegistry, IoContext};

/// Tao 多媒体信息探测工具
#[derive(Parser, Debug)]
#[command(name = "tao-probe", version, about = "纯 Rust 多媒体信息探测工具")]
struct Cli {
    /// 输入文件路径
    input: Option<String>,

    /// 显示流信息
    #[arg(long, default_value_t = true)]
    show_streams: bool,

    /// 显示格式信息
    #[arg(long, default_value_t = true)]
    show_format: bool,

    /// 显示数据包信息 (会读取全部数据包)
    #[arg(long)]
    show_packets: bool,

    /// 输出 JSON 格式
    #[arg(long)]
    json: bool,

    /// 静默模式 (只输出探测结果)
    #[arg(short, long)]
    quiet: bool,
}

// ============================================================
// JSON 输出结构体
// ============================================================

/// 完整探测结果
#[derive(Serialize)]
struct ProbeOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<FormatInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    streams: Option<Vec<StreamInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    packets: Option<PacketSummary>,
}

/// 格式信息
#[derive(Serialize)]
struct FormatInfo {
    filename: String,
    format_name: String,
    nb_streams: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    probe_score: Option<u32>,
}

/// 流信息
#[derive(Serialize)]
struct StreamInfo {
    index: usize,
    codec_type: String,
    codec_name: String,
    time_base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<f64>,
    // 视频字段
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pixel_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_aspect_ratio: Option<String>,
    // 音频字段
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_format: Option<String>,
    // 通用
    #[serde(skip_serializing_if = "Option::is_none")]
    bit_rate: Option<u64>,
    nb_frames: u64,
}

/// 数据包统计
#[derive(Serialize)]
struct PacketSummary {
    total_packets: u64,
    total_bytes: u64,
}

// ============================================================
// 主逻辑
// ============================================================

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    if cli.input.is_none() {
        print_banner();
        return;
    }

    let input_path = cli.input.as_ref().unwrap();

    if !cli.quiet {
        eprintln!(
            "tao-probe 版本 {} -- 纯 Rust 多媒体探测工具",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!("输入文件: {input_path}");
    }

    // 初始化注册表
    let mut format_registry = FormatRegistry::new();
    tao_format::register_all(&mut format_registry);

    // 打开文件
    let mut io = match IoContext::open_read(input_path) {
        Ok(io) => io,
        Err(e) => {
            eprintln!("错误: 无法打开文件 '{input_path}': {e}");
            process::exit(1);
        }
    };

    // 探测格式
    let probe_result = match format_registry.probe_input(&mut io, Some(input_path)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("错误: 无法识别文件格式: {e}");
            process::exit(1);
        }
    };

    if !cli.quiet {
        eprintln!(
            "格式: {} (置信度: {})",
            probe_result.format_id, probe_result.score
        );
    }

    // 创建解封装器
    let mut demuxer = match format_registry.create_demuxer(probe_result.format_id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("错误: 无法创建解封装器: {e}");
            process::exit(1);
        }
    };

    if let Err(e) = demuxer.open(&mut io) {
        eprintln!("错误: 无法解析文件头部: {e}");
        process::exit(1);
    }

    // 收集格式信息
    let format_info = if cli.show_format {
        Some(FormatInfo {
            filename: input_path.clone(),
            format_name: probe_result.format_id.name().to_string(),
            nb_streams: demuxer.streams().len(),
            duration: demuxer.duration(),
            probe_score: Some(probe_result.score),
        })
    } else {
        None
    };

    // 收集流信息
    let streams_info = if cli.show_streams {
        Some(
            demuxer
                .streams()
                .iter()
                .map(build_stream_info)
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    // 读取数据包统计
    let packet_summary = if cli.show_packets {
        let mut total_packets = 0u64;
        let mut total_bytes = 0u64;
        loop {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    total_packets += 1;
                    total_bytes += pkt.size() as u64;
                }
                Err(tao_core::TaoError::Eof) => break,
                Err(e) => {
                    eprintln!("警告: 读取数据包时出错: {e}");
                    break;
                }
            }
        }
        Some(PacketSummary {
            total_packets,
            total_bytes,
        })
    } else {
        None
    };

    // 输出结果
    if cli.json {
        let output = ProbeOutput {
            format: format_info,
            streams: streams_info,
            packets: packet_summary,
        };
        let json = serde_json::to_string_pretty(&output).unwrap();
        println!("{json}");
    } else {
        // 文本格式输出
        if let Some(ref fmt_info) = format_info {
            print_format_text(fmt_info);
        }
        if let Some(ref streams) = streams_info {
            print_streams_text(streams);
        }
        if let Some(ref pkt_sum) = packet_summary {
            print_packets_text(pkt_sum);
        }
    }
}

/// 从 Stream 构建 StreamInfo
fn build_stream_info(stream: &tao_format::Stream) -> StreamInfo {
    let codec_type = match stream.media_type {
        MediaType::Video => "video",
        MediaType::Audio => "audio",
        MediaType::Subtitle => "subtitle",
        MediaType::Data => "data",
        MediaType::Attachment => "attachment",
    }
    .to_string();

    let duration = if stream.duration > 0 && stream.time_base.is_valid() {
        Some(stream.duration as f64 * stream.time_base.to_f64())
    } else {
        None
    };

    let mut info = StreamInfo {
        index: stream.index,
        codec_type,
        codec_name: format!("{}", stream.codec_id),
        time_base: format!("{}/{}", stream.time_base.num, stream.time_base.den),
        duration,
        width: None,
        height: None,
        pixel_format: None,
        frame_rate: None,
        sample_aspect_ratio: None,
        sample_rate: None,
        channels: None,
        channel_layout: None,
        sample_format: None,
        bit_rate: None,
        nb_frames: stream.nb_frames,
    };

    match &stream.params {
        StreamParams::Video(VideoStreamParams {
            width,
            height,
            pixel_format,
            frame_rate,
            sample_aspect_ratio,
            bit_rate,
        }) => {
            info.width = Some(*width);
            info.height = Some(*height);
            info.pixel_format = Some(format!("{pixel_format}"));
            if frame_rate.is_valid() {
                info.frame_rate = Some(format!("{}/{}", frame_rate.num, frame_rate.den));
            }
            if sample_aspect_ratio.is_valid() {
                info.sample_aspect_ratio = Some(format!(
                    "{}/{}",
                    sample_aspect_ratio.num, sample_aspect_ratio.den
                ));
            }
            if *bit_rate > 0 {
                info.bit_rate = Some(*bit_rate);
            }
        }
        StreamParams::Audio(AudioStreamParams {
            sample_rate,
            channel_layout,
            sample_format,
            bit_rate,
            ..
        }) => {
            info.sample_rate = Some(*sample_rate);
            info.channels = Some(channel_layout.channels);
            info.channel_layout = Some(format!("{channel_layout}"));
            info.sample_format = Some(format!("{sample_format}"));
            if *bit_rate > 0 {
                info.bit_rate = Some(*bit_rate);
            }
        }
        _ => {}
    }

    info
}

/// 文本输出: 格式信息
fn print_format_text(info: &FormatInfo) {
    println!("[FORMAT]");
    println!("  文件名       : {}", info.filename);
    println!("  格式名称     : {}", info.format_name);
    println!("  流数量       : {}", info.nb_streams);
    if let Some(dur) = info.duration {
        println!("  时长         : {dur:.3} 秒");
    }
    if let Some(score) = info.probe_score {
        println!("  探测置信度   : {score}");
    }
    println!("[/FORMAT]");
    println!();
}

/// 文本输出: 流信息
fn print_streams_text(streams: &[StreamInfo]) {
    for stream in streams {
        println!("[STREAM #{}]", stream.index);
        println!("  类型         : {}", stream.codec_type);
        println!("  编解码器     : {}", stream.codec_name);
        println!("  时间基       : {}", stream.time_base);
        if let Some(dur) = stream.duration {
            println!("  时长         : {dur:.3} 秒");
        }

        // 视频特有
        if let (Some(w), Some(h)) = (stream.width, stream.height) {
            println!("  分辨率       : {w}x{h}");
        }
        if let Some(ref pf) = stream.pixel_format {
            println!("  像素格式     : {pf}");
        }
        if let Some(ref fr) = stream.frame_rate {
            println!("  帧率         : {fr}");
        }
        if let Some(ref sar) = stream.sample_aspect_ratio {
            println!("  SAR          : {sar}");
        }

        // 音频特有
        if let Some(sr) = stream.sample_rate {
            println!("  采样率       : {sr} Hz");
        }
        if let Some(ch) = stream.channels {
            println!("  声道数       : {ch}");
        }
        if let Some(ref cl) = stream.channel_layout {
            println!("  声道布局     : {cl}");
        }
        if let Some(ref sf) = stream.sample_format {
            println!("  采样格式     : {sf}");
        }

        // 通用
        if let Some(br) = stream.bit_rate {
            println!("  码率         : {} kbps", br / 1000);
        }
        if stream.nb_frames > 0 {
            println!("  帧数         : {}", stream.nb_frames);
        }
        println!("[/STREAM]");
        println!();
    }
}

/// 文本输出: 数据包统计
fn print_packets_text(summary: &PacketSummary) {
    println!("[PACKETS]");
    println!("  数据包总数   : {}", summary.total_packets);
    println!(
        "  数据总量     : {} 字节 ({:.2} KB)",
        summary.total_bytes,
        summary.total_bytes as f64 / 1024.0
    );
    println!("[/PACKETS]");
    println!();
}

/// 打印版本横幅
fn print_banner() {
    println!(
        "tao-probe 版本 {} -- 纯 Rust 多媒体探测工具",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("用法: tao-probe [选项] <输入文件>");
    println!();
    println!("选项:");
    println!("  --show-streams    显示流信息 (默认开启)");
    println!("  --show-format     显示格式信息 (默认开启)");
    println!("  --show-packets    显示数据包统计");
    println!("  --json            以 JSON 格式输出");
    println!("  -q, --quiet       静默模式");
    println!();
    println!("使用 --help 查看完整用法.");
}
