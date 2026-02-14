//! AAC-LC (Low Complexity) 音频解码器.
//!
//! 支持从 MP4/ADTS 容器中解码 AAC-LC 音频为 PCM 数据.
//!
//! # 解码流程
//! 1. 解析 ADTS 帧头 (采样率, 声道数, profile)
//! 2. 解析原始数据块 (SCE, CPE 等语法元素)
//! 3. Huffman 解码频谱系数 + scale factor
//! 4. 反量化
//! 5. IMDCT 变换 (频域 → 时域)
//! 6. 窗函数加窗 + overlap-add
//! 7. 输出 PCM 采样

mod huffman;

use log::debug;
use tao_core::bitreader::BitReader;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

use huffman::AacCodebooks;

/// AAC 采样率索引表
const AAC_SAMPLE_RATES: [u32; 16] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 0,
    0,
];

/// 44100Hz 下 1024 点 LONG 窗口的 SFB 边界 (49 个 band)
const SWB_OFFSET_1024_44100: [usize; 50] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 88, 96, 108, 120, 132, 144, 160,
    176, 196, 216, 240, 264, 292, 320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
    736, 768, 800, 832, 864, 896, 928, 1024,
];

/// 48000Hz 下 1024 点 LONG 窗口的 SFB 边界
const SWB_OFFSET_1024_48000: [usize; 50] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 88, 96, 108, 120, 132, 144, 160,
    176, 196, 216, 240, 264, 292, 320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
    736, 768, 800, 832, 864, 896, 928, 1024,
];

/// AAC-LC 解码器
pub struct AacDecoder {
    sample_rate: u32,
    channels: u32,
    channel_layout: ChannelLayout,
    sample_rate_index: u8,
    output_frame: Option<Frame>,
    opened: bool,
    flushing: bool,
    /// overlap-add 缓冲 (每声道 1024 个浮点样本)
    overlap: Vec<Vec<f32>>,
    first_frame: bool,
    /// Huffman 码本 (在 open 时构建)
    codebooks: Option<AacCodebooks>,
}

