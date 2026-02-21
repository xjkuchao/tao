use std::collections::{HashMap, VecDeque};

use tao_core::{PixelFormat, Rational};

use crate::frame::VideoFrame;
use crate::packet::Packet;

use super::super::{
    DecRefPicMarking, H264Decoder, NalUnit, Pps, RefPlanes, ReferencePicture, SliceHeader, Sps,
};

pub fn build_test_pps() -> Pps {
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
        scaling_list_4x4: None,
        scaling_list_8x8: None,
    }
}

pub fn build_test_sps(sps_id: u32) -> Sps {
    Sps {
        profile_idc: 100,
        constraint_set_flags: 0,
        level_idc: 40,
        sps_id,
        chroma_format_idc: 1,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        max_num_ref_frames: 4,
        gaps_in_frame_num_value_allowed_flag: false,
        width: 16,
        height: 16,
        frame_mbs_only: true,
        direct_8x8_inference_flag: true,
        vui_present: false,
        fps: None,
        max_num_reorder_frames: None,
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
        qpprime_y_zero_transform_bypass_flag: false,
        scaling_list_4x4: [[16; 16]; 6],
        scaling_list_8x8: vec![[16; 64]; 2],
    }
}

pub fn build_test_sps_with_poc_type(sps_id: u32, poc_type: u32) -> Sps {
    let mut sps = build_test_sps(sps_id);
    sps.poc_type = poc_type;
    sps
}

pub fn build_test_slice_header(
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
        direct_spatial_mv_pred_flag: true,
        redundant_pic_cnt: 0,
        num_ref_idx_l0: 1,
        num_ref_idx_l1: 1,
        ref_pic_list_mod_l0: Vec::new(),
        ref_pic_list_mod_l1: Vec::new(),
        luma_log2_weight_denom: 0,
        chroma_log2_weight_denom: 0,
        l0_weights: Vec::new(),
        l1_weights: Vec::new(),
        data_bit_offset: 0,
        cabac_start_byte: 0,
        nal_ref_idc,
        is_idr,
        pic_order_cnt_lsb: poc_lsb,
        delta_poc_bottom: 0,
        delta_poc_0: 0,
        delta_poc_1: 0,
        disable_deblocking_filter_idc: 0,
        slice_alpha_c0_offset_div2: 0,
        slice_beta_offset_div2: 0,
        dec_ref_pic_marking: DecRefPicMarking::default(),
    }
}

