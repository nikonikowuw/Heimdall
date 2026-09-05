//! 视频帧格式转换模块
//!
//! 将 FrameRef 平台原生帧转换为 RGB 图像（用于快照保存、Web 呈现与非加速模型推理）。
//! 所有平台专有显存/缓冲区锁定与像素读取逻辑收敛在此，防止平台差异污染上层 pipeline。

use image::RgbImage;
use types::{FrameHandle, FrameRef, PixelFormat};

use crate::error::MediaError;

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: *mut std::ffi::c_void,
        unlock_flags: u64,
    ) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
    fn CVPixelBufferGetWidth(pixel_buffer: *mut std::ffi::c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut std::ffi::c_void) -> usize;
}

/// 定点数快速 ITU-R BT.601 YUV -> RGB 颜色转换 (放大 1024 倍)
#[inline(always)]
fn yuv_to_rgb_clamped(y_val: i32, u_val: i32, v_val: i32) -> [u8; 3] {
    let r = (y_val + ((1436 * v_val) >> 10)).clamp(0, 255) as u8;
    let g = (y_val - ((352 * u_val + 731 * v_val) >> 10)).clamp(0, 255) as u8;
    let b = (y_val + ((1815 * u_val) >> 10)).clamp(0, 255) as u8;
    [r, g, b]
}

/// 将 FrameRef 转换为标准 RGB 图像
pub fn frame_to_rgb_image(frame: &FrameRef) -> Result<RgbImage, MediaError> {
    let width = frame.width;
    let height = frame.height;

    if width == 0 || height == 0 {
        return Err(MediaError::Decode {
            reason: "帧分辨率非法 (宽/高为 0)".into(),
        });
    }

    match frame.handle() {
        FrameHandle::Host(slice) => match frame.format {
            PixelFormat::Nv12 => {
                let y_stride = frame.stride.hor_stride.max(width) as usize;
                let uv_stride = frame.stride.hor_stride.max(width) as usize;
                fast_nv12_to_rgb_image(slice, width, height, y_stride, uv_stride)
            }
            PixelFormat::Rgb24 => {
                let expected = (width * height * 3) as usize;
                if slice.len() < expected {
                    return Err(MediaError::Decode {
                        reason: "RGB24 数据长度不足".into(),
                    });
                }
                RgbImage::from_raw(width, height, slice[..expected].to_vec()).ok_or_else(|| {
                    MediaError::Decode {
                        reason: "构造 RGB24 图像失败".into(),
                    }
                })
            }
            _ => Err(MediaError::Decode {
                reason: format!("未实现的 Host 像素格式: {:?}", frame.format),
            }),
        },
        #[cfg(target_os = "macos")]
        FrameHandle::ApplePixelBuffer { ptr } => {
            convert_cvpixelbuffer_to_rgb(ptr.as_ptr(), width, height)
        }
        _ => Err(MediaError::Decode {
            reason: "当前平台或当前帧句柄类型不支持直接提取 RGB 图像".into(),
        }),
    }
}

/// 快速整数定点 NV12 到 RgbImage 转换 (ITU-R BT.601)
pub fn fast_nv12_to_rgb_image(
    data: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
) -> Result<RgbImage, MediaError> {
    let w = width as usize;
    let h = height as usize;
    let mut rgb = vec![0u8; w * h * 3];

    let uv_offset = y_stride * h;
    let data_len = data.len();

    for y in 0..h {
        let y_row_start = y * y_stride;
        let uv_row_start = uv_offset + (y >> 1) * uv_stride;
        let rgb_row_start = y * w * 3;

        for x in 0..w {
            let y_idx = y_row_start + x;
            if y_idx >= data_len {
                break;
            }
            let y_val = data[y_idx] as i32;

            let uv_idx = uv_row_start + (x & !1);
            let (u_val, v_val) = if uv_idx + 1 < data_len {
                (data[uv_idx] as i32 - 128, data[uv_idx + 1] as i32 - 128)
            } else {
                (0, 0)
            };

            let out_idx = rgb_row_start + x * 3;
            rgb[out_idx..out_idx + 3].copy_from_slice(&yuv_to_rgb_clamped(y_val, u_val, v_val));
        }
    }

    RgbImage::from_raw(width, height, rgb).ok_or_else(|| MediaError::Decode {
        reason: "创建 RGB 图像缓冲区失败".into(),
    })
}

