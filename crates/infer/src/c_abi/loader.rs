//! C ABI 动态库安全加载与 RAII 资源生命周期管理

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::sync::Arc;

use libloading::{Library, Symbol};

use crate::c_abi::types::*;
use crate::error::InferError;

/// 封装已加载的动态库与 C ABI 虚表
#[derive(Debug)]
pub struct LoadedLib {
    // 必须持有 Library 以保证其内部映射在内存中的函数指针在结构体存活期间有效
    _lib: Library,
    abi: AvAlgoAbi,
}

// SAFETY: C ABI 函数指针均指向动态库只读代码段，可安全在多线程间转移所有权
unsafe impl Send for LoadedLib {}
// SAFETY: C ABI 虚表在多线程并发读取时安全
unsafe impl Sync for LoadedLib {}

impl LoadedLib {
    /// 加载动态库并提取 C ABI 虚表
    pub fn load(lib_path: &Path) -> Result<Self, InferError> {
        let path_str = lib_path.to_string_lossy().to_string();

        // SAFETY: 仅加载指定路径动态库文件
        let lib = unsafe {
            Library::new(lib_path).map_err(|e| InferError::LibraryLoad {
                path: path_str.clone(),
                reason: e.to_string(),
            })?
        };

        // SAFETY: 寻址 C ABI 符号 "av_algo_get_abi"
        let get_abi_fn: Symbol<AvAlgoGetAbiFn> = unsafe {
            lib.get(AV_ALGO_GET_ABI_SYMBOL)
                .map_err(|e| InferError::SymbolLookup {
                    symbol: "av_algo_get_abi".to_string(),
                    reason: e.to_string(),
                })?
        };

        // SAFETY: 调用导出的获取虚表函数，传入当前支持的 API 版本
        let abi_ptr = unsafe { get_abi_fn(AV_ALGO_API_VERSION) };
        if abi_ptr.is_null() {
            return Err(InferError::InvalidAbi {
                reason: "av_algo_get_abi 返回了空指针".to_string(),
            });
        }

        // SAFETY: 验证虚表内存与版本
        let abi = unsafe { *abi_ptr };
        let expected_size = std::mem::size_of::<AvAlgoAbi>() as u32;
        if abi.size != expected_size {
            return Err(InferError::InvalidAbi {
                reason: format!(
                    "虚表大小不匹配: 期望 {expected_size} 字节, 实际 {} 字节",
                    abi.size
                ),
            });
        }

        if abi.api_version != AV_ALGO_API_VERSION {
            return Err(InferError::InvalidAbi {
                reason: format!(
                    "API 版本不匹配: 期望 {AV_ALGO_API_VERSION}, 实际 {}",
                    abi.api_version
                ),
            });
        }

        // 校验关键虚函数指针非空
        if abi.library_open.is_none()
            || abi.library_query.is_none()
            || abi.library_close.is_none()
            || abi.instance_create.is_none()
            || abi.instance_process.is_none()
            || abi.instance_destroy.is_none()
        {
            return Err(InferError::InvalidAbi {
                reason: "C ABI 虚表存在未实现的空函数指针".to_string(),
            });
        }

        Ok(Self { _lib: lib, abi })
    }

    /// 获取 C ABI 虚表引用
    #[inline]
    pub fn abi(&self) -> &AvAlgoAbi {
        &self.abi
    }

    /// 尝试寻址人脸特征提取符号
    pub fn get_extract_face_fn(&self) -> Option<Symbol<'_, AvAlgoExtractFaceFn>> {
        // SAFETY: 寻址动态库代码段导出的 av_algo_extract_face 符号
        unsafe { self._lib.get(AV_ALGO_EXTRACT_FACE_SYMBOL).ok() }
    }
}

/// 算法库元数据信息（Rust 友好版）
#[derive(Debug, Clone)]
pub struct LibraryMeta {
    pub algorithm_id: String,
    pub version: String,
    pub algorithm_type: String,
    pub alarm_type_id: String,
}

/// RAII 管理的算法包句柄
#[derive(Debug)]
pub struct RawAlgoLibrary {
    lib: Arc<LoadedLib>,
    raw: AvAlgoLibrary,
    meta: LibraryMeta,
}

// SAFETY: 算法库句柄底层无线程亲和性，可在多线程转移所有权
unsafe impl Send for RawAlgoLibrary {}
// SAFETY: 算法库只读上下文支持多线程并发访问
unsafe impl Sync for RawAlgoLibrary {}

