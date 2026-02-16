//! MPEG-4 Part 2 码流解析器.
//!
//! 提供对 MPEG-4 Part 2 (ISO/IEC 14496-2) 原始码流的解析能力:
//! - 起始码扫描与类型识别
//! - VOP (Video Object Plane) 边界检测
//! - 从原始 .m4v 字节流中分割完整 VOP 包
//! - VOL/VOS/VO 头部提取

/// MPEG-4 Part 2 起始码类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mpeg4StartCodeType {
    /// 视频对象 (Video Object, 0x00-0x1F)
    VideoObject(u8),
    /// 视频对象层 (Video Object Layer, 0x20-0x2F)
    VideoObjectLayer(u8),
    /// 视觉对象序列起始 (0xB0)
    VisualObjectSequenceStart,
    /// 视觉对象序列结束 (0xB1)
    VisualObjectSequenceEnd,
    /// 用户数据 (0xB2)
    UserData,
    /// 组头 (Group of VOP, 0xB3)
    GroupOfVop,
    /// 视频会话错误 (0xB4)
    VideoSessionError,
    /// 视觉对象 (0xB5)
    VisualObject,
    /// VOP 起始码 (0xB6)
    Vop,
    /// 填充数据 (0xB7-0xB9)
    Filler(u8),
    /// 未知起始码
    Unknown(u8),
}

impl Mpeg4StartCodeType {
    /// 从起始码字节识别类型
    pub fn from_byte(code: u8) -> Self {
        match code {
            0x00..=0x1F => Mpeg4StartCodeType::VideoObject(code),
            0x20..=0x2F => Mpeg4StartCodeType::VideoObjectLayer(code - 0x20),
            0xB0 => Mpeg4StartCodeType::VisualObjectSequenceStart,
            0xB1 => Mpeg4StartCodeType::VisualObjectSequenceEnd,
            0xB2 => Mpeg4StartCodeType::UserData,
            0xB3 => Mpeg4StartCodeType::GroupOfVop,
            0xB4 => Mpeg4StartCodeType::VideoSessionError,
            0xB5 => Mpeg4StartCodeType::VisualObject,
            0xB6 => Mpeg4StartCodeType::Vop,
            0xB7..=0xB9 => Mpeg4StartCodeType::Filler(code),
            other => Mpeg4StartCodeType::Unknown(other),
        }
    }
}

/// 起始码条目: 位置和类型
#[derive(Debug, Clone)]
pub struct StartCodeEntry {
    /// 起始码 00 00 01 xx 中 xx 的位置 (即起始码本身的开始偏移)
    pub offset: usize,
    /// 起始码之后的数据偏移 (offset + 4)
    pub data_offset: usize,
    /// 起始码类型
    pub code_type: Mpeg4StartCodeType,
    /// 原始起始码字节
    pub code_byte: u8,
}

/// 扫描数据中所有 MPEG-4 起始码
///
/// 起始码格式: 00 00 01 xx, 其中 xx 标识起始码类型.
/// 返回按偏移排序的起始码列表.
pub fn scan_start_codes(data: &[u8]) -> Vec<StartCodeEntry> {
    let mut entries = Vec::new();
    if data.len() < 4 {
        return entries;
    }

    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x01 {
            let code = data[i + 3];
            entries.push(StartCodeEntry {
                offset: i,
                data_offset: i + 4,
                code_type: Mpeg4StartCodeType::from_byte(code),
                code_byte: code,
            });
            i += 4;
        } else {
            i += 1;
        }
    }

    entries
}

/// VOP 包: 一个完整的 VOP 及其关联的头信息
#[derive(Debug, Clone)]
pub struct Mpeg4VopPacket {
    /// VOP 数据 (从第一个关联起始码到下一个 VOP/结束)
    ///
    /// 包含 VOP 之前的 VOL/UserData 等头信息和 VOP 本身的编码数据
    pub data: Vec<u8>,
    /// VOP 在原始数据中的起始偏移
    pub source_offset: usize,
    /// VOP 包中 VOP 起始码 (00 00 01 B6) 的相对偏移
    pub vop_offset: usize,
    /// 是否包含 VOL 头
    pub has_vol: bool,
    /// 是否包含 user_data
    pub has_user_data: bool,
}

