use tao_codec::CodecId;
use tao_core::{Rational, SampleFormat};
use tao_filter::FilterGraph;

#[derive(Debug, Clone)]
pub(crate) struct FilterSpec {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

/// 解析滤镜链字符串 (如 "volume=0.5,fade=in:0:3")
pub(crate) fn parse_filter_chain(chain: &str) -> Vec<FilterSpec> {
    chain
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|filter_str| {
            let filter_str = filter_str.trim();
            if let Some(eq_pos) = filter_str.find('=') {
                let name = filter_str[..eq_pos].trim().to_string();
                let args: Vec<String> = filter_str[eq_pos + 1..]
                    .split(':')
                    .map(|s| s.trim().to_string())
                    .collect();
                FilterSpec { name, args }
            } else {
                FilterSpec {
                    name: filter_str.to_string(),
                    args: Vec::new(),
                }
            }
        })
        .collect()
}

/// 构建音频滤镜图
pub(crate) fn build_audio_filter_graph(filters: &Option<Vec<FilterSpec>>) -> Option<FilterGraph> {
    let specs = filters.as_ref()?;
    if specs.is_empty() {
        return None;
    }

    let mut graph = FilterGraph::new();

    for spec in specs {
        match spec.name.as_str() {
            "volume" => {
                let gain: f64 = spec
                    .args
                    .first()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1.0);
                let filter = tao_filter::filters::volume::VolumeFilter::new(gain);
                graph.add_filter(Box::new(filter));
                eprintln!("  [af] volume: gain={gain}");
            }
            "fade" => {
                // fade=in:start_sec:duration_sec 或 fade=out:start_sec:duration_sec
                let fade_type = spec.args.first().map(|s| s.as_str()).unwrap_or("in");
                let start: f64 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let dur: f64 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
                let ft = if fade_type == "out" {
                    tao_filter::filters::fade::FadeType::Out
                } else {
                    tao_filter::filters::fade::FadeType::In
                };
                let filter = tao_filter::filters::fade::FadeFilter::new(ft, start, dur);
                graph.add_filter(Box::new(filter));
                eprintln!("  [af] fade: type={fade_type}, start={start}s, duration={dur}s");
            }
            other => {
                eprintln!("  [af] 未知滤镜: {other}, 跳过");
            }
        }
    }

    if graph.filter_names().is_empty() {
        None
    } else {
        Some(graph)
    }
}

/// 构建视频滤镜图
pub(crate) fn build_video_filter_graph(filters: &Option<Vec<FilterSpec>>) -> Option<FilterGraph> {
    let specs = filters.as_ref()?;
    if specs.is_empty() {
        return None;
    }

    let mut graph = FilterGraph::new();

    for spec in specs {
        match spec.name.as_str() {
            "crop" => {
                // crop=width:height:x:y
                let w: u32 = spec.args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let h: u32 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let x: u32 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: u32 = spec.args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                if w > 0 && h > 0 {
                    let filter = tao_filter::filters::crop::CropFilter::new(x, y, w, h);
                    graph.add_filter(Box::new(filter));
                    eprintln!("  [vf] crop: {w}x{h}+{x}+{y}");
                }
            }
            "pad" => {
                // pad=width:height:x:y:color (color 可选)
                let w: u32 = spec.args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let h: u32 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let x: u32 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y: u32 = spec.args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                if w > 0 && h > 0 {
                    let filter = tao_filter::filters::pad::PadFilter::new(w, h, x, y);
                    graph.add_filter(Box::new(filter));
                    eprintln!("  [vf] pad: {w}x{h}+{x}+{y}");
                }
            }
            "fade" => {
                let fade_type = spec.args.first().map(|s| s.as_str()).unwrap_or("in");
                let start: f64 = spec.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let dur: f64 = spec.args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
                let ft = if fade_type == "out" {
                    tao_filter::filters::fade::FadeType::Out
                } else {
                    tao_filter::filters::fade::FadeType::In
                };
                let filter = tao_filter::filters::fade::FadeFilter::new(ft, start, dur);
                graph.add_filter(Box::new(filter));
                eprintln!("  [vf] fade: type={fade_type}, start={start}s, duration={dur}s");
            }
            other => {
                eprintln!("  [vf] 未知滤镜: {other}, 跳过");
            }
        }
    }

    if graph.filter_names().is_empty() {
        None
    } else {
        Some(graph)
    }
}

// ============================================================
// 解析辅助
// ============================================================

/// 解析分辨率字符串 (如 "1280x720")
pub(crate) fn parse_size(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().ok()?;
        let h = parts[1].parse().ok()?;
        Some((w, h))
    } else {
        None
    }
}

/// 解析帧率字符串 (如 "25" 或 "30000/1001")
pub(crate) fn parse_rate(s: &str) -> Option<Rational> {
    if let Some(slash) = s.find('/') {
        let num: i32 = s[..slash].parse().ok()?;
        let den: i32 = s[slash + 1..].parse().ok()?;
        Some(Rational::new(num, den))
    } else {
        let fps: f64 = s.parse().ok()?;
        if fps > 0.0 {
            Some(Rational::new((fps * 1000.0) as i32, 1000))
        } else {
            None
        }
    }
}

/// PTS 转秒
pub(crate) fn pts_to_sec(pts: i64, num: i32, den: i32) -> f64 {
    if den == 0 {
        return 0.0;
    }
    pts as f64 * num as f64 / den as f64
}

/// 根据 CodecId 获取对应的采样格式
pub(crate) fn codec_id_to_sample_format(codec_id: CodecId) -> Option<SampleFormat> {
    match codec_id {
        CodecId::PcmU8 => Some(SampleFormat::U8),
        CodecId::PcmS16le | CodecId::PcmS16be => Some(SampleFormat::S16),
        CodecId::PcmS24le => Some(SampleFormat::S32),
        CodecId::PcmS32le => Some(SampleFormat::S32),
        CodecId::PcmF32le => Some(SampleFormat::F32),
        CodecId::Aac => Some(SampleFormat::F32),
        CodecId::Flac => Some(SampleFormat::S16),
        _ => None,
    }
}

/// 解析编解码器名称为 CodecId
pub(crate) fn parse_codec_name(name: &str) -> CodecId {
    match name.to_lowercase().as_str() {
        "pcm_u8" => CodecId::PcmU8,
        "pcm_s16le" => CodecId::PcmS16le,
        "pcm_s16be" => CodecId::PcmS16be,
        "pcm_s24le" => CodecId::PcmS24le,
        "pcm_s32le" => CodecId::PcmS32le,
        "pcm_f32le" => CodecId::PcmF32le,
        "rawvideo" => CodecId::RawVideo,
        "aac" => CodecId::Aac,
        "flac" => CodecId::Flac,
        "mp3" => CodecId::Mp3,
        other => {
            eprintln!("警告: 未知编解码器 '{other}', 使用默认");
            CodecId::PcmS16le
        }
    }
}
