//! 专用常驻推理线程与句柄抽象 (Inference Worker & Drop-Oldest Channel)
//!
//! 核心设计保证：
//! 1. **线程常驻 (Context Pinning)**：为推理后端分配专属独立 OS 线程，杜绝 Tokio 协程漂移导致的 NPU 上下文失效；
//! 2. **丢旧帧背压防护 (Drop-Oldest Backpressure Protection)**：单槽容量限制，超载时丢弃旧帧，绝不反压上游视频解码；
//! 3. **超时与异常隔离**：调用端超时断路防止上游管线冻结，结合内部耗时校验与 `catch_unwind` 隔离底层崩溃；
//! 4. **停机超时受控隔离 (Graceful Shutdown & Quarantine Retention)**：退出带超时保护；硬件卡死时将 OS 线程与
//!    硬件句柄移入受控隔离池永久保活，杜绝提前释放导致 Use-After-Free 内存踩踏。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::{oneshot, watch, Notify};
use types::{Detection, FrameRef};

use crate::backend::{InferenceBackend, InferenceResult};
use crate::error::InferError;

/// 工作线程优雅关停超时上限（2500ms，需大于单帧推理超时阈值，超时后强制隔离保活，防止拖死守护进程退出）
pub const DEFAULT_INFER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(2500);

/// 获取当前时间戳（毫秒）
fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 推理 Worker 启动握手超时上限；若库/模型初始化卡死，线程进入隔离池而不是脱管。
const WORKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

/// Worker 控制消息通道容量；控制消息不得静默丢弃，满载时调用方收到明确的拥塞错误。
const WORKER_CONTROL_CHANNEL_CAPACITY: usize = 8;

/// 控制消息排队等待上限；超时说明目标 Worker 已长时间无法接受控制面指令。
const WORKER_CONTROL_SEND_TIMEOUT: Duration = Duration::from_secs(1);

/// 工作线程控制消息
///
/// 所有控制消息只在该 Worker 的专属 OS 线程内串行执行，保证底层 SDK/FFI 调用的
/// 线程归属与创建时一致。
enum WorkerControl {
    /// 在当前硬件上下文内原地更新实例配置
    UpdateConfig {
        config_json: String,
        reply: oneshot::Sender<Result<(), InferError>>,
    },
}

/// 被隔离的挂死推理工作线程条目
///
/// 后端对象由工作线程闭包独占持有；即使线程卡在同步 C ABI 调用中，
/// 保留 JoinHandle 就会同时保留该线程栈、后端 session 和动态库引用。
struct QuarantinedWorkerEntry {
    worker_name: String,
    quarantined_at: std::time::Instant,
    thread_handle: std::thread::JoinHandle<()>,
}

/// 隔离工作线程公开诊断信息
#[derive(Debug, Clone)]
pub struct QuarantinedWorkerInfo {
    pub worker_name: String,
    pub quarantined_duration_ms: u64,
}

static QUARANTINE_POOL: Mutex<Vec<QuarantinedWorkerEntry>> = Mutex::new(Vec::new());

/// 将超时未能退出的工作线程移入受控隔离池（保活句柄与动态库上下文）
fn quarantine_worker(worker_name: String, thread_handle: std::thread::JoinHandle<()>) {
    if let Ok(mut pool) = QUARANTINE_POOL.lock() {
        pool.push(QuarantinedWorkerEntry {
            worker_name,
            quarantined_at: std::time::Instant::now(),
            thread_handle,
        });
    }
}

/// 查询当前系统处于隔离保活状态的推理工作线程数量
pub fn quarantined_workers_count() -> usize {
    QUARANTINE_POOL.lock().map_or(0, |pool| pool.len())
}

