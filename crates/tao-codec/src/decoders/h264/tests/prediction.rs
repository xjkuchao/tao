use super::super::{
    BMotion, PredWeightL0, RefPlanes, sample_h264_chroma_qpel, sample_h264_luma_qpel,
};

use super::helpers::*;

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
