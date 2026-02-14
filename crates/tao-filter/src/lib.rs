//! # tao-filter
//!
//! Tao 多媒体框架滤镜库, 提供音视频滤镜框架.
//!
//! 本 crate 对标 FFmpeg 的 libavfilter, 支持构建滤镜图 (FilterGraph)
//! 对音视频数据进行各种变换处理.

use tao_codec::frame::Frame;
use tao_core::TaoResult;

/// 滤镜 trait
///
/// 所有音视频滤镜都实现此 trait.
/// 滤镜接收输入帧, 处理后输出帧.
pub trait Filter: Send {
    /// 获取滤镜名称
    fn name(&self) -> &str;

    /// 送入一帧数据
    fn send_frame(&mut self, frame: &Frame) -> TaoResult<()>;

    /// 取出一帧处理后的数据
    fn receive_frame(&mut self) -> TaoResult<Frame>;

    /// 刷新滤镜 (处理剩余缓存数据)
    fn flush(&mut self) -> TaoResult<()>;
}

/// 滤镜图
///
/// 由多个滤镜组成的处理管线, 数据从输入端流经各个滤镜后到达输出端.
///
/// 滤镜图支持线性链和复杂拓扑 (如多输入多输出).
pub struct FilterGraph {
    /// 滤镜链中的滤镜列表
    filters: Vec<Box<dyn Filter>>,
}

impl FilterGraph {
    /// 创建空的滤镜图
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// 添加滤镜到图中
    pub fn add_filter(&mut self, filter: Box<dyn Filter>) {
        self.filters.push(filter);
    }

    /// 获取滤镜数量
    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }
}

impl Default for FilterGraph {
    fn default() -> Self {
        Self::new()
    }
}
