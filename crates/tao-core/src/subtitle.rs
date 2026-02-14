//! 字幕解析模块.
//!
//! 支持 SRT 和 ASS/SSA 格式的字幕解析.

use crate::{TaoError, TaoResult};

/// 单个字幕事件/ cue
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleEvent {
    /// 开始时间 (毫秒)
    pub start_ms: u64,
    /// 结束时间 (毫秒)
    pub end_ms: u64,
    /// 文本内容 (可能包含格式标签)
    pub text: String,
    /// 可选样式名 (用于 ASS)
    pub style: Option<String>,
}

/// 解析后的字幕轨道
#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    /// 格式类型
    pub format: SubtitleFormat,
    /// 字幕事件列表, 按开始时间排序
    pub events: Vec<SubtitleEvent>,
}

/// 字幕格式类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    Ass,
}

/// 解析 SRT 时间戳 "HH:MM:SS,mmm" 为毫秒数.
///
/// # 示例
/// ```
/// use tao_core::subtitle::parse_srt_timestamp;
///
/// assert_eq!(parse_srt_timestamp("00:00:01,000"), Some(1000));
/// assert_eq!(parse_srt_timestamp("01:23:45,500"), Some(5025500));
/// ```
pub fn parse_srt_timestamp(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return None;
    }
    let time_parts: Vec<&str> = parts[0].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hours: u64 = time_parts[0].trim().parse().ok()?;
    let minutes: u64 = time_parts[1].trim().parse().ok()?;
    let seconds: u64 = time_parts[2].trim().parse().ok()?;
    let millis: u64 = parts[1].trim().parse().ok()?;
    if millis >= 1000 {
        return None;
    }
    Some(hours * 3600 * 1000 + minutes * 60 * 1000 + seconds * 1000 + millis)
}

/// 解析 SRT 格式字幕内容.
///
/// SRT 格式:
/// - 序号
/// - 时间戳行 `HH:MM:SS,mmm --> HH:MM:SS,mmm`
/// - 文本内容 (可多行)
/// - 空行分隔
pub fn parse_srt(content: &str) -> TaoResult<SubtitleTrack> {
    let mut events = Vec::new();
    let content = content.trim();

    if content.is_empty() {
        return Ok(SubtitleTrack {
            format: SubtitleFormat::Srt,
            events,
        });
    }

    let blocks: Vec<&str> = content.split("\n\n").collect();

    for block in blocks {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 2 {
            continue;
        }

        // 第一行: 序号 (忽略)
        let _index = lines[0].trim();

        // 第二行: 时间戳
        let timestamp_line = lines[1].trim();
        let arrow_pos = timestamp_line.find("-->").ok_or_else(|| {
            TaoError::InvalidData(format!("无效的 SRT 时间戳行: {}", timestamp_line))
        })?;

        let start_str = timestamp_line[..arrow_pos].trim();
        let end_str = timestamp_line[arrow_pos + 3..].trim();

        let start_ms = parse_srt_timestamp(start_str)
            .ok_or_else(|| TaoError::InvalidData(format!("无效的 SRT 开始时间: {}", start_str)))?;
        let end_ms = parse_srt_timestamp(end_str)
            .ok_or_else(|| TaoError::InvalidData(format!("无效的 SRT 结束时间: {}", end_str)))?;

        // 第三行及以后: 文本
        let text = lines[2..].join("\n").trim().to_string();

        events.push(SubtitleEvent {
            start_ms,
            end_ms,
            text,
            style: None,
        });
    }

    events.sort_by_key(|e| e.start_ms);

    Ok(SubtitleTrack {
        format: SubtitleFormat::Srt,
        events,
    })
}

/// 解析 ASS 时间戳 "H:MM:SS.cc" 为毫秒数.
///
/// ASS 使用百分之一秒 (centiseconds), 不是毫秒.
///
/// # 示例
/// ```
/// use tao_core::subtitle::parse_ass_timestamp;
///
/// assert_eq!(parse_ass_timestamp("0:00:01.00"), Some(1000));
/// assert_eq!(parse_ass_timestamp("1:23:45.50"), Some(5025500));
/// ```
pub fn parse_ass_timestamp(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let time_parts: Vec<&str> = parts[0].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hours: u64 = time_parts[0].trim().parse().ok()?;
    let minutes: u64 = time_parts[1].trim().parse().ok()?;
    let seconds: u64 = time_parts[2].trim().parse().ok()?;
    let centis: u64 = parts[1].trim().parse().ok()?;
    if centis >= 100 {
        return None;
    }
    Some(hours * 3600 * 1000 + minutes * 60 * 1000 + seconds * 1000 + centis * 10)
}

