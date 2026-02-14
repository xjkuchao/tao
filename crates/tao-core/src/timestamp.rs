//! 时间戳类型, 用于表示媒体流中的时间点.
//!
//! 对标 FFmpeg 中基于 `time_base` 的时间戳系统.

use crate::rational::Rational;
use std::fmt;

/// 表示"未定义"的时间戳值
pub const NOPTS_VALUE: i64 = i64::MIN;

/// 时间戳
///
/// 包含一个整数值和对应的时间基 (time_base).
/// 实际时间 (秒) = pts * time_base.num / time_base.den.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// 时间戳值, `NOPTS_VALUE` 表示未定义
    pub pts: i64,
    /// 时间基
    pub time_base: Rational,
}

impl Timestamp {
    /// 创建新的时间戳
    pub const fn new(pts: i64, time_base: Rational) -> Self {
        Self { pts, time_base }
    }

    /// 创建未定义的时间戳
    pub const fn none() -> Self {
        Self {
            pts: NOPTS_VALUE,
            time_base: Rational::UNDEFINED,
        }
    }

    /// 判断时间戳是否有效 (非 NOPTS_VALUE)
    pub const fn is_valid(&self) -> bool {
        self.pts != NOPTS_VALUE && self.time_base.is_valid()
    }

    /// 转换为秒 (f64)
    ///
    /// 无效时间戳返回 `f64::NAN`.
    pub fn to_seconds(&self) -> f64 {
        if !self.is_valid() {
            return f64::NAN;
        }
        self.pts as f64 * self.time_base.to_f64()
    }

    /// 将时间戳重缩放到新的时间基
    ///
    /// 通过交叉乘法避免浮点精度损失:
    /// new_pts = pts * old_tb.num * new_tb.den / (old_tb.den * new_tb.num)
    pub fn rescale(&self, new_time_base: Rational) -> Self {
        if !self.is_valid() || !new_time_base.is_valid() {
            return Self::none();
        }
        let num = self.pts as i128 * i128::from(self.time_base.num) * i128::from(new_time_base.den);
        let den = i128::from(self.time_base.den) * i128::from(new_time_base.num);
        if den == 0 {
            return Self::none();
        }
        Self {
            pts: (num / den) as i64,
            time_base: new_time_base,
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_valid() {
            write!(f, "NOPTS")
        } else {
            write!(f, "{:.6}s", self.to_seconds())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_转换为秒() {
        let ts = Timestamp::new(90000, Rational::new(1, 90000));
        assert!((ts.to_seconds() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_timestamp_重缩放() {
        // 从 90kHz 时间基转换到毫秒时间基
        let ts = Timestamp::new(90000, Rational::new(1, 90000));
        let rescaled = ts.rescale(Rational::new(1, 1000));
        assert_eq!(rescaled.pts, 1000);
    }

    #[test]
    fn test_timestamp_无效值() {
        let ts = Timestamp::none();
        assert!(!ts.is_valid());
        assert!(ts.to_seconds().is_nan());
    }
}
