//! 编解码器注册表.
//!
//! 对标 FFmpeg 的编解码器注册机制, 支持动态查找和实例化编解码器.

use std::collections::HashMap;

use tao_core::TaoResult;

use crate::codec_id::CodecId;
use crate::decoder::Decoder;
use crate::encoder::Encoder;

/// 解码器工厂函数类型
pub type DecoderFactory = fn() -> TaoResult<Box<dyn Decoder>>;

/// 编码器工厂函数类型
pub type EncoderFactory = fn() -> TaoResult<Box<dyn Encoder>>;

/// 编解码器注册表
///
/// 管理所有已注册的编解码器, 支持按 CodecId 查找并创建实例.
pub struct CodecRegistry {
    /// 解码器工厂映射
    decoders: HashMap<CodecId, Vec<DecoderEntry>>,
    /// 编码器工厂映射
    encoders: HashMap<CodecId, Vec<EncoderEntry>>,
}

/// 解码器注册条目
struct DecoderEntry {
    /// 解码器名称
    name: String,
    /// 工厂函数
    factory: DecoderFactory,
}

/// 编码器注册条目
struct EncoderEntry {
    /// 编码器名称
    name: String,
    /// 工厂函数
    factory: EncoderFactory,
}

impl CodecRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
            encoders: HashMap::new(),
        }
    }

    /// 注册一个解码器
    pub fn register_decoder(
        &mut self,
        codec_id: CodecId,
        name: impl Into<String>,
        factory: DecoderFactory,
    ) {
        self.decoders
            .entry(codec_id)
            .or_default()
            .push(DecoderEntry {
                name: name.into(),
                factory,
            });
    }

    /// 注册一个编码器
    pub fn register_encoder(
        &mut self,
        codec_id: CodecId,
        name: impl Into<String>,
        factory: EncoderFactory,
    ) {
        self.encoders
            .entry(codec_id)
            .or_default()
            .push(EncoderEntry {
                name: name.into(),
                factory,
            });
    }

    /// 创建指定编解码器 ID 的解码器实例
    pub fn create_decoder(&self, codec_id: CodecId) -> TaoResult<Box<dyn Decoder>> {
        let entries = self.decoders.get(&codec_id).ok_or_else(|| {
            tao_core::TaoError::CodecNotFound(format!("未找到 {} 的解码器", codec_id))
        })?;
        // 使用第一个注册的解码器 (优先级最高)
        let entry = &entries[0];
        (entry.factory)()
    }

    /// 创建指定编解码器 ID 的编码器实例
    pub fn create_encoder(&self, codec_id: CodecId) -> TaoResult<Box<dyn Encoder>> {
        let entries = self.encoders.get(&codec_id).ok_or_else(|| {
            tao_core::TaoError::CodecNotFound(format!("未找到 {} 的编码器", codec_id))
        })?;
        let entry = &entries[0];
        (entry.factory)()
    }

    /// 获取所有已注册的解码器名称
    pub fn list_decoders(&self) -> Vec<(CodecId, &str)> {
        let mut result = Vec::new();
        for (id, entries) in &self.decoders {
            for entry in entries {
                result.push((*id, entry.name.as_str()));
            }
        }
        result
    }

    /// 获取所有已注册的编码器名称
    pub fn list_encoders(&self) -> Vec<(CodecId, &str)> {
        let mut result = Vec::new();
        for (id, entries) in &self.encoders {
            for entry in entries {
                result.push((*id, entry.name.as_str()));
            }
        }
        result
    }
}

impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_注册所有编解码器() {
        let mut registry = CodecRegistry::new();
        crate::register_all(&mut registry);

        let decoders = registry.list_decoders();
        let encoders = registry.list_encoders();

        // 9 个解码器: rawvideo + 6 PCM + FLAC + AAC
        assert_eq!(decoders.len(), 9);
        // 7 个编码器: rawvideo + 6 PCM
        assert_eq!(encoders.len(), 8);
    }

    #[test]
    fn test_按codec_id创建解码器() {
        let mut registry = CodecRegistry::new();
        crate::register_all(&mut registry);

        let codec_ids = [
            CodecId::RawVideo,
            CodecId::PcmU8,
            CodecId::PcmS16le,
            CodecId::PcmS16be,
            CodecId::PcmS24le,
            CodecId::PcmS32le,
            CodecId::PcmF32le,
        ];

        for id in codec_ids {
            let dec = registry.create_decoder(id);
            assert!(dec.is_ok(), "创建 {} 解码器失败", id);
            assert_eq!(dec.unwrap().codec_id(), id);
        }
    }

    #[test]
    fn test_按codec_id创建编码器() {
        let mut registry = CodecRegistry::new();
        crate::register_all(&mut registry);

        let codec_ids = [
            CodecId::RawVideo,
            CodecId::PcmU8,
            CodecId::PcmS16le,
            CodecId::PcmS16be,
            CodecId::PcmS24le,
            CodecId::PcmS32le,
            CodecId::PcmF32le,
        ];

        for id in codec_ids {
            let enc = registry.create_encoder(id);
            assert!(enc.is_ok(), "创建 {} 编码器失败", id);
            assert_eq!(enc.unwrap().codec_id(), id);
        }
    }

    #[test]
    fn test_未注册的编解码器返回错误() {
        let registry = CodecRegistry::new();
        assert!(registry.create_decoder(CodecId::H264).is_err());
        assert!(registry.create_encoder(CodecId::Aac).is_err());
    }
}
