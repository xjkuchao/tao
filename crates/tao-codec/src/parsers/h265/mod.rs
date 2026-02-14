//! H.265/HEVC 码流解析器.
//!
//! 提供对 H.265 HEVC 码流的解析能力:
//! - NAL 单元分割与类型识别 (2 字节 NAL 头)
//! - VPS (Video Parameter Set) 解析
//! - SPS (Sequence Parameter Set) 解析
//! - PPS (Picture Parameter Set) 解析
//! - Annex B ↔ HVCC 格式转换
//!
//! # HEVC NAL 头部 (2 字节)
//! ```text
//! ┌────────────────────────────────────────────┐
//! │ forbidden(1) | type(6) | layer_id(6) | tid(3) │
//! └────────────────────────────────────────────┘
//! ```

pub mod nal;
pub mod sps;

pub use nal::{
    HevcNalUnit, HevcNalUnitType, HvccConfig, build_hvcc_config, hevc_annex_b_to_hvcc,
    hevc_hvcc_to_annex_b, parse_hvcc_config, split_hevc_annex_b, split_hevc_hvcc,
};
pub use sps::{HevcSps, HevcVps, parse_hevc_sps, parse_hevc_vps};
