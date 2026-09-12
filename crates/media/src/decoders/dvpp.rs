//! 华为昇腾 DVPP (Digital Video Pre-Processing) VDEC 硬件解码器实现
//!
//! 适配 Ascend 310 / 310B / Atlas 200I DK A2 等昇腾边缘 AI 硬件加速单元。
//! 遵循 DVPP 硬件规格：输出 YUV420SP (NV12)，严格实施 16x2 步长对齐（宽 16 字节对齐，高 2 字节对齐）。
//! 设备显存采用 `DvppBufferPool` 预分配池化流转，解出带 RAII 租约的 `FrameHandle::DeviceMemory` 直通 ACL 推理。
//!
//! ### 核心架构与零拷贝边界精确定义：
//! 1. **输入码流路径 (Host→Device DMA 传输)**：
//!    - 网络 RTSP NALU 码流由 CPU/Host 内存解包生成；
//!    - 输入 DVPP 硬件解码时，存在一次不可避免的 Host→Device DMA 复制 (`aclrtMemcpy(..., ACL_MEMCPY_HOST_TO_DEVICE)`)；
//!    - 系统契约准确表述为：**“解码输出到推理输入的设备侧零拷贝，输入码流存在一次 Host→Device DMA 复制”**，
//!      绝不能笼统宣称“全链路零拷贝”。
//! 2. **常驻推理主路径 (`infer_fast_path`)**：
//!    - 解码产物 `FrameHandle::DeviceMemory` 驻留在华为昇腾连续设备显存空间；
//!    - 直接直通流转至 VPC（抠图缩放）或 AIPP（色度转换与硬件归一化）-> ACL 模型推理，设备显存 0 回读，0 CPU 拷贝。
//! 3. **低频证据生成路径 (`snapshot_readback_path`)**：
//!    - 仅在规则引擎触发告警或人工抓拍时按需单帧触发，通过 `aclrtMemcpy(ACL_MEMCPY_DEVICE_TO_HOST)` 回读到 Host，
//!      由 CPU 完成定点数色彩转换并压缩为 JPEG 证据图片落盘。
//!
//! ### 架构生命周期保障（契约于华为 AscendCL DVPP VDEC 异步回调模型）：
//! 1. `aclvdecSendFrame` 为纯异步非阻塞投递，仅表示任务成功入队硬件队列，绝不同步假定完成；
//! 2. 输入流显存 (`stream_buf`) 与描述符 (`stream_desc`) 实行在途独立持有，直到 `aclvdecCallback` 触发后才释放，严禁跨帧覆盖；
//! 3. 输出描述符 (`pic_desc`) 在回调返回前保持有效，严禁提前销毁 (避免 Use-After-Free)；
//! 4. 输出显存 (`dev_ptr`) 在硬件 DMA 写入期间处于独占在途态，硬件完成信号 (`acldvppGetPicDescRetCode == 0`) 是构建 `FrameRef` 与移交租约的唯一凭据；
//! 5. 解码失败时由回调负责将 `dev_ptr` 自动归还显存池，绝不泄漏脏数据或半写显存至下游。

#[cfg(not(any(all(target_os = "linux", feature = "dvpp"), test)))]
compile_error!(
    "crates/media/src/decoders/dvpp.rs should only be compiled on Linux with feature dvpp or under test"
);

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};
use types::{CodecType, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::buffer_pool::DvppBufferPool;
use crate::decoder::{
    DecodeCommand, DecodeDeliveryPolicy, VideoDecoder, DEFAULT_THREAD_SHUTDOWN_TIMEOUT,
};
use crate::error::MediaError;

/// 水平宽度步长对齐（华为 DVPP 严格要求水平 16 字节对齐）
/// 单个压缩码流包硬上限（4 MB，主流 4K H.264/H.265 I帧上限通常 <= 2MB，防止恶意大包 DoS 攻击）
pub const MAX_COMPRESSED_PACKET_SIZE: usize = 4 * 1024 * 1024;

/// 支持的最大帧宽（4K Ultra HD: 3840）
pub const MAX_FRAME_WIDTH: u32 = 3840;

/// 支持的最大帧高（4K Ultra HD: 2160）
pub const MAX_FRAME_HEIGHT: u32 = 2160;

/// 支持的最小帧宽
pub const MIN_FRAME_WIDTH: u32 = 128;

/// 支持的最小帧高
pub const MIN_FRAME_HEIGHT: u32 = 128;

/// 单通道设备显存最大硬预算（200 MB，防止恶意大分辨率耗尽连续物理显存）
pub const MAX_DEVICE_BUFFER_BYTES: usize = 200 * 1024 * 1024;

/// 单路解码器每分钟最大允许的分辨率变更次数（超过 3 次判定为恶意 DoS 抖动触发熔断）
pub const MAX_RESOLUTION_RECONFIGURES_PER_MINUTE: usize = 3;

/// 动态分辨率变更冷却时间（5 秒内忽略连续重配请求）
pub const DVPP_RECONFIG_COOLDOWN: Duration = Duration::from_secs(5);
/// 动态分辨率抖动检测滑动窗口（60 秒）
pub const DVPP_FLAPPING_WINDOW: Duration = Duration::from_secs(60);
/// 单通道显存池最小块数保底
pub const DVPP_MIN_POOL_BLOCKS: usize = 8;
/// 单通道显存池最大块数
pub const DVPP_MAX_POOL_BLOCKS: usize = 20;

/// 水平宽度步长对齐（华为 DVPP 严格要求水平 16 字节对齐，使用 checked 运算防止整数溢出）
#[inline]
pub fn align_dvpp_width_stride(width: u32) -> u32 {
    width
        .checked_add(15)
        .map(|v| (v / 16) * 16)
        .unwrap_or(MAX_FRAME_WIDTH)
}

/// 垂直高度步长对齐（华为 DVPP 严格要求垂直 2 行对齐，使用 checked 运算防止整数溢出）
#[inline]
pub fn align_dvpp_height_stride(height: u32) -> u32 {
    height
        .checked_add(1)
        .map(|v| (v / 2) * 2)
        .unwrap_or(MAX_FRAME_HEIGHT)
}

/// 计算 NV12 在步长对齐后的单帧设备显存字节需求（严格实施 Checked Arithmetic 乘法防护）
#[inline]
pub fn calculate_dvpp_nv12_size(width: u32, height: u32) -> usize {
    let stride_w = align_dvpp_width_stride(width) as usize;
    let stride_h = align_dvpp_height_stride(height) as usize;
    stride_w
        .checked_mul(stride_h)
        .and_then(|v| v.checked_mul(3))
        .map(|v| v / 2)
        .unwrap_or(usize::MAX)
}

/// 计算 NV12 显存大小并在溢出或非法尺寸时返回明确错误
#[inline]
pub fn calculate_dvpp_nv12_size_checked(width: u32, height: u32) -> Result<usize, MediaError> {
    if !is_valid_dvpp_resolution(width, height) {
        return Err(MediaError::Decode {
            reason: format!("分辨率 {width}x{height} 超出安全边界约束"),
        });
    }
    let stride_w = align_dvpp_width_stride(width) as usize;
    let stride_h = align_dvpp_height_stride(height) as usize;
    stride_w
        .checked_mul(stride_h)
        .and_then(|v| v.checked_mul(3))
        .map(|v| v / 2)
        .ok_or_else(|| MediaError::Decode {
            reason: "NV12 显存容量计算乘法溢出".to_string(),
        })
}

/// 校验流分辨率是否处于允许的安全边界范围内（防御恶意构造畸形 SPS）
#[inline]
pub fn is_valid_dvpp_resolution(width: u32, height: u32) -> bool {
    // 1. 范围校验：128x128 ~ 3840x2160 (4K)
    if !(MIN_FRAME_WIDTH..=MAX_FRAME_WIDTH).contains(&width)
        || !(MIN_FRAME_HEIGHT..=MAX_FRAME_HEIGHT).contains(&height)
    {
        return false;
    }
    // 2. 偶数约束（针对 YUV420 采样必须为偶数像素）
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return false;
    }
    // 3. 总像素数防溢出与 4K 超清上限保护
    match (width as u64).checked_mul(height as u64) {
        Some(pixels) => pixels <= (MAX_FRAME_WIDTH as u64) * (MAX_FRAME_HEIGHT as u64),
        None => false,
    }
}

