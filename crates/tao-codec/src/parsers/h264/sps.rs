//! H.264 SPS (Sequence Parameter Set) 解析器.
//!
//! SPS 包含编码视频序列的全局参数, 包括:
//! - Profile / Level (编码规格)
//! - 图像宽度和高度 (以宏块为单位, 需要 cropping 调整)
//! - 色度格式 (chroma_format_idc)
//! - 帧率信息 (通过 VUI timing_info)
//! - 参考帧数量等
//!
//! # Exp-Golomb 编码
//!
//! SPS 中大量使用 Exp-Golomb 可变长编码:
//! - `ue(v)`: 无符号 Exp-Golomb
//! - `se(v)`: 有符号 Exp-Golomb

use tao_core::bitreader::BitReader;
use tao_core::{Rational, TaoError, TaoResult};

/// SPS 解析结果
#[derive(Debug, Clone)]
pub struct Sps {
    /// profile_idc (编码规格, 如 66=Baseline, 77=Main, 100=High)
    pub profile_idc: u8,
    /// constraint_set 标志位
    pub constraint_set_flags: u8,
    /// level_idc (编码级别, 如 30=3.0, 41=4.1)
    pub level_idc: u8,
    /// SPS ID (seq_parameter_set_id)
    pub sps_id: u32,
    /// 色度格式 (0=单色, 1=4:2:0, 2=4:2:2, 3=4:4:4)
    pub chroma_format_idc: u32,
    /// 亮度位深 (通常 8)
    pub bit_depth_luma: u32,
    /// 色度位深 (通常 8)
    pub bit_depth_chroma: u32,
    /// 最大参考帧数
    pub max_num_ref_frames: u32,
    /// 图像宽度 (像素, 已应用 cropping)
    pub width: u32,
    /// 图像高度 (像素, 已应用 cropping)
    pub height: u32,
    /// 是否为帧编码 (非场编码)
    pub frame_mbs_only: bool,
    /// 是否存在 VUI 参数
    pub vui_present: bool,
    /// 帧率 (如果 VUI 中有 timing_info)
    pub fps: Option<Rational>,
    /// SAR (Sample Aspect Ratio, 像素宽高比)
    pub sar: Rational,
    /// pic_width_in_mbs_minus1
    pub pic_width_in_mbs: u32,
    /// pic_height_in_map_units_minus1
    pub pic_height_in_map_units: u32,
    /// cropping 偏移
    pub crop_left: u32,
    /// cropping 偏移
    pub crop_right: u32,
    /// cropping 偏移
    pub crop_top: u32,
    /// cropping 偏移
    pub crop_bottom: u32,
    /// log2(max_frame_num) = log2_max_frame_num_minus4 + 4
    pub log2_max_frame_num: u32,
    /// 图像顺序计数类型 (0, 1, 2)
    pub poc_type: u32,
    /// log2(max_pic_order_cnt_lsb) = log2_max_pic_order_cnt_lsb_minus4 + 4 (仅 poc_type==0)
    pub log2_max_poc_lsb: u32,
}

/// 预定义的 SAR 表 (ITU-T H.264 表 E-1)
const SAR_TABLE: [(u32, u32); 17] = [
    (0, 1),    // 0: 未指定
    (1, 1),    // 1: 1:1
    (12, 11),  // 2: 12:11
    (10, 11),  // 3: 10:11
    (16, 11),  // 4: 16:11
    (40, 33),  // 5: 40:33
    (24, 11),  // 6: 24:11
    (20, 11),  // 7: 20:11
    (32, 11),  // 8: 32:11
    (80, 33),  // 9: 80:33
    (18, 11),  // 10: 18:11
    (15, 11),  // 11: 15:11
    (64, 33),  // 12: 64:33
    (160, 99), // 13: 160:99
    (4, 3),    // 14: 4:3
    (3, 2),    // 15: 3:2
    (2, 1),    // 16: 2:1
];

