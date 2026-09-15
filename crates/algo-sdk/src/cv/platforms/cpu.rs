//! 纯 Rust CPU 双线性插值与图像预处理引擎 (保底驱动)

use crate::c_abi::*;
use crate::cv::buffer::CvBuffer;
use crate::cv::engine::CvEngine;
use crate::cv::layout::compute_letterbox_layout;
use crate::cv::types::{CropRect, PixelFormat, PreprocessMode};
use crate::error::AlgoError;
use crate::frame::{FrameHandleView, SafeFrame};

/// 纯 Rust CPU 预处理驱动
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuCvEngine;

/// 双线性插值目标区域参数
#[derive(Debug, Clone, Copy)]
struct DstRegion {
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    dst_x_offset: usize,
    dst_y_offset: usize,
}

/// 预计算的 X 轴双线性插值采样点与权重
#[derive(Debug, Clone, Copy)]
struct XCoordLut {
    sx0_3: usize,
    sx1_3: usize,
    wx0: f32,
    wx1: f32,
}

impl CpuCvEngine {
    pub fn new() -> Self {
        Self
    }

    /// 将输入帧解码/转换为连续的 RGB24 像素
    fn extract_rgb24(&self, frame: &SafeFrame<'_>) -> Result<Vec<u8>, AlgoError> {
        // letterbox/resize 已在入口完成帧校验，这里直接复用校验后的只读视图。
        let w = frame.width() as usize;
        let h = frame.height() as usize;
        if w == 0 || h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "帧宽高不能为 0".to_string(),
            });
        }
        let output_len = w
            .checked_mul(h)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;

        match frame.handle_view() {
            FrameHandleView::Host { data } => {
                if data.is_empty() {
                    return Err(AlgoError::Preprocess {
                        reason: "Host 帧数据为空".to_string(),
                    });
                }
                match frame.pixel_format() {
                    AV_PIX_RGB24 => {
                        let min_row = w.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
                        let stride0 = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            min_row
                        };
                        if stride0 < min_row {
                            return Err(AlgoError::Preprocess {
                                reason: "Host RGB24 stride 小于最小行宽".to_string(),
                            });
                        }
                        let base = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "Host RGB24 plane offset 无效".to_string(),
                            }
                        })?;
                        let required = stride0.checked_mul(h).ok_or(AlgoError::OutOfMemory)?;
                        let source = data.get(base..).ok_or_else(|| AlgoError::Preprocess {
                            reason: "Host RGB24 plane offset 越界".to_string(),
                        })?;
                        if source.len() < required {
                            return Err(AlgoError::Preprocess {
                                reason: "Host RGB24 数据越界".to_string(),
                            });
                        }
                        if stride0 == min_row {
                            Ok(source[..output_len].to_vec())
                        } else {
                            // 逐行解包跨距
                            let mut rgb = Vec::with_capacity(output_len);
                            for y in 0..h {
                                let row_start = y * stride0;
                                rgb.extend_from_slice(&source[row_start..row_start + min_row]);
                            }
                            Ok(rgb)
                        }
                    }
                    AV_PIX_NV12 => {
                        // NV12 转 RGB24，矩阵与 range 来自帧元数据。
                        let y_stride = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            w
                        };
                        let uv_min_row =
                            w.div_ceil(2).checked_mul(2).ok_or(AlgoError::OutOfMemory)?;
                        let uv_stride = if frame.stride(1) > 0 {
                            frame.stride(1) as usize
                        } else {
                            uv_min_row
                        };
                        if y_stride < w || uv_stride < uv_min_row {
                            return Err(AlgoError::Preprocess {
                                reason: "NV12 stride 小于有效像素宽度".to_string(),
                            });
                        }

                        let alloc_h = if frame.alloc_height() > 0 {
                            frame.alloc_height() as usize
                        } else {
                            h
                        };
                        let y_offset = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "NV12 Y plane offset 无效".to_string(),
                            }
                        })?;
                        let y_size = y_stride
                            .checked_mul(alloc_h)
                            .ok_or(AlgoError::OutOfMemory)?;
                        let default_uv_offset =
                            y_offset.checked_add(y_size).ok_or(AlgoError::OutOfMemory)?;
                        let uv_offset = if frame.plane_offset(1) > 0 {
                            usize::try_from(frame.plane_offset(1)).map_err(|_| {
                                AlgoError::Preprocess {
                                    reason: "NV12 UV plane offset 无效".to_string(),
                                }
                            })?
                        } else {
                            default_uv_offset
                        };
                        let y_plane =
                            data.get(y_offset..).ok_or_else(|| AlgoError::Preprocess {
                                reason: "NV12 Y plane offset 越界".to_string(),
                            })?;
                        let uv_plane =
                            data.get(uv_offset..).ok_or_else(|| AlgoError::Preprocess {
                                reason: "NV12 UV plane offset 越界".to_string(),
                            })?;
                        let y_required = y_stride.checked_mul(h).ok_or(AlgoError::OutOfMemory)?;
                        let uv_required = uv_stride
                            .checked_mul(h.div_ceil(2))
                            .ok_or(AlgoError::OutOfMemory)?;
                        if y_plane.len() < y_required || uv_plane.len() < uv_required {
                            return Err(AlgoError::Preprocess {
                                reason: "NV12 数据长度不足".to_string(),
                            });
                        }

                        let mut rgb = vec![0u8; output_len];
                        let conversion = frame.yuv_conversion();
                        let clamp_u8 = |v: f32| -> u8 { v.clamp(0.0, 255.0).round() as u8 };

                        for y in 0..h {
                            let uv_row = (y / 2) * uv_stride;
                            let dst_row = y * w * 3;
                            for x in 0..w {
                                let y_val = y_plane[y * y_stride + x] as f32;
                                let uv_idx = uv_row + (x / 2) * 2;
                                let u_val = uv_plane[uv_idx] as f32;
                                let v_val = uv_plane[uv_idx + 1] as f32;

                                let c = (y_val - conversion.y_offset) * conversion.y_scale;
                                let d = u_val - 128.0;
                                let e = v_val - 128.0;

                                let r = clamp_u8(c + conversion.r_cr * e);
                                let g = clamp_u8(c + conversion.g_cb * d + conversion.g_cr * e);
                                let b = clamp_u8(c + conversion.b_cb * d);

                                let dst_idx = dst_row + x * 3;
                                rgb[dst_idx] = r;
                                rgb[dst_idx + 1] = g;
                                rgb[dst_idx + 2] = b;
                            }
                        }
                        Ok(rgb)
                    }
                    AV_PIX_I420 => {
                        let y_stride = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            w
                        };
                        let chroma_width = w.div_ceil(2);
                        let u_stride = if frame.stride(1) > 0 {
                            frame.stride(1) as usize
                        } else {
                            chroma_width
                        };
                        let v_stride = if frame.stride(2) > 0 {
                            frame.stride(2) as usize
                        } else {
                            u_stride
                        };
                        if y_stride < w || u_stride < chroma_width || v_stride < chroma_width {
                            return Err(AlgoError::Preprocess {
                                reason: "I420 stride 小于有效像素宽度".to_string(),
                            });
                        }

                        let alloc_h = if frame.alloc_height() > 0 {
                            frame.alloc_height() as usize
                        } else {
                            h
                        };
                        let y_offset = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "I420 Y plane offset 无效".to_string(),
                            }
                        })?;
                        let y_size = y_stride
                            .checked_mul(alloc_h)
                            .ok_or(AlgoError::OutOfMemory)?;
                        let u_offset = if frame.plane_offset(1) > 0 {
                            usize::try_from(frame.plane_offset(1)).map_err(|_| {
                                AlgoError::Preprocess {
                                    reason: "I420 U plane offset 无效".to_string(),
                                }
                            })?
                        } else {
                            y_offset.checked_add(y_size).ok_or(AlgoError::OutOfMemory)?
                        };
                        let u_size = u_stride
                            .checked_mul(alloc_h.div_ceil(2))
                            .ok_or(AlgoError::OutOfMemory)?;
                        let v_offset = if frame.plane_offset(2) > 0 {
                            usize::try_from(frame.plane_offset(2)).map_err(|_| {
                                AlgoError::Preprocess {
                                    reason: "I420 V plane offset 无效".to_string(),
                                }
                            })?
                        } else {
                            u_offset.checked_add(u_size).ok_or(AlgoError::OutOfMemory)?
                        };
                        let y_plane =
                            data.get(y_offset..).ok_or_else(|| AlgoError::Preprocess {
                                reason: "I420 Y plane offset 越界".to_string(),
                            })?;
                        let u_plane =
                            data.get(u_offset..).ok_or_else(|| AlgoError::Preprocess {
                                reason: "I420 U plane offset 越界".to_string(),
                            })?;
                        let v_plane =
                            data.get(v_offset..).ok_or_else(|| AlgoError::Preprocess {
                                reason: "I420 V plane offset 越界".to_string(),
                            })?;
                        let y_required = y_stride.checked_mul(h).ok_or(AlgoError::OutOfMemory)?;
                        let chroma_rows = h.div_ceil(2);
                        let u_required = u_stride
                            .checked_mul(chroma_rows)
                            .ok_or(AlgoError::OutOfMemory)?;
                        let v_required = v_stride
                            .checked_mul(chroma_rows)
                            .ok_or(AlgoError::OutOfMemory)?;
                        if y_plane.len() < y_required
                            || u_plane.len() < u_required
                            || v_plane.len() < v_required
                        {
                            return Err(AlgoError::Preprocess {
                                reason: "I420 数据长度不足".to_string(),
                            });
                        }

                        let mut rgb = vec![0u8; output_len];
                        let conversion = frame.yuv_conversion();
                        let clamp_u8 = |v: f32| -> u8 { v.clamp(0.0, 255.0).round() as u8 };
                        for y in 0..h {
                            let chroma_row = y / 2;
                            let u_row = chroma_row * u_stride;
                            let v_row = chroma_row * v_stride;
                            let dst_row = y * w * 3;
                            for x in 0..w {
                                let y_val = y_plane[y * y_stride + x] as f32;
                                let chroma_x = x / 2;
                                let u_val = u_plane[u_row + chroma_x] as f32;
                                let v_val = v_plane[v_row + chroma_x] as f32;
                                let c = (y_val - conversion.y_offset) * conversion.y_scale;
                                let d = u_val - 128.0;
                                let e = v_val - 128.0;
                                let dst_idx = dst_row + x * 3;
                                rgb[dst_idx] = clamp_u8(c + conversion.r_cr * e);
                                rgb[dst_idx + 1] =
                                    clamp_u8(c + conversion.g_cb * d + conversion.g_cr * e);
                                rgb[dst_idx + 2] = clamp_u8(c + conversion.b_cb * d);
                            }
                        }
                        Ok(rgb)
                    }
                    AV_PIX_BGRA => {
                        let min_row = w.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
                        let stride0 = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            min_row
                        };
                        if stride0 < min_row {
                            return Err(AlgoError::Preprocess {
                                reason: "Host BGRA stride 小于最小行宽".to_string(),
                            });
                        }
                        let base = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "Host BGRA plane offset 无效".to_string(),
                            }
                        })?;
                        let required = stride0.checked_mul(h).ok_or(AlgoError::OutOfMemory)?;
                        let source = data.get(base..).ok_or_else(|| AlgoError::Preprocess {
                            reason: "Host BGRA plane offset 越界".to_string(),
                        })?;
                        if source.len() < required {
                            return Err(AlgoError::Preprocess {
                                reason: "Host BGRA 数据越界".to_string(),
                            });
                        }
                        let mut rgb = vec![0u8; output_len];
                        for y in 0..h {
                            let src_row = y * stride0;
                            let dst_row = y * w * 3;
                            for x in 0..w {
                                let src_idx = src_row + x * 4;
                                let dst_idx = dst_row + x * 3;
                                rgb[dst_idx] = source[src_idx + 2];
                                rgb[dst_idx + 1] = source[src_idx + 1];
                                rgb[dst_idx + 2] = source[src_idx];
                            }
                        }
                        Ok(rgb)
                    }
                    _ => Err(AlgoError::IncompatibleFrame {
                        reason: format!("CpuCvEngine 暂不支持格式: {}", frame.pixel_format()),
                    }),
                }
            }
            _ => Err(AlgoError::IncompatibleFrame {
                reason: "CpuCvEngine 仅支持 Host 模式帧，硬件句柄请走对应平台硬件驱动".to_string(),
            }),
        }
    }

    pub(crate) fn crop_rgb_fallback(
        &self,
        frame: &SafeFrame<'_>,
        rect: CropRect,
    ) -> Result<CvBuffer, AlgoError> {
        frame.validate()?;
        let rect = rect.validate(frame.width(), frame.height())?;
        let source = self.extract_rgb24(frame)?;
        let source_width = usize::try_from(frame.width()).map_err(|_| AlgoError::OutOfMemory)?;
        let source_stride = source_width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
        let crop_width = usize::try_from(rect.width).map_err(|_| AlgoError::OutOfMemory)?;
        let crop_height = usize::try_from(rect.height).map_err(|_| AlgoError::OutOfMemory)?;
        let crop_x = usize::try_from(rect.x).map_err(|_| AlgoError::OutOfMemory)?;
        let crop_y = usize::try_from(rect.y).map_err(|_| AlgoError::OutOfMemory)?;
        let crop_row_bytes = crop_width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
        let crop_x_bytes = crop_x.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
        let output_len = crop_row_bytes
            .checked_mul(crop_height)
            .ok_or(AlgoError::OutOfMemory)?;
        let mut source_start = crop_y
            .checked_mul(source_stride)
            .and_then(|row_start| row_start.checked_add(crop_x_bytes))
            .ok_or(AlgoError::OutOfMemory)?;

        // `extract_rgb24` 保证 source 为整帧紧凑 RGB24，rect 已校验落在帧内，
        // 因此行起点只需按源 stride 自增，无需逐行重复 checked 运算。
        let mut output = vec![0u8; output_len];
        for dst_row in output.chunks_exact_mut(crop_row_bytes) {
            let source_end = source_start + crop_row_bytes;
            let Some(source_row) = source.get(source_start..source_end) else {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "ROI 行超出源图像范围".to_string(),
                });
            };
            dst_row.copy_from_slice(source_row);
            source_start += source_stride;
        }
        Ok(CvBuffer::from_host(
            output,
            rect.width,
            rect.height,
            PixelFormat::Rgb24,
        ))
    }

    /// 高性能双线性插值缩放（带 X 轴预计算查找表）
    fn bilinear_resize(
        &self,
        src: &[u8],
        src_w: usize,
        src_h: usize,
        dst: &mut [u8],
        region: DstRegion,
    ) {
        if src_w == 0 || src_h == 0 || region.dst_w == 0 || region.dst_h == 0 {
            return;
        }

        let scale_x = src_w as f32 / region.dst_w as f32;
        let scale_y = src_h as f32 / region.dst_h as f32;

        // 预计算 X 坐标采样点与权重，彻底消除每行对 X 轴的重复浮点乘法与截断
        let mut x_lut = Vec::with_capacity(region.dst_w);
        for dx in 0..region.dst_w {
            let sx = ((dx as f32 + 0.5) * scale_x - 0.5).clamp(0.0, (src_w - 1) as f32);
            let sx0 = sx.floor() as usize;
            let sx1 = (sx0 + 1).min(src_w - 1);
            let wx1 = sx - sx0 as f32;
            let wx0 = 1.0 - wx1;
            x_lut.push(XCoordLut {
                sx0_3: sx0 * 3,
                sx1_3: sx1 * 3,
                wx0,
                wx1,
            });
        }

        let src_stride = src_w * 3;

        for dy in 0..region.dst_h {
            let sy = ((dy as f32 + 0.5) * scale_y - 0.5).clamp(0.0, (src_h - 1) as f32);
            let sy0 = sy.floor() as usize;
            let sy1 = (sy0 + 1).min(src_h - 1);
            let wy1 = sy - sy0 as f32;
            let wy0 = 1.0 - wy1;

            let out_row_start =
                (region.dst_y_offset + dy) * region.dst_stride + region.dst_x_offset * 3;
            let row0_offset = sy0 * src_stride;
            let row1_offset = sy1 * src_stride;

            for (dx, lut) in x_lut.iter().enumerate() {
                let p00_idx = row0_offset + lut.sx0_3;
                let p10_idx = row0_offset + lut.sx1_3;
                let p01_idx = row1_offset + lut.sx0_3;
                let p11_idx = row1_offset + lut.sx1_3;

                let out_idx = out_row_start + dx * 3;

                for c in 0..3 {
                    let p00 = src[p00_idx + c] as f32;
                    let p10 = src[p10_idx + c] as f32;
                    let p01 = src[p01_idx + c] as f32;
                    let p11 = src[p11_idx + c] as f32;

                    let top = p00 * lut.wx0 + p10 * lut.wx1;
                    let bot = p01 * lut.wx0 + p11 * lut.wx1;
                    let val = (top * wy0 + bot * wy1).clamp(0.0, 255.0).round() as u8;

                    dst[out_idx + c] = val;
                }
            }
        }
    }
}

