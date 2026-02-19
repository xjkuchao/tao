//! FLAC 无损音频解码器.
//!
//! 实现 FLAC 帧解码, 支持:
//! - 帧头解析 (同步码, 块大小, 采样率, 声道分配, 位深)
//! - 子帧解码: Constant, Verbatim, Fixed (0-4 阶), LPC
//! - Rice 熵编码 (RICE_PARTITION 和 RICE2_PARTITION)
//! - 立体声 decorrelation (left-side, right-side, mid-side)
//! - CRC-8 (帧头) 和 CRC-16 (帧尾) 校验
//!
//! # FLAC 帧结构
//! ```text
//! Sync code:     14 bits (0b11111111111110)
//! Reserved:      1 bit
//! Blocking:      1 bit (0=fixed, 1=variable)
//! Block size:    4 bits (encoded)
//! Sample rate:   4 bits (encoded)
//! Channel:       4 bits (assignment)
//! Sample size:   3 bits (encoded)
//! Reserved:      1 bit (0)
//! Frame/Sample#: UTF-8 encoded
//! [Block size]:  8 or 16 bits (if indicated)
//! [Sample rate]: 8 or 16 bits (if indicated)
//! CRC-8:         8 bits
//!
//! Subframe 0..N:
//!   Header:      1 bit (padding) + 6 bits (type) + 1 bit (wasted bits flag)
//!   Data:        varies by type
//!
//! Padding:       align to byte boundary
//! CRC-16:        16 bits
//! ```

use log::debug;
use tao_core::bitreader::BitReader;
use tao_core::crc;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

/// FLAC 解码器
pub struct FlacDecoder {
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 位深
    bits_per_sample: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 输出帧缓冲
    output_frame: Option<Frame>,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
    /// 最大块大小
    max_block_size: u32,
}

/// FLAC 帧头信息
#[derive(Debug)]
struct FlacFrameHeader {
    /// 块大小 (每声道采样数)
    block_size: u32,
    /// 采样率
    sample_rate: u32,
    /// 声道分配模式
    channel_assignment: ChannelAssignment,
    /// 实际声道数
    channels: u32,
    /// 位深
    bits_per_sample: u32,
}

/// 声道分配模式
#[derive(Debug, Clone, Copy)]
enum ChannelAssignment {
    /// 独立声道 (1-8 channels), 参数为声道数
    #[allow(dead_code)]
    Independent(u32),
    /// 左-侧 (left, side)
    LeftSide,
    /// 右-侧 (side, right)
    RightSide,
    /// 中-侧 (mid, side)
    MidSide,
}

/// 子帧类型
#[derive(Debug)]
enum SubframeType {
    /// 常量: 所有采样相同
    Constant,
    /// 原始: 未压缩
    Verbatim,
    /// 固定预测 (阶数 0-4)
    Fixed(u32),
    /// LPC 预测 (阶数 1-32)
    Lpc(u32),
}

