//! C ABI 动态库安全加载与 RAII 资源生命周期管理

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use libloading::{Library, Symbol};

use crate::c_abi::types::*;
use crate::error::InferError;

const MAX_FACE_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESULT_JSON_BYTES: usize = 1024 * 1024;
const MAX_RESULT_IMAGES: usize = 1024;

/// 当前进程可读内存映射的快照。
///
/// 一次 ABI 校验可能需要检查头部、JSON 和图片请求数组；复用快照可以避免在同一
/// 回调中重复读取 `/proc/self/maps`。快照仍只覆盖当前同步调用，不能替代沙箱隔离。
#[derive(Debug)]
struct ReadableMemoryMap {
    #[cfg(target_os = "linux")]
    maps: String,
}

impl ReadableMemoryMap {
    #[cfg(target_os = "linux")]
    fn capture() -> Self {
        Self {
            maps: std::fs::read_to_string("/proc/self/maps").unwrap_or_default(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn capture() -> Self {
        Self {}
    }

    #[cfg(target_os = "linux")]
    fn contains(&self, start: usize, end: usize) -> bool {
        readable_linux_memory_range(&self.maps, start, end)
    }

    #[cfg(target_os = "macos")]
    fn contains(&self, start: usize, end: usize) -> bool {
        readable_mach_memory_range(start, end)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn contains(&self, _start: usize, _end: usize) -> bool {
        false
    }
}

/// 检查由插件返回的裸地址是否按指定类型对齐，并覆盖在当前进程的可读映射中。
///
/// 这不是对恶意进程内插件的完整隔离：映射检查与实际解引用之间仍存在 TOCTOU
/// 窗口。因此未信任插件仍必须经过沙箱子进程验证；这里的检查负责拒绝明显的野指针、
/// 整数溢出和跨越不可读内存的常见 ABI 错误，避免安全层直接构造未定义行为的 slice。
fn validate_readable_range(
    memory: &ReadableMemoryMap,
    ptr: *const c_void,
    len: usize,
    alignment: usize,
    name: &str,
) -> Result<(), String> {
    let address = ptr as usize;
    if address == 0 {
        return Err(format!("{name} 指针为空"));
    }
    if alignment == 0 || !address.is_multiple_of(alignment) {
        return Err(format!("{name} 指针未按 {alignment} 字节对齐"));
    }
    let end = address
        .checked_add(len)
        .ok_or_else(|| format!("{name} 地址范围溢出"))?;
    if !memory.contains(address, end) {
        return Err(format!("{name} 指向的内存范围不可读"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn readable_linux_memory_range(maps: &str, start: usize, end: usize) -> bool {
    let mut cursor = start;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let Some(range) = fields.next() else {
            continue;
        };
        let Some(permissions) = fields.next() else {
            continue;
        };
        let Some((range_start, range_end)) = range.split_once('-') else {
            continue;
        };
        let (Ok(range_start), Ok(range_end)) = (
            usize::from_str_radix(range_start, 16),
            usize::from_str_radix(range_end, 16),
        ) else {
            continue;
        };

        if range_start <= cursor && cursor < range_end {
            if !permissions
                .as_bytes()
                .first()
                .is_some_and(|&flag| flag == b'r')
            {
                return false;
            }
            cursor = range_end.min(end);
            if cursor >= end {
                return true;
            }
        }
    }

    false
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn readable_mach_memory_range(start: usize, end: usize) -> bool {
    let mut address = start as u64;
    let end = end as u64;
    let mut remaining = end.saturating_sub(address);
    while remaining > 0 {
        let mut region_address = address;
        let mut region_size = remaining;
        let mut info = [0_i32; VM_REGION_BASIC_INFO_COUNT_64 as usize];
        let mut info_count = VM_REGION_BASIC_INFO_COUNT_64;
        let mut object_name = 0_u32;
        // SAFETY: 读取当前进程的 Mach task 句柄；该句柄由系统提供且仅用于查询内存区域。
        let task = unsafe { libc::mach_task_self_ };
        // SAFETY: 所有输出参数均指向本地可写存储；task 是当前进程任务句柄。
        let status = unsafe {
            mach_vm_region(
                task,
                &mut region_address,
                &mut region_size,
                VM_REGION_BASIC_INFO_64,
                info.as_mut_ptr(),
                &mut info_count,
                &mut object_name,
            )
        };
        if status != 0
            || region_size == 0
            || region_address > address
            || region_address.saturating_add(region_size) <= address
            || info[0] & libc::VM_PROT_READ == 0
        {
            return false;
        }

        let region_end = region_address.saturating_add(region_size);
        let next = region_end.min(end);
        if next <= address {
            return false;
        }
        remaining = end.saturating_sub(next);
        address = next;
    }
    true
}

#[cfg(target_os = "macos")]
const VM_REGION_BASIC_INFO_64: u32 = 9;
#[cfg(target_os = "macos")]
const VM_REGION_BASIC_INFO_COUNT_64: u32 = 9;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_vm_region(
        target_task: u32,
        address: *mut u64,
        size: *mut u64,
        flavor: u32,
        info: *mut i32,
        info_count: *mut u32,
        object_name: *mut u32,
    ) -> i32;
}

/// 校验并复制插件返回的 ABI 虚表。
fn copy_validated_abi(abi_ptr: *const AvAlgoAbi) -> Result<AvAlgoAbi, InferError> {
    let memory = ReadableMemoryMap::capture();
    let expected_size = std::mem::size_of::<AvAlgoAbi>();
    validate_readable_range(
        &memory,
        abi_ptr.cast(),
        std::mem::size_of::<u32>() * 2,
        std::mem::align_of::<AvAlgoAbi>(),
        "AvAlgoAbi 头部",
    )
    .map_err(|reason| InferError::InvalidAbi { reason })?;

    // SAFETY: 上面的范围与对齐校验保证两个 ABI 头字段在当前调用点可读。
    let (size, api_version) = unsafe {
        (
            ptr::read(abi_ptr.cast::<u32>()),
            ptr::read(abi_ptr.cast::<u32>().add(1)),
        )
    };
    if size as usize != expected_size {
        return Err(InferError::InvalidAbi {
            reason: format!("虚表大小不匹配: 期望 {expected_size} 字节, 实际 {size} 字节"),
        });
    }
    if api_version != AV_ALGO_API_VERSION {
        return Err(InferError::InvalidAbi {
            reason: format!("API 版本不匹配: 期望 {AV_ALGO_API_VERSION}, 实际 {api_version}"),
        });
    }
    validate_readable_range(
        &memory,
        abi_ptr.cast(),
        expected_size,
        std::mem::align_of::<AvAlgoAbi>(),
        "AvAlgoAbi",
    )
    .map_err(|reason| InferError::InvalidAbi { reason })?;

    // SAFETY: 已确认当前版本完整 AvAlgoAbi 的内存范围可读且正确对齐。
    Ok(unsafe { ptr::read(abi_ptr) })
}

/// 校验结果回调中由插件提供的完整结果及其变长载荷。
fn copy_validated_result(result: *const AvAlgoResult) -> Result<AvAlgoResult, String> {
    let memory = ReadableMemoryMap::capture();
    validate_readable_range(
        &memory,
        result.cast(),
        std::mem::size_of::<u32>() * 2,
        std::mem::align_of::<AvAlgoResult>(),
        "AvAlgoResult 头部",
    )?;
    // SAFETY: 头部范围和对齐已验证，两个 u32 可以安全读取。
    let (size, api_version) = unsafe {
        (
            ptr::read(result.cast::<u32>()),
            ptr::read(result.cast::<u32>().add(1)),
        )
    };
    let expected_size = std::mem::size_of::<AvAlgoResult>() as u32;
    if size != expected_size {
        return Err(format!(
            "AvAlgoResult.size 无效: 期望 {expected_size}, 实际 {size}"
        ));
    }
    if api_version != AV_ALGO_API_VERSION {
        return Err(format!(
            "AvAlgoResult.api_version 无效: 期望 {AV_ALGO_API_VERSION}, 实际 {api_version}"
        ));
    }
    validate_readable_range(
        &memory,
        result.cast(),
        expected_size as usize,
        std::mem::align_of::<AvAlgoResult>(),
        "AvAlgoResult",
    )?;
    // SAFETY: 完整 AvAlgoResult 已通过范围和头部校验。
    let result = unsafe { ptr::read(result) };

    if !matches!(
        result.kind,
        AV_RESULT_ALARM | AV_RESULT_SELF_TEST | AV_RESULT_RECOGNITION
    ) {
        return Err(format!("AvAlgoResult.kind 无效: {}", result.kind));
    }
    if result.reserved0 != 0 {
        return Err("AvAlgoResult.reserved0 必须为 0".to_string());
    }
    let json_len = result.json_len as usize;
    if json_len > MAX_RESULT_JSON_BYTES {
        return Err(format!(
            "AvAlgoResult.json_len 超过上限: {} > {MAX_RESULT_JSON_BYTES}",
            result.json_len
        ));
    }
    if json_len > 0 {
        validate_readable_range(
            &memory,
            result.json.cast(),
            json_len,
            1,
            "AvAlgoResult.json",
        )?;
    }

    let image_count = result.image_count as usize;
    if image_count > MAX_RESULT_IMAGES {
        return Err(format!(
            "AvAlgoResult.image_count 超过上限: {} > {MAX_RESULT_IMAGES}",
            result.image_count
        ));
    }
    if image_count > 0 {
        let byte_len = image_count
            .checked_mul(std::mem::size_of::<AvAlgoImageReq>())
            .ok_or_else(|| "AvAlgoResult.images 长度计算溢出".to_string())?;
        validate_readable_range(
            &memory,
            result.images.cast(),
            byte_len,
            std::mem::align_of::<AvAlgoImageReq>(),
            "AvAlgoResult.images",
        )?;
        // SAFETY: images 的完整数组范围已验证可读且按 AvAlgoImageReq 对齐。
        let images = unsafe { std::slice::from_raw_parts(result.images, image_count) };
        for (index, image) in images.iter().enumerate() {
            if image.size != std::mem::size_of::<AvAlgoImageReq>() as u32 {
                return Err(format!("AvAlgoImageReq[{index}].size 无效"));
            }
            if image.api_version != AV_ALGO_API_VERSION {
                return Err(format!("AvAlgoImageReq[{index}].api_version 无效"));
            }
            if !image.x.is_finite()
                || !image.y.is_finite()
                || !image.w.is_finite()
                || !image.h.is_finite()
                || image.x < 0.0
                || image.y < 0.0
                || image.w < 0.0
                || image.h < 0.0
                || image.x > 1.0
                || image.y > 1.0
                || image.w > 1.0
                || image.h > 1.0
                || image.x + image.w > 1.0
                || image.y + image.h > 1.0
            {
                return Err(format!("AvAlgoImageReq[{index}] 几何范围无效"));
            }
        }
    } else if !result.images.is_null() {
        return Err("AvAlgoResult.image_count 为 0 时 images 必须为空".to_string());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_abi() -> AvAlgoAbi {
        AvAlgoAbi {
            size: std::mem::size_of::<AvAlgoAbi>() as u32,
            api_version: AV_ALGO_API_VERSION,
            library_open: None,
            library_query: None,
            library_close: None,
            instance_create: None,
            instance_negotiate: None,
            instance_update_config: None,
            instance_set_rules: None,
            instance_process: None,
            instance_flush: None,
            instance_destroy: None,
            last_error: None,
        }
    }

    #[test]
    fn test_abi_validation_accepts_valid_table() {
        let abi = valid_abi();
        let copied = copy_validated_abi(&abi).expect("valid ABI table");
        assert_eq!(copied.size, abi.size);
        assert_eq!(copied.api_version, abi.api_version);
    }

    #[test]
    fn test_abi_validation_rejects_unaligned_table_pointer() {
        let abi = valid_abi();
        // SAFETY: 仅在测试中构造一个刻意错位的裸指针，验证校验器在解引用前拒绝它。
        let pointer = unsafe {
            (std::ptr::addr_of!(abi) as *const u8)
                .add(1)
                .cast::<AvAlgoAbi>()
        };
        let error = copy_validated_abi(pointer).expect_err("unaligned ABI pointer must fail");
        assert!(matches!(error, InferError::InvalidAbi { reason } if reason.contains("对齐")));
    }

    fn valid_result(json: &[u8]) -> AvAlgoResult {
        AvAlgoResult {
            size: std::mem::size_of::<AvAlgoResult>() as u32,
            api_version: AV_ALGO_API_VERSION,
            kind: AV_RESULT_ALARM,
            reserved0: 0,
            frame_id: 1,
            json: json.as_ptr().cast(),
            json_len: json.len() as u32,
            image_count: 0,
            images: std::ptr::null(),
        }
    }

    #[test]
    fn test_result_validation_accepts_valid_payload() {
        let json = br#"{"objects":[]}"#;
        let result = valid_result(json);
        let checked = copy_validated_result(&result).expect("valid result");
        assert_eq!(checked.kind, AV_RESULT_ALARM);
        assert_eq!(checked.json_len, json.len() as u32);
    }

    #[test]
    fn test_result_validation_rejects_invalid_header_and_kind() {
        let json = b"{}";
        let mut result = valid_result(json);

        result.size -= 1;
        assert!(copy_validated_result(&result)
            .expect_err("invalid result size must fail")
            .contains("size"));

        result.size = std::mem::size_of::<AvAlgoResult>() as u32;
        result.api_version = AV_ALGO_API_VERSION + 1;
        assert!(copy_validated_result(&result)
            .expect_err("invalid result version must fail")
            .contains("api_version"));

        result.api_version = AV_ALGO_API_VERSION;
        result.kind = 99;
        assert!(copy_validated_result(&result)
            .expect_err("invalid result kind must fail")
            .contains("kind"));
    }

    #[test]
    fn test_result_validation_rejects_unbounded_or_invalid_payloads() {
        let json = b"{}";
        let mut result = valid_result(json);

        result.json_len = (MAX_RESULT_JSON_BYTES + 1) as u32;
        assert!(copy_validated_result(&result)
            .expect_err("oversized json must fail")
            .contains("json_len"));

        result = valid_result(json);
        result.json = std::ptr::null();
        assert!(copy_validated_result(&result)
            .expect_err("non-zero json length with null pointer must fail")
            .contains("json"));

        result = valid_result(b"");
        result.image_count = (MAX_RESULT_IMAGES + 1) as u32;
        assert!(copy_validated_result(&result)
            .expect_err("oversized image array must fail")
            .contains("image_count"));
    }

    #[test]
    fn test_result_validation_rejects_unaligned_result_pointer() {
        let bytes = vec![0_u8; std::mem::size_of::<AvAlgoResult>() + 1];
        // SAFETY: 仅在测试中构造一个刻意错位的裸指针，验证校验器在解引用前拒绝它。
        let result = unsafe { bytes.as_ptr().add(1).cast::<AvAlgoResult>() };
        assert!(copy_validated_result(result)
            .expect_err("unaligned result pointer must fail")
            .contains("对齐"));
    }
}

/// 封装已加载的动态库与 C ABI 虚表
#[derive(Debug)]
pub struct LoadedLib {
    // 必须持有 Library 以保证其内部映射在内存中的函数指针在结构体存活期间有效
    _lib: Library,
    abi: AvAlgoAbi,
}

// C ABI 函数表中的函数指针只在 `_lib` 保持加载期间使用；Library 自身已提供跨线程所有权语义。

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

        // SAFETY: 调用导出的获取虚表函数，传入当前支持的 API 版本。
        let abi_ptr = unsafe { get_abi_fn(AV_ALGO_API_VERSION) };
        if abi_ptr.is_null() {
            return Err(InferError::InvalidAbi {
                reason: "av_algo_get_abi 返回了空指针".to_string(),
            });
        }

        let abi = copy_validated_abi(abi_ptr)?;
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
        if code != AV_OK {
            // SAFETY: abi 有效；library-level 错误使用空 instance 句柄读取线程局部错误。
            let error = unsafe { check_c_status(code, abi, std::ptr::null_mut()) };
            if !raw_lib.is_null() {
                if let Some(close_fn) = abi.library_close {
                    // SAFETY: raw_lib 由当前 library_open 返回，失败路径尚未交付给调用方。
                    unsafe { close_fn(raw_lib) };
                }
            }
            return Err(error);
        }
        if raw_lib.is_null() {
            return Err(InferError::InvalidAbi {
                reason: "library_open 成功但返回了空库句柄".to_string(),
            });
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

        let query_fn = match abi.library_query {
            Some(query_fn) => query_fn,
            None => {
                if let Some(close_fn) = abi.library_close {
                    // SAFETY: raw_lib 由当前 library_open 返回，失败路径尚未交付给调用方。
                    unsafe { close_fn(raw_lib) };
                }
                return Err(InferError::InvalidAbi {
                    reason: "library_query 函数指针为空".to_string(),
                });
            }
        };

        // SAFETY: raw_lib 为有效句柄；info 为有效栈内存
        let query_code = unsafe { query_fn(raw_lib, &mut info) };
        if query_code != AV_OK {
            // SAFETY: abi 有效；先读取错误详情，再释放已打开的 raw_lib。
            let error = unsafe { check_c_status(query_code, abi, std::ptr::null_mut()) };
            if let Some(close_fn) = abi.library_close {
                // SAFETY: raw_lib 由当前 open_fn 创建且未交付，释放它以防泄漏。
                unsafe { close_fn(raw_lib) };
            }
            return Err(error);
        }
        if info.size != std::mem::size_of::<AvAlgoLibraryInfo>() as u32
            || info.api_version != AV_ALGO_API_VERSION
        {
            if let Some(close_fn) = abi.library_close {
                // SAFETY: raw_lib 由当前 open_fn 创建且未交付，释放它以防泄漏。
                unsafe { close_fn(raw_lib) };
            }
            return Err(InferError::InvalidAbi {
                reason: format!(
                    "AvAlgoLibraryInfo 头部无效: size={}, api_version={}",
                    info.size, info.api_version
                ),
            });
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
        if jpeg_bytes.is_empty() || jpeg_bytes.len() > MAX_FACE_IMAGE_BYTES {
            return Err(InferError::Execution {
                reason: format!(
                    "人脸提取输入图像大小无效: {} bytes（允许 1..={MAX_FACE_IMAGE_BYTES}）",
                    jpeg_bytes.len()
                ),
            });
        }

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

        // SAFETY: input/output 结构体由宿主完整初始化，并在同步调用期间保持有效。
        let status = unsafe { extract_fn(self.raw, &input, &mut output) };
        let expected_output_size = std::mem::size_of::<AvFaceExtractOutput>() as u32;
        if output.size != expected_output_size || output.api_version != AV_ALGO_API_VERSION {
            return Err(InferError::InvalidAbi {
                reason: format!(
                    "AvFaceExtractOutput 头部无效: size={}, api_version={}",
                    output.size, output.api_version
                ),
            });
        }

        if status != AV_OK || output.status_code != 0 {
            // av_algo_extract_face 使用库句柄；last_error 的 inst_or_null 参数在此必须传空，
            // 不能把 AvAlgoLibrary 强转成 AvAlgoInstance。
            // SAFETY: abi 有效；库级错误按 ABI 契约使用空 instance 句柄读取线程局部错误。
            let detail = unsafe { check_c_status(status, self.lib.abi(), std::ptr::null_mut()) };
            return Err(InferError::Execution {
                reason: format!(
                    "人脸特征提取失败: plugin_status={status}, plugin_code={}, detail={detail}",
                    output.status_code
                ),
            });
        }

        if output.embedding.is_null() || output.embedding_dim != 512 {
            return Err(InferError::Execution {
                reason: format!(
                    "人脸特征向量无效: ptr={:?}, dim={}",
                    output.embedding, output.embedding_dim
                ),
            });
        }
        let embedding_len = (output.embedding_dim as usize)
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or_else(|| InferError::Execution {
                reason: "人脸特征向量长度计算溢出".to_string(),
            })?;
        let memory = ReadableMemoryMap::capture();
        validate_readable_range(
            &memory,
            output.embedding.cast(),
            embedding_len,
            std::mem::align_of::<f32>(),
            "人脸特征向量",
        )
        .map_err(|reason| InferError::Execution { reason })?;

        // SAFETY: embedding 的维度、对齐和完整字节范围均已校验。
        let embedding = unsafe {
            std::slice::from_raw_parts(output.embedding, output.embedding_dim as usize).to_vec()
        };
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(InferError::Execution {
                reason: "人脸特征向量包含 NaN/Inf".to_string(),
            });
        }

        let aligned_jpeg_len = output.aligned_jpeg_len as usize;
        let aligned_jpeg = if aligned_jpeg_len > 0 {
            if aligned_jpeg_len > MAX_FACE_IMAGE_BYTES {
                return Err(InferError::Execution {
                    reason: format!("人脸对齐切片尺寸异常超过 {MAX_FACE_IMAGE_BYTES} bytes 限制"),
                });
            }
            if output.aligned_jpeg.is_null() {
                return Err(InferError::Execution {
                    reason: "人脸对齐切片长度非零但指针为空".to_string(),
                });
            }
            validate_readable_range(
                &memory,
                output.aligned_jpeg.cast(),
                aligned_jpeg_len,
                1,
                "人脸对齐切片",
            )
            .map_err(|reason| InferError::Execution { reason })?;
            // SAFETY: aligned_jpeg 的长度上限、非空指针和可读范围均已校验。
            unsafe { std::slice::from_raw_parts(output.aligned_jpeg, aligned_jpeg_len).to_vec() }
        } else {
            if !output.aligned_jpeg.is_null() {
                return Err(InferError::Execution {
                    reason: "人脸对齐切片指针非空但长度为 0".to_string(),
                });
            }
            Vec::new()
        };

        if !output.quality_score.is_finite() || !output.detection_score.is_finite() {
            return Err(InferError::Execution {
                reason: "人脸质量分或检测分包含 NaN/Inf".to_string(),
            });
        }

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
        if user_data.is_null() {
            return;
        }

        let res = match copy_validated_result(result) {
            Ok(result) => result,
            Err(reason) => {
                tracing::warn!(error = %reason, "丢弃不符合 ABI 契约的算法结果回调");
                return;
            }
        };
        if res.json.is_null() || res.json_len == 0 {
            return;
        }

        // SAFETY: copy_validated_result 已验证 json 的非空、上限、映射范围和长度。
        let json_slice =
            unsafe { std::slice::from_raw_parts(res.json.cast::<u8>(), res.json_len as usize) };
        let json_str = String::from_utf8_lossy(json_slice).to_string();

        // SAFETY: user_data 由宿主在 instance_create 时指向存活期内的 Mutex<Vec<String>>。
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
