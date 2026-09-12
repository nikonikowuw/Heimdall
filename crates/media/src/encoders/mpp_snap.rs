//! Rockchip MPP JPEGE + RGA 设备侧裁剪快照编码器（真实 FFI 实现）
//!
//! 全链路硬件加速快照路径（`snapshot_readback_path`）：
//! - 全景：DMA-BUF → MPP VPU JPEG 编码 → bitstream → D2H → 写盘
//! - 特写：DMA-BUF → RGA crop+pad → Scratchpad DMA-BUF → MPP JPEGE → bitstream → 写盘
//!
//! 所有步骤通过 tracing 日志可观测，真机上 `RUST_LOG=media=debug` 即可跟踪。

#![cfg(all(target_os = "linux", feature = "hw-snap-mpp"))]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use tracing::{debug, info, warn};

use crate::dmabuf_sync::wait_dmabuf_readable;
use crate::encoders::{compute_crop_roi, DeviceSnapEncoder};
use crate::error::MediaError;
use crate::rga_crop::{RgaCropJob, RgaRuntime};
use types::{BoundingBox, FrameHandle, FrameRef, PixelFormat};

// ============================================================================
// MPP 编码器 FFI（严格对齐 Rockchip libmpp C ABI 布局）
// ============================================================================

#[allow(non_camel_case_types, dead_code)]
pub(crate) mod ffi {
    use super::*;

    pub const CALLER_TAG: &[u8] = b"heimdall-mpp-snap\0";
    pub const MPP_CTX_ENC: c_int = 1;
    pub const MPP_VIDEO_CODING_MJPEG: c_int = 8;

    // 编码器控制命令字
    pub const CMD_MODULE_CODEC: c_int = 0x00300000;
    pub const CMD_CTX_ID_ENC: c_int = 0x00020000;
    pub const MPP_ENC_CMD_BASE: c_int = CMD_MODULE_CODEC | CMD_CTX_ID_ENC;
    pub const MPP_ENC_SET_CFG: c_int = MPP_ENC_CMD_BASE + 1;

    // 色彩范围
    pub const MPP_FRAME_RANGE_JPEG: c_int = 2;
    pub const MPP_FMT_YUV420SP_NV12: c_int = 0;

    // 缓冲区类型与模式
    pub const MPP_BUFFER_TYPE_EXT_DMA: c_int = 2;
    pub const MPP_BUFFER_TYPE_DRM: c_int = 3;
    pub const MPP_BUFFER_EXTERNAL: c_int = 1;

    // MPP 异步输出端口与有界轮询超时
    pub const MPP_PORT_OUTPUT: c_int = 1;
    pub const MPP_POLL_TIMEOUT_MS: c_int = 100;

    // 返回码
    pub const MPP_OK: c_int = 0;

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct MppBufferInfo {
        pub type_: c_int,
        pub size: usize,
        pub ptr: *mut c_void,
        pub hnd: *mut c_void,
        pub fd: c_int,
        pub index: c_int,
    }

    // MppApi 虚表（严格对齐 Rockchip mpp.h 布局：184 bytes，8字节对齐）
    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct MppApi {
        pub size: c_uint,
        pub version: c_uint,
        pub decode: Option<
            unsafe extern "C" fn(
                ctx: *mut c_void,
                packet: *mut c_void,
                frame: *mut *mut c_void,
            ) -> c_int,
        >,
        pub decode_put_packet:
            Option<unsafe extern "C" fn(ctx: *mut c_void, packet: *mut c_void) -> c_int>,
        pub decode_get_frame:
            Option<unsafe extern "C" fn(ctx: *mut c_void, frame: *mut *mut c_void) -> c_int>,
        pub encode: Option<
            unsafe extern "C" fn(
                ctx: *mut c_void,
                frame: *mut c_void,
                packet: *mut *mut c_void,
            ) -> c_int,
        >,
        pub encode_put_frame:
            Option<unsafe extern "C" fn(ctx: *mut c_void, frame: *mut c_void) -> c_int>,
        pub encode_get_packet:
            Option<unsafe extern "C" fn(ctx: *mut c_void, packet: *mut *mut c_void) -> c_int>,
        pub isp: Option<
            unsafe extern "C" fn(ctx: *mut c_void, dst: *mut c_void, src: *mut c_void) -> c_int,
        >,
        pub isp_put_frame:
            Option<unsafe extern "C" fn(ctx: *mut c_void, frame: *mut c_void) -> c_int>,
        pub isp_get_frame:
            Option<unsafe extern "C" fn(ctx: *mut c_void, frame: *mut *mut c_void) -> c_int>,
        pub poll: Option<
            unsafe extern "C" fn(ctx: *mut c_void, port_type: c_int, timeout: c_int) -> c_int,
        >,
        pub dequeue: Option<
            unsafe extern "C" fn(
                ctx: *mut c_void,
                port_type: c_int,
                task: *mut *mut c_void,
            ) -> c_int,
        >,
        pub enqueue: Option<
            unsafe extern "C" fn(ctx: *mut c_void, port_type: c_int, task: *mut c_void) -> c_int,
        >,
        pub reset: Option<unsafe extern "C" fn(ctx: *mut c_void) -> c_int>,
        pub control:
            Option<unsafe extern "C" fn(ctx: *mut c_void, cmd: c_int, param: *mut c_void) -> c_int>,
        pub reserv: [u32; 16],
    }

