//! H.265/HEVC NAL (Network Abstraction Layer) 单元解析.
//!
//! HEVC NAL 头部为 2 字节 (比 H.264 多一字节):
//! - forbidden_zero_bit (1 bit)
//! - nal_unit_type (6 bits)
//! - nuh_layer_id (6 bits)
//! - nuh_temporal_id_plus1 (3 bits)

use tao_core::{TaoError, TaoResult};

/// HEVC NAL 单元类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HevcNalUnitType {
    /// TRAIL_N (非参考尾随图像)
    TrailN,
    /// TRAIL_R (参考尾随图像)
    TrailR,
    /// TSA_N
    TsaN,
    /// TSA_R
    TsaR,
    /// STSA_N
    StsaN,
    /// STSA_R
    StsaR,
    /// RADL_N
    RadlN,
    /// RADL_R
    RadlR,
    /// RASL_N
    RaslN,
    /// RASL_R
    RaslR,
    /// BLA_W_LP (Broken Link Access)
    BlaWLp,
    /// BLA_W_RADL
    BlaWRadl,
    /// BLA_N_LP
    BlaNLp,
    /// IDR_W_RADL (Instantaneous Decoding Refresh)
    IdrWRadl,
    /// IDR_N_LP
    IdrNLp,
    /// CRA_NUT (Clean Random Access)
    Cra,
    /// VPS (Video Parameter Set)
    Vps,
    /// SPS (Sequence Parameter Set)
    Sps,
    /// PPS (Picture Parameter Set)
    Pps,
    /// AUD (Access Unit Delimiter)
    Aud,
    /// EOS (End of Sequence)
    Eos,
    /// EOB (End of Bitstream)
    Eob,
    /// FD (Filler Data)
    FillerData,
    /// PREFIX_SEI
    PrefixSei,
    /// SUFFIX_SEI
    SuffixSei,
    /// 未知类型
    Unknown(u8),
}

impl HevcNalUnitType {
    /// 从类型编号创建
    pub fn from_type_id(id: u8) -> Self {
        match id {
            0 => Self::TrailN,
            1 => Self::TrailR,
            2 => Self::TsaN,
            3 => Self::TsaR,
            4 => Self::StsaN,
            5 => Self::StsaR,
            6 => Self::RadlN,
            7 => Self::RadlR,
            8 => Self::RaslN,
            9 => Self::RaslR,
            16 => Self::BlaWLp,
            17 => Self::BlaWRadl,
            18 => Self::BlaNLp,
            19 => Self::IdrWRadl,
            20 => Self::IdrNLp,
            21 => Self::Cra,
            32 => Self::Vps,
            33 => Self::Sps,
            34 => Self::Pps,
            35 => Self::Aud,
            36 => Self::Eos,
            37 => Self::Eob,
            38 => Self::FillerData,
            39 => Self::PrefixSei,
            40 => Self::SuffixSei,
            _ => Self::Unknown(id),
        }
    }

    /// 获取类型编号
    pub fn type_id(&self) -> u8 {
        match self {
            Self::TrailN => 0,
            Self::TrailR => 1,
            Self::TsaN => 2,
            Self::TsaR => 3,
            Self::StsaN => 4,
            Self::StsaR => 5,
            Self::RadlN => 6,
            Self::RadlR => 7,
            Self::RaslN => 8,
            Self::RaslR => 9,
            Self::BlaWLp => 16,
            Self::BlaWRadl => 17,
            Self::BlaNLp => 18,
            Self::IdrWRadl => 19,
            Self::IdrNLp => 20,
            Self::Cra => 21,
            Self::Vps => 32,
            Self::Sps => 33,
            Self::Pps => 34,
            Self::Aud => 35,
            Self::Eos => 36,
            Self::Eob => 37,
            Self::FillerData => 38,
            Self::PrefixSei => 39,
            Self::SuffixSei => 40,
            Self::Unknown(id) => *id,
        }
    }

    /// 是否为 VCL (Video Coding Layer) NAL
    pub fn is_vcl(&self) -> bool {
        self.type_id() < 32
    }