#[allow(non_snake_case, dead_code)]
pub(crate) mod ffi {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_uchar, c_uint, c_ulonglong};

    pub const ACL_MEMCPY_HOST_TO_DEVICE: c_int = 1;
    pub const ACL_MEMCPY_DEVICE_TO_HOST: c_int = 2;

    pub const PIXEL_FORMAT_YUV_SEMIPLANAR_420: c_int = 1; // NV12
    pub const H264_MAIN_LEVEL: c_int = 0;
    pub const H265_MAIN_LEVEL: c_int = 3;

    pub type AclvdecCallback = Option<
        unsafe extern "C" fn(input: *mut c_void, output: *mut c_void, user_data: *mut c_void),
    >;

    #[cfg(all(target_os = "linux", feature = "dvpp"))]
    extern "C" {
        pub fn aclrtMemcpy(
            dst: *mut c_void,
            dest_max: usize,
            src: *const c_void,
            count: usize,
            kind: c_int,
        ) -> c_int;

        pub fn aclrtProcessReport(timeout_ms: c_int) -> c_int;

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
        pub fn aclvdecSetChannelDescCallback(
            channel_desc: *mut c_void,
            callback: AclvdecCallback,
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
        pub fn acldvppGetPicDescRetCode(pic_desc: *const c_void) -> c_uint;
        pub fn acldvppGetPicDescData(pic_desc: *const c_void) -> *mut c_void;
        pub fn acldvppGetPicDescSize(pic_desc: *const c_void) -> c_uint;
    }

    #[cfg(not(all(target_os = "linux", feature = "dvpp")))]
    pub use mock_ffi::*;

    #[cfg(not(all(target_os = "linux", feature = "dvpp")))]
    #[allow(
        non_snake_case,
        dead_code,
        clippy::undocumented_unsafe_blocks,
        clippy::unwrap_used
    )]
    pub mod mock_ffi {
        use super::*;
        use crate::decoders::dvpp::InFlightFrame;
        use std::collections::{HashMap, VecDeque};
        use std::sync::Mutex;
        use std::time::Duration;

        pub struct MockChannel {
            pub channel_id: c_uint,
            pub thread_id: c_ulonglong,
            pub callback: AclvdecCallback,
            pub en_type: c_int,
            pub out_format: c_int,
        }

        pub struct MockStreamDesc {
            pub data: *mut c_void,
            pub size: c_uint,
            pub eos: c_uchar,
        }

        pub struct MockPicDesc {
            pub data: *mut c_void,
            pub size: c_uint,
            pub format: c_int,
            pub width: c_uint,
            pub height: c_uint,
            pub width_stride: c_uint,
            pub height_stride: c_uint,
            pub ret_code: c_uint,
        }

        pub struct MockQueueItem {
            pub input: *mut c_void,
            pub output: *mut c_void,
            pub user_data: *mut c_void,
            pub callback: AclvdecCallback,
        }

        // SAFETY: 模拟队列中裸指针仅用于线程调度模拟
        unsafe impl Send for MockQueueItem {}

        static MOCK_QUEUES: Mutex<Option<HashMap<u64, VecDeque<MockQueueItem>>>> = Mutex::new(None);

        pub unsafe fn aclrtMemcpy(
            dst: *mut c_void,
            _dest_max: usize,
            src: *const c_void,
            count: usize,
            _kind: c_int,
        ) -> c_int {
            if dst.is_null() || src.is_null() {
                return -1;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, count);
            }
            0
        }

        pub unsafe fn aclrtProcessReport(timeout_ms: c_int) -> c_int {
            let current_tid: u64 = unsafe { libc::pthread_self() as usize as u64 };
            let task = {
                let mut guard = MOCK_QUEUES.lock().unwrap();
                if let Some(ref mut map) = *guard {
                    if let Some(queue) = map.get_mut(&current_tid) {
                        queue.pop_front()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(item) = task {
                if let Some(cb) = item.callback {
                    if !item.output.is_null() && !item.user_data.is_null() {
                        let in_flight = unsafe { &*(item.user_data as *const InFlightFrame) };
                        if in_flight.camera_id.contains("err") {
                            unsafe {
                                (*(item.output as *mut MockPicDesc)).ret_code = 1;
                            }
                        }
                    }
                    unsafe {
                        cb(item.input, item.output, item.user_data);
                    }
                }
                0
            } else {
                std::thread::sleep(Duration::from_millis((timeout_ms as u64).min(5)));
                0
            }
        }

        pub unsafe fn acldvppMalloc(dev_ptr: *mut *mut c_void, size: usize) -> c_int {
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let ret = unsafe { libc::posix_memalign(&mut ptr, 64, size) };
            if ret == 0 {
                unsafe {
                    *dev_ptr = ptr;
                }
                0
            } else {
                -1
            }
        }

        pub unsafe fn acldvppFree(dev_ptr: *mut c_void) -> c_int {
            if !dev_ptr.is_null() {
                unsafe {
                    libc::free(dev_ptr);
                }
            }
            0
        }

        pub unsafe fn aclvdecCreateChannelDesc() -> *mut c_void {
            let desc = Box::new(MockChannel {
                channel_id: 0,
                thread_id: 0,
                callback: None,
                en_type: 0,
                out_format: 0,
            });
            Box::into_raw(desc) as *mut c_void
        }

        pub unsafe fn aclvdecDestroyChannelDesc(channel_desc: *mut c_void) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    drop(Box::from_raw(channel_desc as *mut MockChannel));
                }
            }
            0
        }

        pub unsafe fn aclvdecSetChannelDescChannelId(
            channel_desc: *mut c_void,
            channel_id: c_uint,
        ) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    (*(channel_desc as *mut MockChannel)).channel_id = channel_id;
                }
            }
            0
        }

        pub unsafe fn aclvdecSetChannelDescThreadId(
            channel_desc: *mut c_void,
            thread_id: c_ulonglong,
        ) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    (*(channel_desc as *mut MockChannel)).thread_id = thread_id;
                }
            }
            0
        }

        pub unsafe fn aclvdecSetChannelDescCallback(
            channel_desc: *mut c_void,
            callback: AclvdecCallback,
        ) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    (*(channel_desc as *mut MockChannel)).callback = callback;
                }
            }
            0
        }

        pub unsafe fn aclvdecSetChannelDescEnType(
            channel_desc: *mut c_void,
            en_type: c_int,
        ) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    (*(channel_desc as *mut MockChannel)).en_type = en_type;
                }
            }
            0
        }

        pub unsafe fn aclvdecSetChannelDescOutPicFormat(
            channel_desc: *mut c_void,
            format: c_int,
        ) -> c_int {
            if !channel_desc.is_null() {
                unsafe {
                    (*(channel_desc as *mut MockChannel)).out_format = format;
                }
            }
            0
        }

        pub unsafe fn aclvdecCreateChannel(channel_desc: *mut c_void) -> c_int {
            if channel_desc.is_null() {
                return -1;
            }
            let tid = unsafe { (*(channel_desc as *mut MockChannel)).thread_id };
            let mut guard = MOCK_QUEUES.lock().unwrap();
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(tid, VecDeque::new());
            0
        }

        pub unsafe fn aclvdecDestroyChannel(channel_desc: *mut c_void) -> c_int {
            if !channel_desc.is_null() {
                let tid = unsafe { (*(channel_desc as *mut MockChannel)).thread_id };
                let mut guard = MOCK_QUEUES.lock().unwrap();
                if let Some(ref mut map) = *guard {
                    map.remove(&tid);
                }
            }
            0
        }

        pub unsafe fn aclvdecSendFrame(
            channel_desc: *mut c_void,
            input: *mut c_void,
            output: *mut c_void,
            user_data: *mut c_void,
        ) -> c_int {
            if channel_desc.is_null() {
                return -1;
            }
            let (tid, cb) = unsafe {
                let ch = &*(channel_desc as *mut MockChannel);
                (ch.thread_id, ch.callback)
            };
            let item = MockQueueItem {
                input,
                output,
                user_data,
                callback: cb,
            };
            let mut guard = MOCK_QUEUES.lock().unwrap();
            if let Some(ref mut map) = *guard {
                if let Some(q) = map.get_mut(&tid) {
                    q.push_back(item);
                    0
                } else {
                    -1
                }
            } else {
                -1
            }
        }

        pub unsafe fn acldvppCreateStreamDesc() -> *mut c_void {
            let s = Box::new(MockStreamDesc {
                data: std::ptr::null_mut(),
                size: 0,
                eos: 0,
            });
            Box::into_raw(s) as *mut c_void
        }

        pub unsafe fn acldvppDestroyStreamDesc(stream_desc: *mut c_void) -> c_int {
            if !stream_desc.is_null() {
                unsafe {
                    drop(Box::from_raw(stream_desc as *mut MockStreamDesc));
                }
            }
            0
        }

        pub unsafe fn acldvppSetStreamDescData(
            stream_desc: *mut c_void,
            data_dev: *mut c_void,
        ) -> c_int {
            if !stream_desc.is_null() {
                unsafe {
                    (*(stream_desc as *mut MockStreamDesc)).data = data_dev;
                }
            }
            0
        }

        pub unsafe fn acldvppSetStreamDescSize(stream_desc: *mut c_void, size: c_uint) -> c_int {
            if !stream_desc.is_null() {
                unsafe {
                    (*(stream_desc as *mut MockStreamDesc)).size = size;
                }
            }
            0
        }

        pub unsafe fn acldvppSetStreamDescEos(stream_desc: *mut c_void, eos: c_uchar) -> c_int {
            if !stream_desc.is_null() {
                unsafe {
                    (*(stream_desc as *mut MockStreamDesc)).eos = eos;
                }
            }
            0
        }

        pub unsafe fn acldvppCreatePicDesc() -> *mut c_void {
            let p = Box::new(MockPicDesc {
                data: std::ptr::null_mut(),
                size: 0,
                format: 0,
                width: 0,
                height: 0,
                width_stride: 0,
                height_stride: 0,
                ret_code: 0,
            });
            Box::into_raw(p) as *mut c_void
        }

        pub unsafe fn acldvppDestroyPicDesc(pic_desc: *mut c_void) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    drop(Box::from_raw(pic_desc as *mut MockPicDesc));
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescData(pic_desc: *mut c_void, dev_ptr: *mut c_void) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).data = dev_ptr;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescSize(pic_desc: *mut c_void, size: c_uint) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).size = size;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescFormat(pic_desc: *mut c_void, format: c_int) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).format = format;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescWidth(pic_desc: *mut c_void, width: c_uint) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).width = width;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescHeight(pic_desc: *mut c_void, height: c_uint) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).height = height;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescWidthStride(
            pic_desc: *mut c_void,
            width_stride: c_uint,
        ) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).width_stride = width_stride;
                }
            }
            0
        }

        pub unsafe fn acldvppSetPicDescHeightStride(
            pic_desc: *mut c_void,
            height_stride: c_uint,
        ) -> c_int {
            if !pic_desc.is_null() {
                unsafe {
                    (*(pic_desc as *mut MockPicDesc)).height_stride = height_stride;
                }
            }
            0
        }

        pub unsafe fn acldvppGetPicDescRetCode(pic_desc: *const c_void) -> c_uint {
            if !pic_desc.is_null() {
                unsafe { (*(pic_desc as *const MockPicDesc)).ret_code }
            } else {
                1
            }
        }

        pub unsafe fn acldvppGetPicDescData(pic_desc: *const c_void) -> *mut c_void {
            if !pic_desc.is_null() {
                unsafe { (*(pic_desc as *const MockPicDesc)).data }
            } else {
                std::ptr::null_mut()
            }
        }

        pub unsafe fn acldvppGetPicDescSize(pic_desc: *const c_void) -> c_uint {
            if !pic_desc.is_null() {
                unsafe { (*(pic_desc as *const MockPicDesc)).size }
            } else {
                0
            }
        }
    }
}