    extern "C" {
        pub fn mpp_create(ctx: *mut *mut c_void, mpi: *mut *mut MppApi) -> c_int;
        pub fn mpp_init(ctx: *mut c_void, ctx_type: c_int, coding: c_int) -> c_int;
        pub fn mpp_destroy(ctx: *mut c_void) -> c_int;
        pub fn mpp_control(ctx: *mut c_void, cmd: c_int, param: *mut c_void) -> c_int;

        pub fn mpp_enc_cfg_init(cfg: *mut *mut c_void) -> c_int;
        pub fn mpp_enc_cfg_deinit(cfg: *mut c_void) -> c_int;
        pub fn mpp_enc_cfg_set_s32(cfg: *mut c_void, name: *const c_char, val: c_int) -> c_int;

        pub fn mpp_frame_init(frame: *mut *mut c_void) -> c_int;
        pub fn mpp_frame_deinit(frame: *mut *mut c_void) -> c_int;
        pub fn mpp_frame_set_width(f: *mut c_void, w: c_uint);
        pub fn mpp_frame_set_height(f: *mut c_void, h: c_uint);
        pub fn mpp_frame_set_hor_stride(f: *mut c_void, s: c_uint);
        pub fn mpp_frame_set_ver_stride(f: *mut c_void, s: c_uint);
        pub fn mpp_frame_set_fmt(f: *mut c_void, fmt: c_int);
        pub fn mpp_frame_set_color_range(f: *mut c_void, range: c_int);
        pub fn mpp_frame_set_buffer(f: *mut c_void, buf: *mut c_void);
        pub fn mpp_frame_set_eos(f: *mut c_void, eos: c_int);

        pub fn mpp_packet_init(pkt: *mut *mut c_void, ptr: *mut c_void, size: usize) -> c_int;
        pub fn mpp_packet_deinit(pkt: *mut *mut c_void) -> c_int;
        pub fn mpp_packet_get_data(pkt: *mut c_void) -> *mut c_void;
        pub fn mpp_packet_get_length(pkt: *mut c_void) -> usize;

        pub fn mpp_buffer_group_get(
            grp: *mut *mut c_void,
            type_: c_int,
            mode: c_int,
            tag: *const c_char,
            caller: *const c_char,
        ) -> c_int;
        pub fn mpp_buffer_group_put(grp: *mut c_void) -> c_int;
        pub fn mpp_buffer_import_with_tag(
            grp: *mut c_void,
            info: *mut MppBufferInfo,
            buffer: *mut *mut c_void,
            tag: *const c_char,
            caller: *const c_char,
        ) -> c_int;
        pub fn mpp_buffer_put_with_caller(buf: *mut c_void, caller: *const c_char) -> c_int;
    }
}

// ============================================================================
// RAII 守卫
// ============================================================================

struct MppEncCtx {
    ctx: *mut c_void,
    mpi: *mut ffi::MppApi,
}

impl Drop for MppEncCtx {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            info!(ctx = ?self.ctx, "MPP 编码上下文 RAII 析构");
            // SAFETY: self.ctx 经非空校验且为 mpp_create 分配的合法指针
            unsafe {
                ffi::mpp_destroy(self.ctx);
            }
        }
    }
}

struct MppBufGroup(*mut c_void);

impl Drop for MppBufGroup {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 为合法有效的 mpp_buffer_group 句柄
            unsafe {
                ffi::mpp_buffer_group_put(self.0);
            }
        }
    }
}

struct MppFrameGuard(*mut c_void);
impl Drop for MppFrameGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 经由 mpp_frame_init 分配，deinit 负责回收内部缓冲引用
            unsafe {
                ffi::mpp_frame_deinit(&mut self.0);
            }
        }
    }
}

struct MppPacketGuard(*mut c_void);
impl MppPacketGuard {
    fn data(&self) -> *mut u8 {
        if self.0.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: self.0 为合法 MppPacket 指针
            unsafe { ffi::mpp_packet_get_data(self.0) as *mut u8 }
        }
    }
    fn len(&self) -> usize {
        if self.0.is_null() {
            0
        } else {
            // SAFETY: self.0 为合法 MppPacket 指针
            unsafe { ffi::mpp_packet_get_length(self.0) }
        }
    }
}
impl Drop for MppPacketGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 经由 mpp_packet_init 分配
            unsafe {
                ffi::mpp_packet_deinit(&mut self.0);
            }
        }
    }
}

struct MppBufImportGuard(*mut c_void);
impl Drop for MppBufImportGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 为成功 import 的 MppBuffer，必须匹配归还
            unsafe {
                ffi::mpp_buffer_put_with_caller(self.0, ffi::CALLER_TAG.as_ptr() as *const _);
            }
        }
    }
}

