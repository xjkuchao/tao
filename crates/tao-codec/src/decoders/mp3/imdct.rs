//! MP3 IMDCT (Inverse Modified Discrete Cosine Transform)
//!
//! 实现 18点 (Long Block) 和 6点 (Short Block) IMDCT, 窗口加权, 重叠相加.

use std::f32::consts::PI;
use std::sync::OnceLock;
use super::side_info::Granule;

/// IMDCT 窗口表 (36 点)
/// 包含 4 种窗口类型: 0=Normal, 1=Start, 2=Short, 3=Stop
static IMDCT_WINDOWS: OnceLock<[[f32; 36]; 4]> = OnceLock::new();

fn get_imdct_windows() -> &'static [[f32; 36]; 4] {
    IMDCT_WINDOWS.get_or_init(|| {
        let mut windows = [[0.0; 36]; 4];
        
        // Block Type 0: Normal
        for i in 0..36 {
            windows[0][i] = (PI / 36.0 * (i as f32 + 0.5)).sin();
        }
        
        // Block Type 1: Start
        for i in 0..18 {
            windows[1][i] = (PI / 36.0 * (i as f32 + 0.5)).sin();
        }
        for i in 18..24 {
            windows[1][i] = (PI / 36.0 * ((i - 18) as f32 + 0.5)).sin(); // Incorrect formula?
            // FFmpeg:
            // 0..18: sin(pi/36 * (i+0.5))
            // 18..24: 1.0 (window[i] = 1.0)
            // 24..30: sin(pi/12 * (i-18+0.5)) -> No.
            // Let's use standard formula.
            // Start block: 
            // 0-17: Normal window (sin(pi/36...))
            // 18-23: 1.0
            // 24-29: Short window (sin(pi/12 * (i-24+0.5)))
            // 30-35: 0.0
        }
        // Correcting Type 1 (Start)
        for i in 0..18 { windows[1][i] = (PI / 36.0 * (i as f32 + 0.5)).sin(); }
        for i in 18..24 { windows[1][i] = 1.0; }
        for i in 24..30 { windows[1][i] = (PI / 12.0 * ((i - 24) as f32 + 0.5)).sin(); }
        for i in 30..36 { windows[1][i] = 0.0; }
        
        // Block Type 3: Stop
        // 0-5: 0.0
        // 6-11: Short window (sin(pi/12 * (i-6+0.5)))
        // 12-17: 1.0
        // 18-35: Normal window (sin(pi/36 * (i+0.5)))
        for i in 0..6 { windows[3][i] = 0.0; }
        for i in 6..12 { windows[3][i] = (PI / 12.0 * ((i - 6) as f32 + 0.5)).sin(); }
        for i in 12..18 { windows[3][i] = 1.0; }
        for i in 18..36 { windows[3][i] = (PI / 36.0 * (i as f32 + 0.5)).sin(); }
        
        // Block Type 2: Short
        // 0-5: 0.0
        // 6-11: Short window
        // 12-17: 0.0 -> Wait, short blocks are 3x12 samples.
        // The window applied here is for the whole granule (36 samples)?
        // No, short blocks have 3 overlapping windows of 12 samples each.
        // But the output of IMDCT is 3x12.
        // The "window" stored here is for overlap-add of the *whole* granule with previous/next?
        // Actually, for short blocks, we compute 3 small IMDCTs, window them, and overlap-add WITHIN the granule.
        // The resulting 18 samples (0..17) are overlapped with previous granule.
        // The samples 18..35 are stored for next.
        // So Type 2 here is effectively:
        // 0-5: 0.0
        // 6-11: sin(pi/12 * (i-6+0.5))
        // 12-35: 0.0?
        // No, this table is used for Long Blocks logic or unified logic?
        // Let's implement specific logic for Short blocks.
        // This table will be used for Normal/Start/Stop.
        // For Short blocks, we need a 12-point window.
        
        windows
    })
}

/// Short Block Window (12 points)
static SHORT_WINDOW: OnceLock<[f32; 12]> = OnceLock::new();
fn get_short_window() -> &'static [f32; 12] {
    SHORT_WINDOW.get_or_init(|| {
        let mut w = [0.0; 12];
        for i in 0..12 {
            w[i] = (PI / 12.0 * (i as f32 + 0.5)).sin();
        }
        w
    })
}

