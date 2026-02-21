use std::collections::{HashMap, VecDeque};

use tao_core::bitreader::BitReader;
use tao_core::{PixelFormat, Rational};

use crate::decoder::Decoder;
use crate::frame::{Frame, VideoFrame};
use crate::packet::Packet;

use super::{
    BMotion, DecRefPicMarking, H264Decoder, MmcoOp, NalUnit, ParameterSetRebuildAction,
    PendingFrameMeta, Pps, PredWeightL0, RefPicListMod, RefPlanes, ReferencePicture, SliceHeader,
    Sps, sample_h264_chroma_qpel, sample_h264_luma_qpel,
    sei::{SeiMessage, parse_sei_rbsp},
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
        scaling_list_4x4: None,
        scaling_list_8x8: None,
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

fn install_basic_parameter_sets(dec: &mut H264Decoder, entropy_coding_mode: u8) {
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

fn push_dummy_reference(dec: &mut H264Decoder, frame_num: u32) {
    push_dummy_reference_with_long_term(dec, frame_num, None);
}

fn push_dummy_reference_with_long_term(
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

fn push_custom_reference(
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

fn push_custom_reference_with_l0_motion(
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

fn push_horizontal_gradient_reference(
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

fn build_constant_ref_planes(dec: &H264Decoder, y: u8, u: u8, v: u8) -> RefPlanes {
    RefPlanes {
        y: vec![y; dec.ref_y.len()],
        u: vec![u; dec.ref_u.len()],
        v: vec![v; dec.ref_v.len()],
        poc: 0,
        is_long_term: false,
    }
}

fn build_test_video_frame_with_pts(pts: i64) -> VideoFrame {
    let mut vf = VideoFrame::new(16, 16, PixelFormat::Yuv420p);
    vf.pts = pts;
    vf.time_base = Rational::new(1, 25);
    vf
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

fn push_sei_ff_coded_value(bytes: &mut Vec<u8>, mut value: usize) {
    while value >= 0xFF {
        bytes.push(0xFF);
        value -= 0xFF;
    }
    bytes.push(value as u8);
}

fn build_recovery_point_payload(
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

fn build_sei_rbsp(payloads: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut rbsp = Vec::new();
    for (payload_type, payload) in payloads {
        push_sei_ff_coded_value(&mut rbsp, *payload_type as usize);
        push_sei_ff_coded_value(&mut rbsp, payload.len());
        rbsp.extend_from_slice(payload);
    }
    rbsp.push(0x80);
    rbsp
}

fn build_sei_avcc_packet(payloads: &[(u32, Vec<u8>)]) -> Packet {
    let mut nalu = vec![0x06]; // nal_unit_type = SEI
    nalu.extend_from_slice(&build_sei_rbsp(payloads));

    let mut avcc = Vec::with_capacity(4 + nalu.len());
    avcc.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
    avcc.extend_from_slice(&nalu);
    Packet::from_data(avcc)
}

fn build_pps_nalu(pps_id: u32, sps_id: u32, entropy: bool, pic_init_qp_minus26: i32) -> NalUnit {
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

#[derive(Clone, Copy)]
enum ExpGolombValue {
    Ue(u32),
    Se(i32),
}

fn build_rbsp_from_exp_golomb(values: &[ExpGolombValue]) -> Vec<u8> {
    let mut bits = Vec::new();
    for value in values {
        match value {
            ExpGolombValue::Ue(v) => write_ue(&mut bits, *v),
            ExpGolombValue::Se(v) => write_se(&mut bits, *v),
        }
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

fn build_high_profile_sps_nalu(
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

fn build_p_slice_header_rbsp(
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

struct PSliceHeaderRbspSpec {
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    cabac_init_idc: u32,
    qp_delta: i32,
    disable_deblocking_filter_idc: u32,
    alpha_offset_div2: i32,
    beta_offset_div2: i32,
}

fn build_p_slice_header_rbsp_with_deblock_offsets(spec: PSliceHeaderRbspSpec) -> Vec<u8> {
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

struct PWeightTableRbspSpec {
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    cabac_init_idc: u32,
    qp_delta: i32,
    disable_deblocking_filter_idc: u32,
    luma_log2_weight_denom: u32,
    chroma_log2_weight_denom: u32,
    luma_weight: Option<(i32, i32)>,
    chroma_weight: Option<([i32; 2], [i32; 2])>,
}

fn build_p_slice_header_rbsp_with_weight_table(spec: PWeightTableRbspSpec) -> Vec<u8> {
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

struct BWeightTableRbspSpec {
    pps_id: u32,
    frame_num: u32,
    poc_lsb: u32,
    direct_spatial_mv_pred_flag: bool,
    cabac_init_idc: u32,
    qp_delta: i32,
    disable_deblocking_filter_idc: u32,
    luma_log2_weight_denom: u32,
    chroma_log2_weight_denom: u32,
    l0_luma_weight: Option<(i32, i32)>,
    l0_chroma_weight: Option<([i32; 2], [i32; 2])>,
    l1_luma_weight: Option<(i32, i32)>,
    l1_chroma_weight: Option<([i32; 2], [i32; 2])>,
}

fn build_b_slice_header_rbsp_with_weight_table(spec: BWeightTableRbspSpec) -> Vec<u8> {
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

fn build_b_slice_header_rbsp_with_direct_flag(
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

fn build_p_slice_header_rbsp_with_redundant_pic_cnt(
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

fn build_linear_plane(width: usize, height: usize, offset: u8, step_x: u8, step_y: u8) -> Vec<u8> {
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
fn test_active_luma_scaling_list_prefers_pps_override() {
    let mut dec = build_test_decoder();
    let mut sps = build_test_sps(0);
    sps.scaling_list_4x4[0] = [11; 16];
    sps.scaling_list_4x4[3] = [13; 16];
    sps.scaling_list_8x8[0] = [12; 64];
    sps.scaling_list_8x8[1] = [14; 64];

    let mut pps = build_test_pps();
    let mut pps_4x4 = [[16u8; 16]; 6];
    pps_4x4[0] = [21; 16];
    pps_4x4[3] = [23; 16];
    let pps_8x8 = vec![[22u8; 64], [24u8; 64]];
    pps.scaling_list_4x4 = Some(pps_4x4);
    pps.scaling_list_8x8 = Some(pps_8x8);

    dec.sps = Some(sps);
    dec.pps = Some(pps);

    assert_eq!(
        dec.active_luma_scaling_list_4x4(true)[0],
        21,
        "Luma Intra 4x4 应优先使用 PPS 覆盖"
    );
    assert_eq!(
        dec.active_luma_scaling_list_4x4(false)[0],
        23,
        "Luma Inter 4x4 应优先使用 PPS 覆盖"
    );
    assert_eq!(
        dec.active_luma_scaling_list_8x8(true)[0],
        22,
        "Luma Intra 8x8 应优先使用 PPS 覆盖"
    );
    assert_eq!(
        dec.active_luma_scaling_list_8x8(false)[0],
        24,
        "Luma Inter 8x8 应优先使用 PPS 覆盖"
    );
}

#[test]
fn test_active_chroma_scaling_list_fallback_to_sps_when_pps_absent() {
    let mut dec = build_test_decoder();
    let mut sps = build_test_sps(0);
    sps.scaling_list_4x4[1] = [31; 16];
    sps.scaling_list_4x4[2] = [32; 16];
    sps.scaling_list_4x4[4] = [34; 16];
    sps.scaling_list_4x4[5] = [35; 16];

    let mut pps = build_test_pps();
    pps.scaling_list_4x4 = None;

    dec.sps = Some(sps);
    dec.pps = Some(pps);

    assert_eq!(
        dec.active_chroma_scaling_list_4x4(true, false)[0],
        31,
        "Chroma U Intra 4x4 应回退到 SPS 矩阵"
    );
    assert_eq!(
        dec.active_chroma_scaling_list_4x4(true, true)[0],
        32,
        "Chroma V Intra 4x4 应回退到 SPS 矩阵"
    );
    assert_eq!(
        dec.active_chroma_scaling_list_4x4(false, false)[0],
        34,
        "Chroma U Inter 4x4 应回退到 SPS 矩阵"
    );
    assert_eq!(
        dec.active_chroma_scaling_list_4x4(false, true)[0],
        35,
        "Chroma V Inter 4x4 应回退到 SPS 矩阵"
    );
}

#[test]
fn test_transform_bypass_requires_sps_flag_and_qp_zero() {
    let mut dec = build_test_decoder();
    let mut sps = build_test_sps(0);
    sps.qpprime_y_zero_transform_bypass_flag = true;
    dec.sps = Some(sps);

    assert!(
        dec.is_transform_bypass_active(0),
        "SPS 开启且 QP=0 时应启用变换旁路"
    );
    assert!(
        !dec.is_transform_bypass_active(1),
        "SPS 开启但 QP!=0 时不应启用变换旁路"
    );

    if let Some(sps_mut) = dec.sps.as_mut() {
        sps_mut.qpprime_y_zero_transform_bypass_flag = false;
    }
    assert!(
        !dec.is_transform_bypass_active(0),
        "SPS 关闭时即使 QP=0 也不应启用变换旁路"
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
fn test_activate_parameter_sets_reject_unsupported_bound_sps() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0.clone());
    dec.sps = Some(sps0);
    dec.active_sps_id = Some(0);

    let pps0 = build_test_pps();
    dec.pps_map.insert(0, pps0.clone());
    dec.pps = Some(pps0);
    dec.active_pps_id = Some(0);

    let mut sps1 = build_test_sps(1);
    sps1.frame_mbs_only = false;
    dec.sps_map.insert(1, sps1);

    let mut pps1 = build_test_pps();
    pps1.pps_id = 1;
    pps1.sps_id = 1;
    dec.pps_map.insert(1, pps1);

    let err = dec
        .activate_parameter_sets(1)
        .expect_err("绑定到不支持 SPS 的 PPS 激活应失败");
    let msg = format!("{}", err);
    assert!(
        msg.contains("不受支持"),
        "错误信息应提示 SPS 不受支持, actual={}",
        msg
    );
    assert_eq!(
        dec.active_sps_id,
        Some(0),
        "失败后不应覆盖当前 active_sps_id"
    );
    assert_eq!(
        dec.active_pps_id,
        Some(0),
        "失败后不应覆盖当前 active_pps_id"
    );
}

#[test]
fn test_parse_sps_pps_from_config_accept_supported_parameter_sets() {
    let mut dec = build_test_decoder();
    let sps = build_sps_nalu(0, 16, 16);
    let pps = build_pps_nalu(0, 0, true, 0);
    let avcc = crate::parsers::h264::build_avcc_config(
        std::slice::from_ref(&sps.data),
        std::slice::from_ref(&pps.data),
        4,
    )
    .expect("构造 avcC 配置失败");
    let config = crate::parsers::h264::parse_avcc_config(&avcc).expect("解析 avcC 配置失败");

    dec.parse_sps_pps_from_config(&config)
        .expect("受支持参数集应可激活");

    assert_eq!(dec.active_sps_id, Some(0), "应激活 sps_id=0");
    assert_eq!(dec.active_pps_id, Some(0), "应激活 pps_id=0");
    assert!(dec.sps.is_some(), "应缓存受支持 SPS");
    assert!(dec.pps.is_some(), "应缓存受支持 PPS");
}

#[test]
fn test_parse_sps_pps_from_config_reject_all_unsupported_sps() {
    let mut dec = build_test_decoder();
    let unsupported_sps = build_high_profile_sps_nalu(0, 2, true, 8, 8); // 4:2:2, 当前不支持
    let pps = build_pps_nalu(0, 0, true, 0);
    let avcc = crate::parsers::h264::build_avcc_config(
        std::slice::from_ref(&unsupported_sps.data),
        std::slice::from_ref(&pps.data),
        4,
    )
    .expect("构造 avcC 配置失败");
    let config = crate::parsers::h264::parse_avcc_config(&avcc).expect("解析 avcC 配置失败");

    let err = dec
        .parse_sps_pps_from_config(&config)
        .expect_err("全部 SPS 不受支持时应失败");
    let msg = format!("{}", err);
    assert!(
        msg.contains("未找到受支持的 SPS"),
        "错误信息应提示未找到受支持 SPS, actual={}",
        msg
    );
    assert!(dec.sps.is_none(), "失败后不应缓存不支持的 SPS");
    assert!(dec.pps.is_none(), "失败后不应激活依赖无效 SPS 的 PPS");
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
fn test_parse_dec_ref_pic_marking_idr_flags() {
    let dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x65]).expect("测试构造 IDR NAL 失败");

    let bits = vec![true, false];
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);
    let marking = dec
        .parse_dec_ref_pic_marking(&mut br, &nalu)
        .expect("IDR dec_ref_pic_marking 应可解析");

    assert!(marking.is_idr, "IDR NAL 应标记 is_idr=true");
    assert!(
        marking.no_output_of_prior_pics,
        "第一个标志位应映射 no_output_of_prior_pics"
    );
    assert!(
        !marking.long_term_reference_flag,
        "第二个标志位应映射 long_term_reference_flag"
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_too_many_mmco_ops() {
    let dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    for _ in 0..65 {
        write_ue(&mut bits, 5); // MMCO5: ClearAll
    }
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("超过上限的 MMCO 操作应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO 操作数量过多"),
        "错误信息应包含 MMCO 上限提示, actual={}",
        msg
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_mmco1_difference_out_of_range() {
    let mut dec = build_test_decoder();
    dec.sps = Some(build_test_sps(0)); // log2_max_frame_num=4 => max difference=15
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    write_ue(&mut bits, 1); // MMCO1
    write_ue(&mut bits, 16); // difference_of_pic_nums_minus1, 超过 max=15
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("MMCO1 difference 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO1"),
        "错误信息应包含 MMCO1, actual={}",
        msg
    );
    assert!(
        msg.contains("difference_of_pic_nums_minus1"),
        "错误信息应包含 difference_of_pic_nums_minus1, actual={}",
        msg
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_mmco3_difference_out_of_range() {
    let mut dec = build_test_decoder();
    dec.sps = Some(build_test_sps(0)); // log2_max_frame_num=4 => max difference=15
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    write_ue(&mut bits, 3); // MMCO3
    write_ue(&mut bits, 16); // difference_of_pic_nums_minus1, 超过 max=15
    write_ue(&mut bits, 0); // long_term_frame_idx
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("MMCO3 difference 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO3"),
        "错误信息应包含 MMCO3, actual={}",
        msg
    );
    assert!(
        msg.contains("difference_of_pic_nums_minus1"),
        "错误信息应包含 difference_of_pic_nums_minus1, actual={}",
        msg
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_mmco2_long_term_pic_num_out_of_range() {
    let dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    write_ue(&mut bits, 2); // MMCO2
    write_ue(&mut bits, 4); // long_term_pic_num, 超过 max=3
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("MMCO2 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO2"),
        "错误信息应包含 MMCO2, actual={}",
        msg
    );
    assert!(
        msg.contains("超范围"),
        "错误信息应提示超范围, actual={}",
        msg
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_mmco4_max_long_term_idx_out_of_range() {
    let dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    write_ue(&mut bits, 4); // MMCO4
    write_ue(&mut bits, 5); // max_long_term_frame_idx_plus1, 超过 max_reference_frames=4
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("MMCO4 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO4"),
        "错误信息应包含 MMCO4, actual={}",
        msg
    );
    assert!(
        msg.contains("超范围"),
        "错误信息应提示超范围, actual={}",
        msg
    );
}

#[test]
fn test_parse_dec_ref_pic_marking_reject_mmco6_long_term_idx_out_of_range() {
    let dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x61]).expect("测试构造非 IDR slice NAL 失败");

    let mut bits = Vec::new();
    bits.push(true); // adaptive_ref_pic_marking_mode_flag
    write_ue(&mut bits, 6); // MMCO6
    write_ue(&mut bits, 4); // long_term_frame_idx, 超过 max=3
    write_ue(&mut bits, 0); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_dec_ref_pic_marking(&mut br, &nalu) {
        Ok(_) => panic!("MMCO6 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("MMCO6"),
        "错误信息应包含 MMCO6, actual={}",
        msg
    );
    assert!(
        msg.contains("超范围"),
        "错误信息应提示超范围, actual={}",
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
    assert_eq!(
        header.slice_alpha_c0_offset_div2, 0,
        "disable_deblocking_filter_idc=1 时 alpha offset 应默认为 0"
    );
    assert_eq!(
        header.slice_beta_offset_div2, 0,
        "disable_deblocking_filter_idc=1 时 beta offset 应默认为 0"
    );
}

#[test]
fn test_parse_slice_header_store_b_direct_spatial_mv_pred_flag() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let pps0 = build_test_pps();
    dec.pps_map.insert(0, pps0);

    let rbsp = build_b_slice_header_rbsp_with_direct_flag(0, 0, 0, false);
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 B-slice NAL 失败");
    let header = dec
        .parse_slice_header(&rbsp, &nalu)
        .expect("B-slice direct_spatial_mv_pred_flag 应可解析");

    assert!(
        !header.direct_spatial_mv_pred_flag,
        "slice header 应保存 direct_spatial_mv_pred_flag=false"
    );
}

#[test]
fn test_parse_slice_header_store_redundant_pic_cnt() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.redundant_pic_cnt_present = true;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_redundant_pic_cnt(0, 1, 2, 3);
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 P-slice NAL 失败");
    let header = dec
        .parse_slice_header(&rbsp, &nalu)
        .expect("带 redundant_pic_cnt 的 slice header 应可解析");

    assert_eq!(
        header.redundant_pic_cnt, 3,
        "slice header 应保存 redundant_pic_cnt"
    );
}

#[test]
fn test_parse_slice_header_reject_alpha_offset_out_of_range() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let pps0 = build_test_pps();
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_deblock_offsets(PSliceHeaderRbspSpec {
        pps_id: 0,
        frame_num: 0,
        poc_lsb: 0,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 0,
        alpha_offset_div2: 7,
        beta_offset_div2: 0,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let err = match dec.parse_slice_header(&rbsp, &nalu) {
        Ok(_) => panic!("alpha offset 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("slice_alpha_c0_offset_div2"),
        "错误信息应包含 alpha offset 字段, actual={}",
        msg
    );
}

#[test]
fn test_parse_slice_header_reject_beta_offset_out_of_range() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let pps0 = build_test_pps();
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_deblock_offsets(PSliceHeaderRbspSpec {
        pps_id: 0,
        frame_num: 0,
        poc_lsb: 0,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 0,
        alpha_offset_div2: 0,
        beta_offset_div2: -7,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let err = match dec.parse_slice_header(&rbsp, &nalu) {
        Ok(_) => panic!("beta offset 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("slice_beta_offset_div2"),
        "错误信息应包含 beta offset 字段, actual={}",
        msg
    );
}

#[test]
fn test_parse_slice_header_store_deblocking_offsets() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let pps0 = build_test_pps();
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_deblock_offsets(PSliceHeaderRbspSpec {
        pps_id: 0,
        frame_num: 1,
        poc_lsb: 2,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 0,
        alpha_offset_div2: 2,
        beta_offset_div2: -1,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let header = dec
        .parse_slice_header(&rbsp, &nalu)
        .expect("带 deblock offset 的 slice header 应可解析");
    assert_eq!(
        header.slice_alpha_c0_offset_div2, 2,
        "slice header 应保存 alpha offset"
    );
    assert_eq!(
        header.slice_beta_offset_div2, -1,
        "slice header 应保存 beta offset"
    );
}

#[test]
fn test_parse_slice_header_reject_luma_weight_denom_out_of_range() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.weighted_pred = true;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_weight_table(PWeightTableRbspSpec {
        pps_id: 0,
        frame_num: 0,
        poc_lsb: 0,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 1,
        luma_log2_weight_denom: 8,
        chroma_log2_weight_denom: 0,
        luma_weight: None,
        chroma_weight: None,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let err = match dec.parse_slice_header(&rbsp, &nalu) {
        Ok(_) => panic!("luma_log2_weight_denom 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("luma_log2_weight_denom"),
        "错误信息应包含 luma_log2_weight_denom, actual={}",
        msg
    );
}

#[test]
fn test_parse_slice_header_reject_chroma_weight_denom_out_of_range() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.weighted_pred = true;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_weight_table(PWeightTableRbspSpec {
        pps_id: 0,
        frame_num: 0,
        poc_lsb: 0,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 1,
        luma_log2_weight_denom: 0,
        chroma_log2_weight_denom: 8,
        luma_weight: None,
        chroma_weight: None,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let err = match dec.parse_slice_header(&rbsp, &nalu) {
        Ok(_) => panic!("chroma_log2_weight_denom 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("chroma_log2_weight_denom"),
        "错误信息应包含 chroma_log2_weight_denom, actual={}",
        msg
    );
}

#[test]
fn test_parse_slice_header_store_l0_pred_weights() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.weighted_pred = true;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_p_slice_header_rbsp_with_weight_table(PWeightTableRbspSpec {
        pps_id: 0,
        frame_num: 1,
        poc_lsb: 1,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 1,
        luma_log2_weight_denom: 1,
        chroma_log2_weight_denom: 2,
        luma_weight: Some((3, -2)),
        chroma_weight: Some(([2, -3], [4, -5])),
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 slice NAL 失败");
    let header = dec
        .parse_slice_header(&rbsp, &nalu)
        .expect("带权重表的 slice header 应可解析");

    assert_eq!(
        header.luma_log2_weight_denom, 1,
        "应保存 luma_log2_weight_denom"
    );
    assert_eq!(
        header.chroma_log2_weight_denom, 2,
        "应保存 chroma_log2_weight_denom"
    );
    assert_eq!(header.l0_weights.len(), 1, "应解析出 1 个 l0 权重项");
    assert_eq!(
        header.l0_weights[0].luma_weight, 3,
        "l0 luma_weight 解析错误"
    );
    assert_eq!(
        header.l0_weights[0].luma_offset, -2,
        "l0 luma_offset 解析错误"
    );
    assert_eq!(
        header.l0_weights[0].chroma_weight,
        [2, -3],
        "l0 chroma_weight 解析错误"
    );
    assert_eq!(
        header.l0_weights[0].chroma_offset,
        [4, -5],
        "l0 chroma_offset 解析错误"
    );
}

#[test]
fn test_parse_slice_header_store_l1_pred_weights_for_b_slice() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.weighted_bipred_idc = 1;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_b_slice_header_rbsp_with_weight_table(BWeightTableRbspSpec {
        pps_id: 0,
        frame_num: 1,
        poc_lsb: 1,
        direct_spatial_mv_pred_flag: true,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 1,
        luma_log2_weight_denom: 2,
        chroma_log2_weight_denom: 1,
        l0_luma_weight: Some((4, -1)),
        l0_chroma_weight: Some(([2, 3], [-2, -3])),
        l1_luma_weight: Some((-5, 6)),
        l1_chroma_weight: Some(([-4, 7], [8, -9])),
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 B-slice NAL 失败");
    let header = dec
        .parse_slice_header(&rbsp, &nalu)
        .expect("B-slice 权重表应可解析");

    assert_eq!(header.l0_weights.len(), 1, "B-slice 应保存 l0_weights");
    assert_eq!(header.l1_weights.len(), 1, "B-slice 应保存 l1_weights");
    assert_eq!(
        header.l1_weights[0].luma_weight, -5,
        "l1 luma_weight 解析错误"
    );
    assert_eq!(
        header.l1_weights[0].luma_offset, 6,
        "l1 luma_offset 解析错误"
    );
    assert_eq!(
        header.l1_weights[0].chroma_weight,
        [-4, 7],
        "l1 chroma_weight 解析错误"
    );
    assert_eq!(
        header.l1_weights[0].chroma_offset,
        [8, -9],
        "l1 chroma_offset 解析错误"
    );
}

#[test]
fn test_parse_slice_header_reject_l1_luma_weight_out_of_range() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0);

    let mut pps0 = build_test_pps();
    pps0.weighted_bipred_idc = 1;
    dec.pps_map.insert(0, pps0);

    let rbsp = build_b_slice_header_rbsp_with_weight_table(BWeightTableRbspSpec {
        pps_id: 0,
        frame_num: 0,
        poc_lsb: 0,
        direct_spatial_mv_pred_flag: true,
        cabac_init_idc: 0,
        qp_delta: 0,
        disable_deblocking_filter_idc: 1,
        luma_log2_weight_denom: 0,
        chroma_log2_weight_denom: 0,
        l0_luma_weight: None,
        l0_chroma_weight: None,
        l1_luma_weight: Some((128, 0)),
        l1_chroma_weight: None,
    });
    let nalu = NalUnit::parse(&[0x01]).expect("测试构造 B-slice NAL 失败");
    let err = match dec.parse_slice_header(&rbsp, &nalu) {
        Ok(_) => panic!("l1 luma_weight 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("luma_weight_l1"),
        "错误信息应包含 luma_weight_l1, actual={}",
        msg
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
fn test_parse_single_ref_pic_list_mod_reject_abs_diff_out_of_range() {
    let mut dec = build_test_decoder();
    dec.sps = Some(build_test_sps(0)); // log2_max_frame_num=4 => max abs_diff=15

    let mut bits = Vec::new();
    write_ue(&mut bits, 0); // short-term subtraction
    write_ue(&mut bits, 16); // abs_diff_pic_num_minus1, 超过 max=15
    write_ue(&mut bits, 3); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_single_ref_pic_list_mod(&mut br) {
        Ok(_) => panic!("abs_diff_pic_num_minus1 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("abs_diff_pic_num_minus1"),
        "错误信息应包含 abs_diff_pic_num_minus1, actual={}",
        msg
    );
    assert!(
        msg.contains("超范围"),
        "错误信息应提示超范围, actual={}",
        msg
    );
}

#[test]
fn test_parse_single_ref_pic_list_mod_reject_long_term_pic_num_out_of_range() {
    let dec = build_test_decoder();

    let mut bits = Vec::new();
    write_ue(&mut bits, 2); // long-term
    write_ue(&mut bits, 4); // long_term_pic_num, 超过 max=3
    write_ue(&mut bits, 3); // 结束符
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);

    let err = match dec.parse_single_ref_pic_list_mod(&mut br) {
        Ok(_) => panic!("long_term_pic_num 超范围应失败"),
        Err(err) => err,
    };
    let msg = format!("{}", err);
    assert!(
        msg.contains("long_term_pic_num"),
        "错误信息应包含 long_term_pic_num, actual={}",
        msg
    );
    assert!(
        msg.contains("超范围"),
        "错误信息应提示超范围, actual={}",
        msg
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
fn test_handle_sps_stores_direct_8x8_inference_flag() {
    let mut dec = build_test_decoder();
    let sps = build_sps_nalu(0, 16, 16);
    dec.handle_sps(&sps);

    let cached = dec.sps.as_ref().expect("handle_sps 后应缓存当前 SPS");
    assert!(
        !cached.direct_8x8_inference_flag,
        "应从 SPS RBSP 正确解析并缓存 direct_8x8_inference_flag"
    );
}

#[test]
fn test_handle_sps_same_id_unsupported_update_keeps_previous_sps() {
    let mut dec = build_test_decoder();
    let sps0 = build_test_sps(0);
    dec.sps_map.insert(0, sps0.clone());
    dec.sps = Some(sps0);
    dec.active_sps_id = Some(0);
    dec.activate_sps(0);
    assert_eq!(dec.width, 16, "基线 SPS 激活后宽度应为 16");
    assert_eq!(dec.height, 16, "基线 SPS 激活后高度应为 16");

    let unsupported_same_id = build_high_profile_sps_nalu(0, 2, true, 8, 8);
    dec.handle_sps(&unsupported_same_id);

    assert_eq!(
        dec.active_sps_id,
        Some(0),
        "同 id 的不支持 SPS 不应改变 active_sps_id"
    );
    assert_eq!(dec.width, 16, "不支持 SPS 不应修改当前解码宽度");
    assert_eq!(dec.height, 16, "不支持 SPS 不应修改当前解码高度");
    let stored = dec.sps_map.get(&0).expect("应保留原有 sps_id=0 的 SPS");
    assert_eq!(
        stored.chroma_format_idc, 1,
        "不支持 SPS 不应覆盖已缓存的可用 SPS"
    );
}

#[test]
fn test_activate_sps_reject_unsupported_chroma_format() {
    let mut dec = build_test_decoder();
    let base = build_test_sps(0);
    dec.sps_map.insert(0, base.clone());
    dec.activate_sps(0);
    assert_eq!(dec.active_sps_id, Some(0), "基线 SPS 应激活成功");

    let mut unsupported = build_test_sps(1);
    unsupported.chroma_format_idc = 2;
    unsupported.width = 32;
    unsupported.height = 32;
    dec.sps_map.insert(1, unsupported);

    dec.activate_sps(1);

    assert_eq!(
        dec.active_sps_id,
        Some(0),
        "不支持的 chroma_format_idc 不应覆盖当前激活 SPS"
    );
    assert_eq!(dec.width, 16, "不支持 SPS 不应修改解码宽度");
    assert_eq!(dec.height, 16, "不支持 SPS 不应修改解码高度");
}

#[test]
fn test_activate_sps_reject_unsupported_interlaced_stream() {
    let mut dec = build_test_decoder();
    let base = build_test_sps(0);
    dec.sps_map.insert(0, base.clone());
    dec.activate_sps(0);
    assert_eq!(dec.active_sps_id, Some(0), "基线 SPS 应激活成功");

    let mut unsupported = build_test_sps(1);
    unsupported.frame_mbs_only = false;
    unsupported.width = 32;
    unsupported.height = 32;
    dec.sps_map.insert(1, unsupported);

    dec.activate_sps(1);

    assert_eq!(
        dec.active_sps_id,
        Some(0),
        "场编码 SPS 当前未支持, 不应覆盖当前激活 SPS"
    );
    assert_eq!(dec.width, 16, "不支持 SPS 不应修改解码宽度");
    assert_eq!(dec.height, 16, "不支持 SPS 不应修改解码高度");
}

#[test]
fn test_activate_sps_reject_unsupported_high_bit_depth() {
    let mut dec = build_test_decoder();
    let base = build_test_sps(0);
    dec.sps_map.insert(0, base.clone());
    dec.activate_sps(0);
    assert_eq!(dec.active_sps_id, Some(0), "基线 SPS 应激活成功");

    let mut unsupported = build_test_sps(1);
    unsupported.bit_depth_luma = 10;
    unsupported.bit_depth_chroma = 10;
    unsupported.width = 32;
    unsupported.height = 32;
    dec.sps_map.insert(1, unsupported);

    dec.activate_sps(1);

    assert_eq!(
        dec.active_sps_id,
        Some(0),
        "高位深 SPS 当前未支持, 不应覆盖当前激活 SPS"
    );
    assert_eq!(dec.width, 16, "不支持 SPS 不应修改解码宽度");
    assert_eq!(dec.height, 16, "不支持 SPS 不应修改解码高度");
}

#[test]
fn test_activate_sps_updates_reorder_depth_from_sps_max_ref_frames() {
    let mut dec = build_test_decoder();
    dec.reorder_depth = 9;

    let mut sps = build_test_sps(3);
    sps.max_num_ref_frames = 1;
    dec.sps_map.insert(3, sps);

    dec.activate_sps(3);

    assert_eq!(
        dec.reorder_depth, 0,
        "未配置覆盖时, reorder_depth 应按 max_num_ref_frames-1 自适应"
    );
}

#[test]
fn test_activate_sps_reorder_depth_clamped_by_max_num_reorder_frames() {
    let mut dec = build_test_decoder();
    dec.reorder_depth = 9;

    let mut sps = build_test_sps(7);
    sps.max_num_ref_frames = 4;
    sps.max_num_reorder_frames = Some(1);
    dec.sps_map.insert(7, sps);

    dec.activate_sps(7);

    assert_eq!(
        dec.reorder_depth, 1,
        "未配置覆盖时, reorder_depth 应被 max_num_reorder_frames 进一步约束"
    );
}

#[test]
fn test_derive_level_max_dpb_frames_limits_large_picture() {
    let mut sps = build_test_sps(5);
    sps.level_idc = 10;
    sps.width = 1280;
    sps.height = 720;
    sps.pic_width_in_mbs = 80;
    sps.pic_height_in_map_units = 45;

    assert_eq!(
        H264Decoder::derive_level_max_dpb_frames(&sps),
        1,
        "Level 1.0 下 1280x720 应被限制为至少 1 帧 DPB"
    );
}

#[test]
fn test_activate_sps_caps_max_reference_frames_by_level_limit() {
    let mut dec = build_test_decoder();
    let mut sps = build_test_sps(6);
    sps.level_idc = 10;
    sps.max_num_ref_frames = 16;
    sps.width = 1280;
    sps.height = 720;
    sps.pic_width_in_mbs = 80;
    sps.pic_height_in_map_units = 45;
    dec.sps_map.insert(6, sps);

    dec.activate_sps(6);

    assert_eq!(dec.active_sps_id, Some(6), "SPS 应激活成功");
    assert_eq!(
        dec.max_reference_frames, 1,
        "应按 level 限制将 max_reference_frames 收敛到 max_dpb_frames"
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
fn test_store_reference_with_marking_mmco_clear_all_resets_frame_num_and_poc() {
    let mut dec = build_test_decoder();
    push_dummy_reference(&mut dec, 2);
    push_dummy_reference_with_long_term(&mut dec, 6, Some(0));
    dec.prev_ref_poc_msb = 32;
    dec.prev_ref_poc_lsb = 7;
    dec.prev_frame_num_offset_type1 = 16;
    dec.prev_frame_num_offset_type2 = 24;

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 11;
    dec.last_poc = 21;
    dec.last_dec_ref_pic_marking = DecRefPicMarking {
        is_idr: false,
        no_output_of_prior_pics: false,
        long_term_reference_flag: false,
        adaptive: true,
        ops: vec![MmcoOp::ClearAll],
    };

    dec.store_reference_with_marking();

    assert_eq!(dec.last_frame_num, 0, "MMCO5 应重置当前 frame_num 为 0");
    assert_eq!(dec.last_poc, 0, "MMCO5 应重置当前 POC 为 0");
    assert_eq!(dec.prev_ref_poc_msb, 0, "MMCO5 应重置 prev_ref_poc_msb");
    assert_eq!(dec.prev_ref_poc_lsb, 0, "MMCO5 应重置 prev_ref_poc_lsb");
    assert_eq!(
        dec.prev_frame_num_offset_type1, 0,
        "MMCO5 应重置 POC type1 的 frame_num_offset"
    );
    assert_eq!(
        dec.prev_frame_num_offset_type2, 0,
        "MMCO5 应重置 POC type2 的 frame_num_offset"
    );
    assert_eq!(
        dec.max_long_term_frame_idx, None,
        "MMCO5 应清空长期参考索引上限"
    );
    assert_eq!(dec.reference_frames.len(), 1, "MMCO5 后仅应保留当前参考帧");
    let current = dec.reference_frames.back().expect("应存在当前参考帧");
    assert_eq!(current.frame_num, 0, "MMCO5 后当前参考帧 frame_num 应为 0");
    assert_eq!(current.poc, 0, "MMCO5 后当前参考帧 POC 应为 0");
    assert_eq!(
        current.long_term_frame_idx, None,
        "MMCO5 后当前参考帧默认应为短期参考"
    );
}

#[test]
fn test_store_reference_with_marking_sliding_window_evicts_lowest_frame_num_wrap() {
    let mut dec = build_test_decoder();
    dec.max_reference_frames = 2;
    push_dummy_reference(&mut dec, 10);
    push_dummy_reference(&mut dec, 2);

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 3;
    dec.last_poc = 3;
    dec.last_dec_ref_pic_marking = DecRefPicMarking::default();

    dec.store_reference_with_marking();

    assert_eq!(dec.reference_frames.len(), 2, "滑动窗口后参考帧容量应为 2");
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 2),
        "滑动窗口应保留 frame_num_wrap 更大的短期参考帧"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 3),
        "滑动窗口应保留当前入队的短期参考帧"
    );
    assert!(
        dec.reference_frames.iter().all(|pic| pic.frame_num != 10),
        "滑动窗口应淘汰最小 frame_num_wrap 的短期参考帧"
    );
}

#[test]
fn test_store_reference_with_marking_sliding_window_prefers_keep_long_term() {
    let mut dec = build_test_decoder();
    dec.max_reference_frames = 2;
    push_dummy_reference_with_long_term(&mut dec, 6, Some(0));
    push_dummy_reference(&mut dec, 7);

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 8;
    dec.last_poc = 8;
    dec.last_dec_ref_pic_marking = DecRefPicMarking::default();

    dec.store_reference_with_marking();

    assert_eq!(dec.reference_frames.len(), 2, "滑动窗口后参考帧容量应为 2");
    assert!(
        dec.reference_frames
            .iter()
            .any(|pic| pic.long_term_frame_idx == Some(0)),
        "滑动窗口应优先淘汰短期参考帧, 长期参考应保留"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 8),
        "滑动窗口后当前短期参考帧应成功入队"
    );
    assert!(
        dec.reference_frames.iter().all(|pic| pic.frame_num != 7),
        "滑动窗口应先移除短期参考帧"
    );
}

#[test]
fn test_store_reference_with_marking_sliding_window_wrap_around() {
    let mut dec = build_test_decoder();
    dec.max_reference_frames = 2;
    push_dummy_reference(&mut dec, 15);
    push_dummy_reference(&mut dec, 0);

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 1;
    dec.last_poc = 1;
    dec.last_dec_ref_pic_marking = DecRefPicMarking::default();

    dec.store_reference_with_marking();

    assert_eq!(
        dec.reference_frames.len(),
        2,
        "回绕场景下滑动窗口后容量应为 2"
    );
    assert!(
        dec.reference_frames.iter().all(|pic| pic.frame_num != 15),
        "回绕时 frame_num=15 的 frame_num_wrap 更小, 应被优先淘汰"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 0),
        "回绕场景应保留较新的短期参考帧"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 1),
        "回绕场景应保留当前短期参考帧"
    );
}

#[test]
fn test_fill_frame_num_gaps_if_needed_inserts_non_existing_references() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    dec.max_reference_frames = 4;
    dec.last_frame_num = 1;
    dec.last_poc = 20;
    push_dummy_reference(&mut dec, 1);

    let mut sps = build_test_sps(0);
    sps.gaps_in_frame_num_value_allowed_flag = true;
    dec.sps = Some(sps.clone());
    dec.sps_map.insert(0, sps);

    let header = build_test_slice_header(4, 1, false, None);
    let prev_for_poc = dec.fill_frame_num_gaps_if_needed(&header, 1);

    assert_eq!(
        prev_for_poc, 3,
        "填补间隙后 prev_frame_num 应推进到 current-1"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 2),
        "应插入 frame_num=2 的不存在参考帧"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 3),
        "应插入 frame_num=3 的不存在参考帧"
    );
    let inserted = dec
        .reference_frames
        .iter()
        .find(|pic| pic.frame_num == 2)
        .expect("应能找到插入的 frame_num=2 参考帧");
    assert_eq!(
        inserted.y[0], 128,
        "不存在参考帧应使用中性像素填充以避免预测路径异常"
    );
    assert_eq!(
        inserted.long_term_frame_idx, None,
        "不存在参考帧应按短期参考帧管理"
    );
}

#[test]
fn test_fill_frame_num_gaps_if_needed_skips_when_flag_disabled() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    dec.max_reference_frames = 4;
    dec.last_frame_num = 1;
    dec.last_poc = 20;
    push_dummy_reference(&mut dec, 1);

    let header = build_test_slice_header(4, 1, false, None);
    let prev_for_poc = dec.fill_frame_num_gaps_if_needed(&header, 1);

    assert_eq!(
        prev_for_poc, 1,
        "未开启 gaps_in_frame_num_value_allowed_flag 时不应修改 prev_frame_num"
    );
    assert_eq!(
        dec.reference_frames.len(),
        1,
        "未开启 gaps 标志时不应插入不存在参考帧"
    );
}

#[test]
fn test_mmco5_sliding_window_and_frame_num_gaps_combined() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    dec.max_reference_frames = 2;
    push_dummy_reference(&mut dec, 2);
    push_dummy_reference(&mut dec, 6);

    let mut sps = build_test_sps(0);
    sps.gaps_in_frame_num_value_allowed_flag = true;
    dec.sps = Some(sps.clone());
    dec.sps_map.insert(0, sps);

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 11;
    dec.last_poc = 22;
    dec.last_dec_ref_pic_marking = DecRefPicMarking {
        is_idr: false,
        no_output_of_prior_pics: false,
        long_term_reference_flag: false,
        adaptive: true,
        ops: vec![MmcoOp::ClearAll],
    };

    dec.store_reference_with_marking();
    assert_eq!(dec.reference_frames.len(), 1, "MMCO5 后仅应保留当前参考帧");
    assert_eq!(
        dec.reference_frames[0].frame_num, 0,
        "MMCO5 后保留帧的 frame_num 应重置为 0"
    );

    let gap_header = build_test_slice_header(2, 1, false, None);
    let prev_for_poc = dec.fill_frame_num_gaps_if_needed(&gap_header, dec.last_frame_num);
    assert_eq!(
        prev_for_poc, 1,
        "gaps 填补后应将 prev_frame_num 推进到 current-1"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 1),
        "gaps 填补应插入 frame_num=1 的不存在参考帧"
    );

    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 1;
    dec.last_frame_num = 2;
    dec.last_poc = 2;
    dec.last_dec_ref_pic_marking = DecRefPicMarking::default();
    dec.store_reference_with_marking();

    assert_eq!(
        dec.reference_frames.len(),
        2,
        "滑动窗口后参考帧容量应保持为 2"
    );
    assert!(
        dec.reference_frames.iter().all(|pic| pic.frame_num != 0),
        "当前 frame_num=2 入队时应淘汰 frame_num_wrap 最小的旧帧(frame_num=0)"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 1),
        "应保留 gaps 插入的 frame_num=1 参考帧"
    );
    assert!(
        dec.reference_frames.iter().any(|pic| pic.frame_num == 2),
        "应保留当前 frame_num=2 参考帧"
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
fn test_reference_list_l0_empty_records_missing_reference_fallback() {
    let mut dec = build_test_decoder();
    let before = dec.missing_reference_fallbacks;

    let l0 = dec.build_reference_list_l0_with_mod(2, &[], 0);

    assert_eq!(l0.len(), 2, "空参考时仍应构造目标长度的 L0 列表");
    assert!(
        l0.iter()
            .all(|refp| refp.y[0] == 128 && refp.u[0] == 128 && refp.v[0] == 128),
        "空参考回退应使用零参考平面"
    );
    assert_eq!(
        dec.missing_reference_fallbacks,
        before + 1,
        "空参考列表应记录一次缺参考回退"
    );
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
fn test_apply_inter_block_l0_out_of_range_ref_idx_uses_zero_reference() {
    let mut dec = build_test_decoder();
    let refs = vec![build_constant_ref_planes(&dec, 12, 34, 56)];
    let before = dec.missing_reference_fallbacks;

    dec.apply_inter_block_l0(&refs, 3, 0, 0, 1, 1, 0, 0, &[], 0, 0);

    assert_eq!(dec.ref_y[0], 128, "ref_idx 越界时亮度应回退零参考");
    assert_eq!(dec.ref_u[0], 128, "ref_idx 越界时 U 应回退零参考");
    assert_eq!(dec.ref_v[0], 128, "ref_idx 越界时 V 应回退零参考");
    assert_eq!(
        dec.missing_reference_fallbacks,
        before + 1,
        "ref_idx 越界应记录一次缺参考回退"
    );
}

#[test]
fn test_apply_inter_block_l0_padding_clamps_to_top_left() {
    let mut dec = build_test_decoder();
    dec.ref_y.fill(0);
    dec.ref_u.fill(0);
    dec.ref_v.fill(0);

    let mut y = vec![0u8; dec.ref_y.len()];
    for row in 0..dec.height as usize {
        for col in 0..dec.width as usize {
            y[row * dec.stride_y + col] = ((row * dec.width as usize + col) % 200 + 1) as u8;
        }
    }
    let mut u = vec![0u8; dec.ref_u.len()];
    let mut v = vec![0u8; dec.ref_v.len()];
    for row in 0..(dec.height as usize / 2) {
        for col in 0..(dec.width as usize / 2) {
            let idx = row * dec.stride_c + col;
            u[idx] = ((idx % 200) + 2) as u8;
            v[idx] = ((idx % 200) + 3) as u8;
        }
    }
    let refs = vec![RefPlanes {
        y,
        u,
        v,
        poc: 0,
        is_long_term: false,
    }];

    dec.apply_inter_block_l0(&refs, 0, 0, 0, 4, 4, -400, -400, &[], 0, 0);

    for row in 0..4 {
        for col in 0..4 {
            let idx = row * dec.stride_y + col;
            assert_eq!(dec.ref_y[idx], 1, "左上越界时亮度应复制左上边界像素");
        }
    }
    for row in 0..2 {
        for col in 0..2 {
            let idx = row * dec.stride_c + col;
            assert_eq!(dec.ref_u[idx], 2, "左上越界时 U 应复制左上边界像素");
            assert_eq!(dec.ref_v[idx], 3, "左上越界时 V 应复制左上边界像素");
        }
    }
}

#[test]
fn test_apply_inter_block_l0_padding_clamps_to_bottom_right() {
    let mut dec = build_test_decoder();
    dec.ref_y.fill(0);
    dec.ref_u.fill(0);
    dec.ref_v.fill(0);

    let mut y = vec![0u8; dec.ref_y.len()];
    for row in 0..dec.height as usize {
        for col in 0..dec.width as usize {
            y[row * dec.stride_y + col] = ((row * dec.width as usize + col) % 200 + 1) as u8;
        }
    }
    let mut u = vec![0u8; dec.ref_u.len()];
    let mut v = vec![0u8; dec.ref_v.len()];
    for row in 0..(dec.height as usize / 2) {
        for col in 0..(dec.width as usize / 2) {
            let idx = row * dec.stride_c + col;
            u[idx] = ((idx % 200) + 2) as u8;
            v[idx] = ((idx % 200) + 3) as u8;
        }
    }
    let refs = vec![RefPlanes {
        y,
        u,
        v,
        poc: 0,
        is_long_term: false,
    }];

    dec.apply_inter_block_l0(&refs, 0, 0, 0, 4, 4, 400, 400, &[], 0, 0);

    for row in 0..4 {
        for col in 0..4 {
            let idx = row * dec.stride_y + col;
            assert_eq!(dec.ref_y[idx], 56, "右下越界时亮度应复制右下边界像素");
        }
    }
    for row in 0..2 {
        for col in 0..2 {
            let idx = row * dec.stride_c + col;
            assert_eq!(dec.ref_u[idx], 65, "右下越界时 U 应复制右下边界像素");
            assert_eq!(dec.ref_v[idx], 66, "右下越界时 V 应复制右下边界像素");
        }
    }
}

#[test]
fn test_apply_b_prediction_block_explicit_bi_weighted() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 1;
    dec.pps = Some(pps);

    let ref_l0 = vec![build_constant_ref_planes(&dec, 10, 20, 30)];
    let ref_l1 = vec![build_constant_ref_planes(&dec, 90, 100, 110)];
    let l0_weights = vec![PredWeightL0 {
        luma_weight: 0,
        luma_offset: 0,
        chroma_weight: [0, 0],
        chroma_offset: [0, 0],
    }];
    let l1_weights = vec![PredWeightL0 {
        luma_weight: 4,
        luma_offset: 0,
        chroma_weight: [4, 4],
        chroma_offset: [0, 0],
    }];

    dec.apply_b_prediction_block(
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        &l0_weights,
        &l1_weights,
        2,
        2,
        &ref_l0,
        &ref_l1,
        0,
        0,
        4,
        4,
    );

    assert_eq!(dec.ref_y[0], 45, "显式双向加权后亮度应按 list1 权重输出");
    assert_eq!(dec.ref_u[0], 50, "显式双向加权后 U 应按 list1 权重输出");
    assert_eq!(dec.ref_v[0], 55, "显式双向加权后 V 应按 list1 权重输出");
}

#[test]
fn test_apply_b_prediction_block_default_bi_weighted_rounding() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 0;
    dec.pps = Some(pps);

    let ref_l0 = vec![build_constant_ref_planes(&dec, 10, 20, 30)];
    let ref_l1 = vec![build_constant_ref_planes(&dec, 91, 101, 111)];

    dec.apply_b_prediction_block(
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        &[],
        &[],
        0,
        0,
        &ref_l0,
        &ref_l1,
        0,
        0,
        4,
        4,
    );

    assert_eq!(dec.ref_y[0], 51, "默认双向加权亮度应为 (L0+L1+1)>>1");
    assert_eq!(dec.ref_u[0], 61, "默认双向加权 U 应为 (L0+L1+1)>>1");
    assert_eq!(dec.ref_v[0], 71, "默认双向加权 V 应为 (L0+L1+1)>>1");
}

#[test]
fn test_apply_b_prediction_block_default_bi_weighted_rounding_fractional_mv() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 0;
    dec.pps = Some(pps);

    let ref_l0 = vec![build_constant_ref_planes(&dec, 40, 50, 60)];
    let mut ref_l1_plane = build_constant_ref_planes(&dec, 0, 0, 0);
    for row in 0..dec.height as usize {
        for col in 0..dec.width as usize {
            ref_l1_plane.y[row * dec.stride_y + col] = ((row * 7 + col * 3) % 200 + 20) as u8;
        }
    }
    for row in 0..(dec.height as usize / 2) {
        for col in 0..(dec.width as usize / 2) {
            let idx = row * dec.stride_c + col;
            ref_l1_plane.u[idx] = ((idx * 5) % 200 + 10) as u8;
            ref_l1_plane.v[idx] = ((idx * 7) % 200 + 30) as u8;
        }
    }
    let ref_l1 = vec![ref_l1_plane.clone()];

    dec.apply_b_prediction_block(
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        Some(BMotion {
            mv_x: 1,
            mv_y: 3,
            ref_idx: 0,
        }),
        &[],
        &[],
        0,
        0,
        &ref_l0,
        &ref_l1,
        0,
        0,
        4,
        4,
    );

    let l1_y = sample_h264_luma_qpel(
        ref_l1[0].y.as_slice(),
        dec.stride_y,
        dec.stride_y,
        dec.mb_height * 16,
        0,
        0,
        1,
        3,
    );
    let l1_u = sample_h264_chroma_qpel(
        ref_l1[0].u.as_slice(),
        dec.stride_c,
        dec.stride_c,
        dec.mb_height * 8,
        0,
        0,
        1,
        3,
    );
    let l1_v = sample_h264_chroma_qpel(
        ref_l1[0].v.as_slice(),
        dec.stride_c,
        dec.stride_c,
        dec.mb_height * 8,
        0,
        0,
        1,
        3,
    );
    let round_avg = |a: u8, b: u8| -> u8 { ((u16::from(a) + u16::from(b) + 1) >> 1) as u8 };
    assert_eq!(
        dec.ref_y[0],
        round_avg(40, l1_y),
        "默认双向融合亮度应按 qpel 插值结果执行 (L0+L1+1)>>1 舍入"
    );
    assert_eq!(
        dec.ref_u[0],
        round_avg(50, l1_u),
        "默认双向融合 U 应按 qpel 插值结果执行 (L0+L1+1)>>1 舍入"
    );
    assert_eq!(
        dec.ref_v[0],
        round_avg(60, l1_v),
        "默认双向融合 V 应按 qpel 插值结果执行 (L0+L1+1)>>1 舍入"
    );
}

#[test]
fn test_implicit_bi_weights_from_poc_distance() {
    let mut dec = build_test_decoder();
    dec.last_poc = 6;

    let (w0, w1) = dec.implicit_bi_weights(0, 8, false, false);
    assert_eq!(w0, 16, "隐式权重 w0 应按 tb/td 距离推导");
    assert_eq!(w1, 48, "隐式权重 w1 应按 tb/td 距离推导");

    let (lt_w0, lt_w1) = dec.implicit_bi_weights(0, 8, true, false);
    assert_eq!(lt_w0, 32, "长期参考参与隐式加权时应回退默认权重");
    assert_eq!(lt_w1, 32, "长期参考参与隐式加权时应回退默认权重");
}

#[test]
fn test_temporal_direct_dist_scale_factor_matches_h264_formula() {
    let mut dec = build_test_decoder();
    dec.last_poc = 6;

    let dsf = dec
        .temporal_direct_dist_scale_factor(0, 8)
        .expect("td 非 0 时应计算 dist_scale_factor");
    assert_eq!(dsf, 192, "dist_scale_factor 应按 tb/td 公式推导");

    let dsf_none = dec.temporal_direct_dist_scale_factor(5, 5);
    assert!(dsf_none.is_none(), "td=0 时不应返回可用的缩放系数");
}

#[test]
fn test_scale_temporal_direct_mv_component_rounding_and_sign() {
    let dec = build_test_decoder();
    let scaled_pos = dec.scale_temporal_direct_mv_component(16, 192);
    assert_eq!(scaled_pos, 12, "正向 MV 应按 ((dsf*mv+128)>>8) 缩放");

    let scaled_neg = dec.scale_temporal_direct_mv_component(-16, 192);
    assert_eq!(scaled_neg, -12, "负向 MV 应保持符号并按同一公式缩放");
}

#[test]
fn test_scale_temporal_direct_mv_pair_component_splits_l0_l1() {
    let dec = build_test_decoder();
    let (mv_l0_pos, mv_l1_pos) = dec.scale_temporal_direct_mv_pair_component(16, 192);
    assert_eq!(mv_l0_pos, 12, "L0 分量应按 dist_scale_factor 缩放");
    assert_eq!(mv_l1_pos, -4, "L1 分量应按 (mv_l0 - mv_col) 计算");

    let (mv_l0_neg, mv_l1_neg) = dec.scale_temporal_direct_mv_pair_component(-16, 192);
    assert_eq!(mv_l0_neg, -12, "负向 L0 分量应按同一缩放公式计算");
    assert_eq!(mv_l1_neg, 4, "负向输入时 L1 分量应保持符号关系");

    let (mv_l0_identity, mv_l1_identity) = dec.scale_temporal_direct_mv_pair_component(11, 256);
    assert_eq!(mv_l0_identity, 11, "dist_scale_factor=256 时 L0 应保持原值");
    assert_eq!(mv_l1_identity, 0, "dist_scale_factor=256 时 L1 应回落为 0");
}

#[test]
fn test_apply_b_prediction_block_implicit_bi_weighted() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 2;
    dec.pps = Some(pps);
    dec.last_poc = 6;

    let mut l0 = build_constant_ref_planes(&dec, 10, 20, 30);
    l0.poc = 0;
    let mut l1 = build_constant_ref_planes(&dec, 90, 100, 110);
    l1.poc = 8;

    dec.apply_b_prediction_block(
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        &[],
        &[],
        0,
        0,
        &[l0],
        &[l1],
        0,
        0,
        4,
        4,
    );

    assert_eq!(dec.ref_y[0], 70, "隐式双向加权亮度应使用推导出的 w0/w1");
    assert_eq!(dec.ref_u[0], 80, "隐式双向加权 U 应使用推导出的 w0/w1");
    assert_eq!(dec.ref_v[0], 90, "隐式双向加权 V 应使用推导出的 w0/w1");
}

#[test]
fn test_apply_b_prediction_block_implicit_long_term_fallback_default() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 2;
    dec.pps = Some(pps);
    dec.last_poc = 6;

    let mut l0 = build_constant_ref_planes(&dec, 10, 20, 30);
    l0.poc = 0;
    l0.is_long_term = true;
    let mut l1 = build_constant_ref_planes(&dec, 90, 100, 110);
    l1.poc = 8;

    dec.apply_b_prediction_block(
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        &[],
        &[],
        0,
        0,
        &[l0],
        &[l1],
        0,
        0,
        4,
        4,
    );

    assert_eq!(dec.ref_y[0], 50, "长期参考参与隐式加权时亮度应回退默认平均");
    assert_eq!(dec.ref_u[0], 60, "长期参考参与隐式加权时 U 应回退默认平均");
    assert_eq!(dec.ref_v[0], 70, "长期参考参与隐式加权时 V 应回退默认平均");
}

#[test]
fn test_apply_b_prediction_block_explicit_single_l1_weight() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 1;
    dec.pps = Some(pps);

    let ref_l1 = vec![build_constant_ref_planes(&dec, 80, 60, 40)];
    let l1_weights = vec![PredWeightL0 {
        luma_weight: 1,
        luma_offset: 10,
        chroma_weight: [1, 1],
        chroma_offset: [2, -2],
    }];

    dec.apply_b_prediction_block(
        None,
        Some(BMotion {
            mv_x: 0,
            mv_y: 0,
            ref_idx: 0,
        }),
        &[],
        &l1_weights,
        1,
        1,
        &[],
        &ref_l1,
        0,
        0,
        4,
        4,
    );

    assert_eq!(dec.ref_y[0], 50, "显式 L1 加权后亮度应应用权重和偏移");
    assert_eq!(dec.ref_u[0], 32, "显式 L1 加权后 U 应应用权重和偏移");
    assert_eq!(dec.ref_v[0], 18, "显式 L1 加权后 V 应应用权重和偏移");
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
        dec.ref_y[row + 2] = 40;
        dec.ref_y[row + 3] = 40;
        dec.ref_y[row + 4] = 48;
        dec.ref_y[row + 5] = 48;
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
        dec.ref_y[row + 2] = 40;
        dec.ref_y[row + 3] = 40;
        dec.ref_y[row + 4] = 48;
        dec.ref_y[row + 5] = 48;
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
fn test_build_output_frame_conceals_uncovered_macroblock_with_reference_pixels() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 0;
    dec.last_poc = 2;
    dec.reorder_depth = 0;
    dec.last_disable_deblocking_filter_idc = 1;

    dec.ref_y.fill(10);
    dec.ref_u.fill(20);
    dec.ref_v.fill(30);
    dec.mb_types.fill(200);
    dec.mb_slice_first_mb.fill(0);

    dec.mb_types[1] = 0;
    dec.mb_slice_first_mb[1] = u32::MAX;

    push_custom_reference(&mut dec, 1, 1, 77, None);

    dec.build_output_frame(0, Rational::new(1, 25), false);
    let frame = match dec.output_queue.pop_front() {
        Some(Frame::Video(vf)) => vf,
        _ => panic!("应输出视频帧"),
    };

    assert_eq!(frame.data[0][0], 10, "已解码宏块应保持当前帧亮度像素");
    assert_eq!(frame.data[0][16], 77, "缺失宏块应使用参考帧亮度像素填充");
    assert_eq!(frame.data[1][0], 20, "已解码宏块应保持当前帧 U 像素");
    assert_eq!(frame.data[1][8], 128, "缺失宏块应使用参考帧 U 像素填充");
    assert_eq!(frame.data[2][0], 30, "已解码宏块应保持当前帧 V 像素");
    assert_eq!(frame.data[2][8], 128, "缺失宏块应使用参考帧 V 像素填充");
    assert_eq!(dec.mb_types[1], 253, "被隐藏修复的宏块应标记为 conceal 态");
}

#[test]
fn test_build_output_frame_conceals_error_macroblock_without_reference_with_neutral_gray() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 0;
    dec.last_nal_ref_idc = 0;
    dec.last_poc = 0;
    dec.reorder_depth = 0;
    dec.last_disable_deblocking_filter_idc = 1;

    dec.ref_y.fill(5);
    dec.ref_u.fill(6);
    dec.ref_v.fill(7);
    dec.mb_types[0] = 252;
    dec.mb_slice_first_mb[0] = 0;

    dec.build_output_frame(1, Rational::new(1, 25), false);
    let frame = match dec.output_queue.pop_front() {
        Some(Frame::Video(vf)) => vf,
        _ => panic!("应输出视频帧"),
    };

    assert_eq!(frame.data[0][0], 128, "无参考帧时亮度应填充中性灰");
    assert_eq!(frame.data[1][0], 128, "无参考帧时 U 应填充中性灰");
    assert_eq!(frame.data[2][0], 128, "无参考帧时 V 应填充中性灰");
    assert_eq!(
        dec.mb_types[0], 253,
        "错误宏块经隐藏修复后应标记为 conceal 态"
    );
}

#[test]
fn test_push_video_for_output_releases_lowest_poc_when_dpb_full() {
    let mut dec = build_test_decoder();
    dec.max_reference_frames = 1;
    dec.reorder_depth = 8;

    dec.push_video_for_output(build_test_video_frame_with_pts(20), 20);
    assert!(dec.output_queue.is_empty(), "DPB 未满时不应提前输出重排帧");

    dec.push_video_for_output(build_test_video_frame_with_pts(10), 10);
    let out = match dec.output_queue.pop_front() {
        Some(Frame::Video(vf)) => vf,
        _ => panic!("DPB 满时应输出视频帧"),
    };
    assert_eq!(out.pts, 10, "DPB 满时应优先输出 POC 更小的帧");
    assert_eq!(dec.reorder_buffer.len(), 1, "输出后应仅保留一帧待重排");
    assert_eq!(
        dec.reorder_buffer[0].poc, 20,
        "输出后重排缓存中应保留较大的 POC"
    );
}

#[test]
fn test_drain_reorder_buffer_outputs_frames_by_poc_ascending() {
    let mut dec = build_test_decoder();
    dec.max_reference_frames = 16;
    dec.reorder_depth = 16;

    dec.push_video_for_output(build_test_video_frame_with_pts(30), 30);
    dec.push_video_for_output(build_test_video_frame_with_pts(10), 10);
    dec.push_video_for_output(build_test_video_frame_with_pts(20), 20);

    assert!(
        dec.output_queue.is_empty(),
        "flush 前不应提前输出, 以便验证 drain 行为"
    );

    dec.drain_reorder_buffer_to_output();
    let mut pts_list = Vec::new();
    while let Some(frame) = dec.output_queue.pop_front() {
        match frame {
            Frame::Video(vf) => pts_list.push(vf.pts),
            Frame::Audio(_) => panic!("重排缓冲仅应输出视频帧"),
        }
    }
    assert_eq!(pts_list, vec![10, 20, 30], "flush 输出应按 POC 升序");
    assert!(dec.reorder_buffer.is_empty(), "drain 后重排缓冲应被清空");
}

#[test]
fn test_decode_cavlc_slice_data_p_skip_run_reconstructs_from_l0_prediction() {
    let mut dec = build_test_decoder();
    push_custom_reference(&mut dec, 3, 3, 77, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb_skip_run = 1, 覆盖单宏块帧
    // ue(1)=010 + rbsp_trailing_bits=1 => 0101_0000
    let rbsp = [0x50u8];
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 77, "P-slice skip 宏块应按 L0 预测重建");
    assert_eq!(dec.mb_types[0], 255, "P-slice skip 宏块应标记为 skip");
}

#[test]
fn test_decode_cavlc_slice_data_p_skip_run_uses_predicted_mv_from_left_neighbor() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 仅解码第 2 个宏块, 并预置左邻宏块运动向量为 +1 像素(qpel=4),
    // 使 P_Skip 的 MVP 可以观测到非零位移.
    dec.mv_l0_x[0] = 4;
    dec.mv_l0_y[0] = 0;
    dec.ref_idx_l0[0] = 0;
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 4, 0, 0);
    dec.mb_types[0] = 200;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 1;
    header.data_bit_offset = 0;

    let rbsp = build_rbsp_from_ues(&[1]); // mb_skip_run=1
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let mb1_y0_idx = 16usize;
    assert_eq!(
        dec.ref_y[mb1_y0_idx], 17,
        "P_Skip 应使用左邻 MVP=+1 像素, 而非直接复制同坐标参考像素"
    );
    assert_eq!(dec.mv_l0_x[1], 4, "P_Skip 应写入预测后的宏块 MV(x)");
    assert_eq!(dec.mv_l0_y[1], 0, "P_Skip 应写入预测后的宏块 MV(y)");
    assert_eq!(dec.ref_idx_l0[1], 0, "P_Skip 应固定使用 L0 的 ref_idx=0");
}

#[test]
fn test_decode_cavlc_slice_data_p_skip_run_missing_reference_fallback() {
    let mut dec = build_test_decoder();
    let before = dec.missing_reference_fallbacks;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb_skip_run = 1, 触发 skip 宏块路径, 此时 reference_frames 为空
    let rbsp = build_rbsp_from_ues(&[1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 128, "缺参考帧时应回退零参考亮度");
    assert_eq!(dec.ref_u[0], 128, "缺参考帧时应回退零参考 U");
    assert_eq!(dec.ref_v[0], 128, "缺参考帧时应回退零参考 V");
    assert_eq!(dec.mb_types[0], 255, "P-slice skip 宏块应标记为 skip");
    assert!(
        dec.missing_reference_fallbacks > before,
        "缺参考帧时应记录回退计数"
    );
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_missing_reference_fallback() {
    let mut dec = build_test_decoder();
    let before = dec.missing_reference_fallbacks;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;

    // mb_skip_run = 1, 触发 B-skip 路径, L0/L1 均无参考
    let rbsp = build_rbsp_from_ues(&[1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 128, "B-slice 缺参考帧时应回退零参考亮度");
    assert_eq!(dec.ref_u[0], 128, "B-slice 缺参考帧时应回退零参考 U");
    assert_eq!(dec.ref_v[0], 128, "B-slice 缺参考帧时应回退零参考 V");
    assert_eq!(dec.mb_types[0], 254, "B-slice skip 宏块应标记为 B-skip");
    assert!(
        dec.missing_reference_fallbacks >= before + 2,
        "B-slice L0/L1 均缺参考时应至少记录两次回退"
    );
}

#[test]
fn test_decode_slice_bad_header_records_malformed_nal_drop() {
    let mut dec = build_test_decoder();
    let nalu = NalUnit::parse(&[0x65]).expect("NAL 头应可解析");
    let before = dec.malformed_nal_drops;

    dec.decode_slice(&nalu);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "slice header 解析失败时应记录坏 NAL 丢弃计数"
    );
    assert_eq!(dec.last_frame_num, 0, "坏 NAL 不应推进帧号状态");
}

#[test]
fn test_decode_slice_skip_when_redundant_pic_cnt_positive() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    dec.last_frame_num = 7;
    dec.last_slice_type = 2;
    dec.ref_y.fill(9);

    if let Some(pps) = dec.pps.as_mut() {
        pps.redundant_pic_cnt_present = true;
    }
    if let Some(pps) = dec.pps_map.get_mut(&0) {
        pps.redundant_pic_cnt_present = true;
    }

    let rbsp = build_p_slice_header_rbsp_with_redundant_pic_cnt(0, 1, 2, 2);
    let mut nalu_data = vec![0x01];
    nalu_data.extend_from_slice(&rbsp);
    let nalu = NalUnit::parse(&nalu_data).expect("测试构造 slice NAL 失败");
    let before_drop = dec.malformed_nal_drops;

    dec.decode_slice(&nalu);

    assert_eq!(
        dec.malformed_nal_drops, before_drop,
        "冗余 slice 跳过不应记录坏 NAL 丢弃计数"
    );
    assert_eq!(dec.last_frame_num, 7, "冗余 slice 跳过不应推进帧号状态");
    assert_eq!(
        dec.last_slice_type, 2,
        "冗余 slice 跳过不应覆盖上一帧 slice 类型"
    );
    assert_eq!(dec.ref_y[0], 9, "冗余 slice 跳过不应改写像素缓冲");
}

#[test]
fn test_decode_slice_not_skip_when_redundant_pic_cnt_zero() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    dec.last_frame_num = 7;

    if let Some(pps) = dec.pps.as_mut() {
        pps.redundant_pic_cnt_present = true;
    }
    if let Some(pps) = dec.pps_map.get_mut(&0) {
        pps.redundant_pic_cnt_present = true;
    }

    let rbsp = build_p_slice_header_rbsp_with_redundant_pic_cnt(0, 1, 2, 0);
    let mut nalu_data = vec![0x01];
    nalu_data.extend_from_slice(&rbsp);
    let nalu = NalUnit::parse(&nalu_data).expect("测试构造 slice NAL 失败");

    dec.decode_slice(&nalu);

    assert_eq!(
        dec.last_frame_num, 1,
        "redundant_pic_cnt=0 时应按正常 slice 路径推进帧号状态"
    );
}

#[test]
fn test_send_packet_without_valid_nal_records_malformed_nal_drop() {
    let mut dec = build_test_decoder();
    let before = dec.malformed_nal_drops;
    let pkt = Packet::from_data(vec![0x80]);

    <H264Decoder as Decoder>::send_packet(&mut dec, &pkt).expect("坏包应被容错丢弃而非报错");

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "无法拆出有效 NAL 时应记录坏 NAL 丢弃计数"
    );
}

#[test]
fn test_decode_slice_data_records_drop_when_activate_parameter_sets_failed() {
    let mut dec = build_test_decoder();
    let before = dec.malformed_nal_drops;
    let header = build_test_slice_header(0, 1, false, None);

    dec.decode_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "参数集激活失败时应记录坏 NAL 丢弃计数"
    );
}

#[test]
fn test_decode_slice_data_records_drop_when_cabac_start_out_of_range() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.pps_id = 0;
    header.cabac_start_byte = 1; // rbsp.len() == 1, 越界

    dec.decode_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "CABAC 起始越界时应记录坏 NAL 丢弃计数"
    );
}

#[test]
fn test_decode_slice_data_records_drop_when_cavlc_bit_offset_out_of_range() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 0);
    let before = dec.malformed_nal_drops;
    dec.ref_y.fill(0);
    dec.ref_u.fill(0);
    dec.ref_v.fill(0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.pps_id = 0;
    header.slice_type = 2; // I slice, 触发 CAVLC fallback
    header.data_bit_offset = 99_999;

    dec.decode_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "CAVLC bit_offset 越界时应记录坏 NAL 丢弃计数"
    );
    assert_eq!(dec.ref_y[0], 128, "CAVLC 越界时应触发 DC fallback 亮度");
}

#[test]
fn test_decode_slice_data_records_drop_when_first_mb_out_of_range_cabac() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 1);
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.pps_id = 0;
    header.first_mb = (dec.mb_width * dec.mb_height) as u32; // 越界
    header.cabac_start_byte = 0;

    dec.decode_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "CABAC first_mb 越界时应记录坏 NAL 丢弃计数"
    );
}

#[test]
fn test_decode_cavlc_slice_data_records_drop_when_first_mb_out_of_range() {
    let mut dec = build_test_decoder();
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 2;
    header.first_mb = (dec.mb_width * dec.mb_height) as u32; // 越界
    header.data_bit_offset = 0;

    dec.decode_cavlc_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "CAVLC first_mb 越界时应记录坏 NAL 丢弃计数"
    );
}

#[test]
fn test_decode_cavlc_slice_data_marks_mb_error_and_skips_following_when_skip_run_truncated() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.first_mb = 0;

    // 全 0 位流会导致第一个 skip_run ue 解码失败.
    dec.decode_cavlc_slice_data(&[0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "宏块 skip_run 解码失败应计入坏 NAL 丢弃统计"
    );
    assert_eq!(dec.mb_types[0], 252, "出错宏块应标记为错误态 mb_type=252");
    assert_eq!(dec.mb_types[1], 0, "异常后应停止本 slice 后续宏块解码");
    assert_eq!(
        dec.mb_slice_first_mb[0], 0,
        "出错宏块应记录所属 slice first_mb"
    );
    assert_eq!(
        dec.mb_slice_first_mb[1],
        u32::MAX,
        "跳过的后续宏块不应被写入 slice first_mb 标记"
    );
}

#[test]
fn test_decode_cavlc_slice_data_marks_mb_error_and_skips_following_when_mb_type_truncated() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.first_mb = 0;

    // skip_run=0 后剩余全 0, 会在 mb_type ue 解码阶段失败.
    dec.decode_cavlc_slice_data(&[0x80, 0x00], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "宏块 mb_type 解码失败应计入坏 NAL 丢弃统计"
    );
    assert_eq!(dec.mb_types[0], 252, "出错宏块应标记为错误态 mb_type=252");
    assert_eq!(dec.mb_types[1], 0, "异常后应停止本 slice 后续宏块解码");
    assert_eq!(
        dec.mb_slice_first_mb[0], 0,
        "出错宏块应记录所属 slice first_mb"
    );
    assert_eq!(
        dec.mb_slice_first_mb[1],
        u32::MAX,
        "跳过的后续宏块不应被写入 slice first_mb 标记"
    );
}

#[test]
fn test_decode_cavlc_slice_data_stops_at_rbsp_trailing_bits_for_partial_slice() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    push_custom_reference(&mut dec, 3, 3, 99, None);
    dec.ref_y.fill(7);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.first_mb = 0;

    // ue(1)=010 + rbsp_trailing_bits=1 => 0101_0000
    let rbsp = [0x50u8];
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let mb0_luma = dec.ref_y[0];
    let mb1_luma = dec.ref_y[16];
    assert_eq!(mb0_luma, 99, "partial slice 应解码 first_mb 对应的首个宏块");
    assert_eq!(
        mb1_luma, 7,
        "到达 rbsp_trailing_bits 后应停止, 不应误解码后续宏块"
    );
    assert_eq!(
        dec.mb_slice_first_mb[0], 0,
        "mb0 的 first_mb 标记应写入当前 slice"
    );
    assert_eq!(
        dec.mb_slice_first_mb[1],
        u32::MAX,
        "未被当前 slice 覆盖的宏块不应写入 first_mb 标记"
    );
}

#[test]
fn test_decode_cavlc_slice_data_merges_multi_slice_by_first_mb_offset() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    push_custom_reference(&mut dec, 4, 4, 66, None);
    dec.ref_y.fill(7);

    let mut header0 = build_test_slice_header(0, 1, false, None);
    header0.slice_type = 0; // P slice
    header0.data_bit_offset = 0;
    header0.first_mb = 0;
    // ue(1)=010 + rbsp_trailing_bits=1 => 0101_0000
    dec.decode_cavlc_slice_data(&[0x50], &header0);

    let mut header1 = build_test_slice_header(0, 1, false, None);
    header1.slice_type = 0; // P slice
    header1.data_bit_offset = 0;
    header1.first_mb = 1;
    dec.decode_cavlc_slice_data(&[0x50], &header1);

    assert_eq!(dec.ref_y[0], 66, "第一个 slice 应覆盖 mb0");
    assert_eq!(dec.ref_y[16], 66, "第二个 slice 应按 first_mb=1 覆盖 mb1");
    assert_eq!(dec.mb_slice_first_mb[0], 0, "mb0 应标记为 first_mb=0");
    assert_eq!(dec.mb_slice_first_mb[1], 1, "mb1 应标记为 first_mb=1");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_intra_mb_type() {
    let mut dec = build_test_decoder();
    push_custom_reference(&mut dec, 3, 3, 77, None);
    dec.ref_y.fill(0);
    dec.ref_u.fill(0);
    dec.ref_v.fill(0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb_skip_run = 0, mb_type = 5(I_4x4 in P-slice domain)
    let rbsp = build_rbsp_from_ues(&[0, 5]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(
        dec.mb_types[0], 1,
        "P-slice 非 skip 且 I 宏块应进入帧内路径"
    );
    assert_eq!(dec.ref_y[0], 128, "帧内预测应生成默认 DC 值");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_ref_idx_l0() {
    let mut dec = build_test_decoder();
    push_custom_reference(&mut dec, 3, 3, 33, None);
    push_custom_reference(&mut dec, 2, 2, 99, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb_skip_run=0, mb_type=0(P_L0_16x16), ref_idx_l0=1
    let rbsp = build_rbsp_from_ues(&[0, 0, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(
        dec.ref_y[0], 99,
        "P-slice 非 skip 互预测应按 ref_idx_l0 选择参考帧"
    );
    assert_eq!(dec.mb_types[0], 200, "P_L0_16x16 应标记为互预测宏块");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_ref_idx_l0_mvd_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb0: skip_run=0, mb_type=0(P_L0_16x16), ref_idx_l0=1, mvd=(0,0)
    // mb1: skip_run=0, mb_type=5(I 宏块), 用于验证 mvd 语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(0), Ue(1), Se(0), Se(0), Ue(0), Ue(5)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 90, "P_L0_16x16 应按 ref_idx_l0 选择参考帧");
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_mvd_affects_prediction() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb0: skip_run=0, mb_type=0(P_L0_16x16), mvd_l0=(4,0), 对应亮度右移 1 像素.
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(0), Se(4), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 1, "mvd_l0=(4,0) 应使首像素向右采样 1 像素");
    assert_eq!(dec.mb_types[0], 200, "P_L0_16x16 应标记为互预测宏块");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_16x8_partition_ref_idx() {
    let mut dec = build_test_decoder();
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb_skip_run=0, mb_type=1(P_L0_L0_16x8), top ref_idx=0, bottom ref_idx=1
    let rbsp = build_rbsp_from_ues(&[0, 1, 0, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 20, "16x8 顶部分区应使用 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        90,
        "16x8 底部分区应使用 ref_idx=1"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_8x16_partition_ref_idx() {
    let mut dec = build_test_decoder();
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb_skip_run=0, mb_type=2(P_L0_L0_8x16), left ref_idx=0, right ref_idx=1
    let rbsp = build_rbsp_from_ues(&[0, 2, 0, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 20, "8x16 左分区应使用 ref_idx=0");
    assert_eq!(dec.ref_y[8], 90, "8x16 右分区应使用 ref_idx=1");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_16x8_directional_mvp_uses_top_partition_mv() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=1(P_L0_L0_16x8)
    // top: mvd=(4,0) => mv=+1 像素
    // bottom: mvd=(0,0) 且同 ref_idx, 应复用 top 分区预测 mv.
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(1), Se(4), Se(0), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 1, "16x8 顶部分区应按 +1 像素位移采样");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        1,
        "16x8 底部分区应复用顶部分区预测 MV"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_8x16_directional_mvp_uses_left_partition_mv() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=2(P_L0_L0_8x16)
    // left: mvd=(4,0) => mv=+1 像素
    // right: mvd=(0,0) 且同 ref_idx, 应复用 left 分区预测 mv.
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(2), Se(4), Se(0), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 1, "8x16 左分区应按 +1 像素位移采样");
    assert_eq!(dec.ref_y[8], 9, "8x16 右分区应复用左分区预测 MV");
}

#[test]
fn test_predict_mv_l0_partition_prefers_single_ref_match_and_fallback_to_d() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    // 目标宏块是 (1,1): A=左(0,1), B=上(1,0), C 不可用时回退 D=左上(0,0).
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 1);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 12, 0, 0);

    let mv_ref1 = dec.predict_mv_l0_partition(1, 1, 0, 0, 4, 1);
    assert_eq!(mv_ref1, (4, 0), "仅左邻 ref_idx 匹配时应直接选用左邻 MV");

    let mv_no_match = dec.predict_mv_l0_partition(1, 1, 0, 0, 4, 2);
    assert_eq!(mv_no_match, (8, 0), "无匹配参考时应回落到 A/B/C 的中值预测");
}

#[test]
fn test_predict_p_skip_mv_returns_zero_when_left_top_mb_motion_are_zero() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    // 目标宏块(1,1)的左/上邻居 mb 级运动信息都为零且 ref_idx=0.
    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    dec.mv_l0_x[left_mb] = 0;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.mv_l0_x[top_mb] = 0;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;

    // 即便 4x4 状态存在非零候选, P_Skip 也应优先走零向量分支.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 12, 0, 0);

    let mv = dec.predict_p_skip_mv(1, 1);
    assert_eq!(mv, (0, 0), "左/上邻居均为零向量时 P_Skip 应输出零 MV");
}

#[test]
fn test_predict_p_skip_mv_uses_partition_predict_when_neighbors_not_both_zero() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    // 构造目标宏块(1,1)的 A/B/C 候选: A=8, B=8, D=0 => 中值应为 8.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 0, 0, 0);

    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    dec.mv_l0_x[left_mb] = 8;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.mv_l0_x[top_mb] = 8;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;

    let mv = dec.predict_p_skip_mv(1, 1);
    assert_eq!(mv, (8, 0), "非零邻居场景应退化到分区中值预测");
}

#[test]
fn test_build_b_direct_motion_spatial_returns_zero_when_left_top_neighbors_are_zero() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    for idx in [left_mb, top_mb] {
        dec.mv_l0_x[idx] = 0;
        dec.mv_l0_y[idx] = 0;
        dec.ref_idx_l0[idx] = 0;
        dec.mv_l1_x[idx] = 0;
        dec.mv_l1_y[idx] = 0;
        dec.ref_idx_l1[idx] = 0;
    }

    let (motion_l0, motion_l1) = dec.build_b_direct_motion(1, 1, 12, -8, true, &[], &[]);
    let motion_l0 = motion_l0.expect("spatial direct 应提供 L0 运动信息");
    let motion_l1 = motion_l1.expect("spatial direct 应提供 L1 运动信息");
    assert_eq!(motion_l0.mv_x, 0, "零向量条件满足时 L0 MV(x) 应归零");
    assert_eq!(motion_l0.mv_y, 0, "零向量条件满足时 L0 MV(y) 应归零");
    assert_eq!(motion_l1.mv_x, 0, "零向量条件满足时 L1 MV(x) 应归零");
    assert_eq!(motion_l1.mv_y, 0, "零向量条件满足时 L1 MV(y) 应归零");
}

#[test]
fn test_build_b_direct_motion_spatial_zero_condition_uses_top_and_diagonal_neighbors() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    // 目标宏块为 (0,1), 无左邻, 依赖上邻与 C 对角邻居.
    let top_mb = dec.mb_index(0, 0).expect("上邻索引应存在");
    let diag_mb = dec.mb_index(1, 0).expect("对角邻索引应存在");
    for idx in [top_mb, diag_mb] {
        dec.mv_l0_x[idx] = 0;
        dec.mv_l0_y[idx] = 0;
        dec.ref_idx_l0[idx] = 0;
        dec.mv_l1_x[idx] = 0;
        dec.mv_l1_y[idx] = 0;
        dec.ref_idx_l1[idx] = 0;
    }

    let (motion_l0, motion_l1) = dec.build_b_direct_motion(0, 1, 12, -8, true, &[], &[]);
    let motion_l0 = motion_l0.expect("spatial direct 应提供 L0 运动信息");
    let motion_l1 = motion_l1.expect("spatial direct 应提供 L1 运动信息");
    assert_eq!(motion_l0.mv_x, 0, "上邻与对角邻均零向量时 L0 MV(x) 应归零");
    assert_eq!(motion_l0.mv_y, 0, "上邻与对角邻均零向量时 L0 MV(y) 应归零");
    assert_eq!(motion_l1.mv_x, 0, "上邻与对角邻均零向量时 L1 MV(x) 应归零");
    assert_eq!(motion_l1.mv_y, 0, "上邻与对角邻均零向量时 L1 MV(y) 应归零");
}

#[test]
fn test_build_b_direct_motion_spatial_l1_fallback_keeps_input_when_neighbors_absent() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    dec.mv_l0_x[left_mb] = 0;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.ref_idx_l1[left_mb] = -1;
    dec.mv_l0_x[top_mb] = 0;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;
    dec.ref_idx_l1[top_mb] = -1;

    let (motion_l0, motion_l1) = dec.build_b_direct_motion(1, 1, 12, -8, true, &[], &[]);
    let motion_l0 = motion_l0.expect("spatial direct 应提供 L0 运动信息");
    let motion_l1 = motion_l1.expect("spatial direct 应提供 L1 运动信息");
    assert_eq!(
        motion_l0.mv_x, 0,
        "L0 邻居存在时应优先使用 L0 邻居预测 MV(x)"
    );
    assert_eq!(
        motion_l0.mv_y, 0,
        "L0 邻居存在时应优先使用 L0 邻居预测 MV(y)"
    );
    assert_eq!(motion_l1.mv_x, 12, "L1 邻居缺失时应回退输入预测 MV(x)");
    assert_eq!(motion_l1.mv_y, -8, "L1 邻居缺失时应回退输入预测 MV(y)");
}

#[test]
fn test_build_b_direct_motion_spatial_uses_independent_l1_neighbor_mv() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");

    dec.mv_l0_x[left_mb] = 0;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.mv_l0_x[top_mb] = 0;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;

    dec.mv_l1_x[left_mb] = 20;
    dec.mv_l1_y[left_mb] = 0;
    dec.ref_idx_l1[left_mb] = 0;
    dec.mv_l1_x[top_mb] = 20;
    dec.mv_l1_y[top_mb] = 0;
    dec.ref_idx_l1[top_mb] = 0;

    let (motion_l0, motion_l1) = dec.build_b_direct_motion(1, 1, 12, -8, true, &[], &[]);
    let motion_l0 = motion_l0.expect("spatial direct 应提供 L0 运动信息");
    let motion_l1 = motion_l1.expect("spatial direct 应提供 L1 运动信息");
    assert_eq!(motion_l0.mv_x, 0, "L0 应独立使用 list0 邻居预测 MV(x)");
    assert_eq!(motion_l0.mv_y, 0, "L0 应独立使用 list0 邻居预测 MV(y)");
    assert_eq!(motion_l1.mv_x, 20, "L1 应独立使用邻居预测 MV(x)");
    assert_eq!(motion_l1.mv_y, 0, "L1 应独立使用邻居预测 MV(y)");
}

#[test]
fn test_build_b_direct_motion_spatial_uses_independent_l0_neighbor_mv() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();

    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");

    dec.mv_l0_x[left_mb] = 20;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.mv_l0_x[top_mb] = 20;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;
    dec.ref_idx_l1[left_mb] = -1;
    dec.ref_idx_l1[top_mb] = -1;

    let (motion_l0, motion_l1) = dec.build_b_direct_motion(1, 1, 12, -8, true, &[], &[]);
    let motion_l0 = motion_l0.expect("spatial direct 应提供 L0 运动信息");
    let motion_l1 = motion_l1.expect("spatial direct 应提供 L1 运动信息");
    assert_eq!(motion_l0.mv_x, 20, "L0 应独立使用 list0 邻居预测 MV(x)");
    assert_eq!(motion_l0.mv_y, 0, "L0 应独立使用 list0 邻居预测 MV(y)");
    assert_eq!(motion_l1.mv_x, 12, "L1 邻居缺失时应回退输入预测 MV(x)");
    assert_eq!(motion_l1.mv_y, -8, "L1 邻居缺失时应回退输入预测 MV(y)");
}

#[test]
fn test_build_b_direct_motion_temporal_prefers_list1_colocated_mb() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference_with_l0_motion(&mut dec, 1, 2, 20, None, (12, -8, 0));
    push_custom_reference_with_l0_motion(&mut dec, 2, 8, 100, None, (24, 4, 0));

    let ref_l0_list = dec.build_reference_list_l0_with_mod(1, &[], 0);
    let ref_l1_list = dec.build_reference_list_l1_with_mod(1, &[], 0);

    let (motion_l0, motion_l1) =
        dec.build_b_direct_motion(0, 0, 7, -3, false, &ref_l0_list, &ref_l1_list);
    let motion_l0 = motion_l0.expect("temporal direct 应提供 L0 运动信息");
    assert_eq!(
        motion_l0.mv_x, 24,
        "temporal direct 应优先使用 list1[0] 共定位宏块的 list0 MV(x)"
    );
    assert_eq!(
        motion_l0.mv_y, 4,
        "temporal direct 应优先使用 list1[0] 共定位宏块的 list0 MV(y)"
    );
    assert!(
        motion_l1.is_none(),
        "最小 temporal direct 路径仍应保持 L1 为空"
    );
}

#[test]
fn test_build_b_direct_motion_temporal_fallbacks_to_l0_colocated_mb() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference_with_l0_motion(&mut dec, 1, 2, 20, None, (14, -6, 0));

    let ref_l0_list = dec.build_reference_list_l0_with_mod(1, &[], 0);
    let ref_l1_list = vec![dec.zero_reference_planes()];

    let (motion_l0, _) = dec.build_b_direct_motion(0, 0, 3, 1, false, &ref_l0_list, &ref_l1_list);
    let motion_l0 = motion_l0.expect("temporal direct 应提供 L0 运动信息");
    assert_eq!(
        motion_l0.mv_x, 14,
        "list1 共定位不可定位时应回退到 list0[0] 共定位宏块 MV(x)"
    );
    assert_eq!(
        motion_l0.mv_y, -6,
        "list1 共定位不可定位时应回退到 list0[0] 共定位宏块 MV(y)"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_16x16_prefers_single_ref_match_mvp() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 预置目标宏块(1,1)的邻居运动信息:
    // A(ref=1,mv=+1px), B(ref=0,mv=+2px), D(ref=0,mv=+3px).
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 1);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 12, 0, 0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb_skip_run=0, mb_type=0(P_L0_16x16), ref_idx_l0=1, mvd=(0,0)
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(0), Ue(1), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(
        dec.ref_y[base], 17,
        "P_L0_16x16 应优先使用与 ref_idx 匹配的左邻候选 MV"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_ref_idx_and_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb0: skip_run=0, mb_type=3(P_8x8), 四个 sub_mb_type=0, ref_idx=[0,1,1,0]
    // 并为每个子分区提供 mvd=(0,0), 验证语法消费顺序。
    // mb1: skip_run=0, mb_type=5(I 宏块), 用于验证前一宏块语法消费对齐正确。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(0),
        Ue(0),
        Ue(0),
        Ue(0),
        Ue(0),
        Ue(1),
        Ue(1),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(5),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.mb_types[0], 203, "P_8x8 宏块应标记为互预测类型");
    assert_eq!(dec.ref_y[0], 20, "左上 8x8 应使用 ref_idx=0");
    assert_eq!(dec.ref_y[8], 90, "右上 8x8 应使用 ref_idx=1");
    assert_eq!(dec.ref_y[8 * dec.stride_y], 90, "左下 8x8 应使用 ref_idx=1");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        20,
        "右下 8x8 应使用 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应成功解析为 I 宏块");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_uses_mvp_from_left_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;

    // mb0: skip_run=0, mb_type=3(P_8x8), sub_mb_type=[2,0,0,0].
    // 对 sub0(4x8): 左半 mvd=(4,0), 右半 mvd=(0,0), 右半应继承左半 MVP.
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(2),
        Ue(0),
        Ue(0),
        Ue(0),
        Se(4),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(
        dec.ref_y[0], 1,
        "4x8 左半分区应按 mvd=(4,0) 向右偏移 1 像素"
    );
    assert_eq!(dec.ref_y[4], 5, "4x8 右半分区应在 mvd=0 时继承左半 MVP");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8ref0_no_ref_idx_parse() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb0: skip_run=0, mb_type=4(P_8x8ref0), 四个 sub_mb_type=0, 不应读取 ref_idx
    // 仍需读取每个子分区 mvd=(0,0), 用于验证语法对齐。
    // mb1: skip_run=0, mb_type=5(I 宏块), 用于验证语法对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(4),
        Ue(0),
        Ue(0),
        Ue(0),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(5),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.mb_types[0], 203, "P_8x8ref0 宏块应标记为互预测类型");
    assert_eq!(dec.ref_y[0], 20, "P_8x8ref0 应固定使用 list0 的首参考帧");
    assert_eq!(dec.ref_y[8], 20, "P_8x8ref0 应固定使用 list0 的首参考帧");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        20,
        "P_8x8ref0 应固定使用 list0 的首参考帧"
    );
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        20,
        "P_8x8ref0 应固定使用 list0 的首参考帧"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应成功解析为 I 宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_blend_l0_l1() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;

    // mb_skip_run = 1, 覆盖单宏块帧
    let rbsp = build_rbsp_from_ues(&[1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 60, "B-slice skip 应融合 L0/L1 预测");
    assert_eq!(dec.mb_types[0], 254, "B-slice skip 宏块应标记为 B skip");
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_temporal_direct_uses_l0_only() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = false;

    // mb_skip_run = 1, 覆盖单宏块帧
    let rbsp = build_rbsp_from_ues(&[1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(
        dec.ref_y[0], 20,
        "B-slice temporal direct 最小路径应先按 L0 单向预测"
    );
    assert_eq!(dec.mb_types[0], 254, "B-slice skip 宏块应标记为 B skip");
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_temporal_direct_uses_colocated_mv() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 1, 2, None);
    push_custom_reference_with_l0_motion(&mut dec, 2, 8, 100, None, (4, 0, 0));

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = false;

    // mb_skip_run = 1, 覆盖单宏块帧
    let rbsp = build_rbsp_from_ues(&[1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(
        dec.ref_y[0], 1,
        "temporal direct 应使用共定位宏块 MV(+1px)驱动 L0 参考采样"
    );
    assert_eq!(dec.mv_l0_x[0], 4, "宏块记录的 L0 MV(x) 应来自共定位宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_uses_predicted_mv_from_left_neighbor() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = false;

    use ExpGolombValue::{Se, Ue};
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0), // mb0: skip_run=0
        Ue(1), // mb0: mb_type=1(B_L0_16x16)
        Se(4),
        Se(0), // mb0: mvd=(+1px,0)
        Ue(1), // mb1: skip_run=1
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(
        dec.ref_y[16], 17,
        "B_Skip 应使用左邻宏块预测 MV, 而非固定零向量"
    );
    assert_eq!(dec.mv_l0_x[1], 4, "B_Skip 应写入预测后的 MV(x)");
    assert_eq!(dec.mv_l0_y[1], 0, "B_Skip 应写入预测后的 MV(y)");
    assert_eq!(dec.mb_types[1], 254, "第二个宏块应按 B_Skip 路径解码");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_direct_uses_predicted_mv_from_left_neighbor() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = false;

    use ExpGolombValue::{Se, Ue};
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0), // mb0: skip_run=0
        Ue(1), // mb0: mb_type=1(B_L0_16x16)
        Se(4),
        Se(0), // mb0: mvd=(+1px,0)
        Ue(0), // mb1: skip_run=0
        Ue(0), // mb1: mb_type=0(B_Direct_16x16)
        Ue(0), // 占位尾码, 避免 mb_type=0 的单比特编码被误判为 rbsp_trailing_bits
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(
        dec.ref_y[16], 17,
        "B_Direct_16x16 应使用左邻宏块预测 MV, 而非固定零向量"
    );
    assert_eq!(dec.mv_l0_x[1], 4, "B_Direct_16x16 应写入预测后的 MV(x)");
    assert_eq!(dec.mv_l0_y[1], 0, "B_Direct_16x16 应写入预测后的 MV(y)");
    assert_eq!(dec.mb_types[1], 254, "第二个宏块应按 B_Direct 路径解码");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_direct_spatial_zero_condition_forces_zero_mv() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1). 预置 4x4 邻居候选使 L0 预测为 +1 像素.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 4, 0, 0);
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 0, 0, 0);

    // 但将左/上宏块的 MB 级 L0/L1 都标记为零向量, 触发 Spatial Direct 零 MV 条件.
    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    for idx in [left_mb, top_mb] {
        dec.mv_l0_x[idx] = 0;
        dec.mv_l0_y[idx] = 0;
        dec.ref_idx_l0[idx] = 0;
        dec.mv_l1_x[idx] = 0;
        dec.mv_l1_y[idx] = 0;
        dec.ref_idx_l1[idx] = 0;
    }

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = true;

    // mb3: skip_run=0, mb_type=0(B_Direct_16x16)
    // 额外追加 ue(0) 作为尾码占位, 避免单比特 mb_type 在 has_more_rbsp_data 判断中被吞掉.
    let rbsp = build_rbsp_from_ues(&[0, 0, 0]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let mb3_base = 16 + 16 * dec.stride_y;
    assert_eq!(
        dec.ref_y[mb3_base], 16,
        "Spatial Direct 零 MV 条件命中时应按零位移采样"
    );
    assert_eq!(
        dec.mv_l0_x[3], 0,
        "Spatial Direct 零 MV 条件命中时 L0 MV(x) 应为 0"
    );
    assert_eq!(
        dec.mv_l0_y[3], 0,
        "Spatial Direct 零 MV 条件命中时 L0 MV(y) 应为 0"
    );
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_direct_spatial_uses_independent_l1_neighbor_mv() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1). L0 预测来源于 4x4 邻居候选: +1 像素.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 4, 0, 0);
    dec.set_l0_motion_block_4x4(0, 0, 16, 16, 0, 0, 0);

    // L1 MB 级邻居单独设置为 +2 像素, 验证 L1 独立预测不复用 L0.
    let left_mb = dec.mb_index(0, 1).expect("左邻索引应存在");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引应存在");
    for idx in [left_mb, top_mb] {
        dec.mv_l0_x[idx] = 0;
        dec.mv_l0_y[idx] = 0;
        dec.ref_idx_l0[idx] = 0;
        dec.mv_l1_x[idx] = 8;
        dec.mv_l1_y[idx] = 0;
        dec.ref_idx_l1[idx] = 0;
    }

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = true;

    // mb3: skip_run=0, mb_type=0(B_Direct_16x16), 加一个占位尾码.
    let rbsp = build_rbsp_from_ues(&[0, 0, 0]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let mb3_base = 16 + 16 * dec.stride_y;
    assert_eq!(
        dec.ref_y[mb3_base], 17,
        "Spatial Direct 应分别使用 list0/list1 邻居 MV, 融合后像素应为 17"
    );
    assert_eq!(dec.mv_l0_x[3], 0, "L0 应独立使用 list0 邻居 MV(x)=0");
    assert_eq!(dec.mv_l0_y[3], 0, "L0 应独立使用 list0 邻居 MV(y)=0");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_l0_only() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=1(B_L0_16x16)
    let rbsp = build_rbsp_from_ues(&[0, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 20, "B_L0_16x16 应仅使用 list0 预测");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_l0_only_ref_idx() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 1, 99, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb_skip_run=0, mb_type=1(B_L0_16x16), ref_idx_l0=1
    let rbsp = build_rbsp_from_ues(&[0, 1, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 99, "B_L0_16x16 应按 ref_idx_l0 选择参考帧");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_l1_only() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=2(B_L1_16x16)
    let rbsp = build_rbsp_from_ues(&[0, 2]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 100, "B_L1_16x16 应仅使用 list1 预测");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_l1_only_ref_idx() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 1, 99, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l1 = 2;

    // mb_skip_run=0, mb_type=2(B_L1_16x16), ref_idx_l1=1
    let rbsp = build_rbsp_from_ues(&[0, 2, 1]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 99, "B_L1_16x16 应按 ref_idx_l1 选择参考帧");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_bi_16x16_ref_idx() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 1, 99, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb_skip_run=0, mb_type=3(B_Bi_16x16), ref_idx_l0=1, ref_idx_l1=1, mvd_l0/mvd_l1 均为 0
    let rbsp =
        build_rbsp_from_exp_golomb(&[Ue(0), Ue(3), Ue(1), Ue(1), Se(0), Se(0), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 99, "B_Bi_16x16 应按 ref_idx_l0/l1 选择参考帧");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_bi_16x16_ref_idx_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 1, 99, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=3(B_Bi_16x16), ref_idx_l0=1, ref_idx_l1=1, mvd_l0/mvd_l1 均为 0
    // mb1: skip_run=0, mb_type=23(intra), 用于验证语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(1),
        Ue(1),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 99, "首个 B_Bi_16x16 应按 ref_idx 选择参考");
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_bi_16x16_mvd_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 1, 99, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=3(B_Bi_16x16), ref_idx_l0=1, ref_idx_l1=1, mvd_l0=(2,-1), mvd_l1=(-2,1)
    // mb1: skip_run=0, mb_type=23(intra), 用于验证 mvd 语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(1),
        Ue(1),
        Se(2),
        Se(-1),
        Se(-2),
        Se(1),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 99, "B_Bi_16x16 应按 ref_idx 选择参考帧");
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b_l0_l1_16x8_ref_idx_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=8(B_L0_L1_16x8), top(ref_idx_l0=0,mvd=0), bottom(ref_idx_l1=0,mvd=0)
    // mb1: skip_run=0, mb_type=23(intra), 用于验证语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(8),
        Ue(0),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "上半分区应使用 L0 ref_idx=0");
    assert_eq!(dec.ref_y[15], 20, "上半分区右侧应使用 L0 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        100,
        "下半分区应使用 L1 ref_idx=0"
    );
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 15],
        100,
        "下半分区右侧应使用 L1 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b_l0_l1_16x8_mvd_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=8(B_L0_L1_16x8), top(ref_idx_l0=0,mvd=(1,0)), bottom(ref_idx_l1=0,mvd=(-1,0))
    // mb1: skip_run=0, mb_type=23(intra), 用于验证 mvd 语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(8),
        Ue(0),
        Ue(0),
        Se(1),
        Se(0),
        Se(-1),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "上半分区应使用 L0 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        100,
        "下半分区应使用 L1 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b_l0_l1_16x8_grouped_syntax_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=8(B_L0_L1_16x8)
    // 语法顺序要求: ref_idx_l0(part0) -> ref_idx_l1(part1) -> mvd_l0(part0) -> mvd_l1(part1)。
    // 这里将 bottom 的 ref_idx_l1 设为 1, 若顺序错误会被 mvd 码字污染并落回错误参考帧。
    // mb1: skip_run=0, mb_type=23(intra), 用于验证位流仍保持对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(8),
        Ue(0),
        Ue(1),
        Se(2),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "上半分区应使用 L0 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        20,
        "下半分区应使用 L1 ref_idx=1 对应的参考帧"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b_l0_l1_8x16_ref_idx_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=9(B_L0_L1_8x16), left(ref_idx_l0=0,mvd=0), right(ref_idx_l1=0,mvd=0)
    // mb1: skip_run=0, mb_type=23(intra), 用于验证语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(9),
        Ue(0),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "左半分区应使用 L0 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        20,
        "左半分区下方应使用 L0 ref_idx=0"
    );
    assert_eq!(dec.ref_y[8], 100, "右半分区应使用 L1 ref_idx=0");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        100,
        "右半分区下方应使用 L1 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b_l0_l1_8x16_grouped_syntax_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=9(B_L0_L1_8x16)
    // 语法顺序要求: ref_idx_l0(part0) -> ref_idx_l1(part1) -> mvd_l0(part0) -> mvd_l1(part1)。
    // 将右分区 ref_idx_l1 设为 1, 若顺序错误会读到 mvd 码字并回退到错误参考帧。
    // mb1: skip_run=0, mb_type=23(intra), 用于验证位流仍保持对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(9),
        Ue(0),
        Ue(1),
        Se(2),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "左分区应使用 L0 ref_idx=0");
    assert_eq!(dec.ref_y[8], 20, "右分区应使用 L1 ref_idx=1 对应的参考帧");
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_l0_ref_idx_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=22(B_8x8), sub_mb_type 全为 1(L0_8x8), ref_idx_l0=[0,1,1,0], mvd 全 0
    // mb1: skip_run=0, mb_type=23(intra), 用于验证位流消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(22),
        Ue(1),
        Ue(1),
        Ue(1),
        Ue(1),
        Ue(0),
        Ue(1),
        Ue(1),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 20, "左上 8x8 应使用 L0 ref_idx=0");
    assert_eq!(dec.ref_y[8], 100, "右上 8x8 应使用 L0 ref_idx=1");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        100,
        "左下 8x8 应使用 L0 ref_idx=1"
    );
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        20,
        "右下 8x8 应使用 L0 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_l1_ref_idx_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=22(B_8x8), sub_mb_type 全为 2(L1_8x8), ref_idx_l1=[0,1,1,0], mvd 全 0
    // mb1: skip_run=0, mb_type=23(intra), 用于验证位流消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(22),
        Ue(2),
        Ue(2),
        Ue(2),
        Ue(2),
        Ue(0),
        Ue(1),
        Ue(1),
        Ue(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 100, "左上 8x8 应使用 L1 ref_idx=0");
    assert_eq!(dec.ref_y[8], 20, "右上 8x8 应使用 L1 ref_idx=1");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        20,
        "左下 8x8 应使用 L1 ref_idx=1"
    );
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        100,
        "右下 8x8 应使用 L1 ref_idx=0"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_mixed_sub_mb_types_alignment() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=22(B_8x8)
    // sub_mb_type: [4(L0_8x4), 6(L1_8x4), 8(Bi_8x4), 12(Bi_4x4)]
    // ref_idx 顺序按规范分组: L0[sub0,sub2,sub3]=[1,0,1], L1[sub1,sub2,sub3]=[1,1,0]
    // mvd 顺序按规范分组: 先 L0(2+2+4 子分区), 再 L1(2+2+4 子分区), 全部取 0.
    // mb1: skip_run=0, mb_type=23(intra), 用于验证语法消费对齐。
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(22),
        Ue(4),
        Ue(6),
        Ue(8),
        Ue(12),
        // L0 ref_idx: sub0, sub2, sub3
        Ue(1),
        Ue(0),
        Ue(1),
        // L1 ref_idx: sub1, sub2, sub3
        Ue(1),
        Ue(1),
        Ue(0),
        // L0 mvd: sub0(2), sub2(2), sub3(4)
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        // L1 mvd: sub1(2), sub2(2), sub3(4)
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Se(0),
        Ue(0),
        Ue(23),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 100, "左上 8x8 应使用 L0 ref_idx=1");
    assert_eq!(dec.ref_y[8], 20, "右上 8x8 应使用 L1 ref_idx=1");
    assert_eq!(dec.ref_y[8 * dec.stride_y], 20, "左下 8x8 应使用 Bi(20,20)");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        100,
        "右下 8x8 应使用 Bi(100,100)"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_direct_no_ref_idx_parse() {
    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;
    header.num_ref_idx_l1 = 2;

    // mb0: skip_run=0, mb_type=22(B_8x8), sub_mb_type 全为 0(Direct_8x8), 不应读取 ref_idx
    // mb1: skip_run=0, mb_type=23(intra), 用于验证语法消费对齐。
    let rbsp = build_rbsp_from_ues(&[0, 22, 0, 0, 0, 0, 0, 23]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[0], 60, "Direct_8x8 最小路径应使用双向融合");
    assert_eq!(dec.ref_y[8], 60, "Direct_8x8 最小路径应使用双向融合");
    assert_eq!(
        dec.ref_y[8 * dec.stride_y],
        60,
        "Direct_8x8 最小路径应使用双向融合"
    );
    assert_eq!(
        dec.ref_y[8 * dec.stride_y + 8],
        60,
        "Direct_8x8 最小路径应使用双向融合"
    );
    assert_eq!(dec.mb_types[1], 1, "第二个宏块应解析为帧内宏块");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_direct_uses_predicted_mv_from_left_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    if let Some(sps) = dec.sps.as_mut() {
        sps.direct_8x8_inference_flag = true;
    }
    if let Some(sps) = dec.sps_map.get_mut(&0) {
        sps.direct_8x8_inference_flag = true;
    }
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.direct_spatial_mv_pred_flag = false;

    // mb0: skip_run=0, mb_type=1(B_L0_16x16), mvd=(+1px,0)
    // mb1: skip_run=0, mb_type=22(B_8x8), sub_mb_type 全为 0(Direct_8x8)
    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(1),
        Se(4),
        Se(0),
        Ue(0),
        Ue(22),
        Ue(0),
        Ue(0),
        Ue(0),
        Ue(0),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    assert_eq!(dec.ref_y[16], 17, "Direct_8x8 左上块应复用左邻 MVP");
    assert_eq!(dec.ref_y[24], 25, "Direct_8x8 右上块应复用左邻 MVP");
    assert_eq!(dec.mv_l0_x[1], 4, "Direct_8x8 宏块应写入预测后的 MV(x)");
    assert_eq!(dec.mv_l0_y[1], 0, "Direct_8x8 宏块应写入预测后的 MV(y)");
}

