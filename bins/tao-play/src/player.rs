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

use tao_codec::CodecId;
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
    Muted(bool),
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
        let audio_nominal_bits = audio_stream.and_then(resolve_audio_nominal_bits);

        // 音频时钟: 用解码输出的累计采样数计算, 不依赖 demuxer PTS
        // (AVI 等容器的音频 PTS 可能不准确, 特别是压缩音频)
        let audio_sample_rate = audio_stream
            .and_then(|s| {
                if let StreamParams::Audio(a) = &s.params {
                    Some(a.sample_rate)
                } else {
                    None
                }
            })
            .unwrap_or(44100);
        let mut audio_cum_samples: u64 = 0;

        info!("开始播放...");
        let start_time = Instant::now();
        let mut eof = false;
        let mut frames_sent = 0u64;
        let mut current_volume = (self.config.volume * 100.0) as u32;
        let mut muted = false;
        // seek 后需要解码至少一帧 (即使暂停)
        let mut seek_flush_pending = false;
        // seek 后立即 EOF 的重试标记 (防止无限循环)
        let mut seek_eof_retried = false;
        // EOF 回退重试时: 跳过前面的帧 (仅构建参考帧), 只显示此 PTS 之后的帧
        let mut seek_skip_until: Option<f64> = None;
        // 仅音频且总时长未知时, EOF 后给设备缓冲一个短暂排空窗口
        let mut audio_eof_wait_start: Option<Instant> = None;

        let total_duration_sec = streams
            .iter()
            .find_map(|s| {
                if s.duration > 0 && s.time_base.den > 0 {
                    Some(s.duration as f64 * s.time_base.num as f64 / s.time_base.den as f64)
                } else {
                    None
                }
            })
            .or_else(|| demuxer.duration())
            .unwrap_or(0.0);

        // seek 上限: 减去一帧, 避免 seek 到 duration 边界导致无帧可解码
        let seek_end_margin = video_stream
            .map(|s| s.time_base.num as f64 / s.time_base.den as f64)
            .unwrap_or(0.1);
        let max_seekable_sec = (total_duration_sec - seek_end_margin).max(0.0);

        if let Some(a) = &audio_sender {
            a.set_volume(current_volume as f32 / 100.0);
            a.set_muted(muted);
        }

        'main: loop {
            // ── 处理控制命令 ──
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    PlayerCommand::TogglePause => {
                        clock.toggle_pause();
                        let paused = clock.is_paused();
                        let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                        info!(
                            "[控制] 切换暂停: paused={}, 时钟={:.3}s",
                            paused, current_sec
                        );
                        status_tx.send(PlayerStatus::Paused(paused)).ok();
                    }
                    PlayerCommand::StepFrame => {
                        // 单步: 如果暂停则恢复 (GUI 侧会在显示一帧后重新暂停)
                        if clock.is_paused() {
                            clock.toggle_pause();
                            status_tx.send(PlayerStatus::Paused(false)).ok();
                        }
                    }
                    PlayerCommand::Seek(offset) => {
                        seek_eof_retried = false;
                        seek_skip_until = None;
                        let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                        let is_paused = clock.is_paused();
                        let target_sec = if total_duration_sec > 0.0 {
                            (current_sec + offset).clamp(0.0, max_seekable_sec)
                        } else {
                            (current_sec + offset).max(0.0)
                        };

                        info!(
                            "[Seek] offset={:+.1}s, 时钟={:.3}s, 目标={:.3}s, 总时长={:.1}s, 暂停={}",
                            offset, current_sec, target_sec, total_duration_sec, is_paused
                        );

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
                                        // 重置音频采样计数器
                                        audio_cum_samples =
                                            (target_sec * audio_sample_rate as f64) as u64;
                                        eof = false;
                                        audio_eof_wait_start = None;
                                        seek_flush_pending = true;
                                        // Seeked 延迟到首帧解码后发送, 避免 GUI 提前清空帧队列
                                        info!(
                                            "[Seek] 成功: demuxer 定位到 ts={}, 流#{}, 已发送帧={}",
                                            ts, stream.index, frames_sent
                                        );
                                    }
                                    Err(e) => {
                                        warn!("[Seek] 失败: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    PlayerCommand::VolumeUp => {
                        current_volume = (current_volume + 5).min(100);
                        muted = false;
                        if let Some(a) = &audio_sender {
                            a.set_volume(current_volume as f32 / 100.0);
                            a.set_muted(false);
                        }
                        status_tx
                            .send(PlayerStatus::Volume(current_volume as f32 / 100.0))
                            .ok();
                        status_tx.send(PlayerStatus::Muted(false)).ok();
                    }
                    PlayerCommand::VolumeDown => {
                        current_volume = current_volume.saturating_sub(5);
                        if let Some(a) = &audio_sender {
                            a.set_volume(current_volume as f32 / 100.0);
                        }
                        status_tx
                            .send(PlayerStatus::Volume(current_volume as f32 / 100.0))
                            .ok();
                    }
                    PlayerCommand::ToggleMute => {
                        muted = !muted;
                        if let Some(a) = &audio_sender {
                            a.set_muted(muted);
                        }
                        status_tx
                            .send(PlayerStatus::Volume(current_volume as f32 / 100.0))
                            .ok();
                        status_tx.send(PlayerStatus::Muted(muted)).ok();
                    }
                    PlayerCommand::Stop => {
                        info!("停止播放");
                        break 'main;
                    }
                }
            }

            // ── 发送状态更新 (低频率) ──
            if frames_sent % 30 == 0 {
                let mut current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                if total_duration_sec > 0.0 {
                    current_sec = current_sec.min(total_duration_sec);
                }
                status_tx
                    .send(PlayerStatus::Time(current_sec, total_duration_sec))
                    .ok();
            }

            let is_paused = clock.is_paused();
            if is_paused && !seek_flush_pending {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            }

            // ── 读取数据包并解码 ──
            if !eof {
                match demuxer.read_packet(&mut io) {
                    Ok(packet) => {
                        let stream_idx = packet.stream_index;

                        // 解码音频 (seek_pending 期间时钟更新已被阻止, 无需跳过音频)
                        if Some(stream_idx) == audio_stream_idx {
                            if let Some(dec) = &mut audio_decoder {
                                if dec.send_packet(&packet).is_ok() {
                                    while let Ok(frame) = dec.receive_frame() {
                                        if let Frame::Audio(af) = &frame {
                                            let nb = af.nb_samples as u64;
                                            let chunk_pts_us = (audio_cum_samples as f64
                                                / audio_sample_rate as f64
                                                * 1_000_000.0)
                                                as i64;
                                            // 跳过阶段: 只累计不发送 (避免播放错位音频)
                                            if seek_skip_until.is_some() {
                                                audio_cum_samples += nb;
                                                continue;
                                            }
                                            if let Some(out) = &audio_sender {
                                                let samples =
                                                    extract_f32_samples(af, audio_nominal_bits);
                                                let chunk = AudioChunk {
                                                    samples,
                                                    pts_us: chunk_pts_us,
                                                };
                                                if out.send(chunk).is_err() {
                                                    break 'main;
                                                }
                                            }
                                            // 仅音频流 seek: 首个音频块即可确认 seek 完成.
                                            if seek_flush_pending && video_stream.is_none() {
                                                status_tx.send(PlayerStatus::Seeked).ok();
                                                clock.confirm_seek();
                                                seek_flush_pending = false;
                                                info!(
                                                    "[Seek] 首个音频块已发送, 确认时钟: PTS={:.3}s",
                                                    chunk_pts_us as f64 / 1_000_000.0
                                                );
                                            }
                                            audio_cum_samples += nb;
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
                                            let frame_pts = pts_us as f64 / 1_000_000.0;

                                            // 跳过阶段: 仅解码构建参考帧, 不入队
                                            if let Some(threshold) = seek_skip_until {
                                                if frame_pts < threshold {
                                                    continue;
                                                }
                                                // 到达显示阈值: 重置时钟和音频采样到此位置
                                                seek_skip_until = None;
                                                let display_us = (frame_pts * 1_000_000.0) as i64;
                                                clock.seek_reset(display_us);
                                                audio_cum_samples =
                                                    (frame_pts * audio_sample_rate as f64) as u64;
                                                info!(
                                                    "[Seek] 跳过完成: 从 PTS={:.3}s 开始显示",
                                                    frame_pts
                                                );
                                            }

                                            let display_frame = build_yuv_frame(vf, pts_us);
                                            if seek_flush_pending {
                                                // 通知 GUI 清空旧帧 (此时首帧已就绪)
                                                status_tx.send(PlayerStatus::Seeked).ok();
                                                info!(
                                                    "[Seek] 首帧已发送: PTS={:.3}s, 确认时钟",
                                                    frame_pts
                                                );
                                                clock.confirm_seek();
                                                seek_flush_pending = false;
                                            }
                                            // bounded channel: 队满时阻塞, 自动背压
                                            if frame_tx.send(display_frame).is_err() {
                                                break 'main;
                                            }
                                            frames_sent += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(TaoError::Eof) => {
                        if seek_flush_pending && !seek_eof_retried && video_stream.is_some() {
                            // seek 到末尾后立即 EOF 且未解码出帧:
                            // 可能 idx1 keyframe 标记不准确, 从较近位置回退重试
                            seek_eof_retried = true;
                            let seek_stream = video_stream.or(audio_stream);
                            let mut retried = false;
                            if let Some(stream) = seek_stream {
                                let tb = &stream.time_base;
                                if tb.num > 0 && tb.den > 0 {
                                    // 只显示最后 ~0.3 秒帧, 从显示点前 1 秒开始解码构建参考帧
                                    let skip_threshold = (max_seekable_sec - 0.3).max(0.0);
                                    let retry_sec = (skip_threshold - 1.0).max(0.0);
                                    let ts = (retry_sec * tb.den as f64 / tb.num as f64) as i64;
                                    if demuxer
                                        .seek(&mut io, stream.index, ts, SeekFlags::default())
                                        .is_ok()
                                    {
                                        if let Some(d) = &mut video_decoder {
                                            d.flush();
                                        }
                                        if let Some(d) = &mut audio_decoder {
                                            d.flush();
                                        }
                                        if let Some(a) = &audio_sender {
                                            a.flush();
                                        }
                                        let retry_us = (retry_sec * 1_000_000.0) as i64;
                                        clock.seek_reset(retry_us);
                                        audio_cum_samples =
                                            (retry_sec * audio_sample_rate as f64) as u64;
                                        seek_skip_until = Some(skip_threshold);
                                        audio_eof_wait_start = None;
                                        info!(
                                            "[Seek] EOF 回退: 从 {:.3}s 解码, 跳过至 {:.3}s 后显示",
                                            retry_sec, skip_threshold
                                        );
                                        retried = true;
                                    }
                                }
                            }
                            if !retried {
                                debug!("demuxer 读取完成 (EOF)");
                                eof = true;
                                audio_eof_wait_start = Some(Instant::now());
                            }
                        } else {
                            debug!("demuxer 读取完成 (EOF)");
                            eof = true;
                            audio_eof_wait_start = Some(Instant::now());
                        }
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

            // 仅音频播放: demux 可能提前到 EOF, 需等时钟接近总时长再结束.
            if video_stream.is_none() && eof {
                if total_duration_sec > 0.0 {
                    let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                    let remain_sec = (total_duration_sec - current_sec).max(0.0);
                    if remain_sec > 0.2 {
                        std::thread::sleep(Duration::from_millis(16));
                        continue;
                    }
                } else {
                    // 未知总时长时, 给音频设备短暂排空窗口后进入 EOF 态.
                    let elapsed = audio_eof_wait_start
                        .map(|t| t.elapsed())
                        .unwrap_or_else(|| Duration::from_millis(0));
                    if elapsed < Duration::from_millis(250) {
                        std::thread::sleep(Duration::from_millis(16));
                        continue;
                    }
                }
            }

            if eof {
                // seek 后无帧可解码就 EOF: 补发 Seeked 保持 GUI 状态一致
                if seek_flush_pending {
                    status_tx.send(PlayerStatus::Seeked).ok();
                    clock.confirm_seek();
                    seek_flush_pending = false;
                }
                // 跟踪 GUI 侧暂停状态 (进入 EOF 前 clock 状态即 GUI 已知状态)
                let mut eof_gui_paused = clock.is_paused();
                // 暂停时钟, 防止 EOF 期间漂移
                if !eof_gui_paused {
                    clock.set_paused(true);
                }
                let final_sec = if total_duration_sec > 0.0 {
                    total_duration_sec
                } else {
                    clock.current_time_us() as f64 / 1_000_000.0
                };
                status_tx
                    .send(PlayerStatus::Time(final_sec, total_duration_sec))
                    .ok();

                let elapsed = start_time.elapsed();
                info!(
                    "播放结束: 发送 {} 帧, 耗时 {:.1}s",
                    frames_sent,
                    elapsed.as_secs_f64()
                );
                status_tx.send(PlayerStatus::End).ok();

                // EOF 等待循环: 处理 Seek/Stop/TogglePause
                loop {
                    match command_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(PlayerCommand::Stop) => {
                            info!("停止播放");
                            break 'main;
                        }
                        Ok(PlayerCommand::Seek(offset)) => {
                            seek_eof_retried = false;
                            seek_skip_until = None;
                            // EOF 后以当前时钟为基准, 再进行总时长约束.
                            let base_sec = if total_duration_sec > 0.0 {
                                (clock.current_time_us() as f64 / 1_000_000.0)
                                    .clamp(0.0, total_duration_sec)
                            } else {
                                (clock.current_time_us() as f64 / 1_000_000.0).max(0.0)
                            };
                            let target_sec = if total_duration_sec > 0.0 {
                                (base_sec + offset).clamp(0.0, max_seekable_sec)
                            } else {
                                (base_sec + offset).max(0.0)
                            };

                            // 前进 seek 到末尾: 无意义, 忽略
                            if offset > 0.0 && target_sec >= max_seekable_sec {
                                info!("[Seek] 已在末尾, 忽略前进 (offset={:+.1}s)", offset);
                                continue;
                            }

                            info!(
                                "[Seek] EOF 后 seek: offset={:+.1}s, 目标={:.3}s",
                                offset, target_sec
                            );

                            let seek_stream = video_stream.or(audio_stream);
                            if let Some(stream) = seek_stream {
                                let tb = &stream.time_base;
                                if tb.num > 0 && tb.den > 0 {
                                    let ts = (target_sec * tb.den as f64 / tb.num as f64) as i64;
                                    match demuxer.seek(
                                        &mut io,
                                        stream.index,
                                        ts,
                                        SeekFlags::default(),
                                    ) {
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
                                            // 恢复时钟
                                            clock.set_paused(false);
                                            let target_us = (target_sec * 1_000_000.0) as i64;
                                            clock.seek_reset(target_us);
                                            // 重置音频采样计数器
                                            audio_cum_samples =
                                                (target_sec * audio_sample_rate as f64) as u64;
                                            eof = false;
                                            audio_eof_wait_start = None;
                                            seek_flush_pending = true;
                                            // Seeked 延迟到首帧解码后发送
                                            info!(
                                                "[Seek] 成功: 从 EOF 恢复, 目标={:.3}s",
                                                target_sec
                                            );
                                        }
                                        Err(e) => {
                                            warn!("[Seek] 失败: {}", e);
                                        }
                                    }
                                }
                            }
                            if !eof {
                                break; // 跳出 EOF 等待循环, 恢复主循环
                            }
                        }
                        Ok(PlayerCommand::TogglePause) => {
                            // EOF 后切换暂停: 用独立变量跟踪 GUI 状态 (时钟已被强制暂停)
                            eof_gui_paused = !eof_gui_paused;
                            status_tx.send(PlayerStatus::Paused(eof_gui_paused)).ok();
                        }
                        Ok(_) => {}  // 忽略其他命令
                        Err(_) => {} // 超时, 继续等待
                    }
                }
                continue; // 跳过主循环末尾, 回到顶部
            }

            // 无视频流时的帧率限制
            if video_stream.is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

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
fn extract_f32_samples(af: &tao_codec::frame::AudioFrame, nominal_bits: Option<u32>) -> Vec<f32> {
    match af.sample_format {
        SampleFormat::F32 => af.data[0]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        SampleFormat::S16 => af.data[0]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        SampleFormat::S32 => {
            let scale = match nominal_bits {
                Some(bits) if (1..32).contains(&bits) => (1u64 << (bits - 1)) as f32,
                _ => 2_147_483_648.0,
            };
            af.data[0]
                .chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / scale)
                .collect()
        }
        _ => {
            vec![0.0f32; af.nb_samples as usize * af.channel_layout.channels as usize]
        }
    }
}

fn resolve_audio_nominal_bits(stream: &Stream) -> Option<u32> {
    if stream.codec_id == CodecId::Flac {
        return parse_flac_bits_per_sample(&stream.extra_data);
    }
    match &stream.params {
        StreamParams::Audio(a) => match a.sample_format {
            SampleFormat::U8 => Some(8),
            SampleFormat::S16 => Some(16),
            SampleFormat::S32 => Some(32),
            _ => None,
        },
        _ => None,
    }
}

fn parse_flac_bits_per_sample(extra_data: &[u8]) -> Option<u32> {
    if extra_data.len() < 34 {
        return None;
    }
    let bps_hi = (u32::from(extra_data[12]) & 0x01) << 4;
    let bps_lo = u32::from(extra_data[13]) >> 4;
    Some((bps_hi | bps_lo) + 1)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use tao_codec::frame::AudioFrame;
    use tao_core::ChannelLayout;

    #[test]
    fn test_parse_flac_bits_per_sample_24bit() {
        let mut extra_data = vec![0u8; 34];
        // 按 FLAC STREAMINFO 打包规则写入 bps=24 (存储值为 bps-1=23=0b10111).
        extra_data[12] = 0x01; // bps 高 1 位
        extra_data[13] = 0x70; // bps 低 4 位 << 4
        assert_eq!(parse_flac_bits_per_sample(&extra_data), Some(24));
    }

    #[test]
    fn test_extract_f32_samples_s32_24bit_scale() {
        let mut af = AudioFrame::new(1, 44_100, SampleFormat::S32, ChannelLayout::MONO);
        // 24-bit 满幅正值 (sign-extended 到 i32).
        let sample = 8_388_607i32;
        af.data[0] = sample.to_le_bytes().to_vec();
        let out = extract_f32_samples(&af, Some(24));
        assert_eq!(out.len(), 1);
        let expected = sample as f32 / 8_388_608.0;
        assert!(
            (out[0] - expected).abs() < 1e-7,
            "S32 24bit 缩放异常: got={}, expected={}",
            out[0],
            expected
        );
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
