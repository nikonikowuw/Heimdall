//! 测试脚手架 (testing)
//!
//! 提供 `MockFrameBuilder`、`MockEmitter` 与 `MockWeights`，
//! 支持在单测中直接加载图片并仿真平台硬件加速格式（NV12 + Stride 步长对齐 + 平台原生句柄）。

use std::ffi::{c_void, CStr};
#[cfg(feature = "testing-image")]
use std::path::Path;

use crate::c_abi::*;
use crate::cv::buffer::CvBuffer;
use crate::error::AlgoError;
use crate::frame::SafeFrame;
use crate::math::NormBox;
use crate::model::{Core, InferenceSession, ModelWeights};

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
    ) -> std::ffi::c_int;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> std::ffi::c_int;
    fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: *mut c_void,
        unlock_flags: u64,
    ) -> std::ffi::c_int;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut c_void,
        plane_index: usize,
    ) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer: *mut c_void, plane_index: usize) -> usize;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
}

#[cfg(all(target_os = "macos", feature = "testing-hardware"))]
const K_CVPIXEL_FORMAT_NV12: u32 = 0x34323076; // '420v'

#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
#[repr(C)]
struct DmaHeapAllocationData {
    len: u64,
    fd: i32,
    fd_flags: u32,
    heap_flags: u64,
}

#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
const DMA_HEAP_IOCTL_ALLOC: libc::c_ulong = 0xc018_4800;

#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
#[repr(C)]
struct DmaBufSync {
    flags: u64,
}

#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
const DMA_BUF_SYNC_WRITE: u64 = 2;
#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
const DMA_BUF_SYNC_START: u64 = 0;
#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
const DMA_BUF_SYNC_END: u64 = 4;

