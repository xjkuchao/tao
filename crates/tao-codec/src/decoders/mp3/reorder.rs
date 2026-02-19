//! MP3 重排序 (Reordering)
//!
//! 针对短块 (Short Blocks), 将频率系数从 Huffman 输出顺序重排为 IMDCT 输入顺序.
//!
//! Huffman 输出顺序: Band0[W0, W1, W2], Band1[W0, W1, W2], ...
//! IMDCT 期望顺序: 每个子带 18 个样本 = [W0(6), W1(6), W2(6)]
//!
//! 重排算法: 将每个 SFB 的 (band, window) 样本映射到 (subband, window, position) 布局.

use super::data::XrSpectrum;
use super::header::MpegVersion;
use super::side_info::Granule;
use super::tables::{SFB_WIDTH_SHORT, samplerate_index};

#[inline]
fn mixed_long_subbands(sample_rate: u32) -> usize {
    // 对齐 minimp3/FFmpeg:
    // 8kHz (MPEG-2.5) mixed block 需保留前 4 个长块子带, 其他采样率为 2.
    if sample_rate == 8000 { 4 } else { 2 }
}

/// 短块重排序
pub fn reorder(
    granule: &Granule,
    xr: &mut XrSpectrum,
    rzero: &mut usize,
    _version: MpegVersion,
    sample_rate: u32,
) {
    if !granule.windows_switching_flag || granule.block_type != 2 {
        return;
    }

    let sr_idx = samplerate_index(sample_rate);
    let sfb_width = &SFB_WIDTH_SHORT[sr_idx];

    if granule.mixed_block_flag {
        let long_sb = mixed_long_subbands(sample_rate);
        let start = long_sb * 18;
        // Mixed blocks: 前 long_sb 个子带是长块, 从 start 开始重排短块部分.
        let end = reorder_short_region(xr, sfb_width, start, *rzero);
        *rzero = (*rzero).max(end);
    } else {
        // 纯短块: 全部重排
        let end = reorder_short_region(xr, sfb_width, 0, *rzero);
        *rzero = (*rzero).max(end);
    }
}

/// 对短块区域执行重排序
///
/// `start_sample`: 短块区域起始位置 (mixed blocks 为 36, 纯短块为 0)
fn reorder_short_region(
    xr: &mut XrSpectrum,
    sfb_width: &[usize; 13],
    start_sample: usize,
    rzero: usize,
) -> usize {
    let mut scratch = *xr;

    let mut start_sfb = 0usize;
    let mut accum = 0usize;
    while start_sfb < 13 && accum + sfb_width[start_sfb] * 3 <= start_sample {
        accum += sfb_width[start_sfb] * 3;
        start_sfb += 1;
    }

    let mut src = start_sample;
    let mut dst = start_sample;

    for &width in sfb_width.iter().skip(start_sfb) {
        if width == 0 || src + 3 * width > 576 {
            break;
        }
        if src >= rzero {
            break;
        }

        let win0 = &xr[src..src + width];
        let win1 = &xr[src + width..src + 2 * width];
        let win2 = &xr[src + 2 * width..src + 3 * width];

        for i in 0..width {
            if dst + 2 >= 576 {
                break;
            }
            scratch[dst] = win0[i];
            scratch[dst + 1] = win1[i];
            scratch[dst + 2] = win2[i];
            dst += 3;
        }

        src += 3 * width;
    }

    *xr = scratch;
    dst
}
