//! H.264 NAL (Network Abstraction Layer) 单元解析.
//!
//! # Annex B 格式
//!
//! Annex B 使用起始码 (start code) 分隔 NAL 单元:
//! - 3 字节起始码: `00 00 01`
//! - 4 字节起始码: `00 00 00 01`
//!
//! # NAL 头部 (1 字节)
//! ```text
//! ┌─────────────────────────────────┐
//! │ forbidden(1) | ref_idc(2) | type(5) │
//! └─────────────────────────────────┘
//! ```
//!
//! # AVCC 格式
//!
//! AVCC (也称 AVC length-prefixed) 使用 4 字节长度前缀:
//! ```text
//! [length: 4 bytes BE] [NAL data: length bytes]
//! ```

use tao_core::TaoResult;

/// NAL 单元类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NalUnitType {
    /// 非 IDR 图像切片 (P/B slice)
    Slice,
    /// 数据分区 A (DPA)
    SliceDpa,
    /// 数据分区 B (DPB)
    SliceDpb,
    /// 数据分区 C (DPC)
    SliceDpc,
    /// IDR 图像切片 (关键帧)
    SliceIdr,
    /// 增补增强信息 (SEI)
    Sei,
    /// 序列参数集 (SPS)
    Sps,
    /// 图像参数集 (PPS)
    Pps,
    /// 访问单元分隔符 (AUD)
    Aud,
    /// 序列结束
    EndOfSequence,
    /// 流结束
    EndOfStream,
    /// 填充数据
    FillerData,
    /// SPS 扩展
    SpsExtension,
    /// 未知类型
    Unknown(u8),
}

impl NalUnitType {
    /// 从 NAL 类型编号创建
    pub fn from_type_id(type_id: u8) -> Self {
        match type_id {
            1 => Self::Slice,
            2 => Self::SliceDpa,
            3 => Self::SliceDpb,
            4 => Self::SliceDpc,
            5 => Self::SliceIdr,
            6 => Self::Sei,
            7 => Self::Sps,
            8 => Self::Pps,
            9 => Self::Aud,
            10 => Self::EndOfSequence,
            11 => Self::EndOfStream,
            12 => Self::FillerData,
            13 => Self::SpsExtension,
            _ => Self::Unknown(type_id),
        }
    }

    /// 获取类型编号
    pub fn type_id(&self) -> u8 {
        match self {
            Self::Slice => 1,
            Self::SliceDpa => 2,
            Self::SliceDpb => 3,
            Self::SliceDpc => 4,
            Self::SliceIdr => 5,
            Self::Sei => 6,
            Self::Sps => 7,
            Self::Pps => 8,
            Self::Aud => 9,
            Self::EndOfSequence => 10,
            Self::EndOfStream => 11,
            Self::FillerData => 12,
            Self::SpsExtension => 13,
            Self::Unknown(id) => *id,
        }
    }

    /// 是否为 VCL (Video Coding Layer) NAL
    pub fn is_vcl(&self) -> bool {
        matches!(
            self,
            Self::Slice | Self::SliceDpa | Self::SliceDpb | Self::SliceDpc | Self::SliceIdr
        )
    }

    /// 是否为关键帧 (IDR)
    pub fn is_idr(&self) -> bool {
        matches!(self, Self::SliceIdr)
    }
}

impl std::fmt::Display for NalUnitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slice => write!(f, "Slice"),
            Self::SliceDpa => write!(f, "SliceDPA"),
            Self::SliceDpb => write!(f, "SliceDPB"),
            Self::SliceDpc => write!(f, "SliceDPC"),
            Self::SliceIdr => write!(f, "IDR"),
            Self::Sei => write!(f, "SEI"),
            Self::Sps => write!(f, "SPS"),
            Self::Pps => write!(f, "PPS"),
            Self::Aud => write!(f, "AUD"),
            Self::EndOfSequence => write!(f, "EndOfSeq"),
            Self::EndOfStream => write!(f, "EndOfStream"),
            Self::FillerData => write!(f, "Filler"),
            Self::SpsExtension => write!(f, "SPSExt"),
            Self::Unknown(id) => write!(f, "Unknown({id})"),
        }
    }
}

