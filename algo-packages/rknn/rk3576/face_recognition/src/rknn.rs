//! Rockchip RKNN Runtime (librknnrt) 动态绑定与安全执行会话。
//!
//! 该模块只封装 RKNN C ABI 和 DMA-BUF 生命周期。模型语义校验由
//! `RknnModelContract` 驱动，业务层只能通过带生命周期的推理方法访问会话。

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::ptr::{null_mut, NonNull};
use std::sync::Arc;

use algo_sdk::cv::DmaBufLayout;
use algo_sdk::error::AlgoError;

pub type RknnContext = u64;

pub const RKNN_SUCC: c_int = 0;
pub const RKNN_QUERY_IN_OUT_NUM: c_int = 0;
pub const RKNN_QUERY_INPUT_ATTR: c_int = 1;
pub const RKNN_QUERY_OUTPUT_ATTR: c_int = 2;

const MAX_DMA_MEM_CACHE: usize = 16;
const MAX_MODEL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DMA_MAP_BYTES: usize = 256 * 1024 * 1024;

#[cfg(target_os = "linux")]
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
#[cfg(target_os = "linux")]
const DMA_BUF_SYNC_READ: u64 = 1;
#[cfg(target_os = "linux")]
const DMA_BUF_SYNC_END: u64 = 1 << 2;

struct DmaBufCpuAccess {
    #[cfg(target_os = "linux")]
    fd: i32,
}

impl DmaBufCpuAccess {
    fn begin(fd: i32) -> Result<Self, AlgoError> {
        #[cfg(target_os = "linux")]
        {
            sync_dma_buf(fd, DMA_BUF_SYNC_READ)?;
            Ok(Self { fd })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = fd;
            Ok(Self {})
        }
    }

