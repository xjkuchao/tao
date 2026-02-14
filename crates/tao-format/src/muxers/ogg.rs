//! Ogg 容器封装器.
//!
//! 对标 FFmpeg 的 Ogg 封装器, 将音视频数据包封装到 Ogg 页面中.
//!
//! # Ogg 页面结构
//! ```text
//! "OggS" (4 bytes)
//! Version (1 byte = 0)
//! Header Type (1 byte): 0x02=BOS, 0x04=EOS
//! Granule Position (8 bytes, LE)
//! Serial Number (4 bytes, LE)
//! Page Sequence (4 bytes, LE)
//! CRC Checksum (4 bytes, LE)
//! Num Segments (1 byte)
//! Segment Table (N bytes)
//! Page Data
//! ```

use tao_codec::Packet;
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// Ogg 封装器
pub struct OggMuxer {
    /// 每个逻辑流的序列号
    serial_numbers: Vec<u32>,
    /// 每个逻辑流的页面序号
    page_sequences: Vec<u32>,
    /// 每个流的粒度位置追踪
    granule_positions: Vec<i64>,
    /// 头部是否已写入
    header_written: bool,
}

impl OggMuxer {
    /// 创建 Ogg 封装器 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            serial_numbers: Vec::new(),
            page_sequences: Vec::new(),
            granule_positions: Vec::new(),
            header_written: false,
        }))
    }

    /// 写入一个 Ogg 页面
    fn write_page(
        io: &mut IoContext,
        header_type: u8,
        granule_position: i64,
        serial_number: u32,
        page_sequence: u32,
        data: &[u8],
    ) -> TaoResult<()> {
        // 段表: 每段最多 255 字节
        let mut segments = Vec::new();
        let mut remaining = data.len();
        while remaining >= 255 {
            segments.push(255u8);
            remaining -= 255;
        }
        segments.push(remaining as u8);

        let num_segments = segments.len() as u8;

        // 构建页面头部 (不含 CRC)
        let mut header = Vec::with_capacity(27 + segments.len());
        header.extend_from_slice(b"OggS");        // capture pattern
        header.push(0);                             // version
        header.push(header_type);                   // header type
        header.extend_from_slice(&granule_position.to_le_bytes()); // granule
        header.extend_from_slice(&serial_number.to_le_bytes());    // serial
        header.extend_from_slice(&page_sequence.to_le_bytes());    // page seq
        header.extend_from_slice(&0u32.to_le_bytes());             // CRC placeholder
        header.push(num_segments);                  // num segments
        header.extend_from_slice(&segments);        // segment table

        // 计算 CRC
        let crc = ogg_crc(&header, data);
        header[22..26].copy_from_slice(&crc.to_le_bytes());

        // 写入
        io.write_all(&header)?;
        io.write_all(data)?;

        Ok(())
    }
}

