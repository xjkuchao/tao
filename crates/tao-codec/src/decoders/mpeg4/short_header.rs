//! MPEG-4 Short Video Header (H.263 baseline) 解码支持
//!
//! 实现 MPEG-4 Part 2 Annex K 中定义的 Short Video Header 模式.
//! 该模式与 ITU-T H.263 baseline 兼容, 使用简化的宏块语法:
//! - 固定分辨率 (SQCIF/QCIF/CIF/4CIF/16CIF 或自定义)
//! - 无 AC/DC 预测
//! - 固定 DC 量化 (除以 8)
//! - 仅支持 1MV 模式 (无 Inter4V)
//! - 无 quarter-pixel
//! - GOB (Group of Blocks) 结构

use log::debug;
use tao_core::{TaoError, TaoResult};

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::block::decode_inter_block_vlc;
use super::tables::{DQUANT_TABLE, ZIGZAG_SCAN};
use super::types::{MacroblockInfo, MbType, MotionVector};
use super::vlc::{decode_cbpy, decode_mcbpc_i, decode_mcbpc_p};
use crate::frame::{PictureType, VideoFrame};

/// Short Video Header 的起始码 (picture_start_code)
///
/// 22 位: 0000 0000 0000 0000 10 0000 (0x000020 的前 22 位)
/// 实际按字节存储: 00 00 80-83 (后 2 位为 temporal_reference 高位)
const SHORT_VIDEO_START_CODE_LEN: u8 = 22;

/// H.263 标准分辨率表
const H263_FORMATS: [(u32, u32); 6] = [
    (0, 0),       // 禁止
    (128, 96),    // Sub-QCIF
    (176, 144),   // QCIF
    (352, 288),   // CIF
    (704, 576),   // 4CIF
    (1408, 1152), // 16CIF
];

/// Short Video Header 图片头信息
#[derive(Debug)]
pub(super) struct ShortVideoHeader {
    /// 时间参考 (8 位)
    pub temporal_reference: u8,
    /// 图片类型: 0=I, 1=P
    pub picture_type: PictureType,
    /// 量化参数 (PQUANT, 1-31)
    pub quant: u8,
    /// 宽度
    pub width: u32,
    /// 高度
    pub height: u32,
}

impl Mpeg4Decoder {
    /// 检测是否为 Short Video Header 格式
    ///
    /// Short Video Header 起始码: 22 位 = 0000 0000 0000 0000 10 0000
    /// 对应字节: 00 00 8x (其中 x 的高 2 位为 00)
    pub(super) fn is_short_video_header(data: &[u8]) -> bool {
        if data.len() < 3 {
            return false;
        }

        // 方法 1: 检查典型的 short video header 字节模式
        // 00 00 8x 其中高位 6 位是 picture_start_code
        if data[0] == 0x00 && data[1] == 0x00 && (data[2] & 0xFC) == 0x80 {
            return true;
        }

        // 方法 2: 扫描数据寻找 short video header 起始码
        for i in 0..data.len().saturating_sub(3) {
            if data[i] == 0x00 && data[i + 1] == 0x00 && (data[i + 2] & 0xFC) == 0x80 {
                return true;
            }
        }

        false
    }

    /// 查找 Short Video Header 起始码的位置
    ///
    /// 返回起始码之后的字节偏移 (22 位之后)
    fn find_short_header_offset(data: &[u8]) -> Option<usize> {
        (0..data.len().saturating_sub(3))
            .find(|&i| data[i] == 0x00 && data[i + 1] == 0x00 && (data[i + 2] & 0xFC) == 0x80)
    }

