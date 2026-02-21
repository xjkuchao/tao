use super::super::{PredWeightL0, RefPlanes};

use super::helpers::*;

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
