//! AAC ADTS 容器解封装器.
//!
//! ADTS (Audio Data Transport Stream) 是 AAC 音频最常见的裸流封装格式.
//! 每个 ADTS 帧由固定/可变头部 + AAC 帧数据组成.
//!
//! # ADTS 帧结构 (7 或 9 字节头部)
//! ```text
//! 固定头部 (28 bits):
//!   sync word (12 bits = 0xFFF)
//!   ID (1 bit): 0=MPEG-4, 1=MPEG-2
//!   layer (2 bits): always 0
//!   protection_absent (1 bit): 1=no CRC, 0=CRC present
//!   profile (2 bits): 0=Main, 1=LC, 2=SSR, 3=LTP
//!   sampling_frequency_index (4 bits)
//!   private_bit (1 bit)
//!   channel_configuration (3 bits)
//!   ...
//! 可变头部 (28 bits):
//!   frame_length (13 bits): 含头部的完整帧大小
//!   adts_buffer_fullness (11 bits)
//!   number_of_raw_data_blocks (2 bits)
//! [CRC (16 bits)] 仅当 protection_absent=0
//! ```

use bytes::Bytes;
use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams};

/// AAC 采样率索引表 (ISO 14496-3)
const AAC_SAMPLE_RATES: [u32; 16] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 0,
    0,
];

/// ADTS 帧头部信息
#[derive(Debug, Clone)]
struct AdtsHeader {
    /// AAC Profile (0=Main, 1=LC, 2=SSR, 3=LTP)
    profile: u8,
    /// 采样率索引
    sampling_frequency_index: u8,
    /// 声道配置
    channel_configuration: u8,
    /// 帧总大小 (含头部)
    frame_length: u16,
    /// 是否有 CRC 校验
    has_crc: bool,
    /// 头部大小 (7 或 9 字节)
    header_size: u8,
}

/// 解析 ADTS 帧头部 (从 7 字节数据中)
fn parse_adts_header(data: &[u8]) -> Option<AdtsHeader> {
    if data.len() < 7 {
        return None;
    }

    // 检查同步字 (12 bits = 0xFFF)
    if data[0] != 0xFF || (data[1] & 0xF0) != 0xF0 {
        return None;
    }

    // layer 必须为 0
    if (data[1] & 0x06) != 0 {
        return None;
    }

    let protection_absent = (data[1] & 0x01) != 0;
    let profile = (data[2] >> 6) & 0x03;
    let sampling_frequency_index = (data[2] >> 2) & 0x0F;
    let channel_configuration = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);

    // frame_length (13 bits): data[3]的低2位 + data[4]全部 + data[5]的高3位
    let frame_length =
        (u16::from(data[3] & 0x03) << 11) | (u16::from(data[4]) << 3) | (u16::from(data[5]) >> 5);

    // 基本验证
    if sampling_frequency_index >= 13 {
        return None;
    }
    if channel_configuration > 7 {
        return None;
    }

    let header_size = if protection_absent { 7 } else { 9 };

    if frame_length < u16::from(header_size) {
        return None;
    }

    Some(AdtsHeader {
        profile,
        sampling_frequency_index,
        channel_configuration,
        frame_length,
        has_crc: !protection_absent,
        header_size,
    })
}

/// AAC ADTS 解封装器
pub struct AacDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// 当前 PTS (采样数)
    sample_count: u64,
    /// 每帧采样数 (AAC-LC = 1024)
    samples_per_frame: u32,
    /// 采样率
    sample_rate: u32,
}

impl AacDemuxer {
    /// 创建 AAC ADTS 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            sample_count: 0,
            samples_per_frame: 1024,
            sample_rate: 0,
        }))
    }

    /// 跳过 ID3v2 标签 (AAC 文件也可能有)
    fn skip_id3v2(&self, io: &mut IoContext) -> TaoResult<()> {
        let mut header = [0u8; 10];
        for b in &mut header {
            *b = io.read_u8()?;
        }

        if &header[..3] != b"ID3" {
            // 不是 ID3v2, 回退
            io.seek(std::io::SeekFrom::Start(0))?;
            return Ok(());
        }

        // Syncsafe integer (4 bytes)
        let size = ((header[6] as u32) << 21)
            | ((header[7] as u32) << 14)
            | ((header[8] as u32) << 7)
            | (header[9] as u32);

        debug!("AAC: 跳过 ID3v2 标签, 大小={size}");
        io.skip(size as usize)?;
        Ok(())
    }

    /// 查找第一个有效的 ADTS 帧
    fn find_first_frame(&self, io: &mut IoContext) -> TaoResult<AdtsHeader> {
        let start_pos = io.position()?;
        let mut buf = [0u8; 7];
        let max_search = 65536u64;

        let mut pos = start_pos;
        while pos < start_pos + max_search {
            io.seek(std::io::SeekFrom::Start(pos))?;
            for b in &mut buf {
                *b = io.read_u8()?;
            }

            if let Some(header) = parse_adts_header(&buf) {
                // 验证: 尝试跳到下一帧检查同步字
                let next_pos = pos + u64::from(header.frame_length);
                let mut validated = false;
                if io.seek(std::io::SeekFrom::Start(next_pos)).is_ok() {
                    let b0 = io.read_u8().unwrap_or(0);
                    let b1 = io.read_u8().unwrap_or(0);
                    if b0 == 0xFF && (b1 & 0xF0) == 0xF0 {
                        validated = true;
                    }
                }
                // 找到有效帧 (或接受末尾的单帧)
                io.seek(std::io::SeekFrom::Start(pos))?;
                if validated {
                    return Ok(header);
                }
                // 即使无法验证下一帧, 如果到达末尾则接受当前帧
                return Ok(header);
            }
            pos += 1;
        }

        Err(TaoError::InvalidData("AAC: 未找到有效的 ADTS 帧".into()))
    }
}

