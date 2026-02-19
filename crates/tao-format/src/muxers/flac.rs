//! FLAC 原生容器封装器.
//!
//! 将 FLAC 编码数据写入 FLAC 原生容器文件.
//!
//! FLAC 文件结构:
//! ```text
//! "fLaC" (4 bytes)              - 魔数
//! METADATA_BLOCK_STREAMINFO     - 流信息 (必须是第一个元数据块)
//! [METADATA_BLOCK_*]            - 其他可选元数据块
//! FRAME...                      - 音频帧
//! ```
//!
//! 元数据块头部:
//! - 1 bit: 是否为最后一个块
//! - 7 bits: 块类型 (0=STREAMINFO, 1=PADDING, 4=VORBIS_COMMENT, ...)
//! - 24 bits: 块数据长度

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// STREAMINFO 块类型
const BLOCK_TYPE_STREAMINFO: u8 = 0;
/// STREAMINFO 数据长度 (固定 34 字节)
const STREAMINFO_LEN: u32 = 34;
/// FLAC 魔数
const FLAC_MAGIC: &[u8; 4] = b"fLaC";

/// FLAC 封装器
pub struct FlacMuxer {
    /// STREAMINFO 数据在文件中的偏移
    streaminfo_offset: u64,
    /// 已写入的帧数
    frame_count: u64,
    /// 采样率
    sample_rate: u32,
    /// 声道数
    channels: u32,
    /// 位深
    bits_per_sample: u32,
    /// 块大小
    block_size: u32,
    /// 最小帧大小 (字节)
    min_frame_size: u32,
    /// 最大帧大小 (字节)
    max_frame_size: u32,
    /// 已写入的总采样数
    total_samples: u64,
}

impl FlacMuxer {
    /// 创建 FLAC 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            streaminfo_offset: 0,
            frame_count: 0,
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            block_size: 4096,
            min_frame_size: u32::MAX,
            max_frame_size: 0,
            total_samples: 0,
        }))
    }

    /// 写入 STREAMINFO 元数据块
    fn write_streaminfo(&self, io: &mut IoContext, is_last: bool) -> TaoResult<()> {
        // 元数据块头部: 1 bit (is_last) + 7 bits (type) + 24 bits (length)
        let header_byte = if is_last { 0x80 } else { 0x00 } | BLOCK_TYPE_STREAMINFO;
        io.write_all(&[header_byte])?;

        // 长度 (24 bits, 大端)
        io.write_all(&[(STREAMINFO_LEN >> 16) as u8])?;
        io.write_all(&[(STREAMINFO_LEN >> 8) as u8])?;
        io.write_all(&[STREAMINFO_LEN as u8])?;

        // STREAMINFO 数据 (34 字节), 初始值
        let si = self.build_streaminfo();
        io.write_all(&si)?;

        Ok(())
    }

    /// 构造 STREAMINFO 数据 (34 字节)
    fn build_streaminfo(&self) -> Vec<u8> {
        let mut si = vec![0u8; 34];
        let bs = self.block_size as u16;

        // min/max block size (16 bits each)
        si[0..2].copy_from_slice(&bs.to_be_bytes());
        si[2..4].copy_from_slice(&bs.to_be_bytes());

        // min/max frame size (24 bits each)
        let min_fs = if self.min_frame_size == u32::MAX {
            0
        } else {
            self.min_frame_size
        };
        si[4] = ((min_fs >> 16) & 0xFF) as u8;
        si[5] = ((min_fs >> 8) & 0xFF) as u8;
        si[6] = (min_fs & 0xFF) as u8;
        si[7] = ((self.max_frame_size >> 16) & 0xFF) as u8;
        si[8] = ((self.max_frame_size >> 8) & 0xFF) as u8;
        si[9] = (self.max_frame_size & 0xFF) as u8;

        // sample_rate (20 bits) + channels-1 (3 bits) + bps-1 (5 bits) + total_samples (36 bits)
        si[10] = ((self.sample_rate >> 12) & 0xFF) as u8;
        si[11] = ((self.sample_rate >> 4) & 0xFF) as u8;
        let sr_low = ((self.sample_rate & 0x0F) << 4) as u8;
        let ch_bits = (((self.channels - 1) & 0x07) << 1) as u8;
        let bps_hi = (((self.bits_per_sample - 1) >> 4) & 0x01) as u8;
        si[12] = sr_low | ch_bits | bps_hi;
        let bps_lo = (((self.bits_per_sample - 1) & 0x0F) << 4) as u8;
        let total_hi = ((self.total_samples >> 32) & 0x0F) as u8;
        si[13] = bps_lo | total_hi;
        let total_lo = (self.total_samples & 0xFFFFFFFF) as u32;
        si[14..18].copy_from_slice(&total_lo.to_be_bytes());

        // MD5 签名 (16 bytes) - 暂留空
        si
    }
}

