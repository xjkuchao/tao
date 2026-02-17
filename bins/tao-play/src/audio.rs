//! 音频输出模块.
//!
//! 使用 SDL2 音频子系统进行跨平台音频输出.

use log::{debug, info};
use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
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

/// SDL2 音频回调结构
struct SdlAudioPlayer {
    /// 接收通道
    receiver: Arc<Mutex<mpsc::Receiver<AudioChunk>>>,
    /// 内部缓冲
    buffer: Vec<f32>,
    /// 媒体时钟
    clock: MediaClock,
    /// 音频转换器 (重采样/声道映射)
    converter: Option<ResampleContext>,
    /// 输入声道数 (用于转换器)
    input_channels: u32,
    /// 输出采样率 (用于时钟计算)
    output_sample_rate: u32,
    /// 输出声道数 (用于时钟计算)
    output_channels: u32,
    /// 是否正在播放
    playing: Arc<Mutex<bool>>,
}

impl AudioCallback for SdlAudioPlayer {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let is_playing = *self.playing.lock().unwrap();
        if !is_playing || self.clock.is_paused() {
            // 输出静音
            for sample in out.iter_mut() {
                *sample = 0.0;
            }
            return;
        }

        let recv = self.receiver.lock().unwrap();

        // 记录最近收到的 PTS, 用于在输出后更准确地更新时钟
        let mut last_chunk_pts = None;

        // 从通道获取更多数据
        while self.buffer.len() < out.len() {
            match recv.try_recv() {
                Ok(chunk) => {
                    last_chunk_pts = Some(chunk.pts_us);
                    if let Some(conv) = &self.converter {
                        match convert_chunk_f32(&chunk.samples, self.input_channels, conv) {
                            Ok(samples) => self.buffer.extend_from_slice(&samples),
                            Err(e) => {
                                debug!("音频转换失败, 回退原始数据: {}", e);
                                self.buffer.extend_from_slice(&chunk.samples);
                            }
                        }
                    } else {
                        self.buffer.extend_from_slice(&chunk.samples);
                    }
                }
                Err(_) => break,
            }
        }

        // 填充输出 (音量已在 player.rs 中应用)
        let available = self.buffer.len().min(out.len());
        for (i, sample) in out.iter_mut().enumerate() {
            if i < available {
                *sample = self.buffer[i];
            } else {
                *sample = 0.0;
            }
        }

        // 移除已消耗的数据
        if available > 0 {
            self.buffer.drain(..available);
        }

        // 在数据实际送出后更新时钟, 并扣除缓冲区中未播放的数据时长
        if let Some(pts) = last_chunk_pts {
            let out_ch = self.output_channels.max(1) as i64;
            let buffered_samples = self.buffer.len() as i64 / out_ch;
            let buffered_us = buffered_samples * 1_000_000 / self.output_sample_rate as i64;
            self.clock.update_audio_pts(pts - buffered_us);
        }
    }
}

/// 音频输出管理器 (留在主线程, 持有 SDL2 设备)
///
/// Drop 时 SDL2 设备自动停止; sender 断开后回调自动填充静音.
pub struct AudioOutput {
    /// SDL2 音频设备 (需要持有以保持播放)
    _device: AudioDevice<SdlAudioPlayer>,
}

/// 音频数据发送端 (可安全跨线程传递给 player 线程)
pub struct AudioSender {
    sender: mpsc::SyncSender<AudioChunk>,
}

impl AudioOutput {
    /// 创建音频输出
    ///
    /// 返回 `(AudioOutput, AudioSender)`:
    /// - `AudioOutput` 保留在主线程, 持有 SDL2 设备
    /// - `AudioSender` 传给 player 线程, 用于发送音频数据
    pub fn new(
        audio_subsystem: &sdl2::AudioSubsystem,
        sample_rate: u32,
        channels: u32,
        clock: MediaClock,
    ) -> Result<(Self, AudioSender), String> {
        let desired_spec = AudioSpecDesired {
            freq: Some(sample_rate as i32),
            channels: Some(channels as u8),
            samples: Some(1024),
        };

        // 音频数据通道 (有界缓冲, 防止 OOM)
        let (sender, receiver) = mpsc::sync_channel::<AudioChunk>(32);
        let receiver = Arc::new(Mutex::new(receiver));
        let playing = Arc::new(Mutex::new(true));

        let playing_clone = playing.clone();
        let receiver_clone = receiver.clone();

        let device = audio_subsystem.open_playback(None, &desired_spec, |spec| {
            let output_sample_rate = spec.freq as u32;
            let output_channels = spec.channels as u32;

            info!(
                "SDL2 音频设备: {}Hz/{}ch (请求 {}Hz/{}ch)",
                output_sample_rate, output_channels, sample_rate, channels
            );

            let converter =
                build_f32_converter(sample_rate, channels, output_sample_rate, output_channels);
            if converter.is_some() {
                info!(
                    "音频参数转换: {}Hz/{}ch -> {}Hz/{}ch",
                    sample_rate, channels, output_sample_rate, output_channels
                );
            }

            SdlAudioPlayer {
                receiver: receiver_clone,
                buffer: Vec::new(),
                clock,
                converter,
                input_channels: channels,
                output_sample_rate,
                output_channels,
                playing: playing_clone,
            }
        })?;

        // 开始播放
        device.resume();

        debug!("SDL2 音频输出已启动: 输入 {}Hz/{}ch", sample_rate, channels);

        let output = Self { _device: device };
        let audio_sender = AudioSender { sender };

        Ok((output, audio_sender))
    }
}

impl AudioSender {
    /// 发送音频数据到播放队列
    pub fn send(&self, chunk: AudioChunk) -> Result<(), String> {
        self.sender
            .send(chunk)
            .map_err(|e| format!("发送音频数据失败: {}", e))
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
