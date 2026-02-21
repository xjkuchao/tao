//! H.264/AVC 视频解码器.
//!
//! 实现 CABAC 熵解码, I_16x16/I_4x4 帧内预测和残差解码.
//! P/B 帧使用 P_Skip (复制参考帧).

mod cabac;
mod cabac_init_ext;
mod cabac_init_pb;
mod common;
mod config;
mod deblock;
mod intra;
mod macroblock_inter;
mod macroblock_intra;
mod macroblock_state;
mod output;
mod parameter_sets;
mod residual;
mod slice_decode;
mod syntax;
#[cfg(test)]
mod tests;

use common::*;
use std::collections::{HashMap, VecDeque};
use syntax::*;

use log::{debug, warn};
use tao_core::bitreader::BitReader;
use tao_core::{PixelFormat, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;
use crate::parsers::h264::{
    NalUnit, NalUnitType, Sps, parse_avcc_config, parse_sps, split_annex_b, split_avcc,
};

use cabac::{CabacCtx, CabacDecoder, init_contexts_i_slice, init_contexts_pb_slice};
use residual::{
    CAT_CHROMA_AC, CAT_CHROMA_DC, CAT_LUMA_8X8, CAT_LUMA_AC, CAT_LUMA_DC, decode_residual_block,
    dequant_chroma_dc, dequant_luma_dc, inverse_hadamard_2x2, inverse_hadamard_4x4,
};

// ============================================================
// PPS 参数
// ============================================================

/// PPS 解析结果
#[derive(Clone)]
struct Pps {
    pps_id: u32,
    sps_id: u32,
    entropy_coding_mode: u8,
    pic_init_qp: i32,
    chroma_qp_index_offset: i32,
    second_chroma_qp_index_offset: i32,
    deblocking_filter_control: bool,
    pic_order_present: bool,
    num_ref_idx_l0_default_active: u32,
    num_ref_idx_l1_default_active: u32,
    weighted_pred: bool,
    weighted_bipred_idc: u32,
    redundant_pic_cnt_present: bool,
    transform_8x8_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParameterSetRebuildAction {
    None,
    RuntimeOnly,
    Full,
}

// ============================================================
// Slice Header
// ============================================================

/// 解析后的 slice header
struct SliceHeader {
    first_mb: u32,
    pps_id: u32,
    slice_type: u32,
    frame_num: u32,
    slice_qp: i32,
    cabac_init_idc: u8,
    num_ref_idx_l0: u32,
    num_ref_idx_l1: u32,
    ref_pic_list_mod_l0: Vec<RefPicListMod>,
    ref_pic_list_mod_l1: Vec<RefPicListMod>,
    luma_log2_weight_denom: u8,
    chroma_log2_weight_denom: u8,
    l0_weights: Vec<PredWeightL0>,
    l1_weights: Vec<PredWeightL0>,
    data_bit_offset: usize,
    cabac_start_byte: usize,
    nal_ref_idc: u8,
    is_idr: bool,
    pic_order_cnt_lsb: Option<u32>,
    delta_poc_bottom: i32,
    delta_poc_0: i32,
    delta_poc_1: i32,
    disable_deblocking_filter_idc: u32,
    slice_alpha_c0_offset_div2: i32,
    slice_beta_offset_div2: i32,
    dec_ref_pic_marking: DecRefPicMarking,
}

#[derive(Clone, Default)]
struct DecRefPicMarking {
    is_idr: bool,
    no_output_of_prior_pics: bool,
    long_term_reference_flag: bool,
    adaptive: bool,
    ops: Vec<MmcoOp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefPicListMod {
    ShortTermSub { abs_diff_pic_num_minus1: u32 },
    ShortTermAdd { abs_diff_pic_num_minus1: u32 },
    LongTerm { long_term_pic_num: u32 },
}

#[derive(Clone, Copy)]
enum MmcoOp {
    ForgetShort {
        difference_of_pic_nums_minus1: u32,
    },
    ForgetLong {
        long_term_pic_num: u32,
    },
    ConvertShortToLong {
        difference_of_pic_nums_minus1: u32,
        long_term_frame_idx: u32,
    },
    TrimLong {
        max_long_term_frame_idx_plus1: u32,
    },
    ClearAll,
    MarkCurrentLong {
        long_term_frame_idx: u32,
    },
}

#[derive(Clone, Copy)]
struct PredWeightL0 {
    luma_weight: i32,
    luma_offset: i32,
    chroma_weight: [i32; 2],
    chroma_offset: [i32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BPredDir {
    Direct,
    L0,
    L1,
    Bi,
}

enum BMbType {
    Intra,
    Direct,
    Inter(u8),
}

#[derive(Clone)]
struct ReferencePicture {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    frame_num: u32,
    poc: i32,
    long_term_frame_idx: Option<u32>,
}

#[derive(Clone, Copy)]
struct BMotion {
    mv_x: i32,
    mv_y: i32,
    ref_idx: i8,
}

#[derive(Clone)]
struct RefPlanes {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    poc: i32,
}

struct ReorderFrameEntry {
    frame: VideoFrame,
    poc: i32,
    decode_order: u64,
}

#[derive(Clone, Copy)]
struct PendingFrameMeta {
    pts: i64,
    time_base: Rational,
    is_keyframe: bool,
}

// ============================================================
// H.264 解码器
// ============================================================

/// H.264 解码器
pub struct H264Decoder {
    sps: Option<Sps>,
    pps: Option<Pps>,
    sps_map: HashMap<u32, Sps>,
    pps_map: HashMap<u32, Pps>,
    active_sps_id: Option<u32>,
    active_pps_id: Option<u32>,
    length_size: usize,
    width: u32,
    height: u32,
    mb_width: usize,
    mb_height: usize,
    ref_y: Vec<u8>,
    ref_u: Vec<u8>,
    ref_v: Vec<u8>,
    stride_y: usize,
    stride_c: usize,
    /// 每个宏块的类型 (0=I_4x4, 1-24=I_16x16, 25=I_PCM, 255=P_Skip)
    mb_types: Vec<u8>,
    /// 每个宏块的 coded_block_pattern (低 4 位为 luma, 高 2 位为 chroma)
    mb_cbp: Vec<u8>,
    /// CABAC 上下文用的 cbp 状态位 (对齐 FFmpeg cbp_table 关键位):
    /// 低 6 位: luma/chroma cbp, bit6: chroma U DC, bit7: chroma V DC, bit8: luma DC.
    mb_cbp_ctx: Vec<u16>,
    /// 每个宏块的色度预测模式 (0-3)
    chroma_pred_modes: Vec<u8>,
    /// 每个宏块的 transform_size_8x8_flag (0/1)
    transform_8x8_flags: Vec<u8>,
    /// Luma CBF 追踪 (4x4 块粒度, 用于 CABAC 上下文)
    cbf_luma: Vec<bool>,
    /// Luma CBF 追踪 (8x8 块粒度, 用于 8x8 transform CABAC 上下文)
    cbf_luma_8x8: Vec<bool>,
    /// Chroma U CBF 追踪 (4x4 块粒度, 用于 CABAC 上下文)
    cbf_chroma_u: Vec<bool>,
    /// Chroma V CBF 追踪 (4x4 块粒度, 用于 CABAC 上下文)
    cbf_chroma_v: Vec<bool>,
    /// Luma DC CBF 追踪 (宏块粒度, 用于 CABAC 上下文)
    cbf_luma_dc: Vec<bool>,
    /// Chroma U DC CBF 追踪 (宏块粒度, 用于 CABAC 上下文)
    cbf_chroma_dc_u: Vec<bool>,
    /// Chroma V DC CBF 追踪 (宏块粒度, 用于 CABAC 上下文)
    cbf_chroma_dc_v: Vec<bool>,
    /// I_4x4 预测模式缓存 (4x4 块粒度)
    i4x4_modes: Vec<u8>,
    /// 上一个宏块的 qp_delta 是否非零
    prev_qp_delta_nz: bool,
    /// 每个宏块 list0 运动向量 X (1/4 像素单位)
    mv_l0_x: Vec<i16>,
    /// 每个宏块 list0 运动向量 Y (1/4 像素单位)
    mv_l0_y: Vec<i16>,
    /// 每个宏块 list0 参考索引 (-1 表示不可用)
    ref_idx_l0: Vec<i8>,
    /// 最近一次成功解析的 slice_type
    last_slice_type: u32,
    /// 最近一次成功解析的 frame_num.
    last_frame_num: u32,
    /// 最近一次 slice 的 nal_ref_idc.
    last_nal_ref_idc: u8,
    /// 最近一次 slice 的 POC.
    last_poc: i32,
    /// 最近一次 slice 的量化参数.
    last_slice_qp: i32,
    /// 最近一次 slice 的去块滤波控制.
    last_disable_deblocking_filter_idc: u32,
    /// 最近一次 slice 的去块滤波 alpha 偏移.
    last_slice_alpha_c0_offset_div2: i32,
    /// 最近一次 slice 的去块滤波 beta 偏移.
    last_slice_beta_offset_div2: i32,
    /// 最近一次参考帧的 POC MSB(type0).
    prev_ref_poc_msb: i32,
    /// 最近一次参考帧的 POC LSB(type0).
    prev_ref_poc_lsb: i32,
    /// POC type1 用的 frame_num_offset.
    prev_frame_num_offset_type1: i32,
    /// POC type2 用的 frame_num_offset.
    prev_frame_num_offset_type2: i32,
    /// 最近一次 slice 解析得到的 dec_ref_pic_marking.
    last_dec_ref_pic_marking: DecRefPicMarking,
    /// 最小 DPB: 短期参考帧队列(按解码顺序).
    reference_frames: VecDeque<ReferencePicture>,
    /// 长期参考帧索引上限, `None` 表示无长期参考帧.
    max_long_term_frame_idx: Option<u32>,
    /// DPB 最大短期参考帧数量.
    max_reference_frames: usize,
    /// 参考帧缺失回退次数(用于容错统计与单测验证).
    missing_reference_fallbacks: u64,
    output_queue: VecDeque<Frame>,
    reorder_buffer: Vec<ReorderFrameEntry>,
    reorder_depth: usize,
    decode_order_counter: u64,
    pending_frame: Option<PendingFrameMeta>,
    opened: bool,
    flushing: bool,
}

impl H264Decoder {
    /// 创建解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sps: None,
            pps: None,
            sps_map: HashMap::new(),
            pps_map: HashMap::new(),
            active_sps_id: None,
            active_pps_id: None,
            length_size: 4,
            width: 0,
            height: 0,
            mb_width: 0,
            mb_height: 0,
            ref_y: Vec::new(),
            ref_u: Vec::new(),
            ref_v: Vec::new(),
            stride_y: 0,
            stride_c: 0,
            mb_types: Vec::new(),
            mb_cbp: Vec::new(),
            mb_cbp_ctx: Vec::new(),
            chroma_pred_modes: Vec::new(),
            transform_8x8_flags: Vec::new(),
            cbf_luma: Vec::new(),
            cbf_luma_8x8: Vec::new(),
            cbf_chroma_u: Vec::new(),
            cbf_chroma_v: Vec::new(),
            cbf_luma_dc: Vec::new(),
            cbf_chroma_dc_u: Vec::new(),
            cbf_chroma_dc_v: Vec::new(),
            i4x4_modes: Vec::new(),
            prev_qp_delta_nz: false,
            mv_l0_x: Vec::new(),
            mv_l0_y: Vec::new(),
            ref_idx_l0: Vec::new(),
            last_slice_type: 0,
            last_frame_num: 0,
            last_nal_ref_idc: 0,
            last_poc: 0,
            last_slice_qp: 26,
            last_disable_deblocking_filter_idc: 0,
            last_slice_alpha_c0_offset_div2: 0,
            last_slice_beta_offset_div2: 0,
            prev_ref_poc_msb: 0,
            prev_ref_poc_lsb: 0,
            prev_frame_num_offset_type1: 0,
            prev_frame_num_offset_type2: 0,
            last_dec_ref_pic_marking: DecRefPicMarking::default(),
            reference_frames: VecDeque::new(),
            max_long_term_frame_idx: None,
            max_reference_frames: 1,
            missing_reference_fallbacks: 0,
            output_queue: VecDeque::new(),
            reorder_buffer: Vec::new(),
            reorder_depth: 2,
            decode_order_counter: 0,
            pending_frame: None,
            opened: false,
            flushing: false,
        }))
    }

    /// 初始化/重新分配帧缓冲
    fn init_buffers(&mut self) {
        self.mb_width = self.width.div_ceil(16) as usize;
        self.mb_height = self.height.div_ceil(16) as usize;
        self.stride_y = self.mb_width * 16;
        self.stride_c = self.mb_width * 8;
        let total_mb = self.mb_width * self.mb_height;
        self.ref_y = vec![128u8; self.stride_y * self.mb_height * 16];
        self.ref_u = vec![128u8; self.stride_c * self.mb_height * 8];
        self.ref_v = vec![128u8; self.stride_c * self.mb_height * 8];
        self.mb_types = vec![0u8; total_mb];
        self.mb_cbp = vec![0u8; total_mb];
        self.mb_cbp_ctx = vec![0u16; total_mb];
        self.chroma_pred_modes = vec![0u8; total_mb];
        self.transform_8x8_flags = vec![0u8; total_mb];
        self.cbf_luma = vec![false; self.mb_width * 4 * self.mb_height * 4];
        self.cbf_luma_8x8 = vec![false; self.mb_width * 2 * self.mb_height * 2];
        self.cbf_chroma_u = vec![false; self.mb_width * 2 * self.mb_height * 2];
        self.cbf_chroma_v = vec![false; self.mb_width * 2 * self.mb_height * 2];
        self.cbf_luma_dc = vec![false; total_mb];
        self.cbf_chroma_dc_u = vec![false; total_mb];
        self.cbf_chroma_dc_v = vec![false; total_mb];
        self.i4x4_modes = vec![2u8; self.mb_width * 4 * self.mb_height * 4];
        self.mv_l0_x = vec![0i16; total_mb];
        self.mv_l0_y = vec![0i16; total_mb];
        self.ref_idx_l0 = vec![-1i8; total_mb];
    }

    /// 处理 SPS NAL 单元
    fn handle_sps(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();
        match parse_sps(&rbsp) {
            Ok(sps) => {
                if let Err(err) = Self::validate_sps_support(&sps) {
                    warn!("H264: 忽略不支持的 SPS, sps_id={}, err={}", sps.sps_id, err);
                    return;
                }
                debug!(
                    "H264: SPS {}x{} profile={} level={}",
                    sps.width, sps.height, sps.profile_idc, sps.level_idc
                );
                let sps_id = sps.sps_id;
                self.sps_map.insert(sps_id, sps.clone());
                if self.active_sps_id.map(|id| id == sps_id).unwrap_or(true) {
                    self.activate_sps(sps_id);
                }
            }
            Err(err) => {
                warn!("H264: SPS 解析失败, err={}", err);
            }
        }
    }

    /// 处理 PPS NAL 单元
    fn handle_pps(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();
        if let Ok(pps) = parameter_sets::parse_pps(&rbsp) {
            debug!(
                "H264: PPS id={} sps={} entropy={} qp={}",
                pps.pps_id,
                pps.sps_id,
                if pps.entropy_coding_mode == 1 {
                    "CABAC"
                } else {
                    "CAVLC"
                },
                pps.pic_init_qp
            );
            let pps_id = pps.pps_id;
            self.pps_map.insert(pps_id, pps);
            if self.active_pps_id.map(|id| id == pps_id).unwrap_or(true) {
                let _ = self.activate_parameter_sets(pps_id);
            }
        }
    }

    /// 重置参考帧缓冲为中性值
    fn reset_reference_planes(&mut self) {
        self.ref_y.fill(128);
        self.ref_u.fill(128);
        self.ref_v.fill(128);
        self.reference_frames.clear();
        self.max_long_term_frame_idx = None;
    }

    /// 提交当前待输出帧, 用于跨包多 slice 拼装.
    fn finalize_pending_frame(&mut self) {
        if let Some(meta) = self.pending_frame.take() {
            self.build_output_frame(meta.pts, meta.time_base, meta.is_keyframe);
        }
    }

    /// 仅解析 first_mb_in_slice, 用于判断是否进入新帧.
    fn parse_slice_first_mb(&self, nalu: &NalUnit) -> Option<u32> {
        let rbsp = nalu.rbsp();
        let mut br = BitReader::new(&rbsp);
        read_ue(&mut br).ok()
    }

    /// 重置宏块级语法与运动缓存.
    fn reset_mb_runtime_state(&mut self) {
        self.mb_types.fill(0);
        self.mb_cbp.fill(0);
        self.mb_cbp_ctx.fill(0);
        self.chroma_pred_modes.fill(0);
        self.transform_8x8_flags.fill(0);
        self.cbf_luma.fill(false);
        self.cbf_luma_8x8.fill(false);
        self.cbf_chroma_u.fill(false);
        self.cbf_chroma_v.fill(false);
        self.cbf_luma_dc.fill(false);
        self.cbf_chroma_dc_u.fill(false);
        self.cbf_chroma_dc_v.fill(false);
        self.i4x4_modes.fill(2);
        self.mv_l0_x.fill(0);
        self.mv_l0_y.fill(0);
        self.ref_idx_l0.fill(-1);
        self.prev_qp_delta_nz = false;
    }

    fn activate_sps(&mut self, sps_id: u32) {
        let Some(sps) = self.sps_map.get(&sps_id).cloned() else {
            return;
        };
        if let Err(err) = Self::validate_sps_support(&sps) {
            warn!("H264: 忽略不支持的 SPS, sps_id={}, err={}", sps_id, err);
            return;
        }
        let sps_changed = self.active_sps_id != Some(sps_id);
        let size_changed = self.width != sps.width || self.height != sps.height;
        self.max_reference_frames = sps.max_num_ref_frames.clamp(1, 16) as usize;
        self.width = sps.width;
        self.height = sps.height;
        self.active_sps_id = Some(sps_id);
        self.sps = Some(sps);
        if size_changed && self.width > 0 && self.height > 0 {
            self.init_buffers();
            self.reset_reference_planes();
            self.reorder_buffer.clear();
            self.decode_order_counter = 0;
        } else if sps_changed {
            self.reset_reference_planes();
            self.reset_mb_runtime_state();
            self.reorder_buffer.clear();
            self.decode_order_counter = 0;
        }
        while self.reference_frames.len() > self.max_reference_frames {
            let _ = self.reference_frames.pop_front();
        }
    }

    fn validate_sps_support(sps: &Sps) -> TaoResult<()> {
        if sps.chroma_format_idc != 1 {
            return Err(TaoError::NotImplemented(format!(
                "H264: 暂不支持 chroma_format_idc={}, 仅支持 4:2:0",
                sps.chroma_format_idc
            )));
        }
        if !sps.frame_mbs_only {
            return Err(TaoError::NotImplemented(
                "H264: 暂不支持场编码(frame_mbs_only_flag=0)".into(),
            ));
        }
        if sps.bit_depth_luma != 8 || sps.bit_depth_chroma != 8 {
            return Err(TaoError::NotImplemented(format!(
                "H264: 暂不支持高位深, bit_depth_luma={}, bit_depth_chroma={}, 仅支持 8-bit",
                sps.bit_depth_luma, sps.bit_depth_chroma
            )));
        }
        Ok(())
    }

    fn activate_parameter_sets(&mut self, pps_id: u32) -> TaoResult<()> {
        let prev_pps = self.pps.clone();
        let pps = self
            .pps_map
            .get(&pps_id)
            .cloned()
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 PPS id={}", pps_id)))?;
        self.activate_sps(pps.sps_id);
        if self.active_sps_id != Some(pps.sps_id) {
            return Err(TaoError::NotImplemented(format!(
                "H264: PPS id={} 依赖的 SPS id={} 当前不受支持",
                pps_id, pps.sps_id
            )));
        }

        let rebuild_action = prev_pps
            .as_ref()
            .map(|old| Self::pps_rebuild_action(old, &pps))
            .unwrap_or(ParameterSetRebuildAction::None);
        match rebuild_action {
            ParameterSetRebuildAction::None => {}
            ParameterSetRebuildAction::RuntimeOnly => {
                self.reset_mb_runtime_state();
            }
            ParameterSetRebuildAction::Full => {
                self.reset_mb_runtime_state();
                self.reset_reference_planes();
                self.reorder_buffer.clear();
                self.decode_order_counter = 0;
            }
        }

        self.active_pps_id = Some(pps_id);
        self.pps = Some(pps);
        Ok(())
    }

    fn pps_rebuild_action(old: &Pps, new: &Pps) -> ParameterSetRebuildAction {
        let need_full = old.sps_id != new.sps_id
            || old.entropy_coding_mode != new.entropy_coding_mode
            || old.transform_8x8_mode != new.transform_8x8_mode;
        if need_full {
            return ParameterSetRebuildAction::Full;
        }

        let need_runtime_only = old.pic_init_qp != new.pic_init_qp
            || old.chroma_qp_index_offset != new.chroma_qp_index_offset
            || old.second_chroma_qp_index_offset != new.second_chroma_qp_index_offset
            || old.deblocking_filter_control != new.deblocking_filter_control
            || old.pic_order_present != new.pic_order_present
            || old.num_ref_idx_l0_default_active != new.num_ref_idx_l0_default_active
            || old.num_ref_idx_l1_default_active != new.num_ref_idx_l1_default_active
            || old.weighted_pred != new.weighted_pred
            || old.weighted_bipred_idc != new.weighted_bipred_idc
            || old.redundant_pic_cnt_present != new.redundant_pic_cnt_present;
        if need_runtime_only {
            ParameterSetRebuildAction::RuntimeOnly
        } else {
            ParameterSetRebuildAction::None
        }
    }
}

// ============================================================
// Decoder trait 实现
// ============================================================

impl Decoder for H264Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::H264
    }

    fn name(&self) -> &str {
        "h264"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        self.sps_map.clear();
        self.pps_map.clear();
        self.sps = None;
        self.pps = None;
        self.active_sps_id = None;
        self.active_pps_id = None;
        self.reference_frames.clear();
        self.last_slice_type = 0;
        self.last_frame_num = 0;
        self.last_nal_ref_idc = 0;
        self.last_poc = 0;
        self.last_slice_qp = 26;
        self.last_disable_deblocking_filter_idc = 0;
        self.last_slice_alpha_c0_offset_div2 = 0;
        self.last_slice_beta_offset_div2 = 0;
        self.prev_ref_poc_msb = 0;
        self.prev_ref_poc_lsb = 0;
        self.prev_frame_num_offset_type1 = 0;
        self.prev_frame_num_offset_type2 = 0;
        self.last_dec_ref_pic_marking = DecRefPicMarking::default();

        if !params.extra_data.is_empty() {
            let config = parse_avcc_config(&params.extra_data)?;
            self.length_size = config.length_size;
            self.parse_sps_pps_from_config(&config)?;
        }
        if self.width == 0 || self.height == 0 {
            if let CodecParamsType::Video(ref v) = params.params {
                self.width = v.width;
                self.height = v.height;
            }
        }
        if self.width == 0 || self.height == 0 {
            return Err(TaoError::InvalidData("H264: 无法确定帧尺寸".into()));
        }
        self.reorder_depth = std::env::var("TAO_H264_REORDER_DEPTH")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);
        self.init_buffers();
        self.output_queue.clear();
        self.reorder_buffer.clear();
        self.decode_order_counter = 0;
        self.pending_frame = None;
        self.opened = true;
        debug!("H264 解码器已打开: {}x{}", self.width, self.height);
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::InvalidData("H264 解码器未打开".into()));
        }
        if packet.is_empty() {
            self.flushing = true;
            self.finalize_pending_frame();
            self.drain_reorder_buffer_to_output();
            return Ok(());
        }
        let mut nalus = split_avcc(&packet.data, self.length_size);
        if nalus.is_empty() {
            nalus = split_annex_b(&packet.data);
        }
        let debug_packet = std::env::var("TAO_H264_DEBUG_PACKET")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if debug_packet {
            eprintln!(
                "[H264][Packet] pts={}, dts={}, size={}, nalus={}",
                packet.pts,
                packet.dts,
                packet.data.len(),
                nalus.len()
            );
            for (i, nalu) in nalus.iter().enumerate() {
                eprintln!(
                    "[H264][Packet]   nalu[{}]: type={:?}, ref_idc={}, rbsp_len={}",
                    i,
                    nalu.nal_type,
                    nalu.ref_idc,
                    nalu.rbsp().len()
                );
            }
        }
        let mut idr_reset_done = false;

        for nalu in &nalus {
            match nalu.nal_type {
                NalUnitType::Sps => self.handle_sps(nalu),
                NalUnitType::Pps => self.handle_pps(nalu),
                NalUnitType::SliceIdr | NalUnitType::Slice => {
                    let is_idr = nalu.nal_type == NalUnitType::SliceIdr;
                    let first_mb = self.parse_slice_first_mb(nalu);
                    let start_new_picture = first_mb == Some(0);

                    if start_new_picture && self.pending_frame.is_some() {
                        self.finalize_pending_frame();
                    }
                    if start_new_picture {
                        // 宏块级语法缓存是逐图像状态, 新图像开始时必须重置.
                        // 否则会把上一帧的 CBF/预测模式/运动缓存污染到当前帧.
                        self.reset_mb_runtime_state();
                    }
                    if is_idr && !idr_reset_done {
                        self.finalize_pending_frame();
                        self.drain_reorder_buffer_to_output();
                        self.reset_reference_planes();
                        idr_reset_done = true;
                    }
                    self.decode_slice(nalu);
                    if self.pending_frame.is_none() {
                        self.pending_frame = Some(PendingFrameMeta {
                            pts: packet.pts,
                            time_base: packet.time_base,
                            is_keyframe: is_idr,
                        });
                    } else if is_idr && let Some(meta) = self.pending_frame.as_mut() {
                        meta.is_keyframe = true;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_queue.pop_front() {
            Ok(frame)
        } else if self.flushing {
            Err(TaoError::Eof)
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        self.output_queue.clear();
        self.reorder_buffer.clear();
        self.decode_order_counter = 0;
        self.pending_frame = None;
        self.flushing = false;
        self.last_slice_type = 0;
        self.last_frame_num = 0;
        self.last_nal_ref_idc = 0;
        self.last_poc = 0;
        self.last_slice_qp = 26;
        self.last_disable_deblocking_filter_idc = 0;
        self.last_slice_alpha_c0_offset_div2 = 0;
        self.last_slice_beta_offset_div2 = 0;
        self.prev_ref_poc_msb = 0;
        self.prev_ref_poc_lsb = 0;
        self.prev_frame_num_offset_type1 = 0;
        self.prev_frame_num_offset_type2 = 0;
        self.last_dec_ref_pic_marking = DecRefPicMarking::default();
        self.reset_reference_planes();
        self.mb_types.fill(0);
        self.mb_cbp.fill(0);
        self.mb_cbp_ctx.fill(0);
        self.chroma_pred_modes.fill(0);
        self.transform_8x8_flags.fill(0);
        self.cbf_luma.fill(false);
        self.cbf_luma_8x8.fill(false);
        self.cbf_chroma_u.fill(false);
        self.cbf_chroma_v.fill(false);
        self.cbf_luma_dc.fill(false);
        self.cbf_chroma_dc_u.fill(false);
        self.cbf_chroma_dc_v.fill(false);
        self.i4x4_modes.fill(2);
        self.mv_l0_x.fill(0);
        self.mv_l0_y.fill(0);
        self.ref_idx_l0.fill(-1);
    }
}
