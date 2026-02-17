//! Vorbis 音频解码器.
//!
//! 目前仅为占位符, 等待自行实现.

use tao_core::{TaoError, TaoResult};

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::decoder::Decoder;
use crate::frame::Frame;
use crate::packet::Packet;

/// Vorbis 解码器
pub struct VorbisDecoder {
    opened: bool,
}

impl VorbisDecoder {
    /// 创建 Vorbis 解码器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Decoder>> {
        Ok(Box::new(Self { opened: false }))
    }
}

impl Decoder for VorbisDecoder {
    fn codec_id(&self) -> CodecId {
        CodecId::Vorbis
    }

    fn name(&self) -> &str {
        "vorbis"
    }

    fn open(&mut self, _params: &CodecParameters) -> TaoResult<()> {
        self.opened = true;
        Ok(())
    }

    fn send_packet(&mut self, _packet: &Packet) -> TaoResult<()> {
        if !self.opened {
            return Err(TaoError::Codec("Vorbis 解码器未打开".into()));
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> TaoResult<Frame> {
        Err(TaoError::Codec("Vorbis 解码器尚未实现".into()))
    }

    fn flush(&mut self) {}
}
