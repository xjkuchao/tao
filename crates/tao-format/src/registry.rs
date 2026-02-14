//! 容器格式注册表.
//!
//! 管理所有已注册的解封装器/封装器, 支持按格式标识查找和自动探测.

use std::collections::HashMap;

use tao_core::TaoResult;

use crate::demuxer::Demuxer;
use crate::format_id::FormatId;
use crate::io::IoContext;
use crate::muxer::Muxer;
use crate::probe::{FormatProbe, ProbeResult};

/// 解封装器工厂函数类型
pub type DemuxerFactory = fn() -> TaoResult<Box<dyn Demuxer>>;

/// 封装器工厂函数类型
pub type MuxerFactory = fn() -> TaoResult<Box<dyn Muxer>>;

/// 容器格式注册表
pub struct FormatRegistry {
    /// 解封装器工厂映射
    demuxers: HashMap<FormatId, DemuxerEntry>,
    /// 封装器工厂映射
    muxers: HashMap<FormatId, MuxerEntry>,
    /// 格式探测器列表
    probes: Vec<Box<dyn FormatProbe + Send>>,
}

/// 解封装器注册条目
struct DemuxerEntry {
    /// 格式名称
    name: String,
    /// 工厂函数
    factory: DemuxerFactory,
}

/// 封装器注册条目
struct MuxerEntry {
    /// 格式名称
    name: String,
    /// 工厂函数
    factory: MuxerFactory,
}

impl FormatRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            demuxers: HashMap::new(),
            muxers: HashMap::new(),
            probes: Vec::new(),
        }
    }

    /// 注册一个解封装器
    pub fn register_demuxer(
        &mut self,
        format_id: FormatId,
        name: impl Into<String>,
        factory: DemuxerFactory,
    ) {
        self.demuxers.insert(
            format_id,
            DemuxerEntry {
                name: name.into(),
                factory,
            },
        );
    }

    /// 注册一个封装器
    pub fn register_muxer(
        &mut self,
        format_id: FormatId,
        name: impl Into<String>,
        factory: MuxerFactory,
    ) {
        self.muxers.insert(
            format_id,
            MuxerEntry {
                name: name.into(),
                factory,
            },
        );
    }

    /// 注册一个格式探测器
    pub fn register_probe(&mut self, probe: Box<dyn FormatProbe + Send>) {
        self.probes.push(probe);
    }

    /// 创建指定格式的解封装器实例
    pub fn create_demuxer(&self, format_id: FormatId) -> TaoResult<Box<dyn Demuxer>> {
        let entry = self.demuxers.get(&format_id).ok_or_else(|| {
            tao_core::TaoError::FormatNotFound(format!("未找到 {} 的解封装器", format_id))
        })?;
        (entry.factory)()
    }

    /// 创建指定格式的封装器实例
    pub fn create_muxer(&self, format_id: FormatId) -> TaoResult<Box<dyn Muxer>> {
        let entry = self.muxers.get(&format_id).ok_or_else(|| {
            tao_core::TaoError::FormatNotFound(format!("未找到 {} 的封装器", format_id))
        })?;
        (entry.factory)()
    }

    /// 探测数据的容器格式
    ///
    /// 遍历所有已注册的探测器, 返回置信度最高的结果.
    pub fn probe(&self, data: &[u8], filename: Option<&str>) -> Option<ProbeResult> {
        let mut best: Option<ProbeResult> = None;
        for probe in &self.probes {
            if let Some(score) = probe.probe(data, filename) {
                let is_better = best.as_ref().is_none_or(|b| score > b.score);
                if is_better {
                    best = Some(ProbeResult {
                        format_id: probe.format_id(),
                        score,
                    });
                }
            }
        }
        best
    }

    /// 获取所有已注册的解封装器名称
    pub fn list_demuxers(&self) -> Vec<(FormatId, &str)> {
        self.demuxers
            .iter()
            .map(|(id, entry)| (*id, entry.name.as_str()))
            .collect()
    }

    /// 获取所有已注册的封装器名称
    pub fn list_muxers(&self) -> Vec<(FormatId, &str)> {
        self.muxers
            .iter()
            .map(|(id, entry)| (*id, entry.name.as_str()))
            .collect()
    }

    /// 探测输入文件格式 (不打开解封装器)
    ///
    /// 读取文件头部数据, 自动探测格式, 然后 seek 回起始位置.
    pub fn probe_input(
        &self,
        io: &mut IoContext,
        filename: Option<&str>,
    ) -> TaoResult<ProbeResult> {
        let probe_size = io.size().unwrap_or(8192).min(8192) as usize;
        let probe_size = probe_size.max(12); // 至少读取 12 字节
        let probe_buf = io.read_bytes(probe_size)?;

        let result = self.probe(&probe_buf, filename).ok_or_else(|| {
            tao_core::TaoError::FormatNotFound("无法识别输入文件格式".to_string())
        })?;

        // seek 回起始位置, 供后续 demuxer 读取
        io.seek(std::io::SeekFrom::Start(0))?;

        Ok(result)
    }

    /// 根据文件自动探测格式并创建解封装器
    ///
    /// 自动探测格式, 创建对应的解封装器, 并调用 `open()` 解析头部.
    pub fn open_input(
        &self,
        io: &mut IoContext,
        filename: Option<&str>,
    ) -> TaoResult<Box<dyn Demuxer>> {
        let result = self.probe_input(io, filename)?;
        let mut demuxer = self.create_demuxer(result.format_id)?;
        demuxer.open(io)?;
        Ok(demuxer)
    }
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::new()
    }
}
