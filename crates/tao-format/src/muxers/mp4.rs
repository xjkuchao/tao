//! MP4 (MPEG-4 Part 14) 封装器.
//!
//! 将音视频数据包写入 MP4 容器. 采用 "mdat 在前, moov 在后" 的布局:
//! 1. `write_header()` - 写入 ftyp box
//! 2. `write_packet()` - 追加数据到 mdat, 记录 sample 元数据
//! 3. `write_trailer()` - 回填 mdat 大小, 构建并写入 moov box
//!
//! # Box 结构
//! ```text
//! ftyp
//! mdat (预留大小, trailer 回填)
//! moov
//! ├── mvhd
//! └── trak (每个流一个)
//!     ├── tkhd
//!     └── mdia
//!         ├── mdhd
//!         ├── hdlr
//!         └── minf
//!             ├── vmhd / smhd
//!             ├── dinf → dref
//!             └── stbl
//!                 ├── stsd (avc1/mp4a 等)
//!                 ├── stts
//!                 ├── stsc
//!                 ├── stsz
//!                 ├── stco / co64
//!                 └── stss (仅视频)
//! ```

use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, TaoError, TaoResult};

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::stream::{Stream, StreamParams};

/// 每个 sample 的元数据
#[derive(Debug, Clone)]
struct SampleEntry {
    /// mdat 中的偏移
    offset: u64,
    /// 数据大小
    size: u32,
    /// 解码时间增量 (duration)
    duration: u32,
    /// CTS = PTS - DTS (仅视频)
    cts_offset: i32,
    /// 是否为关键帧
    is_keyframe: bool,
}

/// 每个轨道的收集信息
struct TrackCollector {
    /// 流索引
    stream_index: usize,
    /// 流信息的克隆
    stream: Stream,
    /// 时间基
    timescale: u32,
    /// 所有 sample
    samples: Vec<SampleEntry>,
    /// 上一个 DTS (用于计算 duration)
    last_dts: i64,
}

/// MP4 封装器
pub struct Mp4Muxer {
    /// 轨道收集器
    tracks: Vec<TrackCollector>,
    /// mdat box 大小字段的偏移
    mdat_offset: u64,
    /// mdat 数据起始偏移
    mdat_data_start: u64,
    /// 已写入的 mdat 数据量
    mdat_written: u64,
}

impl Mp4Muxer {
    /// 创建 MP4 封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Muxer>> {
        Ok(Box::new(Self {
            tracks: Vec::new(),
            mdat_offset: 0,
            mdat_data_start: 0,
            mdat_written: 0,
        }))
    }

    /// 获取 timescale (音频用采样率, 视频用 time_base 的分母)
    fn get_timescale(stream: &Stream) -> u32 {
        match &stream.params {
            StreamParams::Audio(a) => a.sample_rate,
            _ => {
                // 视频: 使用 time_base 分母, 默认 90000
                if stream.time_base.den > 0 {
                    stream.time_base.den as u32
                } else {
                    90000
                }
            }
        }
    }
}

impl Muxer for Mp4Muxer {
    fn format_id(&self) -> FormatId {
        FormatId::Mp4
    }

    fn name(&self) -> &str {
        "mp4"
    }

    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()> {
        if streams.is_empty() {
            return Err(TaoError::InvalidArgument("MP4: 至少需要一个流".into()));
        }

        // 初始化轨道收集器
        for stream in streams {
            let timescale = Self::get_timescale(stream);
            self.tracks.push(TrackCollector {
                stream_index: stream.index,
                stream: stream.clone(),
                timescale,
                samples: Vec::new(),
                last_dts: -1,
            });
        }

        // 写 ftyp box
        write_ftyp(io)?;

        // 写 mdat box (先写 8 字节头, trailer 回填大小)
        self.mdat_offset = io.position()?;
        io.write_u32_be(0)?; // 占位, trailer 回填
        io.write_tag(b"mdat")?;
        self.mdat_data_start = io.position()?;

        debug!("MP4: 写入 ftyp + mdat 头, {} 个轨道", self.tracks.len());
        Ok(())
    }

    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()> {
        // 查找轨道
        let track = self
            .tracks
            .iter_mut()
            .find(|t| t.stream_index == packet.stream_index)
            .ok_or_else(|| {
                TaoError::InvalidArgument(format!("MP4: 未知流索引 {}", packet.stream_index))
            })?;

        let offset = io.position()?;
        io.write_all(&packet.data)?;

        // 计算 duration
        let dts = packet.dts;
        let duration = if track.last_dts >= 0 {
            (dts - track.last_dts).max(0) as u32
        } else {
            // 第一个 sample: 使用 packet.duration 或默认
            if packet.duration > 0 {
                packet.duration as u32
            } else {
                0
            }
        };

        // CTS offset (PTS - DTS)
        let cts_offset = (packet.pts - dts) as i32;

        track.samples.push(SampleEntry {
            offset,
            size: packet.data.len() as u32,
            duration,
            cts_offset,
            is_keyframe: packet.is_keyframe,
        });

        track.last_dts = dts;
        self.mdat_written += packet.data.len() as u64;

        Ok(())
    }

    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 回填 mdat 大小 (8 字节头 + 数据)
        let mdat_total = 8 + self.mdat_written;
        if io.is_seekable() {
            let current = io.position()?;
            io.seek(std::io::SeekFrom::Start(self.mdat_offset))?;
            io.write_u32_be(mdat_total as u32)?;
            io.seek(std::io::SeekFrom::Start(current))?;
        }

