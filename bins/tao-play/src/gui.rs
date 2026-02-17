//! SDL2 视频渲染和事件循环.
//!
//! 使用 SDL2 YUV 纹理进行硬件加速渲染, GPU 做色彩空间转换.

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, TextureAccess};
use sdl2::video::Window;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::player::{PlayerCommand, PlayerStatus, VideoFrame};

/// 运行 SDL2 事件循环 (在主线程)
///
/// 接收视频帧并渲染到窗口, 处理键盘事件.
pub fn run_event_loop(
    mut canvas: Canvas<Window>,
    frame_rx: Receiver<VideoFrame>,
    status_rx: Receiver<PlayerStatus>,
    command_tx: std::sync::mpsc::Sender<PlayerCommand>,
) -> Result<(), String> {
    let texture_creator = canvas.texture_creator();

    // YUV 纹理 (延迟创建, 等待首帧确定尺寸)
    let mut texture = None;
    let mut tex_width = 0u32;
    let mut tex_height = 0u32;

    let sdl_context = canvas.window().subsystem().sdl();
    let mut event_pump = sdl_context.event_pump()?;

    'running: loop {
        // 1. 处理 SDL2 事件
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => {
                    let _ = command_tx.send(PlayerCommand::Stop);
                    break 'running;
                }
                Event::KeyDown {
                    keycode: Some(key), ..
                } => match key {
                    Keycode::Space => {
                        let _ = command_tx.send(PlayerCommand::TogglePause);
                    }
                    Keycode::Right => {
                        let _ = command_tx.send(PlayerCommand::Seek(10.0));
                    }
                    Keycode::Left => {
                        let _ = command_tx.send(PlayerCommand::Seek(-10.0));
                    }
                    Keycode::Up => {
                        let _ = command_tx.send(PlayerCommand::VolumeUp);
                    }
                    Keycode::Down => {
                        let _ = command_tx.send(PlayerCommand::VolumeDown);
                    }
                    Keycode::M => {
                        let _ = command_tx.send(PlayerCommand::ToggleMute);
                    }
                    Keycode::Escape | Keycode::Q => {
                        let _ = command_tx.send(PlayerCommand::Stop);
                        break 'running;
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // 2. 接收状态更新
        while let Ok(status) = status_rx.try_recv() {
            if let PlayerStatus::End = status {
                break 'running;
            }
        }

        // 3. 接收视频帧 (取最新一帧, 丢弃中间帧)
        let mut latest_frame = None;
        while let Ok(frame) = frame_rx.try_recv() {
            latest_frame = Some(frame);
        }

        // 4. 更新纹理并渲染
        if let Some(frame) = latest_frame {
            // 如果尺寸变化或首次, 重新创建纹理
            if texture.is_none() || frame.width != tex_width || frame.height != tex_height {
                tex_width = frame.width;
                tex_height = frame.height;
                texture = Some(
                    texture_creator
                        .create_texture(
                            PixelFormatEnum::IYUV,
                            TextureAccess::Streaming,
                            tex_width,
                            tex_height,
                        )
                        .map_err(|e| e.to_string())?,
                );
            }

            if let Some(tex) = &mut texture {
                // 上传 YUV 数据到 GPU (硬件色彩转换)
                tex.update_yuv(
                    None,
                    &frame.y_data,
                    frame.y_stride,
                    &frame.u_data,
                    frame.u_stride,
                    &frame.v_data,
                    frame.v_stride,
                )
                .map_err(|e| e.to_string())?;

                canvas.clear();
                // SDL2 自动缩放纹理到窗口大小, 保持宽高比由窗口管理
                canvas.copy(tex, None, None)?;
                canvas.present();
            }
        }

        // 5. 短暂休眠避免 CPU 空转 (~120fps 轮询上限)
        std::thread::sleep(Duration::from_millis(8));
    }

    Ok(())
}
