//! 文字绘制滤镜.
//!
//! 在视频帧上绘制文本, 使用内置 5x7 点阵字体.

use tao_codec::frame::{Frame, VideoFrame};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::Filter;

/// 5x7 点阵字体 (ASCII 32-126)
/// 每字符 5 列 7 行, 每列一个字节, 低位对应上方像素
const FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // space
    [0x00, 0x00, 0x5F, 0x00, 0x00], // !
    [0x00, 0x07, 0x00, 0x07, 0x00], // "
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // #
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // $
    [0x23, 0x13, 0x08, 0x64, 0x62], // %
    [0x36, 0x49, 0x56, 0x20, 0x50], // &
    [0x00, 0x08, 0x07, 0x03, 0x00], // '
    [0x00, 0x1C, 0x22, 0x41, 0x00], // (
    [0x00, 0x41, 0x22, 0x1C, 0x00], // )
    [0x2A, 0x1C, 0x7F, 0x1C, 0x2A], // *
    [0x08, 0x08, 0x3E, 0x08, 0x08], // +
    [0x00, 0x80, 0x70, 0x30, 0x00], // ,
    [0x08, 0x08, 0x08, 0x08, 0x08], // -
    [0x00, 0x00, 0x60, 0x60, 0x00], // .
    [0x20, 0x10, 0x08, 0x04, 0x02], // /
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 0
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 1
    [0x72, 0x49, 0x49, 0x49, 0x46], // 2
    [0x21, 0x41, 0x49, 0x4D, 0x33], // 3
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 4
    [0x27, 0x45, 0x45, 0x45, 0x39], // 5
    [0x3C, 0x4A, 0x49, 0x49, 0x31], // 6
    [0x41, 0x21, 0x11, 0x09, 0x07], // 7
    [0x36, 0x49, 0x49, 0x49, 0x36], // 8
    [0x46, 0x49, 0x49, 0x29, 0x1E], // 9
    [0x00, 0x00, 0x14, 0x00, 0x00], // :
    [0x00, 0x40, 0x34, 0x00, 0x00], // ;
    [0x00, 0x08, 0x14, 0x22, 0x41], // <
    [0x14, 0x14, 0x14, 0x14, 0x14], // =
    [0x00, 0x41, 0x22, 0x14, 0x08], // >
    [0x02, 0x01, 0x59, 0x09, 0x06], // ?
    [0x3E, 0x41, 0x5D, 0x59, 0x4E], // @
    [0x7C, 0x12, 0x11, 0x12, 0x7C], // A
    [0x7F, 0x49, 0x49, 0x49, 0x36], // B
    [0x3E, 0x41, 0x41, 0x41, 0x22], // C
    [0x7F, 0x41, 0x41, 0x41, 0x3E], // D
    [0x7F, 0x49, 0x49, 0x49, 0x41], // E
    [0x7F, 0x09, 0x09, 0x09, 0x01], // F
    [0x3E, 0x41, 0x41, 0x51, 0x73], // G
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // H
    [0x00, 0x41, 0x7F, 0x41, 0x00], // I
    [0x20, 0x40, 0x41, 0x3F, 0x01], // J
    [0x7F, 0x08, 0x14, 0x22, 0x41], // K
    [0x7F, 0x40, 0x40, 0x40, 0x40], // L
    [0x7F, 0x02, 0x1C, 0x02, 0x7F], // M
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // N
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // O
    [0x7F, 0x09, 0x09, 0x09, 0x06], // P
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // Q
    [0x7F, 0x09, 0x19, 0x29, 0x46], // R
    [0x26, 0x49, 0x49, 0x49, 0x32], // S
    [0x03, 0x01, 0x7F, 0x01, 0x03], // T
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // U
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // V
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // W
    [0x63, 0x14, 0x08, 0x14, 0x63], // X
    [0x03, 0x04, 0x78, 0x04, 0x03], // Y
    [0x61, 0x59, 0x49, 0x4D, 0x43], // Z
    [0x00, 0x7F, 0x41, 0x41, 0x41], // [
    [0x02, 0x04, 0x08, 0x10, 0x20], // \
    [0x00, 0x41, 0x41, 0x41, 0x7F], // ]
    [0x04, 0x02, 0x01, 0x02, 0x04], // ^
    [0x40, 0x40, 0x40, 0x40, 0x40], // _
    [0x00, 0x03, 0x07, 0x08, 0x00], // `
    [0x20, 0x54, 0x54, 0x78, 0x40], // a
    [0x7F, 0x28, 0x44, 0x44, 0x38], // b
    [0x38, 0x44, 0x44, 0x44, 0x28], // c
    [0x38, 0x44, 0x44, 0x28, 0x7F], // d
    [0x38, 0x54, 0x54, 0x54, 0x18], // e
    [0x00, 0x08, 0x7E, 0x09, 0x02], // f
    [0x18, 0xA4, 0xA4, 0x9C, 0x78], // g
    [0x7F, 0x08, 0x04, 0x04, 0x78], // h
    [0x00, 0x44, 0x7D, 0x40, 0x00], // i
    [0x20, 0x40, 0x40, 0x3D, 0x00], // j
    [0x7F, 0x10, 0x28, 0x44, 0x00], // k
    [0x00, 0x41, 0x7F, 0x40, 0x00], // l
    [0x7C, 0x04, 0x78, 0x04, 0x78], // m
    [0x7C, 0x08, 0x04, 0x04, 0x78], // n
    [0x38, 0x44, 0x44, 0x44, 0x38], // o
    [0xFC, 0x18, 0x24, 0x24, 0x18], // p
    [0x18, 0x24, 0x24, 0x18, 0xFC], // q
    [0x7C, 0x08, 0x04, 0x04, 0x08], // r
    [0x48, 0x54, 0x54, 0x54, 0x24], // s
    [0x04, 0x04, 0x3F, 0x44, 0x24], // t
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // u
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // v
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // w
    [0x44, 0x28, 0x10, 0x28, 0x44], // x
    [0x4C, 0x90, 0x90, 0x90, 0x7C], // y
    [0x44, 0x64, 0x54, 0x4C, 0x44], // z
    [0x00, 0x08, 0x36, 0x41, 0x00], // {
    [0x00, 0x00, 0x77, 0x00, 0x00], // |
    [0x00, 0x41, 0x36, 0x08, 0x00], // }
    [0x02, 0x01, 0x02, 0x04, 0x02], // ~
];

