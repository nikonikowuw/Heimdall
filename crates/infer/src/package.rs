//! 算法包运行时、实例管理与 InferenceBackend 适配接入

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};
use types::{BoundingBox, Detection, FaceEmbedding, FrameHandle, FrameRef, PixelFormat};

use crate::backend::{InferenceBackend, InferenceResult};
use crate::c_abi::loader::{check_c_status, FaceExtraction, LoadedLib, RawAlgoLibrary};
use crate::c_abi::types::*;
use crate::error::InferError;
use crate::sandbox::{
    current_platform_id, find_entry_library, normalize_platform_id, AlgoManifest, AlgoSandbox,
};

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
        let raw_lib = Arc::new(RawAlgoLibrary::open(
            loaded_lib.clone(),
            &canonical_dir,
            &manifest.platform_id,
        )?);

        if raw_lib.meta().algorithm_id != manifest.algorithm_id {
            return Err(InferError::Execution {
                reason: format!(
                    "动态库导出的 algorithm_id [{}] 与 manifest [{}] 不一致",
                    raw_lib.meta().algorithm_id,
                    manifest.algorithm_id
                ),
            });
        }

        Ok(Self {
            manifest,
            package_dir: canonical_dir,
            lib: loaded_lib,
            raw_lib,
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

    /// 调用该算法包提取人脸特征向量与对齐人脸切片
    pub fn extract_face(&self, jpeg_bytes: &[u8]) -> Result<FaceExtraction, InferError> {
        self.raw_lib.extract_face(jpeg_bytes)
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
        Ok(self.detect_with_metadata(frame).await?.detections)
    }

    async fn detect_with_metadata(&self, frame: &FrameRef) -> Result<InferenceResult, InferError> {
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

        let abi = self.lib.abi();
        let process_fn = abi.instance_process.ok_or_else(|| InferError::InvalidAbi {
            reason: "instance_process 为空".to_string(),
        })?;

        // 隔离阻塞的 FFI 推理调用：仅在 Tokio 多线程工作池中调用 block_in_place
        // 在专用 OS 线程或单线程运行时中直接执行，避免触发 Tokio 运行时 Panic
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

    let val: serde_json::Value =
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
    let raw_faces: Vec<RawFaceItem> = val
        .get("faces")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
        .unwrap_or_default();

    let objects_val = match val.get("objects") {
        Some(objs) => objs,
        None if val.is_array() => &val,
        None => {
            // 若没有 objects 但存在 faces 数组，向下兼容纯人脸识别信封
            if !raw_faces.is_empty() {
                let mut detections = Vec::with_capacity(raw_faces.len());
                for f in raw_faces {
                    let raw_bbox = f.bbox.ok_or_else(|| InferError::JsonParse {
                        reason: "人脸对象缺少有效 bbox 坐标字段".to_string(),
                    })?;
                    let bbox = raw_bbox.to_bounding_box()?;
                    let confidence = f.detection_score.ok_or_else(|| InferError::JsonParse {
                        reason: "人脸对象缺少有效 detection_score 置信度字段".to_string(),
                    })?;
                    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                        return Err(InferError::JsonParse {
                            reason: format!("人脸置信度非法: {confidence}"),
                        });
                    }
                    let quality_score = f.quality.and_then(|q| q.score).map(|s| s.clamp(0.0, 1.0));
                    detections.push(ParsedDetection {
                        detection: Detection {
                            class_id: 0,
                            label: "face".to_string(),
                            confidence,
                            quality_score,
                            bbox,
                            face: None,
                        },
                        embedding: None,
                    });
                }
                return Ok(detections);
            }
            return Err(InferError::JsonParse {
                reason: "检测结果缺少 objects 数组字段且根节点非数组".to_string(),
            });
        }
    };

    let raw_list: Vec<RawObject> =
        serde_json::from_value(objects_val.clone()).map_err(|e| InferError::JsonParse {
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
        if !item.confidence.is_finite() || !(0.0..=1.0).contains(&item.confidence) {
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
    /// 算力保活实例句柄（持有期间确保底层 NPU Context / Shared Worker 保持常驻预热）
    warm_instance: Option<AlgoInstance>,
    /// 冷却任务世代号（用于取消先前安排的延迟退火任务）
    cooldown_generation: u64,
    /// 是否正在执行后台异步预热
    is_warming_up: bool,
    /// 并发预热完成唤醒通知
    warmup_notify: Arc<tokio::sync::Notify>,
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
                    let inner = Arc::clone(self);
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
                        h.spawn(async move {
                            tokio::time::sleep(cooldown).await;
                            inner.check_cooldown_expired(&aid, gen);
                        });
                    } else {
                        // 脱离任何 Tokio Runtime 上下文（如独立媒体 OS 线程同步析构），执行即时退火防显存泄露
                        inner.check_cooldown_expired(&aid, gen);
                    }
                }
            }
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

    /// 注册一个已验证的算法包
    pub async fn register(&self, pkg: Arc<AlgoPackage>) {
        let mut map = self.inner.packages.write().await;
        map.insert(pkg.manifest().algorithm_id.clone(), pkg);
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

        let (need_warmup, wait_notify) = {
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
                    is_warming_up: false,
                    warmup_notify: Arc::new(tokio::sync::Notify::new()),
                });
            entry.ref_count += 1;
            entry.cooldown_generation += 1; // 世代号自增使先前的延迟退火任务作废

            if entry.warm_instance.is_some() {
                (false, None)
            } else if entry.is_warming_up {
                (false, Some(Arc::clone(&entry.warmup_notify)))
            } else {
                entry.is_warming_up = true;
                (true, None)
            }
        };

        if need_warmup {
            tracing::info!(
                algorithm_id = %algorithm_id,
                "算法从冷态激活借出租约，预热 NPU 模型上下文"
            );
            let pkg_clone = pkg.clone();
            let inst_id = format!("warm-lease-{algorithm_id}");
            let warm_inst_res =
                tokio::task::spawn_blocking(move || pkg_clone.create_instance(&inst_id, None))
                    .await;

            let inst_opt = match warm_inst_res {
                Ok(Ok(inst)) => Some(inst),
                Ok(Err(e)) => {
                    tracing::warn!(
                        algorithm_id = %algorithm_id,
                        error = %e,
                        "预热暖机实例创建产生告警，降级为按需即时推理模式"
                    );
                    None
                }
                Err(e) => {
                    tracing::warn!(
                        algorithm_id = %algorithm_id,
                        error = %e,
                        "预热暖机调度异常，降级为按需即时推理模式"
                    );
                    None
                }
            };

            let inst_to_drop = {
                let mut state = self
                    .inner
                    .leases
                    .lock()
                    .expect("algo lease state lock poisoned");
                if let Some(entry) = state.get_mut(algorithm_id) {
                    entry.is_warming_up = false;
                    entry.warmup_notify.notify_waiters();
                    if let Some(inst) = inst_opt {
                        if entry.ref_count > 0 {
                            entry.warm_instance = Some(inst);
                            None
                        } else {
                            // 预热异步执行期间租约已被全部释放，无需常驻，直接异步退火回收
                            Some(inst)
                        }
                    } else {
                        None
                    }
                } else {
                    inst_opt
                }
            };

            if let Some(inst) = inst_to_drop {
                tokio::task::spawn_blocking(move || drop(inst));
            }
        } else if let Some(notify) = wait_notify {
            notify.notified().await;
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
}
