//! MPEG-TS (Transport Stream) 封装器.
//!
//! 对标 FFmpeg 的 MPEG-TS 封装器, 将音视频数据包封装到 188 字节 TS 包中.
//!
//! 关键结构:
//! - PAT (Program Association Table): 映射 program 到 PMT PID
//! - PMT (Program Map Table): 映射 stream_type 到 ES PID
//! - PES (Packetized Elementary Stream): 包含压缩数据和时间戳
//! - TS Packet: 188 字节固定大小包

use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::Stream;

/// TS 包大小
const TS_PACKET_SIZE: usize = 188;
/// TS 同步字节
const TS_SYNC: u8 = 0x47;
/// PAT PID
#[allow(dead_code)]
const PID_PAT: u16 = 0x0000;
/// PMT PID
const PID_PMT: u16 = 0x1000;
/// 第一个 ES PID
const PID_ES_BASE: u16 = 0x0100;

/// 流信息
struct TsStream {
    pid: u16,
    stream_type: u8,
    continuity_counter: u8,
    media_type: MediaType,
}

/// MPEG-TS 封装器
pub struct MpegTsMuxer {
    /// 流列表
    ts_streams: Vec<TsStream>,
    /// PAT 连续性计数器
    pat_cc: u8,
    /// PMT 连续性计数器
    pmt_cc: u8,
    /// PAT/PMT 写入间隔计数
    psi_counter: u32,
}

