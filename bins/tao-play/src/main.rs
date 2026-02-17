//! # tao-play
//!
//! Tao 多媒体播放器, 对标 FFmpeg 的 ffplay.
//!
//! 支持:
//! - 音频播放 (通过 SDL2 音频子系统)
//! - 视频显示 (通过 SDL2 YUV 纹理, GPU 硬件色彩转换)
//! - A/V 同步 (基于音频时钟, ffplay 风格的 video_refresh 状态机)
//! - HTTP/HTTPS URL 播放 (通过 ureq 下载)
//! - 基本控制: 空格/P 暂停, F/双击 全屏, S 单步, ESC/Q 退出

mod audio;
mod clock;
mod gui;
mod logging;
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

    /// 日志级别 (-v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    logging::init("tao-play", args.verbose);

    info!("tao-play: 打开 {}", args.input);

    let initial_volume = args.volume.min(100) as f32 / 100.0;

    let config = PlayerConfig {
        input_path: args.input.clone(),
        no_video: args.no_video,
        no_audio: args.no_audio,
        volume: initial_volume,
    };

    let player = match Player::new(config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("初始化失败: {}", e);
            std::process::exit(1);
        }
    };

    let (video_size, audio_info, io, demuxer) = match player.prepare_and_get_size() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("准备播放失败: {}", e);
            std::process::exit(1);
        }
    };

    // ── 窗口尺寸: 对齐 ffplay 的 set_default_window_size ──
    // ffplay 默认使用视频原始尺寸 (不限制屏幕比例)
    let (video_width, video_height) = video_size.unwrap_or((640, 480));

    // ── 初始化 SDL2 ──
    // Windows 高 DPI 感知: 防止系统对窗口进行额外缩放 (对齐 ffplay)
    // 必须在 sdl2::init() 之前设置
    sdl2::hint::set("SDL_WINDOWS_DPI_AWARENESS", "permonitorv2");

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;

    // 纹理缩放质量: 双线性插值 (对齐 ffplay: SDL_HINT_RENDER_SCALE_QUALITY = "linear")
    sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "linear");

    info!(
        "视频尺寸 {}x{}, 窗口 {}x{}",
        video_width, video_height, video_width, video_height
    );

    // ── 窗口标题: 使用输入文件名 (对齐 ffplay) ──
    let window_title = std::path::Path::new(&args.input)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&args.input);

    // ── 渲染器: 硬件加速 + VSync (对齐 ffplay) ──
    // ffplay: SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC, 失败回退到 0
    // 注意: into_canvas() 消耗 Window, VSync 失败时需要重建窗口
    let canvas = {
        let window = video_subsystem
            .window(window_title, video_width, video_height)
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| e.to_string())?;
        match window.into_canvas().accelerated().present_vsync().build() {
            Ok(c) => c,
            Err(_) => {
                log::warn!("VSync 渲染器创建失败, 回退到无 VSync");
                let window = video_subsystem
                    .window(window_title, video_width, video_height)
                    .position_centered()
                    .resizable()
                    .build()
                    .map_err(|e| e.to_string())?;
                window
                    .into_canvas()
                    .accelerated()
                    .build()
                    .map_err(|e| e.to_string())?
            }
        }
    };

    // ── 创建媒体时钟 ──
    let clock = MediaClock::new();

    // ── 创建 SDL2 音频输出 ──
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

    // ── 创建通道 ──
    // 视频帧: bounded channel (capacity=3, 匹配 ffplay VIDEO_PICTURE_QUEUE_SIZE)
    let (frame_tx, frame_rx) = mpsc::sync_channel(3);
    let (status_tx, status_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    // ── 启动 player 线程 ──
    let _player_handle = player.run_with_prepared(
        io,
        demuxer,
        PlayerChannels {
            frame_tx,
            status_tx,
            command_rx,
            audio_sender,
            clock: clock.clone(),
        },
    );

    // ── 主线程: SDL2 事件循环 + video_refresh 状态机 ──
    gui::run_event_loop(canvas, frame_rx, status_rx, command_tx, clock)
}
