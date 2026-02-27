//! CUE Sheet 解析器.
//!
//! 支持读取 CUE 文件并将其中的轨道信息映射为 chapters.
//!
//! CUE 文件结构示例:
//! ```text
//! REM GENRE "Pop"
//! REM DATE "2000"
//! PERFORMER "Jay Chou"
//! TITLE "Jay"
//! FILE "CDImage.wav" WAVE
//!   TRACK 01 AUDIO
//!     TITLE "可爱女人"
//!     PERFORMER "Jay Chou"
//!     INDEX 01 00:00:00
//!   TRACK 02 AUDIO
//!     TITLE "完美主义"
//!     PERFORMER "Jay Chou"
//!     INDEX 01 04:23:17
//! ```
//!
//! 设计说明:
//! - CUE 文件引用外部音频文件 (通常是 WAV/FLAC)
//! - 每个 TRACK 映射为一个 Chapter
//! - demuxer 为底层音频文件创建单个流
//! - 读取数据包时直接委托给底层 demuxer

use encoding_rs::{BIG5, Encoding, GBK, UTF_16BE, UTF_16LE};
use log::debug;
use std::path::{Path, PathBuf};
use tao_codec::Packet;
use tao_core::{TaoError, TaoResult};

use crate::demuxer::{Demuxer, DemuxerChapter, SeekFlags};
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::probe::{FormatProbe, ProbeScore, SCORE_EXTENSION, SCORE_MAX};
use crate::registry::FormatRegistry;
use crate::stream::Stream;

/// CUE 解封装器
pub struct CueDemuxer {
    /// 底层音频 demuxer
    inner_demuxer: Option<Box<dyn Demuxer>>,
    /// 音频文件的 IoContext
    audio_io: Option<IoContext>,
    /// 全局元数据 (REM, PERFORMER, TITLE 等)
    metadata: Vec<(String, String)>,
    /// Chapters (每个 TRACK 一个)
    chapters: Vec<DemuxerChapter>,
}

impl CueDemuxer {
    /// 创建 CUE 解封装器实例 (工厂函数)
    pub fn create() -> TaoResult<Box<dyn Demuxer>> {
        Ok(Box::new(Self {
            inner_demuxer: None,
            audio_io: None,
            metadata: Vec::new(),
            chapters: Vec::new(),
        }))
    }

    /// 解析 CUE 文本内容
    fn parse_cue_text(&mut self, text: &str) -> TaoResult<PathBuf> {
        let mut audio_file_path: Option<PathBuf> = None;
        let mut current_track: Option<CueTrack> = None;
        let mut global_performer: Option<String> = None;
        let mut global_title: Option<String> = None;
        let mut line_count = 0;

        for line in text.lines() {
            let trimmed = line.trim();
            line_count += 1;

            debug!("CUE 行 {}: {}", line_count, trimmed);

            if trimmed.is_empty() || trimmed.starts_with("REM COMMENT") {
                continue;
            }

            // 解析全局元数据
            if let Some(genre) = parse_field(trimmed, "REM GENRE") {
                self.metadata.push(("genre".to_string(), genre));
            } else if let Some(date) = parse_field(trimmed, "REM DATE") {
                self.metadata.push(("date".to_string(), date));
            } else if let Some(performer) = parse_field(trimmed, "PERFORMER") {
                if current_track.is_none() {
                    global_performer = Some(performer.clone());
                    self.metadata.push(("artist".to_string(), performer));
                } else if let Some(ref mut track) = current_track {
                    track.performer = Some(performer);
                }
            } else if let Some(title) = parse_field(trimmed, "TITLE") {
                if current_track.is_none() {
                    global_title = Some(title.clone());
                    self.metadata.push(("album".to_string(), title));
                } else if let Some(ref mut track) = current_track {
                    track.title = Some(title);
                }
            } else if trimmed.starts_with("FILE") {
                // 解析 FILE "path" WAVE
                if let Some(path) = parse_file_path(trimmed) {
                    audio_file_path = Some(path);
                }
            } else if trimmed.starts_with("TRACK") {
                // 保存之前的 track
                if let Some(track) = current_track.take() {
                    self.add_chapter(track, global_performer.as_deref(), global_title.as_deref());
                }
                // 开始新 track
                if let Some(track_num) = parse_track_number(trimmed) {
                    current_track = Some(CueTrack {
                        number: track_num,
                        title: None,
                        performer: None,
                        index_time: None,
                    });
                }
            } else if let Some(time_str) = parse_field(trimmed, "INDEX 01") {
                // INDEX 01 MM:SS:FF
                if let Some(ref mut track) = current_track {
                    track.index_time = parse_msf_time(&time_str);
                }
            }
        }

        // 保存最后一个 track
        if let Some(track) = current_track.take() {
            self.add_chapter(track, global_performer.as_deref(), global_title.as_deref());
        }

        debug!(
            "CUE 解析完成，共 {} 行，文件路径: {:?}",
            line_count, audio_file_path
        );

        audio_file_path
            .ok_or_else(|| TaoError::InvalidData("CUE 文件中未找到 FILE 字段".to_string()))
    }

