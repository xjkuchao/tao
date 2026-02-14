//! 压缩数据包 (Packet).
//!
//! 对标 FFmpeg 的 `AVPacket`, 表示从容器格式中读取的一帧压缩数据.

use bytes::Bytes;
use tao_core::Rational;

/// 压缩数据包
///
/// 从容器格式中读取的一帧压缩数据, 需要送入解码器进行解码.
/// 一个 Packet 通常对应一帧视频或若干帧音频的压缩数据.
#[derive(Debug, Clone)]
pub struct Packet {
    /// 压缩数据
    pub data: Bytes,
    /// 显示时间戳 (PTS)
    pub pts: i64,
    /// 解码时间戳 (DTS)
    pub dts: i64,
    /// 数据包时长 (以 time_base 为单位)
    pub duration: i64,
    /// 时间基
    pub time_base: Rational,
    /// 所属流的索引
    pub stream_index: usize,
    /// 是否为关键帧
    pub is_keyframe: bool,
    /// 在容器中的字节偏移量 (-1 表示未知)
    pub pos: i64,
}

impl Packet {
    /// 创建空数据包
    pub fn empty() -> Self {
        Self {
            data: Bytes::new(),
            pts: tao_core::timestamp::NOPTS_VALUE,
            dts: tao_core::timestamp::NOPTS_VALUE,
            duration: 0,
            time_base: Rational::UNDEFINED,
            stream_index: 0,
            is_keyframe: false,
            pos: -1,
        }
    }

    /// 从数据创建数据包
    pub fn from_data(data: impl Into<Bytes>) -> Self {
        Self {
            data: data.into(),
            ..Self::empty()
        }
    }

    /// 数据大小 (字节)
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// 是否为空包 (flush packet)
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
