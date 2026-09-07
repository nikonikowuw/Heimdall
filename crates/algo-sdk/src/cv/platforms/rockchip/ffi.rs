use std::ffi::{CStr, CString, OsStr};
use std::mem::size_of;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use crate::error::AlgoError;

const IM_STATUS_SUCCESS: c_int = 1;
const IM_STATUS_NOERROR: c_int = 2;
const IM_SYNC: c_int = 1 << 19;
const IM_RGB_FULL_RANGE: c_int = 1 << 8;
const DMA_HEAP_IOCTL_ALLOC: libc::c_ulong = 0xc018_4800;

#[repr(C)]
struct DmaHeapAllocationData {
    len: u64,
    fd: c_int,
    fd_flags: u32,
    heap_flags: u64,
}

/// RGA's C `im_rect` layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImRect {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ColorKeyRange {
    max: c_int,
    min: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImNn {
    scale_r: c_int,
    scale_g: c_int,
    scale_b: c_int,
    offset_r: c_int,
    offset_g: c_int,
    offset_b: c_int,
}

/// The `rga_buffer_t` ABI from librga's `im2d_type.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct RgaBuffer {
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
    pub colorkey_range: ColorKeyRange,
    pub nn: ImNn,
    pub rop_code: c_int,
    pub handle: u32,
}

impl Default for RgaBuffer {
    fn default() -> Self {
        // SAFETY: RgaBuffer is a C POD. Zero is the documented empty buffer state
        // used by librga's wrapper macros when a parameter is invalid.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ImHandleParam {
    width: u32,
    height: u32,
    format: u32,
}

type ImportFdBySize = unsafe extern "C" fn(fd: c_int, size: c_int) -> u32;
type ImportFdByImage =
    unsafe extern "C" fn(fd: c_int, width: c_int, height: c_int, format: c_int) -> u32;
type ImportFdByParam = unsafe extern "C" fn(fd: c_int, param: *mut ImHandleParam) -> u32;
type ReleaseBuffer = unsafe extern "C" fn(handle: u32) -> c_int;
type WrapBuffer = unsafe extern "C" fn(
    handle: u32,
    width: c_int,
    height: c_int,
    wstride: c_int,
    hstride: c_int,
    format: c_int,
) -> RgaBuffer;
type ImCheck = unsafe extern "C" fn(
    src: RgaBuffer,
    dst: RgaBuffer,
    pat: RgaBuffer,
    src_rect: ImRect,
    dst_rect: ImRect,
    pat_rect: ImRect,
    mode_usage: c_int,
) -> c_int;
type ImFill =
    unsafe extern "C" fn(dst: RgaBuffer, rect: ImRect, color: c_int, sync: c_int) -> c_int;
type ImProcess = unsafe extern "C" fn(
    src: RgaBuffer,
    dst: RgaBuffer,
    pat: RgaBuffer,
    src_rect: ImRect,
    dst_rect: ImRect,
    pat_rect: ImRect,
    usage: c_int,
) -> c_int;
type ErrorString = unsafe extern "C" fn(status: c_int) -> *const c_char;

enum ImportBufferFn {
    Size(ImportFdBySize),
    Image(ImportFdByImage),
    Param(ImportFdByParam),
}

struct RgaApi {
    library: NonNull<c_void>,
    import_buffer: ImportBufferFn,
    release_buffer: ReleaseBuffer,
    wrap_buffer: WrapBuffer,
    check: ImCheck,
    fill: ImFill,
    process: ImProcess,
    error_string: ErrorString,
}

// SAFETY: Function pointers are immutable after symbol resolution. The runtime serializes
// calls into librga and the dlopen handle remains alive as long as this API value exists.
unsafe impl Send for RgaApi {}
// SAFETY: See the Send justification. RgaRuntime guards all calls with its mutex.
unsafe impl Sync for RgaApi {}

impl RgaApi {
    fn load() -> Result<Self, AlgoError> {
        let mut last_error = String::from("librga.so was not found");
        for candidate in library_candidates() {
            match Self::load_library(&candidate) {
                Ok(api) => return Ok(api),
                Err(error) => last_error = error,
            }
        }
        Err(AlgoError::Internal {
            reason: format!("RGA runtime unavailable: {last_error}"),
        })
    }

