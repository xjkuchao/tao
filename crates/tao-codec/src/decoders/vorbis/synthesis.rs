use tao_core::{Rational, SampleFormat};

use crate::frame::AudioFrame;

use super::imdct::TimeDomainBlock;

fn vorbis_output_channel_order(channels: usize) -> Vec<usize> {
    match channels {
        // Vorbis 3ch: L, C, R -> 常见输出顺序: L, R, C
        3 => vec![0, 2, 1],
        // Vorbis 5ch: L, C, R, LS, RS -> 常见输出顺序: L, R, C, LS, RS
        5 => vec![0, 2, 1, 3, 4],
        // Vorbis 6ch: L, C, R, LS, RS, LFE -> 常见输出顺序: L, R, C, LFE, LS, RS
        6 => vec![0, 2, 1, 5, 3, 4],
        _ => (0..channels).collect(),
    }
}

/// 将时域块写入 `AudioFrame` (F32 交错).
pub(crate) fn synthesize_frame(
    td: &TimeDomainBlock,
    sample_rate: u32,
    channel_layout: tao_core::ChannelLayout,
    pts: i64,
    duration: i64,
) -> AudioFrame {
    let channels = channel_layout.channels as usize;
    let samples_per_ch = duration.max(0) as usize;
    let mut frame = AudioFrame::new(
        samples_per_ch as u32,
        sample_rate,
        SampleFormat::F32,
        channel_layout,
    );
    frame.pts = pts;
    frame.time_base = Rational::new(1, sample_rate as i32);
    frame.duration = duration;

    let ch_order = vorbis_output_channel_order(channels);
    let mut interleaved = vec![0.0f32; samples_per_ch * channels];
    for s in 0..samples_per_ch {
        for ch in 0..channels {
            let src_ch = *ch_order.get(ch).unwrap_or(&ch);
            let v = td
                .channels
                .get(src_ch)
                .and_then(|c| c.get(s))
                .copied()
                .unwrap_or(0.0);
            interleaved[s * channels + ch] = v;
        }
    }
    frame.data[0] = interleaved
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    frame
}