    /// 是否为 IRAP (Intra Random Access Point) NAL
    pub fn is_irap(&self) -> bool {
        matches!(self.type_id(), 16..=21)
    }

    /// 是否为 IDR NAL
    pub fn is_idr(&self) -> bool {
        matches!(self, Self::IdrWRadl | Self::IdrNLp)
    }
}

/// HEVC NAL 单元
#[derive(Debug, Clone)]
pub struct HevcNalUnit {
    /// NAL 类型
    pub nal_type: HevcNalUnitType,
    /// nuh_layer_id
    pub layer_id: u8,
    /// nuh_temporal_id_plus1
    pub temporal_id_plus1: u8,
    /// NAL 数据 (不含 2 字节 NAL 头)
    pub data: Vec<u8>,
}

impl HevcNalUnit {
    /// 从原始 NAL 数据 (含 2 字节头) 解析
    pub fn parse(data: &[u8]) -> TaoResult<Self> {
        if data.len() < 2 {
            return Err(TaoError::InvalidData("HEVC: NAL 数据太短".into()));
        }
        let nal_type = HevcNalUnitType::from_type_id((data[0] >> 1) & 0x3F);
        let layer_id = ((data[0] & 1) << 5) | (data[1] >> 3);
        let temporal_id_plus1 = data[1] & 0x07;

        Ok(Self {
            nal_type,
            layer_id,
            temporal_id_plus1,
            data: data[2..].to_vec(),
        })
    }
}

// ============================================================
// Annex B 分割
// ============================================================

/// 查找所有起始码位置
fn find_start_codes(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                positions.push(i);
                i += 3;
                continue;
            } else if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                positions.push(i);
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    positions
}

/// 跳过起始码, 返回 NAL 数据起始位置
fn skip_start_code(data: &[u8], pos: usize) -> usize {
    if pos + 3 < data.len()
        && data[pos] == 0
        && data[pos + 1] == 0
        && data[pos + 2] == 0
        && data[pos + 3] == 1
    {
        pos + 4
    } else {
        pos + 3
    }
}

/// 从 Annex B 格式分割 HEVC NAL 单元
pub fn split_hevc_annex_b(data: &[u8]) -> Vec<HevcNalUnit> {
    let offsets = find_start_codes(data);
    let mut nalus = Vec::new();

    for (i, &start) in offsets.iter().enumerate() {
        let end = if i + 1 < offsets.len() {
            offsets[i + 1]
        } else {
            data.len()
        };
        let nal_start = skip_start_code(data, start);
        if nal_start >= end {
            continue;
        }
        let mut nal_end = end;
        while nal_end > nal_start && data[nal_end - 1] == 0x00 {
            nal_end -= 1;
        }
        if nal_end > nal_start {
            if let Ok(nalu) = HevcNalUnit::parse(&data[nal_start..nal_end]) {
                nalus.push(nalu);
            }
        }
    }
    nalus
}

/// 从 HVCC (长度前缀) 格式分割 HEVC NAL 单元
pub fn split_hevc_hvcc(data: &[u8], length_size: usize) -> Vec<HevcNalUnit> {
    let mut nalus = Vec::new();
    let mut pos = 0;

    while pos + length_size <= data.len() {
        let mut len: usize = 0;
        for i in 0..length_size {
            len = (len << 8) | data[pos + i] as usize;
        }
        pos += length_size;
        if pos + len > data.len() {
            break;
        }
        if let Ok(nalu) = HevcNalUnit::parse(&data[pos..pos + len]) {
            nalus.push(nalu);
        }
        pos += len;
    }
    nalus
}

// ============================================================
// 格式转换
// ============================================================

/// Annex B → HVCC 格式
pub fn hevc_annex_b_to_hvcc(data: &[u8]) -> Vec<u8> {
    let nalus = split_hevc_annex_b(data);
    let mut out = Vec::new();
    for nalu in &nalus {
        let nal_data_with_header = build_nal_with_header(nalu);
        let len = nal_data_with_header.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&nal_data_with_header);
    }
    out
}