/// 解析后的 NAL 单元
#[derive(Debug, Clone)]
pub struct NalUnit {
    /// NAL 单元类型
    pub nal_type: NalUnitType,
    /// nal_ref_idc (参考重要性, 0-3)
    pub ref_idc: u8,
    /// NAL 单元原始数据 (不含起始码, 含 NAL 头部字节)
    pub data: Vec<u8>,
}

impl NalUnit {
    /// 从 NAL 数据 (含头部字节) 解析
    pub fn parse(data: &[u8]) -> TaoResult<Self> {
        if data.is_empty() {
            return Err(tao_core::TaoError::InvalidData(
                "H.264: NAL 单元数据为空".into(),
            ));
        }

        let header = data[0];
        let forbidden = (header >> 7) & 1;
        if forbidden != 0 {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: forbidden_zero_bit 非法, value={}",
                forbidden
            )));
        }
        let ref_idc = (header >> 5) & 0x03;
        let type_id = header & 0x1F;

        Ok(Self {
            nal_type: NalUnitType::from_type_id(type_id),
            ref_idc,
            data: data.to_vec(),
        })
    }

    /// 获取 RBSP (Raw Byte Sequence Payload) 数据
    ///
    /// 移除 NAL 头部字节和 emulation prevention 字节 (0x03).
    /// RBSP 是参数集解析所需的纯净数据.
    pub fn rbsp(&self) -> Vec<u8> {
        remove_emulation_prevention(&self.data[1..])
    }
}

/// 从 Annex B 字节流中分割出所有 NAL 单元
///
/// 支持 3 字节 (00 00 01) 和 4 字节 (00 00 00 01) 起始码.
/// 返回的 NAL 单元不含起始码.
pub fn split_annex_b(data: &[u8]) -> Vec<NalUnit> {
    let offsets = find_start_codes(data);
    let mut nalus = Vec::new();

    for (i, &start) in offsets.iter().enumerate() {
        let end = if i + 1 < offsets.len() {
            // 下一个起始码之前
            offsets[i + 1]
        } else {
            data.len()
        };

        // 跳过起始码
        let nal_start = skip_start_code(data, start);
        if nal_start >= end {
            continue;
        }

        // 去除尾部的 0 字节 (trailing zeros)
        let mut nal_end = end;
        while nal_end > nal_start && data[nal_end - 1] == 0x00 {
            nal_end -= 1;
        }

        if nal_end > nal_start {
            if let Ok(nalu) = NalUnit::parse(&data[nal_start..nal_end]) {
                nalus.push(nalu);
            }
        }
    }

    nalus
}

/// 从 AVCC (length-prefixed) 数据中提取 NAL 单元
///
/// `length_size` 通常为 4 (来自 AVCDecoderConfigurationRecord 的 lengthSizeMinusOne + 1)
pub fn split_avcc(data: &[u8], length_size: usize) -> Vec<NalUnit> {
    if !(1..=4).contains(&length_size) {
        return Vec::new();
    }

    let mut nalus = Vec::new();
    let mut pos = 0;

    while pos + length_size <= data.len() {
        // 读取 NAL 长度
        let mut nal_len: usize = 0;
        for i in 0..length_size {
            nal_len = (nal_len << 8) | data[pos + i] as usize;
        }
        pos += length_size;

        if pos + nal_len > data.len() {
            break;
        }

        if let Ok(nalu) = NalUnit::parse(&data[pos..pos + nal_len]) {
            nalus.push(nalu);
        }
        pos += nal_len;
    }

    nalus
}

/// 将 Annex B 格式转换为 AVCC 格式 (4 字节长度前缀)
pub fn annex_b_to_avcc(data: &[u8]) -> Vec<u8> {
    let nalus = split_annex_b(data);
    let mut out = Vec::new();

    for nalu in &nalus {
        let len = nalu.data.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&nalu.data);
    }

    out
}

