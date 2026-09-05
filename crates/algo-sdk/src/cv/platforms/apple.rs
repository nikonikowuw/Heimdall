//! Apple Silicon macOS 硬件图像加速引擎 (AppleCvEngine)
//!
//! 基于 Apple `Accelerate.framework` (vImage SIMD 硬件加速) 与 `CoreVideo` 绑定，
//! 支持从 `CVPixelBuffer` 零拷贝读取 NV12，并通过 NEON/SIMD 硬件指令流完成超低延迟 Letterbox 与 Resize。

#![cfg(target_os = "macos")]

use std::ffi::c_void;

use crate::c_abi::*;
use crate::cv::buffer::CvBuffer;
use crate::cv::engine::CvEngine;
use crate::cv::layout::compute_letterbox_layout;
use crate::cv::platforms::cpu::CpuCvEngine;
use crate::cv::types::{PixelFormat, PreprocessMode};
use crate::error::AlgoError;
use crate::frame::{FrameHandleView, SafeFrame};

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format_type: u32,
        pixel_buffer_attributes: *const c_void,
        pixel_buffer_out: *mut *mut c_void,
    ) -> std::ffi::c_int;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> std::ffi::c_int;
    fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: *mut c_void,
        unlock_flags: u64,
    ) -> std::ffi::c_int;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut c_void,
        plane_index: usize,
    ) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer: *mut c_void, plane_index: usize) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut c_void) -> u32;
    fn CVPixelBufferGetPlaneCount(pixel_buffer: *mut c_void) -> usize;
}

#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn vImageConvert_YpCbCrToARGB_GenerateConversion(
        matrix: *const vImage_YpCbCrToARGBMatrix,
        pixel_range: *const vImage_YpCbCrPixelRange,
        out_info: *mut vImage_YpCbCrToARGB,
        in_yp_cb_cr_type: u32,
        out_argb_type: u32,
        flags: u32,
    ) -> isize;

    fn vImageConvert_420Yp8_CbCr8ToARGB8888(
        src_yp: *const vImage_Buffer,
        src_cb_cr: *const vImage_Buffer,
        dest: *const vImage_Buffer,
        info: *const vImage_YpCbCrToARGB,
        permute_map: *const u8,
        alpha: u8,
        flags: u32,
    ) -> isize;

    fn vImageConvert_RGBA8888toRGB888(
        src: *const vImage_Buffer,
        dest: *const vImage_Buffer,
        flags: u32,
    ) -> isize;

    fn vImageConvert_RGB888toRGBA8888(
        src: *const vImage_Buffer,
        a_src: *const vImage_Buffer,
        alpha: u8,
        dest: *const vImage_Buffer,
        premultiply: bool,
        flags: u32,
    ) -> isize;

    fn vImageScale_ARGB8888(
        src: *const vImage_Buffer,
        dest: *const vImage_Buffer,
        temp_buffer: *mut c_void,
        flags: u32,
    ) -> isize;
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct vImage_Buffer {
    data: *mut c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct vImage_YpCbCrToARGBMatrix {
    yp: f32,
    cr_r: f32,
    cr_g: f32,
    cb_g: f32,
    cb_b: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct vImage_YpCbCrPixelRange {
    yp_bias: i32,
    cb_cr_bias: i32,
    yp_range_max: i32,
    cb_cr_range_max: i32,
    yp_max: i32,
    yp_min: i32,
    cb_cr_max: i32,
    cb_cr_min: i32,
}

#[repr(C, align(16))]
#[derive(Copy, Clone)]
struct vImage_YpCbCrToARGB {
    _opaque: [u8; 128],
}

#[derive(Debug, Copy, Clone)]
struct Nv12Source {
    y_ptr: *mut c_void,
    uv_ptr: *mut c_void,
    y_stride: usize,
    uv_stride: usize,
    width: usize,
    height: usize,
}

struct Nv12Target<'a> {
    buffer: &'a vImage_Buffer,
    permute_map: &'a [u8; 4],
}

#[derive(Debug, Copy, Clone)]
struct SurfaceScaleRequest {
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
    region: (usize, usize, usize, usize),
    fill_color: Option<[u8; 3]>,
}

/// Apple Accelerate vImage 硬件加速引擎
#[derive(Debug, Default, Clone, Copy)]
pub struct AppleCvEngine;

/// 拥有 CoreVideo Create 返回的 +1 引用；转换失败时也会自动释放。
struct OwnedPixelBuffer(*mut c_void);

impl OwnedPixelBuffer {
    fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    fn into_raw(self) -> *mut c_void {
        let ptr = self.0;
        std::mem::forget(self);
        ptr
    }
}

impl Drop for OwnedPixelBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 指针来自 CVPixelBufferCreate 且所有权由当前 RAII 值持有。
            unsafe { CVPixelBufferRelease(self.0) };
        }
    }
}

