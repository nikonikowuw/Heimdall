//! 专用常驻推理线程与句柄抽象 (Inference Worker & Drop-Oldest Channel)
//!
//! 核心设计保证：
//! 1. **线程常驻 (Context Pinning)**：为推理后端分配专属独立 OS 线程，杜绝 Tokio 协程漂移导致的 NPU 上下文失效；
//! 2. **丢旧帧背压防护 (Drop-Oldest Backpressure Protection)**：单槽容量限制，超载时丢弃旧帧，绝不反压上游视频解码；
//! 3. **超时与异常隔离**：单次推理超时硬核中断，结合 `catch_unwind` 隔离底层崩溃；
//! 4. **停机超时隔离 (Graceful Shutdown & Join Timeout)**：退出带超时保护，杜绝底层硬件驱动挂起拖死守护进程。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::{oneshot, watch, Notify};
use types::{Detection, FrameRef};

use crate::backend::InferenceBackend;
use crate::error::InferError;

/// 工作线程优雅关停超时上限（2500ms，需大于单帧推理超时阈值，超时后强制隔离放弃，防止拖死守护进程退出）
pub const DEFAULT_INFER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(2500);

/// 单次推理任务请求
struct InferenceJob {
    frame: FrameRef,
    reply: oneshot::Sender<Result<Vec<Detection>, InferError>>,
}

/// 共享单槽任务队列（Drop-Oldest 丢旧帧机制核心）
struct SharedSlot {
    job: Mutex<Option<InferenceJob>>,
    notify: Notify,
    dropped_count: AtomicU64,
    is_busy: AtomicBool,
}

impl SharedSlot {
    fn new() -> Self {
        Self {
            job: Mutex::new(None),
            notify: Notify::new(),
            dropped_count: AtomicU64::new(0),
            is_busy: AtomicBool::new(false),
        }
    }
}

/// 推理工作线程配置参数
#[derive(Debug, Clone)]
pub struct InferenceWorkerConfig {
    /// 线程名称
    pub worker_name: String,
    /// 单帧推理硬核超时时间（毫秒）
    pub timeout_ms: u64,
}

impl Default for InferenceWorkerConfig {
    fn default() -> Self {
        Self {
            worker_name: "infer-worker".to_string(),
            timeout_ms: 2000,
        }
    }
}

/// 跨线程/跨协程持有的推理客户端句柄
#[derive(Clone)]
pub struct InferenceWorkerHandle {
    slot: Arc<SharedSlot>,
    is_alive: Arc<AtomicBool>,
}

impl std::fmt::Debug for InferenceWorkerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferenceWorkerHandle")
            .field("is_alive", &self.is_alive())
            .field("is_busy", &self.is_busy())
            .field("dropped_count", &self.dropped_frames_count())
            .finish()
    }
}

impl InferenceWorkerHandle {
    /// 提交一帧执行推理检测（具备 Drop-Oldest 丢旧帧保护）
    ///
    /// 若推理线程正在忙碌，新帧将替换掉上一帧在队列中等待的旧帧，
    /// 旧帧调用方将收到超载丢弃错误，确保始终只有最新帧进入计算。
    pub async fn submit(&self, frame: FrameRef) -> Result<Vec<Detection>, InferError> {
        if !self.is_alive.load(Ordering::Relaxed) {
            return Err(InferError::Execution {
                reason: "推理工作线程已退出或停止运行".to_string(),
            });
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let new_job = InferenceJob {
            frame,
            reply: reply_tx,
        };

        {
            let mut guard = self.slot.job.lock().map_err(|_| InferError::Execution {
                reason: "推理共享队列互斥锁中毒".to_string(),
            })?;

            // 若已有待处理任务在等待槽位，丢弃旧任务并通知调用方
            if let Some(stale_job) = guard.replace(new_job) {
                self.slot.dropped_count.fetch_add(1, Ordering::Relaxed);
                let _ = stale_job.reply.send(Err(InferError::Execution {
                    reason: "推理负载过高，跳过过时帧 (drop-oldest)".to_string(),
                }));
            }
        }

        // 通知常驻线程有新任务就绪
        self.slot.notify.notify_one();

        // 等待推理结果返回
        reply_rx.await.map_err(|_| InferError::Execution {
            reason: "推理工作线程通道意外关闭".to_string(),
        })?
    }

    /// 获取因负载过高累计被丢弃的旧帧计数
    #[inline]
    pub fn dropped_frames_count(&self) -> u64 {
        self.slot.dropped_count.load(Ordering::Relaxed)
    }

    /// 查询推理线程是否处于忙碌计算状态
    #[inline]
    pub fn is_busy(&self) -> bool {
        self.slot.is_busy.load(Ordering::Relaxed)
    }

    /// 查询工作线程健康存活状态
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }
}

