//! 8x8 块级 DCT 系数解码
//!
//! Intra 和 Inter 块的 AC/DC 系数解码, 包括 AC/DC 预测.

use super::Mpeg4Decoder;
use super::bitreader::BitReader;
use super::types::PredictorDirection;
use super::vlc::{INTER_AC_VLC, INTRA_AC_VLC, decode_ac_vlc, decode_intra_dc_vlc};

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
    let (dc_pred, direction) = decoder.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);
    let actual_dc = dc_pred.wrapping_add(dc_diff);
    block[0] = actual_dc as i32 * dc_scaler as i32;

    // 2. AC 系数
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
                    block[scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
    }

    // 3. AC 预测
    if ac_pred_flag {
        match direction {
            PredictorDirection::Vertical => {
                let c_idx = match block_idx {
                    0 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                    1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                    2 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                    3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                    4 | 5 => {
                        decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, block_idx)
                    }
                    _ => None,
                };
                if let Some(idx) = c_idx {
                    let pred_ac = decoder.predictor_cache[idx];
                    for i in 1..8 {
                        block[scan[i]] = block[scan[i]].wrapping_add(pred_ac[i] as i32);
                    }
                }
            }
            PredictorDirection::Horizontal => {
                let a_idx = match block_idx {
                    0 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 1),
                    1 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                    2 => decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 3),
                    3 => decoder.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 2),
                    4 | 5 => {
                        decoder.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, block_idx)
                    }
                    _ => None,
                };
                if let Some(idx) = a_idx {
                    let pred_ac = decoder.predictor_cache[idx];
                    for i in 1..8 {
                        block[scan[i * 8]] = block[scan[i * 8]].wrapping_add(pred_ac[7 + i] as i32);
                    }
                }
            }
            _ => {}
        }
    }

    // 4. 更新预测器缓存
    let cache_pos = (mb_y as usize * decoder.mb_stride + mb_x as usize) * 6 + block_idx;
    if let Some(cache) = decoder.predictor_cache.get_mut(cache_pos) {
        cache[0] = actual_dc;
        for i in 1..8 {
            cache[i] = block[scan[i]] as i16;
        }
        for i in 1..8 {
            cache[7 + i] = block[scan[i * 8]] as i16;
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
