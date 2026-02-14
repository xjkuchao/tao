//! # tao-play
//!
//! Tao 多媒体播放器, 对标 FFmpeg 的 ffplay.
//!
//! 支持:
//! - 音频播放 (通过 cpal 跨平台音频输出)
//! - 视频显示 (通过 minifb 窗口渲染)
//! - A/V 同步 (基于音频时钟)
//! - HTTP/HTTPS URL 播放 (通过 ureq 下载)
//! - 基本控制: 空格暂停, ESC/Q 退出

mod audio;
mod clock;
mod player;
mod video;

use clap::Parser;
use log::info;

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

    /// 窗口宽度 (0 = 自动)
    #[arg(long, default_value = "0")]
    width: u32,

    /// 窗口高度 (0 = 自动)
    #[arg(long, default_value = "0")]
    height: u32,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    info!("tao-play: 打开 {}", args.input);

    let config = player::PlayerConfig {
        input_path: args.input,
        no_video: args.no_video,
        no_audio: args.no_audio,
        volume: args.volume.min(100) as f32 / 100.0,
        window_width: args.width,
        window_height: args.height,
    };

    match player::Player::new(config) {
        Ok(mut player) => {
            if let Err(e) = player.run() {
                eprintln!("播放错误: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("初始化失败: {}", e);
            std::process::exit(1);
        }
    }
}