/// 专用常驻推理工作线程 (Inference Worker)
///
/// 拥有专用 OS 线程与单线程 Tokio 事件循环，保证模型上下文生命周期与底层硬件绑定。
pub struct InferenceWorker {
    worker_name: String,
    handle: InferenceWorkerHandle,
    shutdown_tx: Option<watch::Sender<bool>>,
    // Mutex 用于向外层结构（如 PipelineManager）提供 Sync 特征（std::sync::mpsc::Receiver 为 !Sync）
    exit_rx: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for InferenceWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferenceWorker")
            .field("worker_name", &self.worker_name)
            .field("is_alive", &self.handle.is_alive())
            .field("dropped_count", &self.handle.dropped_frames_count())
            .finish()
    }
}

impl InferenceWorker {
    /// 基于指定推理后端创建并启动专用常驻推理线程
    pub fn new(backend: Arc<dyn InferenceBackend>) -> Self {
        let name = format!("infer-worker-{}", backend.name());
        let config = InferenceWorkerConfig {
            worker_name: name,
            ..Default::default()
        };
        Self::with_config(backend, config)
    }

    /// 带自定义配置创建并启动专用常驻推理线程
    pub fn with_config(backend: Arc<dyn InferenceBackend>, config: InferenceWorkerConfig) -> Self {
        let slot = Arc::new(SharedSlot::new());
        let is_alive = Arc::new(AtomicBool::new(true));
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();

        let handle = InferenceWorkerHandle {
            slot: slot.clone(),
            is_alive: is_alive.clone(),
        };

        let worker_name = config.worker_name.clone();
        let thread_worker_name = worker_name.clone();
        let timeout_duration = Duration::from_millis(config.timeout_ms);

        let thread_slot = slot.clone();
        let thread_is_alive = is_alive.clone();

        let thread_handle = std::thread::Builder::new()
            .name(worker_name.clone())
            .spawn(move || {
                tracing::info!(worker_name = %thread_worker_name, "专用常驻推理 OS 线程已启动并绑定");

                // 创建单线程独立运行时，与主进程 Tokio 工作池物理隔离
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(err) => {
                        tracing::error!(error = %err, "创建专用推理单线程运行时失败");
                        thread_is_alive.store(false, Ordering::Relaxed);
                        let _ = exit_tx.send(());
                        return;
                    }
                };

                let loop_worker_name = thread_worker_name.clone();
                let inner_slot = thread_slot.clone();
                rt.block_on(async move {
                    loop {
                        tokio::select! {
                            biased;

                            // 优先监听退出信号
                            changed = shutdown_rx.changed() => {
                                if changed.is_err() || *shutdown_rx.borrow() {
                                    tracing::info!(worker_name = %loop_worker_name, "收到关停信号，常驻推理线程准备退出");
                                    break;
                                }
                            }

                            // 监听待推理帧到达
                            _ = inner_slot.notify.notified() => {
                                let maybe_job = inner_slot.job.lock().ok().and_then(|mut g| g.take());

                                if let Some(job) = maybe_job {
                                    inner_slot.is_busy.store(true, Ordering::Relaxed);

                                    let result = execute_inference(
                                        backend.as_ref(),
                                        &job.frame,
                                        timeout_duration,
                                        &loop_worker_name,
                                    )
                                    .await;

                                    inner_slot.is_busy.store(false, Ordering::Relaxed);
                                    let _ = job.reply.send(result);
                                }
                            }
                        }
                    }
                });

                // 标记存活状态为 false，拒绝后续新提交
                thread_is_alive.store(false, Ordering::Relaxed);

                // 排空并响应队列中可能残留的未执行任务，防止调用方悬挂
                if let Ok(mut guard) = thread_slot.job.lock() {
                    if let Some(stale_job) = guard.take() {
                        let _ = stale_job.reply.send(Err(InferError::Execution {
                            reason: "推理工作线程已退出，未完成任务已取消".to_string(),
                        }));
                    }
                }

                tracing::info!(worker_name = %thread_worker_name, "专用常驻推理 OS 线程已平稳退出");
                let _ = exit_tx.send(());
            })
            .expect("创建专用常驻推理线程失败");

