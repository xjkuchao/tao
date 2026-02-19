//! MP4 Box (Atom) 头部解析.
//!
//! ISO 14496-12 定义的 Box 结构:
//! ```text
//! Size:       4 bytes (big-endian, 含头部本身)
//! Type:       4 bytes (FourCC)
//! [ExtSize]:  8 bytes (仅当 Size==1 时存在, 64-bit 大小)
//! ```
//!
//! 特殊大小值:
//! - 0: Box 延伸到文件末尾
//! - 1: 使用 64-bit 扩展大小

use tao_core::TaoResult;

use crate::io::IoContext;

/// Box 类型枚举 (常用 FourCC)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxType {
    /// ftyp - 文件类型
    Ftyp,
    /// moov - 影片元数据
    Moov,
    /// mvhd - 影片头部
    Mvhd,
    /// trak - 轨道
    Trak,
    /// tkhd - 轨道头部
    Tkhd,
    /// mdia - 媒体
    Mdia,
    /// mdhd - 媒体头部
    Mdhd,
    /// hdlr - 处理器引用
    Hdlr,
    /// minf - 媒体信息
    Minf,
    /// stbl - 采样表
    Stbl,
    /// stsd - 采样描述
    Stsd,
    /// stts - 时间→采样映射
    Stts,
    /// stsc - 采样→块映射
    Stsc,
    /// stsz - 采样大小
    Stsz,
    /// stco - 块偏移 (32位)
    Stco,
    /// co64 - 块偏移 (64位)
    Co64,
    /// stss - 同步采样
    Stss,
    /// ctts - 合成时间偏移
    Ctts,
    /// mdat - 媒体数据
    Mdat,
    /// free - 自由空间
    Free,
    /// skip - 跳过
    Skip,
    /// 未知 box 类型
    Unknown([u8; 4]),
}

impl BoxType {
    /// 从 4 字节 FourCC 创建
    pub fn from_fourcc(fourcc: &[u8; 4]) -> Self {
        match fourcc {
            b"ftyp" => Self::Ftyp,
            b"moov" => Self::Moov,
            b"mvhd" => Self::Mvhd,
            b"trak" => Self::Trak,
            b"tkhd" => Self::Tkhd,
            b"mdia" => Self::Mdia,
            b"mdhd" => Self::Mdhd,
            b"hdlr" => Self::Hdlr,
            b"minf" => Self::Minf,
            b"stbl" => Self::Stbl,
            b"stsd" => Self::Stsd,
            b"stts" => Self::Stts,
            b"stsc" => Self::Stsc,
            b"stsz" => Self::Stsz,
            b"stco" => Self::Stco,
            b"co64" => Self::Co64,
            b"stss" => Self::Stss,
            b"ctts" => Self::Ctts,
            b"mdat" => Self::Mdat,
            b"free" => Self::Free,
            b"skip" => Self::Skip,
            _ => Self::Unknown(*fourcc),
        }
    }
}

impl std::fmt::Display for BoxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(cc) => {
                let s = std::str::from_utf8(cc).unwrap_or("????");
                write!(f, "{s}")
            }
            _ => write!(f, "{self:?}"),
        }
    }
}

/// 已解析的 Box 头部
pub struct BoxHeader {
    /// Box 总大小 (含头部, 0 表示到文件末尾)
    pub size: u64,
    /// Box 类型
    pub box_type: BoxType,
    /// 头部大小 (8 或 16 字节)
    pub header_size: u64,
}

impl BoxHeader {
    /// 内容区域大小 (不含头部)
    pub fn content_size(&self) -> u64 {
        if self.size == 0 {
            u64::MAX // 延伸到文件末尾
        } else {
            self.size - self.header_size
        }
    }
}

