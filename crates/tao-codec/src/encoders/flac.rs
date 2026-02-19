//! FLAC 无损音频编码器.
//!
//! 将 PCM 音频帧编码为 FLAC 帧. 支持:
//! - Constant 子帧 (所有采样相同)
//! - Verbatim 子帧 (未压缩)
//! - Fixed 预测子帧 (0-4 阶)
//! - Rice 熵编码
//! - 自动选择最优子帧类型 (最小编码)
//! - CRC-8 (帧头) 和 CRC-16 (帧尾)

use bytes::Bytes;
use log::debug;
use tao_core::bitwriter::BitWriter;
use tao_core::crc;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::encoder::Encoder;
use crate::frame::Frame;
use crate::packet::Packet;

/// 最大 Rice 参数搜索范围
const MAX_RICE_PARAM: u32 = 14;
/// 最大固定预测阶数
const MAX_FIXED_ORDER: u32 = 4;

/// FLAC 编码器
pub struct FlacEncoder {
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 位深
    bits_per_sample: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 块大小 (每帧每声道采样数)
    block_size: u32,
    /// 输出数据包缓冲
    output_packet: Option<Packet>,
    /// 帧序号
    frame_number: u64,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
    /// 最小帧大小 (统计用)
    min_frame_size: u32,
    /// 最大帧大小 (统计用)
    max_frame_size: u32,
    /// 已编码的总采样数
    total_samples: u64,
}

impl FlacEncoder {
    /// 创建 FLAC 编码器实例
    pub fn create() -> TaoResult<Box<dyn Encoder>> {
        Ok(Box::new(Self {
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            channel_layout: ChannelLayout::MONO,
            block_size: 4096,
            output_packet: None,
            frame_number: 0,
            opened: false,
            flushing: false,
            min_frame_size: u32::MAX,
            max_frame_size: 0,
            total_samples: 0,
        }))
    }

    /// 获取 STREAMINFO 元数据 (34 字节)
    pub fn stream_info(&self) -> Vec<u8> {
        let mut si = vec![0u8; 34];

        // min/max block size
        let bs = self.block_size as u16;
        si[0..2].copy_from_slice(&bs.to_be_bytes());
        si[2..4].copy_from_slice(&bs.to_be_bytes());

        // min/max frame size (24-bit each)
        let min_fs = if self.min_frame_size == u32::MAX {
            0
        } else {
            self.min_frame_size
        };
        si[4] = ((min_fs >> 16) & 0xFF) as u8;
        si[5] = ((min_fs >> 8) & 0xFF) as u8;
        si[6] = (min_fs & 0xFF) as u8;
        si[7] = ((self.max_frame_size >> 16) & 0xFF) as u8;
        si[8] = ((self.max_frame_size >> 8) & 0xFF) as u8;
        si[9] = (self.max_frame_size & 0xFF) as u8;

        // sample_rate (20 bits) + channels-1 (3 bits) + bps-1 (5 bits) + total_samples (36 bits)
        si[10] = ((self.sample_rate >> 12) & 0xFF) as u8;
        si[11] = ((self.sample_rate >> 4) & 0xFF) as u8;
        let sr_low = ((self.sample_rate & 0x0F) << 4) as u8;
        let ch_bits = (((self.channels - 1) & 0x07) << 1) as u8;
        let bps_hi = (((self.bits_per_sample - 1) >> 4) & 0x01) as u8;
        si[12] = sr_low | ch_bits | bps_hi;
        let bps_lo = (((self.bits_per_sample - 1) & 0x0F) << 4) as u8;
        let total_hi = ((self.total_samples >> 32) & 0x0F) as u8;
        si[13] = bps_lo | total_hi;
        let total_lo = (self.total_samples & 0xFFFFFFFF) as u32;
        si[14..18].copy_from_slice(&total_lo.to_be_bytes());

        // MD5 (16 bytes) - 暂不计算
        si
    }

