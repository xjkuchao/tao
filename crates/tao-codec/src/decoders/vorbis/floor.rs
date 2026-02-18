use tao_core::TaoResult;

use super::setup::{MappingConfig, ParsedSetup};

/// floor 恢复阶段上下文.
#[derive(Debug, Clone)]
pub(crate) struct FloorContext {
    pub(crate) channel_count: usize,
    pub(crate) floor_index_per_channel: Vec<usize>,
}

/// 基于 setup 与当前包头信息构建 floor 阶段上下文.
pub(crate) fn build_floor_context(
    setup: &ParsedSetup,
    mapping: &MappingConfig,
    channel_count: usize,
) -> TaoResult<FloorContext> {
    if setup.floor_count == 0 {
        return Err(tao_core::TaoError::InvalidData(
            "Vorbis floor_count 非法".into(),
        ));
    }
    let mut floor_index_per_channel = vec![0usize; channel_count];
    for (ch, slot) in floor_index_per_channel.iter_mut().enumerate() {
        let mux = mapping.channel_mux.get(ch).copied().unwrap_or(0) as usize;
        let floor_idx = mapping.submap_floor.get(mux).copied().ok_or_else(|| {
            tao_core::TaoError::InvalidData("Vorbis mapping floor 子映射索引越界".into())
        })? as usize;
        if floor_idx >= setup.floors.len() {
            return Err(tao_core::TaoError::InvalidData(
                "Vorbis floor 索引越界".into(),
            ));
        }
        *slot = floor_idx;
    }
    Ok(FloorContext {
        channel_count,
        floor_index_per_channel,
    })
}
