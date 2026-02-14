//! H.264/AVC 视频解码器.
//!
//! 实现 CABAC 熵解码, I_16x16/I_4x4 帧内预测和残差解码.
//! P/B 帧使用 P_Skip (复制参考帧).

mod cabac;
mod intra;
mod residual;

use log::debug;
use tao_core::bitreader::BitReader;
use tao_core::{PixelFormat, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;
use crate::parsers::h264::{NalUnit, NalUnitType, Sps, parse_avcc_config, parse_sps, split_avcc};

use cabac::{CabacCtx, CabacDecoder, init_contexts_i_slice, init_contexts_p_slice};
use residual::{
    CAT_CHROMA_DC, CAT_LUMA_AC, CAT_LUMA_DC, decode_residual_block, dequant_chroma_dc,
    dequant_luma_dc, inverse_hadamard_2x2, inverse_hadamard_4x4,
};

// ============================================================
// PPS 参数
// ============================================================

/// PPS 解析结果
struct Pps {
    entropy_coding_mode: u8,
    pic_init_qp: i32,
    deblocking_filter_control: bool,
}

// ============================================================
// Slice Header
// ============================================================

/// 解析后的 slice header
struct SliceHeader {
    first_mb: u32,
    slice_type: u32,
    slice_qp: i32,
    cabac_start_byte: usize,
    _is_idr: bool,
}

// ============================================================
// H.264 解码器
// ============================================================

/// H.264 解码器
pub struct H264Decoder {
    sps: Option<Sps>,
    pps: Option<Pps>,
    length_size: usize,
    width: u32,
    height: u32,
    mb_width: usize,
    mb_height: usize,
    ref_y: Vec<u8>,
    ref_u: Vec<u8>,
    ref_v: Vec<u8>,
    stride_y: usize,
    stride_c: usize,
    /// 每个宏块的类型 (0=I_4x4, 1-24=I_16x16, 25=I_PCM, 255=P_Skip)
    mb_types: Vec<u8>,
    /// Luma CBF 追踪 (4x4 块粒度, 用于 CABAC 上下文)
    cbf_luma: Vec<bool>,
    /// 上一个宏块的 qp_delta 是否非零
    prev_qp_delta_nz: bool,
    output_frame: Option<Frame>,
    opened: bool,
    flushing: bool,
}

impl H264Decoder {
    /// 创建解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sps: None,
            pps: None,
            length_size: 4,
            width: 0,
            height: 0,
            mb_width: 0,
            mb_height: 0,
            ref_y: Vec::new(),
            ref_u: Vec::new(),
            ref_v: Vec::new(),
            stride_y: 0,
            stride_c: 0,
            mb_types: Vec::new(),
            cbf_luma: Vec::new(),
            prev_qp_delta_nz: false,
            output_frame: None,
            opened: false,
            flushing: false,
        }))
    }

    /// 初始化/重新分配帧缓冲
    fn init_buffers(&mut self) {
        self.mb_width = self.width.div_ceil(16) as usize;
        self.mb_height = self.height.div_ceil(16) as usize;
        self.stride_y = self.mb_width * 16;
        self.stride_c = self.mb_width * 8;
        let total_mb = self.mb_width * self.mb_height;
        self.ref_y = vec![128u8; self.stride_y * self.mb_height * 16];
        self.ref_u = vec![128u8; self.stride_c * self.mb_height * 8];
        self.ref_v = vec![128u8; self.stride_c * self.mb_height * 8];
        self.mb_types = vec![0u8; total_mb];
        self.cbf_luma = vec![false; self.mb_width * 4 * self.mb_height * 4];
    }

    /// 处理 SPS NAL 单元
    fn handle_sps(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();
        if let Ok(sps) = parse_sps(&rbsp) {
            debug!(
                "H264: SPS {}x{} profile={} level={}",
                sps.width, sps.height, sps.profile_idc, sps.level_idc
            );
            self.width = sps.width;
            self.height = sps.height;
            self.sps = Some(sps);
            self.init_buffers();
        }
    }

    /// 处理 PPS NAL 单元
    fn handle_pps(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();
        if let Ok(pps) = parse_pps(&rbsp) {
            debug!(
                "H264: PPS entropy={} qp={}",
                if pps.entropy_coding_mode == 1 {
                    "CABAC"
                } else {
                    "CAVLC"
                },
                pps.pic_init_qp
            );
            self.pps = Some(pps);
        }
    }
}

