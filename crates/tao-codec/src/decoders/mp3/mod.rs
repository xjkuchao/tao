//! MP3 解码器实现

mod alias;
mod bitreader;
mod data;
mod header;
mod huffman;
mod huffman_explicit_tables;
mod imdct;
mod reorder;
mod requantize;
mod side_info;
mod stereo;
mod synthesis;
mod tables;

pub mod debug;

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;
use std::collections::VecDeque;
use tao_core::{ChannelLayout, Rational, SampleFormat, TaoError, TaoResult};

use self::bitreader::BitReader;
use self::header::{Mp3Header, MpegVersion};
use self::side_info::SideInfo;

use self::data::GranuleContext;
use self::synthesis::SynthContext;

/// MP3 解码器
pub struct Mp3Decoder {
    /// 输入缓冲区 (存储未处理的数据包)
    buffer: Vec<u8>,
    /// 比特储备库 (Bit Reservoir)
    main_data: VecDeque<u8>,
    /// Granule 解码上下文 [granule][channel]
    granule_data: [[GranuleContext; 2]; 2],
    /// IMDCT 重叠缓冲区 [channel][subband][sample]
    /// 跨 granule 和跨帧保持
    overlap: [[[f32; 18]; 32]; 2],
    /// 合成滤波器状态 (每个 channel 一个)
    synth_ctx: [SynthContext; 2],
    /// 是否已打开
    opened: bool,
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 声道布局
    channel_layout: ChannelLayout,
    /// 累计 PTS
    next_pts: i64,
    /// 已解码帧计数
    frame_count: u32,
    /// Encoder delay (来自 LAME gapless 头, 单位: 样本/每通道)
    /// 在开始输出时需跳过的前置样本数
    encoder_delay: u32,
    /// Encoder padding (来自 LAME gapless 头, 单位: 样本/每通道)
    /// 在结束时需裁剪的后置样本数
    encoder_padding: u32,
    /// 已从输出中跳过的 encoder delay 样本数 (每通道)
    delay_skipped: u32,
    /// 总解码样本数 (每通道, 用于计算结尾裁剪)
    total_decoded_samples: u64,
    /// 总有效样本数 (每通道, 计算自 total_frames * spf - delay - padding)
    valid_samples_total: u64,
}

