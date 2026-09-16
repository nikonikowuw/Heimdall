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
/// RGA 硬件规范允许的物理最小输入/输出维度（RGA2 下限 2px，更严格约束由驱动层 imcheck 裁定）
const RGA_MIN_DIMENSION: u32 = 2;

/// librga 像素格式枚举常量（遵循 Rockchip rga.h 标准，值全部向左偏移 8 位以与 Android HAL 区分）
///
/// 详见 Rockchip 官方 rga.h：
/// - RK_FORMAT_RGBA_8888 = 0x0 << 8 (0x0000)
/// - RK_FORMAT_RGBX_8888 = 0x1 << 8 (0x0100)
/// - RK_FORMAT_YCbCr_420_SP = 0xa << 8 (0x0a00, 即 NV12)
#[allow(non_upper_case_globals)]
pub const RK_FORMAT_RGBA_8888: c_int = 0x0 << 8;
#[allow(non_upper_case_globals)]
pub const RK_FORMAT_RGBX_8888: c_int = 0x1 << 8;
#[allow(non_upper_case_globals)]
pub const RK_FORMAT_YCbCr_420_SP: c_int = 0xa << 8;

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

/// RGA 源 DMA-BUF 面：可见尺寸与分配跨步（NV12）
///
/// 裁剪与降采样两条路径共用同一份布局契约：`hor_stride/ver_stride` 是分配跨步，
/// 可能大于可见宽高（硬解常按 16 对齐分配），因此不能假设 `stride == width`。
#[derive(Debug, Clone, Copy)]
pub struct RgaSource {
    pub fd: c_int,
    pub width: u32,
    pub height: u32,
    pub hor_stride: u32,
    pub ver_stride: u32,
}

impl RgaSource {
    /// 校验源面布局：NV12 可见宽高偶数对齐，分配跨步不小于可见尺寸且偶数对齐
    fn validate(&self) -> Result<(), MediaError> {
        if self.fd < 0
            || self.width == 0
            || self.height == 0
            || (self.width & 1) != 0
            || (self.height & 1) != 0
            || self.hor_stride < self.width
            || self.ver_stride < self.height
            || (self.hor_stride & 1) != 0
            || (self.ver_stride & 1) != 0
        {
            return Err(MediaError::Encode {
                reason: format!("非法 RGA 源面布局: {self:?}"),
            });
        }
        Ok(())
    }
}

/// RGA 目标面：常驻导入句柄 + 本次写入的可见区域 + 分配跨步。
///
/// 目标句柄由调用方预先导入并在生命周期内保持有效；定长缓冲（门控缩略图）的可见尺寸
/// 与分配跨步相等，裁剪填充场景的可见宽可小于分配跨步。
#[derive(Debug, Clone, Copy)]
struct RgaTarget {
    handle: u32,
    width: u32,
    height: u32,
    hor_stride: u32,
    ver_stride: u32,
}

