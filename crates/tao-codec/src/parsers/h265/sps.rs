//! H.265/HEVC VPS 和 SPS 解析器.
//!
//! VPS 包含视频参数集全局信息.
//! SPS 包含编码视频序列的参数: profile/level, 分辨率, 色度格式等.

use tao_core::bitreader::BitReader;
use tao_core::{Rational, TaoError, TaoResult};

use super::nal::remove_emulation_prevention;

/// VPS 解析结果
#[derive(Debug, Clone)]
pub struct HevcVps {
    /// VPS ID
    pub vps_id: u8,
    /// 最大子层数
    pub max_sub_layers: u8,
    /// 时序 ID 嵌套标志
    pub temporal_id_nesting: bool,
    /// general_profile_idc
    pub general_profile_idc: u8,
    /// general_tier_flag
    pub general_tier_flag: bool,
    /// general_level_idc
    pub general_level_idc: u8,
}

/// SPS 解析结果
#[derive(Debug, Clone)]
pub struct HevcSps {
    /// SPS 所引用的 VPS ID
    pub vps_id: u8,
    /// 最大子层数
    pub max_sub_layers: u8,
    /// SPS ID
    pub sps_id: u32,
    /// general_profile_idc
    pub general_profile_idc: u8,
    /// general_tier_flag
    pub general_tier_flag: bool,
    /// general_level_idc
    pub general_level_idc: u8,
    /// 色度格式 (0=单色, 1=4:2:0, 2=4:2:2, 3=4:4:4)
    pub chroma_format_idc: u32,
    /// 亮度位深
    pub bit_depth_luma: u32,
    /// 色度位深
    pub bit_depth_chroma: u32,
    /// 图像宽度 (像素, 已应用 conformance window)
    pub width: u32,
    /// 图像高度 (像素, 已应用 conformance window)
    pub height: u32,
    /// 原始宽度 (像素, 未裁剪)
    pub pic_width: u32,
    /// 原始高度 (像素, 未裁剪)
    pub pic_height: u32,
    /// conformance window 裁剪
    pub conf_win_left: u32,
    pub conf_win_right: u32,
    pub conf_win_top: u32,
    pub conf_win_bottom: u32,
    /// 帧率 (如果 VUI 中有 timing_info)
    pub fps: Option<Rational>,
    /// SAR (Sample Aspect Ratio)
    pub sar: Rational,
}

/// Exp-Golomb 无符号解码
fn read_ue(br: &mut BitReader) -> TaoResult<u32> {
    let mut leading_zeros = 0u32;
    loop {
        if br.read_bits(1)? == 1 {
            break;
        }
        leading_zeros += 1;
        if leading_zeros > 31 {
            return Err(TaoError::InvalidData("HEVC: Exp-Golomb 过长".into()));
        }
    }
    if leading_zeros == 0 {
        return Ok(0);
    }
    let val = br.read_bits(leading_zeros)?;
    Ok((1 << leading_zeros) - 1 + val)
}

/// Exp-Golomb 有符号解码
fn read_se(br: &mut BitReader) -> TaoResult<i32> {
    let code = read_ue(br)?;
    let val = (code + 1).div_ceil(2) as i32;
    if code % 2 == 0 { Ok(-val) } else { Ok(val) }
}

/// 解析 profile_tier_level
fn parse_profile_tier_level(br: &mut BitReader, max_sub_layers: u8) -> TaoResult<(u8, bool, u8)> {
    let _profile_space = br.read_bits(2)?;
    let tier_flag = br.read_bits(1)? != 0;
    let profile_idc = br.read_bits(5)? as u8;

    // general_profile_compatibility_flags (32 bits)
    br.read_bits(32)?;

    // general_progressive_source_flag + interlaced + non_packed + frame_only
    br.read_bits(4)?;

    // 44 bits of constraint flags
    // profile_idc specific constraints (44 bits)
    br.read_bits(32)?;
    br.read_bits(12)?;

    let level_idc = br.read_bits(8)? as u8;

    // sub_layer flags
    if max_sub_layers > 1 {
        let mut sub_layer_profile_present = Vec::new();
        let mut sub_layer_level_present = Vec::new();
        for _ in 0..max_sub_layers - 1 {
            sub_layer_profile_present.push(br.read_bits(1)? != 0);
            sub_layer_level_present.push(br.read_bits(1)? != 0);
        }
        // 对齐到 16 的倍数
        if max_sub_layers < 9 {
            for _ in max_sub_layers - 1..8 {
                br.read_bits(2)?; // reserved
            }
        }
        for i in 0..max_sub_layers as usize - 1 {
            if sub_layer_profile_present[i] {
                br.read_bits(32)?; // sub_layer_profile_space..compatibility
                br.read_bits(32)?;
                br.read_bits(24)?;
            }
            if sub_layer_level_present[i] {
                br.read_bits(8)?;
            }
        }
    }

    Ok((profile_idc, tier_flag, level_idc))
}

