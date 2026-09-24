//! RKNN NPU 专属 OS Worker 线程与邮箱队列管理。
//!
//! 将 RK3588 NPU 上下文常驻于固定专用 OS 线程内，避免跨线程迁移硬件句柄。
//! 采用固定容量的有限 Mailbox 队列，当队列满时执行 Drop-Oldest 丢弃最旧请求，
//! 防止硬件阻塞反压实时流线程。

use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use algo_sdk::cv::{CvBuffer, LetterboxLayout};
use algo_sdk::error::AlgoError;

use crate::detect::{self, PersonCandidate, RawFace};
use crate::manifest::{self, LoadedPackage};
use crate::normalize_embedding;
use crate::rknn::{
    RknnInferenceOutput, RknnModelContract, RknnRuntime, RknnSession, RknnTensorAttr,
};

pub const WORKER_QUEUE_CAPACITY: usize = 6;
pub const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// 邮箱淘汰日志的限流间隔：首条立即告警，其后按该间隔抽样，避免饱和时刷爆日志。
const SHED_LOG_INTERVAL: u64 = 256;

type DetectionPairResult = Result<(Vec<PersonCandidate>, Vec<RawFace>), AlgoError>;
type RegistrationResult = Result<Vec<RawFace>, AlgoError>;

pub(crate) enum InferenceRequest {
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
    shed_total: u64,
    served_total: u64,
}

/// 共享邮箱的累计计数，用于把「跨通道背压」与「算力不足」区分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerQueueStats {
    /// 当前排队深度。
    pub queued: usize,
    /// 因队列饱和被淘汰（调用方收到 `AlgoError::Timeout`）的请求数。
    pub shed_total: u64,
    /// 已完成服务的请求数。
    pub served_total: u64,
}

impl WorkerQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(WorkerQueueState {
                requests: VecDeque::with_capacity(WORKER_QUEUE_CAPACITY),
                closed: false,
                shed_total: 0,
                served_total: 0,
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
                state.shed_total += 1;
                if state.shed_total == 1 || state.shed_total.is_multiple_of(SHED_LOG_INTERVAL) {
                    tracing::warn!(
                        shed_total = state.shed_total,
                        queue_capacity = WORKER_QUEUE_CAPACITY,
                        "RKNN 邮箱饱和，已淘汰最旧请求；被淘汰调用方以超时上报，请按包内聚合吞吐核算路数"
                    );
                }
                reject_dropped_request(oldest);
            }
        }
        state.requests.push_back(request);
        drop(state);
        self.wake.notify_one();
        Ok(())
    }

    fn stats(&self) -> WorkerQueueStats {
        self.state.lock().map_or(
            WorkerQueueStats {
                queued: 0,
                shed_total: 0,
                served_total: 0,
            },
            |s| WorkerQueueStats {
                queued: s.requests.len(),
                shed_total: s.shed_total,
                served_total: s.served_total,
            },
        )
    }

    fn pop(&self) -> Option<InferenceRequest> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(request) = state.requests.pop_front() {
                state.served_total += 1;
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

/// 启动握手向调用方回报的「真实可用会话集」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerSessions {
    /// 640×640 人脸注册检测会话是否真正就绪。
    pub registration_detector: bool,
    /// YOLOv8n 人体检测会话是否真正就绪。
    pub person_detector: bool,
}

