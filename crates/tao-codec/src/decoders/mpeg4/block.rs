//! 8x8 块级 DCT 系数解码
//!
//! Intra 和 Inter 块的 AC/DC 系数解码, 包括 AC/DC 预测.

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::tables::{ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN};
use super::types::PredictorDirection;
use super::vlc::{INTER_AC_VLC, INTRA_AC_VLC, decode_ac_vlc, decode_intra_dc_vlc};

const COEFF_MIN: i32 = -2048;
const COEFF_MAX: i32 = 2047;

fn select_ac_pred_scan(
    ac_pred_flag: bool,
    direction: PredictorDirection,
    default_scan: &[usize; 64],
) -> &[usize; 64] {
    if !ac_pred_flag {
        return default_scan;
    }

    match direction {
        PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,
        PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,
        PredictorDirection::None => default_scan,
    }
}

/// 解码 Intra 块的 DCT 系数
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_intra_block_vlc(
    reader: &mut BitReader,
    plane: usize,
    mb_x: u32,
    mb_y: u32,
    block_idx: usize,
    ac_pred_flag: bool,
    ac_coded: bool,
    decoder: &mut Mpeg4Decoder,
    scan: &[usize; 64],
) -> Option<[i32; 64]> {
    let mut block = [0i32; 64];
    let is_luma = plane == 0;

    // 1. DC 系数
    let dc_scaler = decoder.get_dc_scaler(is_luma);
    let dc_diff = if decoder.use_intra_dc_vlc() {
        decode_intra_dc_vlc(reader, is_luma)?
    } else {
        0
    };
    let (dc_pred_quant, direction) =
        decoder.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);

    // DC 预测在量化域进行，预测器直接存储量化后的值
    let actual_dc_quant = dc_pred_quant.wrapping_add(dc_diff);
    // 反量化：乘以 dc_scaler
    let actual_dc = actual_dc_quant as i32 * dc_scaler as i32;

    block[0] = actual_dc;

    // 2. AC 系数
    let ac_scan = select_ac_pred_scan(ac_pred_flag, direction, scan);

    if ac_coded {
        let start = if decoder.use_intra_dc_vlc() { 1 } else { 0 };
        let mut pos = start;
        while pos < 64 {
            match decode_ac_vlc(reader, INTRA_AC_VLC, true) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[ac_scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
    }

    // 3. AC 预测 (需检查 slice 边界, 跨 slice 的邻居 AC 不可用)
    if ac_pred_flag {
        match direction {
            PredictorDirection::Vertical => {
                // Vertical 预测使用上方邻居
                let (nb_mx, nb_my) = match block_idx {
                    0 | 1 | 4 | 5 => (mb_x as usize, mb_y.wrapping_sub(1) as usize),
                    _ => (mb_x as usize, mb_y as usize), // block 2, 3: 同 MB 内
                };
                let in_slice =
                    matches!(block_idx, 2 | 3) || decoder.is_in_current_slice(nb_mx, nb_my);
                if in_slice {
                    let c_idx = match block_idx {
                        0 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                        1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                        2 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                        4 | 5 => decoder.get_neighbor_block_idx(
                            mb_x as isize,
                            mb_y as isize - 1,
                            block_idx,
                        ),
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = decoder.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i];
                            let value = block[idx] + pred_ac[i] as i32;
                            block[idx] = value.clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
            }
            PredictorDirection::Horizontal => {
                // Horizontal 预测使用左方邻居
                let (nb_mx, nb_my) = match block_idx {
                    0 | 2 | 4 | 5 => (mb_x.wrapping_sub(1) as usize, mb_y as usize),
                    _ => (mb_x as usize, mb_y as usize), // block 1, 3: 同 MB 内
                };
                let in_slice =
                    matches!(block_idx, 1 | 3) || decoder.is_in_current_slice(nb_mx, nb_my);
                if in_slice {
                    let a_idx = match block_idx {
                        0 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 1),
                        1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        2 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 3),
                        3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 2),
                        4 | 5 => decoder.get_neighbor_block_idx(
                            mb_x as isize - 1,
                            mb_y as isize,
                            block_idx,
                        ),
                        _ => None,
                    };
                    if let Some(idx) = a_idx {
                        let pred_ac = decoder.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i * 8];
                            let value = block[idx] + pred_ac[7 + i] as i32;
                            block[idx] = value.clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 4. 更新预测器缓存
    // 注意: 缓存存储量化域的 DC 值 (actual_dc_quant)
    let cache_pos = (mb_y as usize * decoder.mb_stride + mb_x as usize) * 6 + block_idx;
    if let Some(cache) = decoder.predictor_cache.get_mut(cache_pos) {
        cache[0] = actual_dc_quant;
        for i in 1..8 {
            cache[i] = block[ac_scan[i]] as i16;
        }
        for i in 1..8 {
            cache[7 + i] = block[ac_scan[i * 8]] as i16;
        }
    }

    Some(block)
}

/// 解码 Inter 块的 DCT 系数
pub(super) fn decode_inter_block_vlc(
    reader: &mut BitReader,
    scan: &[usize; 64],
) -> Option<[i32; 64]> {
    let mut block = [0i32; 64];
    let mut pos = 0;
    while pos < 64 {
        match decode_ac_vlc(reader, INTER_AC_VLC, false) {
            Ok(None) => break,
            Ok(Some((last, run, level))) => {
                pos += run as usize;
                if pos >= 64 {
                    break;
                }
                block[scan[pos]] = level as i32;
                pos += 1;
                if last {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    Some(block)
}

#[cfg(test)]
mod tests {
    use super::super::bitreader::BitReader;
    use super::super::gmc::GmcParameters;
    use super::super::tables::{
        ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN, STD_INTER_QUANT_MATRIX,
        STD_INTRA_QUANT_MATRIX, ZIGZAG_SCAN,
    };
    use super::super::types::{MacroblockInfo, MotionVector};
    use super::*;
    use tao_core::PixelFormat;

    fn create_decoder_for_test() -> Mpeg4Decoder {
        Mpeg4Decoder {
            width: 32,
            height: 32,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            dpb: Vec::new(),
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: vec![[0; 15]; 2 * 2 * 6],
            mv_cache: vec![[MotionVector::default(); 4]; 4],
            ref_mv_cache: vec![[MotionVector::default(); 4]; 4],
            mb_info: vec![MacroblockInfo::default(); 4],
            mb_stride: 2,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 7,
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

    fn set_predictor_cache(
        decoder: &mut Mpeg4Decoder,
        mb_x: usize,
        mb_y: usize,
        block_idx: usize,
        dc: i16,
        row: i16,
        col: i16,
    ) {
        let pos = (mb_y * decoder.mb_stride + mb_x) * 6 + block_idx;
        let cache = decoder
            .predictor_cache
            .get_mut(pos)
            .expect("预测缓存索引应存在");
        cache[0] = dc;
        for i in 1..8 {
            cache[i] = row;
            cache[7 + i] = col;
        }
    }

    #[test]
    fn test_ac_prediction_vertical_direction() {
        let mut decoder = create_decoder_for_test();
        let mb_x = 1;
        let mb_y = 1;

        set_predictor_cache(&mut decoder, 0, 1, 1, 0, 0, 0);
        set_predictor_cache(&mut decoder, 0, 0, 3, 0, 0, 0);
        set_predictor_cache(&mut decoder, 1, 0, 2, 100, 3000, 0);

        let mut reader = BitReader::new(&[]);
        let block = decode_intra_block_vlc(
            &mut reader,
            0,
            mb_x as u32,
            mb_y as u32,
            0,
            true,
            false,
            &mut decoder,
            &ZIGZAG_SCAN,
        )
        .expect("应能解码 Intra 块");

        for &scan_idx in &ALTERNATE_HORIZONTAL_SCAN[1..8] {
            assert_eq!(block[scan_idx], COEFF_MAX, "垂直预测应使用交替水平扫描");
        }
    }

    #[test]
    fn test_ac_prediction_horizontal_direction() {
        let mut decoder = create_decoder_for_test();
        let mb_x = 1;
        let mb_y = 1;

        set_predictor_cache(&mut decoder, 0, 1, 1, 100, 0, 3000);
        set_predictor_cache(&mut decoder, 0, 0, 3, 0, 0, 0);
        set_predictor_cache(&mut decoder, 1, 0, 2, 0, 0, 0);

        let mut reader = BitReader::new(&[]);
        let block = decode_intra_block_vlc(
            &mut reader,
            0,
            mb_x as u32,
            mb_y as u32,
            0,
            true,
            false,
            &mut decoder,
            &ZIGZAG_SCAN,
        )
        .expect("应能解码 Intra 块");

        for i in 1..8 {
            let idx = ALTERNATE_VERTICAL_SCAN[i * 8];
            assert_eq!(block[idx], COEFF_MAX, "水平预测应使用交替垂直扫描");
        }
    }

    #[test]
    fn test_ac_prediction_clipping() {
        let mut decoder = create_decoder_for_test();
        let mb_x = 1;
        let mb_y = 1;

        set_predictor_cache(&mut decoder, 0, 1, 1, 0, 0, 0);
        set_predictor_cache(&mut decoder, 0, 0, 3, 0, 0, 0);
        set_predictor_cache(&mut decoder, 1, 0, 2, 100, -3000, 0);

        let mut reader = BitReader::new(&[]);
        let block = decode_intra_block_vlc(
            &mut reader,
            0,
            mb_x as u32,
            mb_y as u32,
            0,
            true,
            false,
            &mut decoder,
            &ZIGZAG_SCAN,
        )
        .expect("应能解码 Intra 块");

        for &scan_idx in &ALTERNATE_HORIZONTAL_SCAN[1..8] {
            assert_eq!(block[scan_idx], COEFF_MIN, "AC 预测结果应裁剪到范围内");
        }
    }
}