/// 从原始字节流中分割 VOP 包
///
/// 将原始 MPEG-4 Part 2 字节流分割为独立的 VOP 包.
/// 每个 VOP 包包含解码所需的所有关联头信息 (VOL/VOS/UserData 等).
///
/// # 参数
/// - `data`: 原始 MPEG-4 Part 2 字节流
///
/// # 返回
/// VOP 包列表, 每个包可以独立送入解码器
pub fn split_vop_packets(data: &[u8]) -> Vec<Mpeg4VopPacket> {
    let entries = scan_start_codes(data);
    let mut packets = Vec::new();

    if entries.is_empty() {
        return packets;
    }

    // 找到所有 VOP 起始码的索引
    let vop_indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.code_type == Mpeg4StartCodeType::Vop)
        .map(|(i, _)| i)
        .collect();

    if vop_indices.is_empty() {
        return packets;
    }

    // 对每个 VOP, 收集它之前的非 VOP 头信息 + VOP 数据
    for (vop_idx_pos, &vop_entry_idx) in vop_indices.iter().enumerate() {
        let vop_entry = &entries[vop_entry_idx];

        // 向前搜索: 找到属于此 VOP 的头信息的起始位置
        // 从上一个 VOP 之后 (或数据开头) 到当前 VOP 之间的所有非 VOP 起始码
        let prev_vop_end = if vop_idx_pos > 0 {
            let prev_vop_entry_idx = vop_indices[vop_idx_pos - 1];
            // 上一个 VOP 的数据结束于当前头信息区域的开始
            entries[prev_vop_entry_idx].data_offset
        } else {
            0
        };

        // 找到当前 VOP 之前最早的头信息起始码
        let mut header_start = vop_entry.offset;
        for i in (0..vop_entry_idx).rev() {
            let entry = &entries[i];
            if entry.offset < prev_vop_end {
                break;
            }
            match entry.code_type {
                Mpeg4StartCodeType::Vop => break,
                _ => {
                    header_start = entry.offset;
                }
            }
        }

        // 确定 VOP 数据的结束位置
        let vop_data_end = if vop_idx_pos + 1 < vop_indices.len() {
            // 下一个 VOP 之前的头信息开始处
            let next_vop_entry_idx = vop_indices[vop_idx_pos + 1];
            // 找到下一个 VOP 之前最早的头信息
            let mut next_header_start = entries[next_vop_entry_idx].offset;
            for i in (vop_entry_idx + 1..next_vop_entry_idx).rev() {
                let entry = &entries[i];
                match entry.code_type {
                    Mpeg4StartCodeType::Vop => break,
                    _ => {
                        next_header_start = entry.offset;
                    }
                }
            }
            next_header_start
        } else {
            data.len()
        };

        // 检查是否包含 VOL 和 user_data
        let mut has_vol = false;
        let mut has_user_data = false;
        for entry in entries.iter().take(vop_entry_idx) {
            if entry.offset >= header_start && entry.offset < vop_entry.offset {
                match entry.code_type {
                    Mpeg4StartCodeType::VideoObjectLayer(_) => has_vol = true,
                    Mpeg4StartCodeType::UserData => has_user_data = true,
                    _ => {}
                }
            }
        }

        let vop_offset = vop_entry.offset - header_start;
        let packet_data = data[header_start..vop_data_end].to_vec();

        packets.push(Mpeg4VopPacket {
            data: packet_data,
            source_offset: header_start,
            vop_offset,
            has_vol,
            has_user_data,
        });
    }

    packets
}

/// 提取 VOL (Video Object Layer) 头数据
///
/// 从字节流中提取 VOL 起始码到下一个起始码之间的数据.
/// 用于初始化解码器参数.
pub fn extract_vol_header(data: &[u8]) -> Option<Vec<u8>> {
    let entries = scan_start_codes(data);

    for (i, entry) in entries.iter().enumerate() {
        if let Mpeg4StartCodeType::VideoObjectLayer(_) = entry.code_type {
            let end = if i + 1 < entries.len() {
                entries[i + 1].offset
            } else {
                data.len()
            };
            return Some(data[entry.offset..end].to_vec());
        }
    }

    None
}

