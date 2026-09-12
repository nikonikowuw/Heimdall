//! Linux Rockchip RGA 硬件加速裁剪 FFI（dlopen 动态加载）
//!
//! 独立于 `algo-sdk` 的最小化 RGA 接口，仅暴露快照裁剪所需的子集：
//! - `importbuffer_fd`：DMA-BUF fd → RGA handle
//! - `releasebuffer_handle`：释放 RGA handle
//! - `wrapbuffer_handle_t`：RGA handle → `RgaBuffer` 结构体
//! - `imcheck_t`：裁剪参数合法性校验
//! - `improcess`：执行异步/同步 blit（crop + pad）
//! - `imfill_t`：边界黑色填充
//!
//! 通过 `dlopen("librga.so")` 动态加载，编译时无硬依赖。

#![cfg(all(target_os = "linux", feature = "rga"))]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::Mutex;

use tracing::{debug, warn};

use crate::dmabuf_sync::wait_dmabuf_readable;
use crate::error::MediaError;

// ============================================================================
// librga 常量
// ============================================================================

const IM_STATUS_SUCCESS: c_int = 1;
const IM_STATUS_NOERROR: c_int = 2;
const IM_SYNC: c_int = 1 << 19;
/// 目标 RGA BSP 的安全公共最小 ROI；不足时由上层转 CPU crop。
const RGA_MIN_DIMENSION: u32 = 68;

/// RK_FORMAT_YCbCr_420_SP = 0x100 (NV12)
#[allow(non_upper_case_globals)]
pub const RK_FORMAT_YCbCr_420_SP: c_int = 0x100;

