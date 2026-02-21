use super::*;

impl Mpeg4Decoder {
    pub(super) fn analyze_data_partitions(
        &self,
        data: &[u8],
        fcode: u8,
    ) -> (DataPartitionInfo, u32) {
        let mut info = DataPartitionInfo {
            partition_a: (0, data.len() * 8),
            partition_b: (data.len() * 8, data.len() * 8),
            partition_c: (data.len() * 8, data.len() * 8),
        };
        let mut partition_count = 0u32;

        let total_mbs = self.mb_stride * (self.height as usize).div_ceil(16);
        let mb_bits = if total_mbs > 1 {
            (total_mbs as f32).log2().ceil() as usize
        } else {
            1
        };
        let packet_header_bits = mb_bits + 5 + 1;

        // resync marker 长度 = 16 + fcode 位
        let marker_len = 16 + fcode as usize;

        // 扫描 resync markers (无需创建 BitReader，直接按位扫描)
        let mut bit_pos = 0usize;
        let max_bits = data.len() * 8;

        while bit_pos + marker_len + 8 < max_bits {
            // 对齐到字节边界
            if bit_pos % 8 != 0 {
                bit_pos = bit_pos.div_ceil(8) * 8;
            }

            // 检查是否是 resync marker
            let byte_pos = bit_pos / 8;
            if byte_pos + marker_len.div_ceil(8) >= data.len() {
                break;
            }

            // 检查 resync marker 模式: marker_len 位中前 marker_len-1 位为 0, 最后 1 位为 1
            let mut is_marker = true;
            for i in 0..marker_len {
                let bit_offset = bit_pos + i;
                let byte_idx = bit_offset / 8;
                let bit_idx = 7 - (bit_offset % 8);

                if byte_idx >= data.len() {
                    is_marker = false;
                    break;
                }

                let bit_val = (data[byte_idx] >> bit_idx) & 1;
                if i < marker_len - 1 {
                    // 前 marker_len-1 位应为 0
                    if bit_val != 0 {
                        is_marker = false;
                        break;
                    }
                } else {
                    // 最后 1 位应为 1
                    if bit_val != 1 {
                        is_marker = false;
                        break;
                    }
                }
            }

            if is_marker {
                // 找到 resync marker
                partition_count += 1;
                let partition_start = bit_pos + marker_len;

                match partition_count {
                    1 => {
                        // 第一个 resync marker 标记 Partition A 的结束和 Partition B 的开始
                        info.partition_a.1 = bit_pos;
                        info.partition_b.0 = partition_start;
                    }
                    2 => {
                        // 第二个 resync marker 标记 Partition B 的结束和 Partition C 的开始
                        info.partition_b.1 = bit_pos;
                        info.partition_c.0 = partition_start;
                        info.partition_c.1 = max_bits;
                    }
                    _ => {
                        // 后续 resync markers 可能是下一个 video packet,停止分析
                        break;
                    }
                }

                // 跳过 marker、macroblock_number、quant_scale 与 HEC 标志位
                bit_pos = partition_start + packet_header_bits;
            } else {
                // 按字节步进
                bit_pos += 8;
            }
        }

        // 如果只找到部分分区边界，调整信息
        if partition_count == 0 {
            // 没有 resync marker，整个 video packet 都是 Partition A
            info.partition_a.1 = max_bits;
            info.partition_b.0 = max_bits;
            info.partition_b.1 = max_bits;
            info.partition_c.0 = max_bits;
            info.partition_c.1 = max_bits;
        } else if partition_count == 1 {
            // 只有一个 resync marker，Partition C 为空
            info.partition_c.1 = max_bits;
        }

        (info, partition_count)
    }

    /// 扫描数据分区中的分包边界 (旧版本，保留用于兼容)
    ///
    /// 数据分区的每个分包都有 resync marker。本函数扫描位流以检测分包数量。
    /// 返回找到的分包数（含第一个隐含分包）。
    #[allow(dead_code)]
    pub(super) fn scan_data_partitions(data: &[u8]) -> u32 {
        let mut partition_count = 1u32;
        let mut offset = 0;

        // 简单启发式: 扫描 resync marker 出现次数
        // resync marker pattern: 16+ 个零位 + 1 个一位
        while offset < data.len() {
            // 查找下一个潜在的 resync marker (0x00 0x00 字节序列作为启发式指标)
            if offset + 2 < data.len() && data[offset] == 0x00 && data[offset + 1] == 0x00 {
                partition_count += 1;
            }
            offset += 1;
        }

        partition_count
    }

