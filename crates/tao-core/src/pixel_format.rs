//! 像素格式定义.
//!
//! 对标 FFmpeg 的 `AVPixelFormat`, 定义了视频帧中像素的存储格式.

use std::fmt;

/// 像素格式
///
/// 定义了视频帧中每个像素的数据排列方式.
/// 命名规则: 颜色空间 + 位深 + 排列方式 (P=Planar, LE/BE=字节序).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// 未指定
    None,

    // ========================
    // YUV 平面格式 (Planar)
    // ========================
    /// YUV 4:2:0 平面格式, 8 位 (最常用, H.264/H.265 默认)
    Yuv420p,
    /// YUV 4:2:2 平面格式, 8 位
    Yuv422p,
    /// YUV 4:4:4 平面格式, 8 位
    Yuv444p,
    /// YUV 4:2:0 平面格式, 10 位小端
    Yuv420p10le,
    /// YUV 4:2:0 平面格式, 10 位大端
    Yuv420p10be,
    /// YUV 4:2:2 平面格式, 10 位小端
    Yuv422p10le,
    /// YUV 4:4:4 平面格式, 10 位小端
    Yuv444p10le,

    // ========================
    // YUV 半平面格式 (Semi-Planar / NV)
    // ========================
    /// NV12: Y 平面 + UV 交错, 4:2:0, 8 位 (硬件解码常用)
    Nv12,
    /// NV21: Y 平面 + VU 交错, 4:2:0, 8 位
    Nv21,

    // ========================
    // RGB 打包格式 (Packed)
    // ========================
    /// RGB 各 8 位, 打包
    Rgb24,
    /// BGR 各 8 位, 打包
    Bgr24,
    /// RGBA 各 8 位, 打包
    Rgba,
    /// BGRA 各 8 位, 打包
    Bgra,
    /// ARGB 各 8 位, 打包
    Argb,

    // ========================
    // 灰度格式
    // ========================
    /// 灰度 8 位
    Gray8,
    /// 灰度 16 位小端
    Gray16le,

    // ========================
    // 浮点格式
    // ========================
    /// RGB 各 32 位浮点
    Rgbf32le,
}

