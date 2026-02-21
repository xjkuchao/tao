use super::super::{DecRefPicMarking, MmcoOp, RefPicListMod};

use super::helpers::*;

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
