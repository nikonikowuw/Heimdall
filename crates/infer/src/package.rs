//! 算法包运行时、实例管理与 InferenceBackend 适配接入

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};
use types::{BoundingBox, Detection, FrameHandle, FrameRef, PixelFormat};

use crate::backend::InferenceBackend;
use crate::c_abi::loader::{check_c_status, LoadedLib, RawAlgoLibrary};
use crate::c_abi::types::*;
use crate::error::InferError;
use crate::sandbox::{find_entry_library, AlgoManifest, AlgoSandbox};

/// 算法描述清单固定文件名
pub const ALGO_MANIFEST_FILENAME: &str = "manifest.json";

/// 算法包默认存储与加载目录
pub const DEFAULT_ALGO_PACKAGES_DIR: &str = "algo-packages";

/// 已通过沙箱自检并在主进程中加载的算法包
#[derive(Debug)]
pub struct AlgoPackage {
    manifest: AlgoManifest,
    package_dir: PathBuf,
    lib: Arc<LoadedLib>,
    raw_lib: Arc<RawAlgoLibrary>,
}

impl AlgoPackage {
    /// 通过沙箱安全校验并打开算法包
    pub fn load_and_verify(package_dir: &Path, use_subprocess: bool) -> Result<Self, InferError> {
        let manifest = AlgoSandbox::validate_package(package_dir, use_subprocess)?;
        let entry_lib = find_entry_library(package_dir, &manifest.algorithm_id)?;

        let loaded_lib = Arc::new(LoadedLib::load(&entry_lib)?);
        let raw_lib = Arc::new(RawAlgoLibrary::open(
            loaded_lib.clone(),
            package_dir,
            &manifest.platform_id,
        )?);

        Ok(Self {
            manifest,
            package_dir: package_dir.to_path_buf(),
            lib: loaded_lib,
            raw_lib,
        })
    }

    #[inline]
    pub fn manifest(&self) -> &AlgoManifest {
        &self.manifest
    }

    #[inline]
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    /// 创建一个独立的推理实例
    pub fn create_instance(
        self: &Arc<Self>,
        instance_id: &str,
        config_json: Option<&str>,
    ) -> Result<AlgoInstance, InferError> {
        let inst_id_c = CString::new(instance_id).map_err(|_| InferError::Execution {
            reason: "instance_id 包含非法空字节".to_string(),
        })?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let run_id_c = CString::new(run_id.as_str()).map_err(|_| InferError::Execution {
            reason: "run_id 包含非法空字节".to_string(),
        })?;

        let config_c =
            config_json
                .map(CString::new)
                .transpose()
                .map_err(|_| InferError::Execution {
                    reason: "config_json 包含非法空字节".to_string(),
                })?;

        let (cfg_ptr, cfg_len) = match &config_c {
            Some(c) => (c.as_ptr(), c.as_bytes().len() as u32),
            None => (std::ptr::null(), 0),
        };

        let callback_slot = Box::new(std::sync::Mutex::new(Vec::<String>::new()));

        let args = AvAlgoInstanceArgs {
            size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
            api_version: AV_ALGO_API_VERSION,
            mode: AV_INSTANCE_NORMAL,
            reserved0: 0,
            instance_id: inst_id_c.as_ptr(),
            instance_run_id: run_id_c.as_ptr(),
            config_json: cfg_ptr,
            config_json_len: cfg_len,
            reserved1: 0,
            frame_ops: std::ptr::null(),
            image_ops: std::ptr::null(),
            on_result: Some(crate::c_abi::loader::algo_result_collector),
            result_user: callback_slot.as_ref() as *const std::sync::Mutex<Vec<String>>
                as *mut c_void,
            rules: std::ptr::null(),
            rule_count: 0,
        };

        let mut raw_inst: AvAlgoInstance = std::ptr::null_mut();
        let abi = self.lib.abi();
        let create_fn = abi.instance_create.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_create 为空".to_string(),
        })?;

        // SAFETY: args 在调用期间有效
        let code = unsafe { create_fn(self.raw_lib.raw(), &args, &mut raw_inst) };

        if code != AV_OK || raw_inst.is_null() {
            // SAFETY: 调用方保证 abi 与 inst 内存有效
            return Err(unsafe { check_c_status(code, abi, std::ptr::null_mut()) });
        }

        Ok(AlgoInstance {
            _package: self.clone(),
            lib: self.lib.clone(),
            raw: raw_inst,
            algorithm_id: self.manifest.algorithm_id.clone(),
            callback_slot,
            lock: Mutex::new(()),
        })
    }
}

