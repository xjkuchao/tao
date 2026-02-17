//! 播放器核心逻辑.
//!
//! 实现 demux -> decode -> render 管线.
//! A/V 同步以音频时钟为主.

use log::{debug, info, warn};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use tao_codec::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use tao_codec::frame::Frame;
use tao_core::{MediaType, PixelFormat, SampleFormat, TaoError};
use tao_format::io::IoContext;
use tao_format::registry::FormatRegistry;
use tao_format::stream::{Stream, StreamParams};

use crate::audio::{AudioChunk, AudioOutput};
use crate::clock::MediaClock;

/// 播放准备结果: (视频尺寸, IO 上下文, 解封装器)
type PrepareResult = (
    Option<(u32, u32)>,
    IoContext,
    Box<dyn tao_format::demuxer::Demuxer>,
);

/// 视频帧数据
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    #[allow(dead_code)]
    pub pts: f64,
}

/// 播放器控制命令
#[derive(Debug, Clone)]
pub enum PlayerCommand {
    TogglePause,
    Seek(f64), // 相对时间 (秒)
    VolumeUp,
    VolumeDown,
    ToggleMute,
    Stop,
}

/// 播放器状态更新
#[derive(Debug, Clone)]
pub enum PlayerStatus {
    Time(f64, f64), // 当前时间, 总时长
    Paused(bool),
    Volume(f32),
    End,
    #[allow(dead_code)]
    Error(String),
}

/// 播放器配置
pub struct PlayerConfig {
    pub input_path: String,
    pub no_video: bool,
    pub no_audio: bool,
    pub volume: f32,
}

/// 播放器
pub struct Player {
    config: PlayerConfig,
    registry: FormatRegistry,
}

impl Player {
    /// 创建播放器
    pub fn new(config: PlayerConfig) -> Result<Self, String> {
        // URL 不需要检查文件存在
        if !is_url(&config.input_path) && !Path::new(&config.input_path).exists() {
            return Err(format!("文件不存在: {}", config.input_path));
        }

        let mut registry = FormatRegistry::new();
        tao_format::register_all(&mut registry);

        Ok(Self { config, registry })
    }

    /// 准备播放并获取视频尺寸（一次性打开文件）
    /// 返回 (video_size, io, demuxer) 供后续使用
    pub fn prepare_and_get_size(&self) -> Result<PrepareResult, String> {
        info!("正在打开: {}", self.config.input_path);

        // 打开 I/O
        let mut io = if is_url(&self.config.input_path) {
            IoContext::open_url(&self.config.input_path)
                .map_err(|e| format!("打开 URL 失败: {}", e))?
        } else {
            IoContext::open_read(&self.config.input_path)
                .map_err(|e| format!("打开文件失败: {}", e))?
        };

        // 探测格式
        let filename = if is_url(&self.config.input_path) {
            filename_from_url(&self.config.input_path)
        } else {
            Path::new(&self.config.input_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.config.input_path)
        };

        let demuxer = self
            .registry
            .open_input(&mut io, Some(filename))
            .map_err(|e| format!("探测格式失败: {}", e))?;

        let streams = demuxer.streams();

        // 查找视频流尺寸
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

        Ok((video_size, io, demuxer))
    }

    /// 获取视频流尺寸（探测但不开始播放）
    #[allow(dead_code)]
    pub fn get_video_size(&self) -> Option<(u32, u32)> {
        // 打开 I/O
        let mut io = if is_url(&self.config.input_path) {
            IoContext::open_url(&self.config.input_path).ok()?
        } else {
            IoContext::open_read(&self.config.input_path).ok()?
        };

        // 探测格式
        let filename = if is_url(&self.config.input_path) {
            filename_from_url(&self.config.input_path)
        } else {
            Path::new(&self.config.input_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.config.input_path)
        };

        let demuxer = self.registry.open_input(&mut io, Some(filename)).ok()?;
        let streams = demuxer.streams();

        // 查找视频流
        let video_stream = streams.iter().find(|s| s.media_type == MediaType::Video)?;

        // 提取尺寸
        if let StreamParams::Video(v) = &video_stream.params {
            Some((v.width, v.height))
        } else {
            None
        }
    }

