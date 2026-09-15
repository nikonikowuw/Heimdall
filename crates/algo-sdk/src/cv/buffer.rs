//! 抽象显存句柄与 RAII 容器 (CvBuffer)

use std::any::Any;
use std::ffi::c_void;
use std::slice;

use super::types::PixelFormat;
use crate::c_abi::*;
use crate::error::AlgoError;

/// 底层异构硬件显存句柄 — `pub(crate)` 密封在 SDK 内部，不逃逸到算法业务代码
pub(crate) enum CvBufferKind {
    /// Linux DRM DMA-BUF 文件描述符。
    /// `close_fd` 为 true 时由 SDK 关闭 fd；带 guard 时通常由 guard 管理 fd 所有权。
    DmaBuf {
        fd: i32,
        close_fd: bool,
        size: usize,
        stride: [u32; 4],
        h_stride: u32,
        _guard: Option<Box<dyn Any + Send>>,
    },
    /// Apple CoreVideo CVPixelBufferRef
    ApplePixelBuffer {
        ptr: *mut c_void,
        release: Option<unsafe extern "C" fn(*mut c_void)>,
    },
    /// 华为昇腾 DVPP 原生设备显存指针
    AscendDeviceMemory {
        ptr: *mut c_void,
        release: Option<unsafe extern "C" fn(*mut c_void)>,
    },
    /// Host 主机内存字节数组
    Host(Vec<u8>),
    /// 宿主 AvImageOps 虚表分配的图像视图，析构时自动调用宿主 free 回调
    HostOps {
        view: AvImageView,
        ops_ctx: *mut c_void,
        free_fn: unsafe extern "C" fn(*mut c_void, *mut AvImageView) -> std::ffi::c_int,
    },
}

// SAFETY: CvBufferKind 只在线程间转移所有权；底层句柄的跨线程约束由对应宿主/平台 lease 保证。
unsafe impl Send for CvBufferKind {}

/// DMA-BUF 的物理图像布局，供下游硬件绑定时校验容量和 stride。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaBufLayout {
    pub fd: i32,
    pub size: usize,
    /// packed RGB 的 stride 以字节表示；多平面格式按 plane 保存字节 stride。
    pub stride: [u32; 4],
    pub h_stride: u32,
}

/// 抽象显存图像容器，通过 RAII 自动管理底层异构硬件显存资源
pub struct CvBuffer {
    inner: CvBufferKind,
    width: u32,
    height: u32,
    format: PixelFormat,
}

// SAFETY: CvBuffer 持有底层句柄独占所有权，可在线程间安全转移。
unsafe impl Send for CvBuffer {}

impl CvBuffer {
    /// 从 Host 内存构造
    pub fn from_host(data: Vec<u8>, width: u32, height: u32, format: PixelFormat) -> Self {
        Self {
            inner: CvBufferKind::Host(data),
            width,
            height,
            format,
        }
    }

    /// 从 DMA-BUF 文件描述符构造。
    ///
    /// `guard = None` 时 SDK 接管并关闭 `fd`；`guard = Some(_)` 时调用方的 guard
    /// 负责 fd 所有权（例如携带 `OwnedFd` 或平台 buffer lease）。
    pub fn from_dma_buf(
        fd: i32,
        width: u32,
        height: u32,
        format: PixelFormat,
        guard: Option<Box<dyn Any + Send>>,
    ) -> Self {
        let (stride, size) = match format {
            PixelFormat::Nv12 => {
                let stride = width;
                let size = usize::try_from(stride)
                    .ok()
                    .and_then(|row| row.checked_mul(height as usize))
                    .and_then(|luma| luma.checked_add(luma / 2))
                    .unwrap_or(0);
                ([stride, stride, 0, 0], size)
            }
            PixelFormat::I420 => {
                let y_stride = width;
                let chroma_stride = width.div_ceil(2);
                let y_size = usize::try_from(y_stride)
                    .ok()
                    .and_then(|row| row.checked_mul(height as usize))
                    .unwrap_or(0);
                let chroma_size = usize::try_from(chroma_stride)
                    .ok()
                    .and_then(|row| row.checked_mul(usize::try_from(height.div_ceil(2)).ok()?))
                    .unwrap_or(0);
                let size = y_size
                    .checked_add(chroma_size)
                    .and_then(|total| total.checked_add(chroma_size))
                    .unwrap_or(0);
                ([y_stride, chroma_stride, chroma_stride, 0], size)
            }
            PixelFormat::Bgra => {
                let stride = width.saturating_mul(4);
                let size = usize::try_from(stride)
                    .ok()
                    .and_then(|row| row.checked_mul(height as usize))
                    .unwrap_or(0);
                ([stride, 0, 0, 0], size)
            }
            PixelFormat::Rgb24 => {
                let stride = width.saturating_mul(3);
                let size = usize::try_from(stride)
                    .ok()
                    .and_then(|row| row.checked_mul(height as usize))
                    .unwrap_or(0);
                ([stride, 0, 0, 0], size)
            }
            PixelFormat::Unknown(_) => {
                let stride = width;
                let size = usize::try_from(stride)
                    .ok()
                    .and_then(|row| row.checked_mul(height as usize))
                    .unwrap_or(0);
                ([stride, 0, 0, 0], size)
            }
        };
        Self::from_dma_buf_with_layout(fd, width, height, format, size, stride, height, guard)
    }