/// 算法推理实例句柄
#[derive(Debug)]
pub struct AlgoInstance {
    _package: Arc<AlgoPackage>,
    lib: Arc<LoadedLib>,
    raw: AvAlgoInstance,
    algorithm_id: String,
    // 堆分配的实例回调结果槽，地址在实例生命周期内保持稳定
    callback_slot: Box<std::sync::Mutex<Vec<String>>>,
    // 互斥锁确保单实例推理调用的重入安全
    lock: Mutex<()>,
}

impl AlgoInstance {
    #[inline]
    pub fn algorithm_id(&self) -> &str {
        &self.algorithm_id
    }
}

// SAFETY: 互斥锁保护单实例不会发生跨线程并发重入调用，可在线程间转移所有权
unsafe impl Send for AlgoInstance {}
// SAFETY: 内部使用 Mutex 串行化推理调用，支持跨线程共享引用
unsafe impl Sync for AlgoInstance {}

impl Drop for AlgoInstance {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            if let Some(destroy_fn) = self.lib.abi().instance_destroy {
                // SAFETY: raw 实例只释放一次；destroy 执行后 C 库不会再发起任何回调
                unsafe { destroy_fn(self.raw) };
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

#[async_trait]
impl InferenceBackend for AlgoInstance {
    fn name(&self) -> &'static str {
        "C-ABI-AlgoInstance"
    }

    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        let _guard = self.lock.lock().await;

        // 清空上一轮可能残留的回调结果
        if let Ok(mut lock) = self.callback_slot.lock() {
            lock.clear();
        }

        let now_ns = frame.timestamp * 1_000_000;
        let mut desc = AvFrameDesc::default_nv12(
            frame.width,
            frame.height,
            frame.stride.hor_stride as i32,
            frame.stride.hor_stride as i32,
            now_ns,
        );
        desc.alloc_width = frame.stride.hor_stride;
        desc.alloc_height = frame.stride.ver_stride;

        match frame.format {
            PixelFormat::Nv12 => desc.pixel_format = AV_PIX_NV12,
            PixelFormat::Rgba | PixelFormat::Bgr24 => desc.pixel_format = AV_PIX_BGRA,
            _ => desc.pixel_format = AV_PIX_UNKNOWN,
        }

        // 适配原生平台帧零拷贝句柄直通
        match frame.handle() {
            FrameHandle::ApplePixelBuffer { ptr } => {
                desc.opaque = ptr.as_ptr();
                desc.frame_token = ptr.as_ptr();
                desc.opaque_kind = AV_OPAQUE_CVPIXELBUFFER;
                desc.memory_type = AV_MEM_PLATFORM_SURFACE;
                desc.layout = AV_LAYOUT_PLATFORM_NATIVE;
                desc.pixel_format = AV_PIX_NV12;
            }
            #[cfg(target_os = "linux")]
            FrameHandle::DmaBuf { fd, .. } => {
                use std::os::fd::AsRawFd;
                desc.opaque = fd.as_raw_fd() as usize as *mut c_void;
                desc.opaque_kind = AV_OPAQUE_DMABUF;
                desc.memory_type = AV_MEM_PLATFORM_SURFACE;
                desc.layout = AV_LAYOUT_PLATFORM_NATIVE;
            }
            FrameHandle::Host(slice) => {
                desc.opaque = slice.as_ptr() as *mut c_void;
            }
            _ => {}
        }

