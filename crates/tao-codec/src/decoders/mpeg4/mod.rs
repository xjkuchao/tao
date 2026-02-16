//! MPEG-4 Part 2 视频解码器
//!
//! 实现 MPEG-4 Part 2 (ISO/IEC 14496-2) 视频解码器.
//! 支持 Simple Profile 和 Advanced Simple Profile.
//!
//! 已实现:
//! - I/P/B 帧解码 (B 帧支持 Direct/Forward/Backward/Interpolate 模式)
//! - VOP/VOL 头部解析 (含 complexity_estimation, resync_marker, data_partitioned)
//! - 宏块解码: Intra, Inter, InterQ, IntraQ, Inter4V
//! - 完整 VLC 解码 (Escape Mode 1/2/3)
//! - H.263 和 MPEG 两种反量化类型
//! - DC Scaler (按 MPEG-4 标准 Table 7-1)
//! - 运动补偿: 全像素, 半像素, 四分之一像素
//! - Chroma MV 推导 (含 rounding table)
//! - MV 范围包装 (基于 f_code)
//! - AC/DC 预测
//! - Alternate scan tables (vertical/horizontal)
//! - Mismatch control (MPEG 量化类型)
//! - 边缘扩展 (edge padding)
//! - GMC (全局运动补偿, S-VOP)
//! - Resync marker 检测与错误恢复
//! - 隔行扫描 (field_dct, field_pred)
//! - 整数 IDCT (定点 AAN 算法)
//!
//! ## 模块结构
//!
//! - `types`: 类型定义 (MbType, MotionVector, VolInfo, VopInfo 等)
//! - `tables`: 常量表 (量化矩阵, 扫描表, DC scaler, escape 模式表等)
//! - `bitreader`: 位流读取器与起始码查找
//! - `vlc`: VLC 表和解码函数
//! - `header`: VOL/VOP 头部解析
//! - `block`: 8x8 块级 DCT 系数解码
//! - `idct`: 整数 IDCT (Chen-Wang 算法)
//! - `motion`: 运动向量解码, 预测与运动补偿
//! - `dequant`: 反量化 (H.263/MPEG)

mod bframe;
mod bitreader;
mod block;
mod dequant;
mod gmc;
mod header;
mod idct;
mod motion;
mod short_header;
mod tables;
mod types;
mod vlc;

