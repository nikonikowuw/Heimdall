//! C ABI 类型 1:1 映射
//! 严格匹配 `sdk/include/argus/algo.h`, `types.h`, `result.h`。
//! 必须保持 64 位平台 8 字节对齐。

use std::ffi::{c_char, c_int, c_void};

pub const AV_ALGO_API_VERSION: u32 = 1;
pub const AV_ALGO_GET_ABI_SYMBOL: &[u8] = b"av_algo_get_abi\0";
pub const AV_ALGO_EXTRACT_FACE_SYMBOL: &[u8] = b"av_algo_extract_face\0";

/// 算法包状态码
pub const AV_OK: c_int = 0;
pub const AV_ERR_UNSUPPORTED_API: c_int = -1;
pub const AV_ERR_INVALID_ARG: c_int = -2;
pub const AV_ERR_INCOMPATIBLE_FRAME: c_int = -3;
pub const AV_ERR_CONFIG_INVALID: c_int = -4;
pub const AV_ERR_MODEL_LOAD_FAILED: c_int = -5;
pub const AV_ERR_INFERENCE_FAILED: c_int = -6;
pub const AV_ERR_OUT_OF_MEMORY: c_int = -7;
pub const AV_ERR_NOT_IMPLEMENTED: c_int = -8;
pub const AV_ERR_TIMEOUT: c_int = -9;
pub const AV_ERR_RETRY: c_int = -10;
pub const AV_ERR_INTERNAL: c_int = -99;

/// 像素格式
pub const AV_PIX_UNKNOWN: u32 = 0;
pub const AV_PIX_NV12: u32 = 1;
pub const AV_PIX_BGRA: u32 = 2;
pub const AV_PIX_RGB24: u32 = 3;
pub const AV_PIX_I420: u32 = 4;

/// 内存类型
pub const AV_MEM_UNKNOWN: u32 = 0;
pub const AV_MEM_HOST: u32 = 1;
pub const AV_MEM_PLATFORM_SURFACE: u32 = 2;

/// 图像布局
pub const AV_LAYOUT_UNKNOWN: u32 = 0;
pub const AV_LAYOUT_LINEAR: u32 = 1;
pub const AV_LAYOUT_PLATFORM_NATIVE: u32 = 2;

/// 原生句柄类型
pub const AV_OPAQUE_NONE: u32 = 0;
pub const AV_OPAQUE_CVPIXELBUFFER: u32 = 0x1001;
pub const AV_OPAQUE_DMABUF: u32 = 0x2001;
pub const AV_OPAQUE_ASCEND_DEVICE_MEMORY: u32 = 0x3001;

/// 实例运行模式
pub const AV_INSTANCE_NORMAL: u32 = 1;
pub const AV_INSTANCE_INSTALL_SELF_TEST: u32 = 2;

/// 结果类型
pub const AV_RESULT_ALARM: u32 = 1;
pub const AV_RESULT_SELF_TEST: u32 = 2;
pub const AV_RESULT_RECOGNITION: u32 = 3;

/// 日志函数指针
pub type AvLogFn =
    unsafe extern "C" fn(user: *mut c_void, level: c_int, msg: *const c_char, len: u32);

/// 结果回调函数指针
pub type AvAlgoResultCb = unsafe extern "C" fn(result: *const AvAlgoResult, user_data: *mut c_void);

pub type AvAlgoLibrary = *mut c_void;
pub type AvAlgoInstance = *mut c_void;

/// 帧描述符 (152 字节，固定 64 位 ABI 布局)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvFrameDesc {
    pub size: u32,
    pub api_version: u32,
    pub frame_id: u64,
    pub wall_time_ns: i64,
    pub pts_ns: i64,
    pub modifier: u64,
    pub offset: [u64; 4],
    pub opaque: *mut c_void,
    pub frame_token: *mut c_void,
    pub platform_tag: u32,
    pub opaque_kind: u32,
    pub memory_type: u32,
    pub pixel_format: u32,
    pub layout: u32,
    pub width: u32,
    pub height: u32,
    pub alloc_width: u32,
    pub alloc_height: u32,
    pub stride: [i32; 4],
    pub color_primaries: u16,
    pub color_transfer: u16,
    pub color_matrix: u16,
    pub color_range: u8,
    pub plane_count: u8,
    pub time_synced: u8,
    pub reserved: [u8; 3],
}

