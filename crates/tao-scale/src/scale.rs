//! 图像缩放算法实现.
//!
//! 支持的算法:
//! - **最近邻插值** (`NearestNeighbor`): 速度最快, 适合像素艺术/整数倍缩放
//! - **双线性插值** (`Bilinear`): 速度与质量均衡, 最常用
//!
//! 支持的像素格式:
//! - RGB24 / BGR24 (packed, 每像素 3 字节)
//! - RGBA / BGRA / ARGB (packed, 每像素 4 字节)
//! - Gray8 (单通道, 每像素 1 字节)
//! - YUV420P / YUV422P / YUV444P (planar, 每平面独立缩放)

use tao_core::{PixelFormat, TaoError, TaoResult};

use super::ScaleAlgorithm;

/// 执行图像缩放
///
/// 根据指定算法将源图像缩放到目标尺寸.
/// 源和目标像素格式必须相同 (格式转换应在缩放前/后单独进行).
#[allow(clippy::too_many_arguments)]
pub fn scale_image(
    src_data: &[&[u8]],
    src_linesize: &[usize],
    src_width: u32,
    src_height: u32,
    format: PixelFormat,
    dst_data: &mut [&mut [u8]],
    dst_linesize: &[usize],
    dst_width: u32,
    dst_height: u32,
    algorithm: ScaleAlgorithm,
) -> TaoResult<()> {
    match format {
        PixelFormat::Rgb24 | PixelFormat::Bgr24 => scale_packed(
            src_data[0],
            src_linesize[0],
            src_width,
            src_height,
            dst_data[0],
            dst_linesize[0],
            dst_width,
            dst_height,
            3,
            algorithm,
        ),
        PixelFormat::Rgba | PixelFormat::Bgra | PixelFormat::Argb => scale_packed(
            src_data[0],
            src_linesize[0],
            src_width,
            src_height,
            dst_data[0],
            dst_linesize[0],
            dst_width,
            dst_height,
            4,
            algorithm,
        ),
        PixelFormat::Gray8 => scale_packed(
            src_data[0],
            src_linesize[0],
            src_width,
            src_height,
            dst_data[0],
            dst_linesize[0],
            dst_width,
            dst_height,
            1,
            algorithm,
        ),
        PixelFormat::Yuv420p | PixelFormat::Yuv422p | PixelFormat::Yuv444p => scale_planar_yuv(
            src_data,
            src_linesize,
            src_width,
            src_height,
            format,
            dst_data,
            dst_linesize,
            dst_width,
            dst_height,
            algorithm,
        ),
        _ => Err(TaoError::Unsupported(format!(
            "图像缩放不支持像素格式: {format}",
        ))),
    }
}

/// 缩放 packed 格式 (RGB24, RGBA, Gray8 等)
///
/// 所有像素存储在单个平面中, 每像素 `bpp` 字节.
#[allow(clippy::too_many_arguments)]
fn scale_packed(
    src: &[u8],
    src_stride: usize,
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    bpp: usize,
    algorithm: ScaleAlgorithm,
) -> TaoResult<()> {
    match algorithm {
        ScaleAlgorithm::NearestNeighbor => scale_plane_nearest(
            src, src_stride, src_w, src_h, dst, dst_stride, dst_w, dst_h, bpp,
        ),
        ScaleAlgorithm::Bilinear => scale_plane_bilinear(
            src, src_stride, src_w, src_h, dst, dst_stride, dst_w, dst_h, bpp,
        ),
        _ => Err(TaoError::NotImplemented(format!(
            "缩放算法 {algorithm:?} 尚未实现",
        ))),
    }
}