use log::{debug, warn};
use tao_core::{PixelFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;

use bitreader::{BitReader, find_start_code_offset};
use block::{decode_inter_block_vlc, decode_intra_block_vlc};
use gmc::GmcParameters;
use header::START_CODE_VOP;
use idct::idct_8x8;
use tables::*;
use types::*;
use vlc::{decode_cbpy, decode_mcbpc_i, decode_mcbpc_p};

// ============================================================================
// 数据分区信息结构体
// ============================================================================

/// 数据分区信息 (data_partitioned 时使用)
///
/// ISO/IEC 14496-2 中, 数据分区将编码数据分为三个部分:
/// - Partition A: MB 类型 (MCBPC/CBPY), DQUANT, 运动向量
/// - Partition B: DC 系数 (使用 RVLC 可逆编码)
/// - Partition C: AC 系数
///
/// 使用 resync markers 分隔各分区边界。每个分区起始处有 resync marker。
#[derive(Debug, Clone)]
struct DataPartitionInfo {
    /// Partition A 起始和结束位置 (比特偏移)
    partition_a: (usize, usize),
    /// Partition B 起始和结束位置 (比特偏移)
    partition_b: (usize, usize),
    /// Partition C 起始和结束位置 (比特偏移)
    partition_c: (usize, usize),
}

/// Data Partitioning 中间解码数据
///
/// 用于存储从各分区解码的中间数据，最后重建完整宏块。
#[derive(Debug, Clone)]
struct PartitionedMacroblockData {
    /// 宏块类型
    mb_type: MbType,
    /// CBP (coded block pattern): Y 平面 4 块 + U/V 平面各 1 块
    cbp: u8,
    /// 量化参数
    quant: u8,
    /// AC prediction flag (仅 Intra)
    ac_pred_flag: bool,
    /// 运动向量 (最多 4 个, Inter4V 模式)
    mvs: [MotionVector; 4],
    /// DC 系数 (6 个块: Y0, Y1, Y2, Y3, U, V)
    dc_coeffs: [i16; 6],
    /// AC 系数 (6 个块, 每块 63 个 AC 系数)
    ac_coeffs: [[i16; 63]; 6],
    /// field_dct 标志
    field_dct: bool,
    /// field_pred 标志
    field_pred: bool,
    /// 顶场参考选择
    field_for_top: bool,
    /// 底场参考选择
    field_for_bot: bool,
}

// ============================================================================
// Mpeg4Decoder 结构体
// ============================================================================

/// MPEG-4 Part 2 视频解码器
pub struct Mpeg4Decoder {
    pub(super) width: u32,
    pub(super) height: u32,
    pixel_format: PixelFormat,
    opened: bool,
    /// 较晚的参考帧 (P 帧的前向参考, B 帧的后向参考)
    pub(super) reference_frame: Option<VideoFrame>,
    /// 较早的参考帧 (B 帧的前向参考)
    pub(super) backward_reference: Option<VideoFrame>,
    pending_frame: Option<VideoFrame>,
    /// 解码图像缓冲区（用于 B 帧重排序）
    dpb: Vec<VideoFrame>,
    frame_count: u64,
    pub(super) quant: u8,
    pub(super) vol_info: Option<VolInfo>,
    pub(super) quant_matrix_intra: [u8; 64],
    pub(super) quant_matrix_inter: [u8; 64],
    /// 预测器缓存: [DC, row AC 1-7, col AC 1-7] per block
    pub(super) predictor_cache: Vec<[i16; 15]>,
    /// 运动向量缓存 (每个 MB 存储 4 个 MV)
    pub(super) mv_cache: Vec<[MotionVector; 4]>,
    /// 参考帧 MV 缓存 (用于 B 帧 Direct 模式从共定位 MB 获取 MV)
    pub(super) ref_mv_cache: Vec<[MotionVector; 4]>,
    /// 宏块信息缓存 (per-MB 模式/量化/MV)
    pub(super) mb_info: Vec<MacroblockInfo>,
    pub(super) mb_stride: usize,
    pub(super) f_code_forward: u8,
    pub(super) f_code_backward: u8,
    pub(super) rounding_control: u8,
    pub(super) intra_dc_vlc_thr: u32,
    /// TRD: 两个参考帧之间的时间距离
    pub(super) time_pp: i32,
    /// TRB: 后向参考到当前 B 帧的时间距离
    pub(super) time_bp: i32,
    /// 上一个非 B 帧的 time_base 累加值
    pub(super) last_time_base: i32,
    /// 当前 time_base 累加值
    pub(super) time_base_acc: i32,
    /// 上一个非 B 帧的绝对时间
    pub(super) last_non_b_time: i32,
    /// 当前 VOP 的 GMC 参数
    pub(super) gmc_params: GmcParameters,
    /// DivX packed bitstream 队列 (一个 packet 中拆分出的多个 VOP)
    packed_frames: std::collections::VecDeque<Vec<u8>>,
}

impl Mpeg4Decoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            dpb: Vec::new(),
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            ref_mv_cache: Vec::new(),
            mb_info: Vec::new(),
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
            last_time_base: 0,
            time_base_acc: 0,
            last_non_b_time: 0,
            gmc_params: GmcParameters::default(),
            packed_frames: std::collections::VecDeque::new(),
        }))
    }

    // ========================================================================
    // 辅助函数
    // ========================================================================

    /// 获取 DC scaler
    pub(super) fn get_dc_scaler(&self, is_luma: bool) -> u8 {
        let q = (self.quant as usize).min(31);
        if is_luma {
            DC_SCALER_Y[q]
        } else {
            DC_SCALER_C[q]
        }
    }

    /// 判断当前 VOP 是否使用 Intra DC VLC
    pub(super) fn use_intra_dc_vlc(&self) -> bool {
        let thr = INTRA_DC_THRESHOLD
            .get(self.intra_dc_vlc_thr as usize)
            .copied()
            .unwrap_or(0);
        (self.quant as u32) < thr
    }

    /// 获取预测器缓存索引
    pub(super) fn get_neighbor_block_idx(&self, x: isize, y: isize, idx: usize) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.mb_stride as isize {
            return None;
        }
        let mb_height = (self.height as usize).div_ceil(16);
        if y >= mb_height as isize {
            return None;
        }
        Some((y as usize * self.mb_stride + x as usize) * 6 + idx)
    }

    /// 选择场预测的参考场
    pub(super) fn select_field_for_block(
        block_idx: usize,
        field_for_top: bool,
        field_for_bot: bool,
    ) -> bool {
        if block_idx < 2 {
            field_for_top
        } else {
            field_for_bot
        }
    }

    /// 场预测时将垂直 MV 转换到帧坐标
    pub(super) fn scale_field_mv_y(mv_y: i16) -> i16 {
        let scaled = (mv_y as i32) * 2;
        scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }

    /// 场预测下色度行使用的参考场
    pub(super) fn select_field_for_chroma_line(
        chroma_y: usize,
        field_for_top: bool,
        field_for_bot: bool,
    ) -> bool {
        if chroma_y & 1 == 0 {
            field_for_top
        } else {
            field_for_bot
        }
    }

    /// 计算两个 MV 的平均值
    pub(super) fn average_mv(a: MotionVector, b: MotionVector) -> MotionVector {
        let sum_x = a.x as i32 + b.x as i32;
        let sum_y = a.y as i32 + b.y as i32;
        MotionVector {
            x: (sum_x >> 1).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            y: (sum_y >> 1).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        }
    }

    /// 获取 Intra DC 预测方向和预测值
    pub(super) fn get_intra_predictor(
        &self,
        mb_x: usize,
        mb_y: usize,
        block_idx: usize,
    ) -> (i16, PredictorDirection) {
        let get_dc = |x: isize, y: isize, idx: usize| -> i16 {
            self.get_neighbor_block_idx(x, y, idx)
                .and_then(|pos| self.predictor_cache.get(pos))
                .map(|b| b[0])
                .unwrap_or(1024)
        };

        let (dc_a, dc_b, dc_c) = match block_idx {
            0 => (
                get_dc(mb_x as isize - 1, mb_y as isize, 1),
                get_dc(mb_x as isize - 1, mb_y as isize - 1, 3),
                get_dc(mb_x as isize, mb_y as isize - 1, 2),
            ),
            1 => (
                get_dc(mb_x as isize, mb_y as isize, 0),
                get_dc(mb_x as isize, mb_y as isize - 1, 2),
                get_dc(mb_x as isize, mb_y as isize - 1, 3),
            ),
            2 => (
                get_dc(mb_x as isize - 1, mb_y as isize, 3),
                get_dc(mb_x as isize - 1, mb_y as isize, 1),
                get_dc(mb_x as isize, mb_y as isize, 0),
            ),
            3 => (
                get_dc(mb_x as isize, mb_y as isize, 2),
                get_dc(mb_x as isize, mb_y as isize, 0),
                get_dc(mb_x as isize, mb_y as isize, 1),
            ),
            4 | 5 => (
                get_dc(mb_x as isize - 1, mb_y as isize, block_idx),
                get_dc(mb_x as isize - 1, mb_y as isize - 1, block_idx),
                get_dc(mb_x as isize, mb_y as isize - 1, block_idx),
            ),
            _ => (1024, 1024, 1024),
        };

        let grad_hor = (dc_a - dc_b).abs();
        let grad_ver = (dc_c - dc_b).abs();

        if grad_hor < grad_ver {
            (dc_c, PredictorDirection::Vertical)
        } else {
            (dc_a, PredictorDirection::Horizontal)
        }
    }

    /// 扫描并分析数据分区边界 (改进版)
    ///
    /// 精确检测 resync markers 以确定各分区边界。
    /// 返回分区信息和分包数量估计。
    /// 分析数据分区边界
    ///
    /// 在 data_partitioned 模式下，一个 video packet 被分为三个分区：
    /// 1. Partition A: 宏块类型、量化参数、运动向量
    /// 2. Partition B: DC 系数 (使用 RVLC)
    /// 3. Partition C: AC 系数
    ///
    /// 每个分区起始处有 resync marker (对齐到字节边界后的模式：
    /// stuffing bits + (16 + fcode) 个零 + 1 个一)。
    ///
    /// # 参数
    /// - `data`: VOP 数据 (包含 VOP header + video packets)
    /// - `fcode`: VOP 的 fcode 值
    ///
    /// # 返回
    /// (DataPartitionInfo, partition_count) - 分区信息和分区数量
    fn analyze_data_partitions(&self, data: &[u8], fcode: u8) -> (DataPartitionInfo, u32) {
        let mut info = DataPartitionInfo {
            partition_a: (0, data.len() * 8),
            partition_b: (data.len() * 8, data.len() * 8),
            partition_c: (data.len() * 8, data.len() * 8),
        };
        let mut partition_count = 0u32;

        let total_mbs = self.mb_stride * (self.height as usize).div_ceil(16);
        let mb_bits = if total_mbs > 1 {
            (total_mbs as f32).log2().ceil() as usize
        } else {
            1
        };
        let packet_header_bits = mb_bits + 5 + 1;

        // resync marker 长度 = 16 + fcode 位
        let marker_len = 16 + fcode as usize;

        // 扫描 resync markers (无需创建 BitReader，直接按位扫描)
        let mut bit_pos = 0usize;
        let max_bits = data.len() * 8;

        while bit_pos + marker_len + 8 < max_bits {
            // 对齐到字节边界
            if bit_pos % 8 != 0 {
                bit_pos = bit_pos.div_ceil(8) * 8;
            }

            // 检查是否是 resync marker
            let byte_pos = bit_pos / 8;
            if byte_pos + marker_len.div_ceil(8) >= data.len() {
                break;
            }

            // 检查 resync marker 模式: marker_len 位中前 marker_len-1 位为 0, 最后 1 位为 1
            let mut is_marker = true;
            for i in 0..marker_len {
                let bit_offset = bit_pos + i;
                let byte_idx = bit_offset / 8;
                let bit_idx = 7 - (bit_offset % 8);

                if byte_idx >= data.len() {
                    is_marker = false;
                    break;
                }

                let bit_val = (data[byte_idx] >> bit_idx) & 1;
                if i < marker_len - 1 {
                    // 前 marker_len-1 位应为 0
                    if bit_val != 0 {
                        is_marker = false;
                        break;
                    }
                } else {
                    // 最后 1 位应为 1
                    if bit_val != 1 {
                        is_marker = false;
                        break;
                    }
                }
            }

            if is_marker {
                // 找到 resync marker
                partition_count += 1;
                let partition_start = bit_pos + marker_len;

                match partition_count {
                    1 => {
                        // 第一个 resync marker 标记 Partition A 的结束和 Partition B 的开始
                        info.partition_a.1 = bit_pos;
                        info.partition_b.0 = partition_start;
                    }
                    2 => {
                        // 第二个 resync marker 标记 Partition B 的结束和 Partition C 的开始
                        info.partition_b.1 = bit_pos;
                        info.partition_c.0 = partition_start;
                        info.partition_c.1 = max_bits;
                    }
                    _ => {
                        // 后续 resync markers 可能是下一个 video packet,停止分析
                        break;
                    }
                }

                // 跳过 marker、macroblock_number、quant_scale 与 HEC 标志位
                bit_pos = partition_start + packet_header_bits;
            } else {
                // 按字节步进
                bit_pos += 8;
            }
        }

        // 如果只找到部分分区边界，调整信息
        if partition_count == 0 {
            // 没有 resync marker，整个 video packet 都是 Partition A
            info.partition_a.1 = max_bits;
            info.partition_b.0 = max_bits;
            info.partition_b.1 = max_bits;
            info.partition_c.0 = max_bits;
            info.partition_c.1 = max_bits;
        } else if partition_count == 1 {
            // 只有一个 resync marker，Partition C 为空
            info.partition_c.1 = max_bits;
        }

        (info, partition_count)
    }

    /// 扫描数据分区中的分包边界 (旧版本，保留用于兼容)
    ///
    /// 数据分区的每个分包都有 resync marker。本函数扫描位流以检测分包数量。
    /// 返回找到的分包数（含第一个隐含分包）。
    #[allow(dead_code)]
    fn scan_data_partitions(data: &[u8]) -> u32 {
        let mut partition_count = 1u32;
        let mut offset = 0;

        // 简单启发式: 扫描 resync marker 出现次数
        // resync marker pattern: 16+ 个零位 + 1 个一位
        while offset < data.len() {
            // 查找下一个潜在的 resync marker (0x00 0x00 字节序列作为启发式指标)
            if offset + 2 < data.len() && data[offset] == 0x00 && data[offset + 1] == 0x00 {
                partition_count += 1;
            }
            offset += 1;
        }

        partition_count
    }

    /// 检查 resync marker
    ///
    /// resync marker 是 stuffing bits + (16 + vop_fcode) 个零 + 1 个一.
    /// 先检查到字节边界的 stuffing bits 是否全为 0,
    /// 然后检查后续 (17 + fcode - 1) 位是否为 "000...001" 模式.
    fn check_resync_marker(reader: &BitReader, vop_fcode: u8) -> bool {
        let nbits = reader.bits_to_byte_align();
        // 检查 stuffing bits (到字节边界) 是否全为 0
        if nbits > 0 {
            let Some(stuffing) = reader.peek_bits(nbits) else {
                return false;
            };
            if stuffing != 0 {
                return false;
            }
        }

        // resync marker 长度 = 17 + (fcode - 1) = 16 + fcode
        let marker_len = 16 + vop_fcode as u32;
        let total_bits = nbits as u32 + marker_len;
        if total_bits > 32 {
            return false;
        }

        let Some(bits) = reader.peek_bits(total_bits as u8) else {
            return false;
        };

        // 所有位为 0 除最后一位为 1
        bits == 1
    }

    /// 跳过 resync marker 并解析 video packet header
    ///
    /// 返回 (macroblock_number, new_quant)
    fn parse_video_packet_header(&self, reader: &mut BitReader) -> Option<(u32, u8)> {
        // 跳过 stuffing bits
        let nbits = reader.bits_to_byte_align();
        if nbits > 0 {
            reader.read_bits(nbits)?;
        }

        // 跳过 resync marker (16 + fcode 位)
        let marker_len = 16 + self.f_code_forward;
        reader.read_bits(marker_len)?;

        // macroblock_number (变长: log2(total_mbs) 位)
        let total_mbs = self.mb_stride * (self.height as usize).div_ceil(16);
        let mb_bits = if total_mbs > 1 {
            (total_mbs as f32).log2().ceil() as u8
        } else {
            1
        };
        let mb_number = reader.read_bits(mb_bits)?;

        // quant_scale (5 bits)
        let quant = reader.read_bits(5)? as u8;

        // header_extension_code (1 bit) - 暂时跳过扩展
        let hec = reader.read_bit().unwrap_or(false);
        if hec {
            // 扩展头: modulo_time_base + marker + vop_time_increment + marker
            // + vop_coding_type + intra_dc_vlc_thr + (f_codes)
            // 简化处理: 跳过扩展部分
            while reader.read_bit() == Some(true) {}
            reader.read_bit(); // marker
            let time_inc_resolution = self
                .vol_info
                .as_ref()
                .map(|v| v.vop_time_increment_resolution)
                .unwrap_or(30000);
            let bits = if time_inc_resolution > 1 {
                (time_inc_resolution as f32).log2().ceil() as u8
            } else {
                1
            };
            reader.read_bits(bits.max(1)); // vop_time_increment
            reader.read_bit(); // marker
            reader.read_bits(2); // vop_coding_type
            reader.read_bits(3); // intra_dc_vlc_thr
            // f_code_forward (P/B)
            reader.read_bits(3);
        }

        Some((mb_number, quant))
    }

    // ========================================================================
    // Data Partitioning 解码函数
    // ========================================================================

    /// 从 Partition A 解码宏块头部信息
    ///
    /// 解码内容:
    /// - MCBPC/CBPY (宏块类型和 CBP)
    /// - DQUANT (量化参数变化)
    /// - 运动向量
    /// - field_dct 标志
    fn decode_partition_a_mb_header(
        &mut self,
        reader: &mut BitReader,
        mb_x: u32,
        mb_y: u32,
        is_i_vop: bool,
    ) -> Option<PartitionedMacroblockData> {
        // 1. MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader)?
        } else {
            // P-VOP: not_coded 位
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                return Some(PartitionedMacroblockData {
                    mb_type: MbType::Inter,
                    cbp: 0,
                    quant: self.quant,
                    ac_pred_flag: false,
                    mvs: [MotionVector::default(); 4],
                    dc_coeffs: [0; 6],
                    ac_coeffs: [[0; 63]; 6],
                    field_dct: false,
                    field_pred: false,
                    field_for_top: false,
                    field_for_bot: false,
                });
            }
            decode_mcbpc_p(reader)?
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // AC prediction flag
        let ac_pred_flag = if is_intra {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // 2. CBPY
        let cbpy = decode_cbpy(reader, is_intra).unwrap_or(0);

        // 3. DQUANT
        if mb_type == MbType::IntraQ || mb_type == MbType::InterQ {
            if let Some(dq) = reader.read_bits(2) {
                let delta = DQUANT_TABLE[dq as usize];
                self.quant = ((self.quant as i32 + delta).clamp(1, 31)) as u8;
            }
        }

        // 4. 隔行模式: field_dct
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        let cbp = (cbpy << 2) | cbpc;
        let field_dct = if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // field_pred (仅 P-VOP)
        let mut field_pred = false;
        let mut field_for_top = false;
        let mut field_for_bot = false;
        if interlacing && !is_intra {
            field_pred = reader.read_bit().unwrap_or(false);
            if field_pred {
                field_for_top = reader.read_bit().unwrap_or(false);
                field_for_bot = reader.read_bit().unwrap_or(false);
            }
        }

        // 5. 运动向量解码
        let mut mb_mvs = [MotionVector::default(); 4];
        if !is_intra {
            if field_pred && mb_type != MbType::Inter4V {
                let mut mv_top = MotionVector::default();
                let mut mv_bot = MotionVector::default();
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_top = mv;
                }
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 2) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_bot = mv;
                }
                mb_mvs = [mv_top, mv_top, mv_bot, mv_bot];
            } else if mb_type == MbType::Inter4V {
                for (k, mv_slot) in mb_mvs.iter_mut().enumerate() {
                    if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, k) {
                        self.validate_vector(&mut mv, mb_x, mb_y);
                        *mv_slot = mv;
                    }
                }
            } else if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                self.validate_vector(&mut mv, mb_x, mb_y);
                mb_mvs = [mv; 4];
            }
        }

        Some(PartitionedMacroblockData {
            mb_type,
            cbp,
            quant: self.quant,
            ac_pred_flag,
            mvs: mb_mvs,
            dc_coeffs: [0; 6],
            ac_coeffs: [[0; 63]; 6],
            field_dct,
            field_pred,
            field_for_top,
            field_for_bot,
        })
    }

    /// 从 Partition B 解码 DC 系数
    ///
    /// 使用 RVLC 解码所有块的 DC 系数。
    fn decode_partition_b_dc(
        &mut self,
        reader: &mut BitReader,
        mb_data: &mut PartitionedMacroblockData,
        mb_x: u32,
        mb_y: u32,
    ) -> bool {
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);
        if !is_intra {
            // Inter 块没有 DC 系数在 Partition B
            return true;
        }

        use self::vlc::decode_intra_dc_vlc;

        // 解码 6 个块的 DC 系数 (Y0-Y3, U, V)
        for block_idx in 0..6 {
            let is_luma = block_idx < 4;

            // 使用 use_intra_dc_vlc 决定是否解码 DC
            let dc_diff = if self.use_intra_dc_vlc() {
                if let Some(dc) = decode_intra_dc_vlc(reader, is_luma) {
                    dc
                } else {
                    warn!(
                        "Partition B DC 解码失败, MB ({}, {}), block {}",
                        mb_x, mb_y, block_idx
                    );
                    return false;
                }
            } else {
                0
            };

            // 使用 DC 预测
            let (dc_pred, _direction) =
                self.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);
            let actual_dc = dc_pred.wrapping_add(dc_diff);
            let dc_scaler = self.get_dc_scaler(is_luma);
            mb_data.dc_coeffs[block_idx] = (actual_dc as i32 * dc_scaler as i32) as i16;
        }

        true
    }

    /// 从 Partition C 解码 AC 系数
    ///
    /// 解码所有块的 AC 系数。
    fn decode_partition_c_ac(
        &mut self,
        reader: &mut BitReader,
        mb_data: &mut PartitionedMacroblockData,
        mb_x: u32,
        mb_y: u32,
    ) -> bool {
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);

        // 选择扫描表
        let scan_table = if mb_data.field_dct {
            &ALTERNATE_VERTICAL_SCAN
        } else {
            &ZIGZAG_SCAN
        };

        // 解码 6 个块的 AC 系数
        for block_idx in 0..6 {
            let ac_coded = (mb_data.cbp >> (5 - block_idx)) & 1 != 0;
            if !ac_coded {
                continue;
            }

            let plane = if block_idx < 4 { 0 } else { block_idx - 3 };

            // 解码 AC 系数
            let coeffs = if is_intra {
                // Intra 块: 跳过 DC (已在 Partition B 中解码), 只解码 AC
                self.decode_intra_ac_only(
                    reader,
                    plane,
                    mb_x,
                    mb_y,
                    block_idx,
                    mb_data.ac_pred_flag,
                    scan_table,
                )
            } else {
                // Inter 块
                decode_inter_block_vlc(reader, scan_table)
            };

            if let Some(block_coeffs) = coeffs {
                // 提取 AC 系数 (跳过 DC)
                for (i, &coeff) in block_coeffs.iter().enumerate().skip(1) {
                    mb_data.ac_coeffs[block_idx][i - 1] = coeff as i16;
                }
            } else {
                warn!(
                    "Partition C AC 解码失败, MB ({}, {}), block {}",
                    mb_x, mb_y, block_idx
                );
                return false;
            }
        }

        true
    }

    /// 辅助函数: 仅解码 Intra 块的 AC 系数 (跳过 DC)
    #[allow(clippy::too_many_arguments)]
    fn decode_intra_ac_only(
        &mut self,
        reader: &mut BitReader,
        _plane: usize,
        mb_x: u32,
        mb_y: u32,
        block_idx: usize,
        ac_pred_flag: bool,
        scan_table: &[usize; 64],
    ) -> Option<[i32; 64]> {
        use self::tables::{ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN};
        use self::types::PredictorDirection;
        use self::vlc::{INTRA_AC_VLC, decode_ac_vlc};

        const COEFF_MIN: i32 = -2048;
        const COEFF_MAX: i32 = 2047;

        let mut block = [0i32; 64];
        // DC 已在 Partition B 中解码, 这里只解码 AC

        // 获取 AC 预测方向
        let (_dc_pred, direction) =
            self.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);

        // 选择扫描顺序
        let ac_scan = if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,
                PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,
                PredictorDirection::None => scan_table,
            }
        } else {
            scan_table
        };

        // 解码 AC 系数
        let mut pos = 1; // 从 1 开始 (跳过 DC)
        while pos < 64 {
            match decode_ac_vlc(reader, INTRA_AC_VLC, true) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[ac_scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }

        // AC 预测
        if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => {
                    let c_idx = match block_idx {
                        0 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                        1 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                        2 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        3 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i];
                            block[idx] = block[idx]
                                .wrapping_add(pred_ac[i] as i32)
                                .clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
                PredictorDirection::Horizontal => {
                    let c_idx = match block_idx {
                        0 | 2 => self.get_neighbor_block_idx(
                            mb_x as isize - 1,
                            mb_y as isize,
                            block_idx + 1,
                        ),
                        1 | 3 => {
                            self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, block_idx - 1)
                        }
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in (1..8).step_by(8) {
                            for j in 0..8 {
                                let idx = ac_scan[i + j];
                                block[idx] = block[idx]
                                    .wrapping_add(pred_ac[j + 1] as i32)
                                    .clamp(COEFF_MIN, COEFF_MAX);
                            }
                        }
                    }
                }
                PredictorDirection::None => {}
            }
        }

        Some(block)
    }

    /// 重建分区宏块到帧
    ///
    /// 将从三个分区解码的数据组合并重建到输出帧。
    fn reconstruct_partitioned_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        mb_data: &PartitionedMacroblockData,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);

        // 存储 MV 到缓存
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_data.mvs;
        }

        // 更新宏块信息
        if mb_idx < self.mb_info.len() {
            let mode_code = match mb_data.mb_type {
                MbType::Inter | MbType::InterQ => MacroblockInfo::MODE_INTER,
                MbType::Intra | MbType::IntraQ => MacroblockInfo::MODE_INTRA,
                MbType::Inter4V => MacroblockInfo::MODE_INTER4V,
            };
            self.mb_info[mb_idx] = MacroblockInfo {
                mode: mode_code,
                quant: mb_data.quant,
                mvs: mb_data.mvs,
            };
        }

        let quarterpel = self
            .vol_info
            .as_ref()
            .map(|v| v.quarterpel)
            .unwrap_or(false);
        let field_pred = mb_data.field_pred;
        let use_quarterpel = quarterpel;

        // 重建 Y 平面的 4 个 8x8 块
        for block_idx in 0..4 {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;

            // 组合 DC 和 AC 系数
            let mut block = [0i32; 64];
            block[0] = mb_data.dc_coeffs[block_idx] as i32;
            for i in 0..63 {
                block[i + 1] = mb_data.ac_coeffs[block_idx][i] as i32;
            }

            // 反量化和 IDCT
            self.dequantize(&mut block, mb_data.quant as u32, is_intra);
            idct_8x8(&mut block);

            let mv = mb_data.mvs[block_idx];

            // 写入帧
            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 16 + bx as usize * 8 + x) as isize;
                    let py = (mb_y as usize * 16 + by as usize * 8 + y) as isize;
                    if px < width as isize && py < height as isize {
                        let idx = py as usize * width + px as usize;
                        let residual = block[y * 8 + x];
                        let val = if is_intra {
                            (residual + 128).clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_block(
                                    block_idx,
                                    mb_data.field_for_top,
                                    mb_data.field_for_bot,
                                );
                                let mv_y = Self::scale_field_mv_y(mv.y);
                                Self::motion_compensate_field(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv_y,
                                    self.rounding_control,
                                    use_quarterpel,
                                    field_select,
                                )
                            } else {
                                Self::motion_compensate(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv.y,
                                    self.rounding_control,
                                    use_quarterpel,
                                )
                            };
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[0][idx] = val;
                    }
                }
            }
        }

        // 重建 U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        let (chroma_mv, chroma_mv_top, chroma_mv_bot) = if !is_intra {
            if field_pred {
                if mb_data.mb_type == MbType::Inter4V {
                    let top_avg = Self::average_mv(mb_data.mvs[0], mb_data.mvs[1]);
                    let bot_avg = Self::average_mv(mb_data.mvs[2], mb_data.mvs[3]);
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(top_avg),
                        Self::chroma_mv_1mv(bot_avg),
                    )
                } else {
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(mb_data.mvs[0]),
                        Self::chroma_mv_1mv(mb_data.mvs[2]),
                    )
                }
            } else if mb_data.mb_type == MbType::Inter4V {
                (
                    Self::chroma_mv_4mv(&mb_data.mvs),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            } else {
                (
                    Self::chroma_mv_1mv(mb_data.mvs[0]),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            }
        } else {
            (
                MotionVector::default(),
                MotionVector::default(),
                MotionVector::default(),
            )
        };

        // 色度仍使用半像素 MC, qpel 仅作用于亮度.
        let chroma_quarterpel = false;
        for plane_idx in 0..2 {
            let block_idx = 4 + plane_idx;

            // 组合 DC 和 AC 系数
            let mut block = [0i32; 64];
            block[0] = mb_data.dc_coeffs[block_idx] as i32;
            for i in 0..63 {
                block[i + 1] = mb_data.ac_coeffs[block_idx][i] as i32;
            }

            // 反量化和 IDCT
            self.dequantize(&mut block, mb_data.quant as u32, is_intra);
            idct_8x8(&mut block);

            // 写入帧
            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 8 + x).min(uv_width - 1);
                    let py = (mb_y as usize * 8 + y).min(uv_height - 1);
                    let idx = py * uv_width + px;
                    let residual = block[y * 8 + x];
                    let val = if is_intra {
                        (residual + 128).clamp(0, 255) as u8
                    } else if let Some(ref_frame) = &self.reference_frame {
                        let pred = if field_pred {
                            let field_select = Self::select_field_for_chroma_line(
                                py,
                                mb_data.field_for_top,
                                mb_data.field_for_bot,
                            );
                            let mv = if field_select {
                                chroma_mv_top
                            } else {
                                chroma_mv_bot
                            };
                            let mv_y = Self::scale_field_mv_y(mv.y);
                            Self::motion_compensate_field(
                                ref_frame,
                                plane_idx + 1,
                                px as isize,
                                py as isize,
                                mv.x,
                                mv_y,
                                self.rounding_control,
                                chroma_quarterpel,
                                field_select,
                            )
                        } else {
                            Self::motion_compensate(
                                ref_frame,
                                plane_idx + 1,
                                px as isize,
                                py as isize,
                                chroma_mv.x,
                                chroma_mv.y,
                                self.rounding_control,
                                chroma_quarterpel,
                            )
                        };
                        (pred as i32 + residual).clamp(0, 255) as u8
                    } else {
                        (residual + 128).clamp(0, 255) as u8
                    };
                    frame.data[plane_idx + 1][idx] = val;
                }
            }
        }
    }

    /// 使用 Data Partitioning 模式解码 I/P 帧
    ///
    /// 三步解码流程:
    /// 1. 从 Partition A 解码所有 MB 的头部信息
    /// 2. 从 Partition B 解码所有 DC 系数
    /// 3. 从 Partition C 解码所有 AC 系数
    /// 4. 重建所有宏块到帧
    fn decode_frame_partitioned(
        &mut self,
        packet_data: &[u8],
        part_info: &DataPartitionInfo,
        is_i_vop: bool,
    ) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        let mut frame = self.create_blank_frame(if is_i_vop {
            PictureType::I
        } else {
            PictureType::P
        });

        let mb_w = self.width.div_ceil(16) as usize;
        let mb_h = self.height.div_ceil(16) as usize;
        let total_mbs = mb_w * mb_h;

        debug!(
            "Data Partitioning 解码 {} 帧: {}x{} ({} MB)",
            if is_i_vop { "I" } else { "P" },
            self.width,
            self.height,
            total_mbs
        );

        // 存储所有宏块的中间数据
        let mut mb_data_vec: Vec<Option<PartitionedMacroblockData>> = vec![None; total_mbs];

        // === 步骤 1: 从 Partition A 解码所有 MB 头部 ===
        debug!("  步骤 1: 解码 Partition A (MB 头部)");
        let partition_a_bytes = part_info.partition_a.0 / 8;
        let partition_a_len = (part_info.partition_a.1 - part_info.partition_a.0).div_ceil(8);

        if partition_a_bytes + partition_a_len <= packet_data.len() {
            let partition_a_data =
                &packet_data[partition_a_bytes..partition_a_bytes + partition_a_len];
            let mut reader_a = BitReader::new(partition_a_data);

            for mb_y in 0..mb_h {
                for mb_x in 0..mb_w {
                    let mb_idx = mb_y * mb_w + mb_x;
                    if let Some(mb_data) = self.decode_partition_a_mb_header(
                        &mut reader_a,
                        mb_x as u32,
                        mb_y as u32,
                        is_i_vop,
                    ) {
                        mb_data_vec[mb_idx] = Some(mb_data);
                    } else {
                        warn!("  Partition A 解码失败: MB ({}, {})", mb_x, mb_y);
                        // 使用标准顺序解码作为降级
                        return self.decode_frame_standard(packet_data, is_i_vop);
                    }
                }
            }
        } else {
            warn!("  Partition A 数据不足，降级到标准解码");
            return self.decode_frame_standard(packet_data, is_i_vop);
        }

        // === 步骤 2: 从 Partition B 解码所有 DC 系数 ===
        debug!("  步骤 2: 解码 Partition B (DC 系数)");
        if part_info.partition_b.0 < part_info.partition_b.1 {
            let partition_b_bytes = part_info.partition_b.0 / 8;
            let partition_b_len = (part_info.partition_b.1 - part_info.partition_b.0).div_ceil(8);

            if partition_b_bytes + partition_b_len <= packet_data.len() {
                let partition_b_data =
                    &packet_data[partition_b_bytes..partition_b_bytes + partition_b_len];
                let mut reader_b = BitReader::new(partition_b_data);

                for mb_y in 0..mb_h {
                    for mb_x in 0..mb_w {
                        let mb_idx = mb_y * mb_w + mb_x;
                        if let Some(ref mut mb_data) = mb_data_vec[mb_idx] {
                            if !self.decode_partition_b_dc(
                                &mut reader_b,
                                mb_data,
                                mb_x as u32,
                                mb_y as u32,
                            ) {
                                warn!(
                                    "  Partition B 解码失败: MB ({}, {}), 降级到标准解码",
                                    mb_x, mb_y
                                );
                                return self.decode_frame_standard(packet_data, is_i_vop);
                            }
                        }
                    }
                }
            }
        }

        // === 步骤 3: 从 Partition C 解码所有 AC 系数 ===
        debug!("  步骤 3: 解码 Partition C (AC 系数)");
        if part_info.partition_c.0 < part_info.partition_c.1 {
            let partition_c_bytes = part_info.partition_c.0 / 8;
            let partition_c_len = (part_info.partition_c.1 - part_info.partition_c.0).div_ceil(8);

            if partition_c_bytes + partition_c_len <= packet_data.len() {
                let partition_c_data =
                    &packet_data[partition_c_bytes..partition_c_bytes + partition_c_len];
                let mut reader_c = BitReader::new(partition_c_data);

                for mb_y in 0..mb_h {
                    for mb_x in 0..mb_w {
                        let mb_idx = mb_y * mb_w + mb_x;
                        if let Some(ref mut mb_data) = mb_data_vec[mb_idx] {
                            if !self.decode_partition_c_ac(
                                &mut reader_c,
                                mb_data,
                                mb_x as u32,
                                mb_y as u32,
                            ) {
                                warn!("  Partition C 解码失败: MB ({}, {}), 使用零 AC", mb_x, mb_y);
                                // 继续处理，使用零 AC 系数
                            }
                        }
                    }
                }
            }
        }

        // === 步骤 4: 重建所有宏块 ===
        debug!("  步骤 4: 重建宏块到帧");
        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                let mb_idx = mb_y * mb_w + mb_x;
                if let Some(ref mb_data) = mb_data_vec[mb_idx] {
                    self.reconstruct_partitioned_macroblock(
                        &mut frame,
                        mb_x as u32,
                        mb_y as u32,
                        mb_data,
                    );
                }
            }
        }

        debug!("  Data Partitioning 解码完成");
        Ok(frame)
    }

    /// 标准顺序解码 (降级路径)
    fn decode_frame_standard(
        &mut self,
        packet_data: &[u8],
        is_i_vop: bool,
    ) -> TaoResult<VideoFrame> {
        let vop_offset = find_start_code_offset(packet_data, START_CODE_VOP)
            .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
        let mut reader = BitReader::new(&packet_data[vop_offset..]);

        // 重新解析 VOP header
        let _ = self.parse_vop_header(&mut reader)?;

        if is_i_vop {
            self.decode_i_frame(&mut reader)
        } else {
            self.decode_p_frame(&mut reader)
        }
    }

    // ========================================================================
    // 宏块和帧解码
    // ========================================================================

    /// 解码单个宏块
    fn decode_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        reader: &mut BitReader,
        is_i_vop: bool,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;

        // P-VOP: not_coded 位
        if !is_i_vop {
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                self.copy_mb_from_ref(frame, mb_x, mb_y);
                if mb_idx < self.mv_cache.len() {
                    self.mv_cache[mb_idx] = [MotionVector::default(); 4];
                }
                // 更新宏块信息
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

        // 1. MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader).unwrap_or((MbType::Intra, 0))
        } else {
            decode_mcbpc_p(reader).unwrap_or((MbType::Inter, 0))
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // AC/DC prediction flag
        let ac_pred_flag = if is_intra {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // 2. CBPY
        let cbpy = decode_cbpy(reader, is_intra).unwrap_or(0);

        // 3. DQUANT
        if mb_type == MbType::IntraQ || mb_type == MbType::InterQ {
            if let Some(dq) = reader.read_bits(2) {
                let delta = DQUANT_TABLE[dq as usize];
                self.quant = ((self.quant as i32 + delta).clamp(1, 31)) as u8;
            }
        }

        // 4. 隔行模式: field_dct 和 field_pred
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        let field_dct = if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };
        let mut field_pred = false;
        let mut field_for_top = false;
        let mut field_for_bot = false;
        if interlacing && !is_intra {
            field_pred = reader.read_bit().unwrap_or(false);
            if field_pred {
                // 场预测: 读取顶场和底场参考选择
                field_for_top = reader.read_bit().unwrap_or(false);
                field_for_bot = reader.read_bit().unwrap_or(false);
            }
        }

        // quarterpel 标志
        let quarterpel = self
            .vol_info
            .as_ref()
            .map(|v| v.quarterpel)
            .unwrap_or(false);
        let use_quarterpel = quarterpel;

        // 5. 运动向量解码
        let mut mb_mvs = [MotionVector::default(); 4];

        if !is_intra {
            if field_pred && mb_type != MbType::Inter4V {
                let mut mv_top = MotionVector::default();
                let mut mv_bot = MotionVector::default();
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_top = mv;
                }
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 2) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_bot = mv;
                }
                mb_mvs = [mv_top, mv_top, mv_bot, mv_bot];
            } else if mb_type == MbType::Inter4V {
                for (k, mv_slot) in mb_mvs.iter_mut().enumerate() {
                    if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, k) {
                        self.validate_vector(&mut mv, mb_x, mb_y);
                        *mv_slot = mv;
                    }
                    if mb_idx < self.mv_cache.len() {
                        self.mv_cache[mb_idx][k] = *mv_slot;
                    }
                }
            } else if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
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

        // 6. CBP 组合
        let cbp = (cbpy << 2) | cbpc;

        // 选择扫描表 (field_dct 使用 alternate vertical scan)
        let scan_table = if field_dct {
            &ALTERNATE_VERTICAL_SCAN
        } else {
            &ZIGZAG_SCAN
        };

        // 7. 解码各 8x8 块 - Y 平面
        #[allow(clippy::needless_range_loop)]
        for block_idx in 0..4usize {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;
            let ac_coded = (cbp >> (5 - block_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    0,
                    mb_x,
                    mb_y,
                    block_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                    scan_table,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, is_intra);
            idct_8x8(&mut block);

            let mv = if !is_intra {
                mb_mvs[block_idx]
            } else {
                MotionVector::default()
            };

            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 16 + bx as usize * 8 + x) as isize;
                    let py = (mb_y as usize * 16 + by as usize * 8 + y) as isize;
                    if px < width as isize && py < height as isize {
                        let idx = py as usize * width + px as usize;
                        let residual = block[y * 8 + x];
                        let val = if is_intra {
                            (residual + 128).clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_block(
                                    block_idx,
                                    field_for_top,
                                    field_for_bot,
                                );
                                let mv_y = Self::scale_field_mv_y(mv.y);
                                Self::motion_compensate_field(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv_y,
                                    self.rounding_control,
                                    use_quarterpel,
                                    field_select,
                                )
                            } else {
                                Self::motion_compensate(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv.y,
                                    self.rounding_control,
                                    use_quarterpel,
                                )
                            };
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[0][idx] = val;
                    }
                }
            }
        }

        // U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        let (chroma_mv, chroma_mv_top, chroma_mv_bot) = if !is_intra {
            if field_pred {
                if mb_type == MbType::Inter4V {
                    let top_avg = Self::average_mv(mb_mvs[0], mb_mvs[1]);
                    let bot_avg = Self::average_mv(mb_mvs[2], mb_mvs[3]);
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(top_avg),
                        Self::chroma_mv_1mv(bot_avg),
                    )
                } else {
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(mb_mvs[0]),
                        Self::chroma_mv_1mv(mb_mvs[2]),
                    )
                }
            } else if mb_type == MbType::Inter4V {
                (
                    Self::chroma_mv_4mv(&mb_mvs),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            } else {
                (
                    Self::chroma_mv_1mv(mb_mvs[0]),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            }
        } else {
            (
                MotionVector::default(),
                MotionVector::default(),
                MotionVector::default(),
            )
        };

        // 色度仍使用半像素 MC, qpel 仅作用于亮度.
        let chroma_quarterpel = false;
        for plane_idx in 0..2usize {
            let ac_coded = (cbp >> (1 - plane_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    plane_idx + 1,
                    mb_x,
                    mb_y,
                    4 + plane_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                    scan_table,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, is_intra);
            idct_8x8(&mut block);

            for v in 0..8 {
                for u in 0..8 {
                    let px = (mb_x as usize * 8 + u) as isize;
                    let py = (mb_y as usize * 8 + v) as isize;
                    if px < uv_width as isize && py < uv_height as isize {
                        let idx = py as usize * uv_width + px as usize;
                        let residual = block[v * 8 + u];
                        let val = if is_intra {
                            (residual + 128).clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_chroma_line(
                                    py as usize,
                                    field_for_top,
                                    field_for_bot,
                                );
                                let mv = if field_select {
                                    chroma_mv_top
                                } else {
                                    chroma_mv_bot
                                };
                                let mv_y = Self::scale_field_mv_y(mv.y);
                                Self::motion_compensate_field(
                                    ref_frame,
                                    plane_idx + 1,
                                    px,
                                    py,
                                    mv.x,
                                    mv_y,
                                    self.rounding_control,
                                    chroma_quarterpel,
                                    field_select,
                                )
                            } else {
                                Self::motion_compensate(
                                    ref_frame,
                                    plane_idx + 1,
                                    px,
                                    py,
                                    chroma_mv.x,
                                    chroma_mv.y,
                                    self.rounding_control,
                                    chroma_quarterpel,
                                )
                            };
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[plane_idx + 1][idx] = val;
                    }
                }
            }
        }
    }

    /// 初始化帧解码前的通用状态
    fn init_frame_decode(&mut self) {
        let mb_count = self.mb_stride * (self.height as usize).div_ceil(16);
        let total_blocks = mb_count * 6;
        self.predictor_cache = vec![[1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        // 确保 MV 缓存和宏块信息大小正确
        if self.mv_cache.len() != mb_count {
            self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        }
        if self.mb_info.len() != mb_count {
            self.mb_info = vec![MacroblockInfo::default(); mb_count];
        }
    }

    /// 创建空白帧
    fn create_blank_frame(&self, picture_type: PictureType) -> VideoFrame {
        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = picture_type;
        frame.is_keyframe = picture_type == PictureType::I;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;
        frame
    }

    /// 解码 I 帧
    fn decode_i_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        let mut frame = self.create_blank_frame(PictureType::I);

        let mb_w = self.width.div_ceil(16);
        let mb_h = self.height.div_ceil(16);
        debug!(
            "解码 I 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        let resync_disabled = self
            .vol_info
            .as_ref()
            .map(|v| v.resync_marker_disable)
            .unwrap_or(true);

        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                // 检查 resync marker (错误恢复)
                if !resync_disabled && Self::check_resync_marker(reader, 0) {
                    if let Some((mb_num, new_quant)) = self.parse_video_packet_header(reader) {
                        debug!("I 帧 resync marker: MB={}, quant={}", mb_num, new_quant);
                        self.quant = new_quant;
                    }
                }
                self.decode_macroblock(&mut frame, mb_x, mb_y, reader, true);
            }
        }
        Ok(frame)
    }

    /// 解码 P 帧
    fn decode_p_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        let mut frame = self.create_blank_frame(PictureType::P);

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);
        debug!(
            "解码 P 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        let resync_disabled = self
            .vol_info
            .as_ref()
            .map(|v| v.resync_marker_disable)
            .unwrap_or(true);
        let fcode = self.f_code_forward;

        for mb_y in 0..mb_h as u32 {
            for mb_x in 0..mb_w as u32 {
                // 检查 resync marker (错误恢复)
                if !resync_disabled && Self::check_resync_marker(reader, fcode.saturating_sub(1)) {
                    if let Some((mb_num, new_quant)) = self.parse_video_packet_header(reader) {
                        debug!("P 帧 resync marker: MB={}, quant={}", mb_num, new_quant);
                        self.quant = new_quant;
                    }
                }
                self.decode_macroblock(&mut frame, mb_x, mb_y, reader, false);
            }
        }
        Ok(frame)
    }

    /// 从参考帧复制宏块
    fn copy_mb_from_ref(&self, frame: &mut VideoFrame, mb_x: u32, mb_y: u32) {
        if let Some(ref_frame) = &self.reference_frame {
            let width = self.width as usize;
            let height = self.height as usize;

            for y in 0..16 {
                for x in 0..16 {
                    let px = (mb_x as usize * 16 + x).min(width - 1);
                    let py = (mb_y as usize * 16 + y).min(height - 1);
                    let idx = py * width + px;
                    frame.data[0][idx] = ref_frame.data[0][idx];
                }
            }

            let uv_w = width / 2;
            let uv_h = height / 2;
            for plane in 1..3 {
                for y in 0..8 {
                    for x in 0..8 {
                        let px = (mb_x as usize * 8 + x).min(uv_w - 1);
                        let py = (mb_y as usize * 8 + y).min(uv_h - 1);
                        let idx = py * uv_w + px;
                        frame.data[plane][idx] = ref_frame.data[plane][idx];
                    }
                }
            }
        }
    }
}

