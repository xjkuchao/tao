//! 视频填充 (黑边) 滤镜.
//!
//! 对标 FFmpeg 的 `pad` 滤镜, 在视频帧周围添加填充.

use tao_codec::frame::{Frame, VideoFrame};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::Filter;

/// 填充颜色 (RGB)
#[derive(Debug, Clone, Copy)]
pub struct PadColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl PadColor {
    /// 黑色
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
}

impl Default for PadColor {
    fn default() -> Self {
        Self::BLACK
    }
}

/// 视频填充滤镜
pub struct PadFilter {
    /// 输出宽度
    out_width: u32,
    /// 输出高度
    out_height: u32,
    /// 原图放置 X 偏移
    x: u32,
    /// 原图放置 Y 偏移
    y: u32,
    /// 填充颜色
    color: PadColor,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl PadFilter {
    /// 创建填充滤镜
    pub fn new(out_width: u32, out_height: u32, x: u32, y: u32) -> Self {
        Self {
            out_width,
            out_height,
            x,
            y,
            color: PadColor::BLACK,
            output: None,
        }
    }

    /// 创建填充滤镜 (指定颜色)
    pub fn with_color(out_width: u32, out_height: u32, x: u32, y: u32, color: PadColor) -> Self {
        Self {
            out_width,
            out_height,
            x,
            y,
            color,
            output: None,
        }
    }

    /// 对视频帧添加填充
    fn pad_frame(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        if self.x + frame.width > self.out_width || self.y + frame.height > self.out_height {
            return Err(TaoError::InvalidArgument(format!(
                "pad: 原图 ({}x{}) 放置在 ({}, {}) 超出输出 ({}x{})",
                frame.width, frame.height, self.x, self.y, self.out_width, self.out_height,
            )));
        }

        let bpp = bytes_per_pixel_packed(frame.pixel_format);
        if bpp == 0 {
            return Err(TaoError::Unsupported(format!(
                "pad: 不支持像素格式 {:?}",
                frame.pixel_format,
            )));
        }

        let mut out = VideoFrame::new(self.out_width, self.out_height, frame.pixel_format);
        out.pts = frame.pts;
        out.time_base = frame.time_base;
        out.duration = frame.duration;
        out.is_keyframe = frame.is_keyframe;

        let dst_stride = self.out_width as usize * bpp;
        let mut dst = vec![0u8; dst_stride * self.out_height as usize];

        // 填充背景色
        for row in 0..self.out_height as usize {
            for col in 0..self.out_width as usize {
                let off = row * dst_stride + col * bpp;
                match bpp {
                    3 => {
                        dst[off] = self.color.r;
                        dst[off + 1] = self.color.g;
                        dst[off + 2] = self.color.b;
                    }
                    4 => {
                        dst[off] = self.color.r;
                        dst[off + 1] = self.color.g;
                        dst[off + 2] = self.color.b;
                        dst[off + 3] = 255;
                    }
                    1 => {
                        dst[off] = ((self.color.r as u32 * 77
                            + self.color.g as u32 * 150
                            + self.color.b as u32 * 29)
                            >> 8) as u8;
                    }
                    _ => {}
                }
            }
        }

        // 复制原图到指定位置
        let src = &frame.data[0];
        let src_stride = frame.linesize[0];
        let copy_w = frame.width as usize * bpp;

        for row in 0..frame.height as usize {
            let src_off = row * src_stride;
            let dst_y = self.y as usize + row;
            let dst_off = dst_y * dst_stride + self.x as usize * bpp;
            if src_off + copy_w <= src.len() && dst_off + copy_w <= dst.len() {
                dst[dst_off..dst_off + copy_w]
                    .copy_from_slice(&src[src_off..src_off + copy_w]);
            }
        }

        out.data = vec![dst];
        out.linesize = vec![dst_stride];
        Ok(out)
    }
}

impl Filter for PadFilter {
    fn name(&self) -> &str {
        "pad"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Video(vf) => {
                let result = self.pad_frame(vf)?;
                self.output = Some(Frame::Video(result));
                Ok(())
            }
            Frame::Audio(_) => Err(TaoError::InvalidArgument(
                "pad 滤镜仅支持视频帧".into(),
            )),
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

/// packed 格式每像素字节数
fn bytes_per_pixel_packed(fmt: PixelFormat) -> usize {
    match fmt {
        PixelFormat::Rgb24 | PixelFormat::Bgr24 => 3,
        PixelFormat::Rgba | PixelFormat::Bgra | PixelFormat::Argb => 4,
        PixelFormat::Gray8 => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao_core::Rational;

    fn make_solid_rgb(width: u32, height: u32, r: u8, g: u8, b: u8) -> Frame {
        let bpp = 3;
        let stride = width as usize * bpp;
        let mut data = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let off = y * stride + x * bpp;
                data[off] = r;
                data[off + 1] = g;
                data[off + 2] = b;
            }
        }
        let mut vf = VideoFrame::new(width, height, PixelFormat::Rgb24);
        vf.data = vec![data];
        vf.linesize = vec![stride];
        vf.time_base = Rational::new(1, 30);
        Frame::Video(vf)
    }

    #[test]
    fn test_pad_黑边() {
        let mut filter = PadFilter::new(10, 10, 2, 2);
        let input = make_solid_rgb(6, 6, 255, 0, 0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 10);
            assert_eq!(vf.height, 10);
            assert_eq!(vf.data[0][0], 0);
            assert_eq!(vf.data[0][1], 0);
            assert_eq!(vf.data[0][2], 0);
            let off = 2 * 30 + 2 * 3;
            assert_eq!(vf.data[0][off], 255);
            assert_eq!(vf.data[0][off + 1], 0);
            assert_eq!(vf.data[0][off + 2], 0);
        }
    }

    #[test]
    fn test_pad_越界报错() {
        let mut filter = PadFilter::new(8, 8, 5, 0);
        let input = make_solid_rgb(6, 6, 0, 0, 0);
        assert!(filter.send_frame(&input).is_err());
    }

    #[test]
    fn test_pad_自定义颜色() {
        let color = PadColor {
            r: 0,
            g: 255,
            b: 0,
        };
        let mut filter = PadFilter::with_color(6, 6, 1, 1, color);
        let input = make_solid_rgb(4, 4, 255, 0, 0);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.data[0][0], 0);
            assert_eq!(vf.data[0][1], 255);
            assert_eq!(vf.data[0][2], 0);
        }
    }
}
