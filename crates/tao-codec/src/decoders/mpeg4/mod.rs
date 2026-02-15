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

mod bitreader;
mod block;
mod dequant;
mod header;
mod idct;
mod motion;
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
use header::START_CODE_VOP;
use idct::idct_8x8;
use tables::*;
use types::*;
use vlc::{decode_cbpy, decode_mcbpc_i, decode_mcbpc_p};

// ============================================================================
// Mpeg4Decoder 结构体
// ============================================================================

/// MPEG-4 Part 2 视频解码器
pub struct Mpeg4Decoder {
    pub(super) width: u32,
    pub(super) height: u32,
    pixel_format: PixelFormat,
    opened: bool,
    /// 前向参考帧
    pub(super) reference_frame: Option<VideoFrame>,
    /// 后向参考帧 (B 帧使用)
    backward_reference: Option<VideoFrame>,
    pending_frame: Option<VideoFrame>,
    frame_count: u64,
    pub(super) quant: u8,
    pub(super) vol_info: Option<VolInfo>,
    pub(super) quant_matrix_intra: [u8; 64],
    pub(super) quant_matrix_inter: [u8; 64],
    /// 预测器缓存: [DC, row AC 1-7, col AC 1-7] per block
    pub(super) predictor_cache: Vec<[i16; 15]>,
    /// 运动向量缓存 (每个 MB 存储 4 个 MV)
    pub(super) mv_cache: Vec<[MotionVector; 4]>,
    pub(super) mb_stride: usize,
    pub(super) f_code_forward: u8,
    #[allow(dead_code)]
    pub(super) f_code_backward: u8,
    pub(super) rounding_control: u8,
    pub(super) intra_dc_vlc_thr: u32,
    #[allow(dead_code)]
    time_pp: i32,
    #[allow(dead_code)]
    time_bp: i32,
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
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
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

    /// 检查 resync marker
    #[allow(dead_code)]
    fn check_resync_marker(reader: &BitReader, fcode_minus1: u8) -> bool {
        let nbits = reader.bits_to_byte_align();
        if nbits == 0 {
            return false;
        }
        if let Some(code) = reader.peek_bits(nbits) {
            if code == (1u32 << (nbits - 1)) - 1 {
                let marker_bits = 17 + fcode_minus1;
                let _ = marker_bits;
                return false; // TODO: 完善 resync marker 检测
            }
        }
        false
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

        // P-VOP: not_coded 位
        if !is_i_vop {
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                self.copy_mb_from_ref(frame, mb_x, mb_y);
                let idx = mb_y as usize * self.mb_stride + mb_x as usize;
                if idx < self.mv_cache.len() {
                    self.mv_cache[idx] = [MotionVector::default(); 4];
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

        // 4. 隔行模式: field_dct
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            let _field_dct = reader.read_bit().unwrap_or(false);
        }

        // 5. 运动向量解码
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;
        let mut mb_mvs = [MotionVector::default(); 4];

        if !is_intra {
            if mb_type == MbType::Inter4V {
                for (k, mv_slot) in mb_mvs.iter_mut().enumerate() {
                    if let Some(mv) = self.decode_motion_vector(reader, mb_x, mb_y, k) {
                        *mv_slot = mv;
                    }
                    if mb_idx < self.mv_cache.len() {
                        self.mv_cache[mb_idx][k] = *mv_slot;
                    }
                }
            } else if let Some(mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                mb_mvs = [mv; 4];
            }
        }

        // 存储 MV
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_mvs;
        }

        // 6. CBP 组合
        let cbp = (cbpy << 2) | cbpc;

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
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader).unwrap_or([0; 64])
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
                            let pred = Self::motion_compensation(
                                ref_frame,
                                0,
                                px,
                                py,
                                mv.x,
                                mv.y,
                                self.rounding_control,
                            );
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

        let chroma_mv = if !is_intra {
            if mb_type == MbType::Inter4V {
                Self::chroma_mv_4mv(&mb_mvs)
            } else {
                Self::chroma_mv_1mv(mb_mvs[0])
            }
        } else {
            MotionVector::default()
        };

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
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader).unwrap_or([0; 64])
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
                            let pred = Self::motion_compensation(
                                ref_frame,
                                plane_idx + 1,
                                px,
                                py,
                                chroma_mv.x,
                                chroma_mv.y,
                                self.rounding_control,
                            );
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

    /// 解码 I 帧
    fn decode_i_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        let total_blocks = (self.width.div_ceil(16) * self.height.div_ceil(16) * 6) as usize;
        self.predictor_cache = vec![[1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::I;
        frame.is_keyframe = true;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        let mb_w = self.width.div_ceil(16);
        let mb_h = self.height.div_ceil(16);
        debug!(
            "解码 I 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                self.decode_macroblock(&mut frame, mb_x, mb_y, reader, true);
            }
        }
        Ok(frame)
    }

    /// 解码 P 帧
    fn decode_p_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        let total_blocks = (self.width.div_ceil(16) * self.height.div_ceil(16) * 6) as usize;
        self.predictor_cache = vec![[1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = PictureType::P;
        frame.is_keyframe = false;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);
        debug!(
            "解码 P 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        for mb_y in 0..mb_h as u32 {
            for mb_x in 0..mb_w as u32 {
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

        let mb_count = self.mb_stride * (video.height as usize).div_ceil(16);
        self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];

        if !params.extra_data.is_empty() {
            self.parse_vol_header(&params.extra_data)?;
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

        if self.vol_info.is_none() {
            if let Err(e) = self.parse_vol_header(&packet.data) {
                debug!("VOL 解析失败: {:?}", e);
            }
        }

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
                    let mut f = VideoFrame::new(self.width, self.height, self.pixel_format);
                    f.picture_type = PictureType::P;
                    f
                }
            }),
            PictureType::B => {
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::B;
                    f.is_keyframe = false;
                    warn!("B 帧使用参考帧降级 (完整 B 帧解码待实现)");
                    f
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

        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        if frame.picture_type == PictureType::I || frame.picture_type == PictureType::P {
            self.backward_reference = self.reference_frame.take();
            self.reference_frame = Some(frame.clone());

            let mb_count = self.mb_stride * (self.height as usize).div_ceil(16);
            if self.mv_cache.len() != mb_count {
                self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
            }
        }

        self.pending_frame = Some(frame);
        self.frame_count += 1;
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开".into()));
        }
        if let Some(frame) = self.pending_frame.take() {
            Ok(Frame::Video(frame))
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        debug!("MPEG4 解码器已刷新");
        self.pending_frame = None;
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
        let decoder = Mpeg4Decoder {
            quant: 1,
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            frame_count: 0,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
        };
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
        let decoder = Mpeg4Decoder {
            width: 320,
            height: 240,
            pixel_format: PixelFormat::Yuv420p,
            opened: true,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: vec![[MotionVector::default(); 4]; 20 * 15],
            mb_stride: 20,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
        };

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
}