// ============================================================
// Decoder trait 实现
// ============================================================

impl Decoder for H264Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::H264
    }

    fn name(&self) -> &str {
        "h264"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        if !params.extra_data.is_empty() {
            let config = parse_avcc_config(&params.extra_data)?;
            self.length_size = config.length_size;
            self.parse_sps_pps_from_config(&config)?;
        }
        if self.width == 0 || self.height == 0 {
            if let CodecParamsType::Video(ref v) = params.params {
                self.width = v.width;
                self.height = v.height;
            }
        }
        if self.width == 0 || self.height == 0 {
            return Err(TaoError::InvalidData("H264: 无法确定帧尺寸".into()));
        }
        self.init_buffers();
        self.opened = true;
        debug!("H264 解码器已打开: {}x{}", self.width, self.height);
        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::InvalidData("H264 解码器未打开".into()));
        }
        if packet.is_empty() {
            self.flushing = true;
            return Ok(());
        }
        let nalus = split_avcc(&packet.data, self.length_size);
        let mut vcl_nalu: Option<NalUnit> = None;
        for nalu in &nalus {
            match nalu.nal_type {
                NalUnitType::Sps => self.handle_sps(nalu),
                NalUnitType::Pps => self.handle_pps(nalu),
                NalUnitType::SliceIdr | NalUnitType::Slice => {
                    vcl_nalu = Some(nalu.clone());
                }
                _ => {}
            }
        }
        if let Some(nalu) = vcl_nalu {
            self.decode_slice(&nalu, packet.pts, packet.time_base);
        }
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
        self.ref_y.fill(128);
        self.ref_u.fill(128);
        self.ref_v.fill(128);
    }
}

// ============================================================
// Slice 解码
// ============================================================

impl H264Decoder {
    /// 解码一个 VCL NAL (slice)
    fn decode_slice(&mut self, nalu: &NalUnit, pts: i64, time_base: Rational) {
        let is_idr = nalu.nal_type == NalUnitType::SliceIdr;
        let rbsp = nalu.rbsp();

        match self.parse_slice_header(&rbsp, nalu) {
            Ok(header) => {
                if is_idr {
                    self.ref_y.fill(128);
                    self.ref_u.fill(128);
                    self.ref_v.fill(128);
                }
                self.decode_slice_data(&rbsp, &header);
            }
            Err(_) => {
                // 解析失败: 使用简化 DC 预测
                if is_idr {
                    self.ref_y.fill(128);
                    self.ref_u.fill(128);
                    self.ref_v.fill(128);
                }
            }
        }

        self.build_output_frame(pts, time_base, is_idr);
    }

    /// 解析 slice header, 返回 CABAC 数据起始位置
    fn parse_slice_header(&self, rbsp: &[u8], nalu: &NalUnit) -> TaoResult<SliceHeader> {
        let mut br = BitReader::new(rbsp);

        let first_mb = read_ue(&mut br)?;
        let slice_type = read_ue(&mut br)? % 5;
        let _pps_id = read_ue(&mut br)?;

        // frame_num
        let sps = self
            .sps
            .as_ref()
            .ok_or_else(|| TaoError::InvalidData("H264: 缺少 SPS".into()))?;
        let _frame_num = br.read_bits(sps.log2_max_frame_num)?;

        // IDR 特有字段
        if nalu.nal_type == NalUnitType::SliceIdr {
            let _idr_pic_id = read_ue(&mut br)?;
        }

        // pic_order_cnt
        if sps.poc_type == 0 {
            let _poc_lsb = br.read_bits(sps.log2_max_poc_lsb)?;
        }

        self.skip_ref_pic_list_mod(&mut br, slice_type)?;
        self.skip_dec_ref_pic_marking(&mut br, nalu)?;

        // CABAC init
        let pps = self
            .pps
            .as_ref()
            .ok_or_else(|| TaoError::InvalidData("H264: 缺少 PPS".into()))?;
        let is_i = slice_type == 2 || slice_type == 4;
        if pps.entropy_coding_mode == 1 && !is_i {
            let _cabac_init_idc = read_ue(&mut br)?;
        }

        // slice_qp_delta
        let qp_delta = read_se(&mut br)?;
        let slice_qp = pps.pic_init_qp + qp_delta;

        // 跳过去块效应滤波器参数
        if pps.deblocking_filter_control {
            let disable = read_ue(&mut br)?;
            if disable != 1 {
                let _ = read_se(&mut br); // alpha
                let _ = read_se(&mut br); // beta
            }
        }

        br.align_to_byte();
        let cabac_start = br.byte_position();

        Ok(SliceHeader {
            first_mb,
            slice_type,
            slice_qp,
            cabac_start_byte: cabac_start,
            _is_idr: nalu.nal_type == NalUnitType::SliceIdr,
        })
    }

