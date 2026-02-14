//! AAC-LC 音频编码器.
//!
//! 将 PCM 音频帧编码为 AAC-LC ADTS 格式. 使用简化的 MDCT、量化和熵编码实现.
//!
//! 实现要点:
//! - MDCT 变换 (2048 输入样本 -> 1024 频谱系数)
//! - 正弦窗 (sine window)
//! - 均匀标量量化
//! - 简化的 Huffman/位打包编码
//! - ADTS 帧头 (7 字节, protection_absent=1)

use bytes::Bytes;
use log::debug;
use std::f64::consts::PI;
use tao_core::bitwriter::BitWriter;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::encoder::Encoder;
use crate::frame::Frame;
use crate::packet::Packet;

/// AAC 帧大小 (每声道采样数)
const AAC_FRAME_SIZE: usize = 1024;
/// MDCT 输入长度 (2 * AAC_FRAME_SIZE)
const MDCT_INPUT_SIZE: usize = 2048;

/// ADTS 采样率索引对应的采样率 (Hz)
const SAMPLE_RATE_TABLE: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// AAC-LC 编码器
pub struct AacEncoder {
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 输出数据包缓冲
    output_packet: Option<Packet>,
    /// 帧序号
    frame_number: u64,
    /// 是否已打开
    opened: bool,
    /// 是否已收到刷新信号
    flushing: bool,
    /// 重叠缓冲 (用于 MDCT 窗, 每声道 1024 样本)
    overlap_buffer: Vec<Vec<f32>>,
    /// 输入缓冲 (收集不足 1024 的样本)
    input_buffer: Vec<Vec<f32>>,
    /// 输入缓冲中已收集的采样数
    input_samples: usize,
}

impl AacEncoder {
    /// 创建 AAC-LC 编码器实例
    pub fn create() -> TaoResult<Box<dyn Encoder>> {
        Ok(Box::new(Self {
            sample_rate: 0,
            channels: 0,
            channel_layout: ChannelLayout::MONO,
            output_packet: None,
            frame_number: 0,
            opened: false,
            flushing: false,
            overlap_buffer: Vec::new(),
            input_buffer: Vec::new(),
            input_samples: 0,
        }))
    }

    /// 获取采样率对应的 ADTS 索引
    fn sample_rate_index(&self, sample_rate: u32) -> Option<u8> {
        SAMPLE_RATE_TABLE
            .iter()
            .position(|&sr| sr == sample_rate)
            .map(|i| i as u8)
    }

    /// 从 AudioFrame 提取 F32 交错样本 (每声道一个 Vec)
    fn extract_f32_samples(&self, frame: &crate::frame::AudioFrame) -> TaoResult<Vec<Vec<f32>>> {
        if frame.sample_format != SampleFormat::F32 && frame.sample_format != SampleFormat::F32p {
            return Err(TaoError::Unsupported(format!(
                "AAC 编码器仅支持 F32 格式, 当前为 {}",
                frame.sample_format,
            )));
        }

        let ch = self.channels as usize;
        let nb = frame.nb_samples as usize;

        let mut result = vec![Vec::with_capacity(nb); ch];

        if frame.sample_format.is_planar() {
            // 平面格式: data[i] 为第 i 声道
            for (i, ch_data) in frame.data.iter().enumerate().take(ch) {
                if ch_data.len() < nb * 4 {
                    return Err(TaoError::InvalidData("音频数据长度不足".into()));
                }
                for j in 0..nb {
                    let idx = j * 4;
                    let bytes = [
                        ch_data[idx],
                        ch_data[idx + 1],
                        ch_data[idx + 2],
                        ch_data[idx + 3],
                    ];
                    result[i].push(f32::from_le_bytes(bytes));
                }
            }
        } else {
            // 交错格式: data[0] 包含所有声道交替
            let data = &frame.data[0];
            let expected_len = nb * ch * 4;
            if data.len() < expected_len {
                return Err(TaoError::InvalidData("音频数据长度不足".into()));
            }
            for j in 0..nb {
                for (i, ch_vec) in result.iter_mut().enumerate().take(ch) {
                    let idx = (j * ch + i) * 4;
                    let bytes = [data[idx], data[idx + 1], data[idx + 2], data[idx + 3]];
                    ch_vec.push(f32::from_le_bytes(bytes));
                }
            }
        }

        Ok(result)
    }

    /// 应用正弦窗: w[n] = sin(pi/4096 * (n + 0.5))
    fn apply_sine_window(input: &mut [f64]) {
        for (n, x) in input.iter_mut().enumerate() {
            let w = (PI / 4096.0 * (n as f64 + 0.5)).sin();
            *x *= w;
        }
    }

