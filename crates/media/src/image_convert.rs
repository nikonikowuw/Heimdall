//! 图像格式转换与低频证据回读模块
//!
//! ### 核心架构与路径划分约束（严格三路径分离）：
//!
//! 1. **常驻推理主路径 (`infer_fast_path`)**：
//!    - 流转链路：`VPU / DVPP -> RGA / VPC / AIPP -> RKNN / ACL / ANE`
//!    - 内存驻留：全程保持在物理连续设备显存/统一内存内（`DMA-BUF` / `DeviceMemory` / `CVPixelBuffer`），
//!      实现纯设备侧零拷贝直通推理，**严禁在常驻推理路径中调用本模块的任何图像转换或回读函数**！
//! 2. **低频证据生成路径 (`snapshot_readback_path`)**：
//!    - 流转链路：`VPU / DVPP 硬件句柄 -> Device-to-Host DMA 回读 / 内核 Cache 同步 -> CPU NV12-to-RGB -> JPEG 编码落盘`
//!    - 触发频率：仅在规则引擎命中有违规告警，或人工触发抓拍时按需单帧触发（低频离散事件）；
//!    - 主要入口：[`snapshot_readback_to_rgb_image`]；
//!    - 架构定性：此路径为低频证据生成的显式特例，包含物理 D2H 搬运与 CPU 色彩计算，绝对不属于常驻推理零拷贝。
//! 3. **开发调试回退路径 (`debug_cpu_fallback_path`)**：
//!    - 流转链路：Host 内存切片 -> 纯 CPU 软件定点数 NV12-to-RGB
//!    - 适用场景：本地开发机、单元测试桩或物理上无 NPU/VPU 的保底环境；
//!    - 主要入口：[`debug_cpu_fallback_nv12_to_rgb`]。