    fn end(self) -> Result<(), AlgoError> {
        #[cfg(target_os = "linux")]
        sync_dma_buf(self.fd, DMA_BUF_SYNC_READ | DMA_BUF_SYNC_END)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn sync_dma_buf(fd: i32, flags: u64) -> Result<(), AlgoError> {
    let mut sync = flags;
    // SAFETY: DMA_BUF_IOCTL_SYNC 只读取/更新一个由本函数持有的 u64 ioctl 参数。
    let status = unsafe { libc::ioctl(fd, DMA_BUF_IOCTL_SYNC, &mut sync) };
    if status != 0 {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF cache sync 失败: fd={fd}, flags={flags:#x}, error={}",
                std::io::Error::last_os_error()
            ),
        });
    }
    Ok(())
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RknnTensorType {
    Float32 = 0,
    Float16 = 1,
    Int8 = 2,
    Uint8 = 3,
    Int16 = 4,
    Uint16 = 5,
    Int32 = 6,
    Uint32 = 7,
    Int64 = 8,
    Bool = 9,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RknnTensorFormat {
    Nchw = 0,
    Nhwc = 1,
    Nc1hwc2 = 2,
    Undefined = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RknnTensorQntType {
    None = 0,
    Df = 1,
    AsymmetricChar = 2,
    AffineAsymmetricChar = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RknnInputOutputNum {
    pub n_input: u32,
    pub n_output: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RknnTensorAttr {
    pub index: u32,
    pub n_dims: u32,
    pub dims: [u32; 16],
    pub name: [c_char; 256],
    pub n_elems: u32,
    pub size: u32,
    pub fmt: RknnTensorFormat,
    pub type_: RknnTensorType,
    pub qnt_type: RknnTensorQntType,
    pub fl: i8,
    pub zp: i32,
    pub scale: f32,
    pub w_stride: u32,
    pub size_with_stride: u32,
    pub pass_through: u8,
    pub h_stride: u32,
}

impl Default for RknnTensorAttr {
    fn default() -> Self {
        // SAFETY: RknnTensorAttr 为固定布局的 C POD，零初始化是 librknnrt 的查询前置状态。
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RknnInput {
    pub index: u32,
    pub buf: *mut c_void,
    pub size: u32,
    pub pass_through: u8,
    pub type_: RknnTensorType,
    pub fmt: RknnTensorFormat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RknnOutput {
    pub want_float: u8,
    pub is_prealloc: u8,
    pub index: u32,
    pub buf: *mut c_void,
    pub size: u32,
}

impl Default for RknnOutput {
    fn default() -> Self {
        // SAFETY: RknnOutput 为固定布局的 C POD，零初始化表示由 Runtime 分配输出。
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RknnTensorMem {
    pub virt_addr: *mut c_void,
    pub phys_addr: u64,
    pub fd: i32,
    pub offset: i32,
    pub size: u32,
    pub flags: u32,
    pub priv_data: *mut c_void,
}

type RknnInitFn = unsafe extern "C" fn(
    ctx: *mut RknnContext,
    model: *mut c_void,
    size: u32,
    flag: u32,
    extend: *mut c_void,
) -> c_int;
type RknnDestroyFn = unsafe extern "C" fn(ctx: RknnContext) -> c_int;
type RknnQueryFn =
    unsafe extern "C" fn(ctx: RknnContext, cmd: c_int, info: *mut c_void, size: u32) -> c_int;
type RknnInputsSetFn =
    unsafe extern "C" fn(ctx: RknnContext, n_inputs: u32, inputs: *mut RknnInput) -> c_int;
type RknnRunFn = unsafe extern "C" fn(ctx: RknnContext, extend: *mut c_void) -> c_int;
type RknnOutputsGetFn = unsafe extern "C" fn(
    ctx: RknnContext,
    n_outputs: u32,
    outputs: *mut RknnOutput,
    extend: *mut c_void,
) -> c_int;
type RknnOutputsReleaseFn =
    unsafe extern "C" fn(ctx: RknnContext, n_outputs: u32, outputs: *mut RknnOutput) -> c_int;
type RknnSetCoreMaskFn = unsafe extern "C" fn(ctx: RknnContext, core_mask: c_int) -> c_int;
type RknnCreateMemFromFdFn = unsafe extern "C" fn(
    ctx: RknnContext,
    fd: i32,
    virt_addr: *mut c_void,
    size: u32,
    offset: i32,
) -> *mut RknnTensorMem;
type RknnDestroyMemFn = unsafe extern "C" fn(ctx: RknnContext, mem: *mut RknnTensorMem) -> c_int;
type RknnSetIoMemFn = unsafe extern "C" fn(
    ctx: RknnContext,
    mem: *mut RknnTensorMem,
    attr: *mut RknnTensorAttr,
) -> c_int;

/// RKNN Runtime 动态符号表。
pub struct RknnRuntime {
    _lib: libloading::Library,
    rknn_init: RknnInitFn,
    rknn_destroy: RknnDestroyFn,
    rknn_query: RknnQueryFn,
    rknn_inputs_set: RknnInputsSetFn,
    rknn_run: RknnRunFn,
    rknn_outputs_get: RknnOutputsGetFn,
    rknn_outputs_release: RknnOutputsReleaseFn,
    rknn_set_core_mask: Option<RknnSetCoreMaskFn>,
    rknn_create_mem_from_fd: Option<RknnCreateMemFromFdFn>,
    rknn_destroy_mem: Option<RknnDestroyMemFn>,
    rknn_set_io_mem: Option<RknnSetIoMemFn>,
}

impl std::fmt::Debug for RknnRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RknnRuntime")
            .field("core_mask_supported", &self.rknn_set_core_mask.is_some())
            .field(
                "dma_buf_io_supported",
                &(self.rknn_create_mem_from_fd.is_some()
                    && self.rknn_destroy_mem.is_some()
                    && self.rknn_set_io_mem.is_some()),
            )
            .finish()
    }
}

impl RknnRuntime {
    /// 动态查找并加载 librknnrt.so。
    pub fn load(package_root: &Path) -> Result<Arc<Self>, AlgoError> {
        let candidates = [
            package_root.join("lib/librknnrt.so"),
            package_root.join("lib64/librknnrt.so"),
            std::path::PathBuf::from("librknnrt.so"),
            std::path::PathBuf::from("/usr/lib/librknnrt.so"),
            std::path::PathBuf::from("/usr/lib64/librknnrt.so"),
            std::path::PathBuf::from("/usr/local/lib/librknnrt.so"),
        ];
        let mut last_error = String::from("没有候选路径");

        for path in &candidates {
            // SAFETY: 仅加载调用方指定包目录和固定系统目录中的共享库；Library 持有符号生命周期。
            match unsafe { libloading::Library::new(path) } {
                Ok(library) => {
                    tracing::info!(loaded_from = ?path, "成功加载 librknnrt.so");
                    return Self::from_library(library);
                }
                Err(error) => last_error = error.to_string(),
            }
        }

        Err(AlgoError::ModelLoad {
            reason: format!("未能加载 librknnrt.so，最后错误: {last_error}"),
        })
    }

    fn from_library(library: libloading::Library) -> Result<Arc<Self>, AlgoError> {
        // SAFETY: 符号在 library 保持存活期间有效，函数签名来自 RKNN C ABI。
        unsafe {
            let rknn_init = *library
                .get(b"rknn_init\0")
                .map_err(|error| missing_symbol("rknn_init", error))?;
            let rknn_destroy = *library
                .get(b"rknn_destroy\0")
                .map_err(|error| missing_symbol("rknn_destroy", error))?;
            let rknn_query = *library
                .get(b"rknn_query\0")
                .map_err(|error| missing_symbol("rknn_query", error))?;
            let rknn_inputs_set = *library
                .get(b"rknn_inputs_set\0")
                .map_err(|error| missing_symbol("rknn_inputs_set", error))?;
            let rknn_run = *library
                .get(b"rknn_run\0")
                .map_err(|error| missing_symbol("rknn_run", error))?;
            let rknn_outputs_get = *library
                .get(b"rknn_outputs_get\0")
                .map_err(|error| missing_symbol("rknn_outputs_get", error))?;
            let rknn_outputs_release = *library
                .get(b"rknn_outputs_release\0")
                .map_err(|error| missing_symbol("rknn_outputs_release", error))?;
            let rknn_set_core_mask = library_symbol(&library, b"rknn_set_core_mask\0");
            let rknn_create_mem_from_fd = library_symbol(&library, b"rknn_create_mem_from_fd\0");
            let rknn_destroy_mem = library_symbol(&library, b"rknn_destroy_mem\0");
            let rknn_set_io_mem = library_symbol(&library, b"rknn_set_io_mem\0");

            Ok(Arc::new(Self {
                _lib: library,
                rknn_init,
                rknn_destroy,
                rknn_query,
                rknn_inputs_set,
                rknn_run,
                rknn_outputs_get,
                rknn_outputs_release,
                rknn_set_core_mask,
                rknn_create_mem_from_fd,
                rknn_destroy_mem,
                rknn_set_io_mem,
            }))
        }
    }
}

fn missing_symbol(name: &str, error: libloading::Error) -> AlgoError {
    AlgoError::ModelLoad {
        reason: format!("加载 {name} 符号失败: {error}"),
    }
}

fn library_symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Option<T> {
    // SAFETY: 调用方保证 library 在返回的函数指针使用期间保持存活；缺少可选符号返回 None。
    unsafe { library.get(name).ok().map(|symbol| *symbol) }
}

/// 用于加载期校验的模型输入/输出契约。
#[derive(Debug, Clone)]
pub struct RknnModelContract {
    pub input_width: u32,
    pub input_height: u32,
    pub input_channels: u32,
    pub output_shapes: Vec<[u32; 4]>,
}

/// 单个输出张量的浮点数据视图。
#[derive(Debug, Clone)]
pub struct RknnTensorOutput<'a> {
    pub index: u32,
    pub dims: [u32; 4],
    pub data: &'a [f32],
}

/// RKNN 推理输出。
#[derive(Debug)]
pub enum RknnInferenceOutput<'a> {
    Float32(Vec<&'a [f32]>),
}

struct RknnOutputsGuard<'a> {
    runtime: &'a Arc<RknnRuntime>,
    ctx: RknnContext,
    outputs: Vec<RknnOutput>,
}

impl Drop for RknnOutputsGuard<'_> {
    fn drop(&mut self) {
        if self.ctx == 0 || self.outputs.is_empty() {
            return;
        }
        // SAFETY: outputs 由同一 context 的 rknn_outputs_get 填充，且 guard 保证只释放一次。
        let status = unsafe {
            (self.runtime.rknn_outputs_release)(
                self.ctx,
                self.outputs.len() as u32,
                self.outputs.as_mut_ptr(),
            )
        };
        if status != RKNN_SUCC {
            tracing::warn!(status, "rknn_outputs_release 失败");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DmaIdentity {
    device: u64,
    inode: u64,
    size: usize,
    stride: u32,
    h_stride: u32,
}

struct DmaMemEntry {
    fd: OwnedFd,
    mem: Option<NonNull<RknnTensorMem>>,
    virt_addr: Option<NonNull<c_void>>,
    map_size: usize,
    last_used: u64,
}

// SAFETY: DmaMemEntry 只在线程绑定的 RknnSession 内部使用，NonNull 指针不会跨线程共享。
unsafe impl Send for DmaMemEntry {}

#[derive(Debug, Clone, Copy)]
struct DmaEntryView {
    mem: Option<NonNull<RknnTensorMem>>,
    virt_addr: Option<NonNull<c_void>>,
}

struct HardwareBackend {
    runtime: Arc<RknnRuntime>,
    ctx: RknnContext,
    dma_mem_cache: HashMap<DmaIdentity, DmaMemEntry>,
    access_tick: u64,
}

/// 安全的 RKNN 推理会话。该类型只实现 Send，由上层专用 worker 线程独占使用。
pub struct RknnSession {
    backend: HardwareBackend,
    pub input_attr: RknnTensorAttr,
    pub output_attrs: Vec<RknnTensorAttr>,
    contract: RknnModelContract,
}

impl std::fmt::Debug for RknnSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RknnSession")
            .field("ctx", &self.backend.ctx)
            .field(
                "input_dims",
                &&self.input_attr.dims[..self.input_attr.n_dims as usize],
            )
            .field("input_format", &self.input_attr.fmt)
            .field("output_count", &self.output_attrs.len())
            .field("cached_dma_handles", &self.backend.dma_mem_cache.len())
            .finish()
    }
}

// SAFETY: RKNN context 与 DMA cache 只允许被移动到一个固定 worker 线程，RknnSession 不实现 Sync。
unsafe impl Send for RknnSession {}

impl Drop for RknnSession {
    fn drop(&mut self) {
        let backend = &mut self.backend;
        for (_, entry) in backend.dma_mem_cache.drain() {
            if let (Some(destroy_mem), Some(mem)) = (backend.runtime.rknn_destroy_mem, entry.mem) {
                // SAFETY: mem 由当前 context 的 rknn_create_mem_from_fd 创建，且只销毁一次。
                let status = unsafe { destroy_mem(backend.ctx, mem.as_ptr()) };
                if status != RKNN_SUCC {
                    tracing::warn!(status, "销毁 RKNN DMA tensor memory 失败");
                }
            }
            if let Some(virt_addr) = entry.virt_addr {
                // SAFETY: virt_addr/size 是 mmap 成功后由本结构体独占的映射。
                let status = unsafe { libc::munmap(virt_addr.as_ptr(), entry.map_size) };
                if status != 0 {
                    tracing::warn!(
                        error = ?std::io::Error::last_os_error(),
                        "释放 RKNN DMA 映射失败"
                    );
                }
            }
            drop(entry.fd);
        }

        if backend.ctx != 0 {
            // SAFETY: ctx 由当前 session 独占，Drop 只调用一次 destroy。
            let status = unsafe { (backend.runtime.rknn_destroy)(backend.ctx) };
            if status != RKNN_SUCC {
                tracing::warn!(status, "rknn_destroy 失败");
            }
            backend.ctx = 0;
        }
    }
}

impl RknnSession {
    /// 从模型文件初始化并校验 RKNN 会话。
    pub fn new(
        runtime: Arc<RknnRuntime>,
        model_path: &Path,
        contract: RknnModelContract,
    ) -> Result<Self, AlgoError> {
        let metadata = std::fs::metadata(model_path).map_err(|error| AlgoError::ModelLoad {
            reason: format!("读取 RKNN 模型元数据失败 ({model_path:?}): {error}"),
        })?;
        if !metadata.is_file() {
            return Err(AlgoError::ModelLoad {
                reason: format!("RKNN 模型路径不是普通文件: {model_path:?}"),
            });
        }
        if metadata.len() == 0 || metadata.len() > MAX_MODEL_BYTES {
            return Err(AlgoError::ModelLoad {
                reason: format!("RKNN 模型大小非法: {} bytes", metadata.len()),
            });
        }
        let mut model_bytes = std::fs::read(model_path).map_err(|error| AlgoError::ModelLoad {
            reason: format!("读取 RKNN 模型文件失败 ({model_path:?}): {error}"),
        })?;
        if model_bytes.is_empty() || model_bytes.len() as u64 > MAX_MODEL_BYTES {
            return Err(AlgoError::ModelLoad {
                reason: format!("读取到的 RKNN 模型大小非法: {} bytes", model_bytes.len()),
            });
        }
        let model_len = u32::try_from(model_bytes.len()).map_err(|_| AlgoError::ModelLoad {
            reason: format!(
                "RKNN 模型超过 Runtime uint32 长度限制: {} bytes",
                model_bytes.len()
            ),
        })?;

        let mut ctx = 0;
        // SAFETY: model_bytes 在同步 rknn_init 调用期间保持连续且可写，长度已 checked narrow。
        let status = unsafe {
            (runtime.rknn_init)(
                &mut ctx,
                model_bytes.as_mut_ptr().cast::<c_void>(),
                model_len,
                0,
                null_mut(),
            )
        };
        if status != RKNN_SUCC || ctx == 0 {
            if ctx != 0 {
                // SAFETY: Runtime 返回了非零 context，即使初始化报错也必须释放它。
                unsafe { (runtime.rknn_destroy)(ctx) };
            }
            return Err(AlgoError::ModelLoad {
                reason: format!("rknn_init 初始化模型失败，错误码: {status}"),
            });
        }

        if let Some(set_core_mask) = runtime.rknn_set_core_mask {
            // SAFETY: ctx 已成功初始化，3 是 RK3576 CORE_0_1 掩码。
            let core_status = unsafe { set_core_mask(ctx, 3) };
            if core_status != RKNN_SUCC {
                // SAFETY: 当前 context 仍由本函数独占，失败路径成对销毁。
                unsafe { (runtime.rknn_destroy)(ctx) };
                return Err(AlgoError::ModelLoad {
                    reason: format!("rknn_set_core_mask(3) 失败，错误码: {core_status}"),
                });
            }
        } else {
            tracing::warn!("librknnrt 缺少 rknn_set_core_mask，无法确认双 NPU 核生效");
        }

        let query_result = Self::query_attributes(&runtime, ctx, &contract);
        let (input_attr, output_attrs) = match query_result {
            Ok(attributes) => attributes,
            Err(error) => {
                // SAFETY: 查询失败时 context 仍由当前函数独占。
                unsafe { (runtime.rknn_destroy)(ctx) };
                return Err(error);
            }
        };

        tracing::info!(
            input_dims = ?input_attr.dims[..input_attr.n_dims as usize],
            input_format = ?input_attr.fmt,
            output_count = output_attrs.len(),
            "RKNN 会话初始化完成"
        );

        Ok(Self {
            backend: HardwareBackend {
                runtime,
                ctx,
                dma_mem_cache: HashMap::new(),
                access_tick: 0,
            },
            input_attr,
            output_attrs,
            contract,
        })
    }

    fn query_attributes(
        runtime: &Arc<RknnRuntime>,
        ctx: RknnContext,
        contract: &RknnModelContract,
    ) -> Result<(RknnTensorAttr, Vec<RknnTensorAttr>), AlgoError> {
        let mut io_num = RknnInputOutputNum {
            n_input: 0,
            n_output: 0,
        };
        // SAFETY: io_num 是足够大的可写 C POD，ctx 已初始化。
        let status = unsafe {
            (runtime.rknn_query)(
                ctx,
                RKNN_QUERY_IN_OUT_NUM,
                (&mut io_num as *mut RknnInputOutputNum).cast::<c_void>(),
                std::mem::size_of::<RknnInputOutputNum>() as u32,
            )
        };
        if status != RKNN_SUCC || io_num.n_input != 1 {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "RKNN IO 数量不符合单输入模型: status={status}, inputs={}, outputs={}",
                    io_num.n_input, io_num.n_output
                ),
            });
        }
        if io_num.n_output != contract.output_shapes.len() as u32 {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "RKNN 输出数量不匹配: expected={}, actual={}",
                    contract.output_shapes.len(),
                    io_num.n_output
                ),
            });
        }

        let mut input_attr = RknnTensorAttr {
            index: 0,
            ..Default::default()
        };
        // SAFETY: input_attr 是完整可写 C POD，Runtime 负责填充当前 index 的属性。
        let status = unsafe {
            (runtime.rknn_query)(
                ctx,
                RKNN_QUERY_INPUT_ATTR,
                (&mut input_attr as *mut RknnTensorAttr).cast::<c_void>(),
                std::mem::size_of::<RknnTensorAttr>() as u32,
            )
        };
        if status != RKNN_SUCC {
            return Err(AlgoError::ModelLoad {
                reason: format!("查询 RKNN 输入属性失败，错误码: {status}"),
            });
        }
        validate_input_attr(&input_attr, contract)?;

        let mut output_attrs = Vec::with_capacity(io_num.n_output as usize);
        for index in 0..io_num.n_output {
            let mut attr = RknnTensorAttr {
                index,
                ..Default::default()
            };
            // SAFETY: attr 是完整可写 C POD，Runtime 负责填充当前 index 的属性。
            let status = unsafe {
                (runtime.rknn_query)(
                    ctx,
                    RKNN_QUERY_OUTPUT_ATTR,
                    (&mut attr as *mut RknnTensorAttr).cast::<c_void>(),
                    std::mem::size_of::<RknnTensorAttr>() as u32,
                )
            };
            if status != RKNN_SUCC {
                return Err(AlgoError::ModelLoad {
                    reason: format!("查询 RKNN 输出属性 {index} 失败，错误码: {status}"),
                });
            }
            let output_index = usize::try_from(index).map_err(|_| AlgoError::OutOfMemory)?;
            let expected = contract.output_shapes[output_index];
            validate_output_attr(&attr, expected, index)?;
            output_attrs.push(attr);
        }
        Ok((input_attr, output_attrs))
    }

