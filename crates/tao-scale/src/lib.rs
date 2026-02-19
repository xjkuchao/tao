//! # tao-scale
//!
//! Tao 多媒体框架图像缩放与像素格式转换库.
//!
//! 本 crate 对标 FFmpeg 的 libswscale, 提供:
//! - 像素格式转换 (YUV ↔ RGB, 位深转换等)
//! - 图像缩放 (双线性, 双三次, Lanczos 等算法, 待实现)

pub mod convert;
pub mod scale;

use tao_core::{PixelFormat, TaoResult};

/// 缩放算法
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScaleAlgorithm {
    /// 最近邻 (速度最快, 质量最低)
    NearestNeighbor,
    /// 双线性插值 (速度与质量均衡)
    #[default]
    Bilinear,
    /// 双三次插值 (质量较高)
    Bicubic,
    /// Lanczos (质量最高, 速度较慢)
    Lanczos,
    /// Area 平均 (缩小时效果好)
    Area,
}

/// 图像缩放/转换上下文
///
/// 配置一次后可多次复用, 用于在不同像素格式和分辨率之间转换.
pub struct ScaleContext {
    /// 源宽度
    pub src_width: u32,
    /// 源高度
    pub src_height: u32,
    /// 源像素格式
    pub src_format: PixelFormat,
    /// 目标宽度
    pub dst_width: u32,
    /// 目标高度
    pub dst_height: u32,
    /// 目标像素格式
    pub dst_format: PixelFormat,
    /// 缩放算法
    pub algorithm: ScaleAlgorithm,
}

impl ScaleContext {
    /// 创建新的缩放上下文
    pub fn new(
        src_width: u32,
        src_height: u32,
        src_format: PixelFormat,
        dst_width: u32,
        dst_height: u32,
        dst_format: PixelFormat,
        algorithm: ScaleAlgorithm,
    ) -> Self {
        Self {
            src_width,
            src_height,
            src_format,
            dst_width,
            dst_height,
            dst_format,
            algorithm,
        }
    }

    /// 执行图像缩放/格式转换
    ///
    /// # 参数
    /// - `src_data`: 源图像各平面数据
    /// - `src_linesize`: 源图像各平面行字节数
    /// - `dst_data`: 目标图像各平面数据 (输出)
    /// - `dst_linesize`: 目标图像各平面行字节数
    pub fn scale(
        &self,
        src_data: &[&[u8]],
        src_linesize: &[usize],
        dst_data: &mut [&mut [u8]],
        dst_linesize: &[usize],
    ) -> TaoResult<()> {
        // 分辨率相同时只做格式转换
        if self.src_width == self.dst_width && self.src_height == self.dst_height {
            if self.src_format == self.dst_format {
                // 同格式同分辨率: 直接复制
                return self.copy_planes(src_data, src_linesize, dst_data, dst_linesize);
            }

            // 像素格式转换
            let input = convert::ConvertInput {
                planes: src_data.to_vec(),
                linesize: src_linesize.to_vec(),
                width: self.src_width,
                height: self.src_height,
                format: self.src_format,
            };
            let mut output = convert::ConvertOutput {
                planes: dst_data.iter_mut().map(|s| &mut **s).collect(),
                linesize: dst_linesize.to_vec(),
                width: self.dst_width,
                height: self.dst_height,
                format: self.dst_format,
            };
            return convert::convert(&input, &mut output);
        }

        // 不同格式 + 不同分辨率: 先缩放(同格式), 再转换
        if self.src_format != self.dst_format {
            return self.scale_with_convert(src_data, src_linesize, dst_data, dst_linesize);
        }

        // 同格式不同分辨率: 直接缩放
        scale::scale_image(
            src_data,
            src_linesize,
            self.src_width,
            self.src_height,
            self.src_format,
            dst_data,
            dst_linesize,
            self.dst_width,
            self.dst_height,
            self.algorithm,
        )
    }

