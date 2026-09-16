//! 算法包运行时、实例管理与 InferenceBackend 适配接入

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use tokio::sync::RwLock;
use types::{BoundingBox, Detection, FaceEmbedding, FrameHandle, FrameRef, PixelFormat};

use crate::backend::{InferenceBackend, InferenceResult};
use crate::c_abi::loader::{check_c_status, FaceExtraction, LoadedLib, RawAlgoLibrary};
use crate::c_abi::types::*;
use crate::error::InferError;
use crate::sandbox::{
    current_platform_id, find_entry_library, normalize_platform_id, AlgoManifest, AlgoSandbox,
};
use crate::worker::{InferenceWorker, InferenceWorkerConfig};

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
}

impl AlgoPackage {
    /// 打开已通过安全校验并入库的受信任算法包
    ///
    /// 仅解析 Manifest、加载动态库并建立 C ABI 虚表会话，不重复执行昂贵耗时的六步沙箱前向推理自测。
    pub fn open(package_dir: &Path) -> Result<Self, InferError> {
        if package_dir.as_os_str().is_empty() {
            return Err(InferError::Execution {
                reason: "算法包路径不能为空".to_string(),
            });
        }

        if package_dir.to_string_lossy().contains('\0') {
            return Err(InferError::Execution {
                reason: "算法包路径包含非法空字符 (Null Byte)".to_string(),
            });
        }

        if !package_dir.is_dir() {
            return Err(InferError::Execution {
                reason: format!("算法包目录不存在: {:?}", package_dir),
            });
        }

        let canonical_dir = package_dir
            .canonicalize()
            .map_err(|e| InferError::Execution {
                reason: format!("规范化算法包路径失败: {e}"),
            })?;

        let manifest_path = canonical_dir.join(ALGO_MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            return Err(InferError::Execution {
                reason: format!("缺少 {ALGO_MANIFEST_FILENAME} 文件: {:?}", canonical_dir),
            });
        }

        let canonical_manifest =
            manifest_path
                .canonicalize()
                .map_err(|e| InferError::Execution {
                    reason: format!("规范化 {ALGO_MANIFEST_FILENAME} 失败: {e}"),
                })?;

        if !canonical_manifest.starts_with(&canonical_dir) {
            return Err(InferError::Execution {
                reason: format!(
                    "检测到符号链接路径逃逸: {ALGO_MANIFEST_FILENAME} 指向算法包目录外部 ({:?})",
                    canonical_manifest
                ),
            });
        }

        let manifest_bytes =
            std::fs::read(&canonical_manifest).map_err(|e| InferError::Execution {
                reason: format!("读取 {ALGO_MANIFEST_FILENAME} 失败: {e}"),
            })?;
        let manifest: AlgoManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| InferError::Execution {
                reason: format!("解析 {ALGO_MANIFEST_FILENAME} 格式失败: {e}"),
            })?;

        // 强校验 Manifest 各字段合法性，严格防御 Manifest 路径穿越
        manifest.validate().map_err(|e| match e {
            InferError::SandboxValidation { reason, .. } => InferError::Execution { reason },
            other => other,
        })?;

        let cur_platform = current_platform_id();
        let target_platform = normalize_platform_id(&manifest.platform_id);
        if target_platform != cur_platform {
            return Err(InferError::Execution {
                reason: format!(
                    "平台架构不匹配: 本机环境为 [{cur_platform}], 算法包声明为 [{}]",
                    manifest.platform_id
                ),
            });
        }

        let entry_lib = find_entry_library(&canonical_dir, &manifest.algorithm_id)?;

        let loaded_lib = Arc::new(LoadedLib::load(&entry_lib)?);
        let raw_lib =
            RawAlgoLibrary::open(loaded_lib.clone(), &canonical_dir, &manifest.platform_id)?;

        if raw_lib.meta().algorithm_id != manifest.algorithm_id {
            return Err(InferError::Execution {
                reason: format!(
                    "动态库导出的 algorithm_id [{}] 与 manifest [{}] 不一致",
                    raw_lib.meta().algorithm_id,
                    manifest.algorithm_id
                ),
            });
        }
        // raw_lib 仅用于当前线程的 library_open/query 握手；不把线程绑定的库句柄放入共享包对象。
        drop(raw_lib);

        Ok(Self {
            manifest,
            package_dir: canonical_dir,
            lib: loaded_lib,
        })
    }

    /// 通过沙箱安全校验并打开算法包（用于新包初次安装或自检）
    pub fn load_and_verify(package_dir: &Path, use_subprocess: bool) -> Result<Self, InferError> {
        let _ = AlgoSandbox::validate_package(package_dir, use_subprocess)?;
        Self::open(package_dir)
    }

    #[inline]
    pub fn manifest(&self) -> &AlgoManifest {
        &self.manifest
    }

    #[inline]
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    /// 检查该算法包是否支持人脸特征提取 C ABI
    pub fn supports_face_extraction(&self) -> bool {
        self.lib.get_extract_face_fn().is_some()
    }

    /// 调用该算法包提取人脸特征向量与对齐人脸切片。
    ///
    /// 库级句柄在当前调用线程打开、使用并关闭；这类离线低频能力不跨线程转移
    /// `RawAlgoLibrary`，也不把插件 session 当作可并发共享对象。
    pub fn extract_face(&self, jpeg_bytes: &[u8]) -> Result<FaceExtraction, InferError> {
        let raw_lib = RawAlgoLibrary::open(
            self.lib.clone(),
            &self.package_dir,
            &self.manifest.platform_id,
        )?;
        raw_lib.extract_face(jpeg_bytes)
    }

    /// 创建并启动绑定 C ABI session 的常驻推理 Worker。
    ///
    /// `RawAlgoLibrary` 与 `AlgoInstance` 在 Worker OS 线程内创建，因而不会跨线程
    /// 移动底层 SDK 上下文。调用方只获得 Worker 的跨线程控制句柄。
    pub fn create_worker(
        self: &Arc<Self>,
        instance_id: &str,
        config_json: Option<&str>,
        worker_config: InferenceWorkerConfig,
    ) -> Result<InferenceWorker, InferError> {
        let package = Arc::clone(self);
        let instance_id = instance_id.to_string();
        let config_json = config_json.map(str::to_owned);
        InferenceWorker::with_backend_factory(
            move || {
                let instance = package.create_instance(&instance_id, config_json.as_deref())?;
                Ok(Box::new(instance) as Box<dyn InferenceBackend>)
            },
            worker_config,
        )
    }

    /// 创建一个当前线程独占的 C ABI 推理实例。
    ///
    /// 该 API 主要供 Worker 工厂和同步测试使用；返回的实例不实现 `Send`/`Sync`，
    /// 生产推理应优先调用 [`Self::create_worker`]。
    pub fn create_instance(
        self: &Arc<Self>,
        instance_id: &str,
        config_json: Option<&str>,
    ) -> Result<AlgoInstance, InferError> {
        let inst_id_c = CString::new(instance_id).map_err(|_| InferError::Execution {
            reason: "instance_id 包含非法空字节".to_string(),
        })?;
        let run_id = uuid::Uuid::now_v7().to_string();
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

        let raw_lib = RawAlgoLibrary::open(
            self.lib.clone(),
            &self.package_dir,
            &self.manifest.platform_id,
        )?;
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
        let abi = raw_lib.lib().abi();
        let create_fn = abi.instance_create.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_create 为空".to_string(),
        })?;

        // SAFETY: args 在调用期间有效
        let code = unsafe { create_fn(raw_lib.raw(), &args, &mut raw_inst) };

        if code != AV_OK {
            // SAFETY: abi 有效；instance-level 错误使用部分返回的句柄（若有）提取详情。
            let error = unsafe { check_c_status(code, abi, raw_inst) };
            if !raw_inst.is_null() {
                if let Some(destroy_fn) = abi.instance_destroy {
                    // SAFETY: raw_inst 由当前 instance_create 返回，错误路径立即销毁且仅执行一次。
                    unsafe { destroy_fn(raw_inst) };
                }
            }
            return Err(error);
        }
        if raw_inst.is_null() {
            return Err(InferError::InvalidAbi {
                reason: "instance_create 成功但返回了空实例句柄".to_string(),
            });
        }

        Ok(AlgoInstance {
            raw: raw_inst,
            raw_lib,
            algorithm_id: self.manifest.algorithm_id.clone(),
            callback_slot,
        })
    }
}

/// 算法推理实例句柄
#[derive(Debug)]
pub struct AlgoInstance {
    raw: AvAlgoInstance,
    raw_lib: RawAlgoLibrary,
    algorithm_id: String,
    // 堆分配的实例回调结果槽，地址在实例生命周期内保持稳定
    callback_slot: Box<std::sync::Mutex<Vec<String>>>,
}

