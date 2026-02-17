//! 播放器核心逻辑.
//!
//! 实现 demux -> decode 管线.
//! 解码后的视频帧通过 bounded channel 传递给 GUI 线程,
//! 由 GUI 线程的 video_refresh 状态机控制显示时机 (对齐 ffplay 架构).
//! A/V 同步以音频时钟为主.

use log::{debug, info, warn};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use tao_codec::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use tao_codec::frame::Frame;
use tao_core::{MediaType, PixelFormat, SampleFormat, TaoError};
use tao_format::demuxer::SeekFlags;
use tao_format::io::IoContext;
use tao_format::registry::FormatRegistry;
use tao_format::stream::{Stream, StreamParams};

use crate::audio::{AudioChunk, AudioSender};
use crate::clock::MediaClock;

/// 音频流参数 (用于在主线程创建 SDL2 音频输出)
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u32,
}

/// 播放准备结果: (视频尺寸, 音频信息, IO 上下文, 解封装器)
type PrepareResult = (
    Option<(u32, u32)>,
    Option<AudioInfo>,
    IoContext,
    Box<dyn tao_format::demuxer::Demuxer>,
);

/// 视频帧数据 (YUV420p 平面格式, 由 SDL2 GPU 做色彩转换)
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub y_data: Vec<u8>,
    pub u_data: Vec<u8>,
    pub v_data: Vec<u8>,
    pub y_stride: usize,
    pub u_stride: usize,
    pub v_stride: usize,
    /// PTS (秒) - 用于 GUI 线程的 video_refresh 同步
    pub pts: f64,
}

/// 播放器控制命令
#[derive(Debug, Clone)]
pub enum PlayerCommand {
    TogglePause,
    /// 单步播放: 如果暂停则恢复, GUI 侧设置 step 标志显示一帧后重新暂停
    StepFrame,
    Seek(f64),
    VolumeUp,
    VolumeDown,
    ToggleMute,
    Stop,
}

/// 播放器状态更新
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PlayerStatus {
    Time(f64, f64),
    Paused(bool),
    Volume(f32),
    /// Seek 完成, GUI 应清空帧队列并重置 frame_timer
    Seeked,
    End,
    Error(String),
}

/// 播放器配置
pub struct PlayerConfig {
    pub input_path: String,
    pub no_video: bool,
    pub no_audio: bool,
    pub volume: f32,
}

/// 播放器运行所需的通道和外部资源
pub struct PlayerChannels {
    /// 视频帧发送端 (bounded, capacity=3, 匹配 ffplay VIDEO_PICTURE_QUEUE_SIZE)
    pub frame_tx: SyncSender<VideoFrame>,
    pub status_tx: Sender<PlayerStatus>,
    pub command_rx: Receiver<PlayerCommand>,
    /// 音频发送端 (可选, 由主线程创建 SDL2 音频后提供)
    pub audio_sender: Option<AudioSender>,
    /// 媒体时钟 (由主线程创建)
    pub clock: MediaClock,
}

/// 播放器
pub struct Player {
    config: PlayerConfig,
    registry: FormatRegistry,
}

impl Player {
    /// 创建播放器
    pub fn new(config: PlayerConfig) -> Result<Self, String> {
        if !is_url(&config.input_path) && !Path::new(&config.input_path).exists() {
            return Err(format!("文件不存在: {}", config.input_path));
        }

        let mut registry = FormatRegistry::new();
        tao_format::register_all(&mut registry);

        Ok(Self { config, registry })
    }

