//! MP3 (MPEG Audio Layer III) 容器解封装器.
//!
//! MP3 文件结构:
//! ```text
//! [ID3v2 标签 (可选)]
//! [MPEG 音频帧 #0]
//!   ├── 帧同步码 (11 bits = 0x7FF)
//!   ├── 帧头 (版本, 层, 比特率, 采样率, 声道模式等)
//!   └── 帧数据 (压缩音频)
//! [MPEG 音频帧 #1]
//! ...
//! [ID3v1 标签 (可选, 文件末尾 128 字节)]
//! ```
//!
//! 帧头结构 (32 位):
//! ```text
//! AAAA AAAA  AAAB BCCD  EEEE FFGH  IIJJ KLMM
//! A = 同步位 (11 bit, 全1)   B = MPEG 版本    C = 层
//! D = CRC 保护              E = 比特率索引    F = 采样率索引
//! G = 填充位                H = 私有位        I = 声道模式
//! J = 模式扩展              K = 版权         L = 原始/复制
//! M = 强调
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

/// MPEG 音频版本
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MpegVersion {
    /// MPEG-1
    V1,
    /// MPEG-2
    V2,
    /// MPEG-2.5
    V25,
}

/// MPEG 音频帧头部信息
#[derive(Debug, Clone)]
struct FrameHeader {
    /// MPEG 版本
    version: MpegVersion,
    /// 层 (1, 2, 3)
    layer: u8,
    /// 是否有 CRC 校验
    _has_crc: bool,
    /// 比特率 (bps)
    bitrate: u32,
    /// 采样率 (Hz)
    sample_rate: u32,
    /// 填充字节数 (0 或 1)
    _padding: u32,
    /// 声道模式 (0=立体声, 1=联合立体声, 2=双声道, 3=单声道)
    channel_mode: u8,
    /// 帧总字节数 (含头部)
    frame_size: u32,
    /// 每帧采样数
    samples_per_frame: u32,
}

/// MPEG-1 Layer III 比特率表 (kbps), 索引 1-14
const BITRATE_V1_L3: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

/// MPEG-2/2.5 Layer III 比特率表 (kbps), 索引 1-14
const BITRATE_V2_L3: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// MPEG-1 Layer II 比特率表 (kbps)
const BITRATE_V1_L2: [u32; 15] = [
    0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
];

/// MPEG-1 Layer I 比特率表 (kbps)
const BITRATE_V1_L1: [u32; 15] = [
    0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
];

/// MPEG-1 采样率表
const SAMPLERATE_V1: [u32; 3] = [44100, 48000, 32000];

/// 解析 4 字节帧头
fn parse_frame_header(header: u32) -> Option<FrameHeader> {
    // 检查同步位 (bit 31-21 必须全为 1)
    if (header >> 21) != 0x7FF {
        return None;
    }

    // MPEG 版本 (bit 20-19)
    let version = match (header >> 19) & 0x03 {
        0 => MpegVersion::V25,
        2 => MpegVersion::V2,
        3 => MpegVersion::V1,
        _ => return None, // 1 = reserved
    };

    // 层 (bit 18-17)
    let layer = match (header >> 17) & 0x03 {
        1 => 3,           // Layer III
        2 => 2,           // Layer II
        3 => 1,           // Layer I
        _ => return None, // 0 = reserved
    };

    // CRC (bit 16)
    let has_crc = ((header >> 16) & 1) == 0;

    // 比特率索引 (bit 15-12)
    let br_idx = ((header >> 12) & 0x0F) as usize;
    if br_idx == 0 || br_idx == 15 {
        return None; // free / bad
    }

    let bitrate_kbps = match (version, layer) {
        (MpegVersion::V1, 3) => BITRATE_V1_L3[br_idx],
        (MpegVersion::V1, 2) => BITRATE_V1_L2[br_idx],
        (MpegVersion::V1, 1) => BITRATE_V1_L1[br_idx],
        (_, 3) => BITRATE_V2_L3[br_idx],
        (_, 2) => BITRATE_V2_L3[br_idx],
        (_, 1) => BITRATE_V1_L2[br_idx], // MPEG-2/2.5 Layer I 使用与 V1 L2 相同的表
        _ => return None,
    };
    let bitrate = bitrate_kbps * 1000;

    // 采样率索引 (bit 11-10)
    let sr_idx = ((header >> 10) & 0x03) as usize;
    if sr_idx == 3 {
        return None; // reserved
    }
    let sample_rate = match version {
        MpegVersion::V1 => SAMPLERATE_V1[sr_idx],
        MpegVersion::V2 => SAMPLERATE_V1[sr_idx] / 2,
        MpegVersion::V25 => SAMPLERATE_V1[sr_idx] / 4,
    };

    // 填充 (bit 9)
    let padding = (header >> 9) & 1;

    // 声道模式 (bit 7-6)
    let channel_mode = ((header >> 6) & 0x03) as u8;

    // 每帧采样数
    let samples_per_frame = match (version, layer) {
        (MpegVersion::V1, 1) => 384,
        (MpegVersion::V1, 2) => 1152,
        (MpegVersion::V1, 3) => 1152,
        (_, 1) => 384,
        (_, 2) => 1152,
        (_, 3) => 576,
        _ => return None,
    };

    // 帧大小计算
    let frame_size = if layer == 1 {
        (12 * bitrate / sample_rate + padding) * 4
    } else {
        let factor = if matches!(version, MpegVersion::V1) {
            144
        } else {
            72
        };
        factor * bitrate / sample_rate + padding
    };

    if frame_size < 4 {
        return None;
    }

    Some(FrameHeader {
        version,
        layer,
        _has_crc: has_crc,
        bitrate,
        sample_rate,
        _padding: padding,
        channel_mode,
        frame_size,
        samples_per_frame,
    })
}