        // 修正第一个 sample 的 duration (之前为 0)
        for track in &mut self.tracks {
            fix_first_sample_duration(track);
        }

        // 构建 moov box
        let tracks: Vec<_> = self.tracks.drain(..).collect();
        write_moov(io, &tracks)?;

        debug!("MP4: 写入 moov, mdat 大小={mdat_total}");
        Ok(())
    }
}

// ============================================================
// 修正工具
// ============================================================

/// 修正第一个 sample 的 duration
fn fix_first_sample_duration(track: &mut TrackCollector) {
    if track.samples.len() >= 2 && track.samples[0].duration == 0 {
        track.samples[0].duration = track.samples[1].duration;
    }
}

// ============================================================
// Box 写入函数
// ============================================================

/// 写 ftyp box
fn write_ftyp(io: &mut IoContext) -> TaoResult<()> {
    // ftyp: major_brand=isom, minor_version=0x200, compatible=[isom, iso2, mp41]
    let brands: &[&[u8; 4]] = &[b"isom", b"iso2", b"mp41"];
    let size: u32 = 8 + 4 + 4 + (brands.len() as u32) * 4;

    io.write_u32_be(size)?;
    io.write_tag(b"ftyp")?;
    io.write_tag(b"isom")?; // major_brand
    io.write_u32_be(0x200)?; // minor_version
    for brand in brands {
        io.write_tag(brand)?;
    }

    Ok(())
}

/// 写完整的 moov box
fn write_moov(io: &mut IoContext, tracks: &[TrackCollector]) -> TaoResult<()> {
    let moov_data = build_moov(tracks)?;

    io.write_u32_be((8 + moov_data.len()) as u32)?;
    io.write_tag(b"moov")?;
    io.write_all(&moov_data)?;

    Ok(())
}

/// 构建 moov box 的内容 (不含 box 头)
fn build_moov(tracks: &[TrackCollector]) -> TaoResult<Vec<u8>> {
    let mut buf = Vec::new();

    // mvhd
    let max_duration = tracks
        .iter()
        .map(track_duration_in_timescale)
        .max()
        .unwrap_or(0);
    buf.extend_from_slice(&build_mvhd(max_duration, tracks.len() as u32));

    // trak for each track
    for (i, track) in tracks.iter().enumerate() {
        buf.extend_from_slice(&build_trak(track, i as u32 + 1)?);
    }

    Ok(buf)
}

/// 计算轨道总时长 (以 timescale 为单位)
fn track_duration_in_timescale(track: &TrackCollector) -> u64 {
    track.samples.iter().map(|s| u64::from(s.duration)).sum()
}

/// mvhd box (版本 0)
fn build_mvhd(duration_ticks: u64, next_track_id: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    let timescale: u32 = 1000; // moov 级别用 1000 (毫秒)

    // 重新计算以 moov timescale 为单位的 duration
    // 简化: 直接用 ticks 作为 duration (因为各轨道 timescale 可能不同)
    let duration = (duration_ticks).min(u32::MAX as u64) as u32;

    // version(1)+flags(3) + times(8) + timescale(4) + duration(4)
    // + rate(4) + volume(2) + reserved(10) + matrix(36) + pre_defined(24) + next_track_id(4)
    // = 100 字节内容
    let content_size: u32 = 100;
    let box_size = 8 + content_size;

    write_box_header(&mut buf, box_size, b"mvhd");

    // version(1) + flags(3)
    buf.extend_from_slice(&[0, 0, 0, 0]);
    // creation_time(4) + modification_time(4)
    buf.extend_from_slice(&[0; 8]);
    // timescale(4)
    buf.extend_from_slice(&timescale.to_be_bytes());
    // duration(4)
    buf.extend_from_slice(&duration.to_be_bytes());
    // rate (1.0 = 0x00010000)
    buf.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    // volume (1.0 = 0x0100)
    buf.extend_from_slice(&0x0100u16.to_be_bytes());
    // reserved (10 bytes)
    buf.extend_from_slice(&[0; 10]);
    // matrix (36 bytes) - 单位矩阵
    buf.extend_from_slice(&UNITY_MATRIX);
    // pre_defined (24 bytes)
    buf.extend_from_slice(&[0; 24]);
    // next_track_ID
    buf.extend_from_slice(&(next_track_id + 1).to_be_bytes());

    buf
}

