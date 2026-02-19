//! MP3 抗混叠处理 (Alias Reduction)
//!
//! 在长块处理中, 对子带边界进行"蝴蝶"运算以消除混叠伪影.
//! 短块不做抗混叠; 混合块仅对长块部分 (前 2 个子带边界) 做抗混叠.

use super::header::MpegVersion;
use super::side_info::Granule;

/// 抗混叠系数 (Butterfly Coefficients)
/// cs[i] = 1 / sqrt(1 + ci^2)
/// ca[i] = ci / sqrt(1 + ci^2)
#[allow(clippy::excessive_precision)]
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

#[allow(clippy::excessive_precision)]
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
///
/// `rzero` 是最后一个非零频谱样本之后的索引.
/// 仅对包含非零数据的子带 (以及相邻 1 个子带) 进行蝴蝶运算,
/// 避免将能量泄漏到本应为零的高频子带中.
pub fn alias_reduction(
    granule: &Granule,
    xr: &mut [f32; 576],
    rzero: &mut usize,
    _version: MpegVersion,
    _sample_rate: u32,
) {
    // 纯短块不做抗混叠
    if granule.windows_switching_flag && granule.block_type == 2 && !granule.mixed_block_flag {
        return;
    }

    let sb_limit = if granule.windows_switching_flag && granule.mixed_block_flag {
        2
    } else {
        32
    };

    // 仅处理包含非零数据的子带范围,
    // 多处理 2 个子带以覆盖蝴蝶运算可能"泄漏"到的相邻子带
    let sb_rzero = *rzero / 18;
    let max_sb = sb_limit.min(sb_rzero + 2).min(32);
    *rzero = max_sb * 18;

    for sb in 1..max_sb {
        let sb_start = sb * 18;
        let prev_sb_start = (sb - 1) * 18;

        for i in 0..8 {
            let idx1 = prev_sb_start + 17 - i;
            let idx2 = sb_start + i;

            let x1 = xr[idx1];
            let x2 = xr[idx2];

            xr[idx1] = x1 * CS[i] - x2 * CA[i];
            xr[idx2] = x2 * CS[i] + x1 * CA[i];
        }
    }
}
