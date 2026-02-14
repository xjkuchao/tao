//! tao - 多媒体转码命令行工具
//!
//! 对标 FFmpeg 的 ffmpeg 命令行工具, 提供音视频转码、格式转换等功能.

use clap::Parser;
use std::process;

use tao_codec::{
    CodecId, CodecParameters, CodecRegistry, Decoder, Encoder, Frame, Packet,
    codec_parameters::{AudioCodecParams, CodecParamsType},
};
use tao_core::{MediaType, Rational, SampleFormat, TaoError};
use tao_format::stream::{AudioStreamParams, Stream, StreamParams};
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

    /// 目标采样率 (Hz)
    #[arg(long)]
    ar: Option<u32>,

    /// 目标声道数
    #[arg(long)]
    ac: Option<u32>,

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
    let is_copy = cli.acodec.as_deref() == Some("copy");
    let target_codec_id = if is_copy {
        None
    } else {
        cli.acodec.as_deref().map(parse_codec_name)
    };

    // 为每条流准备编解码器
    let mut stream_processors: Vec<Option<StreamProcessor>> = Vec::new();
    let mut output_streams: Vec<Stream> = Vec::new();

    for stream in &input_streams {
        match stream.media_type {
            MediaType::Audio => {
                if is_copy {
                    // 直接复制
                    output_streams.push(stream.clone());
                    stream_processors.push(None);
                    eprintln!("  流 #{}: 音频 -> 直接复制", stream.index);
                } else {
                    // 转码
                    let out_codec_id = target_codec_id.unwrap_or(stream.codec_id);
                    let processor = create_audio_processor(
                        stream,
                        out_codec_id,
                        &codec_registry,
                        cli.ar,
                        cli.ac,
                    );
                    match processor {
                        Ok((proc, out_stream)) => {
                            eprintln!(
                                "  流 #{}: 音频 {} -> {}",
                                stream.index, stream.codec_id, out_codec_id
                            );
                            output_streams.push(out_stream);
                            stream_processors.push(Some(proc));
                        }
                        Err(e) => {
                            eprintln!("错误: 无法创建流 #{} 的编解码器: {e}", stream.index);
                            process::exit(1);
                        }
                    }
                }
            }
            _ => {
                // 暂不支持视频/字幕转码, 跳过
                eprintln!(
                    "  流 #{}: {} -> 跳过 (暂不支持)",
                    stream.index, stream.media_type
                );
                stream_processors.push(None);
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

    // 处理循环: demux → (decode → encode) → mux
    let mut packet_count = 0u64;
    let mut byte_count = 0u64;

    loop {
        match demuxer.read_packet(&mut input_io) {
            Ok(input_pkt) => {
                let stream_idx = input_pkt.stream_index;

                // 检查此流是否被输出
                let out_stream_idx = output_streams.iter().position(|s| s.index == stream_idx);
                let out_stream_idx = match out_stream_idx {
                    Some(idx) => idx,
                    None => continue, // 跳过未输出的流
                };

                if let Some(ref mut processor) = stream_processors[stream_idx] {
                    // 转码路径: decode → resample → encode → mux
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
                } else {
                    // 直接复制路径
                    let mut out_pkt = input_pkt.clone();
                    out_pkt.stream_index = out_stream_idx;
                    if let Err(e) = muxer.write_packet(&mut output_io, &out_pkt) {
                        eprintln!("错误: 写入数据包失败: {e}");
                        process::exit(1);
                    }
                    packet_count += 1;
                    byte_count += out_pkt.size() as u64;
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

/// 流处理器: 解码器 + 重采样器 + 编码器
struct StreamProcessor {
    decoder: Box<dyn Decoder>,
    encoder: Box<dyn Encoder>,
    resampler: Option<ResampleContext>,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
}

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
                let frame_to_encode = if let Some(ref resampler) = proc.resampler {
                    resample_frame(resampler, &frame, proc.dst_channels, proc.dst_sample_format)?
                } else {
                    frame
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

/// 重采样一帧音频
fn resample_frame(
    resampler: &ResampleContext,
    frame: &Frame,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
) -> Result<Frame, TaoError> {
    match frame {
        Frame::Audio(audio) => {
            // 获取交错格式数据
            let input_data = &audio.data[0];
            let (output_data, nb_out) = resampler.convert(input_data, audio.nb_samples)?;

            let mut out_frame = tao_codec::AudioFrame::new(
                nb_out,
                resampler.dst_sample_rate,
                dst_sample_format,
                tao_core::ChannelLayout::from_channels(dst_channels),
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

/// 为音频流创建处理器
fn create_audio_processor(
    input_stream: &Stream,
    output_codec_id: CodecId,
    codec_registry: &CodecRegistry,
    target_sample_rate: Option<u32>,
    target_channels: Option<u32>,
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
    let out_channel_layout = tao_core::ChannelLayout::from_channels(out_channels);

    // 确定输出采样格式 (根据输出编解码器)
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
        dst_channels: out_channels,
        dst_sample_format: out_sample_format,
    };

    Ok((processor, out_stream))
}

/// 根据 CodecId 获取对应的采样格式
fn codec_id_to_sample_format(codec_id: CodecId) -> Option<SampleFormat> {
    match codec_id {
        CodecId::PcmU8 => Some(SampleFormat::U8),
        CodecId::PcmS16le | CodecId::PcmS16be => Some(SampleFormat::S16),
        CodecId::PcmS24le => Some(SampleFormat::S32), // S24 存为 S32
        CodecId::PcmS32le => Some(SampleFormat::S32),
        CodecId::PcmF32le => Some(SampleFormat::F32),
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
        other => {
            eprintln!("警告: 未知编解码器 '{other}', 使用默认");
            CodecId::PcmS16le
        }
    }
}

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
    println!("  -i <文件>         输入文件路径");
    println!("  -o <文件>         输出文件路径");
    println!("  -c <编解码器>     音频编解码器 (copy/pcm_s16le/pcm_f32le/...)");
    println!("  --ar <频率>       目标采样率 (Hz)");
    println!("  --ac <声道数>     目标声道数");
    println!("  -y                覆盖输出文件");
    println!("  --build-info      显示构建信息");
    println!();
    println!("示例:");
    println!("  tao -i input.wav -o output.wav -c pcm_f32le     转换 PCM 格式");
    println!("  tao -i input.wav -o output.wav -c copy           直接复制");
    println!("  tao -i input.wav -o output.wav --ar 48000        重采样到 48kHz");
    println!("  tao -i input.wav -o output.wav --ac 1            转为单声道");
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