impl AacDecoder {
    /// 创建 AAC 解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
            sample_rate_index: 4,
            output_frame: None,
            opened: false,
            flushing: false,
            overlap: Vec::new(),
            first_frame: true,
            codebooks: None,
        }))
    }

    /// 从 AudioSpecificConfig 解析参数
    fn parse_audio_specific_config(&mut self, data: &[u8]) -> TaoResult<()> {
        if data.len() < 2 {
            return Ok(());
        }
        let aot = (data[0] >> 3) & 0x1F;
        if aot != 2 {
            return Err(TaoError::Unsupported(format!(
                "AAC: 不支持 audioObjectType={aot}, 仅支持 AAC-LC (2)"
            )));
        }
        let freq_idx = ((data[0] & 0x07) << 1) | (data[1] >> 7);
        let chan_config = (data[1] >> 3) & 0x0F;

        if (freq_idx as usize) < AAC_SAMPLE_RATES.len() && AAC_SAMPLE_RATES[freq_idx as usize] > 0 {
            self.sample_rate = AAC_SAMPLE_RATES[freq_idx as usize];
            self.sample_rate_index = freq_idx;
        }
        if chan_config > 0 && chan_config <= 7 {
            self.channels = match chan_config {
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                5 => 5,
                6 => 6,
                7 => 8,
                _ => 2,
            };
            self.channel_layout = ChannelLayout::from_channels(self.channels);
        }
        Ok(())
    }

    /// 获取当前采样率对应的 SFB 边界表
    fn swb_offset(&self) -> &'static [usize] {
        match self.sample_rate {
            48000 => &SWB_OFFSET_1024_48000,
            _ => &SWB_OFFSET_1024_44100,
        }
    }

    /// 解码一个原始 AAC 帧
    fn decode_raw_frame(&mut self, data: &[u8]) -> TaoResult<Vec<Vec<f32>>> {
        let channels = self.channels as usize;
        let mut spectral = vec![vec![0.0f32; 1024]; channels];

        if !data.is_empty() {
            self.parse_raw_data_block(data, &mut spectral)?;
        }

        // IMDCT + 窗函数 + overlap-add
        let mut pcm_out = vec![vec![0.0f32; 1024]; channels];
        for ch in 0..channels {
            let time_domain = imdct_1024(&spectral[ch]);
            let windowed = apply_sine_window(&time_domain);

            if self.first_frame {
                pcm_out[ch][..1024].copy_from_slice(&windowed[..1024]);
            } else {
                for i in 0..1024 {
                    pcm_out[ch][i] = self.overlap[ch][i] + windowed[i];
                }
            }
            if ch < self.overlap.len() {
                self.overlap[ch] = windowed[1024..2048].to_vec();
            }
        }
        self.first_frame = false;
        Ok(pcm_out)
    }

    /// 解析原始数据块 (raw_data_block)
    fn parse_raw_data_block(&self, data: &[u8], spectral: &mut [Vec<f32>]) -> TaoResult<()> {
        let mut br = BitReader::new(data);
        let mut ch_idx = 0usize;
        let codebooks = self
            .codebooks
            .as_ref()
            .ok_or_else(|| TaoError::InvalidData("AAC: 码本未初始化".into()))?;

        debug!(
            "AAC raw_data_block: {} 字节, 前4={:02x?}",
            data.len(),
            &data[..data.len().min(4)]
        );

        while br.bits_left() >= 3 {
            let id_syn_ele = br.read_bits(3)?;
            if id_syn_ele == 7 {
                break; // END
            }
            match id_syn_ele {
                0 => {
                    // SCE: Single Channel Element
                    let _instance_tag = br.read_bits(4)?;
                    if ch_idx < spectral.len() {
                        self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, false)?;
                    }
                    ch_idx += 1;
                }
                1 => {
                    // CPE: Channel Pair Element
                    let _instance_tag = br.read_bits(4)?;
                    let common_window = br.read_bit()? != 0;

                    if common_window {
                        let info = self.parse_ics_info(&mut br)?;
                        // ms_mask_present (2 bits)
                        let ms_mask = br.read_bits(2)?;
                        if ms_mask == 1 {
                            // 读取并跳过 ms_used 标志
                            for _ in 0..info.max_sfb {
                                br.read_bit()?;
                            }
                        }
                        // 两个声道使用共享的 ics_info
                        if ch_idx < spectral.len() {
                            self.parse_ics_with_info(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                info,
                            )?;
                        }
                        ch_idx += 1;
                        if ch_idx < spectral.len() {
                            self.parse_ics_with_info(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                info,
                            )?;
                        }
                        ch_idx += 1;
                    } else {
                        if ch_idx < spectral.len() {
                            self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, false)?;
                        }
                        ch_idx += 1;
                        if ch_idx < spectral.len() {
                            self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, false)?;
                        }
                        ch_idx += 1;
                    }
                }
                3 => {
                    // LFE: 和 SCE 结构相同
                    let _instance_tag = br.read_bits(4)?;
                    if ch_idx < spectral.len() {
                        self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, false)?;
                    }
                    ch_idx += 1;
                }
                4 => {
                    // DSE: Data Stream Element - 跳过
                    let _tag = br.read_bits(4)?;
                    let align = br.read_bit()?;
                    let mut count = br.read_bits(8)? as usize;
                    if count == 255 {
                        count += br.read_bits(8)? as usize;
                    }
                    if align != 0 {
                        br.align_to_byte();
                    }
                    for _ in 0..count {
                        br.read_bits(8)?;
                    }
                }
                6 => {
                    // FIL: Fill Element - 跳过
                    let mut count = br.read_bits(4)? as usize;
                    if count == 15 {
                        count += br.read_bits(8)? as usize - 1;
                    }
                    for _ in 0..count {
                        br.read_bits(8)?;
                    }
                }
                _ => break, // 未知元素, 停止解析
            }
        }
        Ok(())
    }

    /// 解析 ICS info
    fn parse_ics_info(&self, br: &mut BitReader) -> TaoResult<IcsInfo> {
        let _reserved = br.read_bit()?; // ics_reserved_bit
        let window_sequence = br.read_bits(2)?;
        let _window_shape = br.read_bit()?;

        if window_sequence == 2 {
            // EIGHT_SHORT_SEQUENCE
            let max_sfb = br.read_bits(4)? as usize;
            let _scale_factor_grouping = br.read_bits(7)?;
            Ok(IcsInfo {
                window_sequence,
                max_sfb,
                num_swb: 0, // SHORT 窗口暂不支持
            })
        } else {
            // ONLY_LONG / LONG_START / LONG_STOP
            let max_sfb = br.read_bits(6)? as usize;
            let predictor_present = br.read_bit()? != 0;
            if predictor_present {
                // AAC-LC 一般不使用 predictor, 跳过
                // predictor_reset (1 bit)
                let _ = br.read_bit();
            }
            let num_swb = self.swb_offset().len() - 1;
            Ok(IcsInfo {
                window_sequence,
                max_sfb: max_sfb.min(num_swb),
                num_swb,
            })
        }
    }

    /// 解析 individual_channel_stream (独立读取 ics_info)
    fn parse_ics(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        _common_window: bool,
    ) -> TaoResult<()> {
        let global_gain = br.read_bits(8)? as i32;
        let info = self.parse_ics_info(br)?;
        self.decode_ics(br, spectral, codebooks, info, global_gain)
    }

    /// 解析 individual_channel_stream (使用共享 ics_info)
    fn parse_ics_with_info(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        info: IcsInfo,
    ) -> TaoResult<()> {
        let global_gain = br.read_bits(8)? as i32;
        self.decode_ics(br, spectral, codebooks, info, global_gain)
    }

    /// 解码 ICS 内容 (section + scalefactor + spectral)
    fn decode_ics(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        info: IcsInfo,
        global_gain: i32,
    ) -> TaoResult<()> {
        if info.window_sequence == 2 {
            // SHORT 窗口暂不支持, 输出静音
            return Ok(());
        }

        debug!(
            "AAC ICS: win={}, max_sfb={}, gain={}, bits_left={}",
            info.window_sequence,
            info.max_sfb,
            global_gain,
            br.bits_left()
        );

        // 1. section_data
        let sections = parse_section_data(br, info.max_sfb)?;
        {
            let cbs: Vec<_> = sections
                .iter()
                .map(|s| format!("cb{}[{}..{}]", s.sect_cb, s.sect_start, s.sect_end))
                .collect();
            debug!(
                "AAC section_data: {} 个段 {:?}, bits_left={}",
                sections.len(),
                cbs,
                br.bits_left()
            );
        }

        // 2. scale_factor_data
        let scale_factors =
            parse_scale_factor_data(br, &sections, info.num_swb, global_gain, codebooks)?;
        debug!("AAC sf_data: bits_left={}", br.bits_left());

        // 3. pulse_data_present
        let pulse_present = br.read_bit()? != 0;
        if pulse_present {
            skip_pulse_data(br)?;
        }

        // 4. tns_data_present
        let tns_present = br.read_bit()? != 0;
        if tns_present {
            skip_tns_data(br)?;
        }

        // 5. gain_control_data_present (AAC-LC: 始终为 0)
        let _gain_control = br.read_bit()?;

        debug!(
            "AAC 频谱解码前: bits_left={}, pulse={}, tns={}",
            br.bits_left(),
            pulse_present,
            tns_present
        );

        // 6. spectral_data
        decode_spectral_data(
            br,
            spectral,
            &sections,
            &scale_factors,
            codebooks,
            self.swb_offset(),
        )
    }

    /// 跳过 ADTS 头, 返回原始帧数据
    fn strip_adts_header<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        if data.len() >= 7 && data[0] == 0xFF && (data[1] & 0xF0) == 0xF0 {
            let protection_absent = (data[1] & 0x01) != 0;
            let header_size = if protection_absent { 7 } else { 9 };
            if data.len() > header_size {
                return &data[header_size..];
            }
        }
        data
    }
}

