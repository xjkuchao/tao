use crate::decoder::Decoder;
use crate::packet::Packet;

use super::super::sei::{SeiMessage, parse_sei_rbsp};

use super::helpers::*;

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
