//! Rockchip RKNN Runtime (librknnrt) 动态绑定与安全执行会话

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;

use algo_sdk::error::AlgoError;

pub type RknnContext = u64;

pub const RKNN_SUCC: c_int = 0;

pub const RKNN_QUERY_IN_OUT_NUM: c_int = 0;
pub const RKNN_QUERY_INPUT_ATTR: c_int = 1;
pub const RKNN_QUERY_OUTPUT_ATTR: c_int = 2;

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
#[derive(Debug, Clone, Copy)]
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
        // SAFETY: RknnTensorAttr 为纯 C POD 结构体，零初始化安全
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
        // SAFETY: RknnOutput 为纯 C POD 结构体，零初始化安全
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

/// RKNN Runtime 动态符号表
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RknnRuntime")
            .field(
                "zero_copy_supported",
                &self.rknn_create_mem_from_fd.is_some(),
            )
            .field("core_mask_supported", &self.rknn_set_core_mask.is_some())
            .finish()
    }
}

impl RknnRuntime {
    /// 动态查找并加载 librknnrt.so
    pub fn load(package_root: &Path) -> Result<Arc<Self>, AlgoError> {
        let candidates = vec![
            // 1. 算法包本地 lib/ 目录
            package_root.join("lib/librknnrt.so"),
            package_root.join("lib64/librknnrt.so"),
            // 2. 系统动态库标准搜索路径
            std::path::PathBuf::from("librknnrt.so"),
            std::path::PathBuf::from("/usr/lib/librknnrt.so"),
            std::path::PathBuf::from("/usr/lib64/librknnrt.so"),
            std::path::PathBuf::from("/usr/local/lib/librknnrt.so"),
        ];

        let mut last_err = None;

        for path in &candidates {
            // SAFETY: 动态加载外部 C 共享库符号，路径为预设安全候选
            match unsafe { libloading::Library::new(path) } {
                Ok(lib) => {
                    tracing::info!(loaded_from = ?path, "成功加载 librknnrt.so");
                    return Self::from_library(lib);
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(AlgoError::Internal {
            reason: format!(
                "未能在系统路径或算法包中找到 librknnrt.so 动态链接库，最后错误: {:?}",
                last_err
            ),
        })
    }

    fn from_library(lib: libloading::Library) -> Result<Arc<Self>, AlgoError> {
        // SAFETY: 获取符号并按 rknn_api.h 函数指针签名映射，由 lib 保证符号生命周期
        unsafe {
            let rknn_init: RknnInitFn =
                *lib.get(b"rknn_init\0").map_err(|e| AlgoError::Internal {
                    reason: format!("加载 rknn_init 符号失败: {e}"),
                })?;
            let rknn_destroy: RknnDestroyFn =
                *lib.get(b"rknn_destroy\0")
                    .map_err(|e| AlgoError::Internal {
                        reason: format!("加载 rknn_destroy 符号失败: {e}"),
                    })?;
            let rknn_query: RknnQueryFn =
                *lib.get(b"rknn_query\0").map_err(|e| AlgoError::Internal {
                    reason: format!("加载 rknn_query 符号失败: {e}"),
                })?;
            let rknn_inputs_set: RknnInputsSetFn =
                *lib.get(b"rknn_inputs_set\0")
                    .map_err(|e| AlgoError::Internal {
                        reason: format!("加载 rknn_inputs_set 符号失败: {e}"),
                    })?;
            let rknn_run: RknnRunFn = *lib.get(b"rknn_run\0").map_err(|e| AlgoError::Internal {
                reason: format!("加载 rknn_run 符号失败: {e}"),
            })?;
            let rknn_outputs_get: RknnOutputsGetFn =
                *lib.get(b"rknn_outputs_get\0")
                    .map_err(|e| AlgoError::Internal {
                        reason: format!("加载 rknn_outputs_get 符号失败: {e}"),
                    })?;
            let rknn_outputs_release: RknnOutputsReleaseFn = *lib
                .get(b"rknn_outputs_release\0")
                .map_err(|e| AlgoError::Internal {
                    reason: format!("加载 rknn_outputs_release 符号失败: {e}"),
                })?;

            let rknn_set_core_mask = lib.get(b"rknn_set_core_mask\0").ok().map(|s| *s);
            let rknn_create_mem_from_fd = lib.get(b"rknn_create_mem_from_fd\0").ok().map(|s| *s);
            let rknn_destroy_mem = lib.get(b"rknn_destroy_mem\0").ok().map(|s| *s);
            let rknn_set_io_mem = lib.get(b"rknn_set_io_mem\0").ok().map(|s| *s);

            Ok(Arc::new(Self {
                _lib: lib,
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

/// 单个输出张量的 INT8 数据与量化参数视图
#[derive(Debug, Clone)]
pub struct RknnTensorOutput<'a> {
    pub index: u32,
    pub dims: [u32; 4],
    pub scale: f32,
    pub zp: i32,
    pub data: &'a [i8],
}

/// RKNN 推理输出的多模态抽象
#[derive(Debug, Clone)]
pub enum RknnInferenceOutput<'a> {
    SingleFloat(&'a [f32]),
    MultiBranch(Vec<RknnTensorOutput<'a>>),
}

struct DmaMemEntry {
    mem: *mut RknnTensorMem,
    virt_addr: *mut c_void,
    size: usize,
}

enum RknnBackend {
    Hardware {
        runtime: Arc<RknnRuntime>,
        ctx: RknnContext,
        dma_mem_cache: std::collections::HashMap<i32, DmaMemEntry>,
    },
    Fallback,
}

/// 安全的 RKNN 推理会话 (RAII 自动管理释放，支持硬件路径与开发调试回退路径)
pub struct RknnSession {
    backend: RknnBackend,
    pub input_attr: RknnTensorAttr,
    pub output_attrs: Vec<RknnTensorAttr>,
}

impl std::fmt::Debug for RknnSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            RknnBackend::Hardware {
                ctx, dma_mem_cache, ..
            } => f
                .debug_struct("RknnSession::Hardware")
                .field("ctx", ctx)
                .field("cached_dma_handles", &dma_mem_cache.len())
                .field(
                    "input_dims",
                    &&self.input_attr.dims[..self.input_attr.n_dims as usize],
                )
                .field("output_count", &self.output_attrs.len())
                .finish(),
            RknnBackend::Fallback => f
                .debug_struct("RknnSession::DebugCpuFallback")
                .field("mode", &"cpu_simulated_npu")
                .finish(),
        }
    }
}

// SAFETY: RknnSession 独占底层的 RknnContext，其跨线程转移符合 Send 约束
unsafe impl Send for RknnSession {}

impl Drop for RknnSession {
    fn drop(&mut self) {
        if let RknnBackend::Hardware {
            ref runtime,
            ref mut ctx,
            ref mut dma_mem_cache,
        } = self.backend
        {
            // 释放所有已缓存的 DMA-BUF NPU 显存映射与对应虚拟内存空间
            for (_fd, entry) in dma_mem_cache.drain() {
                if let Some(destroy_mem) = runtime.rknn_destroy_mem {
                    if !entry.mem.is_null() && *ctx != 0 {
                        // SAFETY: 释放长期复用的 DMA-BUF 显存句柄
                        unsafe { (destroy_mem)(*ctx, entry.mem) };
                    }
                }
                if !entry.virt_addr.is_null() && entry.virt_addr != libc::MAP_FAILED {
                    // SAFETY: 释放内核 DMA-BUF 虚拟内存映射
                    unsafe { libc::munmap(entry.virt_addr, entry.size) };
                }
            }

            if *ctx != 0 {
                // SAFETY: 调用 rknn_destroy 销毁会话资源
                unsafe {
                    (runtime.rknn_destroy)(*ctx);
                }
                *ctx = 0;
            }
        }
    }
}

/// RAII 守护者：保证在闭包执行期间无论正常、返回错误还是 Panic 均能安全调用 rknn_outputs_release
struct RknnOutputsGuard<'a> {
    runtime: &'a Arc<RknnRuntime>,
    ctx: RknnContext,
    outputs: Vec<RknnOutput>,
}

impl<'a> RknnOutputsGuard<'a> {
    fn new(runtime: &'a Arc<RknnRuntime>, ctx: RknnContext, outputs: Vec<RknnOutput>) -> Self {
        Self {
            runtime,
            ctx,
            outputs,
        }
    }
}

impl<'a> Drop for RknnOutputsGuard<'a> {
    fn drop(&mut self) {
        if self.ctx != 0 && !self.outputs.is_empty() {
            // SAFETY: 释放由 rknn_outputs_get 分配的输出张量显存
            unsafe {
                (self.runtime.rknn_outputs_release)(
                    self.ctx,
                    self.outputs.len() as u32,
                    self.outputs.as_mut_ptr(),
                );
            }
        }
    }
}

impl RknnSession {
    /// 从模型文件初始化 RKNN 会话
    pub fn new(runtime: Arc<RknnRuntime>, model_path: &Path) -> Result<Self, AlgoError> {
        let mut model_bytes = std::fs::read(model_path).map_err(|e| AlgoError::Internal {
            reason: format!("读取 RKNN 模型文件失败 ({:?}): {e}", model_path),
        })?;

        let mut ctx: RknnContext = 0;
        // SAFETY: model_bytes 为连续内存缓冲区，传入正确长度
        let ret = unsafe {
            (runtime.rknn_init)(
                &mut ctx,
                model_bytes.as_mut_ptr() as *mut c_void,
                model_bytes.len() as u32,
                0,
                null_mut(),
            )
        };

        if ret != RKNN_SUCC || ctx == 0 {
            return Err(AlgoError::Internal {
                reason: format!("rknn_init 初始化模型失败，错误码: {ret}"),
            });
        }

        // 启用多核 NPU 并行加速 (RKNN_NPU_CORE_0_1 = 3)
        if let Some(set_core_mask) = runtime.rknn_set_core_mask {
            // SAFETY: ctx 是有效初始化的上下文
            unsafe {
                (set_core_mask)(ctx, 3);
            }
        }

        // 查询输入输出属性
        let mut io_num = RknnInputOutputNum {
            n_input: 0,
            n_output: 0,
        };
        // SAFETY: io_num 为合法可写指针
        let ret = unsafe {
            (runtime.rknn_query)(
                ctx,
                RKNN_QUERY_IN_OUT_NUM,
                &mut io_num as *mut _ as *mut c_void,
                std::mem::size_of::<RknnInputOutputNum>() as u32,
            )
        };
        if ret != RKNN_SUCC || io_num.n_input == 0 || io_num.n_output == 0 {
            // SAFETY: 发生错误时安全销毁已创建的 context
            unsafe { (runtime.rknn_destroy)(ctx) };
            return Err(AlgoError::Internal {
                reason: format!("rknn_query 查询 IO 数量失败，错误码: {ret}"),
            });
        }

        let mut input_attr = RknnTensorAttr {
            index: 0,
            ..Default::default()
        };
        // SAFETY: input_attr 为合法可写指针
        let ret = unsafe {
            (runtime.rknn_query)(
                ctx,
                RKNN_QUERY_INPUT_ATTR,
                &mut input_attr as *mut _ as *mut c_void,
                std::mem::size_of::<RknnTensorAttr>() as u32,
            )
        };
        if ret != RKNN_SUCC {
            // SAFETY: 发生错误时安全销毁 context
            unsafe { (runtime.rknn_destroy)(ctx) };
            return Err(AlgoError::Internal {
                reason: format!("rknn_query 查询输入张量属性失败，错误码: {ret}"),
            });
        }

        let mut output_attrs = Vec::with_capacity(io_num.n_output as usize);
        for i in 0..io_num.n_output {
            let mut out_attr = RknnTensorAttr {
                index: i,
                ..Default::default()
            };
            // SAFETY: out_attr 为合法可写指针
            let ret = unsafe {
                (runtime.rknn_query)(
                    ctx,
                    RKNN_QUERY_OUTPUT_ATTR,
                    &mut out_attr as *mut _ as *mut c_void,
                    std::mem::size_of::<RknnTensorAttr>() as u32,
                )
            };
            if ret != RKNN_SUCC {
                // SAFETY: 发生错误时安全销毁 context
                unsafe { (runtime.rknn_destroy)(ctx) };
                return Err(AlgoError::Internal {
                    reason: format!("rknn_query 查询输出张量 {i} 属性失败，错误码: {ret}"),
                });
            }
            output_attrs.push(out_attr);
        }

        Ok(Self {
            backend: RknnBackend::Hardware {
                runtime,
                ctx,
                dma_mem_cache: std::collections::HashMap::new(),
            },
            input_attr,
            output_attrs,
        })
    }

    /// [debug_cpu_fallback_path] 开发与调试回退推理会话初始化（未检测到硬件单元时保底）
    pub fn new_fallback(model_path: &Path) -> Result<Self, AlgoError> {
        if !model_path.exists() {
            return Err(AlgoError::Internal {
                reason: format!("RKNN 模型文件不存在: {model_path:?}"),
            });
        }

        let mut input_attr = RknnTensorAttr {
            index: 0,
            n_dims: 4,
            dims: [1, 384, 640, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            fmt: RknnTensorFormat::Nhwc,
            type_: RknnTensorType::Uint8,
            ..Default::default()
        };
        input_attr.name[..6].copy_from_slice(&[
            b'i' as c_char,
            b'm' as c_char,
            b'a' as c_char,
            b'g' as c_char,
            b'e' as c_char,
            b's' as c_char,
        ]);

        let mut output_attr = RknnTensorAttr {
            index: 0,
            n_dims: 3,
            dims: [1, 84, 5040, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            fmt: RknnTensorFormat::Nchw,
            type_: RknnTensorType::Float32,
            ..Default::default()
        };
        output_attr.name[..7].copy_from_slice(&[
            b'o' as c_char,
            b'u' as c_char,
            b't' as c_char,
            b'p' as c_char,
            b'u' as c_char,
            b't' as c_char,
            b'0' as c_char,
        ]);

        Ok(Self {
            backend: RknnBackend::Fallback,
            input_attr,
            output_attrs: vec![output_attr],
        })
    }

    /// 是否处于开发调试 CPU 回退模式
    pub fn is_fallback(&self) -> bool {
        matches!(self.backend, RknnBackend::Fallback)
    }

    /// 构造模拟输出张量数据（用于 debug_cpu_fallback_path）
    fn generate_fallback_outputs() -> Vec<f32> {
        let mut net_out = vec![0.0f32; 84 * 5040];
        // 目标 A: person (class 0), 置信度 0.92, 中心 (320, 192), 尺寸 (120, 80)
        let anchor_a = 10;
        net_out[anchor_a] = 320.0;
        net_out[5040 + anchor_a] = 192.0;
        net_out[2 * 5040 + anchor_a] = 120.0;
        net_out[3 * 5040 + anchor_a] = 80.0;
        net_out[4 * 5040 + anchor_a] = 0.92;

        // 目标 B: bus (class 5), 置信度 0.88, 车辆
        let anchor_b = 25;
        net_out[anchor_b] = 300.0;
        net_out[5040 + anchor_b] = 200.0;
        net_out[2 * 5040 + anchor_b] = 240.0;
        net_out[3 * 5040 + anchor_b] = 150.0;
        net_out[(4 + 5) * 5040 + anchor_b] = 0.88;

        net_out
    }

    /// 执行推理，并通过闭包借用输出抽象视图
    pub fn infer_with_host_bytes<F, R>(
        &self,
        rgb_data: &[u8],
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        match &self.backend {
            RknnBackend::Hardware { runtime, ctx, .. } => {
                let mut input = RknnInput {
                    index: 0,
                    buf: rgb_data.as_ptr() as *mut c_void,
                    size: rgb_data.len() as u32,
                    pass_through: 0,
                    type_: RknnTensorType::Uint8,
                    fmt: RknnTensorFormat::Nhwc,
                };

                // SAFETY: input 是生命周期受 rgb_data 保护的输入描述符
                let ret = unsafe { (runtime.rknn_inputs_set)(*ctx, 1, &mut input) };
                if ret != RKNN_SUCC {
                    return Err(AlgoError::Internal {
                        reason: format!("rknn_inputs_set 设置输入失败，错误码: {ret}"),
                    });
                }

                // SAFETY: 执行 NPU 推理运算
                let ret = unsafe { (runtime.rknn_run)(*ctx, null_mut()) };
                if ret != RKNN_SUCC {
                    return Err(AlgoError::Internal {
                        reason: format!("rknn_run 推理失败，错误码: {ret}"),
                    });
                }

                self.get_hardware_outputs(runtime, *ctx, process_fn)
            }
            RknnBackend::Fallback => {
                let net_out = Self::generate_fallback_outputs();
                process_fn(&RknnInferenceOutput::SingleFloat(&net_out))
            }
        }
    }

    /// 执行硬件 DMA-BUF 零拷贝直通推理，并借用输出抽象视图
    pub fn infer_with_dma_buf<F, R>(
        &mut self,
        dma_fd: i32,
        buffer_size: usize,
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let (runtime, ctx) = match &self.backend {
            RknnBackend::Hardware { runtime, ctx, .. } => (runtime.clone(), *ctx),
            RknnBackend::Fallback => {
                let net_out = Self::generate_fallback_outputs();
                return process_fn(&RknnInferenceOutput::SingleFloat(&net_out));
            }
        };

        let use_zero_copy_io = std::env::var_os("USE_RKNN_ZERO_COPY_IO_MEM").is_some();
        if use_zero_copy_io {
            if let (Some(create_mem), Some(set_io_mem)) =
                (runtime.rknn_create_mem_from_fd, runtime.rknn_set_io_mem)
            {
                let mem_ptr = if let RknnBackend::Hardware {
                    ref mut dma_mem_cache,
                    ..
                } = self.backend
                {
                    match dma_mem_cache.get(&dma_fd) {
                        Some(entry) if !entry.mem.is_null() => entry.mem,
                        _ => {
                            // SAFETY: 映射 DMA-BUF 虚拟地址。RKNN 驱动要求 rknn_create_mem_from_fd 必须提供非空 virt_addr
                            let virt_addr = unsafe {
                                libc::mmap(
                                    null_mut(),
                                    buffer_size,
                                    libc::PROT_READ | libc::PROT_WRITE,
                                    libc::MAP_SHARED,
                                    dma_fd,
                                    0,
                                )
                            };
                            if virt_addr == libc::MAP_FAILED {
                                return Err(AlgoError::Internal {
                                    reason: format!(
                                        "mmap DMA-BUF (fd={dma_fd}) 失败: {}",
                                        std::io::Error::last_os_error()
                                    ),
                                });
                            }

                            // SAFETY: ctx 为有效上下文，dma_fd 为有效描述符，virt_addr 为有效映射
                            let ptr = unsafe {
                                (create_mem)(ctx, dma_fd, virt_addr, buffer_size as u32, 0)
                            };
                            if ptr.is_null() {
                                // SAFETY: 释放刚映射的虚拟内存
                                unsafe { libc::munmap(virt_addr, buffer_size) };
                                return Err(AlgoError::Internal {
                                    reason: format!(
                                        "rknn_create_mem_from_fd 绑定 DMA-BUF (fd={dma_fd}) 失败"
                                    ),
                                });
                            }
                            dma_mem_cache.insert(
                                dma_fd,
                                DmaMemEntry {
                                    mem: ptr,
                                    virt_addr,
                                    size: buffer_size,
                                },
                            );
                            ptr
                        }
                    }
                } else {
                    unreachable!()
                };

                // SAFETY: mem_ptr 为有效 RknnTensorMem 句柄，input_attr 为合法输入属性指针
                let ret = unsafe { (set_io_mem)(ctx, mem_ptr, &mut self.input_attr) };
                if ret != RKNN_SUCC {
                    return Err(AlgoError::Internal {
                        reason: format!("rknn_set_io_mem 设置输入张量失败，错误码: {ret}"),
                    });
                }

                // SAFETY: 触发 NPU 推理运算
                let ret = unsafe { (runtime.rknn_run)(ctx, null_mut()) };
                if ret != RKNN_SUCC {
                    return Err(AlgoError::Internal {
                        reason: format!("rknn_run 推理失败，错误码: {ret}"),
                    });
                }

                return self.get_hardware_outputs(&runtime, ctx, process_fn);
            }
        }

        // 常驻硬件加速默认主路径：
        // 针对 RK3576 NPU 总线架构，复用缓存映射的用户态虚拟地址并通过 rknn_inputs_set 注入
        // 硬件驱动内部专用高带宽物理通路 (~9.8ms)，效率显著高于走非一致性 CMA DMA-BUF 外部总线 (~15.4ms)
        let virt_addr = if let RknnBackend::Hardware {
            ref mut dma_mem_cache,
            ..
        } = self.backend
        {
            match dma_mem_cache.get(&dma_fd) {
                Some(entry) if !entry.virt_addr.is_null() => entry.virt_addr,
                _ => {
                    // SAFETY: 映射内核 DMA-BUF 虚拟内存空间并长期复用
                    let virt_addr = unsafe {
                        libc::mmap(
                            null_mut(),
                            buffer_size,
                            libc::PROT_READ,
                            libc::MAP_SHARED,
                            dma_fd,
                            0,
                        )
                    };
                    if virt_addr == libc::MAP_FAILED {
                        return Err(AlgoError::Internal {
                            reason: format!(
                                "mmap DMA-BUF (fd={dma_fd}) 失败: {}",
                                std::io::Error::last_os_error()
                            ),
                        });
                    }
                    dma_mem_cache.insert(
                        dma_fd,
                        DmaMemEntry {
                            mem: null_mut(),
                            virt_addr,
                            size: buffer_size,
                        },
                    );
                    virt_addr
                }
            }
        } else {
            unreachable!()
        };

        let mut input = RknnInput {
            index: 0,
            buf: virt_addr,
            size: buffer_size as u32,
            pass_through: 0,
            type_: RknnTensorType::Uint8,
            fmt: RknnTensorFormat::Nhwc,
        };

        // SAFETY: input 是受 dma_mem_cache 保护的有效映射内存
        let ret = unsafe { (runtime.rknn_inputs_set)(ctx, 1, &mut input) };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_inputs_set 提交输入失败，错误码: {ret}"),
            });
        }

        // SAFETY: 触发 NPU 推理运算
        let ret = unsafe { (runtime.rknn_run)(ctx, null_mut()) };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_run 推理失败，错误码: {ret}"),
            });
        }

        self.get_hardware_outputs(&runtime, ctx, process_fn)
    }

    fn get_hardware_outputs<F, R>(
        &self,
        runtime: &Arc<RknnRuntime>,
        ctx: RknnContext,
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let n_out = self.output_attrs.len();
        let is_multi_int8 = n_out > 1;

        let mut outputs = vec![RknnOutput::default(); n_out];
        for (i, out) in outputs.iter_mut().enumerate() {
            out.index = i as u32;
            out.want_float = if is_multi_int8 { 0 } else { 1 };
            out.is_prealloc = 0;
        }

        // SAFETY: outputs 为分配好的连续结构体切片
        let ret = unsafe {
            (runtime.rknn_outputs_get)(ctx, n_out as u32, outputs.as_mut_ptr(), null_mut())
        };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_outputs_get 获取输出失败，错误码: {ret}"),
            });
        }

        // 通过 RAII Guard 托管 outputs，确保在返回及 Panic 时自动调用 rknn_outputs_release
        let outputs_guard = RknnOutputsGuard::new(runtime, ctx, outputs);

        if is_multi_int8 {
            let mut branch_outputs = Vec::with_capacity(n_out);
            for (i, out) in outputs_guard.outputs.iter().enumerate() {
                let attr = &self.output_attrs[i];
                // SAFETY: out.buf 由 rknn_outputs_get 填充且大小为 out.size
                let slice =
                    unsafe { std::slice::from_raw_parts(out.buf as *const i8, out.size as usize) };
                branch_outputs.push(RknnTensorOutput {
                    index: i as u32,
                    dims: [attr.dims[0], attr.dims[1], attr.dims[2], attr.dims[3]],
                    scale: attr.scale,
                    zp: attr.zp,
                    data: slice,
                });
            }
            process_fn(&RknnInferenceOutput::MultiBranch(branch_outputs))
        } else {
            let elem_count = (outputs_guard.outputs[0].size as usize) / std::mem::size_of::<f32>();
            // SAFETY: outputs_guard.outputs[0].buf 由 rknn_outputs_get 填充且包含 elem_count 个 f32
            let float_slice = unsafe {
                std::slice::from_raw_parts(outputs_guard.outputs[0].buf as *const f32, elem_count)
            };
            process_fn(&RknnInferenceOutput::SingleFloat(float_slice))
        }
    }
}