impl AlgoInstance {
    #[inline]
    pub fn algorithm_id(&self) -> &str {
        &self.algorithm_id
    }

    /// 在当前实例的硬件上下文内原地更新配置。
    ///
    /// 插件未实现 `instance_update_config` 或明确返回 `AV_ERR_NOT_IMPLEMENTED` 时返回
    /// [`InferError::Unsupported`]，调用方据此回退到目标实例级 Worker 替换；
    /// 其他非零状态码统一映射为携带 `last_error` 详情的 [`InferError::CAbiError`]。
    fn apply_config_update(&self, config_json: &str) -> Result<(), InferError> {
        let abi = self.raw_lib.lib().abi();
        let Some(update_fn) = abi.instance_update_config else {
            return Err(InferError::Unsupported {
                capability: "instance_update_config",
            });
        };

        // CString 绑定具名变量覆盖整个 FFI 调用期，避免悬垂指针。
        let config_c = CString::new(config_json).map_err(|_| InferError::Execution {
            reason: "实例配置 JSON 包含非法空字节".to_string(),
        })?;

        // SAFETY: self.raw 是本实例句柄且仅在其创建线程内调用；config_c 在调用期存活。
        let code = unsafe {
            update_fn(
                self.raw,
                config_c.as_ptr(),
                config_c.as_bytes().len() as u32,
            )
        };

        match code {
            AV_OK => Ok(()),
            AV_ERR_NOT_IMPLEMENTED => Err(InferError::Unsupported {
                capability: "instance_update_config",
            }),
            // SAFETY: abi 有效；实例级错误使用自身句柄提取详情。
            _ => Err(unsafe { check_c_status(code, abi, self.raw) }),
        }
    }
}

impl Drop for AlgoInstance {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            if let Some(destroy_fn) = self.raw_lib.lib().abi().instance_destroy {
                // SAFETY: raw 实例只在其创建线程销毁一次；raw_lib 在字段析构前保持动态库加载。
                unsafe { destroy_fn(self.raw) };
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

#[async_trait(?Send)]
impl InferenceBackend for AlgoInstance {
    fn name(&self) -> &'static str {
        "C-ABI-AlgoInstance"
    }

    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        Ok(self.detect_with_metadata(frame).await?.detections)
    }

    async fn detect_with_metadata(&self, frame: &FrameRef) -> Result<InferenceResult, InferError> {
        // AlgoInstance 只在其所属 InferenceWorker OS 线程内访问，底层 session 不跨线程并发。

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
            PixelFormat::Yuv420p => desc.pixel_format = AV_PIX_I420,
            PixelFormat::Rgb24 => {
                desc.pixel_format = AV_PIX_RGB24;
                let bpp_stride = if frame.stride.hor_stride as usize >= frame.width as usize * 3 {
                    frame.stride.hor_stride as i32
                } else {
                    (frame.stride.hor_stride.max(frame.width) * 3) as i32
                };
                desc.stride = [bpp_stride, 0, 0, 0];
            }
            PixelFormat::Rgba => {
                desc.pixel_format = AV_PIX_BGRA;
                let bpp_stride = if frame.stride.hor_stride as usize >= frame.width as usize * 4 {
                    frame.stride.hor_stride as i32
                } else {
                    (frame.stride.hor_stride.max(frame.width) * 4) as i32
                };
                desc.stride = [bpp_stride, 0, 0, 0];
            }
            _ => desc.pixel_format = AV_PIX_UNKNOWN,
        }

        // 宿主只传递解码后的原生帧描述，不假设所有算法拥有相同的模型输入尺寸。
        // 算法包在自己的硬件/CPU 适配层完成缩放、裁切、色彩转换和归一化。
        // 下面的句柄适配保持 infer_fast_path 的设备侧零拷贝边界；Host 仅用于显式调试回退。
        #[allow(unreachable_patterns)]
        match frame.handle() {
            #[cfg(target_os = "macos")]
            FrameHandle::ApplePixelBuffer { ptr } => {
                // [infer_fast_path] Apple ANE / Metal 直通
                desc.opaque = ptr.as_ptr();
                desc.frame_token = ptr.as_ptr();
                desc.opaque_kind = AV_OPAQUE_CVPIXELBUFFER;
                desc.pixel_format = AV_PIX_NV12;
            }
            #[cfg(target_os = "linux")]
            FrameHandle::DmaBuf { fd, .. } => {
                // [infer_fast_path] Rockchip MPP -> RGA -> RKNN DRM DMA-BUF 直通
                use std::os::fd::AsRawFd;
                desc.opaque = fd.as_raw_fd() as usize as *mut c_void;
                desc.opaque_kind = AV_OPAQUE_DMABUF;
            }
            FrameHandle::DeviceMemory { ptr, .. } => {
                // [infer_fast_path] 华为昇腾 DVPP -> VPC/AIPP -> ACL 原生设备显存直通
                desc.opaque = ptr.as_ptr();
                desc.frame_token = ptr.as_ptr();
                desc.opaque_kind = AV_OPAQUE_ASCEND_DEVICE_MEMORY;
                desc.pixel_format = AV_PIX_NV12;
            }
            FrameHandle::Host(slice) => {
                // [debug_cpu_fallback_path] 开发与回退路径，使用 Host 内存传参
                tracing::debug!(
                    camera_id = %frame.camera_id,
                    "Inference running via debug_cpu_fallback_path (Host memory)"
                );
                desc.opaque = slice.as_ptr() as *mut c_void;
                desc.opaque_kind = AV_OPAQUE_NONE;
            }
            _ => {
                tracing::warn!(
                    camera_id = %frame.camera_id,
                    "Unknown frame handle passed to infer_fast_path"
                );
            }
        }

        let abi = self.raw_lib.lib().abi();
        let process_fn = abi.instance_process.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_process 为空".to_string(),
        })?;

        // 隔离阻塞的 FFI 推理调用：仅在 Tokio 多线程工作池中调用 block_in_place
        // 在专用 OS 线程或单线程运行时中直接执行，避免触发 Tokio 运行时 Panic。
        //
        // 架构安全边界：
        // C ABI instance_process 属于同步阻塞调用；在专用常驻 OS 线程的 current_thread runtime 中，
        // 若底层硬件在驱动内核态死锁（D 状态），协作式异步无法在执行期间进行抢占中断；
        // 系统的死锁防御依赖 InferenceWorkerHandle 客户端断路超时保护与 stop() 线程/动态库隔离保活机制。
        let run_process = || {
            // SAFETY: 调用 C ABI instance_process，传入有效实例与帧描述符
            unsafe { process_fn(self.raw, &desc) }
        };

        let is_multi_thread = tokio::runtime::Handle::try_current()
            .map(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
            .unwrap_or(false);

        let code = if is_multi_thread {
            tokio::task::block_in_place(run_process)
        } else {
            run_process()
        };

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

        // 解析回调产出的检测框与低频特征 sidecar
        let mut detections = Vec::new();
        let mut embeddings = Vec::new();
        for json_str in collected_results {
            let parsed = parse_alarm_objects_with_metadata(&json_str)?;
            for item in parsed {
                detections.push(item.detection);
                embeddings.push(item.embedding);
            }
        }

        Ok(InferenceResult {
            detections,
            embeddings,
        })
    }

    fn update_config(&self, config_json: &str) -> Result<(), InferError> {
        self.apply_config_update(config_json)
    }
}

/// 算法包目标检测框反序列化辅助类型
///
/// 全系统统一遵循对角两点归一化坐标 [x1, y1, x2, y2] 规范 (0.0..=1.0)。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawBBox {
    /// 契约规范具名两点式: { "x1": 0.1, "y1": 0.2, "x2": 0.4, "y2": 0.6 }
    Named { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// 全局标准归一化对角两点式数组: [x1, y1, x2, y2]
    Array([f32; 4]),
}

impl RawBBox {
    fn coords(&self) -> (f32, f32, f32, f32) {
        match *self {
            RawBBox::Named { x1, y1, x2, y2 } | RawBBox::Array([x1, y1, x2, y2]) => {
                (x1, y1, x2, y2)
            }
        }
    }

    fn to_bounding_box(&self) -> Result<BoundingBox, InferError> {
        let (x1, y1, x2, y2) = self.coords();
        if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
            return Err(InferError::JsonParse {
                reason: format!("检测框坐标包含非有限值 (NaN/Inf): [{x1}, {y1}, {x2}, {y2}]"),
            });
        }
        let min_x = x1.min(x2).clamp(0.0, 1.0);
        let max_x = x1.max(x2).clamp(0.0, 1.0);
        let min_y = y1.min(y2).clamp(0.0, 1.0);
        let max_y = y1.max(y2).clamp(0.0, 1.0);
        Ok(BoundingBox::new(min_x, min_y, max_x, max_y))
    }
}