/// 预定义 SAR 表 (ITU-T H.265 表 E.1)
const HEVC_SAR_TABLE: [(u32, u32); 17] = [
    (0, 1),
    (1, 1),
    (12, 11),
    (10, 11),
    (16, 11),
    (40, 33),
    (24, 11),
    (20, 11),
    (32, 11),
    (80, 33),
    (18, 11),
    (15, 11),
    (64, 33),
    (160, 99),
    (4, 3),
    (3, 2),
    (2, 1),
];

/// 解析 HEVC VPS
pub fn parse_hevc_vps(rbsp: &[u8]) -> TaoResult<HevcVps> {
    if rbsp.len() < 2 {
        return Err(TaoError::InvalidData("HEVC: VPS RBSP 太短".into()));
    }

    let clean = remove_emulation_prevention(rbsp);
    let mut br = BitReader::new(&clean);

    let vps_id = br.read_bits(4)? as u8;
    br.read_bits(2)?; // vps_reserved_three_2bits
    let _max_layers = br.read_bits(6)? + 1;
    let max_sub_layers = br.read_bits(3)? as u8 + 1;
    let temporal_id_nesting = br.read_bits(1)? != 0;
    br.read_bits(16)?; // vps_reserved_0xffff_16bits

    let (profile_idc, tier_flag, level_idc) = parse_profile_tier_level(&mut br, max_sub_layers)?;

    Ok(HevcVps {
        vps_id,
        max_sub_layers,
        temporal_id_nesting,
        general_profile_idc: profile_idc,
        general_tier_flag: tier_flag,
        general_level_idc: level_idc,
    })
}