// ============================================================================
// Decoder trait 实现
// ============================================================================

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
        self.mb_stride = (video.width as usize).div_ceil(16);
        self.pixel_format = PixelFormat::Yuv420p;
        self.opened = true;
        self.frame_count = 0;
        self.reference_frame = None;
        self.backward_reference = None;
        self.dpb.clear();

        let mb_count = self.mb_stride * (video.height as usize).div_ceil(16);
        self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        self.ref_mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        self.mb_info = vec![MacroblockInfo::default(); mb_count];
        self.last_time_base = 0;
        self.time_base_acc = 0;
        self.last_non_b_time = 0;

        if !params.extra_data.is_empty() {
            self.parse_vol_header(&params.extra_data)?;
            // 从 extradata 中解析 user_data (识别编码器)
            self.parse_user_data(&params.extra_data);
        }

        debug!(
            "打开 MPEG4 解码器: {}x{}, mb_stride={}",
            self.width, self.height, self.mb_stride
        );
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }

        if packet.is_empty() {
            debug!("收到刷新信号");
            return Ok(());
        }

        // Short Video Header (H.263) 检测:
        // 如果数据匹配 short header 起始码且没有 MPEG-4 VOP 起始码,
        // 则使用 H.263 baseline 解码路径
        let has_vop_start_code = find_start_code_offset(&packet.data, START_CODE_VOP).is_some();
        if !has_vop_start_code && Self::is_short_video_header(&packet.data) {
            let header = self.parse_short_video_header(&packet.data)?;
            debug!(
                "检测到 Short Video Header (H.263), TR={}, 使用 H.263 解码路径",
                header.temporal_reference
            );
            let mut frame = self.decode_short_header_frame(&packet.data, &header)?;

            // 设置帧元数据
            frame.pts = packet.pts;
            frame.time_base = packet.time_base;
            frame.duration = packet.duration;

            // 更新参考帧 (I/P 帧)
            if header.picture_type == PictureType::I || header.picture_type == PictureType::P {
                self.reference_frame = Some(frame.clone());
            }

            self.pending_frame = Some(frame);
            self.frame_count += 1;
            return Ok(());
        }

        if self.vol_info.is_none() {
            if let Err(e) = self.parse_vol_header(&packet.data) {
                debug!("VOL 解析失败: {:?}", e);
            }
        }

        // 解析 user_data (识别编码器类型, 检测 packed bitstream 等)
        if self
            .vol_info
            .as_ref()
            .map(|v| v.encoder_info.encoder_type == types::EncoderType::Unknown)
            .unwrap_or(false)
        {
            self.parse_user_data(&packet.data);
        }

        // DivX packed bitstream 处理:
        // 先处理之前缓存的 packed frames
        if let Some(queued_data) = self.packed_frames.pop_front() {
            let queued_packet = Packet {
                data: queued_data.into(),
                pts: packet.pts,
                dts: packet.dts,
                duration: packet.duration,
                time_base: packet.time_base,
                stream_index: packet.stream_index,
                is_keyframe: false,
                pos: -1,
            };
            return self.send_packet_standard(&queued_packet);
        }

        // 检测并拆分 packed bitstream
        let is_packed = self
            .vol_info
            .as_ref()
            .map(|v| v.encoder_info.packed_bitstream)
            .unwrap_or(false);

        if is_packed {
            let vop_offsets = Self::find_all_vop_offsets(&packet.data);
            if vop_offsets.len() > 1 {
                debug!(
                    "DivX packed bitstream: 检测到 {} 个 VOP, 拆分处理",
                    vop_offsets.len()
                );
                // 将后续 VOP 数据缓存到队列
                for i in 1..vop_offsets.len() {
                    let start = vop_offsets[i];
                    // VOP 起始码从 00 00 01 B6 开始, 回退 4 字节以包含起始码
                    let vop_start = start.saturating_sub(4);
                    let end = if i + 1 < vop_offsets.len() {
                        vop_offsets[i + 1].saturating_sub(4)
                    } else {
                        packet.data.len()
                    };
                    if vop_start < end {
                        self.packed_frames
                            .push_back(packet.data[vop_start..end].to_vec());
                    }
                }
                // 只处理第一个 VOP (截断 packet 数据到第二个 VOP 之前)
                let first_end = if vop_offsets.len() > 1 {
                    vop_offsets[1].saturating_sub(4)
                } else {
                    packet.data.len()
                };
                let first_packet = Packet {
                    data: packet.data.slice(..first_end),
                    pts: packet.pts,
                    dts: packet.dts,
                    duration: packet.duration,
                    time_base: packet.time_base,
                    stream_index: packet.stream_index,
                    is_keyframe: packet.is_keyframe,
                    pos: packet.pos,
                };
                return self.send_packet_standard(&first_packet);
            }
        }

        // 如果 VOL 指示 data_partitioned 或者可逆 VLC, 执行分区分析
        let data_partitioned = self
            .vol_info
            .as_ref()
            .map(|v| v.data_partitioned)
            .unwrap_or(false);
        let reversible_vlc = self
            .vol_info
            .as_ref()
            .map(|v| v.reversible_vlc)
            .unwrap_or(false);

        if data_partitioned {
            let fcode = self.f_code_forward.saturating_sub(1);
            let (part_info, partition_count) = self.analyze_data_partitions(&packet.data, fcode);
            debug!("数据分区模式已启用:");
            debug!("  分区数量: {}", partition_count + 1);
            debug!(
                "  Partition A (MB类型/量化/运动向量): 位 [{}, {}) = {} 字节",
                part_info.partition_a.0,
                part_info.partition_a.1,
                (part_info.partition_a.1 - part_info.partition_a.0).div_ceil(8)
            );
            if partition_count >= 1 {
                debug!(
                    "  Partition B (DC系数/RVLC): 位 [{}, {}) = {} 字节",
                    part_info.partition_b.0,
                    part_info.partition_b.1,
                    (part_info.partition_b.1 - part_info.partition_b.0).div_ceil(8)
                );
            }
            if partition_count >= 2 {
                debug!(
                    "  Partition C (AC系数): 位 [{}, {}) = {} 字节",
                    part_info.partition_c.0,
                    part_info.partition_c.1,
                    (part_info.partition_c.1 - part_info.partition_c.0).div_ceil(8)
                );
            }

            if reversible_vlc {
                debug!("  RVLC 可逆编码已启用 (Partition B 使用 RVLC)");
            }

            // === 使用 Data Partitioning 解码 ===
            // 先解析 VOP header 以确定帧类型
            let vop_offset = find_start_code_offset(&packet.data, START_CODE_VOP)
                .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
            let mut reader = BitReader::new(&packet.data[vop_offset..]);
            let vop_info = self.parse_vop_header(&mut reader)?;

            if !vop_info.vop_coded {
                if let Some(ref_frame) = &self.reference_frame {
                    let mut frame = ref_frame.clone();
                    frame.picture_type = vop_info.picture_type;
                    frame.is_keyframe = vop_info.picture_type == PictureType::I;
                    frame.pts = packet.pts;
                    frame.time_base = packet.time_base;
                    frame.duration = packet.duration;
                    self.pending_frame = Some(frame);
                    self.frame_count += 1;
                }
                return Ok(());
            }

            // Data Partitioning 仅支持 I/P 帧
            let mut frame = match vop_info.picture_type {
                PictureType::I => {
                    debug!("  使用 Data Partitioning 解码 I 帧");
                    self.decode_frame_partitioned(&packet.data, &part_info, true)?
                }
                PictureType::P => {
                    debug!("  使用 Data Partitioning 解码 P 帧");
                    self.decode_frame_partitioned(&packet.data, &part_info, false)?
                }
                PictureType::B => {
                    warn!("  B 帧不支持 Data Partitioning, 使用标准解码");
                    return self.send_packet_standard(packet);
                }
                _ => {
                    warn!("  未知帧类型, 使用标准解码");
                    return self.send_packet_standard(packet);
                }
            };

            // 设置帧元数据
            frame.picture_type = vop_info.picture_type;
            frame.is_keyframe = vop_info.picture_type == PictureType::I;
            frame.pts = packet.pts;
            frame.time_base = packet.time_base;
            frame.duration = packet.duration;

            // 更新参考帧
            if vop_info.picture_type == PictureType::I || vop_info.picture_type == PictureType::P {
                self.reference_frame = Some(frame.clone());
            }

            self.pending_frame = Some(frame);
            self.frame_count += 1;

            return Ok(());
        }

        // === 标准解码路径 ===
        self.send_packet_standard(packet)
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }

        // 从 DPB 中取出帧（按 FIFO 顺序）
        if !self.dpb.is_empty() {
            let frame = self.dpb.remove(0);
            return Ok(Frame::Video(frame));
        }

        // 如果还有 pending_frame，也返回（兼容旧逻辑）
        if let Some(frame) = self.pending_frame.take() {
            Ok(Frame::Video(frame))
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        debug!("MPEG4 解码器已刷新，输出 DPB 中剩余 {} 帧", self.dpb.len());
        // flush 时，DPB 中的帧会在 receive_frame 中逐个返回
        // 不需要在这里清空
        self.pending_frame = None;
    }
}

