//! 统一错误类型定义.
//!
//! 所有 Tao crate 共用的错误类型, 支持跨模块传播.

use thiserror::Error;

/// Tao 框架统一错误类型
#[derive(Debug, Error)]
pub enum TaoError {
    /// 无效参数
    #[error("无效参数: {0}")]
    InvalidArgument(String),

    /// 不支持的操作
    #[error("不支持的操作: {0}")]
    Unsupported(String),

    /// 编解码器错误
    #[error("编解码器错误: {0}")]
    Codec(String),

    /// 容器格式错误
    #[error("格式错误: {0}")]
    Format(String),

    /// I/O 错误
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    /// 数据不足, 需要更多输入
    #[error("数据不足, 需要更多输入")]
    NeedMoreData,

    /// 已到达流末尾
    #[error("已到达流末尾")]
    Eof,

    /// 内存分配失败
    #[error("内存分配失败: {0}")]
    OutOfMemory(String),

    /// 未找到指定的编解码器
    #[error("未找到编解码器: {0}")]
    CodecNotFound(String),

    /// 未找到指定的容器格式
    #[error("未找到容器格式: {0}")]
    FormatNotFound(String),

    /// 未找到指定的滤镜
    #[error("未找到滤镜: {0}")]
    FilterNotFound(String),

    /// 未找到指定的流
    #[error("未找到流: 索引 {0}")]
    StreamNotFound(usize),

    /// 无效数据 (损坏的码流等)
    #[error("无效数据: {0}")]
    InvalidData(String),

    /// 功能未实现
    #[error("功能未实现: {0}")]
    NotImplemented(String),

    /// 内部错误 (不应发生)
    #[error("内部错误: {0}")]
    Internal(String),
}

/// Tao 框架统一 Result 类型
pub type TaoResult<T> = Result<T, TaoError>;
