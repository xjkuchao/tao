//! H.264 参数集解析.
//!
//! 当前模块负责 PPS 语法解析和合法性校验.

use tao_core::bitreader::BitReader;
use tao_core::{TaoError, TaoResult};

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

type ParsedPpsScalingLists = ([[u8; 16]; 6], Vec<[u8; 64]>);

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
    let mut scaling_list_4x4 = None;
    let mut scaling_list_8x8 = None;

    if has_more_rbsp_data(&mut br) {
        transform_8x8_mode = br.read_bit()? == 1;
        let pic_scaling_matrix_present = br.read_bit()? == 1;
        if pic_scaling_matrix_present {
            let (list4x4, list8x8) = parse_pps_scaling_lists(&mut br, transform_8x8_mode)?;
            scaling_list_4x4 = Some(list4x4);
            scaling_list_8x8 = Some(list8x8);
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
        scaling_list_4x4,
        scaling_list_8x8,
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

fn parse_pps_scaling_lists(
    br: &mut BitReader,
    transform_8x8_mode: bool,
) -> TaoResult<ParsedPpsScalingLists> {
    let mut lists4x4 = [
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTRA,
        DEFAULT_SCALING_4X4_INTER,
        DEFAULT_SCALING_4X4_INTER,
        DEFAULT_SCALING_4X4_INTER,
    ];
    let mut lists8x8 = if transform_8x8_mode {
        vec![DEFAULT_SCALING_8X8_INTRA, DEFAULT_SCALING_8X8_INTER]
    } else {
        Vec::new()
    };
    let list_count = if transform_8x8_mode { 8 } else { 6 };
    for list_idx in 0..list_count {
        let present = br.read_bit()?;
        if present == 0 {
            apply_pps_absent_scaling_list_fallback(list_idx, &mut lists4x4, &mut lists8x8)?;
            continue;
        }

        if list_idx < 6 {
            let (parsed, use_default) = parse_scaling_list(br, 16)?;
            lists4x4[list_idx] = if use_default {
                default_scaling_list_4x4_by_idx(list_idx)
            } else {
                parsed
            };
        } else {
            let idx8 = list_idx - 6;
            let (parsed, use_default) = parse_scaling_list(br, 64)?;
            lists8x8[idx8] = if use_default {
                default_scaling_list_8x8_by_idx(idx8)
            } else {
                parsed
            };
        }
    }
    Ok((lists4x4, lists8x8))
}

fn apply_pps_absent_scaling_list_fallback(
    list_idx: usize,
    lists4x4: &mut [[u8; 16]; 6],
    lists8x8: &mut [[u8; 64]],
) -> TaoResult<()> {
    if list_idx < 6 {
        lists4x4[list_idx] = if list_idx == 0 || list_idx == 3 {
            default_scaling_list_4x4_by_idx(list_idx)
        } else {
            lists4x4[list_idx - 1]
        };
        return Ok(());
    }

    let idx8 = list_idx - 6;
    if idx8 >= lists8x8.len() {
        return Err(TaoError::InvalidData(format!(
            "H264: PPS scaling_list_8x8 索引越界, idx={}",
            idx8
        )));
    }
    lists8x8[idx8] = if idx8 == 0 || idx8 == 1 {
        default_scaling_list_8x8_by_idx(idx8)
    } else {
        lists8x8[idx8 - 1]
    };
    Ok(())
}

fn parse_scaling_list<const N: usize>(
    br: &mut BitReader,
    size: usize,
) -> TaoResult<([u8; N], bool)> {
    if size != N {
        return Err(TaoError::InvalidData(format!(
            "H264: scaling_list 大小不匹配, expect={}, got={}",
            N, size
        )));
    }
    let mut list = [0u8; N];
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    let mut use_default = false;

    for (idx, slot) in list.iter_mut().enumerate().take(size) {
        if next_scale != 0 {
            let delta_scale = super::read_se(br)?;
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
        assert!(
            pps.scaling_list_4x4.is_none(),
            "未携带扩展时不应有 PPS 4x4 scaling_list 覆盖"
        );
        assert!(
            pps.scaling_list_8x8.is_none(),
            "未携带扩展时不应有 PPS 8x8 scaling_list 覆盖"
        );
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
        assert!(
            pps.scaling_list_4x4.is_none(),
            "pic_scaling_matrix_present_flag=0 时不应输出 4x4 scaling_list 覆盖"
        );
        assert!(
            pps.scaling_list_8x8.is_none(),
            "pic_scaling_matrix_present_flag=0 时不应输出 8x8 scaling_list 覆盖"
        );
    }

    #[test]
    fn test_parse_pps_scaling_lists_custom_and_fallback() {
        let rbsp = build_pps_rbsp_with_custom_scaling_lists();
        let pps = parse_pps(&rbsp).expect("PPS scaling_list 解析失败");
        assert!(pps.transform_8x8_mode, "transform_8x8_mode 应为 true");

        let list4 = pps
            .scaling_list_4x4
            .as_ref()
            .expect("应包含 PPS 4x4 scaling_list 覆盖");
        let list8 = pps
            .scaling_list_8x8
            .as_ref()
            .expect("应包含 PPS 8x8 scaling_list 覆盖");
        assert_eq!(list8.len(), 2, "transform_8x8_mode=true 时应解析 2 组 8x8");

        assert_eq!(
            list4[0],
            super::DEFAULT_SCALING_4X4_INTRA,
            "list0 useDefault 应回退到默认 Intra 4x4"
        );
        assert_eq!(list4[1], list4[0], "list1 absent 应回退到 list0");
        assert_eq!(
            list4[3],
            super::DEFAULT_SCALING_4X4_INTER,
            "list3 useDefault 应回退到默认 Inter 4x4"
        );
        assert_eq!(list4[4], list4[3], "list4 absent 应回退到 list3");

        assert!(
            list8[0].iter().all(|v| *v == 8),
            "list6 自定义常量 8 解析错误"
        );
        assert_eq!(
            list8[1],
            super::DEFAULT_SCALING_8X8_INTER,
            "list7 absent 应回退到默认 Inter 8x8"
        );
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

    fn write_scaling_list_use_default(bits: &mut Vec<bool>) {
        write_se(bits, -8);
    }

    fn write_scaling_list_constant_8(bits: &mut Vec<bool>, size: usize) {
        for _ in 0..size {
            write_se(bits, 0);
        }
    }

    fn build_pps_rbsp_with_custom_scaling_lists() -> Vec<u8> {
        let mut bits = Vec::<bool>::new();

        // 与基础 PPS 保持一致的最小字段.
        write_ue(&mut bits, 0); // pps_id
        write_ue(&mut bits, 0); // sps_id
        write_bit(&mut bits, true); // entropy_coding_mode_flag
        write_bit(&mut bits, false); // pic_order_present_flag
        write_ue(&mut bits, 0); // num_slice_groups_minus1
        write_ue(&mut bits, 0); // num_ref_idx_l0_default_active_minus1
        write_ue(&mut bits, 0); // num_ref_idx_l1_default_active_minus1
        write_bit(&mut bits, false); // weighted_pred_flag
        write_bits(&mut bits, 0, 2); // weighted_bipred_idc
        write_se(&mut bits, 0); // pic_init_qp_minus26
        write_se(&mut bits, 0); // pic_init_qs_minus26
        write_se(&mut bits, 0); // chroma_qp_index_offset
        write_bit(&mut bits, true); // deblocking_filter_control_present_flag
        write_bit(&mut bits, false); // constrained_intra_pred_flag
        write_bit(&mut bits, false); // redundant_pic_cnt_present_flag

        // 扩展字段.
        write_bit(&mut bits, true); // transform_8x8_mode_flag
        write_bit(&mut bits, true); // pic_scaling_matrix_present_flag

        // 共 8 组 list: 0..5 为 4x4, 6..7 为 8x8
        // list0 present + useDefault
        write_bit(&mut bits, true);
        write_scaling_list_use_default(&mut bits);
        // list1 absent -> fallback list0
        write_bit(&mut bits, false);
        // list2 absent -> fallback list1
        write_bit(&mut bits, false);
        // list3 present + useDefault
        write_bit(&mut bits, true);
        write_scaling_list_use_default(&mut bits);
        // list4 absent -> fallback list3
        write_bit(&mut bits, false);
        // list5 absent -> fallback list4
        write_bit(&mut bits, false);
        // list6 present + 常量 8
        write_bit(&mut bits, true);
        write_scaling_list_constant_8(&mut bits, 64);
        // list7 absent -> 默认 Inter 8x8
        write_bit(&mut bits, false);

        write_se(&mut bits, 0); // second_chroma_qp_index_offset
        write_bit(&mut bits, true); // rbsp_trailing_bits
        while !bits.len().is_multiple_of(8) {
            write_bit(&mut bits, false);
        }

        bits_to_bytes(&bits)
    }
}