/// MP3 解封装器
pub struct Mp3Demuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// 第一帧的偏移
    first_frame_offset: u64,
    /// 当前 PTS (累积采样数)
    current_pts: i64,
    /// 每帧采样数
    samples_per_frame: u32,
    /// 总帧数 (来自 Xing/VBRI 头, 0 表示未知)
    total_frames: u64,
    /// 已读取的帧数
    frames_read: u64,
    /// Encoder delay (来自 LAME/iTunSMPB gapless 信息, 单位: 样本)
    encoder_delay: u32,
    /// Trailing padding (来自 LAME/iTunSMPB gapless 信息, 单位: 样本)
    encoder_padding: u32,
}

impl Mp3Demuxer {
    /// 创建 MP3 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            first_frame_offset: 0,
            current_pts: 0,
            samples_per_frame: 1152,
            total_frames: 0,
            frames_read: 0,
            encoder_delay: 0,
            encoder_padding: 0,
        }))
    }

    /// 跳过 ID3v2 标签
    fn skip_id3v2(io: &mut IoContext) -> TaoResult<u64> {
        let mut header = [0u8; 10];
        io.read_exact(&mut header)?;

        // 检查 "ID3" 标识
        if &header[0..3] != b"ID3" {
            // 不是 ID3v2, 回退
            io.seek(std::io::SeekFrom::Start(0))?;
            return Ok(0);
        }

        // ID3v2 大小 (syncsafe integer, 4 bytes, 每字节只用 7 位)
        let size = u64::from(header[6] & 0x7F) << 21
            | u64::from(header[7] & 0x7F) << 14
            | u64::from(header[8] & 0x7F) << 7
            | u64::from(header[9] & 0x7F);

        let total_tag_size = 10 + size;
        io.seek(std::io::SeekFrom::Start(total_tag_size))?;
        debug!("MP3: 跳过 ID3v2 标签, 大小={total_tag_size} 字节");
        Ok(total_tag_size)
    }

    /// 同步到第一个有效帧
    fn find_first_frame(io: &mut IoContext) -> TaoResult<(u64, FrameHeader)> {
        let start = io.position()?;
        let mut buf = [0u8; 4];

        // 最多搜索 64KB
        let limit = start + 65536;
        let mut pos = start;

        while pos < limit {
            io.seek(std::io::SeekFrom::Start(pos))?;
            if io.read_exact(&mut buf).is_err() {
                break;
            }

            let header_val = u32::from_be_bytes(buf);
            if let Some(fh) = parse_frame_header(header_val) {
                // 验证: 检查下一帧也是有效的
                let next_pos = pos + u64::from(fh.frame_size);
                if io.seek(std::io::SeekFrom::Start(next_pos)).is_ok() {
                    let mut next_buf = [0u8; 4];
                    if io.read_exact(&mut next_buf).is_ok() {
                        let next_val = u32::from_be_bytes(next_buf);
                        if parse_frame_header(next_val).is_some() {
                            io.seek(std::io::SeekFrom::Start(pos))?;
                            return Ok((pos, fh));
                        }
                    }
                }
            }
            pos += 1;
        }

        Err(TaoError::InvalidData(
            "MP3: 未找到有效的 MPEG 音频帧".into(),
        ))
    }

    /// 尝试解析 Xing/Info 或 VBRI 头
    /// 返回 (total_frames, encoder_delay, encoder_padding)
    fn parse_vbr_header(
        io: &mut IoContext,
        frame_offset: u64,
        fh: &FrameHeader,
    ) -> TaoResult<(Option<u64>, u32, u32)> {
        // Xing/Info 头部偏移取决于版本和声道
        let xing_offset = match (fh.version, fh.channel_mode) {
            (MpegVersion::V1, 3) => 17, // 单声道
            (MpegVersion::V1, _) => 32, // 立体声
            (_, 3) => 9,                // MPEG-2/2.5 单声道
            (_, _) => 17,               // MPEG-2/2.5 立体声
        };

        let abs_offset = frame_offset + 4 + xing_offset as u64;
        io.seek(std::io::SeekFrom::Start(abs_offset))?;
        let mut tag = [0u8; 4];
        if io.read_exact(&mut tag).is_err() {
            return Ok((None, 0, 0));
        }

        if &tag == b"Xing" || &tag == b"Info" {
            let flags = io.read_u32_be()?;

            // 可选字段: frames, bytes, toc, quality (每个 4/4/100/4 字节)
            let total_frames = if (flags & 0x1) != 0 {
                Some(u64::from(io.read_u32_be()?))
            } else {
                None
            };
            if (flags & 0x2) != 0 {
                // 跳过 total_bytes
                let _ = io.read_u32_be();
            }
            if (flags & 0x4) != 0 {
                // 跳过 TOC (100 字节)
                let mut toc = [0u8; 100];
                let _ = io.read_exact(&mut toc);
            }
            if (flags & 0x8) != 0 {
                // 跳过 quality
                let _ = io.read_u32_be();
            }

            // 读取 LAME/Lavc 扩展头: 9 字节版本字符串 + gapless 信息
            // LAME tag 布局 (从 Xing tag 可选字段之后):
            //   [0..8]   = 编码器版本字符串, 9 字节 (如 "LAME3.99r" 或 "Lavc62.11")
            //   [9]      = 信息标签版本(高4位) + VBR方法(低4位)
            //   [10]     = 低通滤波器频率
            //   [11..14] = Peak Signal Amplitude (4字节)
            //   [15..18] = Radio Replay Gain (4字节)
            //   [19]     = 编码标志 + ATH类型
            //   [20]     = 比特率
            //   [21..23] = encoder delay/padding (3字节)  ← 正确偏移
            //     encoder_delay = (buf[0] << 4) | (buf[1] >> 4)   [12 bit]
            //     encoder_padding = ((buf[1] & 0xF) << 8) | buf[2] [12 bit]
            // 注意: FFmpeg (libavcodec) 编码的文件以 "Lavc" 开头, 但仍遵循相同布局
            let mut lame_buf = [0u8; 24];
            if io.read_exact(&mut lame_buf).is_ok() {
                let d = &lame_buf[21..24];
                let encoder_delay = ((d[0] as u32) << 4) | ((d[1] as u32) >> 4);
                let encoder_padding = (((d[1] as u32) & 0xF) << 8) | (d[2] as u32);
                // 仅当延迟/填充值在合理范围内才接受 (encoder_delay <= 2880, padding <= 2880)
                if encoder_delay <= 2880
                    && encoder_padding <= 2880
                    && (encoder_delay > 0 || encoder_padding > 0)
                {
                    let encoder_tag = &lame_buf[0..4];
                    debug!(
                        "MP3: 发现编码器扩展头 ({:?}), delay={encoder_delay}, padding={encoder_padding}, frames={total_frames:?}",
                        std::str::from_utf8(encoder_tag).unwrap_or("?")
                    );
                    return Ok((total_frames, encoder_delay, encoder_padding));
                }
            }

            debug!("MP3: 发现 Xing 头 (无有效 gapless 扩展), frames={total_frames:?}");
            return Ok((total_frames, 0, 0));
        }

        // 检查 VBRI 头 (固定在帧头+36 字节处)
        let vbri_offset = frame_offset + 4 + 32;
        io.seek(std::io::SeekFrom::Start(vbri_offset))?;
        if io.read_exact(&mut tag).is_ok() && &tag == b"VBRI" {
            let _version = io.read_u16_be()?;
            let _delay = io.read_u16_be()?;
            let _quality = io.read_u16_be()?;
            let _total_bytes = io.read_u32_be()?;
            let total_frames = u64::from(io.read_u32_be()?);
            debug!("MP3: 发现 VBRI 头, frames={total_frames}");
            return Ok((Some(total_frames), 0, 0));
        }

        Ok((None, 0, 0))
    }
}

