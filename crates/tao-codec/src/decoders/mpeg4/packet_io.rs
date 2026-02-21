use super::*;

impl Mpeg4Decoder {
    /// 查找数据中所有 VOP 起始码的偏移 (起始码之后的位置)
    ///
    /// 用于 DivX packed bitstream 拆分: 一个 packet 中可能包含多个 VOP.
    /// 返回每个 VOP 起始码 (00 00 01 B6) 之后的字节偏移列表.
    pub(super) fn find_all_vop_offsets(data: &[u8]) -> Vec<usize> {
        let mut offsets = Vec::new();
        if data.len() < 4 {
            return offsets;
        }
        for idx in 0..(data.len() - 3) {
            if data[idx] == 0x00
                && data[idx + 1] == 0x00
                && data[idx + 2] == 0x01
                && data[idx + 3] == START_CODE_VOP
            {
                offsets.push(idx + 4);
            }
        }
        offsets
    }

    /// 标准解码路径 (非 Data Partitioning)
    pub(super) fn send_packet_standard(&mut self, packet: &Packet) -> TaoResult<()> {
        let vop_offset = find_start_code_offset(&packet.data, START_CODE_VOP)
            .ok_or_else(|| TaoError::InvalidData("未找到 VOP 起始码".into()))?;
        let mut reader = BitReader::new(&packet.data[vop_offset..]);

        let vop_info = self.parse_vop_header(&mut reader)?;

        // Seek 后等待关键帧: 丢弃非 I 帧避免花屏
        if self.wait_keyframe {
            if vop_info.picture_type == PictureType::I {
                self.wait_keyframe = false;
            } else {
                return Ok(());
            }
        }

        if !vop_info.vop_coded {
            if let Some(ref_frame) = &self.reference_frame {
                let mut frame = ref_frame.clone();
                frame.picture_type = vop_info.picture_type;
                frame.is_keyframe = vop_info.picture_type == PictureType::I;
                frame.pts = packet.pts;
                frame.time_base = packet.time_base;
                frame.duration = packet.duration;
                self.pending_frame = Some(frame);
                self.frame_count += 1;
            }
            return Ok(());
        }

        // S-VOP: 解析 GMC sprite trajectory
        if vop_info.is_sprite {
            let sprite_enable = self.vol_info.as_ref().map(|v| v.sprite_enable).unwrap_or(0);
            if sprite_enable == 2 {
                self.gmc_params = self.parse_sprite_trajectory(&mut reader);
            }
        }

        let mut frame = match vop_info.picture_type {
            PictureType::I => self.decode_i_frame(&mut reader)?,
            PictureType::P => self.decode_p_frame(&mut reader).unwrap_or_else(|_| {
                warn!("P 帧解码失败, 使用参考帧降级");
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::P;
                    f.is_keyframe = false;
                    f
                } else {
                    self.create_blank_frame(PictureType::P)
                }
            }),
            PictureType::B => {
                // B 帧需要两个参考帧
                if self.reference_frame.is_some() && self.backward_reference.is_some() {
                    self.decode_b_frame(&mut reader).unwrap_or_else(|e| {
                        warn!("B 帧解码失败: {:?}, 使用参考帧降级", e);
                        if let Some(ref_frame) = &self.reference_frame {
                            let mut f = ref_frame.clone();
                            f.picture_type = PictureType::B;
                            f.is_keyframe = false;
                            f
                        } else {
                            self.create_blank_frame(PictureType::B)
                        }
                    })
                } else if let Some(ref_frame) = &self.reference_frame {
                    warn!("B 帧缺少双参考帧, 使用前向参考帧降级");
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::B;
                    f.is_keyframe = false;
                    f
                } else {
                    warn!("B 帧缺少参考帧, 跳过");
                    return Ok(());
                }
            }
            PictureType::S => self.decode_p_frame(&mut reader).unwrap_or_else(|_| {
                warn!("S-VOP 解码失败, 使用参考帧降级");
                if let Some(ref_frame) = &self.reference_frame {
                    let mut f = ref_frame.clone();
                    f.picture_type = PictureType::S;
                    f.is_keyframe = false;
                    f
                } else {
                    self.create_blank_frame(PictureType::S)
                }
            }),
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "不支持的 VOP 类型: {:?}",
                    vop_info.picture_type
                )));
            }
        };

        if vop_info.picture_type == PictureType::S {
            frame.picture_type = PictureType::S;
            frame.is_keyframe = false;
        }

        frame.pts = packet.pts;
        frame.time_base = packet.time_base;
        frame.duration = packet.duration;

        // B 帧重排序逻辑
        if frame.picture_type == PictureType::B {
            // B 帧存入 DPB，等待下一个 I/P/S 帧时输出
            self.dpb.push(frame);
        } else {
            // I/P/S 帧：先输出 DPB 中的所有 B 帧，再输出当前帧
            // 将当前 I/P/S 帧也加入 DPB
            self.dpb.push(frame.clone());

            // I/P/S 帧: 更新参考帧和 MV 缓存
            if frame.picture_type == PictureType::I
                || frame.picture_type == PictureType::P
                || frame.picture_type == PictureType::S
            {
                // 保存当前 MV 缓存和宏块信息到参考帧缓存 (B 帧 Direct 模式需要)
                self.ref_mv_cache = self.mv_cache.clone();
                self.backward_reference = self.reference_frame.take();
                self.reference_frame = Some(frame);
            }
        }

        self.frame_count += 1;
        Ok(())
    }
}

// ============================================================================
// 测试
// ============================================================================
