//! 音频输出模块.
//!
//! 使用 cpal 进行跨平台音频输出.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{debug, error, info};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tao_core::{ChannelLayout, SampleFormat};
use tao_resample::ResampleContext;

use crate::clock::MediaClock;

/// 音频缓冲区中的数据块
pub struct AudioChunk {
    /// F32 交错采样数据
    pub samples: Vec<f32>,
    /// 这块数据对应的 PTS (微秒)
    pub pts_us: i64,
}

/// 音频输出管理器
pub struct AudioOutput {
    /// cpal 音频流 (需要持有以保持播放)
    _stream: cpal::Stream,
    /// 数据发送端
    sender: mpsc::SyncSender<AudioChunk>,
    /// 是否正在播放
    playing: Arc<Mutex<bool>>,
}

/// 查找设备支持的兼容音频配置
fn find_compatible_config(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u32,
) -> Option<cpal::StreamConfig> {
    let supported: Vec<_> = device.supported_output_configs().ok()?.collect();
    let target_rate = cpal::SampleRate(sample_rate);

    // 优先: 精确匹配采样率和声道数
    for cfg in &supported {
        if cfg.channels() == channels as u16
            && cfg.min_sample_rate() <= target_rate
            && cfg.max_sample_rate() >= target_rate
        {
            return Some(cpal::StreamConfig {
                channels: channels as u16,
                sample_rate: target_rate,
                buffer_size: cpal::BufferSize::Default,
            });
        }
    }

    // 次优: 采样率可精确匹配, 选择声道差距最小的配置.
    let mut best_rate_match: Option<(u32, cpal::StreamConfig)> = None;
    for cfg in &supported {
        if cfg.min_sample_rate() <= target_rate && cfg.max_sample_rate() >= target_rate {
            let ch = u32::from(cfg.channels());
            let diff = ch.abs_diff(channels);
            let candidate = cpal::StreamConfig {
                channels: cfg.channels(),
                sample_rate: target_rate,
                buffer_size: cpal::BufferSize::Default,
            };

            match &best_rate_match {
                None => best_rate_match = Some((diff, candidate)),
                Some((best_diff, _)) if diff < *best_diff => {
                    best_rate_match = Some((diff, candidate))
                }
                _ => {}
            }
        }
    }
    if let Some((_, cfg)) = best_rate_match {
        return Some(cfg);
    }

    // 兜底: 选择总体最接近的采样率/声道配置.
    let mut best_any: Option<(u64, cpal::StreamConfig)> = None;
    for cfg in &supported {
        let chosen_rate = if target_rate < cfg.min_sample_rate() {
            cfg.min_sample_rate()
        } else if target_rate > cfg.max_sample_rate() {
            cfg.max_sample_rate()
        } else {
            target_rate
        };

        let rate_diff = chosen_rate.0.abs_diff(sample_rate) as u64;
        let channel_diff = u32::from(cfg.channels()).abs_diff(channels) as u64;
        // 采样率差异权重更高, 优先减少速度/音高偏差风险.
        let score = rate_diff * 10 + channel_diff;
        let candidate = cpal::StreamConfig {
            channels: cfg.channels(),
            sample_rate: chosen_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        match &best_any {
            None => best_any = Some((score, candidate)),
            Some((best_score, _)) if score < *best_score => best_any = Some((score, candidate)),
            _ => {}
        }
    }

    best_any.map(|(_, cfg)| cfg)
}

impl AudioOutput {
    /// 创建音频输出
    ///
    /// # 参数
    /// - `sample_rate`: 采样率
    /// - `channels`: 声道数
    /// - `clock`: 媒体时钟 (用于 A/V 同步)
    /// - `volume`: 音量 (0.0 ~ 1.0)
    pub fn new(
        sample_rate: u32,
        channels: u32,
        clock: MediaClock,
        _volume: f32,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "找不到音频输出设备".to_string())?;

        info!("音频设备: {:?}", device.name().unwrap_or_default());

        // 优先使用请求的配置, 失败则回退到设备默认配置
        let config = match find_compatible_config(&device, sample_rate, channels) {
            Some(cfg) => cfg,
            None => {
                let default_cfg = device
                    .default_output_config()
                    .map_err(|e| format!("获取默认音频配置失败: {}", e))?;
                info!(
                    "使用设备默认配置: {}Hz, {}ch",
                    default_cfg.sample_rate().0,
                    default_cfg.channels()
                );
                cpal::StreamConfig {
                    channels: default_cfg.channels(),
                    sample_rate: default_cfg.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                }
            }
        };
        let output_sample_rate = config.sample_rate.0;
        let output_channels = u32::from(config.channels);
        let converter =
            build_f32_converter(sample_rate, channels, output_sample_rate, output_channels);
        if converter.is_some() {
            info!(
                "音频参数转换: {}Hz/{}ch -> {}Hz/{}ch",
                sample_rate, channels, output_sample_rate, output_channels
            );
        }

        // 音频数据通道 (有界缓冲, 防止 OOM)
        let (sender, receiver) = mpsc::sync_channel::<AudioChunk>(32);
        let receiver = Arc::new(Mutex::new(receiver));
        let playing = Arc::new(Mutex::new(true));
        let playing_clone = playing.clone();

        // 内部缓冲
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = buffer.clone();
        let clock_clone = clock.clone();

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let is_playing = *playing_clone.lock().unwrap();
                    if !is_playing || clock_clone.is_paused() {
                        // 输出静音
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        return;
                    }

                    let mut buf = buffer_clone.lock().unwrap();
                    let recv = receiver.lock().unwrap();

                    // 记录最近收到的 PTS, 用于在输出后更准确地更新时钟
                    let mut last_chunk_pts = None;

                    // 从通道获取更多数据
                    while buf.len() < data.len() {
                        match recv.try_recv() {
                            Ok(chunk) => {
                                last_chunk_pts = Some(chunk.pts_us);
                                if let Some(conv) = &converter {
                                    match convert_chunk_f32(&chunk.samples, channels, conv) {
                                        Ok(samples) => buf.extend_from_slice(&samples),
                                        Err(e) => {
                                            debug!("音频转换失败, 回退原始数据: {}", e);
                                            buf.extend_from_slice(&chunk.samples);
                                        }
                                    }
                                } else {
                                    buf.extend_from_slice(&chunk.samples);
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    // 填充输出 (音量已在 player.rs 中应用)
                    let available = buf.len().min(data.len());
                    for (i, sample) in data.iter_mut().enumerate() {
                        if i < available {
                            *sample = buf[i];
                        } else {
                            *sample = 0.0;
                        }
                    }

                    // 移除已消耗的数据
                    if available > 0 {
                        buf.drain(..available);
                    }

                    // 在数据实际送出后更新时钟, 并扣除缓冲区中未播放的数据时长
                    // 这比在接收时就更新更准确, 避免时钟超前
                    if let Some(pts) = last_chunk_pts {
                        let out_ch = output_channels.max(1) as i64;
                        let buffered_samples = buf.len() as i64 / out_ch;
                        let buffered_us = buffered_samples * 1_000_000 / output_sample_rate as i64;
                        clock_clone.update_audio_pts(pts - buffered_us);
                    }
                },
                move |err| {
                    error!("音频输出错误: {}", err);
                },
                None,
            )
            .map_err(|e| format!("创建音频流失败: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("启动音频播放失败: {}", e))?;

        debug!(
            "音频输出已启动: 输入 {}Hz/{}ch, 设备 {}Hz/{}ch",
            sample_rate, channels, output_sample_rate, output_channels
        );

        Ok(Self {
            _stream: stream,
            sender,
            playing,
        })
    }

    /// 发送音频数据到播放队列
    pub fn send(&self, chunk: AudioChunk) -> Result<(), String> {
        self.sender
            .send(chunk)
            .map_err(|e| format!("发送音频数据失败: {}", e))
    }

    /// 停止播放
    pub fn stop(&self) {
        *self.playing.lock().unwrap() = false;
    }
}

/// 构建 F32 交错音频转换器.
///
/// 当输入参数与设备输出参数不一致时, 启用重采样和声道映射.
fn build_f32_converter(
    input_rate: u32,
    input_channels: u32,
    output_rate: u32,
    output_channels: u32,
) -> Option<ResampleContext> {
    if input_rate == output_rate && input_channels == output_channels {
        return None;
    }
    Some(ResampleContext::new(
        input_rate,
        SampleFormat::F32,
        ChannelLayout::from_channels(input_channels),
        output_rate,
        SampleFormat::F32,
        ChannelLayout::from_channels(output_channels),
    ))
}

/// 将一个 F32 交错音频块转换为目标设备参数.
fn convert_chunk_f32(
    input_samples: &[f32],
    input_channels: u32,
    converter: &ResampleContext,
) -> Result<Vec<f32>, String> {
    if input_channels == 0 {
        return Ok(Vec::new());
    }
    let channels = input_channels as usize;
    let nb_samples = input_samples.len() / channels;
    if nb_samples == 0 {
        return Ok(Vec::new());
    }

    let mut input_bytes = Vec::with_capacity(input_samples.len() * 4);
    for sample in input_samples {
        input_bytes.extend_from_slice(&sample.to_le_bytes());
    }

    let (output_bytes, _) = converter
        .convert(&input_bytes, nb_samples as u32)
        .map_err(|e| format!("重采样失败: {}", e))?;

    let mut output_samples = Vec::with_capacity(output_bytes.len() / 4);
    for chunk in output_bytes.chunks_exact(4) {
        output_samples.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(output_samples)
}