// ============================================================================
// 私有辅助方法
// ============================================================================

impl Mpeg4Decoder {
    /// 查找数据中所有 VOP 起始码的偏移 (起始码之后的位置)
    ///
    /// 用于 DivX packed bitstream 拆分: 一个 packet 中可能包含多个 VOP.
    /// 返回每个 VOP 起始码 (00 00 01 B6) 之后的字节偏移列表.
    fn find_all_vop_offsets(data: &[u8]) -> Vec<usize> {
        let mut offsets = Vec::new();
        if data.len() < 4 {
            return offsets;
        }
        for idx in 0..(data.len() - 3) {
            if data[idx] == 0x00
                && data[idx + 1] == 0x00
                && data[idx + 2] == 0x01
                && data[idx + 3] == START_CODE_VOP
            {
                offsets.push(idx + 4);
            }
        }
        offsets
    }

    /// 标准解码路径 (非 Data Partitioning)
    fn send_packet_standard(&mut self, packet: &Packet) -> TaoResult<()> {
        let vop_offset = find_start_code_offset(&packet.data, START_CODE_VOP)
            .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
        let mut reader = BitReader::new(&packet.data[vop_offset..]);

        let vop_info = self.parse_vop_header(&mut reader)?;

        if !vop_info.vop_coded {
            if let Some(ref_frame) = &self.reference_frame {
                let mut frame = ref_frame.clone();
                frame.picture_type = vop_info.picture_type;
                frame.is_keyframe = vop_info.picture_type == PictureType::I;
                frame.pts = packet.pts;
                frame.time_base = packet.time_base;
                frame.duration = packet.duration;
                self.pending_frame = Some(frame);
                self.frame_count += 1;
            }
            return Ok(());
        }

        // S-VOP: 解析 GMC sprite trajectory
        if vop_info.is_sprite {
            let sprite_enable = self.vol_info.as_ref().map(|v| v.sprite_enable).unwrap_or(0);
            if sprite_enable == 2 {
                self.gmc_params = self.parse_sprite_trajectory(&mut reader);
            }
        }

        let mut frame = match vop_info.picture_type {
            PictureType::I => self.decode_i_frame(&mut reader)?,
            PictureType::P => self.decode_p_frame(&mut reader).unwrap_or_else(|_| {
                warn!("P 帧解码失败, 使用参考帧降级");
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::P;
                    f.is_keyframe = false;
                    f
                } else {
                    self.create_blank_frame(PictureType::P)
                }
            }),
            PictureType::B => {
                // B 帧需要两个参考帧
                if self.reference_frame.is_some() && self.backward_reference.is_some() {
                    self.decode_b_frame(&mut reader).unwrap_or_else(|e| {
                        warn!("B 帧解码失败: {:?}, 使用参考帧降级", e);
                        if let Some(ref_frame) = &self.reference_frame {
                            let mut f = ref_frame.clone();
                            f.picture_type = PictureType::B;
                            f.is_keyframe = false;
                            f
                        } else {
                            self.create_blank_frame(PictureType::B)
                        }
                    })
                } else if let Some(ref_frame) = &self.reference_frame {
                    warn!("B 帧缺少双参考帧, 使用前向参考帧降级");
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::B;
                    f.is_keyframe = false;
                    f
                } else {
                    warn!("B 帧缺少参考帧, 跳过");
                    return Ok(());
                }
            }
            PictureType::S => self.decode_p_frame(&mut reader).unwrap_or_else(|_| {
                warn!("S-VOP 解码失败, 使用参考帧降级");
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::S;
                    f.is_keyframe = false;
                    f
                } else {
                    self.create_blank_frame(PictureType::S)
                }
            }),
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "不支持的 VOP 类型: {:?}",
                    vop_info.picture_type
                )));
            }
        };

        if vop_info.picture_type == PictureType::S {
            frame.picture_type = PictureType::S;
            frame.is_keyframe = false;
        }

        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        // B 帧重排序逻辑
        if frame.picture_type == PictureType::B {
            // B 帧存入 DPB，等待下一个 I/P/S 帧时输出
            self.dpb.push(frame);
        } else {
            // I/P/S 帧：先输出 DPB 中的所有 B 帧，再输出当前帧
            // 将当前 I/P/S 帧也加入 DPB
            self.dpb.push(frame.clone());

            // I/P/S 帧: 更新参考帧和 MV 缓存
            if frame.picture_type == PictureType::I
                || frame.picture_type == PictureType::P
                || frame.picture_type == PictureType::S
            {
                // 保存当前 MV 缓存和宏块信息到参考帧缓存 (B 帧 Direct 模式需要)
                self.ref_mv_cache = self.mv_cache.clone();
                self.backward_reference = self.reference_frame.take();
                self.reference_frame = Some(frame);
            }
        }

        self.frame_count += 1;
        Ok(())
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::VideoCodecParams;
    use tao_core::Rational;

    /// 创建测试用解码器实例
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
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
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
            gmc_params: GmcParameters::default(),
            packed_frames: std::collections::VecDeque::new(),
        }
    }

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

    #[test]
    fn test_dc_scaler() {
        let decoder = test_decoder(0, 0);
        assert_eq!(decoder.get_dc_scaler(true), 8);
        assert_eq!(decoder.get_dc_scaler(false), 8);
    }

    #[test]
    fn test_cbpy_inter_inversion() {
        let data = [0xB0]; // 1011 0000
        let mut reader = BitReader::new(&data);
        let cbpy_intra = decode_cbpy(&mut reader, true);
        assert_eq!(cbpy_intra, Some(15));

        let mut reader2 = BitReader::new(&data);
        let cbpy_inter = decode_cbpy(&mut reader2, false);
        assert_eq!(cbpy_inter, Some(0));
    }

    #[test]
    fn test_mv_range_wrapping() {
        let decoder = test_decoder(320, 240);
        let pmv = decoder.get_pmv(0, 0, 0);
        assert_eq!(pmv.x, 0);
        assert_eq!(pmv.y, 0);
    }

    #[test]
    fn test_integer_idct() {
        let mut block = [0i32; 64];
        idct_8x8(&mut block);
        for &v in &block {
            assert_eq!(v, 0);
        }

        let mut block2 = [0i32; 64];
        block2[0] = 100;
        idct_8x8(&mut block2);
        let first = block2[0];
        for &v in &block2 {
            assert!(
                (v - first).abs() <= 1,
                "DC-only block 不均匀: {} vs {}",
                v,
                first
            );
        }
    }

    #[test]
    fn test_b_frame_modb_decode() {
        use super::vlc::{decode_b_mb_type, decode_dbquant, decode_modb};

        // MODB = "1" -> no mb_type, no cbp
        let data = [0x80]; // 1000 0000
        let mut reader = BitReader::new(&data);
        let (has_type, has_cbp) = decode_modb(&mut reader);
        assert!(!has_type);
        assert!(!has_cbp);

        // MODB = "01" -> has mb_type, no cbp
        let data = [0x40]; // 0100 0000
        let mut reader = BitReader::new(&data);
        let (has_type, has_cbp) = decode_modb(&mut reader);
        assert!(has_type);
        assert!(!has_cbp);

        // MODB = "00" -> has both
        let data = [0x00]; // 0000 0000
        let mut reader = BitReader::new(&data);
        let (has_type, has_cbp) = decode_modb(&mut reader);
        assert!(has_type);
        assert!(has_cbp);

        // B MB type: "1" -> Direct
        let data = [0x80];
        let mut reader = BitReader::new(&data);
        assert_eq!(decode_b_mb_type(&mut reader), BframeMbMode::Direct);

        // B MB type: "01" -> Interpolate
        let data = [0x40];
        let mut reader = BitReader::new(&data);
        assert_eq!(decode_b_mb_type(&mut reader), BframeMbMode::Interpolate);

        // DBQUANT: "0" -> 0
        let data = [0x00];
        let mut reader = BitReader::new(&data);
        assert_eq!(decode_dbquant(&mut reader), 0);

        // DBQUANT: "10" -> -2
        let data = [0x80];
        let mut reader = BitReader::new(&data);
        assert_eq!(decode_dbquant(&mut reader), -2);

        // DBQUANT: "11" -> +2
        let data = [0xC0];
        let mut reader = BitReader::new(&data);
        assert_eq!(decode_dbquant(&mut reader), 2);
    }

    #[test]
    fn test_direct_mode_mv_computation() {
        let mut decoder = test_decoder(320, 240);
        decoder.time_pp = 3;
        decoder.time_bp = 1;

        // 设置共定位 MV
        let co_mv = MotionVector { x: 6, y: 9 };
        decoder.ref_mv_cache[0] = [co_mv; 4];

        let delta_mv = MotionVector::default();
        let (fwd, bwd) = decoder.compute_direct_mvs(0, delta_mv);

        // forward = TRB/TRD * co_mv = 1/3 * (6,9) = (2, 3)
        assert_eq!(fwd[0].x, 2);
        assert_eq!(fwd[0].y, 3);

        // backward = (TRB-TRD)/TRD * co_mv = -2/3 * (6,9) = (-4, -6)
        assert_eq!(bwd[0].x, -4);
        assert_eq!(bwd[0].y, -6);
    }

    #[test]
    fn test_qpel_mc_full_pixel() {
        // 全像素位置 (dx=0, dy=0): 应直接返回参考像素
        let mut ref_frame = VideoFrame::new(16, 16, PixelFormat::Yuv420p);
        ref_frame.data[0] = vec![0u8; 16 * 16];
        ref_frame.data[1] = vec![128u8; 8 * 8];
        ref_frame.data[2] = vec![128u8; 8 * 8];
        ref_frame.linesize[0] = 16;
        ref_frame.linesize[1] = 8;
        ref_frame.linesize[2] = 8;
        ref_frame.data[0][5 * 16 + 5] = 200;

        // MV = (0, 0) in qpel units
        let val = Mpeg4Decoder::qpel_motion_compensation(&ref_frame, 0, 5, 5, 0, 0, 0);
        assert_eq!(val, 200);

        // MV = (4, 0) in qpel units = 1 full pixel right
        let val = Mpeg4Decoder::qpel_motion_compensation(&ref_frame, 0, 4, 5, 4, 0, 0);
        assert_eq!(val, 200);
    }

    #[test]
    fn test_macroblock_info_modes() {
        assert_eq!(MacroblockInfo::MODE_INTER, 0);
        assert_eq!(MacroblockInfo::MODE_INTRA, 1);
        assert_eq!(MacroblockInfo::MODE_INTER4V, 2);
        assert_eq!(MacroblockInfo::MODE_NOT_CODED, 5);
    }

    #[test]
    fn test_resync_marker_check() {
        // 字节对齐位置, 17 个零 + 1 个一 = 0x00 0x00 0x01 (3 bytes = 24 bits)
        // 但 fcode=1, marker_len = 16+1 = 17 bits
        // 需要 17 bits: 0000_0000_0000_0000_1 = 17 位

        // 已对齐, peek 17 bits = 1
        let data = [0x00, 0x00, 0x80]; // 17 位: 0000_0000_0000_0000_1
        let reader = BitReader::new(&data);
        assert!(Mpeg4Decoder::check_resync_marker(&reader, 1));

        // 非 resync marker
        let data = [0x00, 0x01, 0x00];
        let reader = BitReader::new(&data);
        assert!(!Mpeg4Decoder::check_resync_marker(&reader, 1));
    }
}
