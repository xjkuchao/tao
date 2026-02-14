//! H.265/HEVC 视频解码器.
//!
//! 当前实现: 基础帧生成.
//! - 从 hvcC 配置获取分辨率 (VPS/SPS 解析)
//! - 为每个 NAL 单元生成 YUV420P 帧
//! - IDR/CRA 帧: 生成中灰帧
//! - 其他帧: 复制上一参考帧
//!
//! # 限制
//! - 不进行实际的 CTU/CU 解码
//! - 不进行帧内/帧间预测
//! - 不进行 CABAC 熵解码

use log::debug;
use tao_core::color::{ColorRange, ColorSpace};
use tao_core::{PixelFormat, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;
use crate::parsers::h265::{HevcNalUnitType, parse_hvcc_config, split_hevc_hvcc};

/// HEVC 解码器
pub struct HevcDecoder {
    width: u32,
    height: u32,
    length_size: usize,
    opened: bool,
    output_frame: Option<Frame>,
    flushing: bool,
    frame_count: u64,
    /// 参考帧 YUV 平面
    ref_y: Vec<u8>,
    ref_u: Vec<u8>,
    ref_v: Vec<u8>,
}

impl HevcDecoder {
    /// 创建 HEVC 解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            length_size: 4,
            opened: false,
            output_frame: None,
            flushing: false,
            frame_count: 0,
            ref_y: Vec::new(),
            ref_u: Vec::new(),
            ref_v: Vec::new(),
        }))
    }

    /// 初始化参考帧缓冲区
    fn init_buffers(&mut self) {
        let y_size = (self.width as usize) * (self.height as usize);
        let c_size = y_size / 4;
        self.ref_y = vec![128u8; y_size];
        self.ref_u = vec![128u8; c_size];
        self.ref_v = vec![128u8; c_size];
    }

    /// 判断 NAL 类型是否为 IRAP (IDR/CRA/BLA)
    fn is_irap_nal(nal_type: &HevcNalUnitType) -> bool {
        matches!(
            nal_type,
            HevcNalUnitType::IdrWRadl
                | HevcNalUnitType::IdrNLp
                | HevcNalUnitType::Cra
                | HevcNalUnitType::BlaWLp
                | HevcNalUnitType::BlaWRadl
                | HevcNalUnitType::BlaNLp
        )
    }

    /// 构建输出帧
    fn build_output_frame(&self, pts: i64, is_irap: bool) -> Frame {
        let w = self.width as usize;
        let h = self.height as usize;
        let pic_type = if is_irap {
            PictureType::I
        } else {
            PictureType::P
        };
        Frame::Video(VideoFrame {
            data: vec![
                self.ref_y[..w * h].to_vec(),
                self.ref_u[..w * h / 4].to_vec(),
                self.ref_v[..w * h / 4].to_vec(),
            ],
            linesize: vec![w, w / 2, w / 2],
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Yuv420p,
            pts,
            time_base: Rational::new(1, 90000),
            duration: 1,
            is_keyframe: is_irap,
            picture_type: pic_type,
            sample_aspect_ratio: Rational::new(1, 1),
            color_space: ColorSpace::Unspecified,
            color_range: ColorRange::Unspecified,
        })
    }
}

impl Decoder for HevcDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::H265
    }

    fn name(&self) -> &str {
        "hevc"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        // 从流参数获取基本尺寸
        if let CodecParamsType::Video(ref v) = params.params {
            self.width = v.width;
            self.height = v.height;
        }

        // 从 hvcC 配置解析精确参数
        if !params.extra_data.is_empty() {
            match parse_hvcc_config(&params.extra_data) {
                Ok(config) => {
                    self.length_size = config.length_size as usize;
                    // 从 SPS 获取精确分辨率
                    if let Some(sps_data) = config.sps_list.first() {
                        if sps_data.len() > 2 {
                            match crate::parsers::h265::parse_hevc_sps(&sps_data[2..]) {
                                Ok(sps) => {
                                    self.width = sps.width;
                                    self.height = sps.height;
                                    debug!(
                                        "HEVC SPS: {}x{}, chroma={}, bit_depth={}",
                                        sps.width,
                                        sps.height,
                                        sps.chroma_format_idc,
                                        sps.bit_depth_luma
                                    );
                                }
                                Err(e) => debug!("HEVC: SPS 解析失败: {}", e),
                            }
                        }
                    }
                }
                Err(e) => debug!("HEVC: hvcC 解析失败: {}", e),
            }
        }

        if self.width == 0 || self.height == 0 {
            return Err(TaoError::InvalidData("HEVC: 无法确定视频分辨率".into()));
        }

        self.init_buffers();
        self.opened = true;
        debug!("HEVC 解码器已打开: {}x{}", self.width, self.height);
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::InvalidData("HEVC: 解码器未打开".into()));
        }
        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }

        // 从 hvcC 格式解析 NAL 单元
        let nals = split_hevc_hvcc(&packet.data, self.length_size);

        let mut is_irap = false;
        for unit in &nals {
            if Self::is_irap_nal(&unit.nal_type) {
                is_irap = true;
            }
        }

        // IRAP 帧: 重置为中灰 (完整 CTU 解码会在此处进行)
        if is_irap {
            self.ref_y.fill(128);
            self.ref_u.fill(128);
            self.ref_v.fill(128);
        }
        // 非 IRAP 帧: 保持上一参考帧 (相当于 P 帧复制)

        self.output_frame = Some(self.build_output_frame(packet.pts, is_irap));
        self.frame_count += 1;
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_frame.take() {
            Ok(frame)
        } else if self.flushing {
            Err(TaoError::Eof)
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.flushing = false;
        self.frame_count = 0;
    }
}
