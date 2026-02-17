use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tao_codec::codec_parameters::{CodecParameters, CodecParamsType, VideoCodecParams};
use tao_codec::decoder::Decoder;
use tao_core::{MediaType, TaoError};
use tao_format::{FormatRegistry, IoContext, stream::StreamParams};

// Reuse FfmpegComparer and FrameDiff from ffmpeg_compare module
// We need to include the module here or link to it
mod ffmpeg_compare;
use ffmpeg_compare::{FfmpegComparer, FrameDiff};

const SAMPLE_URL: &str = "https://samples.ffmpeg.org/V-codecs/MPEG4/color16.avi";
const OUTPUT_DIR: &str = "data/debug_color16";

#[test]
#[cfg(feature = "http")]
fn debug_color16_comparison() {
    println!("=== Debugging color16.avi ===");

    // 1. Setup output directory
    let output_dir = PathBuf::from(OUTPUT_DIR);
    fs::create_dir_all(&output_dir).expect("Failed to create output dir");

    // Download sample if not exists
    let local_sample = output_dir.join("color16.avi");
    if !local_sample.exists() {
        println!("Downloading sample to {:?}...", local_sample);
        let status = Command::new("curl")
            .arg("-L")
            .arg("-o")
            .arg(&local_sample)
            .arg(SAMPLE_URL)
            .status()
            .expect("Failed to run curl");

        if !status.success() {
            panic!("Failed to download sample");
        }
    }

    // 2. Generate FFmpeg reference frames
    println!("Generating FFmpeg reference frames...");
    let comparer =
        FfmpegComparer::new(&local_sample, &output_dir).expect("Failed to init comparer");

    // Check FFmpeg availability
    if !FfmpegComparer::check_ffmpeg_available() {
        println!("Skipping test: FFmpeg not available");
        return;
    }

    // Generate ALL frames (0)
    let ref_file = comparer
        .generate_reference_frames(0)
        .expect("Failed to generate ref frames");
    println!("Reference frames generated at: {:?}", ref_file);

    // 3. Decode with Tao
    println!("Decoding with Tao...");
    let mut format_reg = FormatRegistry::new();
    tao_format::register_all(&mut format_reg);

    let mut codec_reg = tao_codec::CodecRegistry::new();
    tao_codec::register_all(&mut codec_reg);

    let mut io = IoContext::open_url(SAMPLE_URL).expect("Failed to open URL");
    let mut demuxer = format_reg
        .open_input(&mut io, None)
        .expect("Failed to open demuxer");

    let video_stream_index = demuxer
        .streams()
        .iter()
        .position(|s| s.media_type == MediaType::Video)
        .expect("No video stream");

    let stream = &demuxer.streams()[video_stream_index];
    let (width, height) = match &stream.params {
        StreamParams::Video(v) => (v.width, v.height),
        _ => panic!("Not video"),
    };
    println!("Video: {}x{}", width, height);

    let codec_params = match &stream.params {
        StreamParams::Video(v) => CodecParameters {
            codec_id: stream.codec_id,
            extra_data: stream.extra_data.clone(),
            bit_rate: v.bit_rate,
            params: CodecParamsType::Video(VideoCodecParams {
                width: v.width,
                height: v.height,
                pixel_format: v.pixel_format,
                frame_rate: v.frame_rate,
                sample_aspect_ratio: v.sample_aspect_ratio,
            }),
        },
        _ => panic!("Not video"),
    };

    let mut decoder = codec_reg
        .create_decoder(stream.codec_id)
        .expect("No decoder");
    decoder.open(&codec_params).expect("Failed to open decoder");

    // Read reference file
    let ref_data = fs::read(&ref_file).expect("Failed to read ref file");
    let frame_size = (width * height + 2 * (width / 2) * (height / 2)) as usize; // YUV420p

    let mut frames_decoded = 0;

    loop {
        match demuxer.read_packet(&mut io) {
            Ok(packet) => {
                if packet.stream_index != video_stream_index {
                    continue;
                }

                if decoder.send_packet(&packet).is_ok() {
                    loop {
                        match decoder.receive_frame() {
                            Ok(frame) => {
                                if let tao_codec::frame::Frame::Video(vf) = frame {
                                    // Compare with reference
                                    let offset = frames_decoded * frame_size;
                                    if offset + frame_size <= ref_data.len() {
                                        let ref_frame = &ref_data[offset..offset + frame_size];

                                        // Reconstruct YUV420p linear buffer from Tao frame (planar)
                                        let mut tao_data = Vec::with_capacity(frame_size);
                                        tao_data.extend_from_slice(&vf.data[0]); // Y
                                        tao_data.extend_from_slice(&vf.data[1]); // U
                                        tao_data.extend_from_slice(&vf.data[2]); // V

                                        let diff =
                                            FrameDiff::compare(&tao_data, ref_frame, width, height)
                                                .expect("Comparison failed");

                                        println!(
                                            "Frame {}: Y_PSNR={:.2} U_PSNR={:.2} V_PSNR={:.2}",
                                            frames_decoded, diff.psnr_y, diff.psnr_u, diff.psnr_v
                                        );

                                        if diff.psnr_y < 30.0
                                            || diff.psnr_u < 30.0
                                            || diff.psnr_v < 30.0
                                        {
                                            println!("  [FAIL] Low quality!");
                                            println!("  Diff Summary: {}", diff.summary());
                                        }
                                    } else {
                                        println!(
                                            "Frame {}: No reference data available",
                                            frames_decoded
                                        );
                                    }
                                    frames_decoded += 1;
                                }
                            }
                            Err(TaoError::NeedMoreData) => break,
                            Err(e) => {
                                println!("Decode error: {:?}", e);
                                break;
                            }
                        }
                    }
                }
            }
            Err(TaoError::Eof) => break,
            Err(e) => {
                println!("Demux error: {:?}", e);
                break;
            }
        }
        if frames_decoded >= 200 {
            break;
        }
    }
}