impl MpegTsMuxer {
    /// 创建 MPEG-TS 封装器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            ts_streams: Vec::new(),
            pat_cc: 0,
            pmt_cc: 0,
            psi_counter: 0,
        }))
    }

    /// 编解码器 -> stream_type
    fn codec_to_stream_type(codec_id: CodecId) -> TaoResult<u8> {
        match codec_id {
            CodecId::H264 => Ok(0x1B),
            CodecId::H265 => Ok(0x24),
            CodecId::Aac => Ok(0x0F),
            CodecId::Mp3 => Ok(0x03),
            CodecId::PcmS16le | CodecId::PcmS16be => Ok(0x03),
            _ => Err(TaoError::Unsupported(format!(
                "MPEG-TS 不支持编解码器: {}",
                codec_id
            ))),
        }
    }

    /// 写入 PAT
    fn write_pat(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let mut packet = [0u8; TS_PACKET_SIZE];
        packet[0] = TS_SYNC;
        // PID=0, PUSI=1
        packet[1] = 0x40;
        packet[2] = 0x00;
        // AFC=01 (payload only), CC
        packet[3] = 0x10 | (self.pat_cc & 0x0F);
        self.pat_cc = self.pat_cc.wrapping_add(1);

        // Pointer field
        packet[4] = 0x00;

        // PAT 内容
        let pat_start = 5;
        packet[pat_start] = 0x00; // table_id
        // section_syntax_indicator=1, '0', reserved='11'
        // section_length = 9 (5 header + 4 program entry)
        let section_length: u16 = 9;
        packet[pat_start + 1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        packet[pat_start + 2] = section_length as u8;
        // transport_stream_id
        packet[pat_start + 3] = 0x00;
        packet[pat_start + 4] = 0x01;
        // reserved, version=0, current=1
        packet[pat_start + 5] = 0xC1;
        // section_number
        packet[pat_start + 6] = 0x00;
        // last_section_number
        packet[pat_start + 7] = 0x00;
        // program_number=1, PMT PID
        packet[pat_start + 8] = 0x00;
        packet[pat_start + 9] = 0x01;
        packet[pat_start + 10] = 0xE0 | ((PID_PMT >> 8) as u8 & 0x1F);
        packet[pat_start + 11] = PID_PMT as u8;

        // CRC32 (简化: 写入固定值, 实际应计算)
        let crc = crc32_mpeg2(&packet[pat_start..pat_start + 12]);
        let crc_pos = pat_start + 12;
        packet[crc_pos..crc_pos + 4].copy_from_slice(&crc.to_be_bytes());

        // 填充
        for b in &mut packet[crc_pos + 4..TS_PACKET_SIZE] {
            *b = 0xFF;
        }

        io.write_all(&packet)?;
        Ok(())
    }

    /// 写入 PMT
    fn write_pmt(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let mut packet = [0u8; TS_PACKET_SIZE];
        packet[0] = TS_SYNC;
        // PID = PMT, PUSI=1
        packet[1] = 0x40 | ((PID_PMT >> 8) as u8 & 0x1F);
        packet[2] = PID_PMT as u8;
        packet[3] = 0x10 | (self.pmt_cc & 0x0F);
        self.pmt_cc = self.pmt_cc.wrapping_add(1);

        // Pointer field
        packet[4] = 0x00;

        let pmt_start = 5;
        packet[pmt_start] = 0x02; // table_id = PMT

        // 计算 section_length
        // 固定部分: 9 bytes (含 CRC)
        // 每个流: 5 bytes (无 ES 描述)
        let es_info_len = 5 * self.ts_streams.len();
        let section_length = 9 + es_info_len;

        packet[pmt_start + 1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        packet[pmt_start + 2] = section_length as u8;
        // program_number
        packet[pmt_start + 3] = 0x00;
        packet[pmt_start + 4] = 0x01;
        // reserved, version=0, current=1
        packet[pmt_start + 5] = 0xC1;
        packet[pmt_start + 6] = 0x00; // section_number
        packet[pmt_start + 7] = 0x00; // last_section_number

        // PCR PID (使用第一个流)
        let pcr_pid = if !self.ts_streams.is_empty() {
            self.ts_streams[0].pid
        } else {
            PID_ES_BASE
        };
        packet[pmt_start + 8] = 0xE0 | ((pcr_pid >> 8) as u8 & 0x1F);
        packet[pmt_start + 9] = pcr_pid as u8;

        // program_info_length = 0
        packet[pmt_start + 10] = 0xF0;
        packet[pmt_start + 11] = 0x00;

        // ES 信息
        let mut pos = pmt_start + 12;
        for ts_stream in &self.ts_streams {
            packet[pos] = ts_stream.stream_type;
            packet[pos + 1] = 0xE0 | ((ts_stream.pid >> 8) as u8 & 0x1F);
            packet[pos + 2] = ts_stream.pid as u8;
            // ES_info_length = 0
            packet[pos + 3] = 0xF0;
            packet[pos + 4] = 0x00;
            pos += 5;
        }

        // CRC32
        let crc = crc32_mpeg2(&packet[pmt_start..pos]);
        packet[pos..pos + 4].copy_from_slice(&crc.to_be_bytes());
        pos += 4;

        // 填充
        for b in &mut packet[pos..TS_PACKET_SIZE] {
            *b = 0xFF;
        }

        io.write_all(&packet)?;
        Ok(())
    }

    /// 写入 PSI (PAT + PMT)
    fn write_psi(&mut self, io: &mut IoContext) -> TaoResult<()> {
        self.write_pat(io)?;
        self.write_pmt(io)?;
        Ok(())
    }

    /// 将 PES 数据包拆分为 TS 包写入
    fn write_pes(
        io: &mut IoContext,
        pid: u16,
        cc: &mut u8,
        stream_id: u8,
        pts: Option<i64>,
        data: &[u8],
        is_start: bool,
    ) -> TaoResult<()> {
        // 构建 PES 包
        let mut pes = Vec::new();
        if is_start {
            // PES start code
            pes.extend_from_slice(&[0x00, 0x00, 0x01]);
            pes.push(stream_id);

            // PES 可选头
            let has_pts = pts.is_some();
            let optional_len = if has_pts { 8 } else { 3 };
            let pes_length = if data.len() + optional_len <= 65535 {
                (data.len() + optional_len) as u16
            } else {
                0 // 无限长度
            };
            pes.extend_from_slice(&pes_length.to_be_bytes());

            // PES flags: data_alignment=1
            pes.push(0x80);
            // PTS flags
            pes.push(if has_pts { 0x80 } else { 0x00 });
            // Header data length
            pes.push(if has_pts { 5 } else { 0 });

            if let Some(pts_val) = pts {
                // 编码 PTS (5 bytes)
                let pts90k = pts_val;
                let mut pts_bytes = [0u8; 5];
                pts_bytes[0] = 0x21 | (((pts90k >> 29) & 0x0E) as u8);
                pts_bytes[1] = ((pts90k >> 22) & 0xFF) as u8;
                pts_bytes[2] = (((pts90k >> 14) & 0xFE) as u8) | 0x01;
                pts_bytes[3] = ((pts90k >> 7) & 0xFF) as u8;
                pts_bytes[4] = (((pts90k << 1) & 0xFE) as u8) | 0x01;
                pes.extend_from_slice(&pts_bytes);
            }
        }
        pes.extend_from_slice(data);

        // 拆分成 TS 包
        let mut offset = 0;
        let mut first = true;

        while offset < pes.len() {
            let mut packet = [0u8; TS_PACKET_SIZE];
            packet[0] = TS_SYNC;

            let pusi = if first { 0x40u8 } else { 0x00u8 };
            packet[1] = pusi | ((pid >> 8) as u8 & 0x1F);
            packet[2] = pid as u8;

            let remaining = pes.len() - offset;
            let payload_max = TS_PACKET_SIZE - 4;

            if remaining >= payload_max {
                // 纯 payload
                packet[3] = 0x10 | (*cc & 0x0F);
                *cc = cc.wrapping_add(1);
                packet[4..TS_PACKET_SIZE].copy_from_slice(&pes[offset..offset + payload_max]);
                offset += payload_max;
            } else {
                // 需要 adaptation field 填充
                let stuff_len = payload_max - remaining;
                if stuff_len >= 2 {
                    // AFC=11 (adaptation + payload)
                    packet[3] = 0x30 | (*cc & 0x0F);
                    *cc = cc.wrapping_add(1);
                    let af_len = (stuff_len - 1) as u8;
                    packet[4] = af_len;
                    if af_len > 0 {
                        packet[5] = 0x00; // flags
                        for b in &mut packet[6..4 + stuff_len] {
                            *b = 0xFF; // stuffing
                        }
                    }
                    let payload_start = 4 + stuff_len;
                    packet[payload_start..payload_start + remaining]
                        .copy_from_slice(&pes[offset..offset + remaining]);
                } else if stuff_len == 1 {
                    packet[3] = 0x30 | (*cc & 0x0F);
                    *cc = cc.wrapping_add(1);
                    packet[4] = 0; // adaptation_field_length = 0
                    packet[5..5 + remaining].copy_from_slice(&pes[offset..offset + remaining]);
                    // 填充剩余
                    for b in &mut packet[5 + remaining..TS_PACKET_SIZE] {
                        *b = 0xFF;
                    }
                } else {
                    packet[3] = 0x10 | (*cc & 0x0F);
                    *cc = cc.wrapping_add(1);
                    packet[4..4 + remaining].copy_from_slice(&pes[offset..offset + remaining]);
                    for b in &mut packet[4 + remaining..TS_PACKET_SIZE] {
                        *b = 0xFF;
                    }
                }
                offset = pes.len();
            }

            io.write_all(&packet)?;
            first = false;
        }

        Ok(())
    }
}

impl Muxer for MpegTsMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::MpegTs
    }

    fn name(&self) -> &str {
        "mpegts"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("MPEG-TS: 没有输入流".into()));
        }

        self.ts_streams.clear();
        for (i, stream) in streams.iter().enumerate() {
            let stream_type = Self::codec_to_stream_type(stream.codec_id)?;
            self.ts_streams.push(TsStream {
                pid: PID_ES_BASE + i as u16,
                stream_type,
                continuity_counter: 0,
                media_type: stream.media_type,
            });
        }

        // 写入初始 PSI
        self.write_psi(io)?;

        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        let idx = packet.stream_index;
        if idx >= self.ts_streams.len() {
            return Err(TaoError::StreamNotFound(idx));
        }

        // 定期重写 PSI
        self.psi_counter += 1;
        if self.psi_counter % 40 == 0 {
            self.write_psi(io)?;
        }

        let pid = self.ts_streams[idx].pid;
        let stream_id = match self.ts_streams[idx].media_type {
            MediaType::Video => 0xE0,
            MediaType::Audio => 0xC0,
            _ => 0xBD,
        };

        let pts = if packet.pts >= 0 {
            Some(packet.pts)
        } else {
            None
        };

        let cc = &mut self.ts_streams[idx].continuity_counter;
        Self::write_pes(io, pid, cc, stream_id, pts, &packet.data, true)?;

        Ok(())
    }

    fn write_trailer(&mut self, _io: &mut IoContext) -> TaoResult<()> {
        // MPEG-TS 不需要特殊的尾部
        Ok(())
    }
}