/// 从 RBSP 数据解析 SPS
pub fn parse_sps(rbsp: &[u8]) -> TaoResult<Sps> {
    if rbsp.len() < 3 {
        return Err(TaoError::InvalidData("H.264: SPS RBSP 太短".into()));
    }

    let mut br = BitReader::new(rbsp);

    // profile_idc (8 bits)
    let profile_idc = br.read_bits(8)? as u8;
    // constraint_set flags (8 bits)
    let constraint_set_flags = br.read_bits(8)? as u8;
    // level_idc (8 bits)
    let level_idc = br.read_bits(8)? as u8;
    // seq_parameter_set_id
    let sps_id = read_ue(&mut br)?;

    let mut chroma_format_idc = 1; // 默认 4:2:0
    let mut bit_depth_luma = 8;
    let mut bit_depth_chroma = 8;

    // High profile 及以上有额外字段
    if is_high_profile(profile_idc) {
        chroma_format_idc = read_ue(&mut br)?;
        if chroma_format_idc == 3 {
            br.skip_bits(1)?; // separate_colour_plane_flag
        }
        bit_depth_luma = read_ue(&mut br)? + 8;
        bit_depth_chroma = read_ue(&mut br)? + 8;
        br.skip_bits(1)?; // qpprime_y_zero_transform_bypass_flag

        // seq_scaling_matrix_present_flag
        let scaling_present = br.read_bit()?;
        if scaling_present == 1 {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            skip_scaling_lists(&mut br, count)?;
        }
    }

    // log2_max_frame_num_minus4
    let log2_max_frame_num = read_ue(&mut br)? + 4;
    // pic_order_cnt_type
    let poc_type = read_ue(&mut br)?;
    let mut log2_max_poc_lsb = 0u32;
    match poc_type {
        0 => {
            log2_max_poc_lsb = read_ue(&mut br)? + 4;
        }
        1 => {
            br.skip_bits(1)?; // delta_pic_order_always_zero_flag
            let _offset_for_non_ref = read_se(&mut br)?;
            let _offset_for_top = read_se(&mut br)?;
            let num_ref_in_poc = read_ue(&mut br)?;
            for _ in 0..num_ref_in_poc {
                let _offset = read_se(&mut br)?;
            }
        }
        _ => {} // poc_type == 2: 无额外字段
    }

    let max_num_ref_frames = read_ue(&mut br)?;
    let _gaps_in_frame_num_allowed = br.read_bit()?;

    // 图像尺寸 (宏块单位)
    let pic_width_in_mbs = read_ue(&mut br)? + 1;
    let pic_height_in_map_units = read_ue(&mut br)? + 1;

    // frame_mbs_only_flag
    let frame_mbs_only = br.read_bit()? == 1;
    if !frame_mbs_only {
        br.skip_bits(1)?; // mb_adaptive_frame_field_flag
    }

    // direct_8x8_inference_flag
    br.skip_bits(1)?;

    // Cropping
    let mut crop_left = 0u32;
    let mut crop_right = 0u32;
    let mut crop_top = 0u32;
    let mut crop_bottom = 0u32;

    let cropping_flag = br.read_bit()?;
    if cropping_flag == 1 {
        crop_left = read_ue(&mut br)?;
        crop_right = read_ue(&mut br)?;
        crop_top = read_ue(&mut br)?;
        crop_bottom = read_ue(&mut br)?;
    }

    // 计算像素尺寸
    let (crop_unit_x, crop_unit_y) = cropping_unit(chroma_format_idc, frame_mbs_only);
    let width = pic_width_in_mbs * 16 - (crop_left + crop_right) * crop_unit_x;
    let height = pic_height_in_map_units * 16 * (if frame_mbs_only { 1 } else { 2 })
        - (crop_top + crop_bottom) * crop_unit_y;

    // VUI 参数
    let mut vui_present = false;
    let mut fps = None;
    let mut sar = Rational::new(1, 1);

    let vui_flag = br.read_bit()?;
    if vui_flag == 1 {
        vui_present = true;
        let (parsed_sar, parsed_fps) = parse_vui(&mut br)?;
        sar = parsed_sar;
        fps = parsed_fps;
    }

    Ok(Sps {
        profile_idc,
        constraint_set_flags,
        level_idc,
        sps_id,
        chroma_format_idc,
        bit_depth_luma,
        bit_depth_chroma,
        max_num_ref_frames,
        width,
        height,
        frame_mbs_only,
        vui_present,
        fps,
        sar,
        pic_width_in_mbs,
        pic_height_in_map_units,
        crop_left,
        crop_right,
        crop_top,
        crop_bottom,
        log2_max_frame_num,
        poc_type,
        log2_max_poc_lsb,
    })
}

// ============================================================
// Exp-Golomb 编码读取
// ============================================================

