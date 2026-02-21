use super::*;

impl Mpeg4Decoder {
    pub(super) fn decode_frame_standard(
        &mut self,
        packet_data: &[u8],
        is_i_vop: bool,
    ) -> TaoResult<VideoFrame> {
        let vop_offset = find_start_code_offset(packet_data, START_CODE_VOP)
            .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
        let mut reader = BitReader::new(&packet_data[vop_offset..]);

        // 重新解析 VOP header
        let _ = self.parse_vop_header(&mut reader)?;

        if is_i_vop {
            self.decode_i_frame(&mut reader)
        } else {
            self.decode_p_frame(&mut reader)
        }
    }

    // ========================================================================
    // 宏块和帧解码
    // ========================================================================

    /// 解码单个宏块
    pub(super) fn decode_macroblock(
        &mut self,
        frame: &mut VideoFrame,
        mb_x: u32,
        mb_y: u32,
        reader: &mut BitReader,
        is_i_vop: bool,
    ) {
        let width = self.width as usize;
        let height = self.height as usize;
        let mb_idx = mb_y as usize * self.mb_stride + mb_x as usize;

        // P-VOP: not_coded 位
        if !is_i_vop {
            let not_coded = reader.read_bit().unwrap_or(false);
            if not_coded {
                self.copy_mb_from_ref(frame, mb_x, mb_y);
                if mb_idx < self.mv_cache.len() {
                    self.mv_cache[mb_idx] = [MotionVector::default(); 4];
                }
                // 更新宏块信息
                if mb_idx < self.mb_info.len() {
                    self.mb_info[mb_idx] = MacroblockInfo {
                        mode: MacroblockInfo::MODE_NOT_CODED,
                        quant: self.quant,
                        mvs: [MotionVector::default(); 4],
                    };
                }
                return;
            }
        }

        // 1. MCBPC
        let (mb_type, cbpc) = if is_i_vop {
            decode_mcbpc_i(reader).unwrap_or((MbType::Intra, 0))
        } else {
            decode_mcbpc_p(reader).unwrap_or((MbType::Inter, 0))
        };

        let is_intra = matches!(mb_type, MbType::Intra | MbType::IntraQ);

        // AC/DC prediction flag
        let ac_pred_flag = if is_intra {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };

        // 2. CBPY
        let cbpy = match decode_cbpy(reader, is_intra) {
            Some(val) => val,
            None => {
                // CBPY 解码失败 - 记录诊断信息
                trace!(
                    "分区宏块CBPY解码失败: 字节位置={}, mb_type={:?}, cbpc={}, is_intra={}",
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

        // 4. 隔行模式: field_dct 和 field_pred
        let interlacing = self
            .vol_info
            .as_ref()
            .map(|v| v.interlacing)
            .unwrap_or(false);
        let field_dct = if interlacing && (cbpy != 0 || cbpc != 0 || is_intra) {
            reader.read_bit().unwrap_or(false)
        } else {
            false
        };
        let mut field_pred = false;
        let mut field_for_top = false;
        let mut field_for_bot = false;
        if interlacing && !is_intra {
            field_pred = reader.read_bit().unwrap_or(false);
            if field_pred {
                // 场预测: 读取顶场和底场参考选择
                field_for_top = reader.read_bit().unwrap_or(false);
                field_for_bot = reader.read_bit().unwrap_or(false);
            }
        }

        // quarterpel 标志
        let quarterpel = self
            .vol_info
            .as_ref()
            .map(|v| v.quarterpel)
            .unwrap_or(false);
        let use_quarterpel = quarterpel;

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
                    if mb_idx < self.mv_cache.len() {
                        self.mv_cache[mb_idx][k] = *mv_slot;
                    }
                }
            } else if let Some(mut mv) = self.decode_motion_vector(reader, mb_x, mb_y, 0) {
                self.validate_vector(&mut mv, mb_x, mb_y);
                mb_mvs = [mv; 4];
            }
        }

        // 存储 MV
        if mb_idx < self.mv_cache.len() {
            self.mv_cache[mb_idx] = mb_mvs;
        }

        // 更新宏块信息
        if mb_idx < self.mb_info.len() {
            let mode_code = match mb_type {
                MbType::Inter | MbType::InterQ => MacroblockInfo::MODE_INTER,
                MbType::Intra | MbType::IntraQ => MacroblockInfo::MODE_INTRA,
                MbType::Inter4V => MacroblockInfo::MODE_INTER4V,
            };
            self.mb_info[mb_idx] = MacroblockInfo {
                mode: mode_code,
                quant: self.quant,
                mvs: mb_mvs,
            };
        }

        // 6. CBP 组合
        let cbp = (cbpy << 2) | cbpc;

        // 选择扫描表 (field_dct 使用 alternate vertical scan)
        let scan_table = if field_dct || self.alternate_vertical_scan {
            &ALTERNATE_VERTICAL_SCAN
        } else {
            &ZIGZAG_SCAN
        };

        // 7. 解码各 8x8 块 - Y 平面
        #[allow(clippy::needless_range_loop)]
        for block_idx in 0..4usize {
            let by = (block_idx / 2) as u32;
            let bx = (block_idx % 2) as u32;
            let ac_coded = (cbp >> (5 - block_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    0,
                    mb_x,
                    mb_y,
                    block_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                    scan_table,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, is_intra);
            idct_8x8(&mut block);

            let mv = if !is_intra {
                mb_mvs[block_idx]
            } else {
                MotionVector::default()
            };

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
                                    field_for_top,
                                    field_for_bot,
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

        // U/V 平面
        let uv_width = width / 2;
        let uv_height = height / 2;

        let (chroma_mv, chroma_mv_top, chroma_mv_bot) = if !is_intra {
            if field_pred {
                if mb_type == MbType::Inter4V {
                    let top_avg = Self::average_mv(mb_mvs[0], mb_mvs[1]);
                    let bot_avg = Self::average_mv(mb_mvs[2], mb_mvs[3]);
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(top_avg),
                        Self::chroma_mv_1mv(bot_avg),
                    )
                } else {
                    (
                        MotionVector::default(),
                        Self::chroma_mv_1mv(mb_mvs[0]),
                        Self::chroma_mv_1mv(mb_mvs[2]),
                    )
                }
            } else if mb_type == MbType::Inter4V {
                (
                    Self::chroma_mv_4mv(&mb_mvs),
                    MotionVector::default(),
                    MotionVector::default(),
                )
            } else {
                (
                    Self::chroma_mv_1mv(mb_mvs[0]),
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
        for plane_idx in 0..2usize {
            let ac_coded = (cbp >> (1 - plane_idx)) & 1 != 0;

            let mut block = if is_intra {
                decode_intra_block_vlc(
                    reader,
                    plane_idx + 1,
                    mb_x,
                    mb_y,
                    4 + plane_idx,
                    ac_pred_flag,
                    ac_coded,
                    self,
                    scan_table,
                )
                .unwrap_or([0; 64])
            } else if ac_coded {
                decode_inter_block_vlc(reader, scan_table).unwrap_or([0; 64])
            } else {
                [0i32; 64]
            };

            self.dequantize(&mut block, self.quant as u32, is_intra);
            idct_8x8(&mut block);

            for v in 0..8 {
                for u in 0..8 {
                    let px = (mb_x as usize * 8 + u) as isize;
                    let py = (mb_y as usize * 8 + v) as isize;
                    if px < uv_width as isize && py < uv_height as isize {
                        let idx = py as usize * uv_width + px as usize;
                        let residual = block[v * 8 + u];
                        let val = if is_intra {
                            residual.clamp(0, 255) as u8
                        } else if let Some(ref_frame) = &self.reference_frame {
                            let pred = if field_pred {
                                let field_select = Self::select_field_for_chroma_line(
                                    py as usize,
                                    field_for_top,
                                    field_for_bot,
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
                                    px,
                                    py,
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
                                    px,
                                    py,
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

    /// 初始化帧解码前的通用状态
    pub(super) fn init_frame_decode(&mut self) {
        let mb_count = self.mb_stride * (self.height as usize).div_ceil(16);
        let total_blocks = mb_count * 6;
        // 初始 DC 预测值: 2^(n-1) = 2^7 = 128 (对于 8 位精度的量化域 DC)
        // 根据 ITU-T H.263 和 ISO/IEC 14496-2，边界 DC 预测器 = 128
        self.predictor_cache = vec![[128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; total_blocks];

        // 确保 MV 缓存和宏块信息大小正确
        if self.mv_cache.len() != mb_count {
            self.mv_cache = vec![[MotionVector::default(); 4]; mb_count];
        }
        if self.mb_info.len() != mb_count {
            self.mb_info = vec![MacroblockInfo::default(); mb_count];
        }
    }

    /// 创建空白帧
    pub(super) fn create_blank_frame(&self, picture_type: PictureType) -> VideoFrame {
        let mut frame = VideoFrame::new(self.width, self.height, self.pixel_format);
        frame.picture_type = picture_type;
        frame.is_keyframe = picture_type == PictureType::I;

        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        frame.data[0] = vec![128u8; y_size];
        frame.data[1] = vec![128u8; uv_size];
        frame.data[2] = vec![128u8; uv_size];
        frame.linesize[0] = self.width as usize;
        frame.linesize[1] = (self.width / 2) as usize;
        frame.linesize[2] = (self.width / 2) as usize;
        frame
    }

    /// 解码 I 帧
    pub(super) fn decode_i_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        // 帧起始: slice 从 (0,0) 开始
        self.resync_mb_x = 0;
        self.resync_mb_y = 0;
        let mut frame = self.create_blank_frame(PictureType::I);

        let mb_w = self.width.div_ceil(16) as usize;
        let mb_h = self.height.div_ceil(16) as usize;
        trace!(
            "解码 I 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        let resync_disabled = self
            .vol_info
            .as_ref()
            .map(|v| v.resync_marker_disable)
            .unwrap_or(true);

        let total_mbs = mb_w * mb_h;
        let mut mb_idx = 0usize;
        while mb_idx < total_mbs {
            // 检查 resync marker (错误恢复)
            if !resync_disabled && Self::check_resync_marker(reader, 0) {
                if let Some((mb_num, new_quant)) = self.parse_video_packet_header(reader) {
                    debug!("I 帧 resync marker: MB={}, quant={}", mb_num, new_quant);
                    self.quant = new_quant;
                    let target = mb_num as usize;
                    if target < total_mbs && target >= mb_idx {
                        mb_idx = target;
                        // 更新 slice 起始位置, 用于 DC/AC/MV 预测边界处理
                        self.resync_mb_x = mb_idx % mb_w;
                        self.resync_mb_y = mb_idx / mb_w;
                    } else {
                        warn!("I 帧 resync marker 宏块号异常: {}", mb_num);
                    }
                }
            }

            let mb_x = (mb_idx % mb_w) as u32;
            let mb_y = (mb_idx / mb_w) as u32;
            self.decode_macroblock(&mut frame, mb_x, mb_y, reader, true);
            mb_idx += 1;
        }
        Ok(frame)
    }

    /// 解码 P 帧
    pub(super) fn decode_p_frame(&mut self, reader: &mut BitReader) -> TaoResult<VideoFrame> {
        self.init_frame_decode();
        // 帧起始: slice 从 (0,0) 开始
        self.resync_mb_x = 0;
        self.resync_mb_y = 0;
        let mut frame = self.create_blank_frame(PictureType::P);

        let mb_w = self.mb_stride;
        let mb_h = (self.height as usize).div_ceil(16);
        trace!(
            "解码 P 帧: {}x{} ({}x{} MB)",
            self.width, self.height, mb_w, mb_h
        );

        let resync_disabled = self
            .vol_info
            .as_ref()
            .map(|v| v.resync_marker_disable)
            .unwrap_or(true);
        let fcode = self.f_code_forward;

        let total_mbs = mb_w * mb_h;
        let mut mb_idx = 0usize;
        while mb_idx < total_mbs {
            // 检查 resync marker (错误恢复)
            if !resync_disabled && Self::check_resync_marker(reader, fcode.saturating_sub(1)) {
                if let Some((mb_num, new_quant)) = self.parse_video_packet_header(reader) {
                    debug!("P 帧 resync marker: MB={}, quant={}", mb_num, new_quant);
                    self.quant = new_quant;
                    let target = mb_num as usize;
                    if target < total_mbs && target >= mb_idx {
                        // 填充 gap: 跳过的 MB 从参考帧复制 (视为 not_coded)
                        for skip_idx in mb_idx..target {
                            let skip_x = (skip_idx % mb_w) as u32;
                            let skip_y = (skip_idx / mb_w) as u32;
                            self.copy_mb_from_ref(&mut frame, skip_x, skip_y);
                            // 清零 MV 缓存
                            if skip_idx < self.mv_cache.len() {
                                self.mv_cache[skip_idx] = [MotionVector::default(); 4];
                            }
                        }
                        mb_idx = target;
                        // 更新 slice 起始位置, 用于 DC/AC/MV 预测边界处理
                        self.resync_mb_x = mb_idx % mb_w;
                        self.resync_mb_y = mb_idx / mb_w;
                    } else {
                        warn!("P 帧 resync marker 宏块号异常: {}", mb_num);
                    }
                }
            }

            // 重新计算坐标 (可能因 resync 跳转而改变)
            let mb_x = (mb_idx % mb_w) as u32;
            let mb_y = (mb_idx / mb_w) as u32;
            self.decode_macroblock(&mut frame, mb_x, mb_y, reader, false);
            mb_idx += 1;
        }
        Ok(frame)
    }

    /// 从参考帧复制宏块
    pub(super) fn copy_mb_from_ref(&self, frame: &mut VideoFrame, mb_x: u32, mb_y: u32) {
        if let Some(ref_frame) = &self.reference_frame {
            let width = self.width as usize;
            let height = self.height as usize;

            for y in 0..16 {
                for x in 0..16 {
                    let px = (mb_x as usize * 16 + x).min(width - 1);
                    let py = (mb_y as usize * 16 + y).min(height - 1);
                    let idx = py * width + px;
                    frame.data[0][idx] = ref_frame.data[0][idx];
                }
            }

            let uv_w = width / 2;
            let uv_h = height / 2;
            for plane in 1..3 {
                for y in 0..8 {
                    for x in 0..8 {
                        let px = (mb_x as usize * 8 + x).min(uv_w - 1);
                        let py = (mb_y as usize * 8 + y).min(uv_h - 1);
                        let idx = py * uv_w + px;
                        frame.data[plane][idx] = ref_frame.data[plane][idx];
                    }
                }
            }
        }
    }
}
