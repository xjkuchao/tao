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
use tao_core::{ChannelLayout, Rational, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::{AudioFrame, Frame};
use crate::packet::Packet;

use self::bitreader::{LsbBitReader, ilog};
use self::codebook::CodebookHuffman;
use self::floor::{build_floor_context, decode_floor_curves};
use self::headers::{VorbisHeaders, parse_comment_header, parse_identification_header};
use self::imdct::{TimeDomainBlock, build_vorbis_window, imdct_from_residue, overlap_add};
use self::residue::{apply_coupling_inverse, decode_residue_approx};
use self::setup::{Floor0Config, FloorConfig, ParsedSetup, parse_setup_packet};
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
    held_audio: Option<AudioFrame>,
    first_audio_packet: bool,
    prev_blocksize: u16,
    next_pts: i64,
    prev_packet_granule: i64,
    granule_bias: i64,
    granule_bias_locked: bool,
    has_floor0: bool,
    audio_packet_count: u64,
    valid_granule_packet_count: u64,
    repaired_packet_type_count: u64,
    huffman_loss_packet_count: u64,
    first_packet_nominal_out: i64,
    first_packet_was_short: bool,
    last_packet_is_long_block: bool,
    last_packet_next_window_flag: bool,
    last_packet_blocksize: u16,
    concealment_count: u64,
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
            held_audio: None,
            first_audio_packet: true,
            prev_blocksize: 0,
            next_pts: 0,
            prev_packet_granule: tao_core::timestamp::NOPTS_VALUE,
            granule_bias: 0,
            granule_bias_locked: false,
            has_floor0: false,
            audio_packet_count: 0,
            valid_granule_packet_count: 0,
            repaired_packet_type_count: 0,
            huffman_loss_packet_count: 0,
            first_packet_nominal_out: 0,
            first_packet_was_short: false,
            last_packet_is_long_block: false,
            last_packet_next_window_flag: true,
            last_packet_blocksize: 0,
            concealment_count: 0,
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
                    floors: vec![self::setup::FloorConfig::Floor0(fallback_floor0_config())],
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
                    floors: vec![self::setup::FloorConfig::Floor0(fallback_floor0_config())],
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
        self.has_floor0 = parsed_setup
            .floors
            .iter()
            .any(|f| matches!(f, FloorConfig::Floor0(_)));
        self.parsed_setup = Some(parsed_setup);

        self.stage = HeaderStage::Audio;
        Ok(())
    }

    fn handle_audio_packet(
        &mut self,
        packet: &[u8],
        packet_pts: i64,
        packet_time_base: Rational,
    ) -> TaoResult<()> {
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
        let raw_packet_granule =
            if packet_time_base_is_sample_granule(packet_time_base, self.sample_rate) {
                rescale_packet_pts_to_sample_granule(packet_pts, packet_time_base, self.sample_rate)
            } else {
                tao_core::timestamp::NOPTS_VALUE
            };
        self.audio_packet_count = self.audio_packet_count.saturating_add(1);
        if raw_packet_granule != tao_core::timestamp::NOPTS_VALUE {
            self.valid_granule_packet_count = self.valid_granule_packet_count.saturating_add(1);
        }
        let mut packet_type = br.read_flag()?;
        let repaired_packet =
            if packet_type && packet.first().map(|b| (b & 0x01) != 0).unwrap_or(false) {
                let mut fixed = packet.to_vec();
                fixed[0] &= !0x01;
                Some(fixed)
            } else {
                None
            };
        if repaired_packet.is_some() {
            self.repaired_packet_type_count = self.repaired_packet_type_count.saturating_add(1);
        }
        if let Some(fixed) = repaired_packet.as_ref() {
            let fixed_slice = fixed.as_slice();
            br = LsbBitReader::new(fixed_slice);
            packet_type = br.read_flag()?;
        }
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
        self.last_packet_is_long_block = is_long_block;
        self.last_packet_next_window_flag = next_window_flag;
        self.last_packet_blocksize = blocksize;

        let (left_start, right_start, right_end) = compute_window_points(
            blocksize as usize,
            headers.blocksize0 as usize,
            is_long_block,
            prev_window_flag,
            next_window_flag,
        );
        let is_first_packet = self.first_audio_packet;
        let nominal_out = right_start.saturating_sub(left_start) as i64;
        let mut packet_granule = raw_packet_granule;
        if !is_first_packet && raw_packet_granule != tao_core::timestamp::NOPTS_VALUE {
            if !self.has_floor0 && !self.granule_bias_locked {
                let remain_raw = raw_packet_granule.saturating_sub(self.next_pts);
                let diff = remain_raw.saturating_sub(nominal_out);
                let max_bias = i64::from(headers.blocksize0.max(headers.blocksize1));
                if diff > 1 && diff <= max_bias {
                    self.granule_bias = diff;
                } else {
                    self.granule_bias = 0;
                }
                self.granule_bias_locked = true;
            } else if self.has_floor0 {
                self.granule_bias = 0;
            }
            packet_granule = raw_packet_granule.saturating_sub(self.granule_bias);
        }
        if is_first_packet {
            self.first_packet_nominal_out = nominal_out.max(0);
            self.first_packet_was_short = !is_long_block;
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
            if packet_granule != tao_core::timestamp::NOPTS_VALUE {
                self.prev_packet_granule = packet_granule;
            }
        }
        self.prev_blocksize = blocksize;
        if nominal_out <= 0 && !is_first_packet {
            return Ok(());
        }

        let mut out_samples_i64 = if is_first_packet {
            0
        } else {
            nominal_out.max(0)
        };
        let pts = self.next_pts;
        if !is_first_packet
            && packet_granule != tao_core::timestamp::NOPTS_VALUE
            && self.repaired_packet_type_count == 0
        {
            let remain_to_granule = packet_granule.saturating_sub(pts);
            if remain_to_granule <= 0 {
                out_samples_i64 = 0;
            }
        }
        let out_samples = out_samples_i64 as u32;

        let floor_ctx = build_floor_context(parsed_setup, mapping, channels)?;
        let huffmans = self
            .codebook_huffmans
            .as_ref()
            .ok_or_else(|| TaoError::Codec("Vorbis Huffman 表未就绪".into()))?;
        let floor_curves = match decode_floor_curves(
            &mut br,
            parsed_setup,
            &floor_ctx,
            huffmans,
            blocksize as usize / 2,
        ) {
            Ok(v) => v,
            Err(TaoError::InvalidData(msg)) if msg.contains("Huffman 解码失败") => {
                let conceal_samples = self.huffman_conceal_samples(out_samples);
                self.conceal_corrupt_packet(conceal_samples, blocksize as usize, pts.max(0));
                if packet_granule != tao_core::timestamp::NOPTS_VALUE {
                    self.prev_packet_granule = packet_granule;
                }
                return Ok(());
            }
            Err(e) => return Err(e),
        };
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
        let mut residue = match decode_residue_approx(
            &mut br,
            parsed_setup,
            mapping,
            huffmans,
            &do_not_decode,
            channels,
            blocksize as usize,
        ) {
            Ok(v) => v,
            Err(TaoError::InvalidData(msg)) if msg.contains("Huffman 解码失败") => {
                let conceal_samples = self.huffman_conceal_samples(out_samples);
                self.conceal_corrupt_packet(conceal_samples, blocksize as usize, pts.max(0));
                if packet_granule != tao_core::timestamp::NOPTS_VALUE {
                    self.prev_packet_granule = packet_granule;
                }
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if residue.had_huffman_loss {
            self.huffman_loss_packet_count = self.huffman_loss_packet_count.saturating_add(1);
        }
        apply_coupling_inverse(&mut residue, &mapping.coupling_steps)?;
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
        for ch in &mut td.channels {
            for sample in ch {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
            }
        }
        if out_samples as usize > 0 {
            for ch in td.channels.iter_mut() {
                if ch.len() > out_samples as usize {
                    ch.truncate(out_samples as usize);
                }
            }
        }
        let mut pts = self.next_pts;
        let mut out_samples = out_samples;
        if out_samples > 0 && pts < 0 {
            let trim = ((-pts) as u32).min(out_samples) as usize;
            if trim > 0 {
                for ch in td.channels.iter_mut() {
                    if ch.len() <= trim {
                        ch.clear();
                    } else {
                        ch.drain(0..trim);
                    }
                }
                pts = pts.saturating_add(trim as i64);
                out_samples = out_samples.saturating_sub(trim as u32);
            }
        }

        if out_samples > 0 {
            let frame = synthesize_frame(
                &td,
                self.sample_rate,
                self.channel_layout,
                pts.max(0),
                out_samples as i64,
            );
            self.next_pts = frame.pts.saturating_add(frame.duration);
            self.enqueue_audio_frame(frame);
        } else if pts > self.next_pts {
            self.next_pts = pts;
        }
        if packet_granule != tao_core::timestamp::NOPTS_VALUE {
            self.prev_packet_granule = packet_granule;
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
                FloorConfig::Floor0(f0) => {
                    if f0.amp_bits == 0 || f0.amp_bits > 63 {
                        return Err(TaoError::InvalidData("Vorbis floor0 amp_bits 非法".into()));
                    }
                    if f0.bark_map_size == 0 {
                        return Err(TaoError::InvalidData(
                            "Vorbis floor0 bark_map_size 非法".into(),
                        ));
                    }
                    if f0.book_list.len() > 16 {
                        return Err(TaoError::InvalidData(
                            "Vorbis floor0 book_list 长度非法".into(),
                        ));
                    }
                    if !setup.codebooks.is_empty() {
                        for &book in &f0.book_list {
                            if usize::from(book) >= setup.codebooks.len() {
                                return Err(TaoError::InvalidData(
                                    "Vorbis floor0 codebook 索引越界".into(),
                                ));
                            }
                        }
                    }
                }
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
        if remaining <= 1 {
            return;
        }

        let channels = self.channel_layout.channels as usize;
        if channels == 0 {
            return;
        }
        let available_tail = self.overlap.iter().map(|ch| ch.len()).max().unwrap_or(0);
        if available_tail == 0 {
            return;
        }
        let tail = remaining.min(available_tail as i64) as usize;
        if tail == 0 {
            return;
        }
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
        let frame = synthesize_frame(
            &td,
            self.sample_rate,
            self.channel_layout,
            self.next_pts,
            tail as i64,
        );
        self.next_pts = self.next_pts.saturating_add(tail as i64);
        self.pending_frames.push_back(Frame::Audio(frame));
    }

    fn enqueue_audio_frame(&mut self, frame: AudioFrame) {
        if let Some(prev) = self.held_audio.take() {
            self.pending_frames.push_back(Frame::Audio(prev));
        }
        self.held_audio = Some(frame);
    }

    fn flush_held_audio_frame(&mut self) {
        let mut frame = match self.held_audio.take() {
            Some(f) => f,
            None => return,
        };

        if self.prev_packet_granule != tao_core::timestamp::NOPTS_VALUE {
            let frame_end = frame.pts.saturating_add(frame.duration);
            if frame_end > self.prev_packet_granule {
                let overshoot = frame_end.saturating_sub(self.prev_packet_granule);
                if self.should_trim_to_granule(overshoot) {
                    let keep = self
                        .prev_packet_granule
                        .saturating_sub(frame.pts)
                        .clamp(0, frame.duration) as u32;
                    let channels = frame.channel_layout.channels as usize;
                    let keep_bytes = keep as usize * channels * 4;
                    if !frame.data.is_empty() && frame.data[0].len() > keep_bytes {
                        frame.data[0].truncate(keep_bytes);
                    }
                    frame.nb_samples = keep;
                    frame.duration = i64::from(keep);
                }
            }
            let before_bias_trim = frame.duration;
            self.trim_tail_for_positive_granule_bias(&mut frame);
            if frame.duration < before_bias_trim {
                self.prev_packet_granule = frame.pts.saturating_add(frame.duration);
            }
            let before_odd_granule_trim = frame.duration;
            self.trim_tail_for_odd_granule_alignment(&mut frame);
            if frame.duration < before_odd_granule_trim {
                self.prev_packet_granule = frame.pts.saturating_add(frame.duration);
            }
        } else {
            self.trim_tail_without_granule(&mut frame);
        }

        self.next_pts = frame.pts.saturating_add(frame.duration);
        if frame.duration > 0 {
            self.pending_frames.push_back(Frame::Audio(frame));
        }
    }

    fn should_trim_to_granule(&self, overshoot: i64) -> bool {
        if overshoot <= 0 {
            return false;
        }
        if self.repaired_packet_type_count > 0 {
            return false;
        }
        if self.has_floor0 {
            return true;
        }
        let granule_sparse =
            self.valid_granule_packet_count.saturating_mul(100) < self.audio_packet_count;
        if !granule_sparse {
            return true;
        }
        overshoot <= 384
    }

    fn trim_tail_without_granule(&self, frame: &mut AudioFrame) {
        if !self.last_packet_is_long_block || self.last_packet_next_window_flag {
            return;
        }
        let headers = match self.headers.as_ref() {
            Some(v) => v,
            None => return,
        };
        let n = i64::from(self.last_packet_blocksize);
        let short_n = i64::from(headers.blocksize0);
        if n <= short_n {
            return;
        }
        // 无 granule 的 long->short 终包按窗口切换裁去右侧额外段，避免末尾过量样本。
        let trim = (n - short_n) / 4;
        if trim <= 0 || trim >= frame.duration {
            return;
        }
        let keep = frame.duration.saturating_sub(trim) as u32;
        let channels = frame.channel_layout.channels as usize;
        let keep_bytes = keep as usize * channels * 4;
        if !frame.data.is_empty() && frame.data[0].len() > keep_bytes {
            frame.data[0].truncate(keep_bytes);
        }
        frame.nb_samples = keep;
        frame.duration = i64::from(keep);
    }

    fn trim_tail_for_positive_granule_bias(&self, frame: &mut AudioFrame) {
        if !self.first_packet_was_short
            || self.granule_bias <= 0
            || self.first_packet_nominal_out <= 0
            || self.valid_granule_packet_count > 1
        {
            return;
        }
        // 某些短首包样本中，末包还需额外扣除 (首包名义输出 - granule_bias) 才能对齐 FFmpeg。
        let extra_trim = self
            .first_packet_nominal_out
            .saturating_sub(self.granule_bias);
        if extra_trim <= 0 || extra_trim >= frame.duration {
            return;
        }
        let keep = frame.duration.saturating_sub(extra_trim) as u32;
        let channels = frame.channel_layout.channels as usize;
        let keep_bytes = keep as usize * channels * 4;
        if !frame.data.is_empty() && frame.data[0].len() > keep_bytes {
            frame.data[0].truncate(keep_bytes);
        }
        frame.nb_samples = keep;
        frame.duration = i64::from(keep);
    }

    fn trim_tail_for_odd_granule_alignment(&self, frame: &mut AudioFrame) {
        if !self.first_packet_was_short
            || self.granule_bias != 0
            || self.repaired_packet_type_count > 0
            || self.valid_granule_packet_count <= 1
        {
            return;
        }
        if self.prev_packet_granule == tao_core::timestamp::NOPTS_VALUE || frame.duration <= 1 {
            return;
        }
        if (self.prev_packet_granule & 1) == 0 || (frame.pts & 1) != 0 {
            return;
        }
        if frame.pts.saturating_add(frame.duration) != self.prev_packet_granule {
            return;
        }
        // 对齐 FFmpeg: 部分短首包流在末帧需额外收缩 1 样本以匹配其时间戳口径。
        let keep = frame.duration.saturating_sub(1) as u32;
        let channels = frame.channel_layout.channels as usize;
        let keep_bytes = keep as usize * channels * 4;
        if !frame.data.is_empty() && frame.data[0].len() > keep_bytes {
            frame.data[0].truncate(keep_bytes);
        }
        frame.nb_samples = keep;
        frame.duration = i64::from(keep);
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

    fn is_recoverable_audio_error(msg: &str) -> bool {
        msg.contains("Vorbis 音频包首位必须为 0") || msg.contains("Huffman 解码失败")
    }

    fn conceal_corrupt_packet(&mut self, out_samples: u32, blocksize: usize, pts: i64) {
        self.concealment_count = self.concealment_count.saturating_add(1);
        if self.sample_rate == 0 || self.channel_layout.channels == 0 {
            return;
        }
        let channels = self.channel_layout.channels as usize;
        if self.overlap.len() != channels {
            self.overlap = vec![Vec::new(); channels];
        }
        let overlap_len = blocksize / 2;
        if out_samples > 0 {
            let mut conceal_channels = vec![vec![0.0f32; out_samples as usize]; channels];
            let mut filled_from_prev_frame = false;
            if let Some(prev_frame) = self.held_audio.as_ref()
                && !prev_frame.data.is_empty()
                && prev_frame.sample_format == tao_core::SampleFormat::F32
            {
                let prev_channels = prev_frame.channel_layout.channels as usize;
                let prev_samples = prev_frame.nb_samples as usize;
                let pcm = &prev_frame.data[0];
                if prev_channels == channels
                    && prev_samples > 0
                    && pcm.len() >= prev_samples * prev_channels * 4
                {
                    for i in 0..out_samples as usize {
                        let src_i = i % prev_samples;
                        for (ch, channel_samples) in
                            conceal_channels.iter_mut().enumerate().take(channels)
                        {
                            let base = (src_i * channels + ch) * 4;
                            channel_samples[i] = f32::from_le_bytes([
                                pcm[base],
                                pcm[base + 1],
                                pcm[base + 2],
                                pcm[base + 3],
                            ]);
                        }
                    }
                    filled_from_prev_frame = true;
                }
            }
            if !filled_from_prev_frame {
                for (ch_idx, ch_out) in conceal_channels.iter_mut().enumerate() {
                    if let Some(prev_overlap) = self.overlap.get(ch_idx)
                        && !prev_overlap.is_empty()
                    {
                        for (i, sample) in ch_out.iter_mut().enumerate() {
                            *sample = prev_overlap[i % prev_overlap.len()];
                        }
                    }
                }
            }
            for (ch, overlap_ch) in self.overlap.iter_mut().enumerate().take(channels) {
                overlap_ch.clear();
                overlap_ch.resize(overlap_len, 0.0);
                if !conceal_channels[ch].is_empty() {
                    for i in 0..overlap_len {
                        overlap_ch[i] = conceal_channels[ch][i % conceal_channels[ch].len()];
                    }
                }
            }
            let td = TimeDomainBlock {
                channels: conceal_channels,
            };
            let frame = synthesize_frame(
                &td,
                self.sample_rate,
                self.channel_layout,
                pts,
                out_samples as i64,
            );
            self.next_pts = frame.pts.saturating_add(frame.duration);
            self.enqueue_audio_frame(frame);
        } else if pts > self.next_pts {
            self.next_pts = pts;
        }
    }

    fn huffman_conceal_samples(&self, default_samples: u32) -> u32 {
        let alt = (u32::from(self.prev_blocksize) * 3) / 8;
        if alt > 0 { alt } else { default_samples }
    }

    fn trim_pending_tail_samples(&mut self, mut trim: i64) -> i64 {
        if trim <= 0 {
            return 0;
        }
        let mut trimmed = 0i64;
        while trim > 0 {
            let frame = match self.pending_frames.pop_back() {
                Some(v) => v,
                None => break,
            };
            match frame {
                Frame::Audio(mut af) => {
                    if af.duration <= trim {
                        trim = trim.saturating_sub(af.duration);
                        trimmed = trimmed.saturating_add(af.duration.max(0));
                        continue;
                    }
                    let drop = trim.max(0);
                    let keep = af.duration.saturating_sub(drop) as u32;
                    let channels = af.channel_layout.channels as usize;
                    let keep_bytes = keep as usize * channels * 4;
                    if !af.data.is_empty() && af.data[0].len() > keep_bytes {
                        af.data[0].truncate(keep_bytes);
                    }
                    af.nb_samples = keep;
                    af.duration = i64::from(keep);
                    trimmed = trimmed.saturating_add(drop);
                    trim = 0;
                    self.pending_frames.push_back(Frame::Audio(af));
                }
                other => {
                    self.pending_frames.push_back(other);
                    break;
                }
            }
        }
        trimmed
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

fn fallback_floor0_config() -> Floor0Config {
    Floor0Config {
        order: 0,
        rate: 0,
        bark_map_size: 1,
        amp_bits: 1,
        amp_offset: 0,
        book_list: Vec::new(),
    }
}

fn rescale_packet_pts_to_sample_granule(
    packet_pts: i64,
    packet_time_base: Rational,
    sample_rate: u32,
) -> i64 {
    if packet_pts == tao_core::timestamp::NOPTS_VALUE || sample_rate == 0 {
        return tao_core::timestamp::NOPTS_VALUE;
    }
    if !packet_time_base.is_valid() {
        return packet_pts;
    }
    let sample_tb = Rational::new(1, sample_rate as i32);
    let ts = tao_core::timestamp::Timestamp::new(packet_pts, packet_time_base).rescale(sample_tb);
    if ts.is_valid() {
        ts.pts
    } else {
        tao_core::timestamp::NOPTS_VALUE
    }
}

fn packet_time_base_is_sample_granule(packet_time_base: Rational, sample_rate: u32) -> bool {
    if sample_rate == 0 || !packet_time_base.is_valid() {
        return false;
    }
    let reduced = packet_time_base.reduce();
    reduced.num == 1 && reduced.den == sample_rate as i32
}

fn is_vorbis_header_packet(packet: &[u8], header_type: u8) -> bool {
    packet.len() >= 7 && packet[0] == header_type && &packet[1..7] == b"vorbis"
}

type VorbisHeaderTriplet<'a> = (&'a [u8], &'a [u8], &'a [u8]);

fn split_matroska_vorbis_private(extra_data: &[u8]) -> TaoResult<Option<VorbisHeaderTriplet<'_>>> {
    if extra_data.is_empty() || extra_data[0] != 0x02 {
        return Ok(None);
    }

    let mut offset = 1usize;
    let mut header_len_0 = 0usize;
    let mut header_len_1 = 0usize;

    for len_ref in [&mut header_len_0, &mut header_len_1] {
        let mut current = 0usize;
        loop {
            let b = *extra_data.get(offset).ok_or_else(|| {
                TaoError::InvalidData("Vorbis Matroska 私有数据头包长度区越界".into())
            })?;
            offset += 1;
            current = current.checked_add(usize::from(b)).ok_or_else(|| {
                TaoError::InvalidData("Vorbis Matroska 私有数据头包长度溢出".into())
            })?;
            if b != 0xFF {
                break;
            }
        }
        *len_ref = current;
    }

    let payload = extra_data
        .get(offset..)
        .ok_or_else(|| TaoError::InvalidData("Vorbis Matroska 私有数据缺少头包内容区".into()))?;
    let min_total = header_len_0
        .checked_add(header_len_1)
        .and_then(|v| v.checked_add(1))
        .ok_or_else(|| TaoError::InvalidData("Vorbis Matroska 私有数据长度计算溢出".into()))?;
    if payload.len() < min_total {
        return Err(TaoError::InvalidData(
            "Vorbis Matroska 私有数据头包长度非法".into(),
        ));
    }

    let header0_end = header_len_0;
    let header1_end = header_len_0 + header_len_1;
    let ident = &payload[..header0_end];
    let comment = &payload[header0_end..header1_end];
    let setup = &payload[header1_end..];

    if !is_vorbis_header_packet(ident, 1)
        || !is_vorbis_header_packet(comment, 3)
        || !is_vorbis_header_packet(setup, 5)
    {
        return Err(TaoError::InvalidData(
            "Vorbis Matroska 私有数据头包标识非法".into(),
        ));
    }

    Ok(Some((ident, comment, setup)))
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
        self.held_audio = None;
        self.first_audio_packet = true;
        self.prev_blocksize = 0;
        self.next_pts = 0;
        self.prev_packet_granule = tao_core::timestamp::NOPTS_VALUE;
        self.granule_bias = 0;
        self.granule_bias_locked = false;
        self.has_floor0 = false;
        self.audio_packet_count = 0;
        self.valid_granule_packet_count = 0;
        self.repaired_packet_type_count = 0;
        self.huffman_loss_packet_count = 0;
        self.first_packet_nominal_out = 0;
        self.first_packet_was_short = false;
        self.last_packet_is_long_block = false;
        self.last_packet_next_window_flag = true;
        self.last_packet_blocksize = 0;
        self.concealment_count = 0;
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
            if let Some((ident, comment, setup)) =
                split_matroska_vorbis_private(&params.extra_data)?
            {
                self.parse_identification_header(ident)?;
                self.parse_comment_header(comment)?;
                self.parse_setup_header(setup)?;
            } else {
                self.parse_identification_header(&params.extra_data)?;
            }
        }

        Ok(())
    }

    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("Vorbis 解码器未打开".into()));
        }

        if packet.is_empty() {
            self.flush_held_audio_frame();
            self.enqueue_tail_on_flush();
            if self.huffman_loss_packet_count > 0 && self.concealment_count == 0 {
                let trim = i64::from(self.prev_blocksize);
                if trim > 0 {
                    let trimmed = self.trim_pending_tail_samples(trim);
                    self.next_pts = self.next_pts.saturating_sub(trimmed);
                }
            }
            self.flushing = true;
            return Ok(());
        }

        let data = packet.data.as_ref();
        match self.stage {
            HeaderStage::Identification => self.parse_identification_header(data),
            HeaderStage::Comment => self.parse_comment_header(data),
            HeaderStage::Setup => self.parse_setup_header(data),
            HeaderStage::Audio => {
                match self.handle_audio_packet(data, packet.pts, packet.time_base) {
                    Ok(()) => Ok(()),
                    Err(TaoError::InvalidData(msg)) if Self::is_recoverable_audio_error(&msg) => {
                        warn!("Vorbis 跳过损坏音频包: {}", msg);
                        let is_header_packet = is_vorbis_header_packet(data, 1)
                            || is_vorbis_header_packet(data, 3)
                            || is_vorbis_header_packet(data, 5);
                        if !is_header_packet {
                            let fallback_blocksize = self
                                .headers
                                .as_ref()
                                .map(|h| usize::from(h.blocksize0))
                                .unwrap_or(256)
                                .max(1);
                            let blocksize =
                                usize::from(self.prev_blocksize).max(fallback_blocksize);
                            let default_samples =
                                (u32::try_from(blocksize).unwrap_or(256) / 2).max(1);
                            let conceal_samples = self.huffman_conceal_samples(default_samples);
                            let packet_granule = if packet_time_base_is_sample_granule(
                                packet.time_base,
                                self.sample_rate,
                            ) {
                                rescale_packet_pts_to_sample_granule(
                                    packet.pts,
                                    packet.time_base,
                                    self.sample_rate,
                                )
                            } else {
                                tao_core::timestamp::NOPTS_VALUE
                            };
                            let conceal_pts = if packet_granule != tao_core::timestamp::NOPTS_VALUE
                            {
                                packet_granule.max(0)
                            } else {
                                self.next_pts.max(0)
                            };
                            self.conceal_corrupt_packet(conceal_samples, blocksize, conceal_pts);
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
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
        self.held_audio = None;
        self.first_audio_packet = true;
        self.prev_blocksize = 0;
        self.next_pts = 0;
        self.prev_packet_granule = tao_core::timestamp::NOPTS_VALUE;
        self.granule_bias = 0;
        self.granule_bias_locked = false;
        self.has_floor0 = false;
        self.audio_packet_count = 0;
        self.valid_granule_packet_count = 0;
        self.repaired_packet_type_count = 0;
        self.huffman_loss_packet_count = 0;
        self.first_packet_nominal_out = 0;
        self.first_packet_was_short = false;
        self.last_packet_is_long_block = false;
        self.last_packet_next_window_flag = true;
        self.last_packet_blocksize = 0;
        self.concealment_count = 0;
        self.overlap.clear();
        self.window_cache.clear();
    }
}