/// 解析 HEVC SPS
pub fn parse_hevc_sps(rbsp: &[u8]) -> TaoResult<HevcSps> {
    if rbsp.len() < 3 {
        return Err(TaoError::InvalidData("HEVC: SPS RBSP 太短".into()));
    }

    let clean = remove_emulation_prevention(rbsp);
    let mut br = BitReader::new(&clean);

    let vps_id = br.read_bits(4)? as u8;
    let max_sub_layers = br.read_bits(3)? as u8 + 1;
    let _temporal_id_nesting = br.read_bits(1)?;

    let (profile_idc, tier_flag, level_idc) = parse_profile_tier_level(&mut br, max_sub_layers)?;

    let sps_id = read_ue(&mut br)?;
    let chroma_format_idc = read_ue(&mut br)?;

    if chroma_format_idc == 3 {
        let _separate_colour_plane = br.read_bits(1)?;
    }

    let pic_width = read_ue(&mut br)?;
    let pic_height = read_ue(&mut br)?;

    let conformance_window = br.read_bits(1)? != 0;
    let (conf_win_left, conf_win_right, conf_win_top, conf_win_bottom) = if conformance_window {
        (
            read_ue(&mut br)?,
            read_ue(&mut br)?,
            read_ue(&mut br)?,
            read_ue(&mut br)?,
        )
    } else {
        (0, 0, 0, 0)
    };

    let bit_depth_luma = read_ue(&mut br)? + 8;
    let bit_depth_chroma = read_ue(&mut br)? + 8;
    let _log2_max_pic_order_cnt = read_ue(&mut br)? + 4;

    let sub_layer_ordering = br.read_bits(1)? != 0;
    let start = if sub_layer_ordering {
        0
    } else {
        max_sub_layers as u32 - 1
    };
    for _ in start..max_sub_layers as u32 {
        read_ue(&mut br)?; // max_dec_pic_buffering
        read_ue(&mut br)?; // max_num_reorder_pics
        read_ue(&mut br)?; // max_latency_increase
    }

    let _log2_min_luma_coding_block = read_ue(&mut br)? + 3;
    let _log2_diff_max_min_luma_coding_block = read_ue(&mut br)?;
    let _log2_min_transform_block = read_ue(&mut br)? + 2;
    let _log2_diff_max_min_transform_block = read_ue(&mut br)?;
    let _max_transform_hierarchy_depth_inter = read_ue(&mut br)?;
    let _max_transform_hierarchy_depth_intra = read_ue(&mut br)?;

    // scaling_list
    let scaling_list_enabled = br.read_bits(1)? != 0;
    if scaling_list_enabled {
        let scaling_list_data_present = br.read_bits(1)? != 0;
        if scaling_list_data_present {
            skip_scaling_list_data(&mut br)?;
        }
    }

    let _amp_enabled = br.read_bits(1)?;
    let _sao_enabled = br.read_bits(1)?;

    // PCM
    let pcm_enabled = br.read_bits(1)? != 0;
    if pcm_enabled {
        br.read_bits(4)?; // pcm_sample_bit_depth_luma
        br.read_bits(4)?; // pcm_sample_bit_depth_chroma
        read_ue(&mut br)?; // log2_min_pcm_luma
        read_ue(&mut br)?; // log2_diff_max_min_pcm_luma
        br.read_bits(1)?; // pcm_loop_filter_disabled
    }

    let num_short_term_rps = read_ue(&mut br)?;
    for i in 0..num_short_term_rps {
        skip_short_term_rps(&mut br, i, num_short_term_rps)?;
    }

    let long_term_ref_pics_present = br.read_bits(1)? != 0;
    if long_term_ref_pics_present {
        let num_long_term_ref_pics = read_ue(&mut br)?;
        let log2_max_poc = _log2_max_pic_order_cnt;
        for _ in 0..num_long_term_ref_pics {
            br.read_bits(log2_max_poc)?; // lt_ref_pic_poc_lsb
            br.read_bits(1)?; // used_by_curr_pic_lt
        }
    }

    let _temporal_mvp_enabled = br.read_bits(1)?;
    let _strong_intra_smoothing = br.read_bits(1)?;

    // VUI parameters
    let mut fps = None;
    let mut sar = Rational::new(1, 1);

    let vui_present = br.read_bits(1)? != 0;
    if vui_present {
        let aspect_ratio_info_present = br.read_bits(1)? != 0;
        if aspect_ratio_info_present {
            let aspect_ratio_idc = br.read_bits(8)? as usize;
            if aspect_ratio_idc == 255 {
                // Extended_SAR
                let sar_w = br.read_bits(16)? as u32;
                let sar_h = br.read_bits(16)? as u32;
                if sar_w > 0 && sar_h > 0 {
                    sar = Rational::new(sar_w as i32, sar_h as i32);
                }
            } else if aspect_ratio_idc < HEVC_SAR_TABLE.len() {
                let (w, h) = HEVC_SAR_TABLE[aspect_ratio_idc];
                if w > 0 && h > 0 {
                    sar = Rational::new(w as i32, h as i32);
                }
            }
        }

        let overscan_info_present = br.read_bits(1)? != 0;
        if overscan_info_present {
            br.read_bits(1)?; // overscan_appropriate
        }

        let video_signal_type_present = br.read_bits(1)? != 0;
        if video_signal_type_present {
            br.read_bits(3)?; // video_format
            br.read_bits(1)?; // video_full_range
            let colour_description_present = br.read_bits(1)? != 0;
            if colour_description_present {
                br.read_bits(8)?; // colour_primaries
                br.read_bits(8)?; // transfer_characteristics
                br.read_bits(8)?; // matrix_coeffs
            }
        }

        let chroma_loc_info_present = br.read_bits(1)? != 0;
        if chroma_loc_info_present {
            read_ue(&mut br)?;
            read_ue(&mut br)?;
        }

        br.read_bits(1)?; // neutral_chroma_indication
        br.read_bits(1)?; // field_seq
        br.read_bits(1)?; // frame_field_info_present

        let default_display_window = br.read_bits(1)? != 0;
        if default_display_window {
            read_ue(&mut br)?;
            read_ue(&mut br)?;
            read_ue(&mut br)?;
            read_ue(&mut br)?;
        }

        let timing_info_present = br.read_bits(1)? != 0;
        if timing_info_present {
            let num_units_in_tick = br.read_bits(32)? as u32;
            let time_scale = br.read_bits(32)? as u32;
            if num_units_in_tick > 0 && time_scale > 0 {
                // HEVC: fps = time_scale / num_units_in_tick
                fps = Some(Rational::new(time_scale as i32, num_units_in_tick as i32));
            }
        }
    }

    // 计算裁剪后分辨率
    let sub_width_c: u32 = if chroma_format_idc == 1 || chroma_format_idc == 2 {
        2
    } else {
        1
    };
    let sub_height_c: u32 = if chroma_format_idc == 1 { 2 } else { 1 };

    let width = pic_width - sub_width_c * (conf_win_left + conf_win_right);
    let height = pic_height - sub_height_c * (conf_win_top + conf_win_bottom);

    Ok(HevcSps {
        vps_id,
        max_sub_layers,
        sps_id,
        general_profile_idc: profile_idc,
        general_tier_flag: tier_flag,
        general_level_idc: level_idc,
        chroma_format_idc,
        bit_depth_luma,
        bit_depth_chroma,
        width,
        height,
        pic_width,
        pic_height,
        conf_win_left,
        conf_win_right,
        conf_win_top,
        conf_win_bottom,
        fps,
        sar,
    })
}

