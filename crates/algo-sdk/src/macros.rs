//! C ABI 导出宏与虚表胶水层 (macros)
//!
//! 提供 `export_algo!` 宏展开为 11 个 C ABI 虚函数入口，
//! 严格隔离跨 FFI Panic，并管理线程局部错误缓存。

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void};
use std::path::PathBuf;
use std::slice;

use crate::c_abi::*;
use crate::error::AlgoError;
use crate::plugin::AlgoPlugin;

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// 设置当前线程的最后一次错误信息
pub fn set_last_error(msg: impl Into<String>) {
    let s = msg.into();
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = s;
    });
}

/// 将最后一次错误拷贝到 C 字符缓冲区
///
/// # Safety
/// `buf` 必须指向容量至少为 `cap` 字节的有效可写内存，且在调用期间不发生并发变异。
pub unsafe fn copy_last_error(buf: *mut c_char, cap: u32) -> c_int {
    if buf.is_null() || cap == 0 {
        return AV_ERR_INVALID_ARG;
    }

    LAST_ERROR.with(|cell| {
        let err = cell.borrow();
        let bytes = err.as_bytes();
        let copy_len = bytes.len().min(cap as usize - 1);

        // SAFETY: 调用方保证 buf 非空且容量至少为 cap 字节
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, copy_len);
            *buf.add(copy_len) = 0;
        }
        AV_OK
    })
}

/// 辅助函数：将 Rust 字符串拷贝到定长 C char 数组中
pub fn copy_str_to_c_chars<const N: usize>(src: &str, dst: &mut [c_char; N]) {
    if N == 0 {
        return;
    }
    let bytes = src.as_bytes();
    let copy_len = bytes.len().min(N - 1);
    for i in 0..copy_len {
        dst[i] = bytes[i] as c_char;
    }
    dst[copy_len] = 0;
}

/// 算法库常驻上下文
#[derive(Debug)]
pub struct LibraryContext {
    pub package_root: PathBuf,
    pub platform_id: String,
}

/// 算法实例常驻上下文
pub struct InstanceContext<P: AlgoPlugin> {
    pub plugin: std::sync::Mutex<P>,
    pub engine: crate::cv::SharedCvEngine,
    pub on_result: Option<AvAlgoResultCb>,
    pub user_data: *mut c_void,
}

impl<P: AlgoPlugin> std::fmt::Debug for InstanceContext<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceContext")
            .field("on_result", &self.on_result.is_some())
            .finish()
    }
}

// SAFETY: InstanceContext 内部由专一的推理通道独占访问，支持线程间转移。
unsafe impl<P: AlgoPlugin> Send for InstanceContext<P> {}
// SAFETY: InstanceContext 内部的插件访问由 Mutex 串行化；宿主回调指针与 user_data
// 必须遵循 ABI 的跨线程调用契约，因此同一实例可被多个导出入口共享读取。
unsafe impl<P: AlgoPlugin> Sync for InstanceContext<P> {}

/// 没有长度字段的 C ABI 字符串的最大扫描长度。
const MAX_C_STRING_BYTES: usize = 16 * 1024;
/// 配置 JSON 的最大输入长度，避免插件初始化被无界输入拖垮。
const MAX_CONFIG_JSON_BYTES: usize = 1024 * 1024;
const MAX_RULES: usize = 4096;
const MAX_RULE_POINTS: usize = 1_000_000;

