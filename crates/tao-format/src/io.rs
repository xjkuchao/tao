//! I/O 抽象层.
//!
//! 对标 FFmpeg 的 `AVIOContext`, 提供统一的读写接口,
//! 支持文件、内存缓冲区、网络流等不同后端.

use std::io::{self, Read, Seek, Write};
use tao_core::TaoResult;

/// I/O 上下文
///
/// 封装底层 I/O 操作, 为解封装器/封装器提供统一的数据读写接口.
pub struct IoContext {
    /// 内部 I/O 实现
    inner: Box<dyn IoBackend>,
    /// 读缓冲区
    buffer: Vec<u8>,
    /// 缓冲区中的有效数据长度
    buf_len: usize,
    /// 缓冲区当前读取位置
    buf_pos: usize,
}

/// I/O 后端 trait
///
/// 实现此 trait 以支持不同的 I/O 来源 (文件、内存、网络等).
pub trait IoBackend: Send {
    /// 读取数据到缓冲区
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// 写入数据
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    /// 全部写入
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
    /// 定位 (seek)
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64>;
    /// 获取当前位置
    fn position(&mut self) -> io::Result<u64>;
    /// 获取总大小 (如果可知)
    fn size(&self) -> Option<u64>;
    /// 是否支持 seek
    fn is_seekable(&self) -> bool;
}

/// 默认缓冲区大小 (32 KB)
const DEFAULT_BUFFER_SIZE: usize = 32 * 1024;

impl IoContext {
    /// 从 I/O 后端创建上下文
    pub fn new(backend: Box<dyn IoBackend>) -> Self {
        Self {
            inner: backend,
            buffer: vec![0u8; DEFAULT_BUFFER_SIZE],
            buf_len: 0,
            buf_pos: 0,
        }
    }

    /// 从文件路径打开 (只读)
    pub fn open_read(path: &str) -> TaoResult<Self> {
        let file = std::fs::File::open(path)?;
        Ok(Self::new(Box::new(FileBackend::new(file))))
    }

    /// 从文件路径打开 (写入)
    pub fn open_write(path: &str) -> TaoResult<Self> {
        let file = std::fs::File::create(path)?;
        Ok(Self::new(Box::new(FileBackend::new(file))))
    }

    /// 从文件路径打开 (读写)
    pub fn open_read_write(path: &str) -> TaoResult<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self::new(Box::new(FileBackend::new(file))))
    }

    // ========================
    // 读取方法
    // ========================

    /// 读取指定字节数
    pub fn read_exact(&mut self, buf: &mut [u8]) -> TaoResult<()> {
        let mut total_read = 0;
        while total_read < buf.len() {
            let buffered = self.buf_len - self.buf_pos;
            if buffered > 0 {
                let to_copy = buffered.min(buf.len() - total_read);
                buf[total_read..total_read + to_copy]
                    .copy_from_slice(&self.buffer[self.buf_pos..self.buf_pos + to_copy]);
                self.buf_pos += to_copy;
                total_read += to_copy;
            } else {
                self.buf_pos = 0;
                self.buf_len = self.inner.read(&mut self.buffer)?;
                if self.buf_len == 0 {
                    return Err(tao_core::TaoError::Eof);
                }
            }
        }
        Ok(())
    }

    /// 读取 1 个字节
    pub fn read_u8(&mut self) -> TaoResult<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    /// 读取 u16 小端
    pub fn read_u16_le(&mut self) -> TaoResult<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    /// 读取 u32 小端
    pub fn read_u32_le(&mut self) -> TaoResult<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    /// 读取 i32 小端
    pub fn read_i32_le(&mut self) -> TaoResult<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    /// 读取 u16 大端
    pub fn read_u16_be(&mut self) -> TaoResult<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }

    /// 读取 u24 大端 (3 字节无符号整数)
    pub fn read_u24_be(&mut self) -> TaoResult<u32> {
        let mut buf = [0u8; 3];
        self.read_exact(&mut buf)?;
        Ok((u32::from(buf[0]) << 16) | (u32::from(buf[1]) << 8) | u32::from(buf[2]))
    }

    /// 读取 u32 大端
    pub fn read_u32_be(&mut self) -> TaoResult<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    }

    /// 读取 i16 大端
    pub fn read_i16_be(&mut self) -> TaoResult<i16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    /// 读取 i32 大端
    pub fn read_i32_be(&mut self) -> TaoResult<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_be_bytes(buf))
    }

    /// 读取 4 字节标签 (FourCC)
    pub fn read_tag(&mut self) -> TaoResult<[u8; 4]> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// 读取指定数量的字节
    pub fn read_bytes(&mut self, count: usize) -> TaoResult<Vec<u8>> {
        let mut buf = vec![0u8; count];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// 跳过指定字节数
    pub fn skip(&mut self, count: usize) -> TaoResult<()> {
        // 先尝试消耗缓冲区中的数据
        let buffered = self.buf_len - self.buf_pos;
        if count <= buffered {
            self.buf_pos += count;
            return Ok(());
        }

        // 跳过缓冲区中所有剩余数据
        let remaining = count - buffered;
        self.buf_pos = self.buf_len;

        // 如果支持 seek, 直接跳过
        if self.inner.is_seekable() {
            self.inner.seek(io::SeekFrom::Current(remaining as i64))?;
        } else {
            // 逐块丢弃读取的数据
            let mut left = remaining;
            while left > 0 {
                let to_read = left.min(self.buffer.len());
                self.buf_len = self.inner.read(&mut self.buffer[..to_read])?;
                if self.buf_len == 0 {
                    return Err(tao_core::TaoError::Eof);
                }
                left -= self.buf_len;
            }
            self.buf_pos = 0;
            self.buf_len = 0;
        }
        Ok(())
    }

    // ========================
    // 写入方法
    // ========================

    /// 写入全部数据
    pub fn write_all(&mut self, buf: &[u8]) -> TaoResult<()> {
        self.inner.write_all(buf)?;
        Ok(())
    }

    /// 写入 u8
    pub fn write_u8(&mut self, v: u8) -> TaoResult<()> {
        self.write_all(&[v])
    }

    /// 写入 u16 小端
    pub fn write_u16_le(&mut self, v: u16) -> TaoResult<()> {
        self.write_all(&v.to_le_bytes())
    }

    /// 写入 u32 小端
    pub fn write_u32_le(&mut self, v: u32) -> TaoResult<()> {
        self.write_all(&v.to_le_bytes())
    }

    /// 写入 i32 小端
    pub fn write_i32_le(&mut self, v: i32) -> TaoResult<()> {
        self.write_all(&v.to_le_bytes())
    }

    /// 写入 u16 大端
    pub fn write_u16_be(&mut self, v: u16) -> TaoResult<()> {
        self.write_all(&v.to_be_bytes())
    }

    /// 写入 u32 大端
    pub fn write_u32_be(&mut self, v: u32) -> TaoResult<()> {
        self.write_all(&v.to_be_bytes())
    }

    /// 写入 u64 大端
    pub fn write_u64_be(&mut self, v: u64) -> TaoResult<()> {
        self.write_all(&v.to_be_bytes())
    }

    /// 写入 i16 大端
    pub fn write_i16_be(&mut self, v: i16) -> TaoResult<()> {
        self.write_all(&v.to_be_bytes())
    }

    /// 写入 i32 大端
    pub fn write_i32_be(&mut self, v: i32) -> TaoResult<()> {
        self.write_all(&v.to_be_bytes())
    }

    /// 写入 4 字节标签 (FourCC)
    pub fn write_tag(&mut self, tag: &[u8; 4]) -> TaoResult<()> {
        self.write_all(tag)
    }

    // ========================
    // 定位方法
    // ========================

    /// 定位 (seek)
    ///
    /// 注意: seek 会清空读缓冲区.
    pub fn seek(&mut self, pos: io::SeekFrom) -> TaoResult<u64> {
        // 清空读缓冲区
        self.buf_pos = 0;
        self.buf_len = 0;
        Ok(self.inner.seek(pos)?)
    }

    /// 获取当前位置
    ///
    /// 考虑读缓冲区中尚未消耗的数据量.
    pub fn position(&mut self) -> TaoResult<u64> {
        let raw_pos = self.inner.position()?;
        let buffered = (self.buf_len - self.buf_pos) as u64;
        Ok(raw_pos - buffered)
    }

    /// 是否支持随机访问
    pub fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    /// 获取总大小
    pub fn size(&self) -> Option<u64> {
        self.inner.size()
    }
}