    /// 编码一个 FLAC 帧
    fn encode_frame(&mut self, samples: &[Vec<i32>], nb_samples: u32) -> TaoResult<Vec<u8>> {
        let mut bw =
            BitWriter::with_capacity(nb_samples as usize * self.channels as usize * 4 + 64);

        // 写入帧头 (不含 CRC-8, 后面回填)
        self.write_frame_header(&mut bw, nb_samples)?;

        // 获取帧头数据, 计算 CRC-8
        let header_data = bw.to_bytes();
        let header_crc = crc::crc8(&header_data);
        bw.write_bits(u32::from(header_crc), 8);

        // 编码每个声道的子帧
        for ch_samples in samples.iter().take(self.channels as usize) {
            self.encode_subframe(&mut bw, ch_samples, self.bits_per_sample)?;
        }

        // 对齐到字节边界
        bw.align_to_byte();

        // 计算 CRC-16
        let frame_data = bw.to_bytes();
        let frame_crc = crc::crc16(&frame_data);
        bw.write_bits(u32::from(frame_crc >> 8), 8);
        bw.write_bits(u32::from(frame_crc & 0xFF), 8);

        Ok(bw.finish())
    }

    /// 写入帧头 (不含 CRC-8)
    fn write_frame_header(&self, bw: &mut BitWriter, nb_samples: u32) -> TaoResult<()> {
        // 同步码 (14 bits)
        bw.write_bits(0b11111111111110, 14);
        // reserved (1 bit)
        bw.write_bit(0);
        // blocking strategy (1 bit): 0 = fixed block size
        bw.write_bit(0);

        // block_size (4 bits)
        let bs_code = encode_block_size_code(nb_samples);
        bw.write_bits(bs_code, 4);

        // sample_rate (4 bits)
        let sr_code = encode_sample_rate_code(self.sample_rate);
        bw.write_bits(sr_code, 4);

        // channel assignment (4 bits): 独立声道
        bw.write_bits(self.channels - 1, 4);

        // sample size (3 bits)
        let ss_code = encode_sample_size_code(self.bits_per_sample);
        bw.write_bits(ss_code, 3);

        // reserved (1 bit)
        bw.write_bit(0);

        // frame number (UTF-8 encoded)
        bw.write_utf8_u64(self.frame_number);

        // 扩展 block_size (if needed)
        match bs_code {
            6 => bw.write_bits(nb_samples - 1, 8),
            7 => bw.write_bits(nb_samples - 1, 16),
            _ => {}
        }

        // 扩展 sample_rate (if needed)
        match sr_code {
            12 => bw.write_bits(self.sample_rate / 1000, 8),
            13 => bw.write_bits(self.sample_rate, 16),
            14 => bw.write_bits(self.sample_rate / 10, 16),
            _ => {}
        }

        Ok(())
    }

    /// 编码一个子帧 (自动选择最优类型)
    fn encode_subframe(&self, bw: &mut BitWriter, samples: &[i32], bps: u32) -> TaoResult<()> {
        let n = samples.len();
        if n == 0 {
            return Ok(());
        }

        // 检查是否全部相同 (Constant 子帧)
        if samples.iter().all(|&s| s == samples[0]) {
            return self.encode_constant_subframe(bw, samples[0], bps);
        }

        // 尝试所有 Fixed 预测阶数, 选择最小编码
        let mut best_order: Option<u32> = None;
        let mut best_bits = u64::MAX;

        for order in 0..=MAX_FIXED_ORDER.min(n as u32 - 1) {
            let residuals = compute_fixed_residuals(samples, order);
            let bits = estimate_rice_bits(&residuals);
            if bits < best_bits {
                best_bits = bits;
                best_order = Some(order);
            }
        }

        // 与 Verbatim 比较
        let verbatim_bits = (n as u64) * u64::from(bps);
        if best_bits + 100 >= verbatim_bits {
            // Verbatim 更好 (加 100 是子帧头开销)
            return self.encode_verbatim_subframe(bw, samples, bps);
        }

        let order = best_order.unwrap_or(0);
        self.encode_fixed_subframe(bw, samples, bps, order)
    }

    /// 编码 Constant 子帧
    fn encode_constant_subframe(&self, bw: &mut BitWriter, value: i32, bps: u32) -> TaoResult<()> {
        // 子帧头: padding(1)=0 + type(6)=000000 + wasted(1)=0
        bw.write_bits(0, 1);
        bw.write_bits(0b000000, 6); // Constant
        bw.write_bit(0);

        // 常量值
        bw.write_bits_signed(value, bps);
        Ok(())
    }

