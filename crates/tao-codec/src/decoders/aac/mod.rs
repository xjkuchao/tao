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
mod imdct;
pub(crate) mod spectral;
pub(crate) mod tables;
#[cfg(test)]
mod tests;
use std::cell::Cell;

use log::info;
use tao_core::bitreader::BitReader;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

use huffman::AacCodebooks;
use imdct::*;
use spectral::*;
use tables::*;

/// AAC-LC 解码器
pub struct AacDecoder {
    sample_rate: u32,
    channels: u32,
    channel_layout: ChannelLayout,
    channel_config: u8,
    use_default_channel_map: bool,
    sample_rate_index: u8,
    output_frame: Option<Frame>,
    opened: bool,
    flushing: bool,
    /// overlap-add 缓冲 (每声道 1024 个浮点样本)
    overlap: Vec<Vec<f32>>,
    first_frame: bool,
    /// Huffman 码本 (在 open 时构建)
    codebooks: Option<AacCodebooks>,
    /// 解码会话配置的首包裁剪样本数.
    default_leading_trim_samples: usize,
    /// 尚未消费的首包裁剪样本数.
    pending_leading_trim_samples: usize,
    /// 每声道上一帧窗口形状 (0=sine, 1=kbd).
    prev_window_shape: Vec<u8>,
    /// 2048 点 sine 窗.
    long_sine_window: Vec<f32>,
    /// 2048 点 kbd 窗.
    long_kbd_window: Vec<f32>,
    /// 256 点 sine 窗.
    short_sine_window: Vec<f32>,
    /// 256 点 kbd 窗.
    short_kbd_window: Vec<f32>,
    /// PNS 随机状态.
    random_state: Cell<u32>,
}