/// 构建 trak box
fn build_trak(track: &TrackCollector, track_id: u32) -> TaoResult<Vec<u8>> {
    let mut inner = Vec::new();

    inner.extend_from_slice(&build_tkhd(track, track_id));
    inner.extend_from_slice(&build_mdia(track)?);

    let mut buf = Vec::new();
    write_box_header(&mut buf, 8 + inner.len() as u32, b"trak");
    buf.extend_from_slice(&inner);
    Ok(buf)
}

/// tkhd box
fn build_tkhd(track: &TrackCollector, track_id: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    let duration = track_duration_in_timescale(track) as u32;

    let (width, height) = match &track.stream.params {
        StreamParams::Video(v) => (v.width, v.height),
        _ => (0, 0),
    };

    // version(1)+flags(3) + times(8) + track_id(4) + reserved(4)
    // + duration(4) + reserved(8) + layer(2)+alt_group(2)
    // + volume(2) + reserved(2) + matrix(36) + width(4) + height(4)
    // = 84 字节内容
    let content_size: u32 = 84;
    write_box_header(&mut buf, 8 + content_size, b"tkhd");

    // version(1) + flags(3) - flag 0x03 = track_enabled | track_in_movie
    buf.extend_from_slice(&[0, 0, 0, 0x03]);
    // creation_time(4) + modification_time(4)
    buf.extend_from_slice(&[0; 8]);
    // track_ID(4)
    buf.extend_from_slice(&track_id.to_be_bytes());
    // reserved(4)
    buf.extend_from_slice(&[0; 4]);
    // duration(4)
    buf.extend_from_slice(&duration.to_be_bytes());
    // reserved(8)
    buf.extend_from_slice(&[0; 8]);
    // layer(2) + alternate_group(2)
    buf.extend_from_slice(&[0; 4]);
    // volume(2) - 音频 0x0100, 视频 0x0000
    if track.stream.media_type == MediaType::Audio {
        buf.extend_from_slice(&0x0100u16.to_be_bytes());
    } else {
        buf.extend_from_slice(&[0; 2]);
    }
    // reserved(2)
    buf.extend_from_slice(&[0; 2]);
    // matrix (36 bytes)
    buf.extend_from_slice(&UNITY_MATRIX);
    // width (16.16 fixed point)
    buf.extend_from_slice(&(width << 16).to_be_bytes());
    // height (16.16 fixed point)
    buf.extend_from_slice(&(height << 16).to_be_bytes());

    buf
}

/// mdia box
fn build_mdia(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let mut inner = Vec::new();

    inner.extend_from_slice(&build_mdhd(track));
    inner.extend_from_slice(&build_hdlr(track));
    inner.extend_from_slice(&build_minf(track)?);

    let mut buf = Vec::new();
    write_box_header(&mut buf, 8 + inner.len() as u32, b"mdia");
    buf.extend_from_slice(&inner);
    Ok(buf)
}

/// mdhd box
fn build_mdhd(track: &TrackCollector) -> Vec<u8> {
    let mut buf = Vec::new();
    let duration = track_duration_in_timescale(track) as u32;

    // version(1)+flags(3) + times(8) + timescale(4)
    // + duration(4) + language(2) + pre_defined(2) = 24 字节
    let content_size: u32 = 24;
    write_box_header(&mut buf, 8 + content_size, b"mdhd");

    // version(1) + flags(3)
    buf.extend_from_slice(&[0, 0, 0, 0]);
    // creation_time(4) + modification_time(4)
    buf.extend_from_slice(&[0; 8]);
    // timescale(4)
    buf.extend_from_slice(&track.timescale.to_be_bytes());
    // duration(4)
    buf.extend_from_slice(&duration.to_be_bytes());
    // language(2) - 'und' = 0x55C4
    buf.extend_from_slice(&0x55C4u16.to_be_bytes());
    // pre_defined(2)
    buf.extend_from_slice(&[0; 2]);

    buf
}

