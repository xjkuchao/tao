//! 有理数类型, 用于时间基 (time_base)、宽高比等场景.
//!
//! 对标 FFmpeg 的 `AVRational`.

use std::fmt;

/// 有理数, 由分子和分母组成
///
/// 广泛用于表示时间基 (time_base)、帧率、宽高比等.
/// 例如: 时间基 1/90000 表示 90kHz 时钟, 帧率 30000/1001 表示 29.97fps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    /// 分子
    pub num: i32,
    /// 分母
    pub den: i32,
}

impl Rational {
    /// 创建新的有理数
    ///
    /// # 参数
    /// - `num`: 分子
    /// - `den`: 分母 (不应为 0)
    pub const fn new(num: i32, den: i32) -> Self {
        Self { num, den }
    }

    /// 零值
    pub const ZERO: Self = Self { num: 0, den: 1 };

    /// 未定义 (分母为 0)
    pub const UNDEFINED: Self = Self { num: 0, den: 0 };

    /// 常用时间基: 微秒 (1/1_000_000)
    pub const MICRO: Self = Self {
        num: 1,
        den: 1_000_000,
    };

    /// 常用时间基: 毫秒 (1/1_000)
    pub const MILLI: Self = Self { num: 1, den: 1_000 };

    /// 判断是否有效 (分母不为 0)
    pub const fn is_valid(&self) -> bool {
        self.den != 0
    }

    /// 转换为 f64 浮点数
    ///
    /// 如果分母为 0, 返回 `f64::NAN`.
    pub fn to_f64(self) -> f64 {
        if self.den == 0 {
            return f64::NAN;
        }
        f64::from(self.num) / f64::from(self.den)
    }

    /// 对有理数进行约分
    pub fn reduce(self) -> Self {
        if self.den == 0 {
            return self;
        }
        let g = gcd(self.num.unsigned_abs(), self.den.unsigned_abs());
        if g == 0 {
            return self;
        }
        let g = g as i32;
        // 保证分母为正
        let sign = if self.den < 0 { -1 } else { 1 };
        Self {
            num: sign * self.num / g,
            den: sign * self.den / g,
        }
    }

    /// 求倒数
    pub const fn invert(self) -> Self {
        Self {
            num: self.den,
            den: self.num,
        }
    }
}

impl std::ops::Mul for Rational {
    type Output = Self;

    /// 两个有理数相乘
    fn mul(self, other: Self) -> Self {
        Self {
            num: self.num * other.num,
            den: self.den * other.den,
        }
        .reduce()
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl From<(i32, i32)> for Rational {
    fn from((num, den): (i32, i32)) -> Self {
        Self { num, den }
    }
}

impl From<i32> for Rational {
    fn from(num: i32) -> Self {
        Self { num, den: 1 }
    }
}

/// 求最大公约数 (欧几里得算法)
fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rational_basic_creation() {
        let r = Rational::new(1, 30);
        assert_eq!(r.num, 1);
        assert_eq!(r.den, 30);
    }

    #[test]
    fn test_rational_to_float() {
        let r = Rational::new(1, 4);
        assert!((r.to_f64() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn test_rational_reduce() {
        let r = Rational::new(30, 60).reduce();
        assert_eq!(r, Rational::new(1, 2));
    }

    #[test]
    fn test_rational_invalid_value() {
        let r = Rational::UNDEFINED;
        assert!(!r.is_valid());
        assert!(r.to_f64().is_nan());
    }

    #[test]
    fn test_rational_display() {
        let r = Rational::new(30000, 1001);
        assert_eq!(format!("{r}"), "30000/1001");
    }

    #[test]
    fn test_rational_reciprocal() {
        let r = Rational::new(1, 25).invert();
        assert_eq!(r, Rational::new(25, 1));
    }
}
