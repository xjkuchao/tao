//! H264 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) TAO_H264_COMPARE_INPUT=data/1_h264.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored
//! 2) TAO_H264_COMPARE_INPUT=data/2_h264.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored
//! 3) TAO_H264_COMPARE_INPUT=https://samples.ffmpeg.org/V-codecs/h264/interlaced_crop.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tao::codec::codec_parameters::{CodecParamsType, VideoCodecParams};
use tao::codec::frame::{Frame, VideoFrame};
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{MediaType, PixelFormat, Rational, TaoError};
use tao::format::stream::StreamParams;
use tao::format::{FormatRegistry, IoContext};

static FF_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const DEFAULT_COMPARE_FRAMES: usize = 120;
const DEFAULT_REQUIRED_PRECISION: f64 = 100.0;

struct SampleEntry {
    id: &'static str,
    path: &'static str,
    profile: &'static str,
    resolution: &'static str,
    description: &'static str,
}

const H264_SAMPLES: &[SampleEntry] = &[
    SampleEntry {
        id: "C1",
        path: "data/h264_samples/c1_cavlc_baseline_720p.mp4",
        profile: "Constrained Baseline",
        resolution: "1280x720",
        description: "CAVLC, 无 B 帧, Level 3.1, MP4",
    },
    SampleEntry {
        id: "C2",
        path: "data/h264_samples/c2_main_cabac_1080p.mov",
        profile: "Main",
        resolution: "1920x1080",
        description: "CABAC, B 帧, Level 4.0, MOV",
    },
    SampleEntry {
        id: "C3",
        path: "data/h264_samples/c3_high_8x8.mkv",
        profile: "High",
        resolution: "704x480",
        description: "CABAC, 8x8 变换, B 帧, MKV",
    },
    SampleEntry {
        id: "E1",
        path: "data/h264_samples/e1_baseline_cavlc_lowres.mp4",
        profile: "Baseline",
        resolution: "352x200",
        description: "CAVLC, Level 2.1, MP4",
    },
    SampleEntry {
        id: "E2",
        path: "data/h264_samples/e2_main_cabac_720p.mov",
        profile: "Main",
        resolution: "1280x720",
        description: "CABAC + B 帧, Level 3.2, MOV",
    },
    SampleEntry {
        id: "E3",
        path: "data/h264_samples/e3_main_cabac_midres.mp4",
        profile: "Main",
        resolution: "640x352",
        description: "CABAC + B 帧, Level 4.0, MP4",
    },
    SampleEntry {
        id: "E4",
        path: "data/h264_samples/e4_main_cabac_lowres.mov",
        profile: "Main",
        resolution: "480x204",
        description: "CABAC + B 帧, Level 2.0, MOV",
    },
    SampleEntry {
        id: "E5",
        path: "data/h264_samples/e5_main_1080p.264",
        profile: "Main",
        resolution: "1920x1088",
        description: "CABAC, Level 4.0, 高码率裸流",
    },
    SampleEntry {
        id: "E6",
        path: "data/h264_samples/e6_high_1080p.h264",
        profile: "High",
        resolution: "1920x1080",
        description: "CABAC + 8x8, Level 4.1, 裸流",
    },
    SampleEntry {
        id: "E7",
        path: "data/h264_samples/e7_high_1080p.mp4",
        profile: "High",
        resolution: "1920x1080",
        description: "CAVLC + 8x8, Level 4.2, yuvj420p, MP4",
    },
    SampleEntry {
        id: "E8",
        path: "data/h264_samples/e8_ipcm.h264",
        profile: "High",
        resolution: "352x288",
        description: "IPCM 宏块边界, Level 5.1, 裸流",
    },
    SampleEntry {
        id: "E9",
        path: "data/h264_samples/e9_cavlc_baseline2.mp4",
        profile: "Baseline",
        resolution: "352x200",
        description: "CAVLC, Level 3.1, MP4",
    },
];

const H264_CUSTOM_SAMPLES: &[SampleEntry] = &[
    SampleEntry {
        id: "X1",
        path: "data/h264_samples/custom_ionly.264",
        profile: "High",
        resolution: "352x288",
        description: "I-only 纯帧内, 裸流",
    },
    SampleEntry {
        id: "X2",
        path: "data/h264_samples/custom_poc1.264",
        profile: "High",
        resolution: "352x288",
        description: "B 帧, 裸流",
    },
    SampleEntry {
        id: "X3",
        path: "data/h264_samples/custom_poc2.264",
        profile: "High",
        resolution: "352x288",
        description: "P-only 无 B 帧, 裸流",
    },
    SampleEntry {
        id: "X4",
        path: "data/h264_samples/custom_multislice.264",
        profile: "High",
        resolution: "352x288",
        description: "多 slice 同帧, 裸流",
    },
];

type DecodeResult = Result<(u32, u32, Vec<Vec<u8>>, Option<u32>), Box<dyn std::error::Error>>;
type FfmpegDecodeResult = Result<(u32, u32, Vec<Vec<u8>>), Box<dyn std::error::Error>>;

#[derive(Default, Clone, Copy)]
struct PlaneStats {
    total_bytes: u64,
    equal_bytes: u64,
    sum_sq: f64,
    max_err: u8,
}

impl PlaneStats {
    fn update(&mut self, reference: &[u8], test: &[u8]) {
        for (&r, &t) in reference.iter().zip(test.iter()) {
            let diff = r.abs_diff(t);
            self.total_bytes += 1;
            if diff == 0 {
                self.equal_bytes += 1;
            }
            self.sum_sq += f64::from(diff) * f64::from(diff);
            if diff > self.max_err {
                self.max_err = diff;
            }
        }
    }

    fn mse(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.sum_sq / self.total_bytes as f64
    }

    fn psnr(&self) -> f64 {
        let mse = self.mse();
        if mse > 0.0 {
            20.0 * (255.0 / mse.sqrt()).log10()
        } else {
            f64::INFINITY
        }
    }

    fn precision_pct(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.equal_bytes as f64) * 100.0 / (self.total_bytes as f64)
    }
}

#[derive(Default, Clone, Copy)]
struct CompareStats {
    frame_count: usize,
    first_mismatch_frame: Option<usize>,
    y: PlaneStats,
    u: PlaneStats,
    v: PlaneStats,
}

