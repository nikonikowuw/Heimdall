//! Rockchip RK3576 人脸检测与 EdgeFace 特征提取算法包。
//!
//! 常驻视频路径只做检测和质量门控；embedding 通过低频抓拍 API 或显式请求执行。
//! RKNN context 常驻独立 OS worker，避免在宿主调用线程之间迁移硬件会话。

pub mod align;
pub mod config;
pub mod detect;
pub mod manifest;
pub mod plugin;
pub mod postprocess;
pub mod quality;
pub mod rknn;

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

enum InferenceRequest {
    DetectHost {
        data: Vec<u8>,
        layout: LetterboxLayout,
        min_score: f32,
        reply: SyncSender<Result<Vec<detect::RawFace>, AlgoError>>,
    },
    DetectDma {
        buffer: CvBuffer,
        letterbox: LetterboxLayout,
        min_score: f32,
        reply: SyncSender<Result<Vec<detect::RawFace>, AlgoError>>,
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
            input_width: package.manifest.models.detector.input.width,
            input_height: package.manifest.models.detector.input.height,
            input_channels: package.manifest.models.detector.input.channels,
            output_shapes: package
                .manifest
                .models
                .detector
                .outputs
                .iter()
                .map(|output| output.shape)
                .collect(),
        };
        let embedder_contract = RknnModelContract {
            input_width: package.manifest.models.embedder.input.width,
            input_height: package.manifest.models.embedder.input.height,
            input_channels: package.manifest.models.embedder.input.channels,
            output_shapes: package
                .manifest
                .models
                .embedder
                .outputs
                .iter()
                .map(|output| output.shape)
                .collect(),
        };

        let detector_path = package.detector_path.clone();
        let embedder_path = package.embedder_path.clone();
        let queue = Arc::new(WorkerQueue::new());
        let worker_queue = Arc::clone(&queue);
        let (ready_tx, ready_rx) = sync_channel(1);

        let thread_handle = std::thread::Builder::new()
            .name("heimdall-rk3576-face-npu".to_string())
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
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }

                while let Some(request) = worker_queue.pop() {
                    match request {
                        InferenceRequest::DetectHost {
                            data,
                            layout,
                            min_score,
                            reply,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    decode_detector(&mut detector, &data, &layout, min_score)
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                        InferenceRequest::DetectDma {
                            buffer,
                            letterbox,
                            min_score,
                            reply,
                        } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    buffer
                                        .as_dma_buf_layout()
                                        .ok_or_else(|| AlgoError::Preprocess {
                                            reason: "worker 收到的 buffer 没有 DMA-BUF 布局"
                                                .to_string(),
                                        })
                                        .and_then(|layout| {
                                            let attrs = detector.output_attrs.clone();
                                            detector.infer_with_dma_buf(&layout, |output| {
                                                decode_detector_output(
                                                    output, &attrs, &letterbox, min_score,
                                                )
                                            })
                                        })
                                }))
                                .unwrap_or_else(|_| Err(worker_panic_error()));
                            let _ = reply.send(result);
                        }
                        InferenceRequest::EmbedHost { data, reply } => {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    embedder.infer_with_host_bytes(&data, |output| {
                                        let RknnInferenceOutput::Float32(values) = output;
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
        min_score: f32,
    ) -> Result<Vec<detect::RawFace>, AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::DetectHost {
            data,
            layout,
            min_score,
            reply,
        })?;
        receive_response(response)
    }

    pub fn detect_dma_buf(
        &self,
        buffer: CvBuffer,
        letterbox: LetterboxLayout,
        min_score: f32,
    ) -> Result<Vec<detect::RawFace>, AlgoError> {
        let (reply, response) = sync_channel(1);
        self.try_send(InferenceRequest::DetectDma {
            buffer,
            letterbox,
            min_score,
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

fn decode_detector_output(
    output: &RknnInferenceOutput<'_>,
    attrs: &[rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<detect::RawFace>, AlgoError> {
    let RknnInferenceOutput::Float32(values) = output;
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

/// 已验证 manifest、模型 hash 和输入输出契约的共享 worker。
#[derive(Debug)]
pub struct SharedModels {
    pub worker: Arc<InferenceWorker>,
    pub detector_width: u32,
    pub detector_height: u32,
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
    let worker = InferenceWorker::start(&package)?;
    let models = Arc::new(SharedModels {
        worker,
        detector_width: package.manifest.models.detector.input.width,
        detector_height: package.manifest.models.detector.input.height,
        embedder_width: package.manifest.models.embedder.input.width,
        embedder_height: package.manifest.models.embedder.input.height,
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

pub(crate) fn open_shared_models(package_root: &Path) -> Result<(), AlgoError> {
    shared_models(package_root).map(|_| ())
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
    library_open_hook: crate::open_shared_models,
    library_close_hook: crate::close_shared_models
);

/// 计算两个 512D 特征向量之间的余弦相似度。
pub fn cosine_similarity(a: &[f32; 512], b: &[f32; 512]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a_sq = 0.0f32;
    let mut norm_b_sq = 0.0f32;
    for (left, right) in a.iter().zip(b.iter()) {
        if !left.is_finite() || !right.is_finite() {
            return 0.0;
        }
        dot += left * right;
        norm_a_sq += left * left;
        norm_b_sq += right * right;
    }
    let denominator = norm_a_sq.sqrt() * norm_b_sq.sqrt();
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
    let mut embedding = [0.0f32; 512];
    for (target, source) in embedding.iter_mut().zip(values) {
        *target = *source / norm;
    }
    Ok(embedding)
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
    for y in 0..layout.scaled_h as usize {
        let row_len = scaled_width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
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

    let (detector_rgb, layout) =
        match prepare_detector_input_for(&image, models.detector_width, models.detector_height) {
            Ok(input) => input,
            Err(error) => {
                algo_sdk::macros::set_last_error(error.to_string());
                write_output_error(output_ref, error.to_c_status());
                return error.to_c_status();
            }
        };
    let (orig_w, orig_h) = (image.width(), image.height());
    let min_score = 0.5;
    let raw_faces = match models.worker.detect_host(detector_rgb, layout, min_score) {
        Ok(faces) => faces,
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

    let aligned = match align::align_face(image.as_raw(), orig_w, orig_h, &best_face.landmarks) {
        Ok(aligned) => aligned,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
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
                min_score: 0.5,
                reply: old_reply,
            })
            .expect("first request should be queued");
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![2],
                layout,
                min_score: 0.5,
                reply: middle_reply,
            })
            .expect("second request should be queued");
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![3],
                layout,
                min_score: 0.5,
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
        assert!(matches!(first, InferenceRequest::DetectHost { data, .. } if data == vec![2]));
        let second = queue.pop().expect("newest request should remain queued");
        assert!(matches!(second, InferenceRequest::DetectHost { data, .. } if data == vec![3]));
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