impl Mp3Decoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            buffer: Vec::with_capacity(4096),
            main_data: VecDeque::with_capacity(4096),
            granule_data: Default::default(),
            overlap: [[[0.0; 18]; 32]; 2],
            synth_ctx: Default::default(),
            opened: false,
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
            next_pts: 0,
            frame_count: 0,
            encoder_delay: 0,
            encoder_padding: 0,
            delay_skipped: 0,
            total_decoded_samples: 0,
            valid_samples_total: 0,
        }))
    }

    /// 查找同步字, 返回偏移量
    fn find_sync_word(data: &[u8]) -> Option<usize> {
        if data.len() < 2 {
            return None;
        }
        (0..data.len() - 1).find(|&i| data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0)
    }

    /// 解码一帧
    fn decode_one_frame(&mut self) -> TaoResult<(usize, Option<Frame>)> {
        // 1. 查找同步字
        let sync_offset = match Self::find_sync_word(&self.buffer) {
            Some(offset) => offset,
            None => {
                let len = self.buffer.len();
                if len > 1 {
                    return Ok((len - 1, None));
                }
                return Ok((0, None));
            }
        };

        if sync_offset > 0 {
            return Ok((sync_offset, None));
        }

        // 2. 解析帧头 (4 字节)
        if self.buffer.len() < 4 {
            return Ok((0, None));
        }

        let header_bytes = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]);

        let header = match Mp3Header::parse(header_bytes) {
            Ok(h) => h,
            Err(_) => return Ok((1, None)),
        };

        // 3. 检查完整帧数据
        if self.buffer.len() < header.frame_size {
            return Ok((0, None));
        }

        let frame_data = &self.buffer[..header.frame_size];

        // 4. 解析 Side Information
        let side_info_start = 4 + if header.has_crc { 2 } else { 0 };
        let side_info_end = side_info_start + header.side_info_size;

        if frame_data.len() < side_info_end {
            return Ok((1, None));
        }

        let mut reader = BitReader::new(&frame_data[side_info_start..side_info_end]);
        let side_info = match SideInfo::parse(&mut reader, &header) {
            Ok(si) => si,
            Err(_) => return Ok((1, None)),
        };

        // 5. 组装本帧 Main Data 视图:
        // [前序复用字节(main_data_begin)] + [当前帧 main_data]
        let main_data_slice = &frame_data[side_info_end..];
        let main_data_begin = side_info.main_data_begin as usize;
        const MAX_MAIN_DATA_BYTES: usize = 2048;

        if main_data_begin > self.main_data.len() {
            // 数据不足: 仅缓存当前数据, 等后续帧补齐
            self.main_data.extend(main_data_slice);
            if self.main_data.len() > MAX_MAIN_DATA_BYTES {
                let remove_cnt = self.main_data.len() - MAX_MAIN_DATA_BYTES;
                self.main_data.drain(0..remove_cnt);
            }
            return Ok((header.frame_size, None));
        }

        let reservoir: Vec<u8> = self.main_data.iter().copied().collect();
        let reuse_start = reservoir.len() - main_data_begin;

        let mut frame_main_data = Vec::with_capacity(main_data_begin + main_data_slice.len());
        frame_main_data.extend_from_slice(&reservoir[reuse_start..]);
        frame_main_data.extend_from_slice(main_data_slice);

        let mut br = BitReader::new(&frame_main_data);

        let huffman = huffman::HuffmanDecoder::new();

        let nch = if header.mode == header::ChannelMode::SingleChannel {
            1
        } else {
            2
        };
        let is_mpeg1 = header.version == MpegVersion::Mpeg1;
        let ngr = if is_mpeg1 { 2 } else { 1 };

        let mut pcm_buffer = Vec::new();

        // 构建 SFB 累积边界表 (用于 Huffman region 边界计算)
        let sfb_long_bounds = tables::build_sfb_long_bounds(header.samplerate);

        for gr in 0..ngr {
            // --- Phase 2: Huffman Decoding ---
            for ch in 0..nch {
                let granule = &side_info.granules[gr][ch];
                let part2_3_length = granule.part2_3_length as usize;
                let scalefac_compress = granule.scalefac_compress;

                let start_bit = br.bit_offset();

                // --- Part 2: Scalefactors ---
                let prev_scalefac = if gr == 1 {
                    self.granule_data[0][ch].scalefac
                } else {
                    [0; 40]
                };

                let scalefac = &mut self.granule_data[gr][ch].scalefac;
                // 初始化 scalefac, 防止前一帧 (可能是不同 block type) 的残留值
                // (长块 band 21 / 短块 band 36-39 等未传输的 scalefactor 必须为 0)
                scalefac.fill(0);

                let (slen1, slen2) = if is_mpeg1 {
                    let idx = scalefac_compress as usize;
                    let table = tables::SLEN_TABLE[idx];
                    (table[0] as usize, table[1] as usize)
                } else {
                    (0, 0)
                };

                if is_mpeg1 && granule.block_type == 2 && granule.mixed_block_flag {
                    // Mixed blocks:
                    // 长块部分 (sfb 0..7) 需遵循 scfsi 复用规则, 仅在 gr=1 生效.
                    let scfsi = &side_info.scfsi[ch];
                    for band in 0..8 {
                        let use_prev = if band < 6 { scfsi[0] } else { scfsi[1] } == 1;
                        if gr == 1 && use_prev {
                            scalefac[band] = prev_scalefac[band];
                            continue;
                        }

                        let len = slen1;
                        if len > 0 {
                            if let Some(val) = br.read_bits(len as u8) {
                                scalefac[band] = val as u8;
                            }
                        } else {
                            scalefac[band] = 0;
                        }
                    }

                    // 短块 scalefactors (sfb 3..11, 3 个窗口)
                    for band in 3..12 {
                        let len = if band < 6 { slen1 } else { slen2 };
                        if len > 0 {
                            for win in 0..3 {
                                if let Some(val) = br.read_bits(len as u8) {
                                    scalefac[8 + (band - 3) * 3 + win] = val as u8;
                                }
                            }
                        } else {
                            for win in 0..3 {
                                scalefac[8 + (band - 3) * 3 + win] = 0;
                            }
                        }
                    }
                } else if is_mpeg1 && granule.block_type == 2 {
                    // Short blocks (12 bands * 3 windows)
                    for band in 0..12 {
                        let len = if band < 6 { slen1 } else { slen2 };
                        if len > 0 {
                            for win in 0..3 {
                                if let Some(val) = br.read_bits(len as u8) {
                                    scalefac[band * 3 + win] = val as u8;
                                }
                            }
                        } else {
                            for win in 0..3 {
                                scalefac[band * 3 + win] = 0;
                            }
                        }
                    }
                } else if is_mpeg1 {
                    // Long blocks (21 bands, 4 groups, scfsi)
                    let scfsi = &side_info.scfsi[ch];
                    let groups = [(0, 6), (6, 11), (11, 16), (16, 21)];

                    for (group_idx, &(start, end)) in groups.iter().enumerate() {
                        let use_prev = gr == 1 && scfsi[group_idx] == 1;

                        for band in start..end {
                            let len = if band < 11 { slen1 } else { slen2 };

                            if use_prev {
                                scalefac[band] = prev_scalefac[band];
                            } else if len > 0 {
                                if let Some(val) = br.read_bits(len as u8) {
                                    scalefac[band] = val as u8;
                                }
                            } else {
                                scalefac[band] = 0;
                            }
                        }
                    }
                }

                // --- Part 3: Huffman Decoding ---
                let big_values = granule.big_values as usize * 2;

                // 使用 SFB 累积边界表计算 region 边界 (样本索引)
                let (region1_start, region2_start) =
                    if granule.windows_switching_flag && granule.block_type == 2 {
                        (36usize, 576usize) // 短块固定边界
                    } else {
                        let r0 = (granule.region0_count + 1) as usize;
                        let r1 = r0 + (granule.region1_count + 1) as usize;
                        (sfb_long_bounds[r0.min(22)], sfb_long_bounds[r1.min(22)])
                    };

                let is = &mut self.granule_data[gr][ch].is;
                is.fill(0);

                let end_bit = start_bit + part2_3_length;

                // Huffman big_values 区域 (带位预算检查)
                let mut i = 0usize;
                while i < big_values.min(576) {
                    if br.bit_offset() >= end_bit {
                        break;
                    }

                    let table_id = if i < region1_start {
                        granule.table_select[0]
                    } else if i < region2_start {
                        granule.table_select[1]
                    } else {
                        granule.table_select[2]
                    };

                    let linbits = tables::HUFFMAN_TABLE_PARAMS[table_id as usize].1;
                    match huffman.decode_big_values(&mut br, table_id, linbits) {
                        Ok((x, y)) => {
                            is[i] = x;
                            if i + 1 < 576 {
                                is[i + 1] = y;
                            }
                            i += 2;
                        }
                        Err(_) => break,
                    }
                }

                // Count1 区域 (四元组)
                let count1_table = if granule.count1table_select { 33 } else { 32 };

                while i < 576 {
                    if br.bit_offset() >= end_bit {
                        break;
                    }

                    if let Ok((v, w, x, y)) = huffman.decode_count1(&mut br, count1_table) {
                        is[i] = v;
                        if i + 1 < 576 {
                            is[i + 1] = w;
                        }
                        if i + 2 < 576 {
                            is[i + 2] = x;
                        }
                        if i + 3 < 576 {
                            is[i + 3] = y;
                        }
                        i += 4;
                    } else {
                        break;
                    }
                }

                // 如果 count1 解码超出了 part2_3_length 边界,
                // 丢弃最后一组四元组 (其值基于越界比特, 不可信)
                if br.bit_offset() > end_bit && i > big_values {
                    i -= 4;
                    for val in is.iter_mut().take((i + 4).min(576)).skip(i) {
                        *val = 0;
                    }
                }

                // rzero: Huffman 解码样本数 (big_values + count1), 之后的样本全为 0
                self.granule_data[gr][ch].rzero = i;

                br.seek_to_bit(end_bit);

                // --- Phase 3: Requantization ---
                self.granule_data[gr][ch].xr.fill(0.0);

                requantize::requantize(
                    granule,
                    &mut self.granule_data[gr][ch],
                    header.version,
                    header.samplerate,
                )?;
            }

            // --- Phase 3: Stereo Processing (在 reorder 之前, 因为立体声处理需要 SFB 顺序) ---
            stereo::process_stereo(
                gr,
                &header,
                &mut self.granule_data,
                &side_info.granules,
                header.samplerate,
            );

            // Joint stereo 处理后, 两个通道的非零范围扩展到两者的最大值
            if nch == 2 && header.mode == header::ChannelMode::JointStereo {
                let max_rzero = self.granule_data[gr][0]
                    .rzero
                    .max(self.granule_data[gr][1].rzero);
                self.granule_data[gr][0].rzero = max_rzero;
                self.granule_data[gr][1].rzero = max_rzero;
            }

            // --- Phase 3: Reorder + Alias Reduction ---
            for ch in 0..nch {
                let granule = &side_info.granules[gr][ch];

                // Reorder (短块重排序)
                reorder::reorder(
                    granule,
                    &mut self.granule_data[gr][ch].xr,
                    header.version,
                    header.samplerate,
                );

                // Alias Reduction (抗混叠, 限制处理范围到 rzero 附近)
                let rzero = self.granule_data[gr][ch].rzero;
                alias::alias_reduction(
                    granule,
                    &mut self.granule_data[gr][ch].xr,
                    rzero,
                    header.version,
                    header.samplerate,
                );
            }

            // --- Phase 4: IMDCT & Synthesis ---
            let mut pcm_ch = [[0.0f32; 576]; 2];

            for (ch, pcm_channel) in pcm_ch.iter_mut().enumerate().take(nch) {
                let granule = &side_info.granules[gr][ch];
                let ctx = &self.granule_data[gr][ch];

                // 1. IMDCT (使用每通道共享的 overlap 缓冲区)
                let mut imdct_out = [0.0; 576];
                imdct::imdct(
                    granule,
                    &ctx.xr,
                    ctx.rzero,
                    &mut self.overlap[ch],
                    &mut imdct_out,
                );

                // 2. Frequency Inversion
                synthesis::frequency_inversion(&mut imdct_out);

                // 3. Polyphase Synthesis
                let synth = &mut self.synth_ctx[ch];

                for k in 0..18 {
                    let mut subband_samples = [0.0; 32];
                    for (sb, sample) in subband_samples.iter_mut().enumerate() {
                        *sample = imdct_out[sb * 18 + k];
                    }

                    let mut synth_out = [0.0; 32];
                    synthesis::synthesis_filter(synth, &subband_samples, &mut synth_out);

                    pcm_channel[k * 32..k * 32 + 32].copy_from_slice(&synth_out);
                }
            }

            // 4. Interleave & Output
            for i in 0..576 {
                for pcm_channel in pcm_ch.iter().take(nch) {
                    pcm_buffer.push(pcm_channel[i]);
                }
            }
        }

        // 8. Gapless 裁剪: 跳过 encoder delay 前缀
        // pcm_buffer 以交错格式存储, 每帧 nb_interleaved = samples_per_ch * nch
        let raw_samples_per_ch = pcm_buffer.len() / nch;

        // 8a. 跳过前置 encoder delay 样本
        let skip_front_per_ch = if self.encoder_delay > 0 && self.delay_skipped < self.encoder_delay
        {
            let remaining_delay = self.encoder_delay - self.delay_skipped;
            remaining_delay.min(raw_samples_per_ch as u32) as usize
        } else {
            0
        };
        self.delay_skipped += skip_front_per_ch as u32;

        // 8b. 累积总解码样本数 (裁剪前置 delay 后的有效部分)
        let usable_per_ch = raw_samples_per_ch - skip_front_per_ch;
        self.total_decoded_samples += usable_per_ch as u64;

        // 8c. 裁剪后置 padding: 若知道 valid_samples_total, 截断超出部分
        let keep_per_ch = if self.encoder_padding > 0 && self.valid_samples_total > 0 {
            let already_output = self.total_decoded_samples - usable_per_ch as u64;
            let can_output = self.valid_samples_total.saturating_sub(already_output) as usize;
            usable_per_ch.min(can_output)
        } else {
            usable_per_ch
        };

        // 构造裁剪后的 PCM (交错)
        let front_interleaved = skip_front_per_ch * nch;
        let keep_interleaved = keep_per_ch * nch;
        let trimmed_pcm = &pcm_buffer[front_interleaved..front_interleaved + keep_interleaved];

        // 若裁剪后无有效样本, 继续下一帧 (不输出空帧)
        if keep_per_ch == 0 {
            // Bit Reservoir 管理: 追加当前帧 main_data, 保留最近窗口
            self.main_data.extend(main_data_slice);
            if self.main_data.len() > MAX_MAIN_DATA_BYTES {
                let remove_cnt = self.main_data.len() - MAX_MAIN_DATA_BYTES;
                self.main_data.drain(0..remove_cnt);
            }
            self.sample_rate = header.samplerate;
            self.channels = nch as u32;
            self.channel_layout = ChannelLayout::from_channels(nch as u32);
            self.frame_count += 1;
            return Ok((header.frame_size, None));
        }

        let nb_samples = keep_per_ch;
        let mut frame = AudioFrame::new(
            nb_samples as u32,
            header.samplerate,
            SampleFormat::F32,
            ChannelLayout::from_channels(nch as u32),
        );
        let pcm_bytes: Vec<u8> = trimmed_pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        frame.data = vec![pcm_bytes];
        frame.pts = self.next_pts;
        frame.time_base = Rational::new(1, header.samplerate as i32);
        frame.duration = nb_samples as i64;

        self.next_pts += nb_samples as i64;
        self.sample_rate = header.samplerate;
        self.channels = nch as u32;
        self.channel_layout = ChannelLayout::from_channels(nch as u32);
        self.frame_count += 1;

        // Bit Reservoir 管理: 追加当前帧 main_data, 保留最近窗口
        self.main_data.extend(main_data_slice);
        if self.main_data.len() > MAX_MAIN_DATA_BYTES {
            let remove_cnt = self.main_data.len() - MAX_MAIN_DATA_BYTES;
            self.main_data.drain(0..remove_cnt);
        }

        Ok((header.frame_size, Some(Frame::Audio(frame))))
    }
}

