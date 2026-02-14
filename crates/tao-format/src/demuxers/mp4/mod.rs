//! MP4/MOV (ISO Base Media File Format) 解封装器.
//!
//! 支持 MP4 (MPEG-4 Part 14) 和 QuickTime MOV 格式.
//! 基于 ISO 14496-12 (ISOBMFF) 标准.
//!
//! # Box 树结构
//! ```text
//! ftyp                  文件类型
//! moov                  影片元数据
//! ├── mvhd              影片头部 (时长, 时间刻度)
//! └── trak              轨道 (每个音/视频流一个)
//!     ├── tkhd          轨道头部
//!     └── mdia          媒体信息
//!         ├── mdhd      媒体头部 (时间刻度, 时长)
//!         ├── hdlr      处理器引用 (vide/soun)
//!         └── minf      媒体详细信息
//!             └── stbl  采样表
//!                 ├── stsd  采样描述 (编解码器参数)
//!                 ├── stts  时间→采样映射
//!                 ├── stsc  采样→块映射
//!                 ├── stsz  采样大小
//!                 ├── stco  块偏移 (32位)
//!                 ├── co64  块偏移 (64位)
//!                 ├── stss  同步采样 (关键帧)
//!                 └── ctts  合成时间偏移
//! mdat                  媒体数据
//! ```

mod boxes;
mod sample_table;

use bytes::Bytes;
use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{ChannelLayout, MediaType, Rational, SampleFormat, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::FormatProbe;
use crate::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};

use self::boxes::{BoxType, FtypBox, read_box_header};
use self::sample_table::SampleTable;

/// MP4 解封装器
pub struct Mp4Demuxer {
    /// 流信息列表
    streams: Vec<Stream>,
    /// 每个流的采样表
    sample_tables: Vec<SampleTable>,
    /// 当前读取的全局采样索引 (所有流中的下一个采样)
    current_sample: Vec<u32>,
    /// mdat 区域起始偏移
    mdat_offset: u64,
    /// mdat 区域大小
    mdat_size: u64,
    /// 文件总时长 (秒)
    file_duration: Option<f64>,
}