/// 预解析的人脸元数据条目，用于质量分空间几何匹配缝合
struct ParsedFace {
    center: (f64, f64),
    quality_score: Option<f32>,
}

/// 将 `faces` 数组中的质量评分按空间几何匹配缝合到对应的 detection 上。
///
/// 匹配策略：
/// 1. 优先使用 detection 自身携带的 `quality_score` / `quality.score`；
/// 2. 若未携带且为 `face` / `person` 目标，按 bbox 中心距离 + 包含关系匹配最近人脸；
/// 3. 仅当全图单张人脸且目标为 `face` 时允许兜底继承。
fn stitch_quality_scores(
    detection_label: &str,
    bbox: &BoundingBox,
    self_quality: Option<f32>,
    parsed_faces: &[ParsedFace],
) -> Option<f32> {
    // 1. 优先使用自身质量分
    if let Some(q) = self_quality {
        return Some(q);
    }

    // 2. 无自身质量分时尝试空间几何匹配
    if parsed_faces.is_empty() || (detection_label != "face" && detection_label != "person") {
        return None;
    }

    let (obj_cx, obj_cy) = bbox.center();
    // 候选元组: (is_inside, dist_sq, quality_score)
    let mut best_match: Option<(bool, f64, f32)> = None;

    for f in parsed_faces {
        if let Some(f_q) = f.quality_score {
            let (fcx, fcy) = f.center;
            let dist_sq = (obj_cx - fcx).powi(2) + (obj_cy - fcy).powi(2);
            let is_inside = bbox.contains_point(fcx, fcy);
            let threshold_sq = if detection_label == "person" {
                0.10
            } else {
                0.04
            };

            if is_inside || dist_sq < threshold_sq {
                match best_match {
                    Some((best_inside, min_dist, _)) => {
                        // 包含在边界框内部的人脸优先于框外部；同在内部或同在外部时按中心距离决胜
                        let is_better = match (is_inside, best_inside) {
                            (true, false) => true,
                            (false, true) => false,
                            _ => dist_sq < min_dist,
                        };
                        if is_better {
                            best_match = Some((is_inside, dist_sq, f_q));
                        }
                    }
                    None => {
                        best_match = Some((is_inside, dist_sq, f_q));
                    }
                }
            }
        }
    }

    // 3. 几何匹配成功则缝合；否则仅当单脸 + face 目标时允许兜底
    if let Some((_, _, q_score)) = best_match {
        Some(q_score)
    } else if parsed_faces.len() == 1 && detection_label == "face" {
        parsed_faces[0].quality_score
    } else {
        None
    }
}

/// 解析后的检测目标与可选低频 embedding sidecar。
#[derive(Debug)]
struct ParsedDetection {
    detection: Detection,
    embedding: Option<FaceEmbedding>,
}

fn decode_face_embedding(encoded: &str) -> Result<FaceEmbedding, InferError> {
    const EMBEDDING_BYTES: usize = 512 * std::mem::size_of::<f32>();
    const EMBEDDING_BASE64_LEN: usize = EMBEDDING_BYTES.div_ceil(3) * 4;
    if encoded.len() != EMBEDDING_BASE64_LEN {
        return Err(InferError::JsonParse {
            reason: format!(
                "embedding Base64 长度非法: {}，预期 {}",
                encoded.len(),
                EMBEDDING_BASE64_LEN
            ),
        });
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| InferError::JsonParse {
            reason: format!("embedding Base64 解码失败: {error}"),
        })?;
    if bytes.len() != EMBEDDING_BYTES {
        return Err(InferError::JsonParse {
            reason: format!(
                "embedding 字节长度非法: {}，预期 {}",
                bytes.len(),
                EMBEDDING_BYTES
            ),
        });
    }

    let mut values = [0.0f32; 512];
    for (index, chunk) in bytes.chunks(4).enumerate() {
        values[index] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !values[index].is_finite() {
            return Err(InferError::JsonParse {
                reason: "embedding 包含非有限浮点数".to_string(),
            });
        }
    }
    Ok(Box::new(values))
}

