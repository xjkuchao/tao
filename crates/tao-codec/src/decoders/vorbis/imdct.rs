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
