//! 解封装器 (Demuxer) trait 定义.
//!
//! 对标 FFmpeg 的 `AVInputFormat`, 定义了从容器格式中读取数据包的接口.

use tao_codec::Packet;
use tao_core::TaoResult;

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::stream::Stream;

/// Chapter 信息（ffprobe 兼容接口壳）.
#[derive(Debug, Clone, Default)]
pub struct DemuxerChapter {
    /// chapter 起始时间（秒）.
    pub start_time: Option<f64>,
    /// chapter 结束时间（秒）.
    pub end_time: Option<f64>,
    /// chapter 元数据.
    pub metadata: Vec<(String, String)>,
}

/// Program 信息（ffprobe 兼容接口壳）.
#[derive(Debug, Clone, Default)]
pub struct DemuxerProgram {
    /// program 标识.
    pub id: i64,
    /// 关联流索引.
    pub stream_indices: Vec<usize>,
    /// program 元数据.
    pub metadata: Vec<(String, String)>,
}

/// Stream group 信息（ffprobe 兼容接口壳）.
#[derive(Debug, Clone, Default)]
pub struct DemuxerStreamGroup {
    /// group 标识.
    pub id: i64,
    /// 关联流索引.
    pub stream_indices: Vec<usize>,
    /// group 元数据.
    pub metadata: Vec<(String, String)>,
}

/// 解封装器 trait
///
/// 从容器格式中读取压缩数据包. 所有格式的解封装器都实现此 trait.
///
/// 使用流程:
/// 1. 调用 `open()` 打开容器并解析头部
/// 2. 调用 `streams()` 获取流信息
/// 3. 循环调用 `read_packet()` 读取数据包
/// 4. 可选: 调用 `seek()` 进行定位
pub trait Demuxer: Send {
    /// 获取格式标识
    fn format_id(&self) -> FormatId;

    /// 获取格式名称
    fn name(&self) -> &str;

    /// 打开容器并解析头部信息
    ///
    /// 读取容器头部, 解析出所有流的信息.
    fn open(&mut self, io: &mut IoContext) -> TaoResult<()>;

    /// 获取所有流信息
    fn streams(&self) -> &[Stream];

    /// 读取下一个数据包
    ///
    /// # 返回
    /// - `Ok(packet)`: 成功读取一个数据包
    /// - `Err(TaoError::Eof)`: 已到达文件末尾
    fn read_packet(&mut self, io: &mut IoContext) -> TaoResult<Packet>;

    /// 定位到指定时间点
    ///
    /// # 参数
    /// - `stream_index`: 目标流索引
    /// - `timestamp`: 目标时间戳 (以流的 time_base 为单位)
    /// - `flags`: Seek 标志
    fn seek(
        &mut self,
        io: &mut IoContext,
        stream_index: usize,
        timestamp: i64,
        flags: SeekFlags,
    ) -> TaoResult<()>;

    /// 获取容器时长 (秒), None 表示未知
    fn duration(&self) -> Option<f64>;

    /// 获取容器元数据
    fn metadata(&self) -> &[(String, String)] {
        &[]
    }

    /// 获取容器长名称（如 `QuickTime / MOV`）.
    ///
    /// 默认未提供.
    fn format_long_name(&self) -> Option<&str> {
        None
    }

    /// 获取容器起始时间（秒）.
    ///
    /// 默认未提供.
    fn start_time(&self) -> Option<f64> {
        None
    }

    /// 获取容器码率（bps）.
    ///
    /// 默认未提供.
    fn bit_rate(&self) -> Option<u64> {
        None
    }

    /// 获取 chapters.
    ///
    /// 默认空切片.
    fn chapters(&self) -> &[DemuxerChapter] {
        &[]
    }

    /// 获取 programs.
    ///
    /// 默认空切片.
    fn programs(&self) -> &[DemuxerProgram] {
        &[]
    }

    /// 获取 stream groups.
    ///
    /// 默认空切片.
    fn stream_groups(&self) -> &[DemuxerStreamGroup] {
        &[]
    }
}

/// Seek 标志
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekFlags {
    /// 向后 seek (寻找目标之前最近的关键帧)
    pub backward: bool,
    /// 基于字节位置 seek (而非时间戳)
    pub byte: bool,
    /// 寻找任意帧 (不仅是关键帧)
    pub any: bool,
}

impl Default for SeekFlags {
    fn default() -> Self {
        Self {
            backward: true,
            byte: false,
            any: false,
        }
    }
}