struct PixelBufferLock {
    ptr: *mut c_void,
    flags: u64,
}

impl PixelBufferLock {
    fn new(ptr: *mut c_void, flags: u64) -> Result<Self, AlgoError> {
        if ptr.is_null() {
            return Err(AlgoError::Preprocess {
                reason: "CVPixelBuffer 指针为空".to_string(),
            });
        }
        // SAFETY: ptr 由调用方的 CVPixelBuffer 句柄契约保证有效。
        let status = unsafe { CVPixelBufferLockBaseAddress(ptr, flags) };
        if status != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("CVPixelBufferLockBaseAddress 失败: {status}"),
            });
        }
        Ok(Self { ptr, flags })
    }
}

impl Drop for PixelBufferLock {
    fn drop(&mut self) {
        // SAFETY: 只有 LockBaseAddress 成功后才创建此 guard，且只解锁一次。
        unsafe {
            CVPixelBufferUnlockBaseAddress(self.ptr, self.flags);
        }
    }
}

const K_CVPIXEL_FORMAT_420V: u32 = 0x34323076; // '420v'
const K_CVPIXEL_FORMAT_420F: u32 = 0x34323066; // '420f'
const K_CVPIXEL_FORMAT_BGRA: u32 = 0x42475241; // 'BGRA'
const KV_IMAGE_420_YP8_CBCR8: u32 = 4;
const KV_IMAGE_ARGB8888: u32 = 0;

fn create_bgra_surface(width: usize, height: usize) -> Result<OwnedPixelBuffer, AlgoError> {
    if width == 0 || height == 0 {
        return Err(AlgoError::Preprocess {
            reason: "CVPixelBuffer 目标尺寸不能为 0".to_string(),
        });
    }
    let mut ptr = std::ptr::null_mut();
    // SAFETY: 输出指针可写，CoreVideo 负责按 width/height 分配完整 BGRA surface。
    let status = unsafe {
        CVPixelBufferCreate(
            std::ptr::null(),
            width,
            height,
            K_CVPIXEL_FORMAT_BGRA,
            std::ptr::null(),
            &mut ptr,
        )
    };
    if status != 0 || ptr.is_null() {
        return Err(AlgoError::OutOfMemory);
    }
    Ok(OwnedPixelBuffer(ptr))
}

impl AppleCvEngine {
    pub fn new() -> Self {
        Self
    }