    /// MDCT 变换: 2048 输入 -> 1024 输出
    /// X[k] = sum_{n=0}^{2047} x[n] * cos(pi/2048 * (2n+1025) * (2k+1))
    fn mdct(input: &[f64]) -> Vec<f64> {
        let mut output = vec![0.0; AAC_FRAME_SIZE];
        let n_inv = 1.0 / 2048.0 * PI;

        for (k, out) in output.iter_mut().enumerate() {
            let mut sum = 0.0;
            let coeff_k = (2 * k + 1) as f64;
            for (n, &x) in input.iter().enumerate() {
                let angle = n_inv * (2.0 * n as f64 + 1025.0) * coeff_k;
                sum += x * angle.cos();
            }
            *out = sum;
        }
        output
    }

    /// 均匀标量量化: 将频谱系数量化为有符号 8 位整数
    fn quantize(spectral: &[f64]) -> Vec<i16> {
        let max_val = spectral.iter().map(|x| x.abs()).fold(0.0_f64, f64::max);
        let scale = if max_val > 1e-10 {
            127.0 / max_val
        } else {
            1.0
        };

        spectral
            .iter()
            .map(|&x| {
                let q = (x * scale).round() as i32;
                q.clamp(-128, 127) as i16
            })
            .collect()
    }

    /// 简化的 Huffman/位打包: 使用简单变长编码
    /// 0 -> 1 bit, ±1 -> 3 bits, ±2..±3 -> 5 bits, ±4..±7 -> 7 bits, 其他 -> 9 bits
    fn encode_spectral(bw: &mut BitWriter, quantized: &[i16]) {
        for &q in quantized {
            let abs_q = q.unsigned_abs() as u32;
            let sign = if q < 0 { 1u32 } else { 0 };
            match abs_q {
                0 => {
                    bw.write_bit(0);
                }
                1 => {
                    bw.write_bits(0b100 | sign, 3);
                }
                2..=3 => {
                    bw.write_bits(0b11000 | (abs_q - 2) << 1 | sign, 5);
                }
                4..=7 => {
                    bw.write_bits(0b1110000 | (abs_q - 4) << 1 | sign, 7);
                }
                _ => {
                    let v = (abs_q - 8).min(255);
                    bw.write_bits(0b111111110 | (v << 1) | sign, 9);
                }
            }
        }
    }

    /// 生成 ADTS 帧头 (7 字节, protection_absent=1)
    fn write_adts_header(
        &self,
        frame_length: usize,
        sample_rate: u32,
        channels: u32,
    ) -> TaoResult<Vec<u8>> {
        let sr_index = self
            .sample_rate_index(sample_rate)
            .ok_or_else(|| TaoError::Unsupported(format!("不支持的采样率: {} Hz", sample_rate)))?;

        let channel_config = channels.min(7) as u8;
        let frame_length_u16 = (frame_length + 7) as u16;

        let mut header = vec![0u8; 7];
        header[0] = 0xFF;
        header[1] = 0xF1;
        header[2] = (1 << 6) | (sr_index << 2) | (channel_config >> 2);
        header[3] = ((channel_config & 0x03) << 6) | ((frame_length_u16 >> 11) & 0x03) as u8;
        header[4] = ((frame_length_u16 >> 3) & 0xFF) as u8;
        header[5] = (((frame_length_u16 & 0x07) << 5) | 0x1F) as u8;
        header[6] = 0xFC;

        Ok(header)
    }

    /// 编码单帧 (每声道 1024 样本)
    fn encode_frame(
        &mut self,
        samples_per_ch: &[Vec<f32>],
        pts: i64,
        time_base: tao_core::Rational,
        duration: i64,
    ) -> TaoResult<Packet> {
        let mut payload = Vec::new();

        for (ch_idx, current) in samples_per_ch
            .iter()
            .enumerate()
            .take(self.channels as usize)
        {
            let overlap = &self.overlap_buffer[ch_idx];

            let mut mdct_input = vec![0.0; MDCT_INPUT_SIZE];
            for (i, &s) in overlap.iter().enumerate() {
                mdct_input[i] = s as f64;
            }
            for (i, &s) in current.iter().enumerate() {
                mdct_input[AAC_FRAME_SIZE + i] = s as f64;
            }

            Self::apply_sine_window(&mut mdct_input);
            let spectral = Self::mdct(&mdct_input);
            let quantized = Self::quantize(&spectral);

            let mut bw = BitWriter::with_capacity(1024);
            Self::encode_spectral(&mut bw, &quantized);
            let bytes = bw.finish();
            payload.extend_from_slice(&bytes);

            self.overlap_buffer[ch_idx] = current.to_vec();
        }

        let header = self.write_adts_header(payload.len(), self.sample_rate, self.channels)?;

        let mut frame_data = header;
        frame_data.extend_from_slice(&payload);

        let mut pkt = Packet::from_data(Bytes::from(frame_data));
        pkt.pts = pts;
        pkt.dts = pts;
        pkt.duration = duration;
        pkt.time_base = time_base;
        pkt.stream_index = 0;
        pkt.is_keyframe = true;

        Ok(pkt)
    }