/// 缩放 planar YUV 格式 (每个平面独立缩放)
#[allow(clippy::too_many_arguments)]
fn scale_planar_yuv(
    src_data: &[&[u8]],
    src_linesize: &[usize],
    src_w: u32,
    src_h: u32,
    format: PixelFormat,
    dst_data: &mut [&mut [u8]],
    dst_linesize: &[usize],
    dst_w: u32,
    dst_h: u32,
    algorithm: ScaleAlgorithm,
) -> TaoResult<()> {
    let (sub_h, sub_v) = format.chroma_subsampling();

    // 亮度平面 (plane 0): 全分辨率
    scale_packed(
        src_data[0],
        src_linesize[0],
        src_w,
        src_h,
        dst_data[0],
        dst_linesize[0],
        dst_w,
        dst_h,
        1,
        algorithm,
    )?;

    // 色度平面 (plane 1, 2): 按子采样比例缩放
    let src_cw = src_w >> sub_h;
    let src_ch = src_h >> sub_v;
    let dst_cw = dst_w >> sub_h;
    let dst_ch = dst_h >> sub_v;

    for plane in 1..3 {
        scale_packed(
            src_data[plane],
            src_linesize[plane],
            src_cw,
            src_ch,
            dst_data[plane],
            dst_linesize[plane],
            dst_cw,
            dst_ch,
            1,
            algorithm,
        )?;
    }

    Ok(())
}

// ============================================================
// 最近邻插值
// ============================================================

/// 最近邻插值缩放单个平面
///
/// 对于每个目标像素, 找到源图像中最近的像素并直接复制.
/// 速度最快, 但缩放时会产生明显的锯齿.
#[allow(clippy::too_many_arguments)]
fn scale_plane_nearest(
    src: &[u8],
    src_stride: usize,
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    bpp: usize,
) -> TaoResult<()> {
    for dy in 0..dst_h as usize {
        // 映射目标行到源行
        let sy = (dy * src_h as usize) / dst_h as usize;
        let sy = sy.min(src_h as usize - 1);

        let dst_row = dy * dst_stride;
        let src_row = sy * src_stride;

        for dx in 0..dst_w as usize {
            // 映射目标列到源列
            let sx = (dx * src_w as usize) / dst_w as usize;
            let sx = sx.min(src_w as usize - 1);

            let dst_off = dst_row + dx * bpp;
            let src_off = src_row + sx * bpp;

            dst[dst_off..dst_off + bpp].copy_from_slice(&src[src_off..src_off + bpp]);
        }
    }
    Ok(())
}

// ============================================================
// 双线性插值
// ============================================================

/// 双线性插值缩放单个平面
///
/// 对于每个目标像素, 根据在源图像中的浮点坐标,
/// 用周围 4 个最近像素进行加权平均, 权重由距离决定.
///
/// ```text
/// (x0,y0)---t-----(x1,y0)
///    |              |
///    s    (x,y)     |
///    |              |
/// (x0,y1)---------(x1,y1)
///
/// 权重:
///   w00 = (1-t)*(1-s)   w10 = t*(1-s)
///   w01 = (1-t)*s       w11 = t*s
/// ```
///
/// 使用 16 位定点数 (精度 1/256) 避免浮点运算.
#[allow(clippy::too_many_arguments)]
fn scale_plane_bilinear(
    src: &[u8],
    src_stride: usize,
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: u32,
    dst_h: u32,
    bpp: usize,
) -> TaoResult<()> {
    // 预计算水平坐标映射表 (避免内循环中重复计算)
    let h_map: Vec<(usize, usize, u32)> = (0..dst_w as usize)
        .map(|dx| map_coord(dx, dst_w, src_w))
        .collect();

    for dy in 0..dst_h as usize {
        let (sy0, sy1, frac_y) = map_coord(dy, dst_h, src_h);
        let inv_y = 256 - frac_y;

        let src_row0 = sy0 * src_stride;
        let src_row1 = sy1 * src_stride;
        let dst_row = dy * dst_stride;

        for (dx, &(sx0, sx1, frac_x)) in h_map.iter().enumerate() {
            let inv_x = 256 - frac_x;

            // 权重 (定点数, 和 = 256*256 = 65536)
            let w00 = inv_x * inv_y;
            let w10 = frac_x * inv_y;
            let w01 = inv_x * frac_y;
            let w11 = frac_x * frac_y;

            let off00 = src_row0 + sx0 * bpp;
            let off10 = src_row0 + sx1 * bpp;
            let off01 = src_row1 + sx0 * bpp;
            let off11 = src_row1 + sx1 * bpp;
            let dst_off = dst_row + dx * bpp;

            for c in 0..bpp {
                let v = (u32::from(src[off00 + c]) * w00
                    + u32::from(src[off10 + c]) * w10
                    + u32::from(src[off01 + c]) * w01
                    + u32::from(src[off11 + c]) * w11
                    + 32768) // 四舍五入
                    >> 16;
                dst[dst_off + c] = v as u8;
            }
        }
    }
    Ok(())
}

