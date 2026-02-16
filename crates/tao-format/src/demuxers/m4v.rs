//! MPEG-4 Part 2 Elementary Stream (M4V) 解封装器.
//!
//! M4V 是 MPEG-4 Part 2 视频的裸流格式, 不含容器结构.
//! 文件由一系列 VOP (Video Object Plane) 和 VOL (Video Object Layer) 组成.
//!
//! # M4V 文件结构
//! ```text
//! [Visual Object Sequence Header (0x000001B0)]
//! [Visual Object (0x000001B5)]
//! [Video Object Layer (0x000001B2)]
//! [VOP Header (0x000001B6)] + compressed data
//! [VOP Header (0x000001B6)] + compressed data
//! ...
//! ```
//!
//! ## Start Code 定义
//! - 0x000001B0: Visual Object Sequence Start
//! - 0x000001B1: Visual Object Sequence End
//! - 0x000001B2: User Data
//! - 0x000001B3: Group of VOP Start
//! - 0x000001B5: Visual Object Start
//! - 0x000001B6: VOP Start
//! - 0x00000120-2F: Video Object Layer Start

use bytes::Bytes;
use log::debug;
use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, PixelFormat, Rational, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore};
use crate::stream::{Stream, StreamParams, VideoStreamParams};

/// MPEG-4 start code 前缀
const START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// 常见 start code
const VISUAL_OBJECT_SEQ_START: u8 = 0xB0;
const VISUAL_OBJECT_SEQ_END: u8 = 0xB1;
const GROUP_OF_VOP: u8 = 0xB3;
const VISUAL_OBJECT: u8 = 0xB5;
const VOP_START: u8 = 0xB6;

/// 检查是否为 MPEG-4 start code
fn is_start_code(code: u8) -> bool {
    // 0x00~0x1F: 视频对象
    // 0x20~0x2F: 视频对象层
    // 0xB0~0xB6: 系统 start code
    (code <= 0x2F) || (0xB0..=0xB6).contains(&code)
}

/// M4V Elementary Stream 解封装器
pub struct M4vDemuxer {
    /// 流信息
    streams: Vec<Stream>,
    /// 当前帧号
    frame_count: u64,
    /// 时间基 (假设 25fps, 实际可从 VOL 中解析)
    timebase: Rational,
    /// 是否已到达文件尾
    eof: bool,
    /// 累积的 VOL/VO 等 extra_data (在遇到第一个 VOP 前收集)
    extra_data: Vec<u8>,
    /// 是否已收集 extra_data
    extra_data_collected: bool,
}

