//! Rockchip MPP (Media Process Platform) 硬件解码器实现
//!
//! 适配 RK3588 / RK3568 / RK3576 等 SoC 的硬件 VPU 解码引擎。
//! 输入 H.264 / H.265 Annex B 裸流包，通过专用 OS 线程与有界命令通道驱动 MPP 同步 C API，
//! 输出带生命周期租约的 DRM DMA-BUF 文件描述符，全链路零 CPU 内存拷贝直通 RGA 与 RKNN。

#![cfg(all(target_os = "linux", feature = "mpp"))]

use std::ffi::c_void;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;
use std::thread::JoinHandle;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};
use types::{CodecType, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::decoder::{DecodeCommand, VideoDecoder};
use crate::error::MediaError;

/// 水平步长对齐（Rockchip MPP 要求水平 16 字节对齐）
#[inline]
pub fn align_hor_stride(width: u32) -> u32 {
    (width + 15) & !15
}

/// 垂直步长对齐（Rockchip MPP 要求垂直 16 字节虚高对齐，如 1080P 对齐至 1088）
#[inline]
pub fn align_ver_stride(height: u32) -> u32 {
    (height + 15) & !15
}

pub(crate) mod ffi {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_uint};

    pub const MPP_CTX_DEC: c_int = 0;
    pub const MPP_VIDEO_CODING_AVC: c_int = 7;
    pub const MPP_VIDEO_CODING_HEVC: c_int = 16777220;

    pub const MPP_DEC_SET_PARSER_SPLIT_MODE: c_int = 0x00010001;
    pub const MPP_DEC_SET_FRAME_INFO: c_int = 0x00010002;
    pub const MPP_DEC_SET_FRAME_BUFFER_COUNT: c_int = 0x00010004;
    pub const MPP_DEC_SET_INFO_CHANGE_READY: c_int = 0x00010005;
    pub const MPP_DEC_SET_OUTPUT_FORMAT: c_int = 0x00010006;

    pub const MPP_FMT_YUV420SP: c_int = 0;
    pub const MPP_FMT_YUV420SP_10BIT: c_int = 2;

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
        pub reset: Option<unsafe extern "C" fn(ctx: *mut c_void) -> c_int>,
        pub control:
            Option<unsafe extern "C" fn(ctx: *mut c_void, cmd: c_int, param: *mut c_void) -> c_int>,
    }

    extern "C" {
        pub fn mpp_create(ctx: *mut *mut c_void, mpi: *mut *mut MppApi) -> c_int;
        pub fn mpp_init(ctx: *mut c_void, ctx_type: c_int, coding: c_int) -> c_int;
        pub fn mpp_destroy(ctx: *mut c_void) -> c_int;

        pub fn mpp_packet_init(packet: *mut *mut c_void, ptr: *mut c_void, size: usize) -> c_int;
        pub fn mpp_packet_deinit(packet: *mut *mut c_void) -> c_int;
        pub fn mpp_packet_set_pts(packet: *mut c_void, pts: i64);
        pub fn mpp_packet_set_eos(packet: *mut c_void) -> c_int;

        pub fn mpp_frame_init(frame: *mut *mut c_void) -> c_int;
        pub fn mpp_frame_deinit(frame: *mut *mut c_void) -> c_int;
        pub fn mpp_frame_get_width(frame: *const c_void) -> c_uint;
        pub fn mpp_frame_get_height(frame: *const c_void) -> c_uint;
        pub fn mpp_frame_get_hor_stride(frame: *const c_void) -> c_uint;
        pub fn mpp_frame_get_ver_stride(frame: *const c_void) -> c_uint;
        pub fn mpp_frame_get_errinfo(frame: *const c_void) -> c_int;
        pub fn mpp_frame_get_discard(frame: *const c_void) -> c_int;
        pub fn mpp_frame_get_eos(frame: *const c_void) -> c_int;
        pub fn mpp_frame_get_fmt(frame: *const c_void) -> c_int;
        pub fn mpp_frame_get_info_change(frame: *const c_void) -> c_int;
        pub fn mpp_frame_get_pts(frame: *const c_void) -> i64;
        pub fn mpp_frame_get_buffer(frame: *const c_void) -> *mut c_void;

        pub fn mpp_buffer_inc_ref(buffer: *mut c_void) -> c_int;
        pub fn mpp_buffer_put(buffer: *mut c_void) -> c_int;
        pub fn mpp_buffer_get_fd(buffer: *mut c_void) -> c_int;
    }
}

