//! 像素格式转换模块.
//!
//! 提供各种像素格式之间的转换功能, 对标 FFmpeg libswscale 的格式转换部分.
//!
//! 支持的转换路径:
//! - RGB24 ↔ YUV420P (BT.601)
//! - RGB24 ↔ Gray8
//! - RGBA → RGB24 / RGB24 → RGBA
//! - BGR24 ↔ RGB24
//! - NV12 ↔ YUV420P
//! - RGB24 → YUV444P
//!
//! 使用 BT.601 标准色彩矩阵:
//! ```text
//! Y  =  0.299 * R + 0.587 * G + 0.114 * B
//! Cb = -0.169 * R - 0.331 * G + 0.500 * B + 128
//! Cr =  0.500 * R - 0.419 * G - 0.081 * B + 128
//! ```

use tao_core::{PixelFormat, TaoError, TaoResult};

/// 像素格式转换输入 (各平面数据切片)
pub struct ConvertInput<'a> {
    /// 各平面数据
    pub planes: Vec<&'a [u8]>,
    /// 各平面行字节数
    pub linesize: Vec<usize>,
    /// 宽度
    pub width: u32,
    /// 高度
    pub height: u32,
    /// 像素格式
    pub format: PixelFormat,
}

/// 像素格式转换输出 (各平面可变数据)
pub struct ConvertOutput<'a> {
    /// 各平面数据
    pub planes: Vec<&'a mut [u8]>,
    /// 各平面行字节数
    pub linesize: Vec<usize>,
    /// 宽度
    pub width: u32,
    /// 高度
    pub height: u32,
    /// 像素格式
    pub format: PixelFormat,
}

/// 检查给定的格式转换是否支持
pub fn is_conversion_supported(src: PixelFormat, dst: PixelFormat) -> bool {
    matches!(
        (src, dst),
        (PixelFormat::Rgb24, PixelFormat::Yuv420p)
            | (PixelFormat::Yuv420p, PixelFormat::Rgb24)
            | (PixelFormat::Rgb24, PixelFormat::Gray8)
            | (PixelFormat::Gray8, PixelFormat::Rgb24)
            | (PixelFormat::Rgba, PixelFormat::Rgb24)
            | (PixelFormat::Rgb24, PixelFormat::Rgba)
            | (PixelFormat::Bgr24, PixelFormat::Rgb24)
            | (PixelFormat::Rgb24, PixelFormat::Bgr24)
            | (PixelFormat::Nv12, PixelFormat::Yuv420p)
            | (PixelFormat::Yuv420p, PixelFormat::Nv12)
            | (PixelFormat::Rgb24, PixelFormat::Yuv444p)
            | (PixelFormat::Yuv444p, PixelFormat::Rgb24)
    )
}

/// 执行像素格式转换
pub fn convert(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    if src.width != dst.width || src.height != dst.height {
        return Err(TaoError::InvalidArgument(
            "像素格式转换要求源和目标分辨率相同".into(),
        ));
    }

    match (src.format, dst.format) {
        (PixelFormat::Rgb24, PixelFormat::Yuv420p) => rgb24_to_yuv420p(src, dst),
        (PixelFormat::Yuv420p, PixelFormat::Rgb24) => yuv420p_to_rgb24(src, dst),
        (PixelFormat::Rgb24, PixelFormat::Gray8) => rgb24_to_gray8(src, dst),
        (PixelFormat::Gray8, PixelFormat::Rgb24) => gray8_to_rgb24(src, dst),
        (PixelFormat::Rgba, PixelFormat::Rgb24) => rgba_to_rgb24(src, dst),
        (PixelFormat::Rgb24, PixelFormat::Rgba) => rgb24_to_rgba(src, dst),
        (PixelFormat::Bgr24, PixelFormat::Rgb24) => bgr24_to_rgb24(src, dst),
        (PixelFormat::Rgb24, PixelFormat::Bgr24) => bgr24_to_rgb24(src, dst), // 对称操作
        (PixelFormat::Nv12, PixelFormat::Yuv420p) => nv12_to_yuv420p(src, dst),
        (PixelFormat::Yuv420p, PixelFormat::Nv12) => yuv420p_to_nv12(src, dst),
        (PixelFormat::Rgb24, PixelFormat::Yuv444p) => rgb24_to_yuv444p(src, dst),
        (PixelFormat::Yuv444p, PixelFormat::Rgb24) => yuv444p_to_rgb24(src, dst),
        _ => Err(TaoError::Unsupported(format!(
            "不支持的格式转换: {} → {}",
            src.format, dst.format,
        ))),
    }
}

