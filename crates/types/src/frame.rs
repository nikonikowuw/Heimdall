use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// 像素格式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Nv12,
    Yuv420p,
    Rgb24,
    Bgr24,
    Rgba,
}

/// 平台对齐跨度信息（硬件加速硬解核心元数据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrideInfo {
    /// 水平虚宽 / 行步长
    pub hor_stride: u32,
    /// 垂直虚高 / 内存行数
    pub ver_stride: u32,
}

impl StrideInfo {
    pub fn new(hor_stride: u32, ver_stride: u32) -> Self {
        Self {
            hor_stride,
            ver_stride,
        }
    }
}

/// 平台原生硬件内存句柄封装
pub enum FrameHandle {
    #[cfg(target_os = "linux")]
    DmaBuf {
        /// 使用 Arc 共享内核文件描述符，彻底消除高频克隆时的 dup 失败与 Panic 风险
        fd: Arc<std::os::fd::OwnedFd>,
        /// 缓冲区池租约 — 持有期间底层 CMA 物理页不被 VPU 缓冲区组回收复用。
        /// None 仅用于测试或非池化场景。
        _lease: Option<Arc<dyn Send + Sync>>,
    },
    /// 设备内存地址（如 Ascend DVPP 显存指针，保持 types 纯净不引入平台专有库）
    DeviceMemory {
        ptr: std::ptr::NonNull<std::ffi::c_void>,
        size: usize,
        /// 显存池租约 — 最后一个 Arc 引用析构时归还显存至预分配池
        _lease: Arc<dyn Send + Sync>,
    },
    /// Apple CVPixelBuffer 原生指针封装
    ApplePixelBuffer {
        ptr: std::ptr::NonNull<std::ffi::c_void>,
    },
    /// 主机内存切片（开发测试或 CPU 回退使用）
    Host(Arc<[u8]>),
}

// SAFETY: FrameHandle 封装的原生句柄（如 CVPixelBuffer、DeviceMemory 或 OwnedFd）
// 其生命周期由 FrameRef 管理，在线程间转移所有权时底层内存地址或内核 fd 保持有效，
// 满足跨线程转移 (Send) 的安全约束。
unsafe impl Send for FrameHandle {}
// SAFETY: FrameHandle 在各线程只读访问时安全。
unsafe impl Sync for FrameHandle {}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferRetain(pixel_buffer: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CVPixelBufferRelease(pixel_buffer: *mut std::ffi::c_void);
}

impl Clone for FrameHandle {
    fn clone(&self) -> Self {
        match self {
            #[cfg(target_os = "linux")]
            Self::DmaBuf { fd, _lease } => Self::DmaBuf {
                fd: Arc::clone(fd),
                _lease: _lease.clone(),
            },
            Self::DeviceMemory { ptr, size, _lease } => Self::DeviceMemory {
                ptr: *ptr,
                size: *size,
                _lease: Arc::clone(_lease),
            },
            Self::ApplePixelBuffer { ptr } => {
                #[cfg(target_os = "macos")]
                // SAFETY: ptr 封装的是有效的 CVPixelBufferRef 指针，克隆句柄时增加引用计数以保持 RAII 对称
                unsafe {
                    CVPixelBufferRetain(ptr.as_ptr());
                }
                Self::ApplePixelBuffer { ptr: *ptr }
            }
            Self::Host(slice) => Self::Host(slice.clone()),
        }
    }
}

impl Drop for FrameHandle {
    fn drop(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::ApplePixelBuffer { ptr } => {
                // SAFETY: ptr 封装的是有效 CVPixelBufferRef，在句柄析构时调用 CVPixelBufferRelease 归还引用
                unsafe {
                    CVPixelBufferRelease(ptr.as_ptr());
                }
            }
            #[cfg(target_os = "linux")]
            Self::DmaBuf { .. } => {
                // OwnedFd 自身析构时安全关闭 fd
            }
            _ => {}
        }
    }
}

impl std::fmt::Debug for FrameHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(target_os = "linux")]
            Self::DmaBuf { fd, .. } => f.debug_struct("DmaBuf").field("fd", fd.as_ref()).finish(),
            Self::DeviceMemory { ptr, size, .. } => f
                .debug_struct("DeviceMemory")
                .field("ptr", ptr)
                .field("size", size)
                .finish(),
            Self::ApplePixelBuffer { ptr } => f
                .debug_struct("ApplePixelBuffer")
                .field("ptr", ptr)
                .finish(),
            Self::Host(slice) => f.debug_struct("Host").field("len", &slice.len()).finish(),
        }
    }
}

/// 跨线程/跨层流转的统一视频帧持有句柄
#[derive(Debug, Clone)]
pub struct FrameRef {
    pub camera_id: String,
    /// 13 位 UTC Unix 毫秒时间戳
    pub timestamp: i64,
    pub width: u32,
    pub height: u32,
    pub stride: StrideInfo,
    pub format: PixelFormat,
    handle: FrameHandle,
}

impl FrameRef {
    pub fn new(
        camera_id: String,
        timestamp: i64,
        width: u32,
        height: u32,
        stride: StrideInfo,
        format: PixelFormat,
        handle: FrameHandle,
    ) -> Self {
        Self {
            camera_id,
            timestamp,
            width,
            height,
            stride,
            format,
            handle,
        }
    }

    /// 获取底层平台原生句柄引用
    pub fn handle(&self) -> &FrameHandle {
        &self.handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TestLease {
        dropped: Arc<AtomicBool>,
    }

    impl Drop for TestLease {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_device_memory_lease_drop() {
        let dropped = Arc::new(AtomicBool::new(false));
        let lease: Arc<dyn Send + Sync> = Arc::new(TestLease {
            dropped: dropped.clone(),
        });

        let dummy_ptr = std::ptr::NonNull::dangling();
        let handle = FrameHandle::DeviceMemory {
            ptr: dummy_ptr,
            size: 1024,
            _lease: lease,
        };

        let cloned = handle.clone();
        assert!(!dropped.load(Ordering::SeqCst));

        drop(handle);
        assert!(
            !dropped.load(Ordering::SeqCst),
            "原句柄 drop 后克隆副本仍应持有租约"
        );

        drop(cloned);
        assert!(
            dropped.load(Ordering::SeqCst),
            "所有句柄副本 drop 后租约必须触发释放"
        );
    }

    #[test]
    fn test_stride_info() {
        let stride = StrideInfo::new(1920, 1088);
        assert_eq!(stride.hor_stride, 1920);
        assert_eq!(stride.ver_stride, 1088);
    }
}
