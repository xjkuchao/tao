//! MP3 位流读取器
//!
//! 支持从字节切片中按位读取数据 (MSB first)

#[derive(Debug, Clone, Copy)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// 当前字节索引
    byte_pos: usize,
    /// 当前字节内的位偏移 (0-7), 0 为最高位 (MSB)
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    /// 创建新的 BitReader
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// 读取 n 位 (最多 32 位) 并返回 u32
    /// 如果剩余位数不足, 返回 None
    pub fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 {
            return None;
        }

        // 检查是否有足够的剩余位数
        let bits_left = self.bits_left();
        if bits_left < n as usize {
            return None;
        }

        let mut result = 0u32;
        let mut bits_to_read = n;

        while bits_to_read > 0 {
            let bits_available_in_byte = 8 - self.bit_pos;
            let bits_this_chunk = bits_to_read.min(bits_available_in_byte);

            let byte = self.data[self.byte_pos];
            // 提取当前字节中的位:
            // 1. 右移以丢弃低位不需要的部分
            // 2. 掩码保留需要的位数
            //
            // 例子: byte=10110011, bit_pos=1, read 3 bits (011)
            // bits_available=7, bits_this_chunk=3
            // shift = 7 - 3 = 4
            // byte >> 4 = 00001011
            // mask = (1<<3)-1 = 00000111
            // val = 011 = 3

            let shift = bits_available_in_byte - bits_this_chunk;
            let mask = (1u32 << bits_this_chunk) - 1;
            let val = ((byte >> shift) as u32) & mask;

            result = (result << bits_this_chunk) | val;

            self.bit_pos += bits_this_chunk;
            if self.bit_pos >= 8 {
                self.byte_pos += 1;
                self.bit_pos = 0;
            }

            bits_to_read -= bits_this_chunk;
        }

        Some(result)
    }

    /// 读取 1 位, 返回 bool (true=1, false=0)
    pub fn read_bool(&mut self) -> Option<bool> {
        self.read_bits(1).map(|v| v != 0)
    }

    /// 窥视 n 位 (不移动游标)
    pub fn peek_bits(&self, n: u8) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 {
            return None;
        }

        let mut temp_reader = Self {
            data: self.data,
            byte_pos: self.byte_pos,
            bit_pos: self.bit_pos,
        };

        temp_reader.read_bits(n)
    }

    /// 跳过 n 位
    pub fn skip_bits(&mut self, n: usize) -> bool {
        let total_bits = self.byte_pos * 8 + self.bit_pos as usize + n;
        let total_bytes = total_bits / 8;
        let new_bit_pos = (total_bits % 8) as u8;

        if total_bytes > self.data.len() || (total_bytes == self.data.len() && new_bit_pos > 0) {
            return false;
        }

        self.byte_pos = total_bytes;
        self.bit_pos = new_bit_pos;
        true
    }

    /// 剩余位数
    pub fn bits_left(&self) -> usize {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_pos) * 8 - self.bit_pos as usize
    }

    /// 当前绝对位偏移 (从起始位置开始的总位数)
    pub fn bit_offset(&self) -> usize {
        self.byte_pos * 8 + self.bit_pos as usize
    }

    /// 定位到指定的绝对位偏移
    pub fn seek_to_bit(&mut self, bit_offset: usize) {
        self.byte_pos = bit_offset / 8;
        self.bit_pos = (bit_offset % 8) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_bits() {
        let data = [0b10110011, 0b11000000];
        let mut reader = BitReader::new(&data);

        // 101
        assert_eq!(reader.read_bits(3), Some(0b101));
        // 1001
        assert_eq!(reader.read_bits(4), Some(0b1001));
        // 1
        assert_eq!(reader.read_bits(1), Some(1));
        // 11
        assert_eq!(reader.read_bits(2), Some(0b11));

        // 剩余 6 位全是 0
        assert_eq!(reader.read_bits(6), Some(0));

        // EOF
        assert_eq!(reader.read_bits(1), None);
    }

    #[test]
    fn test_cross_byte_boundary() {
        // 0xFF 0x00 -> 11111111 00000000
        let data = [0xFF, 0x00];
        let mut reader = BitReader::new(&data);

        // read 12 bits: 11111111 0000
        assert_eq!(reader.read_bits(12), Some(0xFF0));
        assert_eq!(reader.bits_left(), 4);
    }
}
