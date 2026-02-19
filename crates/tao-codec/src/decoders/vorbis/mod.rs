//! Vorbis 音频解码器.
//!
//! 当前阶段实现:
//! - 三个头包 (identification/comment/setup) 解析与校验
//! - setup 中 codebook/floor/residue/mapping/mode 的语法级解析
//! - 解码器状态机与基础错误路径
//!
//! 当前限制:
//! - 尚未实现音频包到 PCM 的完整解码链路 (P3 阶段实现)

mod bitreader;
mod codebook;
mod floor;
mod headers;
mod imdct;
mod residue;
mod setup;
mod synthesis;

use log::warn;
use std::collections::{HashMap, VecDeque};
use tao_core::{ChannelLayout, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::Frame;
use crate::packet::Packet;

use self::bitreader::{LsbBitReader, ilog};
use self::codebook::CodebookHuffman;
use self::floor::{build_floor_context, decode_floor_curves};
use self::headers::{VorbisHeaders, parse_comment_header, parse_identification_header};
use self::imdct::{TimeDomainBlock, build_vorbis_window, imdct_from_residue, overlap_add};
use self::residue::{apply_coupling_inverse, decode_residue_approx};
use self::setup::{FloorConfig, ParsedSetup, parse_setup_packet};
use self::synthesis::synthesize_frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderStage {
    Identification,
    Comment,
    Setup,
    Audio,
}

/// Vorbis 解码器
pub struct VorbisDecoder {
    opened: bool,
    flushing: bool,
    stage: HeaderStage,
    headers: Option<VorbisHeaders>,
    parsed_setup: Option<ParsedSetup>,
    codebook_huffmans: Option<Vec<CodebookHuffman>>,
    sample_rate: u32,
    channel_layout: ChannelLayout,
    setup_degraded: bool,
    setup_degraded_reason: Option<String>,
    pending_frames: VecDeque<Frame>,
    first_audio_packet: bool,
    prev_blocksize: u16,
    next_pts: i64,
    prev_packet_granule: i64,
    overlap: Vec<Vec<f32>>,
    window_cache: HashMap<(usize, usize, bool, bool, bool), Vec<f32>>,
}

impl VorbisDecoder {
    /// 创建 Vorbis 解码器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            opened: false,
            flushing: false,
            stage: HeaderStage::Identification,
            headers: None,
            parsed_setup: None,
            codebook_huffmans: None,
            sample_rate: 0,
            channel_layout: ChannelLayout::STEREO,
            setup_degraded: false,
            setup_degraded_reason: None,
            pending_frames: VecDeque::new(),
            first_audio_packet: true,
            prev_blocksize: 0,
            next_pts: 0,
            prev_packet_granule: tao_core::timestamp::NOPTS_VALUE,
            overlap: Vec::new(),
            window_cache: HashMap::new(),
        }))
    }

    fn parse_identification_header(&mut self, packet: &[u8]) -> TaoResult<()> {
        let (headers, sample_rate, layout) = parse_identification_header(packet)?;
        self.sample_rate = sample_rate;
        self.channel_layout = layout;
        self.headers = Some(headers);
        self.stage = HeaderStage::Comment;
        Ok(())
    }

    fn parse_comment_header(&mut self, packet: &[u8]) -> TaoResult<()> {
        parse_comment_header(packet)?;
        self.stage = HeaderStage::Setup;
        Ok(())
    }

    fn parse_setup_header(&mut self, packet: &[u8]) -> TaoResult<()> {
        let headers = self
            .headers
            .as_ref()
            .ok_or_else(|| TaoError::InvalidData("Vorbis setup 前缺少 identification 头".into()))?;

        let parsed_setup = match parse_setup_packet(packet, headers.channels) {
            Ok(parsed) if !parsed.mode_block_flags.is_empty() => {
                self.setup_degraded = false;
                self.setup_degraded_reason = None;
                parsed
            }
            Ok(_) => {
                self.setup_degraded = false;
                self.setup_degraded_reason = None;
                ParsedSetup {
                    mode_block_flags: vec![false],
                    mode_mappings: vec![0],
                    mappings: vec![self::setup::MappingConfig {
                        coupling_steps: Vec::new(),
                        channel_mux: vec![0; headers.channels as usize],
                        submap_floor: vec![0],
                        submap_residue: vec![0],
                    }],
                    codebooks: Vec::new(),
                    floors: vec![self::setup::FloorConfig::Floor0],
                    residues: vec![self::setup::ResidueConfig {
                        residue_type: 0,
                        begin: 0,
                        end: 0,
                        partition_size: 1,
                        classifications: 1,
                        classbook: 0,
                        cascades: vec![0],
                        books: vec![[None; 8]],
                    }],
                    floor_count: 1,
                    residue_count: 1,
                    mapping_count: 1,
                }
            }
            Err(e) => {
                warn!("Vorbis setup 严格解析失败, 暂降级继续: {}", e);
                self.setup_degraded = true;
                self.setup_degraded_reason = Some(e.to_string());
                let mode_block_flags = if headers.blocksize0 == headers.blocksize1 {
                    vec![false]
                } else {
                    vec![false, true]
                };
                let mode_count = mode_block_flags.len();
                ParsedSetup {
                    mode_block_flags,
                    mode_mappings: vec![0; mode_count],
                    mappings: vec![self::setup::MappingConfig {
                        coupling_steps: Vec::new(),
                        channel_mux: vec![0; headers.channels as usize],
                        submap_floor: vec![0],
                        submap_residue: vec![0],
                    }],
                    codebooks: Vec::new(),
                    floors: vec![self::setup::FloorConfig::Floor0],
                    residues: vec![self::setup::ResidueConfig {
                        residue_type: 0,
                        begin: 0,
                        end: 0,
                        partition_size: 1,
                        classifications: 1,
                        classbook: 0,
                        cascades: vec![0],
                        books: vec![[None; 8]],
                    }],
                    floor_count: 1,
                    residue_count: 1,
                    mapping_count: 1,
                }
            }
        };
        let codebook_huffmans = parsed_setup
            .codebooks
            .iter()
            .map(|cb| CodebookHuffman::from_lengths(&cb.lengths))
            .collect::<TaoResult<Vec<_>>>()?;
        self.codebook_huffmans = Some(codebook_huffmans);
        self.parsed_setup = Some(parsed_setup);

        self.stage = HeaderStage::Audio;
        Ok(())
    }

    fn handle_audio_packet(&mut self, packet: &[u8], packet_pts: i64) -> TaoResult<()> {
        let headers = self
            .headers
            .as_ref()
            .ok_or_else(|| TaoError::Codec("Vorbis 头信息未就绪".into()))?;
        let parsed_setup = self
            .parsed_setup
            .as_ref()
            .ok_or_else(|| TaoError::Codec("Vorbis setup 信息未就绪".into()))?;
        if parsed_setup.mode_block_flags.is_empty() {
            return Err(TaoError::Codec("Vorbis mode 表为空".into()));
        }
        if parsed_setup.mode_mappings.len() != parsed_setup.mode_block_flags.len() {
            return Err(TaoError::InvalidData(
                "Vorbis mode block_flag 与 mapping 表长度不一致".into(),
            ));
        }
        if parsed_setup.floor_count == 0
            || parsed_setup.residue_count == 0
            || parsed_setup.mapping_count == 0
        {
            return Err(TaoError::InvalidData(
                "Vorbis setup 关键信息计数非法".into(),
            ));
        }
        self.validate_setup_runtime(parsed_setup)?;

        let mut br = LsbBitReader::new(packet);
        let channels = self.channel_layout.channels as usize;
        let packet_type = br.read_flag()?;
        if packet_type {
            return Err(TaoError::InvalidData("Vorbis 音频包首位必须为 0".into()));
        }

        let mode_bits = ilog(parsed_setup.mode_block_flags.len() as u32 - 1);
        let mode_number = br.read_bits(mode_bits)? as usize;
        if mode_number >= parsed_setup.mode_block_flags.len() {
            return Err(TaoError::InvalidData(format!(
                "Vorbis mode 索引越界: {}",
                mode_number,
            )));
        }
        let mapping_number = parsed_setup.mode_mappings[mode_number] as usize;
        if mapping_number >= parsed_setup.mappings.len() {
            return Err(TaoError::InvalidData(format!(
                "Vorbis mode->mapping 越界: mode={}, mapping={}",
                mode_number, mapping_number
            )));
        }
        let mapping = &parsed_setup.mappings[mapping_number];
        if !mapping.channel_mux.is_empty() && mapping.channel_mux.len() != channels {
            return Err(TaoError::InvalidData(
                "Vorbis mapping mux 声道数与流信息不一致".into(),
            ));
        }
        for &mux in &mapping.channel_mux {
            if usize::from(mux) >= mapping.submap_floor.len()
                || usize::from(mux) >= mapping.submap_residue.len()
            {
                return Err(TaoError::InvalidData(
                    "Vorbis mapping mux 子映射索引越界".into(),
                ));
            }
        }
        for step in &mapping.coupling_steps {
            if usize::from(step.magnitude) >= channels || usize::from(step.angle) >= channels {
                return Err(TaoError::InvalidData("Vorbis coupling 声道索引越界".into()));
            }
            if step.magnitude == step.angle {
                return Err(TaoError::InvalidData(
                    "Vorbis coupling magnitude/angle 不能相同".into(),
                ));
            }
        }

        let blocksize = if parsed_setup.mode_block_flags[mode_number] {
            headers.blocksize1
        } else {
            headers.blocksize0
        };
        let mut prev_window_flag = true;
        let mut next_window_flag = true;
        let is_long_block = parsed_setup.mode_block_flags[mode_number];
        if is_long_block {
            prev_window_flag = br.read_flag()?;
            next_window_flag = br.read_flag()?;
        }

        let (left_start, right_start, right_end) = compute_window_points(
            blocksize as usize,
            headers.blocksize0 as usize,
            is_long_block,
            prev_window_flag,
            next_window_flag,
        );
        let nominal_out = right_start.saturating_sub(left_start) as i64;
        let is_first_packet = self.first_audio_packet;
        if is_first_packet {
            self.first_audio_packet = false;
            self.prev_blocksize = if is_long_block {
                if prev_window_flag {
                    headers.blocksize1
                } else {
                    headers.blocksize0
                }
            } else {
                headers.blocksize0
            };
            if packet_pts != tao_core::timestamp::NOPTS_VALUE && packet_pts >= 0 {
                self.prev_packet_granule = packet_pts;
                self.next_pts = packet_pts;
            }
        }
        self.prev_blocksize = blocksize;
        if nominal_out <= 0 && !is_first_packet {
            return Ok(());
        }

        let mut out_samples_i64 = if is_first_packet { 0 } else { nominal_out };
        let pts = self.next_pts;
        if !is_first_packet && packet_pts != tao_core::timestamp::NOPTS_VALUE && packet_pts >= 0 {
            let remain_to_granule = packet_pts.saturating_sub(pts);
            if remain_to_granule >= 0 {
                out_samples_i64 = out_samples_i64.min(remain_to_granule);
            }
        }
        let out_samples = out_samples_i64 as u32;

        let floor_ctx = build_floor_context(parsed_setup, mapping, channels)?;
        let huffmans = self
            .codebook_huffmans
            .as_ref()
            .ok_or_else(|| TaoError::Codec("Vorbis Huffman 表未就绪".into()))?;
        let floor_curves = decode_floor_curves(
            &mut br,
            parsed_setup,
            &floor_ctx,
            huffmans,
            blocksize as usize / 2,
        )?;
        let mut do_not_decode: Vec<bool> = floor_curves.nonzero.iter().map(|&v| !v).collect();
        for step in &mapping.coupling_steps {
            let m = usize::from(step.magnitude);
            let a = usize::from(step.angle);
            if m < do_not_decode.len()
                && a < do_not_decode.len()
                && !(do_not_decode[m] && do_not_decode[a])
            {
                do_not_decode[m] = false;
                do_not_decode[a] = false;
            }
        }
        let mut residue = decode_residue_approx(
            &mut br,
            parsed_setup,
            mapping,
            huffmans,
            &do_not_decode,
            channels,
            blocksize as usize,
        )?;
        let disable_coupling = std::env::var("TAO_VORBIS_DISABLE_COUPLING")
            .map(|v| v != "0")
            .unwrap_or(false);
        if !disable_coupling {
            apply_coupling_inverse(&mut residue, &mapping.coupling_steps)?;
        }
        for ch in 0..channels {
            if !floor_curves.nonzero.get(ch).copied().unwrap_or(false) {
                if let Some(sp) = residue.channels.get_mut(ch) {
                    sp.fill(0.0);
                }
                continue;
            }
            if let (Some(sp), Some(curve)) = (
                residue.channels.get_mut(ch),
                floor_curves.channel_curves.get(ch),
            ) {
                let len = sp.len().min(curve.len());
                for i in 0..len {
                    sp[i] *= curve[i];
                }
            }
        }
        if floor_ctx.channel_count != residue.channels.len() {
            return Err(TaoError::Internal("Vorbis 阶段上下文声道数不一致".into()));
        }
        if floor_ctx.floor_index_per_channel.len() != channels {
            return Err(TaoError::Internal("Vorbis floor 上下文声道映射异常".into()));
        }
        if self.overlap.len() != channels {
            self.overlap = vec![Vec::new(); channels];
        }
        let window = self
            .get_or_build_window(
                blocksize as usize,
                headers.blocksize0 as usize,
                is_long_block,
                prev_window_flag,
                next_window_flag,
            )
            .to_vec();
        let td = imdct_from_residue(&residue, blocksize as usize, &window);
        let mut td = overlap_add(
            &td,
            &mut self.overlap,
            &window,
            left_start,
            right_start,
            right_end,
        );
        if out_samples as usize > 0 {
            for ch in td.channels.iter_mut() {
                if ch.len() > out_samples as usize {
                    ch.truncate(out_samples as usize);
                }
            }
        }
        if out_samples > 0 {
            let frame = synthesize_frame(
                &td,
                self.sample_rate,
                self.channel_layout,
                pts,
                out_samples as i64,
            );
            self.next_pts = frame.pts.saturating_add(frame.duration);
            self.pending_frames.push_back(Frame::Audio(frame));
        }
        if packet_pts != tao_core::timestamp::NOPTS_VALUE && packet_pts >= 0 {
            self.prev_packet_granule = packet_pts;
        }
        Ok(())
    }

    fn validate_setup_runtime(&self, setup: &ParsedSetup) -> TaoResult<()> {
        let _decode_fn: fn(&CodebookHuffman, &mut LsbBitReader<'_>) -> TaoResult<u32> =
            CodebookHuffman::decode_symbol;
        for cb in &setup.codebooks {
            if cb.dimensions == 0 || cb.entries == 0 {
                return Err(TaoError::InvalidData("Vorbis codebook 参数非法".into()));
            }
            if cb.lengths.len() != cb.entries as usize {
                return Err(TaoError::InvalidData("Vorbis codebook 长度表不匹配".into()));
            }
            if cb.lookup_type > 2 {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook lookup_type 非法".into(),
                ));
            }
            if cb.lookup_type == 0 && cb.lookup.is_some() {
                return Err(TaoError::InvalidData(
                    "Vorbis codebook lookup_type=0 不应包含 lookup".into(),
                ));
            }
            if cb.lookup_type > 0 {
                let lookup = cb.lookup.as_ref().ok_or_else(|| {
                    TaoError::InvalidData("Vorbis codebook lookup 配置缺失".into())
                })?;
                if lookup.value_bits == 0 || lookup.value_bits > 32 {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook value_bits 非法".into(),
                    ));
                }
                if lookup.lookup_values == 0 {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook lookup_values 非法".into(),
                    ));
                }
                if lookup.multiplicands.len() != lookup.lookup_values as usize {
                    return Err(TaoError::InvalidData(
                        "Vorbis codebook multiplicands 长度非法".into(),
                    ));
                }
            }
            let _ = CodebookHuffman::from_lengths(&cb.lengths)?;
        }

        for floor in &setup.floors {
            match floor {
                FloorConfig::Floor0 => {}
                FloorConfig::Floor1(f1) => {
                    if f1.partitions as usize != f1.partition_classes.len() {
                        return Err(TaoError::InvalidData(
                            "Vorbis floor1 partitions 不匹配".into(),
                        ));
                    }
                    if f1.multiplier == 0 || f1.multiplier > 4 {
                        return Err(TaoError::InvalidData(
                            "Vorbis floor1 multiplier 非法".into(),
                        ));
                    }
                    if f1.range_bits > 15 || f1.x_list.len() < 2 {
                        return Err(TaoError::InvalidData("Vorbis floor1 x_list 非法".into()));
                    }
                    for c in &f1.classes {
                        if c.dimensions == 0 {
                            return Err(TaoError::InvalidData(
                                "Vorbis floor1 class dimensions 非法".into(),
                            ));
                        }
                        if c.subclasses > 3 {
                            return Err(TaoError::InvalidData(
                                "Vorbis floor1 class subclasses 非法".into(),
                            ));
                        }
                        if c.masterbook.is_some() && c.subclasses == 0 {
                            return Err(TaoError::InvalidData(
                                "Vorbis floor1 class masterbook 状态非法".into(),
                            ));
                        }
                        let expect_books = 1usize << c.subclasses;
                        if c.subclass_books.len() != expect_books {
                            return Err(TaoError::InvalidData(
                                "Vorbis floor1 subclass_books 长度非法".into(),
                            ));
                        }
                    }
                }
            }
        }

        for residue in &setup.residues {
            if residue.residue_type > 2 {
                return Err(TaoError::InvalidData("Vorbis residue_type 非法".into()));
            }
            if residue.partition_size == 0 {
                return Err(TaoError::InvalidData(
                    "Vorbis residue partition_size 非法".into(),
                ));
            }
            if residue.end < residue.begin {
                return Err(TaoError::InvalidData("Vorbis residue 区间非法".into()));
            }
            if residue.classifications == 0 {
                return Err(TaoError::InvalidData(
                    "Vorbis residue classifications 非法".into(),
                ));
            }
            if residue.cascades.len() != residue.classifications as usize
                || residue.books.len() != residue.classifications as usize
            {
                return Err(TaoError::InvalidData(
                    "Vorbis residue 分类表长度非法".into(),
                ));
            }
            for (ci, cascade) in residue.cascades.iter().copied().enumerate() {
                for bit in 0..8usize {
                    let has_book = residue.books[ci][bit].is_some();
                    let mask_on = (cascade & (1u8 << bit)) != 0;
                    if has_book != mask_on {
                        return Err(TaoError::InvalidData(
                            "Vorbis residue cascade/book 映射不一致".into(),
                        ));
                    }
                }
            }
            let _ = residue.classbook;
        }
        Ok(())
    }

    fn enqueue_tail_on_flush(&mut self) {
        if self.overlap.is_empty() || self.sample_rate == 0 {
            return;
        }
        if self.prev_packet_granule == tao_core::timestamp::NOPTS_VALUE {
            return;
        }
        let remaining = self.prev_packet_granule.saturating_sub(self.next_pts);
        if remaining <= 0 {
            return;
        }

        let channels = self.channel_layout.channels as usize;
        if channels == 0 {
            return;
        }
        let tail = remaining as usize;
        let mut td_channels = vec![vec![0.0f32; tail]; channels];
        for (ch, dst) in td_channels.iter_mut().enumerate().take(channels) {
            if let Some(src) = self.overlap.get(ch) {
                let n = tail.min(src.len());
                dst[..n].copy_from_slice(&src[..n]);
            }
        }
        if td_channels.iter().all(|c| c.is_empty()) {
            return;
        }

        let td = TimeDomainBlock {
            channels: td_channels,
        };
        const TAIL_ENERGY_EPS: f32 = 1.0e-7;
        let tail_max = td
            .channels
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f32, |m, &v| m.max(v.abs()));
        if tail_max <= TAIL_ENERGY_EPS {
            return;
        }
        let frame = synthesize_frame(
            &td,
            self.sample_rate,
            self.channel_layout,
            self.next_pts,
            remaining,
        );
        self.next_pts = self.next_pts.saturating_add(remaining);
        self.pending_frames.push_back(Frame::Audio(frame));
    }

    fn get_or_build_window(
        &mut self,
        n: usize,
        short_n: usize,
        is_long_block: bool,
        prev_window_flag: bool,
        next_window_flag: bool,
    ) -> &[f32] {
        let key = (
            n,
            short_n,
            is_long_block,
            prev_window_flag,
            next_window_flag,
        );
        self.window_cache
            .entry(key)
            .or_insert_with(|| {
                build_vorbis_window(
                    n,
                    short_n,
                    is_long_block,
                    prev_window_flag,
                    next_window_flag,
                )
            })
            .as_slice()
    }
}