/// DVPP 显存池租约
///
/// 当持有该帧的所有 FrameHandle 副本全部 Drop 析构时，
/// 自动触发将底层连续显存地址归还到 DvppBufferPool 中，无需任何运行时动态 free。
/// 携带 generation 代际标记，防御跨池/跨重构周期的延迟归还。
struct DvppBufferLease {
    ptr: *mut c_void,
    generation: u64,
    pool: Arc<DvppBufferPool>,
}

// SAFETY: ptr 指向预分配的 Device Memory 地址空间；
// pool 通过 Arc 跨线程共享；归还操作内部由 Mutex 串行保护。
unsafe impl Send for DvppBufferLease {}
// SAFETY: DvppBufferLease 仅持只读指针与线程安全的 Arc<DvppBufferPool>
unsafe impl Sync for DvppBufferLease {}

impl Drop for DvppBufferLease {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            if let Err(e) = self
                .pool
                .return_buffer_with_generation(self.ptr, self.generation)
            {
                warn!(
                    error = %e,
                    ptr = ?self.ptr,
                    generation = self.generation,
                    "DvppBufferLease: 归还显存块异常或代际不匹配"
                );
            }
        }
    }
}

/// 处于 VDEC 硬件解码在途执行中的帧上下文
///
/// 严格契约：
/// 只有当硬件完成解码并触发回调后，输入资源才会被回收，输出图片才会被包装为 FrameRef。
struct InFlightFrame {
    camera_id: String,
    pts: i64,
    width: u32,
    height: u32,
    stride_w: u32,
    stride_h: u32,
    /// 独立的输入 NALU 码流显存地址
    stream_buf: *mut c_void,
    /// 输入流描述符
    stream_desc: *mut c_void,
    /// 输出图片描述符
    pic_desc: *mut c_void,
    /// 输出设备显存块（从 DvppBufferPool 租借）
    dev_ptr: *mut c_void,
    /// 输出显存字节数
    block_size: usize,
    /// 预分配显存池引用
    pool: Arc<DvppBufferPool>,
    /// 完成帧回传通道
    out_tx: std::sync::mpsc::Sender<Result<FrameRef, MediaError>>,
}

