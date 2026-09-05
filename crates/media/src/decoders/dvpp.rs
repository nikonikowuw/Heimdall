//! 华为昇腾 DVPP (Digital Video Pre-Processing) VDEC 硬件解码器实现
//!
//! 适配 Ascend 310 / 310B / Atlas 200I DK A2 等昇腾边缘 AI 硬件加速单元。
//! 遵循 DVPP 硬件规格：输出 YUV420SP (NV12)，严格实施 16x2 步长对齐（宽 16 字节对齐，高 2 字节对齐）。
//! 设备显存采用 `DvppBufferPool` 预分配池化流转，解出带 RAII 租约的 `FrameHandle::DeviceMemory` 直通 ACL 推理。

#![cfg(all(target_os = "linux", feature = "dvpp"))]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::thread::JoinHandle;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};
use types::{CodecType, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::buffer_pool::DvppBufferPool;
use crate::decoder::{DecodeCommand, VideoDecoder};
use crate::error::MediaError;

/// 水平宽度步长对齐（华为 DVPP 严格要求水平 16 字节对齐）
#[inline]
pub fn align_dvpp_width_stride(width: u32) -> u32 {
    (width + 15) / 16 * 16
}

/// 垂直高度步长对齐（华为 DVPP 严格要求垂直 2 行对齐）
#[inline]
pub fn align_dvpp_height_stride(height: u32) -> u32 {
    (height + 1) / 2 * 2
}

/// 计算 NV12 在步长对齐后的单帧设备显存字节需求
#[inline]
pub fn calculate_dvpp_nv12_size(width: u32, height: u32) -> usize {
    let stride_w = align_dvpp_width_stride(width) as usize;
    let stride_h = align_dvpp_height_stride(height) as usize;
    stride_w * stride_h * 3 / 2
}

pub(crate) mod ffi {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_uchar, c_uint, c_ulonglong};

    pub const ACL_MEMCPY_HOST_TO_DEVICE: c_int = 1;
    pub const ACL_MEMCPY_DEVICE_TO_HOST: c_int = 2;

    pub const PIXEL_FORMAT_YUV_SEMIPLANAR_420: c_int = 1; // NV12
    pub const H264_MAIN_LEVEL: c_int = 0;
    pub const H265_MAIN_LEVEL: c_int = 3;

    extern "C" {
        pub fn aclrtMemcpy(
            dst: *mut c_void,
            dest_max: usize,
            src: *const c_void,
            count: usize,
            kind: c_int,
        ) -> c_int;

        pub fn acldvppMalloc(dev_ptr: *mut *mut c_void, size: usize) -> c_int;
        pub fn acldvppFree(dev_ptr: *mut c_void) -> c_int;

        pub fn aclvdecCreateChannelDesc() -> *mut c_void;
        pub fn aclvdecDestroyChannelDesc(channel_desc: *mut c_void) -> c_int;
        pub fn aclvdecSetChannelDescChannelId(
            channel_desc: *mut c_void,
            channel_id: c_uint,
        ) -> c_int;
        pub fn aclvdecSetChannelDescThreadId(
            channel_desc: *mut c_void,
            thread_id: c_ulonglong,
        ) -> c_int;
        pub fn aclvdecSetChannelDescEnType(channel_desc: *mut c_void, en_type: c_int) -> c_int;
        pub fn aclvdecSetChannelDescOutPicFormat(channel_desc: *mut c_void, format: c_int)
            -> c_int;
        pub fn aclvdecCreateChannel(channel_desc: *mut c_void) -> c_int;
        pub fn aclvdecDestroyChannel(channel_desc: *mut c_void) -> c_int;

        pub fn aclvdecSendFrame(
            channel_desc: *mut c_void,
            input: *mut c_void,
            output: *mut c_void,
            user_data: *mut c_void,
        ) -> c_int;

        pub fn acldvppCreateStreamDesc() -> *mut c_void;
        pub fn acldvppDestroyStreamDesc(stream_desc: *mut c_void) -> c_int;
        pub fn acldvppSetStreamDescData(stream_desc: *mut c_void, data_dev: *mut c_void) -> c_int;
        pub fn acldvppSetStreamDescSize(stream_desc: *mut c_void, size: c_uint) -> c_int;
        pub fn acldvppSetStreamDescEos(stream_desc: *mut c_void, eos: c_uchar) -> c_int;

        pub fn acldvppCreatePicDesc() -> *mut c_void;
        pub fn acldvppDestroyPicDesc(pic_desc: *mut c_void) -> c_int;
        pub fn acldvppSetPicDescData(pic_desc: *mut c_void, dev_ptr: *mut c_void) -> c_int;
        pub fn acldvppSetPicDescSize(pic_desc: *mut c_void, size: c_uint) -> c_int;
        pub fn acldvppSetPicDescFormat(pic_desc: *mut c_void, format: c_int) -> c_int;
        pub fn acldvppSetPicDescWidth(pic_desc: *mut c_void, width: c_uint) -> c_int;
        pub fn acldvppSetPicDescHeight(pic_desc: *mut c_void, height: c_uint) -> c_int;
        pub fn acldvppSetPicDescWidthStride(pic_desc: *mut c_void, width_stride: c_uint) -> c_int;
        pub fn acldvppSetPicDescHeightStride(pic_desc: *mut c_void, height_stride: c_uint)
            -> c_int;
    }
}