    /// 在后台线程运行播放器
    #[allow(dead_code)]
    pub fn run_async(
        mut self,
        frame_tx: Sender<VideoFrame>,
        status_tx: Sender<PlayerStatus>,
        command_rx: Receiver<PlayerCommand>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Err(e) = self.run_loop(None, None, frame_tx, status_tx, command_rx) {
                // Ignore error sending if channel closed
                // status_tx.send(PlayerStatus::Error(e)).ok();
                warn!("Playback error: {}", e);
            }
        })
    }

    /// 使用预打开的 IO 和 Demuxer 在后台线程运行播放器（避免重复打开文件）
    pub fn run_with_prepared(
        mut self,
        io: IoContext,
        demuxer: Box<dyn tao_format::demuxer::Demuxer>,
        frame_tx: Sender<VideoFrame>,
        status_tx: Sender<PlayerStatus>,
        command_rx: Receiver<PlayerCommand>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Err(e) = self.run_loop(Some(io), Some(demuxer), frame_tx, status_tx, command_rx)
            {
                warn!("Playback error: {}", e);
            }
        })
    }

    fn run_loop(
        &mut self,
        pre_opened_io: Option<IoContext>,
        pre_opened_demuxer: Option<Box<dyn tao_format::demuxer::Demuxer>>,
        frame_tx: Sender<VideoFrame>,
        status_tx: Sender<PlayerStatus>,
        command_rx: Receiver<PlayerCommand>,
    ) -> Result<(), String> {
        // 使用预打开的 IO/Demuxer 或打开新的
        let (mut io, mut demuxer) =
            if let (Some(io), Some(demuxer)) = (pre_opened_io, pre_opened_demuxer) {
                // 使用已打开的（避免重复下载）
                (io, demuxer)
            } else {
                // 重新打开
                info!("正在打开: {}", self.config.input_path);

                let mut io = if is_url(&self.config.input_path) {
                    IoContext::open_url(&self.config.input_path)
                        .map_err(|e| format!("打开 URL 失败: {}", e))?
                } else {
                    IoContext::open_read(&self.config.input_path)
                        .map_err(|e| format!("打开文件失败: {}", e))?
                };

                let filename = if is_url(&self.config.input_path) {
                    filename_from_url(&self.config.input_path)
                } else {
                    Path::new(&self.config.input_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&self.config.input_path)
                };

                let demuxer = self
                    .registry
                    .open_input(&mut io, Some(filename))
                    .map_err(|e| format!("探测格式失败: {}", e))?;

                (io, demuxer)
            };

        let streams = demuxer.streams().to_vec();
        info!("发现 {} 条流", streams.len());

        // 查找音频和视频流
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

        // 创建解码器
        let audio_stream_idx = audio_stream.map(|s| s.index);
        let video_stream_idx = video_stream.map(|s| s.index);

        let mut audio_decoder = if let Some(stream) = audio_stream {
            let mut codec_registry = tao_codec::registry::CodecRegistry::new();
            tao_codec::register_all(&mut codec_registry);
            match codec_registry.create_decoder(stream.codec_id) {
                Ok(mut dec) => {
                    let params = build_codec_params(stream);
                    if let Err(e) = dec.open(&params) {
                        warn!("打开音频解码器失败: {}", e);
                        None
                    } else {
                        Some(dec)
                    }
                }
                Err(e) => {
                    warn!("创建音频解码器失败: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let mut video_decoder = if let Some(stream) = video_stream {
            let mut codec_registry = tao_codec::registry::CodecRegistry::new();
            tao_codec::register_all(&mut codec_registry);
            match codec_registry.create_decoder(stream.codec_id) {
                Ok(mut dec) => {
                    let params = build_codec_params(stream);
                    if let Err(e) = dec.open(&params) {
                        warn!("打开视频解码器失败: {}", e);
                        None
                    } else {
                        Some(dec)
                    }
                }
                Err(e) => {
                    warn!("创建视频解码器失败: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // 创建时钟
        let clock = MediaClock::new();

        // 创建音频输出
        let audio_output = if let (Some(_dec), Some(stream)) = (&audio_decoder, audio_stream) {
            if let StreamParams::Audio(a) = &stream.params {
                match AudioOutput::new(
                    a.sample_rate,
                    a.channel_layout.channels,
                    clock.clone(),
                    self.config.volume,
                ) {
                    Ok(out) => Some(out),
                    Err(e) => {
                        warn!("创建音频输出失败: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // 主播放循环
        info!("开始播放...");
        let start_time = Instant::now();
        let mut eof = false;
        let mut frames_rendered = 0u64;
        let mut current_volume = (self.config.volume * 100.0) as u32;
        let mut muted = false;
        let mut _seek_target: Option<f64> = None;

        // 计算总时长 (秒)
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
            // 处理命令
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    PlayerCommand::TogglePause => {
                        clock.toggle_pause();
                        status_tx.send(PlayerStatus::Paused(clock.is_paused())).ok();
                    }
                    PlayerCommand::Seek(offset) => {
                        // TODO: Implement seek properly
                        info!("Seek request (not implemented completely): {}s", offset);
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
                        status_tx
                            .send(PlayerStatus::Volume(if muted {
                                0.0
                            } else {
                                current_volume as f32 / 100.0
                            }))
                            .ok();
                    }
                    PlayerCommand::Stop => {
                        info!("停止播放");
                        return Ok(());
                    }
                }
            }

            // 发送状态更新 (Low frequency)
            if frames_rendered % 30 == 0 {
                let current_sec = clock.current_time_us() as f64 / 1_000_000.0;
                status_tx
                    .send(PlayerStatus::Time(current_sec, total_duration_sec))
                    .ok();
            }

            if clock.is_paused() {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            }

            // 读取数据包
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
                                            if let Some(out) = &audio_output {
                                                let pts_us = pts_to_us(
                                                    af.pts,
                                                    af.time_base.num,
                                                    af.time_base.den,
                                                );
                                                let mut samples =
                                                    extract_f32_samples(af, &streams, stream_idx);
                                                // 应用实时音量控制
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

                        // 解码视频
                        if Some(stream_idx) == video_stream_idx {
                            if let Some(dec) = &mut video_decoder {
                                if dec.send_packet(&packet).is_ok() {
                                    while let Ok(frame) = dec.receive_frame() {
                                        if let Frame::Video(vf) = &frame {
                                            let frame_pts_us = pts_to_us(
                                                vf.pts,
                                                vf.time_base.num,
                                                vf.time_base.den,
                                            );
                                            let current_us = clock.current_time_us();
                                            let delay_us = frame_pts_us - current_us;

                                            // 帧太迟 (>200ms) 则丢弃, 不阻塞解码循环
                                            if delay_us < -200_000 {
                                                continue;
                                            }

                                            // 帧稍早时短暂让步, 避免 CPU 空转
                                            // 但不做长时间 sleep 以免阻塞音频处理
                                            if delay_us > 5_000 {
                                                std::thread::sleep(Duration::from_millis(1));
                                            }

                                            let rgb_data = convert_frame_to_rgb24(vf);
                                            let display_frame = VideoFrame {
                                                width: vf.width,
                                                height: vf.height,
                                                data: rgb_data,
                                                pts: frame_pts_us as f64 / 1_000_000.0,
                                            };

                                            if frame_tx.send(display_frame).is_err() {
                                                return Ok(());
                                            }
                                            frames_rendered += 1;
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
                        // Continue if possible?
                        // eof = true;
                    }
                }
            }

            // 如果只有视频, 没有音频时钟, 使用系统时钟
            if audio_output.is_none() && !eof {
                let elapsed_us = start_time.elapsed().as_micros() as i64;
                clock.update_audio_pts(elapsed_us);
            }

            // 如果没有视频, 通过音频播放到结束
            if video_stream.is_none() && eof {
                std::thread::sleep(Duration::from_millis(2000));
                break;
            }

            if eof {
                // Check if we should quit
                // For now, just exit
                break;
            }

            // 帧率限制 (避免 CPU 100%)
            if video_stream.is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        // 清理
        if let Some(out) = &audio_output {
            out.stop();
        }

        status_tx.send(PlayerStatus::End).ok();

        let elapsed = start_time.elapsed();
        info!(
            "播放结束: 渲染 {} 帧, 耗时 {:.1}s",
            frames_rendered,
            elapsed.as_secs_f64()
        );

        Ok(())
    }
}

/// 将 PTS 转换为微秒
fn pts_to_us(pts: i64, num: i32, den: i32) -> i64 {
    if den == 0 {
        return 0;
    }
    pts * num as i64 * 1_000_000 / den as i64
}

/// 从音频帧提取 F32 交错采样
fn extract_f32_samples(
    af: &tao_codec::frame::AudioFrame,
    _streams: &[Stream],
    _stream_idx: usize,
) -> Vec<f32> {
    match af.sample_format {
        SampleFormat::F32 => {
            // 已经是 F32 交错
            af.data[0]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        }
        SampleFormat::S16 => {
            // S16 -> F32
            af.data[0]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect()
        }
        SampleFormat::S32 => af.data[0]
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0)
            .collect(),
        _ => {
            // 未知格式, 返回静音
            vec![0.0f32; af.nb_samples as usize * 2]
        }
    }
}

/// 将视频帧转换为 RGB24 数据
fn convert_frame_to_rgb24(vf: &tao_codec::frame::VideoFrame) -> Vec<u8> {
    let w = vf.width as usize;
    let h = vf.height as usize;

    match vf.pixel_format {
        PixelFormat::Rgb24 => {
            // 已经是 RGB24
            vf.data[0].clone()
        }
        PixelFormat::Yuv420p => {
            // YUV420P -> RGB24 (定点整数运算, 避免浮点开销)
            // BT.601 full range: R = Y + 1.402*V, G = Y - 0.344*U - 0.714*V, B = Y + 1.772*U
            // 使用 <<16 定点: 1.402*65536=91881, 0.344*65536=22544, 0.714*65536=46793, 1.772*65536=116130
            const CR_R: i32 = 91881;
            const CB_G: i32 = 22544;
            const CR_G: i32 = 46793;
            const CB_B: i32 = 116130;
            const FP_HALF: i32 = 1 << 15; // 舍入偏移

            let mut rgb = vec![0u8; w * h * 3];
            let y_plane = &vf.data[0];
            let u_plane = &vf.data[1];
            let v_plane = &vf.data[2];
            let y_stride = vf.linesize[0];
            let u_stride = vf.linesize[1];

            for row in 0..h {
                let y_row = row * y_stride;
                let uv_row = (row >> 1) * u_stride;
                let dst_row = row * w * 3;

                for col in 0..w {
                    let y_idx = y_row + col;
                    let uv_idx = uv_row + (col >> 1);

                    let y_val = *y_plane.get(y_idx).unwrap_or(&16) as i32;
                    let cb = *u_plane.get(uv_idx).unwrap_or(&128) as i32 - 128;
                    let cr = *v_plane.get(uv_idx).unwrap_or(&128) as i32 - 128;

                    let r = y_val + ((CR_R * cr + FP_HALF) >> 16);
                    let g = y_val - ((CB_G * cb + CR_G * cr + FP_HALF) >> 16);
                    let b = y_val + ((CB_B * cb + FP_HALF) >> 16);

                    let dst = dst_row + col * 3;
                    rgb[dst] = r.clamp(0, 255) as u8;
                    rgb[dst + 1] = g.clamp(0, 255) as u8;
                    rgb[dst + 2] = b.clamp(0, 255) as u8;
                }
            }
            rgb
        }
        _ => {
            // 其他格式: 灰色填充
            vec![128u8; w * h * 3]
        }
    }
}

/// 判断路径是否为 URL
fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

/// 从 URL 提取文件名 (用于格式探测)
fn filename_from_url(url: &str) -> &str {
    url.rsplit('/')
        .next()
        .and_then(|s| s.split('?').next())
        .unwrap_or(url)
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
