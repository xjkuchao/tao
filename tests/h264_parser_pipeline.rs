//! H.264 码流解析器集成测试

use tao_codec::parsers::h264::{
    NalUnit, NalUnitType, annex_b_to_avcc, avcc_to_annex_b, build_avcc_config, parse_avcc_config,
    parse_sps, split_annex_b, split_avcc,
};

// ============================================================
// NAL 分割与格式转换测试
// ============================================================

/// 构造典型的 H.264 Annex B 码流 (SPS + PPS + IDR)
fn build_typical_annex_b() -> Vec<u8> {
    let mut data = Vec::new();

    // SPS (4字节起始码)
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    data.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E, 0xAB, 0xCD]);

    // PPS (3字节起始码)
    data.extend_from_slice(&[0x00, 0x00, 0x01]);
    data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);

    // IDR 切片 (4字节起始码)
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    data.extend_from_slice(&[0x65, 0x88, 0x80, 0x40, 0x00, 0xFF, 0xFE]);

    // P 切片 (3字节起始码)
    data.extend_from_slice(&[0x00, 0x00, 0x01]);
    data.extend_from_slice(&[0x41, 0x9A, 0x01, 0x02, 0x03]);

    data
}

#[test]
fn test_annex_b_full_parse() {
    let data = build_typical_annex_b();
    let nalus = split_annex_b(&data);

    assert_eq!(nalus.len(), 4, "应该有 4 个 NAL 单元");
    assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
    assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    assert_eq!(nalus[2].nal_type, NalUnitType::SliceIdr);
    assert_eq!(nalus[3].nal_type, NalUnitType::Slice);

    // IDR 是关键帧
    assert!(nalus[2].nal_type.is_idr());
    assert!(nalus[2].nal_type.is_vcl());

    // P slice 不是关键帧
    assert!(!nalus[3].nal_type.is_idr());
    assert!(nalus[3].nal_type.is_vcl());
}

#[test]
fn test_annex_b_avcc_roundtrip_conversion() {
    let annexb = build_typical_annex_b();

    // Annex B → AVCC
    let avcc = annex_b_to_avcc(&annexb);
    let nalus_avcc = split_avcc(&avcc, 4);
    assert_eq!(nalus_avcc.len(), 4);

    // AVCC → Annex B
    let annexb2 = avcc_to_annex_b(&avcc, 4);
    let nalus2 = split_annex_b(&annexb2);
    assert_eq!(nalus2.len(), 4);

    // 验证 NAL 类型一致
    for (a, b) in nalus_avcc.iter().zip(nalus2.iter()) {
        assert_eq!(a.nal_type, b.nal_type);
        assert_eq!(a.data, b.data);
    }
}

#[test]
fn test_avcc_config_build_and_parse_roundtrip() {
    let sps_data = vec![0x67, 0x42, 0x00, 0x1E, 0xAB, 0xCD];
    let pps_data = vec![0x68, 0xCE, 0x38, 0x80];

    // 构建
    let config = build_avcc_config(
        std::slice::from_ref(&sps_data),
        std::slice::from_ref(&pps_data),
        4,
    )
    .unwrap();

    // 解析
    let parsed = parse_avcc_config(&config).unwrap();
    assert_eq!(parsed.length_size, 4);
    assert_eq!(parsed.sps_list.len(), 1);
    assert_eq!(parsed.pps_list.len(), 1);
    assert_eq!(parsed.sps_list[0], sps_data);
    assert_eq!(parsed.pps_list[0], pps_data);

    // 验证 config 的固定字段
    assert_eq!(config[0], 1); // configurationVersion
    assert_eq!(config[1], 0x42); // profile_idc
    assert_eq!(config[3], 0x1E); // level_idc
}

#[test]
fn test_ref_idc_extract() {
    // nal_ref_idc=3, type=7 (SPS): 0b0_11_00111 = 0x67
    let nalu = NalUnit::parse(&[0x67, 0x42]).unwrap();
    assert_eq!(nalu.ref_idc, 3);

    // nal_ref_idc=0, type=6 (SEI): 0b0_00_00110 = 0x06
    let nalu = NalUnit::parse(&[0x06, 0xAA]).unwrap();
    assert_eq!(nalu.ref_idc, 0);

    // nal_ref_idc=2, type=1 (Slice): 0b0_10_00001 = 0x41
    let nalu = NalUnit::parse(&[0x41, 0xBB]).unwrap();
    assert_eq!(nalu.ref_idc, 2);
}

// ============================================================
// SPS 解析集成测试
// ============================================================

