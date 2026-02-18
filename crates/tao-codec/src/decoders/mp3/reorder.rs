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

/// 短块重排序
pub fn reorder(granule: &Granule, xr: &mut XrSpectrum, _version: MpegVersion, sample_rate: u32) {
    if !granule.windows_switching_flag || granule.block_type != 2 {
        return;
    }

    let sr_idx = samplerate_index(sample_rate);
    let sfb_width = &SFB_WIDTH_SHORT[sr_idx];

    if granule.mixed_block_flag {
        // Mixed blocks: 前 2 个子带 (36 个样本) 是长块, 不重排
        // 从第 36 个样本开始的短块部分需要重排
        reorder_short_region(xr, sfb_width, 36);
    } else {
        // 纯短块: 全部重排
        reorder_short_region(xr, sfb_width, 0);
    }
}

/// 对短块区域执行重排序
///
/// `start_sample`: 短块区域起始位置 (mixed blocks 为 36, 纯短块为 0)
fn reorder_short_region(xr: &mut XrSpectrum, sfb_width: &[usize; 13], start_sample: usize) {
    let mut scratch = [0.0f32; 576];

    // 源数据顺序: [sfb0_w0, sfb0_w1, sfb0_w2, sfb1_w0, sfb1_w1, sfb1_w2, ...]
    // 目标: 每子带 18 样本 = [w0(6), w1(6), w2(6)]
    //
    // 每个 window 的频率线按 SFB 顺序排列, 总共 192 条/window.
    // 频率线 f (0..191) 映射到子带 sb = f/6, 子带内位置 pos = f%6

    let mut src_idx = start_sample;

    // 跳过 mixed block 长块部分对应的 SFB
    let start_sfb = if start_sample > 0 {
        // Mixed blocks: 长块占前 3 个 short SFBs (通常前 36 样本 = 3*4*3)
        // 找到 start_sample 对应的 SFB 起始位置
        let mut offset = 0;
        let mut sfb = 0;
        while sfb < 13 && offset + sfb_width[sfb] * 3 <= start_sample {
            offset += sfb_width[sfb] * 3;
            sfb += 1;
        }
        sfb
    } else {
        0
    };

    // 计算每个 window 内的频率线起始偏移
    let mut freq_offset = if start_sfb > 0 {
        sfb_width[..start_sfb].iter().sum::<usize>()
    } else {
        0
    };

    for &width in sfb_width.iter().skip(start_sfb).take(13 - start_sfb) {
        if width == 0 {
            continue;
        }

        for win in 0..3 {
            for s in 0..width {
                if src_idx >= 576 {
                    break;
                }
                let freq_line = freq_offset + s;
                let sb = freq_line / 6;
                let pos = freq_line % 6;
                let dst = sb * 18 + win * 6 + pos;
                if dst < 576 {
                    scratch[dst] = xr[src_idx];
                }
                src_idx += 1;
            }
        }
        freq_offset += width;
    }

    // 将重排后的数据复制回去 (仅短块区域)
    if start_sample > 0 {
        xr[start_sample..].copy_from_slice(&scratch[start_sample..]);
    } else {
        *xr = scratch;
    }
}