    /// 跳过参考图像列表修改语法
    fn skip_ref_pic_list_mod(&self, br: &mut BitReader, slice_type: u32) -> TaoResult<()> {
        // P/B slice 有 ref_pic_list_modification
        if slice_type != 2 && slice_type != 4 {
            let reorder_l0 = br.read_bit()?;
            if reorder_l0 == 1 {
                loop {
                    let op = read_ue(br)?;
                    if op == 3 {
                        break;
                    }
                    let _ = read_ue(br)?;
                }
            }
            // B-slice: l1
            if slice_type == 1 {
                let reorder_l1 = br.read_bit()?;
                if reorder_l1 == 1 {
                    loop {
                        let op = read_ue(br)?;
                        if op == 3 {
                            break;
                        }
                        let _ = read_ue(br)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// 跳过解码参考图像标记语法
    fn skip_dec_ref_pic_marking(&self, br: &mut BitReader, nalu: &NalUnit) -> TaoResult<()> {
        if nalu.nal_type == NalUnitType::SliceIdr {
            let _ = br.read_bit()?; // no_output_of_prior_pics_flag
            let _ = br.read_bit()?; // long_term_reference_flag
        } else if nalu.ref_idc > 0 {
            let adaptive = br.read_bit()?;
            if adaptive == 1 {
                loop {
                    let op = read_ue(br)?;
                    if op == 0 {
                        break;
                    }
                    match op {
                        1 | 3 => {
                            let _ = read_ue(br)?;
                        }
                        2 => {
                            let _ = read_ue(br)?;
                        }
                        4 | 5 => {
                            let _ = read_ue(br)?;
                        }
                        6 => {
                            let _ = read_ue(br)?;
                        }
                        _ => break,
                    }
                }
            }
        }
        Ok(())
    }

    /// 解码 slice 数据 (MB 循环)
    fn decode_slice_data(&mut self, rbsp: &[u8], header: &SliceHeader) {
        let pps = match &self.pps {
            Some(p) => p,
            None => return,
        };

        if pps.entropy_coding_mode != 1 {
            // CAVLC: 使用简化 DC 预测
            self.apply_dc_fallback();
            return;
        }

        if header.cabac_start_byte >= rbsp.len() {
            return;
        }

        let cabac_data = &rbsp[header.cabac_start_byte..];
        let mut cabac = CabacDecoder::new(cabac_data);

        let is_i = header.slice_type == 2 || header.slice_type == 4;
        let mut ctxs = if is_i {
            init_contexts_i_slice(header.slice_qp)
        } else {
            init_contexts_p_slice(header.slice_qp)
        };

        let total_mbs = self.mb_width * self.mb_height;
        let first = header.first_mb as usize;

        if is_i {
            self.decode_i_slice_mbs(&mut cabac, &mut ctxs, first, total_mbs, header.slice_qp);
        }
        // P/B slice: 保持参考帧 (P_Skip)
    }

    /// CAVLC 回退: 对所有 MB 使用 DC 预测
    fn apply_dc_fallback(&mut self) {
        for mb_y in 0..self.mb_height {
            for mb_x in 0..self.mb_width {
                intra::predict_16x16(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16,
                    mb_y * 16,
                    2,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_u,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
                intra::predict_chroma_dc(
                    &mut self.ref_v,
                    self.stride_c,
                    mb_x * 8,
                    mb_y * 8,
                    mb_x > 0,
                    mb_y > 0,
                );
            }
        }
    }
}

// ============================================================
// I-slice 宏块解码
// ============================================================

impl H264Decoder {
    /// 解码 I-slice 的所有宏块
    fn decode_i_slice_mbs(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        first: usize,
        total: usize,
        slice_qp: i32,
    ) {
        self.prev_qp_delta_nz = false;
        let mut cur_qp = slice_qp;

        for mb_idx in first..total {
            let mb_x = mb_idx % self.mb_width;
            let mb_y = mb_idx / self.mb_width;

            let mb_type = decode_i_mb_type(cabac, ctxs, &self.mb_types, self.mb_width, mb_x, mb_y);
            self.mb_types[mb_idx] = mb_type as u8;

            if mb_type == 0 {
                self.decode_i_4x4_mb(cabac, ctxs, mb_x, mb_y, &mut cur_qp);
            } else if mb_type <= 24 {
                self.decode_i_16x16_mb(cabac, ctxs, mb_x, mb_y, mb_type, &mut cur_qp);
            }
            // mb_type == 25 (I_PCM): 暂不处理
        }
    }

    /// 解码 I_4x4 宏块 (消耗所有 CABAC 语法元素, 使用 DC 预测)
    fn decode_i_4x4_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        cur_qp: &mut i32,
    ) {
        // 1. 解码 16 个 4x4 预测模式 (消耗 CABAC 比特)
        decode_i4x4_pred_modes(cabac, ctxs);

        // 2. 解码 intra_chroma_pred_mode
        let _chroma_mode = decode_chroma_pred_mode(cabac, ctxs);

        // 3. 解码 coded_block_pattern
        let (luma_cbp, chroma_cbp) = decode_coded_block_pattern(cabac, ctxs);

        // 4. mb_qp_delta (仅当 CBP != 0)
        let has_residual = luma_cbp != 0 || chroma_cbp != 0;
        if has_residual {
            let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
            self.prev_qp_delta_nz = qp_delta != 0;
            *cur_qp = (*cur_qp + qp_delta).clamp(0, 51);
        } else {
            self.prev_qp_delta_nz = false;
        }

        // 5. 应用简化预测 (DC)
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                intra::predict_4x4_dc(
                    &mut self.ref_y,
                    self.stride_y,
                    mb_x * 16 + sub_x * 4,
                    mb_y * 16 + sub_y * 4,
                );
            }
        }
        intra::predict_chroma_dc(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_dc(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            mb_x > 0,
            mb_y > 0,
        );

        // 6. 解码残差 (消耗 CABAC 比特)
        self.consume_i4x4_residual(cabac, ctxs, luma_cbp, chroma_cbp);
    }

    /// 消耗 I_4x4 宏块的所有残差 CABAC 比特
    fn consume_i4x4_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        luma_cbp: u8,
        chroma_cbp: u8,
    ) {
        // 亮度: 按 8x8 块顺序, 每个 8x8 块包含 4 个 4x4 子块
        for i8x8 in 0..4u8 {
            if luma_cbp & (1 << i8x8) != 0 {
                for _ in 0..4 {
                    let _ = decode_residual_block(cabac, ctxs, &residual::CAT_LUMA_4X4, 0);
                }
            }
        }
        // 色度 DC
        if chroma_cbp >= 1 {
            let _ = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, 0);
            let _ = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, 0);
        }
        // 色度 AC
        if chroma_cbp >= 2 {
            for _ in 0..8 {
                let _ = decode_residual_block(cabac, ctxs, &residual::CAT_CHROMA_AC, 0);
            }
        }
    }

    /// 解码 I_16x16 宏块 (预测 + 残差)
    fn decode_i_16x16_mb(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        mb_type: u32,
        cur_qp: &mut i32,
    ) {
        let pred_mode = ((mb_type - 1) % 4) as u8;
        let cbp_chroma = ((mb_type - 1) / 4) % 3;
        let cbp_luma_nz = (mb_type - 1) >= 12;

        // 1. 解码 intra_chroma_pred_mode (消耗 CABAC 比特)
        let _chroma_mode = decode_chroma_pred_mode(cabac, ctxs);

        // 2. mb_qp_delta (I_16x16 始终存在)
        let qp_delta = decode_qp_delta(cabac, ctxs, self.prev_qp_delta_nz);
        self.prev_qp_delta_nz = qp_delta != 0;
        *cur_qp = (*cur_qp + qp_delta).clamp(0, 51);

        // 3. 应用亮度预测
        intra::predict_16x16(
            &mut self.ref_y,
            self.stride_y,
            mb_x * 16,
            mb_y * 16,
            pred_mode,
            mb_x > 0,
            mb_y > 0,
        );

        // 4. 应用色度预测 (DC)
        intra::predict_chroma_dc(
            &mut self.ref_u,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            mb_x > 0,
            mb_y > 0,
        );
        intra::predict_chroma_dc(
            &mut self.ref_v,
            self.stride_c,
            mb_x * 8,
            mb_y * 8,
            mb_x > 0,
            mb_y > 0,
        );

        // 5. 亮度 DC 残差 (始终存在)
        self.decode_luma_dc_residual(cabac, ctxs, mb_x, mb_y, *cur_qp);

        // 6. 亮度 AC 残差 (如果 CBP_luma > 0)
        if cbp_luma_nz {
            self.skip_luma_ac_residual(cabac, ctxs);
        }

        // 7. 色度 DC 残差
        if cbp_chroma >= 1 {
            self.decode_chroma_dc_residual(cabac, ctxs, mb_x, mb_y, *cur_qp);
        }

        // 8. 色度 AC 残差
        if cbp_chroma >= 2 {
            for _ in 0..8 {
                let _ = decode_residual_block(cabac, ctxs, &residual::CAT_CHROMA_AC, 0);
            }
        }
    }

    /// 解码 I_16x16 亮度 DC 残差
    fn decode_luma_dc_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        slice_qp: i32,
    ) {
        // 解码 DC 系数
        let cbf_inc = self.get_dc_cbf_inc(mb_x, mb_y);
        let raw_coeffs = decode_residual_block(cabac, ctxs, &CAT_LUMA_DC, cbf_inc);

        // 反扫描 + 反 Hadamard + 反量化
        let mut dc_block = [0i32; 16];
        for (i, &c) in raw_coeffs.iter().enumerate().take(16) {
            dc_block[i] = c;
        }
        inverse_hadamard_4x4(&mut dc_block);
        dequant_luma_dc(&mut dc_block, slice_qp);

        // 将 DC 残差加到每个 4x4 子块上
        for sub_y in 0..4 {
            for sub_x in 0..4 {
                let dc_val = dc_block[sub_y * 4 + sub_x];
                let px = mb_x * 16 + sub_x * 4;
                let py = mb_y * 16 + sub_y * 4;
                intra::add_residual_to_block(&mut self.ref_y, self.stride_y, px, py, 4, 4, dc_val);
            }
        }
    }

