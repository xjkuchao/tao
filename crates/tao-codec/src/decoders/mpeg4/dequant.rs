//! 反量化 (H.263 和 MPEG 两种类型)

use super::Mpeg4Decoder;

const COEFF_MIN: i32 = -2048;
const COEFF_MAX: i32 = 2047;

impl Mpeg4Decoder {
    /// 反量化 (区分 H.263 和 MPEG 类型)
    pub(super) fn dequantize(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let quant_type = self.vol_info.as_ref().map(|v| v.quant_type).unwrap_or(0);
        let quant = quant.max(1);

        if quant_type == 0 {
            self.dequant_h263(coefficients, quant, is_intra);
        } else {
            self.dequant_mpeg(coefficients, quant, is_intra);
        }
    }

    /// H.263 反量化
    ///
    /// 对标 FFmpeg dct_unquantize_h263_intra_c / dct_unquantize_h263_inter_c.
    /// Intra/Inter AC 系数均使用: level * 2*QP +/- qadd
    /// 其中 qadd = (QP - 1) | 1
    fn dequant_h263(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let quant_m2 = (quant * 2) as i32;
        let quant_add = if quant % 2 != 0 {
            quant as i32
        } else {
            (quant as i32) - 1
        };

        let start = if is_intra { 1 } else { 0 };

        for coeff in coefficients.iter_mut().skip(start) {
            let level = *coeff;
            if level == 0 {
                continue;
            }
            let value = if level < 0 {
                level * quant_m2 - quant_add
            } else {
                level * quant_m2 + quant_add
            };
            *coeff = value.clamp(COEFF_MIN, COEFF_MAX);
        }
    }

    /// MPEG 反量化
    fn dequant_mpeg(&self, coefficients: &mut [i32; 64], quant: u32, is_intra: bool) {
        let matrix = if is_intra {
            &self.quant_matrix_intra
        } else {
            &self.quant_matrix_inter
        };

        let start = if is_intra { 1 } else { 0 };
        let mut sum: u32 = 0;

        for i in start..64 {
            let level = coefficients[i];
            if level == 0 {
                continue;
            }
            let scale = matrix[i] as i32;
            if is_intra {
                coefficients[i] = (level * quant as i32 * scale) >> 4;
            } else {
                let sign = level < 0;
                let abs_level = level.unsigned_abs() as i32;
                let val = ((2 * abs_level + 1) * scale * quant as i32) >> 4;
                let value = if sign { -val } else { val };
                coefficients[i] = value.clamp(COEFF_MIN, COEFF_MAX);
            }
            sum ^= coefficients[i] as u32;
        }

        // Mismatch control
        if (sum & 1) == 0 {
            coefficients[63] ^= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::gmc::GmcParameters;
    use super::super::tables::{STD_INTER_QUANT_MATRIX, STD_INTRA_QUANT_MATRIX};
    use super::super::types::MacroblockInfo;
    use super::*;
    use tao_core::PixelFormat;

    fn create_decoder_for_test() -> Mpeg4Decoder {
        Mpeg4Decoder {
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Yuv420p,
            opened: false,
            reference_frame: None,
            backward_reference: None,
            pending_frame: None,
            dpb: Vec::new(),
            frame_count: 0,
            quant: 1,
            vol_info: None,
            quant_matrix_intra: STD_INTRA_QUANT_MATRIX,
            quant_matrix_inter: STD_INTER_QUANT_MATRIX,
            predictor_cache: Vec::new(),
            mv_cache: Vec::new(),
            ref_mv_cache: Vec::new(),
            mb_info: vec![MacroblockInfo::default(); 1],
            mb_stride: 0,
            f_code_forward: 1,
            f_code_backward: 1,
            rounding_control: 0,
            intra_dc_vlc_thr: 0,
            time_pp: 0,
            time_bp: 0,
            last_time_base: 0,
            time_base_acc: 0,
            last_non_b_time: 0,
            gmc_params: GmcParameters::default(),
            alternate_vertical_scan: false,
            packed_frames: std::collections::VecDeque::new(),
            resync_mb_x: 0,
            resync_mb_y: 0,
        }
    }

    #[test]
    fn test_dequant_h263_clipping() {
        let decoder = create_decoder_for_test();
        let mut coefficients = [0i32; 64];
        coefficients[1] = 2000;
        coefficients[2] = -2000;

        decoder.dequant_h263(&mut coefficients, 31, false);

        assert_eq!(coefficients[1], COEFF_MAX, "H.263 反量化应裁剪上限");
        assert_eq!(coefficients[2], COEFF_MIN, "H.263 反量化应裁剪下限");
    }

    #[test]
    fn test_dequant_mpeg_mismatch_intra() {
        let decoder = create_decoder_for_test();
        let mut coefficients = [0i32; 64];
        coefficients[1] = 2;

        decoder.dequant_mpeg(&mut coefficients, 1, true);

        assert_eq!(coefficients[63], 1, "Mismatch control 应应用到 Intra 块");
    }
}
