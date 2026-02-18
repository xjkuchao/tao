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
mod floor;
mod headers;
mod imdct;
mod residue;
mod setup;
mod synthesis;

use log::warn;
use std::collections::VecDeque;
use tao_core::{ChannelLayout, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::Frame;
use crate::packet::Packet;

use self::bitreader::{LsbBitReader, ilog};
use self::floor::build_floor_context;
use self::headers::{VorbisHeaders, parse_comment_header, parse_identification_header};
use self::imdct::{imdct_placeholder, overlap_add_placeholder};
use self::residue::decode_residue_placeholder;
use self::setup::{ParsedSetup, parse_setup_packet};
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
    sample_rate: u32,
    channel_layout: ChannelLayout,
    setup_degraded: bool,
    setup_degraded_reason: Option<String>,
    pending_frames: VecDeque<Frame>,
    first_audio_packet: bool,
    prev_blocksize: u16,
    next_pts: i64,
    overlap: Vec<Vec<f32>>,
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
            sample_rate: 0,
            channel_layout: ChannelLayout::STEREO,
            setup_degraded: false,
            setup_degraded_reason: None,
            pending_frames: VecDeque::new(),
            first_audio_packet: true,
            prev_blocksize: 0,
            next_pts: 0,
            overlap: Vec::new(),
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

        self.parsed_setup = Some(match parse_setup_packet(packet, headers.channels) {
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
                ParsedSetup {
                    mode_block_flags,
                    floor_count: 1,
                    residue_count: 1,
                    mapping_count: 1,
                }
            }
        });

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
        if parsed_setup.floor_count == 0
            || parsed_setup.residue_count == 0
            || parsed_setup.mapping_count == 0
        {
            return Err(TaoError::InvalidData(
                "Vorbis setup 关键信息计数非法".into(),
            ));
        }

        let mut br = LsbBitReader::new(packet);
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

        let blocksize = if parsed_setup.mode_block_flags[mode_number] {
            headers.blocksize1
        } else {
            headers.blocksize0
        };

        if self.first_audio_packet {
            self.first_audio_packet = false;
            self.prev_blocksize = blocksize;
            if packet_pts != tao_core::timestamp::NOPTS_VALUE {
                self.next_pts = packet_pts;
            }
            return Ok(());
        }

        let out_samples = ((usize::from(self.prev_blocksize) + usize::from(blocksize)) / 4) as u32;
        self.prev_blocksize = blocksize;
        if out_samples == 0 {
            return Ok(());
        }

        let channels = self.channel_layout.channels as usize;
        let pts = if packet_pts != tao_core::timestamp::NOPTS_VALUE {
            packet_pts
        } else {
            self.next_pts
        };

        let floor_ctx = build_floor_context(parsed_setup, channels)?;
        let residue = decode_residue_placeholder(parsed_setup, channels, blocksize as usize)?;
        if floor_ctx.channel_count != residue.channels.len() {
            return Err(TaoError::Internal("Vorbis 阶段上下文声道数不一致".into()));
        }
        if self.overlap.len() != channels {
            self.overlap = vec![Vec::new(); channels];
        }
        let td = imdct_placeholder(channels, out_samples as usize);
        let td = overlap_add_placeholder(&td, &mut self.overlap, out_samples as usize);
        let frame = synthesize_frame(
            &td,
            self.sample_rate,
            self.channel_layout,
            pts,
            out_samples as i64,
        );

        self.next_pts = frame.pts.saturating_add(frame.duration);
        self.pending_frames.push_back(Frame::Audio(frame));
        Ok(())
    }
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
        self.setup_degraded = false;
        self.setup_degraded_reason = None;
        self.pending_frames.clear();
        self.first_audio_packet = true;
        self.prev_blocksize = 0;
        self.next_pts = 0;
        self.overlap.clear();

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
        self.overlap.clear();
    }
}
