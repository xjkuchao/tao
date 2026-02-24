//! H.264 AnnexB Elementary Stream 解封装器.
//!
//! 处理以 AnnexB start code (00 00 01 或 00 00 00 01) 分隔的
//! H.264 NAL 单元裸流. 每个 packet 包含一个完整的访问单元
//! (从一个 VCL NAL 到下一个 VCL NAL 开始之前的所有 NAL).

use bytes::Bytes;
use tao_codec::{CodecId, Packet};
use tao_core::{MediaType, PixelFormat, Rational, TaoError, TaoResult};

use crate::demuxer::{Demuxer, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::stream::{Stream, StreamParams, VideoStreamParams};

/// H.264 AnnexB ES 解封装器
pub struct H264EsDemuxer {
    streams: Vec<Stream>,
    /// 预读的全部 NAL 数据 (AnnexB 格式)
    data: Vec<u8>,
    /// 当前读取偏移
    offset: usize,
    frame_count: u64,
    eof: bool,
}

impl H264EsDemuxer {
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            streams: Vec::new(),
            data: Vec::new(),
            offset: 0,
            frame_count: 0,
            eof: false,
        }))
    }
}

/// 在 data[start..] 中查找下一个 AnnexB start code (00 00 01 或 00 00 00 01).
/// 返回 start code 起始位置.
fn find_start_code(data: &[u8], start: usize) -> Option<usize> {
    if data.len() < start + 3 {
        return None;
    }
    let mut i = start;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                return Some(i);
            }
            if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// 跳过 start code (00 00 01 或 00 00 00 01), 返回 NAL 首字节的偏移.
fn skip_start_code(data: &[u8], pos: usize) -> usize {
    if pos + 3 < data.len()
        && data[pos] == 0
        && data[pos + 1] == 0
        && data[pos + 2] == 0
        && data[pos + 3] == 1
    {
        pos + 4
    } else {
        pos + 3
    }
}

/// 获取 NAL unit type (低 5 位).
fn nal_type(nal_header: u8) -> u8 {
    nal_header & 0x1F
}

/// VCL NAL 类型 (slice data).
fn is_vcl_nal(nt: u8) -> bool {
    (1..=5).contains(&nt)
}

impl Demuxer for H264EsDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::H264Es
    }

    fn name(&self) -> &str {
        "h264"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        let file_size = io.size().unwrap_or(0) as usize;
        let read_limit = file_size.min(256 * 1024 * 1024);
        let mut buf = vec![0u8; read_limit];
        let mut total_read = 0usize;
        loop {
            if total_read >= buf.len() {
                break;
            }
            match io.read_bytes(buf.len() - total_read) {
                Ok(chunk) => {
                    let n = chunk.len();
                    if n == 0 {
                        break;
                    }
                    buf[total_read..total_read + n].copy_from_slice(&chunk);
                    total_read += n;
                }
                Err(_) => break,
            }
        }
        buf.truncate(total_read);

        if find_start_code(&buf, 0).is_none() {
            return Err(TaoError::InvalidData(
                "H264 ES: 未找到 AnnexB start code".into(),
            ));
        }

        self.data = buf;
        self.offset = 0;

        let stream = Stream {
            index: 0,
            media_type: MediaType::Video,
            codec_id: CodecId::H264,
            time_base: Rational::new(1, 25),
            duration: -1,
            start_time: 0,
            nb_frames: 0,
            extra_data: Vec::new(),
            params: StreamParams::Video(VideoStreamParams {
                width: 0,
                height: 0,
                pixel_format: PixelFormat::Yuv420p,
                frame_rate: Rational::new(25, 1),
                sample_aspect_ratio: Rational::new(1, 1),
                bit_rate: 0,
            }),
            metadata: Vec::new(),
        };
        self.streams.push(stream);
        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn read_packet(&mut self, _io: &mut IoContext) -> TaoResult<Packet> {
        if self.eof || self.offset >= self.data.len() {
            return Err(TaoError::Eof);
        }

        let sc_pos = match find_start_code(&self.data, self.offset) {
            Some(p) => p,
            None => {
                self.eof = true;
                return Err(TaoError::Eof);
            }
        };

        let nal_start = skip_start_code(&self.data, sc_pos);
        if nal_start >= self.data.len() {
            self.eof = true;
            return Err(TaoError::Eof);
        }

        let first_nal_type = nal_type(self.data[nal_start]);

        // 收集当前访问单元: 从第一个 NAL 到下一个 VCL NAL 的开始之前
        let au_start = sc_pos;
        let mut search_pos = nal_start + 1;
        let mut found_vcl = is_vcl_nal(first_nal_type);

        loop {
            let next_sc = match find_start_code(&self.data, search_pos) {
                Some(p) => p,
                None => {
                    // 文件尾, 当前 AU 包含剩余所有数据
                    self.offset = self.data.len();
                    self.eof = true;
                    break;
                }
            };

            let next_nal_start = skip_start_code(&self.data, next_sc);
            if next_nal_start >= self.data.len() {
                self.offset = self.data.len();
                self.eof = true;
                break;
            }

            let next_nt = nal_type(self.data[next_nal_start]);

            if is_vcl_nal(next_nt) {
                if found_vcl {
                    // 新的 VCL NAL 且已有 VCL -> 新的 AU 开始
                    self.offset = next_sc;
                    break;
                }
                found_vcl = true;
            }

            // AUD (NAL type 9) 标志新的 AU
            if next_nt == 9 && found_vcl {
                self.offset = next_sc;
                break;
            }

            search_pos = next_nal_start + 1;
        }

        let au_end = if self.eof {
            self.data.len()
        } else {
            self.offset
        };
        let packet_data = &self.data[au_start..au_end];

        if packet_data.is_empty() {
            self.eof = true;
            return Err(TaoError::Eof);
        }

        let is_keyframe = {
            let mut s = au_start;
            let mut kf = false;
            while let Some(p) = find_start_code(&self.data[..au_end], s) {
                let ns = skip_start_code(&self.data, p);
                if ns < au_end {
                    let nt = nal_type(self.data[ns]);
                    if nt == 5 {
                        kf = true;
                        break;
                    }
                }
                s = ns.max(s + 1);
            }
            kf
        };

        let pts = self.frame_count as i64;
        self.frame_count += 1;

        Ok(Packet {
            stream_index: 0,
            data: Bytes::copy_from_slice(packet_data),
            pts,
            dts: pts,
            is_keyframe,
            duration: 1,
            time_base: Rational::new(1, 25),
            pos: au_start as i64,
        })
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        _stream_index: usize,
        _timestamp: i64,
        _flags: SeekFlags,
    ) -> TaoResult<()> {
        Err(TaoError::NotImplemented("H264 ES 不支持 seek".into()))
    }

    fn duration(&self) -> Option<f64> {
        None
    }
}

