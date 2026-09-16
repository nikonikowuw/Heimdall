//! 视频帧取景预裁剪 (Frame Crop)
//!
//! 把解码帧的取景矩形裁成新的 `FrameRef` 供算法包分析，并**一并返回本次实际生效的取景矩形**：
//! 裁剪与坐标还原必须是同一个决策，调用方不得各自独立判断，否则会出现
//! 「帧未裁切、检测结果却按局部坐标还原」的几何错位。
//!
//! 平台矩阵（宿主只做它该做的那一份）：
//! - macOS（开发/演示）：`CVPixelBuffer` 与 Host 内存按平面逐行裁切，像素格式与量化范围原样保持；
//! - Linux DMA-BUF、昇腾设备内存：**不做 CPU 像素拷贝**，返回 [`MediaError::Unsupported`]，
//!   调用方降级为全景分析。设备侧零拷贝裁切必须由算法包经 algo-sdk (RGA/VPC/AIPP) 能力完成，
//!   宿主不得在常驻推理主路径上以 memcpy 冒充。

use std::sync::Arc;

use types::{BoundingBox, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::encoders::compute_crop_roi;
use crate::error::MediaError;

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const std::ffi::c_void,
        width: usize,
        height: usize,
        pixel_format_type: u32,
        pixel_buffer_attributes: *const std::ffi::c_void,
        pixel_buffer_out: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut std::ffi::c_void) -> u32;
    fn CVPixelBufferRelease(pixel_buffer: *mut std::ffi::c_void);
}

/// `'420v'` = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
#[cfg(target_os = "macos")]
const CV_PIXEL_FORMAT_420_VIDEO_RANGE: u32 = 0x3432_3076;
/// `'420f'` = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
#[cfg(target_os = "macos")]
const CV_PIXEL_FORMAT_420_FULL_RANGE: u32 = 0x3432_3066;
/// kCVPixelBufferLock_ReadOnly
#[cfg(target_os = "macos")]
const CV_LOCK_READ_ONLY: u64 = 1;
/// kCVPixelBufferLock_ReadWrite
#[cfg(target_os = "macos")]
const CV_LOCK_READ_WRITE: u64 = 0;

/// 取景裁剪结果
#[derive(Debug)]
pub struct CroppedFrame {
    /// 送模输入帧
    pub frame: FrameRef,
    /// 本次分析实际生效的取景区域（归一化）；`None` 表示未裁切，检测结果已是全景坐标
    pub roi: Option<BoundingBox>,
}

/// 将视频帧裁剪为指定取景区域的局部特写帧。
///
/// 保持与原帧相同的像素格式；硬件对齐（偶数、16 字节步长、最小边）由
/// [`compute_crop_roi`] 保证，返回值中的 `roi` 是**对齐后**的实际矩形，
/// 坐标还原必须使用它而不是调用方传入的配置值。
pub fn crop_frame(frame: &FrameRef, roi: BoundingBox) -> Result<CroppedFrame, MediaError> {
    if frame.width < 2 || frame.height < 2 {
        return Err(MediaError::Decode {
            reason: "原帧分辨率非法 (宽/高小于 2)".into(),
        });
    }

    let (sx, sy, crop_w, crop_h, w_stride) = compute_crop_roi(frame.width, frame.height, roi, 0.0);

    if crop_w == 0 || crop_h == 0 {
        return Err(MediaError::Decode {
            reason: "取景矩形计算出非法宽高 (0)".into(),
        });
    }

    // 取景矩形覆盖全幅：直接复用原帧句柄（句柄共享，无像素拷贝），坐标即全景坐标。
    if sx == 0 && sy == 0 && crop_w == frame.width && crop_h == frame.height {
        return Ok(CroppedFrame {
            frame: frame.clone(),
            roi: None,
        });
    }

    let applied_roi = effective_roi(frame.width, frame.height, sx, sy, crop_w, crop_h);

    let cropped = match frame.handle() {
        FrameHandle::Host(slice) => {
            crop_host_slice(slice, frame, sx, sy, crop_w, crop_h, w_stride)?
        }
        #[cfg(target_os = "macos")]
        FrameHandle::ApplePixelBuffer { ptr } => {
            crop_apple_pixel_buffer(ptr.as_ptr(), frame, sx, sy, crop_w, crop_h)?
        }
        #[cfg(not(target_os = "macos"))]
        FrameHandle::ApplePixelBuffer { .. } => {
            return Err(MediaError::Decode {
                reason: "ApplePixelBuffer 仅支持 macOS".into(),
            })
        }
        #[cfg(all(target_os = "linux", feature = "rga"))]
        FrameHandle::DmaBuf { fd, .. } => {
            use std::os::fd::AsRawFd;
            crate::rga_crop::crop_dmabuf_rga(
                fd.as_raw_fd(),
                frame,
                sx,
                sy,
                crop_w,
                crop_h,
                w_stride,
            )?
        }
        #[cfg(all(target_os = "linux", not(feature = "rga")))]
        FrameHandle::DmaBuf { .. } => return Err(unsupported_platform_crop()),
        FrameHandle::DeviceMemory { .. } => return Err(unsupported_platform_crop()),
    };

    Ok(CroppedFrame {
        frame: cropped,
        roi: Some(applied_roi),
    })
}