/// 绘制目标 (用于减少 draw_char 参数数量)
struct DrawTarget<'a> {
    data: &'a mut [u8],
    stride: usize,
    width: usize,
    height: usize,
}

/// 文字绘制滤镜
pub struct DrawtextFilter {
    /// 要绘制的文本
    text: String,
    /// 文字左上角 X 坐标
    x: u32,
    /// 文字左上角 Y 坐标
    y: u32,
    /// 文字颜色 (R, G, B)
    color: (u8, u8, u8),
    /// 字体缩放倍数 (1=5x7, 2=10x14 等)
    font_scale: u32,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl DrawtextFilter {
    /// 创建文字绘制滤镜
    pub fn new(text: &str, x: u32, y: u32, color: (u8, u8, u8), font_scale: u32) -> Self {
        Self {
            text: text.to_string(),
            x,
            y,
            color,
            font_scale: font_scale.max(1),
            output: None,
        }
    }

    /// 在 RGB24 数据上绘制单个字符
    fn draw_char(&self, target: &mut DrawTarget<'_>, char_idx: usize, base_x: i32, base_y: i32) {
        if char_idx >= 95 {
            return;
        }
        let glyph = &FONT_5X7[char_idx];
        let scale = self.font_scale as usize;
        let (r, g, b) = self.color;

        for (col, &glyph_col) in glyph.iter().enumerate() {
            for row in 0..7 {
                if (glyph_col >> row) & 1 == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = base_x + (col * scale + sx) as i32;
                        let py = base_y + (row * scale + sy) as i32;
                        if px >= 0
                            && py >= 0
                            && px < target.width as i32
                            && py < target.height as i32
                        {
                            let off = (py as usize) * target.stride + (px as usize) * 3;
                            if off + 3 <= target.data.len() {
                                target.data[off] = r;
                                target.data[off + 1] = g;
                                target.data[off + 2] = b;
                            }
                        }
                    }
                }
            }
        }
    }

    /// 在 RGB24 帧上绘制完整文本
    fn draw_text(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        let mut out = frame.clone();
        let data = &mut out.data[0];
        let stride = frame.linesize[0];
        let width = frame.width as usize;
        let height = frame.height as usize;
        let scale = self.font_scale as usize;
        let char_width = 6 * scale;

        let mut cx = self.x as i32;
        let cy = self.y as i32;

        for c in self.text.chars() {
            let byte = c as u8;
            if !(32..=126).contains(&byte) {
                continue;
            }
            let char_idx = (byte - 32) as usize;
            let mut target = DrawTarget {
                data,
                stride,
                width,
                height,
            };
            self.draw_char(&mut target, char_idx, cx, cy);
            cx += char_width as i32;
        }

        Ok(out)
    }
}

