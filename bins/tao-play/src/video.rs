//! 视频显示模块.
//!
//! 使用 minifb 进行窗口渲染, 支持 OSD 进度条和控制信息.

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
    /// OSD 显示倒计时 (帧数)
    osd_timer: u32,
    /// OSD 文本
    osd_text: String,
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
            osd_timer: 0,
            osd_text: String::new(),
        })
    }

    /// 显示一帧 RGB24 数据
    pub fn display_rgb24(&mut self, data: &[u8], src_width: u32, src_height: u32) {
        let sw = src_width as usize;
        let sh = src_height as usize;

        if sw == self.width && sh == self.height {
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

        // 绘制 OSD 进度条
        if self.osd_timer > 0 {
            self.osd_timer -= 1;
        }

        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }

    /// 在窗口底部绘制进度条 OSD
    pub fn draw_progress_bar(
        &mut self,
        current_sec: f64,
        total_sec: f64,
        volume_pct: u32,
        is_paused: bool,
    ) {
        if total_sec <= 0.0 || self.height < 20 {
            return;
        }

        let bar_height: usize = 8;
        let bar_y: usize = self.height - bar_height - 4;
        let bar_x: usize = 8;
        let bar_width: usize = self.width - 16;

        let progress = (current_sec / total_sec).clamp(0.0, 1.0);
        let filled = (bar_width as f64 * progress) as usize;

        // 绘制背景 (半透明黑)
        for y in bar_y.saturating_sub(2)..self.height.min(bar_y + bar_height + 2) {
            for x in bar_x.saturating_sub(2)..self.width.min(bar_x + bar_width + 2) {
                let idx = y * self.width + x;
                if idx < self.buffer.len() {
                    // 半透明: 将现有像素变暗
                    let pixel = self.buffer[idx];
                    let r = ((pixel >> 16) & 0xFF) / 3;
                    let g = ((pixel >> 8) & 0xFF) / 3;
                    let b = (pixel & 0xFF) / 3;
                    self.buffer[idx] = (r << 16) | (g << 8) | b;
                }
            }
        }

        // 绘制进度条
        for y in bar_y..self.height.min(bar_y + bar_height) {
            for x in bar_x..self.width.min(bar_x + bar_width) {
                let idx = y * self.width + x;
                if idx < self.buffer.len() {
                    let rel_x = x - bar_x;
                    if rel_x < filled {
                        // 已播放部分 (蓝色)
                        self.buffer[idx] = 0x0077CC;
                    } else {
                        // 未播放部分 (灰色)
                        self.buffer[idx] = 0x444444;
                    }
                }
            }
        }

        // 绘制时间文本区域 (简化: 在左下角画像素块表示暂停/音量)
        if is_paused {
            // 暂停标志: 两条竖线
            let px = 4;
            let py = bar_y.saturating_sub(14);
            for y in py..py + 10 {
                for x in px..px + 3 {
                    let idx = y * self.width + x;
                    if idx < self.buffer.len() {
                        self.buffer[idx] = 0xFFFFFF;
                    }
                }
                for x in px + 5..px + 8 {
                    let idx = y * self.width + x;
                    if idx < self.buffer.len() {
                        self.buffer[idx] = 0xFFFFFF;
                    }
                }
            }
        }

        // 音量指示 (右侧小方块)
        let vol_blocks = (volume_pct / 10).min(10) as usize;
        let vol_x = self.width.saturating_sub(20);
        let vol_y = bar_y.saturating_sub(14);
        for i in 0..vol_blocks {
            let bx = vol_x;
            let by = vol_y + (9 - i);
            let idx = by * self.width + bx;
            if idx < self.buffer.len() {
                self.buffer[idx] = 0x00FF00;
                if bx + 1 < self.width {
                    self.buffer[idx + 1] = 0x00FF00;
                }
            }
        }

        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }

    /// 显示 OSD 消息
    pub fn show_osd(&mut self, text: &str) {
        self.osd_text = text.to_string();
        self.osd_timer = 90; // 约 3 秒 (30fps)
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
        self.window
            .is_key_pressed(Key::Space, minifb::KeyRepeat::No)
    }

    /// 检查左方向键 (快退)
    pub fn is_left_pressed(&self) -> bool {
        self.window
            .is_key_pressed(Key::Left, minifb::KeyRepeat::Yes)
    }

    /// 检查右方向键 (快进)
    pub fn is_right_pressed(&self) -> bool {
        self.window
            .is_key_pressed(Key::Right, minifb::KeyRepeat::Yes)
    }

    /// 检查上方向键 (音量+)
    pub fn is_up_pressed(&self) -> bool {
        self.window.is_key_pressed(Key::Up, minifb::KeyRepeat::Yes)
    }

    /// 检查下方向键 (音量-)
    pub fn is_down_pressed(&self) -> bool {
        self.window
            .is_key_pressed(Key::Down, minifb::KeyRepeat::Yes)
    }

    /// 检查 M 键 (静音)
    pub fn is_mute_pressed(&self) -> bool {
        self.window.is_key_pressed(Key::M, minifb::KeyRepeat::No)
    }

    /// 检查 F 键 (全屏 - 在 minifb 中通过提示实现)
    pub fn is_fullscreen_pressed(&self) -> bool {
        self.window.is_key_pressed(Key::F, minifb::KeyRepeat::No)
    }

    /// 更新窗口事件 (无新帧时调用)
    pub fn update(&mut self) {
        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }

    /// 获取 OSD 文本 (调试用)
    #[allow(dead_code)]
    pub fn osd_text(&self) -> &str {
        &self.osd_text
    }
}