/// hdlr box
fn build_hdlr(track: &TrackCollector) -> Vec<u8> {
    let (handler_type, name) = match track.stream.media_type {
        MediaType::Video => (b"vide", b"VideoHandler\0" as &[u8]),
        MediaType::Audio => (b"soun", b"SoundHandler\0" as &[u8]),
        _ => (b"meta", b"MetaHandler\0" as &[u8]),
    };

    let mut buf = Vec::new();
    let content_size = 24 + name.len() as u32;
    write_box_header(&mut buf, 8 + content_size, b"hdlr");

    // version(1) + flags(3)
    buf.extend_from_slice(&[0; 4]);
    // pre_defined(4)
    buf.extend_from_slice(&[0; 4]);
    // handler_type(4)
    buf.extend_from_slice(handler_type);
    // reserved(12)
    buf.extend_from_slice(&[0; 12]);
    // name (null-terminated string)
    buf.extend_from_slice(name);

    buf
}

/// minf box
fn build_minf(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let mut inner = Vec::new();

    // vmhd 或 smhd
    match track.stream.media_type {
        MediaType::Video => inner.extend_from_slice(&build_vmhd()),
        MediaType::Audio => inner.extend_from_slice(&build_smhd()),
        _ => {}
    }

    inner.extend_from_slice(&build_dinf());
    inner.extend_from_slice(&build_stbl(track)?);

    let mut buf = Vec::new();
    write_box_header(&mut buf, 8 + inner.len() as u32, b"minf");
    buf.extend_from_slice(&inner);
    Ok(buf)
}

/// vmhd box (视频媒体头)
fn build_vmhd() -> Vec<u8> {
    let mut buf = Vec::new();
    write_box_header(&mut buf, 20, b"vmhd");
    // version(1) + flags(3) - flags=1
    buf.extend_from_slice(&[0, 0, 0, 1]);
    // graphicsmode(2) + opcolor(6)
    buf.extend_from_slice(&[0; 8]);
    buf
}

/// smhd box (音频媒体头)
fn build_smhd() -> Vec<u8> {
    let mut buf = Vec::new();
    write_box_header(&mut buf, 16, b"smhd");
    // version(1) + flags(3)
    buf.extend_from_slice(&[0; 4]);
    // balance(2) + reserved(2)
    buf.extend_from_slice(&[0; 4]);
    buf
}

/// dinf + dref box
fn build_dinf() -> Vec<u8> {
    // dref with one 'url ' entry (self-contained)
    let mut dref = Vec::new();
    write_box_header(&mut dref, 28, b"dref");
    // version(1) + flags(3)
    dref.extend_from_slice(&[0; 4]);
    // entry_count = 1
    dref.extend_from_slice(&1u32.to_be_bytes());
    // url entry (self-contained: flags=1)
    write_box_header(&mut dref, 12, b"url ");
    dref.extend_from_slice(&[0, 0, 0, 1]); // flags=1 self-contained

    let mut buf = Vec::new();
    write_box_header(&mut buf, 8 + dref.len() as u32, b"dinf");
    buf.extend_from_slice(&dref);
    buf
}

/// stbl box (Sample Table)
fn build_stbl(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let mut inner = Vec::new();

    inner.extend_from_slice(&build_stsd(track)?);
    inner.extend_from_slice(&build_stts(track));
    inner.extend_from_slice(&build_stsc(track));
    inner.extend_from_slice(&build_stsz(track));
    inner.extend_from_slice(&build_stco(track));

    // stss (sync sample table) - 仅视频
    if track.stream.media_type == MediaType::Video {
        inner.extend_from_slice(&build_stss(track));
    }

    // ctts (Composition Time to Sample) - 仅当有非零 CTS
    if has_cts_offsets(track) {
        inner.extend_from_slice(&build_ctts(track));
    }

    let mut buf = Vec::new();
    write_box_header(&mut buf, 8 + inner.len() as u32, b"stbl");
    buf.extend_from_slice(&inner);
    Ok(buf)
}

/// stsd box (Sample Description)
fn build_stsd(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let entry = build_sample_entry(track)?;

    let mut buf = Vec::new();
    let content_size = 8 + entry.len() as u32;
    write_box_header(&mut buf, 8 + content_size, b"stsd");
    // version(1) + flags(3)
    buf.extend_from_slice(&[0; 4]);
    // entry_count = 1
    buf.extend_from_slice(&1u32.to_be_bytes());
    buf.extend_from_slice(&entry);

    Ok(buf)
}

/// 构建 sample entry (avc1 / mp4a 等)
fn build_sample_entry(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    match track.stream.media_type {
        MediaType::Video => build_video_sample_entry(track),
        MediaType::Audio => build_audio_sample_entry(track),
        _ => Err(TaoError::Unsupported("MP4: 不支持的媒体类型".into())),
    }
}