/// H264 AnnexB 格式探测器
pub struct H264EsProbe;

impl FormatProbe for H264EsProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        let mut valid_nal_count = 0u32;
        let mut pos = 0usize;
        let limit = data.len().min(4096);
        while pos + 3 < limit {
            let (sc_end, found) = if pos + 3 < data.len()
                && data[pos] == 0
                && data[pos + 1] == 0
                && data[pos + 2] == 0
                && pos + 4 < data.len()
                && data[pos + 3] == 1
            {
                (pos + 4, true)
            } else if data[pos] == 0 && data[pos + 1] == 0 && data[pos + 2] == 1 {
                (pos + 3, true)
            } else {
                (pos + 1, false)
            };
            if !found {
                pos = sc_end;
                continue;
            }
            if sc_end >= data.len() {
                break;
            }
            let nal_byte = data[sc_end];
            let nt = nal_byte & 0x1F;
            let forbidden = nal_byte >> 7;
            if forbidden == 0 && (1..=13).contains(&nt) {
                valid_nal_count += 1;
            }
            pos = sc_end + 1;
        }
        if valid_nal_count >= 2 {
            return Some(SCORE_MAX);
        }
        if valid_nal_count == 1 {
            return Some(SCORE_MAX - 5);
        }

        if let Some(name) = filename {
            if let Some(ext) = name.rsplit('.').next() {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "h264" || ext_lower == "264" {
                    return Some(SCORE_EXTENSION);
                }
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::H264Es
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h264_probe() {
        let probe = H264EsProbe;
        // SPS (NAL type 7)
        assert!(probe.probe(&[0x00, 0x00, 0x00, 0x01, 0x67], None).is_some());
        // IDR (NAL type 5)
        assert!(probe.probe(&[0x00, 0x00, 0x01, 0x65], None).is_some());
        // 扩展名
        assert!(probe.probe(&[], Some("test.h264")).is_some());
        assert!(probe.probe(&[], Some("test.264")).is_some());
        // 无效
        assert!(probe.probe(&[0xFF, 0xD8, 0xFF, 0xE0], None).is_none());
    }

    #[test]
    fn test_find_start_code() {
        // data: [00 00 00 01 67 42 00 00 01 68]
        //        ^-- 4字节起始码@0       ^-- 3字节起始码@6
        let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x68];
        assert_eq!(find_start_code(&data, 0), Some(0));
        // 跳过首个4字节起始码后(偏移4), 下一个3字节起始码位于偏移6
        assert_eq!(find_start_code(&data, 4), Some(6));
    }
}
