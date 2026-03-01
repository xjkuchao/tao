use crate::decoder::Decoder;
use crate::packet::Packet;
use tao_core::bitreader::BitReader;

use super::super::{H264Decoder, NalUnit};

use super::helpers::*;

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
    dec.height = 32;
    dec.init_buffers();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // P_Skip 放到 (1,1): 左邻 (0,1) 有非零 MV, 上邻 (1,0) 有非零 MV.
    // 规范 8.4.1.1: 两邻居都可用且非零向量时, 走 median 预测.
    let left_mb = dec.mb_index(0, 1).expect("左邻索引");
    let top_mb = dec.mb_index(1, 0).expect("上邻索引");
    // 左邻: mv=(4,0), ref=0 → 不触发零向量快捷
    dec.mv_l0_x[left_mb] = 4;
    dec.mv_l0_y[left_mb] = 0;
    dec.ref_idx_l0[left_mb] = 0;
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 0);
    dec.mb_types[left_mb] = 200;
    // 上邻: mv=(4,0), ref=0 → 不触发零向量快捷
    dec.mv_l0_x[top_mb] = 4;
    dec.mv_l0_y[top_mb] = 0;
    dec.ref_idx_l0[top_mb] = 0;
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 4, 0, 0);
    dec.mb_types[top_mb] = 200;

    // 目标宏块 (1,1) = mb_idx=3 (mb_width=2)
    let target_mb = dec.mb_index(1, 1).expect("目标索引");
    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = target_mb as u32;
    header.data_bit_offset = 0;

    let rbsp = build_rbsp_from_ues(&[1]); // mb_skip_run=1
    dec.decode_cavlc_slice_data(&rbsp, &header);

    // median(4, 4, ?) = 4, 所以 MV=(4,0) → 偏移 +1 像素
    let stride = dec.stride_y;
    let mb1_1_y0_idx = 16 + 16 * stride;
    assert_eq!(
        dec.ref_y[mb1_1_y0_idx], 17,
        "P_Skip 应使用 MVP=+1 像素, 而非直接复制同坐标参考像素"
    );
    assert_eq!(dec.mv_l0_x[target_mb], 4, "P_Skip 应写入预测后的宏块 MV(x)");
    assert_eq!(dec.mv_l0_y[target_mb], 0, "P_Skip 应写入预测后的宏块 MV(y)");
    assert_eq!(
        dec.ref_idx_l0[target_mb], 0,
        "P_Skip 应固定使用 L0 的 ref_idx=0"
    );
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
    dec.width = 48;
    dec.height = 16;
    dec.init_buffers();
    let before = dec.malformed_nal_drops;

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.first_mb = 0;

    // skip_run=1(010) 后构造全 0 截断数据, 触发第二个宏块的 mb_type ue 解码失败.
    dec.decode_cavlc_slice_data(&[0x40], &header);

    assert_eq!(
        dec.malformed_nal_drops,
        before + 1,
        "宏块 mb_type 解码失败应计入坏 NAL 丢弃统计"
    );
    assert_eq!(dec.mb_types[0], 255, "首个宏块应按 skip_run=1 走 P_Skip");
    assert_eq!(dec.mb_types[1], 252, "出错宏块应标记为错误态 mb_type=252");
    assert_eq!(dec.mb_types[2], 0, "异常后应停止本 slice 后续宏块解码");
    assert_eq!(
        dec.mb_slice_first_mb[0], 0,
        "已解码宏块应记录所属 slice first_mb"
    );
    assert_eq!(
        dec.mb_slice_first_mb[1], 0,
        "出错宏块应记录所属 slice first_mb"
    );
    assert_eq!(
        dec.mb_slice_first_mb[2],
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
fn test_decode_cavlc_slice_data_p_skip_at_slice_start_does_not_use_prev_slice_left_mv() {
    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 16;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 5, 5, None);

    let mut header0 = build_test_slice_header(0, 1, false, None);
    header0.slice_type = 0; // P slice
    header0.data_bit_offset = 0;
    header0.first_mb = 0;
    // slice0: mb0 非 skip, P16x16, mvd=(+1px,0), 并在 rbsp_trailing_bits 结束.
    let mut bits0 = Vec::new();
    write_ue(&mut bits0, 0);
    write_ue(&mut bits0, 0);
    write_se(&mut bits0, 4);
    write_se(&mut bits0, 0);
    let rbsp0 = bits_to_bytes(&bits0);
    dec.decode_cavlc_slice_data(&rbsp0, &header0);

    let mut header1 = build_test_slice_header(0, 1, false, None);
    header1.slice_type = 0; // P slice
    header1.data_bit_offset = 0;
    header1.first_mb = 1;
    // slice1: 仅包含 mb1 的 skip_run=1.
    let mut bits1 = Vec::new();
    write_ue(&mut bits1, 1);
    let rbsp1 = bits_to_bytes(&bits1);
    dec.decode_cavlc_slice_data(&rbsp1, &header1);

    assert_eq!(dec.ref_y[0], 1, "mb0 应保持第一 slice 的 +1 像素位移结果");
    assert_eq!(
        dec.ref_y[16], 16,
        "第二 slice 首个 P-skip 宏块不应借用前一 slice 左邻 MV"
    );
    assert_eq!(dec.mb_slice_first_mb[0], 0, "mb0 应保留 first_mb=0");
    assert_eq!(dec.mb_slice_first_mb[1], 1, "mb1 应保留 first_mb=1");
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

    // mb_skip_run = 0, mb_type = 6(I_16x16 in P-slice domain), 后接 I_16x16 最小尾
    let rbsp = build_rbsp_from_exp_golomb_with_tail(
        &[ExpGolombValue::Ue(0), ExpGolombValue::Ue(6)],
        &i16x16_minimal_tail_bits(),
    );
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
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_te(&mut bits, 1, 1);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_ue(&mut bits, 0); // CBP=0
    let rbsp = bits_to_bytes(&bits);
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(
        dec.ref_y[0], 99,
        "P-slice 非 skip 互预测应按 ref_idx_l0 选择参考帧"
    );
    assert_eq!(dec.mb_types[0], 200, "P_L0_16x16 应标记为互预测宏块");
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_ref_idx_l0_mvd_alignment() {
    let mut dec = build_test_decoder();
    let sps_resize = build_sps_nalu(0, 32, 16);
    dec.handle_sps(&sps_resize);
    push_custom_reference(&mut dec, 3, 3, 20, None);
    push_custom_reference(&mut dec, 2, 2, 90, None);

    let mut header = build_test_slice_header(4, 1, false, None);
    header.slice_type = 0; // P slice
    header.data_bit_offset = 0;
    header.num_ref_idx_l0 = 2;

    // mb0: skip_run=0, mb_type=0(P_L0_16x16), ref_idx_l0=1, mvd=(0,0), CBP=0
    // mb1: skip_run=0, mb_type=6(I_16x16), 后接 I_16x16 最小尾, 用于验证 mvd 语法消费对齐。
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_te(&mut bits, 1, 1);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_ue(&mut bits, 0); // CBP=0
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 6);
    bits.extend(i16x16_minimal_tail_bits());
    let rbsp = bits_to_bytes(&bits);
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
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 1);
    write_te(&mut bits, 1, 0);
    write_te(&mut bits, 1, 1);
    let rbsp = bits_to_bytes(&bits);
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
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 2);
    write_te(&mut bits, 1, 0);
    write_te(&mut bits, 1, 1);
    let rbsp = bits_to_bytes(&bits);
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
fn test_decode_cavlc_slice_data_p_non_skip_inter_16x8_part1_prefers_left_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1), 左邻可用且 ref 匹配:
    // - 左邻 MV=+2 像素
    // - 顶部分区 mvd=(+1 像素), 其预测会受左邻影响
    // FFmpeg pred_16x8(n=1) 语义下, part1 应优先取左邻, 不应强制复用 part0.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 8, 0, 0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=1(P_L0_L0_16x8), mvd_top=(4,0), mvd_bottom=(0,0)
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(1), Se(4), Se(0), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(dec.ref_y[base], 19, "part0 应包含左邻预测 + mvd 叠加");
    assert_eq!(
        dec.ref_y[base + 8 * dec.stride_y],
        18,
        "part1 应优先使用左邻 MVP(+2 像素), 不应强制复用 part0"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_8x16_part1_prefers_diagonal_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1):
    // - 左邻 MV=+1 像素
    // - 对角(D)邻居 MV=+3 像素 (当前图宽 2MB, C 不可用时应回退 D)
    // FFmpeg pred_8x16(n=1) 语义下, part1 应优先取对角邻居而非复用 part0.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 4, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 12, 0, 0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;

    // mb_skip_run=0, mb_type=2(P_L0_L0_8x16), mvd_left=(4,0), mvd_right=(0,0)
    let rbsp = build_rbsp_from_exp_golomb(&[Ue(0), Ue(2), Se(4), Se(0), Se(0), Se(0)]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(dec.ref_y[base], 18, "part0 应为 +2 像素位移");
    assert_eq!(
        dec.ref_y[base + 8],
        27,
        "part1 应优先使用对角邻居 MVP(+3 像素), 不应强制复用 part0"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_16x16_prefers_single_ref_match_mvp() {
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
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_te(&mut bits, 1, 1);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    let rbsp = bits_to_bytes(&bits);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(
        dec.ref_y[base], 17,
        "P_L0_16x16 应优先使用与 ref_idx 匹配的左邻候选 MV"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_ref_idx_and_alignment() {
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
    // 并为每个子分区提供 mvd=(0,0), CBP=0; mb1: skip_run=0, mb_type=6(I_16x16), 后接 I_16x16 最小尾.
    let mut bits = Vec::new();
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 3);
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 0);
    write_te(&mut bits, 1, 0);
    write_te(&mut bits, 1, 1);
    write_te(&mut bits, 1, 1);
    write_te(&mut bits, 1, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_se(&mut bits, 0);
    write_ue(&mut bits, 0); // CBP=0
    write_ue(&mut bits, 0);
    write_ue(&mut bits, 6);
    bits.extend(i16x16_minimal_tail_bits());
    let rbsp = bits_to_bytes(&bits);
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
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_8x4_part1_prefers_left_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.set_dir_sub_mv_predictor_for_test(true, false);
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1), sub0=8x4:
    // - A(左邻)=+2 像素
    // - B(当前 sub0 上半)=+3 像素
    // - C(上方右侧)=+5 像素
    // FFmpeg pred_16x8(n=1) 语义下, part1 应直接取 A, 不应退化到中值.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(16, 0, 16, 16, 20, 0, 0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;

    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(1),
        Ue(0),
        Ue(0),
        Ue(0),
        Se(-8),
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
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(dec.ref_y[base], 19, "sub0 上半应为 +3 像素位移");
    assert_eq!(
        dec.ref_y[base + 4 * dec.stride_y],
        18,
        "sub0 下半应优先使用左邻 MVP(+2 像素)"
    );
}

#[test]
fn test_decode_cavlc_slice_data_p_non_skip_inter_p8x8_4x8_part1_prefers_diagonal_neighbor() {
    use ExpGolombValue::{Se, Ue};

    let mut dec = build_test_decoder();
    dec.set_dir_sub_mv_predictor_for_test(false, true);
    dec.width = 32;
    dec.height = 32;
    dec.init_buffers();
    dec.reference_frames.clear();
    push_horizontal_gradient_reference(&mut dec, 3, 3, None);

    // 目标宏块为 (1,1), sub0=4x8:
    // - A(左邻/part0)=+2 像素
    // - B(上邻)=+4 像素
    // - C(对角)=+6 像素
    // FFmpeg pred_4x8 part1 语义下, 应优先使用对角 C.
    dec.set_l0_motion_block_4x4(0, 16, 16, 16, 8, 0, 0);
    dec.set_l0_motion_block_4x4(20, 12, 4, 4, 16, 0, 0);
    dec.set_l0_motion_block_4x4(24, 12, 4, 4, 24, 0, 0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 0; // P slice
    header.first_mb = 3; // 仅解码右下宏块
    header.data_bit_offset = 0;

    let rbsp = build_rbsp_from_exp_golomb(&[
        Ue(0),
        Ue(3),
        Ue(2),
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
        Se(0),
        Se(0),
        Ue(0),
    ]);
    dec.decode_cavlc_slice_data(&rbsp, &header);

    let base = 16 + 16 * dec.stride_y;
    assert_eq!(dec.ref_y[base], 18, "sub0 左半应为 +2 像素位移");
    assert_eq!(
        dec.ref_y[base + 4],
        26,
        "sub0 右半应优先使用对角 MVP(+6 像素)"
    );
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
    // 仍需读取每个子分区 mvd=(0,0), CBP=0; mb1: skip_run=0, mb_type=6(I_16x16), 后接 I_16x16 最小尾.
    let rbsp = build_rbsp_from_exp_golomb_with_tail(
        &[
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
            Ue(0), // CBP=0
            Ue(0),
            Ue(6),
        ],
        &i16x16_minimal_tail_bits(),
    );
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
fn test_decode_cavlc_slice_data_i_minimal_intra_predict() {
    let mut dec = build_test_decoder();
    dec.ref_y.fill(0);
    dec.ref_u.fill(0);
    dec.ref_v.fill(0);

    let mut header = build_test_slice_header(0, 1, false, None);
    header.slice_type = 2; // I slice
    header.data_bit_offset = 0;

    // mb_type = ue(1) (I_16x16), 后接最小 I_16x16 语法: chroma ue(0), qp_delta se(0), luma DC 全零
    let rbsp = build_rbsp_i_slice_one_i16x16_minimal();
    dec.decode_cavlc_slice_data(&rbsp, &header);
    assert_eq!(dec.ref_y[0], 128, "I-slice 最小路径应执行帧内预测");
    assert_eq!(dec.mb_types[0], 1, "I-slice 最小路径应标记为帧内宏块");
}

#[test]
fn test_decode_cavlc_mb_residual_inter_cbp_zero_clears_prev_qp_delta_flag() {
    let mut dec = build_test_decoder();
    dec.prev_qp_delta_nz = true;

    // Inter CBP code_num=0 => cbp=0, 本宏块不含残差也不含 mb_qp_delta.
    let rbsp = build_rbsp_from_ues(&[0]);
    let mut br = BitReader::new(&rbsp);
    let mut cur_qp = 26;

    dec.decode_cavlc_mb_residual(&mut br, 0, 0, &mut cur_qp, false, true);

    assert!(
        !dec.prev_qp_delta_nz,
        "Inter 宏块在无残差时应清零 prev_qp_delta_nz"
    );
    assert_eq!(cur_qp, 26, "无残差时不应改写当前 QP");
}

#[test]
fn test_decode_cavlc_mb_residual_inter_reads_transform8x8_before_qp_delta() {
    let mut dec = build_test_decoder();
    let mut pps = build_test_pps();
    pps.transform_8x8_mode = true;
    dec.pps = Some(pps.clone());
    dec.pps_map.insert(pps.pps_id, pps);

    // 语法顺序(Inter):
    // coded_block_pattern=ue(2)->cbp=1(luma), transform_size_8x8_flag=1, mb_qp_delta=se(+1),
    // 后接 4 个 4x4 块 coeff_token=0(比特 '1') 作为最小残差体.
    let mut bits = Vec::new();
    write_ue(&mut bits, 2);
    bits.push(true);
    write_se(&mut bits, 1);
    bits.extend([true, true, true, true]);
    let rbsp = bits_to_bytes(&bits);
    let mut br = BitReader::new(&rbsp);
    let mut cur_qp = 26;

    dec.decode_cavlc_mb_residual(&mut br, 0, 0, &mut cur_qp, false, true);

    assert!(
        dec.get_transform_8x8_flag(0, 0),
        "Inter CAVLC 应先读取 transform_size_8x8_flag"
    );
    assert_eq!(
        cur_qp, 27,
        "mb_qp_delta 应在 transform_size_8x8_flag 之后按 se(+1) 生效"
    );
    assert!(
        dec.prev_qp_delta_nz,
        "mb_qp_delta 非零后应置位 prev_qp_delta_nz"
    );
    assert_eq!(dec.mb_cbp[0] & 0x0f, 1, "CBP 低四位应保持为 luma_cbp=1");
}