// ============================================================================
// Scratchpad DMA-BUF 分配
// ============================================================================

const DMA_HEAP_PATHS: [&[u8]; 2] = [b"/dev/dma_heap/system-dma32\0", b"/dev/dma_heap/system\0"];

#[repr(C)]
struct DmaHeapAlloc {
    len: u64,
    fd: c_int,
    fd_flags: u32,
    heap_flags: u64,
}

const DMA_HEAP_IOCTL_ALLOC: libc::c_ulong = 0xc018_4800;

fn align_up(value: u32, alignment: u32) -> Result<u32, MediaError> {
    if alignment == 0 {
        return Err(MediaError::Encode {
            reason: "DMA 对齐值不能为 0".into(),
        });
    }
    value
        .checked_add(alignment - 1)
        .map(|v| v / alignment * alignment)
        .ok_or_else(|| MediaError::Encode {
            reason: "DMA 尺寸对齐溢出".into(),
        })
}

fn nv12_size(hor_stride: u32, ver_stride: u32) -> Result<usize, MediaError> {
    (hor_stride as usize)
        .checked_mul(ver_stride as usize)
        .and_then(|v| v.checked_mul(3))
        .and_then(|v| v.checked_div(2))
        .ok_or_else(|| MediaError::Encode {
            reason: "NV12 DMA-BUF 大小计算溢出".into(),
        })
}

fn alloc_dma_buf(size: usize) -> Result<OwnedFd, MediaError> {
    let page_size = 4096usize;
    let alloc_len = size
        .max(1)
        .checked_add(page_size - 1)
        .map(|v| v / page_size * page_size)
        .ok_or_else(|| MediaError::Encode {
            reason: "DMA-BUF 页面对齐溢出".into(),
        })?;

    let mut last_error = String::new();
    for heap_path in DMA_HEAP_PATHS {
        // SAFETY: 系统调用打开标准 Linux DMA 堆字符设备。
        let heap_fd = unsafe {
            libc::open(
                heap_path.as_ptr() as *const _,
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if heap_fd < 0 {
            last_error = std::io::Error::last_os_error().to_string();
            continue;
        }

        let mut alloc = DmaHeapAlloc {
            len: alloc_len as u64,
            fd: -1,
            fd_flags: (libc::O_RDWR | libc::O_CLOEXEC) as u32,
            heap_flags: 0,
        };

        // SAFETY: ioctl DMA_HEAP_IOCTL_ALLOC 遵循 Linux dma-heap UAPI。
        let ret = unsafe { libc::ioctl(heap_fd, DMA_HEAP_IOCTL_ALLOC, &mut alloc) };
        // SAFETY: 关闭临时打开的堆文件描述符。
        unsafe {
            libc::close(heap_fd);
        }

        if ret == 0 && alloc.fd >= 0 {
            // SAFETY: alloc.fd 为系统内核分配的有效文件描述符。
            return Ok(unsafe { OwnedFd::from_raw_fd(alloc.fd) });
        }
        last_error = std::io::Error::last_os_error().to_string();
    }

    Err(MediaError::Encode {
        reason: format!(
            "DMA-BUF 分配失败 ({} bytes)，已尝试 DMA32 与 system heap: {}",
            alloc_len, last_error
        ),
    })
}

// ============================================================================
// MppSnapEncoder
// ============================================================================

/// Rockchip MPP 全链路快照编码器
///
/// 持有 MPP 编码上下文 + buffer_group + RGA 运行时 + Scratchpad DMA-BUF。
/// 由 SnapshotEngine 的固定专用 OS Worker 单线程持有与调用，避免跨线程共享硬件 context。
pub struct MppSnapEncoder {
    enc: MppEncCtx,
    buf_group: MppBufGroup,
    rga: RgaRuntime,
    /// 常驻复用的 4K scratchpad，避免每次裁剪重新分配 CMA。
    scratchpad_fd: OwnedFd,
    /// scratchpad 在 RGA 中的常驻导入句柄。
    scratchpad_rga_handle: u32,
    scratchpad_size: usize,
    /// MPP reset 失败后永久关闭硬件路径，避免反复提交到未知状态的 context。
    ready: AtomicBool,
    /// 当前编码器绑定的分辨率（动态重配时更新）
    bound_w: AtomicU32,
    bound_h: AtomicU32,
    bound_hor_stride: AtomicU32,
    bound_ver_stride: AtomicU32,
    /// 当前编码器生效的 JPEG 质量
    bound_quality: AtomicU8,
}

impl std::fmt::Debug for MppSnapEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MppSnapEncoder")
            .field(
                "bound",
                &format!(
                    "{}x{}",
                    self.bound_w.load(Ordering::Relaxed),
                    self.bound_h.load(Ordering::Relaxed)
                ),
            )
            .field(
                "scratchpad",
                &format!("{}MB", self.scratchpad_size / 1024 / 1024),
            )
            .finish()
    }
}

