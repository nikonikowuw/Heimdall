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
    DmaBuf { fd: std::os::fd::OwnedFd },
    /// 设备内存地址（如 Ascend DVPP 显存指针，保持 types 纯净不引入平台专有库）
    DeviceMemory {
        ptr: std::ptr::NonNull<std::ffi::c_void>,
        size: usize,
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

impl std::fmt::Debug for FrameHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(target_os = "linux")]
            Self::DmaBuf { fd } => f.debug_struct("DmaBuf").field("fd", fd).finish(),
            Self::DeviceMemory { ptr, size } => f
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
#[derive(Debug)]
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