    /// 准备播放并获取视频尺寸（一次性打开文件）
    pub fn prepare_and_get_size(&self) -> Result<PrepareResult, String> {
        info!("正在打开: {}", self.config.input_path);

        let mut io = if is_url(&self.config.input_path) {
            IoContext::open_url(&self.config.input_path)
                .map_err(|e| format!("打开 URL 失败: {}", e))?
        } else {
            IoContext::open_read(&self.config.input_path)
                .map_err(|e| format!("打开文件失败: {}", e))?
        };

        let filename = extract_filename(&self.config.input_path);

        let demuxer = self
            .registry
            .open_input(&mut io, Some(filename))
            .map_err(|e| format!("探测格式失败: {}", e))?;

        let streams = demuxer.streams();

        let video_size = streams
            .iter()
            .find(|s| s.media_type == MediaType::Video)
            .and_then(|stream| {
                if let StreamParams::Video(v) = &stream.params {
                    Some((v.width, v.height))
                } else {
                    None
                }
            });

        let audio_info = if !self.config.no_audio {
            streams
                .iter()
                .find(|s| s.media_type == MediaType::Audio)
                .and_then(|stream| {
                    if let StreamParams::Audio(a) = &stream.params {
                        Some(AudioInfo {
                            sample_rate: a.sample_rate,
                            channels: a.channel_layout.channels,
                        })
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        Ok((video_size, audio_info, io, demuxer))
    }

    /// 使用预打开的 IO 和 Demuxer 在后台线程运行播放器
    pub fn run_with_prepared(
        mut self,
        io: IoContext,
        demuxer: Box<dyn tao_format::demuxer::Demuxer>,
        channels: PlayerChannels,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Err(e) = self.run_loop(Some(io), Some(demuxer), channels) {
                warn!("播放错误: {}", e);
            }
        })
    }

    fn run_loop(
        &mut self,
        pre_opened_io: Option<IoContext>,
        pre_opened_demuxer: Option<Box<dyn tao_format::demuxer::Demuxer>>,
        channels: PlayerChannels,
    ) -> Result<(), String> {
        let PlayerChannels {
            frame_tx,
            status_tx,
            command_rx,
            audio_sender,
            clock,
        } = channels;

        let (mut io, mut demuxer) =
            if let (Some(io), Some(demuxer)) = (pre_opened_io, pre_opened_demuxer) {
                (io, demuxer)
            } else {
                info!("正在打开: {}", self.config.input_path);

                let mut io = if is_url(&self.config.input_path) {
                    IoContext::open_url(&self.config.input_path)
                        .map_err(|e| format!("打开 URL 失败: {}", e))?
                } else {
                    IoContext::open_read(&self.config.input_path)
                        .map_err(|e| format!("打开文件失败: {}", e))?
                };

                let filename = extract_filename(&self.config.input_path);

                let demuxer = self
                    .registry
                    .open_input(&mut io, Some(filename))
                    .map_err(|e| format!("探测格式失败: {}", e))?;

                (io, demuxer)
            };

        let streams = demuxer.streams().to_vec();
        info!("发现 {} 条流", streams.len());

        let audio_stream = if !self.config.no_audio {
            streams.iter().find(|s| s.media_type == MediaType::Audio)
        } else {
            None
        };
        let video_stream = if !self.config.no_video {
            streams.iter().find(|s| s.media_type == MediaType::Video)
        } else {
            None
        };

        if audio_stream.is_none() && video_stream.is_none() {
            return Err("没有找到可播放的音视频流".into());
        }

        let audio_stream_idx = audio_stream.map(|s| s.index);
        let video_stream_idx = video_stream.map(|s| s.index);

        let mut audio_decoder = audio_stream.and_then(create_decoder);
        let mut video_decoder = video_stream.and_then(create_decoder);

        info!("开始播放...");
        let start_time = Instant::now();
        let mut eof = false;
        let mut frames_sent = 0u64;
        let mut current_volume = (self.config.volume * 100.0) as u32;
        let mut muted = false;
        // seek 后需要解码至少一帧 (即使暂停)
        let mut seek_flush_pending = false;

        let total_duration_sec = streams
            .iter()
            .find_map(|s| {
                if s.duration > 0 && s.time_base.den > 0 {
                    Some(s.duration as f64 * s.time_base.num as f64 / s.time_base.den as f64)
                } else {
                    None
                }
            })
            .unwrap_or(0.0);

        loop {
            // ── 处理控制命令 ──
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    PlayerCommand::TogglePause => {
                        clock.toggle_pause();
                        status_tx.send(PlayerStatus::Paused(clock.is_paused())).ok();
                    }
                    PlayerCommand::StepFrame => {
                        // 单步: 如果暂停则恢复 (GUI 侧会在显示一帧后重新暂停)
                        if clock.is_paused() {
                            clock.toggle_pause();
                            status_tx.send(PlayerStatus::Paused(false)).ok();
                        }
                    }
                    PlayerCommand::Seek(offset) => {
                        let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                        let target_sec = if total_duration_sec > 0.0 {
                            (current_sec + offset).clamp(0.0, total_duration_sec)
                        } else {
                            (current_sec + offset).max(0.0)
                        };

                        // 优先视频流 seek (关键帧对齐)
                        let seek_stream = video_stream.or(audio_stream);
                        if let Some(stream) = seek_stream {
                            let tb = &stream.time_base;
                            if tb.num > 0 && tb.den > 0 {
                                let ts = (target_sec * tb.den as f64 / tb.num as f64) as i64;
                                match demuxer.seek(&mut io, stream.index, ts, SeekFlags::default())
                                {
                                    Ok(()) => {
                                        if let Some(d) = &mut video_decoder {
                                            d.flush();
                                        }
                                        if let Some(d) = &mut audio_decoder {
                                            d.flush();
                                        }
                                        if let Some(a) = &audio_sender {
                                            a.flush();
                                        }
                                        let target_us = (target_sec * 1_000_000.0) as i64;
                                        clock.seek_reset(target_us);
                                        eof = false;
                                        seek_flush_pending = true;
                                        status_tx.send(PlayerStatus::Seeked).ok();
                                        info!("Seek 到 {:.1}s", target_sec);
                                    }
                                    Err(e) => {
                                        warn!("Seek 失败: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    PlayerCommand::VolumeUp => {
                        current_volume = (current_volume + 5).min(100);
                        muted = false;
                        status_tx
                            .send(PlayerStatus::Volume(current_volume as f32 / 100.0))
                            .ok();
                    }
                    PlayerCommand::VolumeDown => {
                        current_volume = current_volume.saturating_sub(5);
                        status_tx
                            .send(PlayerStatus::Volume(current_volume as f32 / 100.0))
                            .ok();
                    }
                    PlayerCommand::ToggleMute => {
                        muted = !muted;
                        let vol = if muted {
                            0.0
                        } else {
                            current_volume as f32 / 100.0
                        };
                        status_tx.send(PlayerStatus::Volume(vol)).ok();
                    }
                    PlayerCommand::Stop => {
                        info!("停止播放");
                        return Ok(());
                    }
                }
            }

            // ── 发送状态更新 (低频率) ──
            if frames_sent % 30 == 0 {
                let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                status_tx
                    .send(PlayerStatus::Time(current_sec, total_duration_sec))
                    .ok();
            }

            if clock.is_paused() && !seek_flush_pending {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            }

            // ── 读取数据包并解码 ──
            if !eof {
                match demuxer.read_packet(&mut io) {
                    Ok(packet) => {
                        let stream_idx = packet.stream_index;

                        // 解码音频
                        if Some(stream_idx) == audio_stream_idx {
                            if let Some(dec) = &mut audio_decoder {
                                if dec.send_packet(&packet).is_ok() {
                                    while let Ok(frame) = dec.receive_frame() {
                                        if let Frame::Audio(af) = &frame {
                                            if let Some(out) = &audio_sender {
                                                let pts_us = pts_to_us(
                                                    af.pts,
                                                    af.time_base.num,
                                                    af.time_base.den,
                                                );
                                                let mut samples = extract_f32_samples(af);
                                                let effective_volume = if muted {
                                                    0.0f32
                                                } else {
                                                    current_volume as f32 / 100.0
                                                };
                                                for s in &mut samples {
                                                    *s *= effective_volume;
                                                }
                                                let chunk = AudioChunk { samples, pts_us };
                                                let _ = out.send(chunk);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // 解码视频 → 直接入队, 由 GUI 线程控制显示时机
                        if Some(stream_idx) == video_stream_idx {
                            if let Some(dec) = &mut video_decoder {
                                if dec.send_packet(&packet).is_ok() {
                                    while let Ok(frame) = dec.receive_frame() {
                                        if let Frame::Video(vf) = &frame {
                                            let pts_us = pts_to_us(
                                                vf.pts,
                                                vf.time_base.num,
                                                vf.time_base.den,
                                            );
                                            let display_frame = build_yuv_frame(vf, pts_us);

                                            // bounded channel: 队满时阻塞, 自动背压
                                            if frame_tx.send(display_frame).is_err() {
                                                return Ok(());
                                            }
                                            frames_sent += 1;
                                            seek_flush_pending = false;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(TaoError::Eof) => {
                        info!("到达文件末尾");
                        eof = true;
                    }
                    Err(e) => {
                        debug!("读取数据包错误: {}", e);
                    }
                }
            }

            // 仅视频模式: 用系统时钟驱动
            if audio_sender.is_none() && !eof {
                let elapsed_us = start_time.elapsed().as_micros() as i64;
                clock.update_audio_pts(elapsed_us);
            }

            // 仅音频播放: EOF 后等待音频缓冲区播完
            if video_stream.is_none() && eof {
                std::thread::sleep(Duration::from_millis(2000));
                break;
            }

            if eof {
                break;
            }

            // 无视频流时的帧率限制
            if video_stream.is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        status_tx.send(PlayerStatus::End).ok();

        let elapsed = start_time.elapsed();
        info!(
            "播放结束: 发送 {} 帧, 耗时 {:.1}s",
            frames_sent,
            elapsed.as_secs_f64()
        );

        Ok(())
    }
}

// ── 辅助函数 ─────────────────────────────────────────────────────────────

/// 将 PTS 转换为微秒
fn pts_to_us(pts: i64, num: i32, den: i32) -> i64 {
    if den == 0 {
        return 0;
    }
    pts * num as i64 * 1_000_000 / den as i64
}

/// 从音频帧提取 F32 交错采样
fn extract_f32_samples(af: &tao_codec::frame::AudioFrame) -> Vec<f32> {
    match af.sample_format {
        SampleFormat::F32 => af.data[0]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        SampleFormat::S16 => af.data[0]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        SampleFormat::S32 => af.data[0]
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        _ => {
            vec![0.0f32; af.nb_samples as usize * 2]
        }
    }
}

/// 从解码后的视频帧构建 YUV420p 帧数据
fn build_yuv_frame(vf: &tao_codec::frame::VideoFrame, pts_us: i64) -> VideoFrame {
    let w = vf.width as usize;
    let h = vf.height as usize;

    match vf.pixel_format {
        PixelFormat::Yuv420p => VideoFrame {
            width: vf.width,
            height: vf.height,
            y_data: vf.data[0].clone(),
            u_data: vf.data[1].clone(),
            v_data: vf.data[2].clone(),
            y_stride: vf.linesize[0],
            u_stride: vf.linesize[1],
            v_stride: vf.linesize[2],
            pts: pts_us as f64 / 1_000_000.0,
        },
        _ => {
            let uv_w = w.div_ceil(2);
            let uv_h = h.div_ceil(2);
            VideoFrame {
                width: vf.width,
                height: vf.height,
                y_data: vec![128u8; w * h],
                u_data: vec![128u8; uv_w * uv_h],
                v_data: vec![128u8; uv_w * uv_h],
                y_stride: w,
                u_stride: uv_w,
                v_stride: uv_w,
                pts: pts_us as f64 / 1_000_000.0,
            }
        }
    }
}

/// 提取文件名 (从路径或 URL)
fn extract_filename(path: &str) -> &str {
    if is_url(path) {
        path.rsplit('/')
            .next()
            .and_then(|s| s.split('?').next())
            .unwrap_or(path)
    } else {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
    }
}

fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

/// 创建解码器
fn create_decoder(stream: &Stream) -> Option<Box<dyn tao_codec::decoder::Decoder>> {
    let mut codec_registry = tao_codec::registry::CodecRegistry::new();
    tao_codec::register_all(&mut codec_registry);
    match codec_registry.create_decoder(stream.codec_id) {
        Ok(mut dec) => {
            let params = build_codec_params(stream);
            if let Err(e) = dec.open(&params) {
                warn!("打开解码器失败: {}", e);
                None
            } else {
                Some(dec)
            }
        }
        Err(e) => {
            warn!("创建解码器失败: {}", e);
            None
        }
    }
}

/// 从流信息构建编解码器参数
fn build_codec_params(stream: &Stream) -> CodecParameters {
    match &stream.params {
        StreamParams::Audio(a) => CodecParameters {
            codec_id: stream.codec_id,
            bit_rate: a.bit_rate,
            extra_data: stream.extra_data.clone(),
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: a.sample_rate,
                channel_layout: a.channel_layout,
                sample_format: a.sample_format,
                frame_size: a.frame_size,
            }),
        },
        StreamParams::Video(v) => CodecParameters {
            codec_id: stream.codec_id,
            bit_rate: v.bit_rate,
            extra_data: stream.extra_data.clone(),
            params: CodecParamsType::Video(tao_codec::codec_parameters::VideoCodecParams {
                width: v.width,
                height: v.height,
                pixel_format: v.pixel_format,
                frame_rate: v.frame_rate,
                sample_aspect_ratio: v.sample_aspect_ratio,
            }),
        },
        _ => CodecParameters {
            codec_id: stream.codec_id,
            bit_rate: 0,
            extra_data: stream.extra_data.clone(),
            params: CodecParamsType::None,
        },
    }
}