#[test]
fn test_decode_cavlc_slice_data_b_non_skip_b8x8_direct_right_sub_block_respects_inference_flag() {
    use ExpGolombValue::{Se, Ue};

    fn decode_with_inference_flag(flag: bool) -> u8 {
        let mut dec = build_test_decoder();
        let sps_resize = build_sps_nalu(0, 32, 16);
        dec.handle_sps(&sps_resize);
        if let Some(sps) = dec.sps.as_mut() {
            sps.direct_8x8_inference_flag = flag;
        }
        if let Some(sps) = dec.sps_map.get_mut(&0) {
            sps.direct_8x8_inference_flag = flag;
        }
        dec.last_slice_type = 1;
        dec.last_poc = 5;
        push_horizontal_gradient_reference(&mut dec, 3, 3, None);

        // 左邻宏块提供 +1px 的 L0 预测输入.
        dec.set_l0_motion_block_4x4(0, 0, 16, 16, 4, 0, 0);

        let mut header = build_test_slice_header(0, 1, false, None);
        header.slice_type = 1;
        header.first_mb = 1; // 仅解码右侧宏块.
        header.data_bit_offset = 0;
        header.direct_spatial_mv_pred_flag = false;
        header.num_ref_idx_l0 = 1;
        header.num_ref_idx_l1 = 1;

        // mb1: mb_type=22(B_8x8), sub_mb_type=[2(L1_8x8),0(Direct),0(Direct),0(Direct)].
        // 仅 sub0 读取 L1 mvd(0,0), 其它 direct 子块不读取 mvd/ref_idx.
        let rbsp =
            build_rbsp_from_exp_golomb(&[Ue(0), Ue(22), Ue(2), Ue(0), Ue(0), Ue(0), Se(0), Se(0)]);
        dec.decode_cavlc_slice_data(&rbsp, &header);

        // 右上 8x8 左上角像素(x=24,y=0): 用于观测 sub1(Direct) 的预测差异.
        dec.ref_y[24]
    }

    let pixel_true = decode_with_inference_flag(true);
    let pixel_false = decode_with_inference_flag(false);

    assert_eq!(
        pixel_true, 25,
        "direct_8x8_inference_flag=1 时, 右上 direct 子块应复用 8x8 预测 MV(+1px)"
    );
    assert_eq!(
        pixel_false, 24,
        "direct_8x8_inference_flag=0 时, 右上 direct 子块应按 4x4 粒度独立预测并回落到零 MV"
    );
}