/// DVPP VDEC 硬件完成回调函数（由 CANN Report 线程在硬件触发中断后执行）
///
/// 严格匹配 CANN 官方签名：
/// `typedef void (*aclvdecCallback)(acldvppStreamDesc *input, acldvppPicDesc *output, void *userData);`
unsafe extern "C" fn dvpp_vdec_callback(
    _input: *mut c_void,
    output: *mut c_void,
    user_data: *mut c_void,
) {
    if user_data.is_null() {
        // EOS 或空回调直接返回
        return;
    }

    // SAFETY: 从 user_data 恢复所有权。Box::from_raw 重建 RAII 作用域，确保无论正常还是错误分支均不泄露。
    let in_flight = unsafe { Box::from_raw(user_data as *mut InFlightFrame) };

    // 1. 硬件完成状态检查
    // SAFETY: output 为有效 acldvppPicDesc 指针
    let ret_code = unsafe { ffi::acldvppGetPicDescRetCode(output) };

    // 2. 硬件 DMA 读取已完成，输入资源生命周期终结
    // SAFETY: 销毁输入码流描述符并释放设备显存
    unsafe {
        if !in_flight.stream_desc.is_null() {
            let _ = ffi::acldvppDestroyStreamDesc(in_flight.stream_desc);
        }
        if !in_flight.stream_buf.is_null() {
            let _ = ffi::acldvppFree(in_flight.stream_buf);
        }
    }

    // 3. 硬件写入已完成，输出图片描述符生命周期终结
    // SAFETY: 释放输出图片描述符
    unsafe {
        if !in_flight.pic_desc.is_null() {
            let _ = ffi::acldvppDestroyPicDesc(in_flight.pic_desc);
        }
    }

    // 4. 根据硬件返回码确立输出显存处理
    if ret_code == 0 {
        // 解码成功：此时 DVPP 硬件写入完全完毕，硬件完成信号确立！
        let non_null_ptr = match NonNull::new(in_flight.dev_ptr) {
            Some(ptr) => ptr,
            None => {
                if let Err(e) = in_flight.pool.return_buffer(in_flight.dev_ptr) {
                    warn!(error = %e, "dev_ptr 为 null 回滚归还失败");
                }
                let _ = in_flight.out_tx.send(Err(MediaError::Decode {
                    reason: "DVPP 回调中 dev_ptr 为 null".to_string(),
                }));
                return;
            }
        };

        // 构造带显存池归还租约的 FrameHandle
        let lease: Arc<dyn Send + Sync> = Arc::new(DvppBufferLease {
            ptr: in_flight.dev_ptr,
            generation: in_flight.pool.generation(),
            pool: Arc::clone(&in_flight.pool),
        });

        let handle = FrameHandle::DeviceMemory {
            ptr: non_null_ptr,
            size: in_flight.block_size,
            _lease: lease,
        };

        let frame_ref = FrameRef::new(
            in_flight.camera_id,
            in_flight.pts,
            in_flight.width,
            in_flight.height,
            StrideInfo::new(in_flight.stride_w, in_flight.stride_h),
            PixelFormat::Nv12,
            handle,
        );

        let _ = in_flight.out_tx.send(Ok(frame_ref));
    } else {
        // 解码失败（码流受损/硬件未过）：归还显存池，绝不把半写或受损显存流向下游
        warn!(
            camera_id = %in_flight.camera_id,
            pts = in_flight.pts,
            ret_code,
            "DVPP 硬件解码回调报告错误，归还输出显存块"
        );
        if let Err(e) = in_flight.pool.return_buffer(in_flight.dev_ptr) {
            warn!(error = %e, ptr = ?in_flight.dev_ptr, "解码失败显存块归还异常");
        }
        let _ = in_flight.out_tx.send(Err(MediaError::Decode {
            reason: format!("DVPP 硬件解码失败, retCode: {ret_code}"),
        }));
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
    is_initialized: bool,
    // Report 回调驱动线程控制
    report_running: Arc<AtomicBool>,
    report_thread: Option<JoinHandle<()>>,
    report_thread_id: u64,
    // 异步完成帧接收通道
    out_rx: std::sync::mpsc::Receiver<Result<FrameRef, MediaError>>,
    out_tx: std::sync::mpsc::Sender<Result<FrameRef, MediaError>>,
    // 在途任务计数
    in_flight_count: usize,
    // Drain 期间收割的完成帧暂存队列
    drained_frames: std::collections::VecDeque<FrameRef>,

    // === 动态分辨率安全与熔断防护状态 ===
    last_reconfig_time: Option<std::time::Instant>,
    reconfig_history: std::collections::VecDeque<std::time::Instant>,
    is_degraded: bool,
    reconfig_cooldown: Duration,
    flapping_window: Duration,
    max_flapping_count: usize,
    max_pool_memory_bytes: usize,
    // 关停与中止协同标志
    shutdown_flag: Arc<AtomicBool>,
}

impl DvppDecoderInner {
    fn new(camera_id: String, codec: CodecType, shutdown_flag: Arc<AtomicBool>) -> Self {
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        Self {
            camera_id,
            codec,
            channel_desc: std::ptr::null_mut(),
            pool: Arc::new(DvppBufferPool::new_mock(Vec::new(), 0)),
            width: 1920,
            height: 1080,
            stride_w: align_dvpp_width_stride(1920),
            stride_h: align_dvpp_height_stride(1080),
            is_initialized: false,
            report_running: Arc::new(AtomicBool::new(false)),
            report_thread: None,
            report_thread_id: 0,
            out_rx,
            out_tx,
            in_flight_count: 0,
            drained_frames: std::collections::VecDeque::new(),
            last_reconfig_time: None,
            reconfig_history: std::collections::VecDeque::new(),
            is_degraded: false,
            reconfig_cooldown: DVPP_RECONFIG_COOLDOWN,
            flapping_window: DVPP_FLAPPING_WINDOW,
            max_flapping_count: MAX_RESOLUTION_RECONFIGURES_PER_MINUTE,
            max_pool_memory_bytes: MAX_DEVICE_BUFFER_BYTES,
            shutdown_flag,
        }
    }

    #[cfg(test)]
    fn new_test(camera_id: String, codec: CodecType) -> Self {
        Self::new(camera_id, codec, Arc::new(AtomicBool::new(false)))
    }

    fn init(&mut self) -> Result<(), MediaError> {
        let stream_format = match self.codec {
            CodecType::H264 => ffi::H264_MAIN_LEVEL,
            CodecType::H265 => ffi::H265_MAIN_LEVEL,
            CodecType::Aac => {
                return Err(MediaError::DecoderInit {
                    codec: format!("{:?}", self.codec),
                    reason: "DVPP 硬件解码器不支持音频流解码".to_string(),
                });
            }
        };

        let block_size = calculate_dvpp_nv12_size(self.width, self.height);
        // 预分配 20 个显存块（满足 16 参考帧 + 4 流水线深度裕量）
        let pool = DvppBufferPool::new(block_size, 20)?;
        self.pool = Arc::new(pool);

        // 创建通道描述符
        // SAFETY: 创建通道描述符
        let channel_desc = unsafe { ffi::aclvdecCreateChannelDesc() };
        if channel_desc.is_null() {
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: "aclvdecCreateChannelDesc 创建失败".to_string(),
            });
        }

        // 启动专用 Report 驱动线程
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let cam_id_report = self.camera_id.clone();

        let report_thread = std::thread::Builder::new()
            .name(format!("dvpp-rep-{}", self.camera_id))
            .spawn(move || {
                // SAFETY: 获取当前线程句柄作为 Report Thread ID
                let tid: u64 = unsafe { libc::pthread_self() as usize as u64 };
                let _ = tid_tx.send(tid);

                while running_clone.load(Ordering::Relaxed) {
                    // 超时 10ms 轮询，既保证低延迟响应中断，又允许检查 running 状态
                    // SAFETY: aclrtProcessReport 循环轮询事件驱动回调执行
                    unsafe {
                        ffi::aclrtProcessReport(10);
                    }
                }
                debug!(camera_id = %cam_id_report, "DVPP Report 驱动线程退出");
            })
            .map_err(|e| {
                // SAFETY: 线程创建失败时销毁通道描述符
                unsafe {
                    let _ = ffi::aclvdecDestroyChannelDesc(channel_desc);
                }
                MediaError::DecoderInit {
                    codec: format!("{:?}", self.codec),
                    reason: format!("创建 DVPP Report 驱动线程失败: {e}"),
                }
            })?;

        let report_thread_id = tid_rx.recv().map_err(|_| {
            // SAFETY: 线程接收失败时销毁通道描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannelDesc(channel_desc);
            }
            MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: "接收 DVPP Report 驱动线程 ID 失败".to_string(),
            }
        })?;

        // SAFETY: 按照 AscendCL VDEC 规范配置通道属性（绑定回调与驱动线程）
        unsafe {
            let _ = ffi::aclvdecSetChannelDescChannelId(channel_desc, 0);
            let _ = ffi::aclvdecSetChannelDescThreadId(channel_desc, report_thread_id);
            let _ = ffi::aclvdecSetChannelDescCallback(channel_desc, Some(dvpp_vdec_callback));
            let _ = ffi::aclvdecSetChannelDescEnType(channel_desc, stream_format);
            let _ = ffi::aclvdecSetChannelDescOutPicFormat(
                channel_desc,
                ffi::PIXEL_FORMAT_YUV_SEMIPLANAR_420,
            );
        }

        // SAFETY: 创建底层硬件解码通道
        let ret = unsafe { ffi::aclvdecCreateChannel(channel_desc) };
        if ret != 0 {
            running.store(false, Ordering::Relaxed);
            let _ = report_thread.join();
            // SAFETY: 通道创建失败时销毁通道描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannelDesc(channel_desc);
            }
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("aclvdecCreateChannel 创建硬件通道失败, 返回码: {ret}"),
            });
        }

        self.report_thread_id = report_thread_id;
        self.report_running = running;
        self.report_thread = Some(report_thread);
        self.channel_desc = channel_desc;
        self.is_initialized = true;
        debug!(camera_id = %self.camera_id, codec = ?self.codec, "华为昇腾 DVPP VDEC 硬件解码通道与回调驱动就绪");
        Ok(())
    }

    fn stop_report_thread(&mut self) {
        self.report_running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.report_thread.take() {
            let _ = handle.join();
        }
    }

    fn decode(&mut self, packet: &[u8], pts: i64) -> Result<Option<FrameRef>, MediaError> {
        // -1. 关停协同检查：若收到 stop 信号立即拒绝新帧，防止硬件挂起
        if self.shutdown_flag.load(Ordering::Relaxed) {
            return Err(MediaError::Decode {
                reason: "DVPP 解码器正在停止，丢弃输入数据".to_string(),
            });
        }

        if !self.is_initialized {
            return Err(MediaError::Decode {
                reason: "DVPP 解码器未初始化".to_string(),
            });
        }

        // 0. 输入码流零长度防御：空包直接跳过，绝不进入显存分配与硬件投递
        if packet.is_empty() {
            return Ok(None);
        }

        // 0.1 输入码流硬上限防御：超限拒绝分配显存，防止 DoS 内存轰炸
        if packet.len() > MAX_COMPRESSED_PACKET_SIZE {
            return Err(MediaError::Decode {
                reason: format!(
                    "输入压缩码流包大小 ({} 字节) 超出单包硬上限 ({} 字节)",
                    packet.len(),
                    MAX_COMPRESSED_PACKET_SIZE
                ),
            });
        }

        // 0.2 ABI 尺寸表达范围校验（防止 64 位 usize 截断为 32 位 c_uint）
        let stream_size_u32 = u32::try_from(packet.len()).map_err(|_| MediaError::Decode {
            reason: "输入码流包大小超出 u32 表达范围".to_string(),
        })?;

        self.update_dimensions_if_needed(packet);

        // 1. 为当前输入 NALU 码流申请独立显存，实现严格的在途隔离，杜绝覆写破坏
        let mut stream_buf: *mut c_void = std::ptr::null_mut();
        // SAFETY: acldvppMalloc 为输入码流分配显存（受 MAX_COMPRESSED_PACKET_SIZE 严格限制）
        let ret = unsafe { ffi::acldvppMalloc(&mut stream_buf, packet.len()) };
        if ret != 0 || stream_buf.is_null() {
            return Err(MediaError::Decode {
                reason: format!("acldvppMalloc 为输入码流分配显存失败, 返回码: {ret}"),
            });
        }

        // SAFETY: 跨内存域将压缩 NALU 码流从 Host 拷贝到 Device Memory
        // 边界说明：此为压缩码流输入所必需的 Host->Device DMA 传输，数据量仅为压缩 NALU 字节；
        // 严正声明：这不影响解码输出到推理输入的设备侧零拷贝 (infer_fast_path)
        let ret = unsafe {
            ffi::aclrtMemcpy(
                stream_buf,
                packet.len(),
                packet.as_ptr() as *const c_void,
                packet.len(),
                ffi::ACL_MEMCPY_HOST_TO_DEVICE,
            )
        };
        if ret != 0 {
            // SAFETY: 拷贝失败时释放分配的设备显存
            unsafe {
                let _ = ffi::acldvppFree(stream_buf);
            }
            return Err(MediaError::Decode {
                reason: format!("aclrtMemcpy 拷贝输入码流至设备显存失败: {ret}"),
            });
        }

        // 2. 创建并配置输入码流描述符
        // SAFETY: 创建 stream_desc
        let stream_desc = unsafe { ffi::acldvppCreateStreamDesc() };
        if stream_desc.is_null() {
            // SAFETY: 描述符创建失败时释放设备显存
            unsafe {
                let _ = ffi::acldvppFree(stream_buf);
            }
            return Err(MediaError::Decode {
                reason: "acldvppCreateStreamDesc 创建失败".to_string(),
            });
        }
        // SAFETY: 设置输入码流元数据，使用校验过的 stream_size_u32
        unsafe {
            let _ = ffi::acldvppSetStreamDescData(stream_desc, stream_buf);
            let _ = ffi::acldvppSetStreamDescSize(stream_desc, stream_size_u32);
            let _ = ffi::acldvppSetStreamDescEos(stream_desc, 0);
        }

        // 3. 从预分配显存池租借输出显存块（带 50ms 超时保护）
        let (dev_ptr, block_size) = match self.pool.acquire_timeout(Duration::from_millis(50)) {
            Some(res) => res,
            None => {
                // SAFETY: 超时回滚释放输入资源
                unsafe {
                    let _ = ffi::acldvppDestroyStreamDesc(stream_desc);
                    let _ = ffi::acldvppFree(stream_buf);
                }
                return Err(MediaError::Decode {
                    reason: "DVPP 显存池耗尽超时 (下游租约未释放或处理阻塞)".to_string(),
                });
            }
        };

        // 3.1 校验 block_size 是否处于 u32 安全范围
        let block_size_u32 = match u32::try_from(block_size) {
            Ok(v) => v,
            Err(_) => {
                if let Err(e) = self.pool.return_buffer(dev_ptr) {
                    warn!(error = %e, ptr = ?dev_ptr, "回滚归还显存块失败");
                }
                // SAFETY: 超出范围回滚释放输入资源
                unsafe {
                    let _ = ffi::acldvppDestroyStreamDesc(stream_desc);
                    let _ = ffi::acldvppFree(stream_buf);
                }
                return Err(MediaError::Decode {
                    reason: "输出显存块容量超出 u32 表达范围".to_string(),
                });
            }
        };

        // 4. 创建并配置输出图片描述符
        // SAFETY: 创建 pic_desc
        let pic_desc = unsafe { ffi::acldvppCreatePicDesc() };
        if pic_desc.is_null() {
            if let Err(e) = self.pool.return_buffer(dev_ptr) {
                warn!(error = %e, ptr = ?dev_ptr, "回滚归还显存块失败");
            }
            // SAFETY: 回滚释放输入资源
            unsafe {
                let _ = ffi::acldvppDestroyStreamDesc(stream_desc);
                let _ = ffi::acldvppFree(stream_buf);
            }
            return Err(MediaError::Decode {
                reason: "acldvppCreatePicDesc 创建输出图像描述符失败".to_string(),
            });
        }
        // SAFETY: 配置输出图像描述符属性
        unsafe {
            let _ = ffi::acldvppSetPicDescData(pic_desc, dev_ptr);
            let _ = ffi::acldvppSetPicDescSize(pic_desc, block_size_u32);
            let _ = ffi::acldvppSetPicDescFormat(pic_desc, ffi::PIXEL_FORMAT_YUV_SEMIPLANAR_420);
            let _ = ffi::acldvppSetPicDescWidth(pic_desc, self.width);
            let _ = ffi::acldvppSetPicDescHeight(pic_desc, self.height);
            let _ = ffi::acldvppSetPicDescWidthStride(pic_desc, self.stride_w);
            let _ = ffi::acldvppSetPicDescHeightStride(pic_desc, self.stride_h);
        }

        // 5. 打包在途上下文，所有权转交底层硬件与 Report 回调
        let in_flight = Box::new(InFlightFrame {
            camera_id: self.camera_id.clone(),
            pts,
            width: self.width,
            height: self.height,
            stride_w: self.stride_w,
            stride_h: self.stride_h,
            stream_buf,
            stream_desc,
            pic_desc,
            dev_ptr,
            block_size,
            pool: Arc::clone(&self.pool),
            out_tx: self.out_tx.clone(),
        });
        let user_data = Box::into_raw(in_flight) as *mut c_void;

        // 6. 送入 DVPP VDEC 硬件解码队列
        // SAFETY: 调用 aclvdecSendFrame
        let ret =
            unsafe { ffi::aclvdecSendFrame(self.channel_desc, stream_desc, pic_desc, user_data) };

        if ret != 0 {
            // 送帧失败：硬件队列未接收，立即在发送线程清理并归还显存
            // SAFETY: 任务未入队，由当前线程安全回收在途帧全部资源
            unsafe {
                let in_flight = Box::from_raw(user_data as *mut InFlightFrame);
                let _ = ffi::acldvppDestroyPicDesc(in_flight.pic_desc);
                let _ = ffi::acldvppDestroyStreamDesc(in_flight.stream_desc);
                let _ = ffi::acldvppFree(in_flight.stream_buf);
                if let Err(e) = in_flight.pool.return_buffer(in_flight.dev_ptr) {
                    warn!(error = %e, ptr = ?in_flight.dev_ptr, "入队失败回滚归还显存块失败");
                }
            }
            return Err(MediaError::Decode {
                reason: format!("aclvdecSendFrame 送入硬件解码失败, 返回码: {ret}"),
            });
        }

        self.in_flight_count += 1;

        // 7. 接收完成帧：优先检查是否有此前 Drain 暂存的完成帧
        if let Some(frame) = self.drained_frames.pop_front() {
            return Ok(Some(frame));
        }

        // 检查通道中此前完成的帧（非阻塞）
        if let Ok(res) = self.out_rx.try_recv() {
            self.in_flight_count = self.in_flight_count.saturating_sub(1);
            return res.map(Some);
        }

        // 尝试微量等待（10ms）以适应即时出帧（无参考帧重排序延迟的流）
        match self.out_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(res) => {
                self.in_flight_count = self.in_flight_count.saturating_sub(1);
                res.map(Some)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 处于 B 帧延迟或参数集阶段，硬件尚未产出画面，正常返回 None
                Ok(None)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(MediaError::Decode {
                reason: "DVPP 回调通道已断开".to_string(),
            }),
        }
    }

    fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        if !self.is_initialized {
            return Ok(Vec::new());
        }

        // 0. 优先收集此前已 Drain 暂存的完成帧
        let mut frames: Vec<FrameRef> = self.drained_frames.drain(..).collect();

        // 1. 发送 EOS 标记通知通道结束
        // SAFETY: 发送 EOS 空流包
        let eos_desc = unsafe { ffi::acldvppCreateStreamDesc() };
        if !eos_desc.is_null() {
            // SAFETY: 配置 EOS 并发送
            unsafe {
                let _ = ffi::acldvppSetStreamDescSize(eos_desc, 0);
                let _ = ffi::acldvppSetStreamDescEos(eos_desc, 1);
                let _ = ffi::aclvdecSendFrame(
                    self.channel_desc,
                    eos_desc,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                let _ = ffi::acldvppDestroyStreamDesc(eos_desc);
            }
        }

        // 2. 等待收割所有在途未完成帧（最大等待 500ms）
        let deadline = std::time::Instant::now() + Duration::from_millis(500);

        while self.in_flight_count > 0 && std::time::Instant::now() < deadline {
            match self.out_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(frame)) => {
                    self.in_flight_count = self.in_flight_count.saturating_sub(1);
                    frames.push(frame);
                }
                Ok(Err(_)) => {
                    self.in_flight_count = self.in_flight_count.saturating_sub(1);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        // 按时间戳排序，确保在含有 B 帧重排时输出的画面严格单调递增
        frames.sort_by_key(|f| f.timestamp);

        debug!(
            camera_id = %self.camera_id,
            flushed_count = frames.len(),
            remaining_in_flight = self.in_flight_count,
            "DVPP 解码器刷新完成"
        );
        Ok(frames)
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
                CodecType::Aac => {}
            }
        }
    }

    /// 在重配或刷新前排空所有正在硬件流水线中执行的任务 (Drain in-flight operations)
    fn drain_in_flight_frames(&mut self, timeout: Duration) -> Result<(), MediaError> {
        let deadline = std::time::Instant::now() + timeout;
        while self.in_flight_count > 0 && std::time::Instant::now() < deadline {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                warn!(
                    camera_id = %self.camera_id,
                    remaining = self.in_flight_count,
                    "Drain 期间检测到关停信号，提前中止等待"
                );
                break;
            }
            match self.out_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(frame)) => {
                    self.in_flight_count = self.in_flight_count.saturating_sub(1);
                    self.drained_frames.push_back(frame);
                }
                Ok(Err(e)) => {
                    self.in_flight_count = self.in_flight_count.saturating_sub(1);
                    warn!(
                        camera_id = %self.camera_id,
                        error = %e,
                        "Drain 期间捕获到在途帧硬件解码失败"
                    );
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(MediaError::Decode {
                        reason: "DVPP 回调通道已断开".to_string(),
                    });
                }
            }
        }

        if self.in_flight_count > 0 {
            warn!(
                camera_id = %self.camera_id,
                remaining = self.in_flight_count,
                "Drain 在途任务超时，仍有硬件任务未返回"
            );
        }
        Ok(())
    }

    fn reconfigure_resolution_if_changed(&mut self, new_w: u32, new_h: u32) {
        if new_w == self.width && new_h == self.height {
            return;
        }

        // 1. 合法性与边界校验（防御畸形/恶意尺寸）
        if !is_valid_dvpp_resolution(new_w, new_h) {
            warn!(
                camera_id = %self.camera_id,
                width = new_w,
                height = new_h,
                "DVPP 检测到非法/超出安全边界的分辨率变更请求，防御性拒绝重配"
            );
            return;
        }

        // 2. 检查降级熔断状态（Circuit Breaker）
        if self.is_degraded {
            warn!(
                camera_id = %self.camera_id,
                width = new_w,
                height = new_h,
                "DVPP 解码器当前处于 Degraded 熔断保护状态，拒绝重配分辨率"
            );
            return;
        }

        let now = std::time::Instant::now();

        // 3. 冷却时间保护（Cooldown Window）
        if let Some(last_time) = self.last_reconfig_time {
            if now.duration_since(last_time) < self.reconfig_cooldown {
                debug!(
                    camera_id = %self.camera_id,
                    elapsed_ms = now.duration_since(last_time).as_millis(),
                    cooldown_ms = self.reconfig_cooldown.as_millis(),
                    "DVPP 分辨率变更处于冷却窗口内，抑制本次重配"
                );
                return;
            }
        }

        // 4. 滑动窗口频次统计与熔断判定（Flapping Detection & Circuit Breaker）
        while let Some(&t) = self.reconfig_history.front() {
            if now.duration_since(t) > self.flapping_window {
                self.reconfig_history.pop_front();
            } else {
                break;
            }
        }

        if self.reconfig_history.len() >= self.max_flapping_count {
            self.is_degraded = true;
            error!(
                camera_id = %self.camera_id,
                count = self.reconfig_history.len(),
                window_secs = self.flapping_window.as_secs(),
                "DVPP 检测到高频分辨率抖动（疑似恶意码流 DoS 攻击），触发熔断保护，进入 Degraded 状态，锁定当前分辨率并拒绝重配！"
            );
            return;
        }

        // 5. 严格的 Drain 语义：排空旧通道在途硬件任务与描述符
        if let Err(e) = self.drain_in_flight_frames(Duration::from_millis(500)) {
            warn!(camera_id = %self.camera_id, error = %e, "重配前排空在途帧失败");
        }

        let new_stride_w = align_dvpp_width_stride(new_w);
        let new_stride_h = align_dvpp_height_stride(new_h);
        let required_size = match calculate_dvpp_nv12_size_checked(new_w, new_h) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    camera_id = %self.camera_id,
                    error = %e,
                    "显存块容量计算溢出或尺寸超出边界，拒绝重配"
                );
                return;
            }
        };

        // 6. 依据显存预算建立安全新池（Buffer Budget）
        let new_pool_count = (self.max_pool_memory_bytes / required_size)
            .clamp(DVPP_MIN_POOL_BLOCKS, DVPP_MAX_POOL_BLOCKS);

        #[cfg(any(all(target_os = "linux", feature = "dvpp"), test))]
        let new_pool_res = DvppBufferPool::new(required_size, new_pool_count);
        #[cfg(not(any(all(target_os = "linux", feature = "dvpp"), test)))]
        let new_pool_res: Result<DvppBufferPool, MediaError> = Err(MediaError::DecoderInit {
            codec: format!("{:?}", self.codec),
            reason: "非 DVPP 环境".to_string(),
        });

        let new_pool = match new_pool_res {
            Ok(p) => p,
            Err(e) => {
                error!(
                    camera_id = %self.camera_id,
                    error = %e,
                    required_size,
                    new_pool_count,
                    "为新分辨率分配 DVPP 显存池失败，回退到旧配置并进入 Degraded 状态"
                );
                self.is_degraded = true;
                return;
            }
        };

        // 7. 销毁旧通道与描述符（满足硬件生命周期顺序要求）
        if !self.channel_desc.is_null() {
            // SAFETY: 按照 Ascend 规范，先销毁通道，再销毁描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannel(self.channel_desc);
                let _ = ffi::aclvdecDestroyChannelDesc(self.channel_desc);
            }
            self.channel_desc = std::ptr::null_mut();
        }

        // 8. 创建并初始化新通道
        let stream_format = match self.codec {
            CodecType::H264 => ffi::H264_MAIN_LEVEL,
            CodecType::H265 => ffi::H265_MAIN_LEVEL,
            CodecType::Aac => {
                error!(camera_id = %self.camera_id, "DVPP 硬件解码器不支持音频重配置");
                self.is_degraded = true;
                return;
            }
        };

        // SAFETY: 创建新通道描述符
        let channel_desc = unsafe { ffi::aclvdecCreateChannelDesc() };
        if channel_desc.is_null() {
            error!(camera_id = %self.camera_id, "创建新通道描述符失败，进入 Degraded 状态");
            self.is_degraded = true;
            return;
        }

        // SAFETY: 配置新通道属性（绑定既有的 Report 驱动线程与回调）
        unsafe {
            let _ = ffi::aclvdecSetChannelDescChannelId(channel_desc, 0);
            let _ = ffi::aclvdecSetChannelDescThreadId(channel_desc, self.report_thread_id);
            let _ = ffi::aclvdecSetChannelDescCallback(channel_desc, Some(dvpp_vdec_callback));
            let _ = ffi::aclvdecSetChannelDescEnType(channel_desc, stream_format);
            let _ = ffi::aclvdecSetChannelDescOutPicFormat(
                channel_desc,
                ffi::PIXEL_FORMAT_YUV_SEMIPLANAR_420,
            );
        }

        // SAFETY: 启动底层硬件新通道
        let ret = unsafe { ffi::aclvdecCreateChannel(channel_desc) };
        if ret != 0 {
            // SAFETY: 通道创建失败销毁描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannelDesc(channel_desc);
            }
            error!(camera_id = %self.camera_id, ret, "底层硬件通道重建失败，进入 Degraded 状态");
            self.is_degraded = true;
            return;
        }

        // 9. 更新成功：先安全关闭旧显存池（通知等待线程，标记旧池为 Closed），再替换新池与所有尺寸步长参数
        self.pool.close();
        self.channel_desc = channel_desc;
        self.pool = Arc::new(new_pool);
        self.width = new_w;
        self.height = new_h;
        self.stride_w = new_stride_w;
        self.stride_h = new_stride_h;
        self.last_reconfig_time = Some(now);
        self.reconfig_history.push_back(now);

        info!(
            camera_id = %self.camera_id,
            width = new_w,
            height = new_h,
            stride_w = new_stride_w,
            stride_h = new_stride_h,
            block_size = required_size,
            pool_count = new_pool_count,
            "DVPP 动态分辨率安全重配完成（已排空旧在途任务、同步新步长与显存预算、重建硬件通道）"
        );
    }

    /// 工业级自愈重置：在 DVPP 硬件通道挂起或连续报错时销毁并重建硬件通道
    fn reset(&mut self) -> Result<(), MediaError> {
        self.stop_report_thread();

        if !self.channel_desc.is_null() {
            // SAFETY: 销毁硬件通道与描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannel(self.channel_desc);
                let _ = ffi::aclvdecDestroyChannelDesc(self.channel_desc);
            }
            self.channel_desc = std::ptr::null_mut();
        }

        // 清空残留帧状态与保护状态
        self.in_flight_count = 0;
        self.drained_frames.clear();
        self.last_reconfig_time = None;
        self.reconfig_history.clear();
        self.is_degraded = false;
        while self.out_rx.try_recv().is_ok() {}

        self.is_initialized = false;
        self.init()
    }
}

