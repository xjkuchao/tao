use std::collections::{HashMap, VecDeque};

use tao_core::Rational;

use crate::frame::Frame;

use super::{
    DecRefPicMarking, H264Decoder, MmcoOp, NalUnit, ParameterSetRebuildAction, PendingFrameMeta,
    Pps, RefPicListMod, RefPlanes, ReferencePicture, SliceHeader, Sps, sample_h264_luma_qpel,
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