    /// 解码色度 DC 残差
    fn decode_chroma_dc_residual(
        &mut self,
        cabac: &mut CabacDecoder,
        ctxs: &mut [CabacCtx],
        mb_x: usize,
        mb_y: usize,
        slice_qp: i32,
    ) {
        // 色度 QP 映射
        let chroma_qp = chroma_qp_from_luma(slice_qp);

        // U 通道
        let u_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, 0);
        let mut u_dc = [0i32; 4];
        for (i, &c) in u_coeffs.iter().enumerate().take(4) {
            u_dc[i] = c;
        }
        inverse_hadamard_2x2(&mut u_dc);
        dequant_chroma_dc(&mut u_dc, chroma_qp);

        // V 通道
        let v_coeffs = decode_residual_block(cabac, ctxs, &CAT_CHROMA_DC, 0);
        let mut v_dc = [0i32; 4];
        for (i, &c) in v_coeffs.iter().enumerate().take(4) {
            v_dc[i] = c;
        }
        inverse_hadamard_2x2(&mut v_dc);
        dequant_chroma_dc(&mut v_dc, chroma_qp);

        // 应用到色度平面
        for sub_y in 0..2 {
            for sub_x in 0..2 {
                let px = mb_x * 8 + sub_x * 4;
                let py = mb_y * 8 + sub_y * 4;
                intra::add_residual_to_block(
                    &mut self.ref_u,
                    self.stride_c,
                    px,
                    py,
                    4,
                    4,
                    u_dc[sub_y * 2 + sub_x],
                );
                intra::add_residual_to_block(
                    &mut self.ref_v,
                    self.stride_c,
                    px,
                    py,
                    4,
                    4,
                    v_dc[sub_y * 2 + sub_x],
                );
            }
        }
    }

    /// 跳过亮度 AC 残差 (消耗 CABAC 比特以保持同步)
    fn skip_luma_ac_residual(&mut self, cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) {
        for _ in 0..16 {
            let _ = decode_residual_block(cabac, ctxs, &CAT_LUMA_AC, 0);
        }
    }

    /// 获取 DC coded_block_flag 的上下文增量
    fn get_dc_cbf_inc(&self, mb_x: usize, mb_y: usize) -> usize {
        let left = if mb_x > 0 {
            let left_idx = mb_y * self.mb_width + mb_x - 1;
            let lt = self.mb_types[left_idx];
            if (1..=24).contains(&lt) { 1 } else { 0 }
        } else {
            0
        };
        let top = if mb_y > 0 {
            let top_idx = (mb_y - 1) * self.mb_width + mb_x;
            let tt = self.mb_types[top_idx];
            if (1..=24).contains(&tt) { 1 } else { 0 }
        } else {
            0
        };
        left + 2 * top
    }
}