impl MppSnapEncoder {
    /// 单画板 scratchpad 最大分辨率，覆盖 UHD/4K 快照特写。
    const SCRATCHPAD_MAX_WIDTH: u32 = 4096;
    const SCRATCHPAD_MAX_HEIGHT: u32 = 2160;

    pub fn try_new(quality: u8) -> Result<Self, MediaError> {
        let q = quality.clamp(1, 100);
        info!(quality = q, "初始化 MPP JPEG 编码器 + RGA 裁剪运行时...");

        // 1. 加载 RGA。动态库句柄由 RgaRuntime 持有并在析构时释放。
        let rga = RgaRuntime::try_load()?;
        debug!("RGA 运行时加载成功");

        // 2. 创建并初始化 MJPEG 编码上下文。
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mpi: *mut ffi::MppApi = std::ptr::null_mut();
        // SAFETY: 传入有效指针地址接收 libmpp 上下文与虚表。
        let ret = unsafe { ffi::mpp_create(&mut ctx, &mut mpi) };
        if ret != 0 || ctx.is_null() || mpi.is_null() {
            return Err(MediaError::EncoderInit {
                codec: "MPP-JPEG".into(),
                reason: format!("mpp_create 失败: ret={ret}"),
            });
        }
        let enc = MppEncCtx { ctx, mpi };

        // SAFETY: ctx 已经过校验，第三个参数必须是 MPP_VIDEO_CodingMJPEG。
        let ret = unsafe { ffi::mpp_init(enc.ctx, ffi::MPP_CTX_ENC, ffi::MPP_VIDEO_CODING_MJPEG) };
        if ret != 0 {
            return Err(MediaError::EncoderInit {
                codec: "MPP-JPEG".into(),
                reason: format!("mpp_init MJPEG 失败: ret={ret}"),
            });
        }
        debug!("MPP MJPEG 编码上下文初始化成功");

        // 3. 创建外部 DMA-BUF buffer group。
        let mut bg: *mut c_void = std::ptr::null_mut();
        let tag = b"mpp-snap-enc\0";
        let caller = ffi::CALLER_TAG;
        // SAFETY: 请求外部 DMA-BUF buffer group，输出指针由 MPP 写入。
        let ret = unsafe {
            ffi::mpp_buffer_group_get(
                &mut bg,
                ffi::MPP_BUFFER_TYPE_EXT_DMA,
                ffi::MPP_BUFFER_EXTERNAL,
                tag.as_ptr() as *const _,
                caller.as_ptr() as *const _,
            )
        };
        if ret != 0 || bg.is_null() {
            return Err(MediaError::EncoderInit {
                codec: "MPP-JPEG".into(),
                reason: format!("mpp_buffer_group_get 失败: ret={ret}"),
            });
        }
        let buf_group = MppBufGroup(bg);
        debug!(ptr = ?bg, "MPP 外部 DMA-BUF buffer group 创建成功");

        // 4. 配置 JPEG 质量与 NV12 full-range 输入，任何 setter 失败都拒绝硬件实例。
        Self::configure_quality(enc.ctx, enc.mpi, q)?;
        Self::configure_color_range(enc.ctx, enc.mpi)?;

        // 5. 预分配固定 4K scratchpad，优先使用 DMA32 堆兼容 RGA2。
        let stride = align_up(Self::SCRATCHPAD_MAX_WIDTH, 16)?;
        let scratchpad_size = nv12_size(stride, Self::SCRATCHPAD_MAX_HEIGHT)?;
        let scratchpad_fd = alloc_dma_buf(scratchpad_size)?;
        let scratchpad_rga_handle = rga.import_buffer_fd(
            scratchpad_fd.as_raw_fd(),
            stride,
            Self::SCRATCHPAD_MAX_HEIGHT,
            crate::rga_crop::RK_FORMAT_YCbCr_420_SP,
        )?;
        info!(
            scratchpad_fd = scratchpad_fd.as_raw_fd(),
            scratchpad_size,
            max_width = Self::SCRATCHPAD_MAX_WIDTH,
            max_height = Self::SCRATCHPAD_MAX_HEIGHT,
            "Scratchpad DMA-BUF 分配成功"
        );

        Ok(Self {
            enc,
            buf_group,
            rga,
            scratchpad_fd,
            scratchpad_rga_handle,
            scratchpad_size,
            bound_w: AtomicU32::new(0),
            bound_h: AtomicU32::new(0),
            bound_hor_stride: AtomicU32::new(0),
            bound_ver_stride: AtomicU32::new(0),
            bound_quality: AtomicU8::new(q),
            ready: AtomicBool::new(true),
        })
    }

