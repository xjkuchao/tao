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
    WaitSetup,
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
                    self.phase = InitPhase::WaitSetup;
                    debug!("Theora 注释头解析完成, 等待设置头");
                }
                (0x03, InitPhase::WaitSetup) => {
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

    /// 辅助函数: 直接创建 TheoraDecoder 实例, 方便测试内部状态
    fn create_decoder() -> TheoraDecoder {
        TheoraDecoder {
            initialized: false,
            header: None,
            phase: InitPhase::WaitIdentification,
        }
    }

    #[test]
    fn test_theora_decoder_creation() {
        let decoder = TheoraDecoder::create().unwrap();
        assert_eq!(decoder.name(), "theora");
        assert_eq!(decoder.codec_id(), CodecId::Theora);
    }

    #[test]
    fn test_identification_header_parsing() {
        let mut decoder = create_decoder();

        // 构造 42 字节的 Theora 标识头
        let mut header = vec![0u8; 42];

        // 字节 0-6: 魔数 0x80 + "theora"
        header[0] = 0x80;
        header[1..7].copy_from_slice(b"theora");

        // 字节 7-9: 版本号 3.2.1
        header[7] = 3;
        header[8] = 2;
        header[9] = 1;

        // 字节 10-13: FMBW / FMBH (编码帧宏块宽高, 此处不使用)

        // 字节 14-17: 图像宽高 (按位拼接)
        // 解析逻辑:
        //   pic_width  = (data[14]<<12) | (data[15]<<4) | (data[16]>>4)
        //   pic_height = ((data[16] & 0x0F)<<8) | data[17]
        //   width = pic_width * 16, height = pic_height * 16
        // 设 pic_width=20 => width=320, pic_height=15 => height=240
        header[14] = 0x00; // pic_width 高 8 位 = 0
        header[15] = 0x01; // pic_width 中 8 位: (20 >> 4) = 1
        header[16] = 0x40; // pic_width 低 4 位 | pic_height 高 4 位: (4<<4)|0 = 0x40
        header[17] = 0x0F; // pic_height 低 8 位 = 15

        // 字节 22-25: 帧率分子 (大端 u32 = 25)
        header[22..26].copy_from_slice(&25u32.to_be_bytes());
        // 字节 26-29: 帧率分母 (大端 u32 = 1)
        header[26..30].copy_from_slice(&1u32.to_be_bytes());
        // 字节 30-33: 像素宽高比分子 (大端 u32 = 1)
        header[30..34].copy_from_slice(&1u32.to_be_bytes());
        // 字节 34-37: 像素宽高比分母 (大端 u32 = 1)
        header[34..38].copy_from_slice(&1u32.to_be_bytes());

        let result = decoder.parse_identification_header(&header);
        assert!(result.is_ok(), "解析标识头失败: {:?}", result);

        let header_info = decoder.header.as_ref().unwrap();
        assert_eq!(header_info.width, 320);
        assert_eq!(header_info.height, 240);
        assert_eq!(header_info._version, (3, 2, 1));
        assert_eq!(header_info._frame_rate, Rational::new(25, 1));
    }

    #[test]
    fn test_identification_header_too_short() {
        let mut decoder = create_decoder();
        let short_data = vec![0x80, b't', b'h', b'e', b'o', b'r', b'a'];
        let result = decoder.parse_identification_header(&short_data);
        assert!(result.is_err(), "数据不足时应返回错误");
    }

    #[test]
    fn test_identification_header_invalid_magic() {
        let mut decoder = create_decoder();
        let mut bad_header = vec![0x80, b'b', b'a', b'd', b'd', b'a', b't'];
        bad_header.extend_from_slice(&[0; 35]); // 填充到 42 字节
        let result = decoder.parse_identification_header(&bad_header);
        assert!(result.is_err(), "魔数错误时应返回错误");
    }

    #[test]
    fn test_unsupported_version() {
        let mut decoder = create_decoder();
        // 版本 4.0.0 不支持
        let mut header = vec![0x80, b't', b'h', b'e', b'o', b'r', b'a'];
        header.extend_from_slice(&[4, 0, 0]); // 版本 4.0.0
        header.extend_from_slice(&[0; 32]); // 填充到 42 字节
        let result = decoder.parse_identification_header(&header);
        assert!(result.is_err(), "不支持的版本应返回错误");
    }

    #[test]
    fn test_flush_resets_state() {
        let mut decoder = create_decoder();
        decoder.initialized = true;
        decoder.phase = InitPhase::Ready;
        decoder.header = Some(TheoraHeader {
            width: 320,
            height: 240,
            _frame_rate: Rational::new(25, 1),
            _pixel_aspect_ratio: Rational::new(1, 1),
            _version: (3, 2, 1),
        });

        decoder.flush();

        assert!(decoder.header.is_none());
        assert!(matches!(decoder.phase, InitPhase::WaitIdentification));
    }

    #[test]
    fn test_send_packet_without_init() {
        let mut decoder = create_decoder();
        let packet = Packet::from_data(vec![0x00, 0x01, 0x02]);
        let result = decoder.send_packet(&packet);
        assert!(result.is_err(), "未初始化时发送数据包应返回错误");
    }

    #[test]
    fn test_receive_frame_without_init() {
        let mut decoder = create_decoder();
        let result = decoder.receive_frame();
        assert!(result.is_err(), "未初始化时接收帧应返回错误");
    }

    #[test]
    fn test_empty_packet_accepted() {
        let mut decoder = create_decoder();
        decoder.initialized = true;
        let packet = Packet::empty();
        let result = decoder.send_packet(&packet);
        assert!(result.is_ok(), "空数据包应被接受");
    }
}