pub fn build_test_decoder() -> H264Decoder {
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
        mv_l0_x_4x4: Vec::new(),
        mv_l0_y_4x4: Vec::new(),
        ref_idx_l0_4x4: Vec::new(),
        mv_l1_x: Vec::new(),
        mv_l1_y: Vec::new(),
        ref_idx_l1: Vec::new(),
        mv_l1_x_4x4: Vec::new(),
        mv_l1_y_4x4: Vec::new(),
        ref_idx_l1_4x4: Vec::new(),
        mb_slice_first_mb: Vec::new(),
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
        max_reference_frames: 4,
        missing_reference_fallbacks: 0,
        malformed_nal_drops: 0,
        last_sei_payloads: Vec::new(),
        pending_recovery_point_frame_cnt: None,
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

pub fn install_basic_parameter_sets(dec: &mut H264Decoder, entropy_coding_mode: u8) {
    let sps = build_test_sps(0);
    let mut pps = build_test_pps();
    pps.entropy_coding_mode = entropy_coding_mode;

    dec.sps_map.insert(0, sps.clone());
    dec.pps_map.insert(0, pps.clone());
    dec.sps = Some(sps);
    dec.pps = Some(pps);
    dec.active_sps_id = Some(0);
    dec.active_pps_id = Some(0);
}

pub fn push_dummy_reference(dec: &mut H264Decoder, frame_num: u32) {
    push_dummy_reference_with_long_term(dec, frame_num, None);
}

pub fn push_dummy_reference_with_long_term(
    dec: &mut H264Decoder,
    frame_num: u32,
    long_term_frame_idx: Option<u32>,
) {
    let total_mb = dec.mb_width * dec.mb_height;
    dec.reference_frames.push_back(ReferencePicture {
        y: vec![0u8; dec.ref_y.len()],
        u: vec![0u8; dec.ref_u.len()],
        v: vec![0u8; dec.ref_v.len()],
        mv_l0_x: vec![0i16; total_mb],
        mv_l0_y: vec![0i16; total_mb],
        ref_idx_l0: vec![-1i8; total_mb],
        frame_num,
        poc: frame_num as i32,
        long_term_frame_idx,
    });
}

pub fn push_custom_reference(
    dec: &mut H264Decoder,
    frame_num: u32,
    poc: i32,
    y_value: u8,
    long_term_frame_idx: Option<u32>,
) {
    let total_mb = dec.mb_width * dec.mb_height;
    dec.reference_frames.push_back(ReferencePicture {
        y: vec![y_value; dec.ref_y.len()],
        u: vec![128u8; dec.ref_u.len()],
        v: vec![128u8; dec.ref_v.len()],
        mv_l0_x: vec![0i16; total_mb],
        mv_l0_y: vec![0i16; total_mb],
        ref_idx_l0: vec![-1i8; total_mb],
        frame_num,
        poc,
        long_term_frame_idx,
    });
}

pub fn push_custom_reference_with_l0_motion(
    dec: &mut H264Decoder,
    frame_num: u32,
    poc: i32,
    y_value: u8,
    long_term_frame_idx: Option<u32>,
    motion: (i16, i16, i8),
) {
    let total_mb = dec.mb_width * dec.mb_height;
    let mut mv_l0_x = vec![0i16; total_mb];
    let mut mv_l0_y = vec![0i16; total_mb];
    let mut ref_idx_l0 = vec![-1i8; total_mb];
    if total_mb > 0 {
        mv_l0_x[0] = motion.0;
        mv_l0_y[0] = motion.1;
        ref_idx_l0[0] = motion.2;
    }
    dec.reference_frames.push_back(ReferencePicture {
        y: vec![y_value; dec.ref_y.len()],
        u: vec![128u8; dec.ref_u.len()],
        v: vec![128u8; dec.ref_v.len()],
        mv_l0_x,
        mv_l0_y,
        ref_idx_l0,
        frame_num,
        poc,
        long_term_frame_idx,
    });
}

pub fn push_horizontal_gradient_reference(
    dec: &mut H264Decoder,
    frame_num: u32,
    poc: i32,
    long_term_frame_idx: Option<u32>,
) {
    let total_mb = dec.mb_width * dec.mb_height;
    let mut y = vec![0u8; dec.ref_y.len()];
    for row in 0..dec.height as usize {
        for col in 0..dec.width as usize {
            y[row * dec.stride_y + col] = col.min(u8::MAX as usize) as u8;
        }
    }
    dec.reference_frames.push_back(ReferencePicture {
        y,
        u: vec![128u8; dec.ref_u.len()],
        v: vec![128u8; dec.ref_v.len()],
        mv_l0_x: vec![0i16; total_mb],
        mv_l0_y: vec![0i16; total_mb],
        ref_idx_l0: vec![-1i8; total_mb],
        frame_num,
        poc,
        long_term_frame_idx,
    });
}

pub fn build_constant_ref_planes(dec: &H264Decoder, y: u8, u: u8, v: u8) -> RefPlanes {
    RefPlanes {
        y: vec![y; dec.ref_y.len()],
        u: vec![u; dec.ref_u.len()],
        v: vec![v; dec.ref_v.len()],
        poc: 0,
        is_long_term: false,
    }
}

pub fn build_test_video_frame_with_pts(pts: i64) -> VideoFrame {
    let mut vf = VideoFrame::new(16, 16, PixelFormat::Yuv420p);
    vf.pts = pts;
    vf.time_base = Rational::new(1, 25);
    vf
}

pub fn write_ue(bits: &mut Vec<bool>, value: u32) {
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

pub fn write_se(bits: &mut Vec<bool>, value: i32) {
    let code_num = if value > 0 {
        (value as u32) * 2 - 1
    } else {
        (-value as u32) * 2
    };
    write_ue(bits, code_num);
}

pub fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
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

pub fn push_sei_ff_coded_value(bytes: &mut Vec<u8>, mut value: usize) {
    while value >= 0xFF {
        bytes.push(0xFF);
        value -= 0xFF;
    }
    bytes.push(value as u8);
}

pub fn build_recovery_point_payload(
    recovery_frame_cnt: u32,
    exact_match_flag: bool,
    broken_link_flag: bool,
    changing_slice_group_idc: u8,
) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, recovery_frame_cnt);
    bits.push(exact_match_flag);
    bits.push(broken_link_flag);
    bits.push((changing_slice_group_idc & 0b10) != 0);
    bits.push((changing_slice_group_idc & 0b01) != 0);
    bits_to_bytes(&bits)
}