    fn load_library(path: &OsStr) -> Result<Self, String> {
        let path_bytes = path.as_bytes();
        let path_c = CString::new(path_bytes).map_err(|_| "invalid librga path".to_string())?;
        // SAFETY: CString is NUL terminated and flags are valid for POSIX dlopen.
        let handle = unsafe { libc::dlopen(path_c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        let library = NonNull::new(handle).ok_or_else(last_dl_error)?;

        let loaded: Result<Self, String> = (|| {
            let import_buffer = resolve_import(library.as_ptr())
                .ok_or_else(|| "missing importbuffer_fd symbol".to_string())?;
            Ok(Self {
                library,
                import_buffer,
                release_buffer: resolve_required(library.as_ptr(), &[b"releasebuffer_handle\0"])?,
                wrap_buffer: resolve_required(library.as_ptr(), &[b"wrapbuffer_handle_t\0"])?,
                check: resolve_required(library.as_ptr(), &[b"imcheck_t\0"])?,
                fill: resolve_required(library.as_ptr(), &[b"imfill_t\0"])?,
                process: resolve_required(library.as_ptr(), &[b"improcess\0"])?,
                error_string: resolve_required(
                    library.as_ptr(),
                    &[b"imStrError\0", b"imStrError_t\0"],
                )?,
            })
        })();

        match loaded {
            Ok(api) => Ok(api),
            Err(error) => {
                // SAFETY: library was returned by dlopen and has not been closed elsewhere.
                unsafe { libc::dlclose(library.as_ptr()) };
                Err(format!("{path:?}: {error}"))
            }
        }
    }

    fn import_buffer(
        &self,
        fd: c_int,
        size: c_int,
        width: c_int,
        height: c_int,
        format: c_int,
    ) -> u32 {
        match self.import_buffer {
            ImportBufferFn::Size(function) => {
                // SAFETY: The function pointer was resolved from librga and arguments are checked.
                unsafe { function(fd, size) }
            }
            ImportBufferFn::Image(function) => {
                // SAFETY: The function pointer was resolved from librga and arguments are checked.
                unsafe { function(fd, width, height, format) }
            }
            ImportBufferFn::Param(function) => {
                let mut param = ImHandleParam {
                    width: u32::try_from(width).unwrap_or_default(),
                    height: u32::try_from(height).unwrap_or_default(),
                    format: u32::try_from(format).unwrap_or_default(),
                };
                // SAFETY: param is a valid writable C struct for the duration of the call.
                unsafe { function(fd, &mut param) }
            }
        }
    }
}

impl Drop for RgaApi {
    fn drop(&mut self) {
        // SAFETY: library is the live handle acquired by dlopen for this API object.
        unsafe { libc::dlclose(self.library.as_ptr()) };
    }
}

/// Parameters for one `wrapbuffer_handle_t` call.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RgaWrapParams {
    pub handle: u32,
    pub width: u32,
    pub height: u32,
    pub wstride: u32,
    pub hstride: u32,
    pub format: u32,
    pub color_space_mode: c_int,
}

/// Serialized access to a dynamically loaded librga instance.
pub(crate) struct RgaRuntime {
    api: RgaApi,
    call_lock: Mutex<()>,
}

impl RgaRuntime {
    pub(crate) fn load() -> Result<Arc<Self>, AlgoError> {
        Ok(Arc::new(Self {
            api: RgaApi::load()?,
            call_lock: Mutex::new(()),
        }))
    }

    pub(crate) fn import_buffer_fd(
        &self,
        fd: c_int,
        size: usize,
        width: u32,
        height: u32,
        format: u32,
    ) -> Result<u32, AlgoError> {
        let size = c_int::try_from(size).map_err(|_| AlgoError::OutOfMemory)?;
        let width = c_int::try_from(width).map_err(|_| AlgoError::Preprocess {
            reason: "RGA width exceeds C ABI range".to_string(),
        })?;
        let height = c_int::try_from(height).map_err(|_| AlgoError::Preprocess {
            reason: "RGA height exceeds C ABI range".to_string(),
        })?;
        let format = c_int::try_from(format).map_err(|_| AlgoError::Preprocess {
            reason: "RGA format exceeds C ABI range".to_string(),
        })?;
        let _lock = self.call_lock.lock().map_err(|_| AlgoError::Internal {
            reason: "RGA call lock poisoned".to_string(),
        })?;
        let handle = self.api.import_buffer(fd, size, width, height, format);
        if handle == 0 {
            return Err(AlgoError::Preprocess {
                reason: "librga importbuffer_fd returned a null handle".to_string(),
            });
        }
        Ok(handle)
    }

    pub(crate) fn release_buffer_handle(&self, handle: u32) -> Result<(), AlgoError> {
        if handle == 0 {
            return Ok(());
        }
        let _lock = self.call_lock.lock().map_err(|_| AlgoError::Internal {
            reason: "RGA call lock poisoned".to_string(),
        })?;
        // SAFETY: The handle was returned by the matching librga import function and is released once.
        let status = unsafe { (self.api.release_buffer)(handle) };
        if is_success(status) {
            Ok(())
        } else {
            Err(self.status_error("releasebuffer_handle", status))
        }
    }

    pub(crate) fn source_guard(self: &Arc<Self>, handle: u32) -> RgaHandleGuard {
        RgaHandleGuard {
            runtime: Arc::clone(self),
            handle,
        }
    }

