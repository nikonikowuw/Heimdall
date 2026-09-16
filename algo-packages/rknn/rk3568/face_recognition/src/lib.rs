//! Rockchip RK3568 人脸检测与 EdgeFace 特征提取算法包。
//!
//! 集成 YOLOv8n-face 检测器、ByteTrack 航迹追踪与 EdgeFace-xs 512 维特征提取与时域超球面融合。
//! RKNN context 常驻独立 OS worker，避免在宿主调用线程之间迁移硬件会话。

pub mod align;
pub mod association;
pub mod best_shot;
pub mod bytetrack;
pub mod config;
pub mod detect;
pub mod manifest;
pub mod plugin;
pub mod postprocess;
pub mod quality;
pub mod rknn;
pub mod template_pool;

use std::collections::{HashMap, VecDeque};
use std::ffi::c_int;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

use algo_sdk::c_abi::{AvAlgoLibrary, AvFaceExtractInput, AvFaceExtractOutput};
use algo_sdk::c_abi::{
    AV_ALGO_API_VERSION, AV_ERR_INFERENCE_FAILED, AV_ERR_INTERNAL, AV_ERR_INVALID_ARG, AV_OK,
};
use algo_sdk::cv::{compute_letterbox_layout, CvBuffer, LetterboxLayout};
use algo_sdk::error::AlgoError;
use algo_sdk::export_algo;
use algo_sdk::macros::{validate_abi_header, LibraryContext};
use algo_sdk::plugin::AlgoPlugin;
use image::{ExtendedColorType, ImageReader, Limits, RgbImage};

use manifest::LoadedPackage;
use plugin::FaceRecognizer;
use rknn::{RknnInferenceOutput, RknnModelContract, RknnRuntime, RknnSession};

const WORKER_QUEUE_CAPACITY: usize = 2;
const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DECODED_IMAGE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PREPROCESS_BYTES: usize = 128 * 1024 * 1024;

type DetectionPairResult = Result<(Vec<detect::PersonCandidate>, Vec<detect::RawFace>), AlgoError>;
type RegistrationResult = Result<Vec<detect::RawFace>, AlgoError>;

enum InferenceRequest {
    DetectHost {
        data: Vec<u8>,
        layout: LetterboxLayout,
        min_face_score: f32,
        min_person_score: f32,
        reply: SyncSender<DetectionPairResult>,
    },
    DetectRegistrationHost {
        data: Vec<u8>,
        layout: LetterboxLayout,
        min_face_score: f32,
        reply: SyncSender<RegistrationResult>,
    },
    DetectDma {
        buffer: CvBuffer,
        letterbox: LetterboxLayout,
        min_face_score: f32,
        min_person_score: f32,
        reply: SyncSender<DetectionPairResult>,
    },
    EmbedHost {
        data: Vec<u8>,
        reply: SyncSender<Result<[f32; 512], AlgoError>>,
    },
}

struct WorkerQueue {
    state: Mutex<WorkerQueueState>,
    wake: Condvar,
}

struct WorkerQueueState {
    requests: VecDeque<InferenceRequest>,
    closed: bool,
}

impl WorkerQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(WorkerQueueState {
                requests: VecDeque::with_capacity(WORKER_QUEUE_CAPACITY),
                closed: false,
            }),
            wake: Condvar::new(),
        }
    }

    fn push(&self, request: InferenceRequest) -> Result<(), AlgoError> {
        let mut state = self.state.lock().map_err(|_| AlgoError::Internal {
            reason: "RKNN worker 队列锁已中毒".to_string(),
        })?;
        if state.closed {
            return Err(AlgoError::Internal {
                reason: "RKNN worker 已退出".to_string(),
            });
        }
        if state.requests.len() >= WORKER_QUEUE_CAPACITY {
            if let Some(oldest) = state.requests.pop_front() {
                reject_dropped_request(oldest);
            }
        }
        state.requests.push_back(request);
        drop(state);
        self.wake.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<InferenceRequest> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(request) = state.requests.pop_front() {
                return Some(request);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    fn close(&self) {
        let pending = if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            let pending = std::mem::take(&mut state.requests);
            self.wake.notify_all();
            pending
        } else {
            VecDeque::new()
        };
        for request in pending {
            reject_dropped_request(request);
        }
    }
}

fn reject_dropped_request(request: InferenceRequest) {
    match request {
        InferenceRequest::DetectHost { reply, .. } | InferenceRequest::DetectDma { reply, .. } => {
            let _ = reply.try_send(Err(AlgoError::Timeout));
        }
        InferenceRequest::DetectRegistrationHost { reply, .. } => {
            let _ = reply.try_send(Err(AlgoError::Timeout));
        }
        InferenceRequest::EmbedHost { reply, .. } => {
            let _ = reply.try_send(Err(AlgoError::Timeout));
        }
    }
}