/// 将 AVCC 格式转换为 Annex B 格式 (4 字节起始码)
pub fn avcc_to_annex_b(data: &[u8], length_size: usize) -> Vec<u8> {
    let nalus = split_avcc(data, length_size);
    let mut out = Vec::new();

    for nalu in &nalus {
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&nalu.data);
    }

    out
}

/// avcC 配置解析结果
#[derive(Debug)]
pub struct AvccConfig {
    /// SPS 列表
    pub sps_list: Vec<Vec<u8>>,
    /// PPS 列表
    pub pps_list: Vec<Vec<u8>>,
    /// NAL 长度前缀大小 (字节)
    pub length_size: usize,
}

/// 解析 AVCDecoderConfigurationRecord (MP4 avcC box 内容)
pub fn parse_avcc_config(data: &[u8]) -> TaoResult<AvccConfig> {
    if data.len() < 7 {
        return Err(tao_core::TaoError::InvalidData(
            "H.264: avcC 数据太短".into(),
        ));
    }

    let _version = data[0];
    let _profile = data[1];
    let _compat = data[2];
    let _level = data[3];
    let length_size = ((data[4] & 0x03) + 1) as usize;

    let num_sps = (data[5] & 0x1F) as usize;
    let mut pos = 6;
    let mut sps_list = Vec::new();

    for i in 0..num_sps {
        if pos + 2 > data.len() {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC SPS 长度字段截断, index={}",
                i
            )));
        }
        let sps_len = (u16::from(data[pos]) << 8 | u16::from(data[pos + 1])) as usize;
        pos += 2;
        if sps_len == 0 {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC SPS 长度非法, index={}, len=0",
                i
            )));
        }
        if pos + sps_len > data.len() {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC SPS 数据截断, index={}, declared_len={}, remain={}",
                i,
                sps_len,
                data.len().saturating_sub(pos)
            )));
        }
        sps_list.push(data[pos..pos + sps_len].to_vec());
        pos += sps_len;
    }

    if pos >= data.len() {
        return Err(tao_core::TaoError::InvalidData(
            "H.264: avcC 缺少 numOfPictureParameterSets 字段".into(),
        ));
    }

    let mut pps_list = Vec::new();
    let num_pps = data[pos] as usize;
    pos += 1;
    for i in 0..num_pps {
        if pos + 2 > data.len() {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC PPS 长度字段截断, index={}",
                i
            )));
        }
        let pps_len = (u16::from(data[pos]) << 8 | u16::from(data[pos + 1])) as usize;
        pos += 2;
        if pps_len == 0 {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC PPS 长度非法, index={}, len=0",
                i
            )));
        }
        if pos + pps_len > data.len() {
            return Err(tao_core::TaoError::InvalidData(format!(
                "H.264: avcC PPS 数据截断, index={}, declared_len={}, remain={}",
                i,
                pps_len,
                data.len().saturating_sub(pos)
            )));
        }
        pps_list.push(data[pos..pos + pps_len].to_vec());
        pos += pps_len;
    }

    Ok(AvccConfig {
        sps_list,
        pps_list,
        length_size,
    })
}

/// 构建 AVCDecoderConfigurationRecord
pub fn build_avcc_config(
    sps_list: &[Vec<u8>],
    pps_list: &[Vec<u8>],
    length_size: usize,
) -> TaoResult<Vec<u8>> {
    if sps_list.is_empty() {
        return Err(tao_core::TaoError::InvalidData(
            "H.264: 构建 avcC 需要至少一个 SPS".into(),
        ));
    }

    let sps0 = &sps_list[0];
    if sps0.len() < 4 {
        return Err(tao_core::TaoError::InvalidData(
            "H.264: SPS 数据太短".into(),
        ));
    }

    let mut out = vec![
        1,                                // configurationVersion
        sps0[1],                          // profile_idc
        sps0[2],                          // profile_compatibility
        sps0[3],                          // level_idc
        0xFC | ((length_size as u8) - 1), // lengthSizeMinusOne
        0xE0 | (sps_list.len() as u8),    // numOfSPS
    ];
    for sps in sps_list {
        let len = sps.len() as u16;
        out.push((len >> 8) as u8);
        out.push(len as u8);
        out.extend_from_slice(sps);
    }

    // PPS
    out.push(pps_list.len() as u8);
    for pps in pps_list {
        let len = pps.len() as u16;
        out.push((len >> 8) as u8);
        out.push(len as u8);
        out.extend_from_slice(pps);
    }

    Ok(out)
}

