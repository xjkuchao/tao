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
mod frame_decode;
mod gmc;
mod header;
mod idct;
mod motion;
mod packet_io;
mod partitioned;
mod short_header;
mod tables;
#[cfg(test)]
mod tests;
mod types;
mod vlc;
use log::{debug, trace, warn};
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
    /// VOP 级 alternate_vertical_scan 标志
    alternate_vertical_scan: bool,
    /// DivX packed bitstream 队列 (一个 packet 中拆分出的多个 VOP)
    packed_frames: std::collections::VecDeque<Vec<u8>>,
    /// Seek/flush 后等待关键帧, 丢弃非 I 帧避免花屏
    wait_keyframe: bool,
    /// 当前 video packet (slice) 起始宏块 X 坐标
    resync_mb_x: usize,
    /// 当前 video packet (slice) 起始宏块 Y 坐标
    resync_mb_y: usize,
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
            alternate_vertical_scan: false,
            packed_frames: std::collections::VecDeque::new(),
            wait_keyframe: false,
            resync_mb_x: 0,
            resync_mb_y: 0,
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

    /// 默认 DC 预测值 (量化域)
    ///
    /// 对应 FFmpeg 的 1024 (未缩放域) / dc_scaler.
    /// 对于 8 位精度: 1024 / 8 = 128 (quant 1-4).
    const DC_PRED_DEFAULT: i16 = 128;

    /// 检查指定宏块是否在当前 video packet (slice) 范围内
    ///
    /// 通过比较线性宏块索引与当前 slice 起始位置判断.
    pub(super) fn is_in_current_slice(&self, mb_x: usize, mb_y: usize) -> bool {
        let nb_linear = mb_y * self.mb_stride + mb_x;
        let resync_linear = self.resync_mb_y * self.mb_stride + self.resync_mb_x;
        nb_linear >= resync_linear
    }

    /// 获取 Intra DC 预测方向和预测值
    ///
    /// 参考 FFmpeg mpeg4_pred_dc: 在 slice 边界处将不可用邻居
    /// 重置为默认值, 保证 video packet 间的独立解码能力.
    pub(super) fn get_intra_predictor(
        &self,
        mb_x: usize,
        mb_y: usize,
        block_idx: usize,
    ) -> (i16, PredictorDirection) {
        let def = Self::DC_PRED_DEFAULT;
        let get_dc = |x: isize, y: isize, idx: usize| -> i16 {
            self.get_neighbor_block_idx(x, y, idx)
                .and_then(|pos| self.predictor_cache.get(pos))
                .map(|b| b[0])
                .unwrap_or(def)
        };

        // A = 左, B = 左上, C = 上
        let (mut dc_a, mut dc_b, mut dc_c) = match block_idx {
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
            _ => (def, def, def),
        };

        // Slice 边界处理 (对标 FFmpeg mpeg4_pred_dc):
        // 在 video packet 的首行, 上方和左上邻居属于前一个 packet, 不可用.
        let first_slice_line = mb_y == self.resync_mb_y;
        if first_slice_line && block_idx != 3 {
            // block 3 的上方/左上是同一 MB 内的 block 0/1, 始终可用
            if block_idx != 2 {
                // block 0, 1, 4, 5: 上方和左上来自上一行 MB
                dc_b = def;
                dc_c = def;
            }
            if block_idx != 1 && mb_x == self.resync_mb_x {
                // slice 首列首行: 左邻居也不可用
                dc_b = def;
                dc_a = def;
            }
        }
        // slice 第二行首列: 左上角邻居 (上一行的前一列) 不可用
        if mb_x == self.resync_mb_x
            && mb_y == self.resync_mb_y + 1
            && (block_idx == 0 || block_idx == 4 || block_idx == 5)
        {
            dc_b = def;
        }

        let grad_hor = (dc_a - dc_b).abs();
        let grad_ver = (dc_c - dc_b).abs();

        if grad_hor < grad_ver {
            (dc_c, PredictorDirection::Vertical)
        } else {
            (dc_a, PredictorDirection::Horizontal)
        }
    }
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

        // 如果宽度/高度为 0, 尝试从 extra_data 中解析
        let (width, height) = if video.width == 0 || video.height == 0 {
            if params.extra_data.is_empty() {
                return Err(TaoError::InvalidArgument(
                    "宽度和高度为 0 且没有 extra_data 可解析".into(),
                ));
            }
            debug!(
                "宽度/高度为 0, 从 {} 字节 extra_data 中解析 VOL",
                params.extra_data.len()
            );
            // 解析 VOL header 提取宽度和高度
            self.parse_vol_header(&params.extra_data)?;
            debug!("VOL 解析后尺寸: {}x{}", self.width, self.height);

            // 对于没有 VOL header 的损坏流, 尝试从首个 VOP 中提取尺寸
            if self.width == 0 || self.height == 0 {
                debug!("VOL 未提供尺寸, 尝试从 VOP header 中提取");
                // 查找 VOP start code  (0x000001B6)
                for i in 0..params.extra_data.len().saturating_sub(3) {
                    if params.extra_data[i] == 0x00
                        && params.extra_data[i + 1] == 0x00
                        && params.extra_data[i + 2] == 0x01
                        && params.extra_data[i + 3] == 0xB6
                    {
                        // 先设定默认值 QCIF (176x144)
                        self.width = 176;
                        self.height = 144;
                        debug!("未找到 VOL, 使用默认 QCIF 尺寸: 176x144");
                        break;
                    }
                }
            }

            if self.width == 0 || self.height == 0 {
                return Err(TaoError::InvalidArgument(format!(
                    "从 extra_data 中未能解析出有效的宽度和高度 (解析结果: {}x{})",
                    self.width, self.height
                )));
            }
            (self.width, self.height)
        } else {
            (video.width, video.height)
        };

        self.width = width;
        self.height = height;
        self.mb_stride = (width as usize).div_ceil(16);
        self.pixel_format = PixelFormat::Yuv420p;
        self.opened = true;
        self.frame_count = 0;
        self.reference_frame = None;
        self.backward_reference = None;
        self.dpb.clear();

        let mb_count = self.mb_stride * (height as usize).div_ceil(16);
        self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        self.ref_mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        self.mb_info = vec![MacroblockInfo::default(); mb_count];
        self.last_time_base = 0;
        self.time_base_acc = 0;
        self.last_non_b_time = 0;
        self.alternate_vertical_scan = false;

        if !params.extra_data.is_empty() {
            // 如果之前未解析过 VOL, 现在解析
            if self.vol_info.is_none() {
                self.parse_vol_header(&params.extra_data)?;
            }
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
            trace!(
                "检测到 Short Video Header (H.263), TR={}, 使用 H.263 解码路径",
                header.temporal_reference
            );
            // Seek 后等待关键帧: 丢弃非 I 帧避免花屏
            if self.wait_keyframe {
                if header.picture_type == PictureType::I {
                    self.wait_keyframe = false;
                } else {
                    return Ok(());
                }
            }

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
                trace!(
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
            trace!("数据分区模式已启用:");
            trace!("  分区数量: {}", partition_count + 1);
            trace!(
                "  Partition A (MB类型/量化/运动向量): 位 [{}, {}) = {} 字节",
                part_info.partition_a.0,
                part_info.partition_a.1,
                (part_info.partition_a.1 - part_info.partition_a.0).div_ceil(8)
            );
            if partition_count >= 1 {
                trace!(
                    "  Partition B (DC系数/RVLC): 位 [{}, {}) = {} 字节",
                    part_info.partition_b.0,
                    part_info.partition_b.1,
                    (part_info.partition_b.1 - part_info.partition_b.0).div_ceil(8)
                );
            }
            if partition_count >= 2 {
                trace!(
                    "  Partition C (AC系数): 位 [{}, {}) = {} 字节",
                    part_info.partition_c.0,
                    part_info.partition_c.1,
                    (part_info.partition_c.1 - part_info.partition_c.0).div_ceil(8)
                );
            }

            if reversible_vlc {
                trace!("  RVLC 可逆编码已启用 (Partition B 使用 RVLC)");
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
                    trace!("  使用 Data Partitioning 解码 I 帧");
                    self.decode_frame_partitioned(&packet.data, &part_info, true)?
                }
                PictureType::P => {
                    trace!("  使用 Data Partitioning 解码 P 帧");
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
        debug!("MPEG4 解码器已刷新, 清空参考帧和 DPB");
        self.dpb.clear();
        self.pending_frame = None;
        self.reference_frame = None;
        self.backward_reference = None;
        self.packed_frames.clear();
        self.wait_keyframe = true;
    }
}