fn compute_window_points(
    blocksize: usize,
    short_blocksize: usize,
    is_long_block: bool,
    prev_window_flag: bool,
    next_window_flag: bool,
) -> (usize, usize, usize) {
    let n = blocksize;
    let window_center = n >> 1;
    let left_start = if !is_long_block || prev_window_flag {
        0
    } else {
        (n.saturating_sub(short_blocksize)) >> 2
    };
    let right_start = if !is_long_block || next_window_flag {
        window_center
    } else {
        (n.saturating_mul(3).saturating_sub(short_blocksize)) >> 2
    };
    let right_end = if !is_long_block || next_window_flag {
        n
    } else {
        (n.saturating_mul(3).saturating_add(short_blocksize)) >> 2
    };
    (left_start, right_start, right_end)
}

impl Decoder for VorbisDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Vorbis
    }

    fn name(&self) -> &str {
        "vorbis"
    }

    fn open(&mut self, params: &CodecParameters) -> TaoResult<()> {
        self.opened = true;
        self.flushing = false;
        self.stage = HeaderStage::Identification;
        self.headers = None;
        self.parsed_setup = None;
        self.codebook_huffmans = None;
        self.setup_degraded = false;
        self.setup_degraded_reason = None;
        self.pending_frames.clear();
        self.first_audio_packet = true;
        self.prev_blocksize = 0;
        self.next_pts = 0;
        self.prev_packet_granule = tao_core::timestamp::NOPTS_VALUE;
        self.overlap.clear();
        self.window_cache.clear();
        self.codebook_huffmans = None;

        if let CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            ..
        }) = &params.params
        {
            if *sample_rate > 0 {
                self.sample_rate = *sample_rate;
            }
            self.channel_layout = *channel_layout;
        }

        if !params.extra_data.is_empty() {
            self.parse_identification_header(&params.extra_data)?;
        }

        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("Vorbis 解码器未打开".into()));
        }

        if packet.is_empty() {
            self.enqueue_tail_on_flush();
            self.flushing = true;
            return Ok(());
        }

        let data = packet.data.as_ref();
        match self.stage {
            HeaderStage::Identification => self.parse_identification_header(data),
            HeaderStage::Comment => self.parse_comment_header(data),
            HeaderStage::Setup => self.parse_setup_header(data),
            HeaderStage::Audio => self.handle_audio_packet(data, packet.pts),
        }
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if let Some(frame) = self.pending_frames.pop_front() {
            return Ok(frame);
        }
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.flushing = false;
        self.pending_frames.clear();
        self.first_audio_packet = true;
        self.prev_blocksize = 0;
        self.next_pts = 0;
        self.prev_packet_granule = tao_core::timestamp::NOPTS_VALUE;
        self.overlap.clear();
        self.window_cache.clear();
    }
}
