//! 反量化 (H.263 和 MPEG 两种类型)

use super::Mpeg4Decoder;

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
            if is_intra {
                *coeff = level * quant_m2;
            } else if level < 0 {
                *coeff = level * quant_m2 - quant_add;
            } else {
                *coeff = level * quant_m2 + quant_add;
            }
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
                coefficients[i] = if sign {
                    -(val.min(2048))
                } else {
                    val.min(2047)
                };
            }
            sum ^= coefficients[i] as u32;
        }

        // Mismatch control
        if !is_intra && (sum & 1) == 0 {
            coefficients[63] ^= 1;
        }
    }
}