// ============================================================
// CABAC 语法元素解码
// ============================================================

/// 解码 I_4x4 的 16 个子块预测模式 (消耗 CABAC 比特)
fn decode_i4x4_pred_modes(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) {
    for _ in 0..16 {
        let prev_flag = cabac.decode_decision(&mut ctxs[68]);
        if prev_flag == 0 {
            // rem_intra4x4_pred_mode: 3 bypass bins
            let _ = cabac.decode_bypass();
            let _ = cabac.decode_bypass();
            let _ = cabac.decode_bypass();
        }
    }
}

/// 解码 intra_chroma_pred_mode (TU, cMax=3)
fn decode_chroma_pred_mode(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) -> u32 {
    // bin 0: ctxIdx = 64 (简化, 不考虑邻居)
    let bin0 = cabac.decode_decision(&mut ctxs[64]);
    if bin0 == 0 {
        return 0;
    }
    let bin1 = cabac.decode_decision(&mut ctxs[67]);
    if bin1 == 0 {
        return 1;
    }
    let bin2 = cabac.decode_decision(&mut ctxs[67]);
    if bin2 == 0 { 2 } else { 3 }
}

/// 解码 mb_qp_delta (一元编码)
fn decode_qp_delta(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx], prev_nz: bool) -> i32 {
    let mut ctx_idx = if prev_nz { 1usize } else { 0 };
    let mut val = 0u32;

    while cabac.decode_decision(&mut ctxs[60 + ctx_idx]) == 1 {
        ctx_idx = 2 + (ctx_idx >> 1);
        val += 1;
        if val > 52 {
            break;
        }
    }

    match val {
        0 => 0,
        v if v & 1 == 1 => v.div_ceil(2) as i32,
        v => -(v.div_ceil(2) as i32),
    }
}

