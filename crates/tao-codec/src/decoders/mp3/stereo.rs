//! MP3 立体声处理 (Stereo Processing)
//!
//! 支持 MS Stereo 和 Intensity Stereo

use super::data::GranuleContext;
use super::header::{ChannelMode, Mp3Header};
use super::side_info::Granule;
use super::tables::SFB_WIDTH_LONG_44;

/// 立体声处理
pub fn process_stereo(
    gr: usize,
    header: &Mp3Header,
    granule_data: &mut [[GranuleContext; 2]; 2], // 传入整个数组以访问 L/R
    granules: &[[Granule; 2]; 2],                // 传入 SideInfo 中的 granules
) {
    // 仅 Joint Stereo 模式需要处理
    if header.mode != ChannelMode::JointStereo {
        return;
    }

    // mode_extension:
    // Bit 0: Intensity Stereo (0=off, 1=on)
    // Bit 1: MS Stereo (0=off, 1=on)

    let mode_ext = header.mode_extension;
    let intensity_stereo = (mode_ext & 0x1) != 0;
    let ms_stereo = (mode_ext & 0x2) != 0;

    // 如果都不开启, 不需要处理
    if !intensity_stereo && !ms_stereo {
        return;
    }

    // Rust borrow checker workaround:
    // 获取 gr 这一行的可变引用
    let row = &mut granule_data[gr];
    // split_at_mut(1) -> ([L], [R])
    let (l_slice, r_slice) = row.split_at_mut(1);
    let l_data = &mut l_slice[0];
    let r_data = &mut r_slice[0];

    let l_gr = &granules[gr][0];

    // 1. Intensity Stereo (IS)
    // IS 仅影响高频部分 (above r_gr.big_values*2).

    // 确定 IS 边界 (bound)
    // bound 取决于 window type.
    // 简单起见, 假设 Long blocks.

    let sfb_width = &SFB_WIDTH_LONG_44; // TODO: Select table

    if l_gr.windows_switching_flag && l_gr.block_type == 2 {
        // Short blocks IS
        // TODO: Implement Short block IS
    } else {
        // Long blocks IS
        // 查找 R 的截止点
        // 优化: 只需要计算一次

        // 实际上 IS 边界是由 Huffman 编码中 R channel 的零值区决定的。
        // 但为了简单起见，我们假设所有 R 为 0 的高频区域都是 IS。
        // 并通过扫描 R 数组找到非零边界。

        // 扫描 R 数组找到非零边界
        let mut r_limit = 576;
        while r_limit > 0 && r_data.xr[r_limit - 1] == 0.0 {
            r_limit -= 1;
        }

        // 1. MS Stereo (Low frequency part)
        if ms_stereo {
            for i in 0..r_limit {
                let m = l_data.xr[i];
                let s = r_data.xr[i];
                l_data.xr[i] = (m + s) * 0.70710678; // 1/sqrt(2)
                r_data.xr[i] = (m - s) * 0.70710678;
            }
        }

        // 2. Intensity Stereo (High frequency part)
        if intensity_stereo {
            // 从 r_limit 开始, 按 sfb 处理
            // 找到包含 r_limit 的 sfb
            let mut current_sfb = 0;
            let mut current_offset = 0;

            // Skip bands fully below r_limit
            while current_sfb < 21 && current_offset + sfb_width[current_sfb] <= r_limit {
                current_offset += sfb_width[current_sfb];
                current_sfb += 1;
            }

            // 如果 r_limit 在 sfb 中间, 该 sfb 属于 "mixed" 区域?
            // 通常 encoder 会对齐 sfb. 假设对齐.
            if current_offset < r_limit {
                current_offset += sfb_width[current_sfb];
                current_sfb += 1;
            }

            // IS Loop
            for sfb in current_sfb..21 {
                let width = sfb_width[sfb];
                if width == 0 {
                    continue;
                }

                // R channel scalefactor is IS position
                let is_pos = r_data.scalefac[sfb];

                // Illegal is_pos check
                if is_pos == 7 {
                    // Illegal in MPEG-1
                    // l = l, r = l
                    for i in 0..width {
                        let idx = current_offset + i;
                        if idx < 576 {
                            r_data.xr[idx] = l_data.xr[idx];
                        }
                    }
                } else {
                    let (kl, kr) = get_is_ratios(is_pos);

                    for i in 0..width {
                        let idx = current_offset + i;
                        if idx < 576 {
                            let m = l_data.xr[idx];
                            l_data.xr[idx] = m * kl;
                            r_data.xr[idx] = m * kr;
                        }
                    }
                }

                current_offset += width;
            }
        }
    }
}

// Precomputed IS ratios for MPEG-1 (is_pos 0..6)
// kL, kR = (cos(theta), sin(theta)) where theta = is_pos * PI / 12
fn get_is_ratios(is_pos: u8) -> (f32, f32) {
    match is_pos {
        0 => (1.0, 0.0),
        1 => (0.9659258, 0.2588190),
        2 => (0.8660254, 0.5),
        3 => (0.70710678, 0.70710678),
        4 => (0.5, 0.8660254),
        5 => (0.2588190, 0.9659258),
        6 => (0.0, 1.0),
        _ => (1.0, 0.0), // Illegal or reserved, fallback to Left
    }
}