// ============================================================
// ICS 解析辅助
// ============================================================

#[derive(Clone, Copy)]
struct IcsInfo {
    window_sequence: u32,
    max_sfb: usize,
    num_swb: usize,
}

struct Section {
    sect_cb: u8,
    sect_start: usize,
    sect_end: usize,
}

/// 解析 section_data (LONG_WINDOW)
fn parse_section_data(br: &mut BitReader, max_sfb: usize) -> TaoResult<Vec<Section>> {
    let mut sections = Vec::new();
    let mut k = 0usize;

    while k < max_sfb {
        let sect_cb = br.read_bits(4)? as u8;
        let mut sect_len = 0usize;
        // 读取 sect_len_incr (5 bits for LONG, ESC=31)
        loop {
            let incr = br.read_bits(5)? as usize;
            sect_len += incr;
            if incr != 31 {
                break;
            }
        }
        let sect_end = (k + sect_len).min(max_sfb);
        sections.push(Section {
            sect_cb,
            sect_start: k,
            sect_end,
        });
        k = sect_end;
    }
    Ok(sections)
}

/// 噪声偏移 (ISO 14496-3)
const NOISE_OFFSET: i32 = 90;

/// AAC 特殊码本定义
const NOISE_HCB: u8 = 13;
const INTENSITY_HCB2: u8 = 14;
const INTENSITY_HCB: u8 = 15;

