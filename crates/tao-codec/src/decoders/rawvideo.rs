//! RAW 视频解码器.
//!
//! 将未压缩的原始像素数据从 Packet 转换为 VideoFrame.
//! 不做任何压缩/解压缩, 仅按像素格式拆分平面数据.

use log::debug;
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;

/// RAW 视频解码器
pub struct RawVideoDecoder {
    /// 图像宽度
    width: u32,
    /// 图像高度
    height: u32,
    /// 像素格式
    pixel_format: PixelFormat,
    /// 预计算: 每帧总字节数
    frame_size: usize,
    /// 预计算: 各平面每行字节数
    linesizes: Vec<usize>,
    /// 预计算: 各平面的行数
    plane_heights: Vec<usize>,
    /// 已解码帧缓冲
    output_frame: Option<Frame>,
    /// 是否已打开 (配置参数)
    opened: bool,
    /// 是否已收到刷新信号 (空包)
    flushing: bool,
}

impl RawVideoDecoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::None,
            frame_size: 0,
            linesizes: Vec::new(),
            plane_heights: Vec::new(),
            output_frame: None,
            opened: false,
            flushing: false,
        }))
    }
}

impl Decoder for RawVideoDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::RawVideo
    }

    fn name(&self) -> &str {
        "rawvideo"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let video = match &params.params {
            CodecParamsType::Video(v) => v,
            _ => {
                return Err(TaoError::InvalidArgument(
                    "rawvideo 解码器需要视频参数".into(),
                ));
            }
        };

        if video.width == 0 || video.height == 0 {
            return Err(TaoError::InvalidArgument("宽度和高度不能为 0".into()));
        }
        if video.pixel_format == PixelFormat::None {
            return Err(TaoError::InvalidArgument("像素格式不能为 None".into()));
        }

        let pf = video.pixel_format;
        let frame_size = pf
            .frame_size(video.width, video.height)
            .ok_or_else(|| TaoError::InvalidArgument(format!("无法计算 {} 的帧大小", pf)))?;

        let plane_count = pf.plane_count() as usize;
        let mut linesizes = Vec::with_capacity(plane_count);
        let mut plane_heights = Vec::with_capacity(plane_count);
        for i in 0..plane_count {
            let ls = pf.plane_linesize(i, video.width).ok_or_else(|| {
                TaoError::InvalidArgument(format!("无法计算平面 {} 的 linesize", i))
            })?;
            let ph = pf
                .plane_height(i, video.height)
                .ok_or_else(|| TaoError::InvalidArgument(format!("无法计算平面 {} 的高度", i)))?;
            linesizes.push(ls);
            plane_heights.push(ph);
        }

        self.width = video.width;
        self.height = video.height;
        self.pixel_format = pf;
        self.frame_size = frame_size;
        self.linesizes = linesizes;
        self.plane_heights = plane_heights;
        self.output_frame = None;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 rawvideo 解码器: {}x{}, 格式={}, 帧大小={}",
            self.width, self.height, self.pixel_format, self.frame_size,
        );
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开, 请先调用 open()".into()));
        }
        if self.output_frame.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        // 空包 = flush
        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }

        if packet.data.len() != self.frame_size {
            return Err(TaoError::InvalidData(format!(
                "数据大小 {} 与预期帧大小 {} 不匹配",
                packet.data.len(),
                self.frame_size,
            )));
        }

        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;
        frame.is_keyframe = true;
        frame.picture_type = PictureType::I;

        // 按平面拆分数据
        let mut offset = 0usize;
        for i in 0..self.linesizes.len() {
            let plane_size = self.linesizes[i] * self.plane_heights[i];
            frame.data[i] = packet.data[offset..offset + plane_size].to_vec();
            frame.linesize[i] = self.linesizes[i];
            offset += plane_size;
        }

        self.output_frame = Some(Frame::Video(frame));
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_frame.take() {
            return Ok(frame);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.flushing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::VideoCodecParams;
    use bytes::Bytes;
    use tao_core::Rational;

    fn make_video_params(w: u32, h: u32, pf: PixelFormat) -> CodecParameters {
        CodecParameters {
            codec_id: CodecId::RawVideo,
            extra_data: Vec::new(),
            bit_rate: 0,
            params: CodecParamsType::Video(VideoCodecParams {
                width: w,
                height: h,
                pixel_format: pf,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        }
    }

    #[test]
    fn test_basic_decode_rgb24() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(2, 2, PixelFormat::Rgb24))
            .unwrap();

        // 2x2 RGB24 = 12 字节
        let data: Vec<u8> = (0..12).collect();
        let mut pkt = Packet::from_data(Bytes::from(data.clone()));
        pkt.pts = 100;
        pkt.time_base = Rational::new(1, 25);

        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Video(vf) => {
                assert_eq!(vf.width, 2);
                assert_eq!(vf.height, 2);
                assert_eq!(vf.pixel_format, PixelFormat::Rgb24);
                assert_eq!(vf.data[0], data);
                assert_eq!(vf.linesize[0], 6); // 2*3
                assert!(vf.is_keyframe);
                assert_eq!(vf.picture_type, PictureType::I);
                assert_eq!(vf.pts, 100);
            }
            _ => panic!("期望视频帧"),
        }
    }

    #[test]
    fn test_basic_decode_yuv420p() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(4, 4, PixelFormat::Yuv420p))
            .unwrap();

        // 4x4 YUV420P: Y=16, U=4, V=4 = 24 字节
        let data: Vec<u8> = (0..24).collect();
        let pkt = Packet::from_data(Bytes::from(data));
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();
        match frame {
            Frame::Video(vf) => {
                assert_eq!(vf.data.len(), 3);
                assert_eq!(vf.data[0].len(), 16); // Y: 4*4
                assert_eq!(vf.data[1].len(), 4); // U: 2*2
                assert_eq!(vf.data[2].len(), 4); // V: 2*2
                assert_eq!(vf.linesize[0], 4);
                assert_eq!(vf.linesize[1], 2);
                assert_eq!(vf.linesize[2], 2);
            }
            _ => panic!("期望视频帧"),
        }
    }

    #[test]
    fn test_not_open_error() {
        let mut dec = RawVideoDecoder::create().unwrap();
        let pkt = Packet::from_data(Bytes::from(vec![0u8; 12]));
        let err = dec.send_packet(&pkt).unwrap_err();
        assert!(matches!(err, TaoError::Codec(_)));
    }

    #[test]
    fn test_data_size_mismatch() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(2, 2, PixelFormat::Rgb24))
            .unwrap();
        let pkt = Packet::from_data(Bytes::from(vec![0u8; 10])); // 期望 12
        let err = dec.send_packet(&pkt).unwrap_err();
        assert!(matches!(err, TaoError::InvalidData(_)));
    }

    #[test]
    fn test_flush_and_eof() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(2, 2, PixelFormat::Rgb24))
            .unwrap();

        // 先发正常数据
        let pkt = Packet::from_data(Bytes::from(vec![0u8; 12]));
        dec.send_packet(&pkt).unwrap();
        dec.receive_frame().unwrap();

        // 发送空包 flush
        dec.send_packet(&Packet::empty()).unwrap();
        let err = dec.receive_frame().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_multi_frame_consecutive_decode() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(2, 2, PixelFormat::Gray8))
            .unwrap();

        for i in 0..5u8 {
            let data = vec![i; 4]; // 2x2 Gray8
            let mut pkt = Packet::from_data(Bytes::from(data.clone()));
            pkt.pts = i as i64;
            dec.send_packet(&pkt).unwrap();
            let frame = dec.receive_frame().unwrap();
            match frame {
                Frame::Video(vf) => {
                    assert_eq!(vf.data[0], data);
                    assert_eq!(vf.pts, i as i64);
                }
                _ => panic!("期望视频帧"),
            }
        }
    }

    #[test]
    fn test_receive_before_send() {
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&make_video_params(2, 2, PixelFormat::Rgb24))
            .unwrap();
        let err = dec.receive_frame().unwrap_err();
        assert!(matches!(err, TaoError::NeedMoreData));
    }
}