/// 设备侧零拷贝裁切尚未落地时的统一上报理由（`&'static str`，逐帧失败路径零分配）。
fn unsupported_platform_crop() -> MediaError {
    MediaError::Unsupported(
        "宿主侧 CPU 取景裁剪不适用于 DMA-BUF / 设备内存帧；裁切须由算法包经 algo-sdk (RGA/VPC/AIPP) 或启用 --features rga 在设备侧完成",
    )
}

/// 对齐后真实生效的归一化取景矩形
fn effective_roi(w: u32, h: u32, sx: u32, sy: u32, crop_w: u32, crop_h: u32) -> BoundingBox {
    let (wf, hf) = (w as f32, h as f32);
    BoundingBox::new(
        sx as f32 / wf,
        sy as f32 / hf,
        (sx + crop_w) as f32 / wf,
        (sy + crop_h) as f32 / hf,
    )
}

/// 校验源缓冲长度满足「行字节数 × 行数」，杜绝越界读取
fn ensure_plane_len(
    len: usize,
    row_bytes: usize,
    rows: usize,
    format: PixelFormat,
) -> Result<(), MediaError> {
    let needed = row_bytes.checked_mul(rows).ok_or(MediaError::Decode {
        reason: format!("{format:?} 源缓冲区尺寸溢出"),
    })?;
    if len < needed {
        return Err(MediaError::Decode {
            reason: format!("{format:?} 源数据长度不足: actual={len}, expected={needed}"),
        });
    }
    Ok(())
}