/// 解析 scale_factor_data (ISO 14496-3, 4.5.2.3.4)
///
/// 区分三种码本类型:
/// - 普通码本 (1-11): Huffman 编码 scale factor delta
/// - 噪声 (13): 第一个频带读 9 位原始值, 后续 Huffman 编码
/// - 强度立体声 (14/15): Huffman 编码 IS position delta
fn parse_scale_factor_data(
    br: &mut BitReader,
    sections: &[Section],
    num_swb: usize,
    global_gain: i32,
    codebooks: &AacCodebooks,
) -> TaoResult<Vec<i32>> {
    let mut scale_factors = vec![0i32; num_swb];
    let mut sf = global_gain;
    let mut is_position = 0i32;
    let mut noise_energy = global_gain - NOISE_OFFSET;
    let mut noise_pcm_flag = true; // 第一个噪声频带标志

    for section in sections {
        let cb = section.sect_cb;
        let end = section.sect_end.min(num_swb);
        for sf_slot in scale_factors.iter_mut().take(end).skip(section.sect_start) {
            if cb == 0 {
                // ZERO_HCB: 无 scale factor
                *sf_slot = 0;
            } else if cb == NOISE_HCB {
                // NOISE_HCB (13): 噪声能量
                if noise_pcm_flag {
                    // 第一个噪声频带: 读取 9 位原始值
                    noise_pcm_flag = false;
                    let raw = br.read_bits(9)? as i32;
                    noise_energy = global_gain - NOISE_OFFSET + raw;
                } else {
                    let delta = codebooks.sf_tree.decode(br)? - 60;
                    noise_energy += delta;
                }
                *sf_slot = noise_energy;
            } else if cb == INTENSITY_HCB || cb == INTENSITY_HCB2 {
                // INTENSITY_HCB (15) / INTENSITY_HCB2 (14): IS position
                let delta = codebooks.sf_tree.decode(br)? - 60;
                is_position += delta;
                *sf_slot = is_position;
            } else {
                // 普通频谱码本: scale factor
                let delta = codebooks.sf_tree.decode(br)? - 60;
                sf += delta;
                *sf_slot = sf.clamp(0, 255);
            }
        }
    }
    Ok(scale_factors)
}

/// 解码频谱数据
fn decode_spectral_data(
    br: &mut BitReader,
    spectral: &mut [f32],
    sections: &[Section],
    scale_factors: &[i32],
    codebooks: &AacCodebooks,
    swb_offset: &[usize],
) -> TaoResult<()> {
    for section in sections {
        let cb = section.sect_cb;
        if cb == 0 || cb == NOISE_HCB || cb == INTENSITY_HCB2 || cb == INTENSITY_HCB {
            // ZERO_HCB / NOISE / INTENSITY: 频谱为 0
            continue;
        }

        for sfb in section.sect_start..section.sect_end {
            let start_idx = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end_idx = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            let sf = scale_factors.get(sfb).copied().unwrap_or(0);
            let num_lines = end_idx - start_idx;

            if (1..=11).contains(&cb) {
                let cb_idx = (cb - 1) as usize;
                if let Some(ref spec_cb) = codebooks.spectral[cb_idx] {
                    // 使用 Huffman 解码频谱值
                    let mut i = 0;
                    while i < num_lines {
                        let values = match spec_cb.decode_values(br) {
                            Ok(v) => v,
                            Err(e) => {
                                debug!(
                                    "AAC 频谱解码错误: sfb={}, cb={}, i={}/{}, bits_left={}",
                                    sfb,
                                    cb,
                                    i,
                                    num_lines,
                                    br.bits_left()
                                );
                                return Err(e);
                            }
                        };
                        let count = spec_cb.dim.min(num_lines - i);
                        for (j, &v) in values.iter().enumerate().take(count) {
                            let idx = start_idx + i + j;
                            if idx < spectral.len() {
                                spectral[idx] = inverse_quantize(v, sf);
                            }
                        }
                        i += spec_cb.dim;
                    }
                } else {
                    // 未实现的码本: 无法正确跳过, 返回错误
                    return Err(TaoError::Unsupported(format!("AAC: 频谱码本 {cb} 未实现")));
                }
            }
        }
    }
    Ok(())
}