    /// 执行 Host 输入推理。该路径允许 Runtime 完成 UINT8/NHWC 到模型内部格式转换。
    pub fn infer_with_host_bytes<F, R>(
        &mut self,
        rgb_data: &[u8],
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let expected_size = expected_input_bytes(&self.contract)?;
        if rgb_data.len() != expected_size {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!(
                    "RKNN Host 输入大小不匹配: actual={}, expected={expected_size}",
                    rgb_data.len()
                ),
            });
        }
        let size = u32::try_from(rgb_data.len()).map_err(|_| AlgoError::OutOfMemory)?;
        let mut input = RknnInput {
            index: 0,
            buf: rgb_data.as_ptr().cast_mut().cast::<c_void>(),
            size,
            pass_through: 0,
            type_: RknnTensorType::Uint8,
            fmt: RknnTensorFormat::Nhwc,
        };
        // SAFETY: input.buf 借用 rgb_data，且调用同步完成后才返回。
        let status =
            unsafe { (self.backend.runtime.rknn_inputs_set)(self.backend.ctx, 1, &mut input) };
        if status != RKNN_SUCC {
            return Err(AlgoError::Inference {
                reason: format!("rknn_inputs_set 设置 Host 输入失败，错误码: {status}"),
            });
        }
        self.run_and_process(process_fn)
    }

    /// 执行 DMA-BUF 输入推理。
    ///
    /// 优先使用 `rknn_create_mem_from_fd + rknn_set_io_mem`。若 Runtime/模型输入布局
    /// 不支持直接绑定，则使用同一个 DMA-BUF 的长期 mmap 作为显式兼容路径，并由
    /// `rknn_inputs_set` 负责格式转换；该路径不做 Rust CPU 像素 memcpy，但不宣称纯设备零拷贝。
    pub fn infer_with_dma_buf<F, R>(
        &mut self,
        layout: &DmaBufLayout,
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        validate_dma_layout(layout, &self.contract)?;
        let identity = dma_identity(layout)?;
        let direct_supported = self.backend.runtime.rknn_create_mem_from_fd.is_some()
            && self.backend.runtime.rknn_destroy_mem.is_some()
            && self.backend.runtime.rknn_set_io_mem.is_some();
        let input_w_stride = if self.input_attr.w_stride == 0 {
            self.contract.input_width
        } else {
            self.input_attr.w_stride
        };
        let direct_expected_stride = input_w_stride.checked_mul(self.contract.input_channels);
        let direct_layout_compatible = self.input_attr.fmt == RknnTensorFormat::Nhwc
            && direct_expected_stride == Some(layout.stride[0]);
        let input_attr = self.input_attr;
        let runtime_required = usize::try_from(self.input_attr.size).ok().unwrap_or(0).max(
            usize::try_from(self.input_attr.size_with_stride)
                .ok()
                .unwrap_or(0),
        );
        if runtime_required != 0 && layout.size < runtime_required {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!(
                    "DMA-BUF 容量小于 RKNN 输入要求: actual={}, required={runtime_required}",
                    layout.size
                ),
            });
        }

        if direct_supported && direct_layout_compatible {
            let entry = self.ensure_dma_entry(identity, layout, true, true)?;
            if let Some(mem) = entry.mem {
                let mut attr = input_attr;
                attr.type_ = RknnTensorType::Uint8;
                // 当前 RKNN header 将 h_stride 作为绑定时的物理高度描述。
                attr.h_stride = layout.h_stride;
                if let Some(set_io_mem) = self.backend.runtime.rknn_set_io_mem {
                    // SAFETY: mem 属于当前 context，attr 是本次同步绑定使用的局部 C POD。
                    let status = unsafe { set_io_mem(self.backend.ctx, mem.as_ptr(), &mut attr) };
                    if status == RKNN_SUCC {
                        return self.run_and_process(process_fn);
                    }
                    tracing::debug!(
                        status,
                        "rknn_set_io_mem 绑定 DMA-BUF 返回非 0，回退到 DMA-BUF 映射通道"
                    );
                }
            }
        }

        self.infer_with_mapped_dma(identity, layout, process_fn)
    }

    fn infer_with_mapped_dma<F, R>(
        &mut self,
        identity: DmaIdentity,
        layout: &DmaBufLayout,
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let expected_stride = self
            .contract
            .input_width
            .checked_mul(self.contract.input_channels)
            .ok_or(AlgoError::OutOfMemory)?;
        if layout.stride[0] != expected_stride || layout.h_stride != self.contract.input_height {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!(
                    "RKNN Host 兼容输入不支持带物理 padding 的 RGB DMA-BUF: stride={} expected={}, h_stride={} expected={}",
                    layout.stride[0],
                    expected_stride,
                    layout.h_stride,
                    self.contract.input_height
                ),
            });
        }
        let size = u32::try_from(expected_input_bytes(&self.contract)?)
            .map_err(|_| AlgoError::OutOfMemory)?;
        let entry = self.ensure_dma_entry(identity, layout, true, false)?;
        let virt_addr = entry.virt_addr.ok_or_else(|| AlgoError::Inference {
            reason: "DMA-BUF 兼容路径缺少 CPU 映射".to_string(),
        })?;
        let mut input = RknnInput {
            index: 0,
            buf: virt_addr.as_ptr(),
            size,
            pass_through: 0,
            type_: RknnTensorType::Uint8,
            fmt: RknnTensorFormat::Nhwc,
        };
        // mmap 读取 DMA-BUF 前后显式做 cache sync；兼容路径不能依赖隐式一致性。
        let cpu_access = DmaBufCpuAccess::begin(layout.fd)?;
        // SAFETY: virt_addr 是由当前 session 保持的有效 DMA-BUF mmap，调用同步完成。
        let status =
            unsafe { (self.backend.runtime.rknn_inputs_set)(self.backend.ctx, 1, &mut input) };
        let sync_result = cpu_access.end();
        if status != RKNN_SUCC {
            return Err(AlgoError::Inference {
                reason: format!("rknn_inputs_set 提交 DMA-BUF 兼容输入失败，错误码: {status}"),
            });
        }
        sync_result?;
        self.run_and_process(process_fn)
    }

    fn run_and_process<F, R>(&self, process_fn: F) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        // SAFETY: ctx 已初始化，当前 session 绑定在唯一 worker 线程。
        let status = unsafe { (self.backend.runtime.rknn_run)(self.backend.ctx, null_mut()) };
        if status != RKNN_SUCC {
            return Err(AlgoError::Inference {
                reason: format!("rknn_run 推理失败，错误码: {status}"),
            });
        }

        let output_count = self.output_attrs.len();
        let mut outputs = vec![RknnOutput::default(); output_count];
        for (index, output) in outputs.iter_mut().enumerate() {
            output.index = index as u32;
            output.want_float = 1;
            output.is_prealloc = 0;
        }
        // SAFETY: outputs 是由 Rust 分配的连续 C POD 数组，Runtime 只在调用期间写入。
        let status = unsafe {
            (self.backend.runtime.rknn_outputs_get)(
                self.backend.ctx,
                output_count as u32,
                outputs.as_mut_ptr(),
                null_mut(),
            )
        };
        if status != RKNN_SUCC {
            return Err(AlgoError::Inference {
                reason: format!("rknn_outputs_get 获取输出失败，错误码: {status}"),
            });
        }
        let guard = RknnOutputsGuard {
            runtime: &self.backend.runtime,
            ctx: self.backend.ctx,
            outputs,
        };

        let mut views = Vec::with_capacity(output_count);
        for (index, output) in guard.outputs.iter().enumerate() {
            if output.buf.is_null() {
                return Err(AlgoError::Inference {
                    reason: format!("RKNN 输出 {index} 返回空指针"),
                });
            }
            let expected = usize::try_from(self.output_attrs[index].n_elems)
                .map_err(|_| AlgoError::OutOfMemory)?;
            let required_bytes = expected
                .checked_mul(std::mem::size_of::<f32>())
                .ok_or(AlgoError::OutOfMemory)?;
            if u64::from(output.size)
                < u64::try_from(required_bytes).map_err(|_| AlgoError::OutOfMemory)?
            {
                return Err(AlgoError::Inference {
                    reason: format!(
                        "RKNN 输出 {index} 缓冲区过小: {} < {}",
                        output.size, required_bytes
                    ),
                });
            }
            if !(output.buf as usize).is_multiple_of(std::mem::align_of::<f32>()) {
                return Err(AlgoError::Inference {
                    reason: "RKNN 输出指针对齐非法".to_string(),
                });
            }
            // SAFETY: want_float=1 且 size 已覆盖查询到的 n_elems，guard 保证借用期间有效。
            let values = unsafe { std::slice::from_raw_parts(output.buf.cast::<f32>(), expected) };
            views.push(values);
        }

        process_fn(&RknnInferenceOutput::Float32(views))
    }

    fn ensure_dma_entry(
        &mut self,
        identity: DmaIdentity,
        layout: &DmaBufLayout,
        map_cpu: bool,
        create_direct_mem: bool,
    ) -> Result<DmaEntryView, AlgoError> {
        self.backend.access_tick = self.backend.access_tick.wrapping_add(1);
        let tick = self.backend.access_tick;
        let size = u32::try_from(layout.size).map_err(|_| AlgoError::OutOfMemory)?;

        if self.backend.dma_mem_cache.contains_key(&identity) {
            let needs_mapping = map_cpu
                && self
                    .backend
                    .dma_mem_cache
                    .get(&identity)
                    .and_then(|entry| entry.virt_addr)
                    .is_none();
            if needs_mapping {
                let (virt_addr, map_size) = map_dma_buf(layout.fd, layout.size)?;
                let entry = self
                    .backend
                    .dma_mem_cache
                    .get_mut(&identity)
                    .expect("DMA cache entry checked above");
                entry.virt_addr = Some(virt_addr);
                entry.map_size = map_size;
            }
            let needs_direct_mem = create_direct_mem
                && self
                    .backend
                    .dma_mem_cache
                    .get(&identity)
                    .and_then(|entry| entry.mem)
                    .is_none();
            if needs_direct_mem {
                let (fd, virt_addr) = {
                    let entry = self
                        .backend
                        .dma_mem_cache
                        .get(&identity)
                        .expect("DMA cache entry checked above");
                    (entry.fd.as_raw_fd(), entry.virt_addr)
                };
                let mem = self.create_dma_mem(fd, size, virt_addr);
                if let Some(entry) = self.backend.dma_mem_cache.get_mut(&identity) {
                    entry.mem = mem;
                }
            }
            let entry = self
                .backend
                .dma_mem_cache
                .get_mut(&identity)
                .expect("DMA cache entry checked above");
            entry.last_used = tick;
            return Ok(DmaEntryView {
                mem: entry.mem,
                virt_addr: entry.virt_addr,
            });
        }

        while self.backend.dma_mem_cache.len() >= MAX_DMA_MEM_CACHE {
            let Some(oldest_key) = self
                .backend
                .dma_mem_cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = self.backend.dma_mem_cache.remove(&oldest_key) {
                self.destroy_dma_entry(entry);
            }
        }

        // SAFETY: layout.fd 已由 CvBuffer 提供并经过非负校验；dup 不读取用户指针。
        let duplicate_fd = unsafe { libc::dup(layout.fd) };
        if duplicate_fd < 0 {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!(
                    "dup DMA-BUF fd={} 失败: {}",
                    layout.fd,
                    std::io::Error::last_os_error()
                ),
            });
        }
        // SAFETY: dup 成功返回一个新的、由本结构体接管的 fd。
        let owned_fd = unsafe { OwnedFd::from_raw_fd(duplicate_fd) };
        let (virt_addr, map_size) = if map_cpu {
            let (virt_addr, map_size) = map_dma_buf(owned_fd.as_raw_fd(), layout.size)?;
            (Some(virt_addr), map_size)
        } else {
            (None, 0)
        };
        let mem = if create_direct_mem {
            self.create_dma_mem(owned_fd.as_raw_fd(), size, virt_addr)
        } else {
            None
        };

        let view = DmaEntryView { mem, virt_addr };
        self.backend.dma_mem_cache.insert(
            identity,
            DmaMemEntry {
                fd: owned_fd,
                mem,
                virt_addr,
                map_size,
                last_used: tick,
            },
        );
        Ok(view)
    }

    fn create_dma_mem(
        &self,
        fd: i32,
        size: u32,
        virt_addr: Option<NonNull<c_void>>,
    ) -> Option<NonNull<RknnTensorMem>> {
        let create_mem = self.backend.runtime.rknn_create_mem_from_fd?;
        // SAFETY: ctx 有效，fd 由 cache 持有；virt_addr 若存在则是 cache 生命周期内的映射。
        let ptr = unsafe {
            create_mem(
                self.backend.ctx,
                fd,
                virt_addr.map_or(null_mut(), NonNull::as_ptr),
                size,
                0,
            )
        };
        let mem = NonNull::new(ptr);
        if mem.is_none() {
            tracing::warn!(fd, "rknn_create_mem_from_fd 返回空句柄");
        }
        mem
    }

    fn destroy_dma_entry(&self, entry: DmaMemEntry) {
        if let (Some(destroy_mem), Some(mem)) = (self.backend.runtime.rknn_destroy_mem, entry.mem) {
            // SAFETY: entry.mem 属于当前 context，且 entry 已从 cache 移除，不会重复销毁。
            let status = unsafe { destroy_mem(self.backend.ctx, mem.as_ptr()) };
            if status != RKNN_SUCC {
                tracing::warn!(status, "淘汰 RKNN DMA tensor memory 失败");
            }
        }
        if let Some(virt_addr) = entry.virt_addr {
            // SAFETY: entry 独占该映射，size 与 mmap 时一致。
            let status = unsafe { libc::munmap(virt_addr.as_ptr(), entry.map_size) };
            if status != 0 {
                tracing::warn!(
                    error = ?std::io::Error::last_os_error(),
                    "淘汰 DMA-BUF mmap 失败"
                );
            }
        }
    }
}