    pub(crate) fn wrap_buffer(&self, params: RgaWrapParams) -> Result<RgaBuffer, AlgoError> {
        let width = c_int::try_from(params.width).map_err(|_| AlgoError::Preprocess {
            reason: "RGA width exceeds C ABI range".to_string(),
        })?;
        let height = c_int::try_from(params.height).map_err(|_| AlgoError::Preprocess {
            reason: "RGA height exceeds C ABI range".to_string(),
        })?;
        let wstride = c_int::try_from(params.wstride).map_err(|_| AlgoError::Preprocess {
            reason: "RGA width stride exceeds C ABI range".to_string(),
        })?;
        let hstride = c_int::try_from(params.hstride).map_err(|_| AlgoError::Preprocess {
            reason: "RGA height stride exceeds C ABI range".to_string(),
        })?;
        let format = c_int::try_from(params.format).map_err(|_| AlgoError::Preprocess {
            reason: "RGA format exceeds C ABI range".to_string(),
        })?;
        let _lock = self.call_lock.lock().map_err(|_| AlgoError::Internal {
            reason: "RGA call lock poisoned".to_string(),
        })?;
        // SAFETY: The function pointer is resolved from librga and all integer fields are range checked.
        let mut buffer = unsafe {
            (self.api.wrap_buffer)(params.handle, width, height, wstride, hstride, format)
        };
        buffer.color_space_mode = params.color_space_mode;
        Ok(buffer)
    }

    pub(crate) fn execute(
        &self,
        src: RgaBuffer,
        dst: RgaBuffer,
        src_rect: ImRect,
        dst_rect: ImRect,
        fill_rect: Option<(ImRect, c_int)>,
    ) -> Result<(), AlgoError> {
        let _lock = self.call_lock.lock().map_err(|_| AlgoError::Internal {
            reason: "RGA call lock poisoned".to_string(),
        })?;
        let empty_buffer = RgaBuffer::default();
        let empty_rect = ImRect::default();
        // SAFETY: All buffers and rectangles were built from validated DMA-BUF metadata.
        let check_status =
            unsafe { (self.api.check)(src, dst, empty_buffer, src_rect, dst_rect, empty_rect, 0) };
        if !is_success(check_status) {
            return Err(self.status_error("imcheck_t", check_status));
        }
        if let Some((rect, color)) = fill_rect {
            // SAFETY: dst is a live imported handle and rect is inside the validated destination.
            let fill_status = unsafe { (self.api.fill)(dst, rect, color, 1) };
            if !is_success(fill_status) {
                return Err(self.status_error("imfill_t", fill_status));
            }
        }
        // SAFETY: IM_SYNC makes the call complete before source handles or leases are released.
        let process_status = unsafe {
            (self.api.process)(
                src,
                dst,
                empty_buffer,
                src_rect,
                dst_rect,
                empty_rect,
                IM_SYNC,
            )
        };
        if !is_success(process_status) {
            return Err(self.status_error("improcess", process_status));
        }
        Ok(())
    }

    fn status_error(&self, operation: &str, status: c_int) -> AlgoError {
        let text = {
            // SAFETY: librga returns a static NUL-terminated error string for a status code.
            let pointer = unsafe { (self.api.error_string)(status) };
            if pointer.is_null() {
                format!("status {status}")
            } else {
                // SAFETY: pointer was checked for null and is owned by librga for this call.
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        AlgoError::Preprocess {
            reason: format!("{operation} failed: {text} ({status})"),
        }
    }
}

pub(crate) fn allocate_dma_buf(heap_fd: c_int, size: usize) -> std::io::Result<OwnedFd> {
    let page_size = {
        // SAFETY: sysconf only reads the process-independent page-size setting.
        let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(value)
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(4096)
    };
    let alloc_len = size
        .max(1)
        .checked_add(page_size - 1)
        .map(|length| length / page_size * page_size)
        .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EOVERFLOW))?;
    let mut allocation = DmaHeapAllocationData {
        len: u64::try_from(alloc_len)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EOVERFLOW))?,
        fd: 0,
        fd_flags: (libc::O_RDWR | libc::O_CLOEXEC) as u32,
        heap_flags: 0,
    };
    // SAFETY: allocation is a writable repr(C) dma_heap_allocation_data with the kernel's
    // documented 24-byte layout, and heap_fd is an open dma-heap descriptor.
    let status = unsafe {
        libc::ioctl(
            heap_fd,
            DMA_HEAP_IOCTL_ALLOC,
            &mut allocation as *mut DmaHeapAllocationData,
        )
    };
    if status < 0 || allocation.fd < 0 {
        let error = std::io::Error::last_os_error();
        if allocation.fd >= 0 {
            // SAFETY: the kernel returned an fd that must be closed on the failed allocation path.
            drop(unsafe { OwnedFd::from_raw_fd(allocation.fd) });
        }
        return Err(error);
    }
    // SAFETY: ioctl transferred ownership of this newly-created dma-buf fd to userspace.
    Ok(unsafe { OwnedFd::from_raw_fd(allocation.fd) })
}