#[test]
fn test_apply_b_direct_sub_8x8_respects_direct_8x8_inference_flag() {
    fn apply_with_direct_8x8_inference_flag(flag: bool) -> (u8, i16) {
        let mut dec = build_test_decoder();
        let sps_resize = build_sps_nalu(0, 32, 16);
        dec.handle_sps(&sps_resize);
        if let Some(sps) = dec.sps.as_mut() {
            sps.direct_8x8_inference_flag = flag;
        }
        if let Some(sps) = dec.sps_map.get_mut(&0) {
            sps.direct_8x8_inference_flag = flag;
        }
        dec.last_slice_type = 1;
        dec.last_poc = 5;
        push_horizontal_gradient_reference(&mut dec, 3, 3, None);

        // 左邻宏块预置 +1px 的 L0 运动, 用于构造 16x16 预测输入.
        dec.set_l0_motion_block_4x4(0, 0, 16, 16, 4, 0, 0);

        let (pred_mv_x, pred_mv_y) = dec.predict_mv_l0_16x16(1, 0);
        let ref_l0_list = dec.build_reference_list_l0_with_mod(1, &[], 0);
        let ref_l1_list: Vec<RefPlanes> = Vec::new();
        let _ = dec.apply_b_direct_sub_8x8(
            1,
            0,
            8,
            0,
            pred_mv_x,
            pred_mv_y,
            false,
            &[],
            &[],
            0,
            0,
            &ref_l0_list,
            &ref_l1_list,
        );

        (dec.ref_y[24], dec.mv_l0_x[1])
    }

    let (pix_true, mv_true) = apply_with_direct_8x8_inference_flag(true);
    let (pix_false, mv_false) = apply_with_direct_8x8_inference_flag(false);

    assert_eq!(
        pix_true, 25,
        "direct_8x8_inference_flag=1 时应沿用 8x8 预测 MV(+1px)"
    );
    assert_eq!(
        mv_true, 4,
        "direct_8x8_inference_flag=1 时应记录 8x8 预测 MV(x)=4"
    );

    assert_eq!(
        pix_false, 24,
        "direct_8x8_inference_flag=0 时应按 4x4 粒度独立预测并回落到零 MV"
    );
    assert_eq!(
        mv_false, 0,
        "direct_8x8_inference_flag=0 时最后分区应记录独立 4x4 MV(x)=0"
    );
}