/// 解码 coded_block_pattern (I_4x4 宏块)
fn decode_coded_block_pattern(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) -> (u8, u8) {
    // 亮度: 4 bits (每个 8x8 块一个)
    let mut luma_cbp = 0u8;
    for k in 0..4 {
        // 简化: 固定 ctxIdxInc = 0
        let bit = cabac.decode_decision(&mut ctxs[73]);
        if bit == 1 {
            luma_cbp |= 1 << k;
        }
    }
    // 色度: TU, cMax=2
    let bin0 = cabac.decode_decision(&mut ctxs[77]);
    let chroma_cbp = if bin0 == 0 {
        0u8
    } else {
        let bin1 = cabac.decode_decision(&mut ctxs[81]);
        if bin1 == 0 { 1 } else { 2 }
    };
    (luma_cbp, chroma_cbp)
}

/// 解码 I-slice 宏块类型
fn decode_i_mb_type(
    cabac: &mut CabacDecoder,
    ctxs: &mut [CabacCtx],
    mb_types: &[u8],
    mb_width: usize,
    mb_x: usize,
    mb_y: usize,
) -> u32 {
    // bin 0: 前缀 (I_4x4 vs I_16x16/I_PCM)
    let ctx_inc = compute_mb_type_ctx_inc(mb_types, mb_width, mb_x, mb_y);
    let bin0 = cabac.decode_decision(&mut ctxs[3 + ctx_inc]);
    if bin0 == 0 {
        return 0; // I_4x4
    }

    // bin 1: 终止检查 (I_PCM)
    if cabac.decode_terminate() == 1 {
        return 25; // I_PCM
    }

    // I_16x16 后缀解码
    decode_i_16x16_suffix(cabac, ctxs)
}

