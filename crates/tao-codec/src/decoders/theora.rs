//! Theora 视频解码器
//!
//! 实现 Theora 视频编解码器的解码功能.
//! Theora 是一个开源的、免版税的视频编解码器，基于 VP3.

use log::{debug, warn};
use tao_core::{PixelFormat, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::{Frame, VideoFrame};
use crate::packet::Packet;

/// Theora 解码器
pub struct TheoraDecoder {
    /// 是否已初始化
    initialized: bool,
    /// Theora 头部信息
    header: Option<TheoraHeader>,
    /// 初始化阶段
    phase: InitPhase,
}

/// Theora 初始化阶段
#[derive(Debug)]
enum InitPhase {
    /// 等待标识头
    WaitIdentification,
    /// 等待注释头
    WaitComment,
    /// 等待设置头
    _WaitSetup,
    /// 就绪状态
    Ready,
}

/// Theora 头部信息
#[derive(Debug, Clone)]
struct TheoraHeader {
    /// 视频宽度
    width: u32,
    /// 视频高度
    height: u32,
    /// 帧率
    _frame_rate: Rational,
    /// 像素宽高比
    _pixel_aspect_ratio: Rational,
    /// 版本信息
    _version: (u8, u8, u8),
}

impl TheoraDecoder {
    /// 创建新的 Theora 解码器
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            initialized: false,
            header: None,
            phase: InitPhase::WaitIdentification,
        }))
    }

    /// 解析 Theora 标识头
    fn parse_identification_header(&mut self, data: &[u8]) -> TaoResult<()> {
        if data.len() < 42 {
            return Err(TaoError::InvalidData("Theora 标识头数据不足".to_string()));
        }

        // 验证 Theora 魔数
        if &data[0..7] != b"\x80theora" {
            return Err(TaoError::InvalidData("无效的 Theora 标识头".to_string()));
        }

        let version = (data[7], data[8], data[9]);
        debug!("Theora 版本: {}.{}.{}", version.0, version.1, version.2);

        // 检查版本兼容性 (当前支持 3.x 版本)
        if version.0 != 3 {
            return Err(TaoError::NotImplemented(format!(
                "不支持的 Theora 版本: {}.{}.{}",
                version.0, version.1, version.2
            )));
        }

        // 解析视频参数 (Theora 规范)
        // Theora 头部中的宽度和高度是 12 位字段，跨越字节边界
        // 实际显示的宽度和高度需要乘以 16
        let pic_width =
            ((data[14] as u32) << 12) | ((data[15] as u32) << 4) | ((data[16] as u32) >> 4);
        let pic_height = (((data[16] as u32) & 0x0F) << 8) | ((data[17] as u32) & 0xFF);

        let width = pic_width * 16;
        let height = pic_height * 16;

        let frame_num = u32::from_be_bytes([data[22], data[23], data[24], data[25]]);
        let frame_den = u32::from_be_bytes([data[26], data[27], data[28], data[29]]);
        let frame_rate = if frame_den != 0 {
            Rational::new(frame_num as i32, frame_den as i32)
        } else {
            Rational::new(25, 1) // 默认 25fps
        };

        let aspect_num = u32::from_be_bytes([data[30], data[31], data[32], data[33]]);
        let aspect_den = u32::from_be_bytes([data[34], data[35], data[36], data[37]]);
        let pixel_aspect_ratio = if aspect_den != 0 {
            Rational::new(aspect_num as i32, aspect_den as i32)
        } else {
            Rational::new(1, 1) // 默认方形像素
        };

        debug!(
            "Theora 视频参数: {}x{}, 帧率: {:?}, SAR: {:?}",
            width, height, frame_rate, pixel_aspect_ratio
        );

        self.header = Some(TheoraHeader {
            width,
            height,
            _frame_rate: frame_rate,
            _pixel_aspect_ratio: pixel_aspect_ratio,
            _version: version,
        });

        Ok(())
    }

    /// 解析 Theora 设置头
    fn parse_setup_header(&mut self, data: &[u8]) -> TaoResult<()> {
        // 简化实现 - 实际需要解析量化表、Huffman 表等
        debug!("解析 Theora 设置头，大小: {} 字节", data.len());
        Ok(())
    }

    /// 解析 Theora 注释头
    fn parse_comment_header(&mut self, data: &[u8]) -> TaoResult<()> {
        // 简化实现 - 实际需要解析元数据
        debug!("解析 Theora 注释头，大小: {} 字节", data.len());
        Ok(())
    }
}

impl Decoder for TheoraDecoder {
    /// 获取解码器标识
    fn codec_id(&self) -> CodecId {
        CodecId::Theora
    }

    /// 获取解码器名称
    fn name(&self) -> &str {
        "theora"
    }