/// Host 内存局部切片裁剪 (NV12 / RGB24 / BGR24 / RGBA)
fn crop_host_slice(
    slice: &[u8],
    frame: &FrameRef,
    sx: u32,
    sy: u32,
    crop_w: u32,
    crop_h: u32,
    w_stride: u32,
) -> Result<FrameRef, MediaError> {
    let src_rows = frame.stride.ver_stride.max(frame.height) as usize;

    match frame.format {
        PixelFormat::Nv12 => {
            let src_row = frame.stride.hor_stride.max(frame.width) as usize;
            // NV12 色度行数为 (src_rows + 1) / 2
            let uv_rows = src_rows.div_ceil(2);
            // Y 平面占 src_row × src_rows，其后 UV 交错平面再占 uv_rows 行。
            ensure_plane_len(slice.len(), src_row, src_rows + uv_rows, PixelFormat::Nv12)?;

            // crop_w / crop_h 的偶数性由 compute_crop_roi 保证（YUV420 的 2×2 子采样要求）。
            if !crop_w.is_multiple_of(2) || !crop_h.is_multiple_of(2) {
                return Err(MediaError::Decode {
                    reason: format!("NV12 取景宽高必须为偶数: {crop_w}x{crop_h}"),
                });
            }

            let dst_row = w_stride as usize;
            let dst_y_len = dst_row * crop_h as usize;
            let dst_uv_len = dst_row * (crop_h as usize / 2);
            let mut dst = vec![0u8; dst_y_len + dst_uv_len];

            // 1. 逐行拷贝 Y 平面
            for r in 0..crop_h as usize {
                let src_offset = (sy as usize + r) * src_row + sx as usize;
                let dst_offset = r * dst_row;
                let row_bytes = crop_w as usize;
                let src_slice = slice
                    .get(src_offset..src_offset + row_bytes)
                    .ok_or_else(|| MediaError::Decode {
                        reason: format!(
                            "NV12 Y平面越界: offset={src_offset}, len={row_bytes}, slice_len={}",
                            slice.len()
                        ),
                    })?;
                dst[dst_offset..dst_offset + row_bytes].copy_from_slice(src_slice);
            }

            // 2. 逐行拷贝 UV 交错平面（1 字节 = 2 个亮度像素的色度）
            let src_uv_base = src_row * src_rows;
            for r in 0..(crop_h as usize / 2) {
                let src_offset = src_uv_base + (sy as usize / 2 + r) * src_row + sx as usize;
                let dst_offset = dst_y_len + r * dst_row;
                let row_bytes = crop_w as usize;
                let src_slice = slice
                    .get(src_offset..src_offset + row_bytes)
                    .ok_or_else(|| MediaError::Decode {
                        reason: format!(
                            "NV12 UV平面越界: offset={src_offset}, len={row_bytes}, slice_len={}",
                            slice.len()
                        ),
                    })?;
                dst[dst_offset..dst_offset + row_bytes].copy_from_slice(src_slice);
            }

            Ok(FrameRef::new(
                frame.camera_id.clone(),
                frame.timestamp,
                crop_w,
                crop_h,
                StrideInfo::new(w_stride, crop_h),
                PixelFormat::Nv12,
                FrameHandle::Host(Arc::from(dst)),
            ))
        }
        // 打包像素：每像素 bpp 字节，单平面逐行裁切。
        PixelFormat::Rgb24 | PixelFormat::Bgr24 | PixelFormat::Rgba => {
            let bpp = match frame.format {
                PixelFormat::Rgba => 4,
                _ => 3,
            };
            let min_row_bytes = frame.width as usize * bpp;
            let hor_stride = frame.stride.hor_stride as usize;
            // 若 hor_stride 已大于等于可见像素行字节数，表明其本身已是字节跨步；
            // 仅当 hor_stride 处于像素跨步区间时才需补乘 bpp。
            let src_row = if hor_stride >= min_row_bytes {
                hor_stride
            } else if hor_stride >= frame.width as usize {
                hor_stride * bpp
            } else {
                min_row_bytes
            };
            ensure_plane_len(slice.len(), src_row, src_rows, frame.format)?;

            let dst_row = w_stride as usize * bpp;
            let row_bytes = crop_w as usize * bpp;
            let mut dst = vec![0u8; dst_row * crop_h as usize];

            for r in 0..crop_h as usize {
                let src_offset = (sy as usize + r) * src_row + sx as usize * bpp;
                let dst_offset = r * dst_row;
                let src_slice = slice
                    .get(src_offset..src_offset + row_bytes)
                    .ok_or_else(|| MediaError::Decode {
                        reason: format!(
                            "{:?} 裁切行超出源缓冲: offset={src_offset}, len={row_bytes}, slice_len={}",
                            frame.format,
                            slice.len()
                        ),
                    })?;
                dst[dst_offset..dst_offset + row_bytes].copy_from_slice(src_slice);
            }

            Ok(FrameRef::new(
                frame.camera_id.clone(),
                frame.timestamp,
                crop_w,
                crop_h,
                StrideInfo::new(w_stride, crop_h),
                frame.format,
                FrameHandle::Host(Arc::from(dst)),
            ))
        }
        PixelFormat::Yuv420p => Err(MediaError::Unsupported(
            "Yuv420p 三平面取景裁剪未实现（解码输出不产生该格式）",
        )),
    }
}