    /// 编码 Verbatim 子帧
    fn encode_verbatim_subframe(
        &self,
        bw: &mut BitWriter,
        samples: &[i32],
        bps: u32,
    ) -> TaoResult<()> {
        // 子帧头: padding(1)=0 + type(6)=000001 + wasted(1)=0
        bw.write_bits(0, 1);
        bw.write_bits(0b000001, 6); // Verbatim
        bw.write_bit(0);

        for &sample in samples {
            bw.write_bits_signed(sample, bps);
        }
        Ok(())
    }

    /// 编码 Fixed 预测子帧
    fn encode_fixed_subframe(
        &self,
        bw: &mut BitWriter,
        samples: &[i32],
        bps: u32,
        order: u32,
    ) -> TaoResult<()> {
        // 子帧头: padding(1)=0 + type(6)=001xxx + wasted(1)=0
        bw.write_bits(0, 1);
        bw.write_bits(0b001000 | order, 6); // Fixed, order
        bw.write_bit(0);

        // Warm-up 样本
        for &sample in &samples[..order as usize] {
            bw.write_bits_signed(sample, bps);
        }

        // 残差
        let residuals = compute_fixed_residuals(samples, order);
        self.encode_residual(bw, &residuals, samples.len() as u32, order)?;

        Ok(())
    }

    /// 编码残差 (Rice 编码)
    fn encode_residual(
        &self,
        bw: &mut BitWriter,
        residuals: &[i32],
        block_size: u32,
        predictor_order: u32,
    ) -> TaoResult<()> {
        // 使用 RICE_PARTITION (coding method = 0)
        bw.write_bits(0, 2); // coding method = 0

        // 选择最优分区阶数
        let partition_order = select_partition_order(block_size, predictor_order);
        bw.write_bits(partition_order, 4);

        let num_partitions = 1u32 << partition_order;

        let mut residual_idx = 0usize;
        for partition in 0..num_partitions {
            let partition_samples = if partition == 0 {
                (block_size >> partition_order) - predictor_order
            } else {
                block_size >> partition_order
            } as usize;

            let partition_data = &residuals[residual_idx..residual_idx + partition_samples];

            // 选择最优 Rice 参数
            let rice_param = select_rice_param(partition_data);
            bw.write_bits(rice_param, 4);

            // 编码每个残差
            for &residual in partition_data {
                encode_rice_sample(bw, residual, rice_param);
            }

            residual_idx += partition_samples;
        }

        Ok(())
    }

    /// 从 AudioFrame 中提取 i32 样本
    fn extract_samples(&self, frame: &crate::frame::AudioFrame) -> TaoResult<Vec<Vec<i32>>> {
        let channels = self.channels as usize;
        let nb_samples = frame.nb_samples as usize;
        let bps = self.bits_per_sample;
        let data = &frame.data[0]; // 交错格式

        let mut result = vec![Vec::with_capacity(nb_samples); channels];

        match frame.sample_format {
            SampleFormat::U8 => {
                for i in 0..nb_samples {
                    for (ch, ch_vec) in result.iter_mut().enumerate() {
                        let idx = i * channels + ch;
                        if idx < data.len() {
                            ch_vec.push(data[idx] as i32 - 128);
                        }
                    }
                }
            }
            SampleFormat::S16 => {
                for i in 0..nb_samples {
                    for (ch, ch_vec) in result.iter_mut().enumerate() {
                        let idx = (i * channels + ch) * 2;
                        if idx + 1 < data.len() {
                            let s = i16::from_le_bytes([data[idx], data[idx + 1]]);
                            ch_vec.push(i32::from(s));
                        }
                    }
                }
            }
            SampleFormat::S32 => {
                let shift = if bps <= 24 { 32 - bps } else { 0 };
                for i in 0..nb_samples {
                    for (ch, ch_vec) in result.iter_mut().enumerate() {
                        let idx = (i * channels + ch) * 4;
                        if idx + 3 < data.len() {
                            let s = i32::from_le_bytes([
                                data[idx],
                                data[idx + 1],
                                data[idx + 2],
                                data[idx + 3],
                            ]);
                            if shift > 0 {
                                ch_vec.push(s >> shift << shift >> shift);
                            } else {
                                ch_vec.push(s);
                            }
                        }
                    }
                }
            }
            _ => {
                return Err(TaoError::Unsupported(format!(
                    "FLAC 不支持采样格式: {}",
                    frame.sample_format,
                )));
            }
        }

        Ok(result)
    }
}

