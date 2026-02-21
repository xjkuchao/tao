//! H.264/AVC 视频解码器.
//!
//! 实现 CABAC 熵解码, I_16x16/I_4x4 帧内预测和残差解码.
//! P/B 帧使用 P_Skip (复制参考帧).

mod cabac;
mod cabac_init_ext;
mod cabac_init_pb;
mod deblock;
mod intra;
mod parameter_sets;
mod residual;

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;

use log::debug;
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
    CAT_CHROMA_AC, CAT_CHROMA_DC, CAT_LUMA_8X8, CAT_LUMA_8X8_FALLBACK, CAT_LUMA_AC, CAT_LUMA_DC,
    decode_residual_block, dequant_chroma_dc, dequant_luma_dc, inverse_hadamard_2x2,
    inverse_hadamard_4x4,
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
    data_bit_offset: usize,
    cabac_start_byte: usize,
    nal_ref_idc: u8,
    is_idr: bool,
    pic_order_cnt_lsb: Option<u32>,
    delta_poc_bottom: i32,
    delta_poc_0: i32,
    delta_poc_1: i32,
    disable_deblocking_filter_idc: u32,
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

static EMPTY_REF_PLANES: LazyLock<RefPlanes> = LazyLock::new(|| RefPlanes {
    y: Vec::new(),
    u: Vec::new(),
    v: Vec::new(),
    poc: 0,
});

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
    /// 最近一次 slice 的去块滤波控制.
    last_disable_deblocking_filter_idc: u32,
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
            last_disable_deblocking_filter_idc: 0,
            prev_ref_poc_msb: 0,
            prev_ref_poc_lsb: 0,
            prev_frame_num_offset_type1: 0,
            prev_frame_num_offset_type2: 0,
            last_dec_ref_pic_marking: DecRefPicMarking::default(),
            reference_frames: VecDeque::new(),
            max_long_term_frame_idx: None,
            max_reference_frames: 1,
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
        if let Ok(sps) = parse_sps(&rbsp) {
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

    fn activate_parameter_sets(&mut self, pps_id: u32) -> TaoResult<()> {
        let prev_pps = self.pps.clone();
        let pps = self
            .pps_map
            .get(&pps_id)
            .cloned()
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 PPS id={}", pps_id)))?;
        self.activate_sps(pps.sps_id);

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
        self.last_disable_deblocking_filter_idc = 0;
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
        self.last_disable_deblocking_filter_idc = 0;
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

// ============================================================
// Slice 解码
// ============================================================

impl H264Decoder {
    /// 解码一个 VCL NAL (slice)
    fn decode_slice(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();

        if let Ok(header) = self.parse_slice_header(&rbsp, nalu) {
            let prev_frame_num = self.last_frame_num;
            self.last_slice_type = header.slice_type;
            self.last_nal_ref_idc = header.nal_ref_idc;
            self.last_disable_deblocking_filter_idc = header.disable_deblocking_filter_idc;
            self.last_poc = self.compute_slice_poc(&header, prev_frame_num);
            self.last_frame_num = header.frame_num;
            self.last_dec_ref_pic_marking = header.dec_ref_pic_marking.clone();
            self.decode_slice_data(&rbsp, &header);
        }
    }

    /// 解析 slice header, 返回 CABAC 数据起始位置
    fn parse_slice_header(&self, rbsp: &[u8], nalu: &NalUnit) -> TaoResult<SliceHeader> {
        let mut br = BitReader::new(rbsp);

        let first_mb = read_ue(&mut br)?;
        let slice_type = read_ue(&mut br)? % 5;
        let pps_id = read_ue(&mut br)?;
        let pps = self
            .pps_map
            .get(&pps_id)
            .or({
                if self.pps_map.is_empty() {
                    self.pps.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 PPS id={}", pps_id)))?;
        let sps = self
            .sps_map
            .get(&pps.sps_id)
            .or({
                if self.sps_map.is_empty() {
                    self.sps.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| TaoError::InvalidData(format!("H264: 未找到 SPS id={}", pps.sps_id)))?;

        // frame_num
        let frame_num = br.read_bits(sps.log2_max_frame_num)?;

        let mut field_pic = false;
        if !sps.frame_mbs_only {
            field_pic = br.read_bit()? == 1;
            if field_pic {
                let _bottom_field_flag = br.read_bit()?;
            }
        }

        // IDR 特有字段
        if nalu.nal_type == NalUnitType::SliceIdr {
            let _idr_pic_id = read_ue(&mut br)?;
        }

        // pic_order_cnt
        let mut pic_order_cnt_lsb = None;
        let mut delta_poc_bottom = 0i32;
        let mut delta_poc_0 = 0i32;
        let mut delta_poc_1 = 0i32;
        if sps.poc_type == 0 {
            let poc_lsb = br.read_bits(sps.log2_max_poc_lsb)?;
            pic_order_cnt_lsb = Some(poc_lsb);
            if pps.pic_order_present && !field_pic {
                delta_poc_bottom = read_se(&mut br)?;
            }
        } else if sps.poc_type == 1 && !sps.delta_pic_order_always_zero_flag {
            delta_poc_0 = read_se(&mut br)?;
            if pps.pic_order_present && !field_pic {
                delta_poc_1 = read_se(&mut br)?;
            }
        }

        // 参考索引数量
        if pps.redundant_pic_cnt_present {
            let _redundant_pic_cnt = read_ue(&mut br)?;
        }
        let mut num_ref_idx_l0 = pps.num_ref_idx_l0_default_active;
        let mut num_ref_idx_l1 = pps.num_ref_idx_l1_default_active;

        let is_b = slice_type == 1;
        let is_i = slice_type == 2 || slice_type == 4;
        if !is_i {
            if is_b {
                let _direct_spatial_mv_pred_flag = br.read_bit()?;
            }
            let override_refs = br.read_bit()? == 1;
            if override_refs {
                num_ref_idx_l0 = read_ue(&mut br)? + 1;
                if is_b {
                    num_ref_idx_l1 = read_ue(&mut br)? + 1;
                }
            }
            if num_ref_idx_l0 == 0 || num_ref_idx_l0 > 32 {
                return Err(TaoError::InvalidData(format!(
                    "H264: num_ref_idx_l0_active_minus1 非法, value={}",
                    num_ref_idx_l0.saturating_sub(1)
                )));
            }
            if is_b && (num_ref_idx_l1 == 0 || num_ref_idx_l1 > 32) {
                return Err(TaoError::InvalidData(format!(
                    "H264: num_ref_idx_l1_active_minus1 非法, value={}",
                    num_ref_idx_l1.saturating_sub(1)
                )));
            }
        }

        let (ref_pic_list_mod_l0, ref_pic_list_mod_l1) =
            self.parse_ref_pic_list_mod(&mut br, slice_type, num_ref_idx_l0, num_ref_idx_l1)?;
        let (luma_log2_weight_denom, chroma_log2_weight_denom, l0_weights) = self
            .parse_pred_weight_table(
                &mut br,
                sps,
                pps,
                slice_type,
                num_ref_idx_l0,
                num_ref_idx_l1,
            )?;
        let dec_ref_pic_marking = self.parse_dec_ref_pic_marking(&mut br, nalu)?;

        // CABAC init
        let mut cabac_init_idc = 0u8;
        if pps.entropy_coding_mode == 1 && !is_i {
            let cabac_init_idc_raw = read_ue(&mut br)?;
            if cabac_init_idc_raw > 2 {
                return Err(TaoError::InvalidData(format!(
                    "H264: cabac_init_idc 非法, value={}",
                    cabac_init_idc_raw
                )));
            }
            cabac_init_idc = cabac_init_idc_raw as u8;
        }

        // slice_qp_delta
        let qp_delta = read_se(&mut br)?;
        let slice_qp = pps.pic_init_qp + qp_delta;
        if !(0..=51).contains(&slice_qp) {
            return Err(TaoError::InvalidData(format!(
                "H264: slice_qp 超出范围, slice_qp={}",
                slice_qp
            )));
        }

        // 跳过去块效应滤波器参数
        let mut disable_deblocking_filter_idc = 0u32;
        if pps.deblocking_filter_control {
            let disable = read_ue(&mut br)?;
            if disable > 2 {
                return Err(TaoError::InvalidData(format!(
                    "H264: disable_deblocking_filter_idc 非法, value={}",
                    disable
                )));
            }
            disable_deblocking_filter_idc = disable;
            if disable != 1 {
                let _alpha = read_se(&mut br)?;
                let _beta = read_se(&mut br)?;
            }
        }

        let mut data_bit_offset = br.bits_read();
        if pps.entropy_coding_mode == 1 {
            while br.bits_read() & 7 != 0 {
                let _cabac_alignment_one_bit = br.read_bit()?;
            }
            data_bit_offset = br.bits_read();
        }
        let cabac_start = br.byte_position();

        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if debug_mb {
            eprintln!(
                "[H264][SliceHeader] first_mb={}, pps_id={}, slice_type={}, frame_num={}, slice_qp={}, cabac_init_idc={}, cabac_start_byte={}, idr={}",
                first_mb,
                pps_id,
                slice_type,
                frame_num,
                slice_qp,
                cabac_init_idc,
                cabac_start,
                nalu.nal_type == NalUnitType::SliceIdr
            );
        }

        Ok(SliceHeader {
            first_mb,
            pps_id,
            slice_type,
            frame_num,
            slice_qp,
            cabac_init_idc,
            num_ref_idx_l0,
            num_ref_idx_l1,
            ref_pic_list_mod_l0,
            ref_pic_list_mod_l1,
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
            l0_weights,
            data_bit_offset,
            cabac_start_byte: cabac_start,
            nal_ref_idc: nalu.ref_idc,
            is_idr: nalu.nal_type == NalUnitType::SliceIdr,
            pic_order_cnt_lsb,
            delta_poc_bottom,
            delta_poc_0,
            delta_poc_1,
            disable_deblocking_filter_idc,
            dec_ref_pic_marking,
        })
    }

    fn compute_slice_poc(&mut self, header: &SliceHeader, prev_frame_num: u32) -> i32 {
        let Some(sps) = self.sps.as_ref() else {
            return header.frame_num as i32;
        };

        if header.is_idr {
            self.prev_ref_poc_msb = 0;
            self.prev_ref_poc_lsb = 0;
            self.prev_frame_num_offset_type1 = 0;
            self.prev_frame_num_offset_type2 = 0;
        }

        match sps.poc_type {
            0 => {
                let Some(poc_lsb_u32) = header.pic_order_cnt_lsb else {
                    return header.frame_num as i32;
                };
                let max_poc_lsb = 1i32 << sps.log2_max_poc_lsb.min(30);
                let poc_lsb = poc_lsb_u32 as i32;

                let mut poc_msb = self.prev_ref_poc_msb;
                if !header.is_idr {
                    if poc_lsb < self.prev_ref_poc_lsb
                        && (self.prev_ref_poc_lsb - poc_lsb) >= (max_poc_lsb / 2)
                    {
                        poc_msb += max_poc_lsb;
                    } else if poc_lsb > self.prev_ref_poc_lsb
                        && (poc_lsb - self.prev_ref_poc_lsb) > (max_poc_lsb / 2)
                    {
                        poc_msb -= max_poc_lsb;
                    }
                }

                let poc = poc_msb + poc_lsb + header.delta_poc_bottom;
                if header.nal_ref_idc != 0 {
                    self.prev_ref_poc_msb = poc_msb;
                    self.prev_ref_poc_lsb = poc_lsb;
                }
                poc
            }
            1 => {
                let max_frame_num = self.max_frame_num_modulo() as i32;
                if max_frame_num <= 0 {
                    return header.frame_num as i32;
                }
                let frame_num = header.frame_num as i32;
                let prev_num = prev_frame_num as i32;
                let mut frame_num_offset = if header.is_idr {
                    0
                } else {
                    self.prev_frame_num_offset_type1
                };
                if !header.is_idr && prev_num > frame_num {
                    frame_num_offset += max_frame_num;
                }

                let mut abs_frame_num = if sps.max_num_ref_frames == 0 {
                    0
                } else {
                    frame_num_offset + frame_num
                };
                if header.nal_ref_idc == 0 && abs_frame_num > 0 {
                    abs_frame_num -= 1;
                }

                let mut expected_poc = 0i32;
                if abs_frame_num > 0 && !sps.offset_for_ref_frame.is_empty() {
                    let cycle_len = sps.offset_for_ref_frame.len() as i32;
                    let expected_delta_per_cycle: i32 = sps.offset_for_ref_frame.iter().sum();
                    let pic_order_cnt_cycle_cnt = (abs_frame_num - 1) / cycle_len;
                    let frame_num_in_cycle = (abs_frame_num - 1) % cycle_len;
                    expected_poc = pic_order_cnt_cycle_cnt * expected_delta_per_cycle;
                    for i in 0..=frame_num_in_cycle {
                        expected_poc += sps.offset_for_ref_frame[i as usize];
                    }
                }
                if header.nal_ref_idc == 0 {
                    expected_poc += sps.offset_for_non_ref_pic;
                }

                let top = expected_poc + header.delta_poc_0;
                let bottom = top + sps.offset_for_top_to_bottom_field + header.delta_poc_1;
                if header.nal_ref_idc != 0 {
                    self.prev_frame_num_offset_type1 = frame_num_offset;
                }
                top.min(bottom)
            }
            2 => {
                let max_frame_num = self.max_frame_num_modulo() as i32;
                if max_frame_num <= 0 {
                    return header.frame_num as i32;
                }
                let mut frame_num_offset = if header.is_idr {
                    0
                } else {
                    self.prev_frame_num_offset_type2
                };
                let frame_num = header.frame_num as i32;
                let prev_num = prev_frame_num as i32;
                if !header.is_idr && prev_num > frame_num {
                    frame_num_offset += max_frame_num;
                }

                let mut poc = 2 * (frame_num_offset + frame_num);
                if header.nal_ref_idc == 0 {
                    poc -= 1;
                }
                self.prev_frame_num_offset_type2 = frame_num_offset;
                poc
            }
            _ => header.frame_num as i32,
        }
    }

    /// 解析参考图像列表修改语法
    fn parse_ref_pic_list_mod(
        &self,
        br: &mut BitReader,
        slice_type: u32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
    ) -> TaoResult<(Vec<RefPicListMod>, Vec<RefPicListMod>)> {
        let mut mods_l0 = Vec::new();
        let mut mods_l1 = Vec::new();
        if slice_type == 2 || slice_type == 4 {
            return Ok((mods_l0, mods_l1));
        }

        let reorder_l0 = br.read_bit()?;
        if reorder_l0 == 1 && num_ref_idx_l0 > 0 {
            mods_l0 = self.parse_single_ref_pic_list_mod(br)?;
        }

        if slice_type == 1 {
            let reorder_l1 = br.read_bit()?;
            if reorder_l1 == 1 && num_ref_idx_l1 > 0 {
                mods_l1 = self.parse_single_ref_pic_list_mod(br)?;
            }
        }
        Ok((mods_l0, mods_l1))
    }

    fn parse_single_ref_pic_list_mod(&self, br: &mut BitReader) -> TaoResult<Vec<RefPicListMod>> {
        let mut mods = Vec::new();
        loop {
            let op = read_ue(br)?;
            match op {
                0 => {
                    let abs_diff_pic_num_minus1 = read_ue(br)?;
                    mods.push(RefPicListMod::ShortTermSub {
                        abs_diff_pic_num_minus1,
                    });
                }
                1 => {
                    let abs_diff_pic_num_minus1 = read_ue(br)?;
                    mods.push(RefPicListMod::ShortTermAdd {
                        abs_diff_pic_num_minus1,
                    });
                }
                2 => {
                    let long_term_pic_num = read_ue(br)?;
                    mods.push(RefPicListMod::LongTerm { long_term_pic_num });
                }
                3 => break,
                _ => {
                    return Err(TaoError::InvalidData(format!(
                        "H264: ref_pic_list_modification_idc 非法, value={}",
                        op
                    )));
                }
            }
            if mods.len() > 96 {
                return Err(TaoError::InvalidData(
                    "H264: ref_pic_list_modification 项数过多".into(),
                ));
            }
        }
        Ok(mods)
    }

    /// 解析并(按需)返回 list0 加权预测参数.
    fn parse_pred_weight_table(
        &self,
        br: &mut BitReader,
        sps: &Sps,
        pps: &Pps,
        slice_type: u32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
    ) -> TaoResult<(u8, u8, Vec<PredWeightL0>)> {
        let use_weight_l0 = pps.weighted_pred && (slice_type == 0 || slice_type == 3);
        let use_weight_l1 = pps.weighted_bipred_idc == 1 && slice_type == 1;
        if !use_weight_l0 && !use_weight_l1 {
            return Ok((0, 0, Vec::new()));
        }

        let luma_log2_weight_denom = read_ue(br)?.min(7) as u8;
        let mut chroma_present = false;
        let mut chroma_log2_weight_denom = 0u8;
        if sps.chroma_format_idc != 0 {
            chroma_present = true;
            chroma_log2_weight_denom = read_ue(br)?.min(7) as u8;
        }

        let need_parse_l0 = use_weight_l0 || use_weight_l1;
        let mut l0_weights = Vec::new();
        if need_parse_l0 {
            for _ in 0..num_ref_idx_l0 {
                let mut w = PredWeightL0 {
                    luma_weight: 1 << luma_log2_weight_denom,
                    luma_offset: 0,
                    chroma_weight: [1 << chroma_log2_weight_denom; 2],
                    chroma_offset: [0, 0],
                };
                let luma_weight_flag = br.read_bit()?;
                if luma_weight_flag == 1 {
                    w.luma_weight = read_se(br)?;
                    w.luma_offset = read_se(br)?;
                }
                if chroma_present {
                    let chroma_weight_flag = br.read_bit()?;
                    if chroma_weight_flag == 1 {
                        for c in 0..2 {
                            w.chroma_weight[c] = read_se(br)?;
                            w.chroma_offset[c] = read_se(br)?;
                        }
                    }
                }
                if use_weight_l0 {
                    l0_weights.push(w);
                }
            }
        }
        if use_weight_l1 {
            for _ in 0..num_ref_idx_l1 {
                let luma_weight_flag = br.read_bit()?;
                if luma_weight_flag == 1 {
                    let _ = read_se(br)?;
                    let _ = read_se(br)?;
                }
                if chroma_present {
                    let chroma_weight_flag = br.read_bit()?;
                    if chroma_weight_flag == 1 {
                        for _ in 0..2 {
                            let _ = read_se(br)?;
                            let _ = read_se(br)?;
                        }
                    }
                }
            }
        }
        Ok((luma_log2_weight_denom, chroma_log2_weight_denom, l0_weights))
    }

    /// 解析 dec_ref_pic_marking 语法.
    fn parse_dec_ref_pic_marking(
        &self,
        br: &mut BitReader,
        nalu: &NalUnit,
    ) -> TaoResult<DecRefPicMarking> {
        let mut marking = DecRefPicMarking::default();
        if nalu.nal_type == NalUnitType::SliceIdr {
            marking.is_idr = true;
            marking.no_output_of_prior_pics = br.read_bit()? == 1;
            marking.long_term_reference_flag = br.read_bit()? == 1;
            return Ok(marking);
        }
        if nalu.ref_idc == 0 {
            return Ok(marking);
        }

        marking.adaptive = br.read_bit()? == 1;
        if !marking.adaptive {
            return Ok(marking);
        }

        loop {
            let op = read_ue(br)?;
            match op {
                0 => break,
                1 => {
                    let difference = read_ue(br)?;
                    marking.ops.push(MmcoOp::ForgetShort {
                        difference_of_pic_nums_minus1: difference,
                    });
                }
                2 => {
                    let long_term_pic_num = read_ue(br)?;
                    marking.ops.push(MmcoOp::ForgetLong { long_term_pic_num });
                }
                3 => {
                    let difference = read_ue(br)?;
                    let long_term_frame_idx = read_ue(br)?;
                    marking.ops.push(MmcoOp::ConvertShortToLong {
                        difference_of_pic_nums_minus1: difference,
                        long_term_frame_idx,
                    });
                }
                4 => {
                    let max_long_term_frame_idx_plus1 = read_ue(br)?;
                    marking.ops.push(MmcoOp::TrimLong {
                        max_long_term_frame_idx_plus1,
                    });
                }
                5 => marking.ops.push(MmcoOp::ClearAll),
                6 => {
                    let long_term_frame_idx = read_ue(br)?;
                    marking.ops.push(MmcoOp::MarkCurrentLong {
                        long_term_frame_idx,
                    });
                }
                _ => {
                    return Err(TaoError::InvalidData(format!(
                        "H264: MMCO op 非法, op={}",
                        op
                    )));
                }
            }
        }
        Ok(marking)
    }

    /// 解码 slice 数据 (MB 循环)
    fn decode_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        if self.activate_parameter_sets(header.pps_id).is_err() {
            return;
        }
        let entropy_coding_mode = match &self.pps {
            Some(p) => p.entropy_coding_mode,
            None => return,
        };

        if entropy_coding_mode != 1 {
            self.decode_cavlc_slice_data(rbsp, header);
            return;
        }

        let cabac_start_byte = header.cabac_start_byte;
        if cabac_start_byte >= rbsp.len() {
            return;
        }

        let cabac_data = &rbsp[cabac_start_byte..];
        let mut cabac = CabacDecoder::new(cabac_data);

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        let mut ctxs = if is_i {
            init_contexts_i_slice(header.slice_qp)
        } else {
            init_contexts_pb_slice(header.slice_qp, header.cabac_init_idc)
        };
        let _num_ref_idx_l1 = header.num_ref_idx_l1;

        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;

        if is_i {
            self.decode_i_slice_mbs(&mut cabac, &mut ctxs, first, total_mbs, header.slice_qp);
            return;
        }

        if header.slice_type == 0 || header.slice_type == 3 {
            let ref_l0_list = self.build_reference_list_l0_with_mod(
                header.num_ref_idx_l0,
                &header.ref_pic_list_mod_l0,
                header.frame_num,
            );
            self.decode_p_slice_mbs(
                &mut cabac,
                &mut ctxs,
                first,
                total_mbs,
                header.slice_qp,
                header.num_ref_idx_l0,
                &header.l0_weights,
                header.luma_log2_weight_denom,
                header.chroma_log2_weight_denom,
                &ref_l0_list,
            );
            return;
        }

        let ref_l0_list = self.build_reference_list_l0_with_mod(
            header.num_ref_idx_l0,
            &header.ref_pic_list_mod_l0,
            header.frame_num,
        );
        let ref_l1_list = self.build_reference_list_l1_with_mod(
            header.num_ref_idx_l1,
            &header.ref_pic_list_mod_l1,
            header.frame_num,
        );
        self.decode_b_slice_mbs(
            &mut cabac,
            &mut ctxs,
            first,
            total_mbs,
            header.slice_qp,
            header.num_ref_idx_l0,
            header.num_ref_idx_l1,
            &ref_l0_list,
            &ref_l1_list,
        );
    }

    /// CAVLC 回退: 对所有 MB 使用 DC 预测
    fn apply_dc_fallback(&mut self) {
        for mb_y in 0..self.mb_height {
            for mb_x in 0..self.mb_width {
                intra::predict_16x16(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16,
                    mb_y * 16,
                    2,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_u,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_v,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
            }
        }
    }

    fn copy_macroblock_from_planes(&mut self, mb_x: usize, mb_y: usize, ref_src: &RefPlanes) {
        let y_base_x = mb_x * 16;
        let y_base_y = mb_y * 16;
        for y in 0..16usize {
            let dst_y = y_base_y + y;
            for x in 0..16usize {
                let dst_x = y_base_x + x;
                let dst_idx = dst_y * self.stride_y + dst_x;
                if dst_idx >= self.ref_y.len() {
                    continue;
                }
                let src_idx = dst_y * self.stride_y + dst_x;
                self.ref_y[dst_idx] = *ref_src.y.get(src_idx).unwrap_or(&128);
            }
        }

        let c_base_x = mb_x * 8;
        let c_base_y = mb_y * 8;
        for y in 0..8usize {
            let dst_y = c_base_y + y;
            for x in 0..8usize {
                let dst_x = c_base_x + x;
                let dst_idx = dst_y * self.stride_c + dst_x;
                if dst_idx >= self.ref_u.len() || dst_idx >= self.ref_v.len() {
                    continue;
                }
                self.ref_u[dst_idx] = *ref_src.u.get(dst_idx).unwrap_or(&128);
                self.ref_v[dst_idx] = *ref_src.v.get(dst_idx).unwrap_or(&128);
            }
        }
    }

    /// CAVLC 最小路径: 消费 `mb_skip_run/mb_type`, 并执行基础重建.
    fn decode_cavlc_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;
        if first >= total_mbs {
            return;
        }

        let mut br = BitReader::new(rbsp);
        if br.skip_bits(header.data_bit_offset as u32).is_err() {
            self.apply_dc_fallback();
            return;
        }

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        if is_i {
            for mb_idx in first..total_mbs {
                let _mb_type = read_ue(&mut br).unwrap_or(0);
                self.mb_types[mb_idx] = 1;
                self.mb_cbp[mb_idx] = 0;
                let mb_x = mb_idx % self.mb_width;
                let mb_y = mb_idx / self.mb_width;
                intra::predict_16x16(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16,
                    mb_y * 16,
                    2,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_u,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_v,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
            }
            return;
        }

        let ref_l0 = self
            .build_reference_list_l0_with_mod(
                header.num_ref_idx_l0,
                &header.ref_pic_list_mod_l0,
                header.frame_num,
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| self.zero_reference_planes());
        let mut skip_run_left = 0u32;
        for mb_idx in first..total_mbs {
            if skip_run_left == 0 {
                skip_run_left = read_ue(&mut br).unwrap_or(0);
            }
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            if skip_run_left > 0 {
                self.mb_types[mb_idx] = 255;
                self.mb_cbp[mb_idx] = 0;
                self.copy_macroblock_from_planes(mb_x, mb_y, &ref_l0);
                skip_run_left -= 1;
                continue;
            }
            let _mb_type = read_ue(&mut br).unwrap_or(0);
            self.mb_types[mb_idx] = 255;
            self.mb_cbp[mb_idx] = 0;
            self.copy_macroblock_from_planes(mb_x, mb_y, &ref_l0);
        }
    }
}

// ============================================================
// I-slice 宏块解码
// ============================================================

impl H264Decoder {
    fn cbf_stride(&self) -> usize {
        self.mb_width * 4
    }

    fn cbf_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.cbf_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    fn get_luma_cbf(&self, x4: usize, y4: usize) -> bool {
        self.cbf_index(x4, y4)
            .and_then(|idx| self.cbf_luma.get(idx).copied())
            .unwrap_or(false)
    }

    fn set_luma_cbf(&mut self, x4: usize, y4: usize, coded: bool) {
        if let Some(idx) = self.cbf_index(x4, y4)
            && let Some(slot) = self.cbf_luma.get_mut(idx)
        {
            *slot = coded;
        }
    }

    fn luma_cbf_ctx_inc(&self, x4: usize, y4: usize, intra_defaults: bool) -> usize {
        let left = if x4 > 0 {
            usize::from(self.get_luma_cbf(x4 - 1, y4))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if y4 > 0 {
            usize::from(self.get_luma_cbf(x4, y4 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn luma_8x8_cbf_stride(&self) -> usize {
        self.mb_width * 2
    }

    fn luma_8x8_cbf_index(&self, x8: usize, y8: usize) -> Option<usize> {
        let stride = self.luma_8x8_cbf_stride();
        if stride == 0 {
            return None;
        }
        let h8 = self.mb_height * 2;
        if x8 >= stride || y8 >= h8 {
            return None;
        }
        Some(y8 * stride + x8)
    }

    fn set_luma_8x8_cbf(&mut self, x8: usize, y8: usize, coded: bool) {
        if let Some(idx) = self.luma_8x8_cbf_index(x8, y8)
            && let Some(slot) = self.cbf_luma_8x8.get_mut(idx)
        {
            *slot = coded;
        }
    }

    fn get_luma_8x8_cbf(&self, x8: usize, y8: usize) -> bool {
        self.luma_8x8_cbf_index(x8, y8)
            .and_then(|idx| self.cbf_luma_8x8.get(idx).copied())
            .unwrap_or(false)
    }

    fn luma_8x8_cbf_ctx_inc(&self, x8: usize, y8: usize, intra_defaults: bool) -> usize {
        // 8x8 变换块 CBF 采用 8x8 邻居非零状态作为上下文增量.
        let left = if x8 > 0 {
            usize::from(self.get_luma_8x8_cbf(x8 - 1, y8))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if y8 > 0 {
            usize::from(self.get_luma_8x8_cbf(x8, y8 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn chroma_cbf_stride(&self) -> usize {
        self.mb_width * 2
    }

    fn chroma_cbf_index(&self, x2: usize, y2: usize) -> Option<usize> {
        let stride = self.chroma_cbf_stride();
        if stride == 0 {
            return None;
        }
        let h2 = self.mb_height * 2;
        if x2 >= stride || y2 >= h2 {
            return None;
        }
        Some(y2 * stride + x2)
    }

    fn get_chroma_u_cbf(&self, x2: usize, y2: usize) -> bool {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.cbf_chroma_u.get(idx).copied())
            .unwrap_or(false)
    }

    fn set_chroma_u_cbf(&mut self, x2: usize, y2: usize, coded: bool) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.cbf_chroma_u.get_mut(idx)
        {
            *slot = coded;
        }
    }

    fn get_chroma_v_cbf(&self, x2: usize, y2: usize) -> bool {
        self.chroma_cbf_index(x2, y2)
            .and_then(|idx| self.cbf_chroma_v.get(idx).copied())
            .unwrap_or(false)
    }

    fn set_chroma_v_cbf(&mut self, x2: usize, y2: usize, coded: bool) {
        if let Some(idx) = self.chroma_cbf_index(x2, y2)
            && let Some(slot) = self.cbf_chroma_v.get_mut(idx)
        {
            *slot = coded;
        }
    }

    fn chroma_u_cbf_ctx_inc(&self, x2: usize, y2: usize, intra_defaults: bool) -> usize {
        let left = if x2 > 0 {
            usize::from(self.get_chroma_u_cbf(x2 - 1, y2))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if y2 > 0 {
            usize::from(self.get_chroma_u_cbf(x2, y2 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn chroma_v_cbf_ctx_inc(&self, x2: usize, y2: usize, intra_defaults: bool) -> usize {
        let left = if x2 > 0 {
            usize::from(self.get_chroma_v_cbf(x2 - 1, y2))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if y2 > 0 {
            usize::from(self.get_chroma_v_cbf(x2, y2 - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn chroma_dc_cbf_ctx_inc(&self, mb_x: usize, mb_y: usize, intra_defaults: bool) -> usize {
        let left = if mb_x > 0 {
            usize::from(
                self.mb_index(mb_x - 1, mb_y)
                    .and_then(|idx| self.cbf_chroma_dc_u.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if mb_y > 0 {
            usize::from(
                self.mb_index(mb_x, mb_y - 1)
                    .and_then(|idx| self.cbf_chroma_dc_u.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn set_chroma_dc_u_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_chroma_dc_u.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 6;
                } else {
                    *slot &= !(1u16 << 6);
                }
            }
        }
    }

    fn set_luma_dc_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_luma_dc.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 8;
                } else {
                    *slot &= !(1u16 << 8);
                }
            }
        }
    }

    fn get_luma_dc_cbf(&self, mb_x: usize, mb_y: usize) -> bool {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.cbf_luma_dc.get(idx).copied())
            .unwrap_or(false)
    }

    fn chroma_dc_v_cbf_ctx_inc(&self, mb_x: usize, mb_y: usize, intra_defaults: bool) -> usize {
        let left = if mb_x > 0 {
            usize::from(
                self.mb_index(mb_x - 1, mb_y)
                    .and_then(|idx| self.cbf_chroma_dc_v.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if mb_y > 0 {
            usize::from(
                self.mb_index(mb_x, mb_y - 1)
                    .and_then(|idx| self.cbf_chroma_dc_v.get(idx).copied())
                    .unwrap_or(false),
            )
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }

    fn set_chroma_dc_v_cbf(&mut self, mb_x: usize, mb_y: usize, coded: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.cbf_chroma_dc_v.get_mut(idx) {
                *slot = coded;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                if coded {
                    *slot |= 1u16 << 7;
                } else {
                    *slot &= !(1u16 << 7);
                }
            }
        }
    }

    fn reset_chroma_cbf_mb(&mut self, mb_x: usize, mb_y: usize) {
        self.set_chroma_dc_u_cbf(mb_x, mb_y, false);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, false);
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, false);
                self.set_chroma_v_cbf(x2, y2, false);
            }
        }
    }

    fn reset_luma_8x8_cbf_mb(&mut self, mb_x: usize, mb_y: usize) {
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                self.set_luma_8x8_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, false);
            }
        }
    }

    fn i4x4_mode_stride(&self) -> usize {
        self.mb_width * 4
    }

    fn i4x4_mode_index(&self, x4: usize, y4: usize) -> Option<usize> {
        let stride = self.i4x4_mode_stride();
        if stride == 0 {
            return None;
        }
        let h4 = self.mb_height * 4;
        if x4 >= stride || y4 >= h4 {
            return None;
        }
        Some(y4 * stride + x4)
    }

    fn get_i4x4_mode(&self, x4: usize, y4: usize) -> u8 {
        self.i4x4_mode_index(x4, y4)
            .and_then(|idx| self.i4x4_modes.get(idx).copied())
            .unwrap_or(2)
    }

    fn set_i4x4_mode(&mut self, x4: usize, y4: usize, mode: u8) {
        if let Some(idx) = self.i4x4_mode_index(x4, y4)
            && let Some(slot) = self.i4x4_modes.get_mut(idx)
        {
            *slot = mode.min(8);
        }
    }

    fn mb_index(&self, mb_x: usize, mb_y: usize) -> Option<usize> {
        if mb_x >= self.mb_width || mb_y >= self.mb_height {
            return None;
        }
        Some(mb_y * self.mb_width + mb_x)
    }

    fn get_mb_cbp(&self, mb_x: usize, mb_y: usize) -> u8 {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.mb_cbp.get(idx).copied())
            .unwrap_or(0)
    }

    fn set_mb_cbp(&mut self, mb_x: usize, mb_y: usize, cbp: u8) {
        if let Some(idx) = self.mb_index(mb_x, mb_y) {
            if let Some(slot) = self.mb_cbp.get_mut(idx) {
                *slot = cbp;
            }
            if let Some(slot) = self.mb_cbp_ctx.get_mut(idx) {
                *slot = (*slot & !0x003F) | u16::from(cbp & 0x3F);
            }
        }
    }

    fn get_chroma_pred_mode(&self, mb_x: usize, mb_y: usize) -> u8 {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.chroma_pred_modes.get(idx).copied())
            .unwrap_or(0)
    }

    fn set_chroma_pred_mode(&mut self, mb_x: usize, mb_y: usize, mode: u8) {
        if let Some(idx) = self.mb_index(mb_x, mb_y)
            && let Some(slot) = self.chroma_pred_modes.get_mut(idx)
        {
            *slot = mode.min(3);
        }
    }

    fn set_transform_8x8_flag(&mut self, mb_x: usize, mb_y: usize, flag: bool) {
        if let Some(idx) = self.mb_index(mb_x, mb_y)
            && let Some(slot) = self.transform_8x8_flags.get_mut(idx)
        {
            *slot = u8::from(flag);
        }
    }

    fn get_transform_8x8_flag(&self, mb_x: usize, mb_y: usize) -> bool {
        self.mb_index(mb_x, mb_y)
            .and_then(|idx| self.transform_8x8_flags.get(idx).copied())
            .unwrap_or(0)
            != 0
    }

    /// 解码 transform_size_8x8_flag.
    fn decode_transform_size_8x8_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left = mb_x > 0 && self.get_transform_8x8_flag(mb_x - 1, mb_y);
        let top = mb_y > 0 && self.get_transform_8x8_flag(mb_x, mb_y - 1);
        let idx = 399usize + usize::from(left) + usize::from(top);
        if idx < ctxs.len() {
            cabac.decode_decision(&mut ctxs[idx]) == 1
        } else {
            cabac.decode_decision(&mut ctxs[68]) == 1
        }
    }

    fn decode_chroma_pred_mode(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> u8 {
        let mut ctx = 0usize;
        if mb_x > 0 && self.get_chroma_pred_mode(mb_x - 1, mb_y) != 0 {
            ctx += 1;
        }
        if mb_y > 0 && self.get_chroma_pred_mode(mb_x, mb_y - 1) != 0 {
            ctx += 1;
        }
        if cabac.decode_decision(&mut ctxs[64 + ctx]) == 0 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[67]) == 0 {
            return 1;
        }
        if cabac.decode_decision(&mut ctxs[67]) == 0 {
            2
        } else {
            3
        }
    }

    fn decode_coded_block_pattern(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        intra_defaults: bool,
    ) -> (u8, u8) {
        let unavailable_cbp = if intra_defaults { 0xcf } else { 0x0f };
        let cbp_a = if mb_x > 0 {
            self.get_mb_cbp(mb_x - 1, mb_y)
        } else {
            unavailable_cbp
        };
        let cbp_b = if mb_y > 0 {
            self.get_mb_cbp(mb_x, mb_y - 1)
        } else {
            unavailable_cbp
        };

        let mut luma_cbp = 0u8;
        let mut ctx = usize::from((cbp_a & 0x02) == 0) + (usize::from((cbp_b & 0x04) == 0) << 1);
        let bit0 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit0;

        ctx = usize::from((luma_cbp & 0x01) == 0) + (usize::from((cbp_b & 0x08) == 0) << 1);
        let bit1 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit1 << 1;

        ctx = usize::from((cbp_a & 0x08) == 0) + (usize::from((luma_cbp & 0x01) == 0) << 1);
        let bit2 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit2 << 2;

        ctx = usize::from((luma_cbp & 0x04) == 0) + (usize::from((luma_cbp & 0x02) == 0) << 1);
        let bit3 = cabac.decode_decision(&mut ctxs[73 + ctx]) as u8;
        luma_cbp |= bit3 << 3;

        let cbp_a_chroma = (cbp_a >> 4) & 0x03;
        let cbp_b_chroma = (cbp_b >> 4) & 0x03;
        let mut c_ctx = 0usize;
        if cbp_a_chroma > 0 {
            c_ctx += 1;
        }
        if cbp_b_chroma > 0 {
            c_ctx += 2;
        }
        if cabac.decode_decision(&mut ctxs[77 + c_ctx]) == 0 {
            return (luma_cbp, 0);
        }

        let mut c_ctx2 = 4usize;
        if cbp_a_chroma == 2 {
            c_ctx2 += 1;
        }
        if cbp_b_chroma == 2 {
            c_ctx2 += 2;
        }
        let chroma_cbp = 1u8 + cabac.decode_decision(&mut ctxs[77 + c_ctx2]) as u8;
        (luma_cbp, chroma_cbp)
    }

    /// 解码 I_4x4 宏块的 16 个预测模式
    fn decode_i4x4_pred_modes(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 16] {
        // H.264 规范顺序: 按 8x8 块分组, 每组内按 2x2 子块顺序.
        const I4X4_SCAN_ORDER: [(usize, usize); 16] = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (0, 2),
            (1, 2),
            (0, 3),
            (1, 3),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
        ];

        let mut modes = [2u8; 16];
        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            let left = if x4 > 0 {
                self.get_i4x4_mode(x4 - 1, y4)
            } else {
                2
            };
            let top = if y4 > 0 {
                self.get_i4x4_mode(x4, y4 - 1)
            } else {
                2
            };
            let pred_mode = left.min(top);
            let prev_flag = cabac.decode_decision(&mut ctxs[68]);
            let mode = if prev_flag == 1 {
                pred_mode
            } else {
                let rem = (cabac.decode_decision(&mut ctxs[69])
                    | (cabac.decode_decision(&mut ctxs[69]) << 1)
                    | (cabac.decode_decision(&mut ctxs[69]) << 2)) as u8;
                if rem < pred_mode { rem } else { rem + 1 }
            };
            modes[sub_y * 4 + sub_x] = mode.min(8);
            self.set_i4x4_mode(x4, y4, mode);
        }
        modes
    }

    /// 解码 I_8x8 宏块的 4 个预测模式 (最小可用路径)
    fn decode_i8x8_pred_modes(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> [u8; 4] {
        let mut modes = [2u8; 4];
        for block_y in 0..2 {
            for block_x in 0..2 {
                let x4 = mb_x * 4 + block_x * 2;
                let y4 = mb_y * 4 + block_y * 2;
                let left = if x4 > 0 {
                    self.get_i4x4_mode(x4 - 1, y4)
                } else {
                    2
                };
                let top = if y4 > 0 {
                    self.get_i4x4_mode(x4, y4 - 1)
                } else {
                    2
                };
                let pred_mode = left.min(top);
                let prev_flag = cabac.decode_decision(&mut ctxs[68]);
                let mode = if prev_flag == 1 {
                    pred_mode
                } else {
                    let rem = (cabac.decode_decision(&mut ctxs[69])
                        | (cabac.decode_decision(&mut ctxs[69]) << 1)
                        | (cabac.decode_decision(&mut ctxs[69]) << 2))
                        as u8;
                    if rem < pred_mode { rem } else { rem + 1 }
                }
                .min(8);

                let idx = block_y * 2 + block_x;
                modes[idx] = mode;
                for sub_y in 0..2 {
                    for sub_x in 0..2 {
                        self.set_i4x4_mode(x4 + sub_x, y4 + sub_y, mode);
                    }
                }
            }
        }
        modes
    }

    /// 解码 I-slice 的所有宏块
    fn decode_i_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let ignore_terminate = std::env::var("TAO_H264_IGNORE_TERMINATE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let mut decoded = 0usize;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;

            let mb_type = decode_i_mb_type(cabac, ctxs, &self.mb_types, self.mb_width, mb_x, mb_y);
            self.mb_types[mb_idx] = mb_type as u8;
            decoded += 1;
            if debug_mb && debug_mb_selected(mb_idx, mb_x, mb_y) {
                eprintln!(
                    "[H264][I-slice] mb=({}, {}), mb_type={}",
                    mb_x, mb_y, mb_type
                );
            }

            if mb_type == 0 {
                self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
            } else if mb_type <= 24 {
                self.decode_i_16x16_mb(cabac, ctxs, mb_x, mb_y, mb_type, &mut cur_qp);
            } else if mb_type == 25 {
                if debug_mb {
                    eprintln!(
                        "[H264][I-slice] I_PCM 命中: mb_idx={}, mb=({}, {})",
                        mb_idx, mb_x, mb_y
                    );
                }
                self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                self.prev_qp_delta_nz = false;
            }
            if !ignore_terminate && mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                if debug_mb {
                    eprintln!(
                        "[H264][I-slice] 提前结束: first_mb={}, total_mbs={}, decoded_mbs={}, last_mb=({}, {}), cabac_bits={}/{}, range={}, low=0x{:x}",
                        first,
                        total,
                        decoded,
                        mb_x,
                        mb_y,
                        cabac.bit_pos(),
                        cabac.total_bits(),
                        cabac.range(),
                        cabac.low()
                    );
                }
                break;
            }
        }
        if debug_mb {
            eprintln!(
                "[H264][I-slice] 完成: first_mb={}, total_mbs={}, decoded_mbs={}, cabac_bits={}/{}, range={}, low=0x{:x}",
                first,
                total,
                decoded,
                cabac.bit_pos(),
                cabac.total_bits(),
                cabac.range(),
                cabac.low()
            );
        }
    }

    /// 解码 P-slice 宏块.
    #[allow(clippy::too_many_arguments)]
    fn decode_p_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        num_ref_idx_l0: u32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let mut decoded = 0usize;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_p_mb_skip_flag(cabac, ctxs, mb_x, mb_y);
            decoded += 1;

            if skip {
                self.mb_types[mb_idx] = 255;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
                let (pred_x, pred_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
                self.apply_inter_block_l0(
                    ref_l0_list,
                    0,
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                    pred_x,
                    pred_y,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
                self.mv_l0_x[mb_idx] = pred_x as i16;
                self.mv_l0_y[mb_idx] = pred_y as i16;
                self.ref_idx_l0[mb_idx] = 0;
            } else if let Some(p_mb_type) = self.decode_p_mb_type(cabac, ctxs, mb_x, mb_y) {
                self.decode_p_inter_mb(
                    cabac,
                    ctxs,
                    mb_x,
                    mb_y,
                    p_mb_type,
                    &mut cur_qp,
                    num_ref_idx_l0,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                    ref_l0_list,
                );
            } else {
                let intra_mb_type = decode_intra_mb_type(
                    cabac,
                    ctxs,
                    17,
                    false,
                    &self.mb_types,
                    self.mb_width,
                    mb_x,
                    mb_y,
                );
                self.mb_types[mb_idx] = intra_mb_type as u8;
                if intra_mb_type == 0 {
                    self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
                } else if intra_mb_type <= 24 {
                    self.decode_i_16x16_mb(cabac, ctxs, mb_x, mb_y, intra_mb_type, &mut cur_qp);
                } else if intra_mb_type == 25 {
                    self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                    self.prev_qp_delta_nz = false;
                }
            }

            if mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                if debug_mb {
                    eprintln!(
                        "[H264][P-slice] 提前结束: first_mb={}, total_mbs={}, decoded_mbs={}, last_mb=({}, {}), cabac_bits={}/{}",
                        first,
                        total,
                        decoded,
                        mb_x,
                        mb_y,
                        cabac.bit_pos(),
                        cabac.total_bits()
                    );
                }
                break;
            }
        }

        if debug_mb {
            eprintln!(
                "[H264][P-slice] 完成: first_mb={}, total_mbs={}, decoded_mbs={}, cabac_bits={}/{}",
                first,
                total,
                decoded,
                cabac.bit_pos(),
                cabac.total_bits()
            );
        }
    }

    /// 解码 P-slice 的 mb_skip_flag.
    fn decode_p_mb_skip_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left_non_skip = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .unwrap_or(255)
                != 255;
        let top_non_skip = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .unwrap_or(255)
                != 255;
        let ctx = usize::from(left_non_skip) + (usize::from(top_non_skip) << 1);
        cabac.decode_decision(&mut ctxs[11 + ctx]) == 1
    }

    /// 解码 P-slice 的 mb_type.
    ///
    /// 返回值:
    /// - `Some(0..=3)`: 互预测类型.
    /// - `None`: Intra 宏块, 需走 intra_mb_type 语法.
    fn decode_p_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        _mb_x: usize,
        _mb_y: usize,
    ) -> Option<u8> {
        if cabac.decode_decision(&mut ctxs[14]) == 0 {
            if cabac.decode_decision(&mut ctxs[15]) == 0 {
                let idx = 3 * cabac.decode_decision(&mut ctxs[16]) as u8;
                return Some(idx);
            }
            let idx = 2 - cabac.decode_decision(&mut ctxs[17]) as u8;
            return Some(idx);
        }
        None
    }

    /// 解码 P_8x8 的 sub_mb_type.
    fn decode_p_sub_mb_type(&self, cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) -> u8 {
        if cabac.decode_decision(&mut ctxs[21]) == 1 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[22]) == 0 {
            return 1;
        }
        if cabac.decode_decision(&mut ctxs[23]) == 1 {
            2
        } else {
            3
        }
    }

    /// 解码 B-slice 的 mb_skip_flag.
    fn decode_b_mb_skip_flag(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> bool {
        let left_non_skip = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254 && t != 255)
                .unwrap_or(false);
        let top_non_skip = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t != 254 && t != 255)
                .unwrap_or(false);
        let ctx = usize::from(left_non_skip) + usize::from(top_non_skip);
        cabac.decode_decision(&mut ctxs[24 + ctx]) == 1
    }

    /// 解码 B-slice 的 mb_type.
    fn decode_b_mb_type(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
    ) -> BMbType {
        let left_direct = mb_x > 0
            && self
                .mb_index(mb_x - 1, mb_y)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t == 254)
                .unwrap_or(false);
        let top_direct = mb_y > 0
            && self
                .mb_index(mb_x, mb_y - 1)
                .and_then(|i| self.mb_types.get(i).copied())
                .map(|t| t == 254)
                .unwrap_or(false);

        let mut ctx = 0usize;
        if !left_direct {
            ctx += 1;
        }
        if !top_direct {
            ctx += 1;
        }

        if cabac.decode_decision(&mut ctxs[27 + ctx]) == 0 {
            return BMbType::Direct;
        }
        if cabac.decode_decision(&mut ctxs[30]) == 0 {
            let idx = 1 + cabac.decode_decision(&mut ctxs[32]) as u8;
            return BMbType::Inter(idx);
        }

        let mut bits = (cabac.decode_decision(&mut ctxs[31]) as u8) << 3;
        bits |= (cabac.decode_decision(&mut ctxs[32]) as u8) << 2;
        bits |= (cabac.decode_decision(&mut ctxs[32]) as u8) << 1;
        bits |= cabac.decode_decision(&mut ctxs[32]) as u8;

        if bits < 8 {
            return BMbType::Inter(bits + 3);
        }
        if bits == 13 {
            return BMbType::Intra;
        }
        if bits == 14 {
            return BMbType::Inter(11);
        }
        if bits == 15 {
            return BMbType::Inter(22);
        }

        bits = (bits << 1) | cabac.decode_decision(&mut ctxs[32]) as u8;
        BMbType::Inter(bits - 4)
    }

    /// 解码 B_8x8 的 sub_mb_type.
    fn decode_b_sub_mb_type(&self, cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) -> u8 {
        if cabac.decode_decision(&mut ctxs[36]) == 0 {
            return 0;
        }
        if cabac.decode_decision(&mut ctxs[37]) == 0 {
            return 1 + cabac.decode_decision(&mut ctxs[39]) as u8;
        }
        let mut ty = 3u8;
        if cabac.decode_decision(&mut ctxs[38]) == 1 {
            if cabac.decode_decision(&mut ctxs[39]) == 1 {
                return 11 + cabac.decode_decision(&mut ctxs[39]) as u8;
            }
            ty += 4;
        }
        ty += (cabac.decode_decision(&mut ctxs[39]) as u8) << 1;
        ty += cabac.decode_decision(&mut ctxs[39]) as u8;
        ty
    }

    fn b_mb_partition_info(mb_type_idx: u8) -> Option<(u8, BPredDir, BPredDir)> {
        match mb_type_idx {
            1 => Some((0, BPredDir::L0, BPredDir::Direct)),
            2 => Some((0, BPredDir::L1, BPredDir::Direct)),
            3 => Some((0, BPredDir::Bi, BPredDir::Direct)),
            4 => Some((1, BPredDir::L0, BPredDir::L0)),
            5 => Some((2, BPredDir::L0, BPredDir::L0)),
            6 => Some((1, BPredDir::L1, BPredDir::L1)),
            7 => Some((2, BPredDir::L1, BPredDir::L1)),
            8 => Some((1, BPredDir::L0, BPredDir::L1)),
            9 => Some((2, BPredDir::L0, BPredDir::L1)),
            10 => Some((1, BPredDir::L1, BPredDir::L0)),
            11 => Some((2, BPredDir::L1, BPredDir::L0)),
            12 => Some((1, BPredDir::L0, BPredDir::Bi)),
            13 => Some((2, BPredDir::L0, BPredDir::Bi)),
            14 => Some((1, BPredDir::L1, BPredDir::Bi)),
            15 => Some((2, BPredDir::L1, BPredDir::Bi)),
            16 => Some((1, BPredDir::Bi, BPredDir::L0)),
            17 => Some((2, BPredDir::Bi, BPredDir::L0)),
            18 => Some((1, BPredDir::Bi, BPredDir::L1)),
            19 => Some((2, BPredDir::Bi, BPredDir::L1)),
            20 => Some((1, BPredDir::Bi, BPredDir::Bi)),
            21 => Some((2, BPredDir::Bi, BPredDir::Bi)),
            _ => None,
        }
    }

    fn b_sub_mb_info(sub_mb_type: u8) -> (usize, usize, usize, BPredDir) {
        match sub_mb_type {
            0 => (8, 8, 1, BPredDir::Direct),
            1 => (8, 8, 1, BPredDir::L0),
            2 => (8, 8, 1, BPredDir::L1),
            3 => (8, 8, 1, BPredDir::Bi),
            4 => (8, 4, 2, BPredDir::L0),
            5 => (4, 8, 2, BPredDir::L0),
            6 => (8, 4, 2, BPredDir::L1),
            7 => (4, 8, 2, BPredDir::L1),
            8 => (8, 4, 2, BPredDir::Bi),
            9 => (4, 8, 2, BPredDir::Bi),
            10 => (4, 4, 4, BPredDir::L0),
            11 => (4, 4, 4, BPredDir::L1),
            12 => (4, 4, 4, BPredDir::Bi),
            _ => (8, 8, 1, BPredDir::Direct),
        }
    }

    fn decode_ref_idx_l0(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        num_ref_idx_l0: u32,
    ) -> u32 {
        if num_ref_idx_l0 <= 1 {
            return 0;
        }
        let mut ref_idx = 0u32;
        let mut ctx = 0usize;
        while cabac.decode_decision(&mut ctxs[54 + ctx]) == 1 {
            ref_idx += 1;
            ctx = (ctx >> 2) + 4;
            if ref_idx + 1 >= num_ref_idx_l0 {
                break;
            }
            if ref_idx >= 31 {
                break;
            }
        }
        ref_idx
    }

    fn decode_mb_mvd_component(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        ctx_base: usize,
        amvd: i32,
    ) -> i32 {
        let ctx_inc = if amvd > 32 {
            2usize
        } else if amvd > 2 {
            1usize
        } else {
            0usize
        };
        if cabac.decode_decision(&mut ctxs[ctx_base + ctx_inc]) == 0 {
            return 0;
        }

        let mut mvd = 1i32;
        let mut ctx = ctx_base + 3;
        while mvd < 9 && cabac.decode_decision(&mut ctxs[ctx]) == 1 {
            if mvd < 4 {
                ctx += 1;
            }
            mvd += 1;
        }

        if mvd >= 9 {
            let mut k = 3i32;
            while cabac.decode_bypass() == 1 && k < 24 {
                mvd += 1 << k;
                k += 1;
            }
            while k > 0 {
                k -= 1;
                mvd += (cabac.decode_bypass() as i32) << k;
            }
        }

        if cabac.decode_bypass() == 1 {
            -mvd
        } else {
            mvd
        }
    }

    fn predict_mv_l0_16x16(&self, mb_x: usize, mb_y: usize) -> (i32, i32) {
        let left = if mb_x > 0 {
            self.mb_index(mb_x - 1, mb_y)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };
        let top = if mb_y > 0 {
            self.mb_index(mb_x, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };
        let top_right = if mb_y > 0 && mb_x + 1 < self.mb_width {
            self.mb_index(mb_x + 1, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else if mb_x > 0 && mb_y > 0 {
            self.mb_index(mb_x - 1, mb_y - 1)
                .map(|i| (self.mv_l0_x[i], self.mv_l0_y[i]))
        } else {
            None
        };

        let a = left.unwrap_or((0, 0));
        let b = top.unwrap_or(a);
        let c = top_right.unwrap_or(b);
        (
            median3(a.0 as i32, b.0 as i32, c.0 as i32),
            median3(a.1 as i32, b.1 as i32, c.1 as i32),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_inter_block(
        &mut self,
        src_y: &[u8],
        src_u: &[u8],
        src_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        pred_weight: Option<&PredWeightL0>,
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let luma_src_x = dst_x as i32 + floor_div(mv_x_qpel, 4);
        let luma_src_y = dst_y as i32 + floor_div(mv_y_qpel, 4);
        let luma_fx = mod_floor(mv_x_qpel, 4) as u8;
        let luma_fy = mod_floor(mv_y_qpel, 4) as u8;
        if let Some(weight) = pred_weight {
            weighted_copy_luma_block_with_h264_qpel(
                src_y,
                self.stride_y,
                &mut self.ref_y,
                self.stride_y,
                luma_src_x,
                luma_src_y,
                luma_fx,
                luma_fy,
                dst_x,
                dst_y,
                w,
                h,
                self.stride_y,
                self.mb_height * 16,
                weight.luma_weight,
                weight.luma_offset,
                luma_log2_weight_denom,
            );
        } else {
            copy_luma_block_with_h264_qpel(
                src_y,
                self.stride_y,
                &mut self.ref_y,
                self.stride_y,
                luma_src_x,
                luma_src_y,
                luma_fx,
                luma_fy,
                dst_x,
                dst_y,
                w,
                h,
                self.stride_y,
                self.mb_height * 16,
            );
        }

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let c_src_x = c_dst_x as i32 + floor_div(mv_x_qpel, 8);
        let c_src_y = c_dst_y as i32 + floor_div(mv_y_qpel, 8);
        let c_fx = mod_floor(mv_x_qpel, 8) as u8;
        let c_fy = mod_floor(mv_y_qpel, 8) as u8;
        if let Some(weight) = pred_weight {
            weighted_copy_block_with_qpel_bilinear(
                src_u,
                self.stride_c,
                &mut self.ref_u,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
                weight.chroma_weight[0],
                weight.chroma_offset[0],
                chroma_log2_weight_denom,
            );
            weighted_copy_block_with_qpel_bilinear(
                src_v,
                self.stride_c,
                &mut self.ref_v,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
                weight.chroma_weight[1],
                weight.chroma_offset[1],
                chroma_log2_weight_denom,
            );
        } else {
            copy_block_with_qpel_bilinear(
                src_u,
                self.stride_c,
                &mut self.ref_u,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
            );
            copy_block_with_qpel_bilinear(
                src_v,
                self.stride_c,
                &mut self.ref_v,
                self.stride_c,
                c_src_x,
                c_src_y,
                c_fx,
                c_fy,
                8,
                c_dst_x,
                c_dst_y,
                cw,
                ch,
                self.stride_c,
                self.mb_height * 8,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_inter_block_l0(
        &mut self,
        ref_l0_list: &[RefPlanes],
        ref_idx: u32,
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
    ) {
        let ref_src = select_ref_planes(ref_l0_list, ref_idx.min(i8::MAX as u32) as i8);
        self.apply_inter_block(
            ref_src.y.as_slice(),
            ref_src.u.as_slice(),
            ref_src.v.as_slice(),
            dst_x,
            dst_y,
            w,
            h,
            mv_x_qpel,
            mv_y_qpel,
            p_l0_weight(l0_weights, ref_idx),
            luma_log2_weight_denom,
            chroma_log2_weight_denom,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn blend_inter_block(
        &mut self,
        src_y: &[u8],
        src_u: &[u8],
        src_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv_x_qpel: i32,
        mv_y_qpel: i32,
    ) {
        let luma_src_x = dst_x as i32 + floor_div(mv_x_qpel, 4);
        let luma_src_y = dst_y as i32 + floor_div(mv_y_qpel, 4);
        let luma_fx = mod_floor(mv_x_qpel, 4) as u8;
        let luma_fy = mod_floor(mv_y_qpel, 4) as u8;
        blend_luma_block_with_h264_qpel(
            src_y,
            self.stride_y,
            &mut self.ref_y,
            self.stride_y,
            luma_src_x,
            luma_src_y,
            luma_fx,
            luma_fy,
            dst_x,
            dst_y,
            w,
            h,
            self.stride_y,
            self.mb_height * 16,
        );

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let c_src_x = c_dst_x as i32 + floor_div(mv_x_qpel, 8);
        let c_src_y = c_dst_y as i32 + floor_div(mv_y_qpel, 8);
        let c_fx = mod_floor(mv_x_qpel, 8) as u8;
        let c_fy = mod_floor(mv_y_qpel, 8) as u8;
        blend_block_with_qpel_bilinear(
            src_u,
            self.stride_c,
            &mut self.ref_u,
            self.stride_c,
            c_src_x,
            c_src_y,
            c_fx,
            c_fy,
            8,
            c_dst_x,
            c_dst_y,
            cw,
            ch,
            self.stride_c,
            self.mb_height * 8,
        );
        blend_block_with_qpel_bilinear(
            src_v,
            self.stride_c,
            &mut self.ref_v,
            self.stride_c,
            c_src_x,
            c_src_y,
            c_fx,
            c_fy,
            8,
            c_dst_x,
            c_dst_y,
            cw,
            ch,
            self.stride_c,
            self.mb_height * 8,
        );
    }

    fn implicit_bi_weights(&self, ref_l0_poc: i32, ref_l1_poc: i32) -> (i32, i32) {
        let td = (ref_l1_poc - ref_l0_poc).clamp(-128, 127);
        if td == 0 {
            return (32, 32);
        }
        let tb = (self.last_poc - ref_l0_poc).clamp(-128, 127);
        let tx = (16384 + (td.abs() >> 1)) / td;
        let dist_scale_factor = ((tb * tx + 32) >> 6).clamp(-1024, 1023);
        let w1 = dist_scale_factor >> 2;
        if (-64..=128).contains(&w1) {
            (64 - w1, w1)
        } else {
            (32, 32)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_bi_weighted_block(
        &mut self,
        src_l0_y: &[u8],
        src_l0_u: &[u8],
        src_l0_v: &[u8],
        src_l1_y: &[u8],
        src_l1_u: &[u8],
        src_l1_v: &[u8],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
        mv0_x_qpel: i32,
        mv0_y_qpel: i32,
        mv1_x_qpel: i32,
        mv1_y_qpel: i32,
        w0: i32,
        w1: i32,
    ) {
        let l0_src_x = dst_x as i32 + floor_div(mv0_x_qpel, 4);
        let l0_src_y = dst_y as i32 + floor_div(mv0_y_qpel, 4);
        let l0_fx = mod_floor(mv0_x_qpel, 4) as u8;
        let l0_fy = mod_floor(mv0_y_qpel, 4) as u8;
        let l1_src_x = dst_x as i32 + floor_div(mv1_x_qpel, 4);
        let l1_src_y = dst_y as i32 + floor_div(mv1_y_qpel, 4);
        let l1_fx = mod_floor(mv1_x_qpel, 4) as u8;
        let l1_fy = mod_floor(mv1_y_qpel, 4) as u8;

        for y in 0..h {
            for x in 0..w {
                let px0 = sample_h264_luma_qpel(
                    src_l0_y,
                    self.stride_y,
                    self.stride_y,
                    self.mb_height * 16,
                    l0_src_x + x as i32,
                    l0_src_y + y as i32,
                    l0_fx,
                    l0_fy,
                ) as i32;
                let px1 = sample_h264_luma_qpel(
                    src_l1_y,
                    self.stride_y,
                    self.stride_y,
                    self.mb_height * 16,
                    l1_src_x + x as i32,
                    l1_src_y + y as i32,
                    l1_fx,
                    l1_fy,
                ) as i32;
                let dst_idx = (dst_y + y) * self.stride_y + (dst_x + x);
                if dst_idx < self.ref_y.len() {
                    let v = ((w0 * px0 + w1 * px1 + 32) >> 6).clamp(0, 255) as u8;
                    self.ref_y[dst_idx] = v;
                }
            }
        }

        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let c_dst_x = dst_x / 2;
        let c_dst_y = dst_y / 2;
        let l0_c_src_x = c_dst_x as i32 + floor_div(mv0_x_qpel, 8);
        let l0_c_src_y = c_dst_y as i32 + floor_div(mv0_y_qpel, 8);
        let l1_c_src_x = c_dst_x as i32 + floor_div(mv1_x_qpel, 8);
        let l1_c_src_y = c_dst_y as i32 + floor_div(mv1_y_qpel, 8);
        let l0_c_fx = mod_floor(mv0_x_qpel, 8) as u8;
        let l0_c_fy = mod_floor(mv0_y_qpel, 8) as u8;
        let l1_c_fx = mod_floor(mv1_x_qpel, 8) as u8;
        let l1_c_fy = mod_floor(mv1_y_qpel, 8) as u8;

        for y in 0..ch {
            for x in 0..cw {
                let u0 = sample_bilinear_clamped(
                    src_l0_u,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l0_c_src_x + x as i32,
                    l0_c_src_y + y as i32,
                    l0_c_fx,
                    l0_c_fy,
                    8,
                ) as i32;
                let u1 = sample_bilinear_clamped(
                    src_l1_u,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l1_c_src_x + x as i32,
                    l1_c_src_y + y as i32,
                    l1_c_fx,
                    l1_c_fy,
                    8,
                ) as i32;
                let v0 = sample_bilinear_clamped(
                    src_l0_v,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l0_c_src_x + x as i32,
                    l0_c_src_y + y as i32,
                    l0_c_fx,
                    l0_c_fy,
                    8,
                ) as i32;
                let v1 = sample_bilinear_clamped(
                    src_l1_v,
                    self.stride_c,
                    self.stride_c,
                    self.mb_height * 8,
                    l1_c_src_x + x as i32,
                    l1_c_src_y + y as i32,
                    l1_c_fx,
                    l1_c_fy,
                    8,
                ) as i32;
                let dst_idx = (c_dst_y + y) * self.stride_c + (c_dst_x + x);
                if dst_idx < self.ref_u.len() {
                    self.ref_u[dst_idx] = ((w0 * u0 + w1 * u1 + 32) >> 6).clamp(0, 255) as u8;
                }
                if dst_idx < self.ref_v.len() {
                    self.ref_v[dst_idx] = ((w0 * v0 + w1 * v1 + 32) >> 6).clamp(0, 255) as u8;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_p_inter_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        p_mb_type: u8,
        cur_qp: &mut i32,
        num_ref_idx_l0: u32,
        l0_weights: &[PredWeightL0],
        luma_log2_weight_denom: u8,
        chroma_log2_weight_denom: u8,
        ref_l0_list: &[RefPlanes],
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);

        let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
        let mut final_mv_x = pred_mv_x;
        let mut final_mv_y = pred_mv_y;
        let mut final_ref_idx = 0u32;

        match p_mb_type {
            0 => {
                final_ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                final_mv_x += mvd_x;
                final_mv_y += mvd_y;
                self.apply_inter_block_l0(
                    ref_l0_list,
                    final_ref_idx,
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                    final_mv_x,
                    final_mv_y,
                    l0_weights,
                    luma_log2_weight_denom,
                    chroma_log2_weight_denom,
                );
            }
            1 => {
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = pred_mv_x + mvd_x;
                    let mv_y = pred_mv_y + mvd_y;
                    let y_off = part * 8;
                    self.apply_inter_block_l0(
                        ref_l0_list,
                        ref_idx,
                        mb_x * 16,
                        mb_y * 16 + y_off,
                        16,
                        8,
                        mv_x,
                        mv_y,
                        l0_weights,
                        luma_log2_weight_denom,
                        chroma_log2_weight_denom,
                    );
                    final_mv_x = mv_x;
                    final_mv_y = mv_y;
                    final_ref_idx = ref_idx;
                }
            }
            2 => {
                for part in 0..2usize {
                    let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                    let mv_x = pred_mv_x + mvd_x;
                    let mv_y = pred_mv_y + mvd_y;
                    let x_off = part * 8;
                    self.apply_inter_block_l0(
                        ref_l0_list,
                        ref_idx,
                        mb_x * 16 + x_off,
                        mb_y * 16,
                        8,
                        16,
                        mv_x,
                        mv_y,
                        l0_weights,
                        luma_log2_weight_denom,
                        chroma_log2_weight_denom,
                    );
                    final_mv_x = mv_x;
                    final_mv_y = mv_y;
                    final_ref_idx = ref_idx;
                }
            }
            _ => {
                for sub in 0..4usize {
                    let sub_type = self.decode_p_sub_mb_type(cabac, ctxs);
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    match sub_type {
                        0 => {
                            let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                            let mv_x = pred_mv_x + mvd_x;
                            let mv_y = pred_mv_y + mvd_y;
                            self.apply_inter_block_l0(
                                ref_l0_list,
                                ref_idx,
                                mb_x * 16 + sx,
                                mb_y * 16 + sy,
                                8,
                                8,
                                mv_x,
                                mv_y,
                                l0_weights,
                                luma_log2_weight_denom,
                                chroma_log2_weight_denom,
                            );
                            final_mv_x = mv_x;
                            final_mv_y = mv_y;
                            final_ref_idx = ref_idx;
                        }
                        1 => {
                            for part in 0..2usize {
                                let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    mb_x * 16 + sx,
                                    mb_y * 16 + sy + part * 4,
                                    8,
                                    4,
                                    mv_x,
                                    mv_y,
                                    l0_weights,
                                    luma_log2_weight_denom,
                                    chroma_log2_weight_denom,
                                );
                                final_mv_x = mv_x;
                                final_mv_y = mv_y;
                                final_ref_idx = ref_idx;
                            }
                        }
                        2 => {
                            for part in 0..2usize {
                                let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                let mv_x = pred_mv_x + mvd_x;
                                let mv_y = pred_mv_y + mvd_y;
                                self.apply_inter_block_l0(
                                    ref_l0_list,
                                    ref_idx,
                                    mb_x * 16 + sx + part * 4,
                                    mb_y * 16 + sy,
                                    4,
                                    8,
                                    mv_x,
                                    mv_y,
                                    l0_weights,
                                    luma_log2_weight_denom,
                                    chroma_log2_weight_denom,
                                );
                                final_mv_x = mv_x;
                                final_mv_y = mv_y;
                                final_ref_idx = ref_idx;
                            }
                        }
                        _ => {
                            for part_y in 0..2usize {
                                for part_x in 0..2usize {
                                    let ref_idx =
                                        self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
                                    let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
                                    let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
                                    let mv_x = pred_mv_x + mvd_x;
                                    let mv_y = pred_mv_y + mvd_y;
                                    self.apply_inter_block_l0(
                                        ref_l0_list,
                                        ref_idx,
                                        mb_x * 16 + sx + part_x * 4,
                                        mb_y * 16 + sy + part_y * 4,
                                        4,
                                        4,
                                        mv_x,
                                        mv_y,
                                        l0_weights,
                                        luma_log2_weight_denom,
                                        chroma_log2_weight_denom,
                                    );
                                    final_mv_x = mv_x;
                                    final_mv_y = mv_y;
                                    final_ref_idx = ref_idx;
                                }
                            }
                        }
                    }
                }
            }
        }

        self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = final_ref_idx.min(i8::MAX as u32) as i8;
        self.mb_types[mb_idx] = 200u8.saturating_add(p_mb_type.min(3));

        let (luma_cbp, chroma_cbp) =
            self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, false);
        let cbp = luma_cbp | (chroma_cbp << 4);
        self.set_mb_cbp(mb_x, mb_y, cbp);

        if cbp != 0 {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        let use_8x8 = luma_cbp != 0
            && self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
            && self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);

        if use_8x8 {
            self.decode_i8x8_residual_fallback(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, false);
        } else {
            self.decode_inter_4x4_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp);
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, false);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_b_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;
        let mut decoded = 0usize;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;
            let skip = self.decode_b_mb_skip_flag(cabac, ctxs, mb_x, mb_y);
            decoded += 1;

            if skip {
                self.mb_types[mb_idx] = 254;
                self.set_mb_cbp(mb_x, mb_y, 0);
                self.set_transform_8x8_flag(mb_x, mb_y, false);
                self.set_chroma_pred_mode(mb_x, mb_y, 0);
                self.set_luma_dc_cbf(mb_x, mb_y, false);
                self.reset_chroma_cbf_mb(mb_x, mb_y);
                self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
                let (pred_x, pred_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
                let ref_l0 = select_ref_planes(ref_l0_list, 0);
                self.apply_inter_block(
                    ref_l0.y.as_slice(),
                    ref_l0.u.as_slice(),
                    ref_l0.v.as_slice(),
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                    pred_x,
                    pred_y,
                    None,
                    0,
                    0,
                );
                let ref_l1 = select_ref_planes(ref_l1_list, 0);
                self.blend_inter_block(
                    ref_l1.y.as_slice(),
                    ref_l1.u.as_slice(),
                    ref_l1.v.as_slice(),
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                    pred_x,
                    pred_y,
                );
                self.mv_l0_x[mb_idx] = pred_x as i16;
                self.mv_l0_y[mb_idx] = pred_y as i16;
                self.ref_idx_l0[mb_idx] = 0;
            } else {
                match self.decode_b_mb_type(cabac, ctxs, mb_x, mb_y) {
                    BMbType::Intra => {
                        let intra_mb_type = decode_intra_mb_type(
                            cabac,
                            ctxs,
                            32,
                            false,
                            &self.mb_types,
                            self.mb_width,
                            mb_x,
                            mb_y,
                        );
                        self.mb_types[mb_idx] = intra_mb_type as u8;
                        if intra_mb_type == 0 {
                            self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
                        } else if intra_mb_type <= 24 {
                            self.decode_i_16x16_mb(
                                cabac,
                                ctxs,
                                mb_x,
                                mb_y,
                                intra_mb_type,
                                &mut cur_qp,
                            );
                        } else if intra_mb_type == 25 {
                            self.decode_i_pcm_mb(cabac, mb_x, mb_y);
                            self.prev_qp_delta_nz = false;
                        }
                    }
                    BMbType::Direct => {
                        self.decode_b_inter_mb(
                            cabac,
                            ctxs,
                            mb_x,
                            mb_y,
                            None,
                            &mut cur_qp,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            ref_l0_list,
                            ref_l1_list,
                        );
                    }
                    BMbType::Inter(mb_type_idx) => {
                        self.decode_b_inter_mb(
                            cabac,
                            ctxs,
                            mb_x,
                            mb_y,
                            Some(mb_type_idx),
                            &mut cur_qp,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            ref_l0_list,
                            ref_l1_list,
                        );
                    }
                }
            }

            if mb_idx + 1 < total && cabac.decode_terminate() == 1 {
                if debug_mb {
                    eprintln!(
                        "[H264][B-slice] 提前结束: first_mb={}, total_mbs={}, decoded_mbs={}, last_mb=({}, {}), cabac_bits={}/{}",
                        first,
                        total,
                        decoded,
                        mb_x,
                        mb_y,
                        cabac.bit_pos(),
                        cabac.total_bits()
                    );
                }
                break;
            }
        }

        if debug_mb {
            eprintln!(
                "[H264][B-slice] 完成: first_mb={}, total_mbs={}, decoded_mbs={}, cabac_bits={}/{}",
                first,
                total,
                decoded,
                cabac.bit_pos(),
                cabac.total_bits()
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_b_prediction_block(
        &mut self,
        motion_l0: Option<BMotion>,
        motion_l1: Option<BMotion>,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
        dst_x: usize,
        dst_y: usize,
        w: usize,
        h: usize,
    ) -> (i32, i32, i8) {
        match (motion_l0, motion_l1) {
            (Some(m0), Some(m1)) => {
                let ref_l1 = select_ref_planes(ref_l1_list, m1.ref_idx);
                let ref_l0 = select_ref_planes(ref_l0_list, m0.ref_idx);
                let weighted_bipred_idc = self
                    .pps
                    .as_ref()
                    .map(|p| p.weighted_bipred_idc)
                    .unwrap_or(0);
                if weighted_bipred_idc == 2 {
                    let (w0, w1) = self.implicit_bi_weights(ref_l0.poc, ref_l1.poc);
                    self.apply_bi_weighted_block(
                        ref_l0.y.as_slice(),
                        ref_l0.u.as_slice(),
                        ref_l0.v.as_slice(),
                        ref_l1.y.as_slice(),
                        ref_l1.u.as_slice(),
                        ref_l1.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m0.mv_x,
                        m0.mv_y,
                        m1.mv_x,
                        m1.mv_y,
                        w0,
                        w1,
                    );
                } else {
                    self.apply_inter_block(
                        ref_l0.y.as_slice(),
                        ref_l0.u.as_slice(),
                        ref_l0.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m0.mv_x,
                        m0.mv_y,
                        None,
                        0,
                        0,
                    );
                    self.blend_inter_block(
                        ref_l1.y.as_slice(),
                        ref_l1.u.as_slice(),
                        ref_l1.v.as_slice(),
                        dst_x,
                        dst_y,
                        w,
                        h,
                        m1.mv_x,
                        m1.mv_y,
                    );
                }
                (m0.mv_x, m0.mv_y, m0.ref_idx)
            }
            (Some(m0), None) => {
                let ref_l0 = select_ref_planes(ref_l0_list, m0.ref_idx);
                self.apply_inter_block(
                    ref_l0.y.as_slice(),
                    ref_l0.u.as_slice(),
                    ref_l0.v.as_slice(),
                    dst_x,
                    dst_y,
                    w,
                    h,
                    m0.mv_x,
                    m0.mv_y,
                    None,
                    0,
                    0,
                );
                (m0.mv_x, m0.mv_y, m0.ref_idx)
            }
            (None, Some(m1)) => {
                let ref_l1 = select_ref_planes(ref_l1_list, m1.ref_idx);
                self.apply_inter_block(
                    ref_l1.y.as_slice(),
                    ref_l1.u.as_slice(),
                    ref_l1.v.as_slice(),
                    dst_x,
                    dst_y,
                    w,
                    h,
                    m1.mv_x,
                    m1.mv_y,
                    None,
                    0,
                    0,
                );
                (m1.mv_x, m1.mv_y, m1.ref_idx)
            }
            (None, None) => (0, 0, 0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_b_inter_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        mb_type_idx: Option<u8>,
        cur_qp: &mut i32,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        ref_l0_list: &[RefPlanes],
        ref_l1_list: &[RefPlanes],
    ) {
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);

        let (pred_mv_x, pred_mv_y) = self.predict_mv_l0_16x16(mb_x, mb_y);
        let mut final_mv_x = pred_mv_x;
        let mut final_mv_y = pred_mv_y;
        let mut final_ref_idx = 0i8;

        match mb_type_idx {
            None => {
                self.mb_types[mb_idx] = 254;
                let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                    Some(BMotion {
                        mv_x: pred_mv_x,
                        mv_y: pred_mv_y,
                        ref_idx: 0,
                    }),
                    Some(BMotion {
                        mv_x: pred_mv_x,
                        mv_y: pred_mv_y,
                        ref_idx: 0,
                    }),
                    ref_l0_list,
                    ref_l1_list,
                    mb_x * 16,
                    mb_y * 16,
                    16,
                    16,
                );
                final_mv_x = mv_x;
                final_mv_y = mv_y;
                final_ref_idx = ref_idx;
            }
            Some(22) => {
                self.mb_types[mb_idx] = 222;
                let mut sub_types = [0u8; 4];
                for slot in &mut sub_types {
                    *slot = self.decode_b_sub_mb_type(cabac, ctxs);
                }
                for (sub, sub_type) in sub_types.into_iter().enumerate() {
                    let sx = (sub & 1) * 8;
                    let sy = (sub >> 1) * 8;
                    let (part_w, part_h, part_count, dir) = Self::b_sub_mb_info(sub_type);
                    for part in 0..part_count {
                        let (part_off_x, part_off_y) = match (part_w, part_h, part_count) {
                            (8, 8, _) => (0, 0),
                            (8, 4, _) => (0, part * 4),
                            (4, 8, _) => (part * 4, 0),
                            _ => ((part & 1) * 4, (part >> 1) * 4),
                        };
                        let (motion_l0, motion_l1) = self.decode_b_partition_motion(
                            cabac,
                            ctxs,
                            dir,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            pred_mv_x,
                            pred_mv_y,
                        );
                        let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                            motion_l0,
                            motion_l1,
                            ref_l0_list,
                            ref_l1_list,
                            mb_x * 16 + sx + part_off_x,
                            mb_y * 16 + sy + part_off_y,
                            part_w,
                            part_h,
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx;
                    }
                }
            }
            Some(ty) => {
                self.mb_types[mb_idx] = 210u8.saturating_add(ty.min(40));
                if let Some((shape, dir0, dir1)) = Self::b_mb_partition_info(ty) {
                    let part_count = if shape == 0 { 1 } else { 2 };
                    for part in 0..part_count {
                        let dir = if part == 0 { dir0 } else { dir1 };
                        let (part_w, part_h, part_off_x, part_off_y) = match shape {
                            0 => (16usize, 16usize, 0usize, 0usize),
                            1 => (16usize, 8usize, 0usize, part * 8),
                            _ => (8usize, 16usize, part * 8, 0usize),
                        };
                        let (motion_l0, motion_l1) = self.decode_b_partition_motion(
                            cabac,
                            ctxs,
                            dir,
                            num_ref_idx_l0,
                            num_ref_idx_l1,
                            pred_mv_x,
                            pred_mv_y,
                        );
                        let (mv_x, mv_y, ref_idx) = self.apply_b_prediction_block(
                            motion_l0,
                            motion_l1,
                            ref_l0_list,
                            ref_l1_list,
                            mb_x * 16 + part_off_x,
                            mb_y * 16 + part_off_y,
                            part_w,
                            part_h,
                        );
                        final_mv_x = mv_x;
                        final_mv_y = mv_y;
                        final_ref_idx = ref_idx;
                    }
                }
            }
        }

        self.mv_l0_x[mb_idx] = final_mv_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.mv_l0_y[mb_idx] = final_mv_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.ref_idx_l0[mb_idx] = final_ref_idx;

        let (luma_cbp, chroma_cbp) =
            self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, false);
        let cbp = luma_cbp | (chroma_cbp << 4);
        self.set_mb_cbp(mb_x, mb_y, cbp);

        if cbp != 0 {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        let use_8x8 = luma_cbp != 0
            && self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
            && self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);

        if use_8x8 {
            self.decode_i8x8_residual_fallback(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, false);
        } else {
            self.decode_inter_4x4_residual(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp);
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, false);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_b_partition_motion(
        &self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        dir: BPredDir,
        num_ref_idx_l0: u32,
        num_ref_idx_l1: u32,
        pred_mv_x: i32,
        pred_mv_y: i32,
    ) -> (Option<BMotion>, Option<BMotion>) {
        let mut mv_l0_x = pred_mv_x;
        let mut mv_l0_y = pred_mv_y;
        let mut mv_l1_x = pred_mv_x;
        let mut mv_l1_y = pred_mv_y;
        let mut motion_l0 = None;
        let mut motion_l1 = None;

        if matches!(dir, BPredDir::L0 | BPredDir::Bi) {
            let ref_idx = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l0);
            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
            mv_l0_x += mvd_x;
            mv_l0_y += mvd_y;
            motion_l0 = Some(BMotion {
                mv_x: mv_l0_x,
                mv_y: mv_l0_y,
                ref_idx: ref_idx.min(i8::MAX as u32) as i8,
            });
        }
        if matches!(dir, BPredDir::L1 | BPredDir::Bi) {
            let ref_idx_l1 = self.decode_ref_idx_l0(cabac, ctxs, num_ref_idx_l1);
            let mvd_x = self.decode_mb_mvd_component(cabac, ctxs, 40, 0);
            let mvd_y = self.decode_mb_mvd_component(cabac, ctxs, 47, 0);
            mv_l1_x += mvd_x;
            mv_l1_y += mvd_y;
            motion_l1 = Some(BMotion {
                mv_x: mv_l1_x,
                mv_y: mv_l1_y,
                ref_idx: ref_idx_l1.min(i8::MAX as u32) as i8,
            });
        }

        if motion_l0.is_none() && motion_l1.is_none() {
            (
                Some(BMotion {
                    mv_x: pred_mv_x,
                    mv_y: pred_mv_y,
                    ref_idx: 0,
                }),
                None,
            )
        } else {
            (motion_l0, motion_l1)
        }
    }

    /// 解码并应用互预测 4x4 残差.
    fn decode_inter_4x4_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
    ) {
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }

        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let has_residual_8x8 = luma_cbp & (1 << i8x8) != 0;
            let mut coded_8x8 = false;

            for i_sub in 0..4 {
                let sub_x = i_sub & 1;
                let sub_y = i_sub >> 1;
                let abs_sub_x = x8x8 * 2 + sub_x;
                let abs_sub_y = y8x8 * 2 + sub_y;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;

                if !has_residual_8x8 {
                    self.set_luma_cbf(x4, y4, false);
                    continue;
                }

                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, false);
                let mut raw_coeffs =
                    decode_residual_block(cabac, ctxs, &residual::CAT_LUMA_4X4, cbf_inc);
                let coded = raw_coeffs.iter().any(|&c| c != 0);
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    coded_8x8 = true;
                }
                while raw_coeffs.len() < 16 {
                    raw_coeffs.push(0);
                }

                let mut coeffs_arr = [0i32; 16];
                coeffs_arr.copy_from_slice(&raw_coeffs[..16]);
                residual::dequant_4x4_ac(&mut coeffs_arr, qp);
                residual::apply_4x4_ac_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16 + abs_sub_x * 4,
                    mb_y * 16 + abs_sub_y * 4,
                    &coeffs_arr,
                );
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }

    /// 解码 I_4x4 宏块 (消耗所有 CABAC 语法元素, 使用真正的预测模式)
    fn decode_i_4x4_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        cur_qp: &mut i32,
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        // 1. 可选 transform_size_8x8_flag + 预测模式
        let force_4x4 = std::env::var("TAO_H264_FORCE_4X4")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let use_8x8 = !force_4x4
            && self
                .pps
                .as_ref()
                .map(|p| p.transform_8x8_mode)
                .unwrap_or(false)
            && self.decode_transform_size_8x8_flag(cabac, ctxs, mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, use_8x8);
        let pred_modes_4x4 = if use_8x8 {
            [2u8; 16]
        } else {
            self.decode_i4x4_pred_modes(cabac, ctxs, mb_x, mb_y)
        };
        let pred_modes_8x8 = if use_8x8 {
            self.decode_i8x8_pred_modes(cabac, ctxs, mb_x, mb_y)
        } else {
            [2u8; 4]
        };

        // 2. 解码 intra_chroma_pred_mode
        let chroma_mode = self.decode_chroma_pred_mode(cabac, ctxs, mb_x, mb_y);
        self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

        // 3. 解码 coded_block_pattern
        let (luma_cbp, chroma_cbp) = self.decode_coded_block_pattern(cabac, ctxs, mb_x, mb_y, true);
        self.set_mb_cbp(mb_x, mb_y, luma_cbp | (chroma_cbp << 4));
        if debug_mb && debug_mb_selected(mb_idx, mb_x, mb_y) {
            eprintln!(
                "[H264][I4x4] mb=({}, {}), use_8x8={}, chroma_mode={}, cbp_luma=0x{:x}, cbp_chroma={}",
                mb_x, mb_y, use_8x8, chroma_mode, luma_cbp, chroma_cbp
            );
        }

        // 4. mb_qp_delta (仅当 CBP != 0)
        let has_residual = luma_cbp != 0 || chroma_cbp != 0;
        if has_residual {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);
        } else {
            self.prev_qp_delta_nz = false;
        }

        // 5. 应用真正的预测 (根据预测模式)
        if use_8x8 {
            for block_y in 0..2 {
                for block_x in 0..2 {
                    let mode = pred_modes_8x8[block_y * 2 + block_x];
                    for sub_y in 0..2 {
                        for sub_x in 0..2 {
                            intra::predict_4x4(
                                &mut self.ref_y,
                                self.stride_y,
                                mb_x * 16 + (block_x * 2 + sub_x) * 4,
                                mb_y * 16 + (block_y * 2 + sub_y) * 4,
                                mode,
                            );
                        }
                    }
                }
            }
        }
        intra::predict_chroma_8x8(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_8x8(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 6. 解码残差并应用
        if use_8x8 {
            self.decode_i8x8_residual_fallback(cabac, ctxs, luma_cbp, mb_x, mb_y, *cur_qp, true);
        } else {
            self.decode_i4x4_residual(
                cabac,
                ctxs,
                luma_cbp,
                (mb_x, mb_y),
                *cur_qp,
                &pred_modes_4x4,
            );
        }

        if chroma_cbp >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, chroma_cbp >= 2, true);
        }
    }

    /// 解码 I_PCM 宏块: 字节对齐后直接读取原始样本
    fn decode_i_pcm_mb(&mut self, cabac: &mut CabacDecoder, mb_x: usize, mb_y: usize) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        cabac.align_to_byte_boundary();
        if debug_mb {
            eprintln!(
                "[H264][I_PCM] 对齐后: bytestream_pos={}, raw_pos={}, low=0x{:08x}",
                cabac.bytestream_pos(),
                cabac.raw_pos(),
                cabac.low()
            );
        }
        let ipcm_ptr_adjust = std::env::var("TAO_H264_IPCM_PTR_ADJUST")
            .ok()
            .and_then(|v| v.parse::<isize>().ok())
            .unwrap_or(0);
        if ipcm_ptr_adjust != 0 {
            cabac.adjust_raw_pos(ipcm_ptr_adjust);
            if debug_mb {
                eprintln!(
                    "[H264][I_PCM] 应用 TAO_H264_IPCM_PTR_ADJUST={} 后 raw_pos={}",
                    ipcm_ptr_adjust,
                    cabac.raw_pos()
                );
            }
        }
        // I_PCM 按“全部块可用”更新邻居上下文缓存, 避免后续 CABAC 上下文漂移.
        self.set_mb_cbp(mb_x, mb_y, 0x2f);
        self.set_chroma_pred_mode(mb_x, mb_y, 0);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        self.set_luma_dc_cbf(mb_x, mb_y, true);

        let x0 = mb_x * 16;
        let y0 = mb_y * 16;
        for dy in 0..16 {
            for dx in 0..16 {
                let idx = (y0 + dy) * self.stride_y + x0 + dx;
                if idx < self.ref_y.len() {
                    self.ref_y[idx] = cabac.read_raw_byte();
                } else {
                    let _ = cabac.read_raw_byte();
                }
            }
        }
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, true);
                self.set_i4x4_mode(mb_x * 4 + sub_x, mb_y * 4 + sub_y, 2);
            }
        }
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                self.set_luma_8x8_cbf(mb_x * 2 + sub_x, mb_y * 2 + sub_y, true);
            }
        }

        let cx0 = mb_x * 8;
        let cy0 = mb_y * 8;
        for plane in [&mut self.ref_u, &mut self.ref_v] {
            for dy in 0..8 {
                for dx in 0..8 {
                    let idx = (cy0 + dy) * self.stride_c + cx0 + dx;
                    if idx < plane.len() {
                        plane[idx] = cabac.read_raw_byte();
                    } else {
                        let _ = cabac.read_raw_byte();
                    }
                }
            }
        }
        self.set_chroma_dc_u_cbf(mb_x, mb_y, true);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, true);
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, true);
                self.set_chroma_v_cbf(x2, y2, true);
            }
        }
        let ipcm_restart_ptr_adjust = std::env::var("TAO_H264_IPCM_RESTART_PTR_ADJUST")
            .ok()
            .and_then(|v| v.parse::<isize>().ok())
            .unwrap_or(0);
        if ipcm_restart_ptr_adjust != 0 {
            cabac.adjust_raw_pos(ipcm_restart_ptr_adjust);
            if debug_mb {
                eprintln!(
                    "[H264][I_PCM] 应用 TAO_H264_IPCM_RESTART_PTR_ADJUST={} 后 raw_pos={}",
                    ipcm_restart_ptr_adjust,
                    cabac.raw_pos()
                );
            }
        }
        if debug_mb {
            eprintln!(
                "[H264][I_PCM] 重启前: bytestream_pos={}, raw_pos={}, low=0x{:08x}",
                cabac.bytestream_pos(),
                cabac.raw_pos(),
                cabac.low()
            );
        }
        // I_PCM 后需要重启 CABAC 引擎继续解码后续宏块.
        cabac.restart_engine();
    }

    /// 解码 I_4x4 宏块的残差并应用到预测上
    fn decode_i4x4_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_pos: (usize, usize),
        qp: i32,
        pred_modes: &[u8; 16],
    ) {
        let (mb_x, mb_y) = mb_pos;
        // 先清空当前宏块的 luma CBF 状态.
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }

        // 亮度: 按规范顺序逐块重建, 保证后续块可引用“已重建”邻居样本.
        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let has_residual_8x8 = luma_cbp & (1 << i8x8) != 0;
            let mut coded_8x8 = false;

            for i_sub in 0..4 {
                let sub_x = i_sub & 1;
                let sub_y = i_sub >> 1;
                let abs_sub_x = x8x8 * 2 + sub_x;
                let abs_sub_y = y8x8 * 2 + sub_y;

                let px = mb_x * 16 + abs_sub_x * 4;
                let py = mb_y * 16 + abs_sub_y * 4;
                let x4 = mb_x * 4 + abs_sub_x;
                let y4 = mb_y * 4 + abs_sub_y;

                let mode = pred_modes[abs_sub_y * 4 + abs_sub_x];
                intra::predict_4x4(&mut self.ref_y, self.stride_y, px, py, mode);

                if !has_residual_8x8 {
                    self.set_luma_cbf(x4, y4, false);
                    continue;
                }

                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, true);
                let mut raw_coeffs =
                    decode_residual_block(cabac, ctxs, &residual::CAT_LUMA_4X4, cbf_inc);
                let coded = raw_coeffs.iter().any(|&c| c != 0);
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    coded_8x8 = true;
                }

                while raw_coeffs.len() < 16 {
                    raw_coeffs.push(0);
                }

                let mut coeffs_arr = [0i32; 16];
                coeffs_arr.copy_from_slice(&raw_coeffs[..16]);
                residual::dequant_4x4_ac(&mut coeffs_arr, qp);
                residual::apply_4x4_ac_residual(
                    &mut self.ref_y,
                    self.stride_y,
                    px,
                    py,
                    &coeffs_arr,
                );
            }
            self.set_luma_8x8_cbf(mb_x * 2 + x8x8, mb_y * 2 + y8x8, coded_8x8);
        }
    }

    /// I_8x8 残差最小可用路径: 按 8x8 块消耗语法并近似应用到 4x4 子块.
    #[allow(clippy::too_many_arguments)]
    fn decode_i8x8_residual_fallback(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        mb_x: usize,
        mb_y: usize,
        qp: i32,
        intra_defaults: bool,
    ) {
        let skip_8x8_cbf = std::env::var("TAO_H264_8X8_SKIP_CBF")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            // 对齐 FFmpeg 在 4:2:0 下的默认路径: cat=5 跳过 coded_block_flag.
            .unwrap_or(true);
        let block_cat = if skip_8x8_cbf {
            &CAT_LUMA_8X8_FALLBACK
        } else {
            &CAT_LUMA_8X8
        };

        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);

        for i8x8 in 0..4u8 {
            let x8x8 = (i8x8 & 1) as usize;
            let y8x8 = (i8x8 >> 1) as usize;
            let x8 = mb_x * 2 + x8x8;
            let y8 = mb_y * 2 + y8x8;
            if luma_cbp & (1 << i8x8) == 0 {
                self.set_luma_8x8_cbf(x8, y8, false);
                continue;
            }
            let x4 = mb_x * 4 + x8x8 * 2;
            let y4 = mb_y * 4 + y8x8 * 2;
            let cbf_inc = self.luma_8x8_cbf_ctx_inc(x8, y8, intra_defaults);
            let mut raw_coeffs = decode_residual_block(cabac, ctxs, block_cat, cbf_inc);
            let coded = raw_coeffs.iter().any(|&c| c != 0);
            self.set_luma_8x8_cbf(x8, y8, coded);
            // 对齐 FFmpeg: 8x8 变换块会把非零计数写回 2x2 子块缓存.
            // CABAC 上下文只依赖“是否非零”, 因此四个子块统一使用 8x8 的 coded 状态.
            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    self.set_luma_cbf(x4 + sub_x, y4 + sub_y, coded);
                }
            }
            while raw_coeffs.len() < 64 {
                raw_coeffs.push(0);
            }
            let mut coeffs_raster_8x8 = [0i32; 64];
            for (scan_pos, &raster_idx) in residual::ZIGZAG_8X8.iter().enumerate() {
                coeffs_raster_8x8[raster_idx] = raw_coeffs[scan_pos];
            }

            for sub_y in 0..2 {
                for sub_x in 0..2 {
                    let mut coeffs_arr = [0i32; 16];
                    for (scan_pos, &(row, col)) in residual::ZIGZAG_4X4.iter().enumerate() {
                        let r8 = sub_y * 4 + row;
                        let c8 = sub_x * 4 + col;
                        coeffs_arr[scan_pos] = coeffs_raster_8x8[r8 * 8 + c8];
                    }
                    residual::dequant_4x4_ac(&mut coeffs_arr, qp);

                    let px = mb_x * 16 + x8x8 * 8 + sub_x * 4;
                    let py = mb_y * 16 + y8x8 * 8 + sub_y * 4;
                    residual::apply_4x4_ac_residual(
                        &mut self.ref_y,
                        self.stride_y,
                        px,
                        py,
                        &coeffs_arr,
                    );
                }
            }
        }
    }

    /// 解码 I_16x16 宏块 (预测 + 残差)
    fn decode_i_16x16_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        mb_type: u32,
        cur_qp: &mut i32,
    ) {
        let debug_mb = std::env::var("TAO_H264_DEBUG_MB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let mb_idx = mb_y * self.mb_width + mb_x;
        self.reset_chroma_cbf_mb(mb_x, mb_y);
        self.set_luma_dc_cbf(mb_x, mb_y, false);
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        self.set_transform_8x8_flag(mb_x, mb_y, false);
        // I_16x16 预测模式: mb_type 后缀 2bit 直接映射到 0..3.
        const I16_PRED_MODE_MAP: [u8; 4] = [0, 1, 2, 3];
        let pred_mode = I16_PRED_MODE_MAP[((mb_type - 1) % 4) as usize];
        let cbp_chroma = ((mb_type - 1) / 4) % 3;
        let cbp_luma_nz = (mb_type - 1) >= 12;
        let cbp_luma = if cbp_luma_nz { 0x0f } else { 0x00 };
        self.set_mb_cbp(mb_x, mb_y, cbp_luma | ((cbp_chroma as u8) << 4));
        if debug_mb && debug_mb_selected(mb_idx, mb_x, mb_y) {
            eprintln!(
                "[H264][I16x16] mb=({}, {}), mb_type={}, pred_mode={}, cbp_luma=0x{:x}, cbp_chroma={}",
                mb_x, mb_y, mb_type, pred_mode, cbp_luma, cbp_chroma
            );
        }

        // 1. 解码 intra_chroma_pred_mode (消耗 CABAC 比特)
        let chroma_mode = self.decode_chroma_pred_mode(cabac, ctxs, mb_x, mb_y);
        self.set_chroma_pred_mode(mb_x, mb_y, chroma_mode);

        // 2. mb_qp_delta (I_16x16 始终存在)
        let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
        self.prev_qp_delta_nz = qp_delta != 0;
        *cur_qp = wrap_qp((*cur_qp + qp_delta) as i64);

        // 3. 应用亮度预测
        intra::predict_16x16(
            &mut self.ref_y,
            self.stride_y,
            mb_x * 16,
            mb_y * 16,
            pred_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 4. 应用色度预测 (DC)
        intra::predict_chroma_8x8(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_8x8(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            chroma_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 5. 亮度残差 (DC 始终存在, AC 按 mb_type 的 CBP 决定)
        let dc_coeffs = self.decode_luma_dc_coeffs(cabac, ctxs, mb_x, mb_y, *cur_qp);
        self.decode_i16x16_luma_residual(
            cabac,
            ctxs,
            (mb_x, mb_y),
            *cur_qp,
            &dc_coeffs,
            cbp_luma_nz,
        );

        // 6. 色度残差
        if cbp_chroma >= 1 {
            self.decode_chroma_residual(cabac, ctxs, (mb_x, mb_y), *cur_qp, cbp_chroma >= 2, true);
        }
    }

    /// 解码 I_16x16 亮度 DC 残差
    fn decode_luma_dc_coeffs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        slice_qp: i32,
    ) -> [i32; 16] {
        // 解码 DC 系数
        let cbf_inc = self.get_dc_cbf_inc(mb_x, mb_y, true);
        let raw_coeffs = decode_residual_block(cabac, ctxs, &CAT_LUMA_DC, cbf_inc);
        self.set_luma_dc_cbf(mb_x, mb_y, raw_coeffs.iter().any(|&c| c != 0));

        // 反扫描 + 反 Hadamard + 反量化
        let mut dc_block = [0i32; 16];
        for (scan_pos, &(row, col)) in residual::ZIGZAG_4X4.iter().enumerate() {
            if let Some(&c) = raw_coeffs.get(scan_pos) {
                dc_block[row * 4 + col] = c;
            }
        }
        inverse_hadamard_4x4(&mut dc_block);
        dequant_luma_dc(&mut dc_block, slice_qp);
        dc_block
    }

    /// 解码并应用 I_16x16 的亮度残差
    fn decode_i16x16_luma_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_pos: (usize, usize),
        qp: i32,
        dc_coeffs: &[i32; 16],
        has_luma_ac: bool,
    ) {
        let (mb_x, mb_y) = mb_pos;
        // 对齐 FFmpeg scan8 的 i4x4 索引顺序:
        // i4x4=0..15 对应 8x8 分组遍历, 而非纯行优先遍历.
        const I4X4_SCAN_ORDER: [(usize, usize); 16] = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (3, 0),
            (2, 1),
            (3, 1),
            (0, 2),
            (1, 2),
            (0, 3),
            (1, 3),
            (2, 2),
            (3, 2),
            (2, 3),
            (3, 3),
        ];

        // I_16x16 AC 的 CBF 按 4x4 子块追踪.
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                self.set_luma_cbf(mb_x * 4 + sub_x, mb_y * 4 + sub_y, false);
            }
        }
        self.reset_luma_8x8_cbf_mb(mb_x, mb_y);
        let mut coded_8x8 = [false; 4];

        for &(sub_x, sub_y) in &I4X4_SCAN_ORDER {
            let block_idx = sub_y * 4 + sub_x;
            let mut coeffs_scan = [0i32; 16];
            let x4 = mb_x * 4 + sub_x;
            let y4 = mb_y * 4 + sub_y;
            if has_luma_ac {
                let cbf_inc = self.luma_cbf_ctx_inc(x4, y4, true);
                let raw_ac = decode_residual_block(cabac, ctxs, &CAT_LUMA_AC, cbf_inc);
                let coded = raw_ac.iter().any(|&c| c != 0);
                self.set_luma_cbf(x4, y4, coded);
                if coded {
                    let idx8 = (sub_y / 2) * 2 + (sub_x / 2);
                    coded_8x8[idx8] = true;
                }
                for (scan, &c) in raw_ac.iter().enumerate().take(15) {
                    coeffs_scan[scan + 1] = c;
                }
            } else {
                self.set_luma_cbf(x4, y4, false);
            }
            residual::dequant_4x4_ac(&mut coeffs_scan, qp);
            coeffs_scan[0] = dc_coeffs[block_idx];

            let px = mb_x * 16 + sub_x * 4;
            let py = mb_y * 16 + sub_y * 4;
            residual::apply_4x4_ac_residual(&mut self.ref_y, self.stride_y, px, py, &coeffs_scan);
        }

        for (idx8, coded) in coded_8x8.iter().copied().enumerate() {
            let x8 = idx8 & 1;
            let y8 = idx8 >> 1;
            self.set_luma_8x8_cbf(mb_x * 2 + x8, mb_y * 2 + y8, coded);
        }
    }

    /// 解码并应用色度残差
    fn decode_chroma_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_pos: (usize, usize),
        slice_qp: i32,
        has_chroma_ac: bool,
        intra_defaults: bool,
    ) {
        let (mb_x, mb_y) = mb_pos;
        // 色度 QP 映射(按 PPS 中的 Cb/Cr 偏移分别计算).
        let (chroma_off_u, chroma_off_v) = self
            .pps
            .as_ref()
            .map(|p| (p.chroma_qp_index_offset, p.second_chroma_qp_index_offset))
            .unwrap_or((0, 0));
        let chroma_qp_u = chroma_qp_from_luma_with_offset(slice_qp, chroma_off_u);
        let chroma_qp_v = chroma_qp_from_luma_with_offset(slice_qp, chroma_off_v);

        // U 通道
        let chroma_dc_cbf_inc_u = self.chroma_dc_cbf_ctx_inc(mb_x, mb_y, intra_defaults);
        let u_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, chroma_dc_cbf_inc_u);
        self.set_chroma_dc_u_cbf(mb_x, mb_y, u_coeffs.iter().any(|&c| c != 0));
        let mut u_dc = [0i32; 4];
        for (i, &c) in u_coeffs.iter().enumerate().take(4) {
            u_dc[i] = c;
        }
        inverse_hadamard_2x2(&mut u_dc);
        dequant_chroma_dc(&mut u_dc, chroma_qp_u);

        // V 通道
        let chroma_dc_cbf_inc_v = self.chroma_dc_v_cbf_ctx_inc(mb_x, mb_y, intra_defaults);
        let v_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, chroma_dc_cbf_inc_v);
        self.set_chroma_dc_v_cbf(mb_x, mb_y, v_coeffs.iter().any(|&c| c != 0));
        let mut v_dc = [0i32; 4];
        for (i, &c) in v_coeffs.iter().enumerate().take(4) {
            v_dc[i] = c;
        }
        inverse_hadamard_2x2(&mut v_dc);
        dequant_chroma_dc(&mut v_dc, chroma_qp_v);

        // H.264 语法顺序: 先完整解码 U 的 4 个 AC 块, 再完整解码 V 的 4 个 AC 块.
        let mut u_scans = [[0i32; 16]; 4];
        let mut v_scans = [[0i32; 16]; 4];

        if has_chroma_ac {
            for (block_idx, u_scan) in u_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;

                let cbf_inc_u = self.chroma_u_cbf_ctx_inc(x2, y2, intra_defaults);
                let raw_u_ac = decode_residual_block(cabac, ctxs, &CAT_CHROMA_AC, cbf_inc_u);
                let coded_u = raw_u_ac.iter().any(|&c| c != 0);
                self.set_chroma_u_cbf(x2, y2, coded_u);
                for (scan, &c) in raw_u_ac.iter().enumerate().take(15) {
                    u_scan[scan + 1] = c;
                }
            }
            for (block_idx, v_scan) in v_scans.iter_mut().enumerate() {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;

                let cbf_inc_v = self.chroma_v_cbf_ctx_inc(x2, y2, intra_defaults);
                let raw_v_ac = decode_residual_block(cabac, ctxs, &CAT_CHROMA_AC, cbf_inc_v);
                let coded_v = raw_v_ac.iter().any(|&c| c != 0);
                self.set_chroma_v_cbf(x2, y2, coded_v);
                for (scan, &c) in raw_v_ac.iter().enumerate().take(15) {
                    v_scan[scan + 1] = c;
                }
            }
        } else {
            for block_idx in 0..4usize {
                let sub_x = block_idx & 1;
                let sub_y = block_idx >> 1;
                let x2 = mb_x * 2 + sub_x;
                let y2 = mb_y * 2 + sub_y;
                self.set_chroma_u_cbf(x2, y2, false);
                self.set_chroma_v_cbf(x2, y2, false);
            }
        }

        // 应用到色度平面: 每个 4x4 子块独立重建 (DC + AC)
        for block_idx in 0..4usize {
            let sub_x = block_idx & 1;
            let sub_y = block_idx >> 1;
            let px = mb_x * 8 + sub_x * 4;
            let py = mb_y * 8 + sub_y * 4;

            let mut u_scan = u_scans[block_idx];
            residual::dequant_4x4_ac(&mut u_scan, chroma_qp_u);
            u_scan[0] = u_dc[block_idx];
            residual::apply_4x4_ac_residual(&mut self.ref_u, self.stride_c, px, py, &u_scan);

            let mut v_scan = v_scans[block_idx];
            residual::dequant_4x4_ac(&mut v_scan, chroma_qp_v);
            v_scan[0] = v_dc[block_idx];
            residual::apply_4x4_ac_residual(&mut self.ref_v, self.stride_c, px, py, &v_scan);
        }
    }

    /// 获取 DC coded_block_flag 的上下文增量
    fn get_dc_cbf_inc(&self, mb_x: usize, mb_y: usize, intra_defaults: bool) -> usize {
        let left = if mb_x > 0 {
            usize::from(self.get_luma_dc_cbf(mb_x - 1, mb_y))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        let top = if mb_y > 0 {
            usize::from(self.get_luma_dc_cbf(mb_x, mb_y - 1))
        } else if intra_defaults {
            1usize
        } else {
            0usize
        };
        left + (top << 1)
    }
}