/// 构造 Baseline Profile SPS 的 RBSP 数据 (使用位流编码)
fn build_sps_rbsp(width: u32, height: u32) -> Vec<u8> {
    let mut bits = Vec::new();

    // profile_idc=66 (Baseline)
    push_u8(&mut bits, 66);
    // constraint_set_flags=0xC0
    push_u8(&mut bits, 0xC0);
    // level_idc=31
    push_u8(&mut bits, 31);

    // sps_id=0
    write_ue(&mut bits, 0);
    // log2_max_frame_num_minus4=0
    write_ue(&mut bits, 0);
    // pic_order_cnt_type=0
    write_ue(&mut bits, 0);
    // log2_max_pic_order_cnt_lsb_minus4=0
    write_ue(&mut bits, 0);
    // max_num_ref_frames=4
    write_ue(&mut bits, 4);
    // gaps=0
    bits.push(false);

    let mbs_w = width.div_ceil(16);
    let mbs_h = height.div_ceil(16);
    write_ue(&mut bits, mbs_w - 1);
    write_ue(&mut bits, mbs_h - 1);
    // frame_mbs_only=1
    bits.push(true);
    // direct_8x8=0
    bits.push(false);

    // Cropping
    let raw_w = mbs_w * 16;
    let raw_h = mbs_h * 16;
    if raw_w != width || raw_h != height {
        bits.push(true);
        write_ue(&mut bits, 0);
        write_ue(&mut bits, (raw_w - width) / 2);
        write_ue(&mut bits, 0);
        write_ue(&mut bits, (raw_h - height) / 2);
    } else {
        bits.push(false);
    }

    // VUI=0
    bits.push(false);

    bits_to_bytes(&bits)
}

fn push_u8(bits: &mut Vec<bool>, val: u8) {
    for i in (0..8).rev() {
        bits.push(((val >> i) & 1) != 0);
    }
}

fn write_ue(bits: &mut Vec<bool>, val: u32) {
    if val == 0 {
        bits.push(true);
        return;
    }
    let code = val + 1;
    let n = 32 - code.leading_zeros();
    for _ in 0..n - 1 {
        bits.push(false);
    }
    for i in (0..n).rev() {
        bits.push(((code >> i) & 1) != 0);
    }
}

fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            if bit {
                byte |= 1 << (7 - i);
            }
        }
        bytes.push(byte);
    }
    bytes
}

#[test]
fn test_sps_common_resolution_1920x1080() {
    let rbsp = build_sps_rbsp(1920, 1080);
    let sps = parse_sps(&rbsp).unwrap();

    assert_eq!(sps.profile_idc, 66);
    assert_eq!(sps.level_idc, 31);
    assert_eq!(sps.width, 1920);
    assert_eq!(sps.height, 1080);
    assert!(sps.frame_mbs_only);
    assert_eq!(sps.chroma_format_idc, 1); // 4:2:0
    assert_eq!(sps.max_num_ref_frames, 4);
}

#[test]
fn test_sps_common_resolution_1280x720() {
    let rbsp = build_sps_rbsp(1280, 720);
    let sps = parse_sps(&rbsp).unwrap();

    assert_eq!(sps.width, 1280);
    assert_eq!(sps.height, 720);
    // 720 是 16 的整数倍, 不需要 cropping
    assert_eq!(sps.crop_top, 0);
    assert_eq!(sps.crop_bottom, 0);
}

#[test]
fn test_sps_non_16_aligned_resolution_640x480() {
    let rbsp = build_sps_rbsp(640, 480);
    let sps = parse_sps(&rbsp).unwrap();

    assert_eq!(sps.width, 640);
    assert_eq!(sps.height, 480);
}

#[test]
fn test_sps_need_cropping_1920x1080() {
    let rbsp = build_sps_rbsp(1920, 1080);
    let sps = parse_sps(&rbsp).unwrap();

    // 1080 不是 16 的整数倍: ceil(1080/16)=68, 68*16=1088
    // crop_bottom = (1088-1080)/2 = 4
    assert_eq!(sps.pic_height_in_map_units, 68);
    assert_eq!(sps.crop_bottom, 4);
    assert_eq!(sps.width, 1920);
    assert_eq!(sps.height, 1080);
}

#[test]
fn test_full_pipeline_annex_b_extract_sps_parse_params() {
    // 构造 Annex B 码流, 包含可解析的 SPS
    let sps_rbsp = build_sps_rbsp(1920, 1080);

    // 构造 SPS NAL (0x67 header + RBSP)
    let mut sps_nal = vec![0x67]; // forbidden=0, ref_idc=3, type=7
    sps_nal.extend_from_slice(&sps_rbsp);

    // PPS NAL
    let pps_nal = vec![0x68, 0xCE, 0x38, 0x80];

    // 组装 Annex B
    let mut annexb = Vec::new();
    annexb.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    annexb.extend_from_slice(&sps_nal);
    annexb.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    annexb.extend_from_slice(&pps_nal);

    // 分割 NAL
    let nalus = split_annex_b(&annexb);
    assert_eq!(nalus.len(), 2);

    // 找到 SPS
    let sps_nalu = nalus.iter().find(|n| n.nal_type == NalUnitType::Sps);
    assert!(sps_nalu.is_some());

    // 提取 RBSP 并解析
    let rbsp = sps_nalu.unwrap().rbsp();
    let sps = parse_sps(&rbsp).unwrap();

    assert_eq!(sps.width, 1920);
    assert_eq!(sps.height, 1080);
    assert_eq!(sps.profile_idc, 66);
}