/// RGA 裁剪与填充任务参数结构体（收敛多参数，消除数据泥团代码坏味道）
#[derive(Debug, Clone, Copy)]
pub struct RgaCropJob {
    pub src: RgaSource,
    pub sx: u32,
    pub sy: u32,
    pub crop_w: u32,
    pub crop_h: u32,
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

// SAFETY: RgaRuntime 持有的动态库句柄在生命周期内有效，其内部函数调用通过 Mutex 序列化同步
unsafe impl Send for RgaRuntime {}
// SAFETY: 跨线程共享引用安全
unsafe impl Sync for RgaRuntime {}

static GLOBAL_RGA: std::sync::OnceLock<Option<std::sync::Arc<RgaRuntime>>> =
    std::sync::OnceLock::new();

/// 获取进程级全局 RGA 运行时单例。
pub fn get_global_rga() -> Result<std::sync::Arc<RgaRuntime>, MediaError> {
    let opt = GLOBAL_RGA.get_or_init(|| match RgaRuntime::try_load() {
        Ok(rt) => {
            tracing::info!("RGA 硬件加速运行时加载成功");
            Some(std::sync::Arc::new(rt))
        }
        Err(e) => {
            tracing::warn!(error = %e, "未加载到 librga 运行时，RGA 硬件加速不可用");
            None
        }
    });
    opt.as_ref().cloned().ok_or(MediaError::Unsupported(
        "librga 动态加载失败或当前平台未安装 librga.so",
    ))
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
                reason: format!("RGA importbuffer_fd 失败 (返回空句柄): fd={fd}, {width}x{height}"),
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
    /// 目标使用调用方预先导入的常驻句柄，并在调用方生命周期内保持有效。
    pub fn crop_blit_sync(&self, job: RgaCropJob, dst_handle: u32) -> Result<(), MediaError> {
        job.src.validate()?;
        if dst_handle == 0
            || job.crop_w == 0
            || job.crop_h == 0
            || (job.crop_w & 1) != 0
            || (job.crop_h & 1) != 0
            || job.crop_w < RGA_MIN_DIMENSION
            || job.crop_h < RGA_MIN_DIMENSION
            || match job.sx.checked_add(job.crop_w) {
                Some(v) => v > job.src.width,
                None => true,
            }
            || match job.sy.checked_add(job.crop_h) {
                Some(v) => v > job.src.height,
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
        wait_dmabuf_readable(job.src.fd, 100)?;
        let _lock = self.lock.lock().map_err(|_| MediaError::Encode {
            reason: "RGA 锁中毒".into(),
        })?;

        self.blit_unlocked(
            job.src,
            ImRect {
                x: job.sx as c_int,
                y: job.sy as c_int,
                width: job.crop_w as c_int,
                height: job.crop_h as c_int,
            },
            RgaTarget {
                handle: dst_handle,
                width: job.crop_w,
                height: job.crop_h,
                hor_stride: job.dst_w,
                ver_stride: job.dst_h,
            },
            true,
        )
    }

    /// RGA 同步 blit / 缩放的公共核心：导入源面 → wrapbuffer → imcheck →（可选 imfill）→ improcess。
    ///
    /// 调用方必须已持有 [`Self::lock`]，并已完成源 DMA-BUF 的可读栅障等待。
    /// `fill_black` 为 true 时先将目标可见区域填黑，避免裁剪后未覆盖区域花屏；
    /// 整帧降采样会完整覆盖目标，无需填充。
    fn blit_unlocked(
        &self,
        src: RgaSource,
        src_rect: ImRect,
        dst: RgaTarget,
        fill_black: bool,
    ) -> Result<(), MediaError> {
        // 1. 源 DMA-BUF 随帧导入，作用域结束即释放（严禁 wrapbuffer_fd 脏缓存）
        let ffi_ptr: *const RgaFfi = &self.ffi;
        let src_raw = self.import_buffer_unlocked(
            src.fd,
            src.hor_stride,
            src.ver_stride,
            RK_FORMAT_YCbCr_420_SP,
        )?;
        let src_guard = RgaImportGuard::new(ffi_ptr, src_raw);

        // 2. wrapbuffer 为 RgaBuffer 结构（源守卫在操作完成后释放临时 handle）
        // SAFETY: src_guard.handle 为成功导入的 RGA 句柄
        let mut src_buf = unsafe {
            (self.ffi.wrap_buffer)(
                src_guard.handle,
                src.width as c_int,
                src.height as c_int,
                src.hor_stride as c_int,
                src.ver_stride as c_int,
                RK_FORMAT_YCbCr_420_SP,
            )
        };
        src_buf.color_space_mode = 0;

        // SAFETY: dst.handle 是调用方为仍存活的 DMA-BUF 导入的有效句柄。
        let mut dst_buf = unsafe {
            (self.ffi.wrap_buffer)(
                dst.handle,
                dst.width as c_int,
                dst.height as c_int,
                dst.hor_stride as c_int,
                dst.ver_stride as c_int,
                RK_FORMAT_YCbCr_420_SP,
            )
        };
        dst_buf.color_space_mode = 0;

        // 3. 目标矩形固定为整块可见区域
        let dst_rect = ImRect {
            x: 0,
            y: 0,
            width: dst.width as c_int,
            height: dst.height as c_int,
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

        // 5. 填充目标全黑（防止裁剪后未覆盖区域花屏）
        if fill_black {
            // SAFETY: dst_buf 已经过 wrap_buffer 初始化且在 guard 保护范围内
            let fill_ret = unsafe { (self.ffi.fill)(dst_buf, dst_rect, 0x000000, IM_SYNC) };
            if fill_ret != IM_STATUS_SUCCESS && fill_ret != IM_STATUS_NOERROR {
                warn!(fill_ret, "RGA imfill 黑色填充失败，继续执行 blit");
            }
        }

        // 6. improcess 同步执行裁剪 blit / 缩放
        // SAFETY: 所有 buffer 与 rect 参数经由 imcheck 校验合法
        let process_ret = unsafe {
            (self.ffi.process)(
                src_buf, dst_buf, empty_buf, src_rect, dst_rect, empty_rect, IM_SYNC,
            )
        };

        if process_ret != IM_STATUS_SUCCESS && process_ret != IM_STATUS_NOERROR {
            let err_text = self.status_text(process_ret);
            return Err(MediaError::Encode {
                reason: format!("RGA improcess 失败: {err_text} (status={process_ret})"),
            });
        }

        Ok(())
    }

    /// 构造 RGA 整帧降采样并同步执行（NV12 整帧 → 定长 NV12 缩略图）。
    ///
    /// 与 [`Self::crop_blit_sync`] 的差异：
    /// - 源矩形为整帧可见区域、目标矩形为整张缩略图，缩放由 RGA 硬件完成（`improcess` 按 rect 自动缩放）；
    /// - 目标 stride 恒等于可见宽度，整图被完整覆盖，故不做黑色填充；
    /// - 仅允许缩小：放大既无收益，又凭空拉高 RGA 带宽并破坏门控灵敏度口径。
    ///
    /// 目标句柄由调用方预先导入并常驻（解析为门控缩略图见 `crate::motion_thumb`）。
    pub fn scale_sync(
        &self,
        src: RgaSource,
        dst_handle: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), MediaError> {
        src.validate()?;
        if dst_handle == 0
            || dst_w == 0
            || dst_h == 0
            || (dst_h & 1) != 0
            || dst_w < RGA_MIN_DIMENSION
            || dst_h < RGA_MIN_DIMENSION
            // 目标 stride == 可见宽度，必须满足 RGA2 的 4 像素步长对齐；更严的 RGA3 约束交由 imcheck 裁定
            || !dst_w.is_multiple_of(4)
            || dst_w > src.width
            || dst_h > src.height
        {
            return Err(MediaError::Encode {
                reason: format!("非法 RGA scale 布局: {src:?} → {dst_w}x{dst_h}"),
            });
        }

        // 等待上游硬解完成写栅障后再进入 RGA 临界区，避免持锁执行可能阻塞的 poll。
        wait_dmabuf_readable(src.fd, 100)?;
        let _lock = self.lock.lock().map_err(|_| MediaError::Encode {
            reason: "RGA 锁中毒".into(),
        })?;

        self.blit_unlocked(
            src,
            ImRect {
                x: 0,
                y: 0,
                width: src.width as c_int,
                height: src.height as c_int,
            },
            RgaTarget {
                handle: dst_handle,
                width: dst_w,
                height: dst_h,
                hor_stride: dst_w,
                ver_stride: dst_h,
            },
            false,
        )
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

/// 基于 Rockchip RGA 2D 硬件加速器执行零拷贝取景预裁剪。
///
/// 严格保证：
/// - 纯设备侧硬件零拷贝（DRM DMA-BUF src_fd → RGA 硬件 Blit → DRM DMA-BUF dst_fd）；
/// - 全程零 CPU 像素遍历；
/// - 输出 Stride 严格满足 16 字节硬件对齐约束。
pub fn crop_dmabuf_rga(
    src_fd: std::os::fd::RawFd,
    frame: &types::FrameRef,
    sx: u32,
    sy: u32,
    crop_w: u32,
    crop_h: u32,
    w_stride: u32,
) -> Result<types::FrameRef, MediaError> {
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::sync::Arc;
    use types::{FrameHandle, PixelFormat, StrideInfo};

    if frame.format != PixelFormat::Nv12 {
        return Err(MediaError::Unsupported(
            "RGA 硬件取景预裁剪当前仅支持 NV12 格式帧",
        ));
    }

    let rga = get_global_rga()?;

    // 1. 严格校验 RGA 硬件输入输出步长对齐要求
    crate::rga::RgaPolicyChecker::validate_nv12_strides(
        crate::rga::RgaCore::Rga2,
        crop_w,
        crop_h,
        w_stride,
        crop_h,
    )?;

    // 2. 为裁剪后的目标帧在内核 DMA 堆中分配缓冲区
    let dst_size = (w_stride as usize * crop_h as usize * 3) / 2;
    let dst_fd: OwnedFd = crate::dmabuf_sync::alloc_dma_buf(dst_size)?;

    // 3. 将目标 DMA-BUF 导入为 RGA 句柄
    let dst_handle =
        rga.import_buffer_fd(dst_fd.as_raw_fd(), w_stride, crop_h, RK_FORMAT_YCbCr_420_SP)?;

    // 4. 构造 RGA 裁剪 Job 并同步执行硬件 Blit
    let src_hor_stride = (frame.stride.hor_stride.max(frame.width) + 1) & !1;
    let src_ver_stride = (frame.stride.ver_stride.max(frame.height) + 1) & !1;

    let job = RgaCropJob {
        src: RgaSource {
            fd: src_fd,
            width: frame.width,
            height: frame.height,
            hor_stride: src_hor_stride,
            ver_stride: src_ver_stride,
        },
        sx,
        sy,
        crop_w,
        crop_h,
        dst_w: w_stride,
        dst_h: crop_h,
    };

    let blit_res = rga.crop_blit_sync(job, dst_handle);
    rga.release_buffer_handle(dst_handle);
    blit_res?;

    Ok(types::FrameRef::new(
        frame.camera_id.clone(),
        frame.timestamp,
        crop_w,
        crop_h,
        StrideInfo::new(w_stride, crop_h),
        PixelFormat::Nv12,
        FrameHandle::DmaBuf {
            fd: Arc::new(dst_fd),
            _lease: None,
        },
    ))
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

    #[test]
    fn test_rga_format_constants() {
        // Rockchip rga.h 核心格式常量严格检验，严防将 RGBX_8888 (0x100) 误当作 NV12 (0x0a00)
        assert_eq!(RK_FORMAT_RGBA_8888, 0x0);
        assert_eq!(RK_FORMAT_RGBX_8888, 0x100);
        assert_eq!(RK_FORMAT_YCbCr_420_SP, 0x0a00);
        assert_eq!(RK_FORMAT_YCbCr_420_SP, 2560);
        assert_ne!(RK_FORMAT_YCbCr_420_SP, RK_FORMAT_RGBX_8888);
    }
}
