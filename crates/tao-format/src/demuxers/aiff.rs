//! AIFF / AIFF-C 解封装器.
//!
//! 支持标准 AIFF (大端 PCM) 和 AIFF-C (含压缩类型字段) 文件的读取.
//!
//! AIFF 文件结构:
//! ```text
//! FORM header:  "FORM" + file_size(BE) + "AIFF"/"AIFC"
//! COMM chunk:   "COMM" + chunk_size(BE) + channels(BE16) + numSampleFrames(BE32)
//!              + sampleSize(BE16) + sampleRate(80-bit extended)
//!              [AIFF-C 额外: compressionType(4) + compressionName(pstring)]
//! SSND chunk:   "SSND" + chunk_size(BE) + offset(BE32) + blockSize(BE32) + PCM data...
//! ```

use log::{debug, warn};
use tao_codec::CodecId;
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::stream::{AudioStreamParams, Stream, StreamParams};

/// AIFF 解封装器
pub struct AiffDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// SSND 数据区在文件中的起始偏移 (跳过 offset/blockSize 后)
    data_offset: u64,
    /// 音频数据总大小 (字节)
    data_size: u64,
    /// 当前读取位置 (相对于数据起始)
    data_pos: u64,
    /// 每次读取的数据包大小 (字节)
    packet_size: usize,
    /// 块对齐 (每个采样帧的字节数)
    block_align: u16,
    /// 采样率
    sample_rate: u32,
    /// 元数据
    metadata: Vec<(String, String)>,
    /// 是否为 AIFF-C 格式
    is_aifc: bool,
}

impl AiffDemuxer {
    /// 创建 AIFF 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            data_offset: 0,
            data_size: 0,
            data_pos: 0,
            packet_size: 0,
            block_align: 0,
            sample_rate: 0,
            metadata: Vec::new(),
            is_aifc: false,
        }))
    }

    /// 根据位深和压缩类型确定 CodecId
    fn resolve_codec_id(
        bits_per_sample: u16,
        is_aifc: bool,
        compression_type: Option<&[u8; 4]>,
    ) -> TaoResult<CodecId> {
        if is_aifc {
            if let Some(ct) = compression_type {
                match ct {
                    b"NONE" | b"none" => {
                        // 未压缩大端 PCM
                        return Self::resolve_uncompressed_codec(bits_per_sample);
                    }
                    b"sowt" => {
                        // 小端 PCM (Apple 扩展)
                        return match bits_per_sample {
                            16 => Ok(CodecId::PcmS16le),
                            24 => Ok(CodecId::PcmS24le),
                            32 => Ok(CodecId::PcmS32le),
                            _ => Err(TaoError::Unsupported(format!(
                                "sowt 不支持的位深: {}",
                                bits_per_sample
                            ))),
                        };
                    }
                    b"fl32" | b"FL32" => return Ok(CodecId::PcmF32le),
                    _ => {
                        return Err(TaoError::Unsupported(format!(
                            "AIFF-C 不支持的压缩类型: {}",
                            String::from_utf8_lossy(ct),
                        )));
                    }
                }
            }
        }

        // 标准 AIFF: 大端 PCM
        Self::resolve_uncompressed_codec(bits_per_sample)
    }

    /// 解析未压缩 (大端) PCM 的 CodecId
    fn resolve_uncompressed_codec(bits_per_sample: u16) -> TaoResult<CodecId> {
        match bits_per_sample {
            8 => Ok(CodecId::PcmU8), // AIFF 8 位也是无符号
            16 => Ok(CodecId::PcmS16be),
            24 => Ok(CodecId::PcmS24le), // 解码器内部会做字节序转换
            32 => Ok(CodecId::PcmS32le),
            _ => Err(TaoError::Unsupported(format!(
                "AIFF 不支持的位深: {}",
                bits_per_sample
            ))),
        }
    }

    /// 根据 CodecId 确定采样格式
    fn resolve_sample_format(codec_id: CodecId) -> SampleFormat {
        match codec_id {
            CodecId::PcmU8 => SampleFormat::U8,
            CodecId::PcmS16le | CodecId::PcmS16be => SampleFormat::S16,
            CodecId::PcmS24le | CodecId::PcmS32le => SampleFormat::S32,
            CodecId::PcmF32le => SampleFormat::F32,
            _ => SampleFormat::None,
        }
    }
}