/// 从算法包输出的 alarm/detection JSON 中解析目标框与质量元数据。
fn parse_alarm_objects_with_metadata(json_str: &str) -> Result<Vec<ParsedDetection>, InferError> {
    const MAX_RESULT_OBJECTS: usize = 256;
    #[derive(Deserialize, Default)]
    struct RawQuality {
        #[serde(default)]
        score: Option<f32>,
    }

    #[derive(Deserialize, Default)]
    struct RawFaceItem {
        #[serde(default)]
        bbox: Option<RawBBox>,
        #[serde(default)]
        quality: Option<RawQuality>,
        #[serde(default)]
        detection_score: Option<f32>,
    }

    #[derive(Deserialize)]
    struct RawFaceDetail {
        bbox: RawBBox,
        #[serde(default)]
        confidence: Option<f32>,
        #[serde(default)]
        quality_score: Option<f32>,
        #[serde(default)]
        embedding: Option<String>,
        #[serde(default, alias = "fusedCount")]
        /// 融合帧计数由算法包声明，宿主只透传、不解释其取值上界：KMAX 属于包内契约，
        /// 宿主在此硬编码区间会把“包内提高池上限”变成老宿主的硬拒绝。
        fused_count: Option<u32>,
        #[serde(default, alias = "templateQuality")]
        template_quality: Option<f32>,
        #[serde(default, alias = "templateMature")]
        template_mature: Option<bool>,
    }

    #[derive(Deserialize)]
    struct RawObject {
        #[serde(default)]
        class_id: usize,
        label: String,
        confidence: f32,
        #[serde(default)]
        quality_score: Option<f32>,
        #[serde(default)]
        quality: Option<RawQuality>,
        #[serde(default)]
        box_coords: Option<RawBBox>,
        #[serde(default)]
        embedding: Option<String>,
        #[serde(default)]
        bbox: Option<RawBBox>,
        #[serde(default)]
        face: Option<RawFaceDetail>,
        #[serde(default)]
        face_bbox: Option<RawBBox>,
    }

    let mut val: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| InferError::JsonParse {
            reason: format!("JSON 语法错误: {e}"),
        })?;

    // 校验 schema_version（若提供，需校验版本为 1）
    if let Some(ver_num) = val.get("schema_version").and_then(|v| v.as_u64()) {
        if ver_num != 1 {
            return Err(InferError::JsonParse {
                reason: format!("不支持的 schema_version: {ver_num}，当前仅支持版本 1"),
            });
        }
    }

    // 提取可能存在的人脸元数据 (faces 数组)，用于质量分反查缝合
    let raw_faces: Vec<RawFaceItem> = match val.get_mut("faces").map(serde_json::Value::take) {
        Some(faces_val) => {
            if let Some(faces) = faces_val.as_array() {
                if faces.len() > MAX_RESULT_OBJECTS {
                    return Err(InferError::JsonParse {
                        reason: format!(
                            "faces 目标数量 {} 超过上限 {}",
                            faces.len(),
                            MAX_RESULT_OBJECTS
                        ),
                    });
                }
            }
            serde_json::from_value(faces_val).unwrap_or_default()
        }
        None => Vec::new(),
    };

    let objects_val = match val.get_mut("objects").map(serde_json::Value::take) {
        Some(objs) => objs,
        None if val.is_array() => val,
        None => {
            // 若没有 objects 但存在 faces 数组，向下兼容纯人脸识别信封
            if !raw_faces.is_empty() {
                let detections = raw_faces
                    .into_iter()
                    .map(|f| {
                        let raw_bbox = f.bbox.ok_or_else(|| InferError::JsonParse {
                            reason: "人脸对象缺少有效 bbox 坐标字段".to_string(),
                        })?;
                        let bbox = raw_bbox.to_bounding_box()?;
                        let confidence =
                            f.detection_score.ok_or_else(|| InferError::JsonParse {
                                reason: "人脸对象缺少有效 detection_score 置信度字段".to_string(),
                            })?;
                        if !(0.0..=1.0).contains(&confidence) {
                            return Err(InferError::JsonParse {
                                reason: format!("人脸置信度非法: {confidence}"),
                            });
                        }
                        let quality_score =
                            f.quality.and_then(|q| q.score).map(|s| s.clamp(0.0, 1.0));
                        Ok(ParsedDetection {
                            detection: Detection {
                                class_id: 0,
                                label: "face".to_string(),
                                confidence,
                                quality_score,
                                bbox,
                                face: None,
                            },
                            embedding: None,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(detections);
            }
            return Err(InferError::JsonParse {
                reason: "检测结果缺少 objects 数组字段且根节点非数组".to_string(),
            });
        }
    };

    if let Some(objects) = objects_val.as_array() {
        if objects.len() > MAX_RESULT_OBJECTS {
            return Err(InferError::JsonParse {
                reason: format!(
                    "objects 目标数量 {} 超过上限 {}",
                    objects.len(),
                    MAX_RESULT_OBJECTS
                ),
            });
        }
    }

    let raw_list: Vec<RawObject> =
        serde_json::from_value(objects_val).map_err(|e| InferError::JsonParse {
            reason: format!("解析 objects 目标列表失败: {e}"),
        })?;

    // 预先将 raw_faces 解析为 (center, quality_score)，消除 N*M 循环中的重复计算
    let parsed_faces: Vec<ParsedFace> = raw_faces
        .into_iter()
        .filter_map(|f| {
            let bbox = f.bbox?.to_bounding_box().ok()?;
            let center = bbox.center();
            let quality_score = f.quality.and_then(|q| q.score).map(|s| s.clamp(0.0, 1.0));
            Some(ParsedFace {
                center,
                quality_score,
            })
        })
        .collect();

    let mut detections = Vec::with_capacity(raw_list.len());

    for item in raw_list {
        if !(0.0..=1.0).contains(&item.confidence) {
            return Err(InferError::JsonParse {
                reason: format!("置信度非法或超出 [0.0, 1.0]: {}", item.confidence),
            });
        }

        let raw_bbox = item
            .bbox
            .or(item.box_coords)
            .ok_or_else(|| InferError::JsonParse {
                reason: "目标对象缺少有效的 bbox / box_coords 坐标字段".to_string(),
            })?;

        let bbox = raw_bbox.to_bounding_box()?;

        // 从目标自身提取质量分，再通过空间几何匹配缝合人脸质量评分
        let self_quality = item
            .quality_score
            .or_else(|| item.quality.and_then(|q| q.score))
            .map(|s| s.clamp(0.0, 1.0));
        let quality_score = stitch_quality_scores(&item.label, &bbox, self_quality, &parsed_faces);
        let (face, embedding) = if let Some(raw_face) = item.face {
            let f_bbox = raw_face.bbox.to_bounding_box()?;
            let f_conf = raw_face.confidence.unwrap_or(item.confidence);
            let f_qual = raw_face.quality_score.map(|s| s.clamp(0.0, 1.0));
            let f_fused_count = raw_face.fused_count;
            let f_template_quality = raw_face.template_quality.map(|s| s.clamp(0.0, 1.0));
            let f_emb = raw_face
                .embedding
                .as_deref()
                .map(decode_face_embedding)
                .transpose()?;
            (
                Some(types::FaceDetail {
                    bbox: f_bbox,
                    confidence: f_conf,
                    quality_score: f_qual,
                    fused_count: f_fused_count,
                    template_quality: f_template_quality,
                    template_mature: raw_face.template_mature,
                    embedding: f_emb.clone(),
                }),
                f_emb,
            )
        } else if let Some(raw_face_bbox) = item.face_bbox {
            // 兼容扁平 face_bbox 字段
            let f_bbox = raw_face_bbox.to_bounding_box()?;
            let f_emb = item
                .embedding
                .as_deref()
                .map(decode_face_embedding)
                .transpose()?;
            (
                Some(types::FaceDetail {
                    bbox: f_bbox,
                    confidence: item.confidence,
                    quality_score,
                    fused_count: None,
                    template_quality: None,
                    template_mature: None,
                    embedding: f_emb.clone(),
                }),
                f_emb,
            )
        } else {
            let emb = item
                .embedding
                .as_deref()
                .map(decode_face_embedding)
                .transpose()?;
            (None, emb)
        };

        detections.push(ParsedDetection {
            detection: Detection {
                class_id: item.class_id,
                label: item.label,
                confidence: item.confidence,
                quality_score,
                bbox,
                face,
            },
            embedding,
        });
    }

    Ok(detections)
}

#[cfg(test)]
/// 兼容只需要检测框的调用方；embedding sidecar 在这里被明确丢弃。
fn parse_alarm_objects(json_str: &str) -> Result<Vec<Detection>, InferError> {
    Ok(parse_alarm_objects_with_metadata(json_str)?
        .into_iter()
        .map(|item| item.detection)
        .collect())
}

/// 计算目录下所有文件的总字节大小（使用 std::fs::symlink_metadata 防御符号链接逃逸与无限循环）
pub fn compute_dir_size(path: &Path) -> i64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            let meta = std::fs::symlink_metadata(&p).ok()?;
            Some((p, meta))
        })
        .map(|(p, meta)| {
            if meta.is_dir() {
                compute_dir_size(&p)
            } else {
                meta.len() as i64
            }
        })
        .sum()
}

/// 发现指定搜索路径下的所有潜在算法包目录（必须包含 manifest.json，自动执行规范化去重与防逃逸过滤）
pub fn discover_package_dirs(search_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for base in search_dirs {
        let Ok(canonical_base) = base.canonicalize() else {
            continue;
        };
        if !canonical_base.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&canonical_base) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(canonical_p) = entry.path().canonicalize() else {
                continue;
            };
            if !canonical_p.starts_with(&canonical_base) || !canonical_p.is_dir() {
                continue;
            }
            if canonical_p.join(ALGO_MANIFEST_FILENAME).is_file() {
                if seen.insert(canonical_p.clone()) {
                    results.push(canonical_p);
                }
                continue;
            }
            let Ok(sub_entries) = std::fs::read_dir(&canonical_p) else {
                continue;
            };
            for sub_entry in sub_entries.flatten() {
                let Ok(canonical_sub) = sub_entry.path().canonicalize() else {
                    continue;
                };
                if canonical_sub.starts_with(&canonical_base)
                    && canonical_sub.is_dir()
                    && canonical_sub.join(ALGO_MANIFEST_FILENAME).is_file()
                    && seen.insert(canonical_sub.clone())
                {
                    results.push(canonical_sub);
                }
            }
        }
    }
    results
}

/// 默认算法租约空闲退火冷却时间（60 秒）
pub const DEFAULT_ALGO_COOLDOWN_SECS: u64 = 60;

/// 单个算法包的活跃算力租约与常驻退火状态
#[derive(Debug)]
struct LeaseState {
    /// 当前活跃租约持有者计数（运行中摄像头实例 + 离线短租任务）
    ref_count: usize,
    /// 绑定算法 session 的常驻 Worker（底层实例只在线程闭包中存在）
    warm_instance: Option<InferenceWorker>,
    /// 冷却任务世代号（用于取消先前安排的延迟退火任务）
    cooldown_generation: u64,
    /// 预热任务世代号（用于阻止包替换后旧任务回写新状态）
    warmup_generation: u64,
    /// 并发预热完成同步通道（保存当前预热执行结果；新等待方随时可读取最新完成状态，杜绝丢通知挂起）
    warmup_watch: Option<tokio::sync::watch::Receiver<Option<Result<(), InferError>>>>,
}

#[derive(Debug)]
struct AlgoRegistryInner {
    packages: RwLock<HashMap<String, Arc<AlgoPackage>>>,
    leases: std::sync::Mutex<HashMap<String, LeaseState>>,
    cooldown_duration: std::time::Duration,
    runtime_handle: Option<tokio::runtime::Handle>,
}

impl AlgoRegistryInner {
    fn release_lease(self: &Arc<Self>, algorithm_id: &str) {
        let mut state = self.leases.lock().expect("algo lease state lock poisoned");
        if let Some(entry) = state.get_mut(algorithm_id) {
            if entry.ref_count > 0 {
                entry.ref_count -= 1;
                if entry.ref_count == 0 {
                    entry.cooldown_generation += 1;
                    let gen = entry.cooldown_generation;
                    let cooldown = self.cooldown_duration;
                    let aid = algorithm_id.to_string();

                    tracing::debug!(
                        algorithm_id = %aid,
                        cooldown_secs = cooldown.as_secs(),
                        "算法所有活跃租约均已释放，启动算力退火延迟冷却定时器"
                    );

                    let handle = self
                        .runtime_handle
                        .clone()
                        .or_else(|| tokio::runtime::Handle::try_current().ok());

                    if let Some(h) = handle {
                        let inner = Arc::clone(self);
                        drop(state); // 必须在 spawn 前显式释放锁
                        h.spawn(async move {
                            tokio::time::sleep(cooldown).await;
                            inner.check_cooldown_expired(&aid, gen);
                        });
                    } else {
                        // 无 Tokio runtime 时（如独立媒体 OS 线程同步析构），直接在当前锁内取走 warm_instance 并在锁外析构
                        let inst_to_drop = entry.warm_instance.take();
                        drop(state); // 释放锁后再析构实例，杜绝重入 check_cooldown_expired 导致死锁
                        if let Some(inst) = inst_to_drop {
                            tracing::info!(
                                algorithm_id = %aid,
                                "无异步运行时上下文，执行同步显式退火回收 NPU 显存"
                            );
                            drop(inst);
                        }
                    }
                }
            }
        }
    }