    /// 处理输入样本, 当积累满 1024 样本时编码 (每次最多输出一包)
    fn process_samples(
        &mut self,
        samples_per_ch: Vec<Vec<f32>>,
        pts: i64,
        time_base: tao_core::Rational,
        duration: i64,
    ) -> TaoResult<()> {
        let nb = samples_per_ch[0].len();
        if nb == 0 {
            return Ok(());
        }

        let frames_in_input = nb / AAC_FRAME_SIZE;
        let chunk_duration = if frames_in_input > 0 {
            duration / frames_in_input as i64
        } else {
            duration
        };

        let offset = 0;
        if offset + AAC_FRAME_SIZE <= nb {
            let mut chunk = vec![Vec::with_capacity(AAC_FRAME_SIZE); self.channels as usize];
            for (ch, ch_samples) in samples_per_ch
                .iter()
                .enumerate()
                .take(self.channels as usize)
            {
                chunk[ch] = ch_samples[offset..offset + AAC_FRAME_SIZE].to_vec();
            }
            let chunk_pts = if pts != tao_core::timestamp::NOPTS_VALUE {
                pts + (offset as i64 * duration / nb as i64)
            } else {
                pts
            };
            let pkt = self.encode_frame(&chunk, chunk_pts, time_base, chunk_duration)?;
            self.output_packet = Some(pkt);
            self.frame_number += 1;
            let new_offset = offset + AAC_FRAME_SIZE;
            if new_offset < nb {
                for (ch, ch_samples) in samples_per_ch
                    .iter()
                    .enumerate()
                    .take(self.channels as usize)
                {
                    self.input_buffer[ch].extend_from_slice(&ch_samples[new_offset..]);
                }
                self.input_samples = nb - new_offset;
            }
            return Ok(());
        }

        for (ch, ch_samples) in samples_per_ch
            .iter()
            .enumerate()
            .take(self.channels as usize)
        {
            self.input_buffer[ch].extend_from_slice(ch_samples);
        }
        self.input_samples = nb;

        Ok(())
    }
}