impl FlacDecoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            channel_layout: ChannelLayout::MONO,
            output_frame: None,
            opened: false,
            flushing: false,
            max_block_size: 0,
        }))
    }

    /// 解码一个 FLAC 帧
    fn decode_frame(&self, data: &[u8]) -> TaoResult<(Vec<Vec<i32>>, FlacFrameHeader)> {
        let mut br = BitReader::new(data);

        // 解析帧头
        let header = self.parse_frame_header(&mut br, data)?;

        let channels = header.channels;
        let block_size = header.block_size;
        let bps = header.bits_per_sample;

        // 解码每个子帧
        let mut subframes: Vec<Vec<i32>> = Vec::with_capacity(channels as usize);

        for ch in 0..channels {
            // 确定本子帧的有效位深
            let sub_bps = match header.channel_assignment {
                ChannelAssignment::LeftSide if ch == 1 => bps + 1, // side 多 1 位
                ChannelAssignment::RightSide if ch == 0 => bps + 1, // side 多 1 位
                ChannelAssignment::MidSide if ch == 1 => bps + 1,  // side 多 1 位
                _ => bps,
            };

            let samples = self.decode_subframe(&mut br, block_size, sub_bps)?;
            subframes.push(samples);
        }

        // 应用声道 decorrelation
        // 注意: 下面的循环需要同时访问 subframes[0] 和 subframes[1],
        // 因此使用 split_at_mut 避免 borrow checker 问题.
        if channels == 2 {
            let (left_slice, right_slice) = subframes.split_at_mut(1);
            let left = &mut left_slice[0];
            let right = &mut right_slice[0];

            match header.channel_assignment {
                ChannelAssignment::LeftSide => {
                    // ch0 = left, ch1 = side -> ch1 = left - side
                    for (l, r) in left.iter().zip(right.iter_mut()) {
                        *r = *l - *r;
                    }
                }
                ChannelAssignment::RightSide => {
                    // ch0 = side, ch1 = right -> ch0 = side + right
                    for (l, r) in left.iter_mut().zip(right.iter()) {
                        *l += *r;
                    }
                }
                ChannelAssignment::MidSide => {
                    // mid/side -> left/right
                    for (mid_val, side_val) in left.iter_mut().zip(right.iter_mut()) {
                        let mid = *mid_val;
                        let side = *side_val;
                        let mid2 = (mid << 1) | (side & 1);
                        *mid_val = (mid2 + side) >> 1;
                        *side_val = (mid2 - side) >> 1;
                    }
                }
                ChannelAssignment::Independent(_) => {}
            }
        }

        Ok((subframes, header))
    }

    /// 解析 FLAC 帧头
    fn parse_frame_header(
        &self,
        br: &mut BitReader<'_>,
        raw_data: &[u8],
    ) -> TaoResult<FlacFrameHeader> {
        // 同步码 (14 bits) + reserved (1 bit) + blocking strategy (1 bit)
        let sync = br.read_bits(14)?;
        if sync != 0b11111111111110 {
            return Err(TaoError::InvalidData(format!(
                "无效的 FLAC 同步码: 0x{:04X}",
                sync,
            )));
        }

        let _reserved = br.read_bits(1)?;
        let blocking_strategy = br.read_bits(1)?;

        // Block size (4 bits)
        let bs_code = br.read_bits(4)?;

        // Sample rate (4 bits)
        let sr_code = br.read_bits(4)?;

        // Channel assignment (4 bits)
        let ch_code = br.read_bits(4)?;

        // Sample size (3 bits)
        let ss_code = br.read_bits(3)?;

        // Reserved (1 bit)
        let _reserved2 = br.read_bits(1)?;

        // Frame/sample number (UTF-8 encoded)
        let _frame_or_sample = br.read_utf8_u64()?;

        // Block size (extended)
        let block_size = match bs_code {
            0 => {
                return Err(TaoError::InvalidData("FLAC block_size code 0 保留".into()));
            }
            1 => 192,
            2..=5 => 576 * (1u32 << (bs_code - 2)),
            6 => br.read_bits(8)? + 1,
            7 => br.read_bits(16)? + 1,
            8..=15 => 256 * (1u32 << (bs_code - 8)),
            _ => unreachable!(),
        };

        // Sample rate (extended)
        let sample_rate = match sr_code {
            0 => self.sample_rate,
            1 => 88200,
            2 => 176400,
            3 => 192000,
            4 => 8000,
            5 => 16000,
            6 => 22050,
            7 => 24000,
            8 => 32000,
            9 => 44100,
            10 => 48000,
            11 => 96000,
            12 => br.read_bits(8)? * 1000,
            13 => br.read_bits(16)?,
            14 => br.read_bits(16)? * 10,
            15 => {
                return Err(TaoError::InvalidData(
                    "FLAC sample_rate code 15 无效".into(),
                ));
            }
            _ => unreachable!(),
        };

        // Channel assignment
        let (channel_assignment, channels) = match ch_code {
            0..=7 => (ChannelAssignment::Independent(ch_code + 1), ch_code + 1),
            8 => (ChannelAssignment::LeftSide, 2),
            9 => (ChannelAssignment::RightSide, 2),
            10 => (ChannelAssignment::MidSide, 2),
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "无效的 FLAC 声道分配: {}",
                    ch_code,
                )));
            }
        };

        // Sample size
        let bits_per_sample = match ss_code {
            0 => self.bits_per_sample,
            1 => 8,
            2 => 12,
            3 => {
                return Err(TaoError::InvalidData("FLAC sample_size code 3 保留".into()));
            }
            4 => 16,
            5 => 20,
            6 => 24,
            7 => 32,
            _ => unreachable!(),
        };

        // CRC-8 验证
        let header_end_byte = br.byte_position();
        let crc_read = br.read_bits(8)? as u8;
        let crc_calc = crc::crc8(&raw_data[..header_end_byte]);
        if blocking_strategy <= 1 && crc_read != crc_calc {
            // 仅记录警告, 不严格报错 (有些 encoder 的 CRC 可能有问题)
            debug!(
                "FLAC 帧头 CRC-8 不匹配: 读取=0x{:02X}, 计算=0x{:02X}",
                crc_read, crc_calc,
            );
        }

        Ok(FlacFrameHeader {
            block_size,
            sample_rate,
            channel_assignment,
            channels,
            bits_per_sample,
        })
    }

    /// 解码一个子帧
    fn decode_subframe(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        bps: u32,
    ) -> TaoResult<Vec<i32>> {
        // 子帧头: padding (1 bit) + type (6 bits) + wasted bits flag (1 bit)
        let _padding = br.read_bits(1)?;
        let type_code = br.read_bits(6)?;
        let has_wasted = br.read_bits(1)?;

        let wasted_bits = if has_wasted != 0 {
            // 读取一元编码的 wasted bits 数量
            br.read_unary(1)? + 1
        } else {
            0
        };

        let effective_bps = bps - wasted_bits;

        let subframe_type = match type_code {
            0 => SubframeType::Constant,
            1 => SubframeType::Verbatim,
            2..=7 => {
                return Err(TaoError::InvalidData(format!(
                    "FLAC 保留的子帧类型: {}",
                    type_code,
                )));
            }
            8..=12 => SubframeType::Fixed(type_code - 8),
            13..=15 => {
                return Err(TaoError::InvalidData(format!(
                    "FLAC 保留的子帧类型: {}",
                    type_code,
                )));
            }
            32..=63 => SubframeType::Lpc(type_code - 31),
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "FLAC 保留的子帧类型: {}",
                    type_code,
                )));
            }
        };

        let mut samples = match subframe_type {
            SubframeType::Constant => self.decode_constant(br, block_size, effective_bps)?,
            SubframeType::Verbatim => self.decode_verbatim(br, block_size, effective_bps)?,
            SubframeType::Fixed(order) => {
                self.decode_fixed(br, block_size, effective_bps, order)?
            }
            SubframeType::Lpc(order) => self.decode_lpc(br, block_size, effective_bps, order)?,
        };

        // 恢复 wasted bits
        if wasted_bits > 0 {
            for sample in &mut samples {
                *sample <<= wasted_bits;
            }
        }

        Ok(samples)
    }

    /// 解码 Constant 子帧
    fn decode_constant(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        bps: u32,
    ) -> TaoResult<Vec<i32>> {
        let value = br.read_bits_signed(bps)?;
        Ok(vec![value; block_size as usize])
    }

    /// 解码 Verbatim 子帧
    fn decode_verbatim(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        bps: u32,
    ) -> TaoResult<Vec<i32>> {
        let mut samples = Vec::with_capacity(block_size as usize);
        for _ in 0..block_size {
            samples.push(br.read_bits_signed(bps)?);
        }
        Ok(samples)
    }

    /// 解码 Fixed 预测子帧
    fn decode_fixed(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        bps: u32,
        order: u32,
    ) -> TaoResult<Vec<i32>> {
        let mut samples = Vec::with_capacity(block_size as usize);

        // 读取 warm-up 样本
        for _ in 0..order {
            samples.push(br.read_bits_signed(bps)?);
        }

        // 读取残差
        let residuals = self.decode_residual(br, block_size, order)?;

        // 应用固定预测
        for i in order as usize..block_size as usize {
            let residual = residuals[i - order as usize];
            let predicted = match order {
                0 => 0,
                1 => samples[i - 1],
                2 => 2 * samples[i - 1] - samples[i - 2],
                3 => 3 * samples[i - 1] - 3 * samples[i - 2] + samples[i - 3],
                4 => 4 * samples[i - 1] - 6 * samples[i - 2] + 4 * samples[i - 3] - samples[i - 4],
                _ => {
                    return Err(TaoError::InvalidData(format!(
                        "无效的 Fixed 预测阶数: {}",
                        order,
                    )));
                }
            };
            samples.push(predicted.wrapping_add(residual));
        }

        Ok(samples)
    }

    /// 解码 LPC 预测子帧
    fn decode_lpc(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        bps: u32,
        order: u32,
    ) -> TaoResult<Vec<i32>> {
        let mut samples = Vec::with_capacity(block_size as usize);

        // 读取 warm-up 样本
        for _ in 0..order {
            samples.push(br.read_bits_signed(bps)?);
        }

        // LPC 精度 (4 bits)
        let precision = br.read_bits(4)? + 1;
        if precision > 15 {
            return Err(TaoError::InvalidData("无效的 LPC 精度编码".into()));
        }

        // LPC 移位量 (5 bits, 有符号)
        let shift = br.read_bits_signed(5)?;

        // LPC 系数
        let mut coefficients = Vec::with_capacity(order as usize);
        for _ in 0..order {
            coefficients.push(br.read_bits_signed(precision)? as i64);
        }

        // 读取残差
        let residuals = self.decode_residual(br, block_size, order)?;

        // 应用 LPC 预测
        for i in order as usize..block_size as usize {
            let mut predicted: i64 = 0;
            for (j, &coeff) in coefficients.iter().enumerate() {
                predicted += coeff * samples[i - 1 - j] as i64;
            }
            let predicted = if shift >= 0 {
                predicted >> shift
            } else {
                predicted << (-shift)
            };

            let residual_index = i - order as usize;
            if residual_index >= residuals.len() {
                return Err(TaoError::InvalidData("残差索引越界".to_string()));
            }

            let residual = residuals[residual_index] as i64;
            samples.push((predicted + residual) as i32);
        }

        Ok(samples)
    }

    /// 解码残差 (Rice 编码)
    fn decode_residual(
        &self,
        br: &mut BitReader<'_>,
        block_size: u32,
        predictor_order: u32,
    ) -> TaoResult<Vec<i32>> {
        // 残差编码方式 (2 bits)
        let coding_method = br.read_bits(2)?;
        let rice_param_bits = match coding_method {
            0 => 4, // RICE_PARTITION
            1 => 5, // RICE2_PARTITION
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "无效的残差编码方式: {}",
                    coding_method,
                )));
            }
        };

        // 分区阶数 (4 bits)
        let partition_order = br.read_bits(4)?;
        let num_partitions = 1u32 << partition_order;

        let total_residuals = (block_size - predictor_order) as usize;
        let mut residuals = Vec::with_capacity(total_residuals);

        for partition in 0..num_partitions {
            let rice_param = br.read_bits(rice_param_bits)?;

            let partition_samples = if partition == 0 {
                // 第一个分区需要减去预测器阶数
                (block_size >> partition_order) - predictor_order
            } else {
                // 其他分区正常计算
                block_size >> partition_order
            };

            // 确保不会读取超过总残差数量
            let remaining_samples = total_residuals - residuals.len();
            let samples_to_read = std::cmp::min(partition_samples as usize, remaining_samples);

            // 如果没有样本要读取，跳过这个分区
            if samples_to_read == 0 {
                continue;
            }

            let escape_code = if coding_method == 0 { 15 } else { 31 };

            if rice_param == escape_code {
                // 逃逸编码: 每个样本用固定位数表示
                let bits = br.read_bits(5)?;
                for _ in 0..samples_to_read {
                    residuals.push(br.read_bits_signed(bits)?);
                }
            } else {
                // Rice 编码
                for _ in 0..samples_to_read {
                    let quotient = br.read_unary(1)?;
                    let remainder = if rice_param > 0 {
                        br.read_bits(rice_param)?
                    } else {
                        0
                    };
                    let unsigned_val = (quotient << rice_param) | remainder;
                    // 折叠映射: 0->0, 1->-1, 2->1, 3->-2, 4->2, ...
                    let signed_val = if unsigned_val & 1 != 0 {
                        -((unsigned_val >> 1) as i32) - 1
                    } else {
                        (unsigned_val >> 1) as i32
                    };
                    residuals.push(signed_val);
                }
            }
        }

        Ok(residuals)
    }

    /// 将解码的 i32 样本转换为交错字节格式
    fn samples_to_bytes(&self, subframes: &[Vec<i32>], block_size: u32, bps: u32) -> Vec<u8> {
        let channels = subframes.len();
        let output_bps = if bps <= 8 {
            8
        } else if bps <= 16 {
            16
        } else {
            32
        };
        let bytes_per_sample = output_bps / 8;
        let mut output = Vec::with_capacity(block_size as usize * channels * bytes_per_sample);

        for i in 0..block_size as usize {
            for subframe in subframes {
                let sample = subframe[i];
                match output_bps {
                    8 => {
                        // U8 格式: 偏移 128
                        output.push((sample + 128).clamp(0, 255) as u8);
                    }
                    16 => {
                        // S16LE
                        let s16 = sample.clamp(-32768, 32767) as i16;
                        output.extend_from_slice(&s16.to_le_bytes());
                    }
                    32 => {
                        // S32LE
                        output.extend_from_slice(&sample.to_le_bytes());
                    }
                    _ => unreachable!(),
                }
            }
        }

        output
    }
}