impl CompareStats {
    fn global_total_bytes(&self) -> u64 {
        self.y.total_bytes + self.u.total_bytes + self.v.total_bytes
    }

    fn global_equal_bytes(&self) -> u64 {
        self.y.equal_bytes + self.u.equal_bytes + self.v.equal_bytes
    }

    fn global_sum_sq(&self) -> f64 {
        self.y.sum_sq + self.u.sum_sq + self.v.sum_sq
    }

    fn global_mse(&self) -> f64 {
        let total = self.global_total_bytes();
        if total == 0 {
            return 0.0;
        }
        self.global_sum_sq() / total as f64
    }

    fn global_psnr(&self) -> f64 {
        let mse = self.global_mse();
        if mse > 0.0 {
            20.0 * (255.0 / mse.sqrt()).log10()
        } else {
            f64::INFINITY
        }
    }

    fn global_precision_pct(&self) -> f64 {
        let total = self.global_total_bytes();
        if total == 0 {
            return 0.0;
        }
        (self.global_equal_bytes() as f64) * 100.0 / (total as f64)
    }

    fn global_max_err(&self) -> u8 {
        self.y.max_err.max(self.u.max_err).max(self.v.max_err)
    }
}

#[derive(Clone)]
struct PerFrameReport {
    frame_idx: usize,
    y_psnr: f64,
    u_psnr: f64,
    v_psnr: f64,
    y_max_err: u8,
    u_max_err: u8,
    v_max_err: u8,
    y_precision: f64,
    u_precision: f64,
    v_precision: f64,
}

impl PerFrameReport {
    fn to_json(&self) -> String {
        let fmt_psnr = |v: f64| {
            if v.is_infinite() {
                "\"Infinity\"".to_string()
            } else {
                format!("{:.4}", v)
            }
        };
        format!(
            "{{\"frame_idx\":{},\"y_psnr\":{},\"u_psnr\":{},\"v_psnr\":{},\
             \"y_max_err\":{},\"u_max_err\":{},\"v_max_err\":{},\
             \"y_precision\":{:.6},\"u_precision\":{:.6},\"v_precision\":{:.6}}}",
            self.frame_idx,
            fmt_psnr(self.y_psnr),
            fmt_psnr(self.u_psnr),
            fmt_psnr(self.v_psnr),
            self.y_max_err,
            self.u_max_err,
            self.v_max_err,
            self.y_precision,
            self.u_precision,
            self.v_precision,
        )
    }
}

fn report_enabled() -> bool {
    std::env::var("TAO_H264_COMPARE_REPORT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn compare_frames_limit() -> usize {
    std::env::var("TAO_H264_COMPARE_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_COMPARE_FRAMES)
}

fn required_precision_pct() -> f64 {
    std::env::var("TAO_H264_COMPARE_REQUIRED_PRECISION")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&v| (0.0..=100.0).contains(&v))
        .unwrap_or(DEFAULT_REQUIRED_PRECISION)
}

fn fail_on_ref_fallback_enabled() -> bool {
    std::env::var("TAO_H264_COMPARE_FAIL_ON_REF_FALLBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn make_ffmpeg_tmp_path(tag: &str) -> String {
    let pid = std::process::id();
    let seq = FF_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("data/tmp_{}_{}_{}.yuv", tag, pid, seq)
}

fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

fn open_input(path: &str) -> Result<IoContext, Box<dyn std::error::Error>> {
    if is_url(path) {
        #[cfg(feature = "http")]
        {
            return Ok(IoContext::open_url(path)?);
        }
        #[cfg(not(feature = "http"))]
        {
            return Err("当前构建未启用 http 特性, 无法读取 URL".into());
        }
    }
    Ok(IoContext::open_read(path)?)
}

fn resolve_frame_size(width: u32, height: u32) -> Result<usize, Box<dyn std::error::Error>> {
    if width == 0 || height == 0 {
        return Err("分辨率无效, 无法计算帧大小".into());
    }
    let y = (width as usize) * (height as usize);
    let uv = (width.div_ceil(2) as usize) * (height.div_ceil(2) as usize);
    Ok(y + uv * 2)
}

fn pack_plane(
    src: &[u8],
    linesize: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }
    if linesize < width {
        return Err(format!(
            "视频帧行跨度小于平面宽度: linesize={}, width={}",
            linesize, width
        )
        .into());
    }
    let need = linesize
        .checked_mul(height)
        .ok_or("视频帧行跨度乘法溢出, 无法打包平面")?;
    if src.len() < need {
        return Err(format!("视频平面数据长度不足: 实际={}, 期望>={}", src.len(), need).into());
    }
    let mut out = Vec::with_capacity(width * height);
    for row in 0..height {
        let off = row * linesize;
        out.extend_from_slice(&src[off..off + width]);
    }
    Ok(out)
}

fn pack_yuv420p(vf: &VideoFrame) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if vf.pixel_format != PixelFormat::Yuv420p {
        return Err(format!("当前对比仅支持 YUV420P, 实际像素格式={}", vf.pixel_format).into());
    }
    if vf.data.len() < 3 || vf.linesize.len() < 3 {
        return Err("视频帧平面数量不足, 无法打包 YUV420P".into());
    }

    let w = vf.width as usize;
    let h = vf.height as usize;
    let cw = vf.width.div_ceil(2) as usize;
    let ch = vf.height.div_ceil(2) as usize;

    let y = pack_plane(&vf.data[0], vf.linesize[0], w, h)?;
    let u = pack_plane(&vf.data[1], vf.linesize[1], cw, ch)?;
    let v = pack_plane(&vf.data[2], vf.linesize[2], cw, ch)?;

    let mut out = Vec::with_capacity(y.len() + u.len() + v.len());
    out.extend_from_slice(&y);
    out.extend_from_slice(&u);
    out.extend_from_slice(&v);
    Ok(out)
}