#[test]
fn test_decode_cavlc_slice_data_b_skip_run_explicit_weighted() {
    let mut dec = build_test_decoder();
    dec.last_slice_type = 1;
    dec.last_poc = 5;
    push_custom_reference(&mut dec, 1, 2, 20, None);
    push_custom_reference(&mut dec, 2, 8, 100, None);

    let mut pps = build_test_pps();
    pps.weighted_bipred_idc = 1;
    dec.pps = Some(pps);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 1; // B slice
    header.data_bit_offset = 0;
    header.luma_log2_weight_denom = 2;
    header.chroma_log2_weight_denom = 0;
    header.l0_weights = vec![PredWeightL0 {
        luma_weight: 0,
        luma_offset: 0,
        chroma_weight: [1, 1],
        chroma_offset: [0, 0],
    }];
    header.l1_weights = vec![PredWeightL0 {
        luma_weight: 4,
        luma_offset: 0,
        chroma_weight: [1, 1],
        chroma_offset: [0, 0],
    }];

    let rbsp = build_rbsp_from_ues(&[1]); // mb_skip_run=1
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 50, "显式加权 B-slice skip 应应用权重");
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
fn test_compute_slice_poc_type2_non_ref_wrap_does_not_update_prev_offset() {
    let mut dec = build_test_decoder();
    let sps = build_test_sps_with_poc_type(0, 2);
    dec.sps_map.insert(0, sps.clone());
    dec.sps = Some(sps);
    dec.active_sps_id = Some(0);
    dec.prev_frame_num_offset_type2 = 16;

    let non_ref_wrap = build_test_slice_header(0, 0, false, None);
    let poc_non_ref = dec.compute_slice_poc(&non_ref_wrap, 15);
    assert_eq!(poc_non_ref, 63, "非参考帧 wrap 的 POC 计算错误");
    assert_eq!(
        dec.prev_frame_num_offset_type2, 16,
        "非参考帧不应更新 prev_frame_num_offset_type2"
    );

    let ref_after_non_ref = build_test_slice_header(1, 1, false, None);
    let poc_ref = dec.compute_slice_poc(&ref_after_non_ref, 0);
    assert_eq!(poc_ref, 34, "后续参考帧应基于上一个参考帧偏移继续计算");
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

#[test]
fn test_sample_h264_chroma_qpel_bilinear_weighting() {
    let width = 4usize;
    let height = 4usize;
    let mut plane = vec![0u8; width * height];
    plane[width + 1] = 10;
    plane[width + 2] = 20;
    plane[2 * width + 1] = 30;
    plane[2 * width + 2] = 50;

    let sample = sample_h264_chroma_qpel(&plane, width, width, height, 1, 1, 3, 5);
    assert_eq!(sample, 29, "色度双线性加权结果应符合 H264 1/8 插值公式");
}

#[test]
fn test_sample_h264_chroma_qpel_edge_clamp() {
    let width = 2usize;
    let height = 2usize;
    let plane = vec![10u8, 20u8, 30u8, 40u8];

    let sample = sample_h264_chroma_qpel(&plane, width, width, height, -1, -1, 7, 7);
    assert_eq!(sample, 10, "越界色度采样应按边界复制后再执行插值");
}

#[test]
fn test_parse_sei_known_payloads_and_unknown_skip() {
    let mut uuid = [0u8; 16];
    for (idx, value) in uuid.iter_mut().enumerate() {
        *value = idx as u8;
    }
    let mut user_data_payload = uuid.to_vec();
    user_data_payload.extend_from_slice(&[0xAA, 0xBB]);
    let pic_timing_payload = vec![0x12, 0x34, 0x56];
    let recovery_payload = build_recovery_point_payload(7, true, false, 2);
    let unknown_payload = vec![0xDE, 0xAD, 0xBE, 0xEF];

    let rbsp = build_sei_rbsp(&[
        (0, build_rbsp_from_ues(&[3])),
        (1, pic_timing_payload.clone()),
        (5, user_data_payload),
        (6, recovery_payload),
        (255, unknown_payload.clone()),
    ]);
    let payloads = parse_sei_rbsp(&rbsp).expect("SEI 已知类型解析应成功");
    assert_eq!(payloads.len(), 5, "SEI payload 数量应为 5");

    match &payloads[0].message {
        SeiMessage::BufferingPeriod(bp) => {
            assert_eq!(
                bp.seq_parameter_set_id, 3,
                "buffering_period 的 sps_id 解析错误"
            );
        }
        _ => panic!("第一个 SEI payload 应为 buffering_period"),
    }
    match &payloads[1].message {
        SeiMessage::PicTiming(pic_timing) => {
            assert_eq!(
                pic_timing.raw, pic_timing_payload,
                "pic_timing 原始数据应原样保留"
            );
        }
        _ => panic!("第二个 SEI payload 应为 pic_timing"),
    }
    match &payloads[2].message {
        SeiMessage::UserDataUnregistered(user_data) => {
            assert_eq!(
                user_data.uuid_iso_iec_11578, uuid,
                "user_data_unregistered UUID 解析错误"
            );
            assert_eq!(
                user_data.payload,
                vec![0xAA, 0xBB],
                "user_data_unregistered 用户数据解析错误"
            );
        }
        _ => panic!("第三个 SEI payload 应为 user_data_unregistered"),
    }
    match &payloads[3].message {
        SeiMessage::RecoveryPoint(recovery) => {
            assert_eq!(
                recovery.recovery_frame_cnt, 7,
                "recovery_frame_cnt 解析错误"
            );
            assert!(recovery.exact_match_flag, "exact_match_flag 解析错误");
            assert!(!recovery.broken_link_flag, "broken_link_flag 解析错误");
            assert_eq!(
                recovery.changing_slice_group_idc, 2,
                "changing_slice_group_idc 解析错误"
            );
        }
        _ => panic!("第四个 SEI payload 应为 recovery_point"),
    }
    match &payloads[4].message {
        SeiMessage::Unknown { data } => {
            assert_eq!(data, &unknown_payload, "未知 SEI payload 应原样保留");
        }
        _ => panic!("第五个 SEI payload 应为未知类型"),
    }
}

#[test]
fn test_parse_sei_payload_size_truncated_should_fail() {
    let rbsp = vec![0x06, 0x04, 0x12, 0x34, 0x80];
    let err = parse_sei_rbsp(&rbsp).expect_err("SEI payload 截断应返回错误");
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("SEI payload 截断"),
        "截断错误信息应包含上下文, got={err_msg}"
    );
}