/// 移除 ASS 覆盖标签 (如 `{\b1}`, `{\i1}` 等).
///
/// 不使用正则, 仅查找 `{\...}` 块并移除.
fn strip_ass_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_tag = false;
    let mut brace_depth = 0;

    while let Some(c) = chars.next() {
        if in_tag {
            if c == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    in_tag = false;
                }
            } else if c == '{' {
                brace_depth += 1;
            }
            continue;
        }

        if c == '{' && chars.peek() == Some(&'\\') {
            in_tag = true;
            brace_depth = 1;
            continue;
        }

        result.push(c);
    }

    result
}

/// 解析 ASS/SSA 格式字幕内容.
///
/// 解析 [Events] 段落中的 Dialogue 行, 提取开始时间、结束时间、样式和文本.
pub fn parse_ass(content: &str) -> TaoResult<SubtitleTrack> {
    let mut events = Vec::new();
    let content = content.trim();

    if content.is_empty() {
        return Ok(SubtitleTrack {
            format: SubtitleFormat::Ass,
            events,
        });
    }

    let mut in_events = false;
    let mut format_indices: Option<(usize, usize, usize, usize)> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.eq_ignore_ascii_case("[Events]") {
            in_events = true;
            format_indices = None;
            continue;
        }

        if in_events && line.starts_with("Format:") {
            let format_line = line[7..].trim();
            let format_parts: Vec<&str> = format_line.split(',').map(|s| s.trim()).collect();
            let mut start_idx = None;
            let mut end_idx = None;
            let mut style_idx = None;
            let mut text_idx = None;
            for (i, part) in format_parts.iter().enumerate() {
                match *part {
                    "Start" => start_idx = Some(i),
                    "End" => end_idx = Some(i),
                    "Style" => style_idx = Some(i),
                    "Text" => text_idx = Some(i),
                    _ => {}
                }
            }
            if let (Some(s), Some(e), Some(st), Some(t)) = (start_idx, end_idx, style_idx, text_idx)
            {
                format_indices = Some((s, e, st, t));
            }
            continue;
        }

        if in_events && line.to_lowercase().starts_with("dialogue:") {
            let dialogue_content = line[8..].trim();
            let parts: Vec<&str> = split_ass_dialogue(dialogue_content);

            let (start_idx, end_idx, style_idx, text_idx) = format_indices.unwrap_or((1, 2, 3, 9)); // 默认 ASS 格式顺序

            let start_str = parts.get(start_idx).unwrap_or(&"");
            let end_str = parts.get(end_idx).unwrap_or(&"");
            let style = parts
                .get(style_idx)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let text = parts.get(text_idx).unwrap_or(&"");

            if let (Some(start_ms), Some(end_ms)) =
                (parse_ass_timestamp(start_str), parse_ass_timestamp(end_str))
            {
                let clean_text = strip_ass_tags(text).trim().to_string();
                events.push(SubtitleEvent {
                    start_ms,
                    end_ms,
                    text: clean_text,
                    style,
                });
            }
        }

        if line.starts_with('[') && !line.eq_ignore_ascii_case("[Events]") {
            in_events = false;
        }
    }

    events.sort_by_key(|e| e.start_ms);

    Ok(SubtitleTrack {
        format: SubtitleFormat::Ass,
        events,
    })
}