/// 一个固定容量 mailbox 和一个固定 OS 线程承载 detector/embedder 两个 RKNN context。
/// mailbox 满时淘汰最旧请求，避免实时媒体线程被硬件推理反向阻塞。
pub struct InferenceWorker {
    queue: Arc<WorkerQueue>,
    thread_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for InferenceWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InferenceWorker")
            .field("queueCapacity", &WORKER_QUEUE_CAPACITY)
            .finish()
    }
}

impl Drop for InferenceWorker {
    fn drop(&mut self) {
        self.queue.close();
        let handle = self
            .thread_handle
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(handle) = handle {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

impl InferenceWorker {
    fn start(package: &LoadedPackage) -> Result<Arc<Self>, AlgoError> {
        let runtime = RknnRuntime::load(&package.root)?;
        let detector_contract = RknnModelContract {
            input_width: package.detector_width,
            input_height: package.detector_height,
            input_channels: 3,
            output_shapes: manifest::DETECTOR_OUTPUT_SHAPES.to_vec(),
        };
        let embedder_contract = RknnModelContract {
            input_width: package.embedder_width,
            input_height: package.embedder_height,
            input_channels: 3,
            output_shapes: vec![[1, 512, 1, 1]],
        };
        let person_detector_contract = RknnModelContract {
            input_width: package.detector_width,
            input_height: package.detector_height,
            input_channels: 3,
            output_shapes: manifest::PERSON_DETECTOR_OUTPUT_SHAPES.to_vec(),
        };
        let registration_detector_contract = RknnModelContract {
            input_width: package.registration_detector_width,
            input_height: package.registration_detector_height,
            input_channels: 3,
            output_shapes: manifest::DETECTOR_640X640_OUTPUT_SHAPES.to_vec(),
        };

        let detector_path = package.detector_path.clone();
        let registration_detector_path = package.registration_detector_path.clone();
        let embedder_path = package.embedder_path.clone();
        let person_detector_path = package.person_detector_path.clone();
        let queue = Arc::new(WorkerQueue::new());
        let worker_queue = Arc::clone(&queue);
        let (ready_tx, ready_rx) = sync_channel(1);

        let thread_handle = std::thread::Builder::new()
            .name("heimdall-rk3568-face-npu".to_string())
            .spawn(move || {
                let sessions =
                    RknnSession::new(Arc::clone(&runtime), &detector_path, detector_contract)
                        .and_then(|detector| {
                            RknnSession::new(
                                Arc::clone(&runtime),
                                &embedder_path,
                                embedder_contract,
                            )
                            .map(|embedder| (detector, embedder))
                        });
                let Ok((mut detector, mut embedder)) = sessions else {
                    let _ = ready_tx.send(sessions.map(|_| ()));
                    return;
                };

                let mut registration_detector = if registration_detector_path.is_file() {
                    match RknnSession::new(
                        Arc::clone(&runtime),
                        &registration_detector_path,
                        registration_detector_contract,
                    ) {
                        Ok(session) => {
                            tracing::info!(
                                path = ?registration_detector_path,
                                "成功加载 640x640 人脸注册专用检测模型"
                            );
                            Some(session)
                        }
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "可选 640x640 人脸注册检测模型初始化失败，回退使用 640x384 检测器"
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                let mut person_detector = if person_detector_path.is_file() {
                    match RknnSession::new(
                        Arc::clone(&runtime),
                        &person_detector_path,
                        person_detector_contract,
                    ) {
                        Ok(session) => {
                            tracing::info!(path = ?person_detector_path, "成功加载 YOLOv8n 人体检测模型");
                            Some(session)
                        }
                        Err(error) => {
                            tracing::warn!(%error, "可选 YOLOv8n 人体检测模型初始化未就绪，使用纯人脸推导");
                            None
                        }
                    }
                } else {
                    None
                };

                if ready_tx.send(Ok(())).is_err() {
                    return;
                }

                while let Some(request) = worker_queue.pop() {
                    match request {
                        InferenceRequest::DetectHost {
                            data,
                            layout,
                            min_face_score,
                            min_person_score,
                            reply,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let persons = if let Some(ref mut p_session) = person_detector {
                                        let attrs = p_session.output_attrs.clone();
                                        match p_session.infer_with_host_int8(&data, |output| {
                                            decode_person_output(
                                                output,
                                                &attrs,
                                                &layout,
                                                min_person_score,
                                            )
                                        }) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                tracing::warn!(error = %e, "人体检测推理失败，回退到纯人脸推导");
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        Vec::new()
                                    };
                                    let faces =
                                        decode_detector(&mut detector, &data, &layout, min_face_score)?;
                                    Ok((persons, faces))
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                        InferenceRequest::DetectRegistrationHost {
                            data,
                            layout,
                            min_face_score,
                            reply,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    if let Some(ref mut reg_session) = registration_detector {
                                        let attrs = reg_session.output_attrs.clone();
                                        reg_session.infer_with_host_bytes(&data, |output| {
                                            decode_detector_output(
                                                output,
                                                &attrs,
                                                &layout,
                                                min_face_score,
                                            )
                                        })
                                    } else {
                                        let attrs = detector.output_attrs.clone();
                                        detector.infer_with_host_bytes(&data, |output| {
                                            decode_detector_output(
                                                output,
                                                &attrs,
                                                &layout,
                                                min_face_score,
                                            )
                                        })
                                    }
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                        InferenceRequest::DetectDma {
                            buffer,
                            letterbox,
                            min_face_score,
                            min_person_score,
                            reply,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let layout = buffer.as_dma_buf_layout().ok_or_else(|| {
                                        AlgoError::Preprocess {
                                            reason: "worker 收到的 buffer 没有 DMA-BUF 布局".to_string(),
                                        }
                                    })?;
                                    let persons = if let Some(ref mut p_session) = person_detector {
                                        let attrs = p_session.output_attrs.clone();
                                        match p_session.infer_with_dma_buf_int8(&layout, |output| {
                                            decode_person_output(
                                                output,
                                                &attrs,
                                                &letterbox,
                                                min_person_score,
                                            )
                                        }) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                tracing::warn!(error = %e, "人体检测推理失败，回退到纯人脸推导");
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        Vec::new()
                                    };
                                    let attrs = detector.output_attrs.clone();
                                    let faces = detector.infer_with_dma_buf(&layout, |output| {
                                        decode_detector_output(
                                            output,
                                            &attrs,
                                            &letterbox,
                                            min_face_score,
                                        )
                                    })?;
                                    Ok((persons, faces))
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                        InferenceRequest::EmbedHost { data, reply } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    embedder.infer_with_host_bytes(&data, |output| {
                                        let RknnInferenceOutput::Float32(values) = output else {
                                            return Err(AlgoError::Inference {
                                                reason: "EdgeFace 输出类型不是 Float32".to_string(),
                                            });
                                        };
                                        let values =
                                            values.first().ok_or_else(|| AlgoError::Inference {
                                                reason: "EdgeFace 没有返回 embedding 输出"
                                                    .to_string(),
                                            })?;
                                        normalize_embedding(values)
                                    })
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|error| AlgoError::Internal {
                reason: format!("创建 RKNN 专用 worker 线程失败: {error}"),
            })?;

        let mut thread_handle = Some(thread_handle);
        let result = match ready_rx.recv_timeout(WORKER_REPLY_TIMEOUT) {
            Ok(Ok(())) => Ok(Arc::new(Self {
                queue: Arc::clone(&queue),
                thread_handle: Mutex::new(thread_handle.take()),
            })),
            Ok(Err(error)) => Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(AlgoError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(AlgoError::Internal {
                reason: "RKNN worker 初始化时退出".to_string(),
            }),
        };
        if result.is_err() {
            queue.close();
            if let Some(thread_handle) = thread_handle.take() {
                let _ = thread_handle.join();
            }
        }
        result
    }

    pub fn detect_host(
        &self,
        data: Vec<u8>,
        layout: LetterboxLayout,
        min_face_score: f32,
        min_person_score: f32,
    ) -> Result<(Vec<detect::PersonCandidate>, Vec<detect::RawFace>), AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::DetectHost {
            data,
            layout,
            min_face_score,
            min_person_score,
            reply,
        })?;
        receive_response(response)
    }

    pub fn detect_registration_host(
        &self,
        data: Vec<u8>,
        layout: LetterboxLayout,
        min_face_score: f32,
    ) -> Result<Vec<detect::RawFace>, AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::DetectRegistrationHost {
            data,
            layout,
            min_face_score,
            reply,
        })?;
        receive_response(response)
    }

    pub fn detect_dma_buf(
        &self,
        buffer: CvBuffer,
        letterbox: LetterboxLayout,
        min_face_score: f32,
        min_person_score: f32,
    ) -> Result<(Vec<detect::PersonCandidate>, Vec<detect::RawFace>), AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::DetectDma {
            buffer,
            letterbox,
            min_face_score,
            min_person_score,
            reply,
        })?;
        receive_response(response)
    }

    pub fn embed_host(&self, data: Vec<u8>) -> Result<[f32; 512], AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::EmbedHost { data, reply })?;
        receive_response(response)
    }

    fn try_send(&self, request: InferenceRequest) -> Result<(), AlgoError> {
        self.queue.push(request)
    }
}

fn worker_panic_error() -> AlgoError {
    AlgoError::Internal {
        reason: "RKNN worker 请求处理发生 panic".to_string(),
    }
}

fn receive_response<T>(response: Receiver<Result<T, AlgoError>>) -> Result<T, AlgoError> {
    match response.recv_timeout(WORKER_REPLY_TIMEOUT) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(AlgoError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(AlgoError::Internal {
            reason: "RKNN worker 在响应前退出".to_string(),
        }),
    }
}

fn decode_detector(
    detector: &mut RknnSession,
    data: &[u8],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<detect::RawFace>, AlgoError> {
    let attrs = detector.output_attrs.clone();
    detector.infer_with_host_bytes(data, |output| {
        decode_detector_output(output, &attrs, layout, min_score)
    })
}

fn decode_person_output(
    output: &RknnInferenceOutput<'_>,
    attrs: &[rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<detect::PersonCandidate>, AlgoError> {
    let RknnInferenceOutput::Int8(values) = output else {
        return Err(AlgoError::Inference {
            reason: format!(
                "640x384 人体检测输出必须为 INT8 格式: outputs={}",
                attrs.len()
            ),
        });
    };
    detect::decode_yolov8_person_multi_int8(values, attrs, layout, min_score, 0.45)
}
fn decode_detector_output(
    output: &RknnInferenceOutput<'_>,
    attrs: &[rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<detect::RawFace>, AlgoError> {
    let RknnInferenceOutput::Float32(values) = output else {
        return Err(AlgoError::Inference {
            reason: "人脸检测输出类型不是 Float32".to_string(),
        });
    };
    let shapes: Vec<[u32; 4]> = attrs
        .iter()
        .map(|attr| [attr.dims[0], attr.dims[1], attr.dims[2], attr.dims[3]])
        .collect();
    detect::decode_yolov8_face(values, &shapes, layout, min_score, 0.45)
}

static SHARED_MODELS: OnceLock<Mutex<HashMap<PathBuf, Weak<SharedModels>>>> = OnceLock::new();

fn shared_model_registry() -> &'static Mutex<HashMap<PathBuf, Weak<SharedModels>>> {
    SHARED_MODELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 已验证 manifest 身份与模型输入输出契约的共享 worker。
#[derive(Debug)]
pub struct SharedModels {
    pub worker: Arc<InferenceWorker>,
    pub detector_width: u32,
    pub detector_height: u32,
    pub registration_detector_width: u32,
    pub registration_detector_height: u32,
    pub has_registration_detector: bool,
    pub embedder_width: u32,
    pub embedder_height: u32,
}

pub fn shared_models(package_root: &Path) -> Result<Arc<SharedModels>, AlgoError> {
    let key = package_root
        .canonicalize()
        .map_err(|error| AlgoError::ModelLoad {
            reason: format!("算法包根目录无法规范化 ({package_root:?}): {error}"),
        })?;
    {
        let registry = shared_model_registry()
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "RKNN 模型 registry 锁已中毒".to_string(),
            })?;
        if let Some(models) = registry.get(&key).and_then(Weak::upgrade) {
            return Ok(models);
        }
    }

    let package = LoadedPackage::load(&key)?;
    let has_registration_detector = package.registration_detector_path.is_file();
    let worker = InferenceWorker::start(&package)?;
    let models = Arc::new(SharedModels {
        worker,
        detector_width: package.detector_width,
        detector_height: package.detector_height,
        registration_detector_width: package.registration_detector_width,
        registration_detector_height: package.registration_detector_height,
        has_registration_detector,
        embedder_width: package.embedder_width,
        embedder_height: package.embedder_height,
    });

    let mut registry = shared_model_registry()
        .lock()
        .map_err(|_| AlgoError::Internal {
            reason: "RKNN 模型 registry 锁已中毒".to_string(),
        })?;
    if let Some(existing) = registry.get(&key).and_then(Weak::upgrade) {
        Ok(existing)
    } else {
        registry.insert(key, Arc::downgrade(&models));
        Ok(models)
    }
}

pub(crate) fn close_shared_models(_package_root: &Path) {
    if let Some(registry) = SHARED_MODELS.get() {
        if let Ok(mut registry) = registry.lock() {
            // 仅清理已无实例持有的弱引用；不能因一个 library close 破坏其他实例的共享 worker。
            registry.retain(|_, weak| weak.strong_count() != 0);
        }
    }
}

export_algo!(
    FaceRecognizer,
    algo_id: "face_recognition",
    version: "1.0.0",
    algo_type: "face_recognition",
    alarm_type_id: "face_recognize",
    // library_open 保持轻量（仅动态链接与 ABI 虚表握手）；RKNN 会话在首次
    // instance_create / av_algo_extract_face 时经 shared_models 惰性初始化，
    // 避免冷启动与热切换阶段的无条件模型加载与 CMA 争抢。
    library_open_hook: algo_sdk::macros::noop_library_open,
    library_close_hook: crate::close_shared_models
);

/// 计算两个 512D 特征向量之间的余弦相似度。
pub fn cosine_similarity(a: &[f32; 512], b: &[f32; 512]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a_sq = 0.0f32;
    let mut norm_b_sq = 0.0f32;
    for (&left, &right) in a.iter().zip(b.iter()) {
        if !left.is_finite() || !right.is_finite() {
            return 0.0;
        }
        dot += left * right;
        norm_a_sq += left * left;
        norm_b_sq += right * right;
    }
    let denominator = (norm_a_sq * norm_b_sq).sqrt();
    if !denominator.is_finite() || denominator <= f32::EPSILON {
        return 0.0;
    }
    (dot / denominator).clamp(-1.0, 1.0)
}

/// L2 归一化 embedding。输出长度必须严格匹配 manifest 的 512D 契约。
pub fn normalize_embedding(values: &[f32]) -> Result<[f32; 512], AlgoError> {
    if values.len() != 512 || values.iter().any(|value| !value.is_finite()) {
        return Err(AlgoError::Inference {
            reason: format!(
                "EdgeFace 输出维度或数值非法: expected=512, actual={}",
                values.len()
            ),
        });
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err(AlgoError::Inference {
            reason: "EdgeFace embedding L2 范数无效".to_string(),
        });
    }
    let inv_norm = 1.0 / norm;
    Ok(std::array::from_fn(|i| values[i] * inv_norm))
}

/// 使用 manifest 指定的尺寸执行 Host 侧低频 detector letterbox。
pub fn prepare_detector_input_for(
    image: &RgbImage,
    dst_w: u32,
    dst_h: u32,
) -> Result<(Vec<u8>, LetterboxLayout), AlgoError> {
    if image.width() == 0 || image.height() == 0 || dst_w == 0 || dst_h == 0 {
        return Err(AlgoError::Preprocess {
            reason: "detector 输入图像和目标尺寸不能为 0".to_string(),
        });
    }
    let layout = compute_letterbox_layout(image.width(), image.height(), dst_w, dst_h);
    let canvas_len = usize::try_from(dst_w)
        .ok()
        .and_then(|width| {
            usize::try_from(dst_h)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;
    if canvas_len > MAX_PREPROCESS_BYTES {
        return Err(AlgoError::OutOfMemory);
    }
    let resized = image::imageops::resize(
        image,
        layout.scaled_w,
        layout.scaled_h,
        image::imageops::FilterType::Triangle,
    );
    let mut canvas = vec![114u8; canvas_len];
    let dst_width = dst_w as usize;
    let scaled_width = layout.scaled_w as usize;
    let row_len = scaled_width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
    for y in 0..layout.scaled_h as usize {
        let src_offset = y.checked_mul(row_len).ok_or(AlgoError::OutOfMemory)?;
        let src_end = src_offset
            .checked_add(row_len)
            .ok_or(AlgoError::OutOfMemory)?;
        let dst_offset = (y + layout.pad_top as usize)
            .checked_mul(dst_width)
            .and_then(|offset| offset.checked_add(layout.pad_left as usize))
            .and_then(|offset| offset.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;
        let dst_end = dst_offset
            .checked_add(row_len)
            .ok_or(AlgoError::OutOfMemory)?;
        if src_end > resized.as_raw().len() || dst_end > canvas.len() {
            return Err(AlgoError::Preprocess {
                reason: "detector letterbox 行范围越界".to_string(),
            });
        }
        canvas[dst_offset..dst_end].copy_from_slice(&resized.as_raw()[src_offset..src_end]);
    }
    Ok((canvas, layout))
}

/// 保持旧的本地工具 API，使用 manifest 当前的 detector 默认尺寸。
pub fn prepare_detector_input(image: &RgbImage) -> (Vec<u8>, LetterboxLayout) {
    prepare_detector_input_for(image, 640, 384)
        .unwrap_or_else(|_| (Vec::new(), compute_letterbox_layout(1, 1, 640, 384)))
}

fn encode_aligned_jpeg(rgb: &[u8]) -> Result<Vec<u8>, AlgoError> {
    if rgb.len() != 112 * 112 * 3 {
        return Err(AlgoError::Preprocess {
            reason: "对齐人脸尺寸不是 112×112 RGB".to_string(),
        });
    }
    let mut jpeg = Vec::with_capacity(16 * 1024);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90);
    encoder
        .encode(rgb, 112, 112, ExtendedColorType::Rgb8)
        .map_err(|error| AlgoError::Preprocess {
            reason: format!("对齐人脸 JPEG 编码失败: {error}"),
        })?;
    if jpeg.len() > 65_536 {
        return Err(AlgoError::OutOfMemory);
    }
    Ok(jpeg)
}

fn decode_input_image(bytes: &[u8]) -> Result<RgbImage, AlgoError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AlgoError::Preprocess {
            reason: format!("无法识别输入图像格式: {error}"),
        })?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| AlgoError::Preprocess {
        reason: format!("图像解码失败或超过资源上限: {error}"),
    })?;
    let decoded_bytes = u64::from(decoded.width())
        .checked_mul(u64::from(decoded.height()))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;
    if decoded_bytes > MAX_DECODED_IMAGE_BYTES {
        return Err(AlgoError::OutOfMemory);
    }
    Ok(decoded.to_rgb8())
}

struct ExtractCache {
    embedding: [f32; 512],
    aligned_jpeg: Vec<u8>,
}

thread_local! {
    static EXTRACT_CACHE: std::cell::RefCell<ExtractCache> = const {
        std::cell::RefCell::new(ExtractCache {
            embedding: [0.0; 512],
            aligned_jpeg: Vec::new(),
        })
    };
}

fn write_output_error(output: &mut AvFaceExtractOutput, status: c_int) {
    output.status_code = status.unsigned_abs();
    output.embedding = std::ptr::null();
    output.embedding_dim = 0;
    output.aligned_jpeg = std::ptr::null();
    output.aligned_jpeg_len = 0;
}

fn write_output_success(
    output: &mut AvFaceExtractOutput,
    embedding: [f32; 512],
    quality_score: f32,
    detection_score: f32,
    jpeg: Vec<u8>,
) {
    output.status_code = 0;
    output.embedding_dim = 512;
    output.quality_score = quality_score;
    output.detection_score = detection_score;

    EXTRACT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.embedding = embedding;
        cache.aligned_jpeg = jpeg;
        output.embedding = cache.embedding.as_ptr();
        output.aligned_jpeg = cache.aligned_jpeg.as_ptr();
        output.aligned_jpeg_len = cache.aligned_jpeg.len() as u32;
    });
}

unsafe fn extract_face_impl(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    if lib.is_null() || input.is_null() || output.is_null() {
        return AV_ERR_INVALID_ARG;
    }
    // SAFETY: validate_abi_header 只读取 input 指向的 ABI 头；非空和对齐由该函数校验。
    if let Err(error) = unsafe { validate_abi_header(input, "AvFaceExtractInput") } {
        return error.to_c_status();
    }
    // SAFETY: validate_abi_header 只读取 output 指向的 ABI 头；非空和对齐由该函数校验。
    if let Err(error) = unsafe { validate_abi_header(output, "AvFaceExtractOutput") } {
        return error.to_c_status();
    }
    // SAFETY: 两个结构体已通过非空、对齐、size 和 api_version 校验。
    let input_ref = unsafe { &*input };
    // SAFETY: output 指向调用方提供的完整可写 POD 缓冲区。
    unsafe {
        std::ptr::write_bytes(
            output.cast::<u8>(),
            0,
            std::mem::size_of::<AvFaceExtractOutput>(),
        );
    }
    // SAFETY: output 已通过 ABI 校验且 write_bytes 已完成初始化。
    let output_ref = unsafe { &mut *output };
    output_ref.size = std::mem::size_of::<AvFaceExtractOutput>() as u32;
    output_ref.api_version = AV_ALGO_API_VERSION;

    if input_ref.image_bytes.is_null() || input_ref.image_bytes_len == 0 {
        algo_sdk::macros::set_last_error("image_bytes 为空");
        write_output_error(output_ref, AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    let image_len = input_ref.image_bytes_len as usize;
    if image_len > 32 * 1024 * 1024 {
        algo_sdk::macros::set_last_error("压缩图像输入超过 32 MiB 限制");
        write_output_error(output_ref, AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    // SAFETY: C ABI 输入契约保证 image_bytes 指向 image_bytes_len 个只读字节。
    let image_bytes = unsafe { std::slice::from_raw_parts(input_ref.image_bytes, image_len) };
    let image = match decode_input_image(image_bytes) {
        Ok(image) => image,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
        }
    };

    // SAFETY: lib 是 export_algo!::library_open 返回的 LibraryContext 句柄，且本次调用同步完成。
    let library = unsafe { &*(lib as *const LibraryContext) };
    let models = match shared_models(&library.package_root) {
        Ok(models) => models,
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, status);
            return status;
        }
    };

    let (orig_w, orig_h) = (image.width(), image.height());

    // 快捷路径：若输入本身已为 112x112 标准对齐人脸，直接进入 EdgeFace 提取，
    // 杜绝重缩放至 640x384 灰色 letterbox 导致的特征模糊、关键点漂移与失真。
    if orig_w == 112 && orig_h == 112 {
        let aligned = image.as_raw().to_vec();
        let embedding = match models.worker.embed_host(aligned.clone()) {
            Ok(embedding) => embedding,
            Err(error) => {
                let status = error.to_c_status();
                algo_sdk::macros::set_last_error(format!("EMBED_INFERENCE_FAILED: {error}"));
                write_output_error(output_ref, status);
                return status;
            }
        };
        let jpeg = match encode_aligned_jpeg(&aligned) {
            Ok(jpeg) => jpeg,
            Err(error) => {
                algo_sdk::macros::set_last_error(error.to_string());
                write_output_error(output_ref, error.to_c_status());
                return error.to_c_status();
            }
        };
        write_output_success(output_ref, embedding, 1.0, 1.0, jpeg);
        return AV_OK;
    }

    let (det_w, det_h) = if models.has_registration_detector {
        (
            models.registration_detector_width,
            models.registration_detector_height,
        )
    } else {
        (models.detector_width, models.detector_height)
    };

    let (detector_rgb, layout) = match prepare_detector_input_for(&image, det_w, det_h) {
        Ok(input) => input,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
        }
    };
    // 单图提取放宽检测器初筛门槛（0.30），由后续综合质量评估 (quality.accepted) 执行保真
    let min_score = 0.30;
    let raw_faces = match models
        .worker
        .detect_registration_host(detector_rgb, layout, min_score)
    {
        Ok(res) => res,
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, status);
            return status;
        }
    };

    let min_face_size = 30u32;
    let min_quality_score = 0.3f32;
    let thresholds = config::QualityThresholds {
        min_score: min_quality_score,
        ..config::QualityThresholds::default()
    };
    let Some((best_face, quality)) = raw_faces
        .into_iter()
        .filter_map(|face| {
            let face_width = face.width() * orig_w as f32;
            let face_height = face.height() * orig_h as f32;
            let quality = quality::compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face_width.min(face_height),
                &thresholds,
            );
            quality
                .accepted(&thresholds, min_face_size)
                .then_some((face, quality))
        })
        .max_by(|left, right| left.0.score.total_cmp(&right.0.score))
    else {
        algo_sdk::macros::set_last_error(
            "NO_FACE_DETECTED: 输入图像未检出满足置信度与质量阈值的人脸",
        );
        write_output_error(output_ref, AV_ERR_INFERENCE_FAILED);
        return AV_ERR_INFERENCE_FAILED;
    };

    let aligned = if (orig_w < 320 || orig_h < 320)
        && (best_face.width() * orig_w as f32 > 0.6 * orig_w as f32)
    {
        // 紧凑切片保护：输入为贴脸切片时，原图四周缺少额头与下巴余量。
        // 通过边缘镜像填充（Reflect Padding）扩充 25% 边界，防止仿射对齐逆采样出大面积纯黑像素（0, 0, 0）。
        let pad_x = (orig_w as f32 * 0.25).round() as u32;
        let pad_y = (orig_h as f32 * 0.25).round() as u32;
        let padded_w = orig_w + pad_x * 2;
        let padded_h = orig_h + pad_y * 2;
        let mut padded_image = vec![0u8; (padded_w * padded_h * 3) as usize];
        let raw = image.as_raw();
        for y in 0..padded_h {
            let src_y = if y < pad_y {
                pad_y - y
            } else if y >= orig_h + pad_y {
                let diff = y - (orig_h + pad_y) + 1;
                orig_h.saturating_sub(diff)
            } else {
                y - pad_y
            }
            .min(orig_h - 1) as usize;

            for x in 0..padded_w {
                let src_x = if x < pad_x {
                    pad_x - x
                } else if x >= orig_w + pad_x {
                    let diff = x - (orig_w + pad_x) + 1;
                    orig_w.saturating_sub(diff)
                } else {
                    x - pad_x
                }
                .min(orig_w - 1) as usize;

                let src_idx = (src_y * orig_w as usize + src_x) * 3;
                let dst_idx = (y as usize * padded_w as usize + x as usize) * 3;
                padded_image[dst_idx..dst_idx + 3].copy_from_slice(&raw[src_idx..src_idx + 3]);
            }
        }
        let mut padded_landmarks = [[0.0f32; 2]; 5];
        for (dst, src) in padded_landmarks.iter_mut().zip(&best_face.landmarks) {
            dst[0] = src[0] * orig_w as f32 + pad_x as f32;
            dst[1] = src[1] * orig_h as f32 + pad_y as f32;
        }
        match align::align_face(&padded_image, padded_w, padded_h, &padded_landmarks) {
            Ok(aligned) => aligned,
            Err(error) => {
                algo_sdk::macros::set_last_error(error.to_string());
                write_output_error(output_ref, error.to_c_status());
                return error.to_c_status();
            }
        }
    } else {
        match align::align_face(image.as_raw(), orig_w, orig_h, &best_face.landmarks) {
            Ok(aligned) => aligned,
            Err(error) => {
                algo_sdk::macros::set_last_error(error.to_string());
                write_output_error(output_ref, error.to_c_status());
                return error.to_c_status();
            }
        }
    };
    let embedding = match models.worker.embed_host(aligned.clone()) {
        Ok(embedding) => embedding,
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(format!("EMBED_INFERENCE_FAILED: {error}"));
            write_output_error(output_ref, status);
            return status;
        }
    };
    let jpeg = match encode_aligned_jpeg(&aligned) {
        Ok(jpeg) => jpeg,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
        }
    };
    write_output_success(output_ref, embedding, quality.score, best_face.score, jpeg);
    AV_OK
}