/// 读取无符号 Exp-Golomb 编码值 ue(v)
fn read_ue(br: &mut BitReader) -> TaoResult<u32> {
    let mut leading_zeros = 0u32;
    loop {
        let bit = br.read_bit()?;
        if bit == 1 {
            break;
        }
        leading_zeros += 1;
        if leading_zeros > 31 {
            return Err(TaoError::InvalidData("H.264: Exp-Golomb 前导零过多".into()));
        }
    }

    if leading_zeros == 0 {
        return Ok(0);
    }

    let suffix = br.read_bits(leading_zeros)?;
    Ok((1 << leading_zeros) - 1 + suffix)
}

/// 读取有符号 Exp-Golomb 编码值 se(v)
fn read_se(br: &mut BitReader) -> TaoResult<i32> {
    let code = read_ue(br)?;
    // 映射: 0→0, 1→1, 2→-1, 3→2, 4→-2, ...
    let value = code.div_ceil(2) as i32;
    if code & 1 == 0 { Ok(-value) } else { Ok(value) }
}

// ============================================================
// 辅助函数
// ============================================================

/// 是否为 High Profile 或更高
fn is_high_profile(profile_idc: u8) -> bool {
    matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134
    )
}

/// 获取 cropping 单位
fn cropping_unit(chroma_format_idc: u32, frame_mbs_only: bool) -> (u32, u32) {
    let sub_width = match chroma_format_idc {
        0 | 3 => 1,
        _ => 2, // 4:2:0 和 4:2:2
    };
    let sub_height = match chroma_format_idc {
        0 | 2 | 3 => 1,
        _ => 2, // 4:2:0
    };
    let height_mult = if frame_mbs_only { 1 } else { 2 };

    (sub_width, sub_height * height_mult)
}

/// 跳过 scaling list
fn skip_scaling_lists(br: &mut BitReader, count: usize) -> TaoResult<()> {
    for i in 0..count {
        let present = br.read_bit()?;
        if present == 1 {
            let size = if i < 6 { 16 } else { 64 };
            skip_scaling_list(br, size)?;
        }
    }
    Ok(())
}

/// 跳过单个 scaling list
fn skip_scaling_list(br: &mut BitReader, size: usize) -> TaoResult<()> {
    let mut last_scale: i32 = 8;
    let mut next_scale: i32 = 8;

    for _ in 0..size {
        if next_scale != 0 {
            let delta = read_se(br)?;
            next_scale = (last_scale + delta + 256) % 256;
        }
        last_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
    }
    Ok(())
}