    /// 从 DMA-BUF 构造带有真实物理布局的图像容器。
    #[allow(clippy::too_many_arguments)]
    pub fn from_dma_buf_with_layout(
        fd: i32,
        width: u32,
        height: u32,
        format: PixelFormat,
        size: usize,
        stride: [u32; 4],
        h_stride: u32,
        guard: Option<Box<dyn Any + Send>>,
    ) -> Self {
        let close_fd = guard.is_none();
        Self {
            inner: CvBufferKind::DmaBuf {
                fd,
                close_fd,
                size,
                stride,
                h_stride,
                _guard: guard,
            },
            width,
            height,
            format,
        }
    }

    /// 从 Apple CVPixelBuffer 指针构造。
    /// `release = Some(_)` 时由 SDK 在 Drop 中释放；为 None 表示借用句柄。
    pub fn from_cvpixelbuffer(
        ptr: *mut c_void,
        width: u32,
        height: u32,
        format: PixelFormat,
        release: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Self {
        Self {
            inner: CvBufferKind::ApplePixelBuffer { ptr, release },
            width,
            height,
            format,
        }
    }

    /// 从华为昇腾设备显存指针构造借用 buffer。
    pub fn from_ascend_device_memory(
        ptr: *mut c_void,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Self {
        Self::from_ascend_device_memory_with_release(ptr, width, height, format, None)
    }

    /// 从华为昇腾设备显存指针构造拥有释放回调的 buffer。
    pub fn from_ascend_device_memory_with_release(
        ptr: *mut c_void,
        width: u32,
        height: u32,
        format: PixelFormat,
        release: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Self {
        Self {
            inner: CvBufferKind::AscendDeviceMemory { ptr, release },
            width,
            height,
            format,
        }
    }

    /// 从宿主 AvImageOps 分配的视图构造
    pub(crate) fn from_host_ops(
        view: AvImageView,
        ops_ctx: *mut c_void,
        free_fn: unsafe extern "C" fn(*mut c_void, *mut AvImageView) -> std::ffi::c_int,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            inner: CvBufferKind::HostOps {
                view,
                ops_ctx,
                free_fn,
            },
            width,
            height,
            format,
        }
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// 获取 DMA-BUF 的 fd、容量和物理 stride。
    pub fn as_dma_buf_layout(&self) -> Option<DmaBufLayout> {
        match &self.inner {
            CvBufferKind::DmaBuf {
                fd,
                size,
                stride,
                h_stride,
                ..
            } => Some(DmaBufLayout {
                fd: *fd,
                size: *size,
                stride: *stride,
                h_stride: *h_stride,
            }),
            CvBufferKind::HostOps { view, .. }
                if view.opaque_kind == AV_OPAQUE_DMABUF && !view.opaque.is_null() =>
            {
                let fd = (view.opaque as usize).try_into().ok()?;
                let stride = view.stride.map(|value| u32::try_from(value).unwrap_or(0));
                let stride0 = usize::try_from(stride[0]).ok()?;
                let h_stride = view.height;
                let size = stride0.checked_mul(h_stride as usize)?;
                Some(DmaBufLayout {
                    fd,
                    size,
                    stride,
                    h_stride,
                })
            }
            _ => None,
        }
    }

    /// 获取底层 DMA-BUF 文件描述符（供兼容调用方使用）。
    pub fn as_dma_buf_fd(&self) -> Option<i32> {
        self.as_dma_buf_layout().map(|layout| layout.fd)
    }

    /// 获取宿主分配的完整图像视图，供需要 stride/offset 的平台推理会话使用。
    pub fn as_image_view(&self) -> Option<&AvImageView> {
        match &self.inner {
            CvBufferKind::HostOps { view, .. } => Some(view),
            _ => None,
        }
    }

    /// 获取底层 Host 内存切片（供软解或 CPU 推理使用）
    pub fn as_host_bytes(&self) -> Option<&[u8]> {
        match &self.inner {
            CvBufferKind::Host(vec) => Some(vec.as_slice()),
            CvBufferKind::HostOps { view, .. }
                if view.memory_type == AV_MEM_HOST && !view.data.is_null() =>
            {
                let offset = usize::try_from(view.offset[0]).ok()?;
                let stride = usize::try_from(view.stride[0]).ok()?;
                let len = stride.checked_mul(view.height as usize)?;
                let end = offset.checked_add(len)?;
                if end > isize::MAX as usize {
                    return None;
                }
                // SAFETY: HostOps alloc/validate 合约保证 data 基址覆盖 offset + stride * height，
                // 且借用生命周期受 CvBuffer 保持；调用方不能在此借用期间释放 view。
                let data = unsafe { (view.data as *const u8).add(offset) };
                // SAFETY: data 指向 view.data + offset[0]，其范围已按 stride * height 校验，
                // 且在 CvBuffer 借用期间由 HostOps 保持有效。
                Some(unsafe { slice::from_raw_parts(data, len) })
            }
            _ => None,
        }
    }

    /// 将 RGB24 输出读回紧凑 Host 内存。
    ///
    /// DMA-BUF 路径显式执行 cache sync，并按真实 stride 逐行去除 padding；该接口只应
    /// 用于低频快照或证据生成，不应放入常驻检测热路径。
    pub fn readback_rgb24(&self) -> Result<Vec<u8>, AlgoError> {
        if self.format != PixelFormat::Rgb24 {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!("RGB24 readback 收到不支持的格式: {:?}", self.format),
            });
        }
        match &self.inner {
            CvBufferKind::Host(data) => copy_strided_rgb24(
                data,
                self.width,
                self.height,
                self.width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?,
            ),
            CvBufferKind::HostOps { .. } => {
                let data = self
                    .as_host_bytes()
                    .ok_or_else(|| AlgoError::IncompatibleFrame {
                        reason: "HostOps RGB24 缺少可读 Host 视图".to_string(),
                    })?;
                let stride = self
                    .as_image_view()
                    .and_then(|view| u32::try_from(view.stride[0]).ok())
                    .filter(|stride| *stride > 0)
                    .ok_or_else(|| AlgoError::IncompatibleFrame {
                        reason: "HostOps RGB24 stride 无效".to_string(),
                    })?;
                copy_strided_rgb24(data, self.width, self.height, stride)
            }
            CvBufferKind::DmaBuf {
                fd,
                size,
                stride,
                h_stride,
                ..
            } => readback_dma_rgb24(*fd, *size, self.width, self.height, stride[0], *h_stride),
            _ => Err(AlgoError::IncompatibleFrame {
                reason: "当前图像句柄不支持 RGB24 Host readback".to_string(),
            }),
        }
    }

    /// 获取底层 Host 内存可变切片
    pub fn as_host_bytes_mut(&mut self) -> Option<&mut [u8]> {
        match &mut self.inner {
            CvBufferKind::Host(vec) => Some(vec.as_mut_slice()),
            _ => None,
        }
    }

    /// 获取底层裸指针（若存在）
    pub fn as_raw_ptr(&self) -> Option<*mut c_void> {
        match &self.inner {
            CvBufferKind::ApplePixelBuffer { ptr, .. }
            | CvBufferKind::AscendDeviceMemory { ptr, .. } => Some(*ptr),
            CvBufferKind::HostOps { view, .. } => {
                if view.opaque_kind != AV_OPAQUE_NONE && !view.opaque.is_null() {
                    Some(view.opaque)
                } else if !view.data.is_null() {
                    let offset = usize::try_from(view.offset[0]).ok()?;
                    if offset > isize::MAX as usize {
                        return None;
                    }
                    // SAFETY: HostOps view 由分配回调提供，data + offset 是其首平面地址。
                    Some(unsafe { (view.data as *mut u8).add(offset) as *mut c_void })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

fn copy_strided_rgb24(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
) -> Result<Vec<u8>, AlgoError> {
    let width = usize::try_from(width).map_err(|_| AlgoError::OutOfMemory)?;
    let height = usize::try_from(height).map_err(|_| AlgoError::OutOfMemory)?;
    let stride = usize::try_from(stride).map_err(|_| AlgoError::OutOfMemory)?;
    let row_bytes = width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
    if stride < row_bytes {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!("RGB24 stride 小于有效行宽: {stride} < {row_bytes}"),
        });
    }
    let required = height
        .checked_sub(1)
        .and_then(|rows| rows.checked_mul(stride))
        .and_then(|offset| offset.checked_add(row_bytes))
        .ok_or(AlgoError::OutOfMemory)?;
    if data.len() < required {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!("RGB24 readback 数据不足: {} < {required}", data.len()),
        });
    }
    let output_len = row_bytes
        .checked_mul(height)
        .ok_or(AlgoError::OutOfMemory)?;
    let mut output = vec![0u8; output_len];
    if stride == row_bytes {
        output.copy_from_slice(&data[..output_len]);
        return Ok(output);
    }
    for (row, dst_row) in output.chunks_exact_mut(row_bytes).enumerate() {
        // 上面的 `required` 校验已证明末行不越界，行起点只能是 `row * stride`。
        let source_start = row * stride;
        dst_row.copy_from_slice(&data[source_start..source_start + row_bytes]);
    }
    Ok(output)
}

#[cfg(target_os = "linux")]
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
#[cfg(target_os = "linux")]
const DMA_BUF_SYNC_READ: u64 = 1;
#[cfg(target_os = "linux")]
const DMA_BUF_SYNC_END: u64 = 1 << 2;

#[cfg(target_os = "linux")]
fn sync_dma_buf(fd: i32, flags: u64) -> Result<(), AlgoError> {
    let mut sync = flags;
    // SAFETY: DMA_BUF_IOCTL_SYNC 只读取/更新本函数持有的 u64 ioctl 参数。
    let status = unsafe { libc::ioctl(fd, DMA_BUF_IOCTL_SYNC, &mut sync) };
    if status != 0 {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF RGB24 readback cache sync 失败: fd={fd}, flags={flags:#x}, error={}",
                std::io::Error::last_os_error()
            ),
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn readback_dma_rgb24(
    fd: i32,
    size: usize,
    width: u32,
    height: u32,
    stride: u32,
    h_stride: u32,
) -> Result<Vec<u8>, AlgoError> {
    const MAX_READBACK_BYTES: usize = 128 * 1024 * 1024;
    if fd < 0 || size == 0 || size > MAX_READBACK_BYTES {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!("RGB24 DMA-BUF readback 参数非法: fd={fd}, size={size}"),
        });
    }
    if h_stride < height {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!("RGB24 h_stride 小于有效高度: {h_stride} < {height}"),
        });
    }
    // 行布局（stride 下限与尾行偏移）统一交给 `copy_strided_rgb24` 校验，避免两处推导漂移。
    sync_dma_buf(fd, DMA_BUF_SYNC_READ)?;
    // SAFETY: fd 由 CvBuffer/lease 保持有效，映射长度已按 DMA-BUF 容量校验。
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        let error = AlgoError::IncompatibleFrame {
            reason: format!(
                "mmap RGB24 DMA-BUF fd={fd} 失败: {}",
                std::io::Error::last_os_error()
            ),
        };
        let _ = sync_dma_buf(fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END);
        return Err(error);
    }

    // SAFETY: mmap 成功且映射覆盖 size 字节；copy_strided_rgb24 只读取校验过的行范围。
    let data = unsafe { slice::from_raw_parts(mapped.cast::<u8>(), size) };
    let result = copy_strided_rgb24(data, width, height, stride);
    let sync_result = sync_dma_buf(fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END);
    // SAFETY: mapped/size 与本次 mmap 成功调用严格对应。
    let unmap_status = unsafe { libc::munmap(mapped, size) };
    sync_result?;
    if unmap_status != 0 {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "释放 RGB24 DMA-BUF 映射失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn readback_dma_rgb24(
    _fd: i32,
    _size: usize,
    _width: u32,
    _height: u32,
    _stride: u32,
    _h_stride: u32,
) -> Result<Vec<u8>, AlgoError> {
    Err(AlgoError::IncompatibleFrame {
        reason: "当前平台不支持 DMA-BUF RGB24 readback".to_string(),
    })
}

