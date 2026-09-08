//! Rockchip RKNN Runtime (librknnrt) 动态绑定与安全执行会话
//!
//! 从 `general_detection/src/rknn.rs` 精简而来，保留核心推理能力，
//! 移除 DMA-BUF 零拷贝路径（本次不使用）和 CPU 回退模拟。

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
}

impl std::fmt::Debug for RknnRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RknnRuntime")
            .field("core_mask_supported", &self.rknn_set_core_mask.is_some())
            .finish()
    }
}

impl RknnRuntime {
    /// 动态查找并加载 librknnrt.so
    pub fn load(package_root: &Path) -> Result<Arc<Self>, AlgoError> {
        let candidates = vec![
            package_root.join("lib/librknnrt.so"),
            package_root.join("lib64/librknnrt.so"),
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
                "未能在系统路径或算法包中找到 librknnrt.so，最后错误: {:?}",
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
            }))
        }
    }
}

/// 单个输出张量的浮点数据视图
#[derive(Debug, Clone)]
pub struct RknnTensorOutput<'a> {
    pub index: u32,
    pub dims: [u32; 4],
    pub data: &'a [f32],
}

/// RKNN 推理输出
#[derive(Debug)]
pub enum RknnInferenceOutput<'a> {
    Float32(Vec<&'a [f32]>),
}

/// RAII 守护者：保证 rknn_outputs_release 在 Drop 时被调用
struct RknnOutputsGuard<'a> {
    runtime: &'a Arc<RknnRuntime>,
    ctx: RknnContext,
    outputs: Vec<RknnOutput>,
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

/// 安全的 RKNN 推理会话（RAII 管理 rknn_context 生命周期）
pub struct RknnSession {
    runtime: Arc<RknnRuntime>,
    ctx: RknnContext,
    pub input_attr: RknnTensorAttr,
    pub output_attrs: Vec<RknnTensorAttr>,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

impl std::fmt::Debug for RknnSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RknnSession")
            .field("ctx", &self.ctx)
            .field(
                "input_dims",
                &&self.input_attr.dims[..self.input_attr.n_dims as usize],
            )
            .field("output_count", &self.output_attrs.len())
            .finish()
    }
}

// SAFETY: RknnSession 独占底层 RknnContext，跨线程转移符合 Send 约束。
// 未实现 Sync，因为底层 rknn_context 上的 C API 调用具有可变副作用，禁止并发多线程无锁访问。
unsafe impl Send for RknnSession {}

impl Drop for RknnSession {
    fn drop(&mut self) {
        if self.ctx != 0 {
            // SAFETY: 调用 rknn_destroy 销毁会话资源，ctx 由本结构体独占
            unsafe {
                (self.runtime.rknn_destroy)(self.ctx);
            }
            self.ctx = 0;
        }
    }
}