    /// 检查 resync marker
    ///
    /// resync marker 是 stuffing bits + (16 + vop_fcode) 个零 + 1 个一.
    /// 先检查到字节边界的 stuffing bits 是否全为 0,
    /// 然后检查后续 (17 + fcode - 1) 位是否为 "000...001" 模式.
    pub(super) fn check_resync_marker(reader: &BitReader, vop_fcode: u8) -> bool {
        // 参考 FFmpeg mpeg4_is_resync:
        // MPEG-4 resync marker 前有 stuffing bits (0 + 1...1 到字节对齐),
        // 然后是 prefix_length 个零位 + 1 位.
        //
        // 前缀表: 每个位偏移对应的 16-bit peek 值
        // bit_offset=0: stuffing=01111111 + 8 zeros = 0x7F00
        // bit_offset=1: stuffing=0111111  + 9 zeros = 0x7E00
        // ...
        // bit_offset=7: stuffing=0        + 15 zeros = 0x0000
        const RESYNC_PREFIX: [u16; 8] = [
            0x7F00, 0x7E00, 0x7C00, 0x7800, 0x7000, 0x6000, 0x4000, 0x0000,
        ];

        let Some(v) = reader.peek_bits(16) else {
            return false;
        };

        let bit_offset = reader.bit_position() & 7;
        if v as u16 != RESYNC_PREFIX[bit_offset] {
            return false;
        }

        // 前缀匹配, 需要进一步验证后续有足够的 marker 零位
        // 16-bit peek 中已包含的 marker 零位数 = 8 + bit_offset
        // 需要的总零位数 (prefix_length) = 16 + vop_fcode
        // 还需额外零位 = prefix_length - (8 + bit_offset) = 8 + vop_fcode - bit_offset
        let zeros_seen = 8 + bit_offset;
        let prefix_length = 16 + vop_fcode as usize;
        if zeros_seen >= prefix_length {
            // 已经看到足够的零位, 只需验证下一个位是 1
            // 但我们只 peek 了 16 位, 无法检查第 17 位
            // 对于 I-VOP (fcode=0): prefix=16, zeros_seen=8+bit_offset
            // 当 bit_offset >= 8 时 (不可能) 才满足. 实际不会走这个分支 (I-VOP).
            return true;
        }

        // 需要额外 peek 更多位来验证
        let extra_zeros = prefix_length - zeros_seen;
        let check_bits = (16 + extra_zeros + 1) as u8; // 16 前缀 + 额外零位 + 1 终止位
        if check_bits > 32 {
            return false;
        }
        let Some(full_bits) = reader.peek_bits(check_bits) else {
            return false;
        };
        // 最后 (extra_zeros + 1) 位应为: extra_zeros 个 0 + 1 个 1 → 值为 1
        let tail = full_bits & ((1 << (extra_zeros + 1)) - 1);
        tail == 1
    }

    /// 跳过 resync marker 并解析 video packet header
    ///
    /// 返回 (macroblock_number, new_quant)
    pub(super) fn parse_video_packet_header(&self, reader: &mut BitReader) -> Option<(u32, u8)> {
        // 保存位置, 解析失败时恢复
        let saved_pos = reader.snapshot_position();

        // 跳过 stuffing bits: MPEG-4 标准定义为 '0' + '1...1' 到字节对齐
        // 无论当前是否字节对齐, 至少消耗 1 位 '0'
        reader.read_bits(1)?;
        let align_bits = reader.bits_to_byte_align();
        if align_bits > 0 {
            reader.read_bits(align_bits)?;
        }

        // 跳过 resync marker: prefix_length 个零位 + 1 个终止位
        // 计数连续零位直到遇到 '1'
        let mut zero_count = 0u32;
        loop {
            let bit = reader.read_bit()?;
            if bit {
                break; // 遇到终止位 '1'
            }
            zero_count += 1;
            if zero_count > 32 {
                reader.restore_position(saved_pos);
                return None;
            }
        }

        // macroblock_number (变长: log2(total_mbs) 位)
        let total_mbs = self.mb_stride * (self.height as usize).div_ceil(16);
        let mb_bits = if total_mbs > 1 {
            (total_mbs as f32).log2().ceil() as u8
        } else {
            1
        };
        let mb_number = reader.read_bits(mb_bits)?;

        // quant_scale (5 bits)
        let quant = reader.read_bits(5)? as u8;

        // header_extension_code (1 bit) - 暂时跳过扩展
        let hec = reader.read_bit().unwrap_or(false);
        if hec {
            // 扩展头: modulo_time_base + marker + vop_time_increment + marker
            // + vop_coding_type + intra_dc_vlc_thr + (f_codes)
            // 简化处理: 跳过扩展部分
            while reader.read_bit() == Some(true) {}
            reader.read_bit(); // marker
            let time_inc_resolution = self
                .vol_info
                .as_ref()
                .map(|v| v.vop_time_increment_resolution)
                .unwrap_or(30000);
            let bits = if time_inc_resolution > 1 {
                (time_inc_resolution as f32).log2().ceil() as u8
            } else {
                1
            };
            reader.read_bits(bits.max(1)); // vop_time_increment
            reader.read_bit(); // marker
            reader.read_bits(2); // vop_coding_type
            reader.read_bits(3); // intra_dc_vlc_thr
            // f_code_forward (P/B)
            reader.read_bits(3);
        }

        Some((mb_number, quant))
    }