/// MPP 缓冲区租约封装
///
/// 在帧生命周期终结（所有引用的 FrameHandle 均析构）时，调用 `mpp_buffer_put` 释放引用，
/// 允许底层 CMA 物理页回到 MPP 内部缓冲区池进行下一轮复用。
struct MppBufferLease {
    buf: *mut c_void,
}

// SAFETY: MppBuffer 内部具有线程安全的引用计数机制（MPP 内部加锁互斥），
// Lease 仅在 Drop 时调用 mpp_buffer_put 减计数，因此满足跨线程传递与共享。
unsafe impl Send for MppBufferLease {}
unsafe impl Sync for MppBufferLease {}

impl Drop for MppBufferLease {
    fn drop(&mut self) {
        if !self.buf.is_null() {
            // SAFETY: self.buf 在收帧时经过 mpp_buffer_inc_ref 增持，此处正常释放该引用
            unsafe {
                ffi::mpp_buffer_put(self.buf);
            }
        }
    }
}

/// 专用解码线程独占的 MPP 状态上下文
struct MppDecoderInner {
    camera_id: String,
    codec: CodecType,
    ctx: *mut c_void,
    mpi: *mut ffi::MppApi,
    width: u32,
    height: u32,
    hor_stride: u32,
    ver_stride: u32,
    is_initialized: bool,
}

impl MppDecoderInner {
    fn new(camera_id: String, codec: CodecType) -> Self {
        Self {
            camera_id,
            codec,
            ctx: std::ptr::null_mut(),
            mpi: std::ptr::null_mut(),
            width: 0,
            height: 0,
            hor_stride: 0,
            ver_stride: 0,
            is_initialized: false,
        }
    }