impl Demuxer for Mp3Demuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Mp3Container
    }

    fn name(&self) -> &str {
        "mp3"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 1) 跳过 ID3v2
        Self::skip_id3v2(io)?;

        // 2) 找到第一个有效帧
        let (frame_offset, fh) = Self::find_first_frame(io)?;
        self.first_frame_offset = frame_offset;
        self.samples_per_frame = fh.samples_per_frame;

        // 3) 尝试解析 VBR 头 (含 LAME gapless 信息)
        if let Ok((frames_opt, delay, padding)) = Self::parse_vbr_header(io, frame_offset, &fh) {
            if let Some(frames) = frames_opt {
                self.total_frames = frames;
            }
            self.encoder_delay = delay;
            self.encoder_padding = padding;
            // Xing/Info 帧本身不算数据帧, 跳过它
            self.first_frame_offset = frame_offset + u64::from(fh.frame_size);
        }

        // 4) 创建流
        let channels = if fh.channel_mode == 3 { 1u32 } else { 2u32 };
        let codec_id = match fh.layer {
            3 => CodecId::Mp3,
            2 => CodecId::Mp2,
            _ => CodecId::Mp3,
        };

        let time_base = Rational::new(1, fh.sample_rate as i32);
        let duration = if self.total_frames > 0 {
            (self.total_frames as i64) * i64::from(fh.samples_per_frame)
        } else {
            -1
        };

        // 将 gapless 信息编码到 extra_data:
        //   [0..4]  front_skip (le u32, 每通道, = encoder_delay + decoder_latency)
        //   [4..8]  encoder_padding (le u32, 保留供参考)
        //   [8..16] valid_samples_total (le u64, 每通道)
        //
        // 解码器跳过 front_skip 个样本, 共输出 valid_samples_total 个样本 (每通道).
        // valid_samples_total = total_frames * spf - encoder_delay - encoder_padding
        //   (纯 LAME 公式, 不考虑 decoder_latency, 因为 front_skip 已包含 decoder_latency)
        //
        // MP3 解码器固有延迟 (decoder_latency) = 529 样本 (MPEG1 Layer3 标准)
        // 此值由 FFmpeg 规范确认: skip_samples = encoder_delay + 529
        const MP3_DECODER_LATENCY: u32 = 529;
        let extra_data = if self.encoder_delay > 0 || self.encoder_padding > 0 {
            let front_skip = self.encoder_delay + MP3_DECODER_LATENCY;
            let valid_total = if self.total_frames > 0 {
                let total_spf = self.total_frames * fh.samples_per_frame as u64;
                total_spf
                    .saturating_sub(self.encoder_delay as u64)
                    .saturating_sub(self.encoder_padding as u64)
            } else {
                0u64
            };
            let mut buf = [0u8; 16];
            buf[0..4].copy_from_slice(&front_skip.to_le_bytes());
            buf[4..8].copy_from_slice(&self.encoder_padding.to_le_bytes());
            buf[8..16].copy_from_slice(&valid_total.to_le_bytes());
            buf.to_vec()
        } else {
            Vec::new()
        };

        let stream = Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id,
            time_base,
            duration,
            start_time: 0,
            nb_frames: self.total_frames,
            extra_data,
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: fh.sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::F32,
                bit_rate: u64::from(fh.bitrate),
                frame_size: fh.samples_per_frame,
            }),
            metadata: Vec::new(),
        };

        debug!(
            "MP3: {:?} Layer {} {}Hz {}ch {}kbps",
            fh.version,
            fh.layer,
            fh.sample_rate,
            channels,
            fh.bitrate / 1000,
        );

        self.streams.push(stream);

        // 定位到第一个数据帧
        io.seek(std::io::SeekFrom::Start(self.first_frame_offset))?;

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        // 读取帧头
        let mut header_buf = [0u8; 4];
        loop {
            let pos = io.position()?;
            match io.read_exact(&mut header_buf) {
                Ok(()) => {}
                Err(TaoError::Eof) => return Err(TaoError::Eof),
                Err(e) => return Err(e),
            }

            let header_val = u32::from_be_bytes(header_buf);
            if let Some(fh) = parse_frame_header(header_val) {
                if fh.frame_size < 4 {
                    continue;
                }

                // 读取帧数据 (含头部)
                let data_size = fh.frame_size as usize;
                let mut frame_data = vec![0u8; data_size];
                frame_data[0..4].copy_from_slice(&header_buf);
                if data_size > 4 {
                    io.read_exact(&mut frame_data[4..])?;
                }

                let mut pkt = Packet::from_data(Bytes::from(frame_data));
                pkt.stream_index = 0;
                pkt.pts = self.current_pts;
                pkt.dts = self.current_pts;
                pkt.is_keyframe = true;
                pkt.time_base = self.streams[0].time_base;

                self.current_pts += i64::from(fh.samples_per_frame);
                self.frames_read += 1;

                return Ok(pkt);
            }

            // 同步失败, 往前跳一个字节重试
            io.seek(std::io::SeekFrom::Start(pos + 1))?;
        }
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("MP3 seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        if self.total_frames > 0 && !self.streams.is_empty() {
            let sr = match &self.streams[0].params {
                StreamParams::Audio(a) => a.sample_rate,
                _ => return None,
            };
            if sr > 0 {
                let total_samples = self.total_frames as f64 * self.samples_per_frame as f64;
                return Some(total_samples / sr as f64);
            }
        }
        None
    }
}