impl Encoder for FlacEncoder {
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
                return Err(TaoError::InvalidArgument("FLAC 编码器需要音频参数".into()));
            }
        };

        if audio.sample_rate == 0 {
            return Err(TaoError::InvalidArgument("采样率不能为 0".into()));
        }
        if audio.channel_layout.channels == 0 || audio.channel_layout.channels > 8 {
            return Err(TaoError::InvalidArgument(format!(
                "FLAC 不支持的声道数: {}",
                audio.channel_layout.channels,
            )));
        }

        self.sample_rate = audio.sample_rate;
        self.channels = audio.channel_layout.channels;
        self.channel_layout = audio.channel_layout;

        self.bits_per_sample = match audio.sample_format {
            SampleFormat::U8 => 8,
            SampleFormat::S16 => 16,
            SampleFormat::S32 => 24, // 默认 24 位
            _ => {
                return Err(TaoError::Unsupported(format!(
                    "FLAC 不支持采样格式: {}",
                    audio.sample_format,
                )));
            }
        };

        if audio.frame_size > 0 {
            self.block_size = audio.frame_size;
        }

        self.output_packet = None;
        self.frame_number = 0;
        self.opened = true;
        self.flushing = false;
        self.min_frame_size = u32::MAX;
        self.max_frame_size = 0;
        self.total_samples = 0;

        debug!(
            "打开 FLAC 编码器: {} Hz, {} 声道, {} 位, 块大小={}",
            self.sample_rate, self.channels, self.bits_per_sample, self.block_size,
        );
        Ok(())
    }

    fn send_frame(&mut self, frame: Option<&Frame>) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("编码器未打开, 请先调用 open()".into()));
        }
        if self.output_packet.is_some() {
            return Err(TaoError::NeedMoreData);
        }

        let frame = match frame {
            Some(f) => f,
            None => {
                self.flushing = true;
                return Ok(());
            }
        };

        let audio = match frame {
            Frame::Audio(a) => a,
            Frame::Video(_) => {
                return Err(TaoError::InvalidArgument("FLAC 编码器不接受视频帧".into()));
            }
        };

        // 提取样本
        let samples = self.extract_samples(audio)?;
        let nb_samples = audio.nb_samples;

        // 编码帧
        let frame_data = self.encode_frame(&samples, nb_samples)?;
        let frame_size = frame_data.len() as u32;

        // 更新统计
        self.min_frame_size = self.min_frame_size.min(frame_size);
        self.max_frame_size = self.max_frame_size.max(frame_size);
        self.total_samples += u64::from(nb_samples);

        let mut pkt = Packet::from_data(Bytes::from(frame_data));
        pkt.pts = audio.pts;
        pkt.dts = audio.pts;
        pkt.duration = i64::from(nb_samples);
        pkt.time_base = audio.time_base;
        pkt.is_keyframe = true;

        self.frame_number += 1;
        self.output_packet = Some(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> TaoResult<Packet> {
        if let Some(pkt) = self.output_packet.take() {
            return Ok(pkt);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.output_packet = None;
        self.flushing = false;
    }
}

// ============================================================
// 辅助函数
// ============================================================

/// 计算 Fixed 预测残差
fn compute_fixed_residuals(samples: &[i32], order: u32) -> Vec<i32> {
    let n = samples.len();
    let order = order as usize;
    let mut residuals = Vec::with_capacity(n - order);

    for i in order..n {
        let predicted = match order {
            0 => 0,
            1 => samples[i - 1],
            2 => 2 * samples[i - 1] - samples[i - 2],
            3 => 3 * samples[i - 1] - 3 * samples[i - 2] + samples[i - 3],
            4 => 4 * samples[i - 1] - 6 * samples[i - 2] + 4 * samples[i - 3] - samples[i - 4],
            _ => 0,
        };
        residuals.push(samples[i].wrapping_sub(predicted));
    }

    residuals
}

/// 估算 Rice 编码所需的位数
fn estimate_rice_bits(residuals: &[i32]) -> u64 {
    if residuals.is_empty() {
        return 0;
    }

    // 估算最优 Rice 参数
    let param = estimate_optimal_rice_param(residuals);
    let mut total_bits: u64 = 0;

    for &residual in residuals {
        let unsigned_val = fold_signed(residual);
        let quotient = unsigned_val >> param;
        total_bits += u64::from(quotient) + 1 + u64::from(param);
    }

    total_bits
}

/// 选择最优 Rice 参数
fn select_rice_param(residuals: &[i32]) -> u32 {
    estimate_optimal_rice_param(residuals).min(MAX_RICE_PARAM)
}

/// 估算最优 Rice 参数
fn estimate_optimal_rice_param(residuals: &[i32]) -> u32 {
    if residuals.is_empty() {
        return 0;
    }

    // 计算残差绝对值的平均
    let sum: u64 = residuals.iter().map(|&r| u64::from(fold_signed(r))).sum();
    let avg = sum / residuals.len() as u64;

    // Rice 参数约为 log2(avg)
    if avg == 0 {
        0
    } else {
        (64 - avg.leading_zeros() - 1).min(MAX_RICE_PARAM)
    }
}

/// 将有符号值映射为无符号 (折叠映射)
/// 0->0, -1->1, 1->2, -2->3, 2->4, ...
fn fold_signed(value: i32) -> u32 {
    if value >= 0 {
        (value as u32) << 1
    } else {
        ((-value as u32) << 1) - 1
    }
}

/// 编码单个 Rice 样本
fn encode_rice_sample(bw: &mut BitWriter, value: i32, rice_param: u32) {
    let unsigned_val = fold_signed(value);
    let quotient = unsigned_val >> rice_param;
    let remainder = unsigned_val & ((1u32 << rice_param) - 1);

    // 一元编码: quotient 个 0, 然后 1 个 1
    bw.write_unary(quotient, 1);
    // 余数
    if rice_param > 0 {
        bw.write_bits(remainder, rice_param);
    }
}

/// 选择分区阶数
fn select_partition_order(block_size: u32, predictor_order: u32) -> u32 {
    // 简单策略: 选择使分区大小合理的阶数
    let mut order = 0u32;
    while order < 8 {
        let partition_size = block_size >> (order + 1);
        if partition_size < predictor_order + 4 {
            break;
        }
        order += 1;
    }
    order
}

/// 编码 block_size 代码
fn encode_block_size_code(block_size: u32) -> u32 {
    match block_size {
        192 => 1,
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
        bs if bs <= 256 => 6, // 8-bit
        _ => 7,               // 16-bit
    }
}

/// 编码采样率代码
fn encode_sample_rate_code(sample_rate: u32) -> u32 {
    match sample_rate {
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
        sr if sr % 1000 == 0 && sr / 1000 <= 255 => 12,
        sr if sr <= 65535 => 13,
        sr if sr % 10 == 0 && sr / 10 <= 65535 => 14,
        _ => 0, // 从 STREAMINFO
    }
}

/// 编码采样精度代码
fn encode_sample_size_code(bps: u32) -> u32 {
    match bps {
        8 => 1,
        12 => 2,
        16 => 4,
        20 => 5,
        24 => 6,
        32 => 7,
        _ => 0, // 从 STREAMINFO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::AudioCodecParams;
    use crate::decoders::flac::FlacDecoder;
    use crate::frame::AudioFrame;

    fn make_flac_params(sample_rate: u32, channels: u32, bps: u32) -> CodecParameters {
        let sample_format = match bps {
            8 => SampleFormat::U8,
            16 => SampleFormat::S16,
            24 | 32 => SampleFormat::S32,
            _ => SampleFormat::S16,
        };
        CodecParameters {
            codec_id: CodecId::Flac,
            extra_data: Vec::new(),
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format,
                frame_size: 256,
            }),
        }
    }

    /// 手动构造 STREAMINFO 字节 (34 字节)
    fn build_stream_info(
        sample_rate: u32,
        channels: u32,
        bps: u32,
        block_size: u16,
        total_samples: u64,
    ) -> Vec<u8> {
        let mut si = vec![0u8; 34];

        si[0..2].copy_from_slice(&block_size.to_be_bytes());
        si[2..4].copy_from_slice(&block_size.to_be_bytes());

        // sample_rate(20) + channels-1(3) + bps-1(5) + total_samples(36)
        si[10] = ((sample_rate >> 12) & 0xFF) as u8;
        si[11] = ((sample_rate >> 4) & 0xFF) as u8;
        let sr_low = ((sample_rate & 0x0F) << 4) as u8;
        let ch_bits = (((channels - 1) & 0x07) << 1) as u8;
        let bps_hi = (((bps - 1) >> 4) & 0x01) as u8;
        si[12] = sr_low | ch_bits | bps_hi;
        let bps_lo = (((bps - 1) & 0x0F) << 4) as u8;
        let total_hi = ((total_samples >> 32) & 0x0F) as u8;
        si[13] = bps_lo | total_hi;
        let total_lo = (total_samples & 0xFFFFFFFF) as u32;
        si[14..18].copy_from_slice(&total_lo.to_be_bytes());

        si
    }

    fn make_decoder_params(
        sample_rate: u32,
        channels: u32,
        bps: u32,
        block_size: u16,
    ) -> CodecParameters {
        let sample_format = match bps {
            8 => SampleFormat::U8,
            16 => SampleFormat::S16,
            24 | 32 => SampleFormat::S32,
            _ => SampleFormat::S16,
        };
        let si = build_stream_info(sample_rate, channels, bps, block_size, 0);
        CodecParameters {
            codec_id: CodecId::Flac,
            extra_data: si,
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format,
                frame_size: u32::from(block_size),
            }),
        }
    }

    #[test]
    fn test_open_encoder() {
        let params = make_flac_params(44100, 2, 16);
        let mut enc = FlacEncoder::create().unwrap();
        enc.open(&params).unwrap();
    }

    #[test]
    fn test_encode_all_zero_frame() {
        let params = make_flac_params(44100, 1, 16);
        let mut enc = FlacEncoder::create().unwrap();
        enc.open(&params).unwrap();

        // 全零 S16 单声道, 256 样本
        let data = vec![0u8; 256 * 2];
        let mut af = AudioFrame::new(256, 44100, SampleFormat::S16, ChannelLayout::MONO);
        af.data[0] = data;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert!(!pkt.data.is_empty());
    }

    #[test]
    fn test_codec_roundtrip_all_zero() {
        let params = make_flac_params(44100, 1, 16);
        let mut enc = FlacEncoder::create().unwrap();
        enc.open(&params).unwrap();

        // 编码全零帧
        let original = vec![0u8; 256 * 2];
        let mut af = AudioFrame::new(256, 44100, SampleFormat::S16, ChannelLayout::MONO);
        af.data[0] = original.clone();
        af.pts = 0;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        // 解码
        let dec_params = make_decoder_params(44100, 1, 16, 256);
        let mut dec = FlacDecoder::create().unwrap();
        dec.open(&dec_params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(decoded) => {
                assert_eq!(decoded.nb_samples, 256);
                assert_eq!(decoded.data[0], original);
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_codec_roundtrip_sine_wave() {
        let params = make_flac_params(44100, 1, 16);
        let mut enc = FlacEncoder::create().unwrap();
        enc.open(&params).unwrap();

        // 生成正弦波
        let nb_samples = 256u32;
        let mut pcm = Vec::with_capacity(nb_samples as usize * 2);
        for i in 0..nb_samples {
            let t = i as f64 / 44100.0;
            let val = (t * 440.0 * 2.0 * std::f64::consts::PI).sin();
            let sample = (val * 16000.0) as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
        }

        let mut af = AudioFrame::new(nb_samples, 44100, SampleFormat::S16, ChannelLayout::MONO);
        af.data[0] = pcm.clone();
        af.pts = 0;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        // 解码
        let dec_params = make_decoder_params(44100, 1, 16, nb_samples as u16);
        let mut dec = FlacDecoder::create().unwrap();
        dec.open(&dec_params).unwrap();
        dec.send_packet(&pkt).unwrap();
        let frame = dec.receive_frame().unwrap();

        match frame {
            Frame::Audio(decoded) => {
                assert_eq!(decoded.nb_samples, nb_samples);
                assert_eq!(decoded.data[0], pcm, "FLAC 无损往返: 解码数据应与原始相同");
            }
            _ => panic!("期望音频帧"),
        }
    }

    #[test]
    fn test_flush_and_eof() {
        let params = make_flac_params(44100, 1, 16);
        let mut enc = FlacEncoder::create().unwrap();
        enc.open(&params).unwrap();
        enc.send_frame(None).unwrap();
        let err = enc.receive_packet().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_fold_signed() {
        assert_eq!(fold_signed(0), 0);
        assert_eq!(fold_signed(-1), 1);
        assert_eq!(fold_signed(1), 2);
        assert_eq!(fold_signed(-2), 3);
        assert_eq!(fold_signed(2), 4);
    }
}