/// DVPP 显存池租约
///
/// 当持有该帧的所有 FrameHandle 副本全部 Drop 析构时，
/// 自动触发将底层连续显存地址归还到 DvppBufferPool 中，无需任何运行时动态 free。
struct DvppBufferLease {
    ptr: *mut c_void,
    pool: Arc<DvppBufferPool>,
}

// SAFETY: ptr 指向预分配的 Device Memory 地址空间；
// pool 通过 Arc 跨线程共享；归还操作内部由 Mutex 串行保护。
unsafe impl Send for DvppBufferLease {}
unsafe impl Sync for DvppBufferLease {}

impl Drop for DvppBufferLease {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            self.pool.return_buffer(self.ptr);
        }
    }
}

/// 专用解码线程独占的 DVPP 硬件通道状态
struct DvppDecoderInner {
    camera_id: String,
    codec: CodecType,
    channel_desc: *mut c_void,
    pool: Arc<DvppBufferPool>,
    width: u32,
    height: u32,
    stride_w: u32,
    stride_h: u32,
    stream_desc: *mut c_void,
    stream_buf: *mut c_void,
    stream_buf_capacity: usize,
    is_initialized: bool,
}

impl DvppDecoderInner {
    fn new(camera_id: String, codec: CodecType) -> Self {
        Self {
            camera_id,
            codec,
            channel_desc: std::ptr::null_mut(),
            pool: Arc::new(DvppBufferPool::new_mock(Vec::new(), 0)),
            width: 1920,
            height: 1080,
            stride_w: align_dvpp_width_stride(1920),
            stride_h: align_dvpp_height_stride(1080),
            stream_desc: std::ptr::null_mut(),
            stream_buf: std::ptr::null_mut(),
            stream_buf_capacity: 0,
            is_initialized: false,
        }
    }