fn decode_h264_with_tao(path: &str) -> DecodeResult {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = open_input(path)?;
    let mut demuxer = match format_registry.open_input(&mut io, None) {
        Ok(d) => d,
        Err(_) => {
            io.seek(std::io::SeekFrom::Start(0))?;
            format_registry.open_input(&mut io, Some(path))?
        }
    };

    let stream = demuxer
        .streams()
        .iter()
        .find(|s| s.codec_id == CodecId::H264)
        .or_else(|| {
            demuxer
                .streams()
                .iter()
                .find(|s| s.media_type == MediaType::Video)
        })
        .ok_or("未找到可解码视频流")?
        .clone();
    let stream_index_u32 =
        u32::try_from(stream.index).map_err(|_| "流索引超出 u32 范围, 无法用于 ffmpeg 映射")?;

    if stream.codec_id != CodecId::H264 {
        println!(
            "[{}] 非 H264 视频流({}), 对比结果仅用于当前流调试",
            path, stream.codec_id
        );
    }

    let (width, height, pixel_format, frame_rate, sample_aspect_ratio) = match &stream.params {
        StreamParams::Video(v) => (
            v.width,
            v.height,
            v.pixel_format,
            v.frame_rate,
            v.sample_aspect_ratio,
        ),
        _ => (
            0,
            0,
            PixelFormat::Yuv420p,
            Rational::UNDEFINED,
            Rational::UNDEFINED,
        ),
    };

    let params = CodecParameters {
        codec_id: stream.codec_id,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Video(VideoCodecParams {
            width,
            height,
            pixel_format,
            frame_rate,
            sample_aspect_ratio,
        }),
    };

    let mut decoder = codec_registry.create_decoder(stream.codec_id)?;
    decoder.open(&params)?;

    let mut frames = Vec::<Vec<u8>>::new();
    let mut actual_w = width;
    let mut actual_h = height;
    let frame_limit = compare_frames_limit();
    let drop_negative_pts = std::env::var("TAO_H264_COMPARE_KEEP_NEGATIVE_PTS")
        .map(|v| !(v == "1" || v.eq_ignore_ascii_case("true")))
        .unwrap_or(true);
    let mut demux_eof = false;
    let mut packet_index: u64 = 0;

    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index != stream.index {
                        continue;
                    }
                    packet_index += 1;
                    decoder.send_packet(&pkt).map_err(|e| {
                        format!(
                            "发送 H264 包失败: {}, 包序号={}, pos={}, 大小={}",
                            e,
                            packet_index,
                            pkt.pos,
                            pkt.data.len()
                        )
                    })?;
                }
                Err(TaoError::Eof) => {
                    decoder.send_packet(&Packet::empty()).map_err(|e| {
                        format!("发送 H264 刷新包失败: {}, 已处理包数={}", e, packet_index)
                    })?;
                    demux_eof = true;
                }
                Err(e) => {
                    return Err(
                        format!("读取 H264 包失败: {}, 已处理包数={}", e, packet_index).into(),
                    );
                }
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Video(vf)) => {
                    if drop_negative_pts
                        && vf.pts != tao::core::timestamp::NOPTS_VALUE
                        && vf.pts < 0
                    {
                        continue;
                    }
                    actual_w = vf.width;
                    actual_h = vf.height;
                    let packed = pack_yuv420p(&vf)?;
                    let expect_size = resolve_frame_size(vf.width, vf.height)?;
                    if packed.len() != expect_size {
                        return Err(format!(
                            "打包后视频帧大小异常: 实际={}, 期望={}",
                            packed.len(),
                            expect_size
                        )
                        .into());
                    }
                    frames.push(packed);
                    if frames.len() >= frame_limit {
                        return Ok((actual_w, actual_h, frames, Some(stream_index_u32)));
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => {
                    if demux_eof {
                        return Ok((actual_w, actual_h, frames, Some(stream_index_u32)));
                    }
                    break;
                }
                Err(TaoError::Eof) => {
                    return Ok((actual_w, actual_h, frames, Some(stream_index_u32)));
                }
                Err(e) => {
                    return Err(
                        format!("接收 H264 帧失败: {}, 当前包序号={}", e, packet_index).into(),
                    );
                }
            }
        }
    }
}

struct ProbeResult {
    stream_idx: u32,
    width: u32,
    height: u32,
    is_full_range: bool,
}

fn probe_ffmpeg_video_stream(
    path: &str,
    preferred_stream: Option<u32>,
) -> Result<ProbeResult, Box<dyn std::error::Error>> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_type,width,height,pix_fmt",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()?;
    if !probe.status.success() {
        return Err("ffprobe 执行失败, 无法获取视频流信息".into());
    }

    let probe_s = String::from_utf8_lossy(&probe.stdout);
    let mut fallback: Option<ProbeResult> = None;

    for line in probe_s.lines() {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 || parts[1] != "video" {
            continue;
        }
        let idx = match parts[0].parse::<u32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let width = match parts[2].parse::<u32>() {
            Ok(v) if v > 0 => v,
            _ => continue,
        };
        let height = match parts[3].parse::<u32>() {
            Ok(v) if v > 0 => v,
            _ => continue,
        };
        let pix_fmt = if parts.len() > 4 { parts[4] } else { "" };
        let is_full_range = pix_fmt.contains("yuvj");
        let result = ProbeResult {
            stream_idx: idx,
            width,
            height,
            is_full_range,
        };
        if let Some(want_idx) = preferred_stream
            && idx == want_idx
        {
            return Ok(result);
        }
        if fallback.is_none() {
            fallback = Some(result);
        }
    }

    fallback.ok_or("ffprobe 未找到有效视频流".into())
}

fn decode_h264_with_ffmpeg(
    path: &str,
    preferred_stream: Option<u32>,
    target_size: Option<(u32, u32)>,
) -> FfmpegDecodeResult {
    let probe = probe_ffmpeg_video_stream(path, preferred_stream)?;
    let mut width = probe.width;
    let mut height = probe.height;
    if let Some((tw, th)) = target_size
        && tw > 0
        && th > 0
    {
        width = tw;
        height = th;
    }

    let frame_limit = compare_frames_limit().to_string();
    let map_spec = format!("0:{}", probe.stream_idx);
    let tmp = make_ffmpeg_tmp_path("h264_cmp");
    let skip_deblock = std::env::var("TAO_H264_COMPARE_SKIP_DEBLOCK").is_ok();
    let mut cmd = Command::new("ffmpeg");
    if skip_deblock {
        cmd.args(["-skip_loop_filter", "all"]);
    }
    let out_fmt = if probe.is_full_range {
        "yuvj420p"
    } else {
        "yuv420p"
    };
    cmd.args([
        "-y",
        "-i",
        path,
        "-map",
        &map_spec,
        "-an",
        "-sn",
        "-dn",
        "-pix_fmt",
        out_fmt,
        "-vframes",
        &frame_limit,
        "-f",
        "rawvideo",
        "-loglevel",
        "error",
        &tmp,
    ]);
    let status = cmd.status()?;
    if !status.success() {
        return Err("ffmpeg 解码失败".into());
    }

    let raw = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    let frame_size = resolve_frame_size(width, height)?;
    if frame_size == 0 {
        return Err("无效视频帧大小, 无法对比".into());
    }
    if raw.len() < frame_size {
        return Err(format!(
            "ffmpeg 输出数据过小: 实际={}, 期望>={}",
            raw.len(),
            frame_size
        )
        .into());
    }
    if !raw.len().is_multiple_of(frame_size) {
        eprintln!(
            "[H264] ffmpeg 输出长度不是帧大小整数倍: 总字节={}, 帧大小={}, 尾部字节将忽略",
            raw.len(),
            frame_size
        );
    }
    let frame_count = raw.len() / frame_size;
    let mut frames = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        let off = i * frame_size;
        frames.push(raw[off..off + frame_size].to_vec());
    }
    Ok((width, height, frames))
}