impl Muxer for OggMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Ogg
    }

    fn name(&self) -> &str {
        "ogg"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("Ogg: 没有输入流".into()));
        }

        self.serial_numbers.clear();
        self.page_sequences.clear();
        self.granule_positions.clear();

        for (i, stream) in streams.iter().enumerate() {
            let serial = (i as u32 + 1) * 0x12345;
            self.serial_numbers.push(serial);
            self.page_sequences.push(0);
            self.granule_positions.push(0);

            // 写入 BOS (Beginning Of Stream) 页面
            // 包含编解码器标识头
            let codec_header = match &stream.params {
                StreamParams::Audio(audio) => {
                    // 简化的 Vorbis/Opus 头
                    let mut hdr = Vec::new();
                    // 通用头部: 标识字符串
                    let codec_name = stream.codec_id.name();
                    hdr.push(codec_name.len() as u8);
                    hdr.extend_from_slice(codec_name.as_bytes());
                    // 采样率
                    hdr.extend_from_slice(&audio.sample_rate.to_le_bytes());
                    // 声道数
                    hdr.push(audio.channel_layout.channels as u8);
                    hdr
                }
                StreamParams::Video(video) => {
                    let mut hdr = Vec::new();
                    let codec_name = stream.codec_id.name();
                    hdr.push(codec_name.len() as u8);
                    hdr.extend_from_slice(codec_name.as_bytes());
                    hdr.extend_from_slice(&video.width.to_le_bytes());
                    hdr.extend_from_slice(&video.height.to_le_bytes());
                    hdr
                }
                _ => {
                    let mut hdr = Vec::new();
                    hdr.push(4);
                    hdr.extend_from_slice(b"data");
                    hdr
                }
            };

            // 如果有 extra_data, 使用它替代简化头
            let header_data = if !stream.extra_data.is_empty() {
                &stream.extra_data
            } else {
                &codec_header
            };

            Self::write_page(
                io,
                0x02, // BOS
                0,    // granule = 0 for BOS
                serial,
                0,
                header_data,
            )?;
            self.page_sequences[i] = 1;
        }

        self.header_written = true;
        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        if !self.header_written {
            return Err(TaoError::Codec("Ogg: 头部尚未写入".into()));
        }

        let idx = packet.stream_index;
        if idx >= self.serial_numbers.len() {
            return Err(TaoError::StreamNotFound(idx));
        }

        let serial = self.serial_numbers[idx];
        let page_seq = self.page_sequences[idx];

        // 更新粒度位置 (基于 PTS + duration)
        let granule = if packet.pts >= 0 {
            packet.pts + packet.duration
        } else {
            self.granule_positions[idx] + packet.duration
        };
        self.granule_positions[idx] = granule;

        Self::write_page(io, 0x00, granule, serial, page_seq, &packet.data)?;
        self.page_sequences[idx] = page_seq + 1;

        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 写入 EOS 页面
        for (i, &serial) in self.serial_numbers.iter().enumerate() {
            let page_seq = self.page_sequences[i];
            let granule = self.granule_positions[i];
            Self::write_page(io, 0x04, granule, serial, page_seq, &[])?;
        }
        Ok(())
    }
}

/// Ogg CRC-32 (多项式 0x04C11DB7, 直接算法)
fn ogg_crc(header: &[u8], data: &[u8]) -> u32 {
    static CRC_TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut crc = i << 24;
            for _ in 0..8 {
                if crc & 0x80000000 != 0 {
                    crc = (crc << 1) ^ 0x04C11DB7;
                } else {
                    crc <<= 1;
                }
            }
            table[i as usize] = crc;
        }
        table
    });

    let mut crc = 0u32;
    for &byte in header.iter().chain(data.iter()) {
        let idx = ((crc >> 24) ^ byte as u32) & 0xFF;
        crc = (crc << 8) ^ CRC_TABLE[idx as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{IoContext, MemoryBackend};
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat};
    use crate::stream::AudioStreamParams;

    fn make_audio_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Flac,
            time_base: Rational::new(1, 44100),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::S16,
                bit_rate: 0,
                frame_size: 1024,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_ogg_写入头部() {
        let mut muxer = OggMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();
        let pos = io.position().unwrap();
        assert!(pos > 27, "OggS 页面应至少 27 字节头部");
    }

    #[test]
    fn test_ogg_写入数据包() {
        let mut muxer = OggMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();

        let mut packet = Packet::from_data(vec![1u8, 2, 3, 4, 5]);
        packet.pts = 0;
        packet.dts = 0;
        packet.duration = 1024;
        packet.stream_index = 0;
        packet.is_keyframe = true;
        muxer.write_packet(&mut io, &packet).unwrap();

        let pos = io.position().unwrap();
        // BOS 页面 + 数据页面, 各至少 28 字节
        assert!(pos > 56, "应写入两个 Ogg 页面");
    }

    #[test]
    fn test_ogg_写入尾部() {
        let mut muxer = OggMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_audio_stream()];
        muxer.write_header(&mut io, &streams).unwrap();
        muxer.write_trailer(&mut io).unwrap();

        let pos = io.position().unwrap();
        // BOS + EOS 页面
        assert!(pos > 54, "应写入 BOS 和 EOS 页面");
    }

    #[test]
    fn test_空流报错() {
        let mut muxer = OggMuxer::create().unwrap();
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        assert!(muxer.write_header(&mut io, &[]).is_err());
    }
}