/// HVCC → Annex B 格式
pub fn hevc_hvcc_to_annex_b(data: &[u8], length_size: usize) -> Vec<u8> {
    let nalus = split_hevc_hvcc(data, length_size);
    let mut out = Vec::new();
    for nalu in &nalus {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&build_nal_with_header(nalu));
    }
    out
}

/// 从 NAL 单元重建含 2 字节头的数据
fn build_nal_with_header(nalu: &HevcNalUnit) -> Vec<u8> {
    let byte0 = (nalu.nal_type.type_id() << 1) | (nalu.layer_id >> 5);
    let byte1 = ((nalu.layer_id & 0x1F) << 3) | nalu.temporal_id_plus1;
    let mut out = vec![byte0, byte1];
    out.extend_from_slice(&nalu.data);
    out
}

/// 移除 emulation prevention 字节 (0x03)
pub fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3; // 跳过 0x03
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

// ============================================================
// HEVCDecoderConfigurationRecord
// ============================================================

/// HVCC 配置
pub struct HvccConfig {
    /// VPS NAL 列表
    pub vps_list: Vec<Vec<u8>>,
    /// SPS NAL 列表
    pub sps_list: Vec<Vec<u8>>,
    /// PPS NAL 列表
    pub pps_list: Vec<Vec<u8>>,
    /// NAL 长度字段大小
    pub length_size: u8,
    /// general_profile_idc
    pub general_profile_idc: u8,
    /// general_level_idc
    pub general_level_idc: u8,
}

/// 解析 HEVCDecoderConfigurationRecord
pub fn parse_hvcc_config(data: &[u8]) -> TaoResult<HvccConfig> {
    if data.len() < 23 {
        return Err(TaoError::InvalidData("HEVC: hvcC 数据太短".into()));
    }

    let _config_version = data[0]; // 应为 1
    let general_profile_idc = data[1] & 0x1F;
    // bytes 2-5: general_profile_compatibility_flags
    // bytes 6-11: general_constraint_indicator_flags
    let general_level_idc = data[12];
    // bytes 13-14: min_spatial_segmentation_idc (with reserved)
    // byte 15: parallelismType (with reserved)
    // byte 16: chromaFormat (with reserved)
    // byte 17: bitDepthLumaMinus8 (with reserved)
    // byte 18: bitDepthChromaMinus8 (with reserved)
    // bytes 19-20: avgFrameRate
    // byte 21: constantFrameRate(2) | numTemporalLayers(3) | temporalIdNested(1) | lengthSizeMinusOne(2)
    let length_size = (data[21] & 0x03) + 1;
    let num_arrays = data[22];

    let mut vps_list = Vec::new();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    let mut pos = 23;

    for _ in 0..num_arrays {
        if pos >= data.len() {
            break;
        }
        let nal_type = data[pos] & 0x3F;
        pos += 1;
        if pos + 1 >= data.len() {
            break;
        }
        let num_nalus = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        for _ in 0..num_nalus {
            if pos + 1 >= data.len() {
                break;
            }
            let nal_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + nal_len > data.len() {
                break;
            }
            let nal_data = data[pos..pos + nal_len].to_vec();
            pos += nal_len;

            match nal_type {
                32 => vps_list.push(nal_data),
                33 => sps_list.push(nal_data),
                34 => pps_list.push(nal_data),
                _ => {}
            }
        }
    }

    Ok(HvccConfig {
        vps_list,
        sps_list,
        pps_list,
        length_size,
        general_profile_idc,
        general_level_idc,
    })
}