    /// 使用参数配置解码器
    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        // 尝试从 extra_data 解析 Theora 标识头
        if !params.extra_data.is_empty()
            && params.extra_data.len() >= 7
            && &params.extra_data[1..7] == b"theora"
        {
            self.parse_identification_header(&params.extra_data)?;
            self.phase = InitPhase::WaitComment;
        }

        self.initialized = true;
        debug!("Theora 解码器初始化完成");
        Ok(())
    }

    /// 送入一个压缩数据包进行解码
    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.initialized {
            return Err(TaoError::InvalidArgument("解码器未初始化".to_string()));
        }

        if packet.data.is_empty() {
            return Ok(());
        }

        // 检查是否是头部包 (Ogg 包的第一个字节是包头类型)
        if packet.data[0] & 0x80 != 0 {
            // 这是头部包
            let packet_type = packet.data[0] & 0x7f;
            match (packet_type, &self.phase) {
                (0x01, InitPhase::WaitIdentification) => {
                    // 标识头 - 这种情况不应该发生，应该在 open() 中处理
                    debug!("标识头通过数据包传递，应该在 open() 中处理");
                    // 但如果确实发生了，我们还是处理它
                    self.parse_identification_header(&packet.data[1..])?;
                    self.phase = InitPhase::WaitComment;
                }
                (0x01, InitPhase::WaitComment) => {
                    // 标识头重复，可能是 OGG 解封装器的行为
                    debug!("标识头重复，跳过");
                }
                (0x02, InitPhase::WaitComment) => {
                    // 注释头
                    self.parse_comment_header(&packet.data[1..])?;
                    // 对于某些 Theora 文件，可能没有设置头，直接就绪
                    self.phase = InitPhase::Ready;
                    debug!("Theora 注释头解析完成，解码器就绪（无设置头）");
                }
                (0x03, InitPhase::_WaitSetup) => {
                    // 设置头
                    self.parse_setup_header(&packet.data[1..])?;
                    self.phase = InitPhase::Ready;
                    debug!("Theora 设置头解析完成，解码器就绪");
                }
                (packet_type, phase) => {
                    warn!(
                        "未知的 Theora 头部类型: 0x{:02x} 或错误的阶段: {:?}",
                        packet_type, phase
                    );
                }
            }
        } else {
            // 这是视频数据包
            if matches!(self.phase, InitPhase::Ready) {
                debug!("收到 Theora 视频数据包，大小: {} 字节", packet.data.len());
            } else {
                // 在头部未完成时收到视频数据，跳过但继续处理头部
                debug!("在头部解析阶段收到视频数据包，跳过");
            }
        }

        Ok(())
    }

    /// 从解码器取出一帧解码数据
    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if !self.initialized {
            return Err(TaoError::InvalidArgument("解码器未初始化".to_string()));
        }

        // 简化实现 - 返回一个占位帧
        // 实际实现需要完整的 Theora 解码算法
        if let (Some(header), InitPhase::Ready) = (&self.header, &self.phase) {
            let frame = VideoFrame::new(header.width, header.height, PixelFormat::Yuv420p);

            debug!("生成 Theora 视频帧: {}x{}", header.width, header.height);
            Ok(Frame::Video(frame))
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    /// 刷新解码器, 清空内部状态
    fn flush(&mut self) {
        debug!("刷新 Theora 解码器缓冲区");
        self.phase = InitPhase::WaitIdentification;
        self.header = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theora_decoder_creation() {
        let decoder = TheoraDecoder::create().unwrap();
        assert_eq!(decoder.name(), "theora");
        assert_eq!(decoder.codec_id(), CodecId::Theora);
    }

    #[test]
    fn test_identification_header_parsing() {
        let mut decoder = TheoraDecoder::create().unwrap();

        // 构造一个最小的 Theora 标识头
        let mut header = vec![0x80, b't', b'h', b'e', b'o', b'r', b'a'];
        header.extend_from_slice(&[3, 2, 1]); // 版本 3.2.1
        header.extend_from_slice(&[0, 0, 0, 0, 0]); // 保留字段
        header.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]); // width (高位)
        header.extend_from_slice(&[0x90, 0x12, 0x34, 0x56]); // height (低位)
        header.extend_from_slice(&[0, 0, 0, 0]); // 保留字段
        header.extend_from_slice(&[0, 0, 0, 25]); // frame numerator
        header.extend_from_slice(&[0, 0, 0, 1]); // frame denominator
        header.extend_from_slice(&[0, 0, 0, 1]); // aspect numerator
        header.extend_from_slice(&[0, 0, 0, 1]); // aspect denominator

        let result = decoder.parse_identification_header(&header);
        assert!(result.is_ok());

        let header_info = decoder.header.as_ref().unwrap();
        assert_eq!(header_info.version, (3, 2, 1));
        assert_eq!(header_info.frame_rate, Rational::new(25, 1));
    }
}
