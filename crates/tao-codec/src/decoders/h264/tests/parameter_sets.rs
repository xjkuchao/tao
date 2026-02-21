use tao_core::Rational;

use super::super::{H264Decoder, ParameterSetRebuildAction, PendingFrameMeta};

use super::helpers::*;

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