    fn apply_cfg(
        ctx: *mut c_void,
        mpi: *mut ffi::MppApi,
        values: &[(&[u8], c_int)],
    ) -> Result<(), MediaError> {
        let mut cfg: *mut c_void = std::ptr::null_mut();
        // SAFETY: cfg 是由 MPP 写入的不透明配置句柄输出参数。
        let init_ret = unsafe { ffi::mpp_enc_cfg_init(&mut cfg) };
        if init_ret != 0 || cfg.is_null() {
            return Err(MediaError::Encode {
                reason: format!("mpp_enc_cfg_init 失败: ret={init_ret}"),
            });
        }

        let result = (|| {
            for (name, value) in values {
                // SAFETY: cfg、NUL 结尾配置键和值均由当前线程持有。
                let ret =
                    unsafe { ffi::mpp_enc_cfg_set_s32(cfg, name.as_ptr() as *const _, *value) };
                if ret != 0 {
                    return Err(MediaError::Encode {
                        reason: format!(
                            "mpp_enc_cfg_set_s32 失败: key={}, ret={ret}",
                            String::from_utf8_lossy(name).trim_end_matches('\0')
                        ),
                    });
                }
            }

            // SAFETY: mpi 来自同一 MPP context，control 指针由 MPP 虚表提供。
            let control = unsafe { (*mpi).control };
            let control = control.ok_or_else(|| MediaError::Encode {
                reason: "MPP control 接口为空".into(),
            })?;
            // SAFETY: cfg 已初始化且 control 参数符合 MPP_ENC_SET_CFG 契约。
            let ret = unsafe { control(ctx, ffi::MPP_ENC_SET_CFG, cfg) };
            if ret != ffi::MPP_OK {
                return Err(MediaError::Encode {
                    reason: format!("MPP_ENC_SET_CFG 失败: ret={ret}"),
                });
            }
            Ok(())
        })();

        // SAFETY: cfg 已由 mpp_enc_cfg_init 成功创建，必须成对释放。
        let deinit_ret = unsafe { ffi::mpp_enc_cfg_deinit(cfg) };
        if deinit_ret != 0 {
            return Err(MediaError::Encode {
                reason: format!("mpp_enc_cfg_deinit 失败: ret={deinit_ret}"),
            });
        }
        result
    }

    fn configure_quality(ctx: *mut c_void, mpi: *mut ffi::MppApi, q: u8) -> Result<(), MediaError> {
        // 当前 BSP 的 q_factor 合法范围为 1..=99，API 的 100 映射到最高硬件质量。
        let q_factor = i32::from(q.clamp(1, 99));
        Self::apply_cfg(ctx, mpi, &[(b"jpeg:q_factor\0", q_factor)])?;
        debug!(quality = q, q_factor, "MPP JPEG 质量配置完成");
        Ok(())
    }

    fn configure_color_range(ctx: *mut c_void, mpi: *mut ffi::MppApi) -> Result<(), MediaError> {
        Self::apply_cfg(
            ctx,
            mpi,
            &[
                (b"prep:colorrange\0", ffi::MPP_FRAME_RANGE_JPEG),
                (b"prep:format\0", ffi::MPP_FMT_YUV420SP_NV12),
            ],
        )?;
        debug!("MPP 色彩范围: Full Range + NV12");
        Ok(())
    }

    fn reconfigure_dimensions(
        &self,
        w: u32,
        h: u32,
        hor_stride: u32,
        ver_stride: u32,
    ) -> Result<(), MediaError> {
        Self::apply_cfg(
            self.enc.ctx,
            self.enc.mpi,
            &[
                (b"prep:width\0", w as c_int),
                (b"prep:height\0", h as c_int),
                (b"prep:hor_stride\0", hor_stride as c_int),
                (b"prep:ver_stride\0", ver_stride as c_int),
                (b"prep:format\0", ffi::MPP_FMT_YUV420SP_NV12),
                (b"prep:colorrange\0", ffi::MPP_FRAME_RANGE_JPEG),
            ],
        )
    }

    fn reset_after_error(&self) {
        // MPP reset 会丢弃上下文内尚未完成的异步任务，避免一次 poll/get 失败污染下一次快照。
        // SAFETY: self.enc.mpi 在 MppEncCtx 初始化时经非空校验，且只在线程内使用。
        let reset_fn = unsafe { (*self.enc.mpi).reset };
        if let Some(reset_fn) = reset_fn {
            // SAFETY: reset_fn 来自当前专用线程持有的 MPP context 虚表。
            let ret = unsafe { reset_fn(self.enc.ctx) };
            if ret != ffi::MPP_OK {
                warn!(ret, "MPP 编码上下文 reset 失败，后续请求可能继续降级 CPU");
                self.ready.store(false, Ordering::Release);
            }
        } else {
            warn!("MPP 编码上下文缺少 reset 接口，错误状态无法主动清理");
            self.ready.store(false, Ordering::Release);
        }
        self.bound_w.store(0, Ordering::Relaxed);
        self.bound_h.store(0, Ordering::Relaxed);
        self.bound_hor_stride.store(0, Ordering::Relaxed);
        self.bound_ver_stride.store(0, Ordering::Relaxed);
        self.bound_quality.store(0, Ordering::Relaxed);
    }