/// 解析 VUI 参数 (部分)
///
/// 返回 (SAR, fps)
fn parse_vui(br: &mut BitReader) -> TaoResult<(Rational, Option<Rational>)> {
    let mut sar = Rational::new(1, 1);

    // aspect_ratio_info_present_flag
    let ar_present = br.read_bit()?;
    if ar_present == 1 {
        let ar_idc = br.read_bits(8)? as usize;
        if ar_idc == 255 {
            // Extended_SAR
            let sar_w = br.read_bits(16)?;
            let sar_h = br.read_bits(16)?;
            if sar_w > 0 && sar_h > 0 {
                sar = Rational::new(sar_w as i32, sar_h as i32);
            }
        } else if ar_idc < SAR_TABLE.len() {
            let (w, h) = SAR_TABLE[ar_idc];
            if w > 0 && h > 0 {
                sar = Rational::new(w as i32, h as i32);
            }
        }
    }

    // overscan_info_present_flag
    if br.read_bit()? == 1 {
        br.skip_bits(1)?; // overscan_appropriate_flag
    }

    // video_signal_type_present_flag
    if br.read_bit()? == 1 {
        br.skip_bits(3)?; // video_format
        br.skip_bits(1)?; // video_full_range_flag
        // colour_description_present_flag
        if br.read_bit()? == 1 {
            br.skip_bits(8)?; // colour_primaries
            br.skip_bits(8)?; // transfer_characteristics
            br.skip_bits(8)?; // matrix_coefficients
        }
    }

    // chroma_loc_info_present_flag
    if br.read_bit()? == 1 {
        let _chroma_top = read_ue(br)?;
        let _chroma_bottom = read_ue(br)?;
    }

    // timing_info_present_flag
    let mut fps = None;
    if br.read_bit()? == 1 {
        let num_units = br.read_bits(32)?;
        let time_scale = br.read_bits(32)?;
        let fixed_rate = br.read_bit()?;

        if num_units > 0 && time_scale > 0 {
            // H.264 定义: fps = time_scale / (2 * num_units_in_tick)
            // fixed_frame_rate_flag 表示每个 AU 都是固定帧率
            let _ = fixed_rate;
            fps = Some(Rational::new(time_scale as i32, (num_units * 2) as i32));
        }
    }

    Ok((sar, fps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exp_golomb_ue() {
        // ue(v) 编码: 1 → 0, 010 → 1, 011 → 2, 00100 → 3
        // 0 → "1"
        let data = [0b10000000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_ue(&mut br).unwrap(), 0);

        // 1 → "010"
        let data = [0b01000000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_ue(&mut br).unwrap(), 1);

        // 2 → "011"
        let data = [0b01100000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_ue(&mut br).unwrap(), 2);

        // 3 → "00100"
        let data = [0b00100000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_ue(&mut br).unwrap(), 3);

        // 7 → "00010 00" = 7
        let data = [0b00010000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_ue(&mut br).unwrap(), 7);
    }

    #[test]
    fn test_exp_golomb_se() {
        // se(v): 0→0, 1→1, 2→-1, 3→2, 4→-2
        // ue=0 → se=0
        let data = [0b10000000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_se(&mut br).unwrap(), 0);

        // ue=1 → se=1
        let data = [0b01000000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_se(&mut br).unwrap(), 1);

        // ue=2 → se=-1
        let data = [0b01100000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_se(&mut br).unwrap(), -1);

        // ue=3 → se=2
        let data = [0b00100000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_se(&mut br).unwrap(), 2);

        // ue=4 → se=-2: "00101"
        let data = [0b00101000];
        let mut br = BitReader::new(&data);
        assert_eq!(read_se(&mut br).unwrap(), -2);
    }

    #[test]
    fn test_sps_baseline_profile() {
        // 构造最小的 Baseline Profile SPS RBSP
        // profile_idc=66 (Baseline), constraint=0xC0, level=30
        // 不含 High profile 扩展字段
        let rbsp = build_test_sps_rbsp(66, 0xC0, 30, 1920, 1080, false);
        let sps = parse_sps(&rbsp).unwrap();

        assert_eq!(sps.profile_idc, 66);
        assert_eq!(sps.level_idc, 30);
        assert_eq!(sps.width, 1920);
        assert_eq!(sps.height, 1080);
        assert_eq!(sps.chroma_format_idc, 1); // 默认 4:2:0
    }

    #[test]
    fn test_sps_high_profile() {
        let rbsp = build_test_sps_rbsp(100, 0x00, 41, 1280, 720, true);
        let sps = parse_sps(&rbsp).unwrap();

        assert_eq!(sps.profile_idc, 100);
        assert_eq!(sps.level_idc, 41);
        assert_eq!(sps.width, 1280);
        assert_eq!(sps.height, 720);
    }

    #[test]
    fn test_sps_帧率提取() {
        // 构造带 VUI timing_info 的 SPS
        let rbsp = build_test_sps_with_vui(66, 30, 1920, 1080, 1001, 60000);
        let sps = parse_sps(&rbsp).unwrap();

        assert!(sps.vui_present);
        let fps = sps.fps.unwrap();
        // time_scale=60000, num_units=1001 → fps=60000/2002≈29.97
        assert_eq!(fps.num, 60000);
        assert_eq!(fps.den, 2002);
    }

    #[test]
    fn test_sps_rbsp太短() {
        assert!(parse_sps(&[0x42]).is_err());
    }

    // ============================================================
    // 测试辅助: 构造 SPS RBSP 数据
    // ============================================================

    /// 写入 ue(v)
    fn write_ue(bits: &mut Vec<bool>, val: u32) {
        if val == 0 {
            bits.push(true); // "1"
            return;
        }
        let code = val + 1;
        let num_bits = 32 - code.leading_zeros();
        // 前导零
        for _ in 0..num_bits - 1 {
            bits.push(false);
        }
        // code 本身
        for i in (0..num_bits).rev() {
            bits.push(((code >> i) & 1) != 0);
        }
    }

    /// 将 bit 向量转为字节
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

    /// 构造测试用 SPS RBSP (无 VUI)
    fn build_test_sps_rbsp(
        profile: u8,
        constraints: u8,
        level: u8,
        width: u32,
        height: u32,
        high_profile: bool,
    ) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc (8 bits)
        for i in (0..8).rev() {
            bits.push(((profile >> i) & 1) != 0);
        }
        // constraint_set_flags (8 bits)
        for i in (0..8).rev() {
            bits.push(((constraints >> i) & 1) != 0);
        }
        // level_idc (8 bits)
        for i in (0..8).rev() {
            bits.push(((level >> i) & 1) != 0);
        }

        // sps_id = 0
        write_ue(&mut bits, 0);

        if high_profile {
            // chroma_format_idc = 1 (4:2:0)
            write_ue(&mut bits, 1);
            // bit_depth_luma_minus8 = 0
            write_ue(&mut bits, 0);
            // bit_depth_chroma_minus8 = 0
            write_ue(&mut bits, 0);
            // qpprime_y_zero_transform_bypass_flag = 0
            bits.push(false);
            // seq_scaling_matrix_present_flag = 0
            bits.push(false);
        }

        // log2_max_frame_num_minus4 = 0
        write_ue(&mut bits, 0);
        // pic_order_cnt_type = 0
        write_ue(&mut bits, 0);
        // log2_max_pic_order_cnt_lsb_minus4 = 0
        write_ue(&mut bits, 0);
        // max_num_ref_frames = 4
        write_ue(&mut bits, 4);
        // gaps_in_frame_num_value_allowed_flag = 0
        bits.push(false);

        // pic_width_in_mbs_minus1
        let mbs_w = width.div_ceil(16);
        write_ue(&mut bits, mbs_w - 1);
        // pic_height_in_map_units_minus1
        let mbs_h = height.div_ceil(16);
        write_ue(&mut bits, mbs_h - 1);

        // frame_mbs_only_flag = 1
        bits.push(true);
        // direct_8x8_inference_flag = 0
        bits.push(false);

        // Cropping
        let raw_w = mbs_w * 16;
        let raw_h = mbs_h * 16;
        if raw_w != width || raw_h != height {
            bits.push(true); // frame_cropping_flag
            write_ue(&mut bits, 0); // left
            write_ue(&mut bits, (raw_w - width) / 2); // right
            write_ue(&mut bits, 0); // top
            write_ue(&mut bits, (raw_h - height) / 2); // bottom
        } else {
            bits.push(false);
        }

        // vui_parameters_present_flag = 0
        bits.push(false);

        bits_to_bytes(&bits)
    }

    /// 构造带 VUI timing_info 的 SPS RBSP
    fn build_test_sps_with_vui(
        profile: u8,
        level: u8,
        width: u32,
        height: u32,
        num_units: u32,
        time_scale: u32,
    ) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile/constraints/level
        for i in (0..8).rev() {
            bits.push(((profile >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8)); // constraints
        for i in (0..8).rev() {
            bits.push(((level >> i) & 1) != 0);
        }

        // sps_id=0, log2_max_frame_num_minus4=0, poc_type=0,
        // log2_max_poc_lsb_minus4=0
        write_ue(&mut bits, 0);
        write_ue(&mut bits, 0);
        write_ue(&mut bits, 0);
        write_ue(&mut bits, 0);
        // max_num_ref_frames=4
        write_ue(&mut bits, 4);
        bits.push(false); // gaps

        let mbs_w = width.div_ceil(16);
        let mbs_h = height.div_ceil(16);
        write_ue(&mut bits, mbs_w - 1);
        write_ue(&mut bits, mbs_h - 1);
        bits.push(true); // frame_mbs_only
        bits.push(false); // direct_8x8

        // cropping
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

        // vui_parameters_present_flag = 1
        bits.push(true);

        // aspect_ratio_info_present_flag = 0
        bits.push(false);
        // overscan_info_present_flag = 0
        bits.push(false);
        // video_signal_type_present_flag = 0
        bits.push(false);
        // chroma_loc_info_present_flag = 0
        bits.push(false);

        // timing_info_present_flag = 1
        bits.push(true);
        // num_units_in_tick (32 bits)
        for i in (0..32).rev() {
            bits.push(((num_units >> i) & 1) != 0);
        }
        // time_scale (32 bits)
        for i in (0..32).rev() {
            bits.push(((time_scale >> i) & 1) != 0);
        }
        // fixed_frame_rate_flag = 1
        bits.push(true);

        bits_to_bytes(&bits)
    }
}