// ============================================================
// CABAC 语法元素解码
// ============================================================

/// 解码 mb_qp_delta (一元编码)
fn decode_qp_delta(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx], prev_nz: bool) -> i32 {
    const MAX_QP: u32 = 51;
    let mut ctx_idx = if prev_nz { 1usize } else { 0 };
    let mut val = 0u32;

    while cabac.decode_decision(&mut ctxs[60 + ctx_idx]) == 1 {
        ctx_idx = 2 + (ctx_idx >> 1);
        val += 1;
        if val > 2 * MAX_QP {
            break;
        }
    }

    match val {
        0 => 0,
        v if v & 1 == 1 => v.div_ceil(2) as i32,
        v => -(v.div_ceil(2) as i32),
    }
}

/// 解码 I-slice 宏块类型
fn decode_i_mb_type(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> u32 {
    decode_intra_mb_type(cabac, ctxs, 3, true, mb_types, mb_width, mb_x, mb_y)
}

/// 通用 Intra 宏块类型解码.
#[allow(clippy::too_many_arguments)]
fn decode_intra_mb_type(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    ctx_base: usize,
    intra_slice: bool,
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> u32 {
    let mut state_base = ctx_base;
    if intra_slice {
        let ctx_inc = compute_mb_type_ctx_inc(mb_types, mb_width, mb_x, mb_y);
        let bin0 = cabac.decode_decision(&mut ctxs[state_base + ctx_inc]);
        if bin0 == 0 {
            return 0;
        }
        state_base += 2;
    } else if cabac.decode_decision(&mut ctxs[state_base]) == 0 {
        return 0;
    }

    let skip_ipcm_check = std::env::var("TAO_H264_SKIP_IPCM_CHECK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !skip_ipcm_check && cabac.decode_terminate() == 1 {
        let force_i16 = std::env::var("TAO_H264_FORCE_NO_IPCM")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !force_i16 {
            return 25;
        }
    }

    decode_i_16x16_suffix_with_base(cabac, ctxs, state_base, intra_slice)
}

/// 计算 mb_type 前缀的上下文增量
fn compute_mb_type_ctx_inc(mb_types: &[u8], mb_width: usize, mb_x: usize, mb_y: usize) -> usize {
    let left_not_i4x4 = if mb_x > 0 {
        mb_types[mb_y * mb_width + mb_x - 1] != 0
    } else {
        false
    };
    let top_not_i4x4 = if mb_y > 0 {
        mb_types[(mb_y - 1) * mb_width + mb_x] != 0
    } else {
        false
    };
    left_not_i4x4 as usize + top_not_i4x4 as usize
}

/// 按上下文基址解码 I_16x16 后缀.
fn decode_i_16x16_suffix_with_base(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    state_base: usize,
    intra_slice: bool,
) -> u32 {
    let intra = usize::from(intra_slice);
    let cbp_luma = cabac.decode_decision(&mut ctxs[state_base + 1]);
    let cbp_c0 = cabac.decode_decision(&mut ctxs[state_base + 2]);
    let cbp_chroma = if cbp_c0 == 0 {
        0
    } else {
        let cbp_c1 = cabac.decode_decision(&mut ctxs[state_base + 2 + intra]);
        1 + cbp_c1
    };
    let pm0 = cabac.decode_decision(&mut ctxs[state_base + 3 + intra]);
    let pm1 = cabac.decode_decision(&mut ctxs[state_base + 3 + intra * 2]);
    let pred_mode = pm0 * 2 + pm1;
    1 + pred_mode + 4 * cbp_chroma + 12 * cbp_luma
}

// ============================================================
// avcC 配置解析
// ============================================================

impl H264Decoder {
    fn parse_sps_pps_from_config(
        &mut self,
        config: &crate::parsers::h264::AvccConfig,
    ) -> TaoResult<()> {
        for sps_data in &config.sps_list {
            if let Ok(nalu) = NalUnit::parse(sps_data) {
                self.handle_sps(&nalu);
            }
        }
        for pps_data in &config.pps_list {
            if let Ok(nalu) = NalUnit::parse(pps_data) {
                self.handle_pps(&nalu);
            }
        }
        Ok(())
    }
}

// ============================================================
// 输出帧构建
// ============================================================

impl H264Decoder {
    fn zero_reference_planes(&self) -> RefPlanes {
        RefPlanes {
            y: vec![128u8; self.ref_y.len()],
            u: vec![128u8; self.ref_u.len()],
            v: vec![128u8; self.ref_v.len()],
            poc: self.last_poc,
        }
    }

    fn max_frame_num_modulo(&self) -> u32 {
        let shift = self
            .sps
            .as_ref()
            .map(|s| s.log2_max_frame_num)
            .unwrap_or(4)
            .min(31);
        1u32 << shift
    }

    fn frame_num_backward_distance(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        let cur = self.last_frame_num % max;
        let target = frame_num % max;
        let dist = (cur + max - target) % max;
        if dist == 0 { max } else { dist }
    }

    fn frame_num_forward_distance(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        let cur = self.last_frame_num % max;
        let target = frame_num % max;
        let dist = (target + max - cur) % max;
        if dist == 0 { max } else { dist }
    }

    fn pic_num_subtract(&self, pic_num: u32, sub: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        (pic_num + max - (sub % max)) % max
    }

    fn pic_num_from_frame_num(&self, frame_num: u32) -> u32 {
        let max = self.max_frame_num_modulo();
        if max == 0 {
            return 0;
        }
        frame_num % max
    }

    fn short_term_references(&self) -> Vec<&ReferencePicture> {
        self.reference_frames
            .iter()
            .filter(|pic| pic.long_term_frame_idx.is_none())
            .collect()
    }

    fn long_term_references(&self) -> Vec<&ReferencePicture> {
        self.reference_frames
            .iter()
            .filter(|pic| pic.long_term_frame_idx.is_some())
            .collect()
    }

    fn reference_to_planes(pic: &ReferencePicture) -> RefPlanes {
        RefPlanes {
            y: pic.y.clone(),
            u: pic.u.clone(),
            v: pic.v.clone(),
            poc: pic.poc,
        }
    }

    fn collect_default_reference_list_l0(&self) -> Vec<&ReferencePicture> {
        if self.last_slice_type == 1 {
            let cur_poc = self.last_poc;
            let short_refs = self.short_term_references();
            let mut before: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc < cur_poc)
                .collect();
            let mut after: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc >= cur_poc)
                .collect();
            before.sort_by_key(|pic| std::cmp::Reverse(pic.poc));
            after.sort_by_key(|pic| pic.poc);
            let mut refs = before;
            refs.extend(after);
            let mut long_refs = self.long_term_references();
            long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
            refs.extend(long_refs);
            return refs;
        }

        let mut short_refs = self.short_term_references();
        short_refs.sort_by_key(|pic| {
            (
                self.frame_num_backward_distance(pic.frame_num),
                self.frame_num_forward_distance(pic.frame_num),
            )
        });
        let mut refs = short_refs;
        let mut long_refs = self.long_term_references();
        long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
        refs.extend(long_refs);
        refs
    }

    fn collect_default_reference_list_l1(&self) -> Vec<&ReferencePicture> {
        if self.last_slice_type == 1 {
            let cur_poc = self.last_poc;
            let short_refs = self.short_term_references();
            let mut after: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc > cur_poc)
                .collect();
            let mut before: Vec<&ReferencePicture> = short_refs
                .iter()
                .copied()
                .filter(|pic| pic.poc <= cur_poc)
                .collect();
            after.sort_by_key(|pic| pic.poc);
            before.sort_by_key(|pic| std::cmp::Reverse(pic.poc));
            let mut refs = after;
            refs.extend(before);
            let mut long_refs = self.long_term_references();
            long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
            refs.extend(long_refs);
            return refs;
        }

        let mut short_refs = self.short_term_references();
        short_refs.sort_by_key(|pic| {
            (
                self.frame_num_forward_distance(pic.frame_num),
                self.frame_num_backward_distance(pic.frame_num),
            )
        });
        let mut refs = short_refs;
        let mut long_refs = self.long_term_references();
        long_refs.sort_by_key(|pic| pic.long_term_frame_idx.unwrap_or(u32::MAX));
        refs.extend(long_refs);
        refs
    }

    fn short_term_pic_num_from_ref(
        &self,
        ref_frame_num: u32,
        cur_pic_num: i32,
        max_frame_num: i32,
    ) -> i32 {
        let frame_num = (ref_frame_num % (max_frame_num as u32)) as i32;
        if frame_num > cur_pic_num {
            frame_num - max_frame_num
        } else {
            frame_num
        }
    }

    fn find_short_term_ref_index_by_pic_num(
        &self,
        refs: &[&ReferencePicture],
        pic_num: i32,
        cur_pic_num: i32,
        max_frame_num: i32,
    ) -> Option<usize> {
        refs.iter().position(|pic| {
            pic.long_term_frame_idx.is_none()
                && self.short_term_pic_num_from_ref(pic.frame_num, cur_pic_num, max_frame_num)
                    == pic_num
        })
    }

    fn apply_ref_pic_list_modifications(
        &self,
        refs: &mut Vec<&ReferencePicture>,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) {
        if mods.is_empty() || refs.is_empty() {
            return;
        }
        let max_frame_num = self.max_frame_num_modulo() as i32;
        if max_frame_num <= 0 {
            return;
        }

        let cur_pic_num = cur_frame_num as i32;
        let mut pic_num_pred = cur_pic_num;
        let mut insert_idx = 0usize;

        for &m in mods {
            let target_idx = match m {
                RefPicListMod::ShortTermSub {
                    abs_diff_pic_num_minus1,
                } => {
                    let diff = abs_diff_pic_num_minus1 as i32 + 1;
                    let mut pic_num_no_wrap = pic_num_pred - diff;
                    if pic_num_no_wrap < 0 {
                        pic_num_no_wrap += max_frame_num;
                    }
                    pic_num_pred = pic_num_no_wrap;
                    let pic_num = if pic_num_no_wrap > cur_pic_num {
                        pic_num_no_wrap - max_frame_num
                    } else {
                        pic_num_no_wrap
                    };
                    self.find_short_term_ref_index_by_pic_num(
                        refs.as_slice(),
                        pic_num,
                        cur_pic_num,
                        max_frame_num,
                    )
                }
                RefPicListMod::ShortTermAdd {
                    abs_diff_pic_num_minus1,
                } => {
                    let diff = abs_diff_pic_num_minus1 as i32 + 1;
                    let mut pic_num_no_wrap = pic_num_pred + diff;
                    if pic_num_no_wrap >= max_frame_num {
                        pic_num_no_wrap -= max_frame_num;
                    }
                    pic_num_pred = pic_num_no_wrap;
                    let pic_num = if pic_num_no_wrap > cur_pic_num {
                        pic_num_no_wrap - max_frame_num
                    } else {
                        pic_num_no_wrap
                    };
                    self.find_short_term_ref_index_by_pic_num(
                        refs.as_slice(),
                        pic_num,
                        cur_pic_num,
                        max_frame_num,
                    )
                }
                RefPicListMod::LongTerm { long_term_pic_num } => refs
                    .iter()
                    .position(|pic| pic.long_term_frame_idx == Some(long_term_pic_num)),
            };

            if let Some(src_idx) = target_idx {
                let selected = refs.remove(src_idx);
                let dst_idx = insert_idx.min(refs.len());
                refs.insert(dst_idx, selected);
                insert_idx += 1;
            }
        }
    }

    fn build_reference_list_l0_with_mod(
        &self,
        count: u32,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) -> Vec<RefPlanes> {
        let target = count.max(1) as usize;
        let mut refs = self.collect_default_reference_list_l0();
        self.apply_ref_pic_list_modifications(&mut refs, mods, cur_frame_num);
        let mut out = Vec::with_capacity(target);
        for rank in 0..target {
            if let Some(pic) = refs.get(rank).copied().or_else(|| refs.first().copied()) {
                out.push(Self::reference_to_planes(pic));
            } else {
                out.push(self.zero_reference_planes());
            }
        }
        out
    }

    fn build_reference_list_l1_with_mod(
        &self,
        count: u32,
        mods: &[RefPicListMod],
        cur_frame_num: u32,
    ) -> Vec<RefPlanes> {
        let target = count.max(1) as usize;
        let mut refs = self.collect_default_reference_list_l1();
        self.apply_ref_pic_list_modifications(&mut refs, mods, cur_frame_num);
        let mut out = Vec::with_capacity(target);
        for rank in 0..target {
            if let Some(pic) = refs.get(rank).copied().or_else(|| refs.first().copied()) {
                out.push(Self::reference_to_planes(pic));
            } else {
                out.push(self.zero_reference_planes());
            }
        }
        out
    }

    fn remove_short_term_by_pic_num(&mut self, pic_num: u32) -> bool {
        let pos = self.reference_frames.iter().rposition(|pic| {
            pic.long_term_frame_idx.is_none()
                && self.pic_num_from_frame_num(pic.frame_num) == pic_num
        });
        if let Some(idx) = pos {
            self.reference_frames.remove(idx);
            return true;
        }
        false
    }

    fn remove_long_term_by_idx(&mut self, long_term_frame_idx: u32) -> bool {
        let pos = self
            .reference_frames
            .iter()
            .position(|pic| pic.long_term_frame_idx == Some(long_term_frame_idx));
        if let Some(idx) = pos {
            self.reference_frames.remove(idx);
            return true;
        }
        false
    }

    fn trim_long_term_references(&mut self) {
        let Some(max_idx) = self.max_long_term_frame_idx else {
            self.reference_frames
                .retain(|pic| pic.long_term_frame_idx.is_none());
            return;
        };
        self.reference_frames
            .retain(|pic| pic.long_term_frame_idx.is_none_or(|idx| idx <= max_idx));
    }

    fn enforce_reference_capacity(&mut self) {
        while self.reference_frames.len() > self.max_reference_frames {
            if let Some(idx) = self
                .reference_frames
                .iter()
                .position(|pic| pic.long_term_frame_idx.is_none())
            {
                self.reference_frames.remove(idx);
            } else {
                let _ = self.reference_frames.pop_front();
            }
        }
    }

    fn push_current_reference(&mut self, long_term_frame_idx: Option<u32>) {
        if self.last_nal_ref_idc == 0 {
            return;
        }
        if self.last_slice_type == 1 {
            return;
        }
        self.reference_frames.push_back(ReferencePicture {
            y: self.ref_y.clone(),
            u: self.ref_u.clone(),
            v: self.ref_v.clone(),
            frame_num: self.last_frame_num,
            poc: self.last_poc,
            long_term_frame_idx,
        });
    }

    fn store_reference_with_marking(&mut self) {
        if self.last_nal_ref_idc == 0 || self.last_slice_type == 1 {
            return;
        }

        let marking = self.last_dec_ref_pic_marking.clone();
        let mut current_long_term_idx = None;

        if marking.is_idr {
            if marking.no_output_of_prior_pics {
                self.output_queue.clear();
                self.reorder_buffer.clear();
            }
            self.reference_frames.clear();
            if marking.long_term_reference_flag {
                self.max_long_term_frame_idx = Some(0);
                current_long_term_idx = Some(0);
            } else {
                self.max_long_term_frame_idx = None;
            }
        } else if marking.adaptive {
            for op in marking.ops {
                match op {
                    MmcoOp::ForgetShort {
                        difference_of_pic_nums_minus1,
                    } => {
                        let pic_num_x = self.pic_num_subtract(
                            self.pic_num_from_frame_num(self.last_frame_num),
                            difference_of_pic_nums_minus1 + 1,
                        );
                        let _ = self.remove_short_term_by_pic_num(pic_num_x);
                    }
                    MmcoOp::ForgetLong { long_term_pic_num } => {
                        let _ = self.remove_long_term_by_idx(long_term_pic_num);
                    }
                    MmcoOp::ConvertShortToLong {
                        difference_of_pic_nums_minus1,
                        long_term_frame_idx,
                    } => {
                        let pic_num_x = self.pic_num_subtract(
                            self.pic_num_from_frame_num(self.last_frame_num),
                            difference_of_pic_nums_minus1 + 1,
                        );
                        let _ = self.remove_long_term_by_idx(long_term_frame_idx);
                        if let Some(pos) = self.reference_frames.iter().rposition(|pic| {
                            pic.long_term_frame_idx.is_none()
                                && self.pic_num_from_frame_num(pic.frame_num) == pic_num_x
                        }) && let Some(pic) = self.reference_frames.get_mut(pos)
                        {
                            pic.long_term_frame_idx = Some(long_term_frame_idx);
                        }
                    }
                    MmcoOp::TrimLong {
                        max_long_term_frame_idx_plus1,
                    } => {
                        self.max_long_term_frame_idx = max_long_term_frame_idx_plus1.checked_sub(1);
                        self.trim_long_term_references();
                    }
                    MmcoOp::ClearAll => {
                        self.reference_frames.clear();
                        self.max_long_term_frame_idx = None;
                        self.prev_ref_poc_msb = 0;
                        self.prev_ref_poc_lsb = 0;
                        self.prev_frame_num_offset_type1 = 0;
                        self.prev_frame_num_offset_type2 = 0;
                    }
                    MmcoOp::MarkCurrentLong {
                        long_term_frame_idx,
                    } => {
                        current_long_term_idx = Some(long_term_frame_idx);
                    }
                }
            }
        }

        if let Some(idx) = current_long_term_idx {
            if self.max_long_term_frame_idx.is_none_or(|max| idx <= max) {
                let _ = self.remove_long_term_by_idx(idx);
                self.push_current_reference(Some(idx));
            } else {
                self.push_current_reference(None);
            }
        } else {
            self.push_current_reference(None);
        }
        self.enforce_reference_capacity();
    }

    fn push_video_for_output(&mut self, vf: VideoFrame, poc: i32) {
        let entry = ReorderFrameEntry {
            frame: vf,
            poc,
            decode_order: self.decode_order_counter,
        };
        self.decode_order_counter = self.decode_order_counter.wrapping_add(1);
        let insert_pos = self.reorder_buffer.partition_point(|cur| {
            cur.poc < entry.poc || (cur.poc == entry.poc && cur.decode_order <= entry.decode_order)
        });
        self.reorder_buffer.insert(insert_pos, entry);
        while self.reorder_buffer.len() > self.reorder_depth {
            let out = self.reorder_buffer.remove(0);
            self.output_queue.push_back(Frame::Video(out.frame));
        }
    }

    fn drain_reorder_buffer_to_output(&mut self) {
        while !self.reorder_buffer.is_empty() {
            let out = self.reorder_buffer.remove(0);
            self.output_queue.push_back(Frame::Video(out.frame));
        }
    }

    fn build_output_frame(&mut self, pts: i64, time_base: Rational, is_keyframe: bool) {
        let w = self.width as usize;
        let h = self.height as usize;

        if self.last_disable_deblocking_filter_idc != 1 {
            deblock::apply_simple_deblock_yuv420(
                &mut self.ref_y,
                &mut self.ref_u,
                &mut self.ref_v,
                self.stride_y,
                self.stride_c,
                w,
                h,
            );
        }

        let y_data = copy_plane(&self.ref_y, self.stride_y, w, h);
        let u_data = copy_plane(&self.ref_u, self.stride_c, w / 2, h / 2);
        let v_data = copy_plane(&self.ref_v, self.stride_c, w / 2, h / 2);

        let picture_type = match self.last_slice_type {
            1 => PictureType::B,
            2 | 4 => PictureType::I,
            0 | 3 => PictureType::P,
            _ => {
                if is_keyframe {
                    PictureType::I
                } else {
                    PictureType::P
                }
            }
        };

        let vf = VideoFrame {
            data: vec![y_data, u_data, v_data],
            linesize: vec![w, w / 2, w / 2],
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Yuv420p,
            pts,
            time_base,
            duration: 0,
            is_keyframe,
            picture_type,
            sample_aspect_ratio: Rational::new(1, 1),
            color_space: Default::default(),
            color_range: Default::default(),
        };
        let frame_poc = self.last_poc;
        self.store_reference_with_marking();
        self.push_video_for_output(vf, frame_poc);
    }
}

