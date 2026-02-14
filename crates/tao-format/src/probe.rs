//! 格式探测.
//!
//! 通过分析文件头部数据和文件扩展名, 自动识别容器格式.

use crate::format_id::FormatId;

/// 探测置信度
///
/// 数值越高, 表示对格式判断越有信心.
pub type ProbeScore = u32;

/// 最低探测分数 (仅根据扩展名)
pub const SCORE_EXTENSION: ProbeScore = 50;

/// 中等探测分数 (MIME 类型匹配)
pub const SCORE_MIME: ProbeScore = 75;

/// 最高探测分数 (魔数完全匹配)
pub const SCORE_MAX: ProbeScore = 100;

/// 探测结果
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// 识别出的格式
    pub format_id: FormatId,
    /// 置信度分数
    pub score: ProbeScore,
}

/// 格式探测器 trait
///
/// 每种格式的解封装器可以实现此 trait 以支持自动格式识别.
pub trait FormatProbe {
    /// 根据文件头部数据探测格式
    ///
    /// # 参数
    /// - `data`: 文件开头的若干字节 (通常 4KB ~ 32KB)
    /// - `filename`: 文件名 (可选, 用于扩展名匹配)
    ///
    /// # 返回
    /// - `Some(score)`: 探测成功, 返回置信度
    /// - `None`: 不是此格式
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore>;

    /// 获取此探测器对应的格式标识
    fn format_id(&self) -> FormatId;
}