// ============================================================
// BT.601 颜色空间转换常量 (定点数, 缩放 256 倍)
// ============================================================

/// Y = 0.299*R + 0.587*G + 0.114*B
const Y_R: i32 = 77; // 0.299 * 256
const Y_G: i32 = 150; // 0.587 * 256
const Y_B: i32 = 29; // 0.114 * 256

/// Cb = -0.169*R - 0.331*G + 0.500*B + 128
const CB_R: i32 = -43; // -0.169 * 256
const CB_G: i32 = -85; // -0.331 * 256
const CB_B: i32 = 128; // 0.500 * 256

/// Cr = 0.500*R - 0.419*G - 0.081*B + 128
const CR_R: i32 = 128; // 0.500 * 256
const CR_G: i32 = -107; // -0.419 * 256
const CR_B: i32 = -21; // -0.081 * 256

// ============================================================
// RGB24 ↔ YUV420P
// ============================================================

/// RGB24 → YUV420P (BT.601, 2x2 块色度平均)
fn rgb24_to_yuv420p(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let rgb = src.planes[0];

    let dst_y_stride = dst.linesize[0];
    let dst_u_stride = dst.linesize[1];
    let dst_v_stride = dst.linesize[2];

    // 分离可变引用
    let (y_plane, uv_rest) = dst.planes.split_at_mut(1);
    let (u_plane, v_plane) = uv_rest.split_at_mut(1);
    let y_data = &mut *y_plane[0];
    let u_data = &mut *u_plane[0];
    let v_data = &mut *v_plane[0];

    // 先计算所有 Y 值
    for row in 0..h {
        for col in 0..w {
            let src_off = row * src_stride + col * 3;
            let r = i32::from(rgb[src_off]);
            let g = i32::from(rgb[src_off + 1]);
            let b = i32::from(rgb[src_off + 2]);
            let y = ((Y_R * r + Y_G * g + Y_B * b + 128) >> 8).clamp(0, 255);
            y_data[row * dst_y_stride + col] = y as u8;
        }
    }

    // 色度: 2x2 块取平均
    let chroma_h = h / 2;
    let chroma_w = w / 2;
    for cy in 0..chroma_h {
        for cx in 0..chroma_w {
            let mut sum_r = 0i32;
            let mut sum_g = 0i32;
            let mut sum_b = 0i32;
            let mut count = 0i32;

            for dy in 0..2 {
                for dx in 0..2 {
                    let row = cy * 2 + dy;
                    let col = cx * 2 + dx;
                    if row < h && col < w {
                        let off = row * src_stride + col * 3;
                        sum_r += i32::from(rgb[off]);
                        sum_g += i32::from(rgb[off + 1]);
                        sum_b += i32::from(rgb[off + 2]);
                        count += 1;
                    }
                }
            }

            let avg_r = sum_r / count;
            let avg_g = sum_g / count;
            let avg_b = sum_b / count;

            let cb = ((CB_R * avg_r + CB_G * avg_g + CB_B * avg_b + 128) >> 8) + 128;
            let cr = ((CR_R * avg_r + CR_G * avg_g + CR_B * avg_b + 128) >> 8) + 128;

            u_data[cy * dst_u_stride + cx] = cb.clamp(0, 255) as u8;
            v_data[cy * dst_v_stride + cx] = cr.clamp(0, 255) as u8;
        }
    }

    Ok(())
}