// ============================================================
// 工具函数
// ============================================================

fn median3(a: i32, b: i32, c: i32) -> i32 {
    let mut vals = [a, b, c];
    vals.sort_unstable();
    vals[1]
}

fn floor_div(v: i32, d: i32) -> i32 {
    let mut q = v / d;
    let r = v % d;
    if r != 0 && ((r > 0) != (d > 0)) {
        q -= 1;
    }
    q
}

fn mod_floor(v: i32, d: i32) -> i32 {
    let r = v % d;
    if r < 0 { r + d } else { r }
}

fn p_l0_weight(weights: &[PredWeightL0], ref_idx: u32) -> Option<&PredWeightL0> {
    usize::try_from(ref_idx)
        .ok()
        .and_then(|idx| weights.get(idx))
}

fn select_ref_planes(ref_list: &[RefPlanes], ref_idx: i8) -> &RefPlanes {
    let Some(first) = ref_list.first() else {
        return &EMPTY_REF_PLANES;
    };
    let idx = if ref_idx <= 0 {
        0usize
    } else {
        usize::from(ref_idx as u8).min(ref_list.len().saturating_sub(1))
    };
    ref_list.get(idx).unwrap_or(first)
}

fn apply_weighted_sample(sample: u8, weight: i32, offset: i32, log2_denom: u8) -> u8 {
    let shift = usize::from(log2_denom.min(31));
    let round = if shift > 0 { 1i32 << (shift - 1) } else { 0 };
    let scaled = (sample as i32) * weight;
    let shifted = if shift > 0 {
        (scaled + round) >> shift
    } else {
        scaled
    };
    (shifted + offset).clamp(0, 255) as u8
}