/// 跳过 scaling_list_data
fn skip_scaling_list_data(br: &mut BitReader) -> TaoResult<()> {
    for size_id in 0..4 {
        let count = if size_id == 3 { 2 } else { 6 };
        for _ in 0..count {
            let pred_mode = br.read_bits(1)?;
            if pred_mode == 0 {
                read_ue(br)?; // scaling_list_pred_matrix_id_delta
            } else {
                let coef_num = (1 << (4 + (size_id << 1)).min(6)) as u32;
                if size_id > 1 {
                    read_se(br)?; // scaling_list_dc_coef
                }
                for _ in 0..coef_num {
                    read_se(br)?; // scaling_list_delta_coef
                }
            }
        }
    }
    Ok(())
}

/// 跳过 short_term_ref_pic_set
fn skip_short_term_rps(br: &mut BitReader, idx: u32, _num_sets: u32) -> TaoResult<()> {
    let inter_ref_pic_set_prediction = if idx > 0 {
        br.read_bits(1)? != 0
    } else {
        false
    };

    if inter_ref_pic_set_prediction {
        if idx == _num_sets {
            read_ue(br)?; // delta_idx
        }
        br.read_bits(1)?; // delta_rps_sign
        read_ue(br)?; // abs_delta_rps
    // 这里需要已知 previous RPS 的大小, 简化处理: 假设为 0
    // 完整解析需要维护 RPS 状态
    } else {
        let num_negative = read_ue(br)?;
        let num_positive = read_ue(br)?;
        for _ in 0..num_negative {
            read_ue(br)?; // delta_poc_s0
            br.read_bits(1)?; // used_by_curr_pic_s0
        }
        for _ in 0..num_positive {
            read_ue(br)?; // delta_poc_s1
            br.read_bits(1)?; // used_by_curr_pic_s1
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构建最小 VPS RBSP
    fn build_vps_rbsp() -> Vec<u8> {
        // 手动构建 VPS RBSP 位流
        let mut bits = Vec::new();

        // vps_video_parameter_set_id (4 bits) = 0
        bits.extend_from_slice(&[false; 4]);
        // vps_reserved_three_2bits (2 bits) = 11
        bits.push(true);
        bits.push(true);
        // vps_max_layers_minus1 (6 bits) = 0
        bits.extend_from_slice(&[false; 6]);
        // vps_max_sub_layers_minus1 (3 bits) = 0
        bits.extend_from_slice(&[false; 3]);
        // vps_temporal_id_nesting_flag = 1
        bits.push(true);
        // vps_reserved_0xffff_16bits
        bits.extend(std::iter::repeat_n(true, 16));

        // profile_tier_level:
        // general_profile_space (2 bits) = 0
        bits.extend_from_slice(&[false; 2]);
        // general_tier_flag = 0
        bits.push(false);
        // general_profile_idc (5 bits) = 1 (Main)
        bits.extend_from_slice(&[false, false, false, false, true]);
        // general_profile_compatibility_flags (32 bits)
        bits.extend_from_slice(&[false; 32]);
        // progressive_source + interlaced + non_packed + frame_only (4 bits)
        bits.extend_from_slice(&[false; 4]);
        // constraint flags (44 bits)
        bits.extend_from_slice(&[false; 44]);
        // general_level_idc (8 bits) = 93 (level 3.1)
        let level: u8 = 93;
        for i in (0..8).rev() {
            bits.push((level >> i) & 1 != 0);
        }

        bits_to_bytes(&bits)
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
    fn test_parse_vps() {
        let rbsp = build_vps_rbsp();
        let vps = parse_hevc_vps(&rbsp).unwrap();
        assert_eq!(vps.vps_id, 0);
        assert_eq!(vps.max_sub_layers, 1);
        assert!(vps.temporal_id_nesting);
        assert_eq!(vps.general_profile_idc, 1);
        assert_eq!(vps.general_level_idc, 93);
    }

    #[test]
    fn test_sps_rbsp太短() {
        assert!(parse_hevc_sps(&[0]).is_err());
    }
}