pub fn build_sei_rbsp(payloads: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut rbsp = Vec::new();
    for (payload_type, payload) in payloads {
        push_sei_ff_coded_value(&mut rbsp, *payload_type as usize);
        push_sei_ff_coded_value(&mut rbsp, payload.len());
        rbsp.extend_from_slice(payload);
    }
    rbsp.push(0x80);
    rbsp
}

pub fn build_sei_avcc_packet(payloads: &[(u32, Vec<u8>)]) -> Packet {
    let mut nalu = vec![0x06]; // nal_unit_type = SEI
    nalu.extend_from_slice(&build_sei_rbsp(payloads));

    let mut avcc = Vec::with_capacity(4 + nalu.len());
    avcc.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
    avcc.extend_from_slice(&nalu);
    Packet::from_data(avcc)
}

pub fn build_pps_nalu(
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

pub fn push_bits_fixed(bits: &mut Vec<bool>, value: u32, width: usize) {
    for i in (0..width).rev() {
        bits.push(((value >> i) & 1) != 0);
    }
}

pub fn push_bits_u8(bits: &mut Vec<bool>, value: u8) {
    for i in (0..8).rev() {
        bits.push(((value >> i) & 1) != 0);
    }
}

pub fn build_rbsp_from_ues(values: &[u32]) -> Vec<u8> {
    let mut bits = Vec::new();
    for &v in values {
        write_ue(&mut bits, v);
    }
    bits_to_bytes(&bits)
}

#[derive(Clone, Copy)]
pub enum ExpGolombValue {
    Ue(u32),
    Se(i32),
}

pub fn build_rbsp_from_exp_golomb(values: &[ExpGolombValue]) -> Vec<u8> {
    let mut bits = Vec::new();
    for value in values {
        match value {
            ExpGolombValue::Ue(v) => write_ue(&mut bits, *v),
            ExpGolombValue::Se(v) => write_se(&mut bits, *v),
        }
    }
    bits_to_bytes(&bits)
}

pub fn build_sps_nalu(sps_id: u32, width: u32, height: u32) -> NalUnit {
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

pub fn build_high_profile_sps_nalu(
    sps_id: u32,
    chroma_format_idc: u32,
    frame_mbs_only: bool,
    bit_depth_luma: u32,
    bit_depth_chroma: u32,
) -> NalUnit {
    let mut bits = Vec::new();
    push_bits_u8(&mut bits, 100); // profile_idc: High
    push_bits_u8(&mut bits, 0); // constraint_set_flags
    push_bits_u8(&mut bits, 40); // level_idc
    write_ue(&mut bits, sps_id);

    write_ue(&mut bits, chroma_format_idc);
    if chroma_format_idc == 3 {
        bits.push(false); // separate_colour_plane_flag
    }
    write_ue(&mut bits, bit_depth_luma.saturating_sub(8));
    write_ue(&mut bits, bit_depth_chroma.saturating_sub(8));
    bits.push(false); // qpprime_y_zero_transform_bypass_flag
    bits.push(false); // seq_scaling_matrix_present_flag

    write_ue(&mut bits, 0); // log2_max_frame_num_minus4
    write_ue(&mut bits, 0); // pic_order_cnt_type
    write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
    write_ue(&mut bits, 4); // max_num_ref_frames
    bits.push(false); // gaps_in_frame_num_value_allowed_flag
    write_ue(&mut bits, 0); // pic_width_in_mbs_minus1 => 16
    write_ue(&mut bits, 0); // pic_height_in_map_units_minus1 => 16
    bits.push(frame_mbs_only); // frame_mbs_only_flag
    if !frame_mbs_only {
        bits.push(false); // mb_adaptive_frame_field_flag
    }
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
    NalUnit::parse(&data).expect("测试构造 High Profile SPS NAL 失败")
}

pub fn build_p_slice_header_rbsp(
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    cabac_init_idc: u32,
    qp_delta: i32,
    disable_deblocking_filter_idc: u32,
) -> Vec<u8> {
    build_p_slice_header_rbsp_with_deblock_offsets(PSliceHeaderRbspSpec {
        pps_id,
        frame_num,
        poc_lsb,
        cabac_init_idc,
        qp_delta,
        disable_deblocking_filter_idc,
        alpha_offset_div2: 0,
        beta_offset_div2: 0,
    })
}

pub struct PSliceHeaderRbspSpec {
    pub pps_id: u32,
    pub frame_num: u32,
    pub poc_lsb: u32,
    pub cabac_init_idc: u32,
    pub qp_delta: i32,
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset_div2: i32,
    pub beta_offset_div2: i32,
}

pub fn build_p_slice_header_rbsp_with_deblock_offsets(spec: PSliceHeaderRbspSpec) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // first_mb_in_slice
    write_ue(&mut bits, 0); // slice_type=P
    write_ue(&mut bits, spec.pps_id);
    push_bits_fixed(&mut bits, spec.frame_num, 4);
    push_bits_fixed(&mut bits, spec.poc_lsb, 4);
    bits.push(false); // num_ref_idx_active_override_flag
    bits.push(false); // ref_pic_list_modification_flag_l0
    write_ue(&mut bits, spec.cabac_init_idc);
    write_se(&mut bits, spec.qp_delta); // slice_qp_delta
    write_ue(&mut bits, spec.disable_deblocking_filter_idc);
    if spec.disable_deblocking_filter_idc != 1 {
        write_se(&mut bits, spec.alpha_offset_div2);
        write_se(&mut bits, spec.beta_offset_div2);
    }
    bits.push(true); // rbsp_trailing_bits stop bit
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    bits_to_bytes(&bits)
}

pub struct PWeightTableRbspSpec {
    pub pps_id: u32,
    pub frame_num: u32,
    pub poc_lsb: u32,
    pub cabac_init_idc: u32,
    pub qp_delta: i32,
    pub disable_deblocking_filter_idc: u32,
    pub luma_log2_weight_denom: u32,
    pub chroma_log2_weight_denom: u32,
    pub luma_weight: Option<(i32, i32)>,
    pub chroma_weight: Option<([i32; 2], [i32; 2])>,
}

pub fn build_p_slice_header_rbsp_with_weight_table(spec: PWeightTableRbspSpec) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // first_mb_in_slice
    write_ue(&mut bits, 0); // slice_type=P
    write_ue(&mut bits, spec.pps_id);
    push_bits_fixed(&mut bits, spec.frame_num, 4);
    push_bits_fixed(&mut bits, spec.poc_lsb, 4);
    bits.push(false); // num_ref_idx_active_override_flag
    bits.push(false); // ref_pic_list_modification_flag_l0

    write_ue(&mut bits, spec.luma_log2_weight_denom);
    write_ue(&mut bits, spec.chroma_log2_weight_denom);
    if let Some((weight, offset)) = spec.luma_weight {
        bits.push(true);
        write_se(&mut bits, weight);
        write_se(&mut bits, offset);
    } else {
        bits.push(false);
    }
    if let Some((weights, offsets)) = spec.chroma_weight {
        bits.push(true);
        for c in 0..2 {
            write_se(&mut bits, weights[c]);
            write_se(&mut bits, offsets[c]);
        }
    } else {
        bits.push(false);
    }

    write_ue(&mut bits, spec.cabac_init_idc);
    write_se(&mut bits, spec.qp_delta);
    write_ue(&mut bits, spec.disable_deblocking_filter_idc);
    if spec.disable_deblocking_filter_idc != 1 {
        write_se(&mut bits, 0);
        write_se(&mut bits, 0);
    }
    bits.push(true); // rbsp_trailing_bits stop bit
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    bits_to_bytes(&bits)
}