// ============================================================
// 内部工具函数
// ============================================================

/// 查找所有起始码的位置
fn find_start_codes(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut i = 0;

    while i + 2 < data.len() {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            if data[i + 2] == 0x01 {
                // 3 字节起始码
                positions.push(i);
                i += 3;
                continue;
            } else if i + 3 < data.len() && data[i + 2] == 0x00 && data[i + 3] == 0x01 {
                // 4 字节起始码
                positions.push(i);
                i += 4;
                continue;
            }
        }
        i += 1;
    }

    positions
}

/// 跳过起始码, 返回 NAL 数据的起始位置
fn skip_start_code(data: &[u8], pos: usize) -> usize {
    if pos + 3 < data.len()
        && data[pos] == 0x00
        && data[pos + 1] == 0x00
        && data[pos + 2] == 0x00
        && data[pos + 3] == 0x01
    {
        pos + 4
    } else if pos + 2 < data.len()
        && data[pos] == 0x00
        && data[pos + 1] == 0x00
        && data[pos + 2] == 0x01
    {
        pos + 3
    } else {
        pos
    }
}

/// 移除 emulation prevention 字节 (0x00 0x00 0x03 → 0x00 0x00)
///
/// H.264 规范要求在 RBSP 中, 如果出现连续两个 0x00,
/// 后面必须插入 0x03 以防止与起始码混淆.
/// 解析时需要移除这些 0x03 字节.
fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        // 对齐 FFmpeg: 只要命中 `00 00 03` 序列就移除中间 0x03.
        // 这是 H264 NAL 到 RBSP 的标准去防竞争字节行为.
        let is_emulation_prevention =
            i + 2 < data.len() && data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x03;
        if is_emulation_prevention {
            rbsp.push(0x00);
            rbsp.push(0x00);
            i += 3; // 跳过 0x03
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }

    rbsp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nal_type_create() {
        assert_eq!(NalUnitType::from_type_id(7), NalUnitType::Sps);
        assert_eq!(NalUnitType::from_type_id(8), NalUnitType::Pps);
        assert_eq!(NalUnitType::from_type_id(5), NalUnitType::SliceIdr);
        assert_eq!(NalUnitType::from_type_id(1), NalUnitType::Slice);
        assert_eq!(NalUnitType::from_type_id(9), NalUnitType::Aud);
    }

    #[test]
    fn test_nal_type_property() {
        assert!(NalUnitType::SliceIdr.is_vcl());
        assert!(NalUnitType::SliceIdr.is_idr());
        assert!(NalUnitType::Slice.is_vcl());
        assert!(!NalUnitType::Slice.is_idr());
        assert!(!NalUnitType::Sps.is_vcl());
        assert!(!NalUnitType::Pps.is_vcl());
    }

    #[test]
    fn test_nal_type_type_id() {
        for id in 0..=13 {
            let nt = NalUnitType::from_type_id(id);
            assert_eq!(nt.type_id(), id);
        }
    }

    #[test]
    fn test_nal_unit_parse() {
        // NAL header: forbidden=0, ref_idc=3, type=7 (SPS)
        // 0b0_11_00111 = 0x67
        let data = [0x67, 0x42, 0x00, 0x1E];
        let nalu = NalUnit::parse(&data).unwrap();
        assert_eq!(nalu.nal_type, NalUnitType::Sps);
        assert_eq!(nalu.ref_idc, 3);
    }

    #[test]
    fn test_nal_unit_empty_data_error() {
        assert!(NalUnit::parse(&[]).is_err());
    }

    #[test]
    fn test_nal_unit_reject_forbidden_zero_bit_set() {
        let err = NalUnit::parse(&[0xE7]).expect_err("forbidden_zero_bit=1 应返回错误");
        let msg = format!("{err}");
        assert!(
            msg.contains("forbidden_zero_bit"),
            "错误信息应包含 forbidden_zero_bit, actual={}",
            msg
        );
    }

    #[test]
    fn test_annex_b_split_3_byte_start_code() {
        let data = [
            0x00, 0x00, 0x01, 0x67, 0xAA, 0xBB, // SPS
            0x00, 0x00, 0x01, 0x68, 0xCC, // PPS
            0x00, 0x00, 0x01, 0x65, 0xDD, 0xEE, 0xFF, // IDR
        ];

        let nalus = split_annex_b(&data);
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
        assert_eq!(nalus[2].nal_type, NalUnitType::SliceIdr);
    }

    #[test]
    fn test_annex_b_split_4_byte_start_code() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0xAA, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xBB, // PPS
        ];

        let nalus = split_annex_b(&data);
        assert_eq!(nalus.len(), 2);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    }

    #[test]
    fn test_annex_b_split_mixed_start_code() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0xAA, // SPS (4字节)
            0x00, 0x00, 0x01, 0x68, 0xBB, // PPS (3字节)
        ];

        let nalus = split_annex_b(&data);
        assert_eq!(nalus.len(), 2);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    }

    #[test]
    fn test_avcc_split() {
        let mut data = Vec::new();
        // NAL 1: SPS, 3 bytes
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x03]);
        data.extend_from_slice(&[0x67, 0xAA, 0xBB]);
        // NAL 2: PPS, 2 bytes
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]);
        data.extend_from_slice(&[0x68, 0xCC]);

        let nalus = split_avcc(&data, 4);
        assert_eq!(nalus.len(), 2);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    }

    #[test]
    fn test_avcc_split_reject_invalid_length_size() {
        let data = [0x00, 0x00, 0x00, 0x02, 0x67, 0xAA];
        let nalus_zero = split_avcc(&data, 0);
        let nalus_too_large = split_avcc(&data, 5);
        assert!(
            nalus_zero.is_empty(),
            "length_size=0 应直接返回空结果, 避免死循环"
        );
        assert!(nalus_too_large.is_empty(), "length_size>4 应直接返回空结果");
    }

    #[test]
    fn test_annex_b_to_avcc_convert() {
        let annexb = [
            0x00, 0x00, 0x01, 0x67, 0xAA, // SPS
            0x00, 0x00, 0x01, 0x68, 0xBB, // PPS
        ];

        let avcc = annex_b_to_avcc(&annexb);
        // NAL 1: len=2, data=67 AA
        // NAL 2: len=2, data=68 BB
        assert_eq!(avcc.len(), 4 + 2 + 4 + 2);

        // 验证可以用 split_avcc 解析回来
        let nalus = split_avcc(&avcc, 4);
        assert_eq!(nalus.len(), 2);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    }

    #[test]
    fn test_avcc_to_annex_b_convert() {
        let mut avcc = Vec::new();
        avcc.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]); // len=2
        avcc.extend_from_slice(&[0x67, 0xAA]); // SPS
        avcc.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]); // len=2
        avcc.extend_from_slice(&[0x68, 0xBB]); // PPS

        let annexb = avcc_to_annex_b(&avcc, 4);
        let nalus = split_annex_b(&annexb);
        assert_eq!(nalus.len(), 2);
        assert_eq!(nalus[0].nal_type, NalUnitType::Sps);
        assert_eq!(nalus[1].nal_type, NalUnitType::Pps);
    }

    #[test]
    fn test_emulation_prevention_remove() {
        // 00 00 03 → 00 00
        let data = [0x01, 0x00, 0x00, 0x03, 0x02, 0x03];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, vec![0x01, 0x00, 0x00, 0x02, 0x03]);
    }

    #[test]
    fn test_emulation_prevention_consecutive() {
        // 多个 emulation prevention
        let data = [0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x01];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, vec![0x00, 0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn test_emulation_prevention_remove_when_next_gt_03() {
        // 对齐 FFmpeg: `00 00 03` 统一移除, 即使后一个字节 > 0x03.
        let data = [0x11, 0x00, 0x00, 0x03, 0x04, 0x22];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, vec![0x11, 0x00, 0x00, 0x04, 0x22]);
    }

    #[test]
    fn test_emulation_prevention_remove_when_next_lte_03() {
        // `00 00 03 03` 中的 0x03 为防竞争字节, 需要删除.
        let data = [0x00, 0x00, 0x03, 0x03, 0x80];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, vec![0x00, 0x00, 0x03, 0x80]);
    }

    #[test]
    fn test_avcc_config_parse() {
        // 构造 AVCDecoderConfigurationRecord
        let sps = vec![0x67, 0x42, 0x00, 0x1E, 0xAB];
        let pps = vec![0x68, 0xCE, 0x38, 0x80];

        let config =
            build_avcc_config(std::slice::from_ref(&sps), std::slice::from_ref(&pps), 4).unwrap();
        let parsed = parse_avcc_config(&config).unwrap();

        assert_eq!(parsed.length_size, 4);
        assert_eq!(parsed.sps_list.len(), 1);
        assert_eq!(parsed.pps_list.len(), 1);
        assert_eq!(parsed.sps_list[0], sps);
        assert_eq!(parsed.pps_list[0], pps);
    }

    #[test]
    fn test_avcc_config_no_sps_error() {
        assert!(build_avcc_config(&[], &[], 4).is_err());
    }

    #[test]
    fn test_parse_avcc_config_reject_truncated_sps_length_field() {
        // num_sps=1, 但 SPS 长度字段只有 1 字节.
        let data = [0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE1, 0x00];
        let err = parse_avcc_config(&data).expect_err("SPS 长度字段截断应返回错误");
        let msg = format!("{err}");
        assert!(
            msg.contains("SPS 长度字段截断"),
            "错误信息应包含 SPS 长度字段截断, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_avcc_config_reject_truncated_sps_payload() {
        // num_sps=1, declared_len=4, 实际仅 2 字节.
        let data = [0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x04, 0x67, 0x64];
        let err = parse_avcc_config(&data).expect_err("SPS 数据截断应返回错误");
        let msg = format!("{err}");
        assert!(
            msg.contains("SPS 数据截断"),
            "错误信息应包含 SPS 数据截断, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_avcc_config_reject_missing_num_pps_field() {
        // num_sps=1, SPS 完整, 但缺少 numOfPictureParameterSets 字段.
        let data = [0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x01, 0x67];
        let err = parse_avcc_config(&data).expect_err("缺少 num_pps 字段应返回错误");
        let msg = format!("{err}");
        assert!(
            msg.contains("numOfPictureParameterSets"),
            "错误信息应包含 numOfPictureParameterSets, actual={}",
            msg
        );
    }

    #[test]
    fn test_parse_avcc_config_reject_truncated_pps_payload() {
        // num_sps=0, num_pps=1, declared_len=2, 实际仅 1 字节.
        let data = [0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE0, 0x01, 0x00, 0x02, 0x68];
        let err = parse_avcc_config(&data).expect_err("PPS 数据截断应返回错误");
        let msg = format!("{err}");
        assert!(
            msg.contains("PPS 数据截断"),
            "错误信息应包含 PPS 数据截断, actual={}",
            msg
        );
    }

    #[test]
    fn test_rbsp_extract() {
        // SPS header + emulation prevention
        let data = [0x67, 0x42, 0x00, 0x00, 0x03, 0x01, 0xAA];
        let nalu = NalUnit::parse(&data).unwrap();
        let rbsp = nalu.rbsp();
        // 移除头部 (0x67) 和 emulation prevention
        assert_eq!(rbsp, vec![0x42, 0x00, 0x00, 0x01, 0xAA]);
    }
}