/// 提取所有 user_data 段
pub fn extract_user_data(data: &[u8]) -> Vec<Vec<u8>> {
    let entries = scan_start_codes(data);
    let mut result = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        if entry.code_type == Mpeg4StartCodeType::UserData {
            let end = if i + 1 < entries.len() {
                entries[i + 1].offset
            } else {
                data.len()
            };
            result.push(data[entry.data_offset..end].to_vec());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_code_type_from_byte() {
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0x00),
            Mpeg4StartCodeType::VideoObject(0x00)
        );
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0x20),
            Mpeg4StartCodeType::VideoObjectLayer(0)
        );
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0x2F),
            Mpeg4StartCodeType::VideoObjectLayer(0x0F)
        );
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0xB0),
            Mpeg4StartCodeType::VisualObjectSequenceStart
        );
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0xB2),
            Mpeg4StartCodeType::UserData
        );
        assert_eq!(
            Mpeg4StartCodeType::from_byte(0xB5),
            Mpeg4StartCodeType::VisualObject
        );
        assert_eq!(Mpeg4StartCodeType::from_byte(0xB6), Mpeg4StartCodeType::Vop);
    }

    #[test]
    fn test_scan_start_codes() {
        // 构造: VOS + VO + VOL + VOP
        let data = [
            0x00, 0x00, 0x01, 0xB0, // VOS
            0x01, // profile_and_level
            0x00, 0x00, 0x01, 0xB5, // Visual Object
            0x09, // visual_object_type
            0x00, 0x00, 0x01, 0x00, // Video Object 0
            0x00, 0x00, 0x01, 0x20, // VOL 0
            0xFF, 0xFF, // VOL 数据
            0x00, 0x00, 0x01, 0xB6, // VOP
            0x00, 0x10, // VOP 数据
        ];

        let entries = scan_start_codes(&data);
        assert_eq!(entries.len(), 5, "应找到 5 个起始码");
        assert_eq!(
            entries[0].code_type,
            Mpeg4StartCodeType::VisualObjectSequenceStart
        );
        assert_eq!(entries[1].code_type, Mpeg4StartCodeType::VisualObject);
        assert_eq!(entries[2].code_type, Mpeg4StartCodeType::VideoObject(0));
        assert_eq!(
            entries[3].code_type,
            Mpeg4StartCodeType::VideoObjectLayer(0)
        );
        assert_eq!(entries[4].code_type, Mpeg4StartCodeType::Vop);
    }

    #[test]
    fn test_scan_empty_data() {
        let entries = scan_start_codes(&[]);
        assert!(entries.is_empty());

        let entries = scan_start_codes(&[0x00, 0x00]);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_split_vop_packets_single() {
        // 单个 VOP: VOL + VOP
        let data = [
            0x00, 0x00, 0x01, 0x20, // VOL
            0xFF, 0xFF, // VOL 数据
            0x00, 0x00, 0x01, 0xB6, // VOP
            0x00, 0x10, 0x20, // VOP 数据
        ];

        let packets = split_vop_packets(&data);
        assert_eq!(packets.len(), 1, "应拆分出 1 个 VOP 包");
        assert!(packets[0].has_vol, "第一个 VOP 包应包含 VOL");
        assert_eq!(packets[0].data.len(), data.len(), "单 VOP 包应包含全部数据");
    }

    #[test]
    fn test_split_vop_packets_multiple() {
        // 两个 VOP (模拟 packed bitstream)
        let data = [
            0x00, 0x00, 0x01, 0x20, // VOL
            0xFF, 0xFF, // VOL 数据
            0x00, 0x00, 0x01, 0xB6, // VOP 1
            0x00, 0x10, 0x20, 0x30, // VOP 1 数据
            0x00, 0x00, 0x01, 0xB6, // VOP 2
            0x40, 0x50, // VOP 2 数据
        ];

        let packets = split_vop_packets(&data);
        assert_eq!(packets.len(), 2, "应拆分出 2 个 VOP 包");
        assert!(packets[0].has_vol, "第一个 VOP 包应包含 VOL");
        assert!(!packets[1].has_vol, "第二个 VOP 包不应包含 VOL");
    }

    #[test]
    fn test_split_vop_packets_with_user_data() {
        // VOL + UserData + VOP
        let data = [
            0x00, 0x00, 0x01, 0x20, // VOL
            0xFF, // VOL 数据
            0x00, 0x00, 0x01, 0xB2, // UserData
            b'D', b'i', b'v', b'X', // "DivX"
            0x00, 0x00, 0x01, 0xB6, // VOP
            0x00, 0x10, // VOP 数据
        ];

        let packets = split_vop_packets(&data);
        assert_eq!(packets.len(), 1);
        assert!(packets[0].has_vol);
        assert!(packets[0].has_user_data, "应检测到 user_data");
    }

    #[test]
    fn test_extract_vol_header() {
        let data = [
            0x00, 0x00, 0x01, 0xB0, // VOS
            0x01, // profile
            0x00, 0x00, 0x01, 0x20, // VOL
            0xAA, 0xBB, 0xCC, // VOL 数据
            0x00, 0x00, 0x01, 0xB6, // VOP
            0x00, 0x10, // VOP 数据
        ];

        let vol = extract_vol_header(&data);
        assert!(vol.is_some(), "应找到 VOL");
        let vol = vol.unwrap();
        // VOL 从 00 00 01 20 开始, 到 VOP 起始码之前
        assert_eq!(vol[0..4], [0x00, 0x00, 0x01, 0x20]);
        assert_eq!(vol[4..7], [0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn test_extract_user_data() {
        let data = [
            0x00, 0x00, 0x01, 0xB2, // UserData 1
            b'h', b'e', b'l', b'l', b'o', // "hello"
            0x00, 0x00, 0x01, 0xB6, // VOP (终止 UserData)
            0x10, 0x20,
        ];

        let uds = extract_user_data(&data);
        assert_eq!(uds.len(), 1, "应找到 1 段 user_data");
        assert_eq!(&uds[0], b"hello", "user_data 内容应为 'hello'");
    }

    #[test]
    fn test_no_vop_packets() {
        // 只有头信息, 没有 VOP
        let data = [
            0x00, 0x00, 0x01, 0x20, // VOL
            0xFF, 0xFF,
        ];

        let packets = split_vop_packets(&data);
        assert!(packets.is_empty(), "无 VOP 时应返回空列表");
    }
}