impl RawAlgoLibrary {
    /// 打开算法库
    pub fn open(
        lib: Arc<LoadedLib>,
        package_root: &Path,
        platform_id: &str,
    ) -> Result<Self, InferError> {
        let root_c = CString::new(package_root.to_string_lossy().as_bytes()).map_err(|_| {
            InferError::LibraryLoad {
                path: package_root.to_string_lossy().to_string(),
                reason: "路径包含非法 null 字节".to_string(),
            }
        })?;
        let platform_c =
            CString::new(platform_id.as_bytes()).map_err(|_| InferError::LibraryLoad {
                path: platform_id.to_string(),
                reason: "平台 ID 包含非法 null 字节".to_string(),
            })?;

        let args = AvAlgoLibraryArgs {
            size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
            api_version: AV_ALGO_API_VERSION,
            package_root: root_c.as_ptr(),
            platform_id: platform_c.as_ptr(),
            platform_tag: 0,
            log: Some(default_c_logger),
            log_user: std::ptr::null_mut(),
        };

        let mut raw_lib: AvAlgoLibrary = std::ptr::null_mut();

        let abi = lib.abi();
        let open_fn = abi.library_open.ok_or_else(|| InferError::InvalidAbi {
            reason: "library_open 函数指针为空".to_string(),
        })?;

        // SAFETY: args 在调用期间有效；raw_lib 指向有效栈变量
        let code = unsafe { open_fn(&args, &mut raw_lib) };
        if code != AV_OK || raw_lib.is_null() {
            // SAFETY: 调用方保证 abi 与 inst 内存有效
            return Err(unsafe { check_c_status(code, abi, std::ptr::null_mut()) });
        }

        // 查询元数据
        let mut info = AvAlgoLibraryInfo {
            size: std::mem::size_of::<AvAlgoLibraryInfo>() as u32,
            api_version: AV_ALGO_API_VERSION,
            algorithm_id: [0; 64],
            version: [0; 32],
            algorithm_type: [0; 32],
            alarm_type_id: [0; 64],
        };

        let query_fn = abi.library_query.ok_or_else(|| InferError::InvalidAbi {
            reason: "library_query 函数指针为空".to_string(),
        })?;

        // SAFETY: raw_lib 为有效句柄；info 为有效栈内存
        let query_code = unsafe { query_fn(raw_lib, &mut info) };
        if query_code != AV_OK {
            // SAFETY: 出现错误时释放已打开的 raw_lib
            if let Some(close_fn) = abi.library_close {
                // SAFETY: raw_lib 由当前 open_fn 创建且未交付，释放它以防泄漏
                unsafe { close_fn(raw_lib) };
            }
            // SAFETY: 调用方保证 abi 与 inst 内存有效
            return Err(unsafe { check_c_status(query_code, abi, std::ptr::null_mut()) });
        }

        let meta = LibraryMeta {
            algorithm_id: c_chars_to_string(&info.algorithm_id),
            version: c_chars_to_string(&info.version),
            algorithm_type: c_chars_to_string(&info.algorithm_type),
            alarm_type_id: c_chars_to_string(&info.alarm_type_id),
        };

        Ok(Self {
            lib,
            raw: raw_lib,
            meta,
        })
    }

    #[inline]
    pub fn raw(&self) -> AvAlgoLibrary {
        self.raw
    }

    #[inline]
    pub fn meta(&self) -> &LibraryMeta {
        &self.meta
    }

    #[inline]
    pub fn lib(&self) -> &Arc<LoadedLib> {
        &self.lib
    }

    /// 调用底层 C ABI 进行单张人脸特征提取、对齐与质量评估
    pub fn extract_face(&self, jpeg_bytes: &[u8]) -> Result<FaceExtraction, InferError> {
        let extract_fn =
            self.lib
                .get_extract_face_fn()
                .ok_or_else(|| InferError::SymbolLookup {
                    symbol: "av_algo_extract_face".to_string(),
                    reason: "当前算法库未导出 av_algo_extract_face 符号".to_string(),
                })?;

        let input = AvFaceExtractInput {
            size: std::mem::size_of::<AvFaceExtractInput>() as u32,
            api_version: AV_ALGO_API_VERSION,
            image_bytes: jpeg_bytes.as_ptr(),
            image_bytes_len: jpeg_bytes.len() as u32,
        };

        let mut output = AvFaceExtractOutput {
            size: std::mem::size_of::<AvFaceExtractOutput>() as u32,
            api_version: AV_ALGO_API_VERSION,
            status_code: 0,
            embedding: std::ptr::null(),
            embedding_dim: 0,
            aligned_jpeg: std::ptr::null(),
            aligned_jpeg_len: 0,
            quality_score: 0.0,
            detection_score: 0.0,
        };

        // SAFETY: input/output 结构体满足 ABI 契约且在调用期间保持有效
        let status = unsafe { extract_fn(self.raw, &input, &mut output) };
        if status != AV_OK || output.status_code != 0 {
            return Err(InferError::Execution {
                reason: format!(
                    "人脸特征提取失败: status={status}, code={}",
                    output.status_code
                ),
            });
        }

        if output.embedding.is_null()
            || output.embedding_dim != 512
            || !(output.embedding as usize).is_multiple_of(std::mem::align_of::<f32>())
        {
            return Err(InferError::Execution {
                reason: format!(
                    "人脸特征向量无效: ptr={:?}, dim={}",
                    output.embedding, output.embedding_dim
                ),
            });
        }

        // SAFETY: output.embedding 指向 output.embedding_dim 个连续浮点数
        let embedding = unsafe {
            std::slice::from_raw_parts(output.embedding, output.embedding_dim as usize).to_vec()
        };

        let aligned_jpeg = if !output.aligned_jpeg.is_null() && output.aligned_jpeg_len > 0 {
            if output.aligned_jpeg_len > 32 * 1024 * 1024 {
                return Err(InferError::Execution {
                    reason: "人脸对齐切片尺寸异常超过 32MB 限制".to_string(),
                });
            }
            // SAFETY: output.aligned_jpeg 指向 output.aligned_jpeg_len 个有效字节
            unsafe {
                std::slice::from_raw_parts(output.aligned_jpeg, output.aligned_jpeg_len as usize)
                    .to_vec()
            }
        } else {
            Vec::new()
        };

        Ok(FaceExtraction {
            embedding,
            quality_score: output.quality_score,
            detection_score: output.detection_score,
            aligned_jpeg,
        })
    }
}

