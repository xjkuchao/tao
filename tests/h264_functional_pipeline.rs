//! H264 功能自测流水.
//!
//! 目标:
//! - 先验证解码功能链路可持续运行, 再进入精度收敛阶段.
//! - 对 `data/1_h264.mp4` 与 `data/2_h264.mp4` 执行固定帧数解码稳定性检查.

use tao::codec::codec_parameters::{CodecParamsType, VideoCodecParams};
use tao::codec::frame::{Frame, PictureType};
use tao::codec::packet::Packet;
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{MediaType, TaoError};
use tao::format::stream::StreamParams;
use tao::format::{FormatRegistry, IoContext};

fn decode_h264_frames(path: &str, frame_limit: usize) -> Result<usize, String> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path).map_err(|e| format!("打开输入失败: {}", e))?;
    let mut demuxer = format_registry
        .open_input(&mut io, None)
        .map_err(|e| format!("打开 demuxer 失败: {}", e))?;

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
        .ok_or("未找到 H264 视频流".to_string())?
        .clone();

    let (width, height, pixel_format, frame_rate, sample_aspect_ratio) = match &stream.params {
        StreamParams::Video(v) => (
            v.width,
            v.height,
            v.pixel_format,
            v.frame_rate,
            v.sample_aspect_ratio,
        ),
        _ => return Err("目标流不是视频流".to_string()),
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

    let mut decoder = codec_registry
        .create_decoder(stream.codec_id)
        .map_err(|e| format!("创建解码器失败: {}", e))?;
    decoder
        .open(&params)
        .map_err(|e| format!("打开解码器失败: {}", e))?;

    let mut decoded_frames = 0usize;
    let mut demux_eof = false;

    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index == stream.index {
                        decoder
                            .send_packet(&pkt)
                            .map_err(|e| format!("发送包失败: {}", e))?;
                    }
                }
                Err(TaoError::Eof) => {
                    decoder
                        .send_packet(&Packet::empty())
                        .map_err(|e| format!("发送刷新包失败: {}", e))?;
                    demux_eof = true;
                }
                Err(e) => return Err(format!("读取包失败: {}", e)),
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Video(_vf)) => {
                    decoded_frames += 1;
                    if decoded_frames >= frame_limit {
                        return Ok(decoded_frames);
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => break,
                Err(TaoError::Eof) => return Ok(decoded_frames),
                Err(e) => return Err(format!("取帧失败: {}", e)),
            }
        }

        if demux_eof {
            return Ok(decoded_frames);
        }
    }
}

fn collect_picture_type_stats(
    path: &str,
    frame_limit: usize,
) -> Result<(usize, usize, usize), String> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path).map_err(|e| format!("打开输入失败: {}", e))?;
    let mut demuxer = format_registry
        .open_input(&mut io, None)
        .map_err(|e| format!("打开 demuxer 失败: {}", e))?;

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
        .ok_or("未找到 H264 视频流".to_string())?
        .clone();

    let (width, height, pixel_format, frame_rate, sample_aspect_ratio) = match &stream.params {
        StreamParams::Video(v) => (
            v.width,
            v.height,
            v.pixel_format,
            v.frame_rate,
            v.sample_aspect_ratio,
        ),
        _ => return Err("目标流不是视频流".to_string()),
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

    let mut decoder = codec_registry
        .create_decoder(stream.codec_id)
        .map_err(|e| format!("创建解码器失败: {}", e))?;
    decoder
        .open(&params)
        .map_err(|e| format!("打开解码器失败: {}", e))?;

    let mut count_i = 0usize;
    let mut count_p = 0usize;
    let mut count_b = 0usize;
    let mut decoded_frames = 0usize;
    let mut demux_eof = false;

    loop {
        if !demux_eof {
            match demuxer.read_packet(&mut io) {
                Ok(pkt) => {
                    if pkt.stream_index == stream.index {
                        decoder
                            .send_packet(&pkt)
                            .map_err(|e| format!("发送包失败: {}", e))?;
                    }
                }
                Err(TaoError::Eof) => {
                    decoder
                        .send_packet(&Packet::empty())
                        .map_err(|e| format!("发送刷新包失败: {}", e))?;
                    demux_eof = true;
                }
                Err(e) => return Err(format!("读取包失败: {}", e)),
            }
        }

        loop {
            match decoder.receive_frame() {
                Ok(Frame::Video(vf)) => {
                    decoded_frames += 1;
                    match vf.picture_type {
                        PictureType::I => count_i += 1,
                        PictureType::P => count_p += 1,
                        PictureType::B => count_b += 1,
                        _ => {}
                    }
                    if decoded_frames >= frame_limit {
                        return Ok((count_i, count_p, count_b));
                    }
                }
                Ok(_) => {}
                Err(TaoError::NeedMoreData) => break,
                Err(TaoError::Eof) => return Ok((count_i, count_p, count_b)),
                Err(e) => return Err(format!("取帧失败: {}", e)),
            }
        }

        if demux_eof {
            return Ok((count_i, count_p, count_b));
        }
    }
}