    /// 解析 Short Video Header (H.263 picture header)
    ///
    /// 语法 (按位解析):
    /// - picture_start_code: 22 位 (000000000000000010 0000)
    /// - temporal_reference: 8 位
    /// - marker_bit: 1 位 (= 1)
    /// - zero_bit: 1 位 (= 0)
    /// - split_screen_indicator: 1 位
    /// - document_camera_indicator: 1 位
    /// - full_picture_freeze_release: 1 位
    /// - source_format: 3 位 (分辨率)
    /// - picture_coding_type: 1 位 (0=I, 1=P)
    /// - four_reserved_zero_bits: 4 位
    /// - vop_quant (PQUANT): 5 位
    /// - zero_bit: 1 位
    /// - pei: 1 位 (扩展信息标志, 循环)
    pub(super) fn parse_short_video_header(&mut self, data: &[u8]) -> TaoResult<ShortVideoHeader> {
        let start = Self::find_short_header_offset(data)
            .ok_or_else(|| TaoError::InvalidData("未找到 Short Video Header 起始码".into()))?;

        let mut reader = BitReader::new(&data[start..]);

        // picture_start_code (22 位)
        let psc = reader
            .read_bits(SHORT_VIDEO_START_CODE_LEN)
            .ok_or_else(|| TaoError::InvalidData("Short Video Header 数据不足".into()))?;

        // 高 17 位应为 0, 然后 1, 然后低 4 位为 0
        // 即: 0000_0000_0000_0000_10_0000 = 0x000020
        if psc >> 5 != 1 {
            return Err(TaoError::InvalidData(format!(
                "无效的 Short Video Header 起始码: 0x{:06X}",
                psc
            )));
        }

        // temporal_reference (8 位)
        let temporal_reference = reader
            .read_bits(8)
            .ok_or_else(|| TaoError::InvalidData("无法读取 temporal_reference".into()))?
            as u8;

        // marker_bit (1) + zero_bit (0)
        let _marker = reader.read_bit();
        let _zero = reader.read_bit();

        // split_screen_indicator, document_camera_indicator, full_picture_freeze_release
        let _split = reader.read_bit();
        let _doc_camera = reader.read_bit();
        let _freeze = reader.read_bit();

        // source_format (3 位)
        let source_format = reader
            .read_bits(3)
            .ok_or_else(|| TaoError::InvalidData("无法读取 source_format".into()))?
            as usize;

        let (width, height) = if source_format == 0 || source_format > 5 {
            // 禁止或保留值: 使用已有的宽高
            if self.width > 0 && self.height > 0 {
                (self.width, self.height)
            } else {
                return Err(TaoError::InvalidData(format!(
                    "无效的 H.263 source_format: {}",
                    source_format
                )));
            }
        } else {
            H263_FORMATS[source_format]
        };

        // picture_coding_type (1 位): 0=Intra(I), 1=Inter(P)
        let pct = reader
            .read_bit()
            .ok_or_else(|| TaoError::InvalidData("无法读取 picture_coding_type".into()))?;
        let picture_type = if pct { PictureType::P } else { PictureType::I };

        // four_reserved_zero_bits (4 位)
        let _reserved = reader.read_bits(4);

        // vop_quant / PQUANT (5 位)
        let quant = reader
            .read_bits(5)
            .ok_or_else(|| TaoError::InvalidData("无法读取 PQUANT".into()))?
            as u8;

        // zero_bit (1 位)
        let _zero2 = reader.read_bit();

        // PEI (Supplemental Enhancement Information)
        // 循环读取: 如果 pei=1, 则读 8 位 PSUPP, 再读 pei...
        while reader.read_bit() == Some(true) {
            reader.read_bits(8); // PSUPP
        }

        debug!(
            "Short Video Header: {}x{}, type={:?}, quant={}, tr={}",
            width, height, picture_type, quant, temporal_reference
        );

        Ok(ShortVideoHeader {
            temporal_reference,
            picture_type,
            quant,
            width,
            height,
        })
    }

