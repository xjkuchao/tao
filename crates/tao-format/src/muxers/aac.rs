//! AAC ADTS 封装器.
//!
//! 将原始 AAC 帧包装 ADTS 头部, 输出为 .aac 文件.
//!
//! ADTS (Audio Data Transport Stream) 是 AAC 的流式传输格式,
//! 每个 AAC 帧前有 7 字节的 ADTS 头部.

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// AAC 采样率索引表 (ISO 13818-7)
const SAMPLE_RATE_TABLE: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

/// AAC ADTS 封装器
pub struct AacAdtsMuxer {
    /// 采样率索引 (0-12)
    sample_rate_index: u8,
    /// 声道配置 (1=单声道, 2=立体声, 等)
    channel_config: u8,
    /// AAC 配置档次 (1 = AAC-LC)
    profile: u8,
}

impl AacAdtsMuxer {
    /// 创建 AAC ADTS 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            sample_rate_index: 0,
            channel_config: 0,
            profile: 1, // AAC-LC
        }))
    }

    /// 根据采样率查找索引
    fn sample_rate_to_index(sample_rate: u32) -> TaoResult<u8> {
        SAMPLE_RATE_TABLE
            .iter()
            .position(|&r| r == sample_rate)
            .map(|i| i as u8)
            .ok_or_else(|| {
                TaoError::InvalidArgument(format!("AAC 不支持的采样率: {} Hz", sample_rate))
            })
    }

    /// 声道数转 ADTS channel_config (1-7 直接映射)
    fn channels_to_config(channels: u32) -> TaoResult<u8> {
        if (1..=7).contains(&channels) {
            Ok(channels as u8)
        } else {
            Err(TaoError::InvalidArgument(format!(
                "AAC 不支持的声道数: {}",
                channels
            )))
        }
    }

    /// 写入单个 ADTS 头部 (7 字节)
    fn write_adts_header(
        io: &mut IoContext,
        data_len: usize,
        sample_rate_index: u8,
        channel_config: u8,
        profile: u8,
    ) -> TaoResult<()> {
        let frame_length = 7 + data_len;
        if frame_length > 8191 {
            return Err(TaoError::InvalidData(format!(
                "AAC 帧长度超出 ADTS 限制: {}",
                frame_length
            )));
        }

        let fl = frame_length as u32;
        let header: [u8; 7] = [
            0xFF,
            0xF1, // sync(4) + ID(0) + Layer(00) + Protection absent(1)
            (profile << 6) | (sample_rate_index << 2) | ((channel_config >> 1) & 1),
            ((channel_config & 1) << 7) | ((fl >> 6) & 0x07) as u8,
            (fl >> 5) as u8,
            ((fl & 0x1F) << 3) as u8 | 0x3F, // buffer fullness 0x7FF 高 5 位
            0x7C,                            // buffer fullness 低 6 位 + raw_data_blocks(0)
        ];
        io.write_all(&header)
    }
}

impl Muxer for AacAdtsMuxer {
    fn format_id(&self) -> FormatId {
        FormatId::AacAdts
    }

    fn name(&self) -> &str {
        "aac"
    }

    fn write_header(&mut self, _io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("AAC ADTS 需要至少一个流".into()));
        }

        if streams.len() != 1 {
            return Err(TaoError::InvalidArgument(
                "AAC ADTS 仅支持单个音频流".into(),
            ));
        }

        let stream = &streams[0];
        if stream.codec_id != CodecId::Aac {
            return Err(TaoError::InvalidArgument(format!(
                "AAC ADTS 需要 AAC 编解码器, 当前: {}",
                stream.codec_id
            )));
        }

        let audio = match &stream.params {
            StreamParams::Audio(a) => a,
            _ => {
                return Err(TaoError::InvalidArgument("AAC ADTS 仅支持音频流".into()));
            }
        };

        self.sample_rate_index = Self::sample_rate_to_index(audio.sample_rate)?;
        self.channel_config = Self::channels_to_config(audio.channel_layout.channels)?;

        debug!(
            "AAC ADTS 写入头部: {} Hz (index={}), {} 声道",
            audio.sample_rate, self.sample_rate_index, audio.channel_layout.channels,
        );

        // ADTS 无全局头部, 头部信息在每帧中
        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        if packet.data.is_empty() {
            return Ok(());
        }

        Self::write_adts_header(
            io,
            packet.data.len(),
            self.sample_rate_index,
            self.channel_config,
            self.profile,
        )?;
        io.write_all(&packet.data)?;
        Ok(())
    }

    fn write_trailer(&mut self, _io: &mut IoContext) -> TaoResult<()> {
        // ADTS 为无头部流格式, 无需写入尾部
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;
    use tao_codec::CodecId;
    use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat};

    use crate::stream::{AudioStreamParams, StreamParams};

    fn make_aac_stream(sample_rate: u32, channels: u32) -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Audio,
            codec_id: CodecId::Aac,
            time_base: Rational::new(1, sample_rate as i32),
            duration: 0,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate,
                channel_layout: ChannelLayout::from_channels(channels),
                sample_format: SampleFormat::S16,
                bit_rate: 0,
                frame_size: 1024,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_aac_write_header() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_aac_stream(44100, 2);
        let mut muxer = AacAdtsMuxer::create().unwrap();

        muxer.write_header(&mut io, &[stream]).unwrap();
        muxer.write_trailer(&mut io).unwrap();
    }

    #[test]
    fn test_aac_write_packets() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let stream = make_aac_stream(44100, 1);
        let mut muxer = AacAdtsMuxer::create().unwrap();
        muxer.write_header(&mut io, &[stream]).unwrap();

        // 模拟原始 AAC 帧数据 (任意字节)
        let data = vec![0x12, 0x34, 0x56, 0x78];
        let pkt = Packet::from_data(bytes::Bytes::from(data));
        muxer.write_packet(&mut io, &pkt).unwrap();
        muxer.write_trailer(&mut io).unwrap();

        // 验证 ADTS 同步字 (前 2 字节应为 0xFF 0xF1)
        io.seek(std::io::SeekFrom::Start(0)).unwrap();
        let size = io.size().unwrap() as usize;
        let output = io.read_bytes(size).unwrap();
        assert!(output.len() >= 7, "应至少有 7 字节 ADTS 头");
        assert_eq!(output[0], 0xFF, "ADTS 同步字第一字节");
        assert_eq!(output[1] & 0xF0, 0xF0, "ADTS 同步字第二字节高 4 位");
        assert_eq!(
            &output[7..11],
            &[0x12, 0x34, 0x56, 0x78],
            "原始数据应紧跟头部"
        );
    }

    #[test]
    fn test_aac_empty_stream_error() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));

        let mut muxer = AacAdtsMuxer::create().unwrap();
        let err = muxer.write_header(&mut io, &[]).unwrap_err();
        assert!(matches!(err, TaoError::InvalidArgument(_)));
    }
}
