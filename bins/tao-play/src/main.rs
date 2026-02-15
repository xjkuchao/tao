//! # tao-play
//!
//! Tao 多媒体播放器, 对标 FFmpeg 的 ffplay.
//!
//! 支持:
//! - 音频播放 (通过 cpal 跨平台音频输出)
//! - 视频显示 (通过 egui/eframe 窗口渲染)
//! - A/V 同步 (基于音频时钟)
//! - HTTP/HTTPS URL 播放 (通过 ureq 下载)
//! - 基本控制: 空格暂停, ESC/Q 退出

mod audio;
mod clock;
mod gui;
mod player;

use crate::player::{Player, PlayerConfig};
use clap::Parser;
use eframe::egui;
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

fn main() -> eframe::Result<()> {
    // Force Wayland backend to avoid X11/Wayland混合issues
    unsafe {
        std::env::set_var("WINIT_UNIX_BACKEND", "wayland");
    }

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

    // Create player
    let player = match Player::new(config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("初始化失败: {}", e);
            std::process::exit(1);
        }
    };

    // Prepare and get video size (opens file once)
    let (video_size, io, demuxer) = match player.prepare_and_get_size() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("准备播放失败: {}", e);
            std::process::exit(1);
        }
    };

    let (window_width, window_height) = video_size.unwrap_or((640, 480));
    info!("窗口尺寸: {}x{}", window_width, window_height);

    // Create channels
    let (frame_tx, frame_rx) = mpsc::channel();
    let (status_tx, status_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    // Start player in background thread with pre-opened file
    let _player_handle = player.run_with_prepared(io, demuxer, frame_tx, status_tx, command_rx);

    // Run GUI with proper window size
    // Note: Window positioning on Wayland/WSL is controlled by the compositor.
    // The position hint below may be ignored - this is expected Wayland behavior.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([window_width as f32, window_height as f32])
            .with_title("tao-play")
            .with_position(egui::pos2(f32::NAN, f32::NAN)), // Request centered (compositor decides)
        ..Default::default()
    };

    eframe::run_native(
        "tao-play",
        options,
        Box::new(move |cc| {
            Box::new(gui::PlayerApp::new(
                cc,
                frame_rx,
                status_rx,
                command_tx,
                initial_volume,
            ))
        }),
    )
}
