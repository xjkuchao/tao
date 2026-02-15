//! 位流读取器与起始码查找

/// 位流读取器
pub(super) struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// 读取 n 位 (最多 32 位)
    pub fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 || n > 32 {
            return None;
        }
        let mut result = 0u32;
        let mut bits_to_read = n;
        while bits_to_read > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }
            let bits_available = 8 - self.bit_pos;
            let bits_from_this_byte = bits_to_read.min(bits_available);
            let byte = self.data[self.byte_pos];
            let mask = if bits_from_this_byte >= 8 {
                0xFF
            } else {
                (1u8 << bits_from_this_byte) - 1
            };
            let shift = bits_available - bits_from_this_byte;
            let bits = (byte >> shift) & mask;
            result = result.checked_shl(bits_from_this_byte as u32).unwrap_or(0) | (bits as u32);
            self.bit_pos += bits_from_this_byte;
            if self.bit_pos >= 8 {
                self.byte_pos += 1;
                self.bit_pos = 0;
            }
            bits_to_read -= bits_from_this_byte;
        }
        Some(result)
    }

    /// 窥视 n 位 (不消耗)
    pub fn peek_bits(&self, n: u8) -> Option<u32> {
        if n == 0 || n > 32 {
            return None;
        }
        let mut result = 0u32;
        let mut byte_pos = self.byte_pos;
        let mut bit_pos = self.bit_pos;
        for _ in 0..n {
            if byte_pos >= self.data.len() {
                return None;
            }
            let bit = (self.data[byte_pos] >> (7 - bit_pos)) & 1;
            result = (result << 1) | (bit as u32);
            bit_pos += 1;
            if bit_pos >= 8 {
                bit_pos = 0;
                byte_pos += 1;
            }
        }
        Some(result)
    }

    /// 跳过 n 位
    pub fn skip_bits(&mut self, n: u32) {
        let total_bits = self.byte_pos as u32 * 8 + self.bit_pos as u32 + n;
        self.byte_pos = (total_bits / 8) as usize;
        self.bit_pos = (total_bits % 8) as u8;
    }

    /// 读取单个位
    pub fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|b| b != 0)
    }

    /// 获取剩余可读位数
    #[allow(dead_code)]
    pub fn bits_left(&self) -> usize {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_pos) * 8 - self.bit_pos as usize
    }

    /// 获取当前字节位置
    #[allow(dead_code)]
    pub fn byte_position(&self) -> usize {
        self.byte_pos
    }

    /// 获取当前位位置
    #[allow(dead_code)]
    pub fn bit_position(&self) -> usize {
        self.byte_pos * 8 + self.bit_pos as usize
    }

    /// 到下一个字节边界的位数
    pub fn bits_to_byte_align(&self) -> u8 {
        if self.bit_pos == 0 {
            0
        } else {
            8 - self.bit_pos
        }
    }

    /// 字节对齐
    #[allow(dead_code)]
    pub fn align_to_byte(&mut self) {
        if self.bit_pos != 0 {
            self.byte_pos += 1;
            self.bit_pos = 0;
        }
    }
}

// ============================================================================
// 起始码查找
// ============================================================================

/// 查找特定起始码 (00 00 01 target), 返回起始码之后的偏移
pub(super) fn find_start_code_offset(data: &[u8], target: u8) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    for idx in 0..(data.len() - 3) {
        if data[idx] == 0x00
            && data[idx + 1] == 0x00
            && data[idx + 2] == 0x01
            && data[idx + 3] == target
        {
            return Some(idx + 4);
        }
    }
    None
}

/// 查找范围内的起始码 (00 00 01 [start..=end])
pub(super) fn find_start_code_range(data: &[u8], start: u8, end: u8) -> Option<(u8, usize)> {
    if data.len() < 4 {
        return None;
    }
    for idx in 0..(data.len() - 3) {
        if data[idx] == 0x00 && data[idx + 1] == 0x00 && data[idx + 2] == 0x01 {
            let code = data[idx + 3];
            if (start..=end).contains(&code) {
                return Some((code, idx + 4));
            }
        }
    }
    None
}