#[cfg(all(target_os = "linux", feature = "testing-hardware"))]
fn allocate_real_dma_buf(data: &[u8]) -> Result<std::os::fd::OwnedFd, AlgoError> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let heap_path = std::ffi::CString::new("/dev/dma_heap/system").expect("static path");
    // SAFETY: heap_path 是合法的 NUL 结尾路径，open 返回的 fd 由 OwnedFd 接管。
    let heap_fd = unsafe { libc::open(heap_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if heap_fd < 0 {
        return Err(AlgoError::Internal {
            reason: format!(
                "打开 /dev/dma_heap/system 失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    // SAFETY: heap_fd 是 open 成功返回的唯一 fd。
    let _heap = unsafe { OwnedFd::from_raw_fd(heap_fd) };

    // SAFETY: _SC_PAGESIZE 为只读系统页大小配置查询，无内存安全风险。
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = if page_size > 0 {
        page_size as usize
    } else {
        4096
    };
    let alloc_len = data
        .len()
        .max(1)
        .checked_add(page_size - 1)
        .map(|len| len / page_size * page_size)
        .ok_or(AlgoError::OutOfMemory)?;
    let mut allocation = DmaHeapAllocationData {
        len: alloc_len as u64,
        fd: 0,
        fd_flags: (libc::O_RDWR | libc::O_CLOEXEC) as u32,
        heap_flags: 0,
    };

    // SAFETY: allocation 指向内核要求的固定布局结构体，ioctl 编号匹配
    // dma_heap_allocation_data (u64 + i32 + u32 + u64 = 24 bytes)。
    let status = unsafe {
        libc::ioctl(
            heap_fd,
            DMA_HEAP_IOCTL_ALLOC,
            &mut allocation as *mut DmaHeapAllocationData,
        )
    };
    if status < 0 || allocation.fd < 0 {
        if allocation.fd >= 0 {
            // SAFETY: 内核在失败路径返回了一个需要由用户态关闭的 fd。
            unsafe {
                libc::close(allocation.fd);
            }
        }
        return Err(AlgoError::Internal {
            reason: format!("DMA-Heap 分配失败: {}", std::io::Error::last_os_error()),
        });
    }

    // SAFETY: ioctl 成功后 allocation.fd 是新转移给用户态的 dma-buf fd。
    let dma_fd = unsafe { OwnedFd::from_raw_fd(allocation.fd) };
    // SAFETY: dma-buf fd 已由同一进程打开，映射长度不超过申请长度；映射只在本函数内使用。
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            alloc_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            dma_fd.as_raw_fd(),
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(AlgoError::Internal {
            reason: format!("DMA-BUF mmap 失败: {}", std::io::Error::last_os_error()),
        });
    }
    let mut sync = DmaBufSync {
        flags: DMA_BUF_SYNC_START | DMA_BUF_SYNC_WRITE,
    };
    // SAFETY: sync 是内核定义的 u64 flags 结构体，dma_fd 与映射均有效。
    let start_status = unsafe {
        libc::ioctl(
            dma_fd.as_raw_fd(),
            DMA_BUF_IOCTL_SYNC,
            &mut sync as *mut DmaBufSync,
        )
    };
    if start_status < 0 {
        // SAFETY: mapped 是刚刚成功建立的 alloc_len 字节映射。
        unsafe {
            libc::munmap(mapped, alloc_len);
        }
        return Err(AlgoError::Internal {
            reason: format!(
                "DMA-BUF CPU 写入同步开始失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }

    // SAFETY: mapped 覆盖 alloc_len 字节，data 只复制其实际长度。
    if !data.is_empty() {
        // SAFETY: mapped is a valid writable DMA-BUF mapping and data fits in the allocation.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), mapped.cast::<u8>(), data.len());
        }
    }

    sync.flags = DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE;
    // SAFETY: 与开始同步使用同一个有效的 dma-buf fd 和同步结构体。
    let end_status = unsafe {
        libc::ioctl(
            dma_fd.as_raw_fd(),
            DMA_BUF_IOCTL_SYNC,
            &mut sync as *mut DmaBufSync,
        )
    };
    // SAFETY: mapped 是本函数建立的映射，始终只解除一次。
    unsafe {
        libc::munmap(mapped, alloc_len);
    }
    if end_status < 0 {
        return Err(AlgoError::Internal {
            reason: format!(
                "DMA-BUF CPU 写入同步结束失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }

    Ok(dma_fd)
}

fn required_host_bytes(
    width: u32,
    height: u32,
    pixel_format: u32,
    stride: [i32; 4],
    offset: [u64; 4],
) -> Option<usize> {
    let width = width as usize;
    let height = height as usize;
    let stride0 = if stride[0] > 0 {
        stride[0] as usize
    } else {
        match pixel_format {
            AV_PIX_NV12 | AV_PIX_I420 => width,
            AV_PIX_RGB24 => width.checked_mul(3)?,
            AV_PIX_BGRA => width.checked_mul(4)?,
            _ => return None,
        }
    };
    let offset0 = usize::try_from(offset[0]).ok()?;
    let y_end = offset0.checked_add(stride0.checked_mul(height)?)?;
    match pixel_format {
        AV_PIX_NV12 => {
            let uv_width = width.div_ceil(2).checked_mul(2)?;
            let stride1 = if stride[1] > 0 {
                stride[1] as usize
            } else {
                stride0.max(uv_width)
            };
            let uv_start = if offset[1] > 0 {
                usize::try_from(offset[1]).ok()?
            } else {
                y_end
            };
            uv_start.checked_add(stride1.checked_mul(height.div_ceil(2))?)
        }
        AV_PIX_I420 => {
            let chroma_width = width.div_ceil(2);
            let stride1 = if stride[1] > 0 {
                stride[1] as usize
            } else {
                chroma_width
            };
            let stride2 = if stride[2] > 0 {
                stride[2] as usize
            } else {
                stride1
            };
            let rows = height.div_ceil(2);
            let u_start = if offset[1] > 0 {
                usize::try_from(offset[1]).ok()?
            } else {
                y_end
            };
            let v_start = if offset[2] > 0 {
                usize::try_from(offset[2]).ok()?
            } else {
                u_start.checked_add(stride1.checked_mul(rows)?)?
            };
            v_start.checked_add(stride2.checked_mul(rows)?)
        }
        AV_PIX_RGB24 | AV_PIX_BGRA => Some(y_end),
        _ => None,
    }
}

/// 模拟帧底层内存存储
#[derive(Debug)]
enum MockStorage {
    Host(Vec<u8>),
    #[cfg(target_os = "macos")]
    ApplePixelBuffer(*mut c_void),
    #[cfg(all(target_os = "linux", feature = "testing-hardware"))]
    DmaBuf(std::os::fd::OwnedFd),
}

// SAFETY: MockStorage 管理底层显存独占所有权，支持线程间移动
unsafe impl Send for MockStorage {}

impl Drop for MockStorage {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let MockStorage::ApplePixelBuffer(ptr) = self {
            if !ptr.is_null() {
                // SAFETY: 释放测试 CVPixelBuffer
                unsafe {
                    CVPixelBufferRelease(*ptr);
                }
            }
        }
    }
}

/// 模拟构造的视频帧（持有底层显存/内存所有权）
#[derive(Debug)]
pub struct MockFrame {
    desc: AvFrameDesc,
    _storage: MockStorage,
}

impl MockFrame {
    /// 借用为只读安全帧视图
    pub fn as_safe_frame(&self) -> SafeFrame<'_> {
        SafeFrame::from_ref(&self.desc).expect("MockFrame 内部描述符必须有效")
    }

    /// 获取底层原始 AvFrameDesc
    pub fn raw_desc(&self) -> &AvFrameDesc {
        &self.desc
    }
}

/// 模拟帧构造器
#[derive(Debug)]
pub struct MockFrameBuilder {
    width: u32,
    height: u32,
    pixel_format: u32,
    stride: [i32; 4],
    stride_explicit: bool,
    offset: [u64; 4],
    raw_bytes: Vec<u8>,
    opaque_kind: u32,
    hardware_storage: Option<MockStorage>,
    timestamp_ns: i64,
}

impl Default for MockFrameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MockFrameBuilder {
    pub fn new() -> Self {
        Self {
            width: 640,
            height: 480,
            pixel_format: AV_PIX_RGB24,
            stride: [640 * 3, 0, 0, 0],
            stride_explicit: false,
            offset: [0; 4],
            raw_bytes: Vec::new(),
            opaque_kind: AV_OPAQUE_NONE,
            hardware_storage: None,
            timestamp_ns: 1_000_000,
        }
    }

    pub fn dimensions(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        if !self.stride_explicit {
            self.update_default_strides();
        }
        self.hardware_storage = None;
        self
    }

    pub fn pixel_format(mut self, format: u32) -> Self {
        self.pixel_format = format;
        if !self.stride_explicit {
            self.update_default_strides();
        }
        self.hardware_storage = None;
        self
    }

    pub fn stride(mut self, stride0: i32, stride1: i32) -> Self {
        self.stride[0] = stride0;
        self.stride[1] = stride1;
        self.stride_explicit = true;
        self.hardware_storage = None;
        self
    }

    pub fn plane_offsets(mut self, offset0: u64, offset1: u64, offset2: u64) -> Self {
        self.offset = [offset0, offset1, offset2, 0];
        self.hardware_storage = None;
        self
    }

    pub fn host_data(mut self, data: Vec<u8>) -> Self {
        self.raw_bytes = data;
        self.hardware_storage = None;
        self
    }

    pub fn timestamp_ns(mut self, ts: i64) -> Self {
        self.timestamp_ns = ts;
        self
    }

    pub fn opaque_kind(mut self, kind: u32) -> Self {
        self.opaque_kind = kind;
        self
    }

    fn update_default_strides(&mut self) {
        let width = self.width as usize;
        let chroma_width = width.div_ceil(2);
        self.stride = match self.pixel_format {
            AV_PIX_NV12 => [width as i32, (chroma_width * 2) as i32, 0, 0],
            AV_PIX_I420 => [width as i32, chroma_width as i32, chroma_width as i32, 0],
            AV_PIX_BGRA => [(width * 4) as i32, 0, 0, 0],
            AV_PIX_RGB24 => [(width * 3) as i32, 0, 0, 0],
            _ => [0; 4],
        };
    }

    /// 将当前 RGB24 像素数据转换为硬件标准的 NV12 格式，并根据 alignment 执行行跨距对齐
    pub fn to_nv12(mut self, alignment: u32) -> Self {
        let align = alignment.max(1) as usize;
        let w = self.width as usize;
        let h = self.height as usize;
        let uv_width = w.div_ceil(2).saturating_mul(2);

        let y_stride = w.div_ceil(align).saturating_mul(align);
        let uv_stride = uv_width.div_ceil(align).saturating_mul(align);
        let y_plane_size = y_stride.checked_mul(h).expect("测试帧 Y 平面大小溢出");
        let uv_plane_size = uv_stride
            .checked_mul(h.div_ceil(2))
            .expect("测试帧 UV 平面大小溢出");
        let mut nv12 = vec![0u8; y_plane_size + uv_plane_size];

        let clamp_byte = |v: f32| -> u8 { v.clamp(0.0, 255.0).round() as u8 };

        // 默认若无 raw_bytes 则生成灰色画面
        let rgb = if self.raw_bytes.len() >= w.saturating_mul(h).saturating_mul(3) {
            self.raw_bytes
        } else {
            vec![128u8; w.saturating_mul(h).saturating_mul(3)]
        };

        // 写入 Y 平面
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) * 3;
                let r = rgb[idx] as f32;
                let g = rgb[idx + 1] as f32;
                let b = rgb[idx + 2] as f32;
                let y_val = clamp_byte(16.0 + (65.481 * r + 128.553 * g + 24.966 * b) / 255.0);
                nv12[y * y_stride + x] = y_val;
            }
        }

        // 写入 UV 平面 (2x2 亚采样；奇数宽高复制边缘像素)
        let uv_base = y_plane_size;
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let mut cb_sum = 0.0f32;
                let mut cr_sum = 0.0f32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sample_y = (y + dy).min(h.saturating_sub(1));
                        let sample_x = (x + dx).min(w.saturating_sub(1));
                        let idx = (sample_y * w + sample_x) * 3;
                        let r = rgb[idx] as f32;
                        let g = rgb[idx + 1] as f32;
                        let b = rgb[idx + 2] as f32;
                        cb_sum += 128.0 + (-37.797 * r - 74.203 * g + 112.0 * b) / 255.0;
                        cr_sum += 128.0 + (112.0 * r - 93.786 * g - 18.214 * b) / 255.0;
                    }
                }
                let cb = clamp_byte(cb_sum / 4.0);
                let cr = clamp_byte(cr_sum / 4.0);

                let uv_idx = uv_base + (y / 2) * uv_stride + x;
                nv12[uv_idx] = cb;
                nv12[uv_idx + 1] = cr;
            }
        }

        self.pixel_format = AV_PIX_NV12;
        self.stride = [y_stride as i32, uv_stride as i32, 0, 0];
        self.stride_explicit = true;
        self.offset = [0; 4];
        self.raw_bytes = nv12;
        self.hardware_storage = None;
        self
    }

    /// 从本地图片文件直接加载 (需要开启 feature = "testing-image")
    #[cfg(feature = "testing-image")]
    pub fn from_image_file(path: impl AsRef<Path>) -> Result<Self, AlgoError> {
        let img = image::open(path).map_err(|e| AlgoError::Preprocess {
            reason: format!("加载测试图片失败: {e}"),
        })?;
        let rgb_img = img.to_rgb8();
        let (w, h) = rgb_img.dimensions();

        Ok(Self::new()
            .dimensions(w, h)
            .pixel_format(AV_PIX_RGB24)
            .stride((w * 3) as i32, 0)
            .host_data(rgb_img.into_raw()))
    }

    /// 一键从本地图片加载并转换为当前系统平台的原生硬件加速帧格式
    /// - macOS: CoreVideo NV12 CVPixelBuffer (`AV_OPAQUE_CVPIXELBUFFER`)
    /// - Linux: DMA-BUF fd (`AV_OPAQUE_DMABUF`)
    /// - 兜底: 步长对齐的 NV12 格式
    #[cfg(feature = "testing-hardware")]
    pub fn from_image_hardware(path: impl AsRef<Path>) -> Result<Self, AlgoError> {
        let builder = Self::from_image_file(path)?;

        #[cfg(target_os = "macos")]
        {
            // 转换为 64 字节对齐的 NV12，并写入真实 CVPixelBuffer
            let nv12_builder = builder.to_nv12(64);
            let mut raw_buf: *mut c_void = std::ptr::null_mut();

            // SAFETY: 调用 macOS 原生 CVPixelBufferCreate 创建双平面 420v
            let status = unsafe {
                CVPixelBufferCreate(
                    std::ptr::null(),
                    nv12_builder.width as usize,
                    nv12_builder.height as usize,
                    K_CVPIXEL_FORMAT_NV12,
                    std::ptr::null(),
                    &mut raw_buf,
                )
            };

            if status != 0 || raw_buf.is_null() {
                return Err(AlgoError::Internal {
                    reason: format!("CVPixelBufferCreate 失败: {status}"),
                });
            }

            // SAFETY: 锁定并填充 Y/UV 数据
            let lock_status = unsafe { CVPixelBufferLockBaseAddress(raw_buf, 0) };
            if lock_status != 0 {
                // SAFETY: 锁定失败释放
                unsafe { CVPixelBufferRelease(raw_buf) };
                return Err(AlgoError::Internal {
                    reason: format!("CVPixelBufferLockBaseAddress 失败: {lock_status}"),
                });
            }

            // SAFETY: 获取平面地址并写入
            unsafe {
                let y_ptr = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 0) as *mut u8;
                let uv_ptr = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 1) as *mut u8;
                let ys = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 0);
                let uvs = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 1);

                let src_stride0 = nv12_builder.stride[0] as usize;
                let src_stride1 = nv12_builder.stride[1] as usize;
                let h = nv12_builder.height as usize;
                let w = nv12_builder.width as usize;

                let src_data = &nv12_builder.raw_bytes;
                for y in 0..h {
                    std::ptr::copy_nonoverlapping(
                        src_data.as_ptr().add(y * src_stride0),
                        y_ptr.add(y * ys),
                        w,
                    );
                }

                let uv_base = src_stride0 * h;
                for y in 0..h.div_ceil(2) {
                    std::ptr::copy_nonoverlapping(
                        src_data.as_ptr().add(uv_base + y * src_stride1),
                        uv_ptr.add(y * uvs),
                        w,
                    );
                }

                CVPixelBufferUnlockBaseAddress(raw_buf, 0);
            }

            let mut res = nv12_builder;
            res.opaque_kind = AV_OPAQUE_CVPIXELBUFFER;
            res.raw_bytes = Vec::new(); // 像素数据已转移到 CVPixelBuffer
            res.hardware_storage = Some(MockStorage::ApplePixelBuffer(raw_buf));
            Ok(res)
        }

        #[cfg(target_os = "linux")]
        {
            // 只有 DMA-Heap 返回的 fd 才能作为真实 DMA-BUF 句柄交给硬件。
            let nv12_builder = builder.to_nv12(16);
            let dma_fd = allocate_real_dma_buf(&nv12_builder.raw_bytes)?;

            let mut res = nv12_builder;
            res.opaque_kind = AV_OPAQUE_DMABUF;
            res.hardware_storage = Some(MockStorage::DmaBuf(dma_fd));
            Ok(res)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Ok(builder.to_nv12(16))
        }
    }

    /// 产出拥有完整生命周期的 MockFrame
    pub fn build(self) -> MockFrame {
        let MockFrameBuilder {
            width,
            height,
            pixel_format,
            stride,
            stride_explicit: _,
            offset,
            mut raw_bytes,
            opaque_kind,
            hardware_storage,
            timestamp_ns,
        } = self;

        let required_bytes = required_host_bytes(width, height, pixel_format, stride, offset);
        if hardware_storage.is_none() {
            if raw_bytes.is_empty() {
                let len = required_bytes.unwrap_or(0);
                raw_bytes.resize(len, 0);
            } else if required_bytes.is_some_and(|required| raw_bytes.len() < required) {
                panic!("MockFrame 数据长度不足以覆盖声明的 plane layout");
            }
        }

        let mut desc = AvFrameDesc::default_nv12(width, height, stride[0], stride[1], timestamp_ns);
        desc.pixel_format = pixel_format;
        desc.stride = stride;
        desc.opaque_kind = opaque_kind;
        desc.plane_count = match pixel_format {
            AV_PIX_NV12 => 2,
            AV_PIX_I420 => 3,
            _ => 1,
        };
        desc.offset = if pixel_format == AV_PIX_NV12 || pixel_format == AV_PIX_I420 {
            offset
        } else {
            [offset[0], 0, 0, 0]
        };

        if let Some(storage) = hardware_storage {
            match &storage {
                MockStorage::Host(bytes) => {
                    desc.opaque = bytes.as_ptr() as *mut c_void;
                    desc.memory_type = AV_MEM_HOST;
                    desc.opaque_kind = AV_OPAQUE_NONE;
                }
                #[cfg(target_os = "macos")]
                MockStorage::ApplePixelBuffer(ptr) => {
                    desc.opaque = *ptr;
                    desc.frame_token = *ptr;
                    desc.memory_type = AV_MEM_PLATFORM_SURFACE;
                    desc.layout = AV_LAYOUT_PLATFORM_NATIVE;
                }
                #[cfg(all(target_os = "linux", feature = "testing-hardware"))]
                MockStorage::DmaBuf(fd) => {
                    use std::os::fd::AsRawFd;
                    desc.opaque = fd.as_raw_fd() as usize as *mut c_void;
                    desc.memory_type = AV_MEM_PLATFORM_SURFACE;
                    desc.layout = AV_LAYOUT_PLATFORM_NATIVE;
                }
            }
            return MockFrame {
                desc,
                _storage: storage,
            };
        }

        #[cfg(target_os = "macos")]
        if opaque_kind == AV_OPAQUE_CVPIXELBUFFER {
            let mut raw_buf: *mut c_void = std::ptr::null_mut();
            // SAFETY: 创建 CVPixelBuffer 显存，输出指针由后续 MockStorage 接管。
            let status = unsafe {
                CVPixelBufferCreate(
                    std::ptr::null(),
                    width as usize,
                    height as usize,
                    K_CVPIXEL_FORMAT_NV12,
                    std::ptr::null(),
                    &mut raw_buf,
                )
            };
            if status == 0 && !raw_buf.is_null() {
                // SAFETY: 成功创建后锁定的各平面覆盖整个 CVPixelBuffer；拷贝长度按目标
                // 平面 stride 限定，不读越过 builder 持有的 raw_bytes。
                let lock_status = unsafe { CVPixelBufferLockBaseAddress(raw_buf, 0) };
                if lock_status == 0 {
                    // SAFETY: lock 成功后各 CoreVideo 平面在本次作用域内有效，且所有拷贝
                    // 起止位置都按源 raw_bytes 和目标 stride 检查。
                    unsafe {
                        let y_ptr = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 0) as *mut u8;
                        let uv_ptr = CVPixelBufferGetBaseAddressOfPlane(raw_buf, 1) as *mut u8;
                        let ys = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 0);
                        let uvs = CVPixelBufferGetBytesPerRowOfPlane(raw_buf, 1);
                        let src_stride0 = stride[0].max(width as i32) as usize;
                        let src_stride1 = stride[1].max(width as i32) as usize;
                        let uv_base = src_stride0.saturating_mul(height as usize);
                        let y_bytes = width as usize;
                        let uv_bytes = (width as usize).div_ceil(2) * 2;

                        if !raw_bytes.is_empty()
                            && !y_ptr.is_null()
                            && !uv_ptr.is_null()
                            && raw_bytes.len() >= uv_base
                        {
                            for y in 0..height as usize {
                                let src_start = y.saturating_mul(src_stride0);
                                if src_start
                                    .checked_add(y_bytes)
                                    .is_some_and(|end| end <= raw_bytes.len())
                                {
                                    std::ptr::copy_nonoverlapping(
                                        raw_bytes.as_ptr().add(src_start),
                                        y_ptr.add(y.saturating_mul(ys)),
                                        y_bytes,
                                    );
                                }
                            }
                            for y in 0..height.div_ceil(2) as usize {
                                let src_start =
                                    uv_base.saturating_add(y.saturating_mul(src_stride1));
                                if src_start
                                    .checked_add(uv_bytes)
                                    .is_some_and(|end| end <= raw_bytes.len())
                                {
                                    std::ptr::copy_nonoverlapping(
                                        raw_bytes.as_ptr().add(src_start),
                                        uv_ptr.add(y.saturating_mul(uvs)),
                                        uv_bytes,
                                    );
                                }
                            }
                        }
                        CVPixelBufferUnlockBaseAddress(raw_buf, 0);
                    }
                }
                desc.opaque = raw_buf;
                desc.frame_token = raw_buf;
                desc.memory_type = AV_MEM_PLATFORM_SURFACE;
                desc.layout = AV_LAYOUT_PLATFORM_NATIVE;
                return MockFrame {
                    desc,
                    _storage: MockStorage::ApplePixelBuffer(raw_buf),
                };
            }
        }

        desc.opaque = raw_bytes.as_ptr() as *mut c_void;
        let storage = MockStorage::Host(raw_bytes);
        desc.opaque_kind = AV_OPAQUE_NONE;
        desc.memory_type = AV_MEM_HOST;

        MockFrame {
            desc,
            _storage: storage,
        }
    }
}

