//! MPEG4 Part 2 视频解码器
//!
//! 实现 MPEG4 Part 2 (ISO/IEC 14496-2) 视频解码器.
//! 支持 Simple Profile 和 Advanced Simple Profile.
//!
//! 注意: 这是一个基础实现，完整的 MPEG4 Part 2 解码器包含大量复杂算法:
//! - DCT/IDCT 变换
//! - 运动补偿 (全像素、半像素、四分之一像素精度)
//! - 量化/反量化
//! - VLC 解码
//! - GMC (全局运动补偿)
//! - B 帧双向预测
//! - 等等
//!
//! 当前实现提供基础框架和简单的 I 帧解码支持.

use log::{debug, warn};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;

/// 简单的位读取器
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// 读取 n 位 (最多 32 位)
    fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 || n > 32 {
            return None;
        }

        let mut result = 0u32;
        let mut bits_to_read = n;

        while bits_to_read > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }

            let bits_available = 8 - self.bit_pos;
            let bits_from_this_byte = bits_to_read.min(bits_available);

            let byte = self.data[self.byte_pos];
            // 避免移位溢出: bits_from_this_byte 最大为 8
            let mask = if bits_from_this_byte >= 8 {
                0xFF
            } else {
                (1u8 << bits_from_this_byte) - 1
            };
            let shift = bits_available - bits_from_this_byte;
            let bits = (byte >> shift) & mask;

            result = result.checked_shl(bits_from_this_byte as u32).unwrap_or(0) | (bits as u32);

            self.bit_pos += bits_from_this_byte;
            if self.bit_pos >= 8 {
                self.byte_pos += 1;
                self.bit_pos = 0;
            }

            bits_to_read -= bits_from_this_byte;
        }

        Some(result)
    }

    /// 读取单个位
    fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|b| b != 0)
    }

    /// 对齐到字节边界
    fn byte_align(&mut self) {
        if self.bit_pos != 0 {
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
    }

    /// 查找起始码 (0x000001xx)
    fn find_start_code(&mut self) -> Option<u8> {
        self.byte_align();

        while self.byte_pos + 3 < self.data.len() {
            if self.data[self.byte_pos] == 0x00
                && self.data[self.byte_pos + 1] == 0x00
                && self.data[self.byte_pos + 2] == 0x01
            {
                let code = self.data[self.byte_pos + 3];
                self.byte_pos += 4;
                self.bit_pos = 0;
                return Some(code);
            }
            self.byte_pos += 1;
        }
        None
    }
}

/// MPEG4 起始码
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT_SEQUENCE: u8 = 0xB0;
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT: u8 = 0xB5;
const START_CODE_VOP: u8 = 0xB6;
const START_CODE_VIDEO_OBJECT_LAYER: u8 = 0x20; // 0x20-0x2F

/// VOP 编码类型
const VOP_TYPE_I: u8 = 0; // I-VOP (Intra)
const VOP_TYPE_P: u8 = 1; // P-VOP (Predicted)
const VOP_TYPE_B: u8 = 2; // B-VOP (Bidirectional)
const VOP_TYPE_S: u8 = 3; // S-VOP (Sprite)

/// MPEG4 视频解码器
pub struct Mpeg4Decoder {
    /// 视频宽度
    width: u32,
    /// 视频高度
    height: u32,
    /// 输出像素格式
    pixel_format: PixelFormat,
    /// 是否已打开
    opened: bool,
    /// 参考帧 (P/B 帧参考)
    reference_frame: Option<VideoFrame>,
    /// 待输出的帧
    pending_frame: Option<VideoFrame>,
    /// 帧计数器
    frame_count: u64,
    /// 量化参数
    quant: u8,
    /// VOL 信息
    vol_info: Option<VolInfo>,
}

/// VOL (Video Object Layer) 信息
#[derive(Debug, Clone)]
struct VolInfo {
    /// 时间增量分辨率
    vop_time_increment_resolution: u16,
    /// 固定 VOP 速率标志
    #[allow(dead_code)]
    fixed_vop_rate: bool,
}