    fn init(&mut self) -> Result<(), MediaError> {
        let stream_format = match self.codec {
            CodecType::H264 => ffi::H264_MAIN_LEVEL,
            CodecType::H265 => ffi::H265_MAIN_LEVEL,
        };

        let block_size = calculate_dvpp_nv12_size(self.width, self.height);
        // 预分配 20 个显存块（满足 16 参考帧 + 4 流水线深度裕量）
        let pool = DvppBufferPool::new(block_size, 20)?;
        self.pool = Arc::new(pool);

        // 初始化码流传输输入显存缓冲区（初始 2MB，可按需动态扩容）
        let init_cap = 2 * 1024 * 1024;
        let mut dev_buf: *mut c_void = std::ptr::null_mut();
        // SAFETY: acldvppMalloc 为输入 NALU 码流分配设备连续显存
        let ret = unsafe { ffi::acldvppMalloc(&mut dev_buf, init_cap) };
        if ret != 0 || dev_buf.is_null() {
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("acldvppMalloc 分配输入码流缓冲区失败, 返回码: {ret}"),
            });
        }
        self.stream_buf = dev_buf;
        self.stream_buf_capacity = init_cap;

        // SAFETY: 创建码流输入描述符
        let stream_desc = unsafe { ffi::acldvppCreateStreamDesc() };
        if stream_desc.is_null() {
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: "acldvppCreateStreamDesc 创建失败".to_string(),
            });
        }
        self.stream_desc = stream_desc;

        // 创建并配置 VDEC 解码通道描述符
        // SAFETY: 创建通道描述符
        let channel_desc = unsafe { ffi::aclvdecCreateChannelDesc() };
        if channel_desc.is_null() {
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: "aclvdecCreateChannelDesc 创建失败".to_string(),
            });
        }

        // SAFETY: 配置通道参数
        unsafe {
            let _ = ffi::aclvdecSetChannelDescChannelId(channel_desc, 0);
            let _ = ffi::aclvdecSetChannelDescEnType(channel_desc, stream_format);
            let _ = ffi::aclvdecSetChannelDescOutPicFormat(
                channel_desc,
                ffi::PIXEL_FORMAT_YUV_SEMIPLANAR_420,
            );
        }

        // SAFETY: 创建实际底层硬件解码通道
        let ret = unsafe { ffi::aclvdecCreateChannel(channel_desc) };
        if ret != 0 {
            unsafe {
                let _ = ffi::aclvdecDestroyChannelDesc(channel_desc);
            }
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("aclvdecCreateChannel 创建硬件通道失败, 返回码: {ret}"),
            });
        }

        self.channel_desc = channel_desc;
        self.is_initialized = true;
        debug!(camera_id = %self.camera_id, codec = ?self.codec, "华为昇腾 DVPP VDEC 硬件解码通道初始化就绪");
        Ok(())
    }

    fn ensure_stream_buffer(&mut self, required_size: usize) -> Result<(), MediaError> {
        if required_size <= self.stream_buf_capacity {
            return Ok(());
        }

        let new_cap = required_size.next_power_of_two();
        let mut new_buf: *mut c_void = std::ptr::null_mut();
        // SAFETY: 重新按需分配更大容量的码流缓冲区
        let ret = unsafe { ffi::acldvppMalloc(&mut new_buf, new_cap) };
        if ret != 0 || new_buf.is_null() {
            return Err(MediaError::Decode {
                reason: format!("acldvppMalloc 扩容码流缓冲区失败, 返回码: {ret}"),
            });
        }

        if !self.stream_buf.is_null() {
            // SAFETY: 释放旧缓冲区
            unsafe {
                let _ = ffi::acldvppFree(self.stream_buf);
            }
        }

        self.stream_buf = new_buf;
        self.stream_buf_capacity = new_cap;
        Ok(())
    }

    fn decode(&mut self, packet: &[u8], pts: i64) -> Result<Option<FrameRef>, MediaError> {
        if !self.is_initialized {
            return Err(MediaError::Decode {
                reason: "DVPP 解码器未初始化".to_string(),
            });
        }

        self.ensure_stream_buffer(packet.len())?;
        self.update_dimensions_if_needed(packet);

        // 将 Host 内存的 NALU 数据拷贝到 Device Memory 码流区
        // SAFETY: aclrtMemcpy 跨主机-设备内存传输码流
        let ret = unsafe {
            ffi::aclrtMemcpy(
                self.stream_buf,
                self.stream_buf_capacity,
                packet.as_ptr() as *const c_void,
                packet.len(),
                ffi::ACL_MEMCPY_HOST_TO_DEVICE,
            )
        };
        if ret != 0 {
            return Err(MediaError::Decode {
                reason: format!("aclrtMemcpy 拷贝输入码流至设备显存失败: {ret}"),
            });
        }

        // SAFETY: 设置输入码流元数据
        unsafe {
            let _ = ffi::acldvppSetStreamDescData(self.stream_desc, self.stream_buf);
            let _ = ffi::acldvppSetStreamDescSize(self.stream_desc, packet.len() as u32);
            let _ = ffi::acldvppSetStreamDescEos(self.stream_desc, 0);
        }

        // 从预分配显存池租借一个输出图像块（带 50ms 硬实时超时保护，防止显存枯竭死锁）
        let (dev_ptr, block_size) = self
            .pool
            .acquire_timeout(std::time::Duration::from_millis(50))
            .ok_or_else(|| MediaError::Decode {
                reason: "DVPP 显存池耗尽超时 (下游租约未释放或处理阻塞)".to_string(),
            })?;

        // SAFETY: 创建输出图片描述符
        let pic_desc = unsafe { ffi::acldvppCreatePicDesc() };
        if pic_desc.is_null() {
            self.pool.return_buffer(dev_ptr);
            return Err(MediaError::Decode {
                reason: "acldvppCreatePicDesc 创建输出图像描述符失败".to_string(),
            });
        }

        // SAFETY: 严格配置输出步长对齐与 NV12 格式
        unsafe {
            let _ = ffi::acldvppSetPicDescData(pic_desc, dev_ptr);
            let _ = ffi::acldvppSetPicDescSize(pic_desc, block_size as u32);
            let _ = ffi::acldvppSetPicDescFormat(pic_desc, ffi::PIXEL_FORMAT_YUV_SEMIPLANAR_420);
            let _ = ffi::acldvppSetPicDescWidth(pic_desc, self.width);
            let _ = ffi::acldvppSetPicDescHeight(pic_desc, self.height);
            let _ = ffi::acldvppSetPicDescWidthStride(pic_desc, self.stride_w);
            let _ = ffi::acldvppSetPicDescHeightStride(pic_desc, self.stride_h);
        }

        // 送入 DVPP VDEC 硬件解码执行
        // SAFETY: 调用 aclvdecSendFrame
        let ret = unsafe {
            ffi::aclvdecSendFrame(
                self.channel_desc,
                self.stream_desc,
                pic_desc,
                std::ptr::null_mut(),
            )
        };

        // SAFETY: 释放临时输出图片描述符
        unsafe {
            let _ = ffi::acldvppDestroyPicDesc(pic_desc);
        }

        if ret != 0 {
            self.pool.return_buffer(dev_ptr);
            return Err(MediaError::Decode {
                reason: format!("aclvdecSendFrame 送入硬件解码失败, 返回码: {ret}"),
            });
        }

        let non_null_ptr = NonNull::new(dev_ptr).ok_or_else(|| MediaError::Decode {
            reason: "DVPP 租借显存指针为空".to_string(),
        })?;

        let lease: Arc<dyn Send + Sync> = Arc::new(DvppBufferLease {
            ptr: dev_ptr,
            pool: Arc::clone(&self.pool),
        });

        let handle = FrameHandle::DeviceMemory {
            ptr: non_null_ptr,
            size: block_size,
            _lease: lease,
        };

        let frame_ref = FrameRef::new(
            self.camera_id.clone(),
            pts,
            self.width,
            self.height,
            StrideInfo::new(self.stride_w, self.stride_h),
            PixelFormat::Nv12,
            handle,
        );

        Ok(Some(frame_ref))
    }

    fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        if !self.is_initialized {
            return Ok(Vec::new());
        }

        // 发送带 EOS 标记的包告知通道结束并刷新内部流水线
        // SAFETY: 设置 EOS 标记
        unsafe {
            let _ = ffi::acldvppSetStreamDescSize(self.stream_desc, 0);
            let _ = ffi::acldvppSetStreamDescEos(self.stream_desc, 1);
            let _ = ffi::aclvdecSendFrame(
                self.channel_desc,
                self.stream_desc,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }

        debug!(camera_id = %self.camera_id, "DVPP 解码器刷新残留状态完成");
        Ok(Vec::new())
    }

    /// 动态分辨率自适应更新
    fn update_dimensions_if_needed(&mut self, packet: &[u8]) {
        let nalus = crate::sps::split_annex_b_nalus(packet);
        for nalu in nalus {
            if nalu.is_empty() {
                continue;
            }
            match self.codec {
                CodecType::H264 => {
                    let nal_type = nalu[0] & 0x1F;
                    if nal_type == 7 {
                        if let Ok(info) = crate::sps::parse_h264_sps(nalu) {
                            self.reconfigure_resolution_if_changed(info.width, info.height);
                        }
                    }
                }
                CodecType::H265 => {
                    let nal_type = (nalu[0] >> 1) & 0x3F;
                    if nal_type == 33 {
                        if let Ok(info) = crate::sps::parse_h265_sps(nalu) {
                            self.reconfigure_resolution_if_changed(info.width, info.height);
                        }
                    }
                }
            }
        }
    }

    fn reconfigure_resolution_if_changed(&mut self, new_w: u32, new_h: u32) {
        if new_w == 0 || new_h == 0 || (new_w == self.width && new_h == self.height) {
            return;
        }

        let new_stride_w = align_dvpp_width_stride(new_w);
        let new_stride_h = align_dvpp_height_stride(new_h);
        let required_size = calculate_dvpp_nv12_size(new_w, new_h);

        tracing::info!(
            camera_id = %self.camera_id,
            old_w = self.width,
            old_h = self.height,
            new_w,
            new_h,
            new_stride_w,
            new_stride_h,
            required_size,
            "DVPP 检测到流分辨率动态变更，更新步长并按需扩容显存池"
        );

        self.width = new_w;
        self.height = new_h;
        self.stride_w = new_stride_w;
        self.stride_h = new_stride_h;

        if required_size > self.pool.block_size() {
            #[cfg(all(target_os = "linux", feature = "dvpp"))]
            if let Ok(new_pool) = DvppBufferPool::new(required_size, 20) {
                self.pool = Arc::new(new_pool);
            }
        }
    }

    /// 工业级自愈重置：在 DVPP 硬件通道挂起或连续报错时销毁并重建硬件通道
    fn reset(&mut self) -> Result<(), MediaError> {
        if !self.channel_desc.is_null() {
            unsafe {
                let _ = ffi::aclvdecDestroyChannel(self.channel_desc);
                let _ = ffi::aclvdecDestroyChannelDesc(self.channel_desc);
            }
            self.channel_desc = std::ptr::null_mut();
        }
        if !self.stream_desc.is_null() {
            unsafe {
                let _ = ffi::acldvppDestroyStreamDesc(self.stream_desc);
            }
            self.stream_desc = std::ptr::null_mut();
        }
        if !self.stream_buf.is_null() {
            unsafe {
                let _ = ffi::acldvppFree(self.stream_buf);
            }
            self.stream_buf = std::ptr::null_mut();
            self.stream_buf_capacity = 0;
        }
        self.is_initialized = false;
        self.init()
    }
}

