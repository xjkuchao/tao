use super::*;
use crate::codec_parameters::VideoCodecParams;
use tao_core::Rational;

/// 创建测试用解码器实例
fn test_decoder(width: u32, height: u32) -> Mpeg4Decoder {
    let mb_stride = if width > 0 {
        (width as usize).div_ceil(16)
    } else {
        0
    };
    let mb_count = if width > 0 && height > 0 {
        mb_stride * (height as usize).div_ceil(16)
    } else {
        0
    };
    Mpeg4Decoder {
        width,
        height,
        pixel_format: PixelFormat::Yuv420p,
        opened: width > 0,
        reference_frame: None,
        backward_reference: None,
        pending_frame: None,
        dpb: Vec::new(),
        frame_count: 0,
        quant: 1,
        vol_info: None,
        quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
        quant_matrix_inter: STD_INTER_QUANT_MATRIX,
        predictor_cache: Vec::new(),
        mv_cache: vec![[MotionVector::default(); 4]; mb_count],
        ref_mv_cache: vec![[MotionVector::default(); 4]; mb_count],
        mb_info: vec![MacroblockInfo::default(); mb_count],
        mb_stride,
        f_code_forward: 1,
        f_code_backward: 1,
        rounding_control: 0,
        intra_dc_vlc_thr: 0,
        time_pp: 0,
        time_bp: 0,
        last_time_base: 0,
        time_base_acc: 0,
        last_non_b_time: 0,
        gmc_params: GmcParameters::default(),
        alternate_vertical_scan: false,
        packed_frames: std::collections::VecDeque::new(),
        wait_keyframe: false,
        resync_mb_x: 0,
        resync_mb_y: 0,
    }
}

#[test]
fn test_mpeg4_decoder_create() {
    let decoder = Mpeg4Decoder::create();
    assert!(decoder.is_ok());
}

#[test]
fn test_mpeg4_decoder_open() {
    let mut decoder = Mpeg4Decoder::create().unwrap();
    let params = CodecParameters {
        codec_id: CodecId::Mpeg4,
        bit_rate: 0,
        extra_data: vec![],
        params: CodecParamsType::Video(VideoCodecParams {
            width: 640,
            height: 480,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: Rational::new(25, 1),
            sample_aspect_ratio: Rational::new(1, 1),
        }),
    };
    assert!(decoder.open(&params).is_ok());
}

#[test]
fn test_dc_scaler() {
    let decoder = test_decoder(0, 0);
    assert_eq!(decoder.get_dc_scaler(true), 8);
    assert_eq!(decoder.get_dc_scaler(false), 8);
}

#[test]
fn test_cbpy_inter_inversion() {
    // VLC 码 `11` (2 bits) 映射到 CBPY=15, 对应数据 0xC0 (1100_0000)
    let data = [0xC0]; // 1100 0000
    let mut reader = BitReader::new(&data);
    let cbpy_intra = decode_cbpy(&mut reader, true);
    assert_eq!(cbpy_intra, Some(15));

    let mut reader2 = BitReader::new(&data);
    let cbpy_inter = decode_cbpy(&mut reader2, false);
    assert_eq!(cbpy_inter, Some(0));
}

#[test]
fn test_mv_range_wrapping() {
    let decoder = test_decoder(320, 240);
    let pmv = decoder.get_pmv(0, 0, 0);
    assert_eq!(pmv.x, 0);
    assert_eq!(pmv.y, 0);
}

#[test]
fn test_integer_idct() {
    let mut block = [0i32; 64];
    idct_8x8(&mut block);
    for &v in &block {
        assert_eq!(v, 0);
    }

    let mut block2 = [0i32; 64];
    block2[0] = 100;
    idct_8x8(&mut block2);
    let first = block2[0];
    for &v in &block2 {
        assert!(
            (v - first).abs() <= 1,
            "DC-only block 不均匀: {} vs {}",
            v,
            first
        );
    }
}

#[test]
fn test_b_frame_modb_decode() {
    use super::vlc::{decode_b_mb_type, decode_dbquant, decode_modb};

    // MODB = "1" -> no mb_type, no cbp
    let data = [0x80]; // 1000 0000
    let mut reader = BitReader::new(&data);
    let (has_type, has_cbp) = decode_modb(&mut reader);
    assert!(!has_type);
    assert!(!has_cbp);

    // MODB = "01" -> has mb_type, no cbp
    let data = [0x40]; // 0100 0000
    let mut reader = BitReader::new(&data);
    let (has_type, has_cbp) = decode_modb(&mut reader);
    assert!(has_type);
    assert!(!has_cbp);

    // MODB = "00" -> has both
    let data = [0x00]; // 0000 0000
    let mut reader = BitReader::new(&data);
    let (has_type, has_cbp) = decode_modb(&mut reader);
    assert!(has_type);
    assert!(has_cbp);

    // B MB type: "1" -> Direct
    let data = [0x80];
    let mut reader = BitReader::new(&data);
    assert_eq!(decode_b_mb_type(&mut reader), BframeMbMode::Direct);

    // B MB type: "01" -> Interpolate
    let data = [0x40];
    let mut reader = BitReader::new(&data);
    assert_eq!(decode_b_mb_type(&mut reader), BframeMbMode::Interpolate);

    // DBQUANT: "0" -> 0
    let data = [0x00];
    let mut reader = BitReader::new(&data);
    assert_eq!(decode_dbquant(&mut reader), 0);

    // DBQUANT: "10" -> -2
    let data = [0x80];
    let mut reader = BitReader::new(&data);
    assert_eq!(decode_dbquant(&mut reader), -2);

    // DBQUANT: "11" -> +2
    let data = [0xC0];
    let mut reader = BitReader::new(&data);
    assert_eq!(decode_dbquant(&mut reader), 2);
}