/// 跳过 pulse_data
fn skip_pulse_data(br: &mut BitReader) -> TaoResult<()> {
    let num_pulse = br.read_bits(2)? + 1;
    let _pulse_start_sfb = br.read_bits(6)?;
    for _ in 0..num_pulse {
        let _offset = br.read_bits(5)?;
        let _amp = br.read_bits(4)?;
    }
    Ok(())
}

/// 跳过 tns_data (简化: LONG_WINDOW)
fn skip_tns_data(br: &mut BitReader) -> TaoResult<()> {
    let n_filt = br.read_bits(2)?;
    if n_filt > 0 {
        let coef_res = br.read_bit()?;
        for _ in 0..n_filt {
            let length = br.read_bits(6)?;
            let order = br.read_bits(5)?;
            if order > 0 {
                let _direction = br.read_bit()?;
                let coef_compress = br.read_bit()?;
                let coef_bits = (coef_res + 3 - coef_compress).max(1);
                for _ in 0..order {
                    br.read_bits(coef_bits)?;
                }
            }
            let _ = length; // 使用 length 避免 unused warning
        }
    }
    Ok(())
}

/// 反量化: iq = sign(x) * |x|^(4/3) * 2^(0.25 * (sf - 100))
fn inverse_quantize(x: i32, sf: i32) -> f32 {
    if x == 0 {
        return 0.0;
    }
    let sign = if x > 0 { 1.0f32 } else { -1.0f32 };
    let abs_x = x.unsigned_abs() as f32;
    let pow_val = abs_x.powf(4.0 / 3.0);
    let scale = 2.0f32.powf(0.25 * (sf - 100) as f32);
    sign * pow_val * scale
}

// ============================================================
// Decoder trait 实现
// ============================================================

impl Decoder for AacDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Aac
    }

    fn name(&self) -> &str {
        "aac"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        if let CodecParamsType::Audio(ref audio) = params.params {
            self.sample_rate = audio.sample_rate;
            self.channels = audio.channel_layout.channels;
            self.channel_layout = audio.channel_layout;
        }
        if !params.extra_data.is_empty() {
            self.parse_audio_specific_config(&params.extra_data)?;
        }
        // 构建 Huffman 码本
        self.codebooks = Some(AacCodebooks::build());
        self.overlap = vec![vec![0.0f32; 1024]; self.channels as usize];
        self.first_frame = true;
        self.opened = true;
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::InvalidData("AAC 解码器未打开".into()));
        }
        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }

        let raw_data = self.strip_adts_header(&packet.data);

        // 解码, 失败时输出静音
        let pcm = match self.decode_raw_frame(raw_data) {
            Ok(pcm) => pcm,
            Err(e) => {
                debug!("AAC 帧解码失败: {}, 输出静音", e);
                vec![vec![0.0f32; 1024]; self.channels as usize]
            }
        };

        let channels = self.channels as usize;
        let num_samples = 1024;
        let mut interleaved = vec![0u8; num_samples * channels * 4];

        for i in 0..num_samples {
            for ch in 0..channels {
                let sample = if ch < pcm.len() {
                    pcm[ch][i].clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                let bytes = sample.to_le_bytes();
                let offset = (i * channels + ch) * 4;
                interleaved[offset..offset + 4].copy_from_slice(&bytes);
            }
        }

        let frame = AudioFrame {
            data: vec![interleaved],
            nb_samples: num_samples as u32,
            sample_rate: self.sample_rate,
            channel_layout: self.channel_layout,
            sample_format: SampleFormat::F32,
            pts: packet.pts,
            time_base: tao_core::Rational::new(1, self.sample_rate as i32),
            duration: num_samples as i64,
        };
        self.output_frame = Some(Frame::Audio(frame));
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.output_frame.take() {
            Ok(frame)
        } else if self.flushing {
            Err(TaoError::Eof)
        } else {
            Err(TaoError::NeedMoreData)
        }
    }

    fn flush(&mut self) {
        self.output_frame = None;
        self.flushing = false;
        self.first_frame = true;
        for ch in &mut self.overlap {
            ch.fill(0.0);
        }
    }
}

// ============================================================
// IMDCT + 窗函数
// ============================================================