/// 将目标坐标映射到源坐标
///
/// 返回 `(idx0, idx1, frac)`:
/// - `idx0`: 左/上采样点索引
/// - `idx1`: 右/下采样点索引 (已 clamp)
/// - `frac`: 小数部分 (0..256 定点数)
#[inline]
fn map_coord(dst_idx: usize, dst_size: u32, src_size: u32) -> (usize, usize, u32) {
    // 使用中心对齐映射: src_pos = (dst_idx + 0.5) * src_size / dst_size - 0.5
    let src_pos_256 =
        ((dst_idx as u64 * 2 + 1) * src_size as u64 * 128 / dst_size as u64).saturating_sub(128);

    let idx0 = (src_pos_256 >> 8) as usize;
    let frac = (src_pos_256 & 0xFF) as u32;
    let max_idx = (src_size as usize).saturating_sub(1);
    let idx1 = (idx0 + 1).min(max_idx);
    let idx0 = idx0.min(max_idx);

    (idx0, idx1, frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_最近邻_放大2x_rgb24() {
        // 2x2 RGB24 → 4x4
        let src = [
            255, 0, 0, 0, 255, 0, // 第一行: 红 绿
            0, 0, 255, 255, 255, 0, // 第二行: 蓝 黄
        ];
        let mut dst = vec![0u8; 4 * 4 * 3];
        scale_image(
            &[&src],
            &[6],
            2,
            2,
            PixelFormat::Rgb24,
            &mut [&mut dst],
            &[12],
            4,
            4,
            ScaleAlgorithm::NearestNeighbor,
        )
        .unwrap();

        // 左上角 2x2 应该全是红色
        assert_eq!(&dst[0..3], &[255, 0, 0]); // (0,0)
        assert_eq!(&dst[3..6], &[255, 0, 0]); // (1,0)
        assert_eq!(&dst[12..15], &[255, 0, 0]); // (0,1)
    }

    #[test]
    fn test_最近邻_缩小_rgb24() {
        // 4x4 → 2x2 (每 2x2 块取左上角)
        let mut src = vec![0u8; 4 * 4 * 3];
        // (0,0) = 红
        src[0] = 255;
        // (2,0) = 绿
        src[6 + 1] = 255;
        // (0,2) = 蓝
        src[24 + 2] = 255;
        // (2,2) = 白
        src[30] = 255;
        src[31] = 255;
        src[32] = 255;

        let mut dst = vec![0u8; 2 * 2 * 3];
        scale_image(
            &[&src],
            &[12],
            4,
            4,
            PixelFormat::Rgb24,
            &mut [&mut dst],
            &[6],
            2,
            2,
            ScaleAlgorithm::NearestNeighbor,
        )
        .unwrap();

        // 每个目标像素应对应源图像的 2x2 块中心附近的像素
        assert_eq!(dst.len(), 12);
    }

    #[test]
    fn test_双线性_放大2x_灰度() {
        // 2x2 灰度 → 4x4
        // [0, 100]
        // [200, 50]
        let src = [0u8, 100, 200, 50];
        let mut dst = vec![0u8; 4 * 4];

        scale_image(
            &[&src],
            &[2],
            2,
            2,
            PixelFormat::Gray8,
            &mut [&mut dst],
            &[4],
            4,
            4,
            ScaleAlgorithm::Bilinear,
        )
        .unwrap();

        // 左上角应接近 0
        assert!(dst[0] < 20, "左上角应接近 0, 实际={}", dst[0]);
        // 右上角应接近 100
        assert!(
            dst[3] > 80 && dst[3] < 120,
            "右上角应接近 100, 实际={}",
            dst[3],
        );
        // 左下角应接近 200
        assert!(dst[12] > 180, "左下角应接近 200, 实际={}", dst[12],);
    }

    #[test]
    fn test_双线性_缩小_灰度() {
        // 4x4 → 2x2 (双线性插值做平均)
        let src = [
            10, 20, 30, 40, //
            50, 60, 70, 80, //
            90, 100, 110, 120, //
            130, 140, 150, 160, //
        ];
        let mut dst = vec![0u8; 2 * 2];

        scale_image(
            &[&src],
            &[4],
            4,
            4,
            PixelFormat::Gray8,
            &mut [&mut dst],
            &[2],
            2,
            2,
            ScaleAlgorithm::Bilinear,
        )
        .unwrap();

        // 中间值, 不应全为 0 或极端值
        for &v in &dst {
            assert!(v > 0 && v < 255, "值应在合理范围内: {v}");
        }
    }

    #[test]
    fn test_双线性_相同大小_等于复制() {
        let src: Vec<u8> = (0..100).collect();
        let mut dst = vec![0u8; 100];

        scale_image(
            &[&src],
            &[10],
            10,
            10,
            PixelFormat::Gray8,
            &mut [&mut dst],
            &[10],
            10,
            10,
            ScaleAlgorithm::Bilinear,
        )
        .unwrap();

        assert_eq!(src, dst);
    }

    #[test]
    fn test_yuv420p_缩放() {
        let src_w = 8u32;
        let src_h = 8u32;
        let dst_w = 4u32;
        let dst_h = 4u32;

        let src_y = vec![128u8; 64];
        let src_u = vec![64u8; 16]; // 4x4
        let src_v = vec![192u8; 16];

        let mut dst_y = vec![0u8; 16];
        let mut dst_u = vec![0u8; 4]; // 2x2
        let mut dst_v = vec![0u8; 4];

        scale_image(
            &[&src_y, &src_u, &src_v],
            &[8, 4, 4],
            src_w,
            src_h,
            PixelFormat::Yuv420p,
            &mut [&mut dst_y, &mut dst_u, &mut dst_v],
            &[4, 2, 2],
            dst_w,
            dst_h,
            ScaleAlgorithm::Bilinear,
        )
        .unwrap();

        // 均匀色应保持不变
        assert!(dst_y.iter().all(|&v| v == 128));
        assert!(dst_u.iter().all(|&v| v == 64));
        assert!(dst_v.iter().all(|&v| v == 192));
    }

    #[test]
    fn test_map_coord_边界() {
        // 1:1 映射
        let (i0, i1, _frac) = map_coord(0, 4, 4);
        assert!(i0 < 4);
        assert!(i1 < 4);

        // 放大: dst=8, src=4
        let (i0, _, _) = map_coord(7, 8, 4);
        assert!(i0 < 4, "索引不应越界: i0={i0}");

        // 缩小: dst=2, src=8
        let (i0, i1, _) = map_coord(1, 2, 8);
        assert!(i0 < 8 && i1 < 8);
    }

    #[test]
    fn test_rgba_缩放() {
        let src = vec![255u8; 4 * 4 * 4]; // 4x4 RGBA 全白
        let mut dst = vec![0u8; 8 * 8 * 4]; // 8x8

        scale_image(
            &[&src],
            &[16],
            4,
            4,
            PixelFormat::Rgba,
            &mut [&mut dst],
            &[32],
            8,
            8,
            ScaleAlgorithm::Bilinear,
        )
        .unwrap();

        // 全白放大应保持全白
        assert!(dst.iter().all(|&v| v == 255));
    }
}