fn compare_frame(
    stats: &mut CompareStats,
    frame_idx: usize,
    reference: &[u8],
    test: &[u8],
    width: u32,
    height: u32,
) -> Result<PerFrameReport, Box<dyn std::error::Error>> {
    if reference.len() != test.len() {
        return Err(format!(
            "第 {} 帧大小不匹配: Tao={}, FFmpeg={}",
            frame_idx,
            test.len(),
            reference.len()
        )
        .into());
    }

    let y_size = (width as usize) * (height as usize);
    let uv_size = (width.div_ceil(2) as usize) * (height.div_ceil(2) as usize);
    let expect_size = y_size + uv_size * 2;
    if reference.len() < expect_size {
        return Err(format!(
            "第 {} 帧数据过小: 实际={}, 期望>={}",
            frame_idx,
            reference.len(),
            expect_size
        )
        .into());
    }

    let y_ref = &reference[..y_size];
    let u_ref = &reference[y_size..y_size + uv_size];
    let v_ref = &reference[y_size + uv_size..y_size + uv_size * 2];

    let y_test = &test[..y_size];
    let u_test = &test[y_size..y_size + uv_size];
    let v_test = &test[y_size + uv_size..y_size + uv_size * 2];

    let mut frame_y = PlaneStats::default();
    let mut frame_u = PlaneStats::default();
    let mut frame_v = PlaneStats::default();
    frame_y.update(y_ref, y_test);
    frame_u.update(u_ref, u_test);
    frame_v.update(v_ref, v_test);

    stats.y.update(y_ref, y_test);
    stats.u.update(u_ref, u_test);
    stats.v.update(v_ref, v_test);
    stats.frame_count += 1;

    if stats.first_mismatch_frame.is_none()
        && (!y_ref.iter().zip(y_test).all(|(a, b)| a == b)
            || !u_ref.iter().zip(u_test).all(|(a, b)| a == b)
            || !v_ref.iter().zip(v_test).all(|(a, b)| a == b))
    {
        stats.first_mismatch_frame = Some(frame_idx);
    }

    Ok(PerFrameReport {
        frame_idx,
        y_psnr: frame_y.psnr(),
        u_psnr: frame_u.psnr(),
        v_psnr: frame_v.psnr(),
        y_max_err: frame_y.max_err,
        u_max_err: frame_u.max_err,
        v_max_err: frame_v.max_err,
        y_precision: frame_y.precision_pct(),
        u_precision: frame_u.precision_pct(),
        v_precision: frame_v.precision_pct(),
    })
}

fn compare_video(
    width: u32,
    height: u32,
    reference: &[Vec<u8>],
    test: &[Vec<u8>],
) -> Result<(CompareStats, Vec<PerFrameReport>), Box<dyn std::error::Error>> {
    let mut stats = CompareStats::default();
    let frame_count = reference.len().min(test.len());
    let mut reports = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        let report = compare_frame(&mut stats, i, &reference[i], &test[i], width, height)?;
        reports.push(report);
    }
    Ok((stats, reports))
}