/// 独立的人脸特征提取 C ABI 符号。
///
/// # Safety
/// `lib` 必须来自本动态库导出的 `library_open`，`input`/`output` 必须分别指向
/// 满足 ABI 版本和尺寸约束的有效内存。
#[no_mangle]
pub unsafe extern "C" fn av_algo_extract_face(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: extract_face_impl 在进入业务逻辑前验证 ABI，内部不让 panic 穿越 C 栈。
        unsafe { extract_face_impl(lib, input, output) }
    }));
    match result {
        Ok(status) => status,
        Err(_) => AV_ERR_INTERNAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_face_rejects_null_abi_pointers() {
        // SAFETY: null 指针是此测试专门验证的 C ABI 输入，函数应在解引用前拒绝它。
        let status = unsafe {
            av_algo_extract_face(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut())
        };
        assert_eq!(status, AV_ERR_INVALID_ARG);
    }
    #[test]
    fn worker_queue_drops_oldest_request() {
        let queue = WorkerQueue::new();
        let (old_reply, old_response) = sync_channel(1);
        let (middle_reply, _middle_response) = sync_channel(1);
        let (new_reply, _new_response) = sync_channel(1);
        let layout = compute_letterbox_layout(1, 1, 640, 384);

        queue
            .push(InferenceRequest::DetectHost {
                data: vec![1],
                layout,
                min_face_score: 0.5,
                min_person_score: 0.4,
                reply: old_reply,
            })
            .expect("first request should be queued");
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![2],
                layout,
                min_face_score: 0.5,
                min_person_score: 0.4,
                reply: middle_reply,
            })
            .expect("second request should be queued");
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![3],
                layout,
                min_face_score: 0.5,
                min_person_score: 0.4,
                reply: new_reply,
            })
            .expect("newest request should replace oldest request");

        assert!(matches!(
            old_response
                .recv_timeout(Duration::from_millis(10))
                .expect("dropped request should receive a response"),
            Err(AlgoError::Timeout)
        ));
        let first = queue.pop().expect("middle request should remain queued");
        assert!(matches!(
            first,
            InferenceRequest::DetectHost { data, .. } if data == vec![2]
        ));
        let second = queue.pop().expect("newest request should remain queued");
        assert!(matches!(
            second,
            InferenceRequest::DetectHost { data, .. } if data == vec![3]
        ));
    }
    #[test]
    fn test_cosine_similarity_properties() {
        let mut first = [0.0f32; 512];
        let mut second = [0.0f32; 512];
        first[0] = 1.0;
        second[0] = 1.0;
        assert!((cosine_similarity(&first, &second) - 1.0).abs() < 1e-6);
        second[0] = 0.0;
        second[1] = 1.0;
        assert!(cosine_similarity(&first, &second).abs() < 1e-6);
        let mut scaled = [0.0f32; 512];
        scaled[0] = 3.0;
        assert!((cosine_similarity(&scaled, &first) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_embedding_unit_length_and_dimension() {
        let input = vec![1.0f32; 512];
        let normalized = normalize_embedding(&input).expect("512D 归一化应成功");
        let l2_sq: f32 = normalized.iter().map(|value| value * value).sum();
        assert!((l2_sq.sqrt() - 1.0).abs() < 1e-5);
        assert!(normalize_embedding(&[1.0f32; 513]).is_err());
    }
}