impl Filter for DrawtextFilter {
    fn name(&self) -> &str {
        "drawtext"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Video(vf) => {
                if vf.pixel_format == PixelFormat::Rgb24 {
                    if self.text.is_empty() {
                        self.output = Some(frame.clone());
                    } else {
                        let result = self.draw_text(vf)?;
                        self.output = Some(Frame::Video(result));
                    }
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

    fn make_rgb_frame(width: u32, height: u32) -> Frame {
        let stride = (width as usize) * 3;
        let data = vec![0u8; stride * (height as usize)];
        let mut vf = VideoFrame::new(width, height, PixelFormat::Rgb24);
        vf.data = vec![data];
        vf.linesize = vec![stride];
        vf.time_base = Rational::new(1, 30);
        Frame::Video(vf)
    }

    #[test]
    fn test_draw_text_basic() {
        let mut filter = DrawtextFilter::new("Hello", 10, 10, (255, 255, 255), 1);
        let input = make_rgb_frame(100, 100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 100);
            assert_eq!(vf.height, 100);
            let stride = vf.linesize[0];
            let center_y = 13;
            let center_x = 12;
            let off = center_y * stride + center_x * 3;
            assert_eq!(vf.data[0][off], 255);
            assert_eq!(vf.data[0][off + 1], 255);
            assert_eq!(vf.data[0][off + 2], 255);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_font_scale() {
        let mut filter = DrawtextFilter::new("A", 0, 0, (255, 0, 0), 2);
        let input = make_rgb_frame(50, 50);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            let stride = vf.linesize[0];
            let mut count = 0;
            for y in 0..14u32 {
                for x in 0..10u32 {
                    let off = (y as usize) * stride + (x as usize) * 3;
                    if vf.data[0][off] == 255
                        && vf.data[0][off + 1] == 0
                        && vf.data[0][off + 2] == 0
                    {
                        count += 1;
                    }
                }
            }
            assert!(count > 0);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_empty_text() {
        let mut filter = DrawtextFilter::new("", 10, 10, (255, 255, 255), 1);
        let input = make_rgb_frame(100, 100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 100);
            assert_eq!(vf.height, 100);
            assert!(vf.data[0].iter().all(|&b| b == 0));
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_text_outside_frame() {
        let mut filter = DrawtextFilter::new("Hi", 95, 95, (255, 255, 255), 1);
        let input = make_rgb_frame(100, 100);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 100);
            assert_eq!(vf.height, 100);
        } else {
            panic!("期望视频帧");
        }
    }
}
