//! RAW 视频编码器.
//!
//! 将 VideoFrame 的各平面数据拼接为 Packet.
//! 不做任何压缩, 直接透传像素数据.

use bytes::Bytes;
use log::debug;
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::encoder::Encoder;
use crate::frame::Frame;
use crate::packet::Packet;

/// RAW 视频编码器
pub struct RawVideoEncoder {
    /// 图像宽度
    width: u32,
    /// 图像高度
    height: u32,
    /// 像素格式
    pixel_format: PixelFormat,
    /// 预计算: 每帧总字节数
    frame_size: usize,
    /// 输出数据包缓冲
    output_packet: Option<Packet>,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
}

impl RawVideoEncoder {
    pub fn create() -> TaoResult<Box<dyn Encoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::None,
            frame_size: 0,
            output_packet: None,
            opened: false,
            flushing: false,
        }))
    }
}

impl Encoder for RawVideoEncoder {
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
                    "rawvideo 编码器需要视频参数".into(),
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

        self.width = video.width;
        self.height = video.height;
        self.pixel_format = pf;
        self.frame_size = frame_size;
        self.output_packet = None;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 rawvideo 编码器: {}x{}, 格式={}, 帧大小={}",
            self.width, self.height, self.pixel_format, self.frame_size,
        );
        Ok(())
    }

    fn send_frame(&mut self, frame: Option<&Frame>) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("编码器未打开, 请先调用 open()".into()));
        }
        if self.output_packet.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        let frame = match frame {
            Some(f) => f,
            None => {
                self.flushing = true;
                return Ok(());
            }
        };

        let video = match frame {
            Frame::Video(v) => v,
            Frame::Audio(_) => {
                return Err(TaoError::InvalidArgument(
                    "rawvideo 编码器不接受音频帧".into(),
                ));
            }
        };

        // 拼接所有平面数据
        let mut buf = Vec::with_capacity(self.frame_size);
        for plane_data in &video.data {
            buf.extend_from_slice(plane_data);
        }

        if buf.len() != self.frame_size {
            return Err(TaoError::InvalidData(format!(
                "帧数据大小 {} 与预期 {} 不匹配",
                buf.len(),
                self.frame_size,
            )));
        }

        let mut pkt = Packet::from_data(Bytes::from(buf));
        pkt.pts = video.pts;
        pkt.dts = video.pts; // RAW 视频无 B 帧, DTS = PTS
        pkt.duration = video.duration;
        pkt.time_base = video.time_base;
        pkt.is_keyframe = true;

        self.output_packet = Some(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> TaoResult<Packet> {
        if let Some(pkt) = self.output_packet.take() {
            return Ok(pkt);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_packet = None;
        self.flushing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::VideoCodecParams;
    use crate::frame::VideoFrame;
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
    fn test_basic_encode_rgb24() {
        let mut enc = RawVideoEncoder::create().unwrap();
        enc.open(&make_video_params(2, 2, PixelFormat::Rgb24))
            .unwrap();

        let data: Vec<u8> = (0..12).collect();
        let mut vf = VideoFrame::new(2, 2, PixelFormat::Rgb24);
        vf.data[0] = data.clone();
        vf.linesize[0] = 6;
        vf.pts = 42;
        vf.time_base = Rational::new(1, 25);

        enc.send_frame(Some(&Frame::Video(vf))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(&pkt.data[..], &data[..]);
        assert_eq!(pkt.pts, 42);
        assert_eq!(pkt.dts, 42);
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn test_not_open_error() {
        let mut enc = RawVideoEncoder::create().unwrap();
        let vf = VideoFrame::new(2, 2, PixelFormat::Rgb24);
        let err = enc.send_frame(Some(&Frame::Video(vf))).unwrap_err();
        assert!(matches!(err, TaoError::Codec(_)));
    }

    #[test]
    fn test_flush_and_eof() {
        let mut enc = RawVideoEncoder::create().unwrap();
        enc.open(&make_video_params(2, 2, PixelFormat::Gray8))
            .unwrap();

        enc.send_frame(None).unwrap();
        let err = enc.receive_packet().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_codec_roundtrip_rgb24() {
        use crate::decoders::rawvideo::RawVideoDecoder;

        let params = make_video_params(4, 2, PixelFormat::Rgb24);

        // 编码
        let mut enc = RawVideoEncoder::create().unwrap();
        enc.open(&params).unwrap();

        let original_data: Vec<u8> = (0..24).collect(); // 4*2*3 = 24
        let mut vf = VideoFrame::new(4, 2, PixelFormat::Rgb24);
        vf.data[0] = original_data.clone();
        vf.linesize[0] = 12;
        vf.pts = 10;
        vf.time_base = Rational::new(1, 25);

        enc.send_frame(Some(&Frame::Video(vf))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        // 解码
        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Video(decoded) => {
                assert_eq!(decoded.data[0], original_data);
                assert_eq!(decoded.pts, 10);
            }
            _ => panic!("期望视频帧"),
        }
    }

    #[test]
    fn test_codec_roundtrip_yuv420p() {
        use crate::decoders::rawvideo::RawVideoDecoder;

        let params = make_video_params(4, 4, PixelFormat::Yuv420p);

        let mut enc = RawVideoEncoder::create().unwrap();
        enc.open(&params).unwrap();

        let mut vf = VideoFrame::new(4, 4, PixelFormat::Yuv420p);
        // Y: 4*4=16, U: 2*2=4, V: 2*2=4
        vf.data[0] = vec![10u8; 16];
        vf.data[1] = vec![20u8; 4];
        vf.data[2] = vec![30u8; 4];
        vf.linesize = vec![4, 2, 2];

        enc.send_frame(Some(&Frame::Video(vf.clone()))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        let mut dec = RawVideoDecoder::create().unwrap();
        dec.open(&params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Video(decoded) => {
                assert_eq!(decoded.data[0], vec![10u8; 16]);
                assert_eq!(decoded.data[1], vec![20u8; 4]);
                assert_eq!(decoded.data[2], vec![30u8; 4]);
            }
            _ => panic!("期望视频帧"),
        }
    }
}