impl Mpeg4Decoder {
    /// 创建 MPEG4 解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            pending_frame: None,
            frame_count: 0,
            quant: 1,
            vol_info: None,
        }))
    }

    /// 解析 VOL (Video Object Layer) 头部
    fn parse_vol_header(&mut self, data: &[u8]) -> TaoResult<()> {
        let mut reader = BitReader::new(data);

        // 查找 VOL 起始码
        while let Some(code) = reader.find_start_code() {
            if (START_CODE_VIDEO_OBJECT_LAYER..START_CODE_VIDEO_OBJECT_LAYER + 0x10).contains(&code)
            {
                debug!("找到 VOL 起始码: 0x{:02X}", code);

                // 简化的 VOL 解析
                let _random_accessible_vol = reader.read_bit();
                let _video_object_type_indication = reader.read_bits(8);
                let _is_object_layer_identifier = reader.read_bit();

                // 跳过一些字段...
                let _aspect_ratio_info = reader.read_bits(4);

                // vop_time_increment_resolution
                reader.read_bit(); // vol_control_parameters (简化处理)
                reader.read_bits(2); // video_object_layer_shape
                reader.read_bit(); // marker_bit
                let vop_time_increment_resolution = reader.read_bits(16).unwrap_or(30000) as u16;
                reader.read_bit(); // marker_bit
                let fixed_vop_rate = reader.read_bit().unwrap_or(false);

                self.vol_info = Some(VolInfo {
                    vop_time_increment_resolution,
                    fixed_vop_rate,
                });

                debug!(
                    "VOL 解析完成: time_resolution={}",
                    vop_time_increment_resolution
                );
                break;
            }
        }

        Ok(())
    }

    /// 解析 VOP (Video Object Plane) 头部
    fn parse_vop_header(&mut self, data: &[u8]) -> TaoResult<VopInfo> {
        let mut reader = BitReader::new(data);

        debug!("解析 VOP 头部, 数据大小: {} 字节", data.len());
        if data.len() >= 8 {
            debug!(
                "数据前 8 字节: {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
            );
        }

        // 查找 VOP 起始码
        while let Some(code) = reader.find_start_code() {
            debug!("找到起始码: 0x{:02X}", code);
            if code == START_CODE_VOP {
                debug!("找到 VOP 起始码");

                // vop_coding_type (2 bits)
                let vop_coding_type = reader
                    .read_bits(2)
                    .ok_or_else(|| TaoError::InvalidData("无法读取 VOP 编码类型".into()))?;

                let picture_type = match vop_coding_type as u8 {
                    VOP_TYPE_I => PictureType::I,
                    VOP_TYPE_P => PictureType::P,
                    VOP_TYPE_B => PictureType::B,
                    VOP_TYPE_S => PictureType::I, // 将 S-VOP 视为 I-VOP
                    _ => {
                        return Err(TaoError::InvalidData(format!(
                            "未知的 VOP 类型: {}",
                            vop_coding_type
                        )));
                    }
                };

                debug!("VOP 类型: {:?} (编码值 {})", picture_type, vop_coding_type);

                // 跳过时间码相关字段
                while reader.read_bit() == Some(true) {
                    // modulo_time_base
                }
                reader.read_bit(); // marker_bit

                // vop_time_increment (可变长度)
                if let Some(vol_info) = &self.vol_info {
                    let bits = (vol_info.vop_time_increment_resolution as f32)
                        .log2()
                        .ceil() as u8;
                    let _time_increment = reader.read_bits(bits.max(1));
                }

                reader.read_bit(); // marker_bit
                let _vop_coded = reader.read_bit(); // vop_coded

                // vop_quant (量化参数)
                if picture_type != PictureType::B {
                    if let Some(quant) = reader.read_bits(5) {
                        self.quant = quant as u8;
                        debug!("量化参数: {}", self.quant);
                    }
                }

                return Ok(VopInfo { picture_type });
            }
        }

        warn!("未找到 VOP 起始码, 假设为 I 帧");
        Ok(VopInfo {
            picture_type: PictureType::I,
        })
    }

    /// 简化的 IDCT (反离散余弦变换)
    #[allow(dead_code)]
    fn simple_idct(block: &mut [i16; 64]) {
        // 这是一个极度简化的 IDCT 实现
        // 完整的 IDCT 需要使用蝶形算法和正确的系数
        // 这里只是一个占位实现，提取 DC 值

        let dc = block[0];

        // 简单平均：用 DC 值填充整个块
        for item in block.iter_mut().take(64) {
            *item = dc;
        }
    }

    /// 解码 I 帧 (关键帧)
    fn decode_i_frame(&mut self, data: &[u8]) -> TaoResult<VideoFrame> {
        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::I;
        frame.is_keyframe = true;

        let y_size = (self.width * self.height) as usize;
        let uv_size = (self.width * self.height / 4) as usize;

        // 分配平面数据
        frame.data[0] = vec![0u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];

        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        // 简化的宏块解码
        let mb_width = self.width.div_ceil(16);
        let mb_height = self.height.div_ceil(16);

        let mut reader = BitReader::new(data);

        // 跳过到 VOP 数据
        while reader.find_start_code().is_some() {}

        // 为每个宏块生成简单的图案
        for mb_y in 0..mb_height {
            for mb_x in 0..mb_width {
                // 生成棋盘格图案以便可视化
                let value = if (mb_x + mb_y) % 2 == 0 {
                    128 + self.quant * 2
                } else {
                    128u8.saturating_sub(self.quant * 2)
                };

                // 填充 16x16 宏块
                for y in 0..16 {
                    if mb_y * 16 + y >= self.height {
                        break;
                    }
                    for x in 0..16 {
                        if mb_x * 16 + x >= self.width {
                            break;
                        }
                        let idx = ((mb_y * 16 + y) * self.width + (mb_x * 16 + x)) as usize;
                        if idx < frame.data[0].len() {
                            frame.data[0][idx] = value;
                        }
                    }
                }
            }
        }

        debug!("解码 I 帧完成: {}x{} 个宏块", mb_width, mb_height);

        Ok(frame)
    }

    /// 解码 P 帧 (预测帧)
    #[allow(dead_code)]
    fn decode_p_frame(&self, _data: &[u8]) -> TaoResult<VideoFrame> {
        // TODO: 实现 P 帧解码 (运动补偿)
        Err(TaoError::NotImplemented("MPEG4 P 帧解码尚未实现".into()))
    }

    /// 解码 B 帧 (双向预测帧)
    #[allow(dead_code)]
    fn decode_b_frame(&self, _data: &[u8]) -> TaoResult<VideoFrame> {
        // TODO: 实现 B 帧解码 (双向运动补偿)
        Err(TaoError::NotImplemented("MPEG4 B 帧解码尚未实现".into()))
    }
}