#[test]
fn test_parse_sei_user_data_unregistered_truncated_should_fail() {
    let rbsp = build_sei_rbsp(&[(5, vec![0x11; 8])]);
    let err = parse_sei_rbsp(&rbsp).expect_err("短 user_data_unregistered 应返回错误");
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("user_data_unregistered 截断"),
        "user_data_unregistered 截断错误信息应包含上下文, got={err_msg}"
    );
}

#[test]
fn test_send_packet_sei_updates_last_payloads() {
    let mut dec = build_test_decoder();
    let packet = build_sei_avcc_packet(&[
        (255, vec![0xA5]),
        (6, build_recovery_point_payload(2, false, true, 1)),
    ]);

    dec.send_packet(&packet)
        .expect("send_packet 处理 SEI NAL 应成功");
    assert_eq!(dec.last_sei_payloads.len(), 2, "应记录两条 SEI payload");
    assert_eq!(
        dec.last_sei_payloads[0].payload_type, 255,
        "第一条 SEI payload_type 应为未知类型 255"
    );
    match &dec.last_sei_payloads[0].message {
        SeiMessage::Unknown { data } => {
            assert_eq!(data.as_slice(), [0xA5], "未知 SEI payload 数据应原样保留");
        }
        _ => panic!("第一条 SEI 应按未知类型处理"),
    }
    match &dec.last_sei_payloads[1].message {
        SeiMessage::RecoveryPoint(recovery) => {
            assert_eq!(recovery.recovery_frame_cnt, 2, "恢复点帧计数解析错误");
            assert!(!recovery.exact_match_flag, "exact_match_flag 解析错误");
            assert!(recovery.broken_link_flag, "broken_link_flag 解析错误");
            assert_eq!(
                recovery.changing_slice_group_idc, 1,
                "changing_slice_group_idc 解析错误"
            );
        }
        _ => panic!("第二条 SEI 应为 recovery_point"),
    }
}

