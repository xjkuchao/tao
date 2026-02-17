//! 音频输出模块.
//!
//! 使用 SDL2 音频子系统进行跨平台音频输出.
//! 缓冲区大小和时钟补偿逻辑对齐 ffplay.

use log::{debug, info};
use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tao_core::{ChannelLayout, SampleFormat};
use tao_resample::ResampleContext;

use crate::clock::MediaClock;

// ── ffplay 音频常量 ──────────────────────────────────────────────────────

/// 最小缓冲区大小 (样本数) - ffplay: SDL_AUDIO_MIN_BUFFER_SIZE
const SDL_AUDIO_MIN_BUFFER_SIZE: u16 = 512;
/// 最大回调频率 (次/秒) - ffplay: SDL_AUDIO_MAX_CALLBACKS_PER_SEC
const SDL_AUDIO_MAX_CALLBACKS_PER_SEC: u32 = 30;

/// 音频缓冲区中的数据块
pub struct AudioChunk {
    /// F32 交错采样数据
    pub samples: Vec<f32>,
    /// 这块数据对应的 PTS (微秒)
    pub pts_us: i64,
}

/// SDL2 音频回调结构
struct SdlAudioPlayer {
    receiver: Arc<Mutex<mpsc::Receiver<AudioChunk>>>,
    buffer: Vec<f32>,
    clock: MediaClock,
    converter: Option<ResampleContext>,
    input_channels: u32,
    output_sample_rate: u32,
    output_channels: u32,
    playing: Arc<Mutex<bool>>,
}

impl AudioCallback for SdlAudioPlayer {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let is_playing = *self.playing.lock().unwrap();
        if !is_playing || self.clock.is_paused() {
            for sample in out.iter_mut() {
                *sample = 0.0;
            }
            return;
        }

        let recv = self.receiver.lock().unwrap();

        let mut last_chunk_pts = None;

        // 从通道获取数据填充内部缓冲
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

        // 填充输出
        let available = self.buffer.len().min(out.len());
        for (i, sample) in out.iter_mut().enumerate() {
            *sample = if i < available { self.buffer[i] } else { 0.0 };
        }

        if available > 0 {
            self.buffer.drain(..available);
        }

        // ── 更新音频时钟 (对齐 ffplay sdl_audio_callback) ──
        //
        // ffplay: set_clock_at(&audclk,
        //   audio_clock - (2 * hw_buf_size + write_buf_size) / bytes_per_sec, ...)
        //
        // 假设 SDL 音频驱动有 2 个周期缓冲 (与 ffplay 一致):
        // - hw_buf: 2 个回调缓冲 = 2 * out.len() 个 f32 样本
        // - write_buf: 内部缓冲中未播放的数据 = self.buffer.len() 个 f32 样本
        if let Some(pts) = last_chunk_pts {
            let out_ch = self.output_channels.max(1) as i64;
            let rate = self.output_sample_rate as i64;

            // 内部缓冲中未播放的帧数
            let write_buf_frames = self.buffer.len() as i64 / out_ch;
            // SDL 硬件缓冲 (2 个周期)
            let hw_buf_frames = 2 * out.len() as i64 / out_ch;

            let total_buffered_us = (hw_buf_frames + write_buf_frames) * 1_000_000 / rate;
            self.clock.update_audio_pts(pts - total_buffered_us);
        }
    }
}

/// 音频输出管理器 (留在主线程, 持有 SDL2 设备)
pub struct AudioOutput {
    _device: AudioDevice<SdlAudioPlayer>,
}

/// 音频数据发送端 (可安全跨线程传递给 player 线程)
pub struct AudioSender {
    sender: mpsc::SyncSender<AudioChunk>,
}

impl AudioOutput {
    /// 创建音频输出
    ///
    /// 缓冲区大小按 ffplay 公式计算:
    /// `max(SDL_AUDIO_MIN_BUFFER_SIZE, 2 << log2(freq / SDL_AUDIO_MAX_CALLBACKS_PER_SEC))`
    pub fn new(
        audio_subsystem: &sdl2::AudioSubsystem,
        sample_rate: u32,
        channels: u32,
        clock: MediaClock,
    ) -> Result<(Self, AudioSender), String> {
        let buf_size = compute_audio_buf_size(sample_rate);

        let desired_spec = AudioSpecDesired {
            freq: Some(sample_rate as i32),
            channels: Some(channels as u8),
            samples: Some(buf_size),
        };

        let (sender, receiver) = mpsc::sync_channel::<AudioChunk>(32);
        let receiver = Arc::new(Mutex::new(receiver));
        let playing = Arc::new(Mutex::new(true));

        let playing_clone = playing.clone();
        let receiver_clone = receiver.clone();

        let device = audio_subsystem.open_playback(None, &desired_spec, |spec| {
            let output_sample_rate = spec.freq as u32;
            let output_channels = spec.channels as u32;

            info!(
                "SDL2 音频设备: {}Hz/{}ch, 缓冲区 {} 样本 (请求 {}Hz/{}ch, {} 样本)",
                output_sample_rate, output_channels, spec.samples, sample_rate, channels, buf_size
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

        device.resume();

        debug!(
            "SDL2 音频输出已启动: {}Hz/{}ch, 缓冲区 {} 样本",
            sample_rate, channels, buf_size
        );

        Ok((Self { _device: device }, AudioSender { sender }))
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

/// 按 ffplay 公式计算音频缓冲区大小 (样本数)
///
/// `max(SDL_AUDIO_MIN_BUFFER_SIZE, 2 << av_log2(freq / SDL_AUDIO_MAX_CALLBACKS_PER_SEC))`
fn compute_audio_buf_size(sample_rate: u32) -> u16 {
    let ratio = sample_rate / SDL_AUDIO_MAX_CALLBACKS_PER_SEC;
    if ratio == 0 {
        return SDL_AUDIO_MIN_BUFFER_SIZE;
    }
    let log2_val = ratio.ilog2();
    let buf_size = 2u32 << log2_val;
    buf_size.max(SDL_AUDIO_MIN_BUFFER_SIZE as u32) as u16
}

/// 构建 F32 交错音频转换器
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

/// 将一个 F32 交错音频块转换为目标设备参数
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