#[test]
fn test_direct_mode_mv_computation() {
    let mut decoder = test_decoder(320, 240);
    decoder.time_pp = 3;
    decoder.time_bp = 1;

    // 设置共定位 MV
    let co_mv = MotionVector { x: 6, y: 9 };
    decoder.ref_mv_cache[0] = [co_mv; 4];

    let delta_mv = MotionVector::default();
    let (fwd, bwd) = decoder.compute_direct_mvs(0, delta_mv);

    // forward = TRB/TRD * co_mv = 1/3 * (6,9) = (2, 3)
    assert_eq!(fwd[0].x, 2);
    assert_eq!(fwd[0].y, 3);

    // backward = (TRB-TRD)/TRD * co_mv = -2/3 * (6,9) = (-4, -6)
    assert_eq!(bwd[0].x, -4);
    assert_eq!(bwd[0].y, -6);
}

#[test]
fn test_qpel_mc_full_pixel() {
    // 全像素位置 (dx=0, dy=0): 应直接返回参考像素
    let mut ref_frame = VideoFrame::new(16, 16, PixelFormat::Yuv420p);
    ref_frame.data[0] = vec![0u8; 16 * 16];
    ref_frame.data[1] = vec![128u8; 8 * 8];
    ref_frame.data[2] = vec![128u8; 8 * 8];
    ref_frame.linesize[0] = 16;
    ref_frame.linesize[1] = 8;
    ref_frame.linesize[2] = 8;
    ref_frame.data[0][5 * 16 + 5] = 200;

    // MV = (0, 0) in qpel units
    let val = Mpeg4Decoder::qpel_motion_compensation(&ref_frame, 0, 5, 5, 0, 0, 0);
    assert_eq!(val, 200);

    // MV = (4, 0) in qpel units = 1 full pixel right
    let val = Mpeg4Decoder::qpel_motion_compensation(&ref_frame, 0, 4, 5, 4, 0, 0);
    assert_eq!(val, 200);
}

#[test]
fn test_macroblock_info_modes() {
    assert_eq!(MacroblockInfo::MODE_INTER, 0);
    assert_eq!(MacroblockInfo::MODE_INTRA, 1);
    assert_eq!(MacroblockInfo::MODE_INTER4V, 2);
    assert_eq!(MacroblockInfo::MODE_NOT_CODED, 5);
}

#[test]
fn test_resync_marker_check() {
    // MPEG-4 resync marker 格式: stuffing (0 + 1...1 字节对齐) + prefix_length 零 + 1
    // prefix_length = 16 + vop_fcode

    // 测试 1: 字节对齐, fcode=0 (I-VOP), prefix_length=16
    // stuffing = 0x7F (01111111), marker = 16 zeros + 1
    let data = [0x7F, 0x00, 0x00, 0x80];
    let reader = BitReader::new(&data);
    assert!(Mpeg4Decoder::check_resync_marker(&reader, 0));

    // 测试 2: 字节对齐, fcode=1, prefix_length=17
    // stuffing = 0x7F, marker = 17 zeros + 1
    let data = [0x7F, 0x00, 0x00, 0x40];
    let reader = BitReader::new(&data);
    assert!(Mpeg4Decoder::check_resync_marker(&reader, 1));

    // 测试 3: 非字节对齐 (bit_offset=1), fcode=0
    // 前 1 位是前一个宏块的数据, 之后是 stuffing + marker
    // 从 bit_offset=1: 0111111 00000000 00000000 1
    let data = [0xBF, 0x00, 0x00, 0x80];
    let mut reader = BitReader::new(&data);
    reader.read_bits(1); // 消耗 1 位
    assert!(Mpeg4Decoder::check_resync_marker(&reader, 0));

    // 测试 4: 非 resync marker (无效数据)
    let data = [0x00, 0x01, 0x00];
    let reader = BitReader::new(&data);
    assert!(!Mpeg4Decoder::check_resync_marker(&reader, 0));

    // 测试 5: 非 resync marker (数据中没有正确的 stuffing 模式)
    let data = [0xFF, 0x00, 0x00, 0x80];
    let reader = BitReader::new(&data);
    assert!(!Mpeg4Decoder::check_resync_marker(&reader, 0));
}