/// YUV420P → RGB24 (BT.601)
fn yuv420p_to_rgb24(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;

    let y_data = src.planes[0];
    let u_data = src.planes[1];
    let v_data = src.planes[2];
    let y_stride = src.linesize[0];
    let u_stride = src.linesize[1];
    let v_stride = src.linesize[2];

    let dst_stride = dst.linesize[0];
    let rgb = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let y = i32::from(y_data[row * y_stride + col]);
            let u = i32::from(u_data[(row / 2) * u_stride + col / 2]) - 128;
            let v = i32::from(v_data[(row / 2) * v_stride + col / 2]) - 128;

            // BT.601 逆变换
            let r = (y + ((v * 359 + 128) >> 8)).clamp(0, 255);
            let g = (y - ((u * 88 + v * 183 + 128) >> 8)).clamp(0, 255);
            let b = (y + ((u * 454 + 128) >> 8)).clamp(0, 255);

            let dst_off = row * dst_stride + col * 3;
            rgb[dst_off] = r as u8;
            rgb[dst_off + 1] = g as u8;
            rgb[dst_off + 2] = b as u8;
        }
    }

    Ok(())
}

// ============================================================
// RGB24 ↔ Gray8
// ============================================================

/// RGB24 → Gray8 (BT.601 亮度)
fn rgb24_to_gray8(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let dst_stride = dst.linesize[0];
    let rgb = src.planes[0];
    let gray = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let off = row * src_stride + col * 3;
            let r = i32::from(rgb[off]);
            let g = i32::from(rgb[off + 1]);
            let b = i32::from(rgb[off + 2]);
            let y = ((Y_R * r + Y_G * g + Y_B * b + 128) >> 8).clamp(0, 255);
            gray[row * dst_stride + col] = y as u8;
        }
    }

    Ok(())
}

/// Gray8 → RGB24 (灰度扩展到 RGB)
fn gray8_to_rgb24(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let dst_stride = dst.linesize[0];
    let gray = src.planes[0];
    let rgb = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let val = gray[row * src_stride + col];
            let off = row * dst_stride + col * 3;
            rgb[off] = val;
            rgb[off + 1] = val;
            rgb[off + 2] = val;
        }
    }

    Ok(())
}

// ============================================================
// RGBA ↔ RGB24
// ============================================================

/// RGBA → RGB24 (丢弃 alpha)
fn rgba_to_rgb24(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let dst_stride = dst.linesize[0];
    let rgba = src.planes[0];
    let rgb = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let s = row * src_stride + col * 4;
            let d = row * dst_stride + col * 3;
            rgb[d] = rgba[s];
            rgb[d + 1] = rgba[s + 1];
            rgb[d + 2] = rgba[s + 2];
        }
    }

    Ok(())
}

/// RGB24 → RGBA (alpha = 255)
fn rgb24_to_rgba(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let dst_stride = dst.linesize[0];
    let rgb = src.planes[0];
    let rgba = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let s = row * src_stride + col * 3;
            let d = row * dst_stride + col * 4;
            rgba[d] = rgb[s];
            rgba[d + 1] = rgb[s + 1];
            rgba[d + 2] = rgb[s + 2];
            rgba[d + 3] = 255;
        }
    }

    Ok(())
}

// ============================================================
// BGR24 ↔ RGB24
// ============================================================

/// BGR24 ↔ RGB24 (交换 R 和 B 分量, 双向通用)
fn bgr24_to_rgb24(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let dst_stride = dst.linesize[0];
    let input = src.planes[0];
    let output = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let s = row * src_stride + col * 3;
            let d = row * dst_stride + col * 3;
            output[d] = input[s + 2]; // B→R / R→B
            output[d + 1] = input[s + 1]; // G 不变
            output[d + 2] = input[s]; // R→B / B→R
        }
    }

    Ok(())
}

