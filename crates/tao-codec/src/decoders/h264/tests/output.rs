use tao_core::Rational;

use crate::frame::Frame;

use super::helpers::*;

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
