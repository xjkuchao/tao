//! WAV (RIFF WAVE) 解封装器.
//!
//! 支持标准 PCM WAV 文件的读取.
//!
//! WAV 文件结构:
//! ```text
//! RIFF header:  "RIFF" + file_size-8 + "WAVE"
//! fmt  chunk:   "fmt " + chunk_size + audio_format + channels + sample_rate
//!              + byte_rate + block_align + bits_per_sample
//! data chunk:   "data" + data_size + PCM samples...
//! ```

use log::{debug, warn};
use tao_codec::CodecId;
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::stream::{AudioStreamParams, Stream, StreamParams};

/// WAV 音频格式码
const WAV_FORMAT_PCM: u16 = 0x0001;
/// WAV IEEE 浮点格式码
const WAV_FORMAT_IEEE_FLOAT: u16 = 0x0003;

/// WAV 解封装器
pub struct WavDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// data 块在文件中的起始偏移
    data_offset: u64,
    /// data 块的大小 (字节)
    data_size: u64,
    /// 当前读取位置 (相对于 data 块起始)
    data_pos: u64,
    /// 每次读取的数据包大小 (字节), 通常 = block_align * 若干帧
    packet_size: usize,
    /// 块对齐 (每个采样块的字节数)
    block_align: u16,
    /// 采样率 (用于计算时间戳)
    sample_rate: u32,
    /// 元数据
    metadata: Vec<(String, String)>,
}

impl WavDemuxer {
    /// 创建 WAV 解封装器实例 (工厂函数)
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
        }))
    }

    /// 根据 WAV 格式码和位深确定 CodecId
    fn resolve_codec_id(audio_format: u16, bits_per_sample: u16) -> TaoResult<CodecId> {
        match audio_format {
            WAV_FORMAT_PCM => match bits_per_sample {
                8 => Ok(CodecId::PcmU8),
                16 => Ok(CodecId::PcmS16le),
                24 => Ok(CodecId::PcmS24le),
                32 => Ok(CodecId::PcmS32le),
                _ => Err(TaoError::Unsupported(format!(
                    "不支持的 PCM 位深: {}",
                    bits_per_sample
                ))),
            },
            WAV_FORMAT_IEEE_FLOAT => match bits_per_sample {
                32 => Ok(CodecId::PcmF32le),
                _ => Err(TaoError::Unsupported(format!(
                    "不支持的浮点位深: {}",
                    bits_per_sample
                ))),
            },
            _ => Err(TaoError::Unsupported(format!(
                "不支持的 WAV 格式码: 0x{:04X}",
                audio_format
            ))),
        }
    }

    /// 根据 CodecId 确定采样格式
    fn resolve_sample_format(codec_id: CodecId) -> SampleFormat {
        match codec_id {
            CodecId::PcmU8 => SampleFormat::U8,
            CodecId::PcmS16le => SampleFormat::S16,
            CodecId::PcmS24le | CodecId::PcmS32le => SampleFormat::S32,
            CodecId::PcmF32le => SampleFormat::F32,
            _ => SampleFormat::None,
        }
    }
}

