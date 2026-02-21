use super::*;
use crate::codec_parameters::{AudioCodecParams, CodecParamsType};

fn make_aac_params() -> CodecParameters {
    CodecParameters {
        codec_id: CodecId::Aac,
        extra_data: vec![0x12, 0x10], // AAC-LC, 44100Hz, stereo
        bit_rate: 0,
        params: CodecParamsType::Audio(AudioCodecParams {
            sample_rate: 44100,
            channel_layout: ChannelLayout::from_channels(2),
            sample_format: SampleFormat::F32,
            frame_size: 1024,
        }),
    }
}

#[test]
fn test_create_and_open() {
    let mut decoder = AacDecoder::create().unwrap();
    let params = make_aac_params();
    decoder.open(&params).unwrap();
    assert_eq!(decoder.codec_id(), CodecId::Aac);
    assert_eq!(decoder.name(), "aac");
}

#[test]
fn test_not_open_error() {
    let mut decoder = AacDecoder::create().unwrap();
    let pkt = Packet::from_data(vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x00, 0x00]);
    assert!(decoder.send_packet(&pkt).is_err());
}

#[test]
fn test_silence_frame_decode() {
    let mut decoder = AacDecoder::create().unwrap();
    let params = make_aac_params();
    decoder.open(&params).unwrap();

    let mut adts_frame = vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC];
    adts_frame.extend_from_slice(&[0; 10]);
    let pkt = Packet::from_data(adts_frame);
    decoder.send_packet(&pkt).unwrap();

    let frame = decoder.receive_frame().unwrap();
    if let Frame::Audio(af) = frame {
        assert_eq!(af.nb_samples, 1024);
        assert_eq!(af.sample_rate, 44100);
    } else {
        panic!("应为音频帧");
    }
}

#[test]
fn test_flush_and_eof() {
    let mut decoder = AacDecoder::create().unwrap();
    let params = make_aac_params();
    decoder.open(&params).unwrap();

    let empty_pkt = Packet::empty();
    decoder.send_packet(&empty_pkt).unwrap();
    assert!(matches!(decoder.receive_frame(), Err(TaoError::Eof)));
}

#[test]
fn test_audio_specific_config_parse() {
    let mut dec = AacDecoder {
        sample_rate: 0,
        channels: 0,
        channel_layout: ChannelLayout::from_channels(1),
        channel_config: 1,
        use_default_channel_map: true,
        sample_rate_index: 0,
        output_frame: None,
        opened: false,
        flushing: false,
        overlap: Vec::new(),
        first_frame: true,
        codebooks: None,
        default_leading_trim_samples: 0,
        pending_leading_trim_samples: 0,
        prev_window_shape: Vec::new(),
        long_sine_window: Vec::new(),
        long_kbd_window: Vec::new(),
        short_sine_window: Vec::new(),
        short_kbd_window: Vec::new(),
        random_state: Cell::new(0x1f2e3d4c),
    };
    dec.parse_audio_specific_config(&[0x12, 0x10]).unwrap();
    assert_eq!(dec.sample_rate, 44100);
    assert_eq!(dec.channels, 2);
}

#[test]
fn test_imdct_all_zero() {
    let spectrum = vec![0.0f32; 1024];
    let output = imdct_1024(&spectrum);
    assert_eq!(output.len(), 2048);
    for &s in &output {
        assert_eq!(s, 0.0);
    }
}

#[test]
fn test_sf_huffman_tree_build() {
    let cbs = AacCodebooks::build();
    // 测试 delta=0 (index=60): 码字 "0" (1 bit)
    let data = [0x00u8]; // 第一位是 0
    let mut br = BitReader::new(&data);
    let val = cbs.sf_tree.decode(&mut br).unwrap();
    assert_eq!(val, 60); // SF index 60 = delta 0
}

#[test]
fn test_adts_header_skip() {
    let mut decoder = AacDecoder::create().unwrap();
    let params = make_aac_params();
    decoder.open(&params).unwrap();

    let mut adts_frame = vec![0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC];
    adts_frame.extend_from_slice(&[0; 10]);
    let pkt = Packet::from_data(adts_frame);
    decoder.send_packet(&pkt).unwrap();
    assert!(matches!(decoder.receive_frame(), Ok(Frame::Audio(_))));
}