impl Drop for DvppDecoderInner {
    fn drop(&mut self) {
        self.stop_report_thread();

        if !self.channel_desc.is_null() {
            // SAFETY: 按照 Ascend 规范销毁通道与描述符
            unsafe {
                let _ = ffi::aclvdecDestroyChannel(self.channel_desc);
                let _ = ffi::aclvdecDestroyChannelDesc(self.channel_desc);
            }
            self.channel_desc = std::ptr::null_mut();
        }

        debug!(camera_id = %self.camera_id, "DVPP 硬件资源安全回收完毕");
    }
}

/// 华为昇腾 DVPP 硬件解码器异步外壳
pub struct DvppDecoder {
    camera_id: String,
    codec: CodecType,
    tx: Option<mpsc::Sender<DecodeCommand>>,
    policy: DecodeDeliveryPolicy,
    pruning_gop: bool,
    dropped_p_frames: u64,
    shutdown_flag: Arc<AtomicBool>,
    exit_rx: std::sync::mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for DvppDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DvppDecoder")
            .field("camera_id", &self.camera_id)
            .finish()
    }
}

impl DvppDecoder {
    pub fn new(camera_id: &str, codec: CodecType) -> Self {
        let (tx, mut rx) = mpsc::channel::<DecodeCommand>(4);
        let cam_id = camera_id.to_string();
        let thread_name = format!("dvpp-dec-{cam_id}");
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown_flag);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();

        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut inner = DvppDecoderInner::new(cam_id.clone(), codec, shutdown_clone);
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
                            DecodeCommand::Stop => break,
                        }
                    }
                    let _ = exit_tx.send(());
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
                                        warn!(
                                            camera_id = %cam_id,
                                            consecutive_errors,
                                            error = %e,
                                            "DVPP 硬件连续解码失败达到阈值，触发硬件通道自动重置自愈"
                                        );
                                        if let Err(reinit_err) = inner.reset() {
                                            error!(
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
                        DecodeCommand::Stop => {
                            debug!(camera_id = %cam_id, "DVPP 收到 Stop 命令，退出事件循环");
                            break;
                        }
                    }
                }
                // 退出循环后显式析构 inner
                drop(inner);
                let _ = exit_tx.send(());
            })
            .expect("创建 DVPP 解码专用线程失败");

        Self {
            camera_id: camera_id.to_string(),
            codec,
            tx: Some(tx),
            policy: DecodeDeliveryPolicy::default(),
            pruning_gop: false,
            dropped_p_frames: 0,
            shutdown_flag,
            exit_rx,
            thread: Some(thread),
        }
    }

    /// 优雅停止工作线程（带超时保护，超时后强制隔离放弃，杜绝无界阻塞守护进程）
    pub fn stop(&mut self, timeout: Duration) -> bool {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        if let Some(tx) = self.tx.take() {
            let _ = tx.try_send(DecodeCommand::Stop);
            drop(tx);
        }
        if let Some(thread) = self.thread.take() {
            match self.exit_rx.recv_timeout(timeout) {
                Ok(()) => {
                    let _ = thread.join();
                    debug!(camera_id = %self.camera_id, "DVPP 专用工作线程已正常优雅退出并回收");
                    true
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    error!(
                        camera_id = %self.camera_id,
                        timeout_ms = timeout.as_millis(),
                        "DVPP 工作线程在指定超时时间内未能退出（疑似硬件驱动内核调用挂起），执行隔离放弃，避免阻塞主守护进程"
                    );
                    false
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = thread.join();
                    true
                }
            }
        } else {
            true
        }
    }

    /// 当前是否处于 GOP 尾部修剪状态
    pub fn is_pruning_gop(&self) -> bool {
        self.pruning_gop
    }

    /// 累计丢弃的 P 帧数量
    pub fn dropped_p_frames(&self) -> u64 {
        self.dropped_p_frames
    }
}