impl Demuxer for WavDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Wav
    }

    fn name(&self) -> &str {
        "wav"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 读取 RIFF 头
        let riff_tag = io.read_tag()?;
        if &riff_tag != b"RIFF" {
            return Err(TaoError::InvalidData("不是有效的 RIFF 文件".into()));
        }

        let _file_size = io.read_u32_le()?;

        let wave_tag = io.read_tag()?;
        if &wave_tag != b"WAVE" {
            return Err(TaoError::InvalidData("不是有效的 WAVE 文件".into()));
        }

        debug!("检测到 RIFF/WAVE 文件");

        // 解析各 chunk
        let mut fmt_found = false;
        let mut data_found = false;
        let mut audio_format: u16 = 0;
        let mut channels: u16 = 0;
        let mut sample_rate: u32 = 0;
        let mut _byte_rate: u32 = 0;
        let mut block_align: u16 = 0;
        let mut bits_per_sample: u16 = 0;

        while !data_found {
            let chunk_id = match io.read_tag() {
                Ok(tag) => tag,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };
            let chunk_size = io.read_u32_le()? as u64;
            let chunk_id_str = String::from_utf8_lossy(&chunk_id);

            match &chunk_id {
                b"fmt " => {
                    if chunk_size < 16 {
                        return Err(TaoError::InvalidData("fmt 块大小不足 16 字节".into()));
                    }
                    audio_format = io.read_u16_le()?;
                    channels = io.read_u16_le()?;
                    sample_rate = io.read_u32_le()?;
                    _byte_rate = io.read_u32_le()?;
                    block_align = io.read_u16_le()?;
                    bits_per_sample = io.read_u16_le()?;

                    debug!(
                        "fmt: format={}, channels={}, rate={}, block_align={}, bits={}",
                        audio_format, channels, sample_rate, block_align, bits_per_sample,
                    );

                    // 跳过 fmt 块的扩展部分
                    if chunk_size > 16 {
                        io.skip((chunk_size - 16) as usize)?;
                    }
                    fmt_found = true;
                }
                b"data" => {
                    if !fmt_found {
                        return Err(TaoError::InvalidData("data 块出现在 fmt 块之前".into()));
                    }
                    self.data_offset = io.position()?;
                    self.data_size = chunk_size;
                    data_found = true;
                    debug!("data: offset={}, size={}", self.data_offset, self.data_size);
                }
                _ => {
                    // 跳过未知块
                    warn!("跳过未知块: '{}', 大小={}", chunk_id_str, chunk_size);
                    io.skip(chunk_size as usize)?;
                }
            }

            // WAV 块要求偶数对齐, 奇数大小需要跳过 1 个填充字节
            // 但 data 块不在此处跳过 (我们要读取其数据)
            if !data_found && chunk_size % 2 != 0 {
                io.skip(1)?;
            }
        }

        if !fmt_found {
            return Err(TaoError::InvalidData("未找到 fmt 块".into()));
        }
        if !data_found {
            return Err(TaoError::InvalidData("未找到 data 块".into()));
        }

        // 构建流信息
        let codec_id = Self::resolve_codec_id(audio_format, bits_per_sample)?;
        let sample_format = Self::resolve_sample_format(codec_id);
        let channel_layout = ChannelLayout::from_channels(u32::from(channels));
        let time_base = Rational::new(1, sample_rate as i32);

        // 计算总采样数和时长
        let total_samples = if block_align > 0 {
            self.data_size / u64::from(block_align)
        } else {
            0
        };

        let bit_rate = u64::from(sample_rate) * u64::from(channels) * u64::from(bits_per_sample);

        let stream = Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id,
            time_base,
            duration: total_samples as i64,
            start_time: 0,
            nb_frames: total_samples,
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

        // 每个数据包读取 ~4096 采样 (或至少 1 个 block_align)
        let samples_per_packet = 4096u32;
        self.packet_size = (u32::from(block_align) * samples_per_packet) as usize;
        if self.packet_size == 0 {
            self.packet_size = 4096;
        }

        debug!(
            "WAV 打开完成: {} Hz, {} 声道, {} 位, 总采样数={}",
            sample_rate, channels, bits_per_sample, total_samples,
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

        // 计算本次读取大小 (不超过剩余数据)
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

        // 将时间戳 (采样数) 转换为字节偏移
        let sample = timestamp.max(0) as u64;
        let byte_offset = sample * u64::from(self.block_align);
        let byte_offset = byte_offset.min(self.data_size);

        // 对齐到 block_align
        let ba = u64::from(self.block_align);
        let aligned_offset = (byte_offset / ba) * ba;

        io.seek(std::io::SeekFrom::Start(self.data_offset + aligned_offset))?;
        self.data_pos = aligned_offset;

        debug!("WAV seek: 目标采样={}, 字节偏移={}", sample, aligned_offset);
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

/// WAV 格式探测器
pub struct WavProbe;

impl FormatProbe for WavProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        // 检查 RIFF/WAVE 魔数
        if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
            return Some(SCORE_MAX);
        }

        // 仅根据扩展名
        if let Some(name) = filename {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".wav") || lower.ends_with(".wave") {
                return Some(SCORE_EXTENSION);
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Wav
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构建最简单的 WAV 文件数据 (PCM S16LE, 单声道, 44100Hz)
    fn make_simple_wav(pcm_data: &[u8]) -> Vec<u8> {
        let data_size = pcm_data.len() as u32;
        let file_size = 36 + data_size; // RIFF size = total - 8
        let channels: u16 = 1;
        let sample_rate: u32 = 44100;
        let bits_per_sample: u16 = 16;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * u32::from(block_align);

        let mut buf = Vec::new();
        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());
        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        buf.extend_from_slice(pcm_data);
        buf
    }

    #[test]
    fn test_探测_wav_魔数() {
        let wav = make_simple_wav(&[0; 4]);
        let probe = WavProbe;
        assert_eq!(probe.probe(&wav, None), Some(SCORE_MAX));
    }

    #[test]
    fn test_探测_wav_扩展名() {
        let probe = WavProbe;
        assert_eq!(probe.probe(&[], Some("test.wav")), Some(SCORE_EXTENSION));
        assert_eq!(probe.probe(&[], Some("test.mp3")), None);
    }

    #[test]
    fn test_解封装_基本流信息() {
        // 4 采样的 S16LE 单声道数据 = 8 字节
        let pcm = vec![0x00, 0x01, 0xFF, 0x7F, 0x00, 0x80, 0x01, 0x00];
        let wav = make_simple_wav(&pcm);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(wav)));
        let mut demuxer = WavDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);

        let s = &streams[0];
        assert_eq!(s.media_type, MediaType::Audio);
        assert_eq!(s.codec_id, CodecId::PcmS16le);
        assert_eq!(s.nb_frames, 4); // 8 bytes / 2 block_align = 4 samples
    }

    #[test]
    fn test_解封装_读取数据包() {
        let pcm = vec![0x00, 0x01, 0xFF, 0x7F, 0x00, 0x80, 0x01, 0x00];
        let wav = make_simple_wav(&pcm);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(wav)));
        let mut demuxer = WavDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(&pkt.data[..], &pcm[..]);
        assert_eq!(pkt.pts, 0);
        assert_eq!(pkt.duration, 4); // 4 采样
        assert!(pkt.is_keyframe);

        // 下一次读取应该返回 EOF
        let err = demuxer.read_packet(&mut io).unwrap_err();
        assert!(matches!(err, TaoError::Eof));
    }

    #[test]
    fn test_解封装_时长() {
        let pcm = vec![0u8; 44100 * 2]; // 1 秒的 S16LE 单声道
        let wav = make_simple_wav(&pcm);

        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(wav)));
        let mut demuxer = WavDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let duration = demuxer.duration().unwrap();
        assert!((duration - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_非_riff_文件报错() {
        let bad = b"NOT_RIFF_DATA_HERE".to_vec();
        let mut io = IoContext::new(Box::new(MemoryBackend::from_data(bad)));
        let mut demuxer = WavDemuxer::create().unwrap();
        let err = demuxer.open(&mut io).unwrap_err();
        assert!(matches!(err, TaoError::InvalidData(_)));
    }
}