#[test]
fn test_consume_recovery_point_for_new_picture_countdown_and_mark_keyframe() {
    let mut dec = build_test_decoder();
    dec.pending_recovery_point_frame_cnt = Some(2);

    assert!(
        !dec.consume_recovery_point_for_new_picture(false),
        "倒计时未归零时不应标记随机访问点"
    );
    assert_eq!(
        dec.pending_recovery_point_frame_cnt,
        Some(1),
        "消费一次后计数应减 1"
    );

    assert!(
        !dec.consume_recovery_point_for_new_picture(false),
        "倒计时=1 的图像仍不应标记随机访问点"
    );
    assert_eq!(
        dec.pending_recovery_point_frame_cnt,
        Some(0),
        "倒计时应继续下降到 0"
    );

    assert!(
        dec.consume_recovery_point_for_new_picture(false),
        "倒计时归零后的下一非 IDR 图像应标记随机访问点"
    );
    assert_eq!(
        dec.pending_recovery_point_frame_cnt, None,
        "随机访问点命中后应清除 recovery_point 状态"
    );

    dec.pending_recovery_point_frame_cnt = Some(3);
    assert!(
        !dec.consume_recovery_point_for_new_picture(true),
        "IDR 图像不应走 recovery_point 随机访问点标记"
    );
    assert_eq!(
        dec.pending_recovery_point_frame_cnt, None,
        "遇到 IDR 图像后应清空 recovery_point 状态"
    );
}