type OpenH264DecoderResult = Result<
    (
        Box<dyn tao::codec::Decoder>,
        Box<dyn tao::format::Demuxer>,
        IoContext,
        usize,
    ),
    String,
>;

fn open_h264_decoder_for_sample(path: &str) -> OpenH264DecoderResult {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path).map_err(|e| format!("打开输入失败: {}", e))?;
    let demuxer = format_registry
        .open_input(&mut io, None)
        .map_err(|e| format!("打开 demuxer 失败: {}", e))?;

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
        .ok_or("未找到 H264 视频流".to_string())?
        .clone();

    let (width, height, pixel_format, frame_rate, sample_aspect_ratio) = match &stream.params {
        StreamParams::Video(v) => (
            v.width,
            v.height,
            v.pixel_format,
            v.frame_rate,
            v.sample_aspect_ratio,
        ),
        _ => return Err("目标流不是视频流".to_string()),
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

    let mut decoder = codec_registry
        .create_decoder(stream.codec_id)
        .map_err(|e| format!("创建解码器失败: {}", e))?;
    decoder
        .open(&params)
        .map_err(|e| format!("打开解码器失败: {}", e))?;

    Ok((decoder, demuxer, io, stream.index))
}

#[test]
#[ignore]
fn test_h264_functional_sample1_299_frames() {
    let path = "data/1_h264.mp4";
    assert!(std::path::Path::new(path).exists(), "样本不存在: {}", path);
    let decoded = decode_h264_frames(path, 299).expect("样本1 功能自测失败");
    assert!(
        decoded >= 299,
        "样本1 功能自测失败: 解码帧不足, 期望>=299, 实际={}",
        decoded
    );
}

#[test]
#[ignore]
fn test_h264_functional_sample2_300_frames() {
    let path = "data/2_h264.mp4";
    assert!(std::path::Path::new(path).exists(), "样本不存在: {}", path);
    let decoded = decode_h264_frames(path, 300).expect("样本2 功能自测失败");
    assert!(
        decoded >= 300,
        "样本2 功能自测失败: 解码帧不足, 期望>=300, 实际={}",
        decoded
    );
}

#[test]
#[ignore]
fn test_h264_functional_picture_type_stats_sample1() {
    let path = "data/1_h264.mp4";
    assert!(std::path::Path::new(path).exists(), "样本不存在: {}", path);
    let (count_i, count_p, count_b) =
        collect_picture_type_stats(path, 120).expect("样本1 图片类型统计失败");
    assert!(count_i >= 1, "样本1 应至少包含 1 帧 I 帧, 当前={}", count_i);
    assert!(count_p >= 1, "样本1 应至少包含 1 帧 P 帧, 当前={}", count_p);
    assert!(count_b >= 1, "样本1 应至少包含 1 帧 B 帧, 当前={}", count_b);
}

#[test]
#[ignore]
fn test_h264_functional_pending_frame_flush_output() {
    let path = "data/1_h264.mp4";
    assert!(std::path::Path::new(path).exists(), "样本不存在: {}", path);

    let (mut decoder, mut demuxer, mut io, stream_index) =
        open_h264_decoder_for_sample(path).expect("打开 H264 解码链路失败");

    let first_video_packet = loop {
        match demuxer.read_packet(&mut io) {
            Ok(pkt) => {
                if pkt.stream_index == stream_index {
                    break pkt;
                }
            }
            Err(TaoError::Eof) => panic!("样本过早结束, 未读取到视频包"),
            Err(e) => panic!("读取包失败: {}", e),
        }
    };

    decoder
        .send_packet(&first_video_packet)
        .expect("发送首包失败");

    let state = decoder.receive_frame();
    assert!(
        matches!(state, Err(TaoError::NeedMoreData)),
        "首包后不应直接输出帧, 实际={:?}",
        state
    );

    decoder
        .send_packet(&Packet::empty())
        .expect("发送刷新包失败");

    let frame = decoder.receive_frame();
    assert!(
        matches!(frame, Ok(Frame::Video(_))),
        "刷新后应输出 1 帧视频, 实际={:?}",
        frame
    );
}