    /// 不同格式 + 不同分辨率: 先缩放到目标尺寸, 再做格式转换
    fn scale_with_convert(
        &self,
        src_data: &[&[u8]],
        src_linesize: &[usize],
        dst_data: &mut [&mut [u8]],
        dst_linesize: &[usize],
    ) -> TaoResult<()> {
        // 分配中间缓冲区 (目标尺寸, 源格式)
        let planes = self.src_format.plane_count() as usize;
        let mut tmp_bufs: Vec<Vec<u8>> = Vec::with_capacity(planes);
        let mut tmp_linesizes = Vec::with_capacity(planes);

        for p in 0..planes {
            let ls = self
                .src_format
                .plane_linesize(p, self.dst_width)
                .unwrap_or(0);
            let h = self
                .src_format
                .plane_height(p, self.dst_height)
                .unwrap_or(0);
            tmp_bufs.push(vec![0u8; ls * h]);
            tmp_linesizes.push(ls);
        }

        // 第一步: 缩放 (保持源格式)
        {
            let mut tmp_slices: Vec<&mut [u8]> =
                tmp_bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
            let mut tmp_refs: Vec<&mut [u8]> = tmp_slices.iter_mut().map(|s| &mut **s).collect();
            scale::scale_image(
                src_data,
                src_linesize,
                self.src_width,
                self.src_height,
                self.src_format,
                &mut tmp_refs,
                &tmp_linesizes,
                self.dst_width,
                self.dst_height,
                self.algorithm,
            )?;
        }

        // 第二步: 格式转换 (目标尺寸)
        let tmp_plane_refs: Vec<&[u8]> = tmp_bufs.iter().map(|b| b.as_slice()).collect();
        let input = convert::ConvertInput {
            planes: tmp_plane_refs,
            linesize: tmp_linesizes,
            width: self.dst_width,
            height: self.dst_height,
            format: self.src_format,
        };
        let mut output = convert::ConvertOutput {
            planes: dst_data.iter_mut().map(|s| &mut **s).collect(),
            linesize: dst_linesize.to_vec(),
            width: self.dst_width,
            height: self.dst_height,
            format: self.dst_format,
        };
        convert::convert(&input, &mut output)
    }

    /// 同格式同分辨率的平面复制
    fn copy_planes(
        &self,
        src_data: &[&[u8]],
        src_linesize: &[usize],
        dst_data: &mut [&mut [u8]],
        dst_linesize: &[usize],
    ) -> TaoResult<()> {
        let planes = self.src_format.plane_count() as usize;
        for plane in 0..planes {
            let h = self
                .src_format
                .plane_height(plane, self.src_height)
                .unwrap_or(0);
            let copy_width = self
                .src_format
                .plane_linesize(plane, self.src_width)
                .unwrap_or(0);
            let s_stride = src_linesize.get(plane).copied().unwrap_or(copy_width);
            let d_stride = dst_linesize.get(plane).copied().unwrap_or(copy_width);

            for row in 0..h {
                let src_start = row * s_stride;
                let dst_start = row * d_stride;
                dst_data[plane][dst_start..dst_start + copy_width]
                    .copy_from_slice(&src_data[plane][src_start..src_start + copy_width]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_format_copy() {
        let ctx = ScaleContext::new(
            4,
            4,
            PixelFormat::Rgb24,
            4,
            4,
            PixelFormat::Rgb24,
            ScaleAlgorithm::Bilinear,
        );
        let src = vec![128u8; 4 * 4 * 3];
        let mut dst = vec![0u8; 4 * 4 * 3];
        ctx.scale(&[&src], &[12], &mut [&mut dst], &[12]).unwrap();
        assert_eq!(src, dst);
    }

    #[test]
    fn test_format_convert_rgb_to_yuv() {
        let ctx = ScaleContext::new(
            4,
            4,
            PixelFormat::Rgb24,
            4,
            4,
            PixelFormat::Yuv420p,
            ScaleAlgorithm::Bilinear,
        );
        let mut rgb = vec![0u8; 4 * 4 * 3];
        // 绿色
        for i in 0..16 {
            rgb[i * 3 + 1] = 255;
        }
        let mut y = vec![0u8; 16];
        let mut u = vec![0u8; 4];
        let mut v = vec![0u8; 4];
        ctx.scale(&[&rgb], &[12], &mut [&mut y, &mut u, &mut v], &[4, 2, 2])
            .unwrap();
        // 绿色: Y≈150
        assert!(y[0] > 140 && y[0] < 160, "Y={}", y[0]);
    }

    #[test]
    fn test_resolution_scale_bilinear() {
        let ctx = ScaleContext::new(
            4,
            4,
            PixelFormat::Rgb24,
            8,
            8,
            PixelFormat::Rgb24,
            ScaleAlgorithm::Bilinear,
        );
        // 全白色 4x4
        let src = vec![255u8; 4 * 4 * 3];
        let mut dst = vec![0u8; 8 * 8 * 3];
        ctx.scale(&[&src], &[12], &mut [&mut dst], &[24]).unwrap();
        // 全白放大后应保持全白
        assert!(dst.iter().all(|&v| v == 255));
    }

    #[test]
    fn test_scale_with_format_convert() {
        let ctx = ScaleContext::new(
            4,
            4,
            PixelFormat::Rgb24,
            8,
            8,
            PixelFormat::Yuv420p,
            ScaleAlgorithm::Bilinear,
        );
        // 纯绿色 4x4
        let mut src = vec![0u8; 4 * 4 * 3];
        for i in 0..16 {
            src[i * 3 + 1] = 255;
        }
        let mut y = vec![0u8; 64];
        let mut u = vec![0u8; 16];
        let mut v = vec![0u8; 16];
        ctx.scale(&[&src], &[12], &mut [&mut y, &mut u, &mut v], &[8, 4, 4])
            .unwrap();
        // 绿色 Y 应接近 150
        assert!(y[0] > 140 && y[0] < 160, "Y={}", y[0]);
    }
}
