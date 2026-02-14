//! 解封装器 (Demuxer) trait 定义.
//!
//! 对标 FFmpeg 的 `AVInputFormat`, 定义了从容器格式中读取数据包的接口.

use tao_codec::Packet;
use tao_core::TaoResult;

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::stream::Stream;

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
