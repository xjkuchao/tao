//! H.264 参数集解析.
//!
//! 当前模块负责 PPS 语法解析和合法性校验.

use tao_core::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

/// 解析 PPS 参数.
pub(super) fn parse_pps(rbsp: &[u8]) -> TaoResult<super::Pps> {
    if rbsp.is_empty() {
        return Err(TaoError::InvalidData("H264: PPS RBSP 为空".into()));
    }

    let mut br = BitReader::new(rbsp);
    let pps_id = super::read_ue(&mut br)?;
    if pps_id > 255 {
        return Err(TaoError::InvalidData(format!(
            "H264: pps_id 超出范围, pps_id={}",
            pps_id
        )));
    }

    let sps_id = super::read_ue(&mut br)?;
    if sps_id > 31 {
        return Err(TaoError::InvalidData(format!(
            "H264: sps_id 超出范围, sps_id={}",
            sps_id
        )));
    }

    let entropy = br.read_bit()? as u8;
    let pic_order_present = br.read_bit()? == 1;

    let num_slice_groups_minus1 = super::read_ue(&mut br)?;
    if num_slice_groups_minus1 > 7 {
        return Err(TaoError::InvalidData(format!(
            "H264: num_slice_groups_minus1 超出范围, value={}",
            num_slice_groups_minus1
        )));
    }
    if num_slice_groups_minus1 > 0 {
        skip_pps_slice_groups(&mut br, num_slice_groups_minus1)?;
    }

    let num_ref_idx_l0_default_active_minus1 = super::read_ue(&mut br)?;
    if num_ref_idx_l0_default_active_minus1 > 31 {
        return Err(TaoError::InvalidData(format!(
            "H264: num_ref_idx_l0_default_active_minus1 超出范围, value={}",
            num_ref_idx_l0_default_active_minus1
        )));
    }
    let num_ref_idx_l1_default_active_minus1 = super::read_ue(&mut br)?;
    if num_ref_idx_l1_default_active_minus1 > 31 {
        return Err(TaoError::InvalidData(format!(
            "H264: num_ref_idx_l1_default_active_minus1 超出范围, value={}",
            num_ref_idx_l1_default_active_minus1
        )));
    }

    let num_ref_idx_l0_default_active = num_ref_idx_l0_default_active_minus1 + 1;
    let num_ref_idx_l1_default_active = num_ref_idx_l1_default_active_minus1 + 1;
    let weighted_pred = br.read_bit()? == 1;
    let weighted_bipred_idc = br.read_bits(2)?;
    if weighted_bipred_idc > 2 {
        return Err(TaoError::InvalidData(format!(
            "H264: weighted_bipred_idc 非法, value={}",
            weighted_bipred_idc
        )));
    }

    // pic_init_qp_minus26: se(v)
    let qp_delta = super::read_se(&mut br)?;
    let pic_init_qp = 26 + qp_delta;
    if !(0..=51).contains(&pic_init_qp) {
        return Err(TaoError::InvalidData(format!(
            "H264: pic_init_qp 超出范围, pic_init_qp={}",
            pic_init_qp
        )));
    }

    // pic_init_qs_minus26: se(v)
    let _ = super::read_se(&mut br)?;

    // chroma_qp_index_offset: se(v)
    let chroma_qp_index_offset = super::read_se(&mut br)?;
    validate_chroma_offset("chroma_qp_index_offset", chroma_qp_index_offset)?;

    // deblocking_filter_control_present_flag
    let deblocking = br.read_bit()? == 1;
    // constrained_intra_pred_flag
    let _constrained_intra_pred = br.read_bit()?;
    // redundant_pic_cnt_present_flag
    let redundant_pic_cnt_present = br.read_bit()? == 1;

    let mut transform_8x8_mode = false;
    let mut second_chroma_qp_index_offset = chroma_qp_index_offset;

    if has_more_rbsp_data(&mut br) {
        transform_8x8_mode = br.read_bit()? == 1;
        let pic_scaling_matrix_present = br.read_bit()? == 1;
        if pic_scaling_matrix_present {
            skip_pps_scaling_lists(&mut br, transform_8x8_mode)?;
        }
        second_chroma_qp_index_offset = super::read_se(&mut br)?;
        validate_chroma_offset(
            "second_chroma_qp_index_offset",
            second_chroma_qp_index_offset,
        )?;
    }

    Ok(super::Pps {
        pps_id,
        sps_id,
        entropy_coding_mode: entropy,
        pic_init_qp,
        chroma_qp_index_offset,
        second_chroma_qp_index_offset,
        deblocking_filter_control: deblocking,
        pic_order_present,
        num_ref_idx_l0_default_active,
        num_ref_idx_l1_default_active,
        weighted_pred,
        weighted_bipred_idc,
        redundant_pic_cnt_present,
        transform_8x8_mode,
    })
}