use image::RgbImage;
use types::{FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::error::MediaError;

// ============================================================================
// macOS: Apple Accelerate framework (vImage SIMD 硬件加速) 与 CVPixelBuffer 绑定
// ============================================================================

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

#[cfg(target_os = "macos")]
#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn vImageConvert_YpCbCrToARGB_GenerateConversion(
        matrix: *const std::ffi::c_void,
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
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct vImage_Buffer {
    data: *mut std::ffi::c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct vImage_YpCbCrToARGBMatrix {
    yp: f32,
    cr_r: f32,
    cr_g: f32,
    cb_g: f32,
    cb_b: f32,
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[repr(C, align(16))]
#[derive(Copy, Clone)]
struct vImage_YpCbCrToARGB {
    opaque: [u8; 128],
}

// ============================================================================
// 统一分发入口：严格区隔 snapshot_readback_path 与 debug_cpu_fallback_path
// ============================================================================

/// [snapshot_readback_path] 低频证据生成路径：将 FrameRef 平台原生帧回读并转换为 RGB 图像
///
/// ### 严正路径约束：
/// - **仅用于低频证据路径 (`snapshot_readback_path`)**：
///   如规则引擎告警触发时的快照保存、特写抠图与 Web 管理台人工抓拍展示；
/// - **包含物理 Device-to-Host DMA 传输与 CPU 色彩计算**：
///   - 昇腾 DVPP：通过 `aclrtMemcpy(D2H)` 从 DeviceMemory 显存读回 Host 内存；
///   - 瑞芯微 MPP：通过 `mmap` 与 `DMA_BUF_IOCTL_SYNC` 将连续物理页映射进 CPU 并同步 Cache；
///   - 苹果 macOS：通过 `CVPixelBufferLockBaseAddress` 锁定 CPU 虚拟地址并由 vImage SIMD 转换；
/// - **严禁在常驻推理路径 (`infer_fast_path`) 中调用**：
///   NPU 推理主路径必须走 `DVPP -> VPC/AIPP -> ACL` 或 `MPP -> RGA -> RKNN` 的物理设备侧直通，
///   绝对不走 Host 内存回读与本函数的任何逻辑！
pub fn snapshot_readback_to_rgb_image(frame: &FrameRef) -> Result<RgbImage, MediaError> {
    let width = frame.width;
    let height = frame.height;

    if width == 0 || height == 0 {
        return Err(MediaError::Decode {
            reason: "帧分辨率非法 (宽/高为 0)".into(),
        });
    }

    match frame.handle() {
        FrameHandle::Host(slice) => {
            // [debug_cpu_fallback_path] Host 内存回退转换
            debug_cpu_fallback_nv12_to_rgb(slice, width, height, frame.stride, frame.format)
        }
        #[cfg(target_os = "macos")]
        FrameHandle::ApplePixelBuffer { ptr } => {
            convert_cvpixelbuffer_to_rgb(ptr.as_ptr(), width, height)
        }
        #[cfg(not(target_os = "macos"))]
        FrameHandle::ApplePixelBuffer { .. } => Err(MediaError::Decode {
            reason: "ApplePixelBuffer 仅支持 macOS".into(),
        }),
        #[cfg(target_os = "linux")]
        FrameHandle::DmaBuf { fd, .. } => {
            convert_dmabuf_to_rgb(fd.as_ref(), width, height, frame.stride)
        }
        FrameHandle::DeviceMemory { ptr, size, .. } => {
            convert_devicememory_to_rgb(*ptr, *size, width, height, frame.stride)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        _ => Err(MediaError::Decode {
            reason: "当前操作系统平台不支持直接提取原生硬件加速帧".into(),
        }),
    }
}

/// 兼容别名：调用 [`snapshot_readback_to_rgb_image`]
///
/// 架构警告：此函数执行的是低频证据生成路径（snapshot_readback_path），严禁用于常驻推理！
#[inline]
pub fn frame_to_rgb_image(frame: &FrameRef) -> Result<RgbImage, MediaError> {
    snapshot_readback_to_rgb_image(frame)
}

/// [debug_cpu_fallback_path] 开发与调试 CPU 回退路径专用的色彩转换
pub fn debug_cpu_fallback_nv12_to_rgb(
    slice: &[u8],
    width: u32,
    height: u32,
    stride: StrideInfo,
    format: PixelFormat,
) -> Result<RgbImage, MediaError> {
    match format {
        PixelFormat::Nv12 => {
            let y_stride = stride.hor_stride.max(width) as usize;
            let uv_stride = stride.hor_stride.max(width) as usize;
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
            reason: format!("未实现的 Host 像素格式: {:?}", format),
        }),
    }
}

// ============================================================================
// macOS 平台实现：Apple Accelerate vImage 硬件向量加速与 CPU 双模容灾
// ============================================================================

#[cfg(target_os = "macos")]
fn convert_cvpixelbuffer_to_rgb(
    pixel_buffer: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> Result<RgbImage, MediaError> {
    // 1. 优先调用 Apple Accelerate 硬件向量加速转换 (NEON / AMX 指令直通)
    if let Ok(img) = convert_cvpixelbuffer_to_rgb_accelerate(pixel_buffer, width, height) {
        return Ok(img);
    }
    // 2. 硬件加速失败或环境限制时，无缝回退至 CPU 定点数转换
    convert_cvpixelbuffer_to_rgb_cpu(pixel_buffer, width, height)
}

#[cfg(target_os = "macos")]
fn convert_cvpixelbuffer_to_rgb_accelerate(
    pixel_buffer: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> Result<RgbImage, MediaError> {
    // SAFETY: 锁定 CVPixelBuffer 基础地址以只读方式访问，1 = kCVPixelBufferLock_ReadOnly
    let lock_status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 1) };
    if lock_status != 0 {
        return Err(MediaError::Decode {
            reason: format!("CVPixelBufferLockBaseAddress 失败, code: {lock_status}"),
        });
    }

    struct BufferUnlockGuard(*mut std::ffi::c_void);
    impl Drop for BufferUnlockGuard {
        fn drop(&mut self) {
            // SAFETY: 解除 CVPixelBuffer 锁定
            unsafe {
                CVPixelBufferUnlockBaseAddress(self.0, 1);
            }
        }
    }
    let _guard = BufferUnlockGuard(pixel_buffer);

    // SAFETY: 提取 Y 平面与 UV 交叉平面的数据指针与跨度
    let (y_ptr, uv_ptr, y_stride, uv_stride) = unsafe {
        let y = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 0);
        let uv = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 1);
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

    let src_y = vImage_Buffer {
        data: y_ptr,
        height: h,
        width: w,
        row_bytes: y_stride,
    };

    let src_cbcr = vImage_Buffer {
        data: uv_ptr,
        height: h.div_ceil(2),
        width: w.div_ceil(2),
        row_bytes: uv_stride,
    };

    // 标准 ITU-R BT.601 转换矩阵定义
    let matrix = vImage_YpCbCrToARGBMatrix {
        yp: 1.0,
        cr_r: 1.402,
        cr_g: -0.7141,
        cb_g: -0.3441,
        cb_b: 1.772,
    };

    let pixel_range = vImage_YpCbCrPixelRange {
        yp_bias: 16,
        cb_cr_bias: 128,
        yp_range_max: 235,
        cb_cr_range_max: 240,
        yp_max: 255,
        yp_min: 0,
        cb_cr_max: 255,
        cb_cr_min: 0,
    };

    let mut out_info = vImage_YpCbCrToARGB { opaque: [0u8; 128] };

    // SAFETY: 初始化 Accelerate 色彩空间转换信息结构
    let gen_err = unsafe {
        vImageConvert_YpCbCrToARGB_GenerateConversion(
            &matrix as *const _ as *const std::ffi::c_void,
            &pixel_range,
            &mut out_info,
            4, // kvImage420Yp8_CbCr8 ('420v')
            0, // kvImageARGB8888
            0,
        )
    };

    if gen_err != 0 {
        return Err(MediaError::Decode {
            reason: format!("vImageConvert_YpCbCrToARGB_GenerateConversion 错误: {gen_err}"),
        });
    }

    // 中间 RGBA8888 缓冲区
    let mut rgba_buf = vec![0u8; w * h * 4];
    let dest_rgba = vImage_Buffer {
        data: rgba_buf.as_mut_ptr() as *mut std::ffi::c_void,
        height: h,
        width: w,
        row_bytes: w * 4,
    };

    // permute_map: [1, 2, 3, 0] 将底层 ARGB 映射输出为 RGBA8888
    let permute_map = [1u8, 2, 3, 0];

    // SAFETY: 执行硬件加速 NV12 -> RGBA8888 转换
    let conv_err = unsafe {
        vImageConvert_420Yp8_CbCr8ToARGB8888(
            &src_y,
            &src_cbcr,
            &dest_rgba,
            &out_info,
            permute_map.as_ptr(),
            255,
            0,
        )
    };

    if conv_err != 0 {
        return Err(MediaError::Decode {
            reason: format!("vImageConvert_420Yp8_CbCr8ToARGB8888 错误: {conv_err}"),
        });
    }

    // 最终紧凑 RGB888 目标内存
    let mut rgb_buf = vec![0u8; w * h * 3];
    let dest_rgb = vImage_Buffer {
        data: rgb_buf.as_mut_ptr() as *mut std::ffi::c_void,
        height: h,
        width: w,
        row_bytes: w * 3,
    };

    // SAFETY: 执行 RGBA8888 -> RGB888 压缩转换
    let pack_err = unsafe { vImageConvert_RGBA8888toRGB888(&dest_rgba, &dest_rgb, 0) };

    if pack_err != 0 {
        return Err(MediaError::Decode {
            reason: format!("vImageConvert_RGBA8888toRGB888 错误: {pack_err}"),
        });
    }

    RgbImage::from_raw(w as u32, h as u32, rgb_buf).ok_or_else(|| MediaError::Decode {
        reason: "创建硬件加速 RGB 图像失败".into(),
    })
}

#[cfg(target_os = "macos")]
fn convert_cvpixelbuffer_to_rgb_cpu(
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

    struct BufferUnlockGuard(*mut std::ffi::c_void);
    impl Drop for BufferUnlockGuard {
        fn drop(&mut self) {
            // SAFETY: 解锁 CVPixelBuffer 基础地址
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

        let mut x = 0;
        while x + 1 < w {
            // SAFETY: y_ptr 和 uv_ptr 在 BufferUnlockGuard 存活期有效，偏移在实际平面步长内
            let (y1, y2, u_val, v_val) = unsafe {
                let y1 = *y_ptr.add(y_row_start + x) as i32;
                let y2 = *y_ptr.add(y_row_start + x + 1) as i32;
                let uv_p = uv_ptr.add(uv_row_start + x);
                let u = *uv_p as i32 - 128;
                let v = *uv_p.add(1) as i32 - 128;
                (y1, y2, u, v)
            };

            let c_r = (1436 * v_val) >> 10;
            let c_g = (352 * u_val + 731 * v_val) >> 10;
            let c_b = (1815 * u_val) >> 10;

            let out_idx = rgb_row_start + x * 3;
            rgb[out_idx] = (y1 + c_r).clamp(0, 255) as u8;
            rgb[out_idx + 1] = (y1 - c_g).clamp(0, 255) as u8;
            rgb[out_idx + 2] = (y1 + c_b).clamp(0, 255) as u8;

            rgb[out_idx + 3] = (y2 + c_r).clamp(0, 255) as u8;
            rgb[out_idx + 4] = (y2 - c_g).clamp(0, 255) as u8;
            rgb[out_idx + 5] = (y2 + c_b).clamp(0, 255) as u8;

            x += 2;
        }

        if x < w {
            // SAFETY: y_ptr 与 uv_ptr 在锁定生命周期内有效，行步长与尾像素偏移在分配范围内
            let (y_val, u_val, v_val) = unsafe {
                let y = *y_ptr.add(y_row_start + x) as i32;
                let uv_p = uv_ptr.add(uv_row_start + (x & !1));
                let u = *uv_p as i32 - 128;
                let v = *uv_p.add(1) as i32 - 128;
                (y, u, v)
            };

            let c_r = (1436 * v_val) >> 10;
            let c_g = (352 * u_val + 731 * v_val) >> 10;
            let c_b = (1815 * u_val) >> 10;

            let out_idx = rgb_row_start + x * 3;
            rgb[out_idx] = (y_val + c_r).clamp(0, 255) as u8;
            rgb[out_idx + 1] = (y_val - c_g).clamp(0, 255) as u8;
            rgb[out_idx + 2] = (y_val + c_b).clamp(0, 255) as u8;
        }
    }

    RgbImage::from_raw(w as u32, h as u32, rgb).ok_or_else(|| MediaError::Decode {
        reason: "创建 RGB 图像缓冲区失败".into(),
    })
}

// ============================================================================
// Linux 平台实现：[snapshot_readback_path] 内核 DMA-BUF mmap / cache-sync 映射与 CPU 转换
// ============================================================================

/// [snapshot_readback_path] Linux DMA-BUF 句柄 -> 内核 mmap 映射与 CPU 图像转换
///
/// ### 严正路径声明：
/// 该路径通过 `mmap` 与 `DMA_BUF_IOCTL_SYNC` 将物理连续页映射到 CPU 空间执行软转换；
/// 仅用于低频快照存盘，常驻推理路径 (`infer_fast_path`) 走 `MPP -> RGA -> RKNN` DMA-BUF 零拷贝直通。
#[cfg(target_os = "linux")]
fn convert_dmabuf_to_rgb(
    fd: &std::os::fd::OwnedFd,
    width: u32,
    height: u32,
    stride: StrideInfo,
) -> Result<RgbImage, MediaError> {
    use crate::dmabuf_sync::{wait_dmabuf_readable, DmaBufSyncDirection, DmaBufSyncGuard};
    use std::os::fd::AsRawFd;

    let raw_fd = fd.as_raw_fd();
    if raw_fd < 0 {
        return Err(MediaError::Decode {
            reason: "非法 DMA-BUF 文件描述符".to_string(),
        });
    }

    // 1. 硬件栅障等待（Operation Ordering 证明）：等待硬件 Producer（VPU 或 RGA）彻底完成写入
    wait_dmabuf_readable(raw_fd, 100)?;

    let y_stride = stride.hor_stride.max(width) as usize;
    let uv_stride = stride.hor_stride.max(width) as usize;
    let ver_stride = stride.ver_stride.max(height) as usize;
    let total_size = y_stride
        .checked_mul(ver_stride)
        .and_then(|v| v.checked_mul(3))
        .map(|v| v / 2)
        .ok_or_else(|| MediaError::Decode {
            reason: "步长乘法溢出".to_string(),
        })?;

    // 2. Linux 内核级 mmap 映射 DMA-BUF 连续物理页
    // SAFETY: 基于具有生命周期的 OwnedFd 进行只读共享映射
    let map_ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            total_size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            raw_fd,
            0,
        )
    };

    if map_ptr == libc::MAP_FAILED || map_ptr.is_null() {
        return Err(MediaError::Decode {
            reason: format!(
                "mmap(dma_buf_fd) 映射失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }

    struct MmapGuard {
        ptr: *mut std::ffi::c_void,
        len: usize,
    }
    impl Drop for MmapGuard {
        fn drop(&mut self) {
            // SAFETY: 解除虚拟内存页映射
            unsafe {
                libc::munmap(self.ptr, self.len);
            }
        }
    }
    let _guard = MmapGuard {
        ptr: map_ptr,
        len: total_size,
    };

    // 3. 启用带 EINTR/EAGAIN 循环重试与 RAII 自动回退的 CPU 缓存一致性同步 (Cache Coherency)
    let _sync_guard = DmaBufSyncGuard::acquire(raw_fd, DmaBufSyncDirection::Read)?;

    // 4. 安全读取内存切片并转换为 RgbImage
    // SAFETY: map_ptr 在 MmapGuard 存活期间为有效的映射内存地址，长度为 total_size
    let slice = unsafe { std::slice::from_raw_parts(map_ptr as *const u8, total_size) };
    fast_nv12_to_rgb_image(slice, width, height, y_stride, uv_stride)
    // 析构顺序：
    // 1. _sync_guard Drop -> 自动调用 DMA_BUF_SYNC_END 结束读同步
    // 2. _guard Drop -> 自动调用 munmap 解除内存映射
}

// ============================================================================
// 昇腾平台实现：[snapshot_readback_path] Ascend DVPP DeviceMemory 设备显存回读与 CPU 转换
// ============================================================================

/// [snapshot_readback_path] 华为昇腾 DVPP 设备显存 -> Host 内存回读与 CPU 图像转换
///
/// ### 严正路径声明：
/// 该路径包含：
/// 1. `aclrtMemcpy(ACL_MEMCPY_DEVICE_TO_HOST)`：将 1080p/4K 显存读回 Host CPU 内存；
/// 2. CPU ITU-R BT.601 定点数 NV12 -> RGB 转换。
///
/// 仅作为“告警快照证据生成路径”的显式特例存在；
/// 绝对不属于常驻 NPU 推理主路径（`infer_fast_path`），推理主路径走 `DVPP -> VPC/AIPP -> ACL` 物理设备侧直通。
fn convert_devicememory_to_rgb(
    ptr: std::ptr::NonNull<std::ffi::c_void>,
    size: usize,
    width: u32,
    height: u32,
    stride: StrideInfo,
) -> Result<RgbImage, MediaError> {
    #[cfg(all(target_os = "linux", feature = "dvpp"))]
    {
        let mut host_data = vec![0u8; size];
        // SAFETY: 调用 AscendCL aclrtMemcpy 将 Device 显存高速 DMA 直通复制至 Host 内存
        let ret = unsafe {
            crate::decoders::dvpp::ffi::aclrtMemcpy(
                host_data.as_mut_ptr() as *mut std::ffi::c_void,
                size,
                ptr.as_ptr(),
                size,
                crate::decoders::dvpp::ffi::ACL_MEMCPY_DEVICE_TO_HOST,
            )
        };
        if ret != 0 {
            return Err(MediaError::Decode {
                reason: format!("DVPP DeviceMemory -> Host aclrtMemcpy 传输失败, 错误码: {ret}"),
            });
        }
        let y_stride = stride.hor_stride.max(width) as usize;
        let uv_stride = stride.hor_stride.max(width) as usize;
        fast_nv12_to_rgb_image(&host_data, width, height, y_stride, uv_stride)
    }

    #[cfg(not(all(target_os = "linux", feature = "dvpp")))]
    {
        let _ = (ptr, size, width, height, stride);
        Err(MediaError::Decode {
            reason: "当前运行环境未启用 Linux 昇腾 DVPP 特性，无法直接访问昇腾 DeviceMemory"
                .to_string(),
        })
    }
}

// ============================================================================
// 通用 CPU 定点数颜色空间转换算法 (ITU-R BT.601)
// ============================================================================

/// 定点数快速 ITU-R BT.601 YUV -> RGB 颜色转换 (放大 1024 倍)
#[allow(dead_code)]
#[inline(always)]
fn yuv_to_rgb_clamped(y_val: i32, u_val: i32, v_val: i32) -> [u8; 3] {
    let r = (y_val + ((1436 * v_val) >> 10)).clamp(0, 255) as u8;
    let g = (y_val - ((352 * u_val + 731 * v_val) >> 10)).clamp(0, 255) as u8;
    let b = (y_val + ((1815 * u_val) >> 10)).clamp(0, 255) as u8;
    [r, g, b]
}

/// 快速整数定点 NV12 到 RgbImage 转换 (ITU-R BT.601)
///
/// 工业级成对像素加速 (Paired-Pixel Acceleration)：
/// 1. NV12 水平相邻两像素共享同一组 UV 色度样本，色度增量 (c_r, c_g, c_b) 仅计算一次，消除 50% 移位乘法；
/// 2. 行级安全区间预检 (Row-level bounds pre-check)，在连续安全内存区消除每像素分支预测开销；
/// 3. 直接写入预分配连续 RGB 切片，杜绝小数组构造与 copy_from_slice 开销。
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

        // 快速安全路径：当本行所有 Y 与 UV 像素完整落在 data 切片内时，走无分支直通
        if y_row_start + w <= data_len && uv_row_start + w.div_ceil(2) * 2 <= data_len {
            let mut x = 0;
            while x + 1 < w {
                let y1 = data[y_row_start + x] as i32;
                let y2 = data[y_row_start + x + 1] as i32;

                let uv_idx = uv_row_start + x;
                let u_val = data[uv_idx] as i32 - 128;
                let v_val = data[uv_idx + 1] as i32 - 128;

                let c_r = (1436 * v_val) >> 10;
                let c_g = (352 * u_val + 731 * v_val) >> 10;
                let c_b = (1815 * u_val) >> 10;

                let out_idx = rgb_row_start + x * 3;
                rgb[out_idx] = (y1 + c_r).clamp(0, 255) as u8;
                rgb[out_idx + 1] = (y1 - c_g).clamp(0, 255) as u8;
                rgb[out_idx + 2] = (y1 + c_b).clamp(0, 255) as u8;

                rgb[out_idx + 3] = (y2 + c_r).clamp(0, 255) as u8;
                rgb[out_idx + 4] = (y2 - c_g).clamp(0, 255) as u8;
                rgb[out_idx + 5] = (y2 + c_b).clamp(0, 255) as u8;

                x += 2;
            }

            // 处理可能存在的奇数尾像素
            if x < w {
                let y_val = data[y_row_start + x] as i32;
                let uv_idx = uv_row_start + x;
                let u_val = data[uv_idx] as i32 - 128;
                let v_val = data[uv_idx + 1] as i32 - 128;
                let c_r = (1436 * v_val) >> 10;
                let c_g = (352 * u_val + 731 * v_val) >> 10;
                let c_b = (1815 * u_val) >> 10;

                let out_idx = rgb_row_start + x * 3;
                rgb[out_idx] = (y_val + c_r).clamp(0, 255) as u8;
                rgb[out_idx + 1] = (y_val - c_g).clamp(0, 255) as u8;
                rgb[out_idx + 2] = (y_val + c_b).clamp(0, 255) as u8;
            }
        } else {
            // 安全容灾保底路径：严格带边界保护
            let mut x = 0;
            while x < w {
                let y_idx = y_row_start + x;
                if y_idx >= data_len {
                    break;
                }
                let y_val = data[y_idx] as i32;

                let uv_idx = uv_row_start + (x & !1);
                let (u_val, v_val) = if uv_idx + 1 < data_len {
                    (data[uv_idx] as i32 - 128, data[uv_idx + 1] as i32 - 128)
                } else if uv_idx < data_len {
                    (data[uv_idx] as i32 - 128, 0)
                } else {
                    (0, 0)
                };

                let c_r = (1436 * v_val) >> 10;
                let c_g = (352 * u_val + 731 * v_val) >> 10;
                let c_b = (1815 * u_val) >> 10;

                let out_idx = rgb_row_start + x * 3;
                rgb[out_idx] = (y_val + c_r).clamp(0, 255) as u8;
                rgb[out_idx + 1] = (y_val - c_g).clamp(0, 255) as u8;
                rgb[out_idx + 2] = (y_val + c_b).clamp(0, 255) as u8;
                x += 1;
            }
        }
    }

    RgbImage::from_raw(width, height, rgb).ok_or_else(|| MediaError::Decode {
        reason: "创建 RGB 图像缓冲区失败".into(),
    })
}

// ============================================================================
// 自动化测试集
// ============================================================================

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
    fn test_fast_nv12_to_rgb_conversion_odd_dimensions_and_stride() {
        let width: u32 = 5;
        let height: u32 = 3;
        let y_stride = 8usize;
        let uv_stride = 8usize;

        // 构造包含奇数宽高与跨步对齐的 NV12 缓冲区
        let total_bytes = y_stride * (height as usize) + uv_stride * (height as usize).div_ceil(2);
        let mut data = vec![128u8; total_bytes];

        // 设置第一行像素为特定值
        data[0] = 200;
        data[1] = 100;
        data[2] = 150;
        data[3] = 50;
        data[4] = 80;

        let img = fast_nv12_to_rgb_image(&data, width, height, y_stride, uv_stride)
            .expect("奇数宽高图像转换应成功");

        assert_eq!(img.width(), 5);
        assert_eq!(img.height(), 3);
        assert_eq!(img.get_pixel(0, 0).0, [200, 200, 200]);
        assert_eq!(img.get_pixel(4, 0).0, [80, 80, 80]);
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

    #[test]
    fn test_frame_to_rgb_image_host_rgb24() {
        let frame = FrameRef::new(
            "cam_test".into(),
            1741100000000,
            32,
            32,
            StrideInfo::new(32 * 3, 32),
            PixelFormat::Rgb24,
            FrameHandle::Host(vec![200u8; 32 * 32 * 3].into()),
        );

        let rgb = frame_to_rgb_image(&frame).expect("frame_to_rgb_image 应成功");
        assert_eq!(rgb.width(), 32);
        assert_eq!(rgb.height(), 32);
        assert_eq!(rgb.get_pixel(0, 0).0, [200, 200, 200]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_apple_pixelbuffer_rejected_on_non_macos() {
        let frame = FrameRef::new(
            "cam_non_macos_test".into(),
            1741100000000,
            1,
            1,
            StrideInfo::new(1, 1),
            PixelFormat::Nv12,
            FrameHandle::ApplePixelBuffer {
                ptr: std::ptr::NonNull::dangling(),
            },
        );

        assert!(matches!(
            frame_to_rgb_image(&frame),
            Err(MediaError::Decode { reason }) if reason == "ApplePixelBuffer 仅支持 macOS"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_accelerate_cvpixelbuffer_conversion() {
        // 创建一个实际的 CVPixelBuffer 并验证 Accelerate 硬件转换
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
        }

        let mut pixel_buffer: *mut std::ffi::c_void = std::ptr::null_mut();
        // 0x34323076 = '420v' = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        // SAFETY: 测试中创建 128x128 尺寸的合法 CVPixelBuffer
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null(),
                128,
                128,
                0x34323076,
                std::ptr::null(),
                &mut pixel_buffer,
            )
        };

        assert_eq!(status, 0, "CVPixelBufferCreate 应当成功");
        assert!(!pixel_buffer.is_null());

        let frame = FrameRef::new(
            "cam_mac_test".into(),
            1741100000000,
            128,
            128,
            StrideInfo::new(128, 128),
            PixelFormat::Nv12,
            FrameHandle::ApplePixelBuffer {
                ptr: std::ptr::NonNull::new(pixel_buffer).expect("pixel_buffer 不应为空指针"),
            },
        );

        let rgb = frame_to_rgb_image(&frame).expect("Apple 硬件加速转换应当成功");
        assert_eq!(rgb.width(), 128);
        assert_eq!(rgb.height(), 128);
    }
}