impl Muxer for FlacMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::FlacContainer
    }

    fn name(&self) -> &str {
        "flac"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        // FLAC 只支持单个音频流
        if streams.len() != 1 {
            return Err(TaoError::InvalidArgument("FLAC 仅支持单个音频流".into()));
        }
        let stream = &streams[0];

        if stream.codec_id != CodecId::Flac {
            return Err(TaoError::InvalidArgument(format!(
                "FLAC 封装器仅支持 FLAC 编解码器, 当前: {}",
                stream.codec_id,
            )));
        }

        let audio = match &stream.params {
            StreamParams::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("FLAC 仅支持音频流".into()));
            }
        };

        self.sample_rate = audio.sample_rate;
        self.channels = audio.channel_layout.channels;
        self.block_size = if audio.frame_size > 0 {
            audio.frame_size
        } else {
            4096
        };

        // 从采样格式推断位深
        self.bits_per_sample = match audio.sample_format {
            tao_core::SampleFormat::U8 => 8,
            tao_core::SampleFormat::S16 => 16,
            tao_core::SampleFormat::S32 => 24,
            _ => 16,
        };

        // 如果 extra_data 中有 STREAMINFO, 从中提取位深
        if stream.extra_data.len() >= 18 {
            let bps = extract_bps_from_streaminfo(&stream.extra_data);
            if bps > 0 {
                self.bits_per_sample = bps;
            }
        }

        debug!(
            "写入 FLAC 头: {} Hz, {} 声道, {} 位, 块大小={}",
            self.sample_rate, self.channels, self.bits_per_sample, self.block_size,
        );

        // 写入 fLaC 魔数
        io.write_all(FLAC_MAGIC)?;

        // 记录 STREAMINFO 偏移 (跳过 4 字节块头部)
        self.streaminfo_offset = io.position()?;

        // 写入 STREAMINFO 元数据块 (is_last = true)
        self.write_streaminfo(io, true)?;

        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        // 直接写入 FLAC 帧数据
        io.write_all(&packet.data)?;

        // 更新统计
        let frame_size = packet.data.len() as u32;
        self.min_frame_size = self.min_frame_size.min(frame_size);
        self.max_frame_size = self.max_frame_size.max(frame_size);
        self.total_samples += packet.duration as u64;
        self.frame_count += 1;

        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 回填 STREAMINFO (更新 min/max frame size 和 total_samples)
        let streaminfo = self.build_streaminfo();

        // STREAMINFO 数据从 streaminfo_offset + 4 (跳过块头部) 开始
        let si_data_offset = self.streaminfo_offset + 4;
        io.seek(std::io::SeekFrom::Start(si_data_offset))?;
        io.write_all(&streaminfo)?;

        // 回到末尾
        io.seek(std::io::SeekFrom::End(0))?;

        debug!(
            "完成 FLAC 封装: {} 帧, 总采样数={}, 帧大小 {}-{} 字节",
            self.frame_count,
            self.total_samples,
            if self.min_frame_size == u32::MAX {
                0
            } else {
                self.min_frame_size
            },
            self.max_frame_size,
        );

        Ok(())
    }
}

/// 从 STREAMINFO 字节中提取位深
fn extract_bps_from_streaminfo(si: &[u8]) -> u32 {
    if si.len() < 14 {
        return 0;
    }
    let byte12 = si[12];
    let byte13 = si[13];
    let bps_minus1 = ((u32::from(byte12) & 0x01) << 4) | (u32::from(byte13) >> 4);
    bps_minus1 + 1
}

#[cfg(test)]
mod tests {
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, Rational, SampleFormat};

    use crate::io::{IoContext, MemoryBackend};
    use crate::stream::{AudioStreamParams, Stream, StreamParams};

    use super::FlacMuxer;

    fn make_flac_stream() -> Stream {
        Stream {
            index: 0,
            media_type: tao_core::MediaType::Audio,
            codec_id: CodecId::Flac,
            time_base: Rational::new(1, 44100),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::STEREO,
                sample_format: SampleFormat::S16,
                bit_rate: 0,
                frame_size: 4096,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_write_header() {
        let mut muxer = FlacMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_flac_stream()];

        muxer.write_header(&mut io, &streams).unwrap();

        // 验证写入位置: fLaC(4) + 块头(4) + STREAMINFO(34) = 42
        assert_eq!(io.position().unwrap(), 42);
    }

    #[test]
    fn test_unsupported_non_flac_codec() {
        let mut muxer = FlacMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let mut stream = make_flac_stream();
        stream.codec_id = CodecId::PcmS16le;

        let err = muxer.write_header(&mut io, &[stream]).unwrap_err();
        assert!(
            format!("{}", err).contains("FLAC 封装器仅支持 FLAC 编解码器"),
            "错误信息不符: {}",
            err,
        );
    }
}
