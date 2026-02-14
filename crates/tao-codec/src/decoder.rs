//! 解码器 trait 定义.
//!
//! 所有解码器实现必须实现 `Decoder` trait.

use tao_core::TaoResult;

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::frame::Frame;
use crate::packet::Packet;

/// 解码器 trait
///
/// 定义了解码器的统一接口. 所有具体解码器 (H.264, AAC 等) 都实现此 trait.
///
/// 解码流程:
/// 1. 调用 `send_packet()` 送入压缩数据
/// 2. 调用 `receive_frame()` 取出解码后的帧
/// 3. 重复以上步骤直到所有数据处理完毕
/// 4. 送入空包 (flush) 以获取解码器中缓存的帧
pub trait Decoder: Send {
    /// 获取解码器标识
    fn codec_id(&self) -> CodecId;

    /// 获取解码器名称
    fn name(&self) -> &str;

    /// 使用参数配置解码器
    ///
    /// 对于 RAW/PCM 等无头部信息的编解码器, 必须在解码前调用此方法提供参数.
    /// 默认实现为空操作, 允许不需要额外配置的解码器跳过此步骤.
    fn open(&mut self, _params: &CodecParameters) -> TaoResult<()> {
        Ok(())
    }

    /// 送入一个压缩数据包进行解码
    ///
    /// # 参数
    /// - `packet`: 压缩数据包. 送入空包表示刷新 (flush), 获取缓存帧.
    ///
    /// # 返回
    /// - `Ok(())`: 数据包已接受
    /// - `Err(TaoError::NeedMoreData)`: 解码器内部缓冲区已满, 需要先取出帧
    fn send_packet(&mut self, packet: &Packet) -> TaoResult<()>;

    /// 从解码器取出一帧解码数据
    ///
    /// # 返回
    /// - `Ok(frame)`: 成功取出一帧
    /// - `Err(TaoError::NeedMoreData)`: 需要送入更多数据包
    /// - `Err(TaoError::Eof)`: 所有帧已取出
    fn receive_frame(&mut self) -> TaoResult<Frame>;

    /// 刷新解码器, 清空内部状态
    ///
    /// 用于 seek 后重置解码器状态.
    fn flush(&mut self);
}