impl Decoder for Mp3Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Mp3
    }

    fn name(&self) -> &str {
        "mp3"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        self.opened = true;
        self.buffer.clear();
        self.main_data.clear();
        self.next_pts = 0;
        self.delay_skipped = 0;
        self.total_decoded_samples = 0;

        // 从 extra_data 读取 gapless 信息 (由 MP3 demuxer 从 LAME/Lavc 头写入)
        // 格式: [front_skip_le_u32][padding_le_u32][valid_total_le_u64] 共 16 字节
        // front_skip = encoder_delay (LAME字段) + MP3_DECODER_LATENCY (529)
        // valid_total = total_frames * spf - encoder_delay - encoder_padding (纯 LAME 公式)
        if params.extra_data.len() >= 16 {
            self.encoder_delay =
                u32::from_le_bytes(params.extra_data[0..4].try_into().unwrap_or([0; 4]));
            self.encoder_padding =
                u32::from_le_bytes(params.extra_data[4..8].try_into().unwrap_or([0; 4]));
            self.valid_samples_total =
                u64::from_le_bytes(params.extra_data[8..16].try_into().unwrap_or([0; 8]));
        } else {
            self.encoder_delay = 0;
            self.encoder_padding = 0;
            self.valid_samples_total = 0;
        }

        // 重置 overlap 和 synth 状态
        self.overlap = [[[0.0; 18]; 32]; 2];
        self.synth_ctx = Default::default();
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("MP3 解码器未打开".into()));
        }
        self.buffer.extend_from_slice(&packet.data);
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        loop {
            let (consumed, frame) = self.decode_one_frame()?;
            if consumed > 0 {
                self.buffer.drain(..consumed);
            }

            if let Some(f) = frame {
                return Ok(f);
            }

            if consumed == 0 {
                return Err(TaoError::NeedMoreData);
            }
        }
    }

    fn flush(&mut self) {
        self.buffer.clear();
        self.main_data.clear();
        self.next_pts = 0;
        self.delay_skipped = 0;
        self.total_decoded_samples = 0;
        self.overlap = [[[0.0; 18]; 32]; 2];
        self.synth_ctx = Default::default();
    }
}
