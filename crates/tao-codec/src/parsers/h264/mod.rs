//! H.264/AVC 码流解析器.
//!
//! 提供对 H.264 Annex B 和 AVCC 格式码流的解析能力:
//! - NAL 单元分割与类型识别
//! - SPS (Sequence Parameter Set) 解析
//! - PPS (Picture Parameter Set) 解析
//! - Annex B ↔ AVCC 格式转换

pub mod nal;
pub mod sps;

pub use nal::{
    AvccConfig, NalUnit, NalUnitType, annex_b_to_avcc, avcc_to_annex_b, build_avcc_config,
    parse_avcc_config, split_annex_b, split_avcc,
};
pub use sps::{Sps, parse_sps};