impl AacDecoder {
    /// 创建 AAC 解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
            channel_config: 2,
            use_default_channel_map: true,
            sample_rate_index: 4,
            output_frame: None,
            opened: false,
            flushing: false,
            overlap: Vec::new(),
            first_frame: true,
            codebooks: None,
            default_leading_trim_samples: 0,
            pending_leading_trim_samples: 0,
            prev_window_shape: Vec::new(),
            long_sine_window: Vec::new(),
            long_kbd_window: Vec::new(),
            short_sine_window: Vec::new(),
            short_kbd_window: Vec::new(),
            random_state: Cell::new(0x1f2e3d4c),
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
            self.channels = Self::channels_from_config(chan_config);
            self.channel_layout = ChannelLayout::from_channels(self.channels);
            self.channel_config = chan_config;
            self.use_default_channel_map = true;
        } else if chan_config == 0 {
            // 显式 PCE 声道布局, 不应套用默认声道重排表.
            self.use_default_channel_map = false;
        }
        Ok(())
    }

    fn channels_from_config(channel_config: u8) -> u32 {
        match channel_config {
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 5,
            6 => 6,
            7 => 8,
            _ => 2,
        }
    }

    /// AAC 默认声道配置到常见播放声道顺序的映射.
    ///
    /// 返回值含义: `输出声道索引 -> 解码内部声道索引`.
    fn output_channel_map(&self) -> Option<&'static [usize]> {
        if !self.use_default_channel_map {
            return None;
        }
        const MAP_3: [usize; 3] = [1, 2, 0];
        const MAP_5: [usize; 5] = [1, 2, 0, 3, 4];
        const MAP_6: [usize; 6] = [1, 2, 0, 5, 3, 4];
        const MAP_7: [usize; 8] = [1, 2, 0, 7, 5, 6, 3, 4];
        match self.channel_config {
            3 => Some(&MAP_3),
            5 => Some(&MAP_5),
            6 => Some(&MAP_6),
            7 => Some(&MAP_7),
            _ => None,
        }
    }

    /// 获取当前采样率对应的 SFB 边界表
    fn swb_offset(&self) -> &'static [usize] {
        match self.sample_rate_index {
            0 | 1 => &SWB_OFFSET_1024_96,
            2 => &SWB_OFFSET_1024_64,
            3 | 4 => &SWB_OFFSET_1024_48,
            5 => &SWB_OFFSET_1024_32,
            6 | 7 => &SWB_OFFSET_1024_24,
            8..=10 => &SWB_OFFSET_1024_16,
            11 | 12 => &SWB_OFFSET_1024_8,
            _ => &SWB_OFFSET_1024_48,
        }
    }

    /// 获取当前采样率对应的 SHORT 窗口 SFB 边界表
    fn swb_offset_short(&self) -> &'static [usize] {
        match self.sample_rate_index {
            0..=2 => &SWB_OFFSET_128_96,
            3..=5 => &SWB_OFFSET_128_48,
            6 | 7 => &SWB_OFFSET_128_24,
            8..=10 => &SWB_OFFSET_128_16,
            11 | 12 => &SWB_OFFSET_128_8,
            _ => &SWB_OFFSET_128_48,
        }
    }

    /// 获取当前采样率下 TNS 可作用的最大频带数.
    fn tns_max_bands(&self, is_short: bool) -> usize {
        let idx = (self.sample_rate_index as usize).min(12);
        if is_short {
            TNS_MAX_BANDS_128[idx] as usize
        } else {
            TNS_MAX_BANDS_1024[idx] as usize
        }
    }

    /// 解码一个原始 AAC 帧
    fn decode_raw_frame(&mut self, data: &[u8]) -> TaoResult<Vec<Vec<f32>>> {
        let channels = self.channels as usize;
        if self.overlap.len() != channels {
            self.overlap = vec![vec![0.0f32; 1024]; channels];
            self.first_frame = true;
        }
        if self.prev_window_shape.len() != channels {
            self.prev_window_shape.resize(channels, 0);
        }
        let mut spectral = vec![vec![0.0f32; 1024]; channels];
        let mut window_sequences = vec![0u32; channels];
        let mut window_shapes = self.prev_window_shape.clone();

        if !data.is_empty() {
            self.parse_raw_data_block(
                data,
                &mut spectral,
                &mut window_sequences,
                &mut window_shapes,
            )?;
        }

        // IMDCT + 窗函数 + overlap-add
        let mut pcm_out = vec![vec![0.0f32; 1024]; channels];
        for ch in 0..channels {
            let window_sequence = window_sequences.get(ch).copied().unwrap_or(0);
            let prev_shape = self.prev_window_shape.get(ch).copied().unwrap_or(0);
            let curr_shape = window_shapes.get(ch).copied().unwrap_or(prev_shape);
            let windowed = if window_sequence == 2 {
                synthesize_short_windows(
                    &spectral[ch],
                    curr_shape,
                    &self.short_sine_window,
                    &self.short_kbd_window,
                )
            } else {
                let time_domain = imdct_1024(&spectral[ch]);
                let windows = AacWindowBank {
                    long_sine: &self.long_sine_window,
                    long_kbd: &self.long_kbd_window,
                    short_sine: &self.short_sine_window,
                    short_kbd: &self.short_kbd_window,
                };
                apply_aac_long_window(
                    &time_domain,
                    window_sequence,
                    prev_shape,
                    curr_shape,
                    &windows,
                )
            };

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
            if ch < self.prev_window_shape.len() {
                self.prev_window_shape[ch] = curr_shape;
            }
        }
        self.first_frame = false;
        Ok(pcm_out)
    }

    /// 解析原始数据块 (raw_data_block)
    fn parse_raw_data_block(
        &mut self,
        data: &[u8],
        spectral: &mut [Vec<f32>],
        window_sequences: &mut [u32],
        window_shapes: &mut [u8],
    ) -> TaoResult<()> {
        let mut br = BitReader::new(data);
        let mut ch_idx = 0usize;

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
                        let codebooks = self
                            .codebooks
                            .as_ref()
                            .ok_or_else(|| TaoError::InvalidData("AAC: 码本未初始化".into()))?;
                        let info =
                            self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, None, false)?;
                        if ch_idx < window_sequences.len() {
                            window_sequences[ch_idx] = info.window_sequence;
                        }
                        if ch_idx < window_shapes.len() {
                            window_shapes[ch_idx] = info.window_shape;
                        }
                    }
                    ch_idx += 1;
                }
                1 => {
                    // CPE: Channel Pair Element
                    let _instance_tag = br.read_bits(4)?;
                    let common_window = br.read_bit()? != 0;
                    let codebooks = self
                        .codebooks
                        .as_ref()
                        .ok_or_else(|| TaoError::InvalidData("AAC: 码本未初始化".into()))?;

                    if common_window {
                        let info = self.parse_ics_info(&mut br)?;
                        // ms_mask_present (2 bits)
                        let ms_mask_present = br.read_bits(2)?;
                        let ms_band_count = info.max_sfb * info.num_window_groups;
                        let mut ms_used = vec![false; ms_band_count];
                        if ms_mask_present == 1 {
                            for slot in &mut ms_used {
                                *slot = br.read_bit()? != 0;
                            }
                        } else if ms_mask_present == 2 {
                            ms_used.fill(true);
                        }
                        // 两个声道使用共享的 ics_info
                        let left_idx = ch_idx;
                        let mut left_band_info = IcsBandInfo::default();
                        if ch_idx < spectral.len() {
                            self.parse_ics_with_info(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                Some(&mut left_band_info),
                                info,
                            )?;
                            if ch_idx < window_sequences.len() {
                                window_sequences[ch_idx] = info.window_sequence;
                            }
                            if ch_idx < window_shapes.len() {
                                window_shapes[ch_idx] = info.window_shape;
                            }
                        }
                        ch_idx += 1;
                        let mut right_band_info = IcsBandInfo::default();
                        if ch_idx < spectral.len() {
                            self.parse_ics_with_info(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                Some(&mut right_band_info),
                                info,
                            )?;
                            if ch_idx < window_sequences.len() {
                                window_sequences[ch_idx] = info.window_sequence;
                            }
                            if ch_idx < window_shapes.len() {
                                window_shapes[ch_idx] = info.window_shape;
                            }
                        }
                        let right_idx = ch_idx;
                        if left_idx < spectral.len() && right_idx < spectral.len() {
                            let swb_offset = if info.window_sequence == 2 {
                                self.swb_offset_short()
                            } else {
                                self.swb_offset()
                            };
                            if !ms_used.is_empty() && ms_mask_present != 0 {
                                let (left_slice, right_slice) = spectral.split_at_mut(right_idx);
                                apply_ms_stereo(
                                    &mut left_slice[left_idx],
                                    &mut right_slice[0],
                                    &info,
                                    &ms_used,
                                    Some(&left_band_info.band_types),
                                    Some(&right_band_info.band_types),
                                    swb_offset,
                                );
                            }
                            let (left_slice, right_slice) = spectral.split_at_mut(right_idx);
                            apply_intensity_stereo(
                                &mut left_slice[left_idx],
                                &mut right_slice[0],
                                &info,
                                &right_band_info.band_types,
                                &right_band_info.scale_factors,
                                if ms_mask_present != 0 {
                                    Some(&ms_used)
                                } else {
                                    None
                                },
                                swb_offset,
                            );
                        }
                        ch_idx += 1;
                    } else {
                        if ch_idx < spectral.len() {
                            let info = self.parse_ics(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                None,
                                false,
                            )?;
                            if ch_idx < window_sequences.len() {
                                window_sequences[ch_idx] = info.window_sequence;
                            }
                            if ch_idx < window_shapes.len() {
                                window_shapes[ch_idx] = info.window_shape;
                            }
                        }
                        ch_idx += 1;
                        if ch_idx < spectral.len() {
                            let info = self.parse_ics(
                                &mut br,
                                &mut spectral[ch_idx],
                                codebooks,
                                None,
                                false,
                            )?;
                            if ch_idx < window_sequences.len() {
                                window_sequences[ch_idx] = info.window_sequence;
                            }
                            if ch_idx < window_shapes.len() {
                                window_shapes[ch_idx] = info.window_shape;
                            }
                        }
                        ch_idx += 1;
                    }
                }
                2 => {
                    // CCE: Coupling Channel Element
                    let codebooks = self
                        .codebooks
                        .as_ref()
                        .ok_or_else(|| TaoError::InvalidData("AAC: 码本未初始化".into()))?;
                    if self.skip_cce(&mut br, codebooks).is_err() {
                        break;
                    }
                }
                3 => {
                    // LFE: 和 SCE 结构相同
                    let _instance_tag = br.read_bits(4)?;
                    if ch_idx < spectral.len() {
                        let codebooks = self
                            .codebooks
                            .as_ref()
                            .ok_or_else(|| TaoError::InvalidData("AAC: 码本未初始化".into()))?;
                        let info =
                            self.parse_ics(&mut br, &mut spectral[ch_idx], codebooks, None, false)?;
                        if ch_idx < window_sequences.len() {
                            window_sequences[ch_idx] = info.window_sequence;
                        }
                        if ch_idx < window_shapes.len() {
                            window_shapes[ch_idx] = info.window_shape;
                        }
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
                5 => {
                    // PCE: Program Config Element
                    if self.skip_pce(&mut br).is_err() {
                        break;
                    }
                }
                6 => {
                    // FIL: Fill Element - 跳过
                    let mut count = br.read_bits(4)? as usize;
                    if count == 15 {
                        // 规范为 count += esc_count - 1, 对损坏码流 esc_count=0 做饱和保护, 避免 usize 下溢 panic.
                        let esc_count = br.read_bits(8)? as usize;
                        count += esc_count.saturating_sub(1);
                    }
                    for _ in 0..count {
                        br.read_bits(8)?;
                    }
                }
                _ => {
                    return Err(TaoError::Unsupported(format!(
                        "AAC: 未实现语法元素 id_syn_ele={id_syn_ele}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// 跳过 Program Config Element (PCE).
    fn skip_pce(&mut self, br: &mut BitReader) -> TaoResult<()> {
        // PCE 出现表示当前流采用显式声道元素布局, 禁用默认声道重排.
        self.use_default_channel_map = false;
        let _element_instance_tag = br.read_bits(4)?;
        let _object_type = br.read_bits(2)?;
        let sampling_frequency_index = br.read_bits(4)? as usize;
        let num_front = br.read_bits(4)? as usize;
        let num_side = br.read_bits(4)? as usize;
        let num_back = br.read_bits(4)? as usize;
        let num_lfe = br.read_bits(2)? as usize;
        let num_assoc_data = br.read_bits(3)? as usize;
        let num_valid_cc = br.read_bits(4)? as usize;

        if br.read_bit()? != 0 {
            let _mono_mixdown_tag = br.read_bits(4)?;
        }
        if br.read_bit()? != 0 {
            let _stereo_mixdown_tag = br.read_bits(4)?;
        }
        if br.read_bit()? != 0 {
            let _matrix_mixdown_idx = br.read_bits(2)?;
            let _pseudo_surround = br.read_bit()?;
        }

        let mut pce_channels = 0u32;
        for _ in 0..num_front {
            let is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
            pce_channels += if is_cpe != 0 { 2 } else { 1 };
        }
        for _ in 0..num_side {
            let is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
            pce_channels += if is_cpe != 0 { 2 } else { 1 };
        }
        for _ in 0..num_back {
            let is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
            pce_channels += if is_cpe != 0 { 2 } else { 1 };
        }
        for _ in 0..num_lfe {
            let _tag_select = br.read_bits(4)?;
            pce_channels += 1;
        }
        for _ in 0..num_assoc_data {
            let _tag_select = br.read_bits(4)?;
        }
        for _ in 0..num_valid_cc {
            let _is_ind_sw = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
        }

        br.align_to_byte();
        let comment_field_bytes = br.read_bits(8)? as usize;
        for _ in 0..comment_field_bytes {
            let _comment_byte = br.read_bits(8)?;
        }

        if (1..=8).contains(&pce_channels) && pce_channels <= self.channels {
            self.channels = pce_channels;
            self.channel_layout = ChannelLayout::from_channels(pce_channels);
            self.channel_config = match pce_channels {
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                5 => 5,
                6 => 6,
                8 => 7,
                _ => self.channel_config,
            };
        }
        let _ = sampling_frequency_index;
        Ok(())
    }

    /// 解析并跳过 Coupling Channel Element (CCE).
    ///
    /// 当前仅消费位流保证后续元素对齐, 暂不对目标声道施加耦合增益.
    fn skip_cce(&self, br: &mut BitReader, codebooks: &AacCodebooks) -> TaoResult<()> {
        let mut coupling_point = 2 * br.read_bit()?;
        let num_coupled = br.read_bits(3)? as usize;
        let mut num_gain = 0usize;

        for _ in 0..=num_coupled {
            num_gain += 1;
            let is_cpe = br.read_bit()? != 0;
            let _id_select = br.read_bits(4)?;
            if is_cpe {
                let ch_select = br.read_bits(2)? as u8;
                if ch_select == 3 {
                    num_gain += 1;
                }
            }
        }

        let add_flag = br.read_bit()?;
        if add_flag != 0 || (coupling_point >> 1) != 0 {
            coupling_point += 1;
        }
        let sign = br.read_bit()? as i32;
        let _gain_scale = br.read_bits(2)?;

        let mut cce_spectral = vec![0.0f32; 1024];
        let mut cce_band_info = IcsBandInfo::default();
        let cce_info = self.parse_ics(
            br,
            &mut cce_spectral,
            codebooks,
            Some(&mut cce_band_info),
            false,
        )?;

        for c in 0..num_gain {
            let mut idx = 0usize;
            let mut cge = true;

            if c > 0 {
                cge = coupling_point == 3 || br.read_bit()? != 0;
                if cge {
                    let _gain = codebooks.sf_tree.decode(br)? - 60;
                }
            }

            if coupling_point != 3 {
                for _group in 0..cce_info.num_window_groups {
                    for _sfb in 0..cce_info.max_sfb {
                        let band_type = cce_band_info.band_types.get(idx).copied().unwrap_or(0);
                        if band_type != 0 && !cge {
                            let delta = codebooks.sf_tree.decode(br)? - 60;
                            if delta != 0 && sign != 0 {
                                let _signed_delta = delta >> 1;
                            }
                        }
                        idx += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// 解析 ICS info
    fn parse_ics_info(&self, br: &mut BitReader) -> TaoResult<IcsInfo> {
        let _reserved = br.read_bit()?; // ics_reserved_bit
        let window_sequence = br.read_bits(2)?;
        let window_shape = br.read_bit()? as u8;

        if window_sequence == 2 {
            // EIGHT_SHORT_SEQUENCE
            let max_sfb = br.read_bits(4)? as usize;
            let scale_factor_grouping = br.read_bits(7)? as u8;
            let mut window_group_lengths = [0usize; 8];
            let mut window_group_starts = [0usize; 8];
            let mut num_window_groups = 1usize;
            window_group_lengths[0] = 1;
            window_group_starts[0] = 0;
            for i in 0..7 {
                if (scale_factor_grouping >> (6 - i)) & 1 != 0 {
                    window_group_lengths[num_window_groups - 1] += 1;
                } else {
                    let prev = num_window_groups - 1;
                    window_group_starts[num_window_groups] =
                        window_group_starts[prev] + window_group_lengths[prev];
                    window_group_lengths[num_window_groups] = 1;
                    num_window_groups += 1;
                }
            }
            let num_swb = self.swb_offset_short().len() - 1;
            if max_sfb > num_swb {
                return Err(TaoError::InvalidData(format!(
                    "AAC ICS info 非法: short max_sfb={} 超出 num_swb={}",
                    max_sfb, num_swb
                )));
            }
            Ok(IcsInfo {
                window_sequence,
                window_shape,
                max_sfb,
                num_swb,
                num_window_groups,
                window_group_lengths,
                window_group_starts,
            })
        } else {
            // ONLY_LONG / LONG_START / LONG_STOP
            let max_sfb = br.read_bits(6)? as usize;
            let predictor_present = br.read_bit()? != 0;
            if predictor_present {
                return Err(TaoError::InvalidData(
                    "AAC-LC 不允许 predictor_data_present=1".into(),
                ));
            }
            let num_swb = self.swb_offset().len() - 1;
            if max_sfb > num_swb {
                return Err(TaoError::InvalidData(format!(
                    "AAC ICS info 非法: long max_sfb={} 超出 num_swb={}",
                    max_sfb, num_swb
                )));
            }
            let mut window_group_lengths = [0usize; 8];
            let mut window_group_starts = [0usize; 8];
            window_group_lengths[0] = 1;
            window_group_starts[0] = 0;
            Ok(IcsInfo {
                window_sequence,
                window_shape,
                max_sfb,
                num_swb,
                num_window_groups: 1,
                window_group_lengths,
                window_group_starts,
            })
        }
    }

    /// 解析 individual_channel_stream (独立读取 ics_info)
    fn parse_ics(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        band_info: Option<&mut IcsBandInfo>,
        _common_window: bool,
    ) -> TaoResult<IcsInfo> {
        let global_gain = br.read_bits(8)? as i32;
        let info = self.parse_ics_info(br)?;
        self.decode_ics(br, spectral, codebooks, info, global_gain, band_info)?;
        Ok(info)
    }

    /// 解析 individual_channel_stream (使用共享 ics_info)
    fn parse_ics_with_info(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        band_info: Option<&mut IcsBandInfo>,
        info: IcsInfo,
    ) -> TaoResult<()> {
        let global_gain = br.read_bits(8)? as i32;
        self.decode_ics(br, spectral, codebooks, info, global_gain, band_info)
    }

    /// 解码 ICS 内容 (section + scalefactor + spectral)
    fn decode_ics(
        &self,
        br: &mut BitReader,
        spectral: &mut [f32],
        codebooks: &AacCodebooks,
        info: IcsInfo,
        global_gain: i32,
        mut band_info: Option<&mut IcsBandInfo>,
    ) -> TaoResult<()> {
        // 1. section_data
        let sections = parse_section_data(br, &info).map_err(|e| {
            TaoError::InvalidData(format!(
                "AAC section_data 解析失败: win_seq={}, max_sfb={}, 剩余位={}, 错误={}",
                info.window_sequence,
                info.max_sfb,
                br.bits_left(),
                e
            ))
        })?;
        // 2. scale_factor_data
        let scale_factors = parse_scale_factor_data(br, &sections, &info, global_gain, codebooks)
            .map_err(|e| {
            TaoError::InvalidData(format!(
                "AAC scale_factor_data 解析失败: win_seq={}, max_sfb={}, gain={}, 剩余位={}, 错误={}",
                info.window_sequence,
                info.max_sfb,
                global_gain,
                br.bits_left(),
                e
            ))
        })?;

        if let Some(meta) = band_info.as_mut() {
            meta.band_types = vec![0u8; info.num_window_groups * info.max_sfb];
            for section in &sections {
                let base = section.group * info.max_sfb;
                for sfb in section.sect_start..section.sect_end.min(info.max_sfb) {
                    meta.band_types[base + sfb] = section.sect_cb;
                }
            }
            meta.scale_factors = scale_factors.clone();
        }

        // 3. pulse_data_present
        let pulse_present = br.read_bit()? != 0;
        if pulse_present {
            skip_pulse_data(br).map_err(|e| {
                TaoError::InvalidData(format!(
                    "AAC pulse_data 解析失败: win_seq={}, 剩余位={}, 错误={}",
                    info.window_sequence,
                    br.bits_left(),
                    e
                ))
            })?;
        }

        // 4. tns_data_present
        let tns_present = br.read_bit()? != 0;
        let mut tns_data = None;
        if tns_present {
            tns_data = Some(parse_tns_data(br, info.window_sequence == 2).map_err(|e| {
                TaoError::InvalidData(format!(
                    "AAC tns_data 解析失败: short={}, 剩余位={}, 错误={}",
                    info.window_sequence == 2,
                    br.bits_left(),
                    e
                ))
            })?);
        }

        // 5. gain_control_data_present (AAC-LC: 始终为 0)
        let gain_control = br.read_bit().map_err(|e| {
            TaoError::InvalidData(format!(
                "AAC gain_control 标记读取失败: win_seq={}, 剩余位={}, 错误={}",
                info.window_sequence,
                br.bits_left(),
                e
            ))
        })?;
        if gain_control != 0 {
            skip_gain_control_data(br, info.window_sequence).map_err(|e| {
                TaoError::InvalidData(format!(
                    "AAC gain_control_data 解析失败: win_seq={}, 剩余位={}, 错误={}",
                    info.window_sequence,
                    br.bits_left(),
                    e
                ))
            })?;
        }

        // 6. spectral_data
        let swb_offset = if info.window_sequence == 2 {
            self.swb_offset_short()
        } else {
            self.swb_offset()
        };
        decode_spectral_data(
            br,
            spectral,
            &sections,
            &scale_factors,
            codebooks,
            &info,
            swb_offset,
            &self.random_state,
        )
        .map_err(|e| {
            TaoError::InvalidData(format!(
                "AAC spectral_data 解析失败: win_seq={}, max_sfb={}, 剩余位={}, 错误={}",
                info.window_sequence,
                info.max_sfb,
                br.bits_left(),
                e
            ))
        })?;

        if let Some(tns) = tns_data {
            apply_tns_data(
                spectral,
                &tns,
                &info,
                swb_offset,
                self.tns_max_bands(info.window_sequence == 2),
            );
        }
        Ok(())
    }

    /// 跳过 ADTS 头, 返回原始帧数据与是否存在 ADTS 头标记.
    fn strip_adts_header<'a>(&mut self, data: &'a [u8]) -> (&'a [u8], bool) {
        if data.len() >= 7 && data[0] == 0xFF && (data[1] & 0xF0) == 0xF0 {
            let sampling_frequency_index = ((data[2] >> 2) & 0x0F) as usize;
            let channel_config = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);
            if sampling_frequency_index < AAC_SAMPLE_RATES.len() {
                let sr = AAC_SAMPLE_RATES[sampling_frequency_index];
                if sr > 0 {
                    self.sample_rate = sr;
                    self.sample_rate_index = sampling_frequency_index as u8;
                }
            }
            if (1..=7).contains(&channel_config) {
                let channels = Self::channels_from_config(channel_config);
                if channels != self.channels {
                    self.channels = channels;
                    self.channel_layout = ChannelLayout::from_channels(channels);
                    self.overlap = vec![vec![0.0f32; 1024]; channels as usize];
                    self.first_frame = true;
                }
                self.channel_config = channel_config;
            }
            let protection_absent = (data[1] & 0x01) != 0;
            let header_size = if protection_absent { 7 } else { 9 };
            if data.len() > header_size {
                return (&data[header_size..], true);
            }
            return (&[], true);
        }
        (data, false)
    }
}

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
        self.default_leading_trim_samples = if params.extra_data.is_empty() {
            0
        } else {
            AAC_MP4_LEADING_TRIM_SAMPLES
        };
        self.pending_leading_trim_samples = self.default_leading_trim_samples;
        // 构建 Huffman 码本
        self.codebooks = Some(AacCodebooks::build());
        self.overlap = vec![vec![0.0f32; 1024]; self.channels as usize];
        self.prev_window_shape = vec![0u8; self.channels as usize];
        self.long_sine_window = build_sine_window(2048);
        self.long_kbd_window = build_kbd_window(2048, 4.0);
        self.short_sine_window = build_sine_window(256);
        self.short_kbd_window = build_kbd_window(256, 6.0);
        self.random_state.set(0x1f2e3d4c);
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

        let (raw_data, has_adts_header) = self.strip_adts_header(&packet.data);

        // 解码, 失败时输出静音
        let pcm = match self.decode_raw_frame(raw_data) {
            Ok(pcm) => pcm,
            Err(e) => {
                info!("AAC 帧解码失败: {}, 输出静音", e);
                vec![vec![0.0f32; 1024]; self.channels as usize]
            }
        };

        let channels = self.channels as usize;
        let channel_map = self.output_channel_map();
        let num_samples = 1024;
        let mut interleaved = vec![0u8; num_samples * channels * 4];

        for i in 0..num_samples {
            for ch in 0..channels {
                let src_ch = channel_map
                    .and_then(|map| map.get(ch))
                    .copied()
                    .unwrap_or(ch);
                let sample = if src_ch < pcm.len() {
                    // F32 输出不做 [-1,1] 强制削顶, 仅对异常值做保护, 避免与参考实现产生系统性截断误差.
                    let scaled = pcm[src_ch][i] * AAC_OUTPUT_GAIN;
                    if scaled.is_finite() {
                        scaled.clamp(-8.0, 8.0)
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let bytes = sample.to_le_bytes();
                let offset = (i * channels + ch) * 4;
                interleaved[offset..offset + 4].copy_from_slice(&bytes);
            }
        }

        let mut leading_trim_samples = 0usize;
        if !has_adts_header && self.pending_leading_trim_samples > 0 {
            leading_trim_samples = self.pending_leading_trim_samples.min(num_samples);
            self.pending_leading_trim_samples -= leading_trim_samples;
        }
        let output_samples = num_samples - leading_trim_samples;
        if output_samples == 0 {
            self.output_frame = None;
            return Ok(());
        }
        let payload_offset = leading_trim_samples * channels * 4;
        let output_interleaved = if payload_offset == 0 {
            interleaved
        } else {
            interleaved[payload_offset..].to_vec()
        };
        let output_pts = if packet.pts == tao_core::timestamp::NOPTS_VALUE {
            packet.pts
        } else {
            packet.pts.saturating_add(leading_trim_samples as i64)
        };

        let frame = AudioFrame {
            data: vec![output_interleaved],
            nb_samples: output_samples as u32,
            sample_rate: self.sample_rate,
            channel_layout: self.channel_layout,
            sample_format: SampleFormat::F32,
            pts: output_pts,
            time_base: tao_core::Rational::new(1, self.sample_rate as i32),
            duration: output_samples as i64,
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
        self.pending_leading_trim_samples = self.default_leading_trim_samples;
        self.prev_window_shape.fill(0);
        self.random_state.set(0x1f2e3d4c);
        for ch in &mut self.overlap {
            ch.fill(0.0);
        }
    }
}
