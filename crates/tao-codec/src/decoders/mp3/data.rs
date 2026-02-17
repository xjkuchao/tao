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
#[derive(Debug, Clone)]
pub struct GranuleContext {
    pub scalefac: Scalefactors,
    pub is: IsSpectrum,
    pub xr: XrSpectrum,
    /// IMDCT 重叠缓冲区 (每个 channel 独立)
    /// 32 subbands * 18 samples
    pub overlap: [[f32; 18]; 32],
}

impl Default for GranuleContext {
    fn default() -> Self {
        Self {
            scalefac: [0; 40],
            is: [0; 576],
            xr: [0.0; 576],
            overlap: [[0.0; 18]; 32],
        }
    }
}