impl AvFrameDesc {
    /// 构造标准的 NV12 (BT.709 limited) 帧描述符默认配置
    pub fn default_nv12(
        width: u32,
        height: u32,
        y_stride: i32,
        uv_stride: i32,
        timestamp_ns: i64,
    ) -> Self {
        Self {
            size: std::mem::size_of::<Self>() as u32,
            api_version: AV_ALGO_API_VERSION,
            frame_id: 1,
            wall_time_ns: timestamp_ns,
            pts_ns: timestamp_ns,
            modifier: 0,
            offset: [0; 4],
            opaque: std::ptr::null_mut(),
            frame_token: std::ptr::null_mut(),
            platform_tag: 0,
            opaque_kind: AV_OPAQUE_NONE,
            memory_type: AV_MEM_HOST,
            pixel_format: AV_PIX_NV12,
            layout: AV_LAYOUT_LINEAR,
            width,
            height,
            alloc_width: width,
            alloc_height: height,
            stride: [y_stride, uv_stride, 0, 0],
            color_primaries: 1,
            color_transfer: 1,
            color_matrix: 1,
            color_range: 1,
            plane_count: 2,
            time_synced: 1,
            reserved: [0; 3],
        }
    }
}

/// 帧生命周期管理函数表 (32 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvFrameOps {
    pub size: u32,
    pub api_version: u32,
    pub ctx: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(ctx: *mut c_void, frame_token: *mut c_void) -> c_int>,
    pub release: Option<unsafe extern "C" fn(ctx: *mut c_void, frame_token: *mut c_void) -> c_int>,
}

/// 矩形 (24 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AvRect {
    pub size: u32,
    pub api_version: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 图像视图 (96 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvImageView {
    pub size: u32,
    pub api_version: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub memory_type: u32,
    pub plane_count: u32,
    pub opaque_kind: u32,
    pub stride: [i32; 4],
    pub offset: [u64; 4],
    pub data: *mut c_void,
    pub opaque: *mut c_void,
}

/// 硬件图像操作表 (48 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvImageOps {
    pub size: u32,
    pub api_version: u32,
    pub ctx: *mut c_void,
    pub convert: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            src: *const AvFrameDesc,
            src_roi: *const AvRect,
            dst: *const AvImageView,
            filter: u32,
        ) -> c_int,
    >,
    pub pad: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            dst: *const AvImageView,
            region: *const AvRect,
            value: *const [u8; 4],
        ) -> c_int,
    >,
    pub alloc: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            width: u32,
            height: u32,
            pixel_format: u32,
            out: *mut AvImageView,
        ) -> c_int,
    >,
    pub free: Option<unsafe extern "C" fn(ctx: *mut c_void, image: *mut AvImageView) -> c_int>,
}

/// 规则点 (8 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AvPoint {
    pub x: f32,
    pub y: f32,
}

/// 规则定义 (40 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvRule {
    pub size: u32,
    pub api_version: u32,
    pub role: u32,
    pub mode: u32,
    pub point_count: u32,
    pub points: *const AvPoint,
    pub reserved0: u32,
}

/// 算法库打开参数 (48 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvAlgoLibraryArgs {
    pub size: u32,
    pub api_version: u32,
    pub package_root: *const c_char,
    pub platform_id: *const c_char,
    pub platform_tag: u32,
    pub log: Option<AvLogFn>,
    pub log_user: *mut c_void,
}

/// 算法库信息 (200 字节)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct AvAlgoLibraryInfo {
    pub size: u32,
    pub api_version: u32,
    pub algorithm_id: [c_char; 64],
    pub version: [c_char; 32],
    pub algorithm_type: [c_char; 32],
    pub alarm_type_id: [c_char; 64],
}

impl std::fmt::Debug for AvAlgoLibraryInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvAlgoLibraryInfo")
            .field("size", &self.size)
            .field("api_version", &self.api_version)
            .finish()
    }
}

/// 算法实例创建参数 (96 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvAlgoInstanceArgs {
    pub size: u32,
    pub api_version: u32,
    pub mode: u32,
    pub reserved0: u32,
    pub instance_id: *const c_char,
    pub instance_run_id: *const c_char,
    pub config_json: *const c_char,
    pub config_json_len: u32,
    pub reserved1: u32,
    pub frame_ops: *const AvFrameOps,
    pub image_ops: *const AvImageOps,
    pub on_result: Option<AvAlgoResultCb>,
    pub result_user: *mut c_void,
    pub rules: *const AvRule,
    pub rule_count: u32,
}

