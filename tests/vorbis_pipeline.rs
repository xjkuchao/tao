//! Vorbis 解码管线阶段性测试.
//!
//! 当前用于验证:
//! - data/1.ogg 和 data/2.ogg 的 Vorbis 头包与 setup 解析能通过
//! - 解码器在进入音频包后返回 P3 阶段未实现错误

use tao::codec::codec_parameters::{AudioCodecParams, CodecParamsType};
use tao::codec::{CodecId, CodecParameters, CodecRegistry};
use tao::core::{ChannelLayout, SampleFormat, TaoError};
use tao::format::{FormatRegistry, IoContext};

fn run_vorbis_until_audio_packet(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut format_registry = FormatRegistry::new();
    tao::format::register_all(&mut format_registry);
    let mut codec_registry = CodecRegistry::new();
    tao::codec::register_all(&mut codec_registry);

    let mut io = IoContext::open_read(path)?;
    let mut demuxer = format_registry.open_input(&mut io, Some(path))?;
    demuxer.open(&mut io)?;

    let stream = demuxer
        .streams()
        .iter()
        .find(|s| s.codec_id == CodecId::Vorbis)
        .ok_or("未找到 Vorbis 流")?
        .clone();

    let (sample_rate, channel_layout) = match &stream.params {
        tao::format::stream::StreamParams::Audio(a) => (a.sample_rate, a.channel_layout),
        _ => (44100, ChannelLayout::STEREO),
    };

    let params = CodecParameters {
        codec_id: CodecId::Vorbis,
        extra_data: stream.extra_data,
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate,
            channel_layout,
            sample_format: SampleFormat::F32,
            frame_size: 0,
        }),
    };

    let mut decoder = codec_registry.create_decoder(CodecId::Vorbis)?;
    decoder.open(&params)?;

    loop {
        let pkt = demuxer.read_packet(&mut io)?;
        if pkt.stream_index != stream.index {
            continue;
        }

        match decoder.send_packet(&pkt) {
            Ok(()) => {}
            Err(TaoError::NotImplemented(msg)) => {
                assert!(msg.contains("P3"), "期望进入 P3 未实现阶段, 实际: {}", msg);
                assert!(
                    msg.contains("setup 严格解析通过"),
                    "期望 setup 严格解析通过, 实际: {}",
                    msg
                );
                return Ok(());
            }
            Err(e) => return Err(format!("Vorbis 解码阶段失败: {}", e).into()),
        }
    }
}

#[test]
fn test_vorbis_data1_头包与setup解析通过() {
    run_vorbis_until_audio_packet("data/1.ogg").expect("data/1.ogg 解析失败");
}

#[test]
fn test_vorbis_data2_头包与setup解析通过() {
    run_vorbis_until_audio_packet("data/2.ogg").expect("data/2.ogg 解析失败");
}