impl PixelFormat {
    /// 获取每个像素占用的位数 (packed 格式)
    ///
    /// 对于平面格式, 返回单个 Y/U/V 分量的位深.
    pub const fn bits_per_component(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Yuv420p
            | Self::Yuv422p
            | Self::Yuv444p
            | Self::Nv12
            | Self::Nv21
            | Self::Gray8 => 8,
            Self::Yuv420p10le | Self::Yuv420p10be | Self::Yuv422p10le | Self::Yuv444p10le => 10,
            Self::Gray16le => 16,
            Self::Rgb24 | Self::Bgr24 => 8,
            Self::Rgba | Self::Bgra | Self::Argb => 8,
            Self::Rgbf32le => 32,
        }
    }

    /// 获取色度子采样 (水平, 垂直)
    ///
    /// 返回 (log2 水平子采样, log2 垂直子采样).
    /// 例如 YUV420 返回 (1, 1), 表示色度分辨率为亮度的 1/2 x 1/2.
    pub const fn chroma_subsampling(&self) -> (u32, u32) {
        match self {
            Self::Yuv420p | Self::Yuv420p10le | Self::Yuv420p10be | Self::Nv12 | Self::Nv21 => {
                (1, 1)
            }
            Self::Yuv422p | Self::Yuv422p10le => (1, 0),
            Self::Yuv444p | Self::Yuv444p10le => (0, 0),
            _ => (0, 0),
        }
    }

    /// 是否为平面格式 (Y/U/V 存储在不同平面)
    pub const fn is_planar(&self) -> bool {
        matches!(
            self,
            Self::Yuv420p
                | Self::Yuv422p
                | Self::Yuv444p
                | Self::Yuv420p10le
                | Self::Yuv420p10be
                | Self::Yuv422p10le
                | Self::Yuv444p10le
        )
    }

    /// 平面数量
    pub const fn plane_count(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Yuv420p
            | Self::Yuv422p
            | Self::Yuv444p
            | Self::Yuv420p10le
            | Self::Yuv420p10be
            | Self::Yuv422p10le
            | Self::Yuv444p10le => 3,
            Self::Nv12 | Self::Nv21 => 2,
            Self::Gray8 | Self::Gray16le => 1,
            Self::Rgb24 | Self::Bgr24 | Self::Rgba | Self::Bgra | Self::Argb | Self::Rgbf32le => 1,
        }
    }

    /// 计算指定平面每行的字节数 (linesize / stride)
    ///
    /// # 参数
    /// - `plane`: 平面索引 (从 0 开始)
    /// - `width`: 图像宽度 (像素)
    ///
    /// # 返回
    /// - `Some(bytes)`: 该平面每行的字节数
    /// - `None`: 格式为 None 或平面索引超出范围
    pub fn plane_linesize(&self, plane: usize, width: u32) -> Option<usize> {
        if *self == Self::None || plane >= self.plane_count() as usize {
            return None;
        }
        let w = width as usize;
        let (sub_h, _) = self.chroma_subsampling();
        let sub_h = sub_h as usize;
        Some(match self {
            // 3 平面 YUV 8-bit
            Self::Yuv420p | Self::Yuv422p | Self::Yuv444p => {
                if plane == 0 {
                    w
                } else {
                    w >> sub_h
                }
            }
            // 3 平面 YUV 10-bit (每分量 2 字节)
            Self::Yuv420p10le | Self::Yuv420p10be | Self::Yuv422p10le | Self::Yuv444p10le => {
                if plane == 0 {
                    w * 2
                } else {
                    (w >> sub_h) * 2
                }
            }
            // 半平面 NV12/NV21: plane0=Y, plane1=UV 交错
            Self::Nv12 | Self::Nv21 => w, // plane1: (w/2)*2 = w
            // Packed RGB
            Self::Rgb24 | Self::Bgr24 => w * 3,
            Self::Rgba | Self::Bgra | Self::Argb => w * 4,
            // 灰度
            Self::Gray8 => w,
            Self::Gray16le => w * 2,
            // 浮点 RGB (3 * 4 bytes)
            Self::Rgbf32le => w * 12,
            Self::None => unreachable!(),
        })
    }

    /// 计算指定平面的行数
    ///
    /// # 参数
    /// - `plane`: 平面索引 (从 0 开始)
    /// - `height`: 图像高度 (像素)
    ///
    /// # 返回
    /// - `Some(rows)`: 该平面的行数
    /// - `None`: 格式为 None 或平面索引超出范围
    pub fn plane_height(&self, plane: usize, height: u32) -> Option<usize> {
        if *self == Self::None || plane >= self.plane_count() as usize {
            return None;
        }
        let h = height as usize;
        let (_, sub_v) = self.chroma_subsampling();
        let sub_v = sub_v as usize;
        Some(match self {
            Self::Yuv420p
            | Self::Yuv422p
            | Self::Yuv444p
            | Self::Yuv420p10le
            | Self::Yuv420p10be
            | Self::Yuv422p10le
            | Self::Yuv444p10le => {
                if plane == 0 {
                    h
                } else {
                    h >> sub_v
                }
            }
            // NV12/NV21: plane1 高度为 h/2
            Self::Nv12 | Self::Nv21 => {
                if plane == 0 {
                    h
                } else {
                    h >> sub_v
                }
            }
            // 单平面格式
            _ => h,
        })
    }

    /// 计算整帧的字节数
    ///
    /// # 参数
    /// - `width`: 图像宽度 (像素)
    /// - `height`: 图像高度 (像素)
    ///
    /// # 返回
    /// - `Some(bytes)`: 整帧字节数
    /// - `None`: 格式为 None
    pub fn frame_size(&self, width: u32, height: u32) -> Option<usize> {
        if *self == Self::None {
            return None;
        }
        let mut total = 0usize;
        for plane in 0..self.plane_count() as usize {
            let linesize = self.plane_linesize(plane, width)?;
            let plane_h = self.plane_height(plane, height)?;
            total += linesize * plane_h;
        }
        Some(total)
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::None => "none",
            Self::Yuv420p => "yuv420p",
            Self::Yuv422p => "yuv422p",
            Self::Yuv444p => "yuv444p",
            Self::Yuv420p10le => "yuv420p10le",
            Self::Yuv420p10be => "yuv420p10be",
            Self::Yuv422p10le => "yuv422p10le",
            Self::Yuv444p10le => "yuv444p10le",
            Self::Nv12 => "nv12",
            Self::Nv21 => "nv21",
            Self::Rgb24 => "rgb24",
            Self::Bgr24 => "bgr24",
            Self::Rgba => "rgba",
            Self::Bgra => "bgra",
            Self::Argb => "argb",
            Self::Gray8 => "gray8",
            Self::Gray16le => "gray16le",
            Self::Rgbf32le => "rgbf32le",
        };
        write!(f, "{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yuv420p_frame_size() {
        let pf = PixelFormat::Yuv420p;
        // 1920x1080: Y=1920*1080 + U=960*540 + V=960*540 = 1920*1080*3/2
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 3 / 2));
        // plane linesizes
        assert_eq!(pf.plane_linesize(0, 1920), Some(1920));
        assert_eq!(pf.plane_linesize(1, 1920), Some(960));
        assert_eq!(pf.plane_linesize(2, 1920), Some(960));
        // plane heights
        assert_eq!(pf.plane_height(0, 1080), Some(1080));
        assert_eq!(pf.plane_height(1, 1080), Some(540));
        assert_eq!(pf.plane_height(2, 1080), Some(540));
    }

    #[test]
    fn test_yuv420p10le_frame_size() {
        let pf = PixelFormat::Yuv420p10le;
        // 10bit: 每分量 2 字节
        assert_eq!(pf.plane_linesize(0, 1920), Some(3840));
        assert_eq!(pf.plane_linesize(1, 1920), Some(1920));
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 3));
    }

    #[test]
    fn test_nv12_frame_size() {
        let pf = PixelFormat::Nv12;
        // plane0=Y: w*h, plane1=UV交错: w*(h/2)
        assert_eq!(pf.plane_linesize(0, 1920), Some(1920));
        assert_eq!(pf.plane_linesize(1, 1920), Some(1920));
        assert_eq!(pf.plane_height(0, 1080), Some(1080));
        assert_eq!(pf.plane_height(1, 1080), Some(540));
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 3 / 2));
    }

    #[test]
    fn test_rgb24_frame_size() {
        let pf = PixelFormat::Rgb24;
        assert_eq!(pf.plane_linesize(0, 1920), Some(5760));
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 3));
    }

    #[test]
    fn test_rgba_frame_size() {
        let pf = PixelFormat::Rgba;
        assert_eq!(pf.plane_linesize(0, 1920), Some(7680));
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 4));
    }

    #[test]
    fn test_gray8_frame_size() {
        assert_eq!(PixelFormat::Gray8.frame_size(320, 240), Some(320 * 240));
    }

    #[test]
    fn test_gray16le_frame_size() {
        assert_eq!(
            PixelFormat::Gray16le.frame_size(320, 240),
            Some(320 * 240 * 2)
        );
    }

    #[test]
    fn test_rgbf32le_frame_size() {
        assert_eq!(
            PixelFormat::Rgbf32le.frame_size(320, 240),
            Some(320 * 240 * 12)
        );
    }

    #[test]
    fn test_none_return_none() {
        assert_eq!(PixelFormat::None.frame_size(1920, 1080), None);
        assert_eq!(PixelFormat::None.plane_linesize(0, 1920), None);
        assert_eq!(PixelFormat::None.plane_height(0, 1080), None);
    }

    #[test]
    fn test_plane_index_out_of_bounds_return_none() {
        assert_eq!(PixelFormat::Rgb24.plane_linesize(1, 1920), None);
        assert_eq!(PixelFormat::Yuv420p.plane_linesize(3, 1920), None);
        assert_eq!(PixelFormat::Nv12.plane_linesize(2, 1920), None);
    }

    #[test]
    fn test_yuv422p_frame_size() {
        let pf = PixelFormat::Yuv422p;
        // 422: 水平子采样1, 垂直不子采样
        assert_eq!(pf.plane_linesize(0, 1920), Some(1920));
        assert_eq!(pf.plane_linesize(1, 1920), Some(960));
        assert_eq!(pf.plane_height(0, 1080), Some(1080));
        assert_eq!(pf.plane_height(1, 1080), Some(1080));
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 2));
    }

    #[test]
    fn test_yuv444p_frame_size() {
        let pf = PixelFormat::Yuv444p;
        assert_eq!(pf.frame_size(1920, 1080), Some(1920 * 1080 * 3));
    }
}