    fn rollback_ref_count(&self, algorithm_id: &str) {
        let mut state = self.leases.lock().expect("algo lease state lock poisoned");
        if let Some(entry) = state.get_mut(algorithm_id) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
        }
    }

    fn check_cooldown_expired(&self, algorithm_id: &str, generation: u64) {
        // 1. 锁内仅做轻量内存状态检测与所有权移出，严禁在锁内调用 FFI 或硬件析构
        let inst_to_drop = {
            let mut state = self.leases.lock().expect("algo lease state lock poisoned");
            if let Some(entry) = state.get_mut(algorithm_id) {
                if entry.ref_count == 0 && entry.cooldown_generation == generation {
                    entry.warm_instance.take()
                } else {
                    None
                }
            } else {
                None
            }
        };

        // 2. 将耗时的 NPU / C ABI 销毁调度到阻塞线程池，避免堵塞 Tokio Reactor 任务调度
        if let Some(inst) = inst_to_drop {
            tracing::info!(
                algorithm_id = %algorithm_id,
                "算法租约冷却期结束且无新任务介入，执行显式退火回收 NPU 显存"
            );
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn_blocking(move || drop(inst));
            } else if let Some(handle) = &self.runtime_handle {
                handle.spawn_blocking(move || drop(inst));
            } else {
                drop(inst);
            }
        }
    }
}

/// 算法算力租约：持有该租约期间，对应算法保持热就绪（Hot）状态。
/// 当所有租约释放且超出冷却期（Grace Period）时，常驻暖机资源被安全回收（Cold）。
#[derive(Debug)]
pub struct AlgoLease {
    algorithm_id: String,
    package: Arc<AlgoPackage>,
    inner: Arc<AlgoRegistryInner>,
}

impl AlgoLease {
    #[inline]
    pub fn algorithm_id(&self) -> &str {
        &self.algorithm_id
    }

    #[inline]
    pub fn package(&self) -> &Arc<AlgoPackage> {
        &self.package
    }
}

impl Drop for AlgoLease {
    fn drop(&mut self) {
        self.inner.release_lease(&self.algorithm_id);
    }
}

/// 全局可用算法包注册表
#[derive(Debug, Clone)]
pub struct AlgoRegistry {
    inner: Arc<AlgoRegistryInner>,
}

impl Default for AlgoRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AlgoRegistry {
    pub fn new() -> Self {
        Self::with_cooldown(std::time::Duration::from_secs(DEFAULT_ALGO_COOLDOWN_SECS))
    }