fn analyze_shift_enabled() -> bool {
    std::env::var("TAO_H264_COMPARE_ANALYZE_SHIFT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn analyze_frame_stats_enabled() -> bool {
    std::env::var("TAO_H264_COMPARE_ANALYZE_FRAME_STATS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn print_first_frame_stats(path: &str, width: u32, height: u32, ff: &[u8], tao: &[u8]) {
    let y_size = (width as usize) * (height as usize);
    let y_ff = &ff[..y_size.min(ff.len())];
    let y_tao = &tao[..y_size.min(tao.len())];
    let ff_128 = y_ff.iter().filter(|&&v| v == 128).count();
    let tao_128 = y_tao.iter().filter(|&&v| v == 128).count();
    let ff_mean = if y_ff.is_empty() {
        0.0
    } else {
        y_ff.iter().map(|&v| v as u64).sum::<u64>() as f64 / y_ff.len() as f64
    };
    let tao_mean = if y_tao.is_empty() {
        0.0
    } else {
        y_tao.iter().map(|&v| v as u64).sum::<u64>() as f64 / y_tao.len() as f64
    };
    println!(
        "[{}] 首帧Y统计: FFmpeg(mean={:.3}, v128={}/{}), Tao(mean={:.3}, v128={}/{})",
        path,
        ff_mean,
        ff_128,
        y_ff.len(),
        tao_mean,
        tao_128,
        y_tao.len()
    );

    let w = width as usize;
    let dump_rows = 4.min(height as usize);
    let dump_cols = 128.min(w);
    println!("[{}] 首帧Y像素(前{}行x{}列):", path, dump_rows, dump_cols);
    for row in 0..dump_rows {
        let off = row * w;
        let ff_row: Vec<u8> = y_ff[off..off + dump_cols].to_vec();
        let tao_row: Vec<u8> = y_tao[off..off + dump_cols].to_vec();
        println!("  行{}: FF={:?}", row, ff_row);
        println!("  行{}: Tao={:?}", row, tao_row);
    }

    // 找到第一个不匹配像素
    let mut first_mismatch = None;
    let mut max_err_pos = (0usize, 0usize, 0i32);
    let mut err_hist = [0u64; 8]; // [0, 1, 2-3, 4-7, 8-15, 16-31, 32-63, 64+]
    for i in 0..y_ff.len().min(y_tao.len()) {
        let diff = (y_ff[i] as i32 - y_tao[i] as i32).abs();
        if diff > 0 && first_mismatch.is_none() {
            first_mismatch = Some((i % w, i / w, y_ff[i], y_tao[i], diff));
        }
        if diff > max_err_pos.2 {
            max_err_pos = (i % w, i / w, diff);
        }
        let bucket = if diff == 0 {
            0
        } else if diff == 1 {
            1
        } else if diff <= 3 {
            2
        } else if diff <= 7 {
            3
        } else if diff <= 15 {
            4
        } else if diff <= 31 {
            5
        } else if diff <= 63 {
            6
        } else {
            7
        };
        err_hist[bucket] += 1;
    }
    if let Some((x, y, ff_v, tao_v, d)) = first_mismatch {
        println!(
            "[{}] 首个不匹配: ({},{}) FF={} Tao={} diff={}",
            path, x, y, ff_v, tao_v, d
        );
    }
    println!(
        "[{}] 最大误差位置: ({},{}) diff={}",
        path, max_err_pos.0, max_err_pos.1, max_err_pos.2
    );
    println!(
        "[{}] 误差分布: 0:{} 1:{} 2-3:{} 4-7:{} 8-15:{} 16-31:{} 32-63:{} 64+:{}",
        path,
        err_hist[0],
        err_hist[1],
        err_hist[2],
        err_hist[3],
        err_hist[4],
        err_hist[5],
        err_hist[6],
        err_hist[7]
    );

    // 找到首个大误差 (>=10) 和极大误差 (>=64) 的位置
    let mut first_big = None;
    let mut first_huge = None;
    for i in 0..y_ff.len().min(y_tao.len()) {
        let diff = (y_ff[i] as i32 - y_tao[i] as i32).abs();
        if diff >= 10 && first_big.is_none() {
            first_big = Some((i % w, i / w, y_ff[i], y_tao[i], diff));
        }
        if diff >= 64 && first_huge.is_none() {
            first_huge = Some((i % w, i / w, y_ff[i], y_tao[i], diff));
        }
        if first_big.is_some() && first_huge.is_some() {
            break;
        }
    }
    if let Some((x, y, fv, tv, d)) = first_big {
        let mb_x = x / 16;
        let mb_y = y / 16;
        println!(
            "[{}] 首个大误差(>=10): ({},{}) MB({},{}) FF={} Tao={} diff={}",
            path, x, y, mb_x, mb_y, fv, tv, d
        );
    }
    if let Some((x, y, fv, tv, d)) = first_huge {
        let mb_x = x / 16;
        let mb_y = y / 16;
        println!(
            "[{}] 首个极大误差(>=64): ({},{}) MB({},{}) FF={} Tao={} diff={}",
            path, x, y, mb_x, mb_y, fv, tv, d
        );
    }

    // 显示第一个不匹配位置附近的宏块
    if let Some((x, y, _, _, _)) = first_mismatch {
        let mb_x = x / 16;
        let mb_y = y / 16;
        let start_row = mb_y * 16;
        let start_col = mb_x * 16;
        println!(
            "[{}] 首个不匹配宏块({},{}), 像素({},{})附近:",
            path, mb_x, mb_y, x, y
        );
        for r in start_row..(start_row + 4).min(height as usize) {
            let off = r * w + start_col;
            let end = (off + 16).min(y_ff.len());
            let ff_row: Vec<u8> = y_ff[off..end].to_vec();
            let tao_row: Vec<u8> = y_tao[off..end].to_vec();
            println!("  行{}: FF={:?}", r, ff_row);
            println!("  行{}: Tao={:?}", r, tao_row);
        }
    }

    if std::env::var("TAO_H264_COMPARE_LOCATE_ERROR_MB").as_deref() == Ok("1") {
        let mb_rows = (height as usize + 15) / 16;
        let mb_cols = (width as usize + 15) / 16;
        let mut found = false;
        'outer: for mb_row in 0..mb_rows {
            for mb_col in 0..mb_cols {
                let mut max_d = 0i32;
                for dy in 0..16 {
                    let py = mb_row * 16 + dy;
                    if py >= height as usize {
                        break;
                    }
                    for dx in 0..16 {
                        let px = mb_col * 16 + dx;
                        if px >= w {
                            break;
                        }
                        let i = py * w + px;
                        if i < y_ff.len() && i < y_tao.len() {
                            let d = (y_ff[i] as i32 - y_tao[i] as i32).abs();
                            if d > max_d {
                                max_d = d;
                            }
                        }
                    }
                }
                if max_d >= 1 && !found {
                    println!(
                        "[{}] 首个大误差MB: ({},{}) max_d={}",
                        path, mb_col, mb_row, max_d
                    );
                    for dy in 0..8 {
                        let py = mb_row * 16 + dy;
                        if py >= height as usize {
                            break;
                        }
                        let off = py * w + mb_col * 16;
                        let end = (off + 16).min(y_ff.len());
                        let diff: Vec<i32> = y_ff[off..end]
                            .iter()
                            .zip(y_tao[off..end].iter())
                            .map(|(&a, &b)| a as i32 - b as i32)
                            .collect();
                        println!("  dy{}: diff={:?}", dy, diff);
                    }
                    found = true;
                    break 'outer;
                }
            }
        }
    }
}

/// 估计参考序列与测试序列的最佳帧偏移.
/// 返回 `(最佳偏移, 对齐后的逐字节相等率百分比)`.
fn estimate_best_shift(reference: &[Vec<u8>], test: &[Vec<u8>], max_shift: isize) -> (isize, f64) {
    let mut best_shift = 0isize;
    let mut best_precision = -1.0f64;

    for shift in -max_shift..=max_shift {
        let mut equal = 0u64;
        let mut total = 0u64;
        for (i, r) in reference.iter().enumerate() {
            let j = i as isize + shift;
            if j < 0 {
                continue;
            }
            let j = j as usize;
            if j >= test.len() {
                continue;
            }
            let t = &test[j];
            let n = r.len().min(t.len());
            for k in 0..n {
                total += 1;
                if r[k] == t[k] {
                    equal += 1;
                }
            }
        }
        if total == 0 {
            continue;
        }
        let precision = equal as f64 * 100.0 / total as f64;
        if precision > best_precision {
            best_precision = precision;
            best_shift = shift;
        }
    }
    (best_shift, best_precision.max(0.0))
}

fn resolve_input() -> Result<String, Box<dyn std::error::Error>> {
    let mut after_dd = std::env::args().skip_while(|v| v != "--").skip(1);
    if let Some(arg) = after_dd.next() {
        return Ok(arg);
    }
    if let Ok(env) = std::env::var("TAO_H264_COMPARE_INPUT")
        && !env.trim().is_empty()
    {
        return Ok(env);
    }
    Err("请通过参数或 TAO_H264_COMPARE_INPUT 指定 MP4/H264 文件或 URL".into())
}

fn print_mb_error_map(path: &str, w: u32, h: u32, ref_frame: &[u8], tao_frame: &[u8]) {
    let w = w as usize;
    let h = h as usize;
    let y_size = w * h;
    if ref_frame.len() < y_size || tao_frame.len() < y_size {
        return;
    }
    let mb_w = (w + 15) / 16;
    let mb_h = (h + 15) / 16;
    let mut worst_mb = (0usize, 0usize, 0u32, 0i32);
    println!("[{}] 宏块误差图 (帧0, Y平面, {}x{} MBs):", path, mb_w, mb_h);
    for mby in 0..mb_h {
        for mbx in 0..mb_w {
            let mut max_err: u32 = 0;
            let mut sum_diff: i64 = 0;
            let mut cnt = 0u32;
            for dy in 0..16 {
                let y = mby * 16 + dy;
                if y >= h {
                    break;
                }
                for dx in 0..16 {
                    let x = mbx * 16 + dx;
                    if x >= w {
                        break;
                    }
                    let idx = y * w + x;
                    let diff = ref_frame[idx] as i32 - tao_frame[idx] as i32;
                    max_err = max_err.max(diff.unsigned_abs());
                    sum_diff += diff as i64;
                    cnt += 1;
                }
            }
            if max_err > worst_mb.2 {
                worst_mb = (mbx, mby, max_err, (sum_diff / cnt as i64) as i32);
            }
            if max_err > 10 {
                let avg = sum_diff as f64 / cnt as f64;
                println!(
                    "  MB({},{}) max_err={} avg_diff={:.1}",
                    mbx, mby, max_err, avg
                );
            }
        }
    }
    println!(
        "  最差MB({},{}) max_err={} avg_diff={}",
        worst_mb.0, worst_mb.1, worst_mb.2, worst_mb.3
    );
    let mut detail_mbs: Vec<(usize, usize, u32, i32)> = Vec::new();
    for mby in 0..mb_h {
        for mbx in 0..mb_w {
            let mut max_err: u32 = 0;
            let mut sum_diff: i64 = 0;
            let mut cnt = 0u32;
            for dy in 0..16 {
                let y = mby * 16 + dy;
                if y >= h {
                    break;
                }
                for dx in 0..16 {
                    let x = mbx * 16 + dx;
                    if x >= w {
                        break;
                    }
                    let idx = y * w + x;
                    let diff = ref_frame[idx] as i32 - tao_frame[idx] as i32;
                    max_err = max_err.max(diff.unsigned_abs());
                    sum_diff += diff as i64;
                    cnt += 1;
                }
            }
            if max_err >= 30 {
                detail_mbs.push((mbx, mby, max_err, (sum_diff / cnt as i64) as i32));
            }
        }
    }
    for &(mbx, mby, me, _) in &detail_mbs {
        let mut sub_errs = Vec::new();
        for sby in 0..4 {
            for sbx in 0..4 {
                let mut se: i64 = 0;
                let mut sc = 0u32;
                for dy in 0..4 {
                    let y = mby * 16 + sby * 4 + dy;
                    if y >= h {
                        break;
                    }
                    for dx in 0..4 {
                        let x = mbx * 16 + sbx * 4 + dx;
                        if x >= w {
                            break;
                        }
                        let idx = y * w + x;
                        se += (ref_frame[idx] as i32 - tao_frame[idx] as i32) as i64;
                        sc += 1;
                    }
                }
                if sc > 0 {
                    sub_errs.push(se as f64 / sc as f64);
                }
            }
        }
        println!(
            "  MB({},{}) max_err={} 4x4子块avg_diff={:?}",
            mbx,
            mby,
            me,
            sub_errs
                .iter()
                .map(|v| format!("{:.0}", v))
                .collect::<Vec<_>>()
        );
    }
}

fn print_compare_stats(path: &str, tao_frames: usize, ff_frames: usize, stats: &CompareStats) {
    println!(
        "[{}] Tao对比帧={}, Tao={}, FFmpeg={}, Tao/FFmpeg: max_err={}, psnr={:.4}dB, 精度={:.6}%, FFmpeg=100%",
        path,
        stats.frame_count,
        tao_frames,
        ff_frames,
        stats.global_max_err(),
        stats.global_psnr(),
        stats.global_precision_pct(),
    );
    println!(
        "[{}] 平面Y: max_err={}, psnr={:.4}dB, 精度={:.6}%",
        path,
        stats.y.max_err,
        stats.y.psnr(),
        stats.y.precision_pct(),
    );
    println!(
        "[{}] 平面U: max_err={}, psnr={:.4}dB, 精度={:.6}%",
        path,
        stats.u.max_err,
        stats.u.psnr(),
        stats.u.precision_pct(),
    );
    println!(
        "[{}] 平面V: max_err={}, psnr={:.4}dB, 精度={:.6}%",
        path,
        stats.v.max_err,
        stats.v.psnr(),
        stats.v.precision_pct(),
    );
    if let Some(frame_idx) = stats.first_mismatch_frame {
        eprintln!("[{}] 首个不一致帧索引={}", path, frame_idx);
    }
}

fn run_compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if fail_on_ref_fallback_enabled() {
        println!("[{}] 已启用缺失参考回退硬失败门禁", path);
    }
    let (tao_w, tao_h, tao_frames, tao_stream_index) = decode_h264_with_tao(path)?;

    if std::env::var("TAO_H264_COMPARE_DUMP_TAO").unwrap_or_default() == "1"
        && !tao_frames.is_empty()
    {
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("frame");
        let dump_path = format!("data/tao_{}_frame0.yuv", stem);
        std::fs::write(&dump_path, &tao_frames[0]).ok();
        eprintln!(
            "[诊断] Tao 帧0 已写入 {} ({} 字节)",
            dump_path,
            tao_frames[0].len()
        );
    }

    let (ff_w, ff_h, ff_frames) =
        decode_h264_with_ffmpeg(path, tao_stream_index, Some((tao_w, tao_h)))?;

    if tao_w != ff_w || tao_h != ff_h {
        return Err(format!(
            "H264 对比失败: 分辨率不匹配, Tao={}x{}, FFmpeg={}x{}",
            tao_w, tao_h, ff_w, ff_h
        )
        .into());
    }
    if tao_frames.is_empty() || ff_frames.is_empty() {
        return Err(format!(
            "H264 对比失败: 无可比较帧, Tao={}, FFmpeg={}",
            tao_frames.len(),
            ff_frames.len()
        )
        .into());
    }
    if tao_frames.len() != ff_frames.len() {
        eprintln!(
            "[H264] 帧数差异: Tao={}, FFmpeg={}, 将按最小帧数对比",
            tao_frames.len(),
            ff_frames.len()
        );
    }

    if analyze_frame_stats_enabled() && !ff_frames.is_empty() && !tao_frames.is_empty() {
        print_first_frame_stats(path, tao_w, tao_h, &ff_frames[0], &tao_frames[0]);

        // 检查帧对齐: Tao 帧 0 与 FFmpeg 各帧的精度
        let y_size = (tao_w as usize) * (tao_h as usize);
        if ff_frames.len() >= 3 && tao_frames.len() >= 2 {
            for shift in [-1i32, 0, 1] {
                let mut total_eq = 0u64;
                let mut total_cnt = 0u64;
                let n = ff_frames.len().min(tao_frames.len());
                for fi in 0..n {
                    let fj = fi as i32 + shift;
                    if fj < 0 || fj as usize >= ff_frames.len() {
                        continue;
                    }
                    let sz = ff_frames[fj as usize].len().min(tao_frames[fi].len());
                    let eq = (0..sz)
                        .filter(|&i| ff_frames[fj as usize][i] == tao_frames[fi][i])
                        .count();
                    total_eq += eq as u64;
                    total_cnt += sz as u64;
                }
                if total_cnt > 0 {
                    println!(
                        "[{}] 帧偏移测试 shift={}: 精度={:.2}%",
                        path,
                        shift,
                        total_eq as f64 * 100.0 / total_cnt as f64
                    );
                }
            }
        }

        let n = ff_frames.len().min(tao_frames.len());
        for fi in 0..n {
            let sz = ff_frames[fi].len().min(tao_frames[fi].len());
            let eq = (0..sz)
                .filter(|&i| ff_frames[fi][i] == tao_frames[fi][i])
                .count();
            let max_e = (0..sz)
                .map(|i| (ff_frames[fi][i] as i32 - tao_frames[fi][i] as i32).unsigned_abs())
                .max()
                .unwrap_or(0);
            let ff_y_mean = if y_size <= ff_frames[fi].len() {
                ff_frames[fi][..y_size]
                    .iter()
                    .map(|&v| v as f64)
                    .sum::<f64>()
                    / y_size as f64
            } else {
                0.0
            };
            let tao_y_mean = if y_size <= tao_frames[fi].len() {
                tao_frames[fi][..y_size]
                    .iter()
                    .map(|&v| v as f64)
                    .sum::<f64>()
                    / y_size as f64
            } else {
                0.0
            };
            let ff_pix0 = ff_frames[fi].get(0).copied().unwrap_or(0);
            let tao_pix0 = tao_frames[fi].get(0).copied().unwrap_or(0);
            println!(
                "[{}] 帧{}: 精度={:.2}%, max_err={}, FF_Y均值={:.1} Tao_Y均值={:.1}, pix0: FF={} Tao={}",
                path,
                fi,
                eq as f64 * 100.0 / sz as f64,
                max_e,
                ff_y_mean,
                tao_y_mean,
                ff_pix0,
                tao_pix0
            );
        }
    }

    if analyze_shift_enabled() {
        let max_shift = 8isize;
        let (shift, precision) = estimate_best_shift(&ff_frames, &tao_frames, max_shift);
        println!(
            "[{}] 偏移诊断: shift={}, 对齐精度={:.6}% (搜索范围=±{})",
            path, shift, precision, max_shift
        );
    }

    if std::env::var("TAO_H264_COMPARE_MB_DIAG").unwrap_or_default() == "1"
        && !ff_frames.is_empty()
        && !tao_frames.is_empty()
    {
        print_mb_error_map(path, tao_w, tao_h, &ff_frames[0], &tao_frames[0]);
    }

    let (stats, per_frame_reports) = compare_video(tao_w, tao_h, &ff_frames, &tao_frames)?;
    print_compare_stats(path, tao_frames.len(), ff_frames.len(), &stats);

    if report_enabled() && !per_frame_reports.is_empty() {
        write_per_frame_report(path, &per_frame_reports);
    }

    let required_precision = required_precision_pct();
    if required_precision >= 100.0 {
        if stats.global_equal_bytes() != stats.global_total_bytes() {
            return Err(format!(
                "H264 对比失败: 精度要求 100%, 当前={:.6}%",
                stats.global_precision_pct()
            )
            .into());
        }
    } else if stats.global_precision_pct() + f64::EPSILON < required_precision {
        return Err(format!(
            "H264 对比失败: 精度不足 {:.2}%, 当前={:.6}%",
            required_precision,
            stats.global_precision_pct()
        )
        .into());
    }
    if tao_frames.len() != ff_frames.len() {
        return Err(format!(
            "H264 对比失败: 帧数不一致, Tao={}, FFmpeg={}",
            tao_frames.len(),
            ff_frames.len()
        )
        .into());
    }

    Ok(())
}