/// 1024 点 IMDCT (输入 1024 频谱系数, 输出 2048 时域样本)
fn imdct_1024(spectrum: &[f32]) -> Vec<f32> {
    let n = 1024;
    let n2 = 2 * n;
    let mut output = vec![0.0f32; n2];

    // 快速路径: 全零
    if spectrum.iter().all(|&s| s == 0.0) {
        return output;
    }

    let scale = 2.0 / n as f64;
    let half_n = n as f64 / 2.0;

    for (i, out_sample) in output.iter_mut().enumerate() {
        let mut sum = 0.0f64;
        let n_plus_half = i as f64 + 0.5 + half_n;
        for (k, &spec_val) in spectrum.iter().enumerate() {
            if spec_val == 0.0 {
                continue;
            }
            let k_plus_half = k as f64 + 0.5;
            let angle = std::f64::consts::PI / n as f64 * n_plus_half * k_plus_half;
            sum += spec_val as f64 * angle.cos();
        }
        *out_sample = (sum * scale) as f32;
    }
    output
}

/// 正弦窗函数 (2048 点)
fn apply_sine_window(time_domain: &[f32]) -> Vec<f32> {
    let n = time_domain.len();
    let mut windowed = vec![0.0f32; n];
    for i in 0..n {
        let w = (std::f64::consts::PI / n as f64 * (i as f64 + 0.5)).sin();
        windowed[i] = time_domain[i] * w as f32;
    }
    windowed
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_parameters::{AudioCodecParams, CodecParamsType};

    fn make_aac_params() -> CodecParameters {
        CodecParameters {
            codec_id: CodecId::Aac,
            extra_data: vec![0x12, 0x10], // AAC-LC, 44100Hz, stereo
            bit_rate: 0,
            params: CodecParamsType::Audio(AudioCodecParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::F32,
                frame_size: 1024,
            }),
        }
    }

    #[test]
    fn test_创建与打开() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();
        assert_eq!(decoder.codec_id(), CodecId::Aac);
        assert_eq!(decoder.name(), "aac");
    }

    #[test]
    fn test_未打开报错() {
        let mut decoder = AacDecoder::create().unwrap();
        let pkt = Packet::from_data(vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x00, 0x00]);
        assert!(decoder.send_packet(&pkt).is_err());
    }

    #[test]
    fn test_静音帧解码() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();

        let silent_data = vec![0u8; 10];
        let pkt = Packet::from_data(silent_data);
        decoder.send_packet(&pkt).unwrap();

        let frame = decoder.receive_frame().unwrap();
        if let Frame::Audio(af) = frame {
            assert_eq!(af.nb_samples, 1024);
            assert_eq!(af.sample_rate, 44100);
        } else {
            panic!("应为音频帧");
        }
    }

    #[test]
    fn test_flush_和_eof() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();

        let empty_pkt = Packet::empty();
        decoder.send_packet(&empty_pkt).unwrap();
        assert!(matches!(decoder.receive_frame(), Err(TaoError::Eof)));
    }

    #[test]
    fn test_audio_specific_config_解析() {
        let mut dec = AacDecoder {
            sample_rate: 0,
            channels: 0,
            channel_layout: ChannelLayout::from_channels(1),
            sample_rate_index: 0,
            output_frame: None,
            opened: false,
            flushing: false,
            overlap: Vec::new(),
            first_frame: true,
            codebooks: None,
        };
        dec.parse_audio_specific_config(&[0x12, 0x10]).unwrap();
        assert_eq!(dec.sample_rate, 44100);
        assert_eq!(dec.channels, 2);
    }

    #[test]
    fn test_imdct_全零() {
        let spectrum = vec![0.0f32; 1024];
        let output = imdct_1024(&spectrum);
        assert_eq!(output.len(), 2048);
        for &s in &output {
            assert_eq!(s, 0.0);
        }
    }

    #[test]
    fn test_sf_huffman_树构建() {
        let cbs = AacCodebooks::build();
        // 测试 delta=0 (index=60): 码字 "0" (1 bit)
        let data = [0x00u8]; // 第一位是 0
        let mut br = BitReader::new(&data);
        let val = cbs.sf_tree.decode(&mut br).unwrap();
        assert_eq!(val, 60); // SF index 60 = delta 0
    }

    #[test]
    fn test_adts_头_跳过() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();

        let mut adts_frame = vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC];
        adts_frame.extend_from_slice(&[0; 10]);
        let pkt = Packet::from_data(adts_frame);
        decoder.send_packet(&pkt).unwrap();
        assert!(matches!(decoder.receive_frame(), Ok(Frame::Audio(_))));
    }
}