/// 校验带 `size`/`api_version` 头的 ABI 结构体。
///
/// # Safety
/// `raw` 必须至少指向可读取 ABI 头部的内存；头部通过校验后，调用方必须保证
/// 当前版本结构体的完整内容在本次调用期间可读。
pub unsafe fn validate_abi_header<T>(raw: *const T, name: &str) -> Result<(), AlgoError> {
    if raw.is_null() {
        return Err(AlgoError::Internal {
            reason: format!("{name} 指针为空"),
        });
    }
    if !(raw as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return Err(AlgoError::Internal {
            reason: format!("{name} 指针未按 ABI 对齐"),
        });
    }
    let header = raw.cast::<u32>();
    // SAFETY: 调用方保证前两个 u32 头字段可读。
    let size = unsafe { header.read() } as usize;
    // SAFETY: api_version 与 size 同属 ABI 头部。
    let api_version = unsafe { header.add(1).read() };
    let expected_size = std::mem::size_of::<T>();
    if size < expected_size {
        return Err(AlgoError::Internal {
            reason: format!("{name} 大小不足: {size} < {expected_size}"),
        });
    }
    if api_version != AV_ALGO_API_VERSION {
        return Err(AlgoError::UnsupportedApi);
    }
    Ok(())
}

/// 解析没有长度字段的 C 字符串，并限制最大扫描长度。
///
/// # Safety
/// `ptr` 为空或指向一个在 `max_len` 内以 NUL 结尾、且在本次调用期间保持有效的字节串。
pub unsafe fn parse_c_str_bounded<'a>(
    ptr: *const c_char,
    max_len: usize,
) -> Result<&'a str, AlgoError> {
    if ptr.is_null() {
        return Ok("");
    }
    if max_len == 0 {
        return Err(AlgoError::ConfigParse {
            reason: "C 字符串最大长度不能为 0".to_string(),
        });
    }
    // 逐字节扫描，在遇到 NUL 后立即停止，避免对短 C 字符串无条件读取 max_len。
    let mut length = 0usize;
    while length < max_len {
        // SAFETY: 调用方保证 terminator 或 max_len 范围内的每个字节可读。
        let byte = unsafe { (ptr as *const u8).add(length).read() };
        if byte == 0 {
            // SAFETY: 上方逐字节读取已确认 ptr 指向完整字符串内容。
            let bytes = unsafe { slice::from_raw_parts(ptr as *const u8, length) };
            return std::str::from_utf8(bytes).map_err(|e| AlgoError::ConfigParse {
                reason: format!("非法 UTF-8 字符串: {e}"),
            });
        }
        length += 1;
    }

    Err(AlgoError::ConfigParse {
        reason: format!("C 字符串未在 {max_len} 字节内结束"),
    })
}

/// 解析带显式长度的 UTF-8 配置 JSON；允许并剥离一个尾随 NUL。
///
/// # Safety
/// `ptr` 为空时 `len` 必须为 0；非空时必须指向至少 `len` 个在本次调用期间有效的字节。
pub unsafe fn parse_bytes_with_len<'a>(ptr: *const c_char, len: u32) -> Result<&'a str, AlgoError> {
    let len = len as usize;
    if len == 0 {
        return Ok("");
    }
    if len > MAX_CONFIG_JSON_BYTES {
        return Err(AlgoError::ConfigParse {
            reason: format!("配置 JSON 超过最大长度 {MAX_CONFIG_JSON_BYTES}"),
        });
    }
    if ptr.is_null() {
        return Err(AlgoError::ConfigParse {
            reason: "config_json 非空但指针为空".to_string(),
        });
    }
    // SAFETY: 调用方保证 config_json 指向 len 个有效字节。
    let bytes = unsafe { slice::from_raw_parts(ptr as *const u8, len) };
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.contains(&0) {
        return Err(AlgoError::ConfigParse {
            reason: "config_json 包含嵌入 NUL".to_string(),
        });
    }
    std::str::from_utf8(bytes).map_err(|e| AlgoError::ConfigParse {
        reason: format!("非法 UTF-8 配置 JSON: {e}"),
    })
}

/// 保留旧 helper 名称，但同样使用有界扫描。
///
/// # Safety
/// `ptr` 必须为空或指向在最大长度内以 NUL 结尾的有效 C 字符串。
pub unsafe fn parse_c_str<'a>(ptr: *const c_char) -> Result<&'a str, AlgoError> {
    // SAFETY: 由本函数契约保证有界 C 字符串可读。
    unsafe { parse_c_str_bounded(ptr, MAX_C_STRING_BYTES) }
}