pub struct BWeightTableRbspSpec {
    pub pps_id: u32,
    pub frame_num: u32,
    pub poc_lsb: u32,
    pub direct_spatial_mv_pred_flag: bool,
    pub cabac_init_idc: u32,
    pub qp_delta: i32,
    pub disable_deblocking_filter_idc: u32,
    pub luma_log2_weight_denom: u32,
    pub chroma_log2_weight_denom: u32,
    pub l0_luma_weight: Option<(i32, i32)>,
    pub l0_chroma_weight: Option<([i32; 2], [i32; 2])>,
    pub l1_luma_weight: Option<(i32, i32)>,
    pub l1_chroma_weight: Option<([i32; 2], [i32; 2])>,
}

pub fn build_b_slice_header_rbsp_with_weight_table(spec: BWeightTableRbspSpec) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // first_mb_in_slice
    write_ue(&mut bits, 1); // slice_type=B
    write_ue(&mut bits, spec.pps_id);
    push_bits_fixed(&mut bits, spec.frame_num, 4);
    push_bits_fixed(&mut bits, spec.poc_lsb, 4);
    bits.push(spec.direct_spatial_mv_pred_flag); // direct_spatial_mv_pred_flag
    bits.push(false); // num_ref_idx_active_override_flag
    bits.push(false); // ref_pic_list_modification_flag_l0
    bits.push(false); // ref_pic_list_modification_flag_l1

    write_ue(&mut bits, spec.luma_log2_weight_denom);
    write_ue(&mut bits, spec.chroma_log2_weight_denom);
    if let Some((weight, offset)) = spec.l0_luma_weight {
        bits.push(true);
        write_se(&mut bits, weight);
        write_se(&mut bits, offset);
    } else {
        bits.push(false);
    }
    if let Some((weights, offsets)) = spec.l0_chroma_weight {
        bits.push(true);
        for c in 0..2 {
            write_se(&mut bits, weights[c]);
            write_se(&mut bits, offsets[c]);
        }
    } else {
        bits.push(false);
    }
    if let Some((weight, offset)) = spec.l1_luma_weight {
        bits.push(true);
        write_se(&mut bits, weight);
        write_se(&mut bits, offset);
    } else {
        bits.push(false);
    }
    if let Some((weights, offsets)) = spec.l1_chroma_weight {
        bits.push(true);
        for c in 0..2 {
            write_se(&mut bits, weights[c]);
            write_se(&mut bits, offsets[c]);
        }
    } else {
        bits.push(false);
    }

    write_ue(&mut bits, spec.cabac_init_idc);
    write_se(&mut bits, spec.qp_delta);
    write_ue(&mut bits, spec.disable_deblocking_filter_idc);
    if spec.disable_deblocking_filter_idc != 1 {
        write_se(&mut bits, 0);
        write_se(&mut bits, 0);
    }
    bits.push(true); // rbsp_trailing_bits stop bit
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    bits_to_bytes(&bits)
}