/// 文件 I/O 后端
struct FileBackend {
    file: std::fs::File,
    size: Option<u64>,
}

impl FileBackend {
    fn new(file: std::fs::File) -> Self {
        let size = file.metadata().ok().map(|m| m.len());
        Self { file, size }
    }
}

impl IoBackend for FileBackend {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)
    }

    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }

    fn position(&mut self) -> io::Result<u64> {
        self.file.stream_position()
    }

    fn size(&self) -> Option<u64> {
        self.size
    }

    fn is_seekable(&self) -> bool {
        true
    }
}

/// 内存缓冲区 I/O 后端
///
/// 用于测试和内存中处理.
pub struct MemoryBackend {
    /// 数据缓冲区
    data: Vec<u8>,
    /// 当前位置
    pos: usize,
}

impl MemoryBackend {
    /// 从已有数据创建 (用于读取)
    pub fn from_data(data: Vec<u8>) -> Self {
        Self { data, pos: 0 }
    }

    /// 创建空缓冲区 (用于写入)
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            pos: 0,
        }
    }

    /// 获取内部数据的引用
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// 消耗自身, 返回内部数据
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl IoBackend for MemoryBackend {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.data.len().saturating_sub(self.pos);
        let to_read = buf.len().min(available);
        if to_read == 0 {
            return Ok(0);
        }
        buf[..to_read].copy_from_slice(&self.data[self.pos..self.pos + to_read]);
        self.pos += to_read;
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // 如果当前位置在数据末尾, 追加
        if self.pos >= self.data.len() {
            self.data.extend_from_slice(buf);
        } else {
            // 覆盖已有数据
            let overlap = (self.data.len() - self.pos).min(buf.len());
            self.data[self.pos..self.pos + overlap].copy_from_slice(&buf[..overlap]);
            if buf.len() > overlap {
                self.data.extend_from_slice(&buf[overlap..]);
            }
        }
        self.pos += buf.len();
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write(buf)?;
        Ok(())
    }

    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            io::SeekFrom::Start(offset) => offset as i64,
            io::SeekFrom::End(offset) => self.data.len() as i64 + offset,
            io::SeekFrom::Current(offset) => self.pos as i64 + offset,
        };
        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek 位置不能为负",
            ));
        }
        self.pos = new_pos as usize;
        Ok(self.pos as u64)
    }

    fn position(&mut self) -> io::Result<u64> {
        Ok(self.pos as u64)
    }

    fn size(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }

    fn is_seekable(&self) -> bool {
        true
    }
}
