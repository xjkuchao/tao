//! # tao-play
//!
//! Tao 多媒体播放器, 对标 FFmpeg 的 ffplay.
//!
//! 支持:
//! - 音频播放 (通过 SDL2 音频子系统)
//! - 视频显示 (通过 SDL2 YUV 纹理, GPU 硬件色彩转换)
//! - A/V 同步 (基于音频时钟)
//! - HTTP/HTTPS URL 播放 (通过 ureq 下载)
//! - 基本控制: 空格暂停, ESC/Q 退出

mod audio;
mod clock;
mod gui;
mod player;

use crate::audio::AudioOutput;
use crate::clock::MediaClock;
use crate::player::{Player, PlayerChannels, PlayerConfig};
use clap::Parser;
use log::info;
use std::sync::mpsc;

/// Tao 多媒体播放器 (对标 ffplay)
#[derive(Parser)]
#[command(name = "tao-play", about = "Tao 多媒体播放器")]
struct Args {
    /// 输入文件路径或 URL (支持 http/https)
    input: String,

    /// 是否禁用视频
    #[arg(long = "novideo", help = "禁用视频播放")]
    no_video: bool,

    /// 是否禁用音频
    #[arg(long = "noaudio", help = "禁用音频播放")]
    no_audio: bool,

    /// 音量 (0-100, 默认 100)
    #[arg(long, default_value = "100")]
    volume: u32,
}

fn main() -> Result<(), String> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    info!("tao-play: 打开 {}", args.input);

    let initial_volume = args.volume.min(100) as f32 / 100.0;

    let config = PlayerConfig {
        input_path: args.input.clone(),
        no_video: args.no_video,
        no_audio: args.no_audio,
        volume: initial_volume,
    };

    // 创建播放器
    let player = match Player::new(config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("初始化失败: {}", e);
            std::process::exit(1);
        }
    };

    // 准备播放并获取视频/音频信息 (一次性打开文件)
    let (video_size, audio_info, io, demuxer) = match player.prepare_and_get_size() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("准备播放失败: {}", e);
            std::process::exit(1);
        }
    };

    let (window_width, window_height) = video_size.unwrap_or((640, 480));
    info!("窗口尺寸: {}x{}", window_width, window_height);

    // 初始化 SDL2
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;

    // 创建窗口 + 渲染器
    let window = video_subsystem
        .window("tao-play", window_width, window_height)
        .position_centered()
        .resizable()
        .build()
        .map_err(|e| e.to_string())?;

    let canvas = window
        .into_canvas()
        .accelerated()
        .build()
        .map_err(|e| e.to_string())?;

    // 创建媒体时钟
    let clock = MediaClock::new();

    // 在主线程创建 SDL2 音频输出 (SDL2 要求)
    // _audio_output 必须保留在主线程 (持有 SDL2 设备), audio_sender 传给 player 线程
    let (_audio_output, audio_sender) = if let Some(ai) = &audio_info {
        match AudioOutput::new(&audio_subsystem, ai.sample_rate, ai.channels, clock.clone()) {
            Ok((out, sender)) => (Some(out), Some(sender)),
            Err(e) => {
                log::warn!("创建音频输出失败: {}", e);
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // 创建通道
    let (frame_tx, frame_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    // 在后台线程启动播放器 (audio_sender 可跨线程, _audio_output 留在主线程)
    let _player_handle = player.run_with_prepared(
        io,
        demuxer,
        PlayerChannels {
            frame_tx,
            status_tx,
            command_rx,
            audio_sender,
            clock,
        },
    );

    // 主线程运行 SDL2 事件循环
    gui::run_event_loop(canvas, frame_rx, status_rx, command_tx)
}