fn validate_chroma_offset(field: &str, value: i32) -> TaoResult<()> {
    if !(-12..=12).contains(&value) {
        return Err(TaoError::InvalidData(format!(
            "H264: {} 超出范围, value={}",
            field, value
        )));
    }
    Ok(())
}

/// 跳过 PPS 的 slice group 相关语法.
fn skip_pps_slice_groups(br: &mut BitReader, num_slice_groups_minus1: u32) -> TaoResult<()> {
    let slice_group_map_type = super::read_ue(br)?;
    match slice_group_map_type {
        0 => {
            for _ in 0..=num_slice_groups_minus1 {
                let _run_length_minus1 = super::read_ue(br)?;
            }
        }
        2 => {
            for _ in 0..num_slice_groups_minus1 {
                let _top_left = super::read_ue(br)?;
                let _bottom_right = super::read_ue(br)?;
            }
        }
        3..=5 => {
            let _slice_group_change_direction_flag = br.read_bit()?;
            let _slice_group_change_rate_minus1 = super::read_ue(br)?;
        }
        6 => {
            let pic_size_in_map_units_minus1 = super::read_ue(br)?;
            let group_count = num_slice_groups_minus1 + 1;
            let bits_per_id = bits_for_slice_group_id(group_count);
            for _ in 0..=pic_size_in_map_units_minus1 {
                if bits_per_id > 0 {
                    let _slice_group_id = br.read_bits(bits_per_id)?;
                }
            }
        }
        _ => {
            return Err(TaoError::InvalidData(format!(
                "H264: slice_group_map_type 非法, value={}",
                slice_group_map_type
            )));
        }
    }
    Ok(())
}

fn bits_for_slice_group_id(group_count: u32) -> u32 {
    if group_count <= 1 {
        0
    } else {
        u32::BITS - (group_count - 1).leading_zeros()
    }
}

/// 判断 RBSP 是否仍有有效语法数据 (排除 rbsp_trailing_bits).
fn has_more_rbsp_data(br: &mut BitReader) -> bool {
    let bits_left = br.bits_left();
    if bits_left == 0 {
        return false;
    }
    if bits_left > 8 {
        return true;
    }
    let Ok(rest) = br.peek_bits(bits_left as u32) else {
        return false;
    };
    let trailing = 1u32 << (bits_left - 1);
    rest != trailing
}

/// 跳过 PPS scaling list 语法.
fn skip_pps_scaling_lists(br: &mut BitReader, transform_8x8_mode: bool) -> TaoResult<()> {
    let list_count = if transform_8x8_mode { 8 } else { 6 };
    for i in 0..list_count {
        let present = br.read_bit()?;
        if present == 1 {
            let size = if i < 6 { 16 } else { 64 };
            skip_scaling_list(br, size)?;
        }
    }
    Ok(())
}

