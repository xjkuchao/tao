//! VOL/VOP 头部解析

use log::{debug, warn};
use tao_core::TaoError;

use super::Mpeg4Decoder;
use super::bitreader::{BitReader, find_start_code_range};
use super::tables::ZIGZAG_SCAN;
use super::types::{VolInfo, VopInfo};
use crate::frame::PictureType;

/// MPEG-4 起始码
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT_SEQUENCE: u8 = 0xB0;
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT: u8 = 0xB5;
pub(super) const START_CODE_VOP: u8 = 0xB6;
const START_CODE_VIDEO_OBJECT_LAYER: u8 = 0x20; // 0x20-0x2F

/// VOP 编码类型
const VOP_TYPE_I: u8 = 0;
const VOP_TYPE_P: u8 = 1;
const VOP_TYPE_B: u8 = 2;
const VOP_TYPE_S: u8 = 3;

/// 读取自定义量化矩阵
fn read_quant_matrix(reader: &mut BitReader) -> Option<[u8; 64]> {
    let mut matrix = [0u8; 64];
    for &pos in ZIGZAG_SCAN.iter() {
        let val = reader.read_bits(8)? as u8;
        matrix[pos] = if val == 0 { 1 } else { val };
    }
    Some(matrix)
}

impl Mpeg4Decoder {
    /// 解析 VOL 头部
    pub(super) fn parse_vol_header(&mut self, data: &[u8]) -> Result<(), tao_core::TaoError> {
        let (code, offset) = match find_start_code_range(
            data,
            START_CODE_VIDEO_OBJECT_LAYER,
            START_CODE_VIDEO_OBJECT_LAYER + 0x0F,
        ) {
            Some(value) => value,
            None => return Ok(()),
        };

        debug!("找到 VOL 起始码: 0x{:02X}", code);
        let mut reader = BitReader::new(&data[offset..]);

        let _random_accessible_vol = reader.read_bit();
        let _video_object_type_indication = reader.read_bits(8);
        let is_object_layer_identifier = reader.read_bit().unwrap_or(false);
        if is_object_layer_identifier {
            let _verid = reader.read_bits(4);
            let _priority = reader.read_bits(3);
        }

        let aspect_ratio_info = reader.read_bits(4).unwrap_or(0);
        if aspect_ratio_info == 0xF {
            let _par_w = reader.read_bits(8);
            let _par_h = reader.read_bits(8);
        }

        let vol_control = reader.read_bit().unwrap_or(false);
        if vol_control {
            let _chroma = reader.read_bits(2);
            let _low_delay = reader.read_bit();
            let vbv = reader.read_bit().unwrap_or(false);
            if vbv {
                let _peak = reader.read_bits(15);
                reader.read_bit();
                let _buf = reader.read_bits(15);
                reader.read_bit();
                let _occ = reader.read_bits(15);
                reader.read_bit();
            }
        }

        let shape = reader.read_bits(2).unwrap_or(0);
        reader.read_bit(); // marker
        let time_res = reader.read_bits(16).unwrap_or(30000) as u16;
        reader.read_bit(); // marker
        let fixed_rate = reader.read_bit().unwrap_or(false);

        if fixed_rate {
            let bits = (time_res as f32).log2().ceil() as u8;
            reader.read_bits(bits.max(1));
        }

        if shape == 0 {
            reader.read_bit(); // marker
            let _vol_w = reader.read_bits(13);
            reader.read_bit(); // marker
            let _vol_h = reader.read_bits(13);
            reader.read_bit(); // marker
        }

        let interlacing = reader.read_bit().unwrap_or(false);
        let _obmc_disable = reader.read_bit();

        let sprite_enable = reader.read_bits(1).unwrap_or(0) as u8;
        let mut sprite_warping_points = 0u8;
        if sprite_enable == 1 || sprite_enable == 2 {
            if sprite_enable != 2 {
                reader.read_bits(13); // sprite_width
                reader.read_bit();
                reader.read_bits(13); // sprite_height
                reader.read_bit();
                reader.read_bits(13); // sprite_left
                reader.read_bit();
                reader.read_bits(13); // sprite_top
                reader.read_bit();
            }
            sprite_warping_points = reader.read_bits(6).unwrap_or(0) as u8;
            let _sprite_warping_accuracy = reader.read_bits(2);
            let _sprite_brightness = reader.read_bit();
            if sprite_enable != 2 {
                let _low_latency = reader.read_bit();
            }
        }

        let _not_8_bit = reader.read_bit();
        if _not_8_bit == Some(true) {
            reader.read_bits(4); // quant_precision
            reader.read_bits(4); // bits_per_pixel
        }

        let quant_type = if reader.read_bit().unwrap_or(false) {
            let load_intra = reader.read_bit().unwrap_or(false);
            if load_intra {
                if let Some(matrix) = read_quant_matrix(&mut reader) {
                    self.quant_matrix_intra = matrix;
                }
            }
            let load_inter = reader.read_bit().unwrap_or(false);
            if load_inter {
                if let Some(matrix) = read_quant_matrix(&mut reader) {
                    self.quant_matrix_inter = matrix;
                }
            }
            1u8
        } else {
            0u8
        };

        let quarterpel = reader.read_bit().unwrap_or(false);

        let complexity_disable = reader.read_bit().unwrap_or(true);
        if !complexity_disable {
            warn!("VOL: complexity_estimation 未完全解析, 可能导致后续字段偏移");
        }

        let resync_marker_disable = reader.read_bit().unwrap_or(true);

        let data_partitioned = reader.read_bit().unwrap_or(false);
        if data_partitioned {
            let _reversible_vlc = reader.read_bit();
        }

        self.vol_info = Some(VolInfo {
            vop_time_increment_resolution: time_res,
            fixed_vop_rate: fixed_rate,
            data_partitioned,
            quant_type,
            interlacing,
            quarterpel,
            sprite_enable,
            sprite_warping_points,
            resync_marker_disable,
        });

        debug!(
            "VOL: time_res={}, quant_type={}, interlaced={}, quarterpel={}, sprite={}",
            time_res, quant_type, interlacing, quarterpel, sprite_enable
        );

        Ok(())
    }

