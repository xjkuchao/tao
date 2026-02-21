//! H264 解码精度对比测试.
//!
//! 手动执行示例:
//! 1) TAO_H264_COMPARE_INPUT=data/1_h264.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored
//! 2) TAO_H264_COMPARE_INPUT=data/2_h264.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored
//! 3) TAO_H264_COMPARE_INPUT=https://samples.ffmpeg.org/V-codecs/h264/interlaced_crop.mp4 cargo test --test run_decoder h264:: -- --nocapture --ignored

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

fn probe_ffmpeg_video_stream(
    path: &str,
    preferred_stream: Option<u32>,
) -> Result<(u32, u32, u32), Box<dyn std::error::Error>> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_type,width,height",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()?;
    if !probe.status.success() {
        return Err("ffprobe 执行失败, 无法获取视频流信息".into());
    }

    let probe_s = String::from_utf8_lossy(&probe.stdout);
    let mut fallback: Option<(u32, u32, u32)> = None;

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
        if let Some(want_idx) = preferred_stream
            && idx == want_idx
        {
            return Ok((idx, width, height));
        }
        if fallback.is_none() {
            fallback = Some((idx, width, height));
        }
    }

    fallback.ok_or("ffprobe 未找到有效视频流".into())
}

fn decode_h264_with_ffmpeg(
    path: &str,
    preferred_stream: Option<u32>,
    target_size: Option<(u32, u32)>,
) -> FfmpegDecodeResult {
    let (stream_idx, mut width, mut height) = probe_ffmpeg_video_stream(path, preferred_stream)?;
    if let Some((tw, th)) = target_size
        && tw > 0
        && th > 0
    {
        width = tw;
        height = th;
    }

    let frame_limit = compare_frames_limit().to_string();
    let map_spec = format!("0:{stream_idx}");
    let tmp = make_ffmpeg_tmp_path("h264_cmp");
    let mut cmd = Command::new("ffmpeg");
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
        "yuv420p",
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
) -> Result<(), Box<dyn std::error::Error>> {
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

    Ok(())
}

fn compare_video(
    width: u32,
    height: u32,
    reference: &[Vec<u8>],
    test: &[Vec<u8>],
) -> Result<CompareStats, Box<dyn std::error::Error>> {
    let mut stats = CompareStats::default();
    let frame_count = reference.len().min(test.len());
    for i in 0..frame_count {
        compare_frame(&mut stats, i, &reference[i], &test[i], width, height)?;
    }
    Ok(stats)
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
    let (tao_w, tao_h, tao_frames, tao_stream_index) = decode_h264_with_tao(path)?;
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
    }

    if analyze_shift_enabled() {
        let max_shift = 8isize;
        let (shift, precision) = estimate_best_shift(&ff_frames, &tao_frames, max_shift);
        println!(
            "[{}] 偏移诊断: shift={}, 对齐精度={:.6}% (搜索范围=±{})",
            path, shift, precision, max_shift
        );
    }

    let stats = compare_video(tao_w, tao_h, &ff_frames, &tao_frames)?;
    print_compare_stats(path, tao_frames.len(), ff_frames.len(), &stats);

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