/// 获取当前所有被隔离工作线程的诊断信息
pub fn list_quarantined_workers() -> Vec<QuarantinedWorkerInfo> {
    QUARANTINE_POOL
        .lock()
        .map(|pool| {
            pool.iter()
                .map(|e| QuarantinedWorkerInfo {
                    worker_name: e.worker_name.clone(),
                    quarantined_duration_ms: e.quarantined_at.elapsed().as_millis() as u64,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 尝试回收隔离池中已恢复并退出的工作线程
///
/// 若底层硬件驱动调用后来恢复并平稳退出，可在此处非阻塞回收 OS 线程并释放硬件资源。
/// 返回成功回收并释放的线程数量。
pub fn try_reclaim_quarantined_workers() -> usize {
    let Ok(mut pool) = QUARANTINE_POOL.lock() else {
        return 0;
    };

    let (finished, still_hung): (Vec<_>, Vec<_>) = pool
        .drain(..)
        .partition(|worker| worker.thread_handle.is_finished());

    *pool = still_hung;
    let reclaimed_count = finished.len();

    for worker in finished {
        let _ = worker.thread_handle.join();
        tracing::info!(
            worker_name = %worker.worker_name,
            quarantined_duration_ms = worker.quarantined_at.elapsed().as_millis() as u64,
            "被隔离的推理工作线程已完成退出，已成功回收 OS 线程与硬件句柄"
        );
    }

    reclaimed_count
}

#[cfg(test)]
pub fn clear_quarantine_pool_for_test() {
    if let Ok(mut pool) = QUARANTINE_POOL.lock() {
        pool.clear();
    }
}

/// 推理工作线程生命周期状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// 正常运行中
    Running,
    /// 已平稳优雅关停，OS 线程已 join 回收
    Stopped,
    /// 关停超时，底层硬件/驱动卡死，OS 线程与硬件上下文已被移入隔离池受控保活
    Quarantined,
}

/// 单次推理任务请求
struct InferenceJob {
    frame: FrameRef,
    reply: oneshot::Sender<Result<InferenceResult, InferError>>,
}

/// 共享单槽任务队列（Drop-Oldest 丢旧帧机制核心）
struct SharedSlot {
    job: Mutex<Option<InferenceJob>>,
    notify: Notify,
    dropped_count: AtomicU64,
    is_busy: AtomicBool,
    last_busy_start_ms: AtomicU64,
}

impl SharedSlot {
    fn new() -> Self {
        Self {
            job: Mutex::new(None),
            notify: Notify::new(),
            dropped_count: AtomicU64::new(0),
            is_busy: AtomicBool::new(false),
            last_busy_start_ms: AtomicU64::new(0),
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
    timeout: Duration,
    control_tx: tokio::sync::mpsc::Sender<WorkerControl>,
}

impl std::fmt::Debug for InferenceWorkerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferenceWorkerHandle")
            .field("is_alive", &self.is_alive())
            .field("is_busy", &self.is_busy())
            .field("timeout", &self.timeout)
            .field("dropped_count", &self.dropped_frames_count())
            .finish()
    }
}

impl InferenceWorkerHandle {
    /// 提交一帧执行推理并保留低频特征 sidecar。
    ///
    /// 采用客户端断路超时机制：
    /// 即使底层 OS 线程卡死在同步 C FFI / 内核驱动中无法被抢占，
    /// 调用端（如 AnalysisPump）也会在超时时限后断路返回，防止上游管线被永久挂死。
    pub async fn submit_with_metadata(
        &self,
        frame: FrameRef,
    ) -> Result<InferenceResult, InferError> {
        if !self.is_alive.load(Ordering::Relaxed) {
            return Err(InferError::Execution {
                reason: "推理工作线程已退出或处于隔离状态".to_string(),
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

        self.slot.notify.notify_one();

        // 客户端断路超时保护：
        // 宽限 100ms 让工作线程优先发出内部结构化超时或 Panic 错误；
        // 若工作线程因同步 C FFI / 内核驱动卡死导致没有响应，客户端断路超时切断等待。
        let client_timeout = self.timeout.saturating_add(Duration::from_millis(100));
        match tokio::time::timeout(client_timeout, reply_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(InferError::Execution {
                reason: "推理工作线程通道意外关闭".to_string(),
            }),
            Err(_) => {
                tracing::error!(
                    timeout_ms = self.timeout.as_millis() as u64,
                    "推理工作线程响应超时（疑似底层硬件/驱动死锁），触发调用端断路保护"
                );
                Err(InferError::Timeout(self.timeout))
            }
        }
    }

    /// 提交一帧执行推理检测（具备 Drop-Oldest 丢旧帧保护）。
    pub async fn submit(&self, frame: FrameRef) -> Result<Vec<Detection>, InferError> {
        Ok(self.submit_with_metadata(frame).await?.detections)
    }

    /// 在当前 Worker 的硬件上下文内原地更新实例配置。
    ///
    /// 返回 [`InferError::Unsupported`] 表示该后端不能原地换配置，调用方必须回退到
    /// 目标实例级 Worker 替换；其他错误表示插件拒绝或 FFI 失败，旧配置继续生效。
    pub async fn update_config(&self, config_json: String) -> Result<(), InferError> {
        if !self.is_alive.load(Ordering::Relaxed) {
            return Err(InferError::Execution {
                reason: "推理工作线程已退出或处于隔离状态".to_string(),
            });
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let message = WorkerControl::UpdateConfig {
            config_json,
            reply: reply_tx,
        };

        match tokio::time::timeout(WORKER_CONTROL_SEND_TIMEOUT, self.control_tx.send(message)).await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(InferError::Execution {
                    reason: "推理工作线程控制通道已关闭".to_string(),
                })
            }
            Err(_) => {
                return Err(InferError::Execution {
                    reason: "推理工作线程控制通道拥塞，控制面指令未被接受".to_string(),
                })
            }
        }

        // 与单帧推理一致的客户端断路保护：底层同步 FFI 卡死时不让控制面无限等待。
        let client_timeout = self.timeout.saturating_add(Duration::from_millis(100));
        match tokio::time::timeout(client_timeout, reply_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(InferError::Execution {
                reason: "推理工作线程控制通道意外关闭".to_string(),
            }),
            Err(_) => Err(InferError::Timeout(self.timeout)),
        }
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

    /// 查询单帧推理超时时限
    #[inline]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 查询推理线程是否可能在底层硬件调用中挂起（处于忙碌状态且耗时超过指定阈值）
    pub fn is_stalled(&self, threshold: Duration) -> bool {
        if !self.is_busy() {
            return false;
        }
        let start_ms = self.slot.last_busy_start_ms.load(Ordering::Relaxed);
        start_ms > 0 && current_epoch_ms().saturating_sub(start_ms) > threshold.as_millis() as u64
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
    state: WorkerState,
}

impl std::fmt::Debug for InferenceWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferenceWorker")
            .field("worker_name", &self.worker_name)
            .field("state", &self.state)
            .field("is_alive", &self.handle.is_alive())
            .field("dropped_count", &self.handle.dropped_frames_count())
            .finish()
    }
}

impl InferenceWorker {
    /// 基于指定推理后端创建并启动专用常驻推理线程。
    ///
    /// 后端值在 Worker OS 线程闭包内部取得所有权，避免依赖 `Arc<dyn ...>` 推导
    /// 底层 SDK session 可跨线程共享。C ABI 后端应使用 [`Self::with_backend_factory`]，
    /// 让 session 也在该线程内创建。
    pub fn new<B>(backend: B) -> Self
    where
        B: InferenceBackend + Send,
    {
        let name = format!("infer-worker-{}", backend.name());
        let config = InferenceWorkerConfig {
            worker_name: name,
            ..Default::default()
        };
        Self::with_config(backend, config)
    }

    /// 带自定义配置创建并启动专用常驻推理线程。
    pub fn with_config<B>(backend: B, config: InferenceWorkerConfig) -> Self
    where
        B: InferenceBackend + Send,
    {
        Self::with_backend_factory(
            move || Ok(Box::new(backend) as Box<dyn InferenceBackend>),
            config,
        )
        .expect("创建推理 Worker 失败")
    }

    /// 在 Worker OS 线程内创建后端并绑定其生命周期。
    ///
    /// 工厂返回的后端对象不会跨出线程闭包；因此可以安全承载不实现 `Send`/`Sync`
    /// 的 C ABI session。构造握手只有在工厂和专用 runtime 都成功后才返回。
    pub fn with_backend_factory<F>(
        factory: F,
        config: InferenceWorkerConfig,
    ) -> Result<Self, InferError>
    where
        F: FnOnce() -> Result<Box<dyn InferenceBackend>, InferError> + Send + 'static,
    {
        let slot = Arc::new(SharedSlot::new());
        let is_alive = Arc::new(AtomicBool::new(true));
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let (control_tx, mut control_rx) =
            tokio::sync::mpsc::channel::<WorkerControl>(WORKER_CONTROL_CHANNEL_CAPACITY);

        let timeout_duration = Duration::from_millis(config.timeout_ms);
        let handle = InferenceWorkerHandle {
            slot: slot.clone(),
            is_alive: is_alive.clone(),
            timeout: timeout_duration,
            control_tx: control_tx.clone(),
        };

        let worker_name = config.worker_name.clone();
        let thread_worker_name = worker_name.clone();
        let thread_slot = slot.clone();
        let thread_is_alive = is_alive.clone();
        // 工作线程自持一份发送端：控制面句柄全部释放后，接收端仍能感知通道存活，
        // 避免 `recv()` 持续返回 None 导致 select 空转；线程退出仍由 shutdown watch 驱动。
        let thread_control_guard = control_tx.clone();

        let thread_handle = std::thread::Builder::new()
            .name(worker_name.clone())
            .spawn(move || {
                tracing::info!(worker_name = %thread_worker_name, "专用常驻推理 OS 线程已启动并绑定");

                let backend = match factory() {
                    Ok(backend) => backend,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error));
                        thread_is_alive.store(false, Ordering::Relaxed);
                        let _ = exit_tx.send(());
                        return;
                    }
                };

                // 创建单线程独立运行时，与主进程 Tokio 工作池物理隔离
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(err) => {
                        let error = InferError::Execution {
                            reason: format!("创建专用推理单线程运行时失败: {err}"),
                        };
                        let _ = startup_tx.send(Err(error));
                        tracing::error!(error = %err, "创建专用推理单线程运行时失败");
                        thread_is_alive.store(false, Ordering::Relaxed);
                        let _ = exit_tx.send(());
                        return;
                    }
                };
                let _ = startup_tx.send(Ok(()));

                let loop_worker_name = thread_worker_name.as_str();
                rt.block_on(async {
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

                            // 控制面指令优先于推理帧：配置热更新必须在下一个安全帧边界前落地
                            control = control_rx.recv() => {
                                if let Some(control) = control {
                                    handle_worker_control(
                                        backend.as_ref(),
                                        control,
                                        loop_worker_name,
                                    );
                                }
                            }

                            // 监听待推理帧到达
                            _ = thread_slot.notify.notified() => {
                                let maybe_job = thread_slot.job.lock().ok().and_then(|mut g| g.take());

                                if let Some(job) = maybe_job {
                                    thread_slot.is_busy.store(true, Ordering::Relaxed);
                                    thread_slot.last_busy_start_ms.store(current_epoch_ms(), Ordering::Relaxed);

                                    let result = execute_inference(
                                        backend.as_ref(),
                                        &job.frame,
                                        timeout_duration,
                                        loop_worker_name,
                                    )
                                    .await;

                                    thread_slot.is_busy.store(false, Ordering::Relaxed);
                                    thread_slot.last_busy_start_ms.store(0, Ordering::Relaxed);
                                    let _ = job.reply.send(result);
                                }
                            }
                        }
                    }
                });

                // backend 在此线程闭包结束时析构，保证 C ABI instance_destroy 与创建/调用线程一致。
                thread_is_alive.store(false, Ordering::Relaxed);
                drop(thread_control_guard);

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
            .map_err(|err| InferError::Execution {
                reason: format!("创建专用常驻推理线程失败: {err}"),
            })?;

        match startup_rx.recv_timeout(WORKER_STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                worker_name,
                handle,
                shutdown_tx: Some(shutdown_tx),
                exit_rx: std::sync::Mutex::new(exit_rx),
                thread_handle: Some(thread_handle),
                state: WorkerState::Running,
            }),
            Ok(Err(error)) => {
                let _ = thread_handle.join();
                Err(error)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                tracing::error!(
                    worker_name = %worker_name,
                    timeout_ms = WORKER_STARTUP_TIMEOUT.as_millis() as u64,
                    "推理 Worker 启动握手超时，疑似算法库或模型初始化阻塞，移入隔离池保活"
                );
                quarantine_worker(worker_name, thread_handle);
                Err(InferError::Timeout(WORKER_STARTUP_TIMEOUT))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread_handle.join();
                Err(InferError::Execution {
                    reason: "推理 Worker 启动握手通道异常关闭".to_string(),
                })
            }
        }
    }

    /// 获取跨协程调用的推理客户端句柄
    #[inline]
    pub fn handle(&self) -> InferenceWorkerHandle {
        self.handle.clone()
    }

    /// 查询当前工作线程状态
    #[inline]
    pub fn state(&self) -> WorkerState {
        self.state
    }

    /// 优雅停止工作线程（带超时保护与挂死受控隔离机制）
    ///
    /// - 若在指定时限内平稳退出，join 回收 OS 线程并返回 `true`；
    /// - 若超时（疑似硬件驱动内核调用挂起），立即将工作线程及底层动态库句柄移入全局受管隔离池保活，
    ///   杜绝提前释放导致 Use-After-Free 内存踩踏，并返回 `false`。
    pub fn stop(&mut self, timeout: Duration) -> bool {
        // 1. 立即标记 handle 为非存活状态，防止外部继续提交新帧
        self.handle.is_alive.store(false, Ordering::SeqCst);

        // 2. 幂等性检查：若已经处于终态，直接返回
        match self.state {
            WorkerState::Stopped => return true,
            WorkerState::Quarantined => return false,
            WorkerState::Running => {}
        }

        // 3. 排空并取消 slot 中可能残留的未执行任务，防止调用方悬挂
        if let Ok(mut guard) = self.handle.slot.job.lock() {
            if let Some(stale_job) = guard.take() {
                let _ = stale_job.reply.send(Err(InferError::Execution {
                    reason: "推理工作线程正在停止，任务已取消".to_string(),
                }));
            }
        }

        // 4. 发出退出通知
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }

        // 5. 等待线程平稳退出
        let Some(thread) = self.thread_handle.take() else {
            self.state = WorkerState::Stopped;
            return true;
        };

        let recv_res = self
            .exit_rx
            .lock()
            .map(|rx| rx.recv_timeout(timeout))
            .unwrap_or(Err(std::sync::mpsc::RecvTimeoutError::Disconnected));

        match recv_res {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                self.state = WorkerState::Stopped;
                tracing::debug!(worker_name = %self.worker_name, "推理工作线程已正常优雅退出并回收");
                true
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.state = WorkerState::Quarantined;
                tracing::error!(
                    worker_name = %self.worker_name,
                    timeout_ms = timeout.as_millis() as u64,
                    "推理工作线程在指定超时时间内未能退出（疑似硬件驱动内核调用挂起），移入全局隔离池受控保活，杜绝句柄提前释放导致内存踩踏"
                );
                quarantine_worker(self.worker_name.clone(), thread);
                false
            }
        }
    }

    /// 显式发出停止信号并等待推理线程退出（使用默认 2500ms 超时）
    pub fn shutdown(&mut self) -> bool {
        self.stop(DEFAULT_INFER_SHUTDOWN_TIMEOUT)
    }
}