#[cfg(target_os = "macos")]
fn crop_apple_pixel_buffer(
    src_buf: *mut std::ffi::c_void,
    frame: &FrameRef,
    sx: u32,
    sy: u32,
    crop_w: u32,
    crop_h: u32,
) -> Result<FrameRef, MediaError> {
    if src_buf.is_null() {
        return Err(MediaError::Decode {
            reason: "CVPixelBuffer 源指针为空".into(),
        });
    }

    // SAFETY: 1 = kCVPixelBufferLock_ReadOnly，锁定源缓冲区只读访问
    let lock_res = unsafe { CVPixelBufferLockBaseAddress(src_buf, CV_LOCK_READ_ONLY) };
    if lock_res != 0 {
        return Err(MediaError::Decode {
            reason: format!("锁定源 CVPixelBuffer 失败: {lock_res}"),
        });
    }

    struct SrcGuard(*mut std::ffi::c_void);
    impl Drop for SrcGuard {
        fn drop(&mut self) {
            // SAFETY: 解锁已成功获取只读锁定的源 CVPixelBuffer
            unsafe {
                CVPixelBufferUnlockBaseAddress(self.0, CV_LOCK_READ_ONLY);
            }
        }
    }
    let _src_guard = SrcGuard(src_buf);

    // SAFETY: src_buf 在当前生命周期内已被 LockBaseAddress 锁定且非空
    let (src_y, src_uv, src_ys, src_uvs, src_format) = unsafe {
        (
            CVPixelBufferGetBaseAddressOfPlane(src_buf, 0) as *const u8,
            CVPixelBufferGetBaseAddressOfPlane(src_buf, 1) as *const u8,
            CVPixelBufferGetBytesPerRowOfPlane(src_buf, 0),
            CVPixelBufferGetBytesPerRowOfPlane(src_buf, 1),
            CVPixelBufferGetPixelFormatType(src_buf),
        )
    };

    if src_y.is_null() || src_uv.is_null() {
        return Err(MediaError::Decode {
            reason: "源 CVPixelBuffer 平面基地址指针为空".into(),
        });
    }

    // 目标缓冲沿用源缓冲的像素格式（含量化范围），避免 VideoRange/FullRange 曲线被悄悄改写。
    if src_format != CV_PIXEL_FORMAT_420_VIDEO_RANGE && src_format != CV_PIXEL_FORMAT_420_FULL_RANGE
    {
        return Err(MediaError::Unsupported(
            "CVPixelBuffer 源像素格式非 420 双平面，取景裁剪未实现",
        ));
    }

    let mut dst_buf: *mut std::ffi::c_void = std::ptr::null_mut();
    // 目标缓冲不带 IOSurface 属性（非 IOSurface 支撑）：当前 macOS 消费者按平面锁定在 CPU 上读取
    // （见 image_convert / infer::c_abi::cvpixelbuffer）；若日后接入 Metal / CVTextureCache 零拷贝通道，
    // 需改为带 kCVPixelBufferIOSurfacePropertiesKey 创建，或直接由解码侧提供可硬件裁切的缓冲。
    // SAFETY: 以源缓冲同格式创建目标裁剪尺寸的 CVPixelBuffer
    let create_status = unsafe {
        CVPixelBufferCreate(
            std::ptr::null(),
            crop_w as usize,
            crop_h as usize,
            src_format,
            std::ptr::null(),
            &mut dst_buf,
        )
    };

    if create_status != 0 || dst_buf.is_null() {
        return Err(MediaError::Encode {
            reason: format!("创建目标裁剪 CVPixelBuffer 失败: code={create_status}"),
        });
    }

    // 目标缓冲在此函数内保证释放：正常路径由 FrameHandle 析构触发
    // CVPixelBufferRelease，异常路径由下面的守卫释放，二者互斥。
    struct DstBufferGuard {
        ptr: *mut std::ffi::c_void,
        armed: bool,
    }
    impl DstBufferGuard {
        /// 交出引用所有权给 FrameHandle，禁止守卫再次释放
        fn disarm(mut self) -> *mut std::ffi::c_void {
            self.armed = false;
            self.ptr
        }
    }
    impl Drop for DstBufferGuard {
        fn drop(&mut self) {
            if self.armed {
                // SAFETY: ptr 由 CVPixelBufferCreate 返回且未被任何 FrameHandle 接管
                unsafe {
                    CVPixelBufferRelease(self.ptr);
                }
            }
        }
    }
    let dst_guard = DstBufferGuard {
        ptr: dst_buf,
        armed: true,
    };

    // SAFETY: 0 = kCVPixelBufferLock_ReadWrite，锁定目标缓冲区以写平面
    let dst_lock = unsafe { CVPixelBufferLockBaseAddress(dst_buf, CV_LOCK_READ_WRITE) };
    if dst_lock != 0 {
        return Err(MediaError::Encode {
            reason: format!("锁定目标 CVPixelBuffer 失败: {dst_lock}"),
        });
    }

    // SAFETY: dst_buf 已成功创建并锁定
    let (dst_y, dst_uv, dst_ys, dst_uvs) = unsafe {
        (
            CVPixelBufferGetBaseAddressOfPlane(dst_buf, 0) as *mut u8,
            CVPixelBufferGetBaseAddressOfPlane(dst_buf, 1) as *mut u8,
            CVPixelBufferGetBytesPerRowOfPlane(dst_buf, 0),
            CVPixelBufferGetBytesPerRowOfPlane(dst_buf, 1),
        )
    };

    if dst_y.is_null() || dst_uv.is_null() {
        // SAFETY: 异常路径解锁目标 CVPixelBuffer
        unsafe {
            CVPixelBufferUnlockBaseAddress(dst_buf, CV_LOCK_READ_WRITE);
        }
        return Err(MediaError::Encode {
            reason: "目标 CVPixelBuffer 平面基地址为空".into(),
        });
    }

    // 1. 逐行拷贝 Y 平面
    for r in 0..crop_h as usize {
        // SAFETY: src_y / dst_y 均为已锁定平面基址，行偏移与行宽经 compute_crop_roi 边界保护，不会越界
        unsafe {
            let src_row = src_y.add((sy as usize + r) * src_ys + sx as usize);
            let dst_row = dst_y.add(r * dst_ys);
            std::ptr::copy_nonoverlapping(src_row, dst_row, crop_w as usize);
        }
    }

    // 2. 逐行拷贝 UV 交错平面（1 字节 = 2 个亮度像素的色度）
    for r in 0..(crop_h as usize / 2) {
        // SAFETY: src_uv / dst_uv 均为已锁定的半采样平面基址，行偏移与行宽经 compute_crop_roi 边界保护
        unsafe {
            let src_row = src_uv.add((sy as usize / 2 + r) * src_uvs + sx as usize);
            let dst_row = dst_uv.add(r * dst_uvs);
            std::ptr::copy_nonoverlapping(src_row, dst_row, crop_w as usize);
        }
    }

    // SAFETY: 平面拷贝完成，解锁目标 CVPixelBuffer
    unsafe {
        CVPixelBufferUnlockBaseAddress(dst_buf, CV_LOCK_READ_WRITE);
    }

    let non_null =
        std::ptr::NonNull::new(dst_guard.disarm()).ok_or_else(|| MediaError::Encode {
            reason: "目标 CVPixelBuffer 指针转换失败".into(),
        })?;

    Ok(FrameRef::new(
        frame.camera_id.clone(),
        frame.timestamp,
        crop_w,
        crop_h,
        // CoreVideo 的两个平面行跨步可能不同，消费者按 CVPixelBuffer API 查询平面步长
        // （见 image_convert / infer::c_abi::cvpixelbuffer），此处记录 Y 平面步长。
        StrideInfo::new(dst_ys as u32, crop_h),
        PixelFormat::Nv12,
        FrameHandle::ApplePixelBuffer { ptr: non_null },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12_host_frame(width: u32, height: u32, fill: u8) -> FrameRef {
        let len = (width * height * 3 / 2) as usize;
        FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![fill; len].into()),
        )
    }

    #[test]
    fn test_crop_host_slice_nv12_basic() {
        let (width, height) = (64u32, 64u32);
        let total_size = (width * height * 3 / 2) as usize;
        let mut mock_data = vec![10u8; total_size];

        // 将特写区域 [0.25, 0.25, 0.75, 0.75] 填充为特殊值 99
        let roi = BoundingBox::new(0.25, 0.25, 0.75, 0.75);
        let (sx, sy, cw, ch, _) = compute_crop_roi(width, height, roi, 0.0);
        for y in sy..sy + ch {
            for x in sx..sx + cw {
                mock_data[(y * width + x) as usize] = 99;
            }
        }

        let frame = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Nv12,
            FrameHandle::Host(Arc::from(mock_data)),
        );

        let cropped = crop_frame(&frame, roi).expect("crop_frame should succeed");
        assert_eq!(cropped.frame.width, cw);
        assert_eq!(cropped.frame.height, ch);
        assert_eq!(cropped.frame.format, PixelFormat::Nv12);

        let cropped_slice = match cropped.frame.handle() {
            FrameHandle::Host(slice) => slice.as_ref(),
            _ => panic!("should have host handle"),
        };
        // 验证裁剪出的 Y 平面全为 99
        let dst_stride = cropped.frame.stride.hor_stride as usize;
        for row in 0..ch as usize {
            let row_start = row * dst_stride;
            for col in 0..cw as usize {
                assert_eq!(cropped_slice[row_start + col], 99, "row={row}, col={col}");
            }
        }
    }

    /// 取景矩形覆盖全幅时不得发生任何拷贝，且坐标必须按全景对待。
    #[test]
    fn full_frame_crop_reuses_handle_without_mapping() {
        let frame = nv12_host_frame(64, 64, 7);
        let original_ptr = match frame.handle() {
            FrameHandle::Host(slice) => slice.as_ptr(),
            _ => unreachable!(),
        };

        let cropped = crop_frame(&frame, BoundingBox::new(0.0, 0.0, 1.0, 1.0)).expect("应当成功");
        assert!(cropped.roi.is_none(), "全幅取景不得要求坐标还原");
        match cropped.frame.handle() {
            FrameHandle::Host(slice) => {
                assert_eq!(slice.as_ptr(), original_ptr, "全幅取景应复用原句柄");
            }
            _ => unreachable!(),
        }
    }

    /// 实际生效的取景矩形必须按硬件对齐后的像素矩形回算，而不是配置值。
    #[test]
    fn applied_roi_reflects_aligned_rect() {
        let frame = nv12_host_frame(128, 128, 3);
        // 非对齐输入：x=0.1 → 12.8px → floor 12 → 偶数对齐 12
        let cropped = crop_frame(&frame, BoundingBox::new(0.1, 0.1, 0.6, 0.6)).expect("应当成功");
        let roi = cropped.roi.expect("非全幅取景必须携带生效矩形");
        let (sx, sy, cw, ch, _) =
            compute_crop_roi(128, 128, BoundingBox::new(0.1, 0.1, 0.6, 0.6), 0.0);
        assert!((roi.x1 - sx as f32 / 128.0).abs() < 1e-6);
        assert!((roi.y1 - sy as f32 / 128.0).abs() < 1e-6);
        assert!((roi.x2 - (sx + cw) as f32 / 128.0).abs() < 1e-6);
        assert!((roi.y2 - (sy + ch) as f32 / 128.0).abs() < 1e-6);
        assert_eq!(cropped.frame.width, cw);
        assert_eq!(cropped.frame.height, ch);
    }

    #[test]
    fn packed_formats_validate_source_length() {
        // Bgr24 源按 16 字节对齐行步长存储（hor_stride > 可见宽度，存在水平 padding）：
        // 宿主必须按步长校验整块缓冲长度，长度不足时返回错误而不是越界 panic。
        let (width, height, hor_stride) = (16u32, 8u32, 24u32);
        let padded = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(hor_stride, height),
            PixelFormat::Bgr24,
            FrameHandle::Host(Arc::from(vec![
                5u8;
                hor_stride as usize * 3 * height as usize
            ])),
        );
        let truncated = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(hor_stride, height),
            PixelFormat::Bgr24,
            // 仅提供可见像素长度：缺少 padding 行字节
            FrameHandle::Host(Arc::from(vec![0u8; width as usize * 3 * height as usize])),
        );

        let roi = BoundingBox::new(0.0, 0.0, 0.5, 0.5);
        assert!(
            crop_frame(&padded, roi).is_ok(),
            "行步长大于可见宽度的合法缓冲应可裁切"
        );
        assert!(
            matches!(crop_frame(&truncated, roi), Err(MediaError::Decode { .. })),
            "长度不足的缓冲必须返回错误，不得越界读取"
        );
    }

    /// 设备内存 / DMA-BUF 帧不做宿主侧 CPU 裁切，必须如实上报能力缺失。
    #[test]
    fn device_memory_handle_reports_unsupported() {
        let lease: Arc<dyn Send + Sync> = Arc::new(());
        let frame = FrameRef::new(
            "test_cam".into(),
            1000,
            64,
            64,
            StrideInfo::new(64, 64),
            PixelFormat::Nv12,
            FrameHandle::DeviceMemory {
                ptr: std::ptr::NonNull::dangling(),
                size: 64 * 64 * 3 / 2,
                _lease: lease,
            },
        );

        let err = crop_frame(&frame, BoundingBox::new(0.2, 0.2, 0.8, 0.8)).expect_err("必须拒绝");
        assert!(matches!(err, MediaError::Unsupported(_)), "实际: {err:?}");
    }

    #[test]
    fn tiny_frame_is_rejected() {
        let frame = FrameRef::new(
            "test_cam".into(),
            1000,
            1,
            1,
            StrideInfo::new(1, 1),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![0u8; 8].into()),
        );
        assert!(matches!(
            crop_frame(&frame, BoundingBox::new(0.0, 0.0, 1.0, 1.0)),
            Err(MediaError::Decode { .. })
        ));
    }

    #[test]
    fn rgb24_crop_handles_byte_and_pixel_stride() {
        // 1. 硬件解码器常上报字节跨步 (hor_stride = 64 * 3 = 192)
        let (width, height) = (64u32, 64u32);
        let byte_stride = width * 3;
        let mut buf = vec![0u8; byte_stride as usize * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let off = y * byte_stride as usize + x * 3;
                buf[off] = (x & 0xff) as u8;
                buf[off + 1] = (y & 0xff) as u8;
                buf[off + 2] = 128;
            }
        }

        let frame_byte_stride = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(byte_stride, height),
            PixelFormat::Rgb24,
            FrameHandle::Host(Arc::from(buf.clone())),
        );

        let cropped = crop_frame(&frame_byte_stride, BoundingBox::new(0.0, 0.0, 0.5, 0.5))
            .expect("字节步长 RGB24 裁切应成功");
        assert_eq!(cropped.frame.width, 32);
        assert_eq!(cropped.frame.height, 32);

        // 2. 软件构造帧可能按像素上报跨步 (hor_stride = 64)
        let frame_pixel_stride = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Rgb24,
            FrameHandle::Host(Arc::from(buf)),
        );

        let cropped2 = crop_frame(&frame_pixel_stride, BoundingBox::new(0.0, 0.0, 0.5, 0.5))
            .expect("像素步长 RGB24 裁切应成功");
        assert_eq!(cropped2.frame.width, 32);
        assert_eq!(cropped2.frame.height, 32);
    }

    #[test]
    fn nv12_crop_handles_odd_height() {
        let (width, height) = (64u32, 65u32);
        let uv_rows = height.div_ceil(2);
        let total_bytes = width as usize * (height as usize + uv_rows as usize);
        let buf = vec![128u8; total_bytes];

        let frame = FrameRef::new(
            "test_cam".into(),
            1000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Nv12,
            FrameHandle::Host(Arc::from(buf)),
        );

        // 裁剪接近底部的区域：[0.0, 0.5, 1.0, 1.0]
        let cropped = crop_frame(&frame, BoundingBox::new(0.0, 0.5, 1.0, 1.0))
            .expect("奇数高度 NV12 裁切不应越界 panic");
        assert_eq!(cropped.frame.width, 64);
        assert!(cropped.frame.height >= 32);
    }

    #[test]
    fn zero_sized_frame_is_rejected() {
        let frame = FrameRef::new(
            "test_cam".into(),
            1000,
            0,
            0,
            StrideInfo::new(0, 0),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![0u8; 8].into()),
        );
        assert!(matches!(
            crop_frame(&frame, BoundingBox::new(0.1, 0.1, 0.9, 0.9)),
            Err(MediaError::Decode { .. })
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_cvpixelbuffer_crop_preserves_planes_and_format() {
        // 构造 64×64 的 '420v' 源缓冲，Y 平面按行列填充可校验图案，UV 平面填 200。
        let mut src_buf: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: 测试内创建合法的 64×64 '420v' CVPixelBuffer
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null(),
                64,
                64,
                CV_PIXEL_FORMAT_420_VIDEO_RANGE,
                std::ptr::null(),
                &mut src_buf,
            )
        };
        assert_eq!(status, 0, "CVPixelBufferCreate 应当成功");
        assert!(!src_buf.is_null());

        // SAFETY: 已创建的非空缓冲，读写锁定后填充分辨率足够的平面
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(src_buf, CV_LOCK_READ_WRITE), 0);
            let y = CVPixelBufferGetBaseAddressOfPlane(src_buf, 0) as *mut u8;
            let uv = CVPixelBufferGetBaseAddressOfPlane(src_buf, 1) as *mut u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(src_buf, 0);
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(src_buf, 1);
            for row in 0..64usize {
                for col in 0..64usize {
                    *y.add(row * ys + col) = (row + col) as u8;
                }
            }
            for row in 0..32usize {
                for col in 0..64usize {
                    *uv.add(row * uvs + col) = 200;
                }
            }
            assert_eq!(
                CVPixelBufferUnlockBaseAddress(src_buf, CV_LOCK_READ_WRITE),
                0
            );
        }

        let frame = FrameRef::new(
            "cam_mac_crop".into(),
            1741100000000,
            64,
            64,
            StrideInfo::new(64, 64),
            PixelFormat::Nv12,
            FrameHandle::ApplePixelBuffer {
                ptr: std::ptr::NonNull::new(src_buf).expect("源缓冲非空"),
            },
        );

        // 取景右上 32×32：[0.5, 0.0, 1.0, 0.5] → 期望拷入行列偏移一致的区域
        let cropped =
            crop_frame(&frame, BoundingBox::new(0.5, 0.0, 1.0, 0.5)).expect("裁切应当成功");
        let roi = cropped.roi.expect("必须携带生效矩形");
        assert!((roi.x1 - 0.5).abs() < 1e-6);
        assert_eq!(cropped.frame.width, 32);
        assert_eq!(cropped.frame.height, 32);
        assert_eq!(cropped.frame.format, PixelFormat::Nv12);

        let dst_ptr = match cropped.frame.handle() {
            FrameHandle::ApplePixelBuffer { ptr } => ptr.as_ptr(),
            other => panic!("期望 CVPixelBuffer 句柄，实际 {other:?}"),
        };

        // SAFETY: dst_ptr 由 crop_frame 创建的合法缓冲，只读锁定后校验 Y/UV 平面内容
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(dst_ptr, CV_LOCK_READ_ONLY), 0);
            let y = CVPixelBufferGetBaseAddressOfPlane(dst_ptr, 0) as *const u8;
            let uv = CVPixelBufferGetBaseAddressOfPlane(dst_ptr, 1) as *const u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(dst_ptr, 0);
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(dst_ptr, 1);
            for row in 0..32usize {
                for col in 0..32usize {
                    // 源图案为 (row + col)，目标应为源 (row, col + 32)
                    assert_eq!(
                        *y.add(row * ys + col),
                        (row + col + 32) as u8,
                        "Y plane mismatch"
                    );
                }
            }
            for row in 0..16usize {
                for col in 0..32usize {
                    assert_eq!(*uv.add(row * uvs + col), 200, "UV plane mismatch");
                }
            }
            assert_eq!(
                CVPixelBufferUnlockBaseAddress(dst_ptr, CV_LOCK_READ_ONLY),
                0
            );
        }

        // 目标缓冲由 FrameHandle 析构释放；源缓冲同理。
        drop(cropped);
        drop(frame);
    }
}