// ============================================================
// NV12 ↔ YUV420P
// ============================================================

/// NV12 → YUV420P (UV 交错拆分为独立 U/V 平面)
fn nv12_to_yuv420p(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;

    // 复制 Y 平面
    let y_src = src.planes[0];
    let y_src_stride = src.linesize[0];
    let dst_y_stride = dst.linesize[0];

    let (y_plane, uv_rest) = dst.planes.split_at_mut(1);
    let (u_plane, v_plane) = uv_rest.split_at_mut(1);

    for row in 0..h {
        let src_row = &y_src[row * y_src_stride..row * y_src_stride + w];
        let dst_row = &mut y_plane[0][row * dst_y_stride..row * dst_y_stride + w];
        dst_row.copy_from_slice(src_row);
    }

    // 拆分 UV 交错数据
    let uv_src = src.planes[1];
    let uv_src_stride = src.linesize[1];
    let dst_u_stride = dst.linesize[1];
    let dst_v_stride = dst.linesize[2];
    let chroma_w = w / 2;
    let chroma_h = h / 2;

    for row in 0..chroma_h {
        for col in 0..chroma_w {
            let uv_off = row * uv_src_stride + col * 2;
            u_plane[0][row * dst_u_stride + col] = uv_src[uv_off];
            v_plane[0][row * dst_v_stride + col] = uv_src[uv_off + 1];
        }
    }

    Ok(())
}

/// YUV420P → NV12 (U/V 平面交错合并)
fn yuv420p_to_nv12(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;

    // 复制 Y 平面
    let y_src = src.planes[0];
    let y_src_stride = src.linesize[0];
    let dst_y_stride = dst.linesize[0];

    let (y_plane, uv_plane) = dst.planes.split_at_mut(1);

    for row in 0..h {
        let src_row = &y_src[row * y_src_stride..row * y_src_stride + w];
        let dst_row = &mut y_plane[0][row * dst_y_stride..row * dst_y_stride + w];
        dst_row.copy_from_slice(src_row);
    }

    // 交错合并 U/V 数据
    let u_src = src.planes[1];
    let v_src = src.planes[2];
    let u_stride = src.linesize[1];
    let v_stride = src.linesize[2];
    let dst_uv_stride = dst.linesize[1];
    let chroma_w = w / 2;
    let chroma_h = h / 2;

    for row in 0..chroma_h {
        for col in 0..chroma_w {
            let dst_off = row * dst_uv_stride + col * 2;
            uv_plane[0][dst_off] = u_src[row * u_stride + col];
            uv_plane[0][dst_off + 1] = v_src[row * v_stride + col];
        }
    }

    Ok(())
}

// ============================================================
// RGB24 ↔ YUV444P
// ============================================================

/// RGB24 → YUV444P (BT.601, 无子采样)
fn rgb24_to_yuv444p(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;
    let src_stride = src.linesize[0];
    let rgb = src.planes[0];

    let dst_y_stride = dst.linesize[0];
    let dst_u_stride = dst.linesize[1];
    let dst_v_stride = dst.linesize[2];

    let (y_plane, uv_rest) = dst.planes.split_at_mut(1);
    let (u_plane, v_plane) = uv_rest.split_at_mut(1);

    for row in 0..h {
        for col in 0..w {
            let off = row * src_stride + col * 3;
            let r = i32::from(rgb[off]);
            let g = i32::from(rgb[off + 1]);
            let b = i32::from(rgb[off + 2]);

            let y = ((Y_R * r + Y_G * g + Y_B * b + 128) >> 8).clamp(0, 255);
            let cb = (((CB_R * r + CB_G * g + CB_B * b + 128) >> 8) + 128).clamp(0, 255);
            let cr = (((CR_R * r + CR_G * g + CR_B * b + 128) >> 8) + 128).clamp(0, 255);

            y_plane[0][row * dst_y_stride + col] = y as u8;
            u_plane[0][row * dst_u_stride + col] = cb as u8;
            v_plane[0][row * dst_v_stride + col] = cr as u8;
        }
    }

    Ok(())
}