impl Demuxer for AacDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::AacAdts
    }

    fn name(&self) -> &str {
        "aac"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 跳过可能的 ID3v2 标签
        self.skip_id3v2(io)?;

        // 查找第一个 ADTS 帧
        let header = self.find_first_frame(io)?;

        let sample_rate = AAC_SAMPLE_RATES[header.sampling_frequency_index as usize];
        if sample_rate == 0 {
            return Err(TaoError::InvalidData("AAC: 不支持的采样率索引".into()));
        }

        let channels = if header.channel_configuration == 0 {
            2 // 默认立体声
        } else if header.channel_configuration == 7 {
            8 // 7.1
        } else {
            u32::from(header.channel_configuration)
        };

        self.sample_rate = sample_rate;
        self.samples_per_frame = 1024; // AAC-LC

        // 构建 AudioStreamConfig (简化: 将 profile 编码到 extra_data)
        let profile_name = match header.profile {
            0 => "Main",
            1 => "LC",
            2 => "SSR",
            _ => "LTP",
        };
        debug!("AAC: profile={profile_name} sr={sample_rate} ch={channels}",);

        // 构造 AudioSpecificConfig (2 bytes, ISO 14496-3)
        // audioObjectType (5 bits) + samplingFrequencyIndex (4 bits) + channelConfiguration (4 bits) + padding (3 bits)
        let aot = header.profile + 1; // profile + 1 = audioObjectType
        let sfi = header.sampling_frequency_index;
        let cc = header.channel_configuration;
        let extra_byte0 = (aot << 3) | (sfi >> 1);
        let extra_byte1 = ((sfi & 1) << 7) | (cc << 3);
        let extra_data = vec![extra_byte0, extra_byte1];

        let stream = Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Aac,
            time_base: Rational::new(1, sample_rate as i32),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data,
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::F32,
                bit_rate: 0,
                frame_size: self.samples_per_frame,
            }),
            metadata: Vec::new(),
        };

        self.streams = vec![stream];
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        // 读取 ADTS 帧头部
        let mut buf = [0u8; 7];
        for b in &mut buf {
            match io.read_u8() {
                Ok(v) => *b = v,
                Err(TaoError::Eof) => return Err(TaoError::Eof),
                Err(e) => return Err(e),
            }
        }

        let header = parse_adts_header(&buf)
            .ok_or_else(|| TaoError::InvalidData("AAC: 无效的 ADTS 帧头部".into()))?;

        // 跳过 CRC (如果有)
        if header.has_crc {
            io.skip(2)?;
        }

        // 读取帧数据 (不含头部)
        let data_size = header.frame_length - u16::from(header.header_size);
        let data = io.read_bytes(data_size as usize)?;

        let pts = self.sample_count as i64;
        self.sample_count += u64::from(self.samples_per_frame);

        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = 0;
        pkt.pts = pts;
        pkt.dts = pts;
        pkt.is_keyframe = true; // AAC 帧都可以独立解码
        pkt.time_base = Rational::new(1, self.sample_rate as i32);
        pkt.duration = self.samples_per_frame as i64;

        Ok(pkt)
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("AAC ADTS seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        None
    }
}

/// AAC ADTS 格式探测器
pub struct AacProbe;