fn sample_clamped(src: &[u8], stride: usize, src_w: usize, src_h: usize, x: i32, y: i32) -> u8 {
    let max_x = src_w.saturating_sub(1) as i32;
    let max_y = src_h.saturating_sub(1) as i32;
    let sx = x.clamp(0, max_x) as usize;
    let sy = y.clamp(0, max_y) as usize;
    let idx = sy * stride + sx;
    src.get(idx).copied().unwrap_or(0)
}

fn h264_luma_6tap_filter_raw(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
    horizontal: bool,
) -> i32 {
    let get = |off: i32| -> i32 {
        if horizontal {
            i32::from(sample_clamped(src, stride, src_w, src_h, x + off, y))
        } else {
            i32::from(sample_clamped(src, stride, src_w, src_h, x, y + off))
        }
    };
    get(-2) - 5 * get(-1) + 20 * get(0) + 20 * get(1) - 5 * get(2) + get(3)
}

fn h264_luma_6tap_round(v: i32) -> i32 {
    ((v + 16) >> 5).clamp(0, 255)
}

fn sample_h264_luma_half_h(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    h264_luma_6tap_round(h264_luma_6tap_filter_raw(
        src, stride, src_w, src_h, x, y, true,
    ))
}

fn sample_h264_luma_half_v(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    h264_luma_6tap_round(h264_luma_6tap_filter_raw(
        src, stride, src_w, src_h, x, y, false,
    ))
}