/// YUV444P → RGB24 (BT.601)
fn yuv444p_to_rgb24(src: &ConvertInput, dst: &mut ConvertOutput) -> TaoResult<()> {
    let w = src.width as usize;
    let h = src.height as usize;

    let y_data = src.planes[0];
    let u_data = src.planes[1];
    let v_data = src.planes[2];
    let y_stride = src.linesize[0];
    let u_stride = src.linesize[1];
    let v_stride = src.linesize[2];

    let dst_stride = dst.linesize[0];
    let rgb = &mut dst.planes[0];

    for row in 0..h {
        for col in 0..w {
            let y = i32::from(y_data[row * y_stride + col]);
            let u = i32::from(u_data[row * u_stride + col]) - 128;
            let v = i32::from(v_data[row * v_stride + col]) - 128;

            let r = (y + ((v * 359 + 128) >> 8)).clamp(0, 255);
            let g = (y - ((u * 88 + v * 183 + 128) >> 8)).clamp(0, 255);
            let b = (y + ((u * 454 + 128) >> 8)).clamp(0, 255);

            let off = row * dst_stride + col * 3;
            rgb[off] = r as u8;
            rgb[off + 1] = g as u8;
            rgb[off + 2] = b as u8;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建 RGB24 输入 (红色纯色)
    fn make_rgb24_solid_red(w: u32, h: u32) -> Vec<u8> {
        let mut data = vec![0u8; (w * h * 3) as usize];
        for i in 0..((w * h) as usize) {
            data[i * 3] = 255; // R
            data[i * 3 + 1] = 0; // G
            data[i * 3 + 2] = 0; // B
        }
        data
    }

    #[test]
    fn test_rgb24_to_yuv420p_纯红() {
        let w = 4u32;
        let h = 4u32;
        let rgb = make_rgb24_solid_red(w, h);

        let y_size = (w * h) as usize;
        let uv_size = ((w / 2) * (h / 2)) as usize;
        let mut y_buf = vec![0u8; y_size];
        let mut u_buf = vec![0u8; uv_size];
        let mut v_buf = vec![0u8; uv_size];

        let input = ConvertInput {
            planes: vec![&rgb],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        let mut output = ConvertOutput {
            planes: vec![&mut y_buf, &mut u_buf, &mut v_buf],
            linesize: vec![w as usize, (w / 2) as usize, (w / 2) as usize],
            width: w,
            height: h,
            format: PixelFormat::Yuv420p,
        };

        convert(&input, &mut output).unwrap();

        // BT.601: 纯红 → Y≈76, Cb≈84, Cr≈255
        assert!((y_buf[0] as i32 - 76).abs() <= 2, "Y={}", y_buf[0]);
        assert!((u_buf[0] as i32 - 84).abs() <= 2, "Cb={}", u_buf[0]);
        assert!((v_buf[0] as i32 - 255).abs() <= 2, "Cr={}", v_buf[0]);
    }

    #[test]
    fn test_yuv420p_rgb24_往返() {
        let w = 8u32;
        let h = 8u32;
        // 生成渐变 RGB
        let mut rgb_original = vec![0u8; (w * h * 3) as usize];
        for i in 0..((w * h) as usize) {
            rgb_original[i * 3] = (i * 4) as u8;
            rgb_original[i * 3 + 1] = 128;
            rgb_original[i * 3 + 2] = 64;
        }

        // RGB → YUV420
        let y_size = (w * h) as usize;
        let uv_size = ((w / 2) * (h / 2)) as usize;
        let mut y_buf = vec![0u8; y_size];
        let mut u_buf = vec![0u8; uv_size];
        let mut v_buf = vec![0u8; uv_size];

        let input = ConvertInput {
            planes: vec![&rgb_original],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        let mut yuv_output = ConvertOutput {
            planes: vec![&mut y_buf, &mut u_buf, &mut v_buf],
            linesize: vec![w as usize, (w / 2) as usize, (w / 2) as usize],
            width: w,
            height: h,
            format: PixelFormat::Yuv420p,
        };
        convert(&input, &mut yuv_output).unwrap();

        // YUV420 → RGB
        let mut rgb_result = vec![0u8; (w * h * 3) as usize];
        let yuv_input = ConvertInput {
            planes: vec![&y_buf, &u_buf, &v_buf],
            linesize: vec![w as usize, (w / 2) as usize, (w / 2) as usize],
            width: w,
            height: h,
            format: PixelFormat::Yuv420p,
        };
        let mut rgb_output = ConvertOutput {
            planes: vec![&mut rgb_result],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        convert(&yuv_input, &mut rgb_output).unwrap();

        // 由于 4:2:0 子采样丢失色度精度, 允许较大误差
        // 这是 4:2:0 的固有特性, 不是 bug
        let mut max_diff = 0i32;
        for i in 0..rgb_original.len() {
            let diff = (rgb_original[i] as i32 - rgb_result[i] as i32).abs();
            max_diff = max_diff.max(diff);
        }
        assert!(max_diff <= 20, "YUV420P 往返最大偏差过大: {}", max_diff,);
    }

    #[test]
    fn test_rgb24_gray8_往返() {
        let w = 4u32;
        let h = 4u32;
        // 灰色: R=G=B=128
        let rgb = vec![128u8; (w * h * 3) as usize];
        let mut gray = vec![0u8; (w * h) as usize];

        let input = ConvertInput {
            planes: vec![&rgb],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        let mut gray_out = ConvertOutput {
            planes: vec![&mut gray],
            linesize: vec![w as usize],
            width: w,
            height: h,
            format: PixelFormat::Gray8,
        };
        convert(&input, &mut gray_out).unwrap();

        // 灰色 → 灰度值应接近 128
        for &val in &gray {
            assert!((val as i32 - 128).abs() <= 1, "灰度值={}", val);
        }

        // Gray → RGB
        let mut rgb_result = vec![0u8; (w * h * 3) as usize];
        let gray_input = ConvertInput {
            planes: vec![&gray],
            linesize: vec![w as usize],
            width: w,
            height: h,
            format: PixelFormat::Gray8,
        };
        let mut rgb_out = ConvertOutput {
            planes: vec![&mut rgb_result],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        convert(&gray_input, &mut rgb_out).unwrap();

        // 灰色往返: R=G=B 应接近 128
        for pixel in rgb_result.chunks_exact(3) {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            assert!((pixel[0] as i32 - 128).abs() <= 1);
        }
    }

    #[test]
    fn test_rgba_rgb24() {
        let w = 2u32;
        let h = 2u32;
        let rgba = vec![
            255, 0, 0, 128, // 红 (半透明)
            0, 255, 0, 255, // 绿
            0, 0, 255, 0, // 蓝 (全透明)
            128, 128, 128, 200, // 灰
        ];
        let mut rgb = vec![0u8; (w * h * 3) as usize];

        let input = ConvertInput {
            planes: vec![&rgba],
            linesize: vec![w as usize * 4],
            width: w,
            height: h,
            format: PixelFormat::Rgba,
        };
        let mut output = ConvertOutput {
            planes: vec![&mut rgb],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        convert(&input, &mut output).unwrap();

        assert_eq!(&rgb[0..3], &[255, 0, 0]); // 红
        assert_eq!(&rgb[3..6], &[0, 255, 0]); // 绿
        assert_eq!(&rgb[6..9], &[0, 0, 255]); // 蓝
        assert_eq!(&rgb[9..12], &[128, 128, 128]); // 灰
    }

    #[test]
    fn test_bgr24_rgb24() {
        let w = 2u32;
        let h = 1u32;
        let bgr = vec![255, 0, 0, 0, 255, 0]; // B, G, R / B, G, R
        let mut rgb = vec![0u8; (w * h * 3) as usize];

        let input = ConvertInput {
            planes: vec![&bgr],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Bgr24,
        };
        let mut output = ConvertOutput {
            planes: vec![&mut rgb],
            linesize: vec![w as usize * 3],
            width: w,
            height: h,
            format: PixelFormat::Rgb24,
        };
        convert(&input, &mut output).unwrap();

        assert_eq!(&rgb[0..3], &[0, 0, 255]); // RGB: R=0, G=0, B=255
        assert_eq!(&rgb[3..6], &[0, 255, 0]); // RGB: R=0, G=255, B=0
    }

    #[test]
    fn test_nv12_yuv420p_往返() {
        let w = 4u32;
        let h = 4u32;

        // 构造 NV12 数据
        let mut nv12_y = vec![0u8; (w * h) as usize];
        for (i, val) in nv12_y.iter_mut().enumerate() {
            *val = (i * 16) as u8;
        }
        let nv12_uv = vec![64, 192, 100, 150, 32, 224, 80, 200]; // 4 个 UV 对

        // NV12 → YUV420P
        let y_size = (w * h) as usize;
        let uv_size = ((w / 2) * (h / 2)) as usize;
        let mut yuv_y = vec![0u8; y_size];
        let mut yuv_u = vec![0u8; uv_size];
        let mut yuv_v = vec![0u8; uv_size];

        let input = ConvertInput {
            planes: vec![&nv12_y, &nv12_uv],
            linesize: vec![w as usize, w as usize],
            width: w,
            height: h,
            format: PixelFormat::Nv12,
        };
        let mut output = ConvertOutput {
            planes: vec![&mut yuv_y, &mut yuv_u, &mut yuv_v],
            linesize: vec![w as usize, (w / 2) as usize, (w / 2) as usize],
            width: w,
            height: h,
            format: PixelFormat::Yuv420p,
        };
        convert(&input, &mut output).unwrap();

        // Y 应完全匹配
        assert_eq!(yuv_y, nv12_y);
        // U/V 应从交错中正确拆分
        assert_eq!(yuv_u, vec![64, 100, 32, 80]);
        assert_eq!(yuv_v, vec![192, 150, 224, 200]);

        // YUV420P → NV12 往返
        let mut nv12_y2 = vec![0u8; y_size];
        let mut nv12_uv2 = vec![0u8; uv_size * 2];

        let yuv_input = ConvertInput {
            planes: vec![&yuv_y, &yuv_u, &yuv_v],
            linesize: vec![w as usize, (w / 2) as usize, (w / 2) as usize],
            width: w,
            height: h,
            format: PixelFormat::Yuv420p,
        };
        let mut nv12_output = ConvertOutput {
            planes: vec![&mut nv12_y2, &mut nv12_uv2],
            linesize: vec![w as usize, w as usize],
            width: w,
            height: h,
            format: PixelFormat::Nv12,
        };
        convert(&yuv_input, &mut nv12_output).unwrap();

        assert_eq!(nv12_y2, nv12_y);
        assert_eq!(nv12_uv2, nv12_uv);
    }

    #[test]
    fn test_is_conversion_supported() {
        assert!(is_conversion_supported(
            PixelFormat::Rgb24,
            PixelFormat::Yuv420p
        ));
        assert!(is_conversion_supported(
            PixelFormat::Yuv420p,
            PixelFormat::Rgb24
        ));
        assert!(is_conversion_supported(
            PixelFormat::Nv12,
            PixelFormat::Yuv420p
        ));
        assert!(!is_conversion_supported(
            PixelFormat::Rgb24,
            PixelFormat::Nv12
        ));
    }
}