    /// 核心编码路径的错误恢复包装：硬件提交、轮询或取包失败后清空异步队列。
    fn encode_dmabuf(
        &self,
        fd: RawFd,
        width: u32,
        height: u32,
        hor_stride: u32,
        ver_stride: u32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let result = self.encode_dmabuf_once(fd, width, height, hor_stride, ver_stride, quality);
        if result.is_err() {
            self.reset_after_error();
        }
        result
    }

    /// 核心编码路径：DMA-BUF fd → MPP JPEG → Vec<u8>
    fn encode_dmabuf_once(
        &self,
        fd: RawFd,
        width: u32,
        height: u32,
        hor_stride: u32,
        ver_stride: u32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let ctx = self.enc.ctx;
        let mpi = self.enc.mpi;
        let bg = self.buf_group.0;
        let start = std::time::Instant::now();

        if fd < 0
            || width == 0
            || height == 0
            || width > hor_stride
            || height > ver_stride
            || (hor_stride & 1) != 0
            || (ver_stride & 1) != 0
        {
            return Err(MediaError::Encode {
                reason: format!(
                    "非法 NV12 DMA-BUF 布局: fd={fd}, logical={width}x{height}, stride={hor_stride}x{ver_stride}"
                ),
            });
        }

        // Cache sync 不是设备完成栅障；先等待上游 VPU/RGA 的 dma_resv fence。
        wait_dmabuf_readable(fd, ffi::MPP_POLL_TIMEOUT_MS)?;

        // 0. 动态重配尺寸与真实物理 stride。
        let bound_w = self.bound_w.load(Ordering::Relaxed);
        let bound_h = self.bound_h.load(Ordering::Relaxed);
        let bound_hs = self.bound_hor_stride.load(Ordering::Relaxed);
        let bound_vs = self.bound_ver_stride.load(Ordering::Relaxed);
        if width != bound_w || height != bound_h || hor_stride != bound_hs || ver_stride != bound_vs
        {
            debug!(
                old = ?format!("{}x{} stride {}x{}", bound_w, bound_h, bound_hs, bound_vs),
                new = ?format!("{}x{} stride {}x{}", width, height, hor_stride, ver_stride),
                "编码器尺寸与 stride 重配"
            );
            self.reconfigure_dimensions(width, height, hor_stride, ver_stride)?;
            self.bound_w.store(width, Ordering::Relaxed);
            self.bound_h.store(height, Ordering::Relaxed);
            self.bound_hor_stride.store(hor_stride, Ordering::Relaxed);
            self.bound_ver_stride.store(ver_stride, Ordering::Relaxed);
        }

        // 动态重配质量；setter 失败必须让本次硬件编码失败并进入 CPU 保底。
        let quality = quality.clamp(1, 100);
        let bound_q = self.bound_quality.load(Ordering::Relaxed);
        if quality != bound_q {
            Self::configure_quality(ctx, mpi, quality)?;
            self.bound_quality.store(quality, Ordering::Relaxed);
        }

        let buf_size = nv12_size(hor_stride, ver_stride)?;
        let mut buffer_info = ffi::MppBufferInfo {
            type_: ffi::MPP_BUFFER_TYPE_EXT_DMA,
            size: buf_size,
            ptr: std::ptr::null_mut(),
            hnd: std::ptr::null_mut(),
            fd,
            index: 0,
        };
        let mut mpp_buf: *mut c_void = std::ptr::null_mut();
        // SAFETY: buffer_info 生命周期覆盖 import 调用；fd 由调用方 FrameRef/scratchpad 持有，
        // MPP 只创建对 DMA-BUF 的引用，不接管 fd 所有权。
        let ret = unsafe {
            ffi::mpp_buffer_import_with_tag(
                bg,
                &mut buffer_info,
                &mut mpp_buf,
                ffi::CALLER_TAG.as_ptr() as *const _,
                ffi::CALLER_TAG.as_ptr() as *const _,
            )
        };
        if ret != ffi::MPP_OK || mpp_buf.is_null() {
            return Err(MediaError::Encode {
                reason: format!(
                    "mpp_buffer_import_with_tag 失败: fd={fd}, size={buf_size}, ret={ret}"
                ),
            });
        }
        let _buf_guard = MppBufImportGuard(mpp_buf);
        debug!(
            elapsed_us = start.elapsed().as_micros(),
            "DMA-BUF → MppBuffer 导入完成"
        );

        // 1. 构造 MppFrame。
        let mut frame: *mut c_void = std::ptr::null_mut();
        // SAFETY: 初始化 MPP 帧结构体。
        let frame_ret = unsafe { ffi::mpp_frame_init(&mut frame) };
        if frame_ret != ffi::MPP_OK || frame.is_null() {
            return Err(MediaError::Encode {
                reason: format!("mpp_frame_init 失败: ret={frame_ret}"),
            });
        }
        let _fg = MppFrameGuard(frame);
        // SAFETY: frame 与 mpp_buf 均来自当前 MPP context，元数据布局遵循 mpp_frame API。
        unsafe {
            ffi::mpp_frame_set_width(frame, width);
            ffi::mpp_frame_set_height(frame, height);
            ffi::mpp_frame_set_hor_stride(frame, hor_stride);
            ffi::mpp_frame_set_ver_stride(frame, ver_stride);
            ffi::mpp_frame_set_fmt(frame, ffi::MPP_FMT_YUV420SP_NV12);
            ffi::mpp_frame_set_color_range(frame, ffi::MPP_FRAME_RANGE_JPEG);
            ffi::mpp_frame_set_buffer(frame, mpp_buf);
            ffi::mpp_frame_set_eos(frame, 0);
        }

        // 2. 提交异步编码任务。
        // SAFETY: mpi 经有效性校验，读取 MPP 虚表函数指针。
        let put_fn = unsafe { (*mpi).encode_put_frame }.ok_or_else(|| MediaError::Encode {
            reason: "encode_put_frame 为空".into(),
        })?;
        // SAFETY: frame 属于当前 MPP context，调用发生在专用编码线程。
        let ret = unsafe { put_fn(ctx, frame) };
        if ret != ffi::MPP_OK {
            return Err(MediaError::Encode {
                reason: format!("encode_put_frame 失败: ret={ret}"),
            });
        }

        // 3. MPP async API 要求先轮询 output port，再取 packet；禁止无界阻塞。
        // SAFETY: mpi 经有效性校验，读取 MPP poll 虚表函数指针。
        let poll_fn = unsafe { (*mpi).poll }.ok_or_else(|| MediaError::Encode {
            reason: "MPP poll 接口为空".into(),
        })?;
        // SAFETY: output port 与 100ms 有界超时符合 mpp_task.h 契约。
        let poll_ret = unsafe { poll_fn(ctx, ffi::MPP_PORT_OUTPUT, ffi::MPP_POLL_TIMEOUT_MS) };
        if poll_ret != ffi::MPP_OK {
            return Err(MediaError::Encode {
                reason: format!("MPP output poll 超时或失败: ret={poll_ret}"),
            });
        }

        // 4. 取出编码 packet。
        // SAFETY: mpi 经有效性校验，读取 MPP 虚表函数指针。
        let get_fn = unsafe { (*mpi).encode_get_packet }.ok_or_else(|| MediaError::Encode {
            reason: "encode_get_packet 为空".into(),
        })?;
        let mut pkt: *mut c_void = std::ptr::null_mut();
        // SAFETY: poll 已确认 output queue 可读，pkt 由 MPP 写入。
        let ret = unsafe { get_fn(ctx, &mut pkt) };
        if ret != ffi::MPP_OK || pkt.is_null() {
            return Err(MediaError::Encode {
                reason: format!("encode_get_packet 失败: ret={ret}"),
            });
        }
        let pkt_guard = MppPacketGuard(pkt);
        let jpeg_len = pkt_guard.len();

        let data = pkt_guard.data();
        if data.is_null() || jpeg_len == 0 {
            return Err(MediaError::Encode {
                reason: "编码输出为空".into(),
            });
        }
        // SAFETY: data 指向由 MppPacketGuard 保持有效的紧凑 JPEG bitstream。
        let jpeg = unsafe { std::slice::from_raw_parts(data, jpeg_len) }.to_vec();

        info!(
            jpeg_len,
            elapsed_us = start.elapsed().as_micros(),
            width,
            height,
            quality,
            "MPP JPEG 编码完成"
        );
        Ok(jpeg)
    }
}