/// MP3 格式探测器
pub struct Mp3Probe;

impl FormatProbe for Mp3Probe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // MP3 可带 ID3v2 前缀；若存在，优先检查标签后的真实帧头。
        let mut start = 0usize;
        if data.len() >= 10 && &data[0..3] == b"ID3" {
            let size = ((data[6] & 0x7F) as usize) << 21
                | ((data[7] & 0x7F) as usize) << 14
                | ((data[8] & 0x7F) as usize) << 7
                | (data[9] & 0x7F) as usize;
            start = 10 + size;
        }

        // 检查起始位置是否存在有效帧同步码（含 ID3 偏移场景）
        if data.len() >= start + 4 {
            let header = u32::from_be_bytes([
                data[start],
                data[start + 1],
                data[start + 2],
                data[start + 3],
            ]);
            if parse_frame_header(header).is_some() {
                return Some(crate::probe::SCORE_MAX - 5);
            }
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                if ext.eq_ignore_ascii_case("mp3") {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Mp3Container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    /// 构造一个简单的 MPEG-1 Layer III 帧头
    fn make_mp3_frame_header(bitrate_idx: u8, sr_idx: u8, padding: bool) -> [u8; 4] {
        // 同步: 0xFFE (11 bit) | Version=MPEG1 (11) | Layer III (01) | No CRC (1)
        // = 0xFFFB
        let mut h: u32 = 0xFFFB_0000;
        h |= u32::from(bitrate_idx & 0x0F) << 12;
        h |= u32::from(sr_idx & 0x03) << 10;
        if padding {
            h |= 1 << 9;
        }
        // 声道: 立体声 (00)
        h.to_be_bytes()
    }

    /// 构造一个完整的 MP3 帧 (头部 + 填充)
    fn build_mp3_frame(bitrate_idx: u8, sr_idx: u8, padding: bool) -> Vec<u8> {
        let header = make_mp3_frame_header(bitrate_idx, sr_idx, padding);
        let h_val = u32::from_be_bytes(header);
        let fh = parse_frame_header(h_val).unwrap();
        let mut frame = vec![0u8; fh.frame_size as usize];
        frame[0..4].copy_from_slice(&header);
        frame
    }

    #[test]
    fn test_帧头解析_mpeg1_layer3_128kbps_44100() {
        // bitrate_idx=9 → 128kbps, sr_idx=0 → 44100Hz
        let header = make_mp3_frame_header(9, 0, false);
        let h_val = u32::from_be_bytes(header);
        let fh = parse_frame_header(h_val).unwrap();

        assert_eq!(fh.version, MpegVersion::V1);
        assert_eq!(fh.layer, 3);
        assert_eq!(fh.bitrate, 128_000);
        assert_eq!(fh.sample_rate, 44100);
        assert_eq!(fh.samples_per_frame, 1152);
        // 帧大小 = 144 * 128000 / 44100 = 417
        assert_eq!(fh.frame_size, 417);
    }

    #[test]
    fn test_帧头解析_mpeg1_layer3_320kbps_48000() {
        // bitrate_idx=14 → 320kbps, sr_idx=1 → 48000Hz
        let header = make_mp3_frame_header(14, 1, false);
        let h_val = u32::from_be_bytes(header);
        let fh = parse_frame_header(h_val).unwrap();

        assert_eq!(fh.bitrate, 320_000);
        assert_eq!(fh.sample_rate, 48000);
        assert_eq!(fh.frame_size, 960); // 144*320000/48000
    }

    #[test]
    fn test_帧头解析_无效同步() {
        // 不是有效的同步码
        assert!(parse_frame_header(0x00000000).is_none());
        assert!(parse_frame_header(0x12345678).is_none());
    }

    #[test]
    fn test_探测_id3v2() {
        let probe = Mp3Probe;
        // ID3v2 头 (10字节, size=0) + 有效的 MP3 帧头
        let header = make_mp3_frame_header(9, 0, false);
        let mut data = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(&header);
        assert_eq!(probe.probe(&data, None), Some(crate::probe::SCORE_MAX - 5),);
    }

    #[test]
    fn test_探测_帧同步() {
        let probe = Mp3Probe;
        let header = make_mp3_frame_header(9, 0, false);
        assert!(probe.probe(&header, None).is_some());
    }

    #[test]
    fn test_探测_id3_后非_mp3_帧() {
        let probe = Mp3Probe;
        // ID3(size=0) + "OggS"：不应被识别为 MP3
        let data = b"ID3\x04\x00\x00\x00\x00\x00\x00OggS";
        assert!(probe.probe(data, None).is_none());
    }

    #[test]
    fn test_探测_扩展名() {
        let probe = Mp3Probe;
        assert!(probe.probe(&[], Some("song.mp3")).is_some());
        assert!(probe.probe(&[], Some("video.mp4")).is_none());
    }

    #[test]
    fn test_id3v2_跳过() {
        // 构造 ID3v2 头部 + MP3 帧
        let mut data = Vec::new();
        // ID3v2 头部
        data.extend_from_slice(b"ID3");
        data.push(4); // version major
        data.push(0); // version minor
        data.push(0); // flags
        // size = 100 (syncsafe: 0,0,0,100)
        data.extend_from_slice(&[0, 0, 0, 100]);
        // ID3v2 内容 (100 字节)
        data.extend(std::iter::repeat_n(0u8, 100));
        // 两个连续 MP3 帧
        let frame = build_mp3_frame(9, 0, false);
        data.extend_from_slice(&frame);
        data.extend_from_slice(&frame);

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = Mp3Demuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let streams = demuxer.streams();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec_id, CodecId::Mp3);
    }

    #[test]
    fn test_读取数据包() {
        // 构造 3 个连续帧 (多加一个用于验证)
        let frame = build_mp3_frame(9, 0, false);
        let fh = parse_frame_header(u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]))
            .unwrap();
        let mut data = Vec::new();
        for _ in 0..4 {
            data.extend_from_slice(&frame);
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = Mp3Demuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        // 读取 3 个包 (第 4 个帧是 open 验证用的)
        let pkt0 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt0.stream_index, 0);
        assert_eq!(pkt0.pts, 0);
        assert_eq!(pkt0.data.len(), fh.frame_size as usize);
        assert!(pkt0.is_keyframe);

        let pkt1 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt1.pts, i64::from(fh.samples_per_frame));

        let pkt2 = demuxer.read_packet(&mut io).unwrap();
        assert_eq!(pkt2.pts, 2 * i64::from(fh.samples_per_frame));
    }

    #[test]
    fn test_基本流信息() {
        let frame = build_mp3_frame(9, 0, false); // 128kbps, 44100Hz
        let mut data = Vec::new();
        for _ in 0..3 {
            data.extend_from_slice(&frame);
        }

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));
        let mut demuxer = Mp3Demuxer::create().unwrap();
        demuxer.open(&mut io).unwrap();

        let stream = &demuxer.streams()[0];
        assert_eq!(stream.media_type, MediaType::Audio);
        assert_eq!(stream.codec_id, CodecId::Mp3);

        if let StreamParams::Audio(ref a) = stream.params {
            assert_eq!(a.sample_rate, 44100);
            assert_eq!(a.bit_rate, 128_000);
            assert_eq!(a.frame_size, 1152);
            assert_eq!(a.channel_layout.channels, 2);
        } else {
            panic!("应该是音频流参数");
        }
    }
}