/// 读取一个 Box 头部
pub fn read_box_header(io: &mut IoContext) -> TaoResult<BoxHeader> {
    let size32 = io.read_u32_be()?;
    let fourcc = io.read_tag()?;
    let box_type = BoxType::from_fourcc(&fourcc);

    let (size, header_size) = if size32 == 1 {
        // 64-bit 扩展大小
        let hi = io.read_u32_be()? as u64;
        let lo = io.read_u32_be()? as u64;
        ((hi << 32) | lo, 16u64)
    } else {
        (u64::from(size32), 8u64)
    };

    Ok(BoxHeader {
        size,
        box_type,
        header_size,
    })
}

/// ftyp Box 数据
pub struct FtypBox {
    /// 主品牌
    pub major_brand: [u8; 4],
    /// 次版本号
    pub _minor_version: u32,
    /// 兼容品牌列表
    pub _compatible_brands: Vec<[u8; 4]>,
}

impl FtypBox {
    /// 解析 ftyp box 内容
    pub fn parse(io: &mut IoContext, content_size: u64) -> TaoResult<Self> {
        let major_brand = io.read_tag()?;
        let minor_version = io.read_u32_be()?;

        let remaining = content_size.saturating_sub(8);
        let brand_count = (remaining / 4) as usize;
        let mut compatible_brands = Vec::with_capacity(brand_count);
        for _ in 0..brand_count {
            compatible_brands.push(io.read_tag()?);
        }

        Ok(Self {
            major_brand,
            _minor_version: minor_version,
            _compatible_brands: compatible_brands,
        })
    }

    /// 获取主品牌字符串
    pub fn major_brand_str(&self) -> String {
        String::from_utf8_lossy(&self.major_brand).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MemoryBackend;

    #[test]
    fn test_box_type_identify() {
        assert_eq!(BoxType::from_fourcc(b"ftyp"), BoxType::Ftyp);
        assert_eq!(BoxType::from_fourcc(b"moov"), BoxType::Moov);
        assert_eq!(BoxType::from_fourcc(b"mdat"), BoxType::Mdat);
        assert!(matches!(BoxType::from_fourcc(b"xxxx"), BoxType::Unknown(_)));
    }

    #[test]
    fn test_read_box_header() {
        // 构造一个 size=20, type="ftyp" 的 box
        let mut data = Vec::new();
        data.extend_from_slice(&20u32.to_be_bytes()); // size
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 12]); // content

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));

        let header = read_box_header(&mut io).unwrap();
        assert_eq!(header.box_type, BoxType::Ftyp);
        assert_eq!(header.size, 20);
        assert_eq!(header.header_size, 8);
        assert_eq!(header.content_size(), 12);
    }

    #[test]
    fn test_read_64bit_box_header() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // size = 1 (使用扩展大小)
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&0u32.to_be_bytes()); // 扩展大小高 32 位
        data.extend_from_slice(&1000u32.to_be_bytes()); // 扩展大小低 32 位
        data.extend_from_slice(&[0u8; 984]); // 内容

        let backend = MemoryBackend::from_data(data);
        let mut io = IoContext::new(Box::new(backend));

        let header = read_box_header(&mut io).unwrap();
        assert_eq!(header.box_type, BoxType::Mdat);
        assert_eq!(header.size, 1000);
        assert_eq!(header.header_size, 16);
        assert_eq!(header.content_size(), 984);
    }

    #[test]
    fn test_ftyp_parse() {
        let mut content = Vec::new();
        content.extend_from_slice(b"isom"); // major brand
        content.extend_from_slice(&0u32.to_be_bytes()); // minor version
        content.extend_from_slice(b"isom"); // compatible
        content.extend_from_slice(b"mp41"); // compatible

        let backend = MemoryBackend::from_data(content.clone());
        let mut io = IoContext::new(Box::new(backend));

        let ftyp = FtypBox::parse(&mut io, content.len() as u64).unwrap();
        assert_eq!(&ftyp.major_brand, b"isom");
        assert_eq!(ftyp._minor_version, 0);
        assert_eq!(ftyp._compatible_brands.len(), 2);
        assert_eq!(&ftyp._compatible_brands[0], b"isom");
        assert_eq!(&ftyp._compatible_brands[1], b"mp41");
    }
}