impl Drop for MppSnapEncoder {
    fn drop(&mut self) {
        self.rga.release_buffer_handle(self.scratchpad_rga_handle);
    }
}

impl DeviceSnapEncoder for MppSnapEncoder {
    fn name(&self) -> &'static str {
        "mpp-snap"
    }
    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let (fd, w, h, hs, vs) = match frame.handle() {
            FrameHandle::DmaBuf { fd, .. } if frame.format == PixelFormat::Nv12 => (
                fd.as_raw_fd(),
                frame.width,
                frame.height,
                frame.stride.hor_stride,
                frame.stride.ver_stride,
            ),
            FrameHandle::DmaBuf { .. } => {
                return Err(MediaError::Encode {
                    reason: format!(
                        "MppSnapEncoder 仅支持 NV12 DMA-BUF, 实际格式: {:?}",
                        frame.format
                    ),
                })
            }
            _ => {
                return Err(MediaError::Encode {
                    reason: format!("MppSnapEncoder 需要 DMA-BUF 帧, 实际: {:?}", frame.handle()),
                })
            }
        };
        info!(cam = %frame.camera_id, fd, w, h, quality, "全景编码开始");
        let r = self.encode_dmabuf(fd, w, h, hs, vs, quality);
        if let Err(ref e) = r {
            warn!(cam = %frame.camera_id, error = %e, "全景编码失败");
        }
        r
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let (src_fd, src_w, src_h, src_hor_stride, src_ver_stride) = match frame.handle() {
            FrameHandle::DmaBuf { fd, .. } if frame.format == PixelFormat::Nv12 => (
                fd.as_raw_fd(),
                frame.width,
                frame.height,
                frame.stride.hor_stride,
                frame.stride.ver_stride,
            ),
            FrameHandle::DmaBuf { .. } => {
                return Err(MediaError::Encode {
                    reason: format!(
                        "MppSnapEncoder 仅支持 NV12 DMA-BUF, 实际格式: {:?}",
                        frame.format
                    ),
                })
            }
            _ => {
                return Err(MediaError::Encode {
                    reason: format!("MppSnapEncoder 需要 DMA-BUF 帧, 实际: {:?}", frame.handle()),
                })
            }
        };
        info!(cam = %frame.camera_id, src_fd, src_w, src_h, ?bbox, quality, "特写裁剪编码开始");

        // 1. 计算 ROI（严格遵循硬件下限与偶数对齐约束）
        let (sx, sy, crop_w, crop_h, w_stride) =
            compute_crop_roi(src_w, src_h, bbox, padding_ratio);
        debug!(sx, sy, crop_w, crop_h, w_stride, "ROI 计算完成");

        // 2. 严防 DMA 越界写：校验裁剪尺寸是否超出了预分配的 Scratchpad 单画板容量
        let needed_scratchpad_bytes = nv12_size(w_stride, crop_h)?;
        if needed_scratchpad_bytes > self.scratchpad_size
            || crop_w > Self::SCRATCHPAD_MAX_WIDTH
            || crop_h > Self::SCRATCHPAD_MAX_HEIGHT
        {
            return Err(MediaError::Encode {
                reason: format!(
                    "裁剪尺寸 ({}x{}, stride {}) 超过 Scratchpad 最大画板容量 ({} bytes)，触发 CPU 保底",
                    crop_w, crop_h, w_stride, self.scratchpad_size
                ),
            });
        }

        // 3. RGA crop: src DMA-BUF → scratchpad DMA-BUF
        let dst_fd = self.scratchpad_fd.as_raw_fd();
        let job = RgaCropJob {
            src_fd,
            src_w,
            src_h,
            src_hor_stride,
            src_ver_stride,
            sx,
            sy,
            crop_w,
            crop_h,
            dst_fd,
            dst_w: w_stride,
            dst_h: crop_h,
        };
        self.rga
            .crop_blit_sync(job, self.scratchpad_rga_handle)
            .map_err(|e| MediaError::Encode {
                reason: format!("RGA crop blit 失败: {e}"),
            })?;
        debug!("RGA 设备侧裁剪完成 → scratchpad");

        // 4. MPP 编码裁剪后帧（scratchpad 内部包含针对 dst_fd 的 Cache Sync 闭环）
        let r = self.encode_dmabuf(dst_fd, crop_w, crop_h, w_stride, crop_h, quality);
        if let Err(ref e) = r {
            warn!(cam = %frame.camera_id, error = %e, "特写编码失败");
        }
        r
    }
}