fn sample_h264_luma_half_hv(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    x: i32,
    y: i32,
) -> i32 {
    let h_row =
        |yy: i32| -> i32 { h264_luma_6tap_filter_raw(src, stride, src_w, src_h, x, yy, true) };
    let val = h_row(y - 2) - 5 * h_row(y - 1) + 20 * h_row(y) + 20 * h_row(y + 1)
        - 5 * h_row(y + 2)
        + h_row(y + 3);
    ((val + 512) >> 10).clamp(0, 255)
}

#[allow(clippy::too_many_arguments)]
fn sample_h264_luma_qpel(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    base_x: i32,
    base_y: i32,
    frac_x: u8,
    frac_y: u8,
) -> u8 {
    let dx = usize::from(frac_x & 3);
    let dy = usize::from(frac_y & 3);
    let f = |ox: i32, oy: i32| -> i32 {
        i32::from(sample_clamped(
            src,
            stride,
            src_w,
            src_h,
            base_x + ox,
            base_y + oy,
        ))
    };
    let h = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_h(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let v = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_v(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let hv = |ox: i32, oy: i32| -> i32 {
        sample_h264_luma_half_hv(src, stride, src_w, src_h, base_x + ox, base_y + oy)
    };
    let avg = |a: i32, b: i32| -> i32 { (a + b + 1) >> 1 };

    let val = match (dx, dy) {
        (0, 0) => f(0, 0),
        (1, 0) => avg(f(0, 0), h(0, 0)),
        (2, 0) => h(0, 0),
        (3, 0) => avg(h(0, 0), f(1, 0)),
        (0, 1) => avg(f(0, 0), v(0, 0)),
        (0, 2) => v(0, 0),
        (0, 3) => avg(v(0, 0), f(0, 1)),
        (2, 2) => hv(0, 0),
        (1, 1) => avg(f(0, 0), hv(0, 0)),
        (3, 1) => avg(f(1, 0), hv(0, 0)),
        (1, 3) => avg(f(0, 1), hv(0, 0)),
        (3, 3) => avg(f(1, 1), hv(0, 0)),
        (2, 1) => avg(h(0, 0), hv(0, 0)),
        (2, 3) => avg(hv(0, 0), h(0, 1)),
        (1, 2) => avg(v(0, 0), hv(0, 0)),
        (3, 2) => avg(hv(0, 0), v(1, 0)),
        _ => f(0, 0),
    };
    val.clamp(0, 255) as u8
}

#[allow(clippy::too_many_arguments)]
fn sample_bilinear_clamped(
    src: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    base_x: i32,
    base_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
) -> u8 {
    if frac_base == 0 {
        return sample_clamped(src, stride, src_w, src_h, base_x, base_y);
    }
    let fx = frac_x.min(frac_base);
    let fy = frac_y.min(frac_base);
    if fx == 0 && fy == 0 {
        return sample_clamped(src, stride, src_w, src_h, base_x, base_y);
    }

    let p00 = i32::from(sample_clamped(src, stride, src_w, src_h, base_x, base_y));
    let p10 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y,
    ));
    let p01 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x,
        base_y + 1,
    ));
    let p11 = i32::from(sample_clamped(
        src,
        stride,
        src_w,
        src_h,
        base_x + 1,
        base_y + 1,
    ));

    let fx = i32::from(fx);
    let fy = i32::from(fy);
    let base = i32::from(frac_base);
    let wx0 = base - fx;
    let wy0 = base - fy;
    let den = base * base;
    let sum = p00 * wx0 * wy0 + p10 * fx * wy0 + p01 * wx0 * fy + p11 * fx * fy;
    ((sum + den / 2) / den).clamp(0, 255) as u8
}