/// 视频 sample entry (avc1)
fn build_video_sample_entry(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let (width, height) = match &track.stream.params {
        StreamParams::Video(v) => (v.width as u16, v.height as u16),
        _ => return Err(TaoError::InvalidData("MP4: 视频流缺少参数".into())),
    };

    let fourcc = codec_to_mp4_fourcc(track.stream.codec_id)?;

    let mut buf = Vec::new();

    // 视频 sample entry 基本结构
    let mut entry = Vec::new();
    // reserved(6) + data_reference_index(2)
    entry.extend_from_slice(&[0; 6]);
    entry.extend_from_slice(&1u16.to_be_bytes());
    // pre_defined(2) + reserved(2) + pre_defined(12)
    entry.extend_from_slice(&[0; 16]);
    // width(2) + height(2)
    entry.extend_from_slice(&width.to_be_bytes());
    entry.extend_from_slice(&height.to_be_bytes());
    // horizresolution (72 dpi = 0x00480000)
    entry.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    // vertresolution
    entry.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    // reserved(4)
    entry.extend_from_slice(&[0; 4]);
    // frame_count(2) = 1
    entry.extend_from_slice(&1u16.to_be_bytes());
    // compressorname (32 bytes, padded)
    entry.extend_from_slice(&[0; 32]);
    // depth(2) = 0x0018
    entry.extend_from_slice(&0x0018u16.to_be_bytes());
    // pre_defined(2) = -1
    entry.extend_from_slice(&0xFFFFu16.to_be_bytes());

    // 嵌套 box (如 avcC)
    if track.stream.codec_id == CodecId::H264 && !track.stream.extra_data.is_empty() {
        let mut avcc = Vec::new();
        let avcc_size = 8 + track.stream.extra_data.len() as u32;
        write_box_header(&mut avcc, avcc_size, b"avcC");
        avcc.extend_from_slice(&track.stream.extra_data);
        entry.extend_from_slice(&avcc);
    }

    let box_size = 8 + entry.len() as u32;
    write_box_header(&mut buf, box_size, &fourcc);
    buf.extend_from_slice(&entry);

    Ok(buf)
}

/// 音频 sample entry (mp4a)
fn build_audio_sample_entry(track: &TrackCollector) -> TaoResult<Vec<u8>> {
    let (sample_rate, channels) = match &track.stream.params {
        StreamParams::Audio(a) => (a.sample_rate, a.channel_layout.channels),
        _ => return Err(TaoError::InvalidData("MP4: 音频流缺少参数".into())),
    };

    let fourcc = codec_to_mp4_fourcc(track.stream.codec_id)?;

    let mut entry = Vec::new();
    // reserved(6) + data_reference_index(2)
    entry.extend_from_slice(&[0; 6]);
    entry.extend_from_slice(&1u16.to_be_bytes());
    // reserved(8)
    entry.extend_from_slice(&[0; 8]);
    // channelcount(2)
    entry.extend_from_slice(&(channels as u16).to_be_bytes());
    // samplesize(2) = 16
    entry.extend_from_slice(&16u16.to_be_bytes());
    // pre_defined(2) + reserved(2)
    entry.extend_from_slice(&[0; 4]);
    // samplerate (16.16 fixed point)
    entry.extend_from_slice(&(sample_rate << 16).to_be_bytes());

    // esds box (AAC)
    if track.stream.codec_id == CodecId::Aac {
        entry.extend_from_slice(&build_esds(track, sample_rate, channels)?);
    }

    let mut buf = Vec::new();
    let box_size = 8 + entry.len() as u32;
    write_box_header(&mut buf, box_size, &fourcc);
    buf.extend_from_slice(&entry);

    Ok(buf)
}

