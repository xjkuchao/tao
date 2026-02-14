//! 音频输出模块.
//!
//! 使用 cpal 进行跨平台音频输出.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{debug, error, info};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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
    let supported = device.supported_output_configs().ok()?;
    let target_rate = cpal::SampleRate(sample_rate);

    // 优先: 精确匹配采样率和声道数
    for cfg in supported {
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

    // 回退: 匹配声道数, 使用设备支持的采样率
    let supported = device.supported_output_configs().ok()?;
    for cfg in supported {
        if cfg.channels() == channels as u16 {
            let rate = if cfg.min_sample_rate() <= target_rate
                && cfg.max_sample_rate() >= target_rate
            {
                target_rate
            } else {
                cfg.max_sample_rate()
            };
            return Some(cpal::StreamConfig {
                channels: channels as u16,
                sample_rate: rate,
                buffer_size: cpal::BufferSize::Default,
            });
        }
    }

    None
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
        volume: f32,
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

                    // 从通道获取更多数据
                    while buf.len() < data.len() {
                        match recv.try_recv() {
                            Ok(chunk) => {
                                // 更新时钟
                                clock_clone.update_audio_pts(chunk.pts_us);
                                buf.extend_from_slice(&chunk.samples);
                            }
                            Err(_) => break,
                        }
                    }

                    // 填充输出
                    let available = buf.len().min(data.len());
                    for (i, sample) in data.iter_mut().enumerate() {
                        if i < available {
                            *sample = buf[i] * volume;
                        } else {
                            *sample = 0.0;
                        }
                    }

                    // 移除已消耗的数据
                    if available > 0 {
                        buf.drain(..available);
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

        debug!("音频输出已启动: {}Hz, {}ch", sample_rate, channels);

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