/// 人脸特征提取输出结果（安全封装版）
#[derive(Debug, Clone)]
pub struct FaceExtraction {
    pub embedding: Vec<f32>,
    pub quality_score: f32,
    pub detection_score: f32,
    pub aligned_jpeg: Vec<u8>,
}

impl Drop for RawAlgoLibrary {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            if let Some(close_fn) = self.lib.abi().library_close {
                // SAFETY: raw 由 library_open 产出，Drop 只执行一次
                unsafe { close_fn(self.raw) };
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

/// 默认 C 回调日志桥接函数
unsafe extern "C" fn default_c_logger(
    _user: *mut c_void,
    level: std::ffi::c_int,
    msg: *const c_char,
    _len: u32,
) {
    let _ = std::panic::catch_unwind(|| {
        if msg.is_null() {
            return;
        }
        // SAFETY: msg 是以 null 结尾的有效 C 字符串
        let c_str = unsafe { CStr::from_ptr(msg) };
        let text = c_str.to_string_lossy();

        match level {
            0 => tracing::trace!(target: "algo_c", "{text}"),
            1 => tracing::debug!(target: "algo_c", "{text}"),
            2 => tracing::info!(target: "algo_c", "{text}"),
            3 => tracing::warn!(target: "algo_c", "{text}"),
            _ => tracing::error!(target: "algo_c", "{text}"),
        }
    });
}

/// 检查 C ABI 返回码，并提取底层详细错误信息
///
/// # Safety
/// 调用方必须确保 `abi` 虚表指针有效，且 `inst` 为有效指针或空指针。
pub unsafe fn check_c_status(
    code: std::ffi::c_int,
    abi: &AvAlgoAbi,
    inst: AvAlgoInstance,
) -> InferError {
    let mut err_buf = [0 as c_char; 512];
    let msg = if let Some(last_error_fn) = abi.last_error {
        // SAFETY: err_buf 是本地栈内存
        let res = unsafe { last_error_fn(inst, err_buf.as_mut_ptr(), err_buf.len() as u32) };
        if res == AV_OK {
            err_buf[511] = 0;
            // SAFETY: err_buf 已被限制并保证 null 终止
            let c_str = unsafe { CStr::from_ptr(err_buf.as_ptr()) };
            c_str.to_string_lossy().to_string()
        } else {
            "未知底层错误".to_string()
        }
    } else {
        "未提供 last_error 接口".to_string()
    };

    InferError::CAbiError {
        code,
        message: format!("状态码 {code} ({msg})"),
    }
}

/// 线程安全通用的 C ABI 结果收集回调函数
///
/// # Safety
/// `user_data` 必须是指向存活期内的 `std::sync::Mutex<Vec<String>>` 的指针。
pub unsafe extern "C" fn algo_result_collector(
    result: *const AvAlgoResult,
    user_data: *mut c_void,
) {
    let _ = std::panic::catch_unwind(|| {
        if result.is_null() || user_data.is_null() {
            return;
        }
        // SAFETY: result 是底层传回的只读指针
        let res = unsafe { &*result };
        if res.json.is_null() || res.json_len == 0 {
            return;
        }

        // SAFETY: res.json 指向只读字符串，json_len 为底层计算的有效长度
        let json_slice =
            unsafe { std::slice::from_raw_parts(res.json as *const u8, res.json_len as usize) };
        let json_str = String::from_utf8_lossy(json_slice).to_string();

        // SAFETY: user_data 在调用期间为有效的 Mutex<Vec<String>> 裸指针
        let slot = unsafe { &*(user_data as *const std::sync::Mutex<Vec<String>>) };
        if let Ok(mut lock) = slot.lock() {
            lock.push(json_str);
        }
    });
}

fn c_chars_to_string(bytes: &[c_char]) -> String {
    // SAFETY: c_char 与 u8 在当前 64 位平台具有相同内存尺寸
    let u8_slice: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast(), bytes.len()) };
    let null_pos = u8_slice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(u8_slice.len());
    String::from_utf8_lossy(&u8_slice[..null_pos]).to_string()
}