/// 执行 IMDCT, Windowing, Overlap-Add
/// 
/// 输入: xr[576] (频域)
/// 输出: output[576] (时域, 32 subbands * 18 samples)
/// 状态: overlap[2][32][18] (每个 channel 一个)
pub fn imdct(
    granule: &Granule,
    xr: &[f32; 576],
    overlap: &mut [[f32; 18]; 32],
    output: &mut [f32; 576],
) {
    let windows = get_imdct_windows();
    let short_win = get_short_window();
    let block_type = if granule.windows_switching_flag { granule.block_type } else { 0 };
    
    // Process 32 subbands
    for sb in 0..32 {
        let sb_idx = sb * 18;
        let input_chunk = &xr[sb_idx..sb_idx+18]; // 18 freq samples
        
        let mut raw_out = [0.0; 36]; // IMDCT output (36 samples)
        
        // Determine if this subband is short block
        // Mixed blocks: sb 0, 1 are long, others are short (if block_type=2)
        let is_short = granule.windows_switching_flag && granule.block_type == 2 && 
                       (!granule.mixed_block_flag || sb >= 2);
                       
        if is_short {
            // Short Blocks (3 * 12 samples, overlapping)
            // Input: 18 coefficients.
            // Sorted as: W0[0..5], W1[0..5], W2[0..5] (after reordering)
            // Reordering was done in Phase 3.
            // So input_chunk[0..6] is W0, [6..12] is W1, [12..18] is W2.
            
            // Output of each 6-point IMDCT is 12 samples.
            // They overlap:
            // W0: 0..11
            // W1: 6..17
            // W2: 12..23
            // Total length 24?
            // Wait, Short blocks result in 18 valid samples (plus overlap).
            // Normal IMDCT 18->36.
            // Short IMDCTs produce 12 samples each.
            // 3 windows.
            // Placement in 36-sample buffer:
            // W0 starts at 0? No.
            // ISO 11172-3:
            // "The 18 sample output... is calculated as follows:"
            // For k=0..5, y[k] = 0
            // For k=6..11, y[k] = w0[k-6] * s0[k-6]
            // For k=12..17, y[k] = w0[k-6]*s0[k-6] + w1[k-12]*s1[k-12]
            // ...
            // The standard defines precise overlapping.
            // Let's implement 3x 6-point IMDCT.
            
            let mut w_out = [[0.0; 12]; 3];
            
            for w in 0..3 {
                let win_in = &input_chunk[w*6..(w+1)*6];
                imdct6(win_in, &mut w_out[w]);
                // Windowing
                for i in 0..12 {
                    w_out[w][i] *= short_win[i];
                }
            }
            
            // Overlap and place in raw_out (36 samples)
            // raw_out is initialized to 0.0
            // Window 0: start at 6?
            // Spec:
            // z[0..5] = 0
            // z[6..11] = s[0][0..5]
            // z[12..17] = s[0][6..11] + s[1][0..5]
            // z[18..23] = s[1][6..11] + s[2][0..5]
            // z[24..29] = s[2][6..11]
            // z[30..35] = 0
            
            // Note: s[w] is w_out[w]
            for i in 0..6 {
                raw_out[6+i] += w_out[0][i];
                raw_out[12+i] += w_out[0][6+i];
                
                raw_out[12+i] += w_out[1][i];
                raw_out[18+i] += w_out[1][6+i];
                
                raw_out[18+i] += w_out[2][i];
                raw_out[24+i] += w_out[2][6+i];
            }
            
        } else {
            // Long Blocks (18-point IMDCT)
            imdct18(input_chunk, &mut raw_out);
            
            // Windowing
            let win_idx = if granule.windows_switching_flag && granule.block_type == 2 && granule.mixed_block_flag && sb < 2 {
                 0 // Normal window for long part of mixed block
            } else {
                 block_type as usize
            };
            
            let win = &windows[win_idx];
            for i in 0..36 {
                raw_out[i] *= win[i];
            }
        }
        
        // Overlap-Add
        // Output 18 samples = raw_out[0..18] + prev_overlap
        // Next overlap = raw_out[18..36]
        
        for i in 0..18 {
            output[sb * 18 + i] = raw_out[i] + overlap[sb][i];
            overlap[sb][i] = raw_out[18 + i];
        }
    }
}

/// 6-point IMDCT
/// Input: 6 freq samples
/// Output: 12 time samples
fn imdct6(input: &[f32], output: &mut [f32]) {
    // Naive implementation or optimized? 6 is small.
    // Formula: x[i] = sum(k=0..5) X[k] * cos(pi/12 * (2i + 1 + 6) * (2k + 1))
    // i = 0..11
    
    for i in 0..12 {
        let mut sum = 0.0;
        for k in 0..6 {
            let angle = PI / 12.0 * (2.0 * i as f32 + 7.0) * (2.0 * k as f32 + 1.0);
            sum += input[k] * angle.cos();
        }
        output[i] = sum * (1.0 / 6.0);
    }
}

/// 18-point IMDCT
/// Input: 18 freq samples
/// Output: 36 time samples
fn imdct18(input: &[f32], output: &mut [f32]) {
    // Formula: x[i] = sum(k=0..17) X[k] * cos(pi/36 * (2i + 1 + 18) * (2k + 1))
    // i = 0..35
    
    // Using Fast DCT (Lee's algorithm) or simple O(N^2) since N=18?
    // 18*36 = 648 ops per subband * 32 subbands = 20k ops. Very fast.
    // Optimization: Precompute cosines.
    
    for i in 0..36 {
        let mut sum = 0.0;
        for k in 0..18 {
            let angle = PI / 36.0 * (2.0 * i as f32 + 19.0) * (2.0 * k as f32 + 1.0);
            sum += input[k] * angle.cos();
        }
        output[i] = sum * (1.0 / 18.0);
    }
}