    /// 使用 Short Video Header 模式解码帧
    ///
    /// H.263 baseline 解码流程:
    /// 1. 解析 picture header (已在调用前完成)
    /// 2. 按 GOB 结构解码宏块
    /// 3. 宏块使用简化语法: COD + MCBPC + CBPY + DQUANT + MV + Blocks
    pub(super) fn decode_short_header_frame(
        &mut self,
        data: &[u8],
        header: &ShortVideoHeader,
    ) -> TaoResult<VideoFrame> {
        // 更新解码器尺寸 (Short Video Header 可能改变分辨率)
        if header.width != self.width || header.height != self.height {
            self.width = header.width;
            self.height = header.height;
            self.mb_stride = (header.width as usize).div_ceil(16);
            let mb_count = self.mb_stride * (header.height as usize).div_ceil(16);
            self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
            self.ref_mv_cache = vec![[MotionVector::default(); 4]; mb_count];
            self.mb_info = vec![MacroblockInfo::default(); mb_count];
            self.predictor_cache = vec![[0i16; 15]; mb_count * 6];
        }

        self.quant = header.quant;
        self.init_frame_decode();

        let is_i_vop = header.picture_type == PictureType::I;
        let mut frame = self.create_blank_frame(header.picture_type);

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);

        // 找到 picture header 之后的数据位置
        let start = Self::find_short_header_offset(data)
            .ok_or_else(|| TaoError::InvalidData("未找到 Short Video Header 起始码".into()))?;

        let mut reader = BitReader::new(&data[start..]);

        // 跳过 picture header (重新解析以定位到数据部分)
        // picture_start_code (22) + temporal_reference (8) + marker (1) + zero (1)
        // + split (1) + doc_camera (1) + freeze (1) + source_format (3)
        // + pct (1) + reserved (4) + pquant (5) + zero (1) = 49 位
        reader.skip_bits(SHORT_VIDEO_START_CODE_LEN as u32 + 27);
        // 跳过 PEI 循环
        while reader.read_bit() == Some(true) {
            reader.read_bits(8);
        }

        debug!(
            "Short Video Header 解码: {}x{} ({} MB), type={:?}, quant={}",
            self.width,
            self.height,
            mb_w * mb_h,
            header.picture_type,
            self.quant
        );

        // 按 GOB 结构解码
        let gobs = mb_h; // 每个 GOB 包含一行宏块
        for gob_num in 0..gobs {
            // GOB header (除第一个 GOB 外, 每个 GOB 可能有 GOB header)
            if gob_num > 0 {
                // 检查 GOB 起始码: byte-aligned + GSTUF + GN (5位) + GFID (2位) + GQUANT (5位)
                // GOB 起始码: 0000 0000 0000 0000 1 (17 位)
                self.try_parse_gob_header(&mut reader);
            }

            let mb_y = gob_num as u32;
            for mb_x_idx in 0..mb_w {
                let mb_x = mb_x_idx as u32;
                self.decode_short_header_macroblock(&mut frame, mb_x, mb_y, &mut reader, is_i_vop);
            }
        }

