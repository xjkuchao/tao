//! Tao 多媒体框架性能基准测试.
//!
//! 覆盖编解码、像素格式转换、图像缩放、音频重采样等核心路径.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tao::codec::encoders::{flac::FlacEncoder, pcm::PcmEncoder};
use tao::codec::{AudioCodecParams, CodecParameters, CodecParamsType, Frame};
use tao::core::{ChannelLayout, PixelFormat, Rational, SampleFormat};
use tao::resample::ResampleContext;
use tao::scale::{ScaleAlgorithm, ScaleContext};

/// 创建 S16 单声道音频帧
fn make_s16_frame(nb_samples: u32, sample_rate: u32) -> Frame {
    let mut data = Vec::with_capacity(nb_samples as usize * 2);
    for i in 0..nb_samples {
        let v = ((i % 256) as i16).wrapping_mul(100);
        data.extend_from_slice(&v.to_le_bytes());
    }
    Frame::Audio(tao::codec::frame::AudioFrame {
        data: vec![data],
        nb_samples,
        sample_rate,
        sample_format: SampleFormat::S16,
        channel_layout: ChannelLayout::MONO,
        pts: 0,
        time_base: Rational::new(1, sample_rate as i32),
        duration: nb_samples as i64,
    })
}

/// 创建音频编解码器参数
fn make_audio_params(sample_rate: u32, sample_format: SampleFormat) -> CodecParameters {
    CodecParameters {
        codec_id: tao::codec::CodecId::PcmS16le,
        extra_data: Vec::new(),
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout: ChannelLayout::MONO,
            sample_format,
            frame_size: 0,
        }),
    }
}

fn bench_pcm_encode(c: &mut Criterion) {
    c.bench_function("pcm_encode_1024_s16", |b| {
        let frame = make_s16_frame(1024, 44100);
        let params = make_audio_params(44100, SampleFormat::S16);
        b.iter(|| {
            let mut enc = PcmEncoder::new_s16le().unwrap();
            enc.open(&params).unwrap();
            enc.send_frame(Some(black_box(&frame))).unwrap();
            let _pkt = enc.receive_packet().unwrap();
        });
    });
}

fn bench_flac_encode(c: &mut Criterion) {
    c.bench_function("flac_encode_4096_s16", |b| {
        let frame = make_s16_frame(4096, 44100);
        let params = make_audio_params(44100, SampleFormat::S16);
        b.iter(|| {
            let mut enc = FlacEncoder::create().unwrap();
            enc.open(&params).unwrap();
            enc.send_frame(Some(black_box(&frame))).unwrap();
            let _pkt = enc.receive_packet().unwrap();
        });
    });
}

fn bench_yuv_to_rgb(c: &mut Criterion) {
    c.bench_function("yuv420p_to_rgb24_1920x1080", |b| {
        let w = 1920u32;
        let h = 1080u32;
        let y_size = (w * h) as usize;
        let uv_size = (w * h / 4) as usize;
        let y_plane: Vec<u8> = (0..y_size).map(|i| (i % 256) as u8).collect();
        let u_plane: Vec<u8> = (0..uv_size).map(|i| (i % 256) as u8).collect();
        let v_plane: Vec<u8> = (0..uv_size).map(|i| (i % 256) as u8).collect();

        let ctx = ScaleContext::new(
            w,
            h,
            PixelFormat::Yuv420p,
            w,
            h,
            PixelFormat::Rgb24,
            ScaleAlgorithm::Bilinear,
        );

        let y_linesize = w as usize;
        let u_linesize = (w / 2) as usize;
        let v_linesize = (w / 2) as usize;
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        let rgb_linesize = (w * 3) as usize;

        b.iter(|| {
            ctx.scale(
                &[&y_plane, &u_plane, &v_plane],
                &[y_linesize, u_linesize, v_linesize],
                &mut [rgb.as_mut_slice()],
                &[rgb_linesize],
            )
            .unwrap();
            black_box(&rgb);
        });
    });
}

fn bench_bilinear_scale(c: &mut Criterion) {
    c.bench_function("bilinear_scale_1920x1080_to_640x360", |b| {
        let src_w = 1920u32;
        let src_h = 1080u32;
        let dst_w = 640u32;
        let dst_h = 360u32;

        let src_size = (src_w * src_h * 3) as usize;
        let src_data: Vec<u8> = (0..src_size).map(|i| (i % 256) as u8).collect();
        let mut dst_data = vec![0u8; (dst_w * dst_h * 3) as usize];

        let ctx = ScaleContext::new(
            src_w,
            src_h,
            PixelFormat::Rgb24,
            dst_w,
            dst_h,
            PixelFormat::Rgb24,
            ScaleAlgorithm::Bilinear,
        );

        b.iter(|| {
            ctx.scale(
                &[src_data.as_slice()],
                &[(src_w * 3) as usize],
                &mut [dst_data.as_mut_slice()],
                &[(dst_w * 3) as usize],
            )
            .unwrap();
            black_box(&dst_data);
        });
    });
}

fn bench_audio_resample(c: &mut Criterion) {
    c.bench_function("resample_4096_44100_to_48000", |b| {
        let nb_samples = 4096u32;
        let mut input = Vec::with_capacity(nb_samples as usize * 2);
        for i in 0..nb_samples {
            let v = ((i % 256) as i16).wrapping_mul(100);
            input.extend_from_slice(&v.to_le_bytes());
        }

        let ctx = ResampleContext::new(
            44100,
            SampleFormat::S16,
            ChannelLayout::MONO,
            48000,
            SampleFormat::S16,
            ChannelLayout::MONO,
        );

        b.iter(|| {
            let (out, _nb) = ctx.convert(black_box(&input), nb_samples).unwrap();
            black_box(out);
        });
    });
}

criterion_group!(
    benches,
    bench_pcm_encode,
    bench_flac_encode,
    bench_yuv_to_rgb,
    bench_bilinear_scale,
    bench_audio_resample,
);
criterion_main!(benches);