impl Drop for CvBuffer {
    fn drop(&mut self) {
        match &mut self.inner {
            CvBufferKind::DmaBuf { fd, close_fd, .. } if *close_fd && *fd >= 0 => {
                // SAFETY: close_fd 只在 SDK 接管有效 fd 所有权时为 true。
                unsafe {
                    libc::close(*fd);
                }
            }
            CvBufferKind::HostOps {
                view,
                ops_ctx,
                free_fn,
            } => {
                // SAFETY: view 与 ops_ctx 生命周期受 CvBuffer 保护，free_fn 为宿主注入的释放函数指针。
                unsafe {
                    free_fn(*ops_ctx, view as *mut AvImageView);
                }
            }
            CvBufferKind::ApplePixelBuffer {
                ptr,
                release: Some(release_fn),
            }
            | CvBufferKind::AscendDeviceMemory {
                ptr,
                release: Some(release_fn),
            } if !ptr.is_null() => {
                // SAFETY: release_fn 由构造者提供且 ptr 非空。
                unsafe {
                    release_fn(*ptr);
                }
            }
            _ => {}
        }
    }
}

impl std::fmt::Debug for CvBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CvBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_host_cv_buffer() {
        let data = vec![255u8; 640 * 640 * 3];
        let buf = CvBuffer::from_host(data, 640, 640, PixelFormat::Rgb24);
        assert_eq!(buf.width(), 640);
        assert_eq!(buf.height(), 640);
        assert_eq!(buf.format(), PixelFormat::Rgb24);
        assert_eq!(
            buf.as_host_bytes().expect("应存在 host bytes").len(),
            640 * 640 * 3
        );
        assert!(buf.as_dma_buf_fd().is_none());
    }

    #[test]
    fn test_host_ops_drop_called() {
        static FREED: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" fn mock_free(
            _ctx: *mut c_void,
            _view: *mut AvImageView,
        ) -> std::ffi::c_int {
            FREED.store(true, Ordering::SeqCst);
            0
        }

        let view = AvImageView {
            size: std::mem::size_of::<AvImageView>() as u32,
            api_version: AV_ALGO_API_VERSION,
            width: 320,
            height: 320,
            pixel_format: AV_PIX_RGB24,
            memory_type: AV_MEM_HOST,
            plane_count: 1,
            opaque_kind: AV_OPAQUE_NONE,
            stride: [320 * 3, 0, 0, 0],
            offset: [0; 4],
            data: std::ptr::null_mut(),
            opaque: std::ptr::null_mut(),
        };

        {
            let _buf = CvBuffer::from_host_ops(
                view,
                std::ptr::null_mut(),
                mock_free,
                320,
                320,
                PixelFormat::Rgb24,
            );
            assert!(!FREED.load(Ordering::SeqCst));
        }

        assert!(FREED.load(Ordering::SeqCst), "析构时必须调用 free 回调");
    }

    #[test]
    fn test_readback_rgb24_removes_row_padding() {
        let data = [
            1, 2, 3, 4, 5, 6, 0xaa, 0xbb, // row 0 + padding
            7, 8, 9, 10, 11, 12, 0xcc, 0xdd, // row 1 + padding
        ];
        assert_eq!(
            copy_strided_rgb24(&data, 2, 2, 8).expect("stride readback 应成功"),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }

    #[test]
    fn test_readback_rgb24_rejects_wrong_format() {
        let buf = CvBuffer::from_host(vec![0; 4 * 4 * 3 / 2], 4, 4, PixelFormat::Nv12);
        assert!(matches!(
            buf.readback_rgb24(),
            Err(AlgoError::IncompatibleFrame { .. })
        ));
    }

    #[test]
    fn test_dma_buf_cv_buffer() {
        struct MockGuard(Arc<AtomicBool>);
        impl Drop for MockGuard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        {
            let guard = Box::new(MockGuard(dropped.clone()));
            let buf = CvBuffer::from_dma_buf(99, 1920, 1080, PixelFormat::Nv12, Some(guard));
            assert_eq!(buf.as_dma_buf_fd(), Some(99));
            assert!(!dropped.load(Ordering::SeqCst));
        }
        assert!(dropped.load(Ordering::SeqCst));
    }
}