impl Drop for DvppDecoderInner {
    fn drop(&mut self) {
        if !self.channel_desc.is_null() {
            // SAFETY: 销毁 VDEC 解码通道与通道描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannel(self.channel_desc);
                let _ = ffi::aclvdecDestroyChannelDesc(self.channel_desc);
            }
            self.channel_desc = std::ptr::null_mut();
        }

        if !self.stream_desc.is_null() {
            // SAFETY: 销毁输入码流描述符
            unsafe {
                let _ = ffi::acldvppDestroyStreamDesc(self.stream_desc);
            }
            self.stream_desc = std::ptr::null_mut();
        }

        if !self.stream_buf.is_null() {
            // SAFETY: 释放输入码流设备显存
            unsafe {
                let _ = ffi::acldvppFree(self.stream_buf);
            }
            self.stream_buf = std::ptr::null_mut();
        }

        debug!(camera_id = %self.camera_id, "DVPP 硬件资源安全回收");
    }
}

/// 华为昇腾 DVPP 硬件解码器异步外壳
pub struct DvppDecoder {
    tx: mpsc::Sender<DecodeCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DvppDecoder {
    pub fn new(camera_id: &str, codec: CodecType) -> Self {
        let (tx, mut rx) = mpsc::channel::<DecodeCommand>(4);
        let cam_id = camera_id.to_string();
        let thread_name = format!("dvpp-dec-{cam_id}");

        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut inner = DvppDecoderInner::new(cam_id.clone(), codec);
                if let Err(e) = inner.init() {
                    error!(camera_id = %cam_id, error = %e, "DVPP 解码器线程初始化失败");
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
                                            "DVPP 硬件连续解码失败达到阈值，触发硬件通道自动重置自愈"
                                        );
                                        if let Err(reinit_err) = inner.reset() {
                                            tracing::error!(
                                                camera_id = %cam_id,
                                                error = %reinit_err,
                                                "DVPP 硬件通道自动重置失败"
                                            );
                                        } else {
                                            consecutive_errors = 0;
                                            tracing::info!(
                                                camera_id = %cam_id,
                                                "DVPP 硬件通道已成功自动重置恢复"
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
            .expect("创建 DVPP 解码专用线程失败");

        Self {
            tx,
            thread: Some(thread),
        }
    }
}

#[async_trait]
impl VideoDecoder for DvppDecoder {
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
            reason: "DVPP 解码专用线程已退出".to_string(),
        })?;

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码响应通道已关闭".to_string(),
        })?
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DecodeCommand::Flush { reply: reply_tx };

        self.tx.send(cmd).await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码专用线程已退出".to_string(),
        })?;

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码响应通道已关闭".to_string(),
        })?
    }
}

impl Drop for DvppDecoder {
    fn drop(&mut self) {
        // self.tx 析构使 rx 退出循环
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dvpp_stride_alignment() {
        // 宽 16 字节对齐
        assert_eq!(align_dvpp_width_stride(1920), 1920);
        assert_eq!(align_dvpp_width_stride(1919), 1920);
        assert_eq!(align_dvpp_width_stride(1921), 1936);

        // 高 2 行对齐
        assert_eq!(align_dvpp_height_stride(1080), 1080);
        assert_eq!(align_dvpp_height_stride(1081), 1082);

        // NV12 显存块容量计算
        assert_eq!(calculate_dvpp_nv12_size(1920, 1080), 1920 * 1080 * 3 / 2);
    }
}