/// 构建 HEVCDecoderConfigurationRecord
pub fn build_hvcc_config(
    vps_list: &[&[u8]],
    sps_list: &[&[u8]],
    pps_list: &[&[u8]],
) -> TaoResult<Vec<u8>> {
    if sps_list.is_empty() {
        return Err(TaoError::InvalidData(
            "HEVC: 构建 hvcC 需要至少一个 SPS".into(),
        ));
    }

    // 从 SPS NAL 中提取 profile/level 信息
    // SPS NAL 头 (2 bytes) + sps_video_parameter_set_id(4) + max_sub_layers(3) + temporal_id_nesting(1) + profile_tier_level(...)
    let sps_data = sps_list[0];
    let (general_profile_idc, general_level_idc) = if sps_data.len() >= 15 {
        // 跳过 2 字节 NAL 头, 解析 profile_tier_level
        // profile_tier_level 在 SPS 的 byte[2] 之后
        let rbsp = remove_emulation_prevention(&sps_data[2..]);
        if rbsp.len() >= 12 {
            // general_profile_space(2) + general_tier_flag(1) + general_profile_idc(5) = byte 1
            let profile = rbsp[1] & 0x1F;
            // general_level_idc 在第 12 字节 (index 11)
            let level = if rbsp.len() > 11 { rbsp[11] } else { 0 };
            (profile, level)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let mut buf = Vec::new();

    // configurationVersion = 1
    buf.push(1);
    // general_profile_space(2) | general_tier_flag(1) | general_profile_idc(5)
    buf.push(general_profile_idc & 0x1F);
    // general_profile_compatibility_flags (32 bits)
    buf.extend_from_slice(&[0; 4]);
    // general_constraint_indicator_flags (48 bits)
    buf.extend_from_slice(&[0; 6]);
    // general_level_idc
    buf.push(general_level_idc);
    // min_spatial_segmentation_idc (reserved 4 bits + 12 bits)
    buf.extend_from_slice(&[0xF0, 0x00]);
    // parallelismType (reserved 6 bits + 2 bits)
    buf.push(0xFC);
    // chromaFormat (reserved 6 bits + 2 bits) - 4:2:0 = 1
    buf.push(0xFD);
    // bitDepthLumaMinus8 (reserved 5 bits + 3 bits)
    buf.push(0xF8);
    // bitDepthChromaMinus8 (reserved 5 bits + 3 bits)
    buf.push(0xF8);
    // avgFrameRate
    buf.extend_from_slice(&[0, 0]);
    // constantFrameRate(2) | numTemporalLayers(3) | temporalIdNested(1) | lengthSizeMinusOne(2)
    buf.push(0x03); // lengthSizeMinusOne = 3 (4 bytes)

    // numOfArrays
    let mut num_arrays = 0u8;
    if !vps_list.is_empty() {
        num_arrays += 1;
    }
    if !sps_list.is_empty() {
        num_arrays += 1;
    }
    if !pps_list.is_empty() {
        num_arrays += 1;
    }
    buf.push(num_arrays);

    // VPS array
    if !vps_list.is_empty() {
        buf.push(0x20); // array_completeness=0, NAL_unit_type=32 (VPS)
        buf.extend_from_slice(&(vps_list.len() as u16).to_be_bytes());
        for vps in vps_list {
            buf.extend_from_slice(&(vps.len() as u16).to_be_bytes());
            buf.extend_from_slice(vps);
        }
    }

    // SPS array
    buf.push(0x21); // NAL_unit_type=33 (SPS)
    buf.extend_from_slice(&(sps_list.len() as u16).to_be_bytes());
    for sps in sps_list {
        buf.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        buf.extend_from_slice(sps);
    }

    // PPS array
    if !pps_list.is_empty() {
        buf.push(0x22); // NAL_unit_type=34 (PPS)
        buf.extend_from_slice(&(pps_list.len() as u16).to_be_bytes());
        for pps in pps_list {
            buf.extend_from_slice(&(pps.len() as u16).to_be_bytes());
            buf.extend_from_slice(pps);
        }
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hevc_nal_类型() {
        assert_eq!(HevcNalUnitType::from_type_id(19), HevcNalUnitType::IdrWRadl);
        assert_eq!(HevcNalUnitType::from_type_id(32), HevcNalUnitType::Vps);
        assert_eq!(HevcNalUnitType::from_type_id(33), HevcNalUnitType::Sps);
        assert_eq!(HevcNalUnitType::from_type_id(34), HevcNalUnitType::Pps);
        assert!(HevcNalUnitType::IdrWRadl.is_idr());
        assert!(HevcNalUnitType::IdrWRadl.is_irap());
        assert!(!HevcNalUnitType::TrailR.is_irap());
        assert!(HevcNalUnitType::TrailR.is_vcl());
        assert!(!HevcNalUnitType::Vps.is_vcl());
    }

    #[test]
    fn test_hevc_nal_解析() {
        // NAL 头: type=33 (SPS), layer_id=0, temporal_id=1
        // byte0 = (33 << 1) | 0 = 0x42
        // byte1 = (0 << 3) | 1 = 0x01
        let data = vec![0x42, 0x01, 0xAA, 0xBB];
        let nalu = HevcNalUnit::parse(&data).unwrap();
        assert_eq!(nalu.nal_type, HevcNalUnitType::Sps);
        assert_eq!(nalu.layer_id, 0);
        assert_eq!(nalu.temporal_id_plus1, 1);
        assert_eq!(nalu.data, vec![0xAA, 0xBB]);
    }

    #[test]
    fn test_annex_b_分割() {
        let mut data = Vec::new();
        // VPS NAL: type=32 -> byte0=(32<<1)|0=0x40, byte1=0x01
        data.extend_from_slice(&[0, 0, 0, 1, 0x40, 0x01, 0x11, 0x22]);
        // SPS NAL: type=33 -> byte0=(33<<1)|0=0x42, byte1=0x01
        data.extend_from_slice(&[0, 0, 1, 0x42, 0x01, 0x33]);
        // PPS NAL: type=34 -> byte0=(34<<1)|0=0x44, byte1=0x01
        data.extend_from_slice(&[0, 0, 0, 1, 0x44, 0x01, 0x44]);

        let nalus = split_hevc_annex_b(&data);
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0].nal_type, HevcNalUnitType::Vps);
        assert_eq!(nalus[1].nal_type, HevcNalUnitType::Sps);
        assert_eq!(nalus[2].nal_type, HevcNalUnitType::Pps);
    }

    #[test]
    fn test_annex_b_hvcc_往返() {
        let mut annex_b = Vec::new();
        annex_b.extend_from_slice(&[0, 0, 0, 1, 0x40, 0x01, 0xAA]);
        annex_b.extend_from_slice(&[0, 0, 0, 1, 0x42, 0x01, 0xBB]);

        let hvcc = hevc_annex_b_to_hvcc(&annex_b);
        let back = hevc_hvcc_to_annex_b(&hvcc, 4);
        let nalus_orig = split_hevc_annex_b(&annex_b);
        let nalus_back = split_hevc_annex_b(&back);

        assert_eq!(nalus_orig.len(), nalus_back.len());
        for (a, b) in nalus_orig.iter().zip(nalus_back.iter()) {
            assert_eq!(a.nal_type, b.nal_type);
            assert_eq!(a.data, b.data);
        }
    }

    #[test]
    fn test_emulation_prevention() {
        let data = [0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x00];
        let rbsp = remove_emulation_prevention(&data);
        assert_eq!(rbsp, vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_hvcc_config_构建与解析() {
        // 构建最小 VPS/SPS/PPS NAL (含 2 字节头)
        let vps = vec![
            0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF, 0x01, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x5D, 0xAC, 0x09,
        ];
        let sps = vec![
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x5D, 0xA0, 0x02, 0x80, 0x80,
        ];
        let pps = vec![0x44, 0x01, 0xC1, 0x72, 0xB4, 0x62, 0x40];

        let config =
            build_hvcc_config(&[vps.as_slice()], &[sps.as_slice()], &[pps.as_slice()]).unwrap();

        let parsed = parse_hvcc_config(&config).unwrap();
        assert_eq!(parsed.length_size, 4);
        assert_eq!(parsed.vps_list.len(), 1);
        assert_eq!(parsed.sps_list.len(), 1);
        assert_eq!(parsed.pps_list.len(), 1);
        assert_eq!(parsed.vps_list[0], vps);
        assert_eq!(parsed.sps_list[0], sps);
        assert_eq!(parsed.pps_list[0], pps);
    }
}