// ============================================================================
// 测试（含 ABI 断言，x86 可直接执行）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn test_mpp_encoder_command_values() {
        assert_eq!(ffi::MPP_ENC_CMD_BASE, 0x0032_0000);
        assert_eq!(ffi::MPP_ENC_SET_CFG, 0x0032_0001);
    }

    #[test]
    fn test_mpp_buffer_info_abi_layout_assertion() {
        assert_eq!(size_of::<ffi::MppBufferInfo>(), 40);
        assert_eq!(align_of::<ffi::MppBufferInfo>(), 8);
        assert_eq!(offset_of!(ffi::MppBufferInfo, type_), 0);
        assert_eq!(offset_of!(ffi::MppBufferInfo, size), 8);
        assert_eq!(offset_of!(ffi::MppBufferInfo, ptr), 16);
        assert_eq!(offset_of!(ffi::MppBufferInfo, hnd), 24);
        assert_eq!(offset_of!(ffi::MppBufferInfo, fd), 32);
        assert_eq!(offset_of!(ffi::MppBufferInfo, index), 36);
    }

    #[test]
    fn test_mpp_api_abi_layout_assertion() {
        assert_eq!(size_of::<ffi::MppApi>(), 184);
        assert_eq!(align_of::<ffi::MppApi>(), 8);
        assert_eq!(offset_of!(ffi::MppApi, size), 0);
        assert_eq!(offset_of!(ffi::MppApi, version), 4);
        assert_eq!(offset_of!(ffi::MppApi, decode), 8);
        assert_eq!(offset_of!(ffi::MppApi, decode_put_packet), 16);
        assert_eq!(offset_of!(ffi::MppApi, decode_get_frame), 24);
        assert_eq!(offset_of!(ffi::MppApi, encode), 32);
        assert_eq!(offset_of!(ffi::MppApi, encode_put_frame), 40);
        assert_eq!(offset_of!(ffi::MppApi, encode_get_packet), 48);
        assert_eq!(offset_of!(ffi::MppApi, reset), 104);
        assert_eq!(offset_of!(ffi::MppApi, control), 112);
    }
}