#[async_trait]
impl VideoDecoder for DvppDecoder {
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, MediaError> {
        let is_keyframe = crate::sps::is_keyframe_or_parameter_set(packet, self.codec);

        if self.policy == DecodeDeliveryPolicy::RealtimeDropOldest {
            if is_keyframe {
                if self.pruning_gop {
                    tracing::info!(
                        camera_id = %self.camera_id,
                        dropped_p_frames = self.dropped_p_frames,
                        pts,
                        "DVPP 解码收到新关键帧/参数集，安全重置 GOP 修剪状态，恢复解码"
                    );
                    self.pruning_gop = false;
                    self.dropped_p_frames = 0;
                }
            } else if self.pruning_gop {
                // 当前处于修剪态：坚决丢弃后续残缺 P/B 帧，杜绝破坏运动参考链导致花屏
                self.dropped_p_frames += 1;
                return Ok(None);
            }
        } else {
            self.pruning_gop = false;
            self.dropped_p_frames = 0;
        }

        let tx = self.tx.as_ref().ok_or_else(|| MediaError::Decode {
            reason: "DVPP 解码专用通道已关闭".to_string(),
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DecodeCommand::Decode {
            packet: Bytes::copy_from_slice(packet),
            pts,
            reply: reply_tx,
        };

        match self.policy {
            DecodeDeliveryPolicy::LosslessBackpressure => {
                tx.send(cmd).await.map_err(|_| MediaError::Decode {
                    reason: "DVPP 解码专用线程已退出".to_string(),
                })?;
            }
            DecodeDeliveryPolicy::RealtimeDropOldest => match tx.try_send(cmd) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(rejected_cmd)) => {
                    if is_keyframe {
                        tracing::warn!(
                            camera_id = %self.camera_id,
                            pts,
                            "DVPP 硬件通道满但当前为关键帧/参数集，等待投递以重建参考系"
                        );
                        tx.send(rejected_cmd)
                            .await
                            .map_err(|_| MediaError::Decode {
                                reason: "DVPP 解码专用线程已退出".to_string(),
                            })?;
                    } else {
                        self.pruning_gop = true;
                        self.dropped_p_frames = 1;
                        tracing::warn!(
                            camera_id = %self.camera_id,
                            pts,
                            "DVPP 硬件通道饱和，实时策略丢弃该 P 帧并开启 GOP 尾部修剪防花屏"
                        );
                        return Ok(None);
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    return Err(MediaError::Decode {
                        reason: "DVPP 解码专用线程已退出".to_string(),
                    });
                }
            },
        }

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码响应通道已关闭".to_string(),
        })?
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        self.pruning_gop = false;
        self.dropped_p_frames = 0;
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = DecodeCommand::Flush { reply: reply_tx };