/// 解析 IEEE 754 80-bit 扩展精度浮点数 (大端)
///
/// AIFF 使用这种格式存储采样率.
fn parse_ieee_extended(data: &[u8; 10]) -> f64 {
    let sign = (data[0] >> 7) & 1;
    let exponent = (((data[0] as u16) & 0x7F) << 8) | (data[1] as u16);
    let mantissa = u64::from_be_bytes([
        data[2], data[3], data[4], data[5], data[6], data[7], data[8], data[9],
    ]);

    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    if exponent == 0x7FFF {
        return f64::INFINITY;
    }

    // 80 位扩展精度: 偏移量 = 16383, 尾数有显式整数位
    let f = mantissa as f64 / (1u64 << 63) as f64;
    let value = f * 2.0_f64.powi(exponent as i32 - 16383);
    if sign == 1 { -value } else { value }
}

/// 将 f64 编码为 IEEE 754 80-bit 扩展精度浮点数 (大端)
#[cfg(test)]
fn encode_ieee_extended(value: f64) -> [u8; 10] {
    if value == 0.0 {
        return [0u8; 10];
    }

    let sign: u8 = if value < 0.0 { 1 } else { 0 };
    let val = value.abs();

    // 计算指数和尾数
    let log2 = val.log2().floor() as i32;
    let exponent = (log2 + 16383) as u16;
    let mantissa = (val / 2.0_f64.powi(log2) * (1u64 << 63) as f64) as u64;

    let mut result = [0u8; 10];
    result[0] = (sign << 7) | ((exponent >> 8) as u8 & 0x7F);
    result[1] = (exponent & 0xFF) as u8;
    let m_bytes = mantissa.to_be_bytes();
    result[2..10].copy_from_slice(&m_bytes);
    result
}

