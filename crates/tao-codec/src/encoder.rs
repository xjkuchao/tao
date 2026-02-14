//! 编码器 trait 定义.
//!
//! 所有编码器实现必须实现 `Encoder` trait.

use tao_core::TaoResult;

use crate::codec_id::CodecId;
use crate::codec_parameters::CodecParameters;
use crate::frame::Frame;
use crate::packet::Packet;

/// 编码器 trait
///
/// 定义了编码器的统一接口. 所有具体编码器 (H.264, AAC 等) 都实现此 trait.
///
/// 编码流程:
/// 1. 调用 `send_frame()` 送入原始帧数据
/// 2. 调用 `receive_packet()` 取出压缩数据包
/// 3. 重复以上步骤直到所有数据处理完毕
/// 4. 送入 None 表示编码结束, 刷新编码器缓存
pub trait Encoder: Send {
    /// 获取编码器标识
    fn codec_id(&self) -> CodecId;

    /// 获取编码器名称
    fn name(&self) -> &str;

    /// 使用参数配置编码器
    ///
    /// 对于 RAW/PCM 等编解码器, 必须在编码前调用此方法提供参数.
    /// 默认实现为空操作, 允许不需要额外配置的编码器跳过此步骤.
    fn open(&mut self, _params: &CodecParameters) -> TaoResult<()> {
        Ok(())
    }

    /// 送入一帧原始数据进行编码
    ///
    /// # 参数
    /// - `frame`: 原始帧数据. `None` 表示刷新 (flush), 取出缓存的数据包.
    ///
    /// # 返回
    /// - `Ok(())`: 帧已接受
    /// - `Err(TaoError::NeedMoreData)`: 编码器内部缓冲区已满, 需要先取出数据包
    fn send_frame(&mut self, frame: Option<&Frame>) -> TaoResult<()>;

    /// 从编码器取出一个压缩数据包
    ///
    /// # 返回
    /// - `Ok(packet)`: 成功取出一个数据包
    /// - `Err(TaoError::NeedMoreData)`: 需要送入更多帧
    /// - `Err(TaoError::Eof)`: 所有数据包已取出
    fn receive_packet(&mut self) -> TaoResult<Packet>;

    /// 刷新编码器, 清空内部状态
    fn flush(&mut self);
}
