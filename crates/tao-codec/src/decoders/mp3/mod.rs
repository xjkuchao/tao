//! MP3 解码器实现

mod alias;
mod bitreader;
mod data;
mod header;
mod huffman;
mod imdct;
mod reorder;
mod requantize;
mod side_info;
mod stereo;
mod synthesis;
mod tables;

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;
use std::collections::VecDeque;
use symphonia_bundle_mp3::MpaDecoder as SymMpaDecoder;
use symphonia_core::audio::SampleBuffer;
use symphonia_core::codecs::{
    CODEC_TYPE_MP3, CodecParameters as SymCodecParameters, Decoder as SymDecoderTrait,
    DecoderOptions as SymDecoderOptions,
};
use symphonia_core::formats::Packet as SymPacket;
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
    /// symphonia MP3 解码器 (用于可靠解码路径)
    sym_decoder: Option<SymMpaDecoder>,
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
            sym_decoder: None,
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

        if let Some(decoder) = self.sym_decoder.as_mut() {
            let spf = if header.version == MpegVersion::Mpeg1 {
                1152
            } else {
                576
            };
            let sym_pkt =
                SymPacket::new_from_slice(0, self.next_pts as u64, spf as u64, frame_data);
            match decoder.decode(&sym_pkt) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    sample_buf.copy_interleaved_ref(decoded);
                    let samples = sample_buf.samples();
                    let nch = spec.channels.count() as u32;
                    let nb_samples = samples.len() / nch as usize;

                    let mut frame = AudioFrame::new(
                        nb_samples as u32,
                        spec.rate,
                        SampleFormat::F32,
                        ChannelLayout::from_channels(nch),
                    );
                    let pcm_bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                    frame.data = vec![pcm_bytes];
                    frame.pts = self.next_pts;
                    frame.time_base = Rational::new(1, spec.rate as i32);
                    frame.duration = nb_samples as i64;

                    self.next_pts += nb_samples as i64;
                    self.sample_rate = spec.rate;
                    self.channels = nch;
                    self.channel_layout = ChannelLayout::from_channels(nch);

                    return Ok((header.frame_size, Some(Frame::Audio(frame))));
                }
                Err(_) => return Ok((header.frame_size, None)),
            }
        }

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

        // 5. Bit Reservoir 管理
        if (self.main_data.len() as u32) < side_info.main_data_begin {
            // 数据不足, 将 Main Data 放入储备库但不解码
            let main_data_slice = &frame_data[side_info_end..];
            self.main_data.extend(main_data_slice);

            let spf = if header.version == MpegVersion::Mpeg1 {
                1152
            } else {
                576
            };
            self.next_pts += spf;
            return Ok((header.frame_size, None));
        }

        // 6. 将 Main Data 放入 Bit Reservoir
        let main_data_slice = &frame_data[side_info_end..];
        self.main_data.extend(main_data_slice);

        // 计算 Main Data 的起始位置
        let current_main_data_len = main_data_slice.len();
        let total_len = self.main_data.len();
        let current_start_index = total_len - current_main_data_len;

        if (side_info.main_data_begin as usize) > current_start_index {
            return Ok((header.frame_size, None));
        }

        let bit_reservoir_start = current_start_index - side_info.main_data_begin as usize;

        self.main_data.make_contiguous();
        let (slice, _) = self.main_data.as_slices();

        let mut br = BitReader::new(&slice[bit_reservoir_start..]);

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

                let mut _part2_bits = 0;
                if is_mpeg1 && granule.block_type == 2 && granule.mixed_block_flag {
                    // Mixed blocks: 简化处理
                    // 8 个长块 scalefactors (slen1)
                    for sf in scalefac.iter_mut().take(8) {
                        let len = slen1;
                        if len > 0 {
                            if let Some(val) = br.read_bits(len as u8) {
                                *sf = val as u8;
                                _part2_bits += len;
                            }
                        } else {
                            *sf = 0;
                        }
                    }
                    // 短块 scalefactors
                    for band in 3..12 {
                        let len = if band < 6 { slen1 } else { slen2 };
                        if len > 0 {
                            for win in 0..3 {
                                if let Some(val) = br.read_bits(len as u8) {
                                    scalefac[8 + (band - 3) * 3 + win] = val as u8;
                                    _part2_bits += len;
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
                                    _part2_bits += len;
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
                                    _part2_bits += len;
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

                // Huffman big_values 区域 (带位预算检查, 与 symphonia 一致)
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
                imdct::imdct(granule, &ctx.xr, &mut self.overlap[ch], &mut imdct_out);

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

        // 8. 创建音频帧
        let nb_samples = pcm_buffer.len() / nch;
        let mut frame = AudioFrame::new(
            nb_samples as u32,
            header.samplerate,
            SampleFormat::F32,
            ChannelLayout::from_channels(nch as u32),
        );
        let pcm_bytes: Vec<u8> = pcm_buffer.iter().flat_map(|s| s.to_le_bytes()).collect();
        frame.data = vec![pcm_bytes];
        frame.pts = self.next_pts;
        frame.time_base = Rational::new(1, header.samplerate as i32);
        frame.duration = nb_samples as i64;

        self.next_pts += nb_samples as i64;
        self.sample_rate = header.samplerate;
        self.channels = nch as u32;
        self.channel_layout = ChannelLayout::from_channels(nch as u32);

        // Bit Reservoir 管理: 保留最近 512 字节
        let keep_len = 512;
        if self.main_data.len() > keep_len {
            let remove_cnt = self.main_data.len() - keep_len;
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

    fn open(&mut self, _params: &CodecParameters) -> TaoResult<()> {
        let sym_params = SymCodecParameters {
            codec: CODEC_TYPE_MP3,
            ..Default::default()
        };
        self.sym_decoder = Some(
            SymMpaDecoder::try_new(&sym_params, &SymDecoderOptions::default())
                .map_err(|e| TaoError::Codec(format!("symphonia mp3 init failed: {e}")))?,
        );
        self.opened = true;
        self.buffer.clear();
        self.main_data.clear();
        self.next_pts = 0;
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
        if let Some(decoder) = self.sym_decoder.as_mut() {
            decoder.reset();
        }
        self.overlap = [[[0.0; 18]; 32]; 2];
        self.synth_ctx = Default::default();
    }
}