#[allow(clippy::too_many_arguments)]
fn copy_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                dst[dst_idx] = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blend_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
                dst[dst_idx] = ((u16::from(dst[dst_idx]) + u16::from(sample) + 1) >> 1) as u8;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn weighted_copy_luma_block_with_h264_qpel(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
    weight: i32,
    offset: i32,
    log2_denom: u8,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_h264_luma_qpel(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                );
                dst[dst_idx] = apply_weighted_sample(sample, weight, offset, log2_denom);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn copy_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                dst[dst_idx] = sample_bilinear_clamped(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                    frac_base,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blend_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_bilinear_clamped(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                    frac_base,
                );
                let blended = ((u16::from(dst[dst_idx]) + u16::from(sample) + 1) >> 1) as u8;
                dst[dst_idx] = blended;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn weighted_copy_block_with_qpel_bilinear(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    src_x: i32,
    src_y: i32,
    frac_x: u8,
    frac_y: u8,
    frac_base: u8,
    dst_x: usize,
    dst_y: usize,
    w: usize,
    h: usize,
    src_w: usize,
    src_h: usize,
    weight: i32,
    offset: i32,
    log2_denom: u8,
) {
    if w == 0 || h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let dx = dst_x + x;
            let dy = dst_y + y;
            let dst_idx = dy * dst_stride + dx;
            if dst_idx < dst.len() {
                let sample = sample_bilinear_clamped(
                    src,
                    src_stride,
                    src_w,
                    src_h,
                    src_x + x as i32,
                    src_y + y as i32,
                    frac_x,
                    frac_y,
                    frac_base,
                );
                dst[dst_idx] = apply_weighted_sample(sample, weight, offset, log2_denom);
            }
        }
    }
}

/// 判断当前宏块是否命中调试输出范围.
fn debug_mb_selected(mb_idx: usize, mb_x: usize, mb_y: usize) -> bool {
    if let Ok(spec) = std::env::var("TAO_H264_DEBUG_MB_RANGE") {
        let spec = spec.trim();
        if spec.is_empty() {
            return false;
        }
        if let Some((start_s, end_s)) = spec.split_once(':')
            && let (Ok(start), Ok(end)) = (
                start_s.trim().parse::<usize>(),
                end_s.trim().parse::<usize>(),
            )
        {
            return (start..=end).contains(&mb_idx);
        }
        if let Ok(exact) = spec.parse::<usize>() {
            return mb_idx == exact;
        }
        return false;
    }

    if mb_y <= 2 && mb_x < 80 {
        return true;
    }
    if (620..=645).contains(&mb_idx) {
        return true;
    }
    false
}

