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

use std::collections::VecDeque;
use tao_core::{
    ChannelLayout, Rational, SampleFormat, TaoError,
    TaoResult,
};
use crate::codec_id::CodecId;
use crate::packet::Packet;
use crate::decoder::Decoder;
use crate::frame::{Frame, AudioFrame};
use crate::codec_parameters::CodecParameters;

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
    /// 存储从各个帧中提取出的 main_data
    main_data: VecDeque<u8>,
    /// Granule 解码上下文
    granule_data: [[GranuleContext; 2]; 2],
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
}

impl Mp3Decoder {
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            buffer: Vec::with_capacity(4096),
            main_data: VecDeque::with_capacity(4096),
            granule_data: Default::default(),
            synth_ctx: Default::default(),
            opened: false,
            sample_rate: 44100,
            channels: 2,
            channel_layout: ChannelLayout::from_channels(2),
            next_pts: 0,
        }))
    }

    /// 查找同步字, 返回偏移量
    fn find_sync_word(data: &[u8]) -> Option<usize> {
        if data.len() < 2 {
            return None;
        }
        for i in 0..data.len() - 1 {
            if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
                return Some(i);
            }
        }
        None
    }

    /// 解码一帧
    /// 如果成功解码一帧, 返回 (consumed_bytes, Some(Frame))
    /// 如果数据不足, 返回 (0, None)
    /// 如果出错 (如无效帧), 返回 (skipped_bytes, None)
    fn decode_one_frame(&mut self) -> TaoResult<(usize, Option<Frame>)> {
        // 1. 查找同步字
        let sync_offset = match Self::find_sync_word(&self.buffer) {
            Some(offset) => offset,
            None => {
                // 没有同步字, 丢弃除了最后 1 字节外的所有数据 (防止切断同步字)
                let len = self.buffer.len();
                if len > 1 {
                    return Ok((len - 1, None));
                }
                return Ok((0, None));
            }
        };

        // 丢弃同步字前的垃圾数据
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
            Err(_) => {
                // 伪同步字, 跳过 1 字节重试
                return Ok((1, None));
            }
        };

        // 3. 检查完整帧数据
        if self.buffer.len() < header.frame_size {
            return Ok((0, None));
        }

        // 提取当前帧数据切片
        let frame_data = &self.buffer[..header.frame_size];

        // 4. 解析 Side Information
        // Side Info 紧接在 Header (和 CRC) 之后
        let side_info_start = 4 + if header.has_crc { 2 } else { 0 };
        let side_info_end = side_info_start + header.side_info_size;

        if frame_data.len() < side_info_end {
            // 理论上 frame_size 应该足够包含 side_info, 除非 frame_size 计算错误或 header 损坏
            return Ok((1, None));
        }

        let mut reader = BitReader::new(&frame_data[side_info_start..side_info_end]);
        let side_info = match SideInfo::parse(&mut reader, &header) {
            Ok(si) => si,
            Err(_) => return Ok((1, None)),
        };

        // 5. 检查 Bit Reservoir 是否足够 (backpointer)
        // main_data_begin 指示当前帧的主数据开始位置相对于"当前帧主数据结束位置"之前的偏移?
        // 不, main_data_begin 是相对于"当前帧主数据开始位置"之前的偏移.
        // 即: decoding_start = (current_main_data_start) - main_data_begin
        // self.main_data 目前包含之前的历史数据.
        if (self.main_data.len() as u32) < side_info.main_data_begin {
            // 数据不足 (可能是流的开始), 无法解码当前帧
            // 但我们需要消耗当前帧数据并将其 Main Data 放入 Reservoir
            let main_data_slice = &frame_data[side_info_end..];
            self.main_data.extend(main_data_slice);

            // 更新 PTS (即使是静音也需要)
            let spf = if header.version == MpegVersion::Mpeg1 {
                1152
            } else {
                576
            };
            self.next_pts += spf;

            // 消耗当前帧
            return Ok((header.frame_size, None));
        }

        // 6. 将 Main Data 放入 Bit Reservoir
        // Main Data 在 Side Info 之后, 直到帧尾
        let main_data_slice = &frame_data[side_info_end..];
        self.main_data.extend(main_data_slice);

        // 7. 解码 Main Data (Phase 2 & 3 & 4)
        // 计算 Main Data 在 buffer 中的起始位置
        // main_data_begin 指示当前帧数据的起始点相对于当前帧 Main Data 起始点的偏移
        let current_main_data_len = main_data_slice.len();
        let total_len = self.main_data.len();
        let current_start_index = total_len - current_main_data_len;

        if (side_info.main_data_begin as usize) > current_start_index {
            // 数据不足 (可能是 buffer 被清理了)
            return Ok((header.frame_size, None));
        }

        let bit_reservoir_start = current_start_index - side_info.main_data_begin as usize;

        // 获取连续的 slice
        self.main_data.make_contiguous();
        let (slice, _) = self.main_data.as_slices();
        // slice 现在包含所有数据 (因为 make_contiguous 了)

        // 创建 BitReader
        // 注意: 我们只从 bit_reservoir_start 开始读取, 但长度限制在哪里?
        // 实际上每个 granule 有 part2_3_length.
        let mut br = BitReader::new(&slice[bit_reservoir_start..]);

        let huffman = huffman::HuffmanDecoder::new();

        let nch = if header.mode == crate::decoders::mp3::header::ChannelMode::SingleChannel {
            1
        } else {
            2
        };
        let is_mpeg1 = header.version == MpegVersion::Mpeg1;
        let ngr = if is_mpeg1 { 2 } else { 1 };
        
        let mut pcm_buffer = Vec::new();

        // 重置 granule data (可选, 防止干扰)
        // self.granule_data = Default::default(); // 不必要, 会覆盖

        for gr in 0..ngr {
            // --- Phase 2: Huffman Decoding ---
            for ch in 0..nch {
                let part2_3_length = side_info.granules[gr][ch].part2_3_length as usize;
                let big_values = side_info.granules[gr][ch].big_values as usize * 2;
                let global_gain = side_info.granules[gr][ch].global_gain;
                let scalefac_compress = side_info.granules[gr][ch].scalefac_compress;
                let block_type = side_info.granules[gr][ch].block_type;

                // Calculate start of data for this granule/channel
                let start_pos = br.position();

                // --- Part 2: Scalefactors ---
                // 解决借用冲突: 如果是 gr=1, 先克隆 gr=0 的 scalefactors
                let prev_scalefac = if gr == 1 {
                    self.granule_data[0][ch].scalefac
                } else {
                    [0; 40]
                };

                // 获取当前 channel 的 scalefactor 数据引用
                let scalefac = &mut self.granule_data[gr][ch].scalefac;

                // 计算 scalefactor 长度 (slen1, slen2)
                let (slen1, slen2) = if is_mpeg1 {
                    let idx = granule.scalefac_compress as usize;
                    let table = tables::SLEN_TABLE[idx];
                    (table[0] as usize, table[1] as usize)
                } else {
                    // MPEG-2 LSF logic (simplified placeholder)
                    (0, 0)
                };

                // 解码 Scalefactors
                let mut part2_bits = 0;
                if is_mpeg1 && granule.block_type == 2 && granule.mixed_block_flag {
                    // Mixed blocks (8 Long + 9*3 Short)
                    // Long part (bands 0-7 -> 8 scalefactors)
                    // Short part (bands 3-11 -> 9 scalefactors * 3 windows)

                    // 8 Long scalefactors (bands 0-7)
                    // Bands 0-5: slen1, Bands 6-7: slen1 (since 0-10 use slen1)
                    // Note: scfsi logic applies to long blocks part?
                    // Standard says: "If mixed_block_flag is set... the first 2 subbands are long blocks... the remaining subbands are short blocks"
                    // Wait, bands 0-1 are long blocks (mapping to 0-7 long sfbs?).
                    // Let's simplify: Treat mixed as separate logic.
                    // For now, implement standard Long and Short logic.

                    // 暂时只支持非 mixed blocks 以简化 Phase 2
                    // TODO: Implement mixed blocks
                } else if is_mpeg1 && granule.block_type == 2 {
                    // Short blocks (12 bands * 3 windows)
                    // Order: Band 0 (W0, W1, W2), Band 1 (W0, W1, W2)...
                    // slen1 for bands 0-5, slen2 for bands 6-11

                    for band in 0..12 {
                        let len = if band < 6 { slen1 } else { slen2 };
                        if len > 0 {
                            for win in 0..3 {
                                if let Some(val) = br.read_bits(len as u8) {
                                    scalefac[band * 3 + win] = val as u8;
                                    part2_bits += len;
                                }
                            }
                        } else {
                            for win in 0..3 {
                                scalefac[band * 3 + win] = 0;
                            }
                        }
                    }
                } else if is_mpeg1 {
                    // Long blocks (21 bands)
                    // Bands 0-10: slen1, Bands 11-20: slen2
                    // Granule 0: Read all
                    // Granule 1: Check scfsi

                    // Group 0: bands 0-5
                    // Group 1: bands 6-10
                    // Group 2: bands 11-15
                    // Group 3: bands 16-20

                    let scfsi = &side_info.scfsi[ch];
                    let groups = [(0, 6), (6, 11), (11, 16), (16, 21)];

                    for (group_idx, &(start, end)) in groups.iter().enumerate() {
                        let use_prev = gr == 1 && scfsi[group_idx] == 1;

                        for band in start..end {
                            let len = if band < 11 { slen1 } else { slen2 };

                            if use_prev {
                                // Copy from granule 0
                                // Note: We are writing to granule 1, reading from cloned granule 0
                                scalefac[band] = prev_scalefac[band];
                            } else {
                                // Read from stream
                                if len > 0 {
                                    if let Some(val) = br.read_bits(len as u8) {
                                        scalefac[band] = val as u8;
                                        part2_bits += len;
                                    }
                                } else {
                                    scalefac[band] = 0;
                                }
                            }
                        }
                    }
                }

                // --- Part 3: Huffman Decoding ---
                let _part3_bits = part2_3_length - part2_bits;

                // Big Values
                let big_values = granule.big_values as usize * 2;
                // Regions
                let mut region1_start = granule.region0_count as usize + 1;
                let mut region2_start =
                    granule.region0_count as usize + 1 + granule.region1_count as usize + 1;

                // Adjust for block_type 2 (Short blocks)
                if granule.windows_switching_flag && granule.block_type == 2 {
                    region1_start = 36; // Hardcoded boundary for short blocks
                    region2_start = 576; // End
                }

                // Get IS buffer
                let is = &mut self.granule_data[gr][ch].is;
                // Clear IS buffer (important for zeroing out upper frequencies)
                is.fill(0);

                // Huffman Loop
                // big_values pairs
                for i in (0..big_values).step_by(2) {
                    if i >= 576 {
                        break;
                    }

                    // Determine table_id
                    let table_id = if i < region1_start {
                        granule.table_select[0]
                    } else if i < region2_start {
                        granule.table_select[1]
                    } else {
                        granule.table_select[2]
                    };

                    let linbits = tables::HUFFMAN_TABLE_PARAMS[table_id as usize].1;
                    if let Ok((x, y)) = huffman.decode_big_values(&mut br, table_id, linbits) {
                        is[i] = x;
                        if i + 1 < 576 {
                            is[i + 1] = y;
                        }
                    }
                }

                // Count1 Loop (Quadruples)
                // Decode until part3_bits consumed
                // Note: This is tricky. We need to check bits_left < remaining part3_bits
                // Count1 table is table_select[3] (implied 32 or 33)
                let count1_table = if granule.count1table_select { 33 } else { 32 };

                let mut i = big_values;
                while i < 576 {
                    let current_pos = br.position();
                    let byte_diff = current_pos.0 - start_pos.0;
                    let bit_diff = current_pos.1 as isize - start_pos.1 as isize;
                    let bits_read_so_far = (byte_diff as isize * 8 + bit_diff) as usize;

                    if bits_read_so_far >= part2_3_length {
                        break;
                    }

                    // Decode quad
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

                // Skip padding bits
                // Align to end of granule
                // br.set_position(start_pos + part2_3_length)
                // 由于 BitReader API 限制, 我们手动 skip 剩余
                // 重新计算精确消耗
                let current_pos = br.position();
                let byte_diff = current_pos.0 - start_pos.0;
                let bit_diff = current_pos.1 as isize - start_pos.1 as isize;
                let bits_consumed = (byte_diff as isize * 8 + bit_diff) as usize;

                if bits_consumed < part2_3_length {
                    br.skip_bits(part2_3_length - bits_consumed);
                } else if bits_consumed > part2_3_length {
                    // Over-read! This shouldn't happen with correct logic, but Huffman can over-read.
                    // Backtrack? Or just warn.
                    // For now, ignore.
                }

                // --- Phase 3: Requantization ---
                requantize::requantize(
                    granule,
                    &mut self.granule_data[gr][ch],
                    header.version,
                    header.samplerate,
                )?;

                // --- Phase 3: Reordering (Short blocks) ---
                reorder::reorder(
                    granule,
                    &mut self.granule_data[gr][ch].xr,
                    header.version,
                    header.samplerate,
                );
            }

            // --- Phase 3: Stereo Processing ---
            // Stereo processing requires both channels to be requantized
            stereo::process_stereo(gr, &header, &mut self.granule_data, &side_info.granules);

            // --- Phase 3: Alias Reduction ---
            for ch in 0..nch {
                let granule = &side_info.granules[gr][ch];
                alias::alias_reduction(
                    granule,
                    &mut self.granule_data[gr][ch].xr,
                    header.version,
                    header.samplerate
                );
            }
            
            // --- Phase 4: IMDCT & Synthesis ---
            // 准备 IMDCT 输出缓冲区 (576 samples)
            
            // 存储双声道的 PCM 输出
            let mut pcm_ch = [[0.0f32; 576]; 2];
            
            for ch in 0..nch {
                let granule = &side_info.granules[gr][ch];
                let ctx = &mut self.granule_data[gr][ch];
                
                // 1. IMDCT (输入 xr, 输出到 xr)
                // 由于 IMDCT 输出是时域 576 samples, 而 data::xr 是 576 频域.
                // 我们直接复用 ctx.xr 作为输入, 使用局部 buffer 作为输出.
                let mut imdct_out = [0.0; 576];
                imdct::imdct(granule, &ctx.xr, &mut ctx.overlap, &mut imdct_out);
                
                // 2. Frequency Inversion
                synthesis::frequency_inversion(&mut imdct_out);
                
                // 3. Polyphase Synthesis
                // 按时间顺序处理: k=0..17
                let synth = &mut self.synth_ctx[ch];
                
                for k in 0..18 {
                    // 收集 32 个子带的第 k 个样本
                    let mut subband_samples = [0.0; 32];
                    for sb in 0..32 {
                         subband_samples[sb] = imdct_out[sb * 18 + k];
                    }
                    
                    let mut synth_out = [0.0; 32];
                    synthesis::synthesis_filter(synth, &subband_samples, &mut synth_out);
                    
                    // 存入 pcm_ch
                    for j in 0..32 {
                        pcm_ch[ch][k * 32 + j] = synth_out[j];
                    }
                }
            }
            
            // 4. Interleave & Output
            for i in 0..576 {
                for ch in 0..nch {
                    let val = pcm_ch[ch][i];
                    // Clamp & Convert
                    // Float range depends on decoder scaling.
                    // Assuming +/- 1.0 range (with correct scaling in synthesis window)
                    // If scaling was 32768.0, then values are +/- 32768.0
                    // synthesis.rs used 1.0/32768.0 scaling on window coefficients.
                    // So output should be +/- 1.0 (roughly).
                    // Let's assume +/- 1.0 for now.
                    
                    pcm_buffer.push(val);
                }
            }
        }
        
        // 8. 创建音频帧
        // let nb_samples = pcm_buffer.len() / nch as usize;
        // let mut frame = AudioFrame::new(
        //     nb_samples as u32,
        //     header.samplerate,
        //     SampleFormat::F32,
        //     ChannelLayout::from_channels(nch),
        // );
        
        // let pcm_bytes: Vec<u8> = pcm_buffer.iter().flat_map(|s| s.to_le_bytes()).collect();
        // frame.planes = vec![pcm_bytes];
        // frame.pts = self.next_pts;
        // frame.time_base = Rational::new(1, header.samplerate as i32);
        // frame.duration = nb_samples as i64;
 
        // self.next_pts += nb_samples as i64;
        // self.sample_rate = header.samplerate;
        // self.channels = nch;
        // self.channel_layout = ChannelLayout::from_channels(nch);
 
        // // Bit Reservoir management
        // let keep_len = 512;
        // if self.main_data.len() > keep_len {
        //     let remove_cnt = self.main_data.len() - keep_len;
        //     self.main_data.drain(0..remove_cnt);
        // }
 
        // Ok((header.frame_size, Some(Frame::Audio(frame))))
        
        // 使用 AudioFrame::create
        let nb_samples = pcm_buffer.len() / nch as usize;
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
        self.opened = true;
        self.buffer.clear();
        self.main_data.clear();
        self.next_pts = 0;
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
    }
}