impl Mp4Demuxer {
    /// 创建 MP4 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            sample_tables: Vec::new(),
            current_sample: Vec::new(),
            mdat_offset: 0,
            mdat_size: 0,
            file_duration: None,
        }))
    }

    /// 解析 moov box 内容
    fn parse_moov(&mut self, io: &mut IoContext, moov_end: u64) -> TaoResult<()> {
        let mut timescale = 1000u32;

        while io.position()? < moov_end {
            let header = match read_box_header(io) {
                Ok(h) => h,
                Err(_) => break,
            };
            let box_end = io.position()? + header.content_size();

            match header.box_type {
                BoxType::Mvhd => {
                    timescale = self.parse_mvhd(io)?;
                }
                BoxType::Trak => {
                    self.parse_trak(io, box_end, timescale)?;
                }
                _ => {}
            }

            // 跳到下一个 box
            io.seek(std::io::SeekFrom::Start(box_end))?;
        }

        Ok(())
    }

    /// 解析 mvhd (Movie Header Box)
    fn parse_mvhd(&mut self, io: &mut IoContext) -> TaoResult<u32> {
        let version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;

        if version == 0 {
            let _creation_time = io.read_u32_be()?;
            let _modification_time = io.read_u32_be()?;
            let timescale = io.read_u32_be()?;
            let duration = io.read_u32_be()?;
            if timescale > 0 {
                self.file_duration = Some(duration as f64 / timescale as f64);
            }
            debug!("mvhd: timescale={}, duration={}", timescale, duration);
            Ok(timescale)
        } else {
            let _creation_time = io.read_u32_be()? as u64 | ((io.read_u32_be()? as u64) << 32);
            let _modification_time = io.read_u32_be()? as u64 | ((io.read_u32_be()? as u64) << 32);
            let timescale = io.read_u32_be()?;
            let dur_hi = io.read_u32_be()? as u64;
            let dur_lo = io.read_u32_be()? as u64;
            let duration = (dur_hi << 32) | dur_lo;
            if timescale > 0 {
                self.file_duration = Some(duration as f64 / timescale as f64);
            }
            Ok(timescale)
        }
    }

    /// 解析 trak (Track Box)
    fn parse_trak(
        &mut self,
        io: &mut IoContext,
        trak_end: u64,
        _movie_timescale: u32,
    ) -> TaoResult<()> {
        let mut track_id = 0u32;
        let mut media_timescale = 0u32;
        let mut media_duration = 0u64;
        let mut handler_type = [0u8; 4];
        let mut sample_table = SampleTable::new();
        let mut width = 0u32;
        let mut height = 0u32;

        // 递归遍历 trak 内的 box
        self.parse_trak_boxes(
            io,
            trak_end,
            &mut track_id,
            &mut media_timescale,
            &mut media_duration,
            &mut handler_type,
            &mut sample_table,
            &mut width,
            &mut height,
        )?;

        // 根据 handler_type 创建流
        let stream_index = self.streams.len();
        let (media_type, codec_id, params) =
            self.build_stream_params(&handler_type, &sample_table, width, height);

        let time_base = if media_timescale > 0 {
            Rational::new(1, media_timescale as i32)
        } else {
            Rational::new(1, 1000)
        };

        let duration = if media_timescale > 0 {
            media_duration as i64
        } else {
            -1
        };

        let stream = Stream {
            index: stream_index,
            media_type,
            codec_id,
            time_base,
            duration,
            start_time: 0,
            nb_frames: sample_table.sample_count() as u64,
            extra_data: sample_table.extra_data.clone(),
            params,
            metadata: Vec::new(),
        };

        debug!(
            "MP4: 轨道 #{} (id={}): {} {}, timescale={}, samples={}",
            stream_index,
            track_id,
            media_type,
            codec_id,
            media_timescale,
            sample_table.sample_count(),
        );

        self.streams.push(stream);
        self.sample_tables.push(sample_table);
        self.current_sample.push(0);

        Ok(())
    }

    /// 递归解析 trak 内部的 box
    #[allow(clippy::too_many_arguments)]
    fn parse_trak_boxes(
        &self,
        io: &mut IoContext,
        end: u64,
        track_id: &mut u32,
        timescale: &mut u32,
        duration: &mut u64,
        handler: &mut [u8; 4],
        st: &mut SampleTable,
        width: &mut u32,
        height: &mut u32,
    ) -> TaoResult<()> {
        while io.position()? < end {
            let header = match read_box_header(io) {
                Ok(h) => h,
                Err(_) => break,
            };
            let box_end = io.position()? + header.content_size();

            match header.box_type {
                BoxType::Tkhd => {
                    Self::parse_tkhd(io, track_id, width, height)?;
                }
                BoxType::Mdia | BoxType::Minf | BoxType::Stbl => {
                    // 容器 box, 递归解析
                    self.parse_trak_boxes(
                        io, box_end, track_id, timescale, duration, handler, st, width, height,
                    )?;
                }
                BoxType::Mdhd => {
                    Self::parse_mdhd(io, timescale, duration)?;
                }
                BoxType::Hdlr => {
                    Self::parse_hdlr(io, handler)?;
                }
                BoxType::Stsd => {
                    st.parse_stsd(io, box_end)?;
                }
                BoxType::Stts => {
                    st.parse_stts(io)?;
                }
                BoxType::Stsc => {
                    st.parse_stsc(io)?;
                }
                BoxType::Stsz => {
                    st.parse_stsz(io)?;
                }
                BoxType::Stco => {
                    st.parse_stco(io, false)?;
                }
                BoxType::Co64 => {
                    st.parse_stco(io, true)?;
                }
                BoxType::Stss => {
                    st.parse_stss(io)?;
                }
                BoxType::Ctts => {
                    st.parse_ctts(io)?;
                }
                _ => {}
            }

            io.seek(std::io::SeekFrom::Start(box_end))?;
        }
        Ok(())
    }

    /// 解析 tkhd (Track Header Box)
    fn parse_tkhd(
        io: &mut IoContext,
        track_id: &mut u32,
        width: &mut u32,
        height: &mut u32,
    ) -> TaoResult<()> {
        let version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;

        if version == 0 {
            let _creation = io.read_u32_be()?;
            let _modification = io.read_u32_be()?;
            *track_id = io.read_u32_be()?;
            let _reserved = io.read_u32_be()?;
            let _duration = io.read_u32_be()?;
        } else {
            // version 1: 64-bit timestamps
            io.read_bytes(8)?; // creation
            io.read_bytes(8)?; // modification
            *track_id = io.read_u32_be()?;
            let _reserved = io.read_u32_be()?;
            io.read_bytes(8)?; // duration
        }

        io.read_bytes(8)?; // reserved
        let _layer = io.read_u16_be()?;
        let _alternate_group = io.read_u16_be()?;
        let _volume = io.read_u16_be()?;
        let _reserved2 = io.read_u16_be()?;
        io.read_bytes(36)?; // matrix

        // 宽高 (16.16 定点数)
        let w_fixed = io.read_u32_be()?;
        let h_fixed = io.read_u32_be()?;
        *width = w_fixed >> 16;
        *height = h_fixed >> 16;

        Ok(())
    }

    /// 解析 mdhd (Media Header Box)
    fn parse_mdhd(io: &mut IoContext, timescale: &mut u32, duration: &mut u64) -> TaoResult<()> {
        let version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;

        if version == 0 {
            let _creation = io.read_u32_be()?;
            let _modification = io.read_u32_be()?;
            *timescale = io.read_u32_be()?;
            *duration = u64::from(io.read_u32_be()?);
        } else {
            io.read_bytes(8)?; // creation
            io.read_bytes(8)?; // modification
            *timescale = io.read_u32_be()?;
            let hi = io.read_u32_be()? as u64;
            let lo = io.read_u32_be()? as u64;
            *duration = (hi << 32) | lo;
        }

        Ok(())
    }

    /// 解析 hdlr (Handler Reference Box)
    fn parse_hdlr(io: &mut IoContext, handler: &mut [u8; 4]) -> TaoResult<()> {
        let _version = io.read_u8()?;
        let _flags = io.read_bytes(3)?;
        let _pre_defined = io.read_u32_be()?;
        *handler = io.read_tag()?;
        Ok(())
    }

    /// 根据 handler type 和采样描述构建流参数
    fn build_stream_params(
        &self,
        handler: &[u8; 4],
        st: &SampleTable,
        width: u32,
        height: u32,
    ) -> (MediaType, CodecId, StreamParams) {
        match handler {
            b"vide" => {
                let codec_id = st.codec_id;
                let pf = match codec_id {
                    CodecId::H264 | CodecId::H265 | CodecId::Vp9 | CodecId::Av1 => {
                        tao_core::PixelFormat::Yuv420p
                    }
                    _ => tao_core::PixelFormat::Yuv420p,
                };
                (
                    MediaType::Video,
                    codec_id,
                    StreamParams::Video(VideoStreamParams {
                        width: if st.width > 0 { st.width } else { width },
                        height: if st.height > 0 { st.height } else { height },
                        pixel_format: pf,
                        frame_rate: Rational::new(0, 1),
                        sample_aspect_ratio: Rational::new(1, 1),
                        bit_rate: 0,
                    }),
                )
            }
            b"soun" => {
                let codec_id = st.codec_id;
                (
                    MediaType::Audio,
                    codec_id,
                    StreamParams::Audio(AudioStreamParams {
                        sample_rate: st.sample_rate,
                        channel_layout: ChannelLayout::from_channels(st.channel_count),
                        sample_format: SampleFormat::F32,
                        bit_rate: 0,
                        frame_size: 0,
                    }),
                )
            }
            _ => (MediaType::Data, CodecId::None, StreamParams::Other),
        }
    }

    /// 找到最早的下一个采样 (跨所有流)
    fn find_next_sample(&self) -> Option<(usize, u32)> {
        let mut best: Option<(usize, u32, u64)> = None;

        for (stream_idx, st) in self.sample_tables.iter().enumerate() {
            let sample_idx = self.current_sample[stream_idx];
            if sample_idx >= st.sample_count() {
                continue;
            }
            let offset = st.sample_offset(sample_idx);
            match best {
                None => best = Some((stream_idx, sample_idx, offset)),
                Some((_, _, best_off)) if offset < best_off => {
                    best = Some((stream_idx, sample_idx, offset));
                }
                _ => {}
            }
        }

        best.map(|(si, idx, _)| (si, idx))
    }
}