    /// 解析 VOP 头部
    pub(super) fn parse_vop_header(&mut self, reader: &mut BitReader) -> Result<VopInfo, TaoError> {
        let vop_type = reader
            .read_bits(2)
            .ok_or_else(|| TaoError::InvalidData("无法读取 VOP 编码类型".into()))?;

        let picture_type = match vop_type as u8 {
            VOP_TYPE_I => PictureType::I,
            VOP_TYPE_P => PictureType::P,
            VOP_TYPE_B => PictureType::B,
            VOP_TYPE_S => PictureType::I,
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "未知 VOP 类型: {}",
                    vop_type
                )));
            }
        };

        debug!("VOP 类型: {:?}", picture_type);

        // modulo_time_base
        while reader.read_bit() == Some(true) {}
        reader.read_bit(); // marker

        // vop_time_increment
        if let Some(vol) = &self.vol_info {
            let bits = (vol.vop_time_increment_resolution as f32).log2().ceil() as u8;
            reader.read_bits(bits.max(1));
        }

        reader.read_bit(); // marker
        let vop_coded = reader.read_bit().unwrap_or(true);

        if !vop_coded {
            debug!("VOP 未编码");
            return Ok(VopInfo {
                picture_type,
                vop_coded: false,
                vop_rounding_type: 0,
                intra_dc_vlc_thr: 0,
            });
        }

        // P-VOP: rounding_type 在 intra_dc_vlc_thr 之前
        if picture_type == PictureType::P {
            self.rounding_control = reader.read_bit().unwrap_or(false) as u8;
        }

        // intra_dc_vlc_thr
        let intra_dc_vlc_thr = if picture_type != PictureType::B {
            reader.read_bits(3).unwrap_or(0)
        } else {
            0
        };
        self.intra_dc_vlc_thr = intra_dc_vlc_thr;

        // vop_quant
        if let Some(quant) = reader.read_bits(5) {
            if quant > 0 {
                self.quant = quant as u8;
            }
        }

        // P-VOP: f_code_forward
        if picture_type == PictureType::P {
            if let Some(f) = reader.read_bits(3) {
                self.f_code_forward = f as u8;
            }
        }

        // B-VOP: f_code_forward + f_code_backward
        if picture_type == PictureType::B {
            if let Some(f) = reader.read_bits(3) {
                self.f_code_forward = f as u8;
            }
            if let Some(f) = reader.read_bits(3) {
                self.f_code_backward = f as u8;
            }
        }

        debug!(
            "VOP 头: quant={}, rounding={}, f_code_fwd={}, dc_thr={}",
            self.quant, self.rounding_control, self.f_code_forward, intra_dc_vlc_thr
        );

        Ok(VopInfo {
            picture_type,
            vop_coded: true,
            vop_rounding_type: self.rounding_control,
            intra_dc_vlc_thr,
        })
    }
}