/// 读取无符号 Exp-Golomb
fn read_ue(br: &mut BitReader) -> TaoResult<u32> {
    let mut zeros = 0u32;
    loop {
        let bit = br.read_bit()?;
        if bit == 1 {
            break;
        }
        zeros += 1;
        if zeros > 31 {
            return Err(TaoError::InvalidData("Exp-Golomb 前导零过多".into()));
        }
    }
    if zeros == 0 {
        return Ok(0);
    }
    let suffix = br.read_bits(zeros)?;
    Ok((1 << zeros) - 1 + suffix)
}

/// 读取有符号 Exp-Golomb
fn read_se(br: &mut BitReader) -> TaoResult<i32> {
    let code = read_ue(br)?;
    let value = code.div_ceil(2) as i32;
    if code & 1 == 0 { Ok(-value) } else { Ok(value) }
}

/// QP 按 H.264 规则做 0..51 环绕.
fn wrap_qp(qp: i64) -> i32 {
    let m = 52i64;
    ((qp % m + m) % m) as i32
}

/// Luma QP → Chroma QP 映射 (H.264 Table 8-15)
fn chroma_qp_from_luma_with_offset(qp: i32, offset: i32) -> i32 {
    let qpc = (qp + offset).clamp(0, 51);
    CHROMA_QP_TABLE[qpc as usize]
}

/// 从对齐缓冲区拷贝到紧凑平面
fn copy_plane(src: &[u8], src_stride: usize, w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; w * h];
    for y in 0..h {
        let src_off = y * src_stride;
        let dst_off = y * w;
        let copy_len = w.min(src.len().saturating_sub(src_off));
        if copy_len > 0 && dst_off + copy_len <= dst.len() {
            dst[dst_off..dst_off + copy_len].copy_from_slice(&src[src_off..src_off + copy_len]);
        }
    }
    dst
}

