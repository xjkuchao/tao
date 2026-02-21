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
    /// gaps_in_frame_num_value_allowed_flag.
    pub gaps_in_frame_num_value_allowed_flag: bool,
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
    /// `poc_type==1` 时的 delta_pic_order_always_zero_flag.
    pub delta_pic_order_always_zero_flag: bool,
    /// `poc_type==1` 时的 offset_for_non_ref_pic.
    pub offset_for_non_ref_pic: i32,
    /// `poc_type==1` 时的 offset_for_top_to_bottom_field.
    pub offset_for_top_to_bottom_field: i32,
    /// `poc_type==1` 时的 offset_for_ref_frame 列表.
    pub offset_for_ref_frame: Vec<i32>,
    /// 4x4 量化矩阵列表 (6 组, 已应用默认/回退规则).
    pub scaling_list_4x4: [[u8; 16]; 6],
    /// 8x8 量化矩阵列表 (4:2:0/4:2:2 为 2 组, 4:4:4 为 6 组, 已应用默认/回退规则).
    pub scaling_list_8x8: Vec<[u8; 64]>,
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

const DEFAULT_SCALING_4X4_INTRA: [u8; 16] = [
    6, 13, 20, 28, 13, 20, 28, 32, 20, 28, 32, 37, 28, 32, 37, 42,
];

const DEFAULT_SCALING_4X4_INTER: [u8; 16] = [
    10, 14, 20, 24, 14, 20, 24, 27, 20, 24, 27, 30, 24, 27, 30, 34,
];

const DEFAULT_SCALING_8X8_INTRA: [u8; 64] = [
    6, 10, 13, 16, 18, 23, 25, 27, 10, 11, 16, 18, 23, 25, 27, 29, 13, 16, 18, 23, 25, 27, 29, 31,
    16, 18, 23, 25, 27, 29, 31, 33, 18, 23, 25, 27, 29, 31, 33, 36, 23, 25, 27, 29, 31, 33, 36, 38,
    25, 27, 29, 31, 33, 36, 38, 40, 27, 29, 31, 33, 36, 38, 40, 42,
];