        Ok(frame)
    }

    /// 尝试解析 GOB (Group of Blocks) 头部
    ///
    /// GOB 头部格式:
    /// - GSTUF: 填充位 (到字节边界)
    /// - GOB 起始码: 17 位 (0000 0000 0000 0000 1)
    /// - GN: 5 位 (GOB 编号)
    /// - GFID: 2 位 (帧标识)
    /// - GQUANT: 5 位 (量化参数)
    fn try_parse_gob_header(&mut self, reader: &mut BitReader) {
        let snapshot = reader.snapshot_position();

        // 字节对齐
        let stuff_bits = reader.bits_to_byte_align();
        if stuff_bits > 0 {
            if let Some(stuff) = reader.peek_bits(stuff_bits) {
                // stuffing 位应全为 0
                if stuff != 0 {
                    return;
                }
            }
        }

        // 检查 GOB 起始码 (17 位): stuffing + 16个0 + 1个1
        let total = stuff_bits as u32 + 17;
        if total > 32 {
            return;
        }

        if let Some(bits) = reader.peek_bits(total as u8) {
            if bits != 1 {
                // 不是 GOB 起始码
                return;
            }
        } else {
            return;
        }

        // 确认是 GOB 起始码, 消耗这些位
        reader.skip_bits(total);

        // GN (5 位): GOB 编号
        let _gn = reader.read_bits(5);

        // GFID (2 位): 帧标识
        let _gfid = reader.read_bits(2);

        // GQUANT (5 位): 量化参数
        if let Some(gquant) = reader.read_bits(5) {
            if gquant > 0 && gquant <= 31 {
                self.quant = gquant as u8;
                debug!("GOB header: quant={}", self.quant);
            }
        }

        // 如果不是有效的 GOB header, 恢复位置
        let _ = snapshot;
    }

    /// 解码 Short Video Header 模式下的宏块
    ///
    /// H.263 baseline 宏块语法:
    /// - COD (1 位, 仅 P 帧): 0=编码, 1=未编码(直接复制参考)
    /// - MCBPC (VLC): 宏块类型 + 色度 CBP
    /// - CBPY (VLC): 亮度 CBP
    /// - DQUANT (2 位, 仅 IntraQ/InterQ): 量化参数调整
    /// - MVD (VLC, 仅 Inter): 运动向量差值 (1MV 模式)
    /// - Blocks: 8x8 DCT 系数块
    fn decode_short_header_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        reader: &mut BitReader,
        is_i_vop: bool,
    ) {
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;

        // P-VOP: COD (1 位)
        if !is_i_vop {
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                self.copy_mb_from_ref(frame, mb_x, mb_y);
                if mb_idx < self.mv_cache.len() {
                    self.mv_cache[mb_idx] = [MotionVector::default(); 4];
                }
                if mb_idx < self.mb_info.len() {
                    self.mb_info[mb_idx] = MacroblockInfo {
                        mode: MacroblockInfo::MODE_NOT_CODED,
                        quant: self.quant,
                        mvs: [MotionVector::default(); 4],
                    };
                }
                return;
            }
        }

        // MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader).unwrap_or((MbType::Intra, 0))
        } else {
            decode_mcbpc_p(reader).unwrap_or((MbType::Inter, 0))
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // CBPY
        let cbpy = decode_cbpy(reader, is_intra).unwrap_or(0);

        // DQUANT
        if mb_type == MbType::IntraQ || mb_type == MbType::InterQ {
            if let Some(dq) = reader.read_bits(2) {
                let delta = DQUANT_TABLE[dq as usize];
                self.quant = ((self.quant as i32 + delta).clamp(1, 31)) as u8;
            }
        }

        // 运动向量 (仅 Inter 模式, 1MV)
        let mut mb_mvs = [MotionVector::default(); 4];
        if !is_intra {
            // H.263 短头模式: 使用与 MPEG-4 相同的 MVD VLC, 但 f_code 固定为 1
            if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                self.validate_vector(&mut mv, mb_x, mb_y);
                mb_mvs = [mv; 4];
            }
        }

        // 存储 MV
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_mvs;
        }

        // 更新宏块信息
        if mb_idx < self.mb_info.len() {
            let mode_code = match mb_type {
                MbType::Inter | MbType::InterQ => MacroblockInfo::MODE_INTER,
                MbType::Intra | MbType::IntraQ => MacroblockInfo::MODE_INTRA,
                MbType::Inter4V => MacroblockInfo::MODE_INTER4V,
            };
            self.mb_info[mb_idx] = MacroblockInfo {
                mode: mode_code,
                quant: self.quant,
                mvs: mb_mvs,
            };
        }

        // CBP 组合
        let cbp = (cbpy << 2) | cbpc;

        let width = self.width as usize;
        let height = self.height as usize;

        // 解码各 8x8 块 - Y 平面
        for block_idx in 0..4usize {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;
            let ac_coded = (cbp >> (5 - block_idx)) & 1 != 0;

            let mut block = if is_intra {
                // H.263 Intra: DC 使用固定量化 (除以 8), 无 AC/DC 预测
                self.decode_short_header_intra_block(reader, ac_coded)
            } else if ac_coded {
                decode_inter_block_vlc(reader, &ZIGZAG_SCAN).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            // 反量化
            if is_intra {
                self.dequant_intra_block_h263_short(&mut block);
            } else {
                self.dequant_inter_block_h263(&mut block);
            }

            // IDCT
            super::idct::idct_8x8(&mut block);

            // 写入 Y 平面
            let base_x = mb_x as usize * 16 + bx as usize * 8;
            let base_y = mb_y as usize * 16 + by as usize * 8;
            if !is_intra {
                // Inter: 预测 + 残差
                if let Some(ref_frame) = &self.reference_frame {
                    let mv = mb_mvs[0];
                    for y in 0..8usize {
                        for x in 0..8usize {
                            let px = (base_x + x).min(width - 1);
                            let py = (base_y + y).min(height - 1);
                            let ref_val = Self::half_pixel_mc(
                                ref_frame,
                                0,
                                (base_x + x) as i32,
                                (base_y + y) as i32,
                                mv.x as i32,
                                mv.y as i32,
                                self.rounding_control,
                            );
                            let val = (ref_val as i32 + block[y * 8 + x]).clamp(0, 255);
                            frame.data[0][py * width + px] = val as u8;
                        }
                    }
                }
            } else {
                // Intra: 直接写入
                for y in 0..8usize {
                    for x in 0..8usize {
                        let px = (base_x + x).min(width - 1);
                        let py = (base_y + y).min(height - 1);
                        frame.data[0][py * width + px] = block[y * 8 + x].clamp(0, 255) as u8;
                    }
                }
            }
        }

        // 解码色度块 (U, V)
        let uv_w = width / 2;
        let uv_h = height / 2;
        for plane in 0..2usize {
            let block_idx = 4 + plane;
            let ac_coded = (cbp >> (5 - block_idx)) & 1 != 0;

            let mut block = if is_intra {
                self.decode_short_header_intra_block(reader, ac_coded)
            } else if ac_coded {
                decode_inter_block_vlc(reader, &ZIGZAG_SCAN).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            // 反量化
            if is_intra {
                self.dequant_intra_block_h263_short(&mut block);
            } else {
                self.dequant_inter_block_h263(&mut block);
            }

            // IDCT
            super::idct::idct_8x8(&mut block);

            // 写入色度平面
            let plane_idx = plane + 1;
            let base_x = mb_x as usize * 8;
            let base_y = mb_y as usize * 8;
            if !is_intra {
                if let Some(ref_frame) = &self.reference_frame {
                    // 色度 MV = 亮度 MV / 2 (四舍五入)
                    let mv = mb_mvs[0];
                    let chroma_mv_x = if mv.x >= 0 {
                        (mv.x as i32 + 1) >> 1
                    } else {
                        -((-mv.x as i32 + 1) >> 1)
                    };
                    let chroma_mv_y = if mv.y >= 0 {
                        (mv.y as i32 + 1) >> 1
                    } else {
                        -((-mv.y as i32 + 1) >> 1)
                    };
                    for y in 0..8usize {
                        for x in 0..8usize {
                            let px = (base_x + x).min(uv_w - 1);
                            let py = (base_y + y).min(uv_h - 1);
                            let ref_val = Self::half_pixel_mc(
                                ref_frame,
                                plane_idx,
                                (base_x + x) as i32,
                                (base_y + y) as i32,
                                chroma_mv_x,
                                chroma_mv_y,
                                self.rounding_control,
                            );
                            let val = (ref_val as i32 + block[y * 8 + x]).clamp(0, 255);
                            frame.data[plane_idx][py * uv_w + px] = val as u8;
                        }
                    }
                }
            } else {
                for y in 0..8usize {
                    for x in 0..8usize {
                        let px = (base_x + x).min(uv_w - 1);
                        let py = (base_y + y).min(uv_h - 1);
                        frame.data[plane_idx][py * uv_w + px] =
                            block[y * 8 + x].clamp(0, 255) as u8;
                    }
                }
            }
        }
    }

    /// 解码 Short Video Header 模式的 Intra 块
    ///
    /// H.263 Intra DC 编码: DC 值 = 读取的 INTRADC 值 * 8
    /// 无 AC/DC 预测, INTRADC 使用 8 位固定长度编码
    fn decode_short_header_intra_block(&self, reader: &mut BitReader, ac_coded: bool) -> [i32; 64] {
        let mut block = [0i32; 64];

        // INTRADC (8 位固定长度)
        // 特殊值: 0 和 128 映射为 1024 (即 DC=128*8=1024)
        let raw_dc = reader.read_bits(8).unwrap_or(0) as i32;
        let dc = if raw_dc == 0 || raw_dc == 128 {
            1024
        } else if raw_dc == 255 {
            // 255 也是合法值
            255 * 8
        } else {
            raw_dc * 8
        };
        block[0] = dc;

        // AC 系数: 与 MPEG-4 Inter 块使用相同的 VLC 表
        if ac_coded {
            if let Some(inter_block) = decode_inter_block_vlc(reader, &ZIGZAG_SCAN) {
                // 将 AC 系数复制到 block[1..63]
                block[1..64].copy_from_slice(&inter_block[1..64]);
            }
        }

        block
    }

    /// H.263 Short Header Intra 块反量化
    ///
    /// DC: 已经在解码时乘以 8, 直接使用
    /// AC: 使用 H.263 反量化公式
    fn dequant_intra_block_h263_short(&self, block: &mut [i32; 64]) {
        // DC 保持不变 (已在编码阶段处理)
        // AC 使用 H.263 反量化
        let quant = self.quant as i32;
        let quant2 = quant * 2;
        let odd = if quant % 2 == 1 { quant } else { quant - 1 };

        for coeff in block[1..].iter_mut() {
            if *coeff == 0 {
                continue;
            }
            if *coeff > 0 {
                *coeff = quant2 * *coeff + odd;
            } else {
                *coeff = quant2 * *coeff - odd;
            }
            *coeff = (*coeff).clamp(-2048, 2047);
        }
    }

    /// H.263 Inter 块反量化
    fn dequant_inter_block_h263(&self, block: &mut [i32; 64]) {
        let quant = self.quant as i32;
        let quant2 = quant * 2;
        let odd = if quant % 2 == 1 { quant } else { quant - 1 };

        for coeff in block.iter_mut() {
            if *coeff == 0 {
                continue;
            }
            if *coeff > 0 {
                *coeff = quant2 * *coeff + odd;
            } else {
                *coeff = quant2 * *coeff - odd;
            }
            *coeff = (*coeff).clamp(-2048, 2047);
        }
    }

    /// 半像素运动补偿 (用于 Short Video Header 模式)
    ///
    /// 使用双线性插值, 含 rounding control
    fn half_pixel_mc(
        ref_frame: &VideoFrame,
        plane: usize,
        block_x: i32,
        block_y: i32,
        mv_x: i32,
        mv_y: i32,
        rounding: u8,
    ) -> u8 {
        let (w, h) = if plane == 0 {
            (ref_frame.width as i32, ref_frame.height as i32)
        } else {
            (ref_frame.width as i32 / 2, ref_frame.height as i32 / 2)
        };

        let stride = w;

        // 半像素: MV 值以半像素为单位
        let full_x = block_x + (mv_x >> 1);
        let full_y = block_y + (mv_y >> 1);
        let frac_x = (mv_x & 1) != 0;
        let frac_y = (mv_y & 1) != 0;

        let get_pixel = |x: i32, y: i32| -> u8 {
            let cx = x.clamp(0, w - 1);
            let cy = y.clamp(0, h - 1);
            ref_frame.data[plane][(cy * stride + cx) as usize]
        };

        let round = 1 - rounding as i32;

        if !frac_x && !frac_y {
            get_pixel(full_x, full_y)
        } else if frac_x && !frac_y {
            let a = get_pixel(full_x, full_y) as i32;
            let b = get_pixel(full_x + 1, full_y) as i32;
            ((a + b + round) >> 1) as u8
        } else if !frac_x && frac_y {
            let a = get_pixel(full_x, full_y) as i32;
            let c = get_pixel(full_x, full_y + 1) as i32;
            ((a + c + round) >> 1) as u8
        } else {
            let a = get_pixel(full_x, full_y) as i32;
            let b = get_pixel(full_x + 1, full_y) as i32;
            let c = get_pixel(full_x, full_y + 1) as i32;
            let d = get_pixel(full_x + 1, full_y + 1) as i32;
            ((a + b + c + d + 2 - rounding as i32) >> 2) as u8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao_core::PixelFormat;

    fn test_decoder(width: u32, height: u32) -> Mpeg4Decoder {
        let mb_stride = if width > 0 {
            (width as usize).div_ceil(16)
        } else {
            0
        };
        let mb_count = if width > 0 && height > 0 {
            mb_stride * (height as usize).div_ceil(16)
        } else {
            0
        };
        Mpeg4Decoder {
            width,
            height,
            pixel_format: PixelFormat::Yuv420p,
            opened: width > 0,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            dpb: Vec::new(),
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: super::super::tables::STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: super::super::tables::STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: vec![[MotionVector::default(); 4]; mb_count],
            ref_mv_cache: vec![[MotionVector::default(); 4]; mb_count],
            mb_info: vec![MacroblockInfo::default(); mb_count],
            mb_stride,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
            last_time_base: 0,
            time_base_acc: 0,
            last_non_b_time: 0,
            gmc_params: super::super::gmc::GmcParameters::default(),
            alternate_vertical_scan: false,
            packed_frames: std::collections::VecDeque::new(),
        }
    }

    #[test]
    fn test_is_short_video_header() {
        // 有效的 Short Video Header: 00 00 80 (起始码的前 3 字节)
        assert!(Mpeg4Decoder::is_short_video_header(&[0x00, 0x00, 0x80]));
        assert!(Mpeg4Decoder::is_short_video_header(&[
            0x00, 0x00, 0x80, 0x02
        ]));

        // 无效的数据
        assert!(!Mpeg4Decoder::is_short_video_header(&[0x00, 0x00, 0x01]));
        assert!(!Mpeg4Decoder::is_short_video_header(&[0x00, 0x00]));
        assert!(!Mpeg4Decoder::is_short_video_header(&[]));

        // MPEG-4 VOP 起始码 (不是 short header)
        assert!(!Mpeg4Decoder::is_short_video_header(&[
            0x00, 0x00, 0x01, 0xB6
        ]));
    }

    #[test]
    fn test_parse_short_video_header_i_frame() {
        // 构造一个最小的 I 帧 short video header
        // PSC (22 位): 0000 0000 0000 0000 10 0000 = 0x000020
        // TR (8 位): 0x05
        // marker (1): 1
        // zero (1): 0
        // split (1): 0
        // doc_camera (1): 0
        // freeze (1): 0
        // source_format (3): 010 = QCIF (176x144)
        // pct (1): 0 = I-frame
        // reserved (4): 0000
        // pquant (5): 01010 = 10
        // zero (1): 0
        // pei (1): 0
        //
        // Total: 22 + 8 + 1 + 1 + 1 + 1 + 1 + 3 + 1 + 4 + 5 + 1 + 1 = 50 位
        //
        // 按位: 0000 0000 0000 0000 1000 00 | 00000101 | 1 0 0 0 0 010 | 0 0000 01010 0 0
        // 字节:            00       00          80       0A          E0   02   A0  00
        // 让我重新仔细计算:
        // Bits 0-21: PSC = 0000_0000_0000_0000_10_0000
        // Bits 22-29: TR = 0000_0101
        // Bit 30: marker = 1
        // Bit 31: zero = 0
        // ---- byte boundary at bit 32 ----
        // Bit 32: split = 0
        // Bit 33: doc = 0
        // Bit 34: freeze = 0
        // Bits 35-37: format = 010 (QCIF)
        // Bit 38: pct = 0 (I)
        // Bit 39: reserved高位 = 0
        // ---- byte boundary at bit 40 ----
        // Bits 40-42: reserved(000)
        // Bits 43-47: pquant = 01010 (=10)
        // ---- byte boundary at bit 48 ----
        // Bit 48: zero = 0
        // Bit 49: pei = 0
        // Remaining: 000000

        // 精确按 MSB-first 构造:
        // byte[0] = 0x00 (PSC bits 0-7)
        // byte[1] = 0x00 (PSC bits 8-15)
        // byte[2] = 0x80 (PSC bits 16-21 = 100000, TR bits 0-1 = 00)
        // byte[3] = 0x16 (TR bits 2-7 = 000101, marker=1, zero=0)
        // byte[4] = 0x08 (split=0, doc=0, freeze=0, fmt=010, pct=0, res_high=0)
        // byte[5] = 0x0A (res_low=000, pquant=01010)
        // byte[6] = 0x00 (zero=0, pei=0, padding=000000)

        let data: Vec<u8> = vec![0x00, 0x00, 0x80, 0x16, 0x08, 0x0A, 0x00];

        let mut decoder = test_decoder(0, 0);
        let hdr = decoder.parse_short_video_header(&data);
        assert!(
            hdr.is_ok(),
            "应成功解析 Short Video Header: {:?}",
            hdr.err()
        );

        let hdr = hdr.unwrap();
        assert_eq!(hdr.picture_type, PictureType::I, "应为 I 帧");
        assert_eq!(hdr.width, 176, "QCIF 宽度应为 176");
        assert_eq!(hdr.height, 144, "QCIF 高度应为 144");
        assert_eq!(hdr.quant, 10, "量化参数应为 10");
        assert_eq!(hdr.temporal_reference, 5, "时间参考应为 5");
    }

    #[test]
    fn test_parse_short_video_header_p_frame() {
        // P 帧: pct = 1
        // byte[4] = split(0)+doc(0)+freeze(0)+fmt(010)+pct(1)+res(0) = 0x0A
        let data: Vec<u8> = vec![0x00, 0x00, 0x80, 0x16, 0x0A, 0x0A, 0x00];

        let mut decoder = test_decoder(0, 0);
        let hdr = decoder.parse_short_video_header(&data).unwrap();
        assert_eq!(hdr.picture_type, PictureType::P, "应为 P 帧");
    }

    #[test]
    fn test_parse_short_video_header_cif() {
        // source_format = 011 (CIF, 352x288)
        // byte[4] = split(0)+doc(0)+freeze(0)+fmt(011)+pct(0)+res(0) = 0x0C
        let data: Vec<u8> = vec![0x00, 0x00, 0x80, 0x16, 0x0C, 0x0A, 0x00];

        let mut decoder = test_decoder(0, 0);
        let hdr = decoder.parse_short_video_header(&data).unwrap();
        assert_eq!(hdr.width, 352, "CIF 宽度应为 352");
        assert_eq!(hdr.height, 288, "CIF 高度应为 288");
    }

    #[test]
    fn test_h263_dequant() {
        let mut decoder = test_decoder(16, 16);
        decoder.quant = 4;

        let mut block = [0i32; 64];
        block[1] = 3;
        block[2] = -2;

        decoder.dequant_inter_block_h263(&mut block);

        // quant=4, quant2=8, odd=3 (偶数quant: odd=quant-1=3)
        // coeff[1] = 3 > 0: 8*3+3 = 27
        assert_eq!(block[1], 27);
        // coeff[2] = -2 < 0: 8*(-2)-3 = -19
        assert_eq!(block[2], -19);
    }

    #[test]
    fn test_h263_intra_dc() {
        let decoder = test_decoder(16, 16);
        // 模拟 INTRADC 解码
        // raw_dc = 100 -> dc = 100 * 8 = 800
        let data: Vec<u8> = vec![100, 0x00]; // INTRADC=100, 后续无 AC (cbp=0)
        let mut reader = BitReader::new(&data);
        let block = decoder.decode_short_header_intra_block(&mut reader, false);
        assert_eq!(block[0], 800, "DC 应为 100 * 8 = 800");
    }

    #[test]
    fn test_h263_intra_dc_special_values() {
        let decoder = test_decoder(16, 16);

        // raw_dc = 0 -> dc = 1024
        let data: Vec<u8> = vec![0x00, 0x00];
        let mut reader = BitReader::new(&data);
        let block = decoder.decode_short_header_intra_block(&mut reader, false);
        assert_eq!(block[0], 1024, "DC=0 应映射为 1024");

        // raw_dc = 128 -> dc = 1024
        let data: Vec<u8> = vec![128, 0x00];
        let mut reader = BitReader::new(&data);
        let block = decoder.decode_short_header_intra_block(&mut reader, false);
        assert_eq!(block[0], 1024, "DC=128 应映射为 1024");
    }
}