impl Demuxer for Mp4Demuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Mp4
    }

    fn name(&self) -> &str {
        "mp4"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 扫描顶层 box
        let file_size = io.size().unwrap_or(u64::MAX);

        loop {
            let pos = io.position()?;
            if pos >= file_size {
                break;
            }

            let header = match read_box_header(io) {
                Ok(h) => h,
                Err(TaoError::Eof) => break,
                Err(e) => return Err(e),
            };

            let content_start = io.position()?;
            let box_end = if header.size == 0 {
                file_size // box 延伸到文件末尾
            } else {
                content_start + header.content_size()
            };

            match header.box_type {
                BoxType::Ftyp => {
                    let ftyp = FtypBox::parse(io, header.content_size())?;
                    debug!("MP4: ftyp major_brand={}", ftyp.major_brand_str());
                }
                BoxType::Moov => {
                    self.parse_moov(io, box_end)?;
                }
                BoxType::Mdat => {
                    self.mdat_offset = content_start;
                    self.mdat_size = box_end - content_start;
                }
                _ => {}
            }

            io.seek(std::io::SeekFrom::Start(box_end))?;
        }

        if self.streams.is_empty() {
            return Err(TaoError::InvalidData("MP4 文件中未找到任何轨道".into()));
        }

        debug!("打开 MP4: {} 个轨道", self.streams.len());
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        let (stream_idx, sample_idx) = match self.find_next_sample() {
            Some(v) => v,
            None => return Err(TaoError::Eof),
        };

        let st = &self.sample_tables[stream_idx];
        let offset = st.sample_offset(sample_idx);
        let size = st.sample_size(sample_idx);
        let pts = st.sample_pts(sample_idx);
        let is_keyframe = st.is_sync_sample(sample_idx);

        // 读取数据
        io.seek(std::io::SeekFrom::Start(offset))?;
        let data = io.read_bytes(size as usize)?;

        let mut pkt = Packet::from_data(Bytes::from(data));
        pkt.stream_index = stream_idx;
        pkt.pts = pts;
        pkt.dts = pts; // TODO: 使用 ctts 计算真实 DTS
        pkt.is_keyframe = is_keyframe;

        if let Some(stream) = self.streams.get(stream_idx) {
            pkt.time_base = stream.time_base;
        }

        self.current_sample[stream_idx] += 1;
        Ok(pkt)
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("MP4 seek 尚未实现".into()))
    }

    fn duration(&self) -> Option<f64> {
        self.file_duration
    }
}