#[test]
fn test_send_packet_marks_non_idr_pending_frame_keyframe_from_recovery_point() {
    let mut dec = build_test_decoder();
    install_basic_parameter_sets(&mut dec, 0);

    let mut sei_nalu = vec![0x06];
    sei_nalu.extend_from_slice(&build_sei_rbsp(&[(
        6,
        build_recovery_point_payload(0, true, false, 0),
    )]));

    let mut slice_nalu = vec![0x41]; // non-IDR slice
    slice_nalu.extend_from_slice(&build_p_slice_header_rbsp(0, 0, 0, 0, 0, 1));

    let mut avcc = Vec::new();
    for nalu in [&sei_nalu, &slice_nalu] {
        avcc.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
        avcc.extend_from_slice(nalu);
    }
    let packet = Packet::from_data(avcc);

    dec.send_packet(&packet)
        .expect("recovery_point + non-IDR 包应可正常处理");

    assert_eq!(
        dec.pending_frame.as_ref().map(|meta| meta.is_keyframe),
        Some(true),
        "recovery_frame_cnt=0 时首个非 IDR 图像应标记为随机访问点"
    );
    assert_eq!(
        dec.pending_recovery_point_frame_cnt, None,
        "随机访问点命中后应消费并清空 recovery_point 状态"
    );
}