    /// Host/调试路径使用 RGBA8888 中间缓冲区，最终仍返回紧凑 RGB24。
    fn convert_to_rgba8888(
        &self,
        frame: &SafeFrame<'_>,
    ) -> Result<(Vec<u8>, usize, usize), AlgoError> {
        frame.validate()?;
        let w = frame.width() as usize;
        let h = frame.height() as usize;
        if w == 0 || h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "帧宽高不能为 0".to_string(),
            });
        }

        match frame.handle_view() {
            FrameHandleView::Host { data } => {
                if data.is_empty() {
                    return Err(AlgoError::Preprocess {
                        reason: "Host 数据为空".to_string(),
                    });
                }

                match frame.pixel_format() {
                    AV_PIX_NV12 => {
                        let y_stride = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            w
                        };
                        let uv_stride = if frame.stride(1) > 0 {
                            frame.stride(1) as usize
                        } else {
                            y_stride.max(w.div_ceil(2) * 2)
                        };
                        let alloc_h = (frame.alloc_height() as usize).max(h);
                        let y_offset = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "NV12 Y 平面偏移无效".to_string(),
                            }
                        })?;
                        let y_end = y_offset
                            .checked_add(
                                y_stride
                                    .checked_mul(alloc_h)
                                    .ok_or(AlgoError::OutOfMemory)?,
                            )
                            .ok_or(AlgoError::OutOfMemory)?;
                        let uv_offset = if frame.plane_offset(1) > 0 {
                            usize::try_from(frame.plane_offset(1)).map_err(|_| {
                                AlgoError::Preprocess {
                                    reason: "NV12 UV 平面偏移无效".to_string(),
                                }
                            })?
                        } else {
                            y_end
                        };
                        let uv_end = uv_offset
                            .checked_add(
                                uv_stride
                                    .checked_mul(alloc_h.div_ceil(2))
                                    .ok_or(AlgoError::OutOfMemory)?,
                            )
                            .ok_or(AlgoError::OutOfMemory)?;
                        if uv_end > data.len() {
                            return Err(AlgoError::Preprocess {
                                reason: "NV12 数据长度不足".to_string(),
                            });
                        }

                        // SAFETY: y_end/uv_end 已在 data 长度内，指针只在 vImage 调用期间使用。
                        let y_ptr = unsafe { data.as_ptr().add(y_offset) as *mut c_void };
                        // SAFETY: uv_end 已校验在 data 长度内，uv_ptr 只在 vImage 调用期间使用。
                        let uv_ptr = unsafe { data.as_ptr().add(uv_offset) as *mut c_void };
                        self.nv12_ptrs_to_rgba(
                            Nv12Source {
                                y_ptr,
                                uv_ptr,
                                y_stride,
                                uv_stride,
                                width: w,
                                height: h,
                            },
                            frame.yuv_conversion(),
                        )
                    }
                    AV_PIX_RGB24 => {
                        let minimum = w.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
                        let stride0 = if frame.stride(0) > 0 {
                            frame.stride(0) as usize
                        } else {
                            minimum
                        };
                        let offset = usize::try_from(frame.plane_offset(0)).map_err(|_| {
                            AlgoError::Preprocess {
                                reason: "RGB 平面偏移无效".to_string(),
                            }
                        })?;
                        let source_len = stride0
                            .checked_mul(h)
                            .and_then(|len| offset.checked_add(len))
                            .ok_or(AlgoError::OutOfMemory)?;
                        if stride0 < minimum || source_len > data.len() {
                            return Err(AlgoError::Preprocess {
                                reason: "RGB 数据长度或 stride 不足".to_string(),
                            });
                        }
                        let output_len = w
                            .checked_mul(h)
                            .and_then(|pixels| pixels.checked_mul(4))
                            .ok_or(AlgoError::OutOfMemory)?;
                        // SAFETY: offset 与 source_len 已校验在 data 内，vImage 只按 stride * height 读取。
                        let src_ptr = unsafe { data.as_ptr().add(offset) as *mut c_void };
                        let src_buf = vImage_Buffer {
                            data: src_ptr,
                            height: h,
                            width: w,
                            row_bytes: stride0,
                        };
                        let mut rgba_buf = vec![0u8; output_len];
                        let dst_buf = vImage_Buffer {
                            data: rgba_buf.as_mut_ptr() as *mut c_void,
                            height: h,
                            width: w,
                            row_bytes: w * 4,
                        };

                        // SAFETY: 两个 vImage buffer 均指向有效、互不重叠的内存。
                        let err = unsafe {
                            vImageConvert_RGB888toRGBA8888(
                                &src_buf,
                                std::ptr::null(),
                                255,
                                &dst_buf,
                                false,
                                0,
                            )
                        };
                        if err != 0 {
                            return Err(AlgoError::Preprocess {
                                reason: format!("vImageConvert_RGB888toRGBA8888 失败: {err}"),
                            });
                        }
                        Ok((rgba_buf, w, h))
                    }
                    _ => {
                        let cpu = CpuCvEngine::new();
                        let (buf, _) = cpu.resize(frame, w as u32, h as u32)?;
                        let rgb = buf.as_host_bytes().ok_or(AlgoError::Preprocess {
                            reason: "获取 RGB 失败".to_string(),
                        })?;
                        let output_len = w
                            .checked_mul(h)
                            .and_then(|pixels| pixels.checked_mul(4))
                            .ok_or(AlgoError::OutOfMemory)?;
                        let src_buf = vImage_Buffer {
                            data: rgb.as_ptr() as *mut c_void,
                            height: h,
                            width: w,
                            row_bytes: w * 3,
                        };
                        let mut rgba_buf = vec![0u8; output_len];
                        let dst_buf = vImage_Buffer {
                            data: rgba_buf.as_mut_ptr() as *mut c_void,
                            height: h,
                            width: w,
                            row_bytes: w * 4,
                        };
                        // SAFETY: CPU buffer 与新分配的 RGBA buffer 在调用期间均有效。
                        let err = unsafe {
                            vImageConvert_RGB888toRGBA8888(
                                &src_buf,
                                std::ptr::null(),
                                255,
                                &dst_buf,
                                false,
                                0,
                            )
                        };
                        if err != 0 {
                            return Err(AlgoError::Preprocess {
                                reason: format!("vImageConvert_RGB888toRGBA8888 失败: {err}"),
                            });
                        }
                        Ok((rgba_buf, w, h))
                    }
                }
            }
            FrameHandleView::ApplePixelBuffer { .. } => Err(AlgoError::Internal {
                reason: "原生 CVPixelBuffer 必须走 Apple surface 路径".to_string(),
            }),
            _ => Err(AlgoError::IncompatibleFrame {
                reason: "AppleCvEngine 不支持此硬件句柄".to_string(),
            }),
        }
    }

    /// 根据帧元数据生成 vImage 的转换参数，并直接写入调用方提供的目标 buffer。
    fn convert_nv12_into(
        &self,
        source: Nv12Source,
        target: Nv12Target<'_>,
        conversion: crate::frame::YuvConversion,
    ) -> Result<(), AlgoError> {
        let Nv12Source {
            y_ptr,
            uv_ptr,
            y_stride,
            uv_stride,
            width: w,
            height: h,
        } = source;
        let dest = target.buffer;
        let permute_map = target.permute_map;
        let uv_width = w.div_ceil(2).checked_mul(2).ok_or(AlgoError::OutOfMemory)?;
        let output_row = w.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
        if y_ptr.is_null()
            || uv_ptr.is_null()
            || dest.data.is_null()
            || y_stride < w
            || uv_stride < uv_width
            || dest.width != w
            || dest.height != h
            || dest.row_bytes < output_row
        {
            return Err(AlgoError::Preprocess {
                reason: "vImage NV12 buffer 参数无效".to_string(),
            });
        }

        let src_y = vImage_Buffer {
            data: y_ptr,
            height: h,
            width: w,
            row_bytes: y_stride,
        };
        let src_uv = vImage_Buffer {
            data: uv_ptr,
            height: h.div_ceil(2),
            width: w.div_ceil(2),
            row_bytes: uv_stride,
        };
        let matrix = vImage_YpCbCrToARGBMatrix {
            // pixel_range 负责 video/full range 的 Y 缩放，matrix 只描述色彩标准。
            yp: 1.0,
            cr_r: conversion.matrix_r_cr,
            cr_g: conversion.matrix_g_cr,
            cb_g: conversion.matrix_g_cb,
            cb_b: conversion.matrix_b_cb,
        };
        let pixel_range = if conversion.full_range {
            vImage_YpCbCrPixelRange {
                yp_bias: 0,
                cb_cr_bias: 128,
                yp_range_max: 255,
                cb_cr_range_max: 255,
                yp_max: 255,
                yp_min: 0,
                cb_cr_max: 255,
                cb_cr_min: 0,
            }
        } else {
            vImage_YpCbCrPixelRange {
                yp_bias: 16,
                cb_cr_bias: 128,
                yp_range_max: 235,
                cb_cr_range_max: 240,
                yp_max: 255,
                yp_min: 0,
                cb_cr_max: 255,
                cb_cr_min: 0,
            }
        };
        let mut out_info = vImage_YpCbCrToARGB {
            _opaque: [0u8; 128],
        };

        // SAFETY: matrix、pixel_range、out_info 均为 ABI 对齐且在调用期间有效。
        let gen_err = unsafe {
            vImageConvert_YpCbCrToARGB_GenerateConversion(
                &matrix,
                &pixel_range,
                &mut out_info,
                KV_IMAGE_420_YP8_CBCR8,
                KV_IMAGE_ARGB8888,
                0,
            )
        };
        if gen_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageConvert_YpCbCrToARGB_GenerateConversion 错误: {gen_err}"),
            });
        }

        // SAFETY: 输入/输出 buffer 的尺寸、stride 与地址已在上方校验。
        let conv_err = unsafe {
            vImageConvert_420Yp8_CbCr8ToARGB8888(
                &src_y,
                &src_uv,
                dest,
                &out_info,
                permute_map.as_ptr(),
                255,
                0,
            )
        };
        if conv_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageConvert_420Yp8_CbCr8ToARGB8888 错误: {conv_err}"),
            });
        }
        Ok(())
    }

    fn nv12_ptrs_to_rgba(
        &self,
        source: Nv12Source,
        conversion: crate::frame::YuvConversion,
    ) -> Result<(Vec<u8>, usize, usize), AlgoError> {
        let output_len = source
            .width
            .checked_mul(source.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AlgoError::OutOfMemory)?;
        let mut rgba_buf = vec![0u8; output_len];
        let dest = vImage_Buffer {
            data: rgba_buf.as_mut_ptr() as *mut c_void,
            height: source.height,
            width: source.width,
            row_bytes: source.width * 4,
        };
        self.convert_nv12_into(
            source,
            Nv12Target {
                buffer: &dest,
                permute_map: &[1, 2, 3, 0],
            },
            conversion,
        )?;
        Ok((rgba_buf, source.width, source.height))
    }

    /// 将原生 NV12 CVPixelBuffer 直接转换到另一个 BGRA CVPixelBuffer，避免 Host readback。
    fn convert_native_to_bgra_surface(
        &self,
        frame: &SafeFrame<'_>,
    ) -> Result<(OwnedPixelBuffer, usize, usize), AlgoError> {
        frame.validate()?;
        let ptr = match frame.handle_view() {
            FrameHandleView::ApplePixelBuffer { ptr } => ptr,
            _ => {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "Apple surface 路径需要 CVPixelBuffer".to_string(),
                })
            }
        };
        if frame.pixel_format() != AV_PIX_NV12 {
            return Err(AlgoError::IncompatibleFrame {
                reason: "Apple surface 路径只支持 NV12".to_string(),
            });
        }
        let w = frame.width() as usize;
        let h = frame.height() as usize;
        // SAFETY: ptr 来自受 ABI 校验的 CVPixelBuffer 句柄。
        let (actual_w, actual_h, pixel_type, plane_count) = unsafe {
            (
                CVPixelBufferGetWidth(ptr),
                CVPixelBufferGetHeight(ptr),
                CVPixelBufferGetPixelFormatType(ptr),
                CVPixelBufferGetPlaneCount(ptr),
            )
        };
        if actual_w < w || actual_h < h || plane_count < 2 {
            return Err(AlgoError::Preprocess {
                reason: "CVPixelBuffer 尺寸或平面数不足".to_string(),
            });
        }
        if pixel_type != K_CVPIXEL_FORMAT_420V && pixel_type != K_CVPIXEL_FORMAT_420F {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!("不支持的 CVPixelBuffer NV12 格式: {pixel_type:#x}"),
            });
        }

        let output = create_bgra_surface(w, h)?;
        let source_lock = PixelBufferLock::new(ptr, 1)?;
        let destination_lock = PixelBufferLock::new(output.as_ptr(), 0)?;
        // SAFETY: 两个 buffer 均已成功 lock；CoreVideo 返回的地址在 guard 生命周期内有效。
        let (y_ptr, uv_ptr, y_stride, uv_stride, dst_ptr, dst_stride) = unsafe {
            (
                CVPixelBufferGetBaseAddressOfPlane(ptr, 0),
                CVPixelBufferGetBaseAddressOfPlane(ptr, 1),
                CVPixelBufferGetBytesPerRowOfPlane(ptr, 0),
                CVPixelBufferGetBytesPerRowOfPlane(ptr, 1),
                CVPixelBufferGetBaseAddress(output.as_ptr()),
                CVPixelBufferGetBytesPerRow(output.as_ptr()),
            )
        };
        let destination = vImage_Buffer {
            data: dst_ptr,
            height: h,
            width: w,
            row_bytes: dst_stride,
        };
        self.convert_nv12_into(
            Nv12Source {
                y_ptr,
                uv_ptr,
                y_stride,
                uv_stride,
                width: w,
                height: h,
            },
            Nv12Target {
                buffer: &destination,
                permute_map: &[3, 2, 1, 0],
            },
            frame.yuv_conversion(),
        )?;
        drop(destination_lock);
        drop(source_lock);
        Ok((output, w, h))
    }

    /// 在 CoreVideo BGRA surface 上填充背景并把 source 缩放到指定区域。
    fn scale_to_surface(
        &self,
        source: &OwnedPixelBuffer,
        request: SurfaceScaleRequest,
    ) -> Result<OwnedPixelBuffer, AlgoError> {
        let SurfaceScaleRequest {
            src_width: src_w,
            src_height: src_h,
            dst_width: dst_w,
            dst_height: dst_h,
            region,
            fill_color,
        } = request;
        let (region_x, region_y, region_w, region_h) = region;
        if region_w == 0
            || region_h == 0
            || region_x > dst_w
            || region_y > dst_h
            || region_w > dst_w - region_x
            || region_h > dst_h - region_y
        {
            return Err(AlgoError::Preprocess {
                reason: "CoreVideo 缩放区域越界".to_string(),
            });
        }
        let source_lock = PixelBufferLock::new(source.as_ptr(), 1)?;
        let output = create_bgra_surface(dst_w, dst_h)?;
        let destination_lock = PixelBufferLock::new(output.as_ptr(), 0)?;
        // SAFETY: 两个 surface 均已锁定，地址和 stride 在当前 guard 生命周期内有效。
        let (src_ptr, src_stride, dst_ptr, dst_stride) = unsafe {
            (
                CVPixelBufferGetBaseAddress(source.as_ptr()),
                CVPixelBufferGetBytesPerRow(source.as_ptr()),
                CVPixelBufferGetBaseAddress(output.as_ptr()),
                CVPixelBufferGetBytesPerRow(output.as_ptr()),
            )
        };
        let src_row = src_w.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
        let dst_row = dst_w.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
        if src_ptr.is_null()
            || dst_ptr.is_null()
            || src_stride < src_row
            || dst_stride < dst_row
            || src_stride
                .checked_mul(src_h)
                .is_none_or(|len| len > isize::MAX as usize)
            || dst_stride
                .checked_mul(dst_h)
                .is_none_or(|len| len > isize::MAX as usize)
        {
            return Err(AlgoError::Preprocess {
                reason: "CoreVideo BGRA surface 地址或 stride 无效".to_string(),
            });
        }

        if let Some([r, g, b]) = fill_color {
            for y in 0..dst_h {
                // SAFETY: dst_stride * dst_h 已通过 isize 上限校验，row 长度不超过 stride。
                let row_ptr = unsafe { (dst_ptr as *mut u8).add(y * dst_stride) };
                // SAFETY: row_ptr 指向已锁定 surface 的当前行，dst_row <= dst_stride。
                let row = unsafe { std::slice::from_raw_parts_mut(row_ptr, dst_row) };
                for pixel in row.chunks_exact_mut(4) {
                    pixel[0] = b;
                    pixel[1] = g;
                    pixel[2] = r;
                    pixel[3] = 255;
                }
            }
        }

        let region_offset = region_y
            .checked_mul(dst_stride)
            .and_then(|offset| offset.checked_add(region_x.checked_mul(4)?))
            .ok_or(AlgoError::OutOfMemory)?;
        let region_last = region_offset
            .checked_add(
                (region_h - 1)
                    .checked_mul(dst_stride)
                    .and_then(|offset| offset.checked_add(region_w.checked_mul(4)?))
                    .ok_or(AlgoError::OutOfMemory)?,
            )
            .ok_or(AlgoError::OutOfMemory)?;
        if region_last
            > dst_stride
                .checked_mul(dst_h)
                .ok_or(AlgoError::OutOfMemory)?
        {
            return Err(AlgoError::Preprocess {
                reason: "CoreVideo 缩放目标区域超出 surface".to_string(),
            });
        }
        // SAFETY: region_last 已验证在目标 surface 的可寻址范围内。
        let region_ptr = unsafe { (dst_ptr as *mut u8).add(region_offset) as *mut c_void };
        let src = vImage_Buffer {
            data: src_ptr,
            height: src_h,
            width: src_w,
            row_bytes: src_stride,
        };
        let dest = vImage_Buffer {
            data: region_ptr,
            height: region_h,
            width: region_w,
            row_bytes: dst_stride,
        };
        // SAFETY: source/destination buffer 已校验，缩放区域与背景填充不重叠读写。
        let scale_err = unsafe { vImageScale_ARGB8888(&src, &dest, std::ptr::null_mut(), 0) };
        if scale_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageScale_ARGB8888 失败: {scale_err}"),
            });
        }
        drop(destination_lock);
        drop(source_lock);
        Ok(output)
    }

    fn host_letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let (src_rgba, src_w, src_h) = self.convert_to_rgba8888(frame)?;
        let layout = compute_letterbox_layout(src_w as u32, src_h as u32, dst_w, dst_h);
        let dst_w_usize = dst_w as usize;
        let dst_h_usize = dst_h as usize;
        let canvas_len = dst_w_usize
            .checked_mul(dst_h_usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AlgoError::OutOfMemory)?;
        let mut canvas_rgba = vec![0u8; canvas_len];
        for chunk in canvas_rgba.chunks_exact_mut(4) {
            chunk[0] = fill_color[0];
            chunk[1] = fill_color[1];
            chunk[2] = fill_color[2];
            chunk[3] = 255;
        }
        let row_bytes = dst_w_usize.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
        let offset = (layout.pad_top as usize)
            .checked_mul(row_bytes)
            .and_then(|offset| offset.checked_add((layout.pad_left as usize).checked_mul(4)?))
            .ok_or(AlgoError::OutOfMemory)?;
        let content_end = offset
            .checked_add(
                (layout.scaled_h as usize - 1)
                    .checked_mul(row_bytes)
                    .and_then(|value| value.checked_add((layout.scaled_w as usize).checked_mul(4)?))
                    .ok_or(AlgoError::OutOfMemory)?,
            )
            .ok_or(AlgoError::OutOfMemory)?;
        if content_end > canvas_rgba.len() {
            return Err(AlgoError::Preprocess {
                reason: "Host letterbox 区域越界".to_string(),
            });
        }
        let src = vImage_Buffer {
            data: src_rgba.as_ptr() as *mut c_void,
            height: src_h,
            width: src_w,
            row_bytes: src_w * 4,
        };
        // SAFETY: content_end 已验证，目标指针仍在 canvas_rgba 内。
        let content_ptr = unsafe { canvas_rgba.as_mut_ptr().add(offset) as *mut c_void };
        let dest = vImage_Buffer {
            data: content_ptr,
            height: layout.scaled_h as usize,
            width: layout.scaled_w as usize,
            row_bytes,
        };
        // SAFETY: vImage 只读 src_rgba 并写入已分配的目标区域。
        let scale_err = unsafe { vImageScale_ARGB8888(&src, &dest, std::ptr::null_mut(), 0) };
        if scale_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageScale_ARGB8888 失败: {scale_err}"),
            });
        }

        let rgb_len = dst_w_usize
            .checked_mul(dst_h_usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;
        let mut canvas_rgb = vec![0u8; rgb_len];
        let full_src = vImage_Buffer {
            data: canvas_rgba.as_mut_ptr() as *mut c_void,
            height: dst_h_usize,
            width: dst_w_usize,
            row_bytes,
        };
        let final_rgb = vImage_Buffer {
            data: canvas_rgb.as_mut_ptr() as *mut c_void,
            height: dst_h_usize,
            width: dst_w_usize,
            row_bytes: dst_w_usize * 3,
        };
        // SAFETY: 两个紧凑图像 buffer 均覆盖完整目标尺寸。
        let pack_err = unsafe { vImageConvert_RGBA8888toRGB888(&full_src, &final_rgb, 0) };
        if pack_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageConvert_RGBA8888toRGB888 失败: {pack_err}"),
            });
        }
        Ok((
            CvBuffer::from_host(canvas_rgb, dst_w, dst_h, PixelFormat::Rgb24),
            PreprocessMode::Letterbox(layout),
        ))
    }

    fn host_resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let (src_rgba, src_w, src_h) = self.convert_to_rgba8888(frame)?;
        let dst_w_usize = dst_w as usize;
        let dst_h_usize = dst_h as usize;
        let output_len = dst_w_usize
            .checked_mul(dst_h_usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(AlgoError::OutOfMemory)?;
        let mut dst_rgba = vec![0u8; output_len];
        let src = vImage_Buffer {
            data: src_rgba.as_ptr() as *mut c_void,
            height: src_h,
            width: src_w,
            row_bytes: src_w * 4,
        };
        let dest = vImage_Buffer {
            data: dst_rgba.as_mut_ptr() as *mut c_void,
            height: dst_h_usize,
            width: dst_w_usize,
            row_bytes: dst_w_usize * 4,
        };
        // SAFETY: vImage buffer 均指向有效的独立分配。
        let scale_err = unsafe { vImageScale_ARGB8888(&src, &dest, std::ptr::null_mut(), 0) };
        if scale_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageScale_ARGB8888 失败: {scale_err}"),
            });
        }
        let rgb_len = dst_w_usize
            .checked_mul(dst_h_usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;
        let mut dst_rgb = vec![0u8; rgb_len];
        let final_rgb = vImage_Buffer {
            data: dst_rgb.as_mut_ptr() as *mut c_void,
            height: dst_h_usize,
            width: dst_w_usize,
            row_bytes: dst_w_usize * 3,
        };
        // SAFETY: RGBA 源和 RGB 目标均覆盖完整目标尺寸。
        let pack_err = unsafe { vImageConvert_RGBA8888toRGB888(&dest, &final_rgb, 0) };
        if pack_err != 0 {
            return Err(AlgoError::Preprocess {
                reason: format!("vImageConvert_RGBA8888toRGB888 失败: {pack_err}"),
            });
        }
        Ok((
            CvBuffer::from_host(dst_rgb, dst_w, dst_h, PixelFormat::Rgb24),
            PreprocessMode::Resize,
        ))
    }

    fn native_letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let (source, src_w, src_h) = self.convert_native_to_bgra_surface(frame)?;
        let layout = compute_letterbox_layout(src_w as u32, src_h as u32, dst_w, dst_h);
        let output = self.scale_to_surface(
            &source,
            SurfaceScaleRequest {
                src_width: src_w,
                src_height: src_h,
                dst_width: dst_w as usize,
                dst_height: dst_h as usize,
                region: (
                    layout.pad_left as usize,
                    layout.pad_top as usize,
                    layout.scaled_w as usize,
                    layout.scaled_h as usize,
                ),
                fill_color: Some(fill_color),
            },
        )?;
        let ptr = output.into_raw();
        let buffer = CvBuffer::from_cvpixelbuffer(
            ptr,
            dst_w,
            dst_h,
            PixelFormat::Bgra,
            Some(CVPixelBufferRelease),
        );
        Ok((buffer, PreprocessMode::Letterbox(layout)))
    }

    fn native_resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let (source, src_w, src_h) = self.convert_native_to_bgra_surface(frame)?;
        let output = self.scale_to_surface(
            &source,
            SurfaceScaleRequest {
                src_width: src_w,
                src_height: src_h,
                dst_width: dst_w as usize,
                dst_height: dst_h as usize,
                region: (0, 0, dst_w as usize, dst_h as usize),
                fill_color: None,
            },
        )?;
        let ptr = output.into_raw();
        let buffer = CvBuffer::from_cvpixelbuffer(
            ptr,
            dst_w,
            dst_h,
            PixelFormat::Bgra,
            Some(CVPixelBufferRelease),
        );
        Ok((buffer, PreprocessMode::Resize))
    }
}

impl CvEngine for AppleCvEngine {
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
        if matches!(
            frame.handle_view(),
            FrameHandleView::ApplePixelBuffer { .. }
        ) {
            self.native_letterbox(frame, dst_w, dst_h, fill_color)
        } else {
            self.host_letterbox(frame, dst_w, dst_h, fill_color)
        }
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
        if matches!(
            frame.handle_view(),
            FrameHandleView::ApplePixelBuffer { .. }
        ) {
            self.native_resize(frame, dst_w, dst_h)
        } else {
            self.host_resize(frame, dst_w, dst_h)
        }
    }
}
