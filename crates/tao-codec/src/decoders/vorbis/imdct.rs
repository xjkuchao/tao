/// IMDCT 输出时域样本.
#[derive(Debug, Clone)]
pub(crate) struct TimeDomainBlock {
    pub(crate) channels: Vec<Vec<f32>>,
}

/// 当前阶段的 IMDCT 占位实现: 直接输出零样本块.
pub(crate) fn imdct_placeholder(channel_count: usize, blocksize: usize) -> TimeDomainBlock {
    TimeDomainBlock {
        channels: vec![vec![0.0; blocksize]; channel_count],
    }
}

/// 将当前块与上一块 overlap 区域拼接成输出块.
///
/// 真实实现会执行窗口与重叠相加, 当前阶段用直接覆盖保持接口形态稳定.
pub(crate) fn overlap_add_placeholder(
    td: &TimeDomainBlock,
    overlap: &mut [Vec<f32>],
    out_samples: usize,
) -> TimeDomainBlock {
    let mut out = vec![vec![0.0f32; out_samples]; td.channels.len()];
    for (ch, ch_out) in out.iter_mut().enumerate() {
        for (i, v) in ch_out.iter_mut().enumerate() {
            *v = td
                .channels
                .get(ch)
                .and_then(|c| c.get(i))
                .copied()
                .unwrap_or(0.0);
        }
        if let Some(ch_overlap) = overlap.get_mut(ch) {
            ch_overlap.clear();
            if let Some(ch_src) = td.channels.get(ch) {
                ch_overlap.extend_from_slice(ch_src);
            }
        }
    }
    TimeDomainBlock { channels: out }
}