fn map_dma_buf(fd: i32, size: usize) -> Result<(NonNull<c_void>, usize), AlgoError> {
    if size == 0 || size > MAX_DMA_MAP_BYTES {
        return Err(AlgoError::OutOfMemory);
    }
    let map_size = size;
    // SAFETY: fd 由当前 DMA-BUF lease 提供且保持有效；只建立只读共享映射。
    let mapped = unsafe {
        libc::mmap(
            null_mut(),
            map_size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    let Some(virt_addr) = NonNull::new(mapped).filter(|ptr| ptr.as_ptr() != libc::MAP_FAILED)
    else {
        return Err(AlgoError::Inference {
            reason: format!(
                "mmap DMA-BUF fd={fd} 失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    };
    Ok((virt_addr, map_size))
}
fn validate_input_attr(
    attr: &RknnTensorAttr,
    contract: &RknnModelContract,
) -> Result<(), AlgoError> {
    if attr.n_dims != 4 {
        return Err(AlgoError::ModelLoad {
            reason: format!("RKNN 输入维度数不支持: {}", attr.n_dims),
        });
    }
    let actual = attr.dims[..4].to_vec();
    let nchw = [
        1,
        contract.input_channels,
        contract.input_height,
        contract.input_width,
    ];
    let nhwc = [
        1,
        contract.input_height,
        contract.input_width,
        contract.input_channels,
    ];
    if actual != nchw && actual != nhwc {
        return Err(AlgoError::ModelLoad {
            reason: format!(
                "RKNN 输入形状不匹配: expected={nchw:?} or {nhwc:?}, actual={actual:?}"
            ),
        });
    }
    let logical_elements = u64::from(contract.input_width)
        .checked_mul(u64::from(contract.input_height))
        .and_then(|value| value.checked_mul(u64::from(contract.input_channels)))
        .ok_or(AlgoError::OutOfMemory)?;
    if u64::from(attr.n_elems) < logical_elements
        || u64::from(attr.size) < logical_elements
        || (attr.size_with_stride != 0 && u64::from(attr.size_with_stride) < logical_elements)
        || attr.size == 0
    {
        return Err(AlgoError::ModelLoad {
            reason: format!(
                "RKNN 输入容量不足: elems={}, size={}",
                attr.n_elems, attr.size
            ),
        });
    }
    Ok(())
}

fn expected_input_bytes(contract: &RknnModelContract) -> Result<usize, AlgoError> {
    let elements = u64::from(contract.input_width)
        .checked_mul(u64::from(contract.input_height))
        .and_then(|value| value.checked_mul(u64::from(contract.input_channels)))
        .ok_or(AlgoError::OutOfMemory)?;
    usize::try_from(elements).map_err(|_| AlgoError::OutOfMemory)
}

fn validate_output_attr(
    attr: &RknnTensorAttr,
    expected: [u32; 4],
    index: u32,
) -> Result<(), AlgoError> {
    let actual_dims = usize::try_from(attr.n_dims).map_err(|_| AlgoError::OutOfMemory)?;
    if actual_dims == 0 || actual_dims > 4 {
        return Err(AlgoError::ModelLoad {
            reason: format!("RKNN 输出 {index} 维度数非法: {}", attr.n_dims),
        });
    }
    let mut actual = [1u32; 4];
    actual[..actual_dims].copy_from_slice(&attr.dims[..actual_dims]);
    let embedder_channels_last = expected == [1, 512, 1, 1] && actual == [1, 1, 1, 512];
    if actual != expected && !embedder_channels_last {
        return Err(AlgoError::ModelLoad {
            reason: format!(
                "RKNN 输出 {index} 形状不匹配: expected={expected:?}, actual={:?}",
                &attr.dims[..attr.n_dims.min(16) as usize]
            ),
        });
    }
    let expected_elems = expected
        .iter()
        .try_fold(1u64, |acc, value| acc.checked_mul(u64::from(*value)))
        .ok_or(AlgoError::OutOfMemory)?;
    if u64::from(attr.n_elems) < expected_elems || attr.size == 0 {
        return Err(AlgoError::ModelLoad {
            reason: format!(
                "RKNN 输出 {index} 容量不足: elems={}, size={}",
                attr.n_elems, attr.size
            ),
        });
    }
    Ok(())
}

fn validate_dma_layout(
    layout: &DmaBufLayout,
    contract: &RknnModelContract,
) -> Result<(), AlgoError> {
    if layout.size > MAX_DMA_MAP_BYTES {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF 容量超过单帧上限: {} > {}",
                layout.size, MAX_DMA_MAP_BYTES
            ),
        });
    }
    if layout.fd < 0 || layout.size == 0 || layout.h_stride < contract.input_height {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF 布局无效: fd={}, size={}, h_stride={}",
                layout.fd, layout.size, layout.h_stride
            ),
        });
    }
    let expected_stride = contract
        .input_width
        .checked_mul(contract.input_channels)
        .ok_or(AlgoError::OutOfMemory)?;
    if layout.stride[0] < expected_stride || !layout.stride[0].is_multiple_of(16) {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF RGB stride 非法: actual={}, minimum={}, 需要 16 字节对齐",
                layout.stride[0], expected_stride
            ),
        });
    }
    if !layout.h_stride.is_multiple_of(2) {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!("DMA-BUF h_stride 必须为偶数: {}", layout.h_stride),
        });
    }
    let expected_size = usize::try_from(layout.stride[0])
        .ok()
        .and_then(|stride| stride.checked_mul(layout.h_stride as usize))
        .ok_or(AlgoError::OutOfMemory)?;
    if layout.size < expected_size {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "DMA-BUF 容量不足: size={} < stride*h_stride={expected_size}",
                layout.size
            ),
        });
    }
    Ok(())
}