impl Drop for InferenceWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// 在 Worker 专属 OS 线程内处理一条控制面指令
///
/// 与单帧推理一致地隔离 panic：插件在配置解析或硬件重配中崩溃不能让工作线程死亡。
fn handle_worker_control(
    backend: &dyn InferenceBackend,
    control: WorkerControl,
    worker_name: &str,
) {
    match control {
        WorkerControl::UpdateConfig { config_json, reply } => {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend.update_config(&config_json)
            }))
            .unwrap_or_else(|payload| {
                let msg = format_panic_message(payload);
                tracing::error!(%worker_name, error = %msg, "插件配置热更新发生 Panic，已安全隔离");
                Err(InferError::Execution {
                    reason: format!("配置热更新 Panic 异常: {msg}"),
                })
            });

            match &result {
                Ok(()) => tracing::info!(%worker_name, "算法实例配置已在 Worker 线程内生效"),
                Err(InferError::Unsupported { capability }) => tracing::debug!(
                    %worker_name,
                    capability,
                    "插件不支持原地配置更新，需回退目标实例 Worker 替换"
                ),
                Err(error) => {
                    tracing::warn!(%worker_name, error = %error, "算法实例配置热更新被拒绝")
                }
            }

            // 调用方可能已断路超时；发送失败仅说明无人等待，不影响已发生的配置状态。
            let _ = reply.send(result);
        }
    }
}