#[cfg(target_os = "macos")]
fn convert_cvpixelbuffer_to_rgb(
    pixel_buffer: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> Result<RgbImage, MediaError> {
    // SAFETY: 锁定 CVPixelBuffer 读取 NV12 显存，1 = kCVPixelBufferLock_ReadOnly
    let lock_status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 1) };
    if lock_status != 0 {
        return Err(MediaError::Decode {
            reason: format!("CVPixelBufferLockBaseAddress 失败, code: {lock_status}"),
        });
    }

    // 保证函数返回时必须解锁
    struct BufferUnlockGuard(*mut std::ffi::c_void);
    impl Drop for BufferUnlockGuard {
        fn drop(&mut self) {
            // SAFETY: 解锁 CVPixelBuffer
            unsafe {
                CVPixelBufferUnlockBaseAddress(self.0, 1);
            }
        }
    }
    let _guard = BufferUnlockGuard(pixel_buffer);

    // SAFETY: 在锁定生命周期内读取 Y 和 UV 平面基址与行步长
    let (y_ptr, uv_ptr, y_stride, uv_stride) = unsafe {
        let y = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 0) as *const u8;
        let uv = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 1) as *const u8;
        let ys = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 0);
        let uvs = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 1);
        (y, uv, ys, uvs)
    };

    if y_ptr.is_null() || uv_ptr.is_null() {
        return Err(MediaError::Decode {
            reason: "CVPixelBuffer 平面基地址指针为空".into(),
        });
    }

    // SAFETY: 在锁定生命周期内读取实际缓冲区宽高，避免越界读取
    let (actual_w, actual_h) = unsafe {
        (
            CVPixelBufferGetWidth(pixel_buffer),
            CVPixelBufferGetHeight(pixel_buffer),
        )
    };

    let w = (width as usize).min(actual_w);
    let h = (height as usize).min(actual_h);
    let mut rgb = vec![0u8; w * h * 3];

    for y in 0..h {
        let y_row_start = y * y_stride;
        let uv_row_start = (y >> 1) * uv_stride;
        let rgb_row_start = y * w * 3;

        for x in 0..w {
            // SAFETY: y_ptr 和 uv_ptr 在 BufferUnlockGuard 存活期有效，偏移均在平面步长范围内
            let (y_val, u_val, v_val) = unsafe {
                let yv = *y_ptr.add(y_row_start + x) as i32;
                let uv_p = uv_ptr.add(uv_row_start + (x & !1));
                let uv = *uv_p as i32 - 128;
                let vv = *uv_p.add(1) as i32 - 128;
                (yv, uv, vv)
            };

            let out_idx = rgb_row_start + x * 3;
            rgb[out_idx..out_idx + 3].copy_from_slice(&yuv_to_rgb_clamped(y_val, u_val, v_val));
        }
    }

    RgbImage::from_raw(width, height, rgb).ok_or_else(|| MediaError::Decode {
        reason: "创建 RGB 图像缓冲区失败".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FrameHandle, StrideInfo};

    #[test]
    fn test_fast_nv12_to_rgb_conversion() {
        let width: u32 = 4;
        let height: u32 = 4;
        let y_stride = 4usize;
        let uv_stride = 4usize;

        // 全灰 (Y=128, U=128, V=128) -> RGB 应全为 128
        let mut data =
            vec![128u8; y_stride * (height as usize) + uv_stride * (height as usize / 2)];
        let img = fast_nv12_to_rgb_image(&data, width, height, y_stride, uv_stride)
            .expect("快速转换应成功");

        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 4);
        let p = img.get_pixel(0, 0);
        assert_eq!(p.0, [128, 128, 128]);

        // 全白 (Y=255, U=128, V=128) -> RGB 应全为 255
        data[..16].fill(255);
        let img_white = fast_nv12_to_rgb_image(&data, width, height, y_stride, uv_stride)
            .expect("白色图像转换应成功");
        assert_eq!(img_white.get_pixel(1, 1).0, [255, 255, 255]);
    }

    #[test]
    fn test_frame_to_rgb_image_host_nv12() {
        let frame = FrameRef::new(
            "cam_test".into(),
            1741100000000,
            64,
            64,
            StrideInfo::new(64, 64),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![128u8; 64 * 64 * 3 / 2].into()),
        );

        let rgb = frame_to_rgb_image(&frame).expect("frame_to_rgb_image 应成功");
        assert_eq!(rgb.width(), 64);
        assert_eq!(rgb.height(), 64);
    }
}