pub fn build_b_slice_header_rbsp_with_direct_flag(
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    direct_spatial_mv_pred_flag: bool,
) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // first_mb_in_slice
    write_ue(&mut bits, 1); // slice_type=B
    write_ue(&mut bits, pps_id);
    push_bits_fixed(&mut bits, frame_num, 4);
    push_bits_fixed(&mut bits, poc_lsb, 4);
    bits.push(direct_spatial_mv_pred_flag);
    bits.push(false); // num_ref_idx_active_override_flag
    bits.push(false); // ref_pic_list_modification_flag_l0
    bits.push(false); // ref_pic_list_modification_flag_l1
    write_ue(&mut bits, 0); // cabac_init_idc
    write_se(&mut bits, 0); // slice_qp_delta
    write_ue(&mut bits, 1); // disable_deblocking_filter_idc
    bits.push(true); // rbsp_trailing_bits stop bit
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    bits_to_bytes(&bits)
}

pub fn build_p_slice_header_rbsp_with_redundant_pic_cnt(
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    redundant_pic_cnt: u32,
) -> Vec<u8> {
    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // first_mb_in_slice
    write_ue(&mut bits, 0); // slice_type=P
    write_ue(&mut bits, pps_id);
    push_bits_fixed(&mut bits, frame_num, 4);
    push_bits_fixed(&mut bits, poc_lsb, 4);
    write_ue(&mut bits, redundant_pic_cnt);
    bits.push(false); // num_ref_idx_active_override_flag
    bits.push(false); // ref_pic_list_modification_flag_l0
    write_ue(&mut bits, 0); // cabac_init_idc
    write_se(&mut bits, 0); // slice_qp_delta
    write_ue(&mut bits, 1); // disable_deblocking_filter_idc
    bits.push(true); // rbsp_trailing_bits stop bit
    while bits.len() % 8 != 0 {
        bits.push(false);
    }
    bits_to_bytes(&bits)
}

pub fn build_p_slice_header_rbsp_with_l0_reorder(
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

pub fn build_p_slice_header_rbsp_poc_type1(
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

pub fn build_linear_plane(
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