    // ========================================================================
    // Data Partitioning 解码函数
    // ========================================================================

    /// 从 Partition A 解码宏块头部信息
    ///
    /// 解码内容:
    /// - MCBPC/CBPY (宏块类型和 CBP)
    /// - DQUANT (量化参数变化)
    /// - 运动向量
    /// - field_dct 标志
    pub(super) fn decode_partition_a_mb_header(
        &mut self,
        reader: &mut BitReader,
        mb_x: u32,
        mb_y: u32,
        is_i_vop: bool,
    ) -> Option<PartitionedMacroblockData> {
        // 1. MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader)?
        } else {
            // P-VOP: not_coded 位
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                return Some(PartitionedMacroblockData {
                    mb_type: MbType::Inter,
                    cbp: 0,
                    quant: self.quant,
                    ac_pred_flag: false,
                    mvs: [MotionVector::default(); 4],
                    dc_coeffs: [0; 6],
                    ac_coeffs: [[0; 63]; 6],
                    field_dct: false,
                    field_pred: false,
                    field_for_top: false,
                    field_for_bot: false,
                });
            }
            decode_mcbpc_p(reader)?
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // AC prediction flag
        let ac_pred_flag = if is_intra {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // 2. CBPY
        // 如果CBPY解码失败，记录详细诊断信息
        let cbpy = match decode_cbpy(reader, is_intra) {
            Some(val) => val,
            None => {
                // CBPY 解码失败 - 这是一个严重问题，表明比特流可能不对齐
                // 在这种情况下，使用0作为保守的fallback
                trace!(
                    "宏块CBPY解码失败: 字节位置={}, mb_type={:?}, cbpc={}, is_intra={}",
                    reader.byte_position(),
                    mb_type,
                    cbpc,
                    is_intra
                );
                0
            }
        };

        // 3. DQUANT
        if mb_type == MbType::IntraQ || mb_type == MbType::InterQ {
            if let Some(dq) = reader.read_bits(2) {
                let delta = DQUANT_TABLE[dq as usize];
                self.quant = ((self.quant as i32 + delta).clamp(1, 31)) as u8;
            }
        }

        // 4. 隔行模式: field_dct
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        let cbp = (cbpy << 2) | cbpc;
        let field_dct = if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // field_pred (仅 P-VOP)
        let mut field_pred = false;
        let mut field_for_top = false;
        let mut field_for_bot = false;
        if interlacing && !is_intra {
            field_pred = reader.read_bit().unwrap_or(false);
            if field_pred {
                field_for_top = reader.read_bit().unwrap_or(false);
                field_for_bot = reader.read_bit().unwrap_or(false);
            }
        }

        // 5. 运动向量解码
        let mut mb_mvs = [MotionVector::default(); 4];
        if !is_intra {
            if field_pred && mb_type != MbType::Inter4V {
                let mut mv_top = MotionVector::default();
                let mut mv_bot = MotionVector::default();
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_top = mv;
                }
                if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 2) {
                    self.validate_vector(&mut mv, mb_x, mb_y);
                    mv_bot = mv;
                }
                mb_mvs = [mv_top, mv_top, mv_bot, mv_bot];
            } else if mb_type == MbType::Inter4V {
                for (k, mv_slot) in mb_mvs.iter_mut().enumerate() {
                    if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, k) {
                        self.validate_vector(&mut mv, mb_x, mb_y);
                        *mv_slot = mv;
                    }
                }
            } else if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                self.validate_vector(&mut mv, mb_x, mb_y);
                mb_mvs = [mv; 4];
            }
        }

        Some(PartitionedMacroblockData {
            mb_type,
            cbp,
            quant: self.quant,
            ac_pred_flag,
            mvs: mb_mvs,
            dc_coeffs: [0; 6],
            ac_coeffs: [[0; 63]; 6],
            field_dct,
            field_pred,
            field_for_top,
            field_for_bot,
        })
    }

    /// 从 Partition B 解码 DC 系数
    ///
    /// 使用 RVLC 解码所有块的 DC 系数。
    pub(super) fn decode_partition_b_dc(
        &mut self,
        reader: &mut BitReader,
        mb_data: &mut PartitionedMacroblockData,
        mb_x: u32,
        mb_y: u32,
    ) -> bool {
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);
        if !is_intra {
            // Inter 块没有 DC 系数在 Partition B
            return true;
        }

        use self::vlc::decode_intra_dc_vlc;

        // 解码 6 个块的 DC 系数 (Y0-Y3, U, V)
        for block_idx in 0..6 {
            let is_luma = block_idx < 4;

            // 使用 use_intra_dc_vlc 决定是否解码 DC
            let dc_diff = if self.use_intra_dc_vlc() {
                if let Some(dc) = decode_intra_dc_vlc(reader, is_luma) {
                    dc
                } else {
                    debug!(
                        "Partition B DC 解码失败, MB ({}, {}), block {}",
                        mb_x, mb_y, block_idx
                    );
                    return false;
                }
            } else {
                0
            };

            // 使用 DC 预测
            let (dc_pred, _direction) =
                self.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);
            let actual_dc = dc_pred.wrapping_add(dc_diff);
            let dc_scaler = self.get_dc_scaler(is_luma);
            mb_data.dc_coeffs[block_idx] = (actual_dc as i32 * dc_scaler as i32) as i16;
        }

        true
    }

    /// 从 Partition C 解码 AC 系数
    ///
    /// 解码所有块的 AC 系数。
    pub(super) fn decode_partition_c_ac(
        &mut self,
        reader: &mut BitReader,
        mb_data: &mut PartitionedMacroblockData,
        mb_x: u32,
        mb_y: u32,
    ) -> bool {
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);
        let use_rvlc = self
            .vol_info
            .as_ref()
            .map(|v| v.reversible_vlc)
            .unwrap_or(false);

        // 选择扫描表
        let scan_table = if mb_data.field_dct || self.alternate_vertical_scan {
            &ALTERNATE_VERTICAL_SCAN
        } else {
            &ZIGZAG_SCAN
        };

        // 解码 6 个块的 AC 系数
        for block_idx in 0..6 {
            let ac_coded = (mb_data.cbp >> (5 - block_idx)) & 1 != 0;
            if !ac_coded {
                continue;
            }

            let plane = if block_idx < 4 { 0 } else { block_idx - 3 };

            // 解码 AC 系数
            let coeffs = if is_intra {
                // Intra 块: 跳过 DC (已在 Partition B 中解码), 只解码 AC
                if use_rvlc {
                    self.decode_intra_ac_only_rvlc(
                        reader,
                        plane,
                        mb_x,
                        mb_y,
                        block_idx,
                        mb_data.ac_pred_flag,
                        scan_table,
                    )
                } else {
                    self.decode_intra_ac_only(
                        reader,
                        plane,
                        mb_x,
                        mb_y,
                        block_idx,
                        mb_data.ac_pred_flag,
                        scan_table,
                    )
                }
            } else {
                // Inter 块
                if use_rvlc {
                    self.decode_inter_block_rvlc(reader, scan_table)
                } else {
                    decode_inter_block_vlc(reader, scan_table)
                }
            };

            if let Some(block_coeffs) = coeffs {
                // 提取 AC 系数 (跳过 DC)
                for (i, &coeff) in block_coeffs.iter().enumerate().skip(1) {
                    mb_data.ac_coeffs[block_idx][i - 1] = coeff as i16;
                }
            } else {
                debug!(
                    "Partition C AC 解码失败, MB ({}, {}), block {}",
                    mb_x, mb_y, block_idx
                );
                return false;
            }
        }

        true
    }

    /// 辅助函数: 仅解码 Intra 块的 AC 系数 (跳过 DC)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_intra_ac_only(
        &mut self,
        reader: &mut BitReader,
        _plane: usize,
        mb_x: u32,
        mb_y: u32,
        block_idx: usize,
        ac_pred_flag: bool,
        scan_table: &[usize; 64],
    ) -> Option<[i32; 64]> {
        use self::tables::{ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN};
        use self::types::PredictorDirection;
        use self::vlc::{INTRA_AC_VLC, decode_ac_vlc};

        const COEFF_MIN: i32 = -2048;
        const COEFF_MAX: i32 = 2047;

        let mut block = [0i32; 64];
        // DC 已在 Partition B 中解码, 这里只解码 AC

        // 获取 AC 预测方向
        let (_dc_pred, direction) =
            self.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);

        // 选择扫描顺序
        let ac_scan = if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,
                PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,
                PredictorDirection::None => scan_table,
            }
        } else {
            scan_table
        };

        // 解码 AC 系数
        let mut pos = 1; // 从 1 开始 (跳过 DC)
        while pos < 64 {
            match decode_ac_vlc(reader, INTRA_AC_VLC, true) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[ac_scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }

        // AC 预测
        if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => {
                    let c_idx = match block_idx {
                        0 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                        1 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                        2 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        3 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i];
                            block[idx] = block[idx]
                                .wrapping_add(pred_ac[i] as i32)
                                .clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
                PredictorDirection::Horizontal => {
                    let c_idx = match block_idx {
                        0 | 2 => self.get_neighbor_block_idx(
                            mb_x as isize - 1,
                            mb_y as isize,
                            block_idx + 1,
                        ),
                        1 | 3 => {
                            self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, block_idx - 1)
                        }
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in (1..8).step_by(8) {
                            for j in 0..8 {
                                let idx = ac_scan[i + j];
                                block[idx] = block[idx]
                                    .wrapping_add(pred_ac[j + 1] as i32)
                                    .clamp(COEFF_MIN, COEFF_MAX);
                            }
                        }
                    }
                }
                PredictorDirection::None => {}
            }
        }

        Some(block)
    }

    /// 辅助函数: 仅解码 Intra 块的 AC 系数 (RVLC)
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decode_intra_ac_only_rvlc(
        &mut self,
        reader: &mut BitReader,
        _plane: usize,
        mb_x: u32,
        mb_y: u32,
        block_idx: usize,
        ac_pred_flag: bool,
        scan_table: &[usize; 64],
    ) -> Option<[i32; 64]> {
        use self::tables::{ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN};
        use self::types::PredictorDirection;
        use self::vlc::decode_ac_rvlc_forward;

        const COEFF_MIN: i32 = -2048;
        const COEFF_MAX: i32 = 2047;

        let mut block = [0i32; 64];

        // 获取 AC 预测方向
        let (_dc_pred, direction) =
            self.get_intra_predictor(mb_x as usize, mb_y as usize, block_idx);

        // 选择扫描顺序
        let ac_scan = if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => &ALTERNATE_HORIZONTAL_SCAN,
                PredictorDirection::Horizontal => &ALTERNATE_VERTICAL_SCAN,
                PredictorDirection::None => scan_table,
            }
        } else {
            scan_table
        };

        // 使用 RVLC 解码 AC 系数
        let mut pos = 1;
        while pos < 64 {
            match decode_ac_rvlc_forward(reader, true) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[ac_scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }

        // AC 预测
        if ac_pred_flag {
            match direction {
                PredictorDirection::Vertical => {
                    let c_idx = match block_idx {
                        0 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 2),
                        1 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, 3),
                        2 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        3 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 1),
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize, mb_y as isize - 1, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = c_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i];
                            block[idx] = block[idx]
                                .wrapping_add(pred_ac[i] as i32)
                                .clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
                PredictorDirection::Horizontal => {
                    let a_idx = match block_idx {
                        0 => self.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 1),
                        1 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 0),
                        2 => self.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, 3),
                        3 => self.get_neighbor_block_idx(mb_x as isize, mb_y as isize, 2),
                        4 | 5 => {
                            self.get_neighbor_block_idx(mb_x as isize - 1, mb_y as isize, block_idx)
                        }
                        _ => None,
                    };
                    if let Some(idx) = a_idx {
                        let pred_ac = self.predictor_cache[idx];
                        for i in 1..8 {
                            let idx = ac_scan[i * 8];
                            block[idx] = block[idx]
                                .wrapping_add(pred_ac[7 + i] as i32)
                                .clamp(COEFF_MIN, COEFF_MAX);
                        }
                    }
                }
                _ => {}
            }
        }

        Some(block)
    }

    /// 辅助函数: 解码 Inter 块的 AC 系数 (RVLC)
    pub(super) fn decode_inter_block_rvlc(
        &mut self,
        reader: &mut BitReader,
        scan: &[usize; 64],
    ) -> Option<[i32; 64]> {
        use self::vlc::decode_ac_rvlc_forward;

        let mut block = [0i32; 64];
        let mut pos = 0;
        while pos < 64 {
            match decode_ac_rvlc_forward(reader, false) {
                Ok(None) => break,
                Ok(Some((last, run, level))) => {
                    pos += run as usize;
                    if pos >= 64 {
                        break;
                    }
                    block[scan[pos]] = level as i32;
                    pos += 1;
                    if last {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
        Some(block)
    }

    /// 重建分区宏块到帧
    ///
    /// 将从三个分区解码的数据组合并重建到输出帧。
    pub(super) fn reconstruct_partitioned_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        mb_data: &PartitionedMacroblockData,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;
        let is_intra = matches!(mb_data.mb_type, MbType::Intra | MbType::IntraQ);

        // 存储 MV 到缓存
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_data.mvs;
        }

        // 更新宏块信息
        if mb_idx < self.mb_info.len() {
            let mode_code = match mb_data.mb_type {
                MbType::Inter | MbType::InterQ => MacroblockInfo::MODE_INTER,
                MbType::Intra | MbType::IntraQ => MacroblockInfo::MODE_INTRA,
                MbType::Inter4V => MacroblockInfo::MODE_INTER4V,
            };
            self.mb_info[mb_idx] = MacroblockInfo {
                mode: mode_code,
                quant: mb_data.quant,
                mvs: mb_data.mvs,
            };
        }

        let quarterpel = self
            .vol_info
            .as_ref()
            .map(|v| v.quarterpel)
            .unwrap_or(false);
        let field_pred = mb_data.field_pred;
        let use_quarterpel = quarterpel;

        // 重建 Y 平面的 4 个 8x8 块
        for block_idx in 0..4 {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;

            // 组合 DC 和 AC 系数
            let mut block = [0i32; 64];
            block[0] = mb_data.dc_coeffs[block_idx] as i32;
            for i in 0..63 {
                block[i + 1] = mb_data.ac_coeffs[block_idx][i] as i32;
            }

            // 反量化和 IDCT
            self.dequantize(&mut block, mb_data.quant as u32, is_intra);
            idct_8x8(&mut block);

            let mv = mb_data.mvs[block_idx];

            // 写入帧
            for y in 0..8 {
                for x in 0..8 {
                    let px = (mb_x as usize * 16 + bx as usize * 8 + x) as isize;
                    let py = (mb_y as usize * 16 + by as usize * 8 + y) as isize;
                    if px < width as isize && py < height as isize {
                        let idx = py as usize * width + px as usize;
                        let residual = block[y * 8 + x];
                        let val = if is_intra {
                            residual.clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_block(
                                    block_idx,
                                    mb_data.field_for_top,
                                    mb_data.field_for_bot,
                                );
                                let mv_y = Self::scale_field_mv_y(mv.y);
                                Self::motion_compensate_field(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv_y,
                                    self.rounding_control,
                                    use_quarterpel,
                                    field_select,
                                )
                            } else {
                                Self::motion_compensate(
                                    ref_frame,
                                    0,
                                    px,
                                    py,
                                    mv.x,
                                    mv.y,
                                    self.rounding_control,
                                    use_quarterpel,
                                )
                            };
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[0][idx] = val;
                    }
                }
            }
        }

        // 重建 U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        let (chroma_mv, chroma_mv_top, chroma_mv_bot) = if !is_intra {
            if field_pred {
                if mb_data.mb_type == MbType::Inter4V {
                    let top_avg = Self::average_mv(mb_data.mvs[0], mb_data.mvs[1]);
                    let bot_avg = Self::average_mv(mb_data.mvs[2], mb_data.mvs[3]);
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(top_avg),
                        Self::chroma_mv_1mv(bot_avg),
                    )
                } else {
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(mb_data.mvs[0]),
                        Self::chroma_mv_1mv(mb_data.mvs[2]),
                    )
                }
            } else if mb_data.mb_type == MbType::Inter4V {
                (
                    Self::chroma_mv_4mv(&mb_data.mvs),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            } else {
                (
                    Self::chroma_mv_1mv(mb_data.mvs[0]),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            }
        } else {
            (
                MotionVector::default(),
                MotionVector::default(),
                MotionVector::default(),
            )
        };

        // 色度仍使用半像素 MC, qpel 仅作用于亮度.
        let chroma_quarterpel = false;
        for plane_idx in 0..2 {
            let block_idx = 4 + plane_idx;

            // 组合 DC 和 AC 系数
            let mut block = [0i32; 64];
            block[0] = mb_data.dc_coeffs[block_idx] as i32;
            for i in 0..63 {
                block[i + 1] = mb_data.ac_coeffs[block_idx][i] as i32;
            }

            // 反量化和 IDCT
            self.dequantize(&mut block, mb_data.quant as u32, is_intra);
            idct_8x8(&mut block);

            // 写入帧
            for y in 0..8 {
                for x in 0..8 {
                    let px = mb_x as usize * 8 + x;
                    let py = mb_y as usize * 8 + y;

                    if px < uv_width && py < uv_height {
                        let idx = py * uv_width + px;
                        let residual = block[y * 8 + x];
                        let val = if is_intra {
                            residual.clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_chroma_line(
                                    py,
                                    mb_data.field_for_top,
                                    mb_data.field_for_bot,
                                );
                                let mv = if field_select {
                                    chroma_mv_top
                                } else {
                                    chroma_mv_bot
                                };
                                let mv_y = Self::scale_field_mv_y(mv.y);
                                Self::motion_compensate_field(
                                    ref_frame,
                                    plane_idx + 1,
                                    px as isize,
                                    py as isize,
                                    mv.x,
                                    mv_y,
                                    self.rounding_control,
                                    chroma_quarterpel,
                                    field_select,
                                )
                            } else {
                                Self::motion_compensate(
                                    ref_frame,
                                    plane_idx + 1,
                                    px as isize,
                                    py as isize,
                                    chroma_mv.x,
                                    chroma_mv.y,
                                    self.rounding_control,
                                    chroma_quarterpel,
                                )
                            };
                            (pred as i32 + residual).clamp(0, 255) as u8
                        } else {
                            (residual + 128).clamp(0, 255) as u8
                        };
                        frame.data[plane_idx + 1][idx] = val;
                    }
                }
            }
        }
    }

    /// 使用 Data Partitioning 模式解码 I/P 帧
    ///
    /// 三步解码流程:
    /// 1. 从 Partition A 解码所有 MB 的头部信息
    /// 2. 从 Partition B 解码所有 DC 系数
    /// 3. 从 Partition C 解码所有 AC 系数
    /// 4. 重建所有宏块到帧
    pub(super) fn decode_frame_partitioned(
        &mut self,
        packet_data: &[u8],
        part_info: &DataPartitionInfo,
        is_i_vop: bool,
    ) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        let mut frame = self.create_blank_frame(if is_i_vop {
            PictureType::I
        } else {
            PictureType::P
        });

        let mb_w = self.width.div_ceil(16) as usize;
        let mb_h = self.height.div_ceil(16) as usize;
        let total_mbs = mb_w * mb_h;

        trace!(
            "Data Partitioning 解码 {} 帧: {}x{} ({} MB)",
            if is_i_vop { "I" } else { "P" },
            self.width,
            self.height,
            total_mbs
        );

        // 存储所有宏块的中间数据
        let mut mb_data_vec: Vec<Option<PartitionedMacroblockData>> = vec![None; total_mbs];

        // === 步骤 1: 从 Partition A 解码所有 MB 头部 ===
        trace!("  步骤 1: 解码 Partition A (MB 头部)");
        let partition_a_bytes = part_info.partition_a.0 / 8;
        let partition_a_len = (part_info.partition_a.1 - part_info.partition_a.0).div_ceil(8);

        if partition_a_bytes + partition_a_len <= packet_data.len() {
            let partition_a_data =
                &packet_data[partition_a_bytes..partition_a_bytes + partition_a_len];
            let mut reader_a = BitReader::new(partition_a_data);

            for mb_y in 0..mb_h {
                for mb_x in 0..mb_w {
                    let mb_idx = mb_y * mb_w + mb_x;
                    if let Some(mb_data) = self.decode_partition_a_mb_header(
                        &mut reader_a,
                        mb_x as u32,
                        mb_y as u32,
                        is_i_vop,
                    ) {
                        mb_data_vec[mb_idx] = Some(mb_data);
                    } else {
                        debug!("  Partition A 解码失败: MB ({}, {})", mb_x, mb_y);
                        // 使用标准顺序解码作为降级
                        return self.decode_frame_standard(packet_data, is_i_vop);
                    }
                }
            }
        } else {
            debug!("  Partition A 数据不足，降级到标准解码");
            return self.decode_frame_standard(packet_data, is_i_vop);
        }

        // === 步骤 2: 从 Partition B 解码所有 DC 系数 ===
        trace!("  步骤 2: 解码 Partition B (DC 系数)");
        if part_info.partition_b.0 < part_info.partition_b.1 {
            let partition_b_bytes = part_info.partition_b.0 / 8;
            let partition_b_len = (part_info.partition_b.1 - part_info.partition_b.0).div_ceil(8);

            if partition_b_bytes + partition_b_len <= packet_data.len() {
                let partition_b_data =
                    &packet_data[partition_b_bytes..partition_b_bytes + partition_b_len];
                let mut reader_b = BitReader::new(partition_b_data);

                for mb_y in 0..mb_h {
                    for mb_x in 0..mb_w {
                        let mb_idx = mb_y * mb_w + mb_x;
                        if let Some(ref mut mb_data) = mb_data_vec[mb_idx] {
                            if !self.decode_partition_b_dc(
                                &mut reader_b,
                                mb_data,
                                mb_x as u32,
                                mb_y as u32,
                            ) {
                                debug!(
                                    "  Partition B 解码失败: MB ({}, {}), 降级到标准解码",
                                    mb_x, mb_y
                                );
                                return self.decode_frame_standard(packet_data, is_i_vop);
                            }
                        }
                    }
                }
            }
        }

        // === 步骤 3: 从 Partition C 解码所有 AC 系数 ===
        trace!("  步骤 3: 解码 Partition C (AC 系数)");
        if part_info.partition_c.0 < part_info.partition_c.1 {
            let partition_c_bytes = part_info.partition_c.0 / 8;
            let partition_c_len = (part_info.partition_c.1 - part_info.partition_c.0).div_ceil(8);

            if partition_c_bytes + partition_c_len <= packet_data.len() {
                let partition_c_data =
                    &packet_data[partition_c_bytes..partition_c_bytes + partition_c_len];
                let mut reader_c = BitReader::new(partition_c_data);

                for mb_y in 0..mb_h {
                    for mb_x in 0..mb_w {
                        let mb_idx = mb_y * mb_w + mb_x;
                        if let Some(ref mut mb_data) = mb_data_vec[mb_idx] {
                            if !self.decode_partition_c_ac(
                                &mut reader_c,
                                mb_data,
                                mb_x as u32,
                                mb_y as u32,
                            ) {
                                debug!(
                                    "  Partition C 解码失败: MB ({}, {}), 使用零 AC",
                                    mb_x, mb_y
                                );
                                // 继续处理，使用零 AC 系数
                            }
                        }
                    }
                }
            }
        }

        // === 步骤 4: 重建所有宏块 ===
        trace!("  步骤 4: 重建宏块到帧");
        for mb_y in 0..mb_h {
            for mb_x in 0..mb_w {
                let mb_idx = mb_y * mb_w + mb_x;
                if let Some(ref mb_data) = mb_data_vec[mb_idx] {
                    self.reconstruct_partitioned_macroblock(
                        &mut frame,
                        mb_x as u32,
                        mb_y as u32,
                        mb_data,
                    );
                }
            }
        }

        trace!("  Data Partitioning 解码完成");
        Ok(frame)
    }
}