impl Demuxer for AiffDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Aiff
    }

    fn name(&self) -> &str {
        "aiff"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 读取 FORM 头
        let form_tag = io.read_tag()?;
        if &form_tag != b"FORM" {
            return Err(TaoError::InvalidData("不是有效的 IFF/FORM 文件".into()));
        }

        let _file_size = io.read_u32_be()?;

        let aiff_tag = io.read_tag()?;
        self.is_aifc = match &aiff_tag {
            b"AIFF" => false,
            b"AIFC" => true,
            _ => {
                return Err(TaoError::InvalidData(format!(
                    "不是 AIFF/AIFC 文件: {}",
                    String::from_utf8_lossy(&aiff_tag)
                )));
            }
        };

        debug!(
            "检测到 {} 文件",
            if self.is_aifc { "AIFF-C" } else { "AIFF" }
        );

        // 解析各 chunk
        let mut comm_found = false;
        let mut ssnd_found = false;
        let mut channels: u16 = 0;
        let mut num_sample_frames: u32 = 0;
        let mut bits_per_sample: u16 = 0;
        let mut sample_rate_f64: f64 = 0.0;
        let mut compression_type: Option<[u8; 4]> = None;

        while !ssnd_found {
            let chunk_id = match io.read_tag() {
                Ok(tag) => tag,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };
            let chunk_size = io.read_u32_be()? as u64;
            let chunk_id_str = String::from_utf8_lossy(&chunk_id);

            match &chunk_id {
                b"COMM" => {
                    channels = io.read_u16_be()?;
                    num_sample_frames = io.read_u32_be()?;
                    bits_per_sample = io.read_u16_be()?;

                    // 读取 80 位扩展精度采样率
                    let mut sr_bytes = [0u8; 10];
                    io.read_exact(&mut sr_bytes)?;
                    sample_rate_f64 = parse_ieee_extended(&sr_bytes);

                    let read_so_far = 2 + 4 + 2 + 10; // 18 bytes

                    if self.is_aifc && chunk_size >= 22 {
                        // AIFF-C: 读取压缩类型
                        let ct = io.read_tag()?;
                        compression_type = Some(ct);

                        // 跳过剩余 (compressionName pstring 等)
                        let remaining = chunk_size as usize - read_so_far - 4;
                        if remaining > 0 {
                            io.skip(remaining)?;
                        }
                    } else if chunk_size as usize > read_so_far {
                        io.skip(chunk_size as usize - read_so_far)?;
                    }

                    debug!(
                        "COMM: channels={}, frames={}, bits={}, rate={}",
                        channels, num_sample_frames, bits_per_sample, sample_rate_f64,
                    );
                    comm_found = true;
                }
                b"SSND" => {
                    if !comm_found {
                        return Err(TaoError::InvalidData("SSND 块出现在 COMM 块之前".into()));
                    }
                    // SSND 数据区: offset (4) + blockSize (4) + data
                    let ssnd_offset = io.read_u32_be()? as u64;
                    let _block_size = io.read_u32_be()?;

                    // 跳过 offset 字节
                    if ssnd_offset > 0 {
                        io.skip(ssnd_offset as usize)?;
                    }

                    self.data_offset = io.position()?;
                    // 数据大小 = chunk_size - 8 (offset + blockSize 字段) - ssnd_offset
                    self.data_size = chunk_size.saturating_sub(8 + ssnd_offset);
                    ssnd_found = true;

                    debug!(
                        "SSND: data_offset={}, data_size={}",
                        self.data_offset, self.data_size
                    );
                }
                _ => {
                    warn!("跳过未知 AIFF 块: '{}', 大小={}", chunk_id_str, chunk_size);
                    io.skip(chunk_size as usize)?;
                }
            }

            // IFF 块要求偶数对齐
            if !ssnd_found && chunk_size % 2 != 0 {
                io.skip(1)?;
            }
        }

        if !comm_found {
            return Err(TaoError::InvalidData("未找到 COMM 块".into()));
        }
        if !ssnd_found {
            return Err(TaoError::InvalidData("未找到 SSND 块".into()));
        }

        let sample_rate = sample_rate_f64.round() as u32;
        let codec_id =
            Self::resolve_codec_id(bits_per_sample, self.is_aifc, compression_type.as_ref())?;
        let sample_format = Self::resolve_sample_format(codec_id);
        let channel_layout = ChannelLayout::from_channels(u32::from(channels));
        let time_base = Rational::new(1, sample_rate as i32);
        let block_align = channels * (bits_per_sample / 8);

        let bit_rate = u64::from(sample_rate) * u64::from(channels) * u64::from(bits_per_sample);

        let stream = Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id,
            time_base,
            duration: i64::from(num_sample_frames),
            start_time: 0,
            nb_frames: u64::from(num_sample_frames),
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate,
                channel_layout,
                sample_format,
                bit_rate,
                frame_size: 0,
            }),
            metadata: Vec::new(),
        };

        self.streams = vec![stream];
        self.block_align = block_align;
        self.sample_rate = sample_rate;
        self.data_pos = 0;

        // 每个数据包读取 ~4096 采样
        let samples_per_packet = 4096u32;
        self.packet_size = (u32::from(block_align) * samples_per_packet) as usize;
        if self.packet_size == 0 {
            self.packet_size = 4096;
        }

        debug!(
            "AIFF 打开完成: {} Hz, {} 声道, {} 位, 总帧数={}",
            sample_rate, channels, bits_per_sample, num_sample_frames,
        );

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<tao_codec::Packet> {
        if self.data_pos >= self.data_size {
            return Err(TaoError::Eof);
        }

        let remaining = (self.data_size - self.data_pos) as usize;
        let read_size = self.packet_size.min(remaining);

        // 确保对齐到 block_align
        let aligned_size = if self.block_align > 0 {
            let ba = self.block_align as usize;
            (read_size / ba) * ba
        } else {
            read_size
        };

        if aligned_size == 0 {
            return Err(TaoError::Eof);
        }

        let data = io.read_bytes(aligned_size)?;

        // 计算时间戳
        let sample_offset = if self.block_align > 0 {
            self.data_pos / u64::from(self.block_align)
        } else {
            0
        };
        let nb_samples = if self.block_align > 0 {
            aligned_size as i64 / i64::from(self.block_align)
        } else {
            0
        };

        let mut pkt = tao_codec::Packet::from_data(bytes::Bytes::from(data));
        pkt.stream_index = 0;
        pkt.pts = sample_offset as i64;
        pkt.dts = pkt.pts;
        pkt.duration = nb_samples;
        pkt.time_base = Rational::new(1, self.sample_rate as i32);
        pkt.is_keyframe = true;
        pkt.pos = (self.data_offset + self.data_pos) as i64;

        self.data_pos += aligned_size as u64;

        Ok(pkt)
    }

    fn seek(
        &mut self,
        io: &mut IoContext,
        _stream_index: usize,
        timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        if !io.is_seekable() {
            return Err(TaoError::Unsupported("不支持在非可寻址流上 seek".into()));
        }
        if self.block_align == 0 {
            return Err(TaoError::InvalidData("block_align 为 0, 无法 seek".into()));
        }

        let sample = timestamp.max(0) as u64;
        let byte_offset = sample * u64::from(self.block_align);
        let byte_offset = byte_offset.min(self.data_size);

        let ba = u64::from(self.block_align);
        let aligned_offset = (byte_offset / ba) * ba;

        io.seek(std::io::SeekFrom::Start(self.data_offset + aligned_offset))?;
        self.data_pos = aligned_offset;

        debug!(
            "AIFF seek: 目标采样={}, 字节偏移={}",
            sample, aligned_offset
        );
        Ok(())
    }

    fn duration(&self) -> Option<f64> {
        if self.sample_rate > 0 && self.block_align > 0 {
            let total_samples = self.data_size / u64::from(self.block_align);
            Some(total_samples as f64 / f64::from(self.sample_rate))
        } else {
            None
        }
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

/// AIFF 格式探测器
pub struct AiffProbe;

impl FormatProbe for AiffProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        // 检查 FORM/AIFF 或 FORM/AIFC 魔数
        if data.len() >= 12
            && &data[0..4] == b"FORM"
            && (&data[8..12] == b"AIFF" || &data[8..12] == b"AIFC")
        {
            return Some(SCORE_MAX);
        }

        // 仅根据扩展名
        if let Some(name) = filename {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".aiff") || lower.ends_with(".aif") {
                return Some(SCORE_EXTENSION);
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Aiff
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构建简单的 AIFF 文件 (大端 S16, 单声道)
    fn make_simple_aiff(pcm_data: &[u8], sample_rate: u32, channels: u16, bits: u16) -> Vec<u8> {
        let block_align = channels * (bits / 8);
        let num_sample_frames = if block_align > 0 {
            pcm_data.len() as u32 / block_align as u32
        } else {
            0
        };

        // COMM chunk: 18 bytes
        let comm_size: u32 = 18;
        // SSND chunk: 8 (offset+blockSize) + data
        let ssnd_data_size = pcm_data.len() as u32;
        let ssnd_chunk_size = 8 + ssnd_data_size;
        // FORM size = 4 (AIFF) + 8 (COMM header) + comm_size + 8 (SSND header) + ssnd_chunk_size
        let form_size = 4 + 8 + comm_size + 8 + ssnd_chunk_size;

        let sr_bytes = encode_ieee_extended(sample_rate as f64);

        let mut buf = Vec::new();
        // FORM header
        buf.extend_from_slice(b"FORM");
        buf.extend_from_slice(&form_size.to_be_bytes());
        buf.extend_from_slice(b"AIFF");

        // COMM chunk
        buf.extend_from_slice(b"COMM");
        buf.extend_from_slice(&comm_size.to_be_bytes());
        buf.extend_from_slice(&channels.to_be_bytes());
        buf.extend_from_slice(&num_sample_frames.to_be_bytes());
        buf.extend_from_slice(&bits.to_be_bytes());
        buf.extend_from_slice(&sr_bytes);

        // SSND chunk
        buf.extend_from_slice(b"SSND");
        buf.extend_from_slice(&ssnd_chunk_size.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes()); // offset
        buf.extend_from_slice(&0u32.to_be_bytes()); // blockSize
        buf.extend_from_slice(pcm_data);

        buf
    }

    #[test]
    fn test_探测_aiff_魔数() {
        let aiff = make_simple_aiff(&[0; 4], 44100, 1, 16);
        let probe = AiffProbe;
        assert_eq!(probe.probe(&aiff, None), Some(SCORE_MAX));
    }

    #[test]
    fn test_探测_aiff_扩展名() {
        let probe = AiffProbe;
        assert_eq!(probe.probe(&[], Some("test.aiff")), Some(SCORE_EXTENSION));
        assert_eq!(probe.probe(&[], Some("test.aif")), Some(SCORE_EXTENSION));
        assert_eq!(probe.probe(&[], Some("test.wav")), None);
    }

    #[test]
    fn test_解封装_基本流信息() {
        // 4 采样的 S16BE 单声道 = 8 字节
        let pcm = vec![0x00, 0x01, 0x7F, 0xFF, 0x80, 0x00, 0x00, 0x01];
        let aiff = make_simple_aiff(&pcm, 44100, 1, 16);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(aiff)));
        let mut demuxer = AiffDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);

        let s = &streams[0];
        assert_eq!(s.media_type, MediaType::Audio);
        assert_eq!(s.codec_id, CodecId::PcmS16be);
        assert_eq!(s.nb_frames, 4);
    }

    #[test]
    fn test_解封装_读取数据包() {
        let pcm = vec![0x00, 0x01, 0x7F, 0xFF, 0x80, 0x00, 0x00, 0x01];
        let aiff = make_simple_aiff(&pcm, 44100, 1, 16);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(aiff)));
        let mut demuxer = AiffDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(&pkt.data[..], &pcm[..]);
        assert_eq!(pkt.pts, 0);
        assert_eq!(pkt.duration, 4);
        assert!(pkt.is_keyframe);

        let err = demuxer.read_packet(&mut io).unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_解封装_时长() {
        // 1 秒的 S16BE 单声道: 44100 * 2 = 88200 bytes
        let pcm = vec![0u8; 44100 * 2];
        let aiff = make_simple_aiff(&pcm, 44100, 1, 16);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(aiff)));
        let mut demuxer = AiffDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let duration = demuxer.duration().unwrap();
        assert!((duration - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_ieee_extended_往返() {
        let rates = [8000.0, 11025.0, 22050.0, 44100.0, 48000.0, 96000.0];
        for rate in rates {
            let encoded = encode_ieee_extended(rate);
            let decoded = parse_ieee_extended(&encoded);
            assert!(
                (decoded - rate).abs() < 1.0,
                "rate {} -> encoded {:?} -> decoded {}",
                rate,
                encoded,
                decoded
            );
        }
    }

    #[test]
    fn test_非_form_文件报错() {
        let bad = b"NOT_FORM_DATA_HERE".to_vec();
        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(bad)));
        let mut demuxer = AiffDemuxer::create().unwrap();
        let err = demuxer.open(&mut io).unwrap_err();
        assert!(matches!(err, TaoError::InvalidData(_)));
    }
}