impl CvEngine for CpuCvEngine {
    fn letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        frame.validate()?;
        if dst_w == 0 || dst_h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "目标尺寸不能为 0".to_string(),
            });
        }
        let src_rgb = self.extract_rgb24(frame)?;
        let src_w = frame.width();
        let src_h = frame.height();

        let layout = compute_letterbox_layout(src_w, src_h, dst_w, dst_h);

        // 创建底色画布（若为纯黑底色直接复用 zeroed 内存，避免二次全量遍历）
        let total_bytes = dst_w
            .checked_mul(dst_h)
            .and_then(|pixels| pixels.checked_mul(3))
            .map(|bytes| bytes as usize)
            .ok_or(AlgoError::OutOfMemory)?;
        let mut canvas = vec![0u8; total_bytes];
        if fill_color != [0, 0, 0] {
            for pixel in canvas.as_chunks_mut::<3>().0 {
                pixel[0] = fill_color[0];
                pixel[1] = fill_color[1];
                pixel[2] = fill_color[2];
            }
        }

        // 双线性缩放填充居中区域
        self.bilinear_resize(
            &src_rgb,
            src_w as usize,
            src_h as usize,
            &mut canvas,
            DstRegion {
                dst_w: layout.scaled_w as usize,
                dst_h: layout.scaled_h as usize,
                dst_stride: (dst_w * 3) as usize,
                dst_x_offset: layout.pad_left as usize,
                dst_y_offset: layout.pad_top as usize,
            },
        );

        let buf = CvBuffer::from_host(canvas, dst_w, dst_h, PixelFormat::Rgb24);
        Ok((buf, PreprocessMode::Letterbox(layout)))
    }

    fn resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        frame.validate()?;
        if dst_w == 0 || dst_h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "目标尺寸不能为 0".to_string(),
            });
        }
        let src_rgb = self.extract_rgb24(frame)?;
        let src_w = frame.width();
        let src_h = frame.height();

        let total_bytes = dst_w
            .checked_mul(dst_h)
            .and_then(|pixels| pixels.checked_mul(3))
            .map(|bytes| bytes as usize)
            .ok_or(AlgoError::OutOfMemory)?;
        let mut dst = vec![0u8; total_bytes];

        self.bilinear_resize(
            &src_rgb,
            src_w as usize,
            src_h as usize,
            &mut dst,
            DstRegion {
                dst_w: dst_w as usize,
                dst_h: dst_h as usize,
                dst_stride: (dst_w * 3) as usize,
                dst_x_offset: 0,
                dst_y_offset: 0,
            },
        );

        let buf = CvBuffer::from_host(dst, dst_w, dst_h, PixelFormat::Rgb24);
        Ok((buf, PreprocessMode::Resize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cv::engine::CvEngine;

    #[test]
    fn test_cpu_cv_letterbox_rgb() {
        let w = 100u32;
        let h = 50u32;
        let mut pixels = vec![0u8; (w * h * 3) as usize];
        for chunk in pixels.as_chunks_mut::<3>().0 {
            chunk[0] = 255;
        }

        let mut desc = AvFrameDesc::default_nv12(w, h, (w * 3) as i32, 0, 0);
        desc.pixel_format = AV_PIX_RGB24;
        desc.opaque = pixels.as_ptr() as *mut std::ffi::c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;

        let frame = SafeFrame::from_ref(&desc).expect("帧描述符有效");
        let engine = CpuCvEngine::new();

        let (buf, mode) = engine
            .letterbox(&frame, 200, 200, [114, 114, 114])
            .expect("Letterbox 应当成功");

        assert_eq!(buf.width(), 200);
        assert_eq!(buf.height(), 200);
        match mode {
            PreprocessMode::Letterbox(layout) => {
                assert_eq!(layout.dst_w, 200);
                assert_eq!(layout.dst_h, 200);
                assert_eq!(layout.scaled_w, 200);
                assert_eq!(layout.scaled_h, 100);
                assert_eq!(layout.pad_top, 50);
                assert_eq!(layout.pad_left, 0);
            }
            _ => panic!("预期 Letterbox 模式"),
        }

        let bytes = buf.as_host_bytes().expect("应有 host bytes");
        assert_eq!(bytes[0], 114);
        assert_eq!(bytes[1], 114);
        assert_eq!(bytes[2], 114);

        let center_idx = (100 * 200 + 100) * 3;
        assert_eq!(bytes[center_idx], 255);
        assert_eq!(bytes[center_idx + 1], 0);
        assert_eq!(bytes[center_idx + 2], 0);
    }

    #[test]
    fn test_cpu_cv_i420_with_explicit_plane_offsets() {
        let width = 3u32;
        let height = 3u32;
        let y_stride = 4i32;
        let chroma_stride = 2i32;
        let y_offset = 3usize;
        let u_offset = 20usize;
        let v_offset = 31usize;
        let mut data = [0u8; 35];
        for row in 0..height as usize {
            let start = y_offset + row * y_stride as usize;
            data[start..start + width as usize].fill(128);
        }
        for row in 0..height.div_ceil(2) as usize {
            data[u_offset + row * chroma_stride as usize
                ..u_offset + row * chroma_stride as usize + 2]
                .fill(128);
            data[v_offset + row * chroma_stride as usize
                ..v_offset + row * chroma_stride as usize + 2]
                .fill(128);
        }

        let mut desc = AvFrameDesc::default_nv12(width, height, y_stride, chroma_stride, 0);
        desc.pixel_format = AV_PIX_I420;
        desc.stride[2] = chroma_stride;
        desc.offset = [y_offset as u64, u_offset as u64, v_offset as u64, 0];
        desc.opaque = data.as_ptr() as *mut std::ffi::c_void;

        let frame = SafeFrame::from_ref(&desc).expect("带显式平面偏移的 I420 应有效");
        let (buffer, _) = CpuCvEngine::new()
            .resize(&frame, width, height)
            .expect("I420 resize 应成功");
        let rgb = buffer.as_host_bytes().expect("I420 应产生 Host RGB");
        assert_eq!(rgb.len(), width as usize * height as usize * 3);
        for pixel in rgb.as_chunks::<3>().0 {
            assert!((pixel[0] as i16 - pixel[1] as i16).abs() <= 1);
            assert!((pixel[1] as i16 - pixel[2] as i16).abs() <= 1);
            assert!((pixel[0] as i16 - 130).abs() <= 2);
        }
    }

    #[test]
    fn test_cpu_crop_rgb_returns_only_roi() {
        let width = 4u32;
        let height = 3u32;
        let mut pixels = vec![0u8; (width * height * 3) as usize];
        for (index, pixel) in pixels.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            pixel[0] = index as u8;
            pixel[1] = 100;
            pixel[2] = 200;
        }
        let mut desc = AvFrameDesc::default_nv12(width, height, (width * 3) as i32, 0, 0);
        desc.pixel_format = AV_PIX_RGB24;
        desc.opaque = pixels.as_ptr() as *mut std::ffi::c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;
        let frame = SafeFrame::from_ref(&desc).expect("RGB 帧描述符有效");
        let buffer = CpuCvEngine::new()
            .crop_rgb_fallback(
                &frame,
                CropRect {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2,
                },
            )
            .expect("CPU ROI 裁剪应成功");
        assert_eq!(buffer.width(), 2);
        assert_eq!(buffer.height(), 2);
        assert_eq!(
            buffer.readback_rgb24().expect("ROI readback 应成功"),
            vec![5, 100, 200, 6, 100, 200, 9, 100, 200, 10, 100, 200]
        );
    }

    #[test]
    fn test_cpu_cv_bgra_uses_plane_offset_and_channel_order() {
        let width = 2u32;
        let height = 1u32;
        let mut data = [0u8; 12];
        data[4..12].copy_from_slice(&[
            30, 20, 10, 255, // B, G, R, A
            60, 50, 40, 255,
        ]);
        let mut desc = AvFrameDesc::default_nv12(width, height, 8, 0, 0);
        desc.pixel_format = AV_PIX_BGRA;
        desc.offset = [4, 0, 0, 0];
        desc.opaque = data.as_ptr() as *mut std::ffi::c_void;

        let frame = SafeFrame::from_ref(&desc).expect("带偏移的 BGRA 应有效");
        let (buffer, _) = CpuCvEngine::new()
            .resize(&frame, width, height)
            .expect("BGRA resize 应成功");
        assert_eq!(
            buffer.as_host_bytes().expect("BGRA 应产生 Host RGB"),
            &[10, 20, 30, 40, 50, 60]
        );
    }
}
