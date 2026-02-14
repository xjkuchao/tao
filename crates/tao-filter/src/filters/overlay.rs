//! 视频叠加滤镜.
//!
//! 将静态图像叠加到视频帧的指定位置, 支持 alpha 混合.

use tao_codec::frame::{Frame, VideoFrame};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::Filter;

/// 视频叠加滤镜
///
/// 将 RGB24 格式的静态图像叠加到视频帧上, 支持 alpha 透明度.
pub struct OverlayFilter {
    /// 叠加区域左上角 X 坐标
    x: u32,
    /// 叠加区域左上角 Y 坐标
    y: u32,
    /// 叠加图像宽度
    overlay_width: u32,
    /// 叠加图像高度
    overlay_height: u32,
    /// 叠加图像数据 (RGB24 格式)
    overlay_data: Vec<u8>,
    /// 透明度 (0.0 = 完全透明, 1.0 = 完全不透明)
    alpha: f32,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl OverlayFilter {
    /// 创建叠加滤镜
    ///
    /// # 参数
    /// - `overlay_data`: RGB24 格式, 长度必须为 overlay_width * overlay_height * 3
    pub fn new(
        x: u32,
        y: u32,
        overlay_width: u32,
        overlay_height: u32,
        overlay_data: Vec<u8>,
        alpha: f32,
    ) -> Self {
        Self {
            x,
            y,
            overlay_width,
            overlay_height,
            overlay_data,
            alpha: alpha.clamp(0.0, 1.0),
            output: None,
        }
    }

    /// 创建纯色叠加
    pub fn from_solid_color(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        color: (u8, u8, u8),
        alpha: f32,
    ) -> Self {
        let (r, g, b) = color;
        let size = (width as usize) * (height as usize) * 3;
        let mut overlay_data = Vec::with_capacity(size);
        for _ in 0..(width as usize * height as usize) {
            overlay_data.push(r);
            overlay_data.push(g);
            overlay_data.push(b);
        }
        Self::new(x, y, width, height, overlay_data, alpha)
    }

    /// 将叠加图像混合到 RGB24 帧
    fn blend_overlay(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        let mut out = frame.clone();
        let stride = frame.linesize[0];
        let frame_data = &mut out.data[0];
        let frame_width = frame.width as usize;
        let frame_height = frame.height as usize;

        // 计算实际叠加区域 (裁剪到帧内)
        let start_x = self.x as usize;
        let start_y = self.y as usize;
        let end_x = (start_x + self.overlay_width as usize).min(frame_width);
        let end_y = (start_y + self.overlay_height as usize).min(frame_height);

        if start_x >= frame_width || start_y >= frame_height {
            return Ok(out);
        }

        let overlay_stride = (self.overlay_width as usize) * 3;
        let alpha = self.alpha;

        for dy in start_y..end_y {
            let overlay_row = dy - start_y;
            let overlay_off = overlay_row * overlay_stride;
            let frame_off = dy * stride + start_x * 3;

            for dx in start_x..end_x {
                let overlay_col = dx - start_x;
                let overlay_px = overlay_off + overlay_col * 3;
                let frame_px = frame_off + (dx - start_x) * 3;

                if overlay_px + 3 <= self.overlay_data.len() && frame_px + 3 <= frame_data.len() {
                    for c in 0..3 {
                        let bg = frame_data[frame_px + c] as f32;
                        let fg = self.overlay_data[overlay_px + c] as f32;
                        let blended = fg * alpha + bg * (1.0 - alpha);
                        frame_data[frame_px + c] = blended.round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }

        Ok(out)
    }
}

impl Filter for OverlayFilter {
    fn name(&self) -> &str {
        "overlay"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Video(vf) => {
                if vf.pixel_format == PixelFormat::Rgb24 {
                    let result = self.blend_overlay(vf)?;
                    self.output = Some(Frame::Video(result));
                } else {
                    self.output = Some(frame.clone());
                }
                Ok(())
            }
            Frame::Audio(_) => {
                self.output = Some(frame.clone());
                Ok(())
            }
        }
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        self.output.take().ok_or(TaoError::NeedMoreData)
    }

    fn flush(&mut self) -> TaoResult<()> {
        self.output = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao_core::Rational;

    fn make_rgb_frame(width: u32, height: u32, r: u8, g: u8, b: u8) -> Frame {
        let stride = (width as usize) * 3;
        let mut data = vec![0u8; stride * (height as usize)];
        for i in 0..data.len() / 3 {
            data[i * 3] = r;
            data[i * 3 + 1] = g;
            data[i * 3 + 2] = b;
        }
        let mut vf = VideoFrame::new(width, height, PixelFormat::Rgb24);
        vf.data = vec![data];
        vf.linesize = vec![stride];
        vf.time_base = Rational::new(1, 30);
        Frame::Video(vf)
    }

    #[test]
    fn test_solid_color_overlay() {
        let mut filter = OverlayFilter::from_solid_color(50, 50, 100, 100, (255, 0, 0), 1.0);
        let input = make_rgb_frame(200, 200, 0, 0, 0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 200);
            assert_eq!(vf.height, 200);
            let stride = vf.linesize[0];
            let center_pixel = 100 * stride + 100 * 3;
            assert_eq!(vf.data[0][center_pixel], 255);
            assert_eq!(vf.data[0][center_pixel + 1], 0);
            assert_eq!(vf.data[0][center_pixel + 2], 0);
            let corner = 0 * stride + 0 * 3;
            assert_eq!(vf.data[0][corner], 0);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_alpha_blending() {
        let mut filter = OverlayFilter::from_solid_color(0, 0, 100, 100, (255, 0, 0), 0.5);
        let input = make_rgb_frame(100, 100, 0, 0, 255);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            let center = 50 * vf.linesize[0] + 50 * 3;
            let r = vf.data[0][center];
            let b = vf.data[0][center + 2];
            assert!((r as i32 - 127).abs() < 5);
            assert!((b as i32 - 127).abs() < 5);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_overlay_out_of_bounds() {
        let mut filter = OverlayFilter::from_solid_color(150, 150, 100, 100, (255, 0, 0), 1.0);
        let input = make_rgb_frame(200, 200, 0, 0, 0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 200);
            assert_eq!(vf.height, 200);
            let stride = vf.linesize[0];
            let at_overlay = 175 * stride + 175 * 3;
            assert_eq!(vf.data[0][at_overlay], 255);
            let outside = 50 * stride + 50 * 3;
            assert_eq!(vf.data[0][outside], 0);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_passthrough_non_rgb() {
        let mut filter = OverlayFilter::from_solid_color(0, 0, 10, 10, (255, 0, 0), 1.0);
        let mut vf = VideoFrame::new(100, 100, PixelFormat::Yuv420p);
        vf.data = vec![vec![128; 100 * 100], vec![128; 50 * 50], vec![128; 50 * 50]];
        vf.linesize = vec![100, 50, 50];
        let input = Frame::Video(vf.clone());
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(out_vf) = &output {
            assert_eq!(out_vf.pixel_format, PixelFormat::Yuv420p);
            assert_eq!(out_vf.data, vf.data);
        } else {
            panic!("期望视频帧");
        }
    }
}