fn write_per_frame_report(path: &str, reports: &[PerFrameReport]) {
    let report_dir = Path::new("data/h264_compare_reports");
    if let Err(e) = std::fs::create_dir_all(report_dir) {
        eprintln!("[报告] 创建报告目录失败: {}", e);
        return;
    }

    let sample_name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = format!("{}_{}.json", sample_name, ts);
    let out_path = report_dir.join(&filename);

    let json_items: Vec<String> = reports.iter().map(|r| r.to_json()).collect();
    let json = format!("[\n  {}\n]", json_items.join(",\n  "));

    match std::fs::File::create(&out_path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(json.as_bytes()) {
                eprintln!("[报告] 写入失败: {}", e);
            } else {
                println!("[报告] 逐帧报告已写入: {}", out_path.display());
            }
        }
        Err(e) => eprintln!("[报告] 创建文件失败: {}", e),
    }
}

/// 以指定参数执行精度对比, 供精度回归测试使用.
fn run_compare_with_params(
    url: &str,
    max_frames: usize,
    min_precision: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: 集成测试单线程运行, set_var 不会与其他线程竞争.
    unsafe {
        std::env::set_var("TAO_H264_COMPARE_FRAMES", max_frames.to_string());
        std::env::set_var(
            "TAO_H264_COMPARE_REQUIRED_PRECISION",
            min_precision.to_string(),
        );
    }
    run_compare(url)
}