    fn init(&mut self) -> Result<(), MediaError> {
        let coding_type = match self.codec {
            CodecType::H264 => ffi::MPP_VIDEO_CODING_AVC,
            CodecType::H265 => ffi::MPP_VIDEO_CODING_HEVC,
        };

        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mpi: *mut ffi::MppApi = std::ptr::null_mut();

        // SAFETY: mpp_create 分配 MPP 上下文句柄与函数指针虚表
        let ret = unsafe { ffi::mpp_create(&mut ctx, &mut mpi) };
        if ret != 0 || ctx.is_null() || mpi.is_null() {
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("mpp_create 初始化失败, 返回码: {ret}"),
            });
        }

        // SAFETY: ctx 为 mpp_create 分配的合法指针
        let ret = unsafe { ffi::mpp_init(ctx, ffi::MPP_CTX_DEC, coding_type) };
        if ret != 0 {
            // SAFETY: 失败清理已创建的上下文
            unsafe {
                ffi::mpp_destroy(ctx);
            }
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("mpp_init 解码模式设置失败, 返回码: {ret}"),
            });
        }

        // 开启 MPP 内部的流切分模式（由硬件解复用 Annex B NALU）
        let mut split_mode: std::os::raw::c_uint = 1;
        // SAFETY: control 命令参数合法，指针指向有效局部变量
        unsafe {
            if let Some(ctrl_fn) = (*mpi).control {
                ctrl_fn(
                    ctx,
                    ffi::MPP_DEC_SET_PARSER_SPLIT_MODE,
                    &mut split_mode as *mut _ as *mut c_void,
                );
            }
        }

        self.ctx = ctx;
        self.mpi = mpi;
        self.is_initialized = true;
        debug!(camera_id = %self.camera_id, codec = ?self.codec, "Rockchip MPP 硬件解码器就绪");
        Ok(())
    }

    fn decode(&mut self, packet_data: &[u8], pts: i64) -> Result<Option<FrameRef>, MediaError> {
        if !self.is_initialized {
            return Err(MediaError::Decode {
                reason: "MPP 解码器未初始化".to_string(),
            });
        }

        let mut packet: *mut c_void = std::ptr::null_mut();
        // SAFETY: 基于原始数据切片初始化 MppPacket 容器
        let ret = unsafe {
            ffi::mpp_packet_init(
                &mut packet,
                packet_data.as_ptr() as *mut c_void,
                packet_data.len(),
            )
        };
        if ret != 0 || packet.is_null() {
            return Err(MediaError::Decode {
                reason: format!("mpp_packet_init 失败, 返回码: {ret}"),
            });
        }

        // SAFETY: packet 句柄有效，注入毫秒级 PTS
        unsafe {
            ffi::mpp_packet_set_pts(packet, pts);
        }

        let put_fn =
            unsafe { (*self.mpi).decode_put_packet }.ok_or_else(|| MediaError::Decode {
                reason: "MPP decode_put_packet 函数指针无效".to_string(),
            })?;

        // 工业级加固：带背压流控重试机制（若 MPP 内部缓冲队列满，先 poll 抽取已解帧以腾出硬件槽位再重试）
        let mut ret = unsafe { put_fn(self.ctx, packet) };
        let mut retry_count = 0;
        while ret < 0 && retry_count < 3 {
            retry_count += 1;
            let _ = self.poll_frame(pts);
            std::thread::sleep(std::time::Duration::from_millis(2));
            ret = unsafe { put_fn(self.ctx, packet) };
        }

        // SAFETY: 输入包送入后释放包包装容器（数据已交由 MPP 内核排队）
        unsafe {
            let _ = ffi::mpp_packet_deinit(&mut packet);
        }

        if ret != 0 {
            return Err(MediaError::Decode {
                reason: format!(
                    "mpp decode_put_packet 硬件送包失败 (重试后依然失败), 返回码: {ret}"
                ),
            });
        }

        self.poll_frame(pts)
    }

    fn poll_frame(&mut self, pts: i64) -> Result<Option<FrameRef>, MediaError> {
        let get_fn = unsafe { (*self.mpi).decode_get_frame }.ok_or_else(|| MediaError::Decode {
            reason: "MPP decode_get_frame 函数指针无效".to_string(),
        })?;

        let mut frame: *mut c_void = std::ptr::null_mut();
        // SAFETY: get_fn 尝试收取已解码完成的原生硬件帧
        let ret = unsafe { get_fn(self.ctx, &mut frame) };
        if ret != 0 || frame.is_null() {
            return Ok(None);
        }

        // 优先检查 info_change 事件（流参数突变或首次获取 SPS/PPS）
        // SAFETY: frame 指针有效
        let info_change = unsafe { ffi::mpp_frame_get_info_change(frame) };
        if info_change != 0 {
            // SAFETY: 读取流真实分辨率与虚宽跨度
            let w = unsafe { ffi::mpp_frame_get_width(frame) };
            let h = unsafe { ffi::mpp_frame_get_height(frame) };
            let hor_s = unsafe { ffi::mpp_frame_get_hor_stride(frame) };
            let ver_s = unsafe { ffi::mpp_frame_get_ver_stride(frame) };

            self.width = w;
            self.height = h;
            self.hor_stride = if hor_s > 0 {
                hor_s
            } else {
                align_hor_stride(w)
            };
            self.ver_stride = if ver_s > 0 {
                ver_s
            } else {
                align_ver_stride(h)
            };

            // 配置 MPP buffer group 容量为 20（预分配 CMA DMA-BUF 循环使用）
            let mut buf_count: std::os::raw::c_uint = 20;
            // SAFETY: control 命令配置帧缓冲数量并回传就绪信令
            unsafe {
                if let Some(ctrl_fn) = (*self.mpi).control {
                    // 1. 设置预分配帧缓冲数量
                    ctrl_fn(
                        self.ctx,
                        ffi::MPP_DEC_SET_FRAME_BUFFER_COUNT,
                        &mut buf_count as *mut _ as *mut c_void,
                    );
                    // 2. 回写解析出的帧参数描述符
                    ctrl_fn(self.ctx, ffi::MPP_DEC_SET_FRAME_INFO, frame);
                    // 3. 关键信令：告知 VPU 缓冲区与步长已配置就绪，解除挂起恢复硬件解码流水线
                    ctrl_fn(
                        self.ctx,
                        ffi::MPP_DEC_SET_INFO_CHANGE_READY,
                        std::ptr::null_mut(),
                    );
                }
            }

            debug!(
                camera_id = %self.camera_id,
                width = self.width,
                height = self.height,
                hor_stride = self.hor_stride,
                ver_stride = self.ver_stride,
                "MPP 检测到流信息变更 (info_change)，已回写参数并确认就绪 (INFO_CHANGE_READY)"
            );

            // SAFETY: info_change 帧不携带像素内容，消费后释放
            unsafe {
                let _ = ffi::mpp_frame_deinit(&mut frame);
            }
            // 递归收取后续紧随的有效画面帧
            return self.poll_frame(pts);
        }

        // 检查受损与丢弃标记
        // SAFETY: frame 指针有效
        let err = unsafe { ffi::mpp_frame_get_errinfo(frame) };
        let discard = unsafe { ffi::mpp_frame_get_discard(frame) };
        if err != 0 || discard != 0 {
            // SAFETY: 受损帧安全释放
            unsafe {
                let _ = ffi::mpp_frame_deinit(&mut frame);
            }
            return Ok(None);
        }

        // 检查像素格式（防范 10-bit HDR / 非标准 NV12 格式导致下游花屏）
        // SAFETY: frame 指针有效
        let fmt = unsafe { ffi::mpp_frame_get_fmt(frame) };
        if fmt == ffi::MPP_FMT_YUV420SP_10BIT {
            tracing::warn!(
                camera_id = %self.camera_id,
                fmt,
                "MPP 输出为 10-bit YUV420SP (P010) 画面，已按硬件步长透传，下游处理需注意位深转换"
            );
        }

        // SAFETY: 提取底层 DRM DMA-BUF 内存句柄
        let mpp_buf = unsafe { ffi::mpp_frame_get_buffer(frame) };
        if mpp_buf.is_null() {
            // SAFETY: 空 buffer 帧释放
            unsafe {
                let _ = ffi::mpp_frame_deinit(&mut frame);
            }
            return Ok(None);
        }

        // 1. 增持引用，防止 MPP 内部重用覆写正在流转的底层 CMA 物理页
        // SAFETY: mpp_buf 合法
        unsafe {
            ffi::mpp_buffer_inc_ref(mpp_buf);
        }

        // 2. 提取原生 DMA-BUF fd 并通过 dup 复制独立内核文件描述符
        // SAFETY: mpp_buf 合法
        let raw_fd = unsafe { ffi::mpp_buffer_get_fd(mpp_buf) };
        if raw_fd < 0 {
            // SAFETY: 发生异常时回退引用与帧句柄
            unsafe {
                ffi::mpp_buffer_put(mpp_buf);
                let _ = ffi::mpp_frame_deinit(&mut frame);
            }
            return Err(MediaError::Decode {
                reason: format!("MPP mpp_buffer_get_fd 返回负数错误: {raw_fd}"),
            });
        }

        // SAFETY: libc::dup 创建独立的 fd，使得 FrameHandle 与 MPP 内部引用解耦
        let dup_fd = unsafe { libc::dup(raw_fd) };
        if dup_fd < 0 {
            // SAFETY: 异常回滚
            unsafe {
                ffi::mpp_buffer_put(mpp_buf);
                let _ = ffi::mpp_frame_deinit(&mut frame);
            }
            return Err(MediaError::Decode {
                reason: format!("dup(dma_buf_fd) 失败: {}", std::io::Error::last_os_error()),
            });
        }

        // SAFETY: dup_fd 由 libc::dup 生成，赋予所有权语义
        let owned_fd = unsafe { OwnedFd::from_raw_fd(dup_fd) };
        let lease: Arc<dyn Send + Sync> = Arc::new(MppBufferLease { buf: mpp_buf });

        let handle = FrameHandle::DmaBuf {
            fd: Arc::new(owned_fd),
            _lease: Some(lease),
        };

        // SAFETY: 获取帧附带的 PTS 时间戳
        let frame_pts = unsafe { ffi::mpp_frame_get_pts(frame) };
        let final_pts = if frame_pts != 0 { frame_pts } else { pts };

        // 动态查询该帧的真实硬件步长（Runtime Stride Query），杜绝动态切片或固件对齐漂移
        let frame_hor_s = unsafe { ffi::mpp_frame_get_hor_stride(frame) as u32 };
        let frame_ver_s = unsafe { ffi::mpp_frame_get_ver_stride(frame) as u32 };
        let final_hor_stride = if frame_hor_s > 0 {
            frame_hor_s
        } else {
            self.hor_stride
        };
        let final_ver_stride = if frame_ver_s > 0 {
            frame_ver_s
        } else {
            self.ver_stride
        };

        // 核心时序与完成证明（Producer Completion Fence Guarantee）：
        // 1. MPP decode_get_frame 返回有效 frame，且 errinfo == 0, discard == 0，
        //    根据 Rockchip MPP 驱动规范，此时硬件 VPU 的中断服务例程已执行完成，物理总线写入完全落盘；
        // 2. 结合 mpp_buffer_inc_ref 阻止底层 CMA 缓冲池物理页回收；
        // 3. 构建持有 OwnedFd 和 MppBufferLease 的 FrameHandle::DmaBuf，向下游提供空间互斥与生命周期保障。
        let frame_ref = FrameRef::new(
            self.camera_id.clone(),
            final_pts,
            self.width,
            self.height,
            StrideInfo::new(final_hor_stride, final_ver_stride),
            PixelFormat::Nv12,
            handle,
        );

        // SAFETY: MppFrame 容器释放，底层物理显存由 MppBufferLease 接管保护
        unsafe {
            let _ = ffi::mpp_frame_deinit(&mut frame);
        }

        Ok(Some(frame_ref))
    }

    fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        if !self.is_initialized {
            return Ok(Vec::new());
        }

        let mut packet: *mut c_void = std::ptr::null_mut();
        // SAFETY: 构建零长度的 EOS 数据包注入解码队列
        let ret = unsafe { ffi::mpp_packet_init(&mut packet, std::ptr::null_mut(), 0) };
        if ret == 0 && !packet.is_null() {
            // SAFETY: 设置 EOS 标记并送入
            unsafe {
                ffi::mpp_packet_set_eos(packet);
                if let Some(put_fn) = (*self.mpi).decode_put_packet {
                    let _ = put_fn(self.ctx, packet);
                }
                let _ = ffi::mpp_packet_deinit(&mut packet);
            }
        }

        let mut flushed = Vec::new();
        while let Ok(Some(frame)) = self.poll_frame(0) {
            flushed.push(frame);
        }

        // SAFETY: 重置上下文状态
        unsafe {
            if let Some(reset_fn) = (*self.mpi).reset {
                let _ = reset_fn(self.ctx);
            }
        }

        debug!(camera_id = %self.camera_id, count = flushed.len(), "MPP 解码器刷新残留帧完成");
        Ok(flushed)
    }

    /// 工业级自愈重置：在 VPU 硬件通道挂起或连续报错时销毁并重建硬件上下文
    fn reset(&mut self) -> Result<(), MediaError> {
        if !self.ctx.is_null() {
            // SAFETY: 销毁旧硬件上下文句柄与显存组
            unsafe {
                ffi::mpp_destroy(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
            self.mpi = std::ptr::null_mut();
            self.is_initialized = false;
        }
        self.init()
    }
}

impl Drop for MppDecoderInner {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            // SAFETY: 解码器退出时彻底销毁 MPP 句柄与底层显存组
            unsafe {
                ffi::mpp_destroy(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
            debug!(camera_id = %self.camera_id, "MPP 硬件上下文安全销毁");
        }
    }
}

/// Rockchip MPP 硬件解码器异步外壳
pub struct MppDecoder {
    tx: mpsc::Sender<DecodeCommand>,
    thread: Option<JoinHandle<()>>,
}

impl MppDecoder {
    pub fn new(camera_id: &str, codec: CodecType) -> Self {
        let (tx, mut rx) = mpsc::channel::<DecodeCommand>(4);
        let cam_id = camera_id.to_string();
        let thread_name = format!("mpp-dec-{cam_id}");

        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut inner = MppDecoderInner::new(cam_id.clone(), codec);
                if let Err(e) = inner.init() {
                    error!(camera_id = %cam_id, error = %e, "MPP 解码器线程初始化失败");
                    while let Some(cmd) = rx.blocking_recv() {
                        match cmd {
                            DecodeCommand::Decode { reply, .. } => {
                                let _ = reply.send(Err(MediaError::DecoderInit {
                                    codec: format!("{codec:?}"),
                                    reason: e.to_string(),
                                }));
                            }
                            DecodeCommand::Flush { reply } => {
                                let _ = reply.send(Err(MediaError::DecoderInit {
                                    codec: format!("{codec:?}"),
                                    reason: e.to_string(),
                                }));
                            }
                        }
                    }
                    return;
                }

                let mut consecutive_errors = 0;
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        DecodeCommand::Decode { packet, pts, reply } => {
                            let res = inner.decode(&packet, pts);
                            match &res {
                                Ok(_) => {
                                    consecutive_errors = 0;
                                }
                                Err(e) => {
                                    consecutive_errors += 1;
                                    if consecutive_errors >= 5 {
                                        tracing::warn!(
                                            camera_id = %cam_id,
                                            consecutive_errors,
                                            error = %e,
                                            "MPP 硬件连续解码失败达到阈值，触发硬件上下文自动重置自愈"
                                        );
                                        if let Err(reinit_err) = inner.reset() {
                                            tracing::error!(
                                                camera_id = %cam_id,
                                                error = %reinit_err,
                                                "MPP 硬件上下文自动重置失败"
                                            );
                                        } else {
                                            consecutive_errors = 0;
                                            tracing::info!(
                                                camera_id = %cam_id,
                                                "MPP 硬件上下文已成功自动重置恢复"
                                            );
                                        }
                                    }
                                }
                            }
                            let _ = reply.send(res);
                        }
                        DecodeCommand::Flush { reply } => {
                            let res = inner.flush();
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .expect("创建 MPP 解码专用线程失败");

        Self {
            tx,
            thread: Some(thread),
        }
    }
}

#[async_trait]
impl VideoDecoder for MppDecoder {
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, MediaError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DecodeCommand::Decode {
            packet: Bytes::copy_from_slice(packet),
            pts,
            reply: reply_tx,
        };

        self.tx.send(cmd).await.map_err(|_| MediaError::Decode {
            reason: "MPP 解码专用线程已退出".to_string(),
        })?;

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "MPP 解码响应通道已关闭".to_string(),
        })?
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DecodeCommand::Flush { reply: reply_tx };

        self.tx.send(cmd).await.map_err(|_| MediaError::Decode {
            reason: "MPP 解码专用线程已退出".to_string(),
        })?;

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "MPP 解码响应通道已关闭".to_string(),
        })?
    }
}

impl Drop for MppDecoder {
    fn drop(&mut self) {
        // self.tx 被释放后，rx.blocking_recv() 返回 None，专用线程退出
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpp_stride_alignment() {
        // 水平 16 字节对齐
        assert_eq!(align_hor_stride(1920), 1920);
        assert_eq!(align_hor_stride(1919), 1920);
        assert_eq!(align_hor_stride(1921), 1936);
        assert_eq!(align_hor_stride(640), 640);
        assert_eq!(align_hor_stride(641), 656);

        // 垂直 16 行虚高对齐 (1080P -> 1088)
        assert_eq!(align_ver_stride(1080), 1088);
        assert_eq!(align_ver_stride(720), 720);
        assert_eq!(align_ver_stride(721), 736);
        assert_eq!(align_ver_stride(360), 368);
    }
}