    /// 添加一个 chapter
    fn add_chapter(
        &mut self,
        track: CueTrack,
        global_performer: Option<&str>,
        global_title: Option<&str>,
    ) {
        let start_time = track.index_time;

        let mut metadata = Vec::new();
        if let Some(title) = &track.title {
            metadata.push(("title".to_string(), title.clone()));
        }
        if let Some(performer) = &track.performer {
            metadata.push(("artist".to_string(), performer.clone()));
        } else if let Some(gp) = global_performer {
            metadata.push(("artist".to_string(), gp.to_string()));
        }
        if let Some(album) = global_title {
            metadata.push(("album".to_string(), album.to_string()));
        }
        metadata.push(("track".to_string(), track.number.to_string()));

        self.chapters.push(DemuxerChapter {
            start_time,
            end_time: None, // 将在所有 chapters 解析完后计算
            metadata,
        });
    }

    /// 计算每个 chapter 的结束时间
    fn finalize_chapters(&mut self, total_duration: Option<f64>) {
        for i in 0..self.chapters.len() {
            if i + 1 < self.chapters.len() {
                // 使用下一个 chapter 的开始时间作为本 chapter 的结束时间
                self.chapters[i].end_time = self.chapters[i + 1].start_time;
            } else {
                // 最后一个 chapter 的结束时间是总时长
                self.chapters[i].end_time = total_duration;
            }
        }
    }
}

impl Demuxer for CueDemuxer {
    fn format_id(&self) -> FormatId {
        FormatId::Cue
    }

    fn name(&self) -> &str {
        "cue"
    }

    fn open(&mut self, io: &mut IoContext) -> TaoResult<()> {
        // 1. 读取整个 CUE 文件内容
        // CUE 文件通常很小（几 KB），一次性读取
        let file_size = io
            .size()
            .ok_or_else(|| TaoError::InvalidData("无法获取 CUE 文件大小".to_string()))?
            as usize;

        if file_size == 0 {
            return Err(TaoError::InvalidData("CUE 文件为空".to_string()));
        }

        debug!("CUE 文件大小: {} 字节", file_size);

        // 使用 read_bytes 一次性读取所有数据
        let cue_content = io.read_bytes(file_size)?;

        debug!("成功读取 {} 字节 CUE 内容", cue_content.len());

        // 2. 解析 CUE 文件，获取音频文件路径
        let cue_text = decode_cue_text(&cue_content)?;
        let audio_file_relative_path = self.parse_cue_text(&cue_text)?;
        let audio_file_path = resolve_audio_path(audio_file_relative_path, io.source_path());

        // 3. 音频文件路径处理
        // 注意: CUE 文件中的路径通常是相对路径，相对于 CUE 文件所在目录
        debug!("CUE 文件引用音频文件: {}", audio_file_path.display());

        // 4. 打开音频文件
        let audio_path_str = audio_file_path
            .to_str()
            .ok_or_else(|| TaoError::InvalidData("无效的音频文件路径".to_string()))?;

        let mut audio_io = IoContext::open_read(audio_path_str)?;

        // 5. 使用 FormatRegistry 探测并打开音频文件
        let mut registry = FormatRegistry::new();
        crate::register_all(&mut registry);
        let probe_result = registry.probe_input(&mut audio_io, Some(audio_path_str))?;

        let mut demuxer = registry.create_demuxer(probe_result.format_id)?;

        demuxer.open(&mut audio_io)?;

        // 6. 获取音频文件的总时长
        let total_duration = demuxer.duration();

        // 7. 计算每个 chapter 的结束时间
        self.finalize_chapters(total_duration);

        // 8. 保存底层 demuxer 和 IoContext
        self.inner_demuxer = Some(demuxer);
        self.audio_io = Some(audio_io);

        debug!(
            "CUE 解析完成: {} 个轨道, 总时长: {:?}秒",
            self.chapters.len(),
            total_duration
        );

        Ok(())
    }