/// 构建 esds box (用于 AAC)
fn build_esds(track: &TrackCollector, _sample_rate: u32, _channels: u32) -> TaoResult<Vec<u8>> {
    let extra = &track.stream.extra_data;

    // ES_Descriptor 内容
    let mut es_desc = Vec::new();

    // ES_ID(2)
    es_desc.extend_from_slice(&[0x00, 0x01]);
    // streamDependenceFlag(1) + URL_Flag(1) + OCRstreamFlag(1) + streamPriority(5)
    es_desc.push(0x00);

    // DecoderConfigDescriptor (tag=0x04)
    let mut dec_config = Vec::new();
    dec_config.push(0x40); // objectTypeIndication = AAC-LC
    // streamType(6) + upStream(1) + reserved(1) = 0x15 (audio stream)
    dec_config.push(0x15);
    // bufferSizeDB(3)
    dec_config.extend_from_slice(&[0x00, 0x00, 0x00]);
    // maxBitrate(4)
    dec_config.extend_from_slice(&[0x00, 0x01, 0xF4, 0x00]); // 128kbps
    // avgBitrate(4)
    dec_config.extend_from_slice(&[0x00, 0x01, 0xF4, 0x00]);

    // DecoderSpecificInfo (tag=0x05)
    if !extra.is_empty() {
        dec_config.push(0x05); // tag
        write_desc_length(&mut dec_config, extra.len());
        dec_config.extend_from_slice(extra);
    }

    // 将 DecoderConfigDescriptor 加入 ES_Descriptor
    es_desc.push(0x04); // tag
    write_desc_length(&mut es_desc, dec_config.len());
    es_desc.extend_from_slice(&dec_config);

    // SLConfigDescriptor (tag=0x06, predefined=2)
    es_desc.push(0x06);
    write_desc_length(&mut es_desc, 1);
    es_desc.push(0x02);

    // 完整的 esds box
    let mut esds = Vec::new();
    let esds_content_len = 4 + 1 + desc_length_size(es_desc.len()) + es_desc.len();
    write_box_header(&mut esds, 8 + esds_content_len as u32, b"esds");
    // version(1) + flags(3)
    esds.extend_from_slice(&[0; 4]);
    // ES_Descriptor (tag=0x03)
    esds.push(0x03);
    write_desc_length(&mut esds, es_desc.len());
    esds.extend_from_slice(&es_desc);

    Ok(esds)
}

/// stts box (解码时间到样本映射)
fn build_stts(track: &TrackCollector) -> Vec<u8> {
    // RLE 压缩: 连续相同 duration 合并
    let entries = rle_durations(track);

    let mut buf = Vec::new();
    let content_size = 8 + entries.len() as u32 * 8;
    write_box_header(&mut buf, 8 + content_size, b"stts");
    buf.extend_from_slice(&[0; 4]); // version + flags
    buf.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (count, duration) in &entries {
        buf.extend_from_slice(&count.to_be_bytes());
        buf.extend_from_slice(&duration.to_be_bytes());
    }
    buf
}

/// stsc box (样本到块映射)
fn build_stsc(_track: &TrackCollector) -> Vec<u8> {
    // 简单实现: 每个 sample 一个 chunk
    let mut buf = Vec::new();
    let content_size = 8 + 12; // 1 个条目
    write_box_header(&mut buf, 8 + content_size, b"stsc");
    buf.extend_from_slice(&[0; 4]); // version + flags
    buf.extend_from_slice(&1u32.to_be_bytes()); // entry_count=1
    buf.extend_from_slice(&1u32.to_be_bytes()); // first_chunk=1
    buf.extend_from_slice(&1u32.to_be_bytes()); // samples_per_chunk=1
    buf.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index=1
    buf
}

/// stsz box (样本大小)
fn build_stsz(track: &TrackCollector) -> Vec<u8> {
    let n = track.samples.len() as u32;

    let mut buf = Vec::new();
    let content_size = 12 + n * 4;
    write_box_header(&mut buf, 8 + content_size, b"stsz");
    buf.extend_from_slice(&[0; 4]); // version + flags
    buf.extend_from_slice(&0u32.to_be_bytes()); // sample_size=0 (可变)
    buf.extend_from_slice(&n.to_be_bytes());
    for sample in &track.samples {
        buf.extend_from_slice(&sample.size.to_be_bytes());
    }
    buf
}

/// stco box (块偏移)
fn build_stco(track: &TrackCollector) -> Vec<u8> {
    let n = track.samples.len() as u32;
    let needs_64bit = track.samples.iter().any(|s| s.offset > u32::MAX as u64);

    if needs_64bit {
        // co64
        let mut buf = Vec::new();
        let content_size = 8 + n * 8;
        write_box_header(&mut buf, 8 + content_size, b"co64");
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&n.to_be_bytes());
        for sample in &track.samples {
            buf.extend_from_slice(&sample.offset.to_be_bytes());
        }
        buf
    } else {
        // stco
        let mut buf = Vec::new();
        let content_size = 8 + n * 4;
        write_box_header(&mut buf, 8 + content_size, b"stco");
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&n.to_be_bytes());
        for sample in &track.samples {
            buf.extend_from_slice(&(sample.offset as u32).to_be_bytes());
        }
        buf
    }
}

/// stss box (同步样本 / 关键帧)
fn build_stss(track: &TrackCollector) -> Vec<u8> {
    let keyframes: Vec<u32> = track
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_keyframe)
        .map(|(i, _)| i as u32 + 1) // 1-based
        .collect();

    let mut buf = Vec::new();
    let content_size = 8 + keyframes.len() as u32 * 4;
    write_box_header(&mut buf, 8 + content_size, b"stss");
    buf.extend_from_slice(&[0; 4]);
    buf.extend_from_slice(&(keyframes.len() as u32).to_be_bytes());
    for kf in &keyframes {
        buf.extend_from_slice(&kf.to_be_bytes());
    }
    buf
}

