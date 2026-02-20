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

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use log::{debug, info};
use tao_core::bitreader::BitReader;
use tao_core::{ChannelLayout, SampleFormat, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

use huffman::AacCodebooks;

static AAC_TRACE_ENABLED: OnceLock<bool> = OnceLock::new();
static AAC_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const AAC_TRACE_LIMIT: usize = 160;

fn aac_trace_enabled() -> bool {
    *AAC_TRACE_ENABLED.get_or_init(|| {
        std::env::var("TAO_AAC_TRACE")
            .map(|v| {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false)
    })
}

fn aac_trace_log(message: impl FnOnce() -> String) {
    if !aac_trace_enabled() {
        return;
    }
    let idx = AAC_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    if idx < AAC_TRACE_LIMIT {
        info!("AAC追踪[{}]: {}", idx + 1, message());
    }
}

/// MP4 封装 AAC 常见首包前导裁剪样本数.
const AAC_MP4_LEADING_TRIM_SAMPLES: usize = 1024;
/// AAC 时域输出增益校准.
///
/// AAC 频谱反量化后需经过较大缩放, 该系数用于把时域样本归一到 [-1, 1].
/// 过大将导致大量削顶并破坏与参考实现的一致性.
const AAC_OUTPUT_GAIN: f32 = 0.0004882132;

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

/// 44100Hz 下 128 点 SHORT 窗口的 SFB 边界.
///
/// AAC 采样率索引 4(44.1kHz)与索引 3(48kHz)共用同一套 short SFB 表.
const SWB_OFFSET_128_44100: [usize; 15] =
    [0, 4, 8, 12, 16, 20, 28, 36, 44, 56, 68, 80, 96, 112, 128];

/// 48000Hz 下 128 点 SHORT 窗口的 SFB 边界.
const SWB_OFFSET_128_48000: [usize; 15] =
    [0, 4, 8, 12, 16, 20, 28, 36, 44, 56, 68, 80, 96, 112, 128];

/// AAC TNS 最大频带数表 (索引为采样率索引).
const TNS_MAX_BANDS_1024: [u8; 13] = [31, 31, 34, 40, 42, 51, 46, 46, 42, 42, 42, 39, 39];
const TNS_MAX_BANDS_128: [u8; 13] = [9, 9, 10, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14];

/// AAC TNS 系数量化反查表 (与 FFmpeg `ff_tns_tmp2_map` 一致).
const TNS_TMP2_MAP_1_3: [f32; 4] = [0.0, -0.433_883_73, 0.642_787_6, 0.342_020_15];
const TNS_TMP2_MAP_0_3: [f32; 8] = [
    0.0,
    -0.433_883_73,
    -0.781_831_5,
    -0.974_927_9,
    0.984_807_7,
    0.866_025_4,
    0.642_787_6,
    0.342_020_15,
];
const TNS_TMP2_MAP_1_4: [f32; 8] = [
    0.0,
    -0.207_911_7,
    -0.406_736_64,
    -0.587_785_24,
    0.673_695_6,
    0.526_432_16,
    0.361_241_67,
    0.183_749_51,
];
const TNS_TMP2_MAP_0_4: [f32; 16] = [
    0.0,
    -0.207_911_7,
    -0.406_736_64,
    -0.587_785_24,
    -0.743_144_8,
    -0.866_025_4,
    -0.951_056_54,
    -0.994_521_9,
    0.995_734_16,
    0.961_825_6,
    0.895_163_3,
    0.798_017_2,
    0.673_695_6,
    0.526_432_16,
    0.361_241_67,
    0.183_749_51,
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

    /// 获取当前采样率对应的 SHORT 窗口 SFB 边界表
    fn swb_offset_short(&self) -> &'static [usize] {
        match self.sample_rate {
            48000 => &SWB_OFFSET_128_48000,
            _ => &SWB_OFFSET_128_44100,
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
        &self,
        data: &[u8],
        spectral: &mut [Vec<f32>],
        window_sequences: &mut [u32],
        window_shapes: &mut [u8],
    ) -> TaoResult<()> {
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
            let bits_before_ele = br.bits_left();
            let id_syn_ele = br.read_bits(3)?;
            aac_trace_log(|| {
                format!(
                    "raw_data_block 元素: id_syn_ele={}, 解析前剩余位={}, 已解码声道={}",
                    id_syn_ele, bits_before_ele, ch_idx
                )
            });
            if id_syn_ele == 7 {
                break; // END
            }
            match id_syn_ele {
                0 => {
                    // SCE: Single Channel Element
                    let _instance_tag = br.read_bits(4)?;
                    if ch_idx < spectral.len() {
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
                    aac_trace_log(|| {
                        format!(
                            "CPE: common_window={}, 读取后剩余位={}, 当前声道起点={}",
                            common_window,
                            br.bits_left(),
                            ch_idx
                        )
                    });

                    if common_window {
                        let info = self.parse_ics_info(&mut br)?;
                        // ms_mask_present (2 bits)
                        let ms_mask_present = br.read_bits(2)?;
                        aac_trace_log(|| {
                            format!(
                                "CPE: shared_ics win_seq={}, max_sfb={}, groups={}, ms_mask_present={}, bits_left={}",
                                info.window_sequence,
                                info.max_sfb,
                                info.num_window_groups,
                                ms_mask_present,
                                br.bits_left()
                            )
                        });
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
                    if let Err(e) = self.skip_cce(&mut br, codebooks) {
                        debug!("AAC CCE 解析失败, 忽略剩余元素: {}", e);
                        break;
                    }
                }
                3 => {
                    // LFE: 和 SCE 结构相同
                    let _instance_tag = br.read_bits(4)?;
                    if ch_idx < spectral.len() {
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
                    if let Err(e) = self.skip_pce(&mut br) {
                        debug!("AAC PCE 解析失败, 忽略剩余元素: {}", e);
                        break;
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
                _ => {
                    return Err(TaoError::Unsupported(format!(
                        "AAC: 未实现语法元素 id_syn_ele={id_syn_ele}"
                    )));
                }
            }
            aac_trace_log(|| {
                format!(
                    "raw_data_block 元素结束: id_syn_ele={}, 解析后剩余位={}, 已解码声道={}",
                    id_syn_ele,
                    br.bits_left(),
                    ch_idx
                )
            });
        }
        Ok(())
    }

    /// 跳过 Program Config Element (PCE).
    fn skip_pce(&self, br: &mut BitReader) -> TaoResult<()> {
        let _element_instance_tag = br.read_bits(4)?;
        let _object_type = br.read_bits(2)?;
        let _sampling_frequency_index = br.read_bits(4)?;
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

        for _ in 0..num_front {
            let _is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
        }
        for _ in 0..num_side {
            let _is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
        }
        for _ in 0..num_back {
            let _is_cpe = br.read_bit()?;
            let _tag_select = br.read_bits(4)?;
        }
        for _ in 0..num_lfe {
            let _tag_select = br.read_bits(4)?;
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
        debug!(
            "AAC ICS: win={}, max_sfb={}, gain={}, bits_left={}",
            info.window_sequence,
            info.max_sfb,
            global_gain,
            br.bits_left()
        );

        // 1. section_data
        let bits_before_section = br.bits_left();
        let sections = parse_section_data(br, &info).map_err(|e| {
            TaoError::InvalidData(format!(
                "AAC section_data 解析失败: win_seq={}, max_sfb={}, 剩余位={}, 错误={}",
                info.window_sequence,
                info.max_sfb,
                br.bits_left(),
                e
            ))
        })?;
        aac_trace_log(|| {
            let summary = sections
                .iter()
                .map(|s| {
                    format!(
                        "g{}:cb{}[{}..{}]",
                        s.group, s.sect_cb, s.sect_start, s.sect_end
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "ICS section完成: win_seq={}, max_sfb={}, section_bits={}->{}, sections={}",
                info.window_sequence,
                info.max_sfb,
                bits_before_section,
                br.bits_left(),
                summary
            )
        });
        {
            let cbs: Vec<_> = sections
                .iter()
                .map(|s| {
                    format!(
                        "g{}:cb{}[{}..{}]",
                        s.group, s.sect_cb, s.sect_start, s.sect_end
                    )
                })
                .collect();
            debug!(
                "AAC section_data: {} 个段 {:?}, bits_left={}",
                sections.len(),
                cbs,
                br.bits_left()
            );
        }

        // 2. scale_factor_data
        let bits_before_sf = br.bits_left();
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
        aac_trace_log(|| {
            format!(
                "ICS sf完成: win_seq={}, max_sfb={}, sf_bits={}->{}, sf_slots={}",
                info.window_sequence,
                info.max_sfb,
                bits_before_sf,
                br.bits_left(),
                scale_factors.len()
            )
        });
        debug!("AAC sf_data: bits_left={}", br.bits_left());

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
        aac_trace_log(|| {
            format!(
                "ICS pulse完成: pulse_present={}, 剩余位={}",
                pulse_present,
                br.bits_left()
            )
        });

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
        aac_trace_log(|| {
            format!(
                "ICS tns完成: tns_present={}, short={}, 剩余位={}",
                tns_present,
                info.window_sequence == 2,
                br.bits_left()
            )
        });

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
        aac_trace_log(|| {
            format!(
                "ICS gain完成: gain_control={}, 剩余位={}",
                gain_control,
                br.bits_left()
            )
        });

        debug!(
            "AAC 频谱解码前: bits_left={}, pulse={}, tns={}",
            br.bits_left(),
            pulse_present,
            tns_present
        );

        // 6. spectral_data
        let swb_offset = if info.window_sequence == 2 {
            self.swb_offset_short()
        } else {
            self.swb_offset()
        };
        let bits_before_spectral = br.bits_left();
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
        aac_trace_log(|| {
            format!(
                "ICS spectral完成: win_seq={}, max_sfb={}, spectral_bits={}->{}, 消耗位={}",
                info.window_sequence,
                info.max_sfb,
                bits_before_spectral,
                br.bits_left(),
                bits_before_spectral.saturating_sub(br.bits_left())
            )
        });

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
    fn strip_adts_header<'a>(&self, data: &'a [u8]) -> (&'a [u8], bool) {
        if data.len() >= 7 && data[0] == 0xFF && (data[1] & 0xF0) == 0xF0 {
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

// ============================================================
// ICS 解析辅助
// ============================================================

#[derive(Clone, Copy)]
struct IcsInfo {
    window_sequence: u32,
    window_shape: u8,
    max_sfb: usize,
    num_swb: usize,
    num_window_groups: usize,
    window_group_lengths: [usize; 8],
    window_group_starts: [usize; 8],
}

#[derive(Default, Clone)]
struct IcsBandInfo {
    band_types: Vec<u8>,
    scale_factors: Vec<i32>,
}

struct Section {
    group: usize,
    sect_cb: u8,
    sect_start: usize,
    sect_end: usize,
}

#[derive(Clone)]
struct TnsData {
    num_windows: usize,
    n_filt: [u8; 8],
    length: [[u8; 4]; 8],
    order: [[u8; 4]; 8],
    direction: [[bool; 4]; 8],
    coef: [[[f32; 20]; 4]; 8],
}

impl Default for TnsData {
    fn default() -> Self {
        Self {
            num_windows: 0,
            n_filt: [0; 8],
            length: [[0; 4]; 8],
            order: [[0; 4]; 8],
            direction: [[false; 4]; 8],
            coef: [[[0.0; 20]; 4]; 8],
        }
    }
}

/// 解析 section_data
fn parse_section_data(br: &mut BitReader, info: &IcsInfo) -> TaoResult<Vec<Section>> {
    let mut sections = Vec::new();
    let is_short = info.window_sequence == 2;
    let sect_bits = if is_short { 3 } else { 5 };
    let sect_esc = if is_short { 7 } else { 31 };

    for group in 0..info.num_window_groups {
        let mut k = 0usize;
        while k < info.max_sfb {
            let sect_cb = br.read_bits(4)? as u8;
            if sect_cb == 12 {
                return Err(TaoError::InvalidData(format!(
                    "AAC section_data 非法: group={}, sfb={}, 遇到无效 codebook=12",
                    group, k
                )));
            }
            let mut sect_end = k;
            loop {
                let incr = br.read_bits(sect_bits)? as usize;
                sect_end = sect_end.checked_add(incr).ok_or_else(|| {
                    TaoError::InvalidData(format!(
                        "AAC section_data 非法: group={}, sfb={}, section 长度溢出",
                        group, k
                    ))
                })?;
                if sect_end > info.max_sfb {
                    return Err(TaoError::InvalidData(format!(
                        "AAC section_data 非法: group={}, sfb={}, section_end={} 超出 max_sfb={}",
                        group, k, sect_end, info.max_sfb
                    )));
                }
                if incr != sect_esc {
                    break;
                }
            }
            if sect_end == k {
                return Err(TaoError::InvalidData(format!(
                    "AAC section_data 非法: group={}, sfb={}, section 长度为 0",
                    group, k
                )));
            }
            sections.push(Section {
                group,
                sect_cb,
                sect_start: k,
                sect_end,
            });
            k = sect_end;
        }
    }
    Ok(sections)
}

/// 噪声偏移 (ISO 14496-3)
const NOISE_OFFSET: i32 = 90;
/// 第一个噪声频带的 9bit 预偏移.
const NOISE_PRE: i32 = 256;

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
    info: &IcsInfo,
    global_gain: i32,
    codebooks: &AacCodebooks,
) -> TaoResult<Vec<i32>> {
    let mut scale_factors = vec![0i32; info.num_window_groups * info.num_swb];
    let mut sf = global_gain;
    let mut is_position = 0i32;
    let mut noise_energy = global_gain - NOISE_OFFSET;
    let mut noise_pcm_flag = true; // 第一个噪声频带标志

    for section in sections {
        let cb = section.sect_cb;
        let end = section.sect_end.min(info.num_swb);
        let group_base = section.group * info.num_swb;
        for sfb in section.sect_start..end {
            let sf_slot = &mut scale_factors[group_base + sfb];
            if cb == 0 {
                // ZERO_HCB: 无 scale factor
                *sf_slot = 0;
            } else if cb == NOISE_HCB {
                // NOISE_HCB (13): 噪声能量
                if noise_pcm_flag {
                    // 第一个噪声频带: 读取 9 位原始值
                    noise_pcm_flag = false;
                    let raw = br.read_bits(9)? as i32;
                    noise_energy = global_gain - NOISE_OFFSET + raw - NOISE_PRE;
                } else {
                    let delta = codebooks.sf_tree.decode(br)? - 60;
                    noise_energy += delta;
                }
                noise_energy = noise_energy.clamp(-100, 155);
                *sf_slot = noise_energy;
            } else if cb == INTENSITY_HCB || cb == INTENSITY_HCB2 {
                // INTENSITY_HCB (15) / INTENSITY_HCB2 (14): IS position
                let delta = codebooks.sf_tree.decode(br)? - 60;
                is_position += delta;
                is_position = is_position.clamp(-155, 100);
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
#[allow(clippy::too_many_arguments)]
fn decode_spectral_data(
    br: &mut BitReader,
    spectral: &mut [f32],
    sections: &[Section],
    scale_factors: &[i32],
    codebooks: &AacCodebooks,
    info: &IcsInfo,
    swb_offset: &[usize],
    random_state: &Cell<u32>,
) -> TaoResult<()> {
    let is_short = info.window_sequence == 2;
    for section in sections {
        let cb = section.sect_cb;
        if cb == 0 || cb == INTENSITY_HCB2 || cb == INTENSITY_HCB {
            // ZERO_HCB / INTENSITY: 频谱为 0
            continue;
        }

        for sfb in section.sect_start..section.sect_end {
            let start_idx = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end_idx = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            let sf = scale_factors
                .get(section.group * info.num_swb + sfb)
                .copied()
                .unwrap_or(0);
            let band_width = end_idx.saturating_sub(start_idx);
            let window_group_len = info.window_group_lengths[section.group];
            let group_start = info.window_group_starts[section.group];
            if cb == NOISE_HCB {
                // PNS 噪声重建: 与 FFmpeg 相同的 LCG 随机源 + 频带归一化.
                let target = inverse_quantize(1, sf).abs();
                for win_in_group in 0..window_group_len {
                    let win = group_start + win_in_group;
                    let win_base = win * 128 + start_idx;
                    let mut energy = 0.0f32;
                    for i in 0..band_width {
                        let idx = win_base + i;
                        if idx >= spectral.len() {
                            continue;
                        }
                        let noise = lcg_random(random_state);
                        spectral[idx] = noise;
                        energy += noise * noise;
                    }
                    if energy > 0.0 {
                        let scale = target / energy.sqrt();
                        for i in 0..band_width {
                            let idx = win_base + i;
                            if idx < spectral.len() {
                                spectral[idx] *= scale;
                            }
                        }
                    }
                }
                continue;
            }

            if (1..=11).contains(&cb) {
                let cb_idx = (cb - 1) as usize;
                let spec_cb = codebooks.spectral[cb_idx]
                    .as_ref()
                    .ok_or_else(|| TaoError::Unsupported(format!("AAC: 频谱码本 {cb} 未实现")))?;
                if is_short {
                    // short 窗口必须按每个窗单独解码, 不能把组内多个窗拼接后一次解码.
                    for win_in_group in 0..window_group_len {
                        let win = group_start + win_in_group;
                        let win_base = win * 128 + start_idx;
                        let mut i = 0usize;
                        while i < band_width {
                            let values = match spec_cb.decode_values(br) {
                                Ok(v) => v,
                                Err(e) => {
                                    debug!(
                                        "AAC 频谱解码错误: short sfb={}, cb={}, win={}, i={}/{}, bits_left={}",
                                        sfb,
                                        cb,
                                        win,
                                        i,
                                        band_width,
                                        br.bits_left()
                                    );
                                    return Err(TaoError::InvalidData(format!(
                                        "频谱码字解码失败: short sfb={}, cb={}, win={}, i={}/{}, bits_left={}, 错误={}",
                                        sfb,
                                        cb,
                                        win,
                                        i,
                                        band_width,
                                        br.bits_left(),
                                        e
                                    )));
                                }
                            };
                            let count = spec_cb.dim.min(band_width - i);
                            for (j, &v) in values.iter().enumerate().take(count) {
                                let idx = win_base + i + j;
                                if idx < spectral.len() {
                                    spectral[idx] = inverse_quantize(v, sf);
                                }
                            }
                            i += spec_cb.dim;
                        }
                    }
                } else {
                    // long 窗口按单个频带连续解码.
                    let mut i = 0usize;
                    while i < band_width {
                        let values = match spec_cb.decode_values(br) {
                            Ok(v) => v,
                            Err(e) => {
                                debug!(
                                    "AAC 频谱解码错误: sfb={}, cb={}, i={}/{}, bits_left={}",
                                    sfb,
                                    cb,
                                    i,
                                    band_width,
                                    br.bits_left()
                                );
                                return Err(TaoError::InvalidData(format!(
                                    "频谱码字解码失败: sfb={}, cb={}, i={}/{}, bits_left={}, 错误={}",
                                    sfb,
                                    cb,
                                    i,
                                    band_width,
                                    br.bits_left(),
                                    e
                                )));
                            }
                        };
                        let count = spec_cb.dim.min(band_width - i);
                        for (j, &v) in values.iter().enumerate().take(count) {
                            let idx = start_idx + i + j;
                            if idx < spectral.len() {
                                spectral[idx] = inverse_quantize(v, sf);
                            }
                        }
                        i += spec_cb.dim;
                    }
                }
            }
        }
    }
    Ok(())
}

fn lcg_random(state: &Cell<u32>) -> f32 {
    let next = state
        .get()
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    state.set(next);
    (next as i32) as f32
}

/// 对 CPE 频谱应用 MS 立体声反变换
fn apply_ms_stereo(
    left: &mut [f32],
    right: &mut [f32],
    info: &IcsInfo,
    ms_used: &[bool],
    left_band_types: Option<&[u8]>,
    right_band_types: Option<&[u8]>,
    swb_offset: &[usize],
) {
    let is_short = info.window_sequence == 2;
    for group in 0..info.num_window_groups {
        for sfb in 0..info.max_sfb {
            let mask_idx = group * info.max_sfb + sfb;
            if !ms_used.get(mask_idx).copied().unwrap_or(false) {
                continue;
            }
            if let (Some(left_types), Some(right_types)) = (left_band_types, right_band_types) {
                let left_bt = left_types.get(mask_idx).copied().unwrap_or(0);
                let right_bt = right_types.get(mask_idx).copied().unwrap_or(0);
                if left_bt >= NOISE_HCB || right_bt >= NOISE_HCB {
                    continue;
                }
            }
            let start = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            if end <= start {
                continue;
            }
            if is_short {
                let group_len = info.window_group_lengths[group];
                let group_start = info.window_group_starts[group];
                for win in 0..group_len {
                    let win_base = (group_start + win) * 128;
                    for line in start..end {
                        let idx = win_base + line;
                        if idx >= left.len() || idx >= right.len() {
                            continue;
                        }
                        let l = left[idx];
                        let r = right[idx];
                        left[idx] = l + r;
                        right[idx] = l - r;
                    }
                }
            } else {
                for idx in start..end {
                    if idx >= left.len() || idx >= right.len() {
                        continue;
                    }
                    let l = left[idx];
                    let r = right[idx];
                    left[idx] = l + r;
                    right[idx] = l - r;
                }
            }
        }
    }
}

/// 对 CPE 频谱应用强度立体声 (IS) 重建.
fn apply_intensity_stereo(
    left: &mut [f32],
    right: &mut [f32],
    info: &IcsInfo,
    right_band_types: &[u8],
    right_scale_factors: &[i32],
    ms_used: Option<&[bool]>,
    swb_offset: &[usize],
) {
    let is_short = info.window_sequence == 2;
    for group in 0..info.num_window_groups {
        for sfb in 0..info.max_sfb {
            let band_idx = group * info.max_sfb + sfb;
            let sf_idx = group * info.num_swb + sfb;
            let band_type = right_band_types.get(band_idx).copied().unwrap_or(0);
            if band_type != INTENSITY_HCB && band_type != INTENSITY_HCB2 {
                continue;
            }

            // intensity_position 对应 scalefactor 值.
            let is_position = right_scale_factors.get(sf_idx).copied().unwrap_or(0) as f32;
            let mut sign = if band_type == INTENSITY_HCB2 {
                -1.0f32
            } else {
                1.0f32
            };
            if ms_used
                .and_then(|mask| mask.get(band_idx))
                .copied()
                .unwrap_or(false)
            {
                sign = -sign;
            }
            let scale = sign * 0.5f32.powf(0.25 * is_position);

            let start = swb_offset[sfb.min(swb_offset.len() - 1)];
            let end = swb_offset[(sfb + 1).min(swb_offset.len() - 1)];
            if end <= start {
                continue;
            }

            if is_short {
                let group_len = info.window_group_lengths[group];
                let group_start = info.window_group_starts[group];
                for win in 0..group_len {
                    let win_base = (group_start + win) * 128;
                    for line in start..end {
                        let idx = win_base + line;
                        if idx >= left.len() || idx >= right.len() {
                            continue;
                        }
                        right[idx] = left[idx] * scale;
                    }
                }
            } else {
                for idx in start..end {
                    if idx >= left.len() || idx >= right.len() {
                        continue;
                    }
                    right[idx] = left[idx] * scale;
                }
            }
        }
    }
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

/// 解析 tns_data.
fn parse_tns_data(br: &mut BitReader, is_short: bool) -> TaoResult<TnsData> {
    let mut data = TnsData {
        num_windows: if is_short { 8 } else { 1 },
        ..Default::default()
    };
    let max_order = if is_short { 7u32 } else { 12u32 };

    for w in 0..data.num_windows {
        let n_filt = br.read_bits(if is_short { 1 } else { 2 })? as usize;
        if n_filt > 4 {
            return Err(TaoError::InvalidData(format!(
                "AAC TNS 非法: window={}, n_filt={} 超出上限",
                w, n_filt
            )));
        }
        data.n_filt[w] = n_filt as u8;
        if n_filt == 0 {
            continue;
        }

        let coef_res = br.read_bit()? as usize;
        for filt in 0..n_filt {
            data.length[w][filt] = br.read_bits(if is_short { 4 } else { 6 })? as u8;
            let order = br.read_bits(if is_short { 3 } else { 5 })?;
            if order > max_order {
                return Err(TaoError::InvalidData(format!(
                    "AAC TNS 非法: window={}, filter={}, order={} 超出上限={}",
                    w, filt, order, max_order
                )));
            }
            data.order[w][filt] = order as u8;
            if order == 0 {
                continue;
            }

            data.direction[w][filt] = br.read_bit()? != 0;
            let coef_compress = br.read_bit()? as usize;
            let coef_len = coef_res + 3 - coef_compress;
            let map_idx = 2 * coef_compress + coef_res;
            for i in 0..order as usize {
                let q = br.read_bits(coef_len as u32)? as usize;
                data.coef[w][filt][i] = tns_coef_from_index(map_idx, q)?;
            }
        }
    }
    Ok(data)
}

/// 从 TNS 量化索引恢复滤波系数.
fn tns_coef_from_index(map_idx: usize, q: usize) -> TaoResult<f32> {
    let val = match map_idx {
        0 => TNS_TMP2_MAP_0_3.get(q).copied(),
        1 => TNS_TMP2_MAP_0_4.get(q).copied(),
        2 => TNS_TMP2_MAP_1_3.get(q).copied(),
        3 => TNS_TMP2_MAP_1_4.get(q).copied(),
        _ => None,
    };
    val.ok_or_else(|| {
        TaoError::InvalidData(format!(
            "AAC TNS 系数索引越界: map_idx={}, q={}",
            map_idx, q
        ))
    })
}

/// 将 TNS 反射系数转换为 LPC 系数.
fn compute_tns_lpc(coefs: &[f32]) -> [f32; 20] {
    let mut lpc = [0.0f32; 20];
    for i in 0..coefs.len() {
        let r = -coefs[i];
        lpc[i] = r;
        for j in 0..((i + 1) >> 1) {
            let f = lpc[j];
            let b = lpc[i - 1 - j];
            lpc[j] = f + r * b;
            lpc[i - 1 - j] = b + r * f;
        }
    }
    lpc
}

/// 在频域上应用 TNS all-pole 滤波.
fn apply_tns_data(
    spectral: &mut [f32],
    tns: &TnsData,
    info: &IcsInfo,
    swb_offset: &[usize],
    tns_max_bands: usize,
) {
    let mmm = tns_max_bands.min(info.max_sfb);
    if mmm == 0 || tns.num_windows == 0 {
        return;
    }

    for w in 0..tns.num_windows {
        let mut bottom = info.num_swb;
        for filt in 0..tns.n_filt[w] as usize {
            let top = bottom;
            bottom = top.saturating_sub(tns.length[w][filt] as usize);
            let order = tns.order[w][filt] as usize;
            if order == 0 {
                continue;
            }

            let start_band = bottom.min(mmm);
            let end_band = top.min(mmm);
            let start = swb_offset[start_band.min(swb_offset.len() - 1)];
            let end = swb_offset[end_band.min(swb_offset.len() - 1)];
            let size = end.saturating_sub(start);
            if size == 0 {
                continue;
            }

            let lpc = compute_tns_lpc(&tns.coef[w][filt][..order]);
            let mut pos = if tns.direction[w][filt] {
                (w * 128 + end.saturating_sub(1)) as isize
            } else {
                (w * 128 + start) as isize
            };
            let inc = if tns.direction[w][filt] {
                -1isize
            } else {
                1isize
            };

            for m in 0..size {
                let idx = pos as usize;
                if idx >= spectral.len() {
                    break;
                }
                let mut acc = spectral[idx];
                let tap = m.min(order);
                for i in 1..=tap {
                    let src = (pos - (i as isize) * inc) as usize;
                    if src >= spectral.len() {
                        continue;
                    }
                    acc -= spectral[src] * lpc[i - 1];
                }
                spectral[idx] = acc;
                pos += inc;
            }
        }
    }
}

/// 跳过 gain_control_data (ISO 14496-3 Table 4.55).
fn skip_gain_control_data(br: &mut BitReader, window_sequence: u32) -> TaoResult<()> {
    // [wd_num, wd_test, aloc_size]
    const GAIN_MODE: [[u8; 3]; 4] = [
        [1, 0, 5], // ONLY_LONG_SEQUENCE
        [2, 1, 2], // LONG_START_SEQUENCE
        [8, 0, 2], // EIGHT_SHORT_SEQUENCE
        [2, 1, 5], // LONG_STOP_SEQUENCE
    ];

    let mode = window_sequence as usize;
    if mode >= GAIN_MODE.len() {
        return Err(TaoError::InvalidData(format!(
            "AAC gain_control_data: 无效窗口序列 {}",
            window_sequence
        )));
    }
    let max_band = br.read_bits(2)? as usize;
    for _band in 0..max_band {
        for wd in 0..GAIN_MODE[mode][0] as usize {
            let adjust_num = br.read_bits(3)? as usize;
            for _ in 0..adjust_num {
                let aloc_size = if wd == 0 && GAIN_MODE[mode][1] != 0 {
                    4u32
                } else {
                    GAIN_MODE[mode][2] as u32
                };
                br.skip_bits(4 + aloc_size)?;
            }
        }
    }
    Ok(())
}

/// 反量化: iq = sign(x) * |x|^(4/3) * 2^(0.25 * (sf - 120))
fn inverse_quantize(x: i32, sf: i32) -> f32 {
    if x == 0 {
        return 0.0;
    }
    let sign = if x > 0 { 1.0f32 } else { -1.0f32 };
    let abs_x = x.unsigned_abs() as f32;
    let pow_val = abs_x.powf(4.0 / 3.0);
    let scale = 2.0f32.powf(0.25 * (sf - 120) as f32);
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
        let num_samples = 1024;
        let mut interleaved = vec![0u8; num_samples * channels * 4];

        for i in 0..num_samples {
            for ch in 0..channels {
                let sample = if ch < pcm.len() {
                    (pcm[ch][i] * AAC_OUTPUT_GAIN).clamp(-1.0, 1.0)
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

// ============================================================
// IMDCT + 窗函数
// ============================================================

/// 通用 IMDCT (输入 N 个频谱系数, 输出 2N 个时域样本)
fn imdct(spectrum: &[f32]) -> Vec<f32> {
    let n = spectrum.len();
    let n2 = 2 * n;
    let mut output = vec![0.0f32; n2];

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

/// 1024 点 IMDCT (输入 1024 频谱系数, 输出 2048 时域样本)
fn imdct_1024(spectrum: &[f32]) -> Vec<f32> {
    imdct(spectrum)
}

/// 128 点 IMDCT (输入 128 频谱系数, 输出 256 时域样本)
fn imdct_128(spectrum: &[f32]) -> Vec<f32> {
    imdct(spectrum)
}

/// 合成 EIGHT_SHORT_SEQUENCE 的 2048 点窗后信号
fn synthesize_short_windows(
    spectrum: &[f32],
    window_shape: u8,
    short_sine_window: &[f32],
    short_kbd_window: &[f32],
) -> Vec<f32> {
    let mut output = vec![0.0f32; 2048];
    let short_window = pick_window(window_shape, short_sine_window, short_kbd_window);
    for win in 0..8 {
        let begin = win * 128;
        let end = begin + 128;
        if end > spectrum.len() {
            break;
        }
        let td = imdct_128(&spectrum[begin..end]);
        let write_start = 448 + win * 128;
        for (i, &sample) in td.iter().enumerate() {
            let idx = write_start + i;
            if idx < output.len() {
                output[idx] += sample * short_window[i];
            }
        }
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

struct AacWindowBank<'a> {
    long_sine: &'a [f32],
    long_kbd: &'a [f32],
    short_sine: &'a [f32],
    short_kbd: &'a [f32],
}

/// AAC 长块窗口函数 (ONLY_LONG/LONG_START/LONG_STOP)
fn apply_aac_long_window(
    time_domain: &[f32],
    window_sequence: u32,
    prev_window_shape: u8,
    curr_window_shape: u8,
    windows: &AacWindowBank<'_>,
) -> Vec<f32> {
    let n = time_domain.len();
    if n != 2048 {
        return apply_sine_window(time_domain);
    }
    let mut windowed = vec![0.0f32; n];
    let long_prev = pick_window(prev_window_shape, windows.long_sine, windows.long_kbd);
    let long_curr = pick_window(curr_window_shape, windows.long_sine, windows.long_kbd);
    let short_prev = pick_window(prev_window_shape, windows.short_sine, windows.short_kbd);
    let short_curr = pick_window(curr_window_shape, windows.short_sine, windows.short_kbd);

    for i in 0..n {
        let w = match window_sequence {
            1 => {
                // LONG_START_SEQUENCE
                if i < 1024 {
                    long_prev[i]
                } else if i < 1472 {
                    1.0
                } else if i < 1600 {
                    // 与尾部零区衔接, 需使用短窗后半段 (从 1 递减到 0).
                    short_curr[128 + (i - 1472)]
                } else {
                    0.0
                }
            }
            3 => {
                // LONG_STOP_SEQUENCE
                if i < 448 {
                    0.0
                } else if i < 576 {
                    short_prev[i - 448]
                } else if i < 1024 {
                    1.0
                } else {
                    long_curr[i]
                }
            }
            _ => {
                // ONLY_LONG_SEQUENCE
                if i < 1024 { long_prev[i] } else { long_curr[i] }
            }
        };
        windowed[i] = time_domain[i] * w;
    }
    windowed
}

/// 根据窗口形状选择窗表.
fn pick_window<'a>(shape: u8, sine_window: &'a [f32], kbd_window: &'a [f32]) -> &'a [f32] {
    if shape == 1 { kbd_window } else { sine_window }
}

/// 构建 sine 窗.
fn build_sine_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| (std::f64::consts::PI / len as f64 * (i as f64 + 0.5)).sin() as f32)
        .collect()
}

/// 构建 KBD 窗.
fn build_kbd_window(len: usize, alpha: f64) -> Vec<f32> {
    if len < 2 || len % 2 != 0 {
        return build_sine_window(len);
    }

    let half = len / 2;
    let mut proto = vec![0.0f64; half];
    let mut cum = vec![0.0f64; half];
    let half_f = half as f64;

    for (i, slot) in proto.iter_mut().enumerate() {
        let x = (2.0 * i as f64) / half_f - 1.0;
        let arg = alpha * std::f64::consts::PI * (1.0 - x * x).max(0.0).sqrt();
        *slot = bessel_i0(arg);
    }

    let mut running = 0.0f64;
    for (i, &v) in proto.iter().enumerate() {
        running += v;
        cum[i] = running;
    }
    let denom = cum[half - 1].max(f64::EPSILON);

    let mut window = vec![0.0f32; len];
    for i in 0..half {
        let w = (cum[i] / denom).sqrt() as f32;
        window[i] = w;
        window[len - 1 - i] = w;
    }
    window
}

/// 第一类修正贝塞尔函数 I0.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let half = x * 0.5;
    let mut k = 1.0f64;
    loop {
        term *= (half * half) / (k * k);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
        k += 1.0;
        if k > 50.0 {
            break;
        }
    }
    sum
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
    fn test_create_and_open() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();
        assert_eq!(decoder.codec_id(), CodecId::Aac);
        assert_eq!(decoder.name(), "aac");
    }

    #[test]
    fn test_not_open_error() {
        let mut decoder = AacDecoder::create().unwrap();
        let pkt = Packet::from_data(vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x00, 0x00]);
        assert!(decoder.send_packet(&pkt).is_err());
    }

    #[test]
    fn test_silence_frame_decode() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();

        let mut adts_frame = vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC];
        adts_frame.extend_from_slice(&[0; 10]);
        let pkt = Packet::from_data(adts_frame);
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
    fn test_flush_and_eof() {
        let mut decoder = AacDecoder::create().unwrap();
        let params = make_aac_params();
        decoder.open(&params).unwrap();

        let empty_pkt = Packet::empty();
        decoder.send_packet(&empty_pkt).unwrap();
        assert!(matches!(decoder.receive_frame(), Err(TaoError::Eof)));
    }

    #[test]
    fn test_audio_specific_config_parse() {
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
            default_leading_trim_samples: 0,
            pending_leading_trim_samples: 0,
            prev_window_shape: Vec::new(),
            long_sine_window: Vec::new(),
            long_kbd_window: Vec::new(),
            short_sine_window: Vec::new(),
            short_kbd_window: Vec::new(),
            random_state: Cell::new(0x1f2e3d4c),
        };
        dec.parse_audio_specific_config(&[0x12, 0x10]).unwrap();
        assert_eq!(dec.sample_rate, 44100);
        assert_eq!(dec.channels, 2);
    }

    #[test]
    fn test_imdct_all_zero() {
        let spectrum = vec![0.0f32; 1024];
        let output = imdct_1024(&spectrum);
        assert_eq!(output.len(), 2048);
        for &s in &output {
            assert_eq!(s, 0.0);
        }
    }

    #[test]
    fn test_sf_huffman_tree_build() {
        let cbs = AacCodebooks::build();
        // 测试 delta=0 (index=60): 码字 "0" (1 bit)
        let data = [0x00u8]; // 第一位是 0
        let mut br = BitReader::new(&data);
        let val = cbs.sf_tree.decode(&mut br).unwrap();
        assert_eq!(val, 60); // SF index 60 = delta 0
    }

    #[test]
    fn test_adts_header_skip() {
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