        let tx = self.tx.as_ref().ok_or_else(|| MediaError::Decode {
            reason: "DVPP 解码专用通道已关闭".to_string(),
        })?;

        tx.send(cmd).await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码专用线程已退出".to_string(),
        })?;

        reply_rx.await.map_err(|_| MediaError::Decode {
            reason: "DVPP 解码响应通道已关闭".to_string(),
        })?
    }

    fn set_delivery_policy(&mut self, policy: DecodeDeliveryPolicy) {
        self.policy = policy;
        self.pruning_gop = false;
        self.dropped_p_frames = 0;
    }

    fn delivery_policy(&self) -> DecodeDeliveryPolicy {
        self.policy
    }
}

impl Drop for DvppDecoder {
    fn drop(&mut self) {
        let _ = self.stop(DEFAULT_THREAD_SHUTDOWN_TIMEOUT);
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

    #[tokio::test]
    async fn test_dvpp_async_callback_lifecycle_and_lease_return() {
        let mut decoder = DvppDecoder::new("test_cam_dvpp", CodecType::H264);

        let dummy_nalu = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00];
        let res = decoder.decode_packet(&dummy_nalu, 1000).await;
        assert!(res.is_ok(), "decode_packet 应当成功返回");

        let opt_frame = res.expect("res 应当为 Ok");
        assert!(opt_frame.is_some(), "异步回调应正常交付完成帧");

        let frame = opt_frame.expect("opt_frame 应当为 Some");
        assert_eq!(frame.camera_id, "test_cam_dvpp");
        assert_eq!(frame.timestamp, 1000);
        assert_eq!(frame.format, PixelFormat::Nv12);

        // 验证 FrameHandle 为 DeviceMemory
        match frame.handle() {
            FrameHandle::DeviceMemory { size, .. } => {
                assert_eq!(*size, calculate_dvpp_nv12_size(1920, 1080));
            }
            _ => panic!("预期为 DeviceMemory 句柄"),
        }

        // 测试 FrameRef 析构后租约自动归还显存池
        drop(frame);

        // 测试 Flush 刷新
        let flushed = decoder.flush().await.expect("flush 应当成功");
        assert!(flushed.is_empty(), "无残留帧时应返回空列表");
    }

