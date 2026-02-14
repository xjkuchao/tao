//! 视频显示模块.
//!
//! 使用 minifb 进行窗口渲染.

use log::debug;
use minifb::{Key, Window, WindowOptions};

/// 视频显示窗口
pub struct VideoDisplay {
    /// minifb 窗口
    window: Window,
    /// 窗口宽度
    width: usize,
    /// 窗口高度
    height: usize,
    /// 像素缓冲区 (ARGB 格式)
    buffer: Vec<u32>,
}

impl VideoDisplay {
    /// 创建视频显示窗口
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self, String> {
        let w = width as usize;
        let h = height as usize;

        let window = Window::new(
            title,
            w,
            h,
            WindowOptions {
                resize: true,
                scale_mode: minifb::ScaleMode::AspectRatioStretch,
                ..WindowOptions::default()
            },
        )
        .map_err(|e| format!("创建窗口失败: {}", e))?;

        debug!("视频窗口已创建: {}x{}", w, h);

        Ok(Self {
            window,
            width: w,
            height: h,
            buffer: vec![0u32; w * h],
        })
    }

    /// 显示一帧 RGB24 数据
    pub fn display_rgb24(&mut self, data: &[u8], src_width: u32, src_height: u32) {
        let sw = src_width as usize;
        let sh = src_height as usize;

        // 转换 RGB24 -> ARGB (minifb 使用 0RGB 格式)
        if sw == self.width && sh == self.height {
            // 直接映射
            for y in 0..sh.min(self.height) {
                for x in 0..sw.min(self.width) {
                    let src_idx = (y * sw + x) * 3;
                    if src_idx + 2 < data.len() {
                        let r = data[src_idx] as u32;
                        let g = data[src_idx + 1] as u32;
                        let b = data[src_idx + 2] as u32;
                        self.buffer[y * self.width + x] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        } else {
            // 简单缩放 (最近邻)
            for y in 0..self.height {
                let src_y = y * sh / self.height;
                for x in 0..self.width {
                    let src_x = x * sw / self.width;
                    let src_idx = (src_y * sw + src_x) * 3;
                    if src_idx + 2 < data.len() {
                        let r = data[src_idx] as u32;
                        let g = data[src_idx + 1] as u32;
                        let b = data[src_idx + 2] as u32;
                        self.buffer[y * self.width + x] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }

        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }

    /// 检查窗口是否仍然打开
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// 检查是否按下了 ESC 或 Q 键
    pub fn should_quit(&self) -> bool {
        self.window.is_key_down(Key::Escape) || self.window.is_key_down(Key::Q)
    }

    /// 检查是否按下了空格键 (暂停/继续)
    pub fn is_space_pressed(&self) -> bool {
        self.window.is_key_pressed(Key::Space, minifb::KeyRepeat::No)
    }

    /// 更新窗口事件 (无新帧时调用)
    pub fn update(&mut self) {
        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }
}