    fn streams(&self) -> &[Stream] {
        self.inner_demuxer
            .as_ref()
            .map(|d| d.streams())
            .unwrap_or(&[])
    }

    fn read_packet(&mut self, _io: &mut IoContext) -> TaoResult<Packet> {
        let demuxer = self
            .inner_demuxer
            .as_mut()
            .ok_or_else(|| TaoError::InvalidData("CUE demuxer 未初始化".to_string()))?;
        let audio_io = self
            .audio_io
            .as_mut()
            .ok_or_else(|| TaoError::InvalidData("音频文件 IoContext 未初始化".to_string()))?;
        demuxer.read_packet(audio_io)
    }

    fn seek(
        &mut self,
        _io: &mut IoContext,
        stream_index: usize,
        timestamp: i64,
        flags: SeekFlags,
    ) -> TaoResult<()> {
        let demuxer = self
            .inner_demuxer
            .as_mut()
            .ok_or_else(|| TaoError::InvalidData("CUE demuxer 未初始化".to_string()))?;
        let audio_io = self
            .audio_io
            .as_mut()
            .ok_or_else(|| TaoError::InvalidData("音频文件 IoContext 未初始化".to_string()))?;
        demuxer.seek(audio_io, stream_index, timestamp, flags)
    }

    fn duration(&self) -> Option<f64> {
        self.inner_demuxer.as_ref().and_then(|d| d.duration())
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }

    fn chapters(&self) -> &[DemuxerChapter] {
        &self.chapters
    }

    fn format_long_name(&self) -> Option<&str> {
        Some("CUE Sheet")
    }
}

fn resolve_audio_path(path_from_cue: PathBuf, cue_source: Option<&str>) -> PathBuf {
    if path_from_cue.is_absolute() {
        return path_from_cue;
    }

    let base_dir = cue_source.and_then(|p| Path::new(p).parent().map(|dir| dir.to_path_buf()));

    if let Some(dir) = base_dir {
        let joined = dir.join(&path_from_cue);
        debug!(
            "CUE 相对路径解析: base={}, file={}, resolved={}",
            dir.display(),
            path_from_cue.display(),
            joined.display()
        );
        return joined;
    }

    path_from_cue
}

fn decode_cue_text(data: &[u8]) -> TaoResult<String> {
    if data.is_empty() {
        return Err(TaoError::InvalidData("CUE 文件为空".to_string()));
    }

    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let text = String::from_utf8_lossy(&data[3..]).to_string();
        debug!("CUE 编码探测: UTF-8 BOM");
        return Ok(text);
    }

    if data.starts_with(&[0xFF, 0xFE]) {
        if let Some(text) = decode_with_encoding(&data[2..], UTF_16LE, "UTF-16LE BOM") {
            return Ok(text);
        }
    }

    if data.starts_with(&[0xFE, 0xFF]) {
        if let Some(text) = decode_with_encoding(&data[2..], UTF_16BE, "UTF-16BE BOM") {
            return Ok(text);
        }
    }

    if let Ok(text) = std::str::from_utf8(data) {
        debug!("CUE 编码探测: UTF-8");
        return Ok(text.to_string());
    }

    if let Some(text) = decode_with_encoding(data, GBK, "GBK") {
        return Ok(text);
    }

    if let Some(text) = decode_with_encoding(data, BIG5, "Big5") {
        return Ok(text);
    }

    if let Some(text) = decode_with_encoding(data, UTF_16LE, "UTF-16LE") {
        return Ok(text);
    }

    if let Some(text) = decode_with_encoding(data, UTF_16BE, "UTF-16BE") {
        return Ok(text);
    }

    let (text, _) = GBK.decode_without_bom_handling(data);
    debug!("CUE 编码探测: GBK 宽松解码");
    Ok(text.into_owned())
}