/// MPEG-2 CRC32 (多项式 0x04C11DB7)
fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04C11DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{IoContext, MemoryBackend};
    use crate::stream::{AudioStreamParams, StreamParams, VideoStreamParams};
    use tao_core::{ChannelLayout, PixelFormat, Rational, SampleFormat};

    fn make_video_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 90000),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(30, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        }
    }

    fn make_audio_stream() -> Stream {
        Stream {
            index: 1,
            media_type: MediaType::Audio,
            codec_id: CodecId::Aac,
            time_base: Rational::new(1, 90000),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::S16,
                bit_rate: 128000,
                frame_size: 1024,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_ts_写入头部() {
        let mut muxer = MpegTsMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream()];
        muxer.write_header(&mut io, &streams).unwrap();
        let pos = io.position().unwrap();
        // PAT + PMT = 2 * 188 = 376 bytes
        assert_eq!(pos, (2 * TS_PACKET_SIZE) as u64);
    }

    #[test]
    fn test_ts_写入数据包() {
        let mut muxer = MpegTsMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream()];
        muxer.write_header(&mut io, &streams).unwrap();

        let mut packet = Packet::from_data(vec![0x00, 0x00, 0x00, 0x01, 0x65, 0xAB, 0xCD]);
        packet.pts = 90000;
        packet.dts = 90000;
        packet.duration = 3000;
        packet.stream_index = 0;
        packet.is_keyframe = true;
        muxer.write_packet(&mut io, &packet).unwrap();

        let pos = io.position().unwrap();
        // PAT + PMT + 至少一个 PES TS 包
        assert!(pos >= (3 * TS_PACKET_SIZE) as u64);
        // 所有包应 188 字节对齐
        assert_eq!(pos % TS_PACKET_SIZE as u64, 0);
    }

    #[test]
    fn test_ts_音视频() {
        let mut muxer = MpegTsMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream(), make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();

        let mut v_pkt = Packet::from_data(vec![0x00, 0x00, 0x00, 0x01, 0x65]);
        v_pkt.pts = 0;
        v_pkt.dts = 0;
        v_pkt.duration = 3000;
        v_pkt.stream_index = 0;
        v_pkt.is_keyframe = true;

        let mut a_pkt = Packet::from_data(vec![0xFF, 0xF1, 0x50, 0x80]);
        a_pkt.pts = 0;
        a_pkt.dts = 0;
        a_pkt.duration = 2090;
        a_pkt.stream_index = 1;
        a_pkt.is_keyframe = true;
        muxer.write_packet(&mut io, &v_pkt).unwrap();
        muxer.write_packet(&mut io, &a_pkt).unwrap();

        let pos = io.position().unwrap();
        assert_eq!(pos % TS_PACKET_SIZE as u64, 0, "所有 TS 包应 188 字节对齐");
    }

    #[test]
    fn test_空流报错() {
        let mut muxer = MpegTsMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        assert!(muxer.write_header(&mut io, &[]).is_err());
    }

    #[test]
    fn test_crc32() {
        // 验证 CRC 函数不崩溃
        let data = [0x00, 0x01, 0x02, 0x03];
        let crc = crc32_mpeg2(&data);
        assert_ne!(crc, 0);
    }
}