        Self {
            worker_name,
            handle,
            shutdown_tx: Some(shutdown_tx),
            exit_rx: std::sync::Mutex::new(exit_rx),
            thread_handle: Some(thread_handle),
        }
    }

    /// 获取跨协程调用的推理客户端句柄
    #[inline]
    pub fn handle(&self) -> InferenceWorkerHandle {
        self.handle.clone()
    }

    /// 优雅停止工作线程（带超时保护，超时后强制隔离放弃，杜绝无界阻塞守护进程）
    pub fn stop(&mut self, timeout: Duration) -> bool {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(thread) = self.thread_handle.take() {
            let recv_res = self
                .exit_rx
                .lock()
                .map(|rx| rx.recv_timeout(timeout))
                .unwrap_or(Err(std::sync::mpsc::RecvTimeoutError::Disconnected));

            match recv_res {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = thread.join();
                    tracing::debug!(worker_name = %self.worker_name, "推理工作线程已正常优雅退出并回收");
                    true
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    tracing::error!(
                        worker_name = %self.worker_name,
                        timeout_ms = timeout.as_millis() as u64,
                        "推理工作线程在指定超时时间内未能退出（疑似硬件驱动内核调用挂起），执行隔离放弃，避免阻塞主守护进程"
                    );
                    false
                }
            }
        } else {
            true
        }
    }

    /// 显式发出停止信号并等待推理线程退出（使用默认 500ms 超时）
    pub fn shutdown(&mut self) -> bool {
        self.stop(DEFAULT_INFER_SHUTDOWN_TIMEOUT)
    }
}

impl Drop for InferenceWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// 执行带 Panic 异常隔离与超时保护的单帧推理
async fn execute_inference(
    backend: &dyn InferenceBackend,
    frame: &FrameRef,
    timeout: Duration,
    worker_name: &str,
) -> Result<Vec<Detection>, InferError> {
    let call_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.detect(frame)));

    let infer_fut = match call_res {
        Ok(fut) => fut,
        Err(payload) => {
            let msg = format_panic_message(payload);
            tracing::error!(%worker_name, error = %msg, "推理后端启动调用发生 Panic，已安全隔离");
            return Err(InferError::Execution {
                reason: format!("推理后端启动 Panic 异常: {msg}"),
            });
        }
    };

    // 使用 futures::FutureExt::catch_unwind 替代手写 unsafe Pin 投影，完全在安全 Rust 边界内运转
    let safe_fut = std::panic::AssertUnwindSafe(infer_fut).catch_unwind();
    match tokio::time::timeout(timeout, safe_fut).await {
        Ok(Ok(detect_res)) => detect_res,
        Ok(Err(panic_payload)) => {
            let msg = format_panic_message(panic_payload);
            tracing::error!(%worker_name, error = %msg, "推理线程捕获到 Future 异步执行 Panic，已安全隔离");
            Err(InferError::Execution {
                reason: format!("推理后端执行 Panic 异常: {msg}"),
            })
        }
        Err(_) => {
            tracing::error!(
                %worker_name,
                timeout_ms = timeout.as_millis() as u64,
                "单帧推理超时超过设定阈值，已触发超时保护"
            );
            Err(InferError::Execution {
                reason: format!("单帧推理超时超过 {timeout:?}"),
            })
        }
    }
}

