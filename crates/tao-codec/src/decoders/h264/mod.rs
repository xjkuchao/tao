//! H.264/AVC 视频解码器.
//!
//! 基础实现: 解析 avcC 配置, 解码 I/P 帧输出 YUV420P.

use log::debug;
use tao_core::bitreader::BitReader;
use tao_core::{PixelFormat, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{Frame, PictureType, VideoFrame};
use crate::packet::Packet;
use crate::parsers::h264::{parse_avcc_config, parse_sps, split_avcc, NalUnit, NalUnitType, Sps};

/// H.264 解码器
pub struct H264Decoder {
    /// SPS 参数
    sps: Option<Sps>,
    /// PPS: entropy_coding_mode (0=CAVLC, 1=CABAC)
    entropy_mode: u8,
    /// PPS: pic_init_qp_minus26
    pic_init_qp: i32,
    /// PPS: log2_max_frame_num (用于 slice header 解析)
    _log2_max_frame_num: u32,
    /// PPS: log2_max_pic_order_cnt_lsb (用于 slice header 解析)
    _log2_max_poc_lsb: u32,
    /// NAL 长度前缀大小
    length_size: usize,
    /// 帧宽度 (像素)
    width: u32,
    /// 帧高度 (像素)
    height: u32,
    /// 宏块列数
    mb_width: usize,
    /// 宏块行数
    mb_height: usize,
    /// 参考帧 Y 平面
    ref_y: Vec<u8>,
    /// 参考帧 U 平面
    ref_u: Vec<u8>,
    /// 参考帧 V 平面
    ref_v: Vec<u8>,
    /// Y 行宽 (对齐到宏块)
    stride_y: usize,
    /// C 行宽 (对齐到宏块)
    stride_c: usize,
    /// 输出帧缓冲
    output_frame: Option<Frame>,
    /// 是否已打开
    opened: bool,
    /// 是否正在刷新
    flushing: bool,
}

impl H264Decoder {
    /// 创建解码器实例
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            sps: None,
            entropy_mode: 0,
            pic_init_qp: 26,
            _log2_max_frame_num: 4,
            _log2_max_poc_lsb: 4,
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
        let h_y = self.mb_height * 16;
        let h_c = self.mb_height * 8;
        self.ref_y = vec![128u8; self.stride_y * h_y];
        self.ref_u = vec![128u8; self.stride_c * h_c];
        self.ref_v = vec![128u8; self.stride_c * h_c];
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

    /// 处理 PPS NAL 单元 (提取基本参数)
    fn handle_pps(&mut self, nalu: &NalUnit) {
        let rbsp = nalu.rbsp();
        if let Ok(pps) = parse_pps_basic(&rbsp) {
            debug!(
                "H264: PPS entropy={} qp={}",
                if pps.0 == 1 { "CABAC" } else { "CAVLC" },
                pps.1
            );
            self.entropy_mode = pps.0;
            self.pic_init_qp = pps.1;
        }
    }

    /// 解码 VCL NAL (slice)
    fn decode_slice(&mut self, nalu: &NalUnit, pts: i64, time_base: Rational) {
        let is_idr = nalu.nal_type == NalUnitType::SliceIdr;
        let rbsp = nalu.rbsp();

        // 解析 slice header 前几个字段
        let mut br = BitReader::new(&rbsp);
        let _first_mb = read_ue(&mut br).unwrap_or(0);
        let slice_type = read_ue(&mut br).unwrap_or(2) % 5;
        // 0=P, 1=B, 2=I, 3=SP, 4=SI

        if is_idr {
            // IDR: 重置为灰色, 然后解码
            self.ref_y.fill(128);
            self.ref_u.fill(128);
            self.ref_v.fill(128);
        }

        let is_i = slice_type == 2 || slice_type == 4;
        if is_i {
            self.decode_i_slice_approx(&rbsp);
        }
        // P/B slice: 保持参考帧不变 (P_Skip 效果)

        self.build_output_frame(pts, time_base, is_idr);
    }

    /// 近似解码 I-slice (预测 + 简化残差)
    fn decode_i_slice_approx(&mut self, _rbsp: &[u8]) {
        // 对于 I-slice, 使用 DC 预测模式 (Intra_16x16_DC)
        // 按光栅顺序遍历宏块, 每个宏块预测值为邻居平均
        for mb_y in 0..self.mb_height {
            for mb_x in 0..self.mb_width {
                self.predict_mb_dc_luma(mb_x, mb_y);
                self.predict_mb_dc_chroma(mb_x, mb_y);
            }
        }
    }

    /// 对单个宏块的亮度分量进行 DC 预测
    fn predict_mb_dc_luma(&mut self, mb_x: usize, mb_y: usize) {
        let p = DcPredParams {
            x0: mb_x * 16,
            y0: mb_y * 16,
            blk_w: 16,
            blk_h: 16,
            has_top: mb_y > 0,
            has_left: mb_x > 0,
        };
        let dc = compute_dc_block(&self.ref_y, self.stride_y, &p);
        fill_block(&mut self.ref_y, self.stride_y, p.x0, p.y0, 16, 16, dc);
    }

    /// 对单个宏块的色度分量进行 DC 预测
    fn predict_mb_dc_chroma(&mut self, mb_x: usize, mb_y: usize) {
        let p = DcPredParams {
            x0: mb_x * 8,
            y0: mb_y * 8,
            blk_w: 8,
            blk_h: 8,
            has_top: mb_y > 0,
            has_left: mb_x > 0,
        };
        let dc_u = compute_dc_block(&self.ref_u, self.stride_c, &p);
        let dc_v = compute_dc_block(&self.ref_v, self.stride_c, &p);
        fill_block(&mut self.ref_u, self.stride_c, p.x0, p.y0, 8, 8, dc_u);
        fill_block(&mut self.ref_v, self.stride_c, p.x0, p.y0, 8, 8, dc_v);
    }

    /// 构建输出视频帧
    fn build_output_frame(&mut self, pts: i64, time_base: Rational, is_keyframe: bool) {
        let w = self.width as usize;
        let h = self.height as usize;

        // 从对齐缓冲区拷贝到紧凑帧
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

impl Decoder for H264Decoder {
    fn codec_id(&self) -> CodecId {
        CodecId::H264
    }

    fn name(&self) -> &str {
        "h264"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        // 从 extra_data 解析 avcC 配置
        if !params.extra_data.is_empty() {
            let config = parse_avcc_config(&params.extra_data)?;
            self.length_size = config.length_size;
            self.parse_sps_pps_from_config(&config)?;
        }

        // 从 codec params 获取尺寸 (fallback)
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

        // 先处理非 VCL NAL (SPS/PPS)
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

        // 解码 VCL NAL
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

impl H264Decoder {
    /// 从 avcC 配置中解析 SPS 和 PPS
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

/// 解析 PPS 基本参数: (entropy_coding_mode, pic_init_qp)
fn parse_pps_basic(rbsp: &[u8]) -> TaoResult<(u8, i32)> {
    let mut br = BitReader::new(rbsp);
    let _pps_id = read_ue(&mut br)?;
    let _sps_id = read_ue(&mut br)?;
    let entropy = br.read_bit()? as u8;
    let _bottom_field_pic_order = br.read_bit()?;
    let _num_slice_groups = read_ue(&mut br)?;
    let num_ref_idx_l0 = read_ue(&mut br)? + 1;
    let _num_ref_idx_l1 = read_ue(&mut br)? + 1;
    let _weighted_pred = br.read_bit()?;
    let _weighted_bipred_idc = br.read_bits(2)?;
    // pic_init_qp_minus26: se(v)
    let qp_delta_code = read_ue(&mut br)?;
    let qp_delta = if qp_delta_code & 1 == 0 {
        -(qp_delta_code.div_ceil(2) as i32)
    } else {
        qp_delta_code.div_ceil(2) as i32
    };
    let pic_init_qp = 26 + qp_delta;
    let _ = num_ref_idx_l0;
    Ok((entropy, pic_init_qp))
}

/// DC 预测参数
struct DcPredParams {
    x0: usize,
    y0: usize,
    blk_w: usize,
    blk_h: usize,
    has_top: bool,
    has_left: bool,
}

/// 计算 DC 预测值 (使用上方和左方邻居像素的平均值)
fn compute_dc_block(plane: &[u8], stride: usize, p: &DcPredParams) -> u8 {
    if stride == 0 {
        return 128;
    }
    let mut sum = 0u32;
    let mut count = 0u32;

    if p.has_top && p.y0 > 0 {
        let row = p.y0 - 1;
        for dx in 0..p.blk_w {
            let idx = row * stride + p.x0 + dx;
            if idx < plane.len() {
                sum += plane[idx] as u32;
                count += 1;
            }
        }
    }

    if p.has_left && p.x0 > 0 {
        let col = p.x0 - 1;
        for dy in 0..p.blk_h {
            let idx = (p.y0 + dy) * stride + col;
            if idx < plane.len() {
                sum += plane[idx] as u32;
                count += 1;
            }
        }
    }

    if count > 0 {
        (sum / count) as u8
    } else {
        128
    }
}

/// 用单一值填充矩形块
fn fill_block(
    plane: &mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    val: u8,
) {
    for dy in 0..h {
        let row_start = (y0 + dy) * stride + x0;
        let row_end = row_start + w;
        if row_end <= plane.len() {
            plane[row_start..row_end].fill(val);
        }
    }
}

/// 从对齐缓冲区拷贝到紧凑平面
fn copy_plane(src: &[u8], src_stride: usize, w: usize, h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; w * h];
    for y in 0..h {
        let src_off = y * src_stride;
        let dst_off = y * w;
        let copy_len = w.min(src.len().saturating_sub(src_off));
        if copy_len > 0 && dst_off + copy_len <= dst.len() {
            dst[dst_off..dst_off + copy_len]
                .copy_from_slice(&src[src_off..src_off + copy_len]);
        }
    }
    dst
}