// ============================================================================
// librga C 结构体（ABI 对齐 im2d_type.h）
// ============================================================================

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ImHandleParam {
    pub width: u32,
    pub height: u32,
    pub format: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ImRect {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RgaBuffer {
    pub vir_addr: *mut c_void,
    pub phy_addr: *mut c_void,
    pub fd: c_int,
    pub width: c_int,
    pub height: c_int,
    pub wstride: c_int,
    pub hstride: c_int,
    pub format: c_int,
    pub color_space_mode: c_int,
    pub global_alpha: c_int,
    pub rd_mode: c_int,
    pub color: c_int,
    pub colorkey_range: [c_int; 2],
    pub nn: [c_int; 6],
    pub rop_code: c_int,
    pub handle: u32,
}

// ============================================================================
// 函数指针类型
// ============================================================================

type ImportBufferFn = unsafe extern "C" fn(fd: c_int, param: *mut ImHandleParam) -> u32;
type ReleaseBufferFn = unsafe extern "C" fn(handle: u32) -> c_int;
type WrapBufferFn = unsafe extern "C" fn(
    handle: u32,
    width: c_int,
    height: c_int,
    wstride: c_int,
    hstride: c_int,
    format: c_int,
) -> RgaBuffer;
type ImCheckFn = unsafe extern "C" fn(
    src: RgaBuffer,
    dst: RgaBuffer,
    pat: RgaBuffer,
    src_rect: ImRect,
    dst_rect: ImRect,
    pat_rect: ImRect,
    mode: c_int,
) -> c_int;
type ImProcessFn = unsafe extern "C" fn(
    src: RgaBuffer,
    dst: RgaBuffer,
    pat: RgaBuffer,
    src_rect: ImRect,
    dst_rect: ImRect,
    pat_rect: ImRect,
    mode: c_int,
) -> c_int;
type ImFillFn =
    unsafe extern "C" fn(dst: RgaBuffer, rect: ImRect, color: c_int, sync_mode: c_int) -> c_int;
type ImStrErrorFn = unsafe extern "C" fn(status: c_int) -> *const c_char;

// ============================================================================
// RGA 运行时（dlopen 加载）
// ============================================================================

struct DlopenGuard(NonNull<c_void>);

impl DlopenGuard {
    fn into_inner(self) -> NonNull<c_void> {
        let lib = self.0;
        std::mem::forget(self);
        lib
    }
}

impl Drop for DlopenGuard {
    fn drop(&mut self) {
        // SAFETY: 句柄来自 dlopen，且只在解析失败路径由此 guard 持有。
        unsafe {
            libc::dlclose(self.0.as_ptr());
        }
    }
}

struct RgaFfi {
    import_buffer: ImportBufferFn,
    release_buffer: ReleaseBufferFn,
    wrap_buffer: WrapBufferFn,
    check: ImCheckFn,
    process: ImProcessFn,
    fill: ImFillFn,
    error_string: ImStrErrorFn,
}

// ============================================================================
// RGA 导入句柄 RAII 守卫
// ============================================================================

/// RAII 守卫：包装 RGA `importbuffer_fd` 返回的 handle，析构时自动释放。
/// 防止手动 `release_buffer` 路径因 panic 或提前 return 导致句柄泄漏。
struct RgaImportGuard {
    ffi: *const RgaFfi,
    handle: u32,
}

impl RgaImportGuard {
    /// 创建守卫。`handle == 0` 表示无效，Drop 中会跳过释放。
    fn new(ffi: *const RgaFfi, handle: u32) -> Self {
        Self { ffi, handle }
    }
}

impl Drop for RgaImportGuard {
    fn drop(&mut self) {
        if self.handle != 0 && !self.ffi.is_null() {
            // SAFETY: ffi 指针在 RgaRuntime 生命周期内有效，
            // RgaImportGuard 仅在 crop_blit_sync 栈帧内使用，不逃逸到外层
            unsafe { ((*self.ffi).release_buffer)(self.handle) };
        }
    }
}

/// RGA 裁剪与填充任务参数结构体（收敛多参数，消除数据泥团代码坏味道）
#[derive(Debug, Clone, Copy)]
pub struct RgaCropJob {
    pub src_fd: c_int,
    pub src_w: u32,
    pub src_h: u32,
    pub src_hor_stride: u32,
    pub src_ver_stride: u32,
    pub sx: u32,
    pub sy: u32,
    pub crop_w: u32,
    pub crop_h: u32,
    pub dst_fd: c_int,
    pub dst_w: u32,
    pub dst_h: u32,
}

/// 安全包装的 RGA 运行时，单例持有
pub struct RgaRuntime {
    library: NonNull<c_void>,
    ffi: RgaFfi,
    lock: Mutex<()>,
}

impl std::fmt::Debug for RgaRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgaRuntime").finish()
    }
}

impl Drop for RgaRuntime {
    fn drop(&mut self) {
        // SAFETY: library 来自成功的 dlopen；所有导入 handle 均由上层在 runtime
        // 析构前释放，且析构后不会再调用其中的函数指针。
        unsafe {
            libc::dlclose(self.library.as_ptr());
        }
    }
}

impl RgaRuntime {
    /// 尝试加载 librga.so 并解析符号
    pub fn try_load() -> Result<Self, MediaError> {
        let candidates = ["librga.so", "librga.so.2", "librga.so.1", "librga.so.0"];

        let mut last_error = String::new();
        for name in &candidates {
            match Self::try_load_one(name) {
                Ok(rt) => {
                    debug!(lib = name, "librga 动态加载成功");
                    return Ok(rt);
                }
                Err(e) => {
                    last_error = e.to_string();
                }
            }
        }

        Err(MediaError::EncoderInit {
            codec: "RGA".into(),
            reason: format!("librga.so 加载失败: {last_error}"),
        })
    }

