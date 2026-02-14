//! 封装器 (Muxer) trait 定义.
//!
//! 对标 FFmpeg 的 `AVOutputFormat`, 定义了将数据包写入容器格式的接口.

use tao_codec::Packet;
use tao_core::TaoResult;

use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::stream::Stream;

/// 封装器 trait
///
/// 将压缩数据包写入容器格式. 所有格式的封装器都实现此 trait.
///
/// 使用流程:
/// 1. 配置输出流信息
/// 2. 调用 `write_header()` 写入容器头部
/// 3. 循环调用 `write_packet()` 写入数据包
/// 4. 调用 `write_trailer()` 写入容器尾部并关闭
pub trait Muxer: Send {
    /// 获取格式标识
    fn format_id(&self) -> FormatId;

    /// 获取格式名称
    fn name(&self) -> &str;

    /// 写入容器头部
    ///
    /// # 参数
    /// - `streams`: 输出流信息列表
    fn write_header(&mut self, io: &mut IoContext, streams: &[Stream]) -> TaoResult<()>;

    /// 写入一个数据包
    fn write_packet(&mut self, io: &mut IoContext, packet: &Packet) -> TaoResult<()>;

    /// 写入容器尾部, 完成封装
    fn write_trailer(&mut self, io: &mut IoContext) -> TaoResult<()>;
}