pub(crate) struct RgaHandleGuard {
    runtime: Arc<RgaRuntime>,
    handle: u32,
}

impl Drop for RgaHandleGuard {
    fn drop(&mut self) {
        if let Err(error) = self.runtime.release_buffer_handle(self.handle) {
            tracing::warn!(handle = self.handle, %error, "failed to release source RGA handle");
        }
    }
}

fn library_candidates() -> Vec<std::ffi::OsString> {
    if let Some(path) = std::env::var_os("ARGUS_RGA_LIBRARY") {
        return vec![path];
    }
    vec![
        std::ffi::OsString::from("librga.so"),
        std::ffi::OsString::from("librga.so.2"),
        std::ffi::OsString::from("librga.so.1"),
    ]
}

fn resolve_import(handle: *mut c_void) -> Option<ImportBufferFn> {
    resolve_symbol::<ImportFdBySize>(handle, &[b"_Z15importbuffer_fdii\0"])
        .map(ImportBufferFn::Size)
        .or_else(|| {
            resolve_symbol::<ImportFdByImage>(handle, &[b"_Z15importbuffer_fdiiii\0"])
                .map(ImportBufferFn::Image)
        })
        .or_else(|| {
            resolve_symbol::<ImportFdByParam>(handle, &[b"importbuffer_fd\0"])
                .map(ImportBufferFn::Param)
        })
}

fn resolve_required<T: Copy>(handle: *mut c_void, names: &[&[u8]]) -> Result<T, String> {
    resolve_symbol(handle, names).ok_or_else(|| {
        let names = names
            .iter()
            .map(|name| {
                String::from_utf8_lossy(name)
                    .trim_end_matches('\0')
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("missing symbol: {names}")
    })
}

fn resolve_symbol<T: Copy>(handle: *mut c_void, names: &[&[u8]]) -> Option<T> {
    for name in names {
        // SAFETY: dlsym accepts a live dlopen handle and a NUL-terminated symbol name.
        let symbol = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
        if !symbol.is_null() {
            if size_of::<T>() != size_of::<*mut c_void>() {
                return None;
            }
            // SAFETY: POSIX dlsym returns a function address; the caller chooses T matching
            // the exact symbol signature declared by librga.
            return Some(unsafe { std::mem::transmute_copy(&symbol) });
        }
    }
    None
}

fn last_dl_error() -> String {
    // SAFETY: dlerror returns either null or a process-owned NUL-terminated diagnostic.
    let pointer = unsafe { libc::dlerror() };
    if pointer.is_null() {
        "dlopen failed without a diagnostic".to_string()
    } else {
        // SAFETY: pointer was returned by dlerror and remains valid until the next dl call.
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

fn is_success(status: c_int) -> bool {
    matches!(status, IM_STATUS_SUCCESS | IM_STATUS_NOERROR)
}

pub(crate) fn rgb_fill_color([red, green, blue]: [u8; 3]) -> c_int {
    // im_color_t is { red, green, blue, alpha }; RGA's legacy fill parameter
    // uses the same little-endian byte ordering on supported Rockchip targets.
    (u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)) as c_int
}

pub(crate) fn rgb_color_space_mode() -> c_int {
    IM_RGB_FULL_RANGE
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, offset_of};

    use super::*;

    #[test]
    fn rga_buffer_abi_matches_upstream_layout() {
        assert_eq!(size_of::<ImHandleParam>(), 12);
        assert_eq!(align_of::<ImHandleParam>(), 4);
        assert_eq!(size_of::<DmaHeapAllocationData>(), 24);
        assert_eq!(align_of::<DmaHeapAllocationData>(), 8);
        assert_eq!(offset_of!(DmaHeapAllocationData, fd), 8);
        assert_eq!(size_of::<ImRect>(), 16);
        assert_eq!(align_of::<ImRect>(), 4);
        assert_eq!(size_of::<RgaBuffer>(), 96);
        assert_eq!(align_of::<RgaBuffer>(), 8);
        assert_eq!(offset_of!(RgaBuffer, vir_addr), 0);
        assert_eq!(offset_of!(RgaBuffer, fd), 16);
        assert_eq!(offset_of!(RgaBuffer, wstride), 28);
        assert_eq!(offset_of!(RgaBuffer, format), 36);
        assert_eq!(offset_of!(RgaBuffer, handle), 92);
    }

    #[test]
    fn fill_color_uses_rgb_byte_order() {
        assert_eq!(rgb_fill_color([0x11, 0x22, 0x33]), 0x0033_2211);
    }
}