fn dma_identity(layout: &DmaBufLayout) -> Result<DmaIdentity, AlgoError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat 指向足够大的可写 libc::stat，fd 已由 SafeFrame/CvBuffer 校验为非负。
    let status = unsafe { libc::fstat(layout.fd, stat.as_mut_ptr()) };
    if status != 0 {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "fstat DMA-BUF fd={} 失败: {}",
                layout.fd,
                std::io::Error::last_os_error()
            ),
        });
    }
    // SAFETY: fstat 返回成功后完整写入 stat。
    let stat = unsafe { stat.assume_init() };
    Ok(DmaIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
        size: layout.size,
        stride: layout.stride[0],
        h_stride: layout.h_stride,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_layout_matches_rknn_header_shape() {
        assert_eq!(std::mem::align_of::<RknnTensorAttr>(), 4);
        assert_eq!(std::mem::size_of::<RknnTensorAttr>(), 376);
        assert_eq!(std::mem::align_of::<RknnTensorMem>(), 8);
        assert_eq!(std::mem::size_of::<RknnTensorMem>(), 40);
        assert_eq!(std::mem::size_of::<RknnInput>(), 32);
        assert_eq!(std::mem::size_of::<RknnOutput>(), 24);
    }
    #[test]
    fn validates_detector_contract_shapes() {
        let contract = RknnModelContract {
            input_width: 640,
            input_height: 384,
            input_channels: 3,
            output_shapes: vec![[1, 64, 48, 80]; 12],
        };
        let mut attr = RknnTensorAttr {
            n_dims: 4,
            dims: [1, 3, 384, 640, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            n_elems: 640 * 384 * 3,
            size: 640 * 384 * 3,
            ..Default::default()
        };
        assert!(validate_input_attr(&attr, &contract).is_ok());
        attr.dims[1] = 4;
        assert!(validate_input_attr(&attr, &contract).is_err());
    }

    #[test]
    fn rejects_short_dma_layout() {
        let contract = RknnModelContract {
            input_width: 640,
            input_height: 384,
            input_channels: 3,
            output_shapes: Vec::new(),
        };
        let layout = DmaBufLayout {
            fd: 3,
            size: 640 * 384,
            stride: [640 * 3, 0, 0, 0],
            h_stride: 384,
        };
        assert!(validate_dma_layout(&layout, &contract).is_err());
    }
}