fn format_panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "未知 panic 异常".to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use types::{BoundingBox, FrameHandle, PixelFormat, StrideInfo};

    #[derive(Debug)]
    struct MockEchoBackend {
        sleep_ms: u64,
    }

    #[async_trait]
    impl InferenceBackend for MockEchoBackend {
        fn name(&self) -> &'static str {
            "MockEcho"
        }

        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
            if self.sleep_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            }
            Ok(vec![Detection {
                class_id: 0,
                label: "person".to_string(),
                confidence: 0.95,
                bbox: BoundingBox::new(0.1, 0.1, 0.2, 0.2),
            }])
        }
    }

    fn make_dummy_frame(timestamp: i64) -> FrameRef {
        FrameRef::new(
            "cam_mock".to_string(),
            timestamp,
            1920,
            1080,
            StrideInfo::new(1920, 1080),
            PixelFormat::Nv12,
            FrameHandle::Host(Arc::from(vec![0u8; 100])),
        )
    }

    #[tokio::test]
    async fn test_inference_worker_basic_execution() {
        let backend = Arc::new(MockEchoBackend { sleep_ms: 10 });
        let worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        assert!(handle.is_alive());
        let frame = make_dummy_frame(1000);
        let detections = handle.submit(frame).await.expect("推理调用失败");
        assert_eq!(detections.len(), 1);
        assert_eq!(detections[0].label, "person");
        assert_eq!(handle.dropped_frames_count(), 0);
    }

    #[tokio::test]
    async fn test_inference_worker_drop_oldest_behavior() {
        // 后端单次耗时 80ms
        let backend = Arc::new(MockEchoBackend { sleep_ms: 80 });
        let worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        // 提交第一个帧（将占用工作线程 80ms）
        let h1 = handle.clone();
        let f1 = make_dummy_frame(1000);
        let t1 = tokio::spawn(async move { h1.submit(f1).await });

        // 稍微等待 10ms 让任务开始进入推理
        tokio::time::sleep(Duration::from_millis(10)).await;

        // 在工作线程忙碌时，迅速塞入第二个帧（进入 waiting slot）
        let h2 = handle.clone();
        let f2 = make_dummy_frame(2000);
        let t2 = tokio::spawn(async move { h2.submit(f2).await });

        // 再次塞入第三个帧（应该将第二个帧从 slot 中弹出并丢弃！）
        tokio::time::sleep(Duration::from_millis(10)).await;
        let h3 = handle.clone();
        let f3 = make_dummy_frame(3000);
        let t3 = tokio::spawn(async move { h3.submit(f3).await });

        let res1 = t1.await.unwrap();
        let res2 = t2.await.unwrap();
        let res3 = t3.await.unwrap();

        assert!(res1.is_ok(), "首个任务应该正常完成");
        assert!(res2.is_err(), "被插队的旧任务应该收到 drop-oldest 错误");
        assert!(res3.is_ok(), "最后到来的最新任务应该成功完成");
        assert_eq!(handle.dropped_frames_count(), 1, "丢弃计数必须为 1");
    }

    #[tokio::test]
    async fn test_inference_worker_timeout() {
        // 后端单次耗时 200ms
        let backend = Arc::new(MockEchoBackend { sleep_ms: 200 });
        let config = InferenceWorkerConfig {
            worker_name: "test-timeout-worker".to_string(),
            timeout_ms: 50, // 设定 50ms 超时
        };
        let worker = InferenceWorker::with_config(backend, config);
        let handle = worker.handle();

        let frame = make_dummy_frame(1000);
        let res = handle.submit(frame).await;
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("超时"), "必须正确返回超时错误: {err_msg}");
    }

    #[derive(Debug)]
    struct PanickingBackend;

    #[async_trait]
    impl InferenceBackend for PanickingBackend {
        fn name(&self) -> &'static str {
            "PanickingMock"
        }

        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
            panic!("底层硬件算子致命断言失败 (模拟 Panic)");
        }
    }

    #[tokio::test]
    async fn test_inference_worker_panic_isolation() {
        let backend = Arc::new(PanickingBackend);
        let worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        let frame = make_dummy_frame(1000);
        let res = handle.submit(frame).await;
        assert!(res.is_err(), "Panic 必须被捕获并转换为 Err");
        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("Panic"),
            "错误信息必须明确标注 Panic 异常: {err_msg}"
        );
        // 关键断言：OS 工作线程未崩溃，依然健康存活！
        assert!(handle.is_alive(), "专用工作线程必须在 Panic 发生后存活");

        // 验证后续请求仍然能继续接收处理
        let frame2 = make_dummy_frame(2000);
        let res2 = handle.submit(frame2).await;
        assert!(res2.is_err());
    }

    #[tokio::test]
    async fn test_inference_worker_stop_and_drain_lifecycle() {
        let backend = Arc::new(MockEchoBackend { sleep_ms: 50 });
        let mut worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        assert!(handle.is_alive());

        // 提交一个任务，紧接着停止工作线程
        let h = handle.clone();
        let f = make_dummy_frame(1000);
        let fut = tokio::spawn(async move { h.submit(f).await });

        // 显式停止，带超时保护
        let stopped = worker.stop(Duration::from_millis(300));
        assert!(stopped, "工作线程应当在时限内平稳停止并回收");
        assert!(!handle.is_alive(), "工作线程停止后 is_alive 应当变为 false");

        let _ = fut.await;

        // 停止后再提交任务应该立即被拒绝
        let res_after = handle.submit(make_dummy_frame(2000)).await;
        assert!(res_after.is_err());
    }
}
