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
mod headers;
mod setup;

use log::warn;
use tao_core::{ChannelLayout, TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::{AudioCodecParams, CodecParameters, CodecParamsType};
use crate::decoder::Decoder;
use crate::frame::Frame;
use crate::packet::Packet;

use self::bitreader::{LsbBitReader, ilog};
use self::headers::{VorbisHeaders, parse_comment_header, parse_identification_header};
use self::setup::parse_setup_packet;

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
    mode_block_flags: Vec<bool>,
    sample_rate: u32,
    channel_layout: ChannelLayout,
    setup_degraded: bool,
    setup_degraded_reason: Option<String>,
}

impl VorbisDecoder {
    /// 创建 Vorbis 解码器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self {
            opened: false,
            flushing: false,
            stage: HeaderStage::Identification,
            headers: None,
            mode_block_flags: Vec::new(),
            sample_rate: 0,
            channel_layout: ChannelLayout::STEREO,
            setup_degraded: false,
            setup_degraded_reason: None,
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

        self.mode_block_flags = match parse_setup_packet(packet, headers.channels) {
            Ok(modes) if !modes.is_empty() => {
                self.setup_degraded = false;
                self.setup_degraded_reason = None;
                modes
            }
            Ok(_) => {
                self.setup_degraded = false;
                self.setup_degraded_reason = None;
                vec![false]
            }
            Err(e) => {
                warn!("Vorbis setup 严格解析失败, 暂降级继续: {}", e);
                self.setup_degraded = true;
                self.setup_degraded_reason = Some(e.to_string());
                if headers.blocksize0 == headers.blocksize1 {
                    vec![false]
                } else {
                    vec![false, true]
                }
            }
        };

        self.stage = HeaderStage::Audio;
        Ok(())
    }

    fn handle_audio_packet(&mut self, packet: &[u8]) -> TaoResult<()> {
        let headers = self
            .headers
            .as_ref()
            .ok_or_else(|| TaoError::Codec("Vorbis 头信息未就绪".into()))?;
        if self.mode_block_flags.is_empty() {
            return Err(TaoError::Codec("Vorbis mode 表为空".into()));
        }

        let mut br = LsbBitReader::new(packet);
        let packet_type = br.read_flag()?;
        if packet_type {
            return Err(TaoError::InvalidData("Vorbis 音频包首位必须为 0".into()));
        }

        let mode_bits = ilog(self.mode_block_flags.len() as u32 - 1);
        let mode_number = br.read_bits(mode_bits)? as usize;
        if mode_number >= self.mode_block_flags.len() {
            return Err(TaoError::InvalidData(format!(
                "Vorbis mode 索引越界: {}",
                mode_number,
            )));
        }

        let _blocksize = if self.mode_block_flags[mode_number] {
            headers.blocksize1
        } else {
            headers.blocksize0
        };

        let msg = if self.setup_degraded {
            format!(
                "Vorbis 音频解码主链路尚未实现 (P3 阶段, setup 为降级路径: {})",
                self.setup_degraded_reason.as_deref().unwrap_or("未知原因")
            )
        } else {
            "Vorbis 音频解码主链路尚未实现 (P3 阶段, setup 严格解析通过)".to_string()
        };
        Err(TaoError::NotImplemented(msg))
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
        self.mode_block_flags.clear();
        self.setup_degraded = false;
        self.setup_degraded_reason = None;

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
            HeaderStage::Audio => self.handle_audio_packet(data),
        }
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        if self.flushing {
            return Err(TaoError::Eof);
        }
        Err(TaoError::NeedMoreData)
    }

    fn flush(&mut self) {
        self.flushing = false;
    }
}
