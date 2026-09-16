//! 运动门控缩略图（Rockchip RGA 硬件降采样 + 常驻 CPU 映射）
//!
//! 常驻推理主路径的运动门控需要一份 CPU 可读像素，而硬解产物是 DMA-BUF：CPU 既不持有映射，
//! 也不允许把整帧 1080p 回读（`infer_fast_path` 的零拷贝边界）。本模块用 RGA 把整帧 NV12 降到
//! 一张定长缩略图（320×180）并常驻 `mmap`，每帧只回读 Y 平面（57,600 字节），
//! 是常驻推理路径上唯一被允许的 CPU readback。
//!
//! 只回读 Y 平面：门控只判断"画面是否变化"，色度平面既不参与差分也无信息增益。

#![cfg(all(target_os = "linux", feature = "rga"))]

use std::ffi::c_void;
use std::os::fd::{AsRawFd, OwnedFd};

use types::{FrameHandle, FrameRef, PixelFormat};

use crate::dmabuf_sync::{alloc_dma_buf, DmaBufSyncDirection, DmaBufSyncGuard};
use crate::error::MediaError;
use crate::rga_crop::{get_global_rga, RK_FORMAT_YCbCr_420_SP, RgaRuntime, RgaSource};

/// NV12 (YUV420SP) 平面总字节数
fn nv12_len(width: u32, height: u32) -> Result<usize, MediaError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(3))
        .map(|v| v / 2)
        .ok_or_else(|| MediaError::Encode {
            reason: "NV12 缩略图尺寸计算溢出".to_string(),
        })
}

/// 常驻 DMA-BUF 只读映射区（析构即 `munmap`）
struct MmapRegion {
    ptr: *mut c_void,
    len: usize,
}

impl Drop for MmapRegion {
    fn drop(&mut self) {
        // SAFETY: ptr/len 来自成功的 mmap，本结构独占该映射直至析构
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

// SAFETY: 映射区只被持有它的 `MotionThumbnailScaler` 在单一解码线程内访问；
// `munmap` 与结构析构严格成对，不存在跨线程共享的可变别名。
unsafe impl Send for MmapRegion {}

/// RGA 门控缩略图缩放器（每路相机一份：常驻 DMA-BUF + 常驻 CPU 映射 + 常驻 RGA 句柄）
pub struct MotionThumbnailScaler {
    rga: std::sync::Arc<RgaRuntime>,
    map: MmapRegion,
    dst_fd: OwnedFd,
    dst_handle: u32,
    width: u32,
    height: u32,
    /// Y 平面宿主暂存（容量在构造时按需预留，运行期不再分配）
    y_scratch: Vec<u8>,
}

impl std::fmt::Debug for MotionThumbnailScaler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotionThumbnailScaler")
            .field("size", &format!("{}x{}", self.width, self.height))
            .field(
                "y_plane_bytes",
                &(self.width as usize * self.height as usize),
            )
            .finish()
    }
}

impl Drop for MotionThumbnailScaler {
    fn drop(&mut self) {
        self.rga.release_buffer_handle(self.dst_handle);
    }
}

impl MotionThumbnailScaler {
    /// 创建常驻缩略图：分配 DMA-BUF、导入常驻 RGA 句柄、建立常驻 CPU 只读映射。
    ///
    /// 硬件约束：目标宽度必须 4 像素对齐（RGA2 目标 stride 下限），高度 2 像素对齐（NV12）。
    pub fn new(width: u32, height: u32) -> Result<Self, MediaError> {
        if width == 0 || height == 0 || !width.is_multiple_of(4) || !height.is_multiple_of(2) {
            return Err(MediaError::Encode {
                reason: format!(
                    "非法门控缩略图尺寸: {width}x{height} (宽度需 4 像素对齐、高度需 2 像素对齐)"
                ),
            });
        }

        let len = nv12_len(width, height)?;
        let rga = get_global_rga()?;
        let dst_fd = alloc_dma_buf(len)?;
        let raw_fd = dst_fd.as_raw_fd();
        let dst_handle = rga.import_buffer_fd(raw_fd, width, height, RK_FORMAT_YCbCr_420_SP)?;

        // SAFETY: 基于具有生命周期的 OwnedFd 建立只读共享映射，映射在 MmapRegion 析构时解除
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                raw_fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED || ptr.is_null() {
            let error = std::io::Error::last_os_error();
            rga.release_buffer_handle(dst_handle);
            return Err(MediaError::Encode {
                reason: format!("门控缩略图 mmap 失败: {error}"),
            });
        }

        Ok(Self {
            rga,
            map: MmapRegion { ptr, len },
            dst_fd,
            dst_handle,
            width,
            height,
            y_scratch: Vec::with_capacity(width as usize * height as usize),
        })
    }

