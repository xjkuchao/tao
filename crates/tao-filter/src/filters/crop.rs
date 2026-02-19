//! 视频裁剪滤镜.
//!
//! 对标 FFmpeg 的 `crop` 滤镜, 从视频帧中裁剪指定区域.

use tao_codec::frame::{Frame, VideoFrame};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::Filter;

/// 视频裁剪滤镜
pub struct CropFilter {
    /// 裁剪区域左上角 X 坐标
    x: u32,
    /// 裁剪区域左上角 Y 坐标
    y: u32,
    /// 裁剪后宽度
    width: u32,
    /// 裁剪后高度
    height: u32,
    /// 输出帧缓冲
    output: Option<Frame>,
}

impl CropFilter {
    /// 创建裁剪滤镜
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            output: None,
        }
    }

    /// 裁剪视频帧
    fn crop_frame(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        if self.x + self.width > frame.width || self.y + self.height > frame.height {
            return Err(TaoError::InvalidArgument(format!(
                "crop: 裁剪区域 ({}+{}, {}+{}) 超出帧大小 ({}x{})",
                self.x, self.width, self.y, self.height, frame.width, frame.height,
            )));
        }

        let bpp = bytes_per_pixel(frame.pixel_format);
        let is_planar = matches!(
            frame.pixel_format,
            PixelFormat::Yuv420p | PixelFormat::Yuv422p | PixelFormat::Yuv444p
        );

        if is_planar {
            self.crop_planar(frame)
        } else if bpp > 0 {
            self.crop_packed(frame, bpp)
        } else {
            Err(TaoError::Unsupported(format!(
                "crop: 不支持像素格式 {:?}",
                frame.pixel_format,
            )))
        }
    }

    /// 裁剪 packed 格式
    fn crop_packed(&self, frame: &VideoFrame, bpp: usize) -> TaoResult<VideoFrame> {
        let mut out = VideoFrame::new(self.width, self.height, frame.pixel_format);
        out.pts = frame.pts;
        out.time_base = frame.time_base;
        out.duration = frame.duration;
        out.is_keyframe = frame.is_keyframe;

        let src = &frame.data[0];
        let src_stride = frame.linesize[0];
        let dst_stride = self.width as usize * bpp;
        let mut dst = vec![0u8; dst_stride * self.height as usize];

        for row in 0..self.height as usize {
            let src_y = self.y as usize + row;
            let src_off = src_y * src_stride + self.x as usize * bpp;
            let dst_off = row * dst_stride;
            if src_off + dst_stride <= src.len() {
                dst[dst_off..dst_off + dst_stride]
                    .copy_from_slice(&src[src_off..src_off + dst_stride]);
            }
        }

        out.data = vec![dst];
        out.linesize = vec![dst_stride];
        Ok(out)
    }

    /// 裁剪 planar YUV 格式
    fn crop_planar(&self, frame: &VideoFrame) -> TaoResult<VideoFrame> {
        let (sub_h, sub_v) = frame.pixel_format.chroma_subsampling();

        let mut out = VideoFrame::new(self.width, self.height, frame.pixel_format);
        out.pts = frame.pts;
        out.time_base = frame.time_base;
        out.duration = frame.duration;
        out.is_keyframe = frame.is_keyframe;

        let y_plane = crop_plane(
            &frame.data[0],
            frame.linesize[0],
            self.x as usize,
            self.y as usize,
            self.width as usize,
            self.height as usize,
        );

        let cx = (self.x as usize) >> sub_h;
        let cy = (self.y as usize) >> sub_v;
        let cw = (self.width as usize) >> sub_h;
        let ch = (self.height as usize) >> sub_v;

        let u_plane = crop_plane(&frame.data[1], frame.linesize[1], cx, cy, cw, ch);
        let v_plane = crop_plane(&frame.data[2], frame.linesize[2], cx, cy, cw, ch);

        out.data = vec![y_plane, u_plane, v_plane];
        out.linesize = vec![self.width as usize, cw, cw];
        Ok(out)
    }
}

impl Filter for CropFilter {
    fn name(&self) -> &str {
        "crop"
    }

    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()> {
        match frame {
            Frame::Video(vf) => {
                let result = self.crop_frame(vf)?;
                self.output = Some(Frame::Video(result));
                Ok(())
            }
            Frame::Audio(_) => Err(TaoError::InvalidArgument("crop 滤镜仅支持视频帧".into())),
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

/// 裁剪单个平面
fn crop_plane(
    src: &[u8],
    src_stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut dst = vec![0u8; width * height];
    for row in 0..height {
        let src_off = (y + row) * src_stride + x;
        let dst_off = row * width;
        if src_off + width <= src.len() {
            dst[dst_off..dst_off + width].copy_from_slice(&src[src_off..src_off + width]);
        }
    }
    dst
}

/// 获取每像素字节数 (packed 格式)
fn bytes_per_pixel(fmt: PixelFormat) -> usize {
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

    fn make_rgb_frame(width: u32, height: u32) -> Frame {
        let bpp = 3;
        let stride = width as usize * bpp;
        let mut data = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let off = y * stride + x * bpp;
                data[off] = x as u8;
                data[off + 1] = y as u8;
                data[off + 2] = 128;
            }
        }
        let mut vf = VideoFrame::new(width, height, PixelFormat::Rgb24);
        vf.data = vec![data];
        vf.linesize = vec![stride];
        vf.time_base = Rational::new(1, 30);
        Frame::Video(vf)
    }

    #[test]
    fn test_crop_basic() {
        let mut filter = CropFilter::new(2, 3, 4, 5);
        let input = make_rgb_frame(10, 10);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 4);
            assert_eq!(vf.height, 5);
            assert_eq!(vf.data[0][0], 2);
            assert_eq!(vf.data[0][1], 3);
            assert_eq!(vf.data[0][2], 128);
        } else {
            panic!("期望视频帧");
        }
    }

    #[test]
    fn test_crop_out_of_bounds_error() {
        let mut filter = CropFilter::new(8, 0, 4, 4);
        let input = make_rgb_frame(10, 10);
        assert!(filter.send_frame(&input).is_err());
    }

    #[test]
    fn test_crop_full_frame() {
        let mut filter = CropFilter::new(0, 0, 10, 10);
        let input = make_rgb_frame(10, 10);
        filter.send_frame(&input).unwrap();
        let output = filter.receive_frame().unwrap();
        if let Frame::Video(vf) = &output {
            assert_eq!(vf.width, 10);
            assert_eq!(vf.height, 10);
        }
    }

    #[test]
    fn test_crop_audio_frame_error() {
        let mut filter = CropFilter::new(0, 0, 4, 4);
        let af = Frame::Audio(tao_codec::frame::AudioFrame::new(
            1024,
            44100,
            tao_core::SampleFormat::F32,
            tao_core::ChannelLayout::from_channels(2),
        ));
        assert!(filter.send_frame(&af).is_err());
    }
}