const DEFAULT_SCALING_8X8_INTER: [u8; 64] = [
    9, 13, 15, 17, 19, 21, 22, 24, 13, 13, 17, 19, 21, 22, 24, 25, 15, 17, 19, 21, 22, 24, 25, 27,
    17, 19, 21, 22, 24, 25, 27, 28, 19, 21, 22, 24, 25, 27, 28, 30, 21, 22, 24, 25, 27, 28, 30, 32,
    22, 24, 25, 27, 28, 30, 32, 33, 24, 25, 27, 28, 30, 32, 33, 35,
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
    if sps_id > 31 {
        return Err(TaoError::InvalidData(format!(
            "H.264: sps_id 超出范围, sps_id={}",
            sps_id
        )));
    }

    let mut chroma_format_idc = 1; // 默认 4:2:0
    let mut separate_colour_plane_flag = false;
    let mut bit_depth_luma = 8;
    let mut bit_depth_chroma = 8;
    let mut scaling_list_4x4 = default_scaling_lists_4x4();
    let mut scaling_list_8x8 = default_scaling_lists_8x8(chroma_format_idc);

    // High profile 及以上有额外字段
    if is_high_profile(profile_idc) {
        chroma_format_idc = read_ue(&mut br)?;
        if chroma_format_idc > 3 {
            return Err(TaoError::InvalidData(format!(
                "H.264: chroma_format_idc 非法, value={}",
                chroma_format_idc
            )));
        }
        if chroma_format_idc == 3 {
            separate_colour_plane_flag = br.read_bit()? == 1;
        }
        scaling_list_8x8 = default_scaling_lists_8x8(chroma_format_idc);
        bit_depth_luma = read_ue(&mut br)? + 8;
        bit_depth_chroma = read_ue(&mut br)? + 8;
        if !(8..=14).contains(&bit_depth_luma) {
            return Err(TaoError::InvalidData(format!(
                "H.264: bit_depth_luma 非法, value={}",
                bit_depth_luma
            )));
        }
        if !(8..=14).contains(&bit_depth_chroma) {
            return Err(TaoError::InvalidData(format!(
                "H.264: bit_depth_chroma 非法, value={}",
                bit_depth_chroma
            )));
        }
        br.skip_bits(1)?; // qpprime_y_zero_transform_bypass_flag

        // seq_scaling_matrix_present_flag
        let scaling_present = br.read_bit()?;
        if scaling_present == 1 {
            parse_seq_scaling_lists(
                &mut br,
                chroma_format_idc,
                &mut scaling_list_4x4,
                &mut scaling_list_8x8,
            )?;
        }
    }

    // log2_max_frame_num_minus4
    let log2_max_frame_num_minus4 = read_ue(&mut br)?;
    if log2_max_frame_num_minus4 > 12 {
        return Err(TaoError::InvalidData(format!(
            "H.264: log2_max_frame_num_minus4 超出范围, value={}",
            log2_max_frame_num_minus4
        )));
    }
    let log2_max_frame_num = log2_max_frame_num_minus4 + 4;

    // pic_order_cnt_type
    let poc_type = read_ue(&mut br)?;
    if poc_type > 2 {
        return Err(TaoError::InvalidData(format!(
            "H.264: pic_order_cnt_type 非法, value={}",
            poc_type
        )));
    }
    let mut log2_max_poc_lsb = 0u32;
    let mut delta_pic_order_always_zero_flag = false;
    let mut offset_for_non_ref_pic = 0i32;
    let mut offset_for_top_to_bottom_field = 0i32;
    let mut offset_for_ref_frame = Vec::new();
    match poc_type {
        0 => {
            let log2_max_poc_lsb_minus4 = read_ue(&mut br)?;
            if log2_max_poc_lsb_minus4 > 12 {
                return Err(TaoError::InvalidData(format!(
                    "H.264: log2_max_pic_order_cnt_lsb_minus4 超出范围, value={}",
                    log2_max_poc_lsb_minus4
                )));
            }
            log2_max_poc_lsb = log2_max_poc_lsb_minus4 + 4;
        }
        1 => {
            delta_pic_order_always_zero_flag = br.read_bit()? == 1;
            offset_for_non_ref_pic = read_se(&mut br)?;
            offset_for_top_to_bottom_field = read_se(&mut br)?;
            let num_ref_in_poc = read_ue(&mut br)?;
            if num_ref_in_poc > 255 {
                return Err(TaoError::InvalidData(format!(
                    "H.264: num_ref_frames_in_pic_order_cnt_cycle 超出范围, value={}",
                    num_ref_in_poc
                )));
            }
            for _ in 0..num_ref_in_poc {
                let offset = read_se(&mut br)?;
                offset_for_ref_frame.push(offset);
            }
        }
        _ => {} // poc_type == 2: 无额外字段
    }

    let max_num_ref_frames = read_ue(&mut br)?;
    if max_num_ref_frames > 16 {
        return Err(TaoError::InvalidData(format!(
            "H.264: max_num_ref_frames 超出范围, value={}",
            max_num_ref_frames
        )));
    }
    let gaps_in_frame_num_value_allowed_flag = br.read_bit()? == 1;

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
    let chroma_array_type = if separate_colour_plane_flag {
        0
    } else {
        chroma_format_idc
    };
    let (crop_unit_x, crop_unit_y) = cropping_unit(chroma_array_type, frame_mbs_only);
    let raw_width = pic_width_in_mbs
        .checked_mul(16)
        .ok_or_else(|| TaoError::InvalidData("H.264: 计算宽度时发生溢出".into()))?;
    let frame_height_in_mbs = pic_height_in_map_units
        .checked_mul(if frame_mbs_only { 1 } else { 2 })
        .ok_or_else(|| TaoError::InvalidData("H.264: 计算高度时发生溢出".into()))?;
    let raw_height = frame_height_in_mbs
        .checked_mul(16)
        .ok_or_else(|| TaoError::InvalidData("H.264: 计算高度时发生溢出".into()))?;
    let crop_x = crop_left
        .checked_add(crop_right)
        .and_then(|v| v.checked_mul(crop_unit_x))
        .ok_or_else(|| TaoError::InvalidData("H.264: 计算水平裁剪时发生溢出".into()))?;
    let crop_y = crop_top
        .checked_add(crop_bottom)
        .and_then(|v| v.checked_mul(crop_unit_y))
        .ok_or_else(|| TaoError::InvalidData("H.264: 计算垂直裁剪时发生溢出".into()))?;
    if crop_x >= raw_width || crop_y >= raw_height {
        return Err(TaoError::InvalidData(format!(
            "H.264: 裁剪参数非法, raw={}x{}, crop_x={}, crop_y={}",
            raw_width, raw_height, crop_x, crop_y
        )));
    }
    let width = raw_width - crop_x;
    let height = raw_height - crop_y;
    if width == 0 || height == 0 {
        return Err(TaoError::InvalidData(format!(
            "H.264: 图像尺寸非法, width={}, height={}",
            width, height
        )));
    }

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
        gaps_in_frame_num_value_allowed_flag,
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
        delta_pic_order_always_zero_flag,
        offset_for_non_ref_pic,
        offset_for_top_to_bottom_field,
        offset_for_ref_frame,
        scaling_list_4x4,
        scaling_list_8x8,
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

fn default_scaling_lists_4x4() -> [[u8; 16]; 6] {
    [
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTER,
        DEFAULT_SCALING_4X4_INTER,
        DEFAULT_SCALING_4X4_INTER,
    ]
}

fn default_scaling_lists_8x8(chroma_format_idc: u32) -> Vec<[u8; 64]> {
    let list_count = if chroma_format_idc == 3 { 6 } else { 2 };
    let mut lists = Vec::with_capacity(list_count);
    for idx in 0..list_count {
        lists.push(default_scaling_list_8x8_by_idx(idx));
    }
    lists
}

fn default_scaling_list_4x4_by_idx(idx: usize) -> [u8; 16] {
    if idx < 3 {
        DEFAULT_SCALING_4X4_INTRA
    } else {
        DEFAULT_SCALING_4X4_INTER
    }
}

fn default_scaling_list_8x8_by_idx(idx: usize) -> [u8; 64] {
    if idx % 2 == 0 {
        DEFAULT_SCALING_8X8_INTRA
    } else {
        DEFAULT_SCALING_8X8_INTER
    }
}

fn parse_seq_scaling_lists(
    br: &mut BitReader,
    chroma_format_idc: u32,
    scaling_list_4x4: &mut [[u8; 16]; 6],
    scaling_list_8x8: &mut [[u8; 64]],
) -> TaoResult<()> {
    let list_count = if chroma_format_idc != 3 { 8 } else { 12 };
    for list_idx in 0..list_count {
        let present = br.read_bit()?;
        if present == 0 {
            apply_absent_scaling_list_fallback(list_idx, scaling_list_4x4, scaling_list_8x8)?;
            continue;
        }

        if list_idx < 6 {
            let (list, use_default) = parse_scaling_list_4x4(br)?;
            scaling_list_4x4[list_idx] = if use_default {
                default_scaling_list_4x4_by_idx(list_idx)
            } else {
                list
            };
        } else {
            let idx8 = list_idx - 6;
            let (list, use_default) = parse_scaling_list_8x8(br)?;
            scaling_list_8x8[idx8] = if use_default {
                default_scaling_list_8x8_by_idx(idx8)
            } else {
                list
            };
        }
    }
    Ok(())
}

fn apply_absent_scaling_list_fallback(
    list_idx: usize,
    scaling_list_4x4: &mut [[u8; 16]; 6],
    scaling_list_8x8: &mut [[u8; 64]],
) -> TaoResult<()> {
    if list_idx < 6 {
        scaling_list_4x4[list_idx] = if list_idx == 0 || list_idx == 3 {
            default_scaling_list_4x4_by_idx(list_idx)
        } else {
            scaling_list_4x4[list_idx - 1]
        };
        return Ok(());
    }

    let idx8 = list_idx - 6;
    if idx8 >= scaling_list_8x8.len() {
        return Err(TaoError::InvalidData(format!(
            "H.264: SPS scaling_list_8x8 索引越界, idx={}",
            idx8
        )));
    }
    scaling_list_8x8[idx8] = if idx8 < 2 {
        default_scaling_list_8x8_by_idx(idx8)
    } else {
        scaling_list_8x8[idx8 - 2]
    };
    Ok(())
}

fn parse_scaling_list_4x4(br: &mut BitReader) -> TaoResult<([u8; 16], bool)> {
    let mut list = [0u8; 16];
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    let mut use_default = false;
    for (idx, slot) in list.iter_mut().enumerate() {
        if next_scale != 0 {
            let delta_scale = read_se(br)?;
            let sum = i64::from(last_scale) + i64::from(delta_scale) + 256;
            next_scale = sum.rem_euclid(256) as i32;
            if idx == 0 && next_scale == 0 {
                use_default = true;
            }
        }
        let cur_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
        *slot = cur_scale as u8;
        last_scale = cur_scale;
    }
    Ok((list, use_default))
}

fn parse_scaling_list_8x8(br: &mut BitReader) -> TaoResult<([u8; 64], bool)> {
    let mut list = [0u8; 64];
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    let mut use_default = false;
    for (idx, slot) in list.iter_mut().enumerate() {
        if next_scale != 0 {
            let delta_scale = read_se(br)?;
            let sum = i64::from(last_scale) + i64::from(delta_scale) + 256;
            next_scale = sum.rem_euclid(256) as i32;
            if idx == 0 && next_scale == 0 {
                use_default = true;
            }
        }
        let cur_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
        *slot = cur_scale as u8;
        last_scale = cur_scale;
    }
    Ok((list, use_default))
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
            if sar_w == 0 || sar_h == 0 {
                return Err(TaoError::InvalidData(format!(
                    "H.264: VUI Extended_SAR 非法, sar_w={}, sar_h={}",
                    sar_w, sar_h
                )));
            }
            sar = Rational::new(sar_w as i32, sar_h as i32);
        } else if ar_idc < SAR_TABLE.len() {
            let (w, h) = SAR_TABLE[ar_idc];
            if w > 0 && h > 0 {
                sar = Rational::new(w as i32, h as i32);
            }
        } else {
            return Err(TaoError::InvalidData(format!(
                "H.264: VUI aspect_ratio_idc 非法, value={}",
                ar_idc
            )));
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

        if num_units == 0 {
            return Err(TaoError::InvalidData(
                "H.264: VUI num_units_in_tick 不能为 0".into(),
            ));
        }
        if time_scale == 0 {
            return Err(TaoError::InvalidData(
                "H.264: VUI time_scale 不能为 0".into(),
            ));
        }
        // H.264 定义: fps = time_scale / (2 * num_units_in_tick)
        // fixed_frame_rate_flag 表示每个 AU 都是固定帧率
        let _ = fixed_rate;
        fps = Some(Rational::new(time_scale as i32, (num_units * 2) as i32));
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
    fn test_sps_frame_rate_extract() {
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
    fn test_sps_rbsp_too_short() {
        assert!(parse_sps(&[0x42]).is_err());
    }

    #[test]
    fn test_sps_reject_sps_id_out_of_range() {
        let rbsp = build_test_sps_rbsp_custom(32, 0, 4);
        let err = parse_sps(&rbsp).expect_err("sps_id 超范围应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("sps_id"),
            "错误信息应包含 sps_id, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_invalid_poc_type() {
        let rbsp = build_test_sps_rbsp_custom(0, 3, 4);
        let err = parse_sps(&rbsp).expect_err("poc_type 非法应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("pic_order_cnt_type"),
            "错误信息应包含 pic_order_cnt_type, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_too_many_ref_frames() {
        let rbsp = build_test_sps_rbsp_custom(0, 0, 17);
        let err = parse_sps(&rbsp).expect_err("max_num_ref_frames 超范围应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("max_num_ref_frames"),
            "错误信息应包含 max_num_ref_frames, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_invalid_chroma_format_idc() {
        let rbsp = build_test_high_profile_sps_with_chroma(4);
        let err = parse_sps(&rbsp).expect_err("chroma_format_idc 非法应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("chroma_format_idc"),
            "错误信息应包含 chroma_format_idc, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_too_many_poc_cycle_offsets() {
        let rbsp = build_test_sps_poc_type1_with_cycle(256);
        let err = parse_sps(&rbsp).expect_err("num_ref_frames_in_pic_order_cnt_cycle 超范围应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("num_ref_frames_in_pic_order_cnt_cycle"),
            "错误信息应包含 num_ref_frames_in_pic_order_cnt_cycle, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_invalid_vui_aspect_ratio_idc() {
        let rbsp = build_test_sps_with_custom_vui(17, None, Some((1001, 60000)));
        let err = parse_sps(&rbsp).expect_err("非法 aspect_ratio_idc 应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("aspect_ratio_idc"),
            "错误信息应包含 aspect_ratio_idc, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_invalid_extended_sar() {
        let rbsp = build_test_sps_with_custom_vui(255, Some((0, 1)), Some((1001, 60000)));
        let err = parse_sps(&rbsp).expect_err("Extended_SAR 为 0 应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("Extended_SAR"),
            "错误信息应包含 Extended_SAR, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_zero_num_units_in_tick() {
        let rbsp = build_test_sps_with_custom_vui(1, None, Some((0, 60000)));
        let err = parse_sps(&rbsp).expect_err("num_units_in_tick=0 应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("num_units_in_tick"),
            "错误信息应包含 num_units_in_tick, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_reject_zero_time_scale() {
        let rbsp = build_test_sps_with_custom_vui(1, None, Some((1001, 0)));
        let err = parse_sps(&rbsp).expect_err("time_scale=0 应失败");
        let msg = format!("{}", err);
        assert!(
            msg.contains("time_scale"),
            "错误信息应包含 time_scale, actual={}",
            msg
        );
    }

    #[test]
    fn test_sps_parse_poc_type1_fields() {
        let rbsp = build_test_sps_poc_type1_with_cycle(2);
        let sps = parse_sps(&rbsp).expect("poc_type1 SPS 解析失败");
        assert_eq!(sps.poc_type, 1, "poc_type 应为 1");
        assert!(
            !sps.delta_pic_order_always_zero_flag,
            "delta_pic_order_always_zero_flag 解析错误"
        );
        assert_eq!(
            sps.offset_for_non_ref_pic, 0,
            "offset_for_non_ref_pic 解析错误"
        );
        assert_eq!(
            sps.offset_for_top_to_bottom_field, 0,
            "offset_for_top_to_bottom_field 解析错误"
        );
        assert_eq!(
            sps.offset_for_ref_frame.len(),
            2,
            "offset_for_ref_frame 长度错误"
        );
        assert_eq!(
            sps.offset_for_ref_frame,
            vec![0, 0],
            "offset_for_ref_frame 解析错误"
        );
    }

    #[test]
    fn test_sps_parse_scaling_lists_non444_custom_and_fallback() {
        let rbsp = build_test_high_profile_sps_with_scaling_lists(1);
        let sps = parse_sps(&rbsp).expect("带 scaling_list 的 SPS 解析失败");

        assert_eq!(
            sps.scaling_list_4x4[0], DEFAULT_SCALING_4X4_INTRA,
            "list0 useDefault 应回退到默认 Intra 4x4 矩阵"
        );
        assert_eq!(
            sps.scaling_list_4x4[1], DEFAULT_SCALING_4X4_INTRA,
            "list1 未显式给出时应回退到 list0"
        );
        assert_eq!(
            sps.scaling_list_4x4[3], DEFAULT_SCALING_4X4_INTER,
            "list3 useDefault 应回退到默认 Inter 4x4 矩阵"
        );
        assert_eq!(
            sps.scaling_list_8x8.len(),
            2,
            "4:2:0 应仅有 2 组 8x8 scaling_list"
        );

        assert!(
            sps.scaling_list_8x8[0].iter().all(|v| *v == 8),
            "自定义 list6 应解析为常量 8"
        );
        assert_eq!(
            sps.scaling_list_8x8[1], DEFAULT_SCALING_8X8_INTER,
            "list7 未显式给出时应使用默认 Inter 8x8 矩阵"
        );
    }

    #[test]
    fn test_sps_parse_scaling_lists_444_absent_uses_same_parity_fallback() {
        let rbsp = build_test_high_profile_sps_with_scaling_lists(3);
        let sps = parse_sps(&rbsp).expect("4:4:4 scaling_list SPS 解析失败");

        assert_eq!(
            sps.scaling_list_8x8.len(),
            6,
            "4:4:4 应有 6 组 8x8 scaling_list"
        );
        assert!(
            sps.scaling_list_8x8[0].iter().all(|v| *v == 8),
            "list6 自定义常量 8 解析错误"
        );
        assert_eq!(
            sps.scaling_list_8x8[1], DEFAULT_SCALING_8X8_INTER,
            "list7 useDefault 应回退到默认 Inter 8x8 矩阵"
        );
        assert_eq!(
            sps.scaling_list_8x8[2], sps.scaling_list_8x8[0],
            "list8 未显式给出时应回退到同奇偶的 list6"
        );
        assert_eq!(
            sps.scaling_list_8x8[3], sps.scaling_list_8x8[1],
            "list9 未显式给出时应回退到同奇偶的 list7"
        );
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

    /// 写入 se(v)
    fn write_se(bits: &mut Vec<bool>, val: i32) {
        let code_num = if val <= 0 {
            (-2 * val) as u32
        } else {
            (2 * val - 1) as u32
        };
        write_ue(bits, code_num);
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

    /// 构造带自定义 `sps_id/poc_type/max_num_ref_frames` 的最小 SPS RBSP.
    fn build_test_sps_rbsp_custom(sps_id: u32, poc_type: u32, max_num_ref_frames: u32) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc=66(Baseline), constraints=0, level=30
        for i in (0..8).rev() {
            bits.push(((66u8 >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8));
        for i in (0..8).rev() {
            bits.push(((30u8 >> i) & 1) != 0);
        }

        write_ue(&mut bits, sps_id);
        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, poc_type);
        if poc_type == 0 {
            write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
        } else if poc_type == 1 {
            bits.push(false); // delta_pic_order_always_zero_flag
            write_ue(&mut bits, 0); // offset_for_non_ref_pic(se=0)
            write_ue(&mut bits, 0); // offset_for_top_to_bottom_field(se=0)
            write_ue(&mut bits, 0); // num_ref_frames_in_pic_order_cnt_cycle
        }

        write_ue(&mut bits, max_num_ref_frames);
        bits.push(false); // gaps_in_frame_num_value_allowed_flag
        write_ue(&mut bits, 19); // width: (19+1)*16 = 320
        write_ue(&mut bits, 14); // height: (14+1)*16 = 240
        bits.push(true); // frame_mbs_only_flag
        bits.push(false); // direct_8x8_inference_flag
        bits.push(false); // frame_cropping_flag
        bits.push(false); // vui_parameters_present_flag

        bits_to_bytes(&bits)
    }

    /// 构造高 profile SPS, 用于测试 `chroma_format_idc` 校验.
    fn build_test_high_profile_sps_with_chroma(chroma_format_idc: u32) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc=100(High), constraints=0, level=40
        for i in (0..8).rev() {
            bits.push(((100u8 >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8));
        for i in (0..8).rev() {
            bits.push(((40u8 >> i) & 1) != 0);
        }

        write_ue(&mut bits, 0); // sps_id
        write_ue(&mut bits, chroma_format_idc);
        if chroma_format_idc == 3 {
            bits.push(false); // separate_colour_plane_flag
        }
        write_ue(&mut bits, 0); // bit_depth_luma_minus8
        write_ue(&mut bits, 0); // bit_depth_chroma_minus8
        bits.push(false); // qpprime_y_zero_transform_bypass_flag
        bits.push(false); // seq_scaling_matrix_present_flag

        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, 0); // pic_order_cnt_type
        write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
        write_ue(&mut bits, 4); // max_num_ref_frames
        bits.push(false); // gaps_in_frame_num_value_allowed_flag
        write_ue(&mut bits, 19); // width=320
        write_ue(&mut bits, 14); // height=240
        bits.push(true); // frame_mbs_only_flag
        bits.push(false); // direct_8x8_inference_flag
        bits.push(false); // frame_cropping_flag
        bits.push(false); // vui_parameters_present_flag

        bits_to_bytes(&bits)
    }

    fn build_test_sps_poc_type1_with_cycle(num_ref_in_cycle: u32) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc=66(Baseline), constraints=0, level=30
        for i in (0..8).rev() {
            bits.push(((66u8 >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8));
        for i in (0..8).rev() {
            bits.push(((30u8 >> i) & 1) != 0);
        }

        write_ue(&mut bits, 0); // sps_id
        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, 1); // pic_order_cnt_type
        bits.push(false); // delta_pic_order_always_zero_flag
        write_ue(&mut bits, 0); // offset_for_non_ref_pic(se=0)
        write_ue(&mut bits, 0); // offset_for_top_to_bottom_field(se=0)
        write_ue(&mut bits, num_ref_in_cycle);
        for _ in 0..num_ref_in_cycle {
            write_ue(&mut bits, 0); // offset_for_ref_frame[i]
        }
        write_ue(&mut bits, 4); // max_num_ref_frames
        bits.push(false); // gaps_in_frame_num_value_allowed_flag
        write_ue(&mut bits, 19); // width=320
        write_ue(&mut bits, 14); // height=240
        bits.push(true); // frame_mbs_only_flag
        bits.push(false); // direct_8x8_inference_flag
        bits.push(false); // frame_cropping_flag
        bits.push(false); // vui_parameters_present_flag

        bits_to_bytes(&bits)
    }

    /// 构造带 `scaling_list` 语法的 High Profile SPS.
    ///
    /// 语法设计:
    /// - list0: useDefault(4x4 Intra)
    /// - list1/list2: absent, 触发回退
    /// - list3: useDefault(4x4 Inter)
    /// - list4/list5: absent, 触发回退
    /// - list6: 自定义常量 8
    /// - list7: absent(4:2:0/4:2:2) 或 useDefault(4:4:4)
    /// - list8..11(仅 4:4:4): absent, 触发同奇偶回退
    fn build_test_high_profile_sps_with_scaling_lists(chroma_format_idc: u32) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc=100(High), constraints=0, level=40
        for i in (0..8).rev() {
            bits.push(((100u8 >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8));
        for i in (0..8).rev() {
            bits.push(((40u8 >> i) & 1) != 0);
        }

        write_ue(&mut bits, 0); // sps_id
        write_ue(&mut bits, chroma_format_idc);
        if chroma_format_idc == 3 {
            bits.push(false); // separate_colour_plane_flag
        }
        write_ue(&mut bits, 0); // bit_depth_luma_minus8
        write_ue(&mut bits, 0); // bit_depth_chroma_minus8
        bits.push(false); // qpprime_y_zero_transform_bypass_flag
        bits.push(true); // seq_scaling_matrix_present_flag

        let list_count = if chroma_format_idc == 3 { 12 } else { 8 };
        for idx in 0..list_count {
            match idx {
                0 => {
                    bits.push(true);
                    write_se(&mut bits, -8); // useDefaultScalingMatrixFlag
                }
                1 | 2 | 4 | 5 => bits.push(false),
                3 => {
                    bits.push(true);
                    write_se(&mut bits, -8); // useDefaultScalingMatrixFlag
                }
                6 => {
                    bits.push(true);
                    for _ in 0..64 {
                        write_se(&mut bits, 0); // 解析为常量 8
                    }
                }
                7 => {
                    if chroma_format_idc == 3 {
                        bits.push(true);
                        write_se(&mut bits, -8); // useDefaultScalingMatrixFlag
                    } else {
                        bits.push(false);
                    }
                }
                _ => bits.push(false),
            }
        }

        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, 0); // pic_order_cnt_type
        write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
        write_ue(&mut bits, 4); // max_num_ref_frames
        bits.push(false); // gaps_in_frame_num_value_allowed_flag
        write_ue(&mut bits, 19); // width=320
        write_ue(&mut bits, 14); // height=240
        bits.push(true); // frame_mbs_only_flag
        bits.push(false); // direct_8x8_inference_flag
        bits.push(false); // frame_cropping_flag
        bits.push(false); // vui_parameters_present_flag

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

    fn build_test_sps_with_custom_vui(
        aspect_ratio_idc: u8,
        extended_sar: Option<(u32, u32)>,
        timing_info: Option<(u32, u32)>,
    ) -> Vec<u8> {
        let mut bits = Vec::new();

        // profile_idc=66, constraints=0, level=30
        for i in (0..8).rev() {
            bits.push(((66u8 >> i) & 1) != 0);
        }
        bits.extend(std::iter::repeat_n(false, 8));
        for i in (0..8).rev() {
            bits.push(((30u8 >> i) & 1) != 0);
        }

        // 最小 SPS 主体
        write_ue(&mut bits, 0); // sps_id
        write_ue(&mut bits, 0); // log2_max_frame_num_minus4
        write_ue(&mut bits, 0); // pic_order_cnt_type
        write_ue(&mut bits, 0); // log2_max_pic_order_cnt_lsb_minus4
        write_ue(&mut bits, 4); // max_num_ref_frames
        bits.push(false); // gaps
        write_ue(&mut bits, 19); // width=320
        write_ue(&mut bits, 14); // height=240
        bits.push(true); // frame_mbs_only
        bits.push(false); // direct_8x8
        bits.push(false); // frame_cropping_flag

        // vui_parameters_present_flag = 1
        bits.push(true);

        // aspect_ratio_info_present_flag = 1
        bits.push(true);
        for i in (0..8).rev() {
            bits.push(((aspect_ratio_idc >> i) & 1) != 0);
        }
        if aspect_ratio_idc == 255 {
            let (sar_w, sar_h) = extended_sar.unwrap_or((1, 1));
            for i in (0..16).rev() {
                bits.push(((sar_w >> i) & 1) != 0);
            }
            for i in (0..16).rev() {
                bits.push(((sar_h >> i) & 1) != 0);
            }
        }

        // overscan_info_present_flag = 0
        bits.push(false);
        // video_signal_type_present_flag = 0
        bits.push(false);
        // chroma_loc_info_present_flag = 0
        bits.push(false);

        // timing_info_present_flag
        if let Some((num_units, time_scale)) = timing_info {
            bits.push(true);
            for i in (0..32).rev() {
                bits.push(((num_units >> i) & 1) != 0);
            }
            for i in (0..32).rev() {
                bits.push(((time_scale >> i) & 1) != 0);
            }
            bits.push(true); // fixed_frame_rate_flag
        } else {
            bits.push(false);
        }

        bits_to_bytes(&bits)
    }
}