/// 执行带 Panic 异常隔离与超时保护的单帧推理
async fn execute_inference(
    backend: &dyn InferenceBackend,
    frame: &FrameRef,
    timeout: Duration,
    worker_name: &str,
) -> Result<InferenceResult, InferError> {
    let infer_fut = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.detect_with_metadata(frame)
    })) {
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
                "单帧推理耗时超过设定阈值，已触发超时保护"
            );
            Err(InferError::Timeout(timeout))
        }
    }
}

fn format_panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|&s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "未知 panic 异常".to_string())
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

    #[async_trait(?Send)]
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
                quality_score: None,
                bbox: BoundingBox::new(0.1, 0.1, 0.2, 0.2),
                face: None,
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

    #[async_trait(?Send)]
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

    /// 支持原地热更新的后端：阈值在 Worker 线程内被改写，且必须由创建它的线程持有。
    #[derive(Debug, Default)]
    struct HotUpdatableBackend {
        threshold: std::sync::Mutex<f32>,
        update_owner: std::sync::Mutex<Option<std::thread::ThreadId>>,
    }

    #[async_trait(?Send)]
    impl InferenceBackend for HotUpdatableBackend {
        fn name(&self) -> &'static str {
            "HotUpdatable"
        }

        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
            let threshold = *self.threshold.lock().unwrap();
            Ok(vec![Detection {
                class_id: 0,
                label: "person".to_string(),
                confidence: threshold,
                quality_score: None,
                bbox: BoundingBox::new(0.1, 0.1, 0.2, 0.2),
                face: None,
            }])
        }

        fn update_config(&self, config_json: &str) -> Result<(), InferError> {
            let parsed: serde_json::Value =
                serde_json::from_str(config_json).map_err(|err| InferError::JsonParse {
                    reason: err.to_string(),
                })?;
            let threshold = parsed
                .get("threshold")
                .and_then(|value| value.as_f64())
                .ok_or_else(|| InferError::Execution {
                    reason: "缺少 threshold".to_string(),
                })?;
            *self.threshold.lock().unwrap() = threshold as f32;
            *self.update_owner.lock().unwrap() = Some(std::thread::current().id());
            Ok(())
        }
    }

    /// 配置热更新必须在该 Worker 的专属 OS 线程内执行，且新配置从下一帧开始生效。
    #[tokio::test]
    async fn test_inference_worker_hot_config_update_on_worker_thread() {
        let backend = Arc::new(HotUpdatableBackend::default());
        let backend_ref = backend.clone();
        let caller_thread_id = std::thread::current().id();

        let worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        let before = handle
            .submit(make_dummy_frame(1000))
            .await
            .expect("推理调用失败");
        assert_eq!(before[0].confidence, 0.0);

        handle
            .update_config(r#"{"threshold":0.88}"#.to_string())
            .await
            .expect("热更新应成功");

        let after = handle
            .submit(make_dummy_frame(2000))
            .await
            .expect("推理调用失败");
        assert_eq!(after[0].confidence, 0.88, "新配置必须从下一帧开始生效");

        let worker_thread_id = backend_ref
            .update_owner
            .lock()
            .unwrap()
            .expect("必须记录更新线程");
        assert_ne!(
            worker_thread_id, caller_thread_id,
            "配置更新必须发生在 Worker 专属线程，而不是调用方线程"
        );
    }

    /// 插件不支持原地更新时必须返回明确的 Unsupported，不得误报配置已生效。
    #[tokio::test]
    async fn test_inference_worker_config_update_reports_unsupported() {
        let backend = Arc::new(MockEchoBackend { sleep_ms: 0 });
        let worker = InferenceWorker::new(backend);
        let handle = worker.handle();

        let err = handle
            .update_config(r#"{"threshold":0.5}"#.to_string())
            .await
            .expect_err("未实现热更新必须返回错误");
        assert!(
            matches!(err, InferError::Unsupported { .. }),
            "必须返回 Unsupported 以便调用方回退 Worker 替换，实际: {err:?}"
        );
    }

    #[derive(Debug)]
    struct PanickingUpdateBackend;

    #[async_trait(?Send)]
    impl InferenceBackend for PanickingUpdateBackend {
        fn name(&self) -> &'static str {
            "PanickingUpdate"
        }

        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
            Ok(Vec::new())
        }

        fn update_config(&self, _config_json: &str) -> Result<(), InferError> {
            panic!("插件配置解析致命断言失败 (模拟 Panic)");
        }
    }

    #[tokio::test]
    async fn test_inference_worker_config_update_panic_isolation() {
        clear_quarantine_pool_for_test();
        let worker = InferenceWorker::new(Arc::new(PanickingUpdateBackend));
        let handle = worker.handle();

        let err = handle
            .update_config("{}".to_string())
            .await
            .expect_err("Panic 必须被捕获并转换为 Err");
        assert!(matches!(err, InferError::Execution { .. }));
        assert!(handle.is_alive(), "配置热更新 Panic 不得杀死常驻工作线程");
        assert!(
            handle.submit(make_dummy_frame(1000)).await.is_ok(),
            "热更新失败后线程仍必须能够继续推理"
        );
    }

    #[tokio::test]
    async fn test_inference_worker_config_update_rejected_after_stop() {
        let mut worker = InferenceWorker::new(Arc::new(HotUpdatableBackend::default()));
        let handle = worker.handle();

        assert!(worker.stop(Duration::from_millis(300)));
        let err = handle
            .update_config("{}".to_string())
            .await
            .expect_err("已停止的 Worker 必须拒绝配置更新");
        assert!(matches!(err, InferError::Execution { .. }));
    }

    #[derive(Debug)]
    struct BlockingFfiMockBackend {
        sleep_ms: u64,
    }

    #[async_trait(?Send)]
    impl InferenceBackend for BlockingFfiMockBackend {
        fn name(&self) -> &'static str {
            "BlockingFfiMock"
        }

        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
            // 模拟底层同步 C FFI 阻塞（不可中断，不让出 Tokio 协程）
            std::thread::sleep(Duration::from_millis(self.sleep_ms));
            Ok(vec![Detection {
                class_id: 1,
                label: "car".to_string(),
                confidence: 0.9,
                quality_score: None,
                bbox: BoundingBox::new(0.0, 0.0, 1.0, 1.0),
                face: None,
            }])
        }
    }

    #[tokio::test]
    async fn test_inference_worker_blocking_ffi_timeout_breaker() {
        // 模拟底层 C FFI 阻塞 300ms
        let backend = Arc::new(BlockingFfiMockBackend { sleep_ms: 300 });
        let config = InferenceWorkerConfig {
            worker_name: "test-blocking-ffi-worker".to_string(),
            timeout_ms: 50, // 设定 50ms 超时，客户端断路阈值为 50 + 100 = 150ms
        };
        let worker = InferenceWorker::with_config(backend, config);
        let handle = worker.handle();

        let start = std::time::Instant::now();
        let frame = make_dummy_frame(1000);
        let res = handle.submit(frame).await;
        let elapsed = start.elapsed();

        // 必须在客户端断路超时触发返回，而不是死等 300ms 阻塞
        assert!(res.is_err(), "卡死 FFI 任务必须触发超时断路");
        assert!(
            elapsed < Duration::from_millis(280),
            "客户端断路超时必须在底层 FFI 完成前抢先救回调用方，当前耗时: {elapsed:?}"
        );
        let err = res.unwrap_err();
        assert!(matches!(err, InferError::Timeout(_)));

        // 验证 is_stalled 能够检测到底层线程处于忙碌挂起状态
        if handle.is_busy() {
            assert!(handle.is_stalled(Duration::from_millis(20)));
        }
    }

    #[tokio::test]
    async fn test_inference_worker_stop_quarantines_hung_thread() {
        clear_quarantine_pool_for_test();

        // 后端在 C FFI 同步阻塞 400ms
        let backend = Arc::new(BlockingFfiMockBackend { sleep_ms: 400 });
        let config = InferenceWorkerConfig {
            worker_name: "test-quarantine-worker".to_string(),
            timeout_ms: 500,
        };
        let mut worker = InferenceWorker::with_config(backend, config);
        let handle = worker.handle();

        // 提交阻塞任务，立即占用工作线程
        let h = handle.clone();
        let f = make_dummy_frame(1000);
        let _fut = tokio::spawn(async move { h.submit(f).await });

        // 等待 20ms 确保工作线程已进入 FFI 阻塞
        tokio::time::sleep(Duration::from_millis(20)).await;

        // stop 仅等待 50ms，必然发生超时
        let stopped = worker.stop(Duration::from_millis(50));
        assert!(
            !stopped,
            "卡在 FFI 中的工作线程在 stop 超时时必须返回 false"
        );
        assert_eq!(worker.state(), WorkerState::Quarantined);
        assert!(!handle.is_alive(), "隔离后的句柄必须处于非存活状态");

        // 验证隔离池成功接管 OS 线程与后端句柄，杜绝脱管丢失
        assert_eq!(quarantined_workers_count(), 1);
        let list = list_quarantined_workers();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].worker_name, "test-quarantine-worker");

        // 等待底层 400ms 模拟阻塞结束
        tokio::time::sleep(Duration::from_millis(450)).await;

        // 尝试非阻塞回收已退出的隔离线程
        let reclaimed = try_reclaim_quarantined_workers();
        assert_eq!(reclaimed, 1, "底层 C FFI 结束后隔离线程应当被成功回收");
        assert_eq!(quarantined_workers_count(), 0);
    }
}