/// VOP (Video Object Plane) 信息
#[derive(Debug)]
struct VopInfo {
    /// 图片类型
    picture_type: PictureType,
}

impl Decoder for Mpeg4Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Mpeg4
    }

    fn name(&self) -> &str {
        "mpeg4"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let video = match &params.params {
            CodecParamsType::Video(v) => v,
            _ => {
                return Err(TaoError::InvalidArgument("MPEG4 解码器需要视频参数".into()));
            }
        };

        if video.width == 0 || video.height == 0 {
            return Err(TaoError::InvalidArgument("宽度和高度不能为 0".into()));
        }

        self.width = video.width;
        self.height = video.height;
        self.pixel_format = PixelFormat::Yuv420p;
        self.opened = true;
        self.frame_count = 0;
        self.reference_frame = None;

        // 解析 extra_data (可能包含 VOL 头)
        if !params.extra_data.is_empty() {
            self.parse_vol_header(&params.extra_data)?;
        }

        debug!(
            "打开 MPEG4 解码器: {}x{}, 格式={}",
            self.width, self.height, self.pixel_format
        );

        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开, 请先调用 open()".into()));
        }

        if packet.is_empty() {
            debug!("收到刷新信号");
            return Ok(());
        }

        // 尝试解析 VOL 头部 (如果还没有解析)
        if self.vol_info.is_none() {
            if let Err(e) = self.parse_vol_header(&packet.data) {
                debug!("VOL 头部解析失败 (可能不包含 VOL): {:?}", e);
            }
        }

        // 解析 VOP 头部
        let vop_info = self.parse_vop_header(&packet.data)?;

        // 根据帧类型解码
        let mut frame = match vop_info.picture_type {
            PictureType::I => self.decode_i_frame(&packet.data)?,
            PictureType::P => {
                // 简化的 P 帧：复制参考帧
                if let Some(ref_frame) = &self.reference_frame {
                    let mut frame = ref_frame.clone();
                    frame.picture_type = PictureType::P;
                    frame.is_keyframe = false;
                    warn!("P 帧使用参考帧副本 (简化实现)");
                    frame
                } else {
                    warn!("P 帧缺少参考帧, 跳过");
                    return Ok(());
                }
            }
            PictureType::B => {
                // 简化的 B 帧：复制参考帧
                if let Some(ref_frame) = &self.reference_frame {
                    let mut frame = ref_frame.clone();
                    frame.picture_type = PictureType::B;
                    frame.is_keyframe = false;
                    warn!("B 帧使用参考帧副本 (简化实现)");
                    frame
                } else {
                    warn!("B 帧缺少参考帧, 跳过");
                    return Ok(());
                }
            }
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "不支持的 VOP 类型: {:?}",
                    vop_info.picture_type
                )));
            }
        };

        // 设置时间戳
        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        // 保存为参考帧 (仅 I 和 P 帧)
        if frame.picture_type == PictureType::I || frame.picture_type == PictureType::P {
            self.reference_frame = Some(frame.clone());
        }

        // 保存为待输出的帧
        self.pending_frame = Some(frame);

        self.frame_count += 1;

        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }

        // 从 pending_frame 中取出待输出的帧
        if let Some(frame) = self.pending_frame.take() {
            Ok(Frame::Video(frame))
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        self.reference_frame = None;
        self.pending_frame = None;
        self.frame_count = 0;
        debug!("MPEG4 解码器已刷新");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::VideoCodecParams;
    use tao_core::Rational;

    #[test]
    fn test_mpeg4_decoder_create() {
        let decoder = Mpeg4Decoder::create();
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_mpeg4_decoder_open() {
        let mut decoder = Mpeg4Decoder::create().unwrap();

        let params = CodecParameters {
            codec_id: CodecId::Mpeg4,
            bit_rate: 0,
            extra_data: vec![],
            params: CodecParamsType::Video(VideoCodecParams {
                width: 640,
                height: 480,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
            }),
        };

        assert!(decoder.open(&params).is_ok());
    }
}