/// 校验并借用规则数组；规则和点坐标都必须满足 C ABI 与归一化契约。
///
/// # Safety
/// `rules` 非空时必须指向至少 `count` 个连续有效的 `AvRule`，每个 rule 的 points
/// 指针在本次调用期间保持有效。
pub unsafe fn validate_rules<'a>(
    rules: *const AvRule,
    count: u32,
) -> Result<&'a [AvRule], AlgoError> {
    let count = count as usize;
    if count == 0 {
        return Ok(&[]);
    }
    if count > MAX_RULES {
        return Err(AlgoError::ConfigParse {
            reason: format!("规则数量超过上限 {MAX_RULES}"),
        });
    }
    if rules.is_null() {
        return Err(AlgoError::ConfigParse {
            reason: "规则数量非零但 rules 指针为空".to_string(),
        });
    }
    if !(rules as usize).is_multiple_of(std::mem::align_of::<AvRule>()) {
        return Err(AlgoError::ConfigParse {
            reason: "rules 指针未按 ABI 对齐".to_string(),
        });
    }
    // SAFETY: count 已受上限约束，调用方保证数组内存有效。
    let rules_slice = unsafe { slice::from_raw_parts(rules, count) };
    for rule in rules_slice {
        if rule.size < std::mem::size_of::<AvRule>() as u32 {
            return Err(AlgoError::ConfigParse {
                reason: "规则结构体大小不足".to_string(),
            });
        }
        if rule.api_version != AV_ALGO_API_VERSION {
            return Err(AlgoError::UnsupportedApi);
        }
        let point_count = rule.point_count as usize;
        if point_count > MAX_RULE_POINTS {
            return Err(AlgoError::ConfigParse {
                reason: format!("规则点数量超过上限 {MAX_RULE_POINTS}"),
            });
        }
        if point_count == 0 {
            continue;
        }
        if rule.points.is_null()
            || !(rule.points as usize).is_multiple_of(std::mem::align_of::<AvPoint>())
        {
            return Err(AlgoError::ConfigParse {
                reason: "规则 points 指针无效".to_string(),
            });
        }
        // SAFETY: points 指向 point_count 个连续有效点，且 count 已受上限约束。
        let points = unsafe { slice::from_raw_parts(rule.points, point_count) };
        if points.iter().any(|point| {
            !point.x.is_finite()
                || !point.y.is_finite()
                || !(0.0..=1.0).contains(&point.x)
                || !(0.0..=1.0).contains(&point.y)
        }) {
            return Err(AlgoError::ConfigParse {
                reason: "规则坐标必须是 [0, 1] 内的有限数".to_string(),
            });
        }
    }
    Ok(rules_slice)
}

/// 校验帧能力协商结构体的计数范围。
pub fn validate_frame_caps(caps: &AvFrameCaps) -> Result<(), AlgoError> {
    if caps.size < std::mem::size_of::<AvFrameCaps>() as u32 {
        return Err(AlgoError::Internal {
            reason: "AvFrameCaps 大小不足".to_string(),
        });
    }
    if caps.api_version != AV_ALGO_API_VERSION {
        return Err(AlgoError::UnsupportedApi);
    }
    if caps.pixel_format_count > 8 || caps.memory_type_count > 4 {
        return Err(AlgoError::ConfigParse {
            reason: "AvFrameCaps 计数超出固定数组容量".to_string(),
        });
    }
    if (caps.max_width != 0 && caps.max_width < caps.min_width)
        || (caps.max_height != 0 && caps.max_height < caps.min_height)
    {
        return Err(AlgoError::ConfigParse {
            reason: "AvFrameCaps 最大尺寸小于最小尺寸".to_string(),
        });
    }
    Ok(())
}

/// 默认算法包 library 生命周期 hook；未声明专用 hook 的包保持原行为。
#[doc(hidden)]
pub fn noop_library_open(_package_root: &std::path::Path) -> Result<(), AlgoError> {
    Ok(())
}

/// 默认算法包 library 生命周期 hook；未声明专用 hook 的包保持原行为。
#[doc(hidden)]
pub fn noop_library_close(_package_root: &std::path::Path) {}