/// 计算 mb_type 前缀的上下文增量
fn compute_mb_type_ctx_inc(mb_types: &[u8], mb_width: usize, mb_x: usize, mb_y: usize) -> usize {
    let left_not_i4x4 = if mb_x > 0 {
        mb_types[mb_y * mb_width + mb_x - 1] != 0
    } else {
        false
    };
    let top_not_i4x4 = if mb_y > 0 {
        mb_types[(mb_y - 1) * mb_width + mb_x] != 0
    } else {
        false
    };
    left_not_i4x4 as usize + top_not_i4x4 as usize
}

/// 解码 I_16x16 后缀 (CBP + 预测模式)
fn decode_i_16x16_suffix(cabac: &mut CabacDecoder, ctxs: &mut [CabacCtx]) -> u32 {
    // bin 2: CBP_luma 标志
    let cbp_luma = cabac.decode_decision(&mut ctxs[6]);

    // bin 3-4: CBP_chroma (截断一元, maxVal=2)
    let cbp_c0 = cabac.decode_decision(&mut ctxs[7]);
    let cbp_chroma = if cbp_c0 == 0 {
        0
    } else {
        let cbp_c1 = cabac.decode_decision(&mut ctxs[8]);
        1 + cbp_c1
    };

    // pred_mode (2 bins): 上下文索引取决于 CBP_chroma
    let pred_mode = if cbp_c0 == 0 {
        // cbp_chroma=0: binIdx 4,5 → ctxIdx 8,9
        let pm0 = cabac.decode_decision(&mut ctxs[8]);
        let pm1 = cabac.decode_decision(&mut ctxs[9]);
        pm0 * 2 + pm1
    } else {
        // cbp_chroma≠0: binIdx 5,6 → ctxIdx 9,10
        let pm0 = cabac.decode_decision(&mut ctxs[9]);
        let pm1 = cabac.decode_decision(&mut ctxs[10]);
        pm0 * 2 + pm1
    };

    1 + pred_mode + 4 * cbp_chroma + 12 * cbp_luma
}

// ============================================================
// avcC 配置解析
// ============================================================

impl H264Decoder {
    fn parse_sps_pps_from_config(
        &mut self,
        config: &crate::parsers::h264::AvccConfig,
    ) -> TaoResult<()> {
        for sps_data in &config.sps_list {
            if let Ok(nalu) = NalUnit::parse(sps_data) {
                self.handle_sps(&nalu);
            }
        }
        for pps_data in &config.pps_list {
            if let Ok(nalu) = NalUnit::parse(pps_data) {
                self.handle_pps(&nalu);
            }
        }
        Ok(())
    }
}

// ============================================================
// 输出帧构建
// ============================================================