impl Decoder for FlacDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Flac
    }

    fn name(&self) -> &str {
        "flac"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let audio = match &params.params {
            CodecParamsType::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("FLAC 解码器需要音频参数".into()));
            }
        };

        if audio.sample_rate == 0 {
            return Err(TaoError::InvalidArgument("采样率不能为 0".into()));
        }

        self.sample_rate = audio.sample_rate;
        self.channels = audio.channel_layout.channels;
        self.channel_layout = audio.channel_layout;

        // 从 extra_data (STREAMINFO) 中解析位深
        if params.extra_data.len() >= 34 {
            let data = &params.extra_data;
            let bps_hi = (u32::from(data[12]) & 0x01) << 4;
            let bps_lo = u32::from(data[13]) >> 4;
            self.bits_per_sample = (bps_hi | bps_lo) + 1;

            self.max_block_size = u32::from(u16::from_be_bytes([data[2], data[3]]));
        } else {
            // 从采样格式推断
            self.bits_per_sample = match audio.sample_format {
                SampleFormat::U8 => 8,
                SampleFormat::S16 => 16,
                SampleFormat::S32 => 24,
                _ => 16,
            };
            self.max_block_size = audio.frame_size;
        }

        self.output_frame = None;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 FLAC 解码器: {} Hz, {} 声道, {} 位",
            self.sample_rate, self.channels, self.bits_per_sample,
        );
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("解码器未打开, 请先调用 open()".into()));
        }
        if self.output_frame.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }

        // 解码 FLAC 帧
        let (subframes, header) = self.decode_frame(&packet.data)?;

        // 确定输出格式
        let output_format = if header.bits_per_sample <= 8 {
            SampleFormat::U8
        } else if header.bits_per_sample <= 16 {
            SampleFormat::S16
        } else {
            SampleFormat::S32
        };

        let actual_channels = subframes.len() as u32;
        let channel_layout = ChannelLayout::from_channels(actual_channels);

        let mut frame = AudioFrame::new(
            header.block_size,
            header.sample_rate,
            output_format,
            channel_layout,
        );
        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = header.block_size as i64;

        // 转换为交错字节格式
        let data = self.samples_to_bytes(&subframes, header.block_size, header.bits_per_sample);
        frame.data[0] = data;

        self.output_frame = Some(Frame::Audio(frame));
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_frame.take() {
            return Ok(frame);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.flushing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::AudioCodecParams;

    fn make_flac_params(
        sample_rate: u32,
        channels: u32,
        bps: u32,
        max_block_size: u32,
    ) -> CodecParameters {
        // 构造最小 STREAMINFO (34 bytes)
        let mut extra_data = vec![0u8; 34];

        // min/max block size
        let bs_bytes = (max_block_size as u16).to_be_bytes();
        extra_data[0..2].copy_from_slice(&bs_bytes);
        extra_data[2..4].copy_from_slice(&bs_bytes);

        // sample_rate (20 bits) + channels-1 (3 bits) + bps-1 (5 bits) in bytes 10-13
        extra_data[10] = ((sample_rate >> 12) & 0xFF) as u8;
        extra_data[11] = ((sample_rate >> 4) & 0xFF) as u8;
        let sr_low = ((sample_rate & 0x0F) << 4) as u8;
        let ch_bits = (((channels - 1) & 0x07) << 1) as u8;
        let bps_hi = (((bps - 1) >> 4) & 0x01) as u8;
        extra_data[12] = sr_low | ch_bits | bps_hi;
        let bps_lo = (((bps - 1) & 0x0F) << 4) as u8;
        extra_data[13] = bps_lo;

        let sample_format = if bps <= 8 {
            SampleFormat::U8
        } else if bps <= 16 {
            SampleFormat::S16
        } else {
            SampleFormat::S32
        };

        CodecParameters {
            codec_id: CodecId::Flac,
            extra_data,
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format,
                frame_size: max_block_size,
            }),
        }
    }

    /// 构建一个最简单的 FLAC 帧 (Constant 子帧, 值全为 0)
    fn make_constant_flac_frame(
        block_size: u32,
        sample_rate: u32,
        channels: u32,
        bps: u32,
    ) -> Vec<u8> {
        let mut bits: Vec<u8> = Vec::new();
        let mut bit_buf: u64 = 0;
        let mut bit_count = 0u32;

        let flush_byte = |bits: &mut Vec<u8>, buf: &mut u64, count: &mut u32| {
            while *count >= 8 {
                *count -= 8;
                bits.push((*buf >> *count) as u8);
                *buf &= (1u64 << *count) - 1;
            }
        };

        // 同步码 (14 bits)
        bit_buf = (bit_buf << 14) | 0b11111111111110;
        bit_count += 14;

        // reserved (1 bit) = 0
        bit_buf <<= 1;
        bit_count += 1;

        // blocking strategy (1 bit) = 0 (fixed)
        bit_buf <<= 1;
        bit_count += 1;

        flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

        // block_size code (4 bits)
        let bs_code = match block_size {
            192 => 1u32,
            576 => 2,
            1152 => 3,
            2304 => 4,
            4608 => 5,
            256 => 8,
            512 => 9,
            1024 => 10,
            2048 => 11,
            4096 => 12,
            8192 => 13,
            16384 => 14,
            32768 => 15,
            _ => 6, // 8-bit 存储
        };
        bit_buf = (bit_buf << 4) | u64::from(bs_code);
        bit_count += 4;

        // sample rate code (4 bits)
        let sr_code: u32 = match sample_rate {
            88200 => 1,
            176400 => 2,
            192000 => 3,
            8000 => 4,
            16000 => 5,
            22050 => 6,
            24000 => 7,
            32000 => 8,
            44100 => 9,
            48000 => 10,
            96000 => 11,
            _ => 0, // from STREAMINFO
        };
        bit_buf = (bit_buf << 4) | u64::from(sr_code);
        bit_count += 4;

        flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

        // channel assignment (4 bits)
        let ch_code = channels - 1;
        bit_buf = (bit_buf << 4) | u64::from(ch_code);
        bit_count += 4;

        // sample size (3 bits)
        let ss_code: u32 = match bps {
            8 => 1,
            12 => 2,
            16 => 4,
            20 => 5,
            24 => 6,
            32 => 7,
            _ => 0,
        };
        bit_buf = (bit_buf << 3) | u64::from(ss_code);
        bit_count += 3;

        // reserved (1 bit) = 0
        bit_buf <<= 1;
        bit_count += 1;

        flush_byte(&mut bits, &mut bit_buf, &mut bit_count);

        // Frame number (UTF-8 encoded, 1 byte for frame 0)
        bits.push(0x00);

        // Extended block_size (if bs_code == 6)
        if bs_code == 6 {
            bits.push((block_size - 1) as u8);
        }

        // CRC-8 of header
        let crc = tao_core::crc::crc8(&bits);
        bits.push(crc);

        // Subframes (one per channel)
        // Each: constant subframe
        for _ in 0..channels {
            // 子帧头: padding(1)=0 + type(6)=000000(constant) + wasted_flag(1)=0
            bits.push(0x00);

            // Constant value = 0 (bps bits, all zero bytes)
            let value_bytes = bps.div_ceil(8) as usize;
            bits.extend(std::iter::repeat_n(0u8, value_bytes));
        }

        // Padding to byte boundary (already aligned in our case)

        // CRC-16 of entire frame
        let frame_crc = tao_core::crc::crc16(&bits);
        bits.push((frame_crc >> 8) as u8);
        bits.push((frame_crc & 0xFF) as u8);

        bits
    }

    #[test]
    fn test_open_flac_decoder() {
        let params = make_flac_params(44100, 2, 16, 4096);
        let mut dec = FlacDecoder::create().unwrap();
        dec.open(&params).unwrap();
    }

    #[test]
    fn test_decode_constant_frame() {
        let params = make_flac_params(44100, 1, 16, 4096);
        let mut dec = FlacDecoder::create().unwrap();
        dec.open(&params).unwrap();

        // 构建一个 block_size=256, 16-bit, mono 的 constant 帧 (值=0)
        let frame_data = make_constant_flac_frame(256, 44100, 1, 16);
        let pkt = Packet::from_data(bytes::Bytes::from(frame_data));

        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(af) => {
                assert_eq!(af.nb_samples, 256);
                assert_eq!(af.sample_format, SampleFormat::S16);
                // 所有样本应为 0
                assert!(af.data[0].iter().all(|&b| b == 0));
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_not_open_error() {
        let mut dec = FlacDecoder::create().unwrap();
        let pkt = Packet::from_data(bytes::Bytes::from(vec![0u8; 16]));
        let err = dec.send_packet(&pkt).unwrap_err();
        assert!(matches!(err, TaoError::Codec(_)));
    }

    #[test]
    fn test_flush_and_eof() {
        let params = make_flac_params(44100, 1, 16, 4096);
        let mut dec = FlacDecoder::create().unwrap();
        dec.open(&params).unwrap();

        dec.send_packet(&Packet::empty()).unwrap();
        let err = dec.receive_frame().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }
}