    /// 缩略图尺寸 `(宽, 高)`
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 把一张 NV12 DMA-BUF 帧降采样进常驻缩略图，返回 Y 平面只读切片。
    ///
    /// 返回切片借用了内部宿主暂存，由编译器保证同一时刻只有一帧在消费这张缩略图；
    /// 内容在下一次调用时被覆写。
    pub fn scale_y_plane(&mut self, frame: &FrameRef) -> Result<&[u8], MediaError> {
        let FrameHandle::DmaBuf { fd, .. } = frame.handle() else {
            return Err(MediaError::Encode {
                reason: format!("门控缩略图仅支持 DMA-BUF 帧, 实际: {:?}", frame.handle()),
            });
        };
        if frame.format != PixelFormat::Nv12 {
            return Err(MediaError::Encode {
                reason: format!("门控缩略图仅支持 NV12, 实际: {:?}", frame.format),
            });
        }

        let (src_w, src_h) = (frame.width, frame.height);
        if src_w < self.width || src_h < self.height {
            return Err(MediaError::Encode {
                reason: format!(
                    "源帧 {src_w}x{src_h} 小于门控缩略图 {}x{}，拒绝放大",
                    self.width, self.height
                ),
            });
        }

        let src_fd = fd.as_raw_fd();
        let dst_fd = self.dst_fd.as_raw_fd();
        // 源可见区域与分配跨步分开传入；硬解常见 `stride > width`（按 16 对齐分配）
        let source = RgaSource {
            fd: src_fd,
            width: src_w,
            height: src_h,
            hor_stride: frame.stride.hor_stride.max(src_w),
            ver_stride: frame.stride.ver_stride.max(src_h),
        };
        // `scale_sync` 为同步提交（IM_SYNC），返回时 RGA 写栅障已闭合
        self.rga
            .scale_sync(source, self.dst_handle, self.width, self.height)?;

        // 目标缓冲可能来自 cached 堆（cma / system-dma32）：CPU 读取前后必须成对执行
        // DMA_BUF_IOCTL_SYNC，且真正的 CPU 读（拷贝）必须落在 START/END 之间
        // （与 `image_convert::convert_nv12_dmabuf_to_rgb` 同构）。
        let sync_guard = DmaBufSyncGuard::acquire(dst_fd, DmaBufSyncDirection::Read)?;
        let y_len = self.width as usize * self.height as usize;
        // SAFETY: map.ptr 在本结构生命周期内始终是长度 len >= y_len 的有效只读映射
        let mapped = unsafe { std::slice::from_raw_parts(self.map.ptr as *const u8, y_len) };
        self.y_scratch.clear();
        self.y_scratch.extend_from_slice(mapped);
        sync_guard.finish()?;

        Ok(&self.y_scratch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nv12_len() {
        assert_eq!(nv12_len(320, 180).expect("合法尺寸"), 320 * 180 * 3 / 2);
        assert!(nv12_len(u32::MAX, u32::MAX).is_err());
    }

    /// 硬件依赖用例：无 `/dev/rga` 或 librga 时跳过，必须在具备 RGA 的板端显式执行：
    /// `cargo test -p media --features rga -- --ignored`
    #[test]
    #[ignore = "需要 Rockchip RGA 硬件与 /dev/dma_heap"]
    fn test_scale_y_plane_from_synthetic_dmabuf() {
        let (src_w, src_h) = (1920u32, 1080u32);
        let src_len = nv12_len(src_w, src_h).expect("源尺寸合法");
        let src_fd = alloc_dma_buf(src_len).expect("分配源 DMA-BUF");

        // 源帧填满 200 灰度：整图降采样后缩略图所有像素必须同值（缩放不引入边界黑边）
        // SAFETY: 基于 OwnedFd 的只读共享映射，长度为 src_len
        let src_ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                src_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                src_fd.as_raw_fd(),
                0,
            )
        };
        assert_ne!(src_ptr, libc::MAP_FAILED, "源 DMA-BUF mmap 失败");
        // SAFETY: src_ptr 为刚建立的合法映射，长度 src_len
        unsafe { std::ptr::write_bytes(src_ptr as *mut u8, 200, src_len) };
        // SAFETY: 解除临时映射
        unsafe { libc::munmap(src_ptr, src_len) };

        let frame = FrameRef::new(
            "cam_thumb".to_string(),
            1000,
            src_w,
            src_h,
            types::StrideInfo::new(src_w, src_h),
            PixelFormat::Nv12,
            FrameHandle::DmaBuf {
                fd: std::sync::Arc::new(src_fd),
                _lease: None,
            },
        );

        let mut scaler = MotionThumbnailScaler::new(320, 180).expect("创建门控缩略图");
        let y = scaler.scale_y_plane(&frame).expect("RGA 降采样");
        assert_eq!(y.len(), 320 * 180);
        assert!(
            y.iter().all(|v| *v == 200),
            "整帧同值时缩略图不得出现未被覆盖的黑色区域"
        );
    }
}