impl Encoder for AacEncoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Aac
    }

    fn name(&self) -> &str {
        "aac_lc"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        let audio = match &params.params {
            CodecParamsType::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("AAC 编码器需要音频参数".into()));
            }
        };

        if audio.sample_rate == 0 {
            return Err(TaoError::InvalidArgument("采样率不能为 0".into()));
        }
        if audio.channel_layout.channels == 0 || audio.channel_layout.channels > 8 {
            return Err(TaoError::InvalidArgument(format!(
                "AAC 不支持的声道数: {}",
                audio.channel_layout.channels,
            )));
        }

        if self.sample_rate_index(audio.sample_rate).is_none() {
            return Err(TaoError::Unsupported(format!(
                "AAC 不支持的采样率: {} Hz",
                audio.sample_rate,
            )));
        }

        self.sample_rate = audio.sample_rate;
        self.channels = audio.channel_layout.channels;
        self.channel_layout = audio.channel_layout;
        self.overlap_buffer = vec![vec![0.0; AAC_FRAME_SIZE]; self.channels as usize];
        self.input_buffer = vec![Vec::new(); self.channels as usize];
        self.input_samples = 0;
        self.output_packet = None;
        self.frame_number = 0;
        self.opened = true;
        self.flushing = false;

        debug!(
            "打开 AAC-LC 编码器: {} Hz, {} 声道",
            self.sample_rate, self.channels,
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
                if self.input_samples > 0 {
                    let mut padded = self.input_buffer.clone();
                    for ch_data in padded.iter_mut().take(self.channels as usize) {
                        while ch_data.len() < AAC_FRAME_SIZE {
                            ch_data.push(0.0);
                        }
                        ch_data.truncate(AAC_FRAME_SIZE);
                    }
                    self.process_samples(
                        padded,
                        tao_core::timestamp::NOPTS_VALUE,
                        tao_core::Rational::new(1, self.sample_rate as i32),
                        AAC_FRAME_SIZE as i64,
                    )?;
                    self.input_samples = 0;
                    self.input_buffer.iter_mut().for_each(|v| v.clear());
                }
                return Ok(());
            }
        };

        let audio = match frame {
            Frame::Audio(a) => a,
            Frame::Video(_) => {
                return Err(TaoError::InvalidArgument("AAC 编码器不接受视频帧".into()));
            }
        };

        let mut samples_per_ch = self.extract_f32_samples(audio)?;

        if self.input_samples > 0 {
            for (ch, ch_samples) in samples_per_ch
                .iter_mut()
                .enumerate()
                .take(self.channels as usize)
            {
                let mut combined = Vec::with_capacity(self.input_samples + ch_samples.len());
                combined.append(&mut self.input_buffer[ch]);
                combined.extend_from_slice(ch_samples);
                *ch_samples = combined;
            }
            self.input_samples = 0;
            self.input_buffer.iter_mut().for_each(|v| v.clear());
        }

        let pts = audio.pts;
        let time_base = audio.time_base;
        let duration = audio.duration;

        self.process_samples(samples_per_ch, pts, time_base, duration)?;
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
        self.input_samples = 0;
        for v in &mut self.input_buffer {
            v.clear();
        }
        for v in &mut self.overlap_buffer {
            v.clear();
            v.resize(AAC_FRAME_SIZE, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::AudioCodecParams;
    use crate::frame::AudioFrame;
    use tao_core::Rational;

    fn make_aac_params(sample_rate: u32, channels: u32) -> CodecParameters {
        CodecParameters {
            codec_id: CodecId::Aac,
            extra_data: Vec::new(),
            bit_rate: 128000,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::F32,
                frame_size: AAC_FRAME_SIZE as u32,
            }),
        }
    }

    #[test]
    fn test_创建与打开() {
        let params = make_aac_params(44100, 2);
        let mut enc = AacEncoder::create().unwrap();
        enc.open(&params).unwrap();
        assert_eq!(enc.codec_id(), CodecId::Aac);
        assert_eq!(enc.name(), "aac_lc");
    }

    #[test]
    fn test_编码静音帧() {
        let params = make_aac_params(44100, 1);
        let mut enc = AacEncoder::create().unwrap();
        enc.open(&params).unwrap();

        let nb_samples = 1024u32;
        let data = vec![0.0f32; nb_samples as usize];
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();

        let mut af = AudioFrame::new(nb_samples, 44100, SampleFormat::F32, ChannelLayout::MONO);
        af.data[0] = bytes;
        af.pts = 0;
        af.time_base = Rational::new(1, 44100);
        af.duration = 1024;

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        assert!(!pkt.data.is_empty());
        assert!(pkt.data.len() >= 7);
        assert_eq!(pkt.data[0], 0xFF, "ADTS sync word 高字节");
        assert_eq!(
            pkt.data[1], 0xF1,
            "ADTS sync word 低字节 + protection_absent=1"
        );
    }

    #[test]
    fn test_flush_和_eof() {
        let params = make_aac_params(44100, 1);
        let mut enc = AacEncoder::create().unwrap();
        enc.open(&params).unwrap();
        enc.send_frame(None).unwrap();
        let err = enc.receive_packet().unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_adts_header_格式() {
        let params = make_aac_params(44100, 2);
        let mut enc = AacEncoder::create().unwrap();
        enc.open(&params).unwrap();

        let nb_samples = 1024u32;
        let data = vec![0.0f32; nb_samples as usize];
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();

        let mut af = AudioFrame::new(nb_samples, 44100, SampleFormat::F32, ChannelLayout::STEREO);
        af.data[0] = bytes.repeat(2);

        enc.send_frame(Some(&Frame::Audio(af))).unwrap();
        let pkt = enc.receive_packet().unwrap();

        assert!(pkt.data.len() >= 7);
        let h = &pkt.data[0..7];
        assert_eq!(h[0], 0xFF);
        assert_eq!(h[1], 0xF1);
        let profile = (h[2] >> 6) & 0x03;
        assert_eq!(profile, 1, "AAC-LC profile = 1");
        let sr_index = (h[2] >> 2) & 0x0F;
        assert_eq!(sr_index, 4, "44100 Hz -> index 4");
        let channel_config = ((h[2] & 0x01) << 2) | (h[3] >> 6);
        assert_eq!(channel_config, 2, "立体声 channel_config = 2");
    }

    #[test]
    fn test_mdct_静音输入() {
        let input = vec![0.0; MDCT_INPUT_SIZE];
        let output = AacEncoder::mdct(&input);
        assert_eq!(output.len(), AAC_FRAME_SIZE);
        for &v in &output {
            assert!((v.abs() < 1e-10), "静音输入应得零输出");
        }
    }

    #[test]
    fn test_sine_window_范围() {
        let mut buf = vec![1.0; MDCT_INPUT_SIZE];
        AacEncoder::apply_sine_window(&mut buf);
        for &v in &buf {
            assert!(v >= 0.0 && v <= 1.0, "窗函数值应在 [0,1]");
        }
    }
}