/// 帧能力协商结构体 (84 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvFrameCaps {
    pub size: u32,
    pub api_version: u32,
    pub pixel_format_count: u32,
    pub pixel_formats: [u32; 8],
    pub memory_type_count: u32,
    pub memory_types: [u32; 4],
    pub required_opaque_kind: u32,
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

/// C ABI 虚函数表 (96 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvAlgoAbi {
    pub size: u32,
    pub api_version: u32,

    pub library_open: Option<
        unsafe extern "C" fn(args: *const AvAlgoLibraryArgs, out: *mut AvAlgoLibrary) -> c_int,
    >,
    pub library_query:
        Option<unsafe extern "C" fn(lib: AvAlgoLibrary, out: *mut AvAlgoLibraryInfo) -> c_int>,
    pub library_close: Option<unsafe extern "C" fn(lib: AvAlgoLibrary) -> c_int>,

    pub instance_create: Option<
        unsafe extern "C" fn(
            lib: AvAlgoLibrary,
            args: *const AvAlgoInstanceArgs,
            out: *mut AvAlgoInstance,
        ) -> c_int,
    >,
    pub instance_negotiate: Option<
        unsafe extern "C" fn(
            inst: AvAlgoInstance,
            offered: *const AvFrameCaps,
            accepted: *mut AvFrameCaps,
        ) -> c_int,
    >,
    pub instance_update_config:
        Option<unsafe extern "C" fn(inst: AvAlgoInstance, json: *const c_char, len: u32) -> c_int>,
    pub instance_set_rules: Option<
        unsafe extern "C" fn(inst: AvAlgoInstance, rules: *const AvRule, count: u32) -> c_int,
    >,
    pub instance_process:
        Option<unsafe extern "C" fn(inst: AvAlgoInstance, frame: *const AvFrameDesc) -> c_int>,
    pub instance_flush: Option<unsafe extern "C" fn(inst: AvAlgoInstance) -> c_int>,
    pub instance_destroy: Option<unsafe extern "C" fn(inst: AvAlgoInstance) -> c_int>,

    pub last_error: Option<
        unsafe extern "C" fn(inst_or_null: AvAlgoInstance, buf: *mut c_char, cap: u32) -> c_int,
    >,
}

/// 算法图像提取请求 (32 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AvAlgoImageReq {
    pub size: u32,
    pub api_version: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub purpose: u32,
    pub reserved0: u32,
}

/// 算法检测结果结构体 (48 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvAlgoResult {
    pub size: u32,
    pub api_version: u32,
    pub kind: u32,
    pub reserved0: u32,
    pub frame_id: u64,
    pub json: *const c_char,
    pub json_len: u32,
    pub image_count: u32,
    pub images: *const AvAlgoImageReq,
}

/// 人脸特征提取输入参数 (40 字节)
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AvFaceExtractInput {
    pub size: u32,
    pub api_version: u32,
    pub image_bytes: *const u8,
    pub image_bytes_len: u32,
    pub min_detection_score: f32,
    pub min_face_size: f32,
    pub min_quality_score: f32,
    pub reserved: u32,
}

/// 人脸特征提取输出结果 (67892 字节)
#[repr(C)]
pub struct AvFaceExtractOutput {
    pub size: u32,
    pub api_version: u32,
    pub status_code: u32,
    pub reserved: u32,
    pub error_message: [c_char; 256],
    pub embedding: [f32; 512],
    pub embedding_dim: u32,
    pub bbox: [f32; 4],
    pub quality_score: f32,
    pub detection_score: f32,
    pub aligned_jpeg_data: [u8; 65536],
    pub aligned_jpeg_len: u32,
    pub reserved1: u32,
}

impl std::fmt::Debug for AvFaceExtractOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvFaceExtractOutput")
            .field("size", &self.size)
            .field("api_version", &self.api_version)
            .field("status_code", &self.status_code)
            .field("embedding_dim", &self.embedding_dim)
            .field("quality_score", &self.quality_score)
            .field("aligned_jpeg_len", &self.aligned_jpeg_len)
            .finish()
    }
}

pub type AvAlgoGetAbiFn = unsafe extern "C" fn(requested_api_version: u32) -> *const AvAlgoAbi;
pub type AvAlgoExtractFaceFn = unsafe extern "C" fn(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int;
