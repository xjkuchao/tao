//! MP3 重排序 (Reordering)
//!
//! 针对短块 (Short Blocks), 将频率系数从子带交织顺序重排为窗口分组顺序.
//! Huffman/Requantize 输出: Band0[W0, W1, W2], Band1[W0, W1, W2]...
//! Reorder 输出: W0[Band0..Band12], W1[Band0..Band12], W2[Band0..Band12]

use super::data::XrSpectrum;
use super::header::MpegVersion;
use super::side_info::Granule;
use super::tables::SFB_WIDTH_SHORT_44;

/// 短块重排序
pub fn reorder(granule: &Granule, xr: &mut XrSpectrum, _version: MpegVersion, _sample_rate: u32) {
    if !granule.windows_switching_flag || granule.block_type != 2 {
        return;
    }

    if granule.mixed_block_flag {
        // Mixed blocks: Lower bands (0-1) are long, upper are short
        // TODO: Implement mixed block reordering
        return;
    }

    // Pure short blocks
    // 44.1kHz widths
    // ISO 11172-3 Table B.8 (44.1kHz):
    // Long: 4,4,4,4,4,4,6,6,8,8,10,12,16,20,24,28,34,42,50,54,76
    // Short: 4,4,4,4,4,4,6,6,8,8,10,12,18 (Total 13? Last is remaining?)

    // Reordering algorithm:
    // Src: [Band0_W0, Band0_W1, Band0_W2, Band1_W0...]
    // Dst: [W0_Band0, W0_Band1... W1_Band0...]
    // 实际上是 3 个独立的 192-sample 块.
    // W0: 0..191, W1: 192..383, W2: 384..575

    // Since we don't have correct table yet, let's just implement the logic with a placeholder table
    // and use a scratch buffer.

    let mut scratch = [0.0f32; 576];
    // Copy original to scratch
    scratch.copy_from_slice(xr);

    // Use the same table as requantize (need to unify!)
    let sfb_width = &SFB_WIDTH_SHORT_44; // Use tables.rs
    // We need to pass the correct table or access it from a common place.

    let mut src_idx = 0;
    let mut dst_idx = [0, 192, 384]; // Start of W0, W1, W2

    for band in 0..13 {
        // 13 bands?
        let width = sfb_width[band];
        if width == 0 {
            continue;
        }

        for win in 0..3 {
            for _ in 0..width {
                if src_idx < 576 {
                    xr[dst_idx[win]] = scratch[src_idx];
                    src_idx += 1;
                    dst_idx[win] += 1;
                }
            }
        }
    }
}