/// 模拟结果捕获器，替代 `ResultEmitter` 捕获算法发射的检测结果
#[derive(Debug, Default)]
pub struct MockEmitter {
    detections: Vec<NormBox>,
    raw_json_events: Vec<String>,
    self_test_passed: bool,
}

impl MockEmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn detections(&self) -> &[NormBox] {
        &self.detections
    }

    pub fn raw_json_events(&self) -> &[String] {
        &self.raw_json_events
    }

    pub fn is_self_test_passed(&self) -> bool {
        self.self_test_passed
    }

    /// 记录一次 C 回调结果
    pub fn record_c_result(&mut self, result: &AvAlgoResult) {
        if result.kind == AV_RESULT_SELF_TEST {
            self.self_test_passed = true;
        }

        if !result.json.is_null() {
            // SAFETY: result.json 指向合法的 C 字符串
            let cstr = unsafe { CStr::from_ptr(result.json) };
            if let Ok(json_str) = cstr.to_str() {
                self.raw_json_events.push(json_str.to_string());

                // 解析 objects
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(objs) = val.get("objects").and_then(|v| v.as_array()) {
                        for o in objs {
                            let class_id =
                                o.get("class_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let confidence =
                                o.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                            let label: Option<&'static str> = o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);
                            let coords = o.get("bbox").and_then(|v| v.as_array());
                            if let Some(c) = coords {
                                if c.len() == 4 {
                                    let x = c[0].as_f64().unwrap_or(0.0) as f32;
                                    let y = c[1].as_f64().unwrap_or(0.0) as f32;
                                    let w = c[2].as_f64().unwrap_or(0.0) as f32;
                                    let h = c[3].as_f64().unwrap_or(0.0) as f32;
                                    let mut b = NormBox::new(x, y, w, h, confidence, class_id);
                                    b.label = label;
                                    self.detections.push(b);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 模拟纯 CPU 推理桩
#[derive(Debug, Default)]
pub struct MockWeights;

impl ModelWeights for MockWeights {
    type Session = MockSession;
    fn session_on(&self, core: Core) -> Result<Self::Session, AlgoError> {
        Ok(MockSession { core })
    }
}

#[derive(Debug)]
pub struct MockSession {
    pub core: Core,
}

impl InferenceSession for MockSession {
    type Output = Vec<NormBox>;
    fn infer(&mut self, _input: &CvBuffer) -> Result<Self::Output, AlgoError> {
        Ok(vec![
            NormBox::new(0.2, 0.2, 0.3, 0.3, 0.95, 0).with_label("mock_object")
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_frame_builder_nv12() {
        let builder = MockFrameBuilder::new().dimensions(1920, 1080).to_nv12(16);

        let frame = builder.build();
        let safe = frame.as_safe_frame();

        assert_eq!(safe.width(), 1920);
        assert_eq!(safe.height(), 1080);
        assert_eq!(safe.pixel_format(), AV_PIX_NV12);
        assert_eq!(safe.stride(0), 1920);
        assert_eq!(safe.stride(1), 1920);

        match safe.handle_view() {
            crate::frame::FrameHandleView::Host { data } => {
                assert_eq!(data.len(), 1920 * 1080 * 3 / 2);
            }
            _ => panic!("预期 Host NV12 句柄"),
        }
    }

    #[test]
    fn test_mock_frame_builder_updates_default_stride_and_preserves_odd_nv12() {
        let frame = MockFrameBuilder::new().dimensions(3, 3).to_nv12(4).build();
        let safe = frame.as_safe_frame();

        assert_eq!(safe.width(), 3);
        assert_eq!(safe.height(), 3);
        assert_eq!(safe.stride(0), 4);
        assert_eq!(safe.stride(1), 4);
        match safe.handle_view() {
            crate::frame::FrameHandleView::Host { data } => {
                assert_eq!(data.len(), 4 * 3 + 4 * 2);
            }
            _ => panic!("预期 Host NV12 句柄"),
        }
    }

    #[test]
    fn test_mock_frame_builder_explicit_rgb_offset() {
        let frame = MockFrameBuilder::new()
            .dimensions(2, 2)
            .plane_offsets(4, 0, 0)
            .host_data(vec![0u8; 16])
            .build();
        let safe = frame.as_safe_frame();
        assert_eq!(safe.plane_offset(0), 4);
        match safe.handle_view() {
            crate::frame::FrameHandleView::Host { data } => assert_eq!(data.len(), 16),
            _ => panic!("预期 Host 句柄"),
        }
    }
}
