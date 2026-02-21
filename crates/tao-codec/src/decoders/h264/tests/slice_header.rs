use tao_core::bitreader::BitReader;

use super::super::{NalUnit, RefPicListMod};

use super::helpers::*;

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