/// 跳过单个 scaling list.
fn skip_scaling_list(br: &mut BitReader, size: usize) -> TaoResult<()> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = super::read_se(br)?;
            next_scale = (last_scale + delta_scale + 256) % 256;
        }
        last_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_pps;

    struct PpsTestInput {
        pps_id: u32,
        sps_id: u32,
        entropy: bool,
        pic_order_present: bool,
        num_slice_groups_minus1: u32,
        num_ref_idx_l0_default_active_minus1: u32,
        num_ref_idx_l1_default_active_minus1: u32,
        weighted_pred: bool,
        weighted_bipred_idc: u32,
        pic_init_qp_minus26: i32,
        pic_init_qs_minus26: i32,
        chroma_qp_index_offset: i32,
        deblocking: bool,
        constrained_intra_pred: bool,
        redundant_pic_cnt_present: bool,
        ext: Option<PpsExtInput>,
    }

    struct PpsExtInput {
        transform_8x8_mode: bool,
        pic_scaling_matrix_present: bool,
        second_chroma_qp_index_offset: i32,
    }

    #[test]
    fn test_parse_pps_basic() {
        let rbsp = build_pps_rbsp(PpsTestInput {
            pps_id: 3,
            sps_id: 1,
            entropy: true,
            pic_order_present: false,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 0,
            num_ref_idx_l1_default_active_minus1: 1,
            weighted_pred: true,
            weighted_bipred_idc: 2,
            pic_init_qp_minus26: -4,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: -2,
            deblocking: true,
            constrained_intra_pred: false,
            redundant_pic_cnt_present: true,
            ext: None,
        });
        let pps = parse_pps(&rbsp).expect("PPS 解析失败");
        assert_eq!(pps.pps_id, 3, "pps_id 解析错误");
        assert_eq!(pps.sps_id, 1, "sps_id 解析错误");
        assert_eq!(pps.entropy_coding_mode, 1, "entropy_coding_mode 解析错误");
        assert!(!pps.pic_order_present, "pic_order_present 解析错误");
        assert_eq!(
            pps.num_ref_idx_l0_default_active, 1,
            "L0 默认参考数解析错误"
        );
        assert_eq!(
            pps.num_ref_idx_l1_default_active, 2,
            "L1 默认参考数解析错误"
        );
        assert!(pps.weighted_pred, "weighted_pred 解析错误");
        assert_eq!(pps.weighted_bipred_idc, 2, "weighted_bipred_idc 解析错误");
        assert_eq!(pps.pic_init_qp, 22, "pic_init_qp 解析错误");
        assert_eq!(
            pps.chroma_qp_index_offset, -2,
            "chroma_qp_index_offset 解析错误"
        );
        assert_eq!(
            pps.second_chroma_qp_index_offset, -2,
            "second_chroma_qp_index_offset 默认值错误"
        );
        assert!(
            pps.deblocking_filter_control,
            "deblocking_filter_control 解析错误"
        );
        assert!(
            pps.redundant_pic_cnt_present,
            "redundant_pic_cnt_present 解析错误"
        );
        assert!(!pps.transform_8x8_mode, "transform_8x8_mode 默认值错误");
    }

    #[test]
    fn test_parse_pps_with_extension() {
        let rbsp = build_pps_rbsp(PpsTestInput {
            pps_id: 7,
            sps_id: 2,
            entropy: false,
            pic_order_present: true,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 2,
            num_ref_idx_l1_default_active_minus1: 0,
            weighted_pred: false,
            weighted_bipred_idc: 1,
            pic_init_qp_minus26: 0,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 1,
            deblocking: false,
            constrained_intra_pred: true,
            redundant_pic_cnt_present: false,
            ext: Some(PpsExtInput {
                transform_8x8_mode: true,
                pic_scaling_matrix_present: false,
                second_chroma_qp_index_offset: -1,
            }),
        });
        let pps = parse_pps(&rbsp).expect("带扩展字段 PPS 解析失败");
        assert_eq!(pps.pps_id, 7, "pps_id 解析错误");
        assert_eq!(pps.sps_id, 2, "sps_id 解析错误");
        assert_eq!(pps.entropy_coding_mode, 0, "entropy_coding_mode 解析错误");
        assert!(pps.pic_order_present, "pic_order_present 解析错误");
        assert_eq!(
            pps.num_ref_idx_l0_default_active, 3,
            "L0 默认参考数解析错误"
        );
        assert_eq!(
            pps.num_ref_idx_l1_default_active, 1,
            "L1 默认参考数解析错误"
        );
        assert_eq!(
            pps.second_chroma_qp_index_offset, -1,
            "second_chroma_qp_index_offset 解析错误"
        );
        assert!(pps.transform_8x8_mode, "transform_8x8_mode 扩展解析错误");
    }

    #[test]
    fn test_parse_pps_reject_weighted_bipred_idc_3() {
        let rbsp = build_pps_rbsp(PpsTestInput {
            pps_id: 0,
            sps_id: 0,
            entropy: true,
            pic_order_present: false,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 0,
            num_ref_idx_l1_default_active_minus1: 0,
            weighted_pred: false,
            weighted_bipred_idc: 3,
            pic_init_qp_minus26: 0,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 0,
            deblocking: true,
            constrained_intra_pred: false,
            redundant_pic_cnt_present: false,
            ext: None,
        });
        match parse_pps(&rbsp) {
            Ok(_) => panic!("weighted_bipred_idc=3 应失败"),
            Err(err) => {
                let msg = format!("{}", err);
                assert!(
                    msg.contains("weighted_bipred_idc"),
                    "错误信息应包含字段名, actual={}",
                    msg
                );
            }
        }
    }

    #[test]
    fn test_parse_pps_reject_pic_init_qp_out_of_range() {
        let rbsp = build_pps_rbsp(PpsTestInput {
            pps_id: 0,
            sps_id: 0,
            entropy: true,
            pic_order_present: false,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 0,
            num_ref_idx_l1_default_active_minus1: 0,
            weighted_pred: false,
            weighted_bipred_idc: 0,
            pic_init_qp_minus26: 40,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 0,
            deblocking: true,
            constrained_intra_pred: false,
            redundant_pic_cnt_present: false,
            ext: None,
        });
        match parse_pps(&rbsp) {
            Ok(_) => panic!("pic_init_qp 超范围应失败"),
            Err(err) => {
                let msg = format!("{}", err);
                assert!(
                    msg.contains("pic_init_qp"),
                    "错误信息应包含字段名, actual={}",
                    msg
                );
            }
        }
    }

    #[test]
    fn test_parse_pps_reject_chroma_offset_out_of_range() {
        let rbsp = build_pps_rbsp(PpsTestInput {
            pps_id: 0,
            sps_id: 0,
            entropy: true,
            pic_order_present: false,
            num_slice_groups_minus1: 0,
            num_ref_idx_l0_default_active_minus1: 0,
            num_ref_idx_l1_default_active_minus1: 0,
            weighted_pred: false,
            weighted_bipred_idc: 0,
            pic_init_qp_minus26: 0,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 13,
            deblocking: true,
            constrained_intra_pred: false,
            redundant_pic_cnt_present: false,
            ext: None,
        });
        match parse_pps(&rbsp) {
            Ok(_) => panic!("chroma_qp_index_offset 超范围应失败"),
            Err(err) => {
                let msg = format!("{}", err);
                assert!(
                    msg.contains("chroma_qp_index_offset"),
                    "错误信息应包含字段名, actual={}",
                    msg
                );
            }
        }
    }

    fn build_pps_rbsp(input: PpsTestInput) -> Vec<u8> {
        let mut bits = Vec::<bool>::new();

        write_ue(&mut bits, input.pps_id);
        write_ue(&mut bits, input.sps_id);
        write_bit(&mut bits, input.entropy);
        write_bit(&mut bits, input.pic_order_present);
        write_ue(&mut bits, input.num_slice_groups_minus1);
        if input.num_slice_groups_minus1 > 0 {
            // 单测里只覆盖 map_type=0 的最小路径.
            write_ue(&mut bits, 0);
            for _ in 0..=input.num_slice_groups_minus1 {
                write_ue(&mut bits, 0);
            }
        }

        write_ue(&mut bits, input.num_ref_idx_l0_default_active_minus1);
        write_ue(&mut bits, input.num_ref_idx_l1_default_active_minus1);
        write_bit(&mut bits, input.weighted_pred);
        write_bits(&mut bits, input.weighted_bipred_idc, 2);
        write_se(&mut bits, input.pic_init_qp_minus26);
        write_se(&mut bits, input.pic_init_qs_minus26);
        write_se(&mut bits, input.chroma_qp_index_offset);
        write_bit(&mut bits, input.deblocking);
        write_bit(&mut bits, input.constrained_intra_pred);
        write_bit(&mut bits, input.redundant_pic_cnt_present);

        if let Some(ext) = input.ext {
            write_bit(&mut bits, ext.transform_8x8_mode);
            write_bit(&mut bits, ext.pic_scaling_matrix_present);
            if ext.pic_scaling_matrix_present {
                let list_count = if ext.transform_8x8_mode { 8 } else { 6 };
                for _ in 0..list_count {
                    // list_present_flag = 0, 不写 scaling_list 数据.
                    write_bit(&mut bits, false);
                }
            }
            write_se(&mut bits, ext.second_chroma_qp_index_offset);
        }

        // rbsp_trailing_bits
        write_bit(&mut bits, true);
        while !bits.len().is_multiple_of(8) {
            write_bit(&mut bits, false);
        }

        bits_to_bytes(&bits)
    }

    fn write_bit(bits: &mut Vec<bool>, v: bool) {
        bits.push(v);
    }

    fn write_bits(bits: &mut Vec<bool>, value: u32, n: usize) {
        for i in (0..n).rev() {
            bits.push(((value >> i) & 1) == 1);
        }
    }

    fn write_ue(bits: &mut Vec<bool>, val: u32) {
        let code_num = val + 1;
        let leading_zeros = code_num.ilog2();
        for _ in 0..leading_zeros {
            bits.push(false);
        }
        for i in (0..=leading_zeros).rev() {
            bits.push(((code_num >> i) & 1) == 1);
        }
    }

    fn write_se(bits: &mut Vec<bool>, val: i32) {
        let code_num = if val <= 0 {
            (-val as u32) * 2
        } else {
            (val as u32) * 2 - 1
        };
        write_ue(bits, code_num);
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bits.len().div_ceil(8));
        let mut cur = 0u8;
        for (i, &bit) in bits.iter().enumerate() {
            cur = (cur << 1) | u8::from(bit);
            if i % 8 == 7 {
                out.push(cur);
                cur = 0;
            }
        }
        out
    }
}
