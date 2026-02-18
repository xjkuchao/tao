//! MP3 解码数据结构
//!
//! 存储中间解码结果 (Scalefactors, IS, XR)

/// 频谱系数 (Integer Samples)
/// 576 个系数 (Long/Short blocks)
pub type IsSpectrum = [i32; 576];

/// 反量化后的频谱 (Requantized Samples)
pub type XrSpectrum = [f32; 576];

/// 比例因子 (Scalefactors)
/// Long blocks: 21 (0-20)
/// Short blocks: 12 bands * 3 windows = 36 (0-35)
/// Mixed blocks: 8 (Long) + 9*3 (Short) = 35
/// Max size: 40 (safe margin)
pub type Scalefactors = [u8; 40];

/// Granule 解码上下文
/// 注意: overlap 缓冲区不在此结构中, 而是在解码器状态中
/// 因为 overlap 需要跨 granule 和跨帧保持
#[derive(Debug, Clone)]
pub struct GranuleContext {
    pub scalefac: Scalefactors,
    pub is: IsSpectrum,
    pub xr: XrSpectrum,
    /// rzero: 最后一个非零 Huffman 样本之后的索引 (big_values + count1 样本数)
    pub rzero: usize,
}

impl Default for GranuleContext {
    fn default() -> Self {
        Self {
            scalefac: [0; 40],
            is: [0; 576],
            xr: [0.0; 576],
            rzero: 0,
        }
    }
}