impl RknnSession {
    /// 从模型文件初始化 RKNN 会话
    pub fn new(runtime: Arc<RknnRuntime>, model_path: &Path) -> Result<Self, AlgoError> {
        let mut model_bytes = std::fs::read(model_path).map_err(|e| AlgoError::Internal {
            reason: format!("读取 RKNN 模型文件失败 ({model_path:?}): {e}"),
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

        tracing::info!(
            input_dims = ?input_attr.dims[..input_attr.n_dims as usize],
            output_count = io_num.n_output,
            "RKNN 会话初始化完成"
        );

        Ok(Self {
            runtime,
            ctx,
            input_attr,
            output_attrs,
            _not_sync: std::marker::PhantomData,
        })
    }

    /// 执行推理，返回所有输出张量的 float32 视图
    ///
    /// 对于 mixed precision 模型，RKNN runtime 自动将 INT8 反量化为 float32。
    /// 输出通过 RAII Guard 扩展生命周期，闭包返回后自动释放。
    pub fn infer_with_host_bytes<F, R>(
        &self,
        rgb_data: &[u8],
        process_fn: F,
    ) -> Result<R, AlgoError>
    where
        F: FnOnce(&RknnInferenceOutput<'_>) -> Result<R, AlgoError>,
    {
        let mut input = RknnInput {
            index: 0,
            buf: rgb_data.as_ptr() as *mut c_void,
            size: rgb_data.len() as u32,
            pass_through: 0,
            type_: RknnTensorType::Uint8,
            fmt: RknnTensorFormat::Nhwc,
        };

        // SAFETY: input 是生命周期受 rgb_data 保护的输入描述符
        let ret = unsafe { (self.runtime.rknn_inputs_set)(self.ctx, 1, &mut input) };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_inputs_set 设置输入失败，错误码: {ret}"),
            });
        }

        // SAFETY: 执行 NPU 推理运算
        let ret = unsafe { (self.runtime.rknn_run)(self.ctx, null_mut()) };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_run 推理失败，错误码: {ret}"),
            });
        }

        // 获取输出：want_float=1 让 runtime 自动反量化为 float32
        let n_out = self.output_attrs.len();
        let mut outputs = vec![RknnOutput::default(); n_out];
        for (i, out) in outputs.iter_mut().enumerate() {
            out.index = i as u32;
            out.want_float = 1;
            out.is_prealloc = 0;
        }

        // SAFETY: outputs 为分配好的连续结构体切片
        let ret = unsafe {
            (self.runtime.rknn_outputs_get)(
                self.ctx,
                n_out as u32,
                outputs.as_mut_ptr(),
                null_mut(),
            )
        };
        if ret != RKNN_SUCC {
            return Err(AlgoError::Internal {
                reason: format!("rknn_outputs_get 获取输出失败，错误码: {ret}"),
            });
        }

        // RAII Guard 确保 outputs 在闭包执行期间及 Panic 时自动释放
        let _guard = RknnOutputsGuard {
            runtime: &self.runtime,
            ctx: self.ctx,
            outputs,
        };

        // 构造 float32 切片视图
        let float_views: Vec<&[f32]> = _guard
            .outputs
            .iter()
            .map(|out| {
                let elem_count = out.size as usize / std::mem::size_of::<f32>();
                // SAFETY: out.buf 由 rknn_outputs_get 填充，want_float=1 保证数据为 float32
                unsafe { std::slice::from_raw_parts(out.buf as *const f32, elem_count) }
            })
            .collect();

        let inference_output = RknnInferenceOutput::Float32(float_views);
        process_fn(&inference_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn test_rknn_ffi_structure_sizes_and_alignments() {
        assert_eq!(size_of::<RknnInputOutputNum>(), 8);
        assert_eq!(align_of::<RknnInputOutputNum>(), 4);

        assert_eq!(size_of::<RknnInput>(), 32);
        assert_eq!(align_of::<RknnInput>(), 8);

        assert_eq!(size_of::<RknnOutput>(), 24);
        assert_eq!(align_of::<RknnOutput>(), 8);

        assert_eq!(size_of::<RknnTensorAttr>(), 376);
        assert_eq!(align_of::<RknnTensorAttr>(), 4);

        assert_eq!(size_of::<RknnTensorType>(), 4);
        assert_eq!(size_of::<RknnTensorFormat>(), 4);
        assert_eq!(size_of::<RknnTensorQntType>(), 4);
    }

    #[test]
    fn test_rknn_ffi_structure_field_offsets() {
        // RknnInputOutputNum
        assert_eq!(offset_of!(RknnInputOutputNum, n_input), 0);
        assert_eq!(offset_of!(RknnInputOutputNum, n_output), 4);

        // RknnInput
        assert_eq!(offset_of!(RknnInput, index), 0);
        assert_eq!(offset_of!(RknnInput, buf), 8);
        assert_eq!(offset_of!(RknnInput, size), 16);
        assert_eq!(offset_of!(RknnInput, pass_through), 20);
        assert_eq!(offset_of!(RknnInput, type_), 24);
        assert_eq!(offset_of!(RknnInput, fmt), 28);

        // RknnOutput
        assert_eq!(offset_of!(RknnOutput, want_float), 0);
        assert_eq!(offset_of!(RknnOutput, is_prealloc), 1);
        assert_eq!(offset_of!(RknnOutput, index), 4);
        assert_eq!(offset_of!(RknnOutput, buf), 8);
        assert_eq!(offset_of!(RknnOutput, size), 16);

        // RknnTensorAttr
        assert_eq!(offset_of!(RknnTensorAttr, index), 0);
        assert_eq!(offset_of!(RknnTensorAttr, n_dims), 4);
        assert_eq!(offset_of!(RknnTensorAttr, dims), 8);
        assert_eq!(offset_of!(RknnTensorAttr, name), 72);
        assert_eq!(offset_of!(RknnTensorAttr, n_elems), 328);
        assert_eq!(offset_of!(RknnTensorAttr, size), 332);
        assert_eq!(offset_of!(RknnTensorAttr, fmt), 336);
        assert_eq!(offset_of!(RknnTensorAttr, type_), 340);
        assert_eq!(offset_of!(RknnTensorAttr, qnt_type), 344);
        assert_eq!(offset_of!(RknnTensorAttr, fl), 348);
        assert_eq!(offset_of!(RknnTensorAttr, zp), 352);
        assert_eq!(offset_of!(RknnTensorAttr, scale), 356);
        assert_eq!(offset_of!(RknnTensorAttr, w_stride), 360);
        assert_eq!(offset_of!(RknnTensorAttr, size_with_stride), 364);
        assert_eq!(offset_of!(RknnTensorAttr, pass_through), 368);
        assert_eq!(offset_of!(RknnTensorAttr, h_stride), 372);
    }
}