/// ctts box (Composition Time to Sample)
fn build_ctts(track: &TrackCollector) -> Vec<u8> {
    // 版本 1 (允许负 offset)
    let entries = rle_cts_offsets(track);

    let mut buf = Vec::new();
    let content_size = 8 + entries.len() as u32 * 8;
    write_box_header(&mut buf, 8 + content_size, b"ctts");
    // version=1 (支持负值 CTS)
    buf.extend_from_slice(&[0, 0, 0, 0]);
    buf.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (count, offset) in &entries {
        buf.extend_from_slice(&count.to_be_bytes());
        buf.extend_from_slice(&offset.to_be_bytes());
    }
    buf
}

// ============================================================
// 工具函数
// ============================================================

/// 写 box 头 (size + fourcc) 到内存缓冲区
fn write_box_header(buf: &mut Vec<u8>, size: u32, fourcc: &[u8; 4]) {
    buf.extend_from_slice(&size.to_be_bytes());
    buf.extend_from_slice(fourcc);
}

/// 单位矩阵 (3x3, 每个元素 4 字节, 固定点)
const UNITY_MATRIX: [u8; 36] = [
    0x00, 0x01, 0x00, 0x00, // a = 1.0
    0x00, 0x00, 0x00, 0x00, // b = 0
    0x00, 0x00, 0x00, 0x00, // u = 0
    0x00, 0x00, 0x00, 0x00, // c = 0
    0x00, 0x01, 0x00, 0x00, // d = 1.0
    0x00, 0x00, 0x00, 0x00, // v = 0
    0x00, 0x00, 0x00, 0x00, // x = 0
    0x00, 0x00, 0x00, 0x00, // y = 0
    0x40, 0x00, 0x00, 0x00, // w = 1.0 (fixed 2.30)
];

/// CodecId → MP4 fourcc
fn codec_to_mp4_fourcc(codec_id: CodecId) -> TaoResult<[u8; 4]> {
    match codec_id {
        CodecId::H264 => Ok(*b"avc1"),
        CodecId::H265 => Ok(*b"hvc1"),
        CodecId::Aac => Ok(*b"mp4a"),
        CodecId::Mp3 => Ok(*b".mp3"),
        CodecId::Opus => Ok(*b"Opus"),
        CodecId::Flac => Ok(*b"fLaC"),
        CodecId::Vp9 => Ok(*b"vp09"),
        CodecId::Av1 => Ok(*b"av01"),
        _ => Err(TaoError::Unsupported(format!(
            "MP4: 不支持编解码器 {}",
            codec_id
        ))),
    }
}

/// RLE 压缩 duration 列表
fn rle_durations(track: &TrackCollector) -> Vec<(u32, u32)> {
    let mut entries = Vec::new();
    for sample in &track.samples {
        if let Some(last) = entries.last_mut() {
            let (count, dur): &mut (u32, u32) = last;
            if *dur == sample.duration {
                *count += 1;
                continue;
            }
        }
        entries.push((1, sample.duration));
    }
    entries
}

/// 检查是否有非零 CTS offset
fn has_cts_offsets(track: &TrackCollector) -> bool {
    track.samples.iter().any(|s| s.cts_offset != 0)
}

/// RLE 压缩 CTS offset 列表
fn rle_cts_offsets(track: &TrackCollector) -> Vec<(u32, i32)> {
    let mut entries = Vec::new();
    for sample in &track.samples {
        if let Some(last) = entries.last_mut() {
            let (count, cts): &mut (u32, i32) = last;
            if *cts == sample.cts_offset {
                *count += 1;
                continue;
            }
        }
        entries.push((1, sample.cts_offset));
    }
    entries
}

/// 写 MPEG-4 descriptor 长度 (可变长编码)
fn write_desc_length(buf: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        buf.push(len as u8);
    } else if len < 0x4000 {
        buf.push(0x80 | ((len >> 7) as u8));
        buf.push((len & 0x7F) as u8);
    } else if len < 0x20_0000 {
        buf.push(0x80 | ((len >> 14) as u8));
        buf.push(0x80 | ((len >> 7) as u8 & 0x7F));
        buf.push((len & 0x7F) as u8);
    } else {
        buf.push(0x80 | ((len >> 21) as u8));
        buf.push(0x80 | ((len >> 14) as u8 & 0x7F));
        buf.push(0x80 | ((len >> 7) as u8 & 0x7F));
        buf.push((len & 0x7F) as u8);
    }
}

