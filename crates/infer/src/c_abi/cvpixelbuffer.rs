//! macOS CoreVideo CVPixelBuffer 原生封装
//! 用于为 macOS CoreML 算法包提供零拷贝 NV12 硬件加速帧

#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        width: usize,
        height: usize,
        pixel_format_type: u32,
        pixel_buffer_attributes: *const c_void,
        pixel_buffer_out: *mut *mut c_void,
    ) -> i32;

    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut c_void, unlock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut c_void,
        plane_index: usize,
    ) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer: *mut c_void, plane_index: usize) -> usize;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
}

// FourCC '420v' = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
#[cfg(target_os = "macos")]
pub const K_CVPIXEL_FORMAT_420_YP_CB_CR_8_BI_PLANAR_VIDEO_RANGE: u32 = 0x34323076;

/// RAII 管理的 CVPixelBufferRef
#[derive(Debug)]
pub struct NativePixelBuffer {
    #[cfg(target_os = "macos")]
    raw: *mut c_void,
    #[cfg(not(target_os = "macos"))]
    _dummy: (),
}

// SAFETY: CVPixelBufferRef 是跨线程安全的，支持多线程移动
unsafe impl Send for NativePixelBuffer {}
// SAFETY: CVPixelBufferRef 内部线程安全，支持跨线程并发引用
unsafe impl Sync for NativePixelBuffer {}

impl NativePixelBuffer {
    #[cfg(target_os = "macos")]
    /// 将 RGB 像素转换为原生的 NV12 CVPixelBuffer
    pub fn from_rgb_to_nv12(
        rgb_pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Self, crate::error::InferError> {
        let width = (width & !1) as usize;
        let height = (height & !1) as usize;

        let mut raw_buf: *mut c_void = std::ptr::null_mut();
        // SAFETY: 创建原生 CoreVideo 双平面 NV12 像素缓冲区
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null(),
                width,
                height,
                K_CVPIXEL_FORMAT_420_YP_CB_CR_8_BI_PLANAR_VIDEO_RANGE,
                std::ptr::null(),
                &mut raw_buf,
            )
        };

        if status != 0 || raw_buf.is_null() {
            return Err(crate::error::InferError::Execution {
                reason: format!("CVPixelBufferCreate 失败, status: {status}"),
            });
        }

        // SAFETY: 锁定缓冲区以写入 Y 和 UV 数据
        let lock_status = unsafe { CVPixelBufferLockBaseAddress(raw_buf, 0) };
        if lock_status != 0 {
            // SAFETY: 锁定失败时释放 raw_buf 所有权
            unsafe { CVPixelBufferRelease(raw_buf) };
            return Err(crate::error::InferError::Execution {
                reason: format!("CVPixelBufferLockBaseAddress 失败, status: {lock_status}"),
            });
        }

        // SAFETY: 获取平面 0 (Y) 和平面 1 (UV) 的基地址与步长
        let (y_ptr, uv_ptr, y_stride, uv_stride) = unsafe {
            let y = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 0) as *mut u8;
            let uv = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 1) as *mut u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 0);
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 1);
            (y, uv, ys, uvs)
        };

        let clamp_byte = |v: f64| -> u8 { v.round().clamp(0.0, 255.0) as u8 };

        // 1. 写入 Y 平面 (BT.709 limited range)
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 3;
                let r = rgb_pixels[idx] as f64;
                let g = rgb_pixels[idx + 1] as f64;
                let b = rgb_pixels[idx + 2] as f64;
                let y_val = clamp_byte(16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0);
                // SAFETY: 内存步长与尺寸受 CoreVideo 控制
                unsafe {
                    *y_ptr.add(y * y_stride + x) = y_val;
                }
            }
        }

        // 2. 写入 UV 平面 (2x2 亚采样)
        for y in (0..height).step_by(2) {
            for x in (0..width).step_by(2) {
                let mut cb_sum = 0.0;
                let mut cr_sum = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let idx = ((y + dy) * width + (x + dx)) * 3;
                        let r = rgb_pixels[idx] as f64;
                        let g = rgb_pixels[idx + 1] as f64;
                        let b = rgb_pixels[idx + 2] as f64;
                        cb_sum += 128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0;
                        cr_sum += 128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0;
                    }
                }
                let cb = clamp_byte(cb_sum / 4.0);
                let cr = clamp_byte(cr_sum / 4.0);

                // SAFETY: UV 交替存储 (NV12 格式: U0, V0, U1, V1, ...)
                unsafe {
                    let uv_target = uv_ptr.add((y / 2) * uv_stride + x);
                    *uv_target = cb;
                    *uv_target.add(1) = cr;
                }
            }
        }

        // SAFETY: 解锁缓冲区
        unsafe {
            CVPixelBufferUnlockBaseAddress(raw_buf, 0);
        }

        Ok(Self { raw: raw_buf })
    }

    #[cfg(target_os = "macos")]
    #[inline]
    pub fn as_raw(&self) -> *mut c_void {
        self.raw
    }

    #[cfg(target_os = "macos")]
    #[inline]
    pub fn strides(&self) -> (i32, i32) {
        // SAFETY: 查询 CoreVideo 平面步长
        unsafe {
            let ys = CVPixelBufferGetBytesPerRowOfPlane(self.raw, 0) as i32;
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(self.raw, 1) as i32;
            (ys, uvs)
        }
    }
}

impl Drop for NativePixelBuffer {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if !self.raw.is_null() {
            // SAFETY: 释放 CoreVideo 像素缓冲区所有权
            unsafe {
                CVPixelBufferRelease(self.raw);
            }
            self.raw = std::ptr::null_mut();
        }
    }
}