/// Chroma QP 映射表 (H.264 Table 8-15)
#[rustfmt::skip]
const CHROMA_QP_TABLE: [i32; 52] = [
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 29, 30,
    31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38,
    39, 39, 39, 39,
];

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    use tao_core::Rational;

    use crate::frame::Frame;

    use super::{
        DecRefPicMarking, H264Decoder, MmcoOp, NalUnit, ParameterSetRebuildAction,
        PendingFrameMeta, Pps, RefPicListMod, RefPlanes, ReferencePicture, SliceHeader, Sps,
        sample_h264_luma_qpel,
    };

    fn build_test_pps() -> Pps {
        Pps {
            pps_id: 0,
            sps_id: 0,
            entropy_coding_mode: 1,
            pic_init_qp: 26,
            chroma_qp_index_offset: 0,
            second_chroma_qp_index_offset: 0,
            deblocking_filter_control: true,
            pic_order_present: false,
            num_ref_idx_l0_default_active: 1,
            num_ref_idx_l1_default_active: 1,
            weighted_pred: false,
            weighted_bipred_idc: 0,
            redundant_pic_cnt_present: false,
            transform_8x8_mode: false,
        }
    }

    fn build_test_sps(sps_id: u32) -> Sps {
        Sps {
            profile_idc: 100,
            constraint_set_flags: 0,
            level_idc: 40,
            sps_id,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            max_num_ref_frames: 4,
            width: 16,
            height: 16,
            frame_mbs_only: true,
            vui_present: false,
            fps: None,
            sar: Rational::new(1, 1),
            pic_width_in_mbs: 1,
            pic_height_in_map_units: 1,
            crop_left: 0,
            crop_right: 0,
            crop_top: 0,
            crop_bottom: 0,
            log2_max_frame_num: 4,
            poc_type: 0,
            log2_max_poc_lsb: 4,
            delta_pic_order_always_zero_flag: false,
            offset_for_non_ref_pic: 0,
            offset_for_top_to_bottom_field: 0,
            offset_for_ref_frame: Vec::new(),
        }
    }

    fn build_test_sps_with_poc_type(sps_id: u32, poc_type: u32) -> Sps {
        let mut sps = build_test_sps(sps_id);
        sps.poc_type = poc_type;
        sps
    }

    fn build_test_slice_header(
        frame_num: u32,
        nal_ref_idc: u8,
        is_idr: bool,
        poc_lsb: Option<u32>,
    ) -> SliceHeader {
        SliceHeader {
            first_mb: 0,
            pps_id: 0,
            slice_type: 0,
            frame_num,
            slice_qp: 26,
            cabac_init_idc: 0,
            num_ref_idx_l0: 1,
            num_ref_idx_l1: 1,
            ref_pic_list_mod_l0: Vec::new(),
            ref_pic_list_mod_l1: Vec::new(),
            luma_log2_weight_denom: 0,
            chroma_log2_weight_denom: 0,
            l0_weights: Vec::new(),
            data_bit_offset: 0,
            cabac_start_byte: 0,
            nal_ref_idc,
            is_idr,
            pic_order_cnt_lsb: poc_lsb,
            delta_poc_bottom: 0,
            delta_poc_0: 0,
            delta_poc_1: 0,
            disable_deblocking_filter_idc: 0,
            dec_ref_pic_marking: DecRefPicMarking::default(),
        }
    }

    fn build_test_decoder() -> H264Decoder {
        let mut dec = H264Decoder {
            sps: None,
            pps: None,
            sps_map: HashMap::new(),
            pps_map: HashMap::new(),
            active_sps_id: None,
            active_pps_id: None,
            length_size: 4,
            width: 16,
            height: 16,
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
            last_disable_deblocking_filter_idc: 0,
            prev_ref_poc_msb: 0,
            prev_ref_poc_lsb: 0,
            prev_frame_num_offset_type1: 0,
            prev_frame_num_offset_type2: 0,
            last_dec_ref_pic_marking: DecRefPicMarking::default(),
            reference_frames: VecDeque::new(),
            max_long_term_frame_idx: None,
            max_reference_frames: 4,
            output_queue: VecDeque::new(),
            reorder_buffer: Vec::new(),
            reorder_depth: 2,
            decode_order_counter: 0,
            pending_frame: None,
            opened: true,
            flushing: false,
        };
        dec.init_buffers();
        dec
    }

    fn push_dummy_reference(dec: &mut H264Decoder, frame_num: u32) {
        push_dummy_reference_with_long_term(dec, frame_num, None);
    }

    fn push_dummy_reference_with_long_term(
        dec: &mut H264Decoder,
        frame_num: u32,
        long_term_frame_idx: Option<u32>,
    ) {
        dec.reference_frames.push_back(ReferencePicture {
            y: vec![0u8; dec.ref_y.len()],
            u: vec![0u8; dec.ref_u.len()],
            v: vec![0u8; dec.ref_v.len()],
            frame_num,
            poc: frame_num as i32,
            long_term_frame_idx,
        });
    }

    fn push_custom_reference(
        dec: &mut H264Decoder,
        frame_num: u32,
        poc: i32,
        y_value: u8,
        long_term_frame_idx: Option<u32>,
    ) {
        dec.reference_frames.push_back(ReferencePicture {
            y: vec![y_value; dec.ref_y.len()],
            u: vec![128u8; dec.ref_u.len()],
            v: vec![128u8; dec.ref_v.len()],
            frame_num,
            poc,
            long_term_frame_idx,
        });
    }

    fn build_constant_ref_planes(dec: &H264Decoder, y: u8, u: u8, v: u8) -> RefPlanes {
        RefPlanes {
            y: vec![y; dec.ref_y.len()],
            u: vec![u; dec.ref_u.len()],
            v: vec![v; dec.ref_v.len()],
            poc: 0,
        }
    }

    fn write_ue(bits: &mut Vec<bool>, value: u32) {
        if value == 0 {
            bits.push(true);
            return;
        }
        let code_num = value + 1;
        let num_bits = 32 - code_num.leading_zeros();
        for _ in 0..(num_bits - 1) {
            bits.push(false);
        }
        for i in (0..num_bits).rev() {
            bits.push(((code_num >> i) & 1) != 0);
        }
    }

    fn write_se(bits: &mut Vec<bool>, value: i32) {
        let code_num = if value > 0 {
            (value as u32) * 2 - 1
        } else {
            (-value as u32) * 2
        };
        write_ue(bits, code_num);
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut idx = 0usize;
        while idx < bits.len() {
            let mut byte = 0u8;
            for bit_idx in 0..8 {
                let src_idx = idx + bit_idx;
                if src_idx < bits.len() && bits[src_idx] {
                    byte |= 1 << (7 - bit_idx);
                }
            }
            bytes.push(byte);
            idx += 8;
        }
        bytes
    }

    fn build_pps_nalu(
        pps_id: u32,
        sps_id: u32,
        entropy: bool,
        pic_init_qp_minus26: i32,
    ) -> NalUnit {
        let mut bits = Vec::new();
        write_ue(&mut bits, pps_id);
        write_ue(&mut bits, sps_id);
        bits.push(entropy);
        bits.push(false); // pic_order_present_flag
        write_ue(&mut bits, 0); // num_slice_groups_minus1
        write_ue(&mut bits, 0); // num_ref_idx_l0_default_active_minus1
        write_ue(&mut bits, 0); // num_ref_idx_l1_default_active_minus1
        bits.push(false); // weighted_pred_flag
        bits.push(false); // weighted_bipred_idc bit1
        bits.push(false); // weighted_bipred_idc bit0
        write_se(&mut bits, pic_init_qp_minus26);
        write_se(&mut bits, 0); // pic_init_qs_minus26
        write_se(&mut bits, 0); // chroma_qp_index_offset
        bits.push(true); // deblocking_filter_control_present_flag
        bits.push(false); // constrained_intra_pred_flag
        bits.push(false); // redundant_pic_cnt_present_flag
        // rbsp_trailing_bits
        bits.push(true);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }

        let mut data = Vec::with_capacity(1 + bits.len().div_ceil(8));
        data.push(0x68); // nal_ref_idc=3, nal_unit_type=8(PPS)
        data.extend_from_slice(&bits_to_bytes(&bits));
        NalUnit::parse(&data).expect("测试构造 PPS NAL 失败")
    }

    fn push_bits_fixed(bits: &mut Vec<bool>, value: u32, width: usize) {
        for i in (0..width).rev() {
            bits.push(((value >> i) & 1) != 0);
        }
    }

    fn push_bits_u8(bits: &mut Vec<bool>, value: u8) {
        for i in (0..8).rev() {
            bits.push(((value >> i) & 1) != 0);
        }
    }

    fn build_rbsp_from_ues(values: &[u32]) -> Vec<u8> {
        let mut bits = Vec::new();
        for &v in values {
            write_ue(&mut bits, v);
        }
        bits_to_bytes(&bits)
    }

    fn build_sps_nalu(sps_id: u32, width: u32, height: u32) -> NalUnit {
        let mut bits = Vec::new();
        push_bits_u8(&mut bits, 66); // profile_idc: Baseline
        push_bits_u8(&mut bits, 0); // constraint_set_flags
        push_bits_u8(&mut bits, 30); // level_idc
        write_ue(&mut bits, sps_id);
        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, 0); // pic_order_cnt_type
        write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
        write_ue(&mut bits, 4); // max_num_ref_frames
        bits.push(false); // gaps_in_frame_num_value_allowed_flag

        let mbs_w = width.div_ceil(16);
        let mbs_h = height.div_ceil(16);
        write_ue(&mut bits, mbs_w - 1);
        write_ue(&mut bits, mbs_h - 1);
        bits.push(true); // frame_mbs_only_flag
        bits.push(false); // direct_8x8_inference_flag
        bits.push(false); // frame_cropping_flag
        bits.push(false); // vui_parameters_present_flag
        bits.push(true); // rbsp_trailing_bits stop bit
        while bits.len() % 8 != 0 {
            bits.push(false);
        }

        let mut data = Vec::with_capacity(1 + bits.len().div_ceil(8));
        data.push(0x67); // nal_ref_idc=3, nal_unit_type=7(SPS)
        data.extend_from_slice(&bits_to_bytes(&bits));
        NalUnit::parse(&data).expect("测试构造 SPS NAL 失败")
    }

    fn build_p_slice_header_rbsp(
        pps_id: u32,
        frame_num: u32,
        poc_lsb: u32,
        cabac_init_idc: u32,
        qp_delta: i32,
        disable_deblocking_filter_idc: u32,
    ) -> Vec<u8> {
        let mut bits = Vec::new();
        write_ue(&mut bits, 0); // first_mb_in_slice
        write_ue(&mut bits, 0); // slice_type=P
        write_ue(&mut bits, pps_id);
        push_bits_fixed(&mut bits, frame_num, 4);
        push_bits_fixed(&mut bits, poc_lsb, 4);
        bits.push(false); // num_ref_idx_active_override_flag
        bits.push(false); // ref_pic_list_modification_flag_l0
        write_ue(&mut bits, cabac_init_idc);
        write_se(&mut bits, qp_delta); // slice_qp_delta
        write_ue(&mut bits, disable_deblocking_filter_idc);
        if disable_deblocking_filter_idc != 1 {
            write_se(&mut bits, 0); // slice_alpha_c0_offset_div2
            write_se(&mut bits, 0); // slice_beta_offset_div2
        }
        bits.push(true); // rbsp_trailing_bits stop bit
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits_to_bytes(&bits)
    }

    fn build_p_slice_header_rbsp_with_l0_reorder(
        pps_id: u32,
        frame_num: u32,
        poc_lsb: u32,
        op_idc: u32,
        op_value: u32,
    ) -> Vec<u8> {
        let mut bits = Vec::new();
        write_ue(&mut bits, 0); // first_mb_in_slice
        write_ue(&mut bits, 0); // slice_type=P
        write_ue(&mut bits, pps_id);
        push_bits_fixed(&mut bits, frame_num, 4);
        push_bits_fixed(&mut bits, poc_lsb, 4);
        bits.push(false); // num_ref_idx_active_override_flag
        bits.push(true); // ref_pic_list_modification_flag_l0
        write_ue(&mut bits, op_idc);
        write_ue(&mut bits, op_value);
        write_ue(&mut bits, 3); // end
        bits.push(false); // adaptive_ref_pic_marking_mode_flag
        write_se(&mut bits, 0); // slice_qp_delta
        write_ue(&mut bits, 1); // disable_deblocking_filter_idc
        bits.push(true); // rbsp_trailing_bits stop bit
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits_to_bytes(&bits)
    }

    fn build_p_slice_header_rbsp_poc_type1(
        pps_id: u32,
        frame_num: u32,
        delta_poc_0: i32,
        delta_poc_1: i32,
        disable_deblocking_filter_idc: u32,
    ) -> Vec<u8> {
        let mut bits = Vec::new();
        write_ue(&mut bits, 0); // first_mb_in_slice
        write_ue(&mut bits, 0); // slice_type=P
        write_ue(&mut bits, pps_id);
        push_bits_fixed(&mut bits, frame_num, 4);
        write_se(&mut bits, delta_poc_0);
        write_se(&mut bits, delta_poc_1);
        bits.push(false); // num_ref_idx_active_override_flag
        bits.push(false); // ref_pic_list_modification_flag_l0
        bits.push(false); // adaptive_ref_pic_marking_mode_flag
        write_se(&mut bits, 0); // slice_qp_delta
        write_ue(&mut bits, disable_deblocking_filter_idc);
        if disable_deblocking_filter_idc != 1 {
            write_se(&mut bits, 0); // slice_alpha_c0_offset_div2
            write_se(&mut bits, 0); // slice_beta_offset_div2
        }
        bits.push(true); // rbsp_trailing_bits stop bit
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        bits_to_bytes(&bits)
    }

    fn build_linear_plane(
        width: usize,
        height: usize,
        offset: u8,
        step_x: u8,
        step_y: u8,
    ) -> Vec<u8> {
        let mut plane = vec![0u8; width * height];
        for y in 0..height {
            for x in 0..width {
                let v = usize::from(offset) + usize::from(step_x) * x + usize::from(step_y) * y;
                plane[y * width + x] = (v.min(255)) as u8;
            }
        }
        plane
    }

    #[test]
    fn test_pps_rebuild_action_none_for_identical_pps() {
        let old = build_test_pps();
        let new = build_test_pps();
        assert_eq!(
            H264Decoder::pps_rebuild_action(&old, &new),
            ParameterSetRebuildAction::None,
            "相同 PPS 不应触发重建"
        );
    }

    #[test]
    fn test_pps_rebuild_action_runtime_on_qp_related_change() {
        let old = build_test_pps();
        let mut new = build_test_pps();
        new.pic_init_qp = 22;
        assert_eq!(
            H264Decoder::pps_rebuild_action(&old, &new),
            ParameterSetRebuildAction::RuntimeOnly,
            "QP 相关字段变化应触发运行时重建"
        );
    }

    #[test]
    fn test_pps_rebuild_action_runtime_on_weighted_pred_change() {
        let old = build_test_pps();
        let mut new = build_test_pps();
        new.weighted_pred = true;
        assert_eq!(
            H264Decoder::pps_rebuild_action(&old, &new),
            ParameterSetRebuildAction::RuntimeOnly,
            "加权预测字段变化应触发运行时重建"
        );
    }

    #[test]
    fn test_pps_rebuild_action_full_on_entropy_change() {
        let old = build_test_pps();
        let mut new = build_test_pps();
        new.entropy_coding_mode = 0;
        assert_eq!(
            H264Decoder::pps_rebuild_action(&old, &new),
            ParameterSetRebuildAction::Full,
            "熵编码模式变化应触发完整重建"
        );
    }

    #[test]
    fn test_pps_rebuild_action_full_on_sps_change() {
        let old = build_test_pps();
        let mut new = build_test_pps();
        new.sps_id = 1;
        new.pic_init_qp = 30;
        assert_eq!(
            H264Decoder::pps_rebuild_action(&old, &new),
            ParameterSetRebuildAction::Full,
            "SPS 绑定变化应优先触发完整重建"
        );
    }

    #[test]
    fn test_activate_parameter_sets_runtime_only_keeps_references() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0.clone());
        dec.sps = Some(sps0);
        dec.active_sps_id = Some(0);

        let pps0 = build_test_pps();
        let mut pps1 = build_test_pps();
        pps1.pps_id = 1;
        pps1.pic_init_qp = 24;
        dec.pps_map.insert(0, pps0.clone());
        dec.pps_map.insert(1, pps1.clone());
        dec.pps = Some(pps0);
        dec.active_pps_id = Some(0);

        dec.mb_types[0] = 9;
        dec.prev_qp_delta_nz = true;
        dec.decode_order_counter = 7;
        dec.pending_frame = Some(PendingFrameMeta {
            pts: 1,
            time_base: Rational::new(1, 25),
            is_keyframe: false,
        });
        push_dummy_reference(&mut dec, 10);

        dec.activate_parameter_sets(1)
            .expect("运行时重建 PPS 激活失败");
        assert_eq!(dec.active_pps_id, Some(1), "active_pps_id 未切换");
        assert_eq!(dec.mb_types[0], 0, "运行时重建应重置宏块状态");
        assert!(!dec.prev_qp_delta_nz, "运行时重建应重置 prev_qp_delta_nz");
        assert_eq!(dec.reference_frames.len(), 1, "运行时重建不应清空参考帧");
        assert_eq!(
            dec.decode_order_counter, 7,
            "运行时重建不应重置 decode_order_counter"
        );
    }

    #[test]
    fn test_activate_parameter_sets_full_resets_references() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0.clone());
        dec.sps = Some(sps0);
        dec.active_sps_id = Some(0);

        let pps0 = build_test_pps();
        let mut pps1 = build_test_pps();
        pps1.pps_id = 1;
        pps1.entropy_coding_mode = 0;
        dec.pps_map.insert(0, pps0.clone());
        dec.pps_map.insert(1, pps1.clone());
        dec.pps = Some(pps0);
        dec.active_pps_id = Some(0);

        dec.mb_types[0] = 9;
        dec.prev_qp_delta_nz = true;
        dec.decode_order_counter = 11;
        dec.pending_frame = Some(PendingFrameMeta {
            pts: 2,
            time_base: Rational::new(1, 30),
            is_keyframe: true,
        });
        push_dummy_reference(&mut dec, 20);

        dec.activate_parameter_sets(1)
            .expect("完整重建 PPS 激活失败");
        assert_eq!(dec.active_pps_id, Some(1), "active_pps_id 未切换");
        assert_eq!(dec.mb_types[0], 0, "完整重建应重置宏块状态");
        assert!(!dec.prev_qp_delta_nz, "完整重建应重置 prev_qp_delta_nz");
        assert!(dec.reference_frames.is_empty(), "完整重建应清空参考帧缓存");
        assert_eq!(
            dec.decode_order_counter, 0,
            "完整重建应重置 decode_order_counter"
        );
    }

    #[test]
    fn test_handle_pps_same_id_runtime_update_keeps_references() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0.clone());
        dec.sps = Some(sps0);
        dec.active_sps_id = Some(0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0.clone());
        dec.pps = Some(pps0);
        dec.active_pps_id = Some(0);

        dec.mb_types[0] = 9;
        dec.prev_qp_delta_nz = true;
        dec.decode_order_counter = 3;
        push_dummy_reference(&mut dec, 8);

        let pps_runtime_update = build_pps_nalu(0, 0, true, -2);
        dec.handle_pps(&pps_runtime_update);

        assert_eq!(dec.active_pps_id, Some(0), "active_pps_id 应保持为 0");
        assert_eq!(dec.reference_frames.len(), 1, "运行时重建不应清空参考帧");
        assert_eq!(dec.mb_types[0], 0, "运行时重建应重置宏块状态");
        assert!(!dec.prev_qp_delta_nz, "运行时重建应重置 prev_qp_delta_nz");
        assert_eq!(
            dec.decode_order_counter, 3,
            "运行时重建不应重置 decode_order_counter"
        );
        assert_eq!(
            dec.pps.as_ref().map(|p| p.pic_init_qp),
            Some(24),
            "PPS pic_init_qp 应更新为 24"
        );
    }

    #[test]
    fn test_handle_pps_same_id_full_update_resets_references() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0.clone());
        dec.sps = Some(sps0);
        dec.active_sps_id = Some(0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0.clone());
        dec.pps = Some(pps0);
        dec.active_pps_id = Some(0);

        dec.mb_types[0] = 9;
        dec.prev_qp_delta_nz = true;
        dec.decode_order_counter = 5;
        push_dummy_reference(&mut dec, 12);

        let pps_full_update = build_pps_nalu(0, 0, false, 0);
        dec.handle_pps(&pps_full_update);

        assert_eq!(dec.active_pps_id, Some(0), "active_pps_id 应保持为 0");
        assert_eq!(dec.mb_types[0], 0, "完整重建应重置宏块状态");
        assert!(!dec.prev_qp_delta_nz, "完整重建应重置 prev_qp_delta_nz");
        assert!(dec.reference_frames.is_empty(), "完整重建应清空参考帧");
        assert_eq!(
            dec.decode_order_counter, 0,
            "完整重建应重置 decode_order_counter"
        );
        assert_eq!(
            dec.pps.as_ref().map(|p| p.entropy_coding_mode),
            Some(0),
            "PPS entropy_coding_mode 应更新为 CAVLC"
        );
    }

    #[test]
    fn test_parse_slice_header_reject_invalid_cabac_init_idc() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp(0, 0, 0, 3, 0, 1);
        let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
        let err = match dec.parse_slice_header(&rbsp, &nalu) {
            Ok(_) => panic!("cabac_init_idc=3 应失败"),
            Err(err) => err,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("cabac_init_idc"),
            "错误信息应包含 cabac_init_idc, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_slice_header_reject_slice_qp_out_of_range() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp(0, 0, 0, 0, 40, 1);
        let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
        let err = match dec.parse_slice_header(&rbsp, &nalu) {
            Ok(_) => panic!("slice_qp 超范围应失败"),
            Err(err) => err,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("slice_qp"),
            "错误信息应包含 slice_qp, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_slice_header_reject_invalid_deblocking_idc() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp(0, 0, 0, 0, 0, 3);
        let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
        let err = match dec.parse_slice_header(&rbsp, &nalu) {
            Ok(_) => panic!("disable_deblocking_filter_idc=3 应失败"),
            Err(err) => err,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("disable_deblocking_filter_idc"),
            "错误信息应包含 disable_deblocking_filter_idc, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_slice_header_accept_deblocking_idc_1() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0);

        let pps0 = build_test_pps();
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp(0, 0, 0, 0, 0, 1);
        let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
        let header = dec
            .parse_slice_header(&rbsp, &nalu)
            .expect("disable_deblocking_filter_idc=1 应可解析");
        assert_eq!(
            header.disable_deblocking_filter_idc, 1,
            "slice header 应保存 disable_deblocking_filter_idc"
        );
    }

    #[test]
    fn test_parse_slice_header_poc_type1_delta_parse() {
        let mut dec = build_test_decoder();
        let mut sps0 = build_test_sps_with_poc_type(0, 1);
        sps0.delta_pic_order_always_zero_flag = false;
        dec.sps_map.insert(0, sps0);

        let mut pps0 = build_test_pps();
        pps0.entropy_coding_mode = 0;
        pps0.pic_order_present = true;
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp_poc_type1(0, 0, 2, -1, 1);
        let nalu = NalUnit::parse(&[0x21]).expect("测试构造 slice NAL 失败");
        let header = dec
            .parse_slice_header(&rbsp, &nalu)
            .expect("poc_type1 slice header 应可解析");
        assert_eq!(header.delta_poc_0, 2, "delta_poc_0 解析错误");
        assert_eq!(header.delta_poc_1, -1, "delta_poc_1 解析错误");
    }

    #[test]
    fn test_parse_slice_header_ref_pic_list_mod_l0_short_term_sub() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0);

        let mut pps0 = build_test_pps();
        pps0.entropy_coding_mode = 0;
        dec.pps_map.insert(0, pps0);

        let rbsp = build_p_slice_header_rbsp_with_l0_reorder(0, 10, 0, 0, 1);
        let nalu = NalUnit::parse(&[0x21]).expect("测试构造 slice NAL 失败");
        let header = dec
            .parse_slice_header(&rbsp, &nalu)
            .expect("带 L0 重排的 slice header 应可解析");
        assert_eq!(
            header.ref_pic_list_mod_l0.len(),
            1,
            "应解析出 1 条 L0 重排项"
        );
        assert_eq!(
            header.ref_pic_list_mod_l0[0],
            RefPicListMod::ShortTermSub {
                abs_diff_pic_num_minus1: 1
            },
            "L0 重排项解析结果错误"
        );
    }

    #[test]
    fn test_handle_sps_same_id_size_change_resets_reference_state() {
        let mut dec = build_test_decoder();
        let sps0 = build_test_sps(0);
        dec.sps_map.insert(0, sps0.clone());
        dec.sps = Some(sps0);
        dec.active_sps_id = Some(0);

        dec.decode_order_counter = 9;
        push_dummy_reference(&mut dec, 22);
        assert_eq!(dec.width, 16, "初始宽度应为 16");
        assert_eq!(dec.height, 16, "初始高度应为 16");
        assert_eq!(dec.mb_width, 1, "初始宏块宽度应为 1");

        let sps_resize = build_sps_nalu(0, 32, 16);
        dec.handle_sps(&sps_resize);

        assert_eq!(dec.active_sps_id, Some(0), "active_sps_id 应保持为 0");
        assert_eq!(dec.width, 32, "SPS 切换后宽度应更新为 32");
        assert_eq!(dec.height, 16, "SPS 切换后高度应保持 16");
        assert_eq!(dec.mb_width, 2, "SPS 切换后宏块宽度应更新为 2");
        assert!(dec.reference_frames.is_empty(), "尺寸变化应清空参考帧缓存");
        assert_eq!(
            dec.decode_order_counter, 0,
            "尺寸变化应重置 decode_order_counter"
        );
    }

    #[test]
    fn test_store_reference_with_marking_mmco_forget_short_and_long() {
        let mut dec = build_test_decoder();
        push_dummy_reference(&mut dec, 1);
        push_dummy_reference(&mut dec, 2);
        push_dummy_reference(&mut dec, 3);
        push_dummy_reference_with_long_term(&mut dec, 7, Some(1));

        dec.last_slice_type = 0;
        dec.last_nal_ref_idc = 1;
        dec.last_frame_num = 4;
        dec.last_poc = 4;
        dec.last_dec_ref_pic_marking = DecRefPicMarking {
            is_idr: false,
            no_output_of_prior_pics: false,
            long_term_reference_flag: false,
            adaptive: true,
            ops: vec![
                MmcoOp::ForgetShort {
                    difference_of_pic_nums_minus1: 0,
                },
                MmcoOp::ForgetLong {
                    long_term_pic_num: 1,
                },
            ],
        };

        dec.store_reference_with_marking();

        let has_removed_short = dec.reference_frames.iter().all(|pic| pic.frame_num != 3);
        assert!(has_removed_short, "MMCO1 应移除 pic_num=3 的短期参考帧");
        let has_removed_long = dec
            .reference_frames
            .iter()
            .all(|pic| pic.long_term_frame_idx != Some(1));
        assert!(
            has_removed_long,
            "MMCO2 应移除 long_term_pic_num=1 的长期参考帧"
        );
        let has_current = dec
            .reference_frames
            .iter()
            .any(|pic| pic.frame_num == 4 && pic.long_term_frame_idx.is_none());
        assert!(has_current, "当前帧应按短期参考帧入队");
    }

    #[test]
    fn test_store_reference_with_marking_mmco_mark_current_long() {
        let mut dec = build_test_decoder();
        dec.last_slice_type = 0;
        dec.last_nal_ref_idc = 1;
        dec.last_frame_num = 5;
        dec.last_poc = 5;
        dec.last_dec_ref_pic_marking = DecRefPicMarking {
            is_idr: false,
            no_output_of_prior_pics: false,
            long_term_reference_flag: false,
            adaptive: true,
            ops: vec![
                MmcoOp::TrimLong {
                    max_long_term_frame_idx_plus1: 3,
                },
                MmcoOp::MarkCurrentLong {
                    long_term_frame_idx: 2,
                },
            ],
        };

        dec.store_reference_with_marking();

        assert_eq!(
            dec.max_long_term_frame_idx,
            Some(2),
            "MMCO4 应更新长期参考帧索引上限"
        );
        let current = dec.reference_frames.back().expect("应存在当前参考帧");
        assert_eq!(
            current.long_term_frame_idx,
            Some(2),
            "MMCO6 应将当前帧标记为长期参考帧"
        );
    }

    #[test]
    fn test_store_reference_with_marking_idr_long_term_reference() {
        let mut dec = build_test_decoder();
        push_dummy_reference(&mut dec, 1);
        push_dummy_reference_with_long_term(&mut dec, 6, Some(2));

        dec.last_slice_type = 2;
        dec.last_nal_ref_idc = 1;
        dec.last_frame_num = 8;
        dec.last_poc = 8;
        dec.last_dec_ref_pic_marking = DecRefPicMarking {
            is_idr: true,
            no_output_of_prior_pics: false,
            long_term_reference_flag: true,
            adaptive: false,
            ops: Vec::new(),
        };

        dec.store_reference_with_marking();

        assert_eq!(
            dec.max_long_term_frame_idx,
            Some(0),
            "IDR long_term_reference_flag=1 应将长期参考上限设为 0"
        );
        assert_eq!(dec.reference_frames.len(), 1, "IDR 后应仅保留当前参考帧");
        let current = dec.reference_frames.back().expect("应存在当前参考帧");
        assert_eq!(
            current.long_term_frame_idx,
            Some(0),
            "IDR 长期参考帧应标记为 long_term_frame_idx=0"
        );
    }

    #[test]
    fn test_store_reference_with_marking_mmco_convert_short_to_long() {
        let mut dec = build_test_decoder();
        push_dummy_reference(&mut dec, 2);
        push_dummy_reference(&mut dec, 3);
        push_dummy_reference_with_long_term(&mut dec, 6, Some(0));

        dec.last_slice_type = 0;
        dec.last_nal_ref_idc = 1;
        dec.last_frame_num = 4;
        dec.last_poc = 4;
        dec.last_dec_ref_pic_marking = DecRefPicMarking {
            is_idr: false,
            no_output_of_prior_pics: false,
            long_term_reference_flag: false,
            adaptive: true,
            ops: vec![MmcoOp::ConvertShortToLong {
                difference_of_pic_nums_minus1: 0,
                long_term_frame_idx: 0,
            }],
        };

        dec.store_reference_with_marking();

        let converted = dec
            .reference_frames
            .iter()
            .any(|pic| pic.frame_num == 3 && pic.long_term_frame_idx == Some(0));
        assert!(converted, "MMCO3 应将命中的短期参考帧转为指定长期参考帧");
        let old_long_removed = dec
            .reference_frames
            .iter()
            .filter(|pic| pic.long_term_frame_idx == Some(0))
            .count();
        assert_eq!(
            old_long_removed, 1,
            "MMCO3 转换前应先清理相同 long_term_frame_idx 的旧长期参考帧"
        );
    }

    #[test]
    fn test_reference_list_l0_short_term_before_long_term() {
        let mut dec = build_test_decoder();
        dec.last_slice_type = 0; // P slice
        dec.last_frame_num = 10;
        dec.last_poc = 10;

        push_custom_reference(&mut dec, 8, 8, 8, None);
        push_custom_reference(&mut dec, 9, 9, 9, None);
        push_custom_reference(&mut dec, 2, 2, 200, Some(0));

        let l0 = dec.build_reference_list_l0_with_mod(3, &[], 10);
        assert_eq!(l0.len(), 3, "L0 参考列表长度应为 3");
        assert_eq!(
            l0[0].y[0], 9,
            "L0 rank0 应优先选择最近短期参考帧(frame_num=9)"
        );
        assert_eq!(l0[1].y[0], 8, "L0 rank1 应为次近短期参考帧(frame_num=8)");
        assert_eq!(l0[2].y[0], 200, "L0 rank2 应追加长期参考帧");
    }

    #[test]
    fn test_reference_list_l0_with_short_term_reorder() {
        let mut dec = build_test_decoder();
        dec.last_slice_type = 0; // P slice
        dec.last_frame_num = 10;
        dec.last_poc = 10;

        push_custom_reference(&mut dec, 8, 8, 8, None);
        push_custom_reference(&mut dec, 9, 9, 9, None);
        push_custom_reference(&mut dec, 2, 2, 200, Some(0));

        let mods = [RefPicListMod::ShortTermSub {
            abs_diff_pic_num_minus1: 1,
        }];
        let l0 = dec.build_reference_list_l0_with_mod(3, &mods, 10);
        assert_eq!(l0.len(), 3, "L0 参考列表长度应为 3");
        assert_eq!(l0[0].y[0], 8, "重排后 L0 rank0 应切换到 frame_num=8");
        assert_eq!(l0[1].y[0], 9, "重排后 L0 rank1 应为 frame_num=9");
        assert_eq!(l0[2].y[0], 200, "长期参考帧应保持在后续位置");
    }

    #[test]
    fn test_reference_list_l0_with_long_term_reorder() {
        let mut dec = build_test_decoder();
        dec.last_slice_type = 0; // P slice
        dec.last_frame_num = 10;
        dec.last_poc = 10;

        push_custom_reference(&mut dec, 8, 8, 8, None);
        push_custom_reference(&mut dec, 9, 9, 9, None);
        push_custom_reference(&mut dec, 2, 2, 200, Some(0));

        let mods = [RefPicListMod::LongTerm {
            long_term_pic_num: 0,
        }];
        let l0 = dec.build_reference_list_l0_with_mod(3, &mods, 10);
        assert_eq!(l0.len(), 3, "L0 参考列表长度应为 3");
        assert_eq!(l0[0].y[0], 200, "长期参考重排后应进入 L0 rank0");
        assert_eq!(l0[1].y[0], 9, "原先短期参考应后移");
    }

    #[test]
    fn test_apply_inter_block_l0_selects_ref_by_ref_idx() {
        let mut dec = build_test_decoder();
        let ref0 = build_constant_ref_planes(&dec, 12, 34, 56);
        let ref1 = build_constant_ref_planes(&dec, 201, 202, 203);
        let refs = vec![ref0, ref1];

        dec.apply_inter_block_l0(&refs, 1, 0, 0, 16, 16, 0, 0, &[], 0, 0);
        assert_eq!(dec.ref_y[0], 201, "ref_idx=1 时亮度应来自第二个参考帧");
        assert_eq!(dec.ref_u[0], 202, "ref_idx=1 时 U 应来自第二个参考帧");
        assert_eq!(dec.ref_v[0], 203, "ref_idx=1 时 V 应来自第二个参考帧");
    }

    #[test]
    fn test_build_output_frame_respects_disable_deblocking_filter_idc() {
        let mut dec = build_test_decoder();
        dec.last_slice_type = 2;
        dec.last_nal_ref_idc = 0;
        dec.last_poc = 0;
        dec.reorder_depth = 0;

        for y in 0..dec.height as usize {
            let row = y * dec.stride_y;
            dec.ref_y[row + 3] = 40;
            dec.ref_y[row + 4] = 48;
        }

        dec.last_disable_deblocking_filter_idc = 1;
        dec.build_output_frame(0, Rational::new(1, 25), true);
        let frame_no_filter = match dec.output_queue.pop_front() {
            Some(Frame::Video(vf)) => vf,
            _ => panic!("应输出视频帧"),
        };
        assert_eq!(frame_no_filter.data[0][3], 40, "禁用去块时左边界值不应变化");
        assert_eq!(frame_no_filter.data[0][4], 48, "禁用去块时右边界值不应变化");

        for y in 0..dec.height as usize {
            let row = y * dec.stride_y;
            dec.ref_y[row + 3] = 40;
            dec.ref_y[row + 4] = 48;
        }
        dec.last_disable_deblocking_filter_idc = 0;
        dec.build_output_frame(1, Rational::new(1, 25), true);
        let frame_filter = match dec.output_queue.pop_front() {
            Some(Frame::Video(vf)) => vf,
            _ => panic!("应输出视频帧"),
        };
        assert!(
            frame_filter.data[0][3] > 40,
            "启用去块时左边界值应被平滑提升"
        );
        assert!(
            frame_filter.data[0][4] < 48,
            "启用去块时右边界值应被平滑回拉"
        );
    }

    #[test]
    fn test_decode_cavlc_slice_data_p_skip_run_copy_reference() {
        let mut dec = build_test_decoder();
        push_custom_reference(&mut dec, 3, 3, 77, None);

        let mut header = build_test_slice_header(0, 1, false, None);
        header.slice_type = 0; // P slice
        header.data_bit_offset = 0;

        // mb_skip_run = 1, 覆盖单宏块帧
        let rbsp = build_rbsp_from_ues(&[1]);
        dec.decode_cavlc_slice_data(&rbsp, &header);
        assert_eq!(dec.ref_y[0], 77, "P-slice skip 宏块应复制参考帧像素");
        assert_eq!(dec.mb_types[0], 255, "P-slice skip 宏块应标记为 skip");
    }

    #[test]
    fn test_decode_cavlc_slice_data_i_minimal_intra_predict() {
        let mut dec = build_test_decoder();
        dec.ref_y.fill(0);
        dec.ref_u.fill(0);
        dec.ref_v.fill(0);

        let mut header = build_test_slice_header(0, 1, false, None);
        header.slice_type = 2; // I slice
        header.data_bit_offset = 0;

        // mb_type = ue(0), 最小 I 宏块路径
        let rbsp = build_rbsp_from_ues(&[0]);
        dec.decode_cavlc_slice_data(&rbsp, &header);
        assert_eq!(dec.ref_y[0], 128, "I-slice 最小路径应执行帧内预测");
        assert_eq!(dec.mb_types[0], 1, "I-slice 最小路径应标记为帧内宏块");
    }

    #[test]
    fn test_compute_slice_poc_type2_wrap_and_non_ref() {
        let mut dec = build_test_decoder();
        let sps = build_test_sps_with_poc_type(0, 2);
        dec.sps_map.insert(0, sps.clone());
        dec.sps = Some(sps);
        dec.active_sps_id = Some(0);

        let h1 = build_test_slice_header(14, 1, false, None);
        let poc1 = dec.compute_slice_poc(&h1, 13);
        assert_eq!(poc1, 28, "poc_type2 第一个参考帧 POC 计算错误");

        let h2 = build_test_slice_header(15, 1, false, None);
        let poc2 = dec.compute_slice_poc(&h2, 14);
        assert_eq!(poc2, 30, "poc_type2 连续参考帧 POC 计算错误");

        let h3 = build_test_slice_header(0, 1, false, None);
        let poc3 = dec.compute_slice_poc(&h3, 15);
        assert_eq!(poc3, 32, "poc_type2 wrap 后参考帧 POC 计算错误");

        let h4 = build_test_slice_header(1, 0, false, None);
        let poc4 = dec.compute_slice_poc(&h4, 0);
        assert_eq!(poc4, 33, "poc_type2 非参考帧 POC 计算错误");
    }

    #[test]
    fn test_compute_slice_poc_type2_idr_resets_offset() {
        let mut dec = build_test_decoder();
        let sps = build_test_sps_with_poc_type(0, 2);
        dec.sps_map.insert(0, sps.clone());
        dec.sps = Some(sps);
        dec.active_sps_id = Some(0);
        dec.prev_frame_num_offset_type2 = 32;

        let h = build_test_slice_header(0, 1, true, None);
        let poc = dec.compute_slice_poc(&h, 15);
        assert_eq!(poc, 0, "IDR 帧 POC 应为 0");
        assert_eq!(
            dec.prev_frame_num_offset_type2, 0,
            "IDR 后应重置 prev_frame_num_offset_type2"
        );
    }

    #[test]
    fn test_compute_slice_poc_type1_basic_and_non_ref() {
        let mut dec = build_test_decoder();
        let mut sps = build_test_sps_with_poc_type(0, 1);
        sps.delta_pic_order_always_zero_flag = false;
        sps.offset_for_non_ref_pic = -1;
        sps.offset_for_top_to_bottom_field = 1;
        sps.offset_for_ref_frame = vec![2, -1];
        dec.sps_map.insert(0, sps.clone());
        dec.sps = Some(sps);
        dec.active_sps_id = Some(0);

        let mut h1 = build_test_slice_header(0, 1, false, None);
        h1.delta_poc_0 = 0;
        let poc1 = dec.compute_slice_poc(&h1, 0);
        assert_eq!(poc1, 0, "poc_type1 首帧 POC 计算错误");

        let mut h2 = build_test_slice_header(1, 1, false, None);
        h2.delta_poc_0 = 1;
        let poc2 = dec.compute_slice_poc(&h2, 0);
        assert_eq!(poc2, 3, "poc_type1 参考帧 POC 计算错误");

        let mut h3 = build_test_slice_header(2, 0, false, None);
        h3.delta_poc_0 = 0;
        let poc3 = dec.compute_slice_poc(&h3, 1);
        assert_eq!(poc3, 1, "poc_type1 非参考帧 POC 计算错误");
    }

    #[test]
    fn test_compute_slice_poc_type1_wrap_and_idr_reset() {
        let mut dec = build_test_decoder();
        let mut sps = build_test_sps_with_poc_type(0, 1);
        sps.delta_pic_order_always_zero_flag = false;
        sps.offset_for_non_ref_pic = 0;
        sps.offset_for_top_to_bottom_field = 0;
        sps.offset_for_ref_frame = vec![1];
        dec.sps_map.insert(0, sps.clone());
        dec.sps = Some(sps);
        dec.active_sps_id = Some(0);

        let h1 = build_test_slice_header(15, 1, false, None);
        let _ = dec.compute_slice_poc(&h1, 14);

        let h2 = build_test_slice_header(0, 1, false, None);
        let poc_wrap = dec.compute_slice_poc(&h2, 15);
        assert_eq!(poc_wrap, 16, "poc_type1 frame_num wrap 计算错误");

        dec.prev_frame_num_offset_type1 = 48;
        let h3 = build_test_slice_header(0, 1, true, None);
        let poc_idr = dec.compute_slice_poc(&h3, 15);
        assert_eq!(poc_idr, 0, "poc_type1 IDR POC 应重置为 0");
        assert_eq!(
            dec.prev_frame_num_offset_type1, 0,
            "poc_type1 IDR 后 frame_num_offset 应重置"
        );
    }

    #[test]
    fn test_sample_h264_luma_qpel_full_pixel_passthrough() {
        let width = 16usize;
        let height = 8usize;
        let plane = build_linear_plane(width, height, 3, 7, 5);
        let sample = sample_h264_luma_qpel(&plane, width, width, height, 5, 3, 0, 0);
        assert_eq!(sample, plane[3 * width + 5], "整像素采样应保持原值");
    }

    #[test]
    fn test_sample_h264_luma_qpel_horizontal_half_uses_6tap() {
        let width = 16usize;
        let height = 8usize;
        let plane = build_linear_plane(width, height, 0, 10, 0);
        let sample_half = sample_h264_luma_qpel(&plane, width, width, height, 3, 2, 2, 0);
        assert_eq!(sample_half, 35, "水平半像素应使用 H264 6-tap 滤波");
    }

    #[test]
    fn test_sample_h264_luma_qpel_horizontal_quarter_average() {
        let width = 16usize;
        let height = 8usize;
        let plane = build_linear_plane(width, height, 0, 10, 0);
        let sample_q1 = sample_h264_luma_qpel(&plane, width, width, height, 3, 2, 1, 0);
        let sample_q3 = sample_h264_luma_qpel(&plane, width, width, height, 3, 2, 3, 0);
        assert_eq!(sample_q1, 33, "1/4 像素应为整像素与半像素平均");
        assert_eq!(sample_q3, 38, "3/4 像素应为半像素与下一整像素平均");
    }
}
