use tao_codec::codec_parameters::{AudioCodecParams, CodecParamsType, VideoCodecParams};
use tao_codec::frame::AudioFrame;
use tao_codec::{CodecId, CodecParameters, CodecRegistry, Decoder, Encoder, Frame, Packet};
use tao_core::{ChannelLayout, MediaType, PixelFormat, Rational, SampleFormat, TaoError};
use tao_filter::FilterGraph;
use tao_format::stream::{AudioStreamParams, Stream, StreamParams, VideoStreamParams};
use tao_resample::ResampleContext;

use crate::filter::{
    FilterSpec, build_audio_filter_graph, build_video_filter_graph, codec_id_to_sample_format,
};

pub(crate) struct StreamProcessor {
    decoder: Box<dyn Decoder>,
    encoder: Box<dyn Encoder>,
    resampler: Option<ResampleContext>,
    filter_graph: Option<FilterGraph>,
    video_scaler: Option<VideoScaleConfig>,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
}

/// 视频缩放配置
pub(crate) struct VideoScaleConfig {
    dst_width: u32,
    dst_height: u32,
    dst_pixel_format: PixelFormat,
}

// ============================================================
// 转码/刷新
// ============================================================

/// 转码一个数据包
pub(crate) fn transcode_packet(
    proc: &mut StreamProcessor,
    input_pkt: &Packet,
    out_stream_idx: usize,
) -> Result<Vec<Packet>, TaoError> {
    proc.decoder.send_packet(input_pkt)?;

    let mut output_packets = Vec::new();

    loop {
        match proc.decoder.receive_frame() {
            Ok(frame) => {
                // 应用滤镜
                let filtered_frame = if let Some(ref mut graph) = proc.filter_graph {
                    graph.process_frame(&frame)?
                } else {
                    frame
                };

                // 视频缩放
                let scaled_frame = if let Some(ref scale_cfg) = proc.video_scaler {
                    scale_video_frame(&filtered_frame, scale_cfg)?
                } else {
                    filtered_frame
                };

                // 音频重采样
                let frame_to_encode = if let Some(ref resampler) = proc.resampler {
                    resample_frame(
                        resampler,
                        &scaled_frame,
                        proc.dst_channels,
                        proc.dst_sample_format,
                    )?
                } else {
                    scaled_frame
                };

                proc.encoder.send_frame(Some(&frame_to_encode))?;

                loop {
                    match proc.encoder.receive_packet() {
                        Ok(mut pkt) => {
                            pkt.stream_index = out_stream_idx;
                            output_packets.push(pkt);
                        }
                        Err(TaoError::NeedMoreData) => break,
                        Err(TaoError::Eof) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(TaoError::NeedMoreData) => break,
            Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }

    Ok(output_packets)
}

/// 刷新编码器
pub(crate) fn flush_encoder(
    proc: &mut StreamProcessor,
    out_stream_idx: usize,
) -> Result<Vec<Packet>, TaoError> {
    proc.encoder.send_frame(None)?;

    let mut output_packets = Vec::new();
    loop {
        match proc.encoder.receive_packet() {
            Ok(mut pkt) => {
                pkt.stream_index = out_stream_idx;
                output_packets.push(pkt);
            }
            Err(TaoError::NeedMoreData) | Err(TaoError::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(output_packets)
}

// ============================================================
// 视频缩放
// ============================================================

/// 缩放视频帧
pub(crate) fn scale_video_frame(
    frame: &Frame,
    config: &VideoScaleConfig,
) -> Result<Frame, TaoError> {
    use tao_codec::frame::VideoFrame;

    match frame {
        Frame::Video(vf) => {
            if vf.width == config.dst_width
                && vf.height == config.dst_height
                && vf.pixel_format == config.dst_pixel_format
            {
                return Ok(frame.clone());
            }

            let ctx = tao_scale::ScaleContext::new(
                vf.width,
                vf.height,
                vf.pixel_format,
                config.dst_width,
                config.dst_height,
                config.dst_pixel_format,
                tao_scale::ScaleAlgorithm::Bilinear,
            );

            // 准备源数据
            let src_planes: Vec<&[u8]> = vf.data.iter().map(|d| d.as_slice()).collect();
            let src_linesize: Vec<usize> = vf.linesize.clone();

            // 分配目标帧
            let dst_fmt = config.dst_pixel_format;
            let dst_w = config.dst_width;
            let dst_h = config.dst_height;
            let plane_count = dst_fmt.plane_count() as usize;

            let mut dst_bufs: Vec<Vec<u8>> = Vec::with_capacity(plane_count);
            let mut dst_linesizes: Vec<usize> = Vec::with_capacity(plane_count);

            for p in 0..plane_count {
                let ls = dst_fmt
                    .plane_linesize(p, dst_w)
                    .unwrap_or(dst_w as usize * 3);
                let h = dst_fmt.plane_height(p, dst_h).unwrap_or(dst_h as usize);
                dst_bufs.push(vec![0u8; ls * h]);
                dst_linesizes.push(ls);
            }

            {
                let mut dst_slices: Vec<&mut [u8]> =
                    dst_bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
                ctx.scale(&src_planes, &src_linesize, &mut dst_slices, &dst_linesizes)?;
            }

            let mut out_frame = VideoFrame::new(dst_w, dst_h, dst_fmt);
            out_frame.data = dst_bufs;
            out_frame.linesize = dst_linesizes;
            out_frame.pts = vf.pts;
            out_frame.time_base = vf.time_base;

            Ok(Frame::Video(out_frame))
        }
        _ => Ok(frame.clone()),
    }
}

// ============================================================
// 音频重采样
// ============================================================

/// 重采样一帧音频
pub(crate) fn resample_frame(
    resampler: &ResampleContext,
    frame: &Frame,
    dst_channels: u32,
    dst_sample_format: SampleFormat,
) -> Result<Frame, TaoError> {
    match frame {
        Frame::Audio(audio) => {
            let input_data = &audio.data[0];
            let (output_data, nb_out) = resampler.convert(input_data, audio.nb_samples)?;

            let mut out_frame = AudioFrame::new(
                nb_out,
                resampler.dst_sample_rate,
                dst_sample_format,
                ChannelLayout::from_channels(dst_channels),
            );
            out_frame.data[0] = output_data;
            out_frame.pts = audio.pts;
            out_frame.time_base = audio.time_base;
            out_frame.duration = nb_out as i64;

            Ok(Frame::Audio(out_frame))
        }
        _ => Err(TaoError::Unsupported("视频帧重采样尚未实现".to_string())),
    }
}

// ============================================================
// 音频处理器创建
// ============================================================

/// 为音频流创建处理器
pub(crate) fn create_audio_processor(
    input_stream: &Stream,
    output_codec_id: CodecId,
    codec_registry: &CodecRegistry,
    target_sample_rate: Option<u32>,
    target_channels: Option<u32>,
    audio_filters: &Option<Vec<FilterSpec>>,
) -> Result<(StreamProcessor, Stream), TaoError> {
    let audio_params = match &input_stream.params {
        StreamParams::Audio(a) => a,
        _ => {
            return Err(TaoError::InvalidArgument("不是音频流".to_string()));
        }
    };

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(input_stream.codec_id)?;
    let dec_params = CodecParameters {
        codec_id: input_stream.codec_id,
        extra_data: input_stream.extra_data.clone(),
        bit_rate: audio_params.bit_rate,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: audio_params.sample_rate,
            channel_layout: audio_params.channel_layout,
            sample_format: audio_params.sample_format,
            frame_size: audio_params.frame_size,
        }),
    };
    decoder.open(&dec_params)?;

    // 确定输出参数
    let out_sample_rate = target_sample_rate.unwrap_or(audio_params.sample_rate);
    let out_channels = target_channels.unwrap_or(audio_params.channel_layout.channels);
    let out_channel_layout = ChannelLayout::from_channels(out_channels);

    let out_sample_format =
        codec_id_to_sample_format(output_codec_id).unwrap_or(audio_params.sample_format);

    // 创建编码器
    let mut encoder = codec_registry.create_encoder(output_codec_id)?;
    let enc_params = CodecParameters {
        codec_id: output_codec_id,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: out_sample_rate,
            channel_layout: out_channel_layout,
            sample_format: out_sample_format,
            frame_size: 0,
        }),
    };
    encoder.open(&enc_params)?;

    // 判断是否需要重采样
    let need_resample = audio_params.sample_rate != out_sample_rate
        || audio_params.channel_layout.channels != out_channels
        || audio_params.sample_format != out_sample_format;

    let resampler = if need_resample {
        Some(ResampleContext::new(
            audio_params.sample_rate,
            audio_params.sample_format,
            audio_params.channel_layout,
            out_sample_rate,
            out_sample_format,
            out_channel_layout,
        ))
    } else {
        None
    };

    // 创建音频滤镜图
    let filter_graph = build_audio_filter_graph(audio_filters);

    // 构建输出流描述
    let out_stream = Stream {
        index: input_stream.index,
        media_type: MediaType::Audio,
        codec_id: output_codec_id,
        time_base: Rational::new(1, out_sample_rate as i32),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Audio(AudioStreamParams {
            sample_rate: out_sample_rate,
            channel_layout: out_channel_layout,
            sample_format: out_sample_format,
            bit_rate: 0,
            frame_size: 0,
        }),
        metadata: input_stream.metadata.clone(),
    };

    let processor = StreamProcessor {
        decoder,
        encoder,
        resampler,
        filter_graph,
        video_scaler: None,
        dst_channels: out_channels,
        dst_sample_format: out_sample_format,
    };

    Ok((processor, out_stream))
}

// ============================================================
// 视频处理器创建
// ============================================================

/// 为视频流创建处理器
pub(crate) fn create_video_processor(
    input_stream: &Stream,
    output_codec_id: CodecId,
    codec_registry: &CodecRegistry,
    target_size: Option<(u32, u32)>,
    target_rate: Option<Rational>,
    video_filters: &Option<Vec<FilterSpec>>,
) -> Result<(StreamProcessor, Stream), TaoError> {
    let video_params = match &input_stream.params {
        StreamParams::Video(v) => v,
        _ => {
            return Err(TaoError::InvalidArgument("不是视频流".to_string()));
        }
    };

    // 创建解码器
    let mut decoder = codec_registry.create_decoder(input_stream.codec_id)?;
    let dec_params = CodecParameters {
        codec_id: input_stream.codec_id,
        extra_data: input_stream.extra_data.clone(),
        bit_rate: video_params.bit_rate,
        params: CodecParamsType::Video(VideoCodecParams {
            width: video_params.width,
            height: video_params.height,
            pixel_format: video_params.pixel_format,
            frame_rate: video_params.frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
        }),
    };
    decoder.open(&dec_params)?;

    // 确定输出参数
    let (out_width, out_height) = target_size.unwrap_or((video_params.width, video_params.height));
    let out_pixel_format = video_params.pixel_format;
    let out_frame_rate = target_rate.unwrap_or(video_params.frame_rate);

    // 创建编码器
    let mut encoder = codec_registry.create_encoder(output_codec_id)?;
    let enc_params = CodecParameters {
        codec_id: output_codec_id,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Video(VideoCodecParams {
            width: out_width,
            height: out_height,
            pixel_format: out_pixel_format,
            frame_rate: out_frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
        }),
    };
    encoder.open(&enc_params)?;

    // 缩放配置
    let needs_scale = out_width != video_params.width || out_height != video_params.height;
    let video_scaler = if needs_scale {
        Some(VideoScaleConfig {
            dst_width: out_width,
            dst_height: out_height,
            dst_pixel_format: out_pixel_format,
        })
    } else {
        None
    };

    // 创建视频滤镜图
    let filter_graph = build_video_filter_graph(video_filters);

    // 构建输出流描述
    let out_stream = Stream {
        index: input_stream.index,
        media_type: MediaType::Video,
        codec_id: output_codec_id,
        time_base: Rational::new(out_frame_rate.den, out_frame_rate.num),
        duration: 0,
        start_time: 0,
        nb_frames: 0,
        extra_data: Vec::new(),
        params: StreamParams::Video(VideoStreamParams {
            width: out_width,
            height: out_height,
            pixel_format: out_pixel_format,
            frame_rate: out_frame_rate,
            sample_aspect_ratio: video_params.sample_aspect_ratio,
            bit_rate: 0,
        }),
        metadata: input_stream.metadata.clone(),
    };

    let processor = StreamProcessor {
        decoder,
        encoder,
        resampler: None,
        filter_graph,
        video_scaler,
        dst_channels: 0,
        dst_sample_format: SampleFormat::None,
    };

    Ok((processor, out_stream))
}