    fn try_load_one(name: &str) -> Result<Self, MediaError> {
        let c_name = CString::new(name).map_err(|_| MediaError::EncoderInit {
            codec: "RGA".into(),
            reason: "库名包含 NUL 字节".into(),
        })?;

        // SAFETY: dlopen 调用标准 POSIX API，传递安全构造的 NUL 结尾 CString 字符串
        let handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        let lib = NonNull::new(handle).ok_or_else(|| {
            // SAFETY: dlerror 返回由 libc 管理的错误说明静态指针或 null
            let msg = unsafe {
                let ptr = libc::dlerror();
                if ptr.is_null() {
                    "unknown dlopen error".to_string()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            MediaError::EncoderInit {
                codec: "RGA".into(),
                reason: format!("{name}: {msg}"),
            }
        })?;

        let _library_guard = DlopenGuard(lib);

        // SAFETY: lib 为经由 dlopen 校验有效的动态库句柄，所有函数签名严格对照 librga im2d C ABI
        let (import_buffer, release_buffer, wrap_buffer, check, process, fill, error_string) = unsafe {
            (
                Self::resolve_sym::<ImportBufferFn>(lib.as_ptr(), &[b"importbuffer_fd\0"])?,
                Self::resolve_sym::<ReleaseBufferFn>(lib.as_ptr(), &[b"releasebuffer_handle\0"])?,
                Self::resolve_sym::<WrapBufferFn>(lib.as_ptr(), &[b"wrapbuffer_handle_t\0"])?,
                Self::resolve_sym::<ImCheckFn>(lib.as_ptr(), &[b"imcheck_t\0"])?,
                Self::resolve_sym::<ImProcessFn>(
                    lib.as_ptr(),
                    &[b"improcess\0", b"improcess_t\0"],
                )?,
                Self::resolve_sym::<ImFillFn>(lib.as_ptr(), &[b"imfill_t\0"])?,
                Self::resolve_sym::<ImStrErrorFn>(
                    lib.as_ptr(),
                    &[b"imStrError\0", b"imStrError_t\0"],
                )?,
            )
        };

        Ok(Self {
            library: _library_guard.into_inner(),
            ffi: RgaFfi {
                import_buffer,
                release_buffer,
                wrap_buffer,
                check,
                process,
                fill,
                error_string,
            },
            lock: Mutex::new(()),
        })
    }

    /// 解析动态库符号
    ///
    /// # Safety
    /// 调用方必须确保 handle 为合法的动态库句柄且返回类型 T 严格匹配符号原型
    unsafe fn resolve_sym<T: Copy>(handle: *mut c_void, names: &[&[u8]]) -> Result<T, MediaError> {
        for name in names {
            // SAFETY: dlsym 接收合法的库句柄与 NUL 结尾符号名
            let ptr = unsafe { libc::dlsym(handle, name.as_ptr() as *const c_char) };
            if let Some(non_null) = NonNull::new(ptr) {
                let fn_ptr = non_null.as_ptr();
                // SAFETY: 调用方保证 T 为函数指针，通过 transmute_copy 拷贝 8 字节裸指针值
                return Ok(unsafe { std::mem::transmute_copy(&fn_ptr) });
            }
        }
        Err(MediaError::EncoderInit {
            codec: "RGA".into(),
            reason: format!(
                "符号未找到: {:?}",
                names
                    .iter()
                    .map(|n| CStr::from_bytes_with_nul(n).unwrap_or_default())
                    .collect::<Vec<_>>()
            ),
        })
    }

    // ========================================================================
    // 公开 API
    // ========================================================================

    /// 导入 DMA-BUF fd 为 RGA handle。
    ///
    /// 目标 SDK 的导出 C wrapper 原型是
    /// `importbuffer_fd(int, im_handle_param_t *)`；C++ 的四整数重载不能
    /// 通过未修饰符号名安全调用，因此这里明确只解析上述 C ABI。
    pub fn import_buffer_fd(
        &self,
        fd: c_int,
        width: u32,
        height: u32,
        format: c_int,
    ) -> Result<u32, MediaError> {
        let _lock = self.lock.lock().map_err(|_| MediaError::Encode {
            reason: "RGA 锁中毒".into(),
        })?;
        self.import_buffer_unlocked(fd, width, height, format)
    }

    fn import_buffer_unlocked(
        &self,
        fd: c_int,
        width: u32,
        height: u32,
        format: c_int,
    ) -> Result<u32, MediaError> {
        if fd < 0 || width == 0 || height == 0 {
            return Err(MediaError::Encode {
                reason: format!("非法 RGA import 参数: fd={fd}, {width}x{height}"),
            });
        }
        let mut param = ImHandleParam {
            width,
            height,
            format: format as u32,
        };
        // SAFETY: param 与其字段严格匹配 im_handle_param_t，调用期间保持有效。
        let handle = unsafe { (self.ffi.import_buffer)(fd, &mut param) };
        if handle == 0 {
            return Err(MediaError::Encode {
                reason: format!("RGA importbuffer_fd 失败: fd={fd}, {width}x{height}"),
            });
        }
        Ok(handle)
    }

    /// 释放 RGA handle
    pub fn release_buffer_handle(&self, handle: u32) {
        if handle == 0 {
            return;
        }
        let _lock = match self.lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        // SAFETY: handle 为 import_buffer_fd 返回的有效句柄
        unsafe {
            (self.ffi.release_buffer)(handle);
        }
    }

    /// 构造 RGA 裁剪 + 填充 + blit 并同步执行。
    ///
    /// `dst_handle` 必须由 `job.dst_fd` 预先导入，并在调用方生命周期内保持有效。
    pub fn crop_blit_sync(&self, job: RgaCropJob, dst_handle: u32) -> Result<(), MediaError> {
        if job.src_fd < 0
            || job.dst_fd < 0
            || dst_handle == 0
            || job.src_w == 0
            || job.src_h == 0
            || (job.src_w & 1) != 0
            || (job.src_h & 1) != 0
            || job.src_hor_stride < job.src_w
            || job.src_ver_stride < job.src_h
            || (job.src_hor_stride & 1) != 0
            || (job.src_ver_stride & 1) != 0
            || job.crop_w == 0
            || job.crop_h == 0
            || (job.crop_w & 1) != 0
            || (job.crop_h & 1) != 0
            || job.crop_w < RGA_MIN_DIMENSION
            || job.crop_h < RGA_MIN_DIMENSION
            || match job.sx.checked_add(job.crop_w) {
                Some(v) => v > job.src_w,
                None => true,
            }
            || match job.sy.checked_add(job.crop_h) {
                Some(v) => v > job.src_h,
                None => true,
            }
            || job.dst_w < job.crop_w
            || job.dst_h < job.crop_h
            || (job.dst_w & 1) != 0
            || (job.dst_h & 1) != 0
        {
            return Err(MediaError::Encode {
                reason: format!("非法 RGA crop 布局: {job:?}"),
            });
        }

        // 等待上游设备完成后再进入 RGA 临界区，避免持有锁执行可能阻塞的 poll。
        wait_dmabuf_readable(job.src_fd, 100)?;
        let _lock = self.lock.lock().map_err(|_| MediaError::Encode {
            reason: "RGA 锁中毒".into(),
        })?;

        // 1. 源 DMA-BUF 随帧导入；目标使用输出池初始化时导入的常驻句柄。
        let ffi_ptr: *const RgaFfi = &self.ffi;
        let src_raw = self.import_buffer_unlocked(
            job.src_fd,
            job.src_hor_stride,
            job.src_ver_stride,
            RK_FORMAT_YCbCr_420_SP,
        )?;
        let src_guard = RgaImportGuard::new(ffi_ptr, src_raw);

        // 2. wrapbuffer 为 RgaBuffer 结构（源守卫在操作完成后释放临时 handle）
        // SAFETY: src_guard.handle 为成功导入的 RGA 句柄
        let mut src_buf = unsafe {
            (self.ffi.wrap_buffer)(
                src_guard.handle,
                job.src_w as c_int,
                job.src_h as c_int,
                job.src_hor_stride as c_int,
                job.src_ver_stride as c_int,
                RK_FORMAT_YCbCr_420_SP,
            )
        };
        src_buf.color_space_mode = 0;

        // SAFETY: dst_handle 是调用方为仍存活的 scratchpad DMA-BUF 导入的有效句柄。
        let mut dst_buf = unsafe {
            (self.ffi.wrap_buffer)(
                dst_handle,
                job.crop_w as c_int,
                job.crop_h as c_int,
                job.dst_w as c_int,
                job.dst_h as c_int,
                RK_FORMAT_YCbCr_420_SP,
            )
        };
        dst_buf.color_space_mode = 0;

        // 3. 裁剪矩形
        let src_rect = ImRect {
            x: job.sx as c_int,
            y: job.sy as c_int,
            width: job.crop_w as c_int,
            height: job.crop_h as c_int,
        };
        let dst_rect = ImRect {
            x: 0,
            y: 0,
            width: job.crop_w as c_int,
            height: job.crop_h as c_int,
        };

        // 4. imcheck 校验
        let empty_buf = RgaBuffer::default();
        let empty_rect = ImRect::default();
        // SAFETY: 结构体入参按值复制传递给 librga C ABI 函数
        let check_ret = unsafe {
            (self.ffi.check)(
                src_buf, dst_buf, empty_buf, src_rect, dst_rect, empty_rect, 0,
            )
        };
        if check_ret != IM_STATUS_SUCCESS && check_ret != IM_STATUS_NOERROR {
            let err_text = self.status_text(check_ret);
            return Err(MediaError::Encode {
                reason: format!("RGA imcheck 校验失败: {err_text} (status={check_ret})"),
            });
        }

        // 5. 填充目标全黑（防止未覆盖区域花屏）
        let fill_rect = ImRect {
            x: 0,
            y: 0,
            width: job.crop_w as c_int,
            height: job.crop_h as c_int,
        };
        // SAFETY: dst_buf 已经过 wrap_buffer 初始化且在 guard 保护范围内
        let fill_ret = unsafe { (self.ffi.fill)(dst_buf, fill_rect, 0x000000, IM_SYNC) };
        if fill_ret != IM_STATUS_SUCCESS && fill_ret != IM_STATUS_NOERROR {
            warn!(fill_ret, "RGA imfill 黑色填充失败，继续执行 blit");
        }

        // 6. improcess 同步执行裁剪 blit
        // SAFETY: 所有 buffer 与 rect 参数经由 imcheck 校验合法
        let process_ret = unsafe {
            (self.ffi.process)(
                src_buf, dst_buf, empty_buf, src_rect, dst_rect, empty_rect, IM_SYNC,
            )
        };

        if process_ret != IM_STATUS_SUCCESS && process_ret != IM_STATUS_NOERROR {
            let err_text = self.status_text(process_ret);
            return Err(MediaError::Encode {
                reason: format!("RGA improcess blit 失败: {err_text} (status={process_ret})"),
            });
        }

        Ok(())
    }

    fn status_text(&self, status: c_int) -> String {
        // SAFETY: imStrError 返回 librga 内部静态错误字符串指针
        let ptr = unsafe { (self.ffi.error_string)(status) };
        if ptr.is_null() {
            format!("status={status}")
        } else {
            // SAFETY: ptr 经非空校验，指向 NUL 结尾的静态只读 C 字符串
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn test_rga_buffer_abi_layout() {
        assert_eq!(size_of::<ImHandleParam>(), 12);
        assert_eq!(align_of::<ImHandleParam>(), 4);
        assert_eq!(offset_of!(ImHandleParam, width), 0);
        assert_eq!(offset_of!(ImHandleParam, height), 4);
        assert_eq!(offset_of!(ImHandleParam, format), 8);

        assert_eq!(size_of::<RgaBuffer>(), 96);
        assert_eq!(align_of::<RgaBuffer>(), 8);
        assert_eq!(offset_of!(RgaBuffer, nn), 64);
        assert_eq!(offset_of!(RgaBuffer, rop_code), 88);
        assert_eq!(offset_of!(RgaBuffer, handle), 92);
    }
}
