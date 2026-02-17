//! VOL/VOP 头部解析

use log::{debug, trace, warn};
use tao_core::TaoError;

use super::Mpeg4Decoder;
use super::bitreader::{BitReader, find_start_code_range};
use super::tables::ZIGZAG_SCAN;
use super::types::{EncoderInfo, EncoderType, VolInfo, VopInfo};
use crate::frame::PictureType;

/// MPEG-4 起始码
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT_SEQUENCE: u8 = 0xB0;
#[allow(dead_code)]
const START_CODE_VISUAL_OBJECT: u8 = 0xB5;
pub(super) const START_CODE_VOP: u8 = 0xB6;
pub(super) const START_CODE_USER_DATA: u8 = 0xB2;
const START_CODE_VIDEO_OBJECT_LAYER: u8 = 0x20; // 0x20-0x2F

/// VOP 编码类型
const VOP_TYPE_I: u8 = 0;
const VOP_TYPE_P: u8 = 1;
const VOP_TYPE_B: u8 = 2;
const VOP_TYPE_S: u8 = 3;

/// 从指定偏移查找下一个起始码 (00 00 01), 返回起始码的位置
fn find_next_start_code(data: &[u8], start: usize) -> Option<usize> {
    if data.len() < start + 3 {
        return None;
    }
    (start..(data.len() - 2))
        .find(|&idx| data[idx] == 0x00 && data[idx + 1] == 0x00 && data[idx + 2] == 0x01)
}

/// 读取自定义量化矩阵
fn read_quant_matrix(reader: &mut BitReader) -> Option<[u8; 64]> {
    let mut matrix = [0u8; 64];
    for &pos in ZIGZAG_SCAN.iter() {
        let val = reader.read_bits(8)? as u8;
        matrix[pos] = if val == 0 { 1 } else { val };
    }
    Some(matrix)
}

fn read_marker_bit(reader: &mut BitReader, context: &str) -> bool {
    match reader.read_bit() {
        Some(true) => true,
        _ => {
            warn!("复杂度估计 marker 缺失: {}", context);
            false
        }
    }
}