/// 一个固定容量 mailbox 和一个固定 OS 线程承载 detector/embedder 两个 RKNN context。
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
    pub fn start(package: &LoadedPackage) -> Result<(Arc<Self>, WorkerSessions), AlgoError> {
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
        let is_yolov6 = package
            .person_detector_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.contains("yolov6"))
            .unwrap_or(false);

        let person_detector_contract = RknnModelContract {
            input_width: package.detector_width,
            input_height: package.detector_height,
            input_channels: 3,
            output_shapes: if is_yolov6 {
                manifest::YOLOV6_PERSON_DETECTOR_OUTPUT_SHAPES.to_vec()
            } else {
                manifest::PERSON_DETECTOR_OUTPUT_SHAPES.to_vec()
            },
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
            .name("heimdall-rk3588-face-npu".to_string())
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
                let (mut detector, mut embedder) = match sessions {
                    Ok(sessions) => sessions,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
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
                    let model_label = if is_yolov6 { "YOLOv6n" } else { "YOLOv8n" };
                    match RknnSession::new(
                        Arc::clone(&runtime),
                        &person_detector_path,
                        person_detector_contract,
                    ) {
                        Ok(session) => {
                            tracing::info!(path = ?person_detector_path, "成功加载 {model_label} 人体检测模型");
                            Some(session)
                        }
                        Err(error) => {
                            tracing::warn!(%error, "可选 {model_label} 人体检测模型初始化未就绪，使用纯人脸推导");
                            None
                        }
                    }
                } else {
                    None
                };

                let live_sessions = WorkerSessions {
                    registration_detector: registration_detector.is_some(),
                    person_detector: person_detector.is_some(),
                };
                if ready_tx.send(Ok(live_sessions)).is_err() {
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
                                    let persons = person_detector
                                        .as_mut()
                                        .and_then(|p_session| {
                                            let attrs = p_session.output_attrs.clone();
                                            p_session.infer_with_host_int8(&data, |output| {
                                                decode_person_output(
                                                    output,
                                                    &attrs,
                                                    &layout,
                                                    min_person_score,
                                                )
                                            }).map_err(|e| {
                                                tracing::warn!(error = %e, "人体检测推理失败，回退到纯人脸推导");
                                                e
                                            }).ok()
                                        })
                                        .unwrap_or_default();
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
                                    let session = registration_detector
                                        .as_mut()
                                        .unwrap_or(&mut detector);
                                    let attrs = session.output_attrs.clone();
                                    session.infer_with_host_bytes(&data, |output| {
                                        decode_detector_output(
                                            output,
                                            &attrs,
                                            &layout,
                                            min_face_score,
                                        )
                                    })
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
                                    let persons = person_detector
                                        .as_mut()
                                        .and_then(|p_session| {
                                            let attrs = p_session.output_attrs.clone();
                                            p_session.infer_with_dma_buf_int8(&layout, |output| {
                                                decode_person_output(
                                                    output,
                                                    &attrs,
                                                    &letterbox,
                                                    min_person_score,
                                                )
                                            }).map_err(|e| {
                                                tracing::warn!(error = %e, "人体检测推理失败，回退到纯人脸推导");
                                                e
                                            }).ok()
                                        })
                                        .unwrap_or_default();
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
            Ok(Ok(sessions)) => Ok((
                Arc::new(Self {
                    queue: Arc::clone(&queue),
                    thread_handle: Mutex::new(thread_handle.take()),
                }),
                sessions,
            )),
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
    ) -> Result<(Vec<PersonCandidate>, Vec<RawFace>), AlgoError> {
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
    ) -> Result<Vec<RawFace>, AlgoError> {
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
    ) -> Result<(Vec<PersonCandidate>, Vec<RawFace>), AlgoError> {
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

    /// 共享邮箱计数：`shed_total > 0` 表示有通道正在被别的通道挤掉帧。
    pub fn stats(&self) -> WorkerQueueStats {
        self.queue.stats()
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
) -> Result<Vec<RawFace>, AlgoError> {
    let attrs = detector.output_attrs.clone();
    detector.infer_with_host_bytes(data, |output| {
        decode_detector_output(output, &attrs, layout, min_score)
    })
}

fn decode_person_output(
    output: &RknnInferenceOutput<'_>,
    attrs: &[RknnTensorAttr],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<PersonCandidate>, AlgoError> {
    let RknnInferenceOutput::Int8(values) = output else {
        return Err(AlgoError::Inference {
            reason: format!(
                "640x384 人体检测输出必须为 INT8 格式: outputs={}",
                attrs.len()
            ),
        });
    };
    detect::decode_person_multi_int8(values, attrs, layout, min_score, 0.45)
}

fn decode_detector_output(
    output: &RknnInferenceOutput<'_>,
    attrs: &[RknnTensorAttr],
    layout: &LetterboxLayout,
    min_score: f32,
) -> Result<Vec<RawFace>, AlgoError> {
    let RknnInferenceOutput::Float32(values) = output else {
        return Err(AlgoError::Inference {
            reason: "人脸检测输出类型不是 Float32".to_string(),
        });
    };
    let shapes: Vec<[u32; 4]> = attrs
        .iter()
        .map(|attr| [attr.dims[0], attr.dims[1], attr.dims[2], attr.dims[3]])
        .collect();
    detect::decode_scrfd_face(values, &shapes, layout, min_score, 0.45)
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::compute_letterbox_layout;

    #[test]
    fn worker_queue_drops_oldest_request() {
        let queue = WorkerQueue::new();
        let (old_reply, old_response) = sync_channel(1);
        let layout = compute_letterbox_layout(1, 1, 640, 384);

        // 填满容量为 WORKER_QUEUE_CAPACITY (6) 的队列
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![1],
                layout,
                min_face_score: 0.5,
                min_person_score: 0.4,
                reply: old_reply,
            })
            .expect("first request should be queued");

        for i in 2..=WORKER_QUEUE_CAPACITY {
            let (reply, _rx) = sync_channel(1);
            queue
                .push(InferenceRequest::DetectHost {
                    data: vec![i as u8],
                    layout,
                    min_face_score: 0.5,
                    min_person_score: 0.4,
                    reply,
                })
                .expect("request within capacity should be queued");
        }

        // 第 7 个请求压入，此时触发 Drop-Oldest 弹出并拒绝第 1 个请求
        let (new_reply, _new_rx) = sync_channel(1);
        queue
            .push(InferenceRequest::DetectHost {
                data: vec![(WORKER_QUEUE_CAPACITY + 1) as u8],
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
        let stats = queue.stats();
        assert_eq!(stats.shed_total, 1, "淘汰计数必须累加");
        assert_eq!(stats.queued, WORKER_QUEUE_CAPACITY, "队列深度保持在上限内");
        let first = queue.pop().expect("second request should remain queued");
        assert!(matches!(
            first,
            InferenceRequest::DetectHost { data, .. } if data == vec![2]
        ));
        assert_eq!(
            queue.stats().served_total,
            1,
            "出队请求应计入已服务计数，供 shed/(shed+served) 比值使用"
        );
    }
}