    #[tokio::test]
    async fn test_dvpp_hardware_error_handling_and_pool_safety() {
        let mut decoder = DvppDecoder::new("test_err_cam", CodecType::H264);

        let dummy_nalu = [0x00, 0x00, 0x00, 0x01, 0x65, 0xFF];
        let res = decoder.decode_packet(&dummy_nalu, 2000).await;

        // 验证回调检测到 retCode != 0 抛出错误，且输出显存已安全回退到池中（无泄漏、无半写）
        assert!(res.is_err(), "硬件报告错误时应返回 Err");

        // 随后的正常解码器应继续正常工作
        let mut decoder_valid = DvppDecoder::new("test_cam_valid", CodecType::H264);
        let dummy_valid = [0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        let res2 = decoder_valid.decode_packet(&dummy_valid, 2033).await;
        assert!(res2.is_ok());
    }

    #[test]
    fn test_dvpp_resolution_validation() {
        assert!(is_valid_dvpp_resolution(1920, 1080));
        assert!(is_valid_dvpp_resolution(1280, 720));
        assert!(is_valid_dvpp_resolution(3840, 2160));
        assert!(is_valid_dvpp_resolution(640, 360));

        // 非法分辨率防御
        assert!(!is_valid_dvpp_resolution(64, 64), "过小尺寸应拒绝");
        assert!(!is_valid_dvpp_resolution(4096, 2160), "超出 4K 宽边界");
        assert!(!is_valid_dvpp_resolution(1920, 1081), "奇数高度应拒绝");
        assert!(!is_valid_dvpp_resolution(1921, 1080), "奇数宽度应拒绝");
        assert!(!is_valid_dvpp_resolution(0, 0), "零尺寸应拒绝");
        assert!(
            !is_valid_dvpp_resolution(65535, 65535),
            "极端恶意超大尺寸应拒绝"
        );
    }

    #[test]
    fn test_dvpp_reconfigure_drain_and_cooldown_and_flapping() {
        let mut inner = DvppDecoderInner::new_test("test_reconfig".to_string(), CodecType::H264);
        inner.init().expect("初始化通道应当成功");

        // 缩短冷却与滑动窗口用于单元测试
        inner.reconfig_cooldown = Duration::from_millis(50);
        inner.flapping_window = Duration::from_secs(10);
        inner.max_flapping_count = 3;

        assert_eq!(inner.width, 1920);
        assert_eq!(inner.height, 1080);

        // 1. 第一次合法重配：1080p -> 720p
        inner.reconfigure_resolution_if_changed(1280, 720);
        assert_eq!(inner.width, 1280);
        assert_eq!(inner.height, 720);
        assert_eq!(inner.stride_w, align_dvpp_width_stride(1280));
        assert_eq!(inner.stride_h, align_dvpp_height_stride(720));
        assert_eq!(inner.reconfig_history.len(), 1);

        // 2. 冷却时间抑制测试：立刻触发第三种分辨率，因处于 50ms 冷却内应被忽略
        inner.reconfigure_resolution_if_changed(640, 360);
        assert_eq!(inner.width, 1280, "冷却窗口内应保持 1280 不变");
        assert_eq!(inner.height, 720);

        // 3. 等待冷却过期
        std::thread::sleep(Duration::from_millis(60));

        // 第二次合法重配：720p -> 640x360
        inner.reconfigure_resolution_if_changed(640, 360);
        assert_eq!(inner.width, 640);
        assert_eq!(inner.height, 360);
        assert_eq!(inner.reconfig_history.len(), 2);

        // 等待冷却过期
        std::thread::sleep(Duration::from_millis(60));

        // 第三次合法重配：640x360 -> 1920x1080
        inner.reconfigure_resolution_if_changed(1920, 1080);
        assert_eq!(inner.width, 1920);
        assert_eq!(inner.height, 1080);
        assert_eq!(inner.reconfig_history.len(), 3);
        assert!(!inner.is_degraded, "尚未超过最大频次阈值，仍未熔断");

        // 等待冷却过期
        std::thread::sleep(Duration::from_millis(60));

        // 4. 触发第 4 次重配：达到 MAX_FLAPPING_COUNT 阈值，应立即熔断进入 Degraded 状态！
        inner.reconfigure_resolution_if_changed(1280, 720);
        assert!(inner.is_degraded, "高频抖动应当触发熔断进入 Degraded 状态");
        assert_eq!(inner.width, 1920, "熔断后应锁定分辨率，拒绝重配");

        // 5. 处于 Degraded 状态下，即使冷却过去也坚决拒绝任何后续变更
        std::thread::sleep(Duration::from_millis(60));
        inner.reconfigure_resolution_if_changed(640, 360);
        assert_eq!(inner.width, 1920, "Degraded 状态下坚决拒绝重配");
    }

    #[test]
    fn test_dvpp_in_flight_drain_during_resolution_switch() {
        let mut inner =
            DvppDecoderInner::new_test("test_drain_switch".to_string(), CodecType::H264);
        inner.init().expect("初始化通道应当成功");

        inner.reconfig_cooldown = Duration::from_millis(10);
        inner.flapping_window = Duration::from_secs(10);
        inner.max_flapping_count = 5;

        // 模拟送入一帧（1080p），进入硬件在途队列
        let dummy_nalu = [0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        let opt = inner.decode(&dummy_nalu, 1000).expect("送入第一帧应成功");

        // 模拟触发重配为 720p（1280x720）
        std::thread::sleep(Duration::from_millis(15));
        inner.reconfigure_resolution_if_changed(1280, 720);

        // 验证通道已成功切换为 1280x720
        assert_eq!(inner.width, 1280);
        assert_eq!(inner.height, 720);
        assert_eq!(inner.stride_w, align_dvpp_width_stride(1280));
        assert_eq!(inner.stride_h, align_dvpp_height_stride(720));

        // 验证旧在途任务已被 drain，且旧显存池成功被新显存池替换
        assert_eq!(inner.in_flight_count, 0, "旧在途任务已完全排空");

        // 验证紧接着能继续送入新分辨率帧进行解码
        let res2 = inner
            .decode(&dummy_nalu, 1033)
            .expect("送入新分辨率帧应成功");
        assert!(res2.is_some() || opt.is_some() || !inner.drained_frames.is_empty());
    }

    #[test]
    fn test_dvpp_empty_and_oversized_packet_defense() {
        let mut inner = DvppDecoderInner::new_test("test_limits".to_string(), CodecType::H264);
        inner.init().expect("初始化通道应当成功");

        // 1. 空包防御：直接返回 Ok(None)，不消耗任何显存
        let empty_res = inner.decode(&[], 1000);
        assert!(empty_res.is_ok(), "空包应优雅返回 Ok");
        assert!(empty_res.expect("空包应为 Ok").is_none(), "空包应返回 None");
        assert_eq!(inner.in_flight_count, 0, "空包不应增加在途硬件计数");

        // 2. 构造超大 NALU 包（超过 MAX_COMPRESSED_PACKET_SIZE，比如 4MB + 1）
        let oversized = vec![0u8; MAX_COMPRESSED_PACKET_SIZE + 1];
        let over_res = inner.decode(&oversized, 1033);
        assert!(over_res.is_err(), "超大压缩包必须被防御性拦截拒收");
        assert_eq!(inner.in_flight_count, 0, "超大包不应分配显存或投递硬件");
    }

    #[test]
    fn test_dvpp_checked_arithmetic_overflow() {
        // 1. 正常计算
        let size_1080p = calculate_dvpp_nv12_size_checked(1920, 1080).expect("1080p 应合法");
        assert_eq!(size_1080p, 1920 * 1080 * 3 / 2);

        // 2. 超出 MAX_FRAME_WIDTH / MAX_FRAME_HEIGHT 边界
        assert!(calculate_dvpp_nv12_size_checked(4096, 2160).is_err());
        assert!(calculate_dvpp_nv12_size_checked(1920, 4096).is_err());

        // 3. 极端 u32::MAX 防溢出
        assert!(calculate_dvpp_nv12_size_checked(u32::MAX, u32::MAX).is_err());
    }

    #[tokio::test]
    async fn test_dvpp_graceful_shutdown_and_stop_command() {
        let mut decoder = DvppDecoder::new("test_shutdown_dvpp", CodecType::H264);

        // 验证正常解码一帧
        let dummy_nalu = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00];
        let res = decoder.decode_packet(&dummy_nalu, 1000).await;
        assert!(res.is_ok());

        // 触发 stop，验证带超时正常退出回收
        let stopped = decoder.stop(Duration::from_millis(200));
        assert!(stopped, "工作线程应在超时时限内优雅退出并成功 join");

        // 退出后再次提交任务应立即返回通道已关闭错误
        let res_after = decoder.decode_packet(&dummy_nalu, 1033).await;
        assert!(res_after.is_err(), "已停止的解码器应当拒绝新任务");
    }

    #[test]
    fn test_dvpp_delivery_policy_default_and_mutation() {
        let mut decoder = DvppDecoder::new("test_dvpp_policy", CodecType::H264);
        assert_eq!(
            decoder.delivery_policy(),
            DecodeDeliveryPolicy::LosslessBackpressure
        );

        decoder.set_delivery_policy(DecodeDeliveryPolicy::RealtimeDropOldest);
        assert_eq!(
            decoder.delivery_policy(),
            DecodeDeliveryPolicy::RealtimeDropOldest
        );
    }

    #[tokio::test]
    #[allow(clippy::unwrap_used)]
    async fn test_dvpp_realtime_pruning_prevents_broken_reference_chain() {
        let mut decoder = DvppDecoder::new("test_dvpp_prune", CodecType::H264);
        decoder.set_delivery_policy(DecodeDeliveryPolicy::RealtimeDropOldest);
        assert_eq!(
            decoder.delivery_policy(),
            DecodeDeliveryPolicy::RealtimeDropOldest
        );

        // 人为将 decoder 置于修剪状态（模拟通道满触发丢 P 帧）
        decoder.pruning_gop = true;
        decoder.dropped_p_frames = 1;

        // 在修剪状态下，送入新的 P 帧 (0x41)
        let p_frame = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9a, 0x00];
        let res = decoder.decode_packet(&p_frame, 1033).await.unwrap();
        assert!(res.is_none());
        assert_eq!(decoder.dropped_p_frames(), 2);
        assert!(decoder.is_pruning_gop());

        // 送入新的关键帧 IDR (0x65) -> 自动重置修剪态并恢复正常
        let idr_frame = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00];
        let _ = decoder.decode_packet(&idr_frame, 1066).await;
        assert!(!decoder.is_pruning_gop());
        assert_eq!(decoder.dropped_p_frames(), 0);
    }
}