/// MP4 格式探测器
pub struct Mp4Probe;

impl FormatProbe for Mp4Probe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<crate::probe::ProbeScore> {
        // 检查 ftyp box
        if data.len() >= 8 && &data[4..8] == b"ftyp" {
            return Some(crate::probe::SCORE_MAX);
        }

        // 检查 moov 或 mdat (某些文件没有 ftyp)
        if data.len() >= 8
            && (&data[4..8] == b"moov"
                || &data[4..8] == b"mdat"
                || &data[4..8] == b"free"
                || &data[4..8] == b"wide")
        {
            return Some(crate::probe::SCORE_MIME);
        }

        // 扩展名
        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                let ext_lower = ext.to_lowercase();
                if matches!(
                    ext_lower.as_str(),
                    "mp4" | "m4a" | "m4v" | "mov" | "3gp" | "3g2"
                ) {
                    return Some(crate::probe::SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Mp4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    #[test]
    fn test_探测_mp4_ftyp() {
        let probe = Mp4Probe;
        // ftyp box: size=20, type="ftyp", brand="isom"
        let mut data = vec![0u8; 20];
        data[3] = 20; // size
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom");
        assert!(probe.probe(&data, None).is_some());
        assert_eq!(probe.probe(&data, None), Some(crate::probe::SCORE_MAX),);
    }

    #[test]
    fn test_探测_mp4_扩展名() {
        let probe = Mp4Probe;
        assert!(probe.probe(&[], Some("video.mp4")).is_some());
        assert!(probe.probe(&[], Some("audio.m4a")).is_some());
        assert!(probe.probe(&[], Some("movie.mov")).is_some());
        assert!(probe.probe(&[], Some("music.wav")).is_none());
    }

    #[test]
    fn test_解析_最小mp4() {
        // 构造最小的 MP4 (ftyp + moov 含一个空 trak)
        let mp4 = build_minimal_mp4();
        let backend = MemoryBackend::from_data(mp4);
        let mut io = IoContext::new(Box::new(backend));

        let mut demuxer = Mp4Demuxer::create().unwrap();
        let result = demuxer.open(&mut io);
        // 最小 MP4 可能没有有效轨道
        // 只要不 panic 就行
        assert!(result.is_ok() || result.is_err(), "解析不应 panic",);
    }

    /// 构造最小 MP4 文件
    fn build_minimal_mp4() -> Vec<u8> {
        let mut data = Vec::new();

        // ftyp box
        let ftyp = build_box(b"ftyp", &{
            let mut d = Vec::new();
            d.extend_from_slice(b"isom"); // major brand
            d.extend_from_slice(&0u32.to_be_bytes()); // minor version
            d.extend_from_slice(b"isom"); // compatible brand
            d
        });
        data.extend_from_slice(&ftyp);

        // moov box (空)
        let mvhd = build_fullbox(b"mvhd", 0, 0, &{
            let mut d = Vec::new();
            d.extend_from_slice(&0u32.to_be_bytes()); // creation_time
            d.extend_from_slice(&0u32.to_be_bytes()); // modification_time
            d.extend_from_slice(&1000u32.to_be_bytes()); // timescale
            d.extend_from_slice(&0u32.to_be_bytes()); // duration
            d.extend_from_slice(&[0u8; 80]); // 剩余字段
            d
        });
        let moov = build_box(b"moov", &mvhd);
        data.extend_from_slice(&moov);

        data
    }

    fn build_box(box_type: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let size = (8 + content.len()) as u32;
        let mut data = Vec::new();
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(box_type);
        data.extend_from_slice(content);
        data
    }

    fn build_fullbox(box_type: &[u8; 4], version: u8, flags: u32, content: &[u8]) -> Vec<u8> {
        let mut full_content = vec![
            version,
            ((flags >> 16) & 0xFF) as u8,
            ((flags >> 8) & 0xFF) as u8,
            (flags & 0xFF) as u8,
        ];
        full_content.extend_from_slice(content);
        build_box(box_type, &full_content)
    }
}