/// descriptor 长度的编码字节数
fn desc_length_size(len: usize) -> usize {
    if len < 0x80 {
        1
    } else if len < 0x4000 {
        2
    } else if len < 0x20_0000 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;
    use tao_core::{ChannelLayout, PixelFormat, Rational, SampleFormat};

    use crate::stream::AudioStreamParams;
    use crate::stream::VideoStreamParams;

    fn make_video_stream() -> Stream {
        Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 90000),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: vec![
                // 最小 avcC: version=1, profile=66, compat=0, level=30,
                // lengthSize=4, numSPS=1, spsLen=4, sps, numPPS=1, ppsLen=2, pps
                0x01, 0x42, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x04, 0x67, 0x42, 0x00, 0x1E, 0x01, 0x00,
                0x02, 0x68, 0xCE,
            ],
            params: StreamParams::Video(VideoStreamParams {
                width: 320,
                height: 240,
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
            time_base: Rational::new(1, 44100),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: vec![0x12, 0x10], // AAC-LC, 44100Hz, stereo
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 44100,
                channel_layout: ChannelLayout::from_channels(2),
                sample_format: SampleFormat::F32,
                bit_rate: 128000,
                frame_size: 1024,
            }),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn test_ftyp_write() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        write_ftyp(&mut io).unwrap();
        let pos = io.position().unwrap();
        assert_eq!(pos, 28); // 8(header) + 4(major) + 4(minor) + 3*4(brands) = 28
    }

    #[test]
    fn test_write_video_only() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream()];

        let mut muxer = Mp4Muxer::create().unwrap();
        muxer.write_header(&mut io, &streams).unwrap();

        // 写入 3 个视频包
        for i in 0..3 {
            let mut pkt = Packet::from_data(vec![0xAA; 100]);
            pkt.stream_index = 0;
            pkt.pts = i * 3000;
            pkt.dts = i * 3000;
            pkt.duration = 3000;
            pkt.is_keyframe = i == 0;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        muxer.write_trailer(&mut io).unwrap();
        let pos = io.position().unwrap();
        assert!(pos > 0, "应该写入了数据");
    }

    #[test]
    fn test_write_av() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let streams = vec![make_video_stream(), make_audio_stream()];

        let mut muxer = Mp4Muxer::create().unwrap();
        muxer.write_header(&mut io, &streams).unwrap();

        // 视频包
        for i in 0..3 {
            let mut pkt = Packet::from_data(vec![0xBB; 200]);
            pkt.stream_index = 0;
            pkt.pts = i * 3000;
            pkt.dts = i * 3000;
            pkt.duration = 3000;
            pkt.is_keyframe = i == 0;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        // 音频包
        for i in 0..5 {
            let mut pkt = Packet::from_data(vec![0xCC; 50]);
            pkt.stream_index = 1;
            pkt.pts = i * 1024;
            pkt.dts = i * 1024;
            pkt.duration = 1024;
            pkt.is_keyframe = true;
            muxer.write_packet(&mut io, &pkt).unwrap();
        }

        muxer.write_trailer(&mut io).unwrap();
        let pos = io.position().unwrap();
        assert!(pos > 500, "MP4 应有 ftyp + mdat + moov");
    }

    #[test]
    fn test_codec_fourcc_mapping() {
        assert_eq!(codec_to_mp4_fourcc(CodecId::H264).unwrap(), *b"avc1");
        assert_eq!(codec_to_mp4_fourcc(CodecId::H265).unwrap(), *b"hvc1");
        assert_eq!(codec_to_mp4_fourcc(CodecId::Aac).unwrap(), *b"mp4a");
        assert!(codec_to_mp4_fourcc(CodecId::None).is_err());
    }

    #[test]
    fn test_rle_duration_compress() {
        let track = TrackCollector {
            stream_index: 0,
            stream: make_video_stream(),
            timescale: 90000,
            samples: vec![
                SampleEntry {
                    offset: 0,
                    size: 100,
                    duration: 3000,
                    cts_offset: 0,
                    is_keyframe: true,
                },
                SampleEntry {
                    offset: 100,
                    size: 100,
                    duration: 3000,
                    cts_offset: 0,
                    is_keyframe: false,
                },
                SampleEntry {
                    offset: 200,
                    size: 100,
                    duration: 6000,
                    cts_offset: 0,
                    is_keyframe: false,
                },
            ],
            last_dts: 0,
        };
        let entries = rle_durations(&track);
        assert_eq!(entries, vec![(2, 3000), (1, 6000)]);
    }

    #[test]
    fn test_empty_stream_error() {
        let backend = MemoryBackend::new();
        let mut io = IoContext::new(Box::new(backend));
        let mut muxer = Mp4Muxer::create().unwrap();
        assert!(muxer.write_header(&mut io, &[]).is_err());
    }
}
