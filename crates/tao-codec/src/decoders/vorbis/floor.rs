use tao_core::TaoResult;

use super::setup::ParsedSetup;

/// floor 恢复阶段上下文.
#[derive(Debug, Clone)]
pub(crate) struct FloorContext {
    pub(crate) channel_count: usize,
}

/// 基于 setup 与当前包头信息构建 floor 阶段上下文.
pub(crate) fn build_floor_context(
    setup: &ParsedSetup,
    channel_count: usize,
) -> TaoResult<FloorContext> {
    if setup.floor_count == 0 {
        return Err(tao_core::TaoError::InvalidData(
            "Vorbis floor_count 非法".into(),
        ));
    }
    Ok(FloorContext { channel_count })
}