fn parse_complexity_estimation(reader: &mut BitReader) -> (u16, u16, u16) {
    let mut bits_i = 0u16;
    let mut bits_p = 0u16;
    let mut bits_b = 0u16;

    let snapshot = reader.snapshot_position();
    let estimation_method = match reader.read_bits(2) {
        Some(value) => value as u8,
        None => return (0, 0, 0),
    };

    if estimation_method >= 2 {
        warn!("复杂度估计方法非法: {}", estimation_method);
        reader.restore_position(snapshot);
        return (0, 0, 0);
    }

    let shape_disable = reader.read_bit().unwrap_or(true);
    if !shape_disable {
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
    }

    let texture_disable = reader.read_bit().unwrap_or(true);
    if !texture_disable {
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
    }

    if !read_marker_bit(reader, "分段1") {
        reader.restore_position(snapshot);
        return (0, 0, 0);
    }

    let dct_disable = reader.read_bit().unwrap_or(true);
    if !dct_disable {
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_i += 4;
        }
    }

    let motion_disable = reader.read_bit().unwrap_or(true);
    if !motion_disable {
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_b += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
    }

    if !read_marker_bit(reader, "分段2") {
        reader.restore_position(snapshot);
        return (0, 0, 0);
    }

    if estimation_method == 1 {
        if reader.read_bit().unwrap_or(false) {
            bits_i += 8;
        }
        if reader.read_bit().unwrap_or(false) {
            bits_p += 8;
        }
    }

    (bits_i, bits_p, bits_b)
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
        let mut video_object_layer_verid = 1u8;
        if is_object_layer_identifier {
            video_object_layer_verid = reader.read_bits(4).unwrap_or(1) as u8;
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
            let vol_w = reader.read_bits(13).unwrap_or(0);
            reader.read_bit(); // marker
            let vol_h = reader.read_bits(13).unwrap_or(0);
            reader.read_bit(); // marker

            // 保存 VOL 宽度和高度到解码器
            if vol_w > 0 && vol_h > 0 {
                self.width = vol_w;
                self.height = vol_h;
                debug!("从 VOL 解析到尺寸: {}x{}", vol_w, vol_h);
            }
        }

        let interlacing = reader.read_bit().unwrap_or(false);
        let _obmc_disable = reader.read_bit();

        let sprite_enable = if video_object_layer_verid >= 2 {
            reader.read_bits(2).unwrap_or(0) as u8
        } else {
            reader.read_bits(1).unwrap_or(0) as u8
        };
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

        // quarterpel 仅在 vo_ver_id != 1 时从比特流读取 (MPEG-4 标准)
        let quarterpel = if video_object_layer_verid != 1 {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        let complexity_disable = reader.read_bit().unwrap_or(true);
        let (complexity_bits_i, complexity_bits_p, complexity_bits_b) = if !complexity_disable {
            parse_complexity_estimation(&mut reader)
        } else {
            (0, 0, 0)
        };

        let resync_marker_disable = reader.read_bit().unwrap_or(true);

        let data_partitioned = reader.read_bit().unwrap_or(false);
        let mut reversible_vlc = false;
        if data_partitioned {
            reversible_vlc = reader.read_bit().unwrap_or(false);
        }

        self.vol_info = Some(VolInfo {
            video_object_layer_verid,
            vop_time_increment_resolution: time_res,
            fixed_vop_rate: fixed_rate,
            data_partitioned,
            reversible_vlc,
            quant_type,
            interlacing,
            quarterpel,
            sprite_enable,
            sprite_warping_points,
            complexity_estimation_bits_i: complexity_bits_i,
            complexity_estimation_bits_p: complexity_bits_p,
            complexity_estimation_bits_b: complexity_bits_b,
            resync_marker_disable,
            encoder_info: EncoderInfo::default(),
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
            VOP_TYPE_S => PictureType::S,
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "未知 VOP 类型: {}",
                    vop_type
                )));
            }
        };

        trace!("VOP 类型: {:?}", picture_type);

        // modulo_time_base (计数 '1' 位)
        let mut modulo_time_incr = 0i32;
        while reader.read_bit() == Some(true) {
            modulo_time_incr += 1;
        }
        reader.read_bit(); // marker

        // vop_time_increment
        let time_inc_resolution = self
            .vol_info
            .as_ref()
            .map(|v| v.vop_time_increment_resolution)
            .unwrap_or(30000);
        let time_inc_bits = if time_inc_resolution > 1 {
            (time_inc_resolution as f32).log2().ceil() as u8
        } else {
            1
        };
        let vop_time_increment = reader.read_bits(time_inc_bits.max(1)).unwrap_or(0) as i32;

        // 时间跟踪 (计算 TRD/TRB, 用于 B 帧 Direct 模式)
        let resolution = time_inc_resolution as i32;
        if picture_type != PictureType::B {
            self.last_time_base = self.time_base_acc;
            self.time_base_acc += modulo_time_incr;
            let abs_time = self.time_base_acc * resolution + vop_time_increment;
            self.time_pp = abs_time - self.last_non_b_time;
            if self.time_pp <= 0 {
                self.time_pp = 1;
            }
            self.last_non_b_time = abs_time;
        } else {
            let abs_time =
                (self.last_time_base + modulo_time_incr) * resolution + vop_time_increment;
            self.time_bp = abs_time - self.last_non_b_time;
        }

        reader.read_bit(); // marker
        let vop_coded = reader.read_bit().unwrap_or(true);

        let is_sprite = picture_type == PictureType::S;

        if !vop_coded {
            debug!("VOP 未编码");
            return Ok(VopInfo {
                picture_type,
                vop_coded: false,
                is_sprite,
                vop_rounding_type: 0,
                intra_dc_vlc_thr: 0,
                alternate_vertical_scan_flag: false,
            });
        }

        // P-VOP: rounding_type 在 intra_dc_vlc_thr 之前
        if picture_type == PictureType::P {
            self.rounding_control = reader.read_bit().unwrap_or(false) as u8;
        }

        // 复杂度估计 (仅用于跳过比特)
        if let Some(vol_info) = self.vol_info.as_ref() {
            if vol_info.complexity_estimation_bits_i > 0 {
                reader.skip_bits(vol_info.complexity_estimation_bits_i as u32);
            }
            if picture_type != PictureType::I && vol_info.complexity_estimation_bits_p > 0 {
                reader.skip_bits(vol_info.complexity_estimation_bits_p as u32);
            }
            if picture_type == PictureType::B && vol_info.complexity_estimation_bits_b > 0 {
                reader.skip_bits(vol_info.complexity_estimation_bits_b as u32);
            }
        }

        // intra_dc_vlc_thr
        let intra_dc_vlc_thr = if picture_type != PictureType::B {
            reader.read_bits(3).unwrap_or(0)
        } else {
            0
        };
        self.intra_dc_vlc_thr = intra_dc_vlc_thr;

        let alternate_vertical_scan_flag = if self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false)
        {
            let _top_field_first = reader.read_bit().unwrap_or(false);
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };
        self.alternate_vertical_scan = alternate_vertical_scan_flag;

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

        trace!(
            "VOP 头: type={:?}, quant={}, rounding={}, f_fwd={}, f_bwd={}, dc_thr={}, time_pp={}, time_bp={}",
            picture_type,
            self.quant,
            self.rounding_control,
            self.f_code_forward,
            self.f_code_backward,
            intra_dc_vlc_thr,
            self.time_pp,
            self.time_bp
        );

        Ok(VopInfo {
            picture_type,
            vop_coded: true,
            is_sprite,
            vop_rounding_type: self.rounding_control,
            intra_dc_vlc_thr,
            alternate_vertical_scan_flag,
        })
    }

    /// 解析 user_data, 识别编码器类型和版本
    ///
    /// user_data 位于起始码 0x000001B2 之后, 直到下一个起始码为止.
    /// 常见编码器在 user_data 中写入标识字符串:
    /// - DivX: "DivX503b1393p" 或类似格式, 末尾 'p' 表示 packed bitstream
    /// - Xvid: "XviD" + 4字节 build 号
    /// - FFmpeg/Lavc: "Lavc" 或 "FFmpeg" 前缀
    pub(super) fn parse_user_data(&mut self, data: &[u8]) {
        // 查找所有 user_data 起始码
        let mut offset = 0;
        while offset + 4 < data.len() {
            // 查找 00 00 01 B2
            if data[offset] == 0x00
                && data[offset + 1] == 0x00
                && data[offset + 2] == 0x01
                && data[offset + 3] == START_CODE_USER_DATA
            {
                let ud_start = offset + 4;
                // user_data 持续到下一个起始码 (00 00 01 xx)
                let ud_end = find_next_start_code(data, ud_start).unwrap_or(data.len());
                let ud_bytes = &data[ud_start..ud_end];

                if let Some(info) = Self::identify_encoder(ud_bytes) {
                    debug!(
                        "识别编码器: {:?}, 版本={}, build={}, packed={}",
                        info.encoder_type, info.version, info.build, info.packed_bitstream
                    );
                    if let Some(vol) = self.vol_info.as_mut() {
                        vol.encoder_info = info;
                    }
                    return;
                }
            }
            offset += 1;
        }
    }

    /// 从 user_data 字节中识别编码器类型
    fn identify_encoder(ud_bytes: &[u8]) -> Option<EncoderInfo> {
        // 转换为 ASCII 字符串 (忽略非 ASCII 字节)
        let text: String = ud_bytes
            .iter()
            .take_while(|&&b| b != 0x00)
            .map(|&b| b as char)
            .collect();

        if text.is_empty() {
            return None;
        }

        // DivX 检测: "DivX" 后跟版本号, 末尾可能有 'p' (packed bitstream)
        if let Some(divx_pos) = text.find("DivX") {
            let after_divx = &text[divx_pos + 4..];
            let (version, build, packed) = Self::parse_divx_version(after_divx);
            return Some(EncoderInfo {
                encoder_type: EncoderType::DivX,
                version,
                build,
                packed_bitstream: packed,
            });
        }

        // Xvid 检测: "XviD" 后跟 4 字节 build 号
        if text.contains("XviD") || text.contains("xvid") || text.contains("Xvid") {
            let build = if ud_bytes.len() >= 8 {
                // Xvid 在 "XviD" 之后放 4 字节小端 build 号
                let xvid_pos = text
                    .find("XviD")
                    .or_else(|| text.find("xvid"))
                    .or_else(|| text.find("Xvid"))
                    .unwrap_or(0);
                let build_offset = xvid_pos + 4;
                if build_offset + 4 <= ud_bytes.len() {
                    u32::from_le_bytes([
                        ud_bytes[build_offset],
                        ud_bytes[build_offset + 1],
                        ud_bytes[build_offset + 2],
                        ud_bytes[build_offset + 3],
                    ])
                } else {
                    0
                }
            } else {
                0
            };
            return Some(EncoderInfo {
                encoder_type: EncoderType::Xvid,
                version: 0,
                build,
                packed_bitstream: false,
            });
        }

        // FFmpeg/Lavc 检测
        if text.starts_with("Lavc") || text.starts_with("FFmpeg") {
            let version = Self::parse_lavc_version(&text);
            return Some(EncoderInfo {
                encoder_type: EncoderType::Lavc,
                version,
                build: 0,
                packed_bitstream: false,
            });
        }

        None
    }

    /// 解析 DivX 版本字符串, 如 "503b1393p"
    ///
    /// 格式: 主版本(1-3位数) + 子版本(可选) + 'b' + build号 + 可选'p'(packed)
    /// 例如: "503b1393p" -> version=503, build=1393, packed=true
    fn parse_divx_version(s: &str) -> (u32, u32, bool) {
        let packed = s.ends_with('p');

        // 提取版本号 (前面的数字)
        let mut version = 0u32;
        let mut chars = s.chars().peekable();
        while let Some(&ch) = chars.peek() {
            if ch.is_ascii_digit() {
                version = version * 10 + (ch as u32 - '0' as u32);
                chars.next();
            } else {
                break;
            }
        }

        // 跳过 'b' 或 'Build'
        let mut build = 0u32;
        if chars.peek() == Some(&'b') || chars.peek() == Some(&'B') {
            chars.next();
            // 读取 build 号
            while let Some(&ch) = chars.peek() {
                if ch.is_ascii_digit() {
                    build = build * 10 + (ch as u32 - '0' as u32);
                    chars.next();
                } else {
                    break;
                }
            }
        }

        (version, build, packed)
    }

    /// 解析 Lavc 版本字符串, 如 "Lavc57.48.101"
    fn parse_lavc_version(s: &str) -> u32 {
        let version_str = if let Some(stripped) = s.strip_prefix("Lavc") {
            stripped
        } else if let Some(stripped) = s.strip_prefix("FFmpeg") {
            // 跳过 "FFmpeg" 前缀和可能的空格/点
            stripped.trim_start_matches(|c: char| !c.is_ascii_digit())
        } else {
            s
        };

        // 解析 "主版本.次版本.修订版本" -> 主版本 * 10000 + 次版本 * 100 + 修订版本
        let parts: Vec<u32> = version_str
            .split('.')
            .take(3)
            .filter_map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect();

        match parts.len() {
            3 => parts[0] * 10000 + parts[1] * 100 + parts[2],
            2 => parts[0] * 10000 + parts[1] * 100,
            1 => parts[0] * 10000,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::gmc::GmcParameters;
    use super::super::tables::{STD_INTER_QUANT_MATRIX, STD_INTRA_QUANT_MATRIX};
    use super::*;
    use tao_core::PixelFormat;

    struct TestBitWriter {
        data: Vec<u8>,
        cur: u8,
        filled: u8,
    }

    impl TestBitWriter {
        fn new() -> Self {
            Self {
                data: Vec::new(),
                cur: 0,
                filled: 0,
            }
        }

        fn push_bit(&mut self, bit: bool) {
            self.cur = (self.cur << 1) | (bit as u8);
            self.filled += 1;
            if self.filled == 8 {
                self.data.push(self.cur);
                self.cur = 0;
                self.filled = 0;
            }
        }

        fn push_bits(&mut self, value: u32, count: u8) {
            for idx in (0..count).rev() {
                let bit = ((value >> idx) & 1) != 0;
                self.push_bit(bit);
            }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.filled > 0 {
                self.cur <<= 8 - self.filled;
                self.data.push(self.cur);
                self.filled = 0;
            }
            self.data
        }
    }

    fn wrap_vol_start_code(mut payload: Vec<u8>) -> Vec<u8> {
        let mut data = vec![0x00, 0x00, 0x01, 0x20];
        data.append(&mut payload);
        data
    }

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
            mb_info: Vec::new(),
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
            wait_keyframe: false,
            resync_mb_x: 0,
            resync_mb_y: 0,
        }
    }

    fn write_basic_vol_header(
        writer: &mut TestBitWriter,
        interlacing: bool,
        verid: u8,
        sprite_enable: u8,
    ) {
        writer.push_bit(false);
        writer.push_bits(1, 8);

        if verid > 1 {
            writer.push_bit(true);
            writer.push_bits(verid as u32, 4);
            writer.push_bits(1, 3);
        } else {
            writer.push_bit(false);
        }

        writer.push_bits(1, 4);
        writer.push_bit(false);

        writer.push_bits(0, 2);
        writer.push_bit(true);
        writer.push_bits(1000, 16);
        writer.push_bit(true);
        writer.push_bit(false);

        writer.push_bit(true);
        writer.push_bits(320, 13);
        writer.push_bit(true);
        writer.push_bits(240, 13);
        writer.push_bit(true);

        writer.push_bit(interlacing);
        writer.push_bit(true);

        if verid >= 2 {
            writer.push_bits(sprite_enable as u32, 2);
        } else {
            writer.push_bits(sprite_enable as u32, 1);
        }

        if sprite_enable == 1 || sprite_enable == 2 {
            if sprite_enable != 2 {
                writer.push_bits(1, 13);
                writer.push_bit(true);
                writer.push_bits(1, 13);
                writer.push_bit(true);
                writer.push_bits(0, 13);
                writer.push_bit(true);
                writer.push_bits(0, 13);
                writer.push_bit(true);
            }
            writer.push_bits(1, 6);
            writer.push_bits(0, 2);
            writer.push_bit(false);
            if sprite_enable != 2 {
                writer.push_bit(false);
            }
        }

        writer.push_bit(false); // not_8_bit
        writer.push_bit(false); // quant_type = false

        // quarterpel 仅在 verid != 1 时写入
        if verid != 1 {
            writer.push_bit(false); // quarterpel = false
        }
    }

    #[test]
    fn test_vol_header_parse_basic() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let data = wrap_vol_start_code(writer.finish());
        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(vol.video_object_layer_verid, 1, "verid 应为 1");
        assert_eq!(
            vol.vop_time_increment_resolution, 1000,
            "time_res 应为 1000"
        );
        assert_eq!(vol.quant_type, 0, "量化类型应为 H.263");
        assert!(!vol.interlacing, "应为非隔行");
        assert!(!vol.quarterpel, "应为非四分像素");
        assert_eq!(vol.sprite_enable, 0, "sprite_enable 应为 0");
        assert!(vol.resync_marker_disable, "应禁用 resync marker");
        assert_eq!(
            vol.complexity_estimation_bits_i, 0,
            "复杂度估计 I 跳过位应为 0"
        );
        assert_eq!(
            vol.complexity_estimation_bits_p, 0,
            "复杂度估计 P 跳过位应为 0"
        );
        assert_eq!(
            vol.complexity_estimation_bits_b, 0,
            "复杂度估计 B 跳过位应为 0"
        );
    }

    #[test]
    fn test_vol_header_complexity_estimation() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);

        writer.push_bit(false);
        writer.push_bits(0, 2);

        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(false);

        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);

        writer.push_bit(true);

        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(true);

        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);
        writer.push_bit(true);

        writer.push_bit(true);

        writer.push_bit(true);
        writer.push_bit(false);

        let data = wrap_vol_start_code(writer.finish());
        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(
            vol.complexity_estimation_bits_i, 60,
            "复杂度估计 I 跳过位应为 60"
        );
        assert_eq!(
            vol.complexity_estimation_bits_p, 32,
            "复杂度估计 P 跳过位应为 32"
        );
        assert_eq!(
            vol.complexity_estimation_bits_b, 8,
            "复杂度估计 B 跳过位应为 8"
        );
    }

    #[test]
    fn test_vol_header_sprite_enable_verid2() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 2, 2);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let data = wrap_vol_start_code(writer.finish());
        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(vol.video_object_layer_verid, 2, "verid 应为 2");
        assert_eq!(vol.sprite_enable, 2, "sprite_enable 应为 2");
        assert_eq!(vol.sprite_warping_points, 1, "sprite_warping_points 应为 1");
    }

    #[test]
    fn test_vop_header_svop_detection() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let vol_data = wrap_vol_start_code(writer.finish());
        let mut decoder = create_decoder_for_test();
        decoder
            .parse_vol_header(&vol_data)
            .expect("解析 VOL 头失败");

        let mut vop_writer = TestBitWriter::new();
        vop_writer.push_bits(3, 2);
        vop_writer.push_bit(false);
        vop_writer.push_bit(true);
        vop_writer.push_bits(1, 10);
        vop_writer.push_bit(true);
        vop_writer.push_bit(true);
        vop_writer.push_bits(0, 3);
        vop_writer.push_bits(2, 5);

        let vop_data = vop_writer.finish();
        let mut reader = BitReader::new(&vop_data);
        let vop = decoder
            .parse_vop_header(&mut reader)
            .expect("解析 VOP 头失败");

        assert_eq!(vop.picture_type, PictureType::S, "应识别为 S-VOP");
        assert!(vop.is_sprite, "S-VOP 应标记为 sprite");
        assert!(vop.vop_coded, "S-VOP 应为编码帧");
        assert!(
            !vop.alternate_vertical_scan_flag,
            "非隔行时不应启用交错扫描"
        );
    }

    #[test]
    fn test_user_data_divx_packed() {
        // 构造: VOL 起始码 + VOL 数据 + user_data 起始码 + "DivX503b1393p"
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true); // complexity_disable
        writer.push_bit(true); // resync_marker_disable
        writer.push_bit(false); // data_partitioned = false

        let vol_data = wrap_vol_start_code(writer.finish());

        let mut data = vol_data;
        // user_data 起始码: 00 00 01 B2
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xB2]);
        // "DivX503b1393p"
        data.extend_from_slice(b"DivX503b1393p");
        data.push(0x00); // null terminator

        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");
        decoder.parse_user_data(&data);

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(
            vol.encoder_info.encoder_type,
            EncoderType::DivX,
            "应识别为 DivX 编码器"
        );
        assert_eq!(vol.encoder_info.version, 503, "DivX 版本应为 503");
        assert_eq!(vol.encoder_info.build, 1393, "DivX build 应为 1393");
        assert!(
            vol.encoder_info.packed_bitstream,
            "应检测到 packed bitstream"
        );
    }

    #[test]
    fn test_user_data_divx_non_packed() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let vol_data = wrap_vol_start_code(writer.finish());
        let mut data = vol_data;
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xB2]);
        data.extend_from_slice(b"DivX501b1018");
        data.push(0x00);

        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");
        decoder.parse_user_data(&data);

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(vol.encoder_info.encoder_type, EncoderType::DivX);
        assert_eq!(vol.encoder_info.version, 501);
        assert!(!vol.encoder_info.packed_bitstream, "不应检测到 packed");
    }

    #[test]
    fn test_user_data_xvid() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let vol_data = wrap_vol_start_code(writer.finish());
        let mut data = vol_data;
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xB2]);
        data.extend_from_slice(b"XviD");
        // 4 字节小端 build 号 = 1234
        data.extend_from_slice(&1234u32.to_le_bytes());

        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");
        decoder.parse_user_data(&data);

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(vol.encoder_info.encoder_type, EncoderType::Xvid);
        assert_eq!(vol.encoder_info.build, 1234);
    }

    #[test]
    fn test_user_data_lavc() {
        let mut writer = TestBitWriter::new();
        write_basic_vol_header(&mut writer, false, 1, 0);
        writer.push_bit(true);
        writer.push_bit(true);
        writer.push_bit(false);

        let vol_data = wrap_vol_start_code(writer.finish());
        let mut data = vol_data;
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xB2]);
        data.extend_from_slice(b"Lavc57.48.101");
        data.push(0x00);

        let mut decoder = create_decoder_for_test();
        decoder.parse_vol_header(&data).expect("解析 VOL 头失败");
        decoder.parse_user_data(&data);

        let vol = decoder.vol_info.as_ref().expect("应生成 VOL 信息");
        assert_eq!(vol.encoder_info.encoder_type, EncoderType::Lavc);
        assert_eq!(
            vol.encoder_info.version,
            57 * 10000 + 48 * 100 + 101,
            "Lavc 版本应为 574901"
        );
    }

    #[test]
    fn test_divx_version_parsing() {
        // 测试 parse_divx_version 的各种格式
        let (v, b, p) = Mpeg4Decoder::parse_divx_version("503b1393p");
        assert_eq!(v, 503);
        assert_eq!(b, 1393);
        assert!(p);

        let (v, b, p) = Mpeg4Decoder::parse_divx_version("501b1018");
        assert_eq!(v, 501);
        assert_eq!(b, 1018);
        assert!(!p);

        let (v, b, p) = Mpeg4Decoder::parse_divx_version("400");
        assert_eq!(v, 400);
        assert_eq!(b, 0);
        assert!(!p);
    }
}