fn decode_with_encoding(data: &[u8], encoding: &'static Encoding, label: &str) -> Option<String> {
    let (text, had_errors) = encoding.decode_without_bom_handling(data);
    if had_errors {
        return None;
    }
    debug!("CUE 编码探测: {}", label);
    Some(text.into_owned())
}

/// CUE 轨道信息 (临时解析结构)
#[derive(Debug)]
struct CueTrack {
    number: u32,
    title: Option<String>,
    performer: Option<String>,
    index_time: Option<f64>,
}

/// 解析字段值 (去除引号)
fn parse_field(line: &str, prefix: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix(prefix) {
        return Some(unquote(rest.trim()));
    }
    None
}

/// 解析 FILE 字段的路径
fn parse_file_path(line: &str) -> Option<PathBuf> {
    // FILE "path" WAVE or FILE path WAVE
    let trimmed = line.trim();
    if !trimmed.starts_with("FILE") {
        return None;
    }

    let rest = trimmed["FILE".len()..].trim();

    // 查找引号
    if let Some(quote_start) = rest.find('"') {
        // 查找结束引号
        if let Some(quote_end) = rest[quote_start + 1..].find('"') {
            let path = &rest[quote_start + 1..quote_start + 1 + quote_end];
            return Some(PathBuf::from(path));
        }
    }

    // 没有引号，按空格分割
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if !parts.is_empty() {
        // 第一个部分是路径，最后一个可能是 WAVE/MP3 等
        return Some(PathBuf::from(parts[0]));
    }

    None
}

/// 解析 TRACK 编号
fn parse_track_number(line: &str) -> Option<u32> {
    // TRACK 01 AUDIO
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 && parts[0] == "TRACK" {
        return parts[1].parse().ok();
    }
    None
}

/// 解析 MSF 时间 (MM:SS:FF) -> 秒
///
/// MM = 分钟
/// SS = 秒
/// FF = 帧 (75 帧 = 1 秒)
fn parse_msf_time(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() == 3 {
        let minutes: u32 = parts[0].parse().ok()?;
        let seconds: u32 = parts[1].parse().ok()?;
        let frames: u32 = parts[2].parse().ok()?;
        let total_seconds = (minutes * 60 + seconds) as f64 + (frames as f64 / 75.0);
        return Some(total_seconds);
    }
    None
}

/// 去除字符串的引号
fn unquote(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// CUE 格式探测器
pub struct CueProbe;

impl FormatProbe for CueProbe {
    fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeScore> {
        // 检查文件扩展名
        if let Some(name) = filename {
            if name.to_lowercase().ends_with(".cue") {
                return Some(SCORE_EXTENSION);
            }
        }

        // 检查内容特征
        if data.len() >= 10 {
            let content = String::from_utf8_lossy(data);
            if content.contains("FILE") && content.contains("TRACK") {
                return Some(SCORE_MAX);
            }
        }

        None
    }

    fn format_id(&self) -> FormatId {
        FormatId::Cue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_msf_time() {
        assert_eq!(parse_msf_time("00:00:00"), Some(0.0));
        assert_eq!(parse_msf_time("01:00:00"), Some(60.0));
        assert_eq!(parse_msf_time("00:00:75"), Some(1.0));
        assert_eq!(parse_msf_time("04:23:17"), Some(263.0 + 17.0 / 75.0));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("no quotes"), "no quotes");
        assert_eq!(unquote("  \"trimmed\"  "), "trimmed");
    }

    #[test]
    fn test_parse_field() {
        assert_eq!(
            parse_field("PERFORMER \"Jay Chou\"", "PERFORMER"),
            Some("Jay Chou".to_string())
        );
        assert_eq!(
            parse_field("TITLE \"可爱女人\"", "TITLE"),
            Some("可爱女人".to_string())
        );
        assert_eq!(
            parse_field("INDEX 01 00:00:00", "INDEX 01"),
            Some("00:00:00".to_string())
        );
    }

    #[test]
    fn test_parse_track_number() {
        assert_eq!(parse_track_number("TRACK 01 AUDIO"), Some(1));
        assert_eq!(parse_track_number("TRACK 12 AUDIO"), Some(12));
        assert_eq!(parse_track_number("INVALID"), None);
    }
}