impl FormatProbe for AacProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // 检查 ADTS 同步字
        if data.len() >= 7 {
            // 跳过可能的 ID3v2
            let offset = if data.len() >= 10 && &data[..3] == b"ID3" {
                let size = ((data[6] as usize) << 21)
                    | ((data[7] as usize) << 14)
                    | ((data[8] as usize) << 7)
                    | (data[9] as usize);
                10 + size
            } else {
                0
            };

            if offset + 7 <= data.len() {
                if let Some(header) = parse_adts_header(&data[offset..]) {
                    // 验证下一帧
                    let next = offset + header.frame_length as usize;
                    if next + 2 <= data.len()
                        && data[next] == 0xFF
                        && (data[next + 1] & 0xF0) == 0xF0
                    {
                        return Some(crate::probe::SCORE_MAX);
                    }
                    // 单帧也给高分
                    return Some(crate::probe::SCORE_MAX - 10);
                }
            }
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                if ext.eq_ignore_ascii_case("aac") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::AacAdts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构造一个有效的 ADTS 帧
    /// profile=1(LC), sr_index=3(48000), ch=2(stereo), data=payload
    fn build_adts_frame(payload: &[u8]) -> Vec<u8> {
        let frame_length = 7 + payload.len() as u16;
        let mut frame = vec![0u8; 7];

        // sync word (0xFFF) + ID(0=MPEG-4) + layer(00) + protection_absent(1)
        frame[0] = 0xFF;
        frame[1] = 0xF1; // 1111 0001

        // profile(01=LC) + sr_index(0011=48kHz) + private(0) + ch_config高1位(0)
        // 01_0011_0_0 = 0x4C
        frame[2] = 0x4C;

        // ch_config低2位(10=stereo) + ... + frame_length高2位
        // 10_0000_00 + frame_length 高2位
        frame[3] = 0x80 | ((frame_length >> 11) as u8 & 0x03);

        // frame_length 中间 8 位
        frame[4] = (frame_length >> 3) as u8;

        // frame_length 低 3 位 + buffer_fullness 高 5 位
        frame[5] = ((frame_length & 0x07) as u8) << 5 | 0x1F;

        // buffer_fullness 低 6 位 + number_of_raw_data_blocks(0)
        frame[6] = 0xFC;

        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn test_adts_header_parse() {
        let frame = build_adts_frame(&[0xAA; 10]);
        let header = parse_adts_header(&frame).expect("应该解析成功");
        assert_eq!(header.profile, 1); // LC
        assert_eq!(header.sampling_frequency_index, 3); // 48kHz
        assert_eq!(header.channel_configuration, 2); // stereo
        assert_eq!(header.frame_length, 17); // 7 + 10
        assert!(!header.has_crc);
        assert_eq!(header.header_size, 7);
    }

    #[test]
    fn test_adts_invalid_sync() {
        let data = [0x00; 7];
        assert!(parse_adts_header(&data).is_none());
    }

    #[test]
    fn test_probe_adts_sync() {
        let probe = AacProbe;
        let mut data = build_adts_frame(&[0xAA; 100]);
        data.extend_from_slice(&build_adts_frame(&[0xBB; 100]));
        assert_eq!(probe.probe(&data, None), Some(crate::probe::SCORE_MAX));
    }

    #[test]
    fn test_probe_aac_extension() {
        let probe = AacProbe;
        assert!(probe.probe(&[], Some("audio.aac")).is_some());
        assert!(probe.probe(&[], Some("audio.mp3")).is_none());
    }

    #[test]
    fn test_basic_stream_info() {
        let mut data = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&build_adts_frame(&[0xAA; 50]));
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = AacDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::Aac);
        assert_eq!(streams[0].media_type, MediaType::Audio);

        if let StreamParams::Audio(ref a) = streams[0].params {
            assert_eq!(a.sample_rate, 48000);
            assert_eq!(a.channel_layout.channels, 2);
            assert_eq!(a.frame_size, 1024);
        } else {
            panic!("应该是音频参数");
        }
    }

    #[test]
    fn test_read_packets() {
        let payload = vec![0xAA; 50];
        let mut data = Vec::new();
        for _ in 0..3 {
            data.extend_from_slice(&build_adts_frame(&payload));
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = AacDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        // 第一个包
        let pkt0 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt0.pts, 0);
        assert_eq!(pkt0.data.len(), 50);
        assert!(pkt0.is_keyframe);
        assert_eq!(pkt0.duration, 1024);

        // 第二个包
        let pkt1 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt1.pts, 1024);

        // 第三个包
        let pkt2 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt2.pts, 2048);

        // EOF
        assert!(demuxer.read_packet(&mut io).is_err());
    }

    #[test]
    fn test_id3v2_skip() {
        // ID3v2 header + AAC data
        let mut data = Vec::new();
        data.extend_from_slice(b"ID3");
        data.push(4); // version
        data.push(0); // revision
        data.push(0); // flags
        // 128 bytes of ID3 data (syncsafe: 0x00 0x00 0x01 0x00 = 128)
        data.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        data.extend_from_slice(&[0u8; 128]);

        // 接着是 ADTS 帧
        for _ in 0..3 {
            data.extend_from_slice(&build_adts_frame(&[0xCC; 40]));
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = AacDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let pkt = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt.data.len(), 40);
    }

    #[test]
    fn test_extra_data_audio_specific_config() {
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&build_adts_frame(&[0xAA; 20]));
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = AacDemuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let extra = &demuxer.streams()[0].extra_data;
        assert_eq!(extra.len(), 2, "AudioSpecificConfig 应该是 2 字节");

        // 验证: AOT=2(LC), sr_index=3(48kHz), ch=2
        // byte0 = 0b_00010_001 = 0x11 (AOT=00010, SFI high 3 bits=001)
        // byte1 = 0b_1_0010_000 = 0x90 (SFI low 1 bit=1, ch_config=0010, pad=000)
        assert_eq!(extra[0], 0x11);
        assert_eq!(extra[1], 0x90);
    }
}