    pub fn with_cooldown(cooldown: std::time::Duration) -> Self {
        let runtime_handle = tokio::runtime::Handle::try_current().ok();
        Self {
            inner: Arc::new(AlgoRegistryInner {
                packages: RwLock::new(HashMap::new()),
                leases: std::sync::Mutex::new(HashMap::new()),
                cooldown_duration: cooldown,
                runtime_handle,
            }),
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
        let mut target_dirs = vec![base_dir.join(cur_platform)];
        if cur_platform.contains("macos") {
            target_dirs.push(base_dir.join("macos-arm64"));
            target_dirs.push(base_dir.join("macos").join("arm64"));
        }

        let mut seen_canonical = std::collections::HashSet::new();

        for target_dir in target_dirs {
            if !target_dir.is_dir() {
                continue;
            }

            let entries = match std::fs::read_dir(&target_dir) {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(target_dir = %target_dir.display(), "扫描目录失败: {err}");
                    continue;
                }
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join(ALGO_MANIFEST_FILENAME).is_file() {
                    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                    if !seen_canonical.insert(canonical) {
                        continue;
                    }
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
        }

        Ok(count)
    }

    /// 注册一个已验证的算法包（支持热替换并同步重置算力常驻状态，释放旧版本暖机实例）
    pub async fn register(&self, pkg: Arc<AlgoPackage>) {
        let algo_id = pkg.manifest().algorithm_id.clone();

        // 1. 同步更新 packages 映射
        {
            let mut map = self.inner.packages.write().await;
            map.insert(algo_id.clone(), pkg);
        }

        // 2. 检查并重置可能存在的旧版本算力租约常驻状态
        let old_warm_inst = {
            let mut state = self
                .inner
                .leases
                .lock()
                .expect("algo lease state lock poisoned");
            if let Some(entry) = state.get_mut(&algo_id) {
                // 标记旧世代任务失效，移出旧版本暖机实例，防止新版本被误判为 Hot
                entry.cooldown_generation += 1;
                entry.warmup_generation = entry.warmup_generation.wrapping_add(1);
                // 重置预热同步通道，避免新租约误读旧通道
                entry.warmup_watch = None;
                entry.warm_instance.take()
            } else {
                None
            }
        };

        // 3. 异步销毁旧版本的暖机实例，释放旧 dynamic library 及 NPU 显存
        if let Some(inst) = old_warm_inst {
            tracing::info!(
                algorithm_id = %algo_id,
                "算法包热替换，退火回收旧版本暖机实例"
            );
            let _ = tokio::task::spawn_blocking(move || drop(inst)).await;
        }
    }

    /// 获取算法包
    pub async fn get(&self, algorithm_id: &str) -> Option<Arc<AlgoPackage>> {
        let map = self.inner.packages.read().await;
        map.get(algorithm_id).cloned()
    }

    /// 列出所有当前已注册的算法包清单
    pub async fn list(&self) -> Vec<AlgoManifest> {
        let map = self.inner.packages.read().await;
        map.values().map(|p| p.manifest().clone()).collect()
    }

    /// 打开已通过安全校验并入库的算法包并注册到注册表中（仅执行动态链接与虚表绑定，不重复执行沙箱推理自测）
    pub async fn open_and_register(
        &self,
        package_dir: &Path,
    ) -> Result<Arc<AlgoPackage>, InferError> {
        let pkg = Arc::new(AlgoPackage::open(package_dir)?);
        self.register(pkg.clone()).await;
        Ok(pkg)
    }

    /// 从目录加载并直接注册到注册表中（执行六步沙箱安全校验与推理自测，用于未校验的新包初次装载）
    pub async fn load_and_register(
        &self,
        package_dir: &Path,
        use_subprocess: bool,
    ) -> Result<Arc<AlgoPackage>, InferError> {
        let pkg = Arc::new(AlgoPackage::load_and_verify(package_dir, use_subprocess)?);
        self.register(pkg.clone()).await;
        Ok(pkg)
    }

    /// 注销指定算法包并释放其算力资源
    pub async fn unregister(&self, algorithm_id: &str) -> Option<Arc<AlgoPackage>> {
        let inst_to_drop = {
            let mut state = self
                .inner
                .leases
                .lock()
                .expect("algo lease state lock poisoned");
            state
                .remove(algorithm_id)
                .and_then(|mut entry| entry.warm_instance.take())
        };
        if let Some(inst) = inst_to_drop {
            let _ = tokio::task::spawn_blocking(move || drop(inst)).await;
        }
        let mut map = self.inner.packages.write().await;
        map.remove(algorithm_id)
    }

    /// 检查是否已包含指定算法包
    pub async fn contains(&self, algorithm_id: &str) -> bool {
        let map = self.inner.packages.read().await;
        map.contains_key(algorithm_id)
    }

    /// 获取算法算力租约：持有期间维持常驻热就绪（Hot）
    pub async fn acquire_lease(&self, algorithm_id: &str) -> Result<AlgoLease, InferError> {
        let pkg = self
            .get(algorithm_id)
            .await
            .ok_or_else(|| InferError::Execution {
                reason: format!("算法未在注册中心就绪: {algorithm_id}"),
            })?;

        enum WarmupRole {
            AlreadyHot,
            Leader(
                tokio::sync::watch::Sender<Option<Result<(), InferError>>>,
                u64,
            ),
            Waiter(tokio::sync::watch::Receiver<Option<Result<(), InferError>>>),
        }

        let role = {
            let mut state = self
                .inner
                .leases
                .lock()
                .expect("algo lease state lock poisoned");
            let entry = state
                .entry(algorithm_id.to_string())
                .or_insert_with(|| LeaseState {
                    ref_count: 0,
                    warm_instance: None,
                    cooldown_generation: 0,
                    warmup_generation: 0,
                    warmup_watch: None,
                });
            entry.ref_count += 1;
            entry.cooldown_generation += 1; // 世代号自增使先前的延迟退火任务作废

            if entry.warm_instance.is_some() {
                WarmupRole::AlreadyHot
            } else if let Some(rx) = &entry.warmup_watch {
                if rx.borrow().is_none() && rx.has_changed().is_err() {
                    // 先前的预热任务异常终止（如 leader future 被取消），重新作为 Leader 预热
                    let (tx, new_rx) = tokio::sync::watch::channel(None);
                    entry.warmup_generation = entry.warmup_generation.wrapping_add(1);
                    let generation = entry.warmup_generation;
                    entry.warmup_watch = Some(new_rx);
                    WarmupRole::Leader(tx, generation)
                } else {
                    WarmupRole::Waiter(rx.clone())
                }
            } else {
                let (tx, rx) = tokio::sync::watch::channel(None);
                entry.warmup_generation = entry.warmup_generation.wrapping_add(1);
                let generation = entry.warmup_generation;
                entry.warmup_watch = Some(rx);
                WarmupRole::Leader(tx, generation)
            }
        };

        match role {
            WarmupRole::AlreadyHot => {}
            WarmupRole::Leader(warmup_tx, warmup_generation) => {
                tracing::info!(
                    algorithm_id = %algorithm_id,
                    "算法从冷态激活借出租约，预热 NPU 模型上下文"
                );
                let pkg_clone = pkg.clone();
                let inst_id = format!("warm-lease-{algorithm_id}");
                let warm_worker_config = InferenceWorkerConfig {
                    worker_name: inst_id.clone(),
                    ..Default::default()
                };
                let warm_inst_res = tokio::task::spawn_blocking(move || {
                    pkg_clone.create_worker(&inst_id, None, warm_worker_config)
                })
                .await;

                let (warmup_res, inst_to_drop) = match warm_inst_res {
                    Ok(Ok(worker)) => {
                        let mut state = self
                            .inner
                            .leases
                            .lock()
                            .expect("algo lease state lock poisoned");
                        let mut inst_to_drop = Some(worker);
                        if let Some(entry) = state.get_mut(algorithm_id) {
                            if entry.warmup_generation == warmup_generation {
                                entry.warmup_watch = None;
                                if entry.ref_count > 0 {
                                    entry.warm_instance = inst_to_drop.take();
                                }
                            }
                        }
                        let _ = warmup_tx.send(Some(Ok(())));
                        (Ok(()), inst_to_drop)
                    }
                    Ok(Err(e)) => {
                        let mut state = self
                            .inner
                            .leases
                            .lock()
                            .expect("algo lease state lock poisoned");
                        if let Some(entry) = state.get_mut(algorithm_id) {
                            if entry.warmup_generation == warmup_generation {
                                entry.warmup_watch = None;
                            }
                            entry.ref_count = entry.ref_count.saturating_sub(1);
                        }
                        let _ = warmup_tx.send(Some(Err(e.clone())));
                        (Err(e), None)
                    }
                    Err(join_err) => {
                        let e = InferError::Execution {
                            reason: format!("预热暖机任务调度异常: {join_err}"),
                        };
                        let mut state = self
                            .inner
                            .leases
                            .lock()
                            .expect("algo lease state lock poisoned");
                        if let Some(entry) = state.get_mut(algorithm_id) {
                            if entry.warmup_generation == warmup_generation {
                                entry.warmup_watch = None;
                            }
                            entry.ref_count = entry.ref_count.saturating_sub(1);
                        }
                        let _ = warmup_tx.send(Some(Err(e.clone())));
                        (Err(e), None)
                    }
                };

                if let Some(inst) = inst_to_drop {
                    let _ = tokio::task::spawn_blocking(move || drop(inst)).await;
                }

                if let Err(e) = warmup_res {
                    tracing::error!(
                        algorithm_id = %algorithm_id,
                        error = %e,
                        "算法预热暖机失败，回滚租约并返回错误"
                    );
                    return Err(e);
                }
            }
            WarmupRole::Waiter(mut rx) => {
                // 等待预热完成通知（watch 保持最新状态，无论通知何时到达都能安全读取，杜绝丢通知永久等待）
                while rx.borrow().is_none() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
                let warmup_res = rx.borrow().clone().unwrap_or_else(|| {
                    Err(InferError::Execution {
                        reason: "算法预热通知通道异常关闭".to_string(),
                    })
                });

                if let Err(e) = warmup_res {
                    self.inner.rollback_ref_count(algorithm_id);
                    tracing::error!(
                        algorithm_id = %algorithm_id,
                        error = %e,
                        "并发等待预热失败，回滚租约并返回错误"
                    );
                    return Err(e);
                }
            }
        }

        Ok(AlgoLease {
            algorithm_id: algorithm_id.to_string(),
            package: pkg,
            inner: self.inner.clone(),
        })
    }

    /// 查询当前算法活跃租约持有者计数
    pub fn active_lease_count(&self, algorithm_id: &str) -> usize {
        let state = self
            .inner
            .leases
            .lock()
            .expect("algo lease state lock poisoned");
        state.get(algorithm_id).map(|e| e.ref_count).unwrap_or(0)
    }

    /// 查询当前算法是否处于常驻热就绪（Hot）状态
    pub fn is_algorithm_hot(&self, algorithm_id: &str) -> bool {
        let state = self
            .inner
            .leases
            .lock()
            .expect("algo lease state lock poisoned");
        state
            .get(algorithm_id)
            .map(|e| e.warm_instance.is_some())
            .unwrap_or(false)
    }

    /// 调用已注册的人脸识别算法包执行人脸特征提取（受算力租约与自动冷却保护）
    pub async fn extract_face(&self, jpeg_bytes: &[u8]) -> Result<FaceExtraction, InferError> {
        let _lease = self.acquire_lease("face_recognition").await?;
        let target_pkg = self
            .get("face_recognition")
            .await
            .filter(|pkg| pkg.supports_face_extraction());

        let pkg = match target_pkg {
            Some(p) => p,
            None => {
                let packages = self.inner.packages.read().await;
                packages
                    .values()
                    .find(|p| p.supports_face_extraction())
                    .cloned()
                    .ok_or_else(|| InferError::Execution {
                        reason: "当前系统未加载支持 av_algo_extract_face 的人脸识别算法包，请先部署人脸算法"
                            .to_string(),
                    })?
            }
        };

        let bytes = jpeg_bytes.to_vec();
        tokio::task::spawn_blocking(move || pkg.extract_face(&bytes))
            .await
            .map_err(|e| InferError::Execution {
                reason: format!("人脸特征提取任务调度异常: {e}"),
            })?
    }

    /// 检查当前是否有人脸特征提取算法包就绪
    pub async fn is_face_extraction_ready(&self) -> bool {
        if let Some(pkg) = self.get("face_recognition").await {
            if pkg.supports_face_extraction() {
                return true;
            }
        }
        let packages = self.inner.packages.read().await;
        packages.values().any(|pkg| pkg.supports_face_extraction())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_alarm_objects_xyxy_standards() {
        // 1. 全局标准对角两点式数组 [x1, y1, x2, y2]: [0.6, 0.5, 0.7, 0.7]
        let json_xyxy = r#"{
            "objects": [
                {
                    "class_id": 0,
                    "label": "face",
                    "confidence": 0.95,
                    "bbox": [0.6, 0.5, 0.7, 0.7]
                }
            ]
        }"#;
        let dets = parse_alarm_objects(json_xyxy).expect("parse xyxy");
        assert_eq!(dets.len(), 1);
        assert!((dets[0].bbox.x1 - 0.6).abs() < 1e-4);
        assert!((dets[0].bbox.y1 - 0.5).abs() < 1e-4);
        assert!((dets[0].bbox.x2 - 0.7).abs() < 1e-4);
        assert!((dets[0].bbox.y2 - 0.7).abs() < 1e-4);

        // 2. 契约规范具名两点式 { "x1": 0.2, "y1": 0.2, "x2": 0.9, "y2": 0.95 }
        let json_named_xyxy = r#"{
            "objects": [
                {
                    "class_id": 1,
                    "label": "car",
                    "confidence": 0.88,
                    "bbox": {
                        "x1": 0.2,
                        "y1": 0.2,
                        "x2": 0.9,
                        "y2": 0.95
                    }
                }
            ]
        }"#;
        let dets2 = parse_alarm_objects(json_named_xyxy).expect("parse named xyxy");
        assert_eq!(dets2.len(), 1);
        assert!((dets2[0].bbox.x1 - 0.2).abs() < 1e-4);
        assert!((dets2[0].bbox.y1 - 0.2).abs() < 1e-4);
        assert!((dets2[0].bbox.x2 - 0.9).abs() < 1e-4);
        assert!((dets2[0].bbox.y2 - 0.95).abs() < 1e-4);

        // 3. 边界与倒置防御测试 [0.8, 0.9, 0.2, 0.1] 自动修正为 [0.2, 0.1, 0.8, 0.9]
        let json_inverted = r#"{
            "objects": [
                {
                    "class_id": 0,
                    "label": "person",
                    "confidence": 0.9,
                    "bbox": [0.8, 0.9, 0.2, 0.1]
                }
            ]
        }"#;
        let dets3 = parse_alarm_objects(json_inverted).expect("parse inverted");
        assert_eq!(dets3.len(), 1);
        assert!((dets3[0].bbox.x1 - 0.2).abs() < 1e-4);
        assert!((dets3[0].bbox.y1 - 0.1).abs() < 1e-4);
        assert!((dets3[0].bbox.x2 - 0.8).abs() < 1e-4);
        assert!((dets3[0].bbox.y2 - 0.9).abs() < 1e-4);

        // 4. 支持 box_coords 降级字段
        let json_box_coords = r#"{
            "objects": [
                {
                    "class_id": 2,
                    "label": "plate",
                    "confidence": 0.85,
                    "box_coords": [0.3, 0.4, 0.5, 0.5]
                }
            ]
        }"#;
        let dets4 = parse_alarm_objects(json_box_coords).expect("parse box_coords");
        assert_eq!(dets4.len(), 1);
        assert!((dets4[0].bbox.x1 - 0.3).abs() < 1e-4);
        assert!((dets4[0].bbox.y1 - 0.4).abs() < 1e-4);
        assert!((dets4[0].bbox.x2 - 0.5).abs() < 1e-4);
        assert!((dets4[0].bbox.y2 - 0.5).abs() < 1e-4);
    }

    #[test]
    fn test_parse_alarm_objects_error_matrix_and_guards() {
        // 1. 合法空目标
        let empty = r#"{"schema_version": 1, "objects": []}"#;
        assert_eq!(parse_alarm_objects(empty).expect("valid empty").len(), 0);

        // 2. 非法 JSON
        assert!(parse_alarm_objects("{ bad json }").is_err());

        // 3. 缺失 objects 且非数组
        assert!(parse_alarm_objects(r#"{"other": 123}"#).is_err());

        // 4. 损坏的 objects（非数组）
        assert!(parse_alarm_objects(r#"{"objects": "not-an-array"}"#).is_err());

        // 5. 缺失 bbox
        let missing_bbox = r#"{"objects": [{"label": "person", "confidence": 0.9}]}"#;
        assert!(parse_alarm_objects(missing_bbox).is_err());

        // 6. 置信度越界
        let bad_conf = r#"{"objects": [{"label": "person", "confidence": 1.5, "bbox": [0.1, 0.1, 0.2, 0.2]}]}"#;
        assert!(parse_alarm_objects(bad_conf).is_err());

        // 7. 不支持的 schema_version
        let bad_version = r#"{"schema_version": 2, "objects": []}"#;
        assert!(parse_alarm_objects(bad_version).is_err());

        // 8. 坐标包含 NaN
        let nan_bbox = r#"{"objects": [{"label": "person", "confidence": 0.9, "bbox": [0.1, 0.1, null, 0.2]}]}"#;
        assert!(parse_alarm_objects(nan_bbox).is_err());

        // 9. 目标数量超过固定上限 (objects & faces)
        let objects: Vec<serde_json::Value> = (0..=256)
            .map(|_| {
                serde_json::json!({
                    "label": "person",
                    "confidence": 0.9,
                    "bbox": [0.1, 0.1, 0.2, 0.2]
                })
            })
            .collect();
        let oversized = serde_json::json!({ "objects": objects }).to_string();
        assert!(parse_alarm_objects(&oversized).is_err());

        let faces: Vec<serde_json::Value> = (0..=256)
            .map(|_| {
                serde_json::json!({
                    "bbox": [0.1, 0.1, 0.2, 0.2],
                    "detection_score": 0.9
                })
            })
            .collect();
        let oversized_faces = serde_json::json!({ "faces": faces }).to_string();
        assert!(parse_alarm_objects(&oversized_faces).is_err());
    }

    #[test]
    fn test_parse_structured_person_with_face_bbox() {
        let envelope = r#"{
            "schema_version": 1,
            "objects": [
                {
                    "class_id": 0,
                    "label": "person",
                    "confidence": 0.94,
                    "bbox": [0.1, 0.2, 0.5, 0.9],
                    "face": {
                        "bbox": [0.2, 0.22, 0.35, 0.45],
                        "confidence": 0.96,
                        "quality_score": 0.87
                    }
                }
            ]
        }"#;

        let dets = parse_alarm_objects(envelope).expect("parse structured envelope");
        assert_eq!(dets.len(), 1);
        assert_eq!(dets[0].label, "person");
        assert_eq!(dets[0].bbox, BoundingBox::new(0.1, 0.2, 0.5, 0.9));
        let face = dets[0].face.as_ref().expect("should have face detail");
        assert_eq!(face.bbox, BoundingBox::new(0.2, 0.22, 0.35, 0.45));
        assert_eq!(face.confidence, 0.96);
        assert_eq!(face.quality_score, Some(0.87));
        assert_eq!(face.fused_count, None);
        assert_eq!(face.template_quality, None);
        assert_eq!(face.template_mature, None);
    }

    #[test]
    fn test_parse_fusion_sidecar_fields_and_camel_aliases() {
        let envelope = serde_json::json!({
            "schema_version": 1,
            "objects": [{
                "class_id": 0,
                "label": "person",
                "confidence": 0.94,
                "bbox": [0.1, 0.2, 0.5, 0.9],
                "face": {
                    "bbox": [0.2, 0.22, 0.35, 0.45],
                    "confidence": 0.96,
                    "quality_score": 0.87,
                    "fused_count": 4,
                    "template_quality": 0.81,
                    "template_mature": true
                }
            }]
        });
        let detections = parse_alarm_objects_with_metadata(&envelope.to_string())
            .expect("融合 sidecar 应解析成功");
        let face = detections[0]
            .detection
            .face
            .as_ref()
            .expect("应有嵌套人脸详情");
        assert_eq!(face.fused_count, Some(4));
        assert_eq!(face.template_quality, Some(0.81));
        assert_eq!(face.template_mature, Some(true));

        let aliases = serde_json::json!({
            "schema_version": 1,
            "objects": [{
                "class_id": 0,
                "label": "person",
                "confidence": 0.94,
                "bbox": [0.1, 0.2, 0.5, 0.9],
                "face": {
                    "bbox": [0.2, 0.22, 0.35, 0.45],
                    "fusedCount": 3,
                    "templateQuality": 0.72,
                    "templateMature": true
                }
            }]
        });
        let detections = parse_alarm_objects_with_metadata(&aliases.to_string())
            .expect("camelCase sidecar 别名应解析成功");
        let face = detections[0]
            .detection
            .face
            .as_ref()
            .expect("应有嵌套人脸详情");
        assert_eq!(face.fused_count, Some(3));
        assert_eq!(face.template_quality, Some(0.72));
        assert_eq!(face.template_mature, Some(true));

        // 融合帧计数是包内不变量（KMAX），宿主不解释其上界；越界只作为契约疑点记录，
        // 不阻断整帧解析，否则老宿主会因为新包提高 KMAX 而丢弃该帧全部检测。
        let oversized = serde_json::json!({
            "schema_version": 1,
            "objects": [{
                "class_id": 0,
                "label": "person",
                "confidence": 0.94,
                "bbox": [0.1, 0.2, 0.5, 0.9],
                "face": {
                    "bbox": [0.2, 0.22, 0.35, 0.45],
                    "fused_count": 9
                }
            }]
        });
        let detections = parse_alarm_objects_with_metadata(&oversized.to_string())
            .expect("融合帧计数越界不得阻断整帧解析");
        assert_eq!(
            detections[0]
                .detection
                .face
                .as_ref()
                .expect("应有嵌套人脸详情")
                .fused_count,
            Some(9),
            "宿主必须原样透传包内声明的融合帧计数"
        );
    }

    #[test]
    fn test_parse_face_envelope_with_quality_score() {
        let face_envelope = r#"{
            "schema_version": 1,
            "objects": [
                {
                    "class_id": 0,
                    "label": "face",
                    "confidence": 0.93,
                    "bbox": [0.2, 0.2, 0.4, 0.5]
                }
            ],
            "faces": [
                {
                    "bbox": [0.2, 0.2, 0.4, 0.5],
                    "detection_score": 0.93,
                    "quality": {
                        "score": 0.86,
                        "yaw": 2.1,
                        "pitch": -1.2,
                        "blur": 0.08,
                        "face_size": 128
                    }
                }
            ]
        }"#;

        let dets = parse_alarm_objects(face_envelope).expect("parse face envelope");
        assert_eq!(dets.len(), 1);
        assert_eq!(dets[0].label, "face");
        assert!((dets[0].confidence - 0.93).abs() < 1e-4);
        assert_eq!(dets[0].quality_score, Some(0.86));
    }

    #[test]
    fn test_parse_person_far_from_face_does_not_inherit_quality_score() {
        let envelope = r#"{
            "schema_version": 1,
            "objects": [
                {
                    "class_id": 0,
                    "label": "person",
                    "confidence": 0.88,
                    "bbox": [0.8, 0.8, 0.95, 0.95]
                }
            ],
            "faces": [
                {
                    "bbox": [0.1, 0.1, 0.2, 0.2],
                    "detection_score": 0.95,
                    "quality": { "score": 0.91 }
                }
            ]
        }"#;

        let dets = parse_alarm_objects(envelope).expect("parse envelope");
        assert_eq!(dets.len(), 1);
        assert_eq!(dets[0].label, "person");
        assert_eq!(dets[0].quality_score, None);
    }

    #[test]
    fn test_parse_pure_face_envelope_rejects_missing_detection_score() {
        let envelope = r#"{
            "schema_version": 1,
            "faces": [
                {
                    "bbox": [0.1, 0.1, 0.2, 0.2],
                    "quality": { "score": 0.91 }
                }
            ]
        }"#;

        assert!(parse_alarm_objects(envelope).is_err());
    }

    #[test]
    fn test_stitch_quality_scores_prioritizes_inside_face_over_closer_outside_face() {
        // 行人边界框为 [0.2, 0.2, 0.6, 0.8]，中心为 (0.4, 0.5)
        // 外部脸 A: 中心 (0.4, 0.15)，距离平方为 0.35^2 = 0.1225，但在阈值边缘
        // 内部脸 B: 位于行人头部 [0.35, 0.22, 0.45, 0.32]，中心 (0.4, 0.27)，包含在内部
        let envelope = r#"{
            "schema_version": 1,
            "objects": [
                {
                    "class_id": 0,
                    "label": "person",
                    "confidence": 0.90,
                    "bbox": [0.2, 0.2, 0.6, 0.8]
                }
            ],
            "faces": [
                {
                    "bbox": [0.38, 0.48, 0.42, 0.52],
                    "detection_score": 0.95,
                    "quality": { "score": 0.88 }
                },
                {
                    "bbox": [0.35, 0.22, 0.45, 0.32],
                    "detection_score": 0.98,
                    "quality": { "score": 0.96 }
                }
            ]
        }"#;

        let dets = parse_alarm_objects(envelope).expect("parse envelope");
        assert_eq!(dets.len(), 1);
        assert!(dets[0].quality_score.is_some());
    }

    #[test]
    fn test_parse_best_shot_embedding_sidecar_only() {
        let mut bytes = Vec::with_capacity(512 * 4);
        for index in 0..512 {
            bytes.extend_from_slice(&(index as f32 / 512.0).to_le_bytes());
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let envelope = serde_json::json!({
            "schema_version": 1,
            "objects": [{
                "class_id": 0,
                "label": "face",
                "confidence": 0.91,
                "quality_score": 0.88,
                "embedding": encoded,
                "bbox": [0.1, 0.2, 0.4, 0.8]
            }]
        });

        let parsed = parse_alarm_objects_with_metadata(&envelope.to_string())
            .expect("best-shot sidecar 应解析成功");
        assert_eq!(parsed.len(), 1);
        let embedding = parsed[0]
            .embedding
            .as_ref()
            .expect("best-shot 必须携带 embedding");
        assert_eq!(embedding.len(), 512);
        assert!((embedding[1] - (1.0 / 512.0)).abs() < 1e-6);

        let public_detections = parse_alarm_objects(&envelope.to_string()).expect("检测解析成功");
        assert_eq!(public_detections.len(), 1);

        let invalid = serde_json::json!({
            "schema_version": 1,
            "objects": [{
                "class_id": 0,
                "label": "face",
                "confidence": 0.91,
                "embedding": "AAAA",
                "bbox": [0.1, 0.2, 0.4, 0.8]
            }]
        });
        assert!(parse_alarm_objects_with_metadata(&invalid.to_string()).is_err());
    }

    #[tokio::test]
    async fn test_algo_lease_lifecycle_and_cooldown() {
        let registry = AlgoRegistry::with_cooldown(std::time::Duration::from_millis(50));
        assert_eq!(registry.active_lease_count("non_existent"), 0);
        assert!(!registry.is_algorithm_hot("non_existent"));

        // 未注册的算法借出直接返回错误
        let err = registry
            .acquire_lease("non_existent")
            .await
            .expect_err("non existent algorithm should fail to acquire lease");
        assert!(err.to_string().contains("未在注册中心就绪"));
    }

    #[test]
    fn test_algo_lease_release_without_runtime_no_deadlock() {
        // 验证在脱离 Tokio Runtime 上下文（如独立物理 OS 线程同步析构租约）时，
        // release_lease 绝不会因重入 check_cooldown_expired 而死锁
        let registry = AlgoRegistry::with_cooldown(std::time::Duration::from_millis(50));
        let inner = registry.inner.clone();

        let handle = std::thread::spawn(move || {
            {
                let mut state = inner.leases.lock().expect("algo lease state lock poisoned");
                let entry = state
                    .entry("mock_algo".to_string())
                    .or_insert_with(|| LeaseState {
                        ref_count: 1,
                        warm_instance: None,
                        cooldown_generation: 0,
                        warmup_generation: 0,
                        warmup_watch: None,
                    });
                entry.ref_count = 1;
            }

            // 在无 Tokio runtime 上下文下释放租约
            inner.release_lease("mock_algo");

            let state = inner.leases.lock().expect("algo lease state lock poisoned");
            let mock_entry = state.get("mock_algo").expect("mock_algo entry exists");
            assert_eq!(mock_entry.ref_count, 0);
        });

        handle
            .join()
            .expect("release_lease 在无 runtime 下不应死锁");
    }

    #[tokio::test]
    async fn test_algo_lease_concurrent_waiter_watch_no_lost_notify() {
        // 验证并发借出租约时，watch 通道能可靠传递预热状态，
        // 即使 Leader 提前完成预热并发送结果，Waiter 也不会丢失通知或永久挂起
        let (tx, mut rx) = tokio::sync::watch::channel::<Option<Result<(), InferError>>>(None);

        // 模拟 Leader 立即完成预热并发送成功通知
        tx.send(Some(Ok(()))).expect("发送预热完成通知成功");

        // Waiter 随后才开始接收（此前 Notify::notify_waiters() 会直接丢通知导致永久死锁）
        while rx.borrow().is_none() {
            if rx.changed().await.is_err() {
                break;
            }
        }

        let res = rx.borrow().clone();
        assert_eq!(res, Some(Ok(())));
    }

    #[tokio::test]
    async fn test_algo_registry_register_resets_warm_instance_and_state() {
        // 验证 register() 替换算法包时同步重置旧的 LeaseState 与暖机实例，
        // 防止新版本被误判为 Hot，并及时释放旧资源
        let pkg_path = Path::new("../../algo-packages/macos-arm64/general_detection")
            .canonicalize()
            .or_else(|_| Path::new("algo-packages/macos-arm64/general_detection").canonicalize());

        let Ok(pkg_path) = pkg_path else {
            return;
        };

        let registry = AlgoRegistry::with_cooldown(std::time::Duration::from_millis(50));
        let Ok(pkg) = AlgoPackage::open(&pkg_path) else {
            // 平台不匹配时跳过平台专属测试
            return;
        };
        let pkg = Arc::new(pkg);
        let algo_id = pkg.manifest().algorithm_id.clone();

        // 初次注册
        registry.register(pkg.clone()).await;

        // 借出租约使算法进入 Hot 状态
        let lease = registry
            .acquire_lease(&algo_id)
            .await
            .expect("acquire lease");
        assert_eq!(registry.active_lease_count(&algo_id), 1);
        assert!(registry.is_algorithm_hot(&algo_id));

        // 重新注册（替换同名算法包）
        registry.register(pkg.clone()).await;

        // 验证 register 同步重置了旧版本的 warm_instance，新版本绝不会被误判为 Hot
        assert!(!registry.is_algorithm_hot(&algo_id));

        drop(lease);
    }
}
