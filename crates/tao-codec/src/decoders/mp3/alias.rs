//! MP3 抗混叠处理 (Alias Reduction)
//!
//! 在长块处理中，对子带边界进行“蝴蝶”运算以消除混叠伪影。

use super::header::MpegVersion;
use super::side_info::Granule;

/// 抗混叠系数 (Butterfly Coefficients)
/// cs[i] = 1 / sqrt(1 + ci^2)
/// ca[i] = ci / sqrt(1 + ci^2)
const CS: [f32; 8] = [
    0.8574929257,
    0.8817419973,
    0.9496286491,
    0.9833145925,
    0.9955178161,
    0.9991605582,
    0.9998991952,
    0.9999931551,
];

const CA: [f32; 8] = [
    -0.5144957554,
    -0.4717319684,
    -0.3133774542,
    -0.1819131996,
    -0.0945741925,
    -0.0409655829,
    -0.0141985686,
    -0.0036999747,
];

/// 抗混叠处理
pub fn alias_reduction(
    granule: &Granule,
    xr: &mut [f32; 576],
    _version: MpegVersion,
    _sample_rate: u32,
) {
    // 仅对长块 (Long Blocks) 进行抗混叠
    // 混合块 (Mixed Blocks) 的长块部分也需要处理
    if granule.windows_switching_flag && granule.block_type == 2 && !granule.mixed_block_flag {
        return;
    }

    // 确定处理的子带数量
    // Mixed blocks: 2 subbands (0, 1) are long blocks
    // Long blocks: 32 subbands (0..31) -> 31 boundaries (1..31)

    let max_sb = if granule.mixed_block_flag {
        2 // Process boundaries 1 (between sb0 and sb1) ? No.
    // Spec: "In mixed blocks... the first 2 subbands are long blocks... the remaining... short"
    // Alias reduction is done for the long block part.
    // So we process boundary between sb0 and sb1?
    // And boundary between sb1 and sb2?
    // "Alias reduction is not applied to the boundary between the last long block subband and the first short block subband"
    // So only boundary 1 (between 0 and 1).
    } else {
        32
    };

    // Alias reduction loops over subbands sb = 1 to 31
    // (Processing boundary between sb-1 and sb)

    for sb in 1..max_sb {
        let sb_start = sb * 18;
        let prev_sb_start = (sb - 1) * 18;

        for i in 0..8 {
            let idx1 = prev_sb_start + 17 - i;
            let idx2 = sb_start + i;

            let x1 = xr[idx1];
            let x2 = xr[idx2];

            let cs = CS[i];
            let ca = CA[i];

            // Butterfly
            xr[idx1] = x1 * cs - x2 * ca;
            xr[idx2] = x2 * cs + x1 * ca;
        }
    }
}
