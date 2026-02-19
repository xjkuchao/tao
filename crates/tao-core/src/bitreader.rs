//! 比特流读取器.
//!
//! 提供从字节缓冲区中按位读取数据的能力, 是所有压缩编解码器 (FLAC, H.264, AAC 等) 的基础设施.
//!
//! 按大端位序读取 (MSB first), 这是多媒体编解码器中最常用的位序.

use crate::{TaoError, TaoResult};

/// 比特流读取器
///
/// 从字节缓冲区中按位读取数据, 使用大端位序 (MSB first).
///
/// # 示例
/// ```
/// use tao_core::bitreader::BitReader;
///
/// let data = [0b10110001, 0b01010101];
/// let mut br = BitReader::new(&data);
/// assert_eq!(br.read_bits(4).unwrap(), 0b1011);
/// assert_eq!(br.read_bits(4).unwrap(), 0b0001);
/// assert_eq!(br.read_bits(8).unwrap(), 0b01010101);
/// ```
pub struct BitReader<'a> {
    /// 源数据
    data: &'a [u8],
    /// 当前字节索引
    byte_pos: usize,
    /// 当前字节中的位位置 (0-7, 0 表示最高位)
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    /// 创建新的比特流读取器
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// 获取已读取的总位数
    pub fn bits_read(&self) -> usize {
        self.byte_pos * 8 + self.bit_pos as usize
    }

    /// 获取剩余可读位数
    pub fn bits_left(&self) -> usize {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_pos) * 8 - self.bit_pos as usize
    }

    /// 是否已到达末尾
    pub fn is_eof(&self) -> bool {
        self.bits_left() == 0
    }

    /// 读取 1 个位
    pub fn read_bit(&mut self) -> TaoResult<u32> {
        if self.byte_pos >= self.data.len() {
            return Err(TaoError::Eof);
        }

        let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos >= 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }

        Ok(u32::from(bit))
    }

    /// 读取 N 个位 (最多 32 位)
    ///
    /// 按大端位序读取, 返回值的低 N 位有效.
    pub fn read_bits(&mut self, n: u32) -> TaoResult<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(TaoError::InvalidArgument(format!(
                "read_bits: n={} 超过 32 位",
                n,
            )));
        }
        if (n as usize) > self.bits_left() {
            return Err(TaoError::Eof);
        }

        let mut result: u32 = 0;
        let mut remaining = n;

        while remaining > 0 {
            let available = 8 - self.bit_pos as u32;
            let to_read = remaining.min(available);

            // 从当前字节中提取位
            let shift = available - to_read;
            let mask = ((1u32 << to_read) - 1) as u8;
            let bits = (self.data[self.byte_pos] >> shift) & mask;

            result = (result << to_read) | u32::from(bits);

            self.bit_pos += to_read as u8;
            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
            remaining -= to_read;
        }

        Ok(result)
    }

    /// 读取 N 个位 (最多 64 位)
    pub fn read_bits_u64(&mut self, n: u32) -> TaoResult<u64> {
        if n <= 32 {
            return self.read_bits(n).map(u64::from);
        }
        if n > 64 {
            return Err(TaoError::InvalidArgument(format!(
                "read_bits_u64: n={} 超过 64 位",
                n,
            )));
        }

        let high_bits = n - 32;
        let high = self.read_bits(high_bits)? as u64;
        let low = self.read_bits(32)? as u64;
        Ok((high << 32) | low)
    }

    /// 读取有符号整数 (二进制补码)
    pub fn read_bits_signed(&mut self, n: u32) -> TaoResult<i32> {
        let val = self.read_bits(n)?;
        if n == 0 {
            return Ok(0);
        }
        // n == 32 时, val 的全部 32 位有效, 直接转换为 i32 (二进制补码)
        if n >= 32 {
            return Ok(val as i32);
        }
        // 符号扩展: 若最高有效位为 1, 则填充高位
        if (val >> (n - 1)) & 1 != 0 {
            Ok(val as i32 | !((1i32 << n) - 1))
        } else {
            Ok(val as i32)
        }
    }

    /// 读取一元编码值 (unary code)
    ///
    /// 计算连续出现的 `stop_bit` 的反面的位数, 直到遇到 `stop_bit`.
    ///
    /// 例如, `read_unary(1)` 从 `0001...` 中读取得到 3 (三个 0 后跟一个 1).
    pub fn read_unary(&mut self, stop_bit: u32) -> TaoResult<u32> {
        let stop = stop_bit & 1;
        let mut count = 0u32;
        loop {
            let bit = self.read_bit()?;
            if bit == stop {
                return Ok(count);
            }
            count += 1;
        }
    }

    /// 读取 UTF-8 风格的可变长度编码 (FLAC 使用)
    ///
    /// 这不是真正的 UTF-8, 而是 FLAC 自定义的变长编码.
    /// 返回解码后的值.
    pub fn read_utf8_u64(&mut self) -> TaoResult<u64> {
        let first = self.read_bits(8)? as u8;

        // 确定编码长度
        let (value, extra_bytes) = if first & 0x80 == 0 {
            (u64::from(first), 0)
        } else if first & 0xE0 == 0xC0 {
            (u64::from(first & 0x1F), 1)
        } else if first & 0xF0 == 0xE0 {
            (u64::from(first & 0x0F), 2)
        } else if first & 0xF8 == 0xF0 {
            (u64::from(first & 0x07), 3)
        } else if first & 0xFC == 0xF8 {
            (u64::from(first & 0x03), 4)
        } else if first & 0xFE == 0xFC {
            (u64::from(first & 0x01), 5)
        } else if first == 0xFE {
            (0u64, 6)
        } else {
            return Err(TaoError::InvalidData(format!(
                "无效的 UTF-8 变长编码首字节: 0x{:02X}",
                first,
            )));
        };

        let mut result = value;
        for _ in 0..extra_bytes {
            let byte = self.read_bits(8)? as u8;
            if byte & 0xC0 != 0x80 {
                return Err(TaoError::InvalidData(
                    "无效的 UTF-8 变长编码后续字节".into(),
                ));
            }
            result = (result << 6) | u64::from(byte & 0x3F);
        }

        Ok(result)
    }

    /// 窥视 N 个位 (不移动位置)
    pub fn peek_bits(&mut self, n: u32) -> TaoResult<u32> {
        let saved_byte = self.byte_pos;
        let saved_bit = self.bit_pos;
        let result = self.read_bits(n);
        self.byte_pos = saved_byte;
        self.bit_pos = saved_bit;
        result
    }

    /// 跳过 N 个位
    pub fn skip_bits(&mut self, n: u32) -> TaoResult<()> {
        if (n as usize) > self.bits_left() {
            return Err(TaoError::Eof);
        }

        let total_bits = self.bit_pos as u32 + n;
        self.byte_pos += (total_bits / 8) as usize;
        self.bit_pos = (total_bits % 8) as u8;

        Ok(())
    }

    /// 对齐到下一个字节边界
    ///
    /// 如果当前已在字节边界, 则不做任何事.
    pub fn align_to_byte(&mut self) {
        if self.bit_pos > 0 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
    }

    /// 获取当前字节位置
    pub fn byte_position(&self) -> usize {
        self.byte_pos
    }

    /// 从当前位置读取原始字节切片
    ///
    /// 仅在字节对齐时可用.
    pub fn read_bytes(&mut self, n: usize) -> TaoResult<&'a [u8]> {
        if self.bit_pos != 0 {
            return Err(TaoError::InvalidArgument("read_bytes 需要字节对齐".into()));
        }

        let end = self.byte_pos + n;
        if end > self.data.len() {
            return Err(TaoError::Eof);
        }

        let slice = &self.data[self.byte_pos..end];
        self.byte_pos = end;
        Ok(slice)
    }

    /// 获取底层数据的引用
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_bits_basic() {
        let data = [0b10110001, 0b01010101];
        let mut br = BitReader::new(&data);

        assert_eq!(br.read_bits(1).unwrap(), 1);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        assert_eq!(br.read_bits(2).unwrap(), 0b11);
        assert_eq!(br.read_bits(4).unwrap(), 0b0001);
        assert_eq!(br.read_bits(8).unwrap(), 0b01010101);

        assert!(br.is_eof());
    }

    #[test]
    fn test_read_bits_32_bit() {
        let data = [0xFF, 0x00, 0xFF, 0x00];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_bits(32).unwrap(), 0xFF00FF00);
    }

    #[test]
    fn test_read_bits_signed() {
        let data = [0b11111000]; // -1 in 5 bits = 0b11111
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_bits_signed(5).unwrap(), -1);

        let data2 = [0b01010000]; // 10 in 5 bits = 0b01010
        let mut br2 = BitReader::new(&data2);
        assert_eq!(br2.read_bits_signed(5).unwrap(), 10);
    }

    #[test]
    fn test_read_unary() {
        // 0001... -> unary(1) = 3
        let data = [0b00010000];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_unary(1).unwrap(), 3);

        // 1110... -> unary(0) = 3
        let data2 = [0b11100000];
        let mut br2 = BitReader::new(&data2);
        assert_eq!(br2.read_unary(0).unwrap(), 3);
    }

    #[test]
    fn test_read_utf8_u64() {
        // 单字节: 0x42 = 'B'
        let data = [0x42];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_utf8_u64().unwrap(), 0x42);

        // 双字节: 0xC0 0x80 = value 0
        let data2 = [0xC0, 0x80];
        let mut br2 = BitReader::new(&data2);
        assert_eq!(br2.read_utf8_u64().unwrap(), 0);
    }

    #[test]
    fn test_peek_bits() {
        let data = [0b10110001];
        let mut br = BitReader::new(&data);

        assert_eq!(br.peek_bits(4).unwrap(), 0b1011);
        assert_eq!(br.peek_bits(4).unwrap(), 0b1011); // 不移动
        assert_eq!(br.read_bits(4).unwrap(), 0b1011); // 现在移动了
        assert_eq!(br.peek_bits(4).unwrap(), 0b0001);
    }

    #[test]
    fn test_skip_bits() {
        let data = [0b10110001, 0b01010101];
        let mut br = BitReader::new(&data);

        br.skip_bits(4).unwrap();
        assert_eq!(br.read_bits(4).unwrap(), 0b0001);
        br.skip_bits(4).unwrap();
        assert_eq!(br.read_bits(4).unwrap(), 0b0101);
    }

    #[test]
    fn test_align_to_byte() {
        let data = [0b10110001, 0b01010101];
        let mut br = BitReader::new(&data);

        br.read_bits(3).unwrap();
        br.align_to_byte();
        assert_eq!(br.byte_position(), 1);
        assert_eq!(br.read_bits(8).unwrap(), 0b01010101);
    }

    #[test]
    fn test_bits_left() {
        let data = [0x00, 0x00];
        let mut br = BitReader::new(&data);

        assert_eq!(br.bits_left(), 16);
        br.read_bits(5).unwrap();
        assert_eq!(br.bits_left(), 11);
        br.read_bits(11).unwrap();
        assert_eq!(br.bits_left(), 0);
        assert!(br.is_eof());
    }

    #[test]
    fn test_read_bytes() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let mut br = BitReader::new(&data);

        let bytes = br.read_bytes(2).unwrap();
        assert_eq!(bytes, &[0x01, 0x02]);
        let bytes = br.read_bytes(2).unwrap();
        assert_eq!(bytes, &[0x03, 0x04]);
    }

    #[test]
    fn test_read_bits_u64() {
        let data = [0xFF, 0x00, 0xFF, 0x00, 0xAA, 0xBB, 0xCC, 0xDD];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_bits_u64(64).unwrap(), 0xFF00FF00AABBCCDD);
    }

    #[test]
    fn test_eof_error() {
        let data = [0x00];
        let mut br = BitReader::new(&data);

        br.read_bits(8).unwrap();
        assert!(br.read_bits(1).is_err());
    }
}