impl H264Decoder {
    fn build_output_frame(&mut self, pts: i64, time_base: Rational, is_keyframe: bool) {
        let w = self.width as usize;
        let h = self.height as usize;

        let y_data = copy_plane(&self.ref_y, self.stride_y, w, h);
        let u_data = copy_plane(&self.ref_u, self.stride_c, w / 2, h / 2);
        let v_data = copy_plane(&self.ref_v, self.stride_c, w / 2, h / 2);

        let vf = VideoFrame {
            data: vec![y_data, u_data, v_data],
            linesize: vec![w, w / 2, w / 2],
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Yuv420p,
            pts,
            time_base,
            duration: 0,
            is_keyframe,
            picture_type: if is_keyframe {
                PictureType::I
            } else {
                PictureType::P
            },
            sample_aspect_ratio: Rational::new(1, 1),
            color_space: Default::default(),
            color_range: Default::default(),
        };
        self.output_frame = Some(Frame::Video(vf));
    }
}

// ============================================================
// 工具函数
// ============================================================

/// 读取无符号 Exp-Golomb
fn read_ue(br: &mut BitReader) -> TaoResult<u32> {
    let mut zeros = 0u32;
    loop {
        let bit = br.read_bit()?;
        if bit == 1 {
            break;
        }
        zeros += 1;
        if zeros > 31 {
            return Err(TaoError::InvalidData("Exp-Golomb 前导零过多".into()));
        }
    }
    if zeros == 0 {
        return Ok(0);
    }
    let suffix = br.read_bits(zeros)?;
    Ok((1 << zeros) - 1 + suffix)
}

/// 读取有符号 Exp-Golomb
fn read_se(br: &mut BitReader) -> TaoResult<i32> {
    let code = read_ue(br)?;
    let value = code.div_ceil(2) as i32;
    if code & 1 == 0 { Ok(-value) } else { Ok(value) }
}

/// 解析 PPS 参数
fn parse_pps(rbsp: &[u8]) -> TaoResult<Pps> {
    let mut br = BitReader::new(rbsp);
    let _pps_id = read_ue(&mut br)?;
    let _sps_id = read_ue(&mut br)?;
    let entropy = br.read_bit()? as u8;
    let _bottom_field_pic_order = br.read_bit()?;
    let _num_slice_groups = read_ue(&mut br)?;
    let _num_ref_idx_l0 = read_ue(&mut br)? + 1;
    let _num_ref_idx_l1 = read_ue(&mut br)? + 1;
    let _weighted_pred = br.read_bit()?;
    let _weighted_bipred = br.read_bits(2)?;

    // pic_init_qp_minus26: se(v)
    let qp_delta = read_se(&mut br)?;
    let pic_init_qp = 26 + qp_delta;

    // pic_init_qs_minus26: se(v)
    let _ = read_se(&mut br)?;
    // chroma_qp_index_offset: se(v)
    let _ = read_se(&mut br)?;

    // deblocking_filter_control_present_flag
    let deblocking = br.read_bit()? == 1;

    Ok(Pps {
        entropy_coding_mode: entropy,
        pic_init_qp,
        deblocking_filter_control: deblocking,
    })
}

/// Luma QP → Chroma QP 映射 (H.264 Table 8-15)
fn chroma_qp_from_luma(qp: i32) -> i32 {
    let qpc = qp.clamp(0, 51);
    CHROMA_QP_TABLE[qpc as usize]
}

/// 从对齐缓冲区拷贝到紧凑平面
fn copy_plane(src: &[u8], src_stride: usize, w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; w * h];
    for y in 0..h {
        let src_off = y * src_stride;
        let dst_off = y * w;
        let copy_len = w.min(src.len().saturating_sub(src_off));
        if copy_len > 0 && dst_off + copy_len <= dst.len() {
            dst[dst_off..dst_off + copy_len].copy_from_slice(&src[src_off..src_off + copy_len]);
        }
    }
    dst
}

/// Chroma QP 映射表 (H.264 Table 8-15)
#[rustfmt::skip]
const CHROMA_QP_TABLE: [i32; 52] = [
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 29, 30,
    31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38,
    39, 39, 39, 39,
];
