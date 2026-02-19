//! 比特流写入器.
//!
//! 提供向字节缓冲区按位写入数据的能力, 是压缩编码器的基础设施.
//!
//! 按大端位序写入 (MSB first), 与 BitReader 对应.

/// 比特流写入器
///
/// 向字节缓冲区按位写入数据, 使用大端位序 (MSB first).
///
/// # 示例
/// ```
/// use tao_core::bitwriter::BitWriter;
///
/// let mut bw = BitWriter::new();
/// bw.write_bits(0b1011, 4);
/// bw.write_bits(0b0001, 4);
/// bw.write_bits(0b01010101, 8);
/// let data = bw.finish();
/// assert_eq!(data, vec![0b10110001, 0b01010101]);
/// ```
pub struct BitWriter {
    /// 输出缓冲区
    data: Vec<u8>,
    /// 当前字节 (正在填充)
    current_byte: u8,
    /// 当前字节中已填充的位数 (0-7)
    bit_count: u8,
}

impl BitWriter {
    /// 创建新的比特流写入器
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            current_byte: 0,
            bit_count: 0,
        }
    }

    /// 以指定容量创建比特流写入器
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            current_byte: 0,
            bit_count: 0,
        }
    }

    /// 获取已写入的总位数
    pub fn bits_written(&self) -> usize {
        self.data.len() * 8 + self.bit_count as usize
    }

    /// 写入 1 个位
    pub fn write_bit(&mut self, bit: u32) {
        self.current_byte = (self.current_byte << 1) | (bit & 1) as u8;
        self.bit_count += 1;
        if self.bit_count >= 8 {
            self.data.push(self.current_byte);
            self.current_byte = 0;
            self.bit_count = 0;
        }
    }

    /// 写入 N 个位 (最多 32 位)
    ///
    /// 值的低 N 位被写入, 高位在前 (大端).
    pub fn write_bits(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32, "write_bits: n={} 超过 32 位", n);

        if n == 0 {
            return;
        }

        let mut remaining = n;
        while remaining > 0 {
            let available = 8 - self.bit_count as u32;
            let to_write = remaining.min(available);

            // 提取要写入的位
            let shift = remaining - to_write;
            let mask = if to_write >= 32 {
                u32::MAX
            } else {
                (1u32 << to_write) - 1
            };
            let bits = ((value >> shift) & mask) as u8;

            if to_write >= 8 {
                // 整字节写入 (bit_count 必定为 0)
                self.current_byte = bits;
            } else {
                self.current_byte = (self.current_byte << to_write) | bits;
            }
            self.bit_count += to_write as u8;

            if self.bit_count >= 8 {
                self.data.push(self.current_byte);
                self.current_byte = 0;
                self.bit_count = 0;
            }

            remaining -= to_write;
        }
    }

    /// 写入 N 个位 (最多 64 位)
    pub fn write_bits_u64(&mut self, value: u64, n: u32) {
        if n <= 32 {
            self.write_bits(value as u32, n);
        } else {
            let high_bits = n - 32;
            self.write_bits((value >> 32) as u32, high_bits);
            self.write_bits(value as u32, 32);
        }
    }

    /// 写入有符号整数 (二进制补码)
    pub fn write_bits_signed(&mut self, value: i32, n: u32) {
        let mask = (1u64 << n) - 1;
        self.write_bits((value as u32) & mask as u32, n);
    }

    /// 写入一元编码
    ///
    /// 写入 `count` 个 `!stop_bit`, 然后一个 `stop_bit`.
    pub fn write_unary(&mut self, count: u32, stop_bit: u32) {
        let fill = 1 - (stop_bit & 1);
        for _ in 0..count {
            self.write_bit(fill);
        }
        self.write_bit(stop_bit & 1);
    }

    /// 写入 UTF-8 风格变长编码 (FLAC 使用)
    pub fn write_utf8_u64(&mut self, value: u64) {
        if value < 0x80 {
            self.write_bits(value as u32, 8);
        } else if value < 0x800 {
            self.write_bits(0xC0 | ((value >> 6) as u32 & 0x1F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        } else if value < 0x10000 {
            self.write_bits(0xE0 | ((value >> 12) as u32 & 0x0F), 8);
            self.write_bits(0x80 | ((value >> 6) as u32 & 0x3F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        } else if value < 0x200000 {
            self.write_bits(0xF0 | ((value >> 18) as u32 & 0x07), 8);
            self.write_bits(0x80 | ((value >> 12) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 6) as u32 & 0x3F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        } else if value < 0x4000000 {
            self.write_bits(0xF8 | ((value >> 24) as u32 & 0x03), 8);
            self.write_bits(0x80 | ((value >> 18) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 12) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 6) as u32 & 0x3F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        } else if value < 0x80000000 {
            self.write_bits(0xFC | ((value >> 30) as u32 & 0x01), 8);
            self.write_bits(0x80 | ((value >> 24) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 18) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 12) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 6) as u32 & 0x3F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        } else {
            self.write_bits(0xFE, 8);
            self.write_bits(0x80 | ((value >> 30) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 24) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 18) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 12) as u32 & 0x3F), 8);
            self.write_bits(0x80 | ((value >> 6) as u32 & 0x3F), 8);
            self.write_bits(0x80 | (value as u32 & 0x3F), 8);
        }
    }

    /// 对齐到字节边界 (用 0 填充)
    pub fn align_to_byte(&mut self) {
        if self.bit_count > 0 {
            let pad = 8 - self.bit_count;
            self.current_byte <<= pad;
            self.data.push(self.current_byte);
            self.current_byte = 0;
            self.bit_count = 0;
        }
    }

    /// 完成写入, 返回字节数据
    ///
    /// 如果当前不在字节边界, 自动用 0 填充.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.data
    }

    /// 获取当前已完成的字节数据引用
    ///
    /// 注意: 不包括正在填充的当前字节.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// 获取当前已完成的字节数据 (包括对齐后的当前字节)
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.align_to_byte();
        self.data.clone()
    }

    /// 写入完整字节
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if self.bit_count == 0 {
            // 快速路径: 已对齐
            self.data.extend_from_slice(bytes);
        } else {
            for &b in bytes {
                self.write_bits(u32::from(b), 8);
            }
        }
    }
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitreader::BitReader;

    #[test]
    fn test_write_bits_basic() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b1011, 4);
        bw.write_bits(0b0001, 4);
        let data = bw.finish();
        assert_eq!(data, vec![0b10110001]);
    }

    #[test]
    fn test_write_bits_cross_byte() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b10110001, 8);
        bw.write_bits(0b01010101, 8);
        let data = bw.finish();
        assert_eq!(data, vec![0b10110001, 0b01010101]);
    }

    #[test]
    fn test_write_bits_32_bit() {
        let mut bw = BitWriter::new();
        bw.write_bits(0xFF00FF00, 32);
        let data = bw.finish();
        assert_eq!(data, vec![0xFF, 0x00, 0xFF, 0x00]);
    }

    #[test]
    fn test_write_bit_bit_by_bit() {
        let mut bw = BitWriter::new();
        bw.write_bit(1);
        bw.write_bit(0);
        bw.write_bit(1);
        bw.write_bit(1);
        bw.write_bit(0);
        bw.write_bit(0);
        bw.write_bit(0);
        bw.write_bit(1);
        let data = bw.finish();
        assert_eq!(data, vec![0b10110001]);
    }

    #[test]
    fn test_write_unary() {
        let mut bw = BitWriter::new();
        bw.write_unary(3, 1); // 0001
        bw.write_unary(0, 1); // 1
        bw.write_bits(0, 3); // 000 填充
        let data = bw.finish();
        assert_eq!(data, vec![0b00011000]);
    }

    #[test]
    fn test_align_to_byte() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b101, 3);
        bw.align_to_byte();
        bw.write_bits(0xFF, 8);
        let data = bw.finish();
        assert_eq!(data, vec![0b10100000, 0xFF]);
    }

    #[test]
    fn test_write_bytes() {
        let mut bw = BitWriter::new();
        bw.write_bytes(&[0x01, 0x02, 0x03]);
        let data = bw.finish();
        assert_eq!(data, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_read_write_roundtrip_utf8() {
        for value in [0u64, 1, 127, 128, 0x7FF, 0x800, 0xFFFF, 0x10000, 0x1FFFFF] {
            let mut bw = BitWriter::new();
            bw.write_utf8_u64(value);
            let data = bw.finish();

            let mut br = BitReader::new(&data);
            let decoded = br.read_utf8_u64().unwrap();
            assert_eq!(decoded, value, "UTF-8 往返失败: value={}", value);
        }
    }

    #[test]
    fn test_read_write_roundtrip_bits() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b10110, 5);
        bw.write_bits(0xFF, 8);
        bw.write_bits(0, 3);
        let data = bw.finish();

        let mut br = BitReader::new(&data);
        assert_eq!(br.read_bits(5).unwrap(), 0b10110);
        assert_eq!(br.read_bits(8).unwrap(), 0xFF);
        assert_eq!(br.read_bits(3).unwrap(), 0);
    }

    #[test]
    fn test_read_write_roundtrip_signed() {
        let mut bw = BitWriter::new();
        bw.write_bits_signed(-1, 5);
        bw.write_bits_signed(10, 5);
        bw.write_bits_signed(-128, 8);
        bw.align_to_byte();
        let data = bw.finish();

        let mut br = BitReader::new(&data);
        assert_eq!(br.read_bits_signed(5).unwrap(), -1);
        assert_eq!(br.read_bits_signed(5).unwrap(), 10);
        assert_eq!(br.read_bits_signed(8).unwrap(), -128);
    }
}