        let abi = self.lib.abi();
        let process_fn = abi.instance_process.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_process 为空".to_string(),
        })?;

        // 隔离阻塞的 FFI 推理调用，防止卡死 Tokio worker 线程
        let code = tokio::task::block_in_place(|| {
            // SAFETY: 调用 C ABI instance_process，传入有效实例与帧描述符
            unsafe { process_fn(self.raw, &desc) }
        });

        if code != AV_OK {
            // SAFETY: 调用方保证 abi 与 self.raw 内存有效
            return Err(unsafe { check_c_status(code, abi, self.raw) });
        }

        // 读取本轮收集的所有回调结果
        let collected_results = self
            .callback_slot
            .lock()
            .map(|mut lock| std::mem::take(&mut *lock))
            .unwrap_or_default();

        // 解析回调产出的检测框 JSON
        let mut detections = Vec::new();
        for json_str in collected_results {
            if let Ok(parsed) = parse_alarm_objects(&json_str) {
                detections.extend(parsed);
            }
        }

        Ok(detections)
    }
}

/// 从算法包输出的 alarm/detection JSON 中解析目标框
fn parse_alarm_objects(json_str: &str) -> Result<Vec<Detection>, InferError> {
    #[derive(Deserialize)]
    struct RawObject {
        #[serde(default)]
        class_id: usize,
        #[serde(default)]
        label: String,
        #[serde(default)]
        confidence: f32,
        #[serde(default)]
        box_coords: Option<[f32; 4]>,
        #[serde(default)]
        bbox: Option<[f32; 4]>,
    }

    let val: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| InferError::JsonParse {
            reason: e.to_string(),
        })?;

    let objects = val.get("objects").unwrap_or(&val);
    let raw_list: Vec<RawObject> = serde_json::from_value(objects.clone()).unwrap_or_default();
    let mut detections = Vec::with_capacity(raw_list.len());

    for item in raw_list {
        let coords = item
            .bbox
            .or(item.box_coords)
            .unwrap_or([0.0, 0.0, 0.0, 0.0]);
        detections.push(Detection {
            class_id: item.class_id,
            label: item.label,
            confidence: item.confidence,
            bbox: BoundingBox::new(coords[0], coords[1], coords[2], coords[3]),
        });
    }

    Ok(detections)
}

/// 全局可用算法包注册表
#[derive(Debug, Default)]
pub struct AlgoRegistry {
    packages: RwLock<HashMap<String, Arc<AlgoPackage>>>,
}

impl AlgoRegistry {
    pub fn new() -> Self {
        Self {
            packages: RwLock::new(HashMap::new()),
        }
    }

    /// 扫描根目录下各平台的算法包目录并热注册
    pub async fn scan_and_register(
        &self,
        base_dir: &Path,
        use_subprocess: bool,
    ) -> Result<usize, InferError> {
        let mut count = 0;
        let cur_platform = crate::sandbox::current_platform_id();
        let target_dir = base_dir.join(cur_platform);

        if !target_dir.is_dir() {
            tracing::warn!(
                target_dir = %target_dir.display(),
                "当前平台专属算法包目录不存在"
            );
            return Ok(0);
        }

        let entries = std::fs::read_dir(&target_dir).map_err(|e| InferError::Execution {
            reason: format!("扫描目录失败: {e}"),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(ALGO_MANIFEST_FILENAME).is_file() {
                match AlgoPackage::load_and_verify(&path, use_subprocess) {
                    Ok(pkg) => {
                        let id = pkg.manifest().algorithm_id.clone();
                        tracing::info!(
                            algorithm_id = %id,
                            version = %pkg.manifest().version,
                            "成功热注册可用算法包"
                        );
                        self.register(Arc::new(pkg)).await;
                        count += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            path = %path.display(),
                            error = %e,
                            "算法包沙箱校验失败，跳过加载"
                        );
                    }
                }
            }
        }

        Ok(count)
    }

    /// 注册一个已验证的算法包
    pub async fn register(&self, pkg: Arc<AlgoPackage>) {
        let mut map = self.packages.write().await;
        map.insert(pkg.manifest().algorithm_id.clone(), pkg);
    }

    /// 获取算法包
    pub async fn get(&self, algorithm_id: &str) -> Option<Arc<AlgoPackage>> {
        let map = self.packages.read().await;
        map.get(algorithm_id).cloned()
    }

    /// 列出所有当前已注册的算法包清单
    pub async fn list(&self) -> Vec<AlgoManifest> {
        let map = self.packages.read().await;
        map.values().map(|p| p.manifest().clone()).collect()
    }
}