#[test]
#[ignore]
fn test_h264_compare() {
    let input = resolve_input().expect("缺少对比输入参数");
    run_compare(&input).expect("H264 对比失败");
}

#[test]
#[ignore]
fn test_h264_compare_sample_1() {
    let path = "data/1_h264.mp4";
    assert!(Path::new(path).exists(), "样本不存在: {}", path);
    run_compare(path).expect("样本1 H264 对比失败");
}

#[test]
#[ignore]
fn test_h264_compare_sample_2() {
    let path = "data/2_h264.mp4";
    assert!(Path::new(path).exists(), "样本不存在: {}", path);
    run_compare(path).expect("样本2 H264 对比失败");
}

/// C1 样本精度回归: CAVLC Baseline 720p (MP4)
#[test]
#[ignore]
fn test_h264_accuracy_c1() {
    let s = &H264_SAMPLES[0];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// C2 样本精度回归: Main + CABAC + B 帧 1080p (MOV)
#[test]
#[ignore]
fn test_h264_accuracy_c2() {
    let s = &H264_SAMPLES[1];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// C3 样本精度回归: High + 8x8 + CABAC (MKV)
#[test]
#[ignore]
fn test_h264_accuracy_c3() {
    let s = &H264_SAMPLES[2];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E1 样本精度回归: Baseline + CAVLC 低分辨率 (MP4)
#[test]
#[ignore]
fn test_h264_accuracy_e1() {
    let s = &H264_SAMPLES[3];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E2 样本精度回归: Main + CABAC 720p (MOV)
#[test]
#[ignore]
fn test_h264_accuracy_e2() {
    let s = &H264_SAMPLES[4];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E3 样本精度回归: Main + CABAC 中分辨率 (MP4)
#[test]
#[ignore]
fn test_h264_accuracy_e3() {
    let s = &H264_SAMPLES[5];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E4 样本精度回归: Main + CABAC 低分辨率 (MOV)
#[test]
#[ignore]
fn test_h264_accuracy_e4() {
    let s = &H264_SAMPLES[6];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E5 样本精度回归: Main + 1080p 裸流
#[test]
#[ignore]
fn test_h264_accuracy_e5() {
    let s = &H264_SAMPLES[7];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E6 样本精度回归: High + 1080p 裸流
#[test]
#[ignore]
fn test_h264_accuracy_e6() {
    let s = &H264_SAMPLES[8];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E7 样本精度回归: High + 1080p (MP4, yuvj420p)
#[test]
#[ignore]
fn test_h264_accuracy_e7() {
    let s = &H264_SAMPLES[9];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E8 样本精度回归: IPCM 边界 (裸流)
#[test]
#[ignore]
fn test_h264_accuracy_e8() {
    let s = &H264_SAMPLES[10];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// E9 样本精度回归: CAVLC Baseline 低分辨率 2 (MP4)
#[test]
#[ignore]
fn test_h264_accuracy_e9() {
    let s = &H264_SAMPLES[11];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// X1 自制样本: I-only 纯帧内 (裸流)
#[test]
#[ignore]
fn test_h264_accuracy_x1() {
    let s = &H264_CUSTOM_SAMPLES[0];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// X2 自制样本: B 帧 (裸流)
#[test]
#[ignore]
fn test_h264_accuracy_x2() {
    let s = &H264_CUSTOM_SAMPLES[1];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// X3 自制样本: P-only 无 B 帧 (裸流)
#[test]
#[ignore]
fn test_h264_accuracy_x3() {
    let s = &H264_CUSTOM_SAMPLES[2];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// X4 自制样本: 多 slice 同帧 (裸流)
#[test]
#[ignore]
fn test_h264_accuracy_x4() {
    let s = &H264_CUSTOM_SAMPLES[3];
    assert!(Path::new(s.path).exists(), "样本不存在: {}", s.path);
    run_compare_with_params(s.path, 10, 1.0)
        .unwrap_or_else(|e| panic!("{} ({}) 精度回归失败: {}", s.id, s.description, e));
}

/// 批量对比全部样本, 输出汇总报告.
/// 不因单个样本失败而中断, 最终汇总所有结果.
#[test]
#[ignore]
fn test_h264_accuracy_all() {
    let all_samples: Vec<&SampleEntry> = H264_SAMPLES
        .iter()
        .chain(H264_CUSTOM_SAMPLES.iter())
        .collect();
    let total = all_samples.len();
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut skip = 0usize;
    let mut failures = Vec::<String>::new();

    for s in &all_samples {
        if !Path::new(s.path).exists() {
            println!("[{}] 跳过: 样本不存在 {}", s.id, s.path);
            skip += 1;
            continue;
        }
        print!(
            "[{}] {} {} ({}) ... ",
            s.id, s.resolution, s.profile, s.description
        );
        match run_compare_with_params(s.path, 10, 1.0) {
            Ok(()) => {
                println!("通过");
                pass += 1;
            }
            Err(e) => {
                println!("失败: {}", e);
                failures.push(format!("{} ({}): {}", s.id, s.path, e));
                fail += 1;
            }
        }
    }

    println!("\n=== H264 精度回归汇总 ===");
    println!(
        "通过: {}, 失败: {}, 跳过: {}, 总计: {}",
        pass, fail, skip, total
    );

    if !failures.is_empty() {
        println!("\n失败详情:");
        for f in &failures {
            println!("  - {}", f);
        }
        panic!("H264 精度回归: {}/{} 样本失败", fail, total);
    }
    if skip > 0 {
        println!("警告: {} 个样本被跳过(文件不存在)", skip);
    }
}