/// 按逗号分割 ASS Dialogue 行, 正确处理 Text 字段内可能包含的逗号.
///
/// ASS 格式前 9 个字段不含逗号, 第 10 个 (Text) 字段可包含逗号.
fn split_ass_dialogue(line: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    const TEXT_FIELD_INDEX: usize = 9;

    for (i, c) in line.char_indices() {
        if result.len() < TEXT_FIELD_INDEX && c == ',' {
            result.push(line[start..i].trim());
            start = i + 1;
        }
    }
    result.push(line[start..].trim());

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_srt_basic() {
        let content = r#"1
00:00:01,000 --> 00:00:04,000
Hello, world!

2
00:00:05,500 --> 00:00:09,000
This is a subtitle.
Multiple lines are ok."#;
        let track = parse_srt(content).unwrap();
        assert_eq!(track.format, SubtitleFormat::Srt);
        assert_eq!(track.events.len(), 2);
        assert_eq!(track.events[0].start_ms, 1000);
        assert_eq!(track.events[0].end_ms, 4000);
        assert_eq!(track.events[0].text, "Hello, world!");
        assert_eq!(track.events[0].style, None);
        assert_eq!(track.events[1].start_ms, 5500);
        assert_eq!(track.events[1].end_ms, 9000);
        assert_eq!(
            track.events[1].text,
            "This is a subtitle.\nMultiple lines are ok."
        );
    }

    #[test]
    fn test_parse_srt_multiline() {
        let content = r#"1
00:00:00,000 --> 00:00:02,000
Line one
Line two
Line three"#;
        let track = parse_srt(content).unwrap();
        assert_eq!(track.events.len(), 1);
        assert_eq!(track.events[0].text, "Line one\nLine two\nLine three");
    }

    #[test]
    fn test_parse_srt_timestamp() {
        assert_eq!(parse_srt_timestamp("00:00:00,000"), Some(0));
        assert_eq!(parse_srt_timestamp("00:00:01,000"), Some(1000));
        assert_eq!(parse_srt_timestamp("00:01:00,500"), Some(60500));
        assert_eq!(parse_srt_timestamp("01:00:00,999"), Some(3600999));
        assert_eq!(parse_srt_timestamp("01:23:45,678"), Some(5025678));
        assert_eq!(parse_srt_timestamp("invalid"), None);
        assert_eq!(parse_srt_timestamp("00:00:01,1000"), None);
    }

    #[test]
    fn test_parse_srt_empty() {
        let track = parse_srt("").unwrap();
        assert_eq!(track.format, SubtitleFormat::Srt);
        assert!(track.events.is_empty());
    }

    #[test]
    fn test_parse_ass_basic() {
        let content = r#"[Script Info]
Title: Example
ScriptType: v4.00+

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, ...
Style: Default,Arial,20,&H00FFFFFF,...

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,Hello world!
Dialogue: 0,0:00:05.50,0:00:09.00,Default,,0,0,0,,Another line"#;
        let track = parse_ass(content).unwrap();
        assert_eq!(track.format, SubtitleFormat::Ass);
        assert_eq!(track.events.len(), 2);
        assert_eq!(track.events[0].start_ms, 1000);
        assert_eq!(track.events[0].end_ms, 4000);
        assert_eq!(track.events[0].text, "Hello world!");
        assert_eq!(track.events[1].start_ms, 5500);
        assert_eq!(track.events[1].end_ms, 9000);
        assert_eq!(track.events[1].text, "Another line");
    }

    #[test]
    fn test_parse_ass_timestamp() {
        assert_eq!(parse_ass_timestamp("0:00:00.00"), Some(0));
        assert_eq!(parse_ass_timestamp("0:00:01.00"), Some(1000));
        assert_eq!(parse_ass_timestamp("0:00:01.50"), Some(1500));
        assert_eq!(parse_ass_timestamp("0:01:00.00"), Some(60000));
        assert_eq!(parse_ass_timestamp("1:23:45.50"), Some(5025500));
        assert_eq!(parse_ass_timestamp("invalid"), None);
        assert_eq!(parse_ass_timestamp("0:00:01.100"), None);
    }

    #[test]
    fn test_parse_ass_strip_tags() {
        let content = r#"[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,{\b1}Bold{\b0} text
Dialogue: 0,0:00:02.00,0:00:04.00,Default,,0,0,0,,{\i1}Italic{\i0} here"#;
        let track = parse_ass(content).unwrap();
        assert_eq!(track.events[0].text, "Bold text");
        assert_eq!(track.events[1].text, "Italic here");
    }

    #[test]
    fn test_parse_ass_empty() {
        let track = parse_ass("").unwrap();
        assert_eq!(track.format, SubtitleFormat::Ass);
        assert!(track.events.is_empty());
    }

    #[test]
    fn test_parse_ass_with_styles() {
        let content = r#"[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Text one
Dialogue: 0,0:00:01.00,0:00:02.00,Title,,0,0,0,,Text two"#;
        let track = parse_ass(content).unwrap();
        assert_eq!(track.events[0].style, Some("Default".to_string()));
        assert_eq!(track.events[1].style, Some("Title".to_string()));
    }
}