impl M4vDemuxer {
    /// 创建 M4V 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            frame_count: 0,
            timebase: Rational::new(1, 25), // 默认 25fps
            eof: false,
            extra_data: Vec::new(),
            extra_data_collected: false,
        }))
    }

    /// 查找下一个 start code, 返回 start code 的字节码和起始位置
    fn find_next_start_code(&self, io: &mut IoContext) -> TaoResult<Option<(u8, u64)>> {
        let mut window = [0u8; 3];

        // 读取初始 3 字节窗口
        for b in &mut window {
            match io.read_u8() {
                Ok(val) => *b = val,
                Err(e) if e.to_string().contains("unexpected end of file") => return Ok(None),
                Err(e) => return Err(e),
            }
        }

        loop {
            // 检查窗口是否匹配 start code 前缀
            if window == START_CODE_PREFIX {
                let pos_before = io.position()? - 3; // start code 前缀的位置
                // 读取 start code 字节
                match io.read_u8() {
                    Ok(code) if is_start_code(code) => return Ok(Some((code, pos_before))),
                    Ok(byte) => {
                        // 不是有效 start code, 将字节加入窗口继续
                        window[0] = window[1];
                        window[1] = window[2];
                        window[2] = byte;
                    }
                    Err(e) if e.to_string().contains("unexpected end of file") => return Ok(None),
                    Err(e) => return Err(e),
                }
            } else {
                // 滑动窗口
                window[0] = window[1];
                window[1] = window[2];
                match io.read_u8() {
                    Ok(byte) => window[2] = byte,
                    Err(e) if e.to_string().contains("unexpected end of file") => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// 读取从当前位置到下一个 start code 之间的数据
    fn read_until_next_start_code(&self, io: &mut IoContext) -> TaoResult<Vec<u8>> {
        let mut data = Vec::new();
        let mut window = [0u8; 3];

        // 读取初始窗口
        for b in &mut window {
            match io.read_u8() {
                Ok(val) => {
                    data.push(val);
                    *b = val;
                }
                Err(e) if e.to_string().contains("unexpected end of file") => {
                    return Ok(data);
                }
                Err(e) => return Err(e),
            }
        }

        loop {
            if window == START_CODE_PREFIX {
                // 找到 start code 前缀, 回退 3 字节并移除数据末尾的前缀
                io.seek(std::io::SeekFrom::Current(-4))?;
                data.truncate(data.len() - 3);
                return Ok(data);
            }

            window[0] = window[1];
            window[1] = window[2];
            match io.read_u8() {
                Ok(byte) => {
                    data.push(byte);
                    window[2] = byte;
                }
                Err(e) if e.to_string().contains("unexpected end of file") => {
                    return Ok(data);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Demuxer for M4vDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Mpeg4Es
    }

    fn name(&self) -> &str {
        "m4v"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        debug!("M4V: 开始解析 MPEG-4 Elementary Stream");

        // 收集所有序列头部信息 (直到第一个 VOP), 特别是 VOL header
        let mut extra_data = Vec::new();
        let mut has_vol = false;
        loop {
            let (code, pos) = self
                .find_next_start_code(io)?
                .ok_or_else(|| TaoError::InvalidData("M4V 文件为空".into()))?;

            if code == VOP_START {
                // 如果还没找到 VOL, 可能文件将 VOL 嵌入在首个 VOP 之后
                // 先收集这个 VOP 和后续数据到 extra_data
                if !has_vol {
                    extra_data.extend_from_slice(&[0x00, 0x00, 0x01, VOP_START]);
                    extra_data.extend(self.read_until_next_start_code(io)?);

                    // 继续查找下一个 start code
                    let next_result = self.find_next_start_code(io)?;
                    if let Some((next_code, next_pos)) = next_result {
                        // 检查是否为 VOL
                        if (0x20..=0x2F).contains(&next_code) {
                            has_vol = true;
                            extra_data.extend_from_slice(&[0x00, 0x00, 0x01, next_code]);
                            extra_data.extend(self.read_until_next_start_code(io)?);
                            // 继续收集更多头部
                            continue;
                        } else {
                            // 不是 VOL, 回退并结束收集
                            io.seek(std::io::SeekFrom::Start(next_pos))?;
                            break;
                        }
                    } else {
                        break;
                    }
                } else {
                    // 已有 VOL, 回退并结束收集
                    io.seek(std::io::SeekFrom::Start(pos))?;
                    break;
                }
            }

            // 收集所有非 VOP 的头部信息
            let is_vol = (0x20..=0x2F).contains(&code);
            if is_vol {
                has_vol = true;
            }

            extra_data.extend_from_slice(&[0x00, 0x00, 0x01, code]);
            extra_data.extend(self.read_until_next_start_code(io)?);

            if code == VISUAL_OBJECT_SEQ_END {
                return Err(TaoError::InvalidData("M4V 文件在首个 VOP 前就结束".into()));
            }
        }

        debug!(
            "M4V: 收集到 {} 字节 extra_data (含VOL: {})",
            extra_data.len(),
            has_vol
        );

        // 创建视频流
        let stream = Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::Mpeg4,
            time_base: self.timebase,
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: extra_data.clone(),
            params: StreamParams::Video(VideoStreamParams {
                width: 0,  // 解码器将从 extra_data 中解析
                height: 0, // 解码器将从 extra_data 中解析
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        };

        self.streams.push(stream);
        self.extra_data = extra_data;
        self.extra_data_collected = true;

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet> {
        if self.eof {
            return Err(TaoError::Eof);
        }

        // 累积一个完整的访问单元 (从 VOP start 到下一个 VOP start 或文件尾)
        let mut packet_data = Vec::new();

        // 第一个数据包需包含 extra_data
        if self.frame_count == 0 && self.extra_data_collected {
            packet_data.extend_from_slice(&self.extra_data);
        }

        let mut found_vop = false;

        loop {
            let result = self.find_next_start_code(io)?;
            let (code, code_pos) = match result {
                Some((c, pos)) => (c, pos),
                None => {
                    self.eof = true;
                    if packet_data.is_empty() || !found_vop {
                        return Err(TaoError::Eof);
                    }
                    break;
                }
            };

            match code {
                VOP_START => {
                    if found_vop {
                        // 遇到新的 VOP, 当前数据包完成
                        // 回退到此 VOP start code 前
                        io.seek(std::io::SeekFrom::Start(code_pos))?;
                        break;
                    }
                    found_vop = true;
                    // 包含 start code
                    packet_data.extend_from_slice(&[0x00, 0x00, 0x01, VOP_START]);
                    // 读取到下一个 start code 的数据
                    packet_data.extend(self.read_until_next_start_code(io)?);
                }
                VISUAL_OBJECT_SEQ_END => {
                    self.eof = true;
                    if packet_data.is_empty() || !found_vop {
                        return Err(TaoError::Eof);
                    }
                    break;
                }
                _ => {
                    // VOL, Visual Object, User Data 等头部信息
                    // 在第一帧后忽略 (已包含在 extra_data 中)
                    if self.frame_count == 0 {
                        packet_data.extend_from_slice(&[0x00, 0x00, 0x01, code]);
                        packet_data.extend(self.read_until_next_start_code(io)?);
                    } else {
                        // 跳过后续的 VOL 等头部
                        self.read_until_next_start_code(io)?;
                    }
                }
            }
        }

        if packet_data.is_empty() || !found_vop {
            return Err(TaoError::Eof);
        }

        let pts = self.frame_count as i64;
        let dts = self.frame_count as i64;
        self.frame_count += 1;

        let packet = Packet {
            stream_index: 0,
            data: Bytes::from(packet_data),
            pts,
            dts,
            is_keyframe: true, // 简化处理, 每个数据包都标记为关键帧
            duration: 1,
            time_base: self.timebase,
            pos: -1,
        };

        Ok(packet)
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented(
            "M4V Elementary Stream 不支持 seek".into(),
        ))
    }

    fn duration(&self) -> Option<f64> {
        None
    }
}

/// M4V 格式探测器
pub struct M4vProbe;

impl FormatProbe for M4vProbe {
    fn probe(&self, data: &[u8], _filename: Option<&str>) -> Option<ProbeScore> {
        // 至少需要 4 字节检查 start code
        if data.len() < 4 {
            return None;
        }

        // 检查是否以 MPEG-4 start code 开始
        if data[0..3] != START_CODE_PREFIX {
            return None;
        }

        let code = data[3];

        match code {
            // Visual Object Sequence Start
            VISUAL_OBJECT_SEQ_START => Some(90),
            // Visual Object
            VISUAL_OBJECT => Some(80),
            // Video Object / Video Object Layer (0x00-0x2F)
            0x00..=0x2F => Some(70),
            // Group of VOP
            GROUP_OF_VOP => Some(60),
            // VOP start
            VOP_START => Some(50),
            _ => None,
        }
    }

    fn format_id(&self) -> FormatId {
        FormatId::Mpeg4Es
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_m4v_probe() {
        let probe = M4vProbe;

        // Visual Object Sequence Start
        assert_eq!(probe.probe(&[0x00, 0x00, 0x01, 0xB0], None), Some(90));

        // Visual Object
        assert_eq!(probe.probe(&[0x00, 0x00, 0x01, 0xB5], None), Some(80));

        // Video Object Layer
        assert_eq!(probe.probe(&[0x00, 0x00, 0x01, 0x20], None), Some(70));

        // VOP start
        assert_eq!(probe.probe(&[0x00, 0x00, 0x01, 0xB6], None), Some(50));

        // 非 MPEG-4
        assert_eq!(probe.probe(&[0x00, 0x00, 0x01, 0xC0], None), None);
        assert_eq!(probe.probe(&[0xFF, 0xD8, 0xFF, 0xE0], None), None);
    }
}
