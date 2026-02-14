//! 播放器核心逻辑.
//!
//! 实现 demux -> decode -> render 管线.
//! A/V 同步以音频时钟为主.

use log::{debug, info, warn};
use std::path::Path;
use std::time::{Duration, Instant};

use tao_codec::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use tao_codec::frame::Frame;
use tao_core::{MediaType, PixelFormat, SampleFormat, TaoError};
use tao_format::io::IoContext;
use tao_format::registry::FormatRegistry;
use tao_format::stream::{Stream, StreamParams};

use crate::audio::{AudioChunk, AudioOutput};
use crate::clock::MediaClock;
use crate::video::VideoDisplay;

/// 播放器配置
pub struct PlayerConfig {
    pub input_path: String,
    pub no_video: bool,
    pub no_audio: bool,
    pub volume: f32,
    pub window_width: u32,
    pub window_height: u32,
}

/// 播放器
pub struct Player {
    config: PlayerConfig,
    registry: FormatRegistry,
}

impl Player {
    /// 创建播放器
    pub fn new(config: PlayerConfig) -> Result<Self, String> {
        // 检查文件存在
        if !Path::new(&config.input_path).exists() {
            return Err(format!("文件不存在: {}", config.input_path));
        }

        let registry = FormatRegistry::new();

        Ok(Self { config, registry })
    }

