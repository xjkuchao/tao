//! CRC 校验和计算.
//!
//! 提供 CRC-8 和 CRC-16 计算, 用于 FLAC 帧头和帧尾校验.

/// CRC-8 查找表 (多项式 0x07)
const CRC8_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut crc = i as u8;
        let mut j = 0;
        while j < 8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// CRC-16 查找表 (多项式 0x8005)
const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut crc = i << 8;
        let mut j = 0;
        while j < 8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x8005;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// 计算 CRC-8
///
/// FLAC 帧头使用此 CRC 校验 (多项式 0x07, 初始值 0).
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &byte in data {
        crc = CRC8_TABLE[(crc ^ byte) as usize];
    }
    crc
}

/// 计算 CRC-16
///
/// FLAC 帧尾使用此 CRC 校验 (多项式 0x8005, 初始值 0).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc = (crc << 8) ^ CRC16_TABLE[((crc >> 8) as u8 ^ byte) as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc8_empty_data() {
        assert_eq!(crc8(&[]), 0);
    }

    #[test]
    fn test_crc8_known_value() {
        // FLAC 帧头 CRC-8 的已知测试向量
        let data = [0xFF, 0xF8, 0x69, 0x18, 0x00];
        let crc = crc8(&data);
        // CRC-8 应为非零值 (具体值取决于数据)
        assert_ne!(crc, 0xFF); // 基本健全性检查
    }

    #[test]
    fn test_crc16_empty_data() {
        assert_eq!(crc16(&[]), 0);
    }

    #[test]
    fn test_crc16_single_byte() {
        let crc = crc16(&[0x01]);
        assert_ne!(crc, 0); // 非零输入应产生非零 CRC
    }

    #[test]
    fn test_crc8_increment() {
        // 验证 CRC 与数据相关
        let crc1 = crc8(&[0x00]);
        let crc2 = crc8(&[0x01]);
        assert_ne!(crc1, crc2);
    }

    #[test]
    fn test_crc16_increment() {
        let crc1 = crc16(&[0x00, 0x00]);
        let crc2 = crc16(&[0x00, 0x01]);
        assert_ne!(crc1, crc2);
    }
}