/// 一行导出算法包 C ABI 虚表与可选 library 生命周期 hook
#[macro_export]
macro_rules! export_algo {
    (
        $plugin_ty:ty,
        algo_id: $id:expr,
        version: $ver:expr,
        algo_type: $atype:expr,
        alarm_type_id: $alarm:expr
    ) => {
        $crate::export_algo!(
            $plugin_ty,
            algo_id: $id,
            version: $ver,
            algo_type: $atype,
            alarm_type_id: $alarm,
            library_open_hook: $crate::macros::noop_library_open,
            library_close_hook: $crate::macros::noop_library_close
        );
    };
    (
        $plugin_ty:ty,
        algo_id: $id:expr,
        version: $ver:expr,
        algo_type: $atype:expr,
        alarm_type_id: $alarm:expr,
        library_open_hook: $open_hook:path,
        library_close_hook: $close_hook:path
    ) => {
        // ── 11 个 C ABI 回调入口（带 catch_unwind Panic 防火墙） ──

        unsafe extern "C" fn __algo_library_open(
            args: *const $crate::c_abi::AvAlgoLibraryArgs,
            out: *mut $crate::c_abi::AvAlgoLibrary,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if args.is_null() || out.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                if let Err(error) = unsafe {
                    $crate::macros::validate_abi_header(args, "AvAlgoLibraryArgs")
                } {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                // SAFETY: ABI 头校验通过，args 指向完整的当前版本结构体。
                let raw_args = unsafe { &*args };
                // SAFETY: package_root 与 platform_id 指针由宿主提供
                let root_str = match unsafe { $crate::macros::parse_c_str(raw_args.package_root) } {
                    Ok(s) => s,
                    Err(e) => {
                        $crate::macros::set_last_error(e.to_string());
                        return e.to_c_status();
                    }
                };
                let platform_str =
                    match unsafe { $crate::macros::parse_c_str(raw_args.platform_id) } {
                        Ok(s) => s,
                        Err(e) => {
                            $crate::macros::set_last_error(e.to_string());
                            return e.to_c_status();
                        }
                    };

                let lib_ctx = Box::new($crate::macros::LibraryContext {
                    package_root: std::path::PathBuf::from(root_str),
                    platform_id: platform_str.to_string(),
                });

                if let Err(error) = $open_hook(&lib_ctx.package_root) {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }

                // SAFETY: out 校验过非空
                unsafe {
                    *out = Box::into_raw(lib_ctx) as $crate::c_abi::AvAlgoLibrary;
                }
                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("library_open 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_library_query(
            lib: $crate::c_abi::AvAlgoLibrary,
            out: *mut $crate::c_abi::AvAlgoLibraryInfo,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if lib.is_null() || out.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                if let Err(error) = unsafe {
                    $crate::macros::validate_abi_header(out, "AvAlgoLibraryInfo")
                } {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                // SAFETY: library_query 只接受由 library_open 创建的非空句柄。
                let _ = lib;
                // SAFETY: out 已通过 ABI 头校验。
                let info = unsafe { &mut *out };
                info.size = std::mem::size_of::<$crate::c_abi::AvAlgoLibraryInfo>() as u32;
                info.api_version = $crate::c_abi::AV_ALGO_API_VERSION;

                $crate::macros::copy_str_to_c_chars($id, &mut info.algorithm_id);
                $crate::macros::copy_str_to_c_chars($ver, &mut info.version);
                $crate::macros::copy_str_to_c_chars($atype, &mut info.algorithm_type);
                $crate::macros::copy_str_to_c_chars($alarm, &mut info.alarm_type_id);

                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("library_query 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_library_close(
            lib: $crate::c_abi::AvAlgoLibrary,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if !lib.is_null() {
                    // SAFETY: 回收 library_open 中通过 Box::into_raw 创建的内存
                    let lib_ctx = unsafe {
                        Box::from_raw(lib as *mut $crate::macros::LibraryContext)
                    };
                    $close_hook(&lib_ctx.package_root);
                    drop(lib_ctx);
                }
                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("library_close 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_create(
            lib: $crate::c_abi::AvAlgoLibrary,
            args: *const $crate::c_abi::AvAlgoInstanceArgs,
            out: *mut $crate::c_abi::AvAlgoInstance,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if lib.is_null() || args.is_null() || out.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                if let Err(error) = unsafe {
                    $crate::macros::validate_abi_header(args, "AvAlgoInstanceArgs")
                } {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                // SAFETY: ABI 头校验通过，句柄和 args 在本次同步调用期间有效。
                let lib_ctx = unsafe { &*(lib as *const $crate::macros::LibraryContext) };
                let raw_args = unsafe { &*args };
                if raw_args.mode != $crate::c_abi::AV_INSTANCE_NORMAL
                    && raw_args.mode != $crate::c_abi::AV_INSTANCE_INSTALL_SELF_TEST
                {
                    let error = $crate::error::AlgoError::ConfigParse {
                        reason: "实例运行模式未知".to_string(),
                    };
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                unsafe {
                    *out = std::ptr::null_mut();
                }

                let instance_id = match unsafe {
                    $crate::macros::parse_c_str(raw_args.instance_id)
                } {
                    Ok(s) if !s.is_empty() => s,
                    Ok(_) => {
                        let error = $crate::error::AlgoError::ConfigParse {
                            reason: "instance_id 不能为空".to_string(),
                        };
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };

                let config_json = match unsafe {
                    $crate::macros::parse_bytes_with_len(
                        raw_args.config_json,
                        raw_args.config_json_len,
                    )
                } {
                    Ok(json) => json,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };
                let config = if config_json.trim().is_empty() {
                    Default::default()
                } else {
                    match serde_json::from_str(config_json) {
                        Ok(config) => config,
                        Err(error) => {
                            $crate::macros::set_last_error(format!("配置解析错误: {error}"));
                            return $crate::c_abi::AV_ERR_CONFIG_INVALID;
                        }
                    }
                };

                // SAFETY: image_ops 由宿主提供，engine_for_image_ops 会复制并校验 ABI 表。
                let engine = match unsafe { $crate::cv::engine_for_image_ops(raw_args.image_ops) }
                {
                    Ok(engine) => engine,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };

                let rules = match unsafe {
                    $crate::macros::validate_rules(raw_args.rules, raw_args.rule_count)
                } {
                    Ok(rules) => rules,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };

                let ctx = $crate::plugin::InitContext {
                    package_root: &lib_ctx.package_root,
                    platform_id: &lib_ctx.platform_id,
                    instance_id,
                    is_self_test: raw_args.mode == $crate::c_abi::AV_INSTANCE_INSTALL_SELF_TEST,
                };

                let mut plugin = match <$plugin_ty as $crate::plugin::AlgoPlugin>::init(&ctx, config) {
                    Ok(plugin) => plugin,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };
                if let Err(error) = plugin.set_rules(rules) {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }

                let inst_ctx = Box::new($crate::macros::InstanceContext {
                    plugin: std::sync::Mutex::new(plugin),
                    engine,
                    on_result: raw_args.on_result,
                    user_data: raw_args.result_user,
                });

                // SAFETY: out 非空且由调用方提供可写句柄槽位。
                unsafe {
                    *out = Box::into_raw(inst_ctx) as $crate::c_abi::AvAlgoInstance;
                }
                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_create 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_negotiate(
            inst: $crate::c_abi::AvAlgoInstance,
            offered: *const $crate::c_abi::AvFrameCaps,
            accepted: *mut $crate::c_abi::AvFrameCaps,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if inst.is_null() || offered.is_null() || accepted.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                let offered_result = unsafe {
                    $crate::macros::validate_abi_header(offered, "AvFrameCaps")
                };
                if let Err(error) = offered_result {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                let accepted_result = unsafe {
                    $crate::macros::validate_abi_header(
                        accepted as *const $crate::c_abi::AvFrameCaps,
                        "AvFrameCaps",
                    )
                };
                if let Err(error) = accepted_result {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                // 先按值读取两个完整 POD，避免对部分重叠的 offered/accepted 同时创建
                // 共享引用与可变引用；之后用局部副本写回，完全覆盖别名场景。
                let offered_value = unsafe { offered.read() };
                let accepted_value = unsafe { accepted.read() };
                if let Err(error) = $crate::macros::validate_frame_caps(&offered_value)
                    .and_then(|_| $crate::macros::validate_frame_caps(&accepted_value))
                {
                    $crate::macros::set_last_error(error.to_string());
                    return error.to_c_status();
                }
                if offered != accepted as *const $crate::c_abi::AvFrameCaps {
                    // SAFETY: 两个指针均已通过 ABI/对齐/完整结构体校验，局部副本与目标
                    // 不存在别名读写关系，即使原始区域部分重叠也不会破坏读取结果。
                    unsafe {
                        accepted.write(offered_value);
                    }
                }
                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_negotiate 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_update_config(
            inst: $crate::c_abi::AvAlgoInstance,
            json: *const std::ffi::c_char,
            len: u32,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if inst.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                let config_json = match unsafe { $crate::macros::parse_bytes_with_len(json, len) } {
                    Ok(json) => json,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };
                let config = if config_json.trim().is_empty() {
                    Default::default()
                } else {
                    match serde_json::from_str::<
                        <$plugin_ty as $crate::plugin::AlgoPlugin>::Config,
                    >(config_json)
                    {
                        Ok(config) => config,
                        Err(error) => {
                            $crate::macros::set_last_error(format!("配置解析错误: {error}"));
                            return $crate::c_abi::AV_ERR_CONFIG_INVALID;
                        }
                    }
                };
                // SAFETY: inst 必须是由当前 ABI 的 instance_create 创建的有效句柄。
                let ctx = unsafe {
                    &*(inst as *const $crate::macros::InstanceContext<$plugin_ty>)
                };
                let mut plugin = match ctx.plugin.lock() {
                    Ok(plugin) => plugin,
                    Err(_) => {
                        let error = $crate::error::AlgoError::Internal {
                            reason: "插件实例锁已中毒".to_string(),
                        };
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };
                match plugin.update_config(config) {
                    Ok(()) => $crate::c_abi::AV_OK,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        error.to_c_status()
                    }
                }
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_update_config 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_set_rules(
            inst: $crate::c_abi::AvAlgoInstance,
            rules: *const $crate::c_abi::AvRule,
            count: u32,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if inst.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                let rules_slice = match unsafe { $crate::macros::validate_rules(rules, count) } {
                    Ok(rules) => rules,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };
                // SAFETY: inst 必须是由当前 ABI 的 instance_create 创建的有效句柄。
                let ctx = unsafe {
                    &*(inst as *const $crate::macros::InstanceContext<$plugin_ty>)
                };
                let mut plugin = match ctx.plugin.lock() {
                    Ok(plugin) => plugin,
                    Err(_) => {
                        let error = $crate::error::AlgoError::Internal {
                            reason: "插件实例锁已中毒".to_string(),
                        };
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };

                match plugin.set_rules(rules_slice) {
                    Ok(()) => $crate::c_abi::AV_OK,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        error.to_c_status()
                    }
                }
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_set_rules 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_process(
            inst: $crate::c_abi::AvAlgoInstance,
            frame: *const $crate::c_abi::AvFrameDesc,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if inst.is_null() || frame.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                // SAFETY: inst 必须是由当前 ABI 的 instance_create 创建的有效句柄。
                let ctx = unsafe {
                    &*(inst as *const $crate::macros::InstanceContext<$plugin_ty>)
                };
                let safe_frame = match unsafe {
                    $crate::frame::SafeFrame::from_raw_checked(frame)
                } {
                    Ok(frame) => frame,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        return error.to_c_status();
                    }
                };

                let result = $crate::cv::with_engine(ctx.engine.clone(), || {
                    let mut plugin = ctx.plugin.lock().map_err(|_| {
                        $crate::error::AlgoError::Internal {
                            reason: "插件实例锁已中毒".to_string(),
                        }
                    })?;
                    let mut emitter = unsafe {
                        $crate::emitter::ResultEmitter::from_raw(
                            safe_frame.frame_id(),
                            ctx.on_result,
                            ctx.user_data,
                        )
                    };
                    plugin.process(safe_frame, &mut emitter)
                });

                match result {
                    Ok(()) => $crate::c_abi::AV_OK,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        error.to_c_status()
                    }
                }
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_process 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_flush(
            inst: $crate::c_abi::AvAlgoInstance,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if inst.is_null() {
                    return $crate::c_abi::AV_ERR_INVALID_ARG;
                }
                // SAFETY: inst 必须是由当前 ABI 的 instance_create 创建的有效句柄。
                let ctx = unsafe {
                    &*(inst as *const $crate::macros::InstanceContext<$plugin_ty>)
                };
                let result = $crate::cv::with_engine(ctx.engine.clone(), || {
                    let mut plugin = ctx.plugin.lock().map_err(|_| {
                        $crate::error::AlgoError::Internal {
                            reason: "插件实例锁已中毒".to_string(),
                        }
                    })?;
                    let mut emitter = unsafe {
                        $crate::emitter::ResultEmitter::from_raw(0, ctx.on_result, ctx.user_data)
                    };
                    plugin.flush(&mut emitter)
                });

                match result {
                    Ok(()) => $crate::c_abi::AV_OK,
                    Err(error) => {
                        $crate::macros::set_last_error(error.to_string());
                        error.to_c_status()
                    }
                }
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_flush 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_instance_destroy(
            inst: $crate::c_abi::AvAlgoInstance,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if !inst.is_null() {
                    // SAFETY: 回收 instance_create 中创建的内存
                    let _ = unsafe {
                        Box::from_raw(inst as *mut $crate::macros::InstanceContext<$plugin_ty>)
                    };
                }
                $crate::c_abi::AV_OK
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("instance_destroy 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        unsafe extern "C" fn __algo_last_error(
            _inst: $crate::c_abi::AvAlgoInstance,
            buf: *mut std::ffi::c_char,
            cap: u32,
        ) -> std::ffi::c_int {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // SAFETY: copy_last_error 只在指针和容量校验后写入调用方缓冲区。
                unsafe { $crate::macros::copy_last_error(buf, cap) }
            }))
            .unwrap_or_else(|_| {
                $crate::macros::set_last_error("last_error 发生 Panic 崩溃");
                $crate::c_abi::AV_ERR_INTERNAL
            })
        }

        // 静态虚表实例
        static __ALGO_ABI: $crate::c_abi::AvAlgoAbi = $crate::c_abi::AvAlgoAbi {
            size: std::mem::size_of::<$crate::c_abi::AvAlgoAbi>() as u32,
            api_version: $crate::c_abi::AV_ALGO_API_VERSION,
            library_open: Some(__algo_library_open),
            library_query: Some(__algo_library_query),
            library_close: Some(__algo_library_close),
            instance_create: Some(__algo_instance_create),
            instance_negotiate: Some(__algo_instance_negotiate),
            instance_update_config: Some(__algo_instance_update_config),
            instance_set_rules: Some(__algo_instance_set_rules),
            instance_process: Some(__algo_instance_process),
            instance_flush: Some(__algo_instance_flush),
            instance_destroy: Some(__algo_instance_destroy),
            last_error: Some(__algo_last_error),
        };

        /// C ABI 唯一定位符号导出
        #[no_mangle]
        pub unsafe extern "C" fn av_algo_get_abi(
            requested_api_version: u32,
        ) -> *const $crate::c_abi::AvAlgoAbi {
            if requested_api_version == $crate::c_abi::AV_ALGO_API_VERSION {
                &__ALGO_ABI
            } else {
                std::ptr::null()
            }
        }
    };
}