    /// 运行播放器主循环
    pub fn run(&mut self) -> Result<(), String> {
        info!("正在打开: {}", self.config.input_path);

        // 打开输入文件
        let mut io = IoContext::open_read(&self.config.input_path)
            .map_err(|e| format!("打开文件失败: {}", e))?;

        // 探测格式并创建解封装器
        let filename = Path::new(&self.config.input_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.config.input_path);
        let mut demuxer = self
            .registry
            .open_input(&mut io, Some(filename))
            .map_err(|e| format!("探测格式失败: {}", e))?;

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

        // 打印流信息
        if let Some(a) = audio_stream {
            info!(
                "音频流 #{}: {} ({:?})",
                a.index, a.codec_id, a.params
            );
        }
        if let Some(v) = video_stream {
            info!(
                "视频流 #{}: {} ({:?})",
                v.index, v.codec_id, v.params
            );
        }

        // 创建解码器
        let audio_stream_idx = audio_stream.map(|s| s.index);
        let video_stream_idx = video_stream.map(|s| s.index);

        let mut audio_decoder = if let Some(stream) = audio_stream {
            let codec_registry = tao_codec::registry::CodecRegistry::new();
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
            let codec_registry = tao_codec::registry::CodecRegistry::new();
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

        // 创建视频窗口
        let mut video_display = if video_decoder.is_some() {
            if let Some(stream) = video_stream {
                if let StreamParams::Video(v) = &stream.params {
                    let w = if self.config.window_width > 0 {
                        self.config.window_width
                    } else {
                        v.width.max(320)
                    };
                    let h = if self.config.window_height > 0 {
                        self.config.window_height
                    } else {
                        v.height.max(240)
                    };

                    let title = format!(
                        "tao-play - {} ({}x{})",
                        Path::new(&self.config.input_path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        v.width,
                        v.height
                    );

                    match VideoDisplay::new(w, h, &title) {
                        Ok(display) => Some(display),
                        Err(e) => {
                            warn!("创建视频窗口失败: {}", e);
                            None
                        }
                    }
                } else {
                    None
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

        loop {
            // 检查窗口关闭
            if let Some(display) = &video_display {
                if !display.is_open() || display.should_quit() {
                    info!("用户关闭窗口");
                    break;
                }
            }

            // 检查暂停
            if let Some(display) = &video_display {
                if display.is_space_pressed() {
                    clock.toggle_pause();
                    info!(
                        "{}",
                        if clock.is_paused() {
                            "已暂停"
                        } else {
                            "继续播放"
                        }
                    );
                }
            }

            if clock.is_paused() {
                if let Some(display) = &mut video_display {
                    display.update();
                }
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
                                                let samples =
                                                    extract_f32_samples(af, &streams, stream_idx);
                                                let chunk =
                                                    AudioChunk { samples, pts_us };
                                                if out.send(chunk).is_err() {
                                                    debug!("音频队列已满, 跳过");
                                                }
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
                                            // A/V 同步: 计算帧显示时间
                                            let frame_pts_us = pts_to_us(
                                                vf.pts,
                                                vf.time_base.num,
                                                vf.time_base.den,
                                            );
                                            let current_us = clock.current_time_us();
                                            let delay_us = frame_pts_us - current_us;

                                            // 如果帧还没到显示时间, 等待
                                            if delay_us > 1000 {
                                                std::thread::sleep(Duration::from_micros(
                                                    delay_us.min(50000) as u64,
                                                ));
                                            }

                                            // 渲染帧
                                            if let Some(display) = &mut video_display {
                                                let rgb_data = convert_frame_to_rgb24(vf);
                                                display.display_rgb24(
                                                    &rgb_data,
                                                    vf.width,
                                                    vf.height,
                                                );
                                                frames_rendered += 1;
                                            }
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
                        eof = true;
                    }
                }
            }

            // 如果只有视频, 没有音频时钟, 使用系统时钟
            if audio_output.is_none() && !eof {
                let elapsed_us = start_time.elapsed().as_micros() as i64;
                clock.update_audio_pts(elapsed_us);
            }

            // 如果没有视频, 通过音频播放到结束
            if video_display.is_none() && eof {
                // 等一会让音频播完
                std::thread::sleep(Duration::from_millis(100));
                break;
            }

            // 如果有视频但 EOF, 再显示一会
            if eof {
                if let Some(display) = &mut video_display {
                    display.update();
                    if display.should_quit() || !display.is_open() {
                        break;
                    }
                } else {
                    break;
                }
                std::thread::sleep(Duration::from_millis(16));
            }

            // 帧率限制 (避免 CPU 100%)
            if video_display.is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        // 清理
        if let Some(out) = &audio_output {
            out.stop();
        }

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
        SampleFormat::S32 => {
            af.data[0]
                .chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0)
                .collect()
        }
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
            // YUV420P -> RGB24 (简化转换)
            let mut rgb = vec![0u8; w * h * 3];
            let y_plane = &vf.data[0];
            let u_plane = &vf.data[1];
            let v_plane = &vf.data[2];
            let y_stride = vf.linesize[0];
            let u_stride = vf.linesize[1];

            for row in 0..h {
                for col in 0..w {
                    let y_idx = row * y_stride + col;
                    let u_idx = (row / 2) * u_stride + col / 2;

                    let y = if y_idx < y_plane.len() {
                        y_plane[y_idx] as f32
                    } else {
                        16.0
                    };
                    let u = if u_idx < u_plane.len() {
                        u_plane[u_idx] as f32 - 128.0
                    } else {
                        0.0
                    };
                    let v = if u_idx < v_plane.len() {
                        v_plane[u_idx] as f32 - 128.0
                    } else {
                        0.0
                    };

                    let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
                    let g = (y - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
                    let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;

                    let dst_idx = (row * w + col) * 3;
                    rgb[dst_idx] = r;
                    rgb[dst_idx + 1] = g;
                    rgb[dst_idx + 2] = b;
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
            params: CodecParamsType::Video(
                tao_codec::codec_parameters::VideoCodecParams {
                    width: v.width,
                    height: v.height,
                    pixel_format: v.pixel_format,
                    frame_rate: v.frame_rate,
                    sample_aspect_ratio: v.sample_aspect_ratio,
                },
            ),
        },
        _ => CodecParameters {
            codec_id: stream.codec_id,
            bit_rate: 0,
            extra_data: stream.extra_data.clone(),
            params: CodecParamsType::None,
        },
    }
}
