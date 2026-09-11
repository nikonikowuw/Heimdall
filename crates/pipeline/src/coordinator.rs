//! 分析任务运行时协调器 (Task Runtime Coordinator)
//!
//! 该模块只负责摄像头级运行时编排。媒体解码、算法实例和推理线程的具体实现
//! 分别由 `media` 与 `infer` 所有，协调器通过明确的资源顺序完成启动、回滚和停止。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex as TokioMutex, OwnedMutexGuard, RwLock as TokioRwLock};
use tokio_util::task::AbortOnDropHandle;
use types::{CodecType, TransportPolicy};

use crate::error::PipelineError;
use crate::manager::PipelineManager;

const MAX_ALGO_PARAMS_BYTES: usize = 64 * 1024;
const ATTACH_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// 分析管线协调器错误枚举
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("参数校验失败: {reason}")]
    Validation { reason: String },

    #[error("媒体会话或订阅异常: {0}")]
    Media(#[from] media::error::MediaError),

    #[error("算法包未找到: {algorithm_id}")]
    AlgorithmNotFound { algorithm_id: String },

    #[error("算法实例创建失败: {reason}")]
    AlgorithmInstance { reason: String },

    #[error("管线错误: {0}")]
    Pipeline(#[from] PipelineError),

    #[error("摄像头管线启动已被停止请求取消")]
    Cancelled,

    #[error("Tokio 阻塞任务异常: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

/// 单个算法实例启动配置
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLaunchConfig {
    /// 算法包唯一标识
    pub algorithm_id: String,
    /// 算法初始化业务自定义参数
    pub algo_params: serde_json::Value,
    /// 分析采样目标帧率 (0 表示不限帧率，1..=60 为目标帧率)
    pub target_fps: u32,
}

impl InstanceLaunchConfig {
    /// 从持久化参数解析构建启动配置并校验
    pub fn from_persisted(
        algorithm_id: impl Into<String>,
        params_json: &str,
        analysis_fps: i32,
    ) -> Result<Self, String> {
        if !(0..=60).contains(&analysis_fps) {
            return Err(format!(
                "analysis_fps 超出 0..=60 范围 (当前: {analysis_fps})"
            ));
        }
        let algo_params = match serde_json::from_str::<serde_json::Value>(params_json) {
            Ok(value) if value.is_object() => value,
            Ok(_) => return Err("params_json 不是有效 JSON Object".to_string()),
            Err(err) => return Err(format!("params_json 无法解析: {err}")),
        };
        let target_fps = if analysis_fps > 0 {
            analysis_fps as u32
        } else {
            10
        };
        Ok(Self {
            algorithm_id: algorithm_id.into(),
            algo_params,
            target_fps,
        })
    }
}

/// 启动单路摄像头分析管线参数
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartCameraPipelineParams {
    /// 摄像头唯一标识
    pub camera_id: String,
    /// 主码流 RTSP 拉流地址 (用于高分辨率按需关键帧抓拍)
    pub main_rtsp_url: String,
    /// 主码流视频编码格式
    pub main_codec: CodecType,
    /// 子码流 RTSP 拉流地址 (用于常驻硬件解码与 NPU 推理)
    pub sub_rtsp_url: String,
    /// 子码流视频编码格式
    pub sub_codec: CodecType,
    /// 网络流传输协议策略 (Auto / TCP / UDP)
    pub transport_policy: TransportPolicy,
    /// 绑定的算法实例集合
    pub instances: Vec<InstanceLaunchConfig>,
    /// 是否启用简易帧差运动门控 (静止场景跳过推理)
    pub motion_gate_enabled: bool,
}

impl StartCameraPipelineParams {
    /// 兼容单算法构造器
    #[allow(clippy::too_many_arguments)]
    pub fn single(
        camera_id: impl Into<String>,
        main_rtsp_url: impl Into<String>,
        main_codec: CodecType,
        sub_rtsp_url: impl Into<String>,
        sub_codec: CodecType,
        transport_policy: TransportPolicy,
        algorithm_id: impl Into<String>,
        algo_params: serde_json::Value,
        target_fps: u32,
        motion_gate_enabled: bool,
    ) -> Self {
        Self {
            camera_id: camera_id.into(),
            main_rtsp_url: main_rtsp_url.into(),
            main_codec,
            sub_rtsp_url: sub_rtsp_url.into(),
            sub_codec,
            transport_policy,
            motion_gate_enabled,
            instances: vec![InstanceLaunchConfig {
                algorithm_id: algorithm_id.into(),
                algo_params,
                target_fps,
            }],
        }
    }

    /// 获取主算法 ID (首个算法实例)
    pub fn primary_algorithm_id(&self) -> &str {
        self.instances
            .first()
            .map(|i| i.algorithm_id.as_str())
            .unwrap_or("")
    }

    /// 获取主算法采样帧率
    pub fn primary_target_fps(&self) -> u32 {
        self.instances.first().map(|i| i.target_fps).unwrap_or(0)
    }

    /// 获取主算法自定义参数
    pub fn primary_algo_params(&self) -> serde_json::Value {
        self.instances
            .first()
            .map(|i| i.algo_params.clone())
            .unwrap_or_else(|| serde_json::json!({}))
    }

    /// 校验启动参数合法性，并在进入媒体/FFI 层前拒绝明显无效输入。
    pub fn validate(&self) -> Result<(), CoordinatorError> {
        let cam_id = self.camera_id.trim();
        if cam_id.is_empty() {
            return Err(CoordinatorError::Validation {
                reason: "camera_id 不能为空".to_string(),
            });
        }
        if cam_id.len() > 128 || cam_id.contains('\0') {
            return Err(CoordinatorError::Validation {
                reason: "camera_id 长度或字符非法".to_string(),
            });
        }

        validate_rtsp_url("main_rtsp_url", &self.main_rtsp_url)?;
        validate_rtsp_url("sub_rtsp_url", &self.sub_rtsp_url)?;

        if self.instances.is_empty() {
            return Err(CoordinatorError::Validation {
                reason: "至少需要一个算法实例".to_string(),
            });
        }

        let mut seen = HashSet::with_capacity(self.instances.len());
        for inst in &self.instances {
            let algo_id = inst.algorithm_id.trim();
            if algo_id.is_empty() {
                return Err(CoordinatorError::Validation {
                    reason: "algorithm_id 不能为空".to_string(),
                });
            }
            if algo_id.len() > 128 || algo_id.contains('\0') {
                return Err(CoordinatorError::Validation {
                    reason: "algorithm_id 长度或字符非法".to_string(),
                });
            }
            if !seen.insert(algo_id.to_string()) {
                return Err(CoordinatorError::Validation {
                    reason: format!("存在重复的 algorithm_id: {algo_id}"),
                });
            }

            if inst.target_fps > 60 {
                return Err(CoordinatorError::Validation {
                    reason: format!("target_fps 必须在 0..=60 之间 (当前: {})", inst.target_fps),
                });
            }

            let algo_params = inst.algo_params.to_string();
            if algo_params.len() > MAX_ALGO_PARAMS_BYTES || algo_params.contains('\0') {
                return Err(CoordinatorError::Validation {
                    reason: format!(
                        "algo_params 序列化后必须小于 {} 字节且不能包含 NUL",
                        MAX_ALGO_PARAMS_BYTES
                    ),
                });
            }
        }

        Ok(())
    }
}

fn validate_rtsp_url(field: &str, url: &str) -> Result<(), CoordinatorError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(CoordinatorError::Validation {
            reason: format!("{field} 不能为空"),
        });
    }

    let parsed = media::rtsp::parse_and_clean_rtsp_url(trimmed).map_err(|err| {
        CoordinatorError::Validation {
            reason: format!("{field} 无效: {err}"),
        }
    })?;
    if parsed.host.trim().is_empty() {
        return Err(CoordinatorError::Validation {
            reason: format!("{field} 缺少主机地址"),
        });
    }
    Ok(())
}

/// 某路摄像头当前活跃的分析运行时条目
pub struct ActiveRuntimeEntry {
    pub camera_id: String,
    pub generation: u64,
    pub params: StartCameraPipelineParams,
    pub main_stream_key: String,
    pub main_attach_handle: AbortOnDropHandle<()>,
    pub sub_stream_key: String,
    pub sub_session: Arc<media::stream_hub::CameraStreamSession>,
    pub worker_handle: infer::InferenceWorkerHandle,
    pub worker_handles: Vec<(String, infer::InferenceWorkerHandle)>,
    pub algo_leases: Vec<infer::AlgoLease>,
    pub started_at_ms: i64,
}

impl std::fmt::Debug for ActiveRuntimeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveRuntimeEntry")
            .field("camera_id", &self.camera_id)
            .field("generation", &self.generation)
            .field("params", &self.params)
            .field("main_stream_key", &self.main_stream_key)
            .field("sub_stream_key", &self.sub_stream_key)
            .field("worker_handle", &self.worker_handle)
            .field("started_at_ms", &self.started_at_ms)
            .finish()
    }
}

/// 单算法实例运行时查询信息
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRuntimeInfo {
    pub algorithm_id: String,
    pub target_fps: u32,
}

/// 摄像头管线运行时查询概要视图
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraPipelineRuntimeInfo {
    pub camera_id: String,
    pub generation: u64,
    pub algorithm_id: String,
    pub target_fps: u32,
    pub instances: Vec<InstanceRuntimeInfo>,
    pub motion_gate_enabled: bool,
    pub is_pump_running: bool,
    pub frames_decoded: u64,
    pub frames_inferred: u64,
    pub alarms_triggered: u64,
    pub started_at_ms: i64,
}

/// 启动操作获得的 camera 级串行槽状态
enum StartSlot {
    AlreadyRunning(u64),
    Cancelled,
    Acquired(OwnedMutexGuard<()>),
}

/// 视频分析任务运行时生命周期协调器
#[derive(Debug, Clone)]
pub struct TaskRuntimeCoordinator {
    pipeline_mgr: Arc<PipelineManager>,
    stream_hub: Arc<media::StreamHub>,
    algo_registry: Arc<infer::package::AlgoRegistry>,
    runtimes: Arc<TokioRwLock<HashMap<String, ActiveRuntimeEntry>>>,
    generation_counter: Arc<AtomicU64>,
    /// camera 级生命周期锁。锁故意按 camera_id 保留，避免删除/重建时产生竞态。
    operation_locks: Arc<TokioRwLock<HashMap<String, Arc<TokioMutex<()>>>>>,
    starting: Arc<TokioRwLock<HashMap<String, usize>>>,
    cancel_requested: Arc<TokioRwLock<HashMap<String, usize>>>,
}

impl TaskRuntimeCoordinator {
    /// 创建新的分析任务运行时协调器
    pub fn new(
        pipeline_mgr: Arc<PipelineManager>,
        stream_hub: Arc<media::StreamHub>,
        algo_registry: Arc<infer::package::AlgoRegistry>,
    ) -> Self {
        Self {
            pipeline_mgr,
            stream_hub,
            algo_registry,
            runtimes: Arc::new(TokioRwLock::new(HashMap::new())),
            generation_counter: Arc::new(AtomicU64::new(1)),
            operation_locks: Arc::new(TokioRwLock::new(HashMap::new())),
            starting: Arc::new(TokioRwLock::new(HashMap::new())),
            cancel_requested: Arc::new(TokioRwLock::new(HashMap::new())),
        }
    }

    /// 获取底层管线管理器引用
    pub fn pipeline_manager(&self) -> &Arc<PipelineManager> {
        &self.pipeline_mgr
    }

    /// 获取底层流媒体中心引用
    pub fn stream_hub(&self) -> &Arc<media::StreamHub> {
        &self.stream_hub
    }

    /// 获取算法包注册表引用
    pub fn algo_registry(&self) -> &Arc<infer::package::AlgoRegistry> {
        &self.algo_registry
    }
}

/// 视频分析任务运行时服务抽象接口 (支持依赖注入与 Mock 测试)
#[async_trait::async_trait]
pub trait TaskRuntimeService: Send + Sync + std::fmt::Debug {
    /// 启动单路摄像机分析管线
    async fn start_camera_pipeline(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError>;

    /// 停止单路摄像机分析管线。
    /// `Ok(false)` 仅表示调用时没有已注册的运行时或分析泵。
    async fn stop_camera_pipeline(&self, camera_id: &str) -> Result<bool, CoordinatorError>;

    /// 停止所有活跃的分析管线
    async fn stop_all(&self);

    /// 查询单路摄像机运行时概要
    async fn get_runtime_info(&self, camera_id: &str) -> Option<CameraPipelineRuntimeInfo>;

    /// 列出所有摄像机的运行时概要
    async fn list_runtime_infos(&self) -> Vec<CameraPipelineRuntimeInfo>;

    /// 检查指定摄像机是否有活跃运行时
    async fn has_active_runtime(&self, camera_id: &str) -> bool;

    /// 为指定摄像机配置空间几何布防规则 (ROI 区域入侵、越界绊线、Mask 遮罩)
    async fn set_camera_rules(&self, camera_id: &str, rules: Vec<types::DetectionRule>);
}

#[async_trait::async_trait]
impl TaskRuntimeService for TaskRuntimeCoordinator {
    async fn start_camera_pipeline(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError> {
        self.start_camera_pipeline(params).await
    }

    async fn stop_camera_pipeline(&self, camera_id: &str) -> Result<bool, CoordinatorError> {
        self.stop_camera_pipeline_checked(camera_id).await
    }

    async fn stop_all(&self) {
        self.stop_all().await
    }

    async fn get_runtime_info(&self, camera_id: &str) -> Option<CameraPipelineRuntimeInfo> {
        self.get_runtime_info(camera_id).await
    }

    async fn list_runtime_infos(&self) -> Vec<CameraPipelineRuntimeInfo> {
        self.list_runtime_infos().await
    }

    async fn has_active_runtime(&self, camera_id: &str) -> bool {
        self.is_pipeline_running(camera_id).await
    }

    async fn set_camera_rules(&self, camera_id: &str, rules: Vec<types::DetectionRule>) {
        self.pipeline_mgr.set_camera_rules(camera_id, rules).await;
    }
}

impl TaskRuntimeCoordinator {
    async fn mark_starting(&self, camera_id: &str) {
        let mut starting = self.starting.write().await;
        *starting.entry(camera_id.to_string()).or_default() += 1;
    }

    async fn finish_starting(&self, camera_id: &str) {
        let mut starting = self.starting.write().await;
        if let Some(count) = starting.get_mut(camera_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                starting.remove(camera_id);
            }
        }
    }

    async fn request_start_cancellation(&self, camera_id: &str) -> bool {
        let starting = self.starting.read().await;
        let Some(count) = starting.get(camera_id).copied() else {
            return false;
        };
        self.cancel_requested
            .write()
            .await
            .insert(camera_id.to_string(), count);
        true
    }

    async fn camera_operation_guard(&self, camera_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let locks = self.operation_locks.read().await;
            locks.get(camera_id).cloned()
        };
        let lock = match lock {
            Some(lock) => lock,
            None => {
                let mut locks = self.operation_locks.write().await;
                locks
                    .entry(camera_id.to_string())
                    .or_insert_with(|| Arc::new(TokioMutex::new(())))
                    .clone()
            }
        };
        lock.lock_owned().await
    }

    async fn acquire_start_slot(
        &self,
        params: &StartCameraPipelineParams,
    ) -> Result<StartSlot, CoordinatorError> {
        params.validate()?;
        let camera_id = params.camera_id.clone();
        let guard = self.camera_operation_guard(&camera_id).await;

        let cancelled = {
            let mut requested = self.cancel_requested.write().await;
            if let Some(count) = requested.get_mut(&camera_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    requested.remove(&camera_id);
                }
                true
            } else {
                false
            }
        };
        if cancelled {
            return Ok(StartSlot::Cancelled);
        }

        let current = {
            let runtimes = self.runtimes.read().await;
            runtimes
                .get(&camera_id)
                .map(|entry| (entry.params == *params, entry.generation))
        };
        let pump_running = self.pipeline_mgr.is_analysis_pump_running(&camera_id).await;

        if let Some((same_params, generation)) = current {
            if same_params && pump_running {
                tracing::debug!(
                    camera_id = %camera_id,
                    generation,
                    "重复启动在资源分配前命中幂等运行时条目"
                );
                return Ok(StartSlot::AlreadyRunning(generation));
            }
        }

        if current.is_some() || pump_running {
            tracing::info!(camera_id = %camera_id, "启动新配置前回收旧摄像头运行时");
            self.stop_camera_pipeline_locked(&camera_id).await;
        }

        Ok(StartSlot::Acquired(guard))
    }

    /// 启动单路摄像头的分析管线。
    pub async fn start_camera_pipeline(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError> {
        let camera_id = params.camera_id.clone();
        self.mark_starting(&camera_id).await;
        let coordinator = self.clone();
        tokio::spawn(async move {
            let result = coordinator.start_camera_pipeline_inner(params).await;
            coordinator.finish_starting(&camera_id).await;
            result
        })
        .await?
    }

    async fn start_camera_pipeline_inner(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError> {
        let _operation_guard = match self.acquire_start_slot(&params).await? {
            StartSlot::AlreadyRunning(generation) => return Ok(generation),
            StartSlot::Cancelled => return Err(CoordinatorError::Cancelled),
            StartSlot::Acquired(guard) => guard,
        };

        let mut instance_configs = Vec::with_capacity(params.instances.len());
        let mut workers = Vec::with_capacity(params.instances.len());
        let mut leases = Vec::with_capacity(params.instances.len());

        for inst in &params.instances {
            let lease = match self.algo_registry.acquire_lease(&inst.algorithm_id).await {
                Ok(lease) => lease,
                Err(err) => {
                    shutdown_workers(workers).await;
                    let msg = err.to_string();
                    if msg.contains("未在注册中心就绪") {
                        return Err(CoordinatorError::AlgorithmNotFound {
                            algorithm_id: inst.algorithm_id.clone(),
                        });
                    }
                    return Err(CoordinatorError::AlgorithmInstance {
                        reason: format!("获取算法算力租约失败 ({}): {err}", inst.algorithm_id),
                    });
                }
            };
            let pkg = lease.package().clone();
            leases.push(lease);

            let camera_id = params.camera_id.clone();
            let algo_params = if inst.algo_params.is_null() {
                None
            } else {
                Some(inst.algo_params.to_string())
            };
            let algorithm_type = pkg.manifest().algorithm_type.clone();
            let instance_result = tokio::task::spawn_blocking(move || {
                pkg.create_instance(&camera_id, algo_params.as_deref())
            })
            .await;
            let instance = match instance_result {
                Ok(Ok(instance)) => instance,
                Ok(Err(err)) => {
                    shutdown_workers(workers).await;
                    return Err(CoordinatorError::AlgorithmInstance {
                        reason: err.to_string(),
                    });
                }
                Err(err) => {
                    shutdown_workers(workers).await;
                    return Err(CoordinatorError::TaskJoin(err));
                }
            };
            let worker = infer::InferenceWorker::new(Arc::new(instance));
            let handle = worker.handle();
            instance_configs.push(crate::pump::WorkerInstanceConfig {
                algorithm_id: inst.algorithm_id.clone(),
                algorithm_type,
                target_fps: inst.target_fps,
                config_json: if inst.algo_params.is_null() {
                    None
                } else {
                    Some(inst.algo_params.to_string())
                },
            });
            workers.push((inst.algorithm_id.clone(), handle, Some(worker)));
        }

        let decoder_camera_id = params.camera_id.clone();
        let decoder_codec = params.sub_codec;
        let decoder = match tokio::task::spawn_blocking(move || {
            media::create_decoder(&decoder_camera_id, decoder_codec)
        })
        .await
        {
            Ok(decoder) => decoder,
            Err(err) => {
                shutdown_workers(workers).await;
                return Err(CoordinatorError::TaskJoin(err));
            }
        };

        self.start_resources_locked(params, decoder, instance_configs, workers, leases)
            .await
    }

    /// 使用指定的 VideoDecoder 与 InferenceWorker 启动分析管线 (向后兼容/测试注入)
    pub async fn start_camera_pipeline_with_decoder_and_worker(
        &self,
        params: StartCameraPipelineParams,
        decoder: Box<dyn media::decoder::VideoDecoder + Send>,
        worker: infer::InferenceWorker,
    ) -> Result<u64, CoordinatorError> {
        let camera_id = params.camera_id.clone();
        self.mark_starting(&camera_id).await;
        let coordinator = self.clone();
        tokio::spawn(async move {
            let result = coordinator
                .start_camera_pipeline_with_decoder_and_worker_inner(params, decoder, worker)
                .await;
            coordinator.finish_starting(&camera_id).await;
            result
        })
        .await?
    }

    async fn start_camera_pipeline_with_decoder_and_worker_inner(
        &self,
        params: StartCameraPipelineParams,
        decoder: Box<dyn media::decoder::VideoDecoder + Send>,
        worker: infer::InferenceWorker,
    ) -> Result<u64, CoordinatorError> {
        let _operation_guard = match self.acquire_start_slot(&params).await {
            Ok(StartSlot::AlreadyRunning(generation)) => {
                dispose_decoder(decoder).await;
                shutdown_worker(worker).await;
                return Ok(generation);
            }
            Ok(StartSlot::Cancelled) => {
                dispose_decoder(decoder).await;
                shutdown_worker(worker).await;
                return Err(CoordinatorError::Cancelled);
            }
            Ok(StartSlot::Acquired(guard)) => guard,
            Err(err) => {
                dispose_decoder(decoder).await;
                shutdown_worker(worker).await;
                return Err(err);
            }
        };

        let handle = worker.handle();
        let mut instance_configs = Vec::with_capacity(params.instances.len());
        let mut workers = Vec::with_capacity(params.instances.len());
        let mut managed_worker_opt = Some(worker);

        if params.instances.is_empty() {
            instance_configs.push(crate::pump::WorkerInstanceConfig {
                algorithm_id: "default".to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: 0,
                config_json: None,
            });
            workers.push(("default".to_string(), handle, managed_worker_opt));
        } else {
            for inst in &params.instances {
                instance_configs.push(crate::pump::WorkerInstanceConfig {
                    algorithm_id: inst.algorithm_id.clone(),
                    algorithm_type: "detection".to_string(),
                    target_fps: inst.target_fps,
                    config_json: None,
                });
                workers.push((
                    inst.algorithm_id.clone(),
                    handle.clone(),
                    managed_worker_opt.take(),
                ));
            }
        }

        self.start_resources_locked(params, decoder, instance_configs, workers, vec![])
            .await
    }

    /// 已经持有 camera operation lock 时装配资源；所有失败路径都在本函数内回滚。
    async fn start_resources_locked(
        &self,
        params: StartCameraPipelineParams,
        decoder: Box<dyn media::decoder::VideoDecoder + Send>,
        instance_configs: Vec<crate::pump::WorkerInstanceConfig>,
        workers: Vec<(
            String,
            infer::InferenceWorkerHandle,
            Option<infer::InferenceWorker>,
        )>,
        algo_leases: Vec<infer::AlgoLease>,
    ) -> Result<u64, CoordinatorError> {
        let camera_id = params.camera_id.clone();
        let main_stream_key = format!("{camera_id}:main");
        let sub_stream_key = format!("{camera_id}:sub");
        let mut main_subscribed = false;
        let mut main_attach_handle: Option<AbortOnDropHandle<()>> = None;
        let mut sub_ai_enabled = false;
        let mut pump_started = false;
        let mut decoder_opt = Some(decoder);
        let mut workers_opt = Some(workers);

        let primary_handle = workers_opt
            .as_ref()
            .and_then(|w| w.first())
            .map(|(_, h, _)| h.clone())
            .expect("workers must not be empty");

        let worker_handles = workers_opt
            .as_ref()
            .map(|w| w.iter().map(|(id, h, _)| (id.clone(), h.clone())).collect())
            .unwrap_or_default();

        let result: Result<u64, CoordinatorError> = async {
            let main_rx = self
                .stream_hub
                .subscribe(
                    &main_stream_key,
                    &params.main_rtsp_url,
                    params.transport_policy,
                )
                .await?;
            main_subscribed = true;

            main_attach_handle = Some(AbortOnDropHandle::new(
                self.pipeline_mgr.attach_main_stream(&camera_id, main_rx),
            ));

            self.stream_hub
                .set_ai_enabled(
                    &sub_stream_key,
                    &params.sub_rtsp_url,
                    params.transport_policy,
                    true,
                )
                .await;
            sub_ai_enabled = true;

            let sub_session = self
                .stream_hub
                .get_or_create_session(
                    &sub_stream_key,
                    &params.sub_rtsp_url,
                    params.transport_policy,
                )
                .await;
            let active_decoder = decoder_opt
                .take()
                .expect("decoder is owned by startup transaction");
            let active_workers = workers_opt
                .take()
                .expect("workers are owned by startup transaction");

            self.pipeline_mgr
                .start_analysis_pump_multi_worker(
                    &camera_id,
                    sub_session.clone(),
                    active_decoder,
                    instance_configs,
                    active_workers,
                    params.motion_gate_enabled,
                )
                .await;
            pump_started = true;
            self.pipeline_mgr.set_ai_active(&camera_id, true).await;

            let generation = self.generation_counter.fetch_add(1, Ordering::SeqCst);
            let entry = ActiveRuntimeEntry {
                camera_id: camera_id.clone(),
                generation,
                params: params.clone(),
                main_stream_key: main_stream_key.clone(),
                main_attach_handle: main_attach_handle
                    .take()
                    .expect("main attach handle is owned by startup transaction"),
                sub_stream_key: sub_stream_key.clone(),
                sub_session,
                worker_handle: primary_handle,
                worker_handles,
                algo_leases,
                started_at_ms: chrono::Utc::now().timestamp_millis(),
            };
            self.runtimes.write().await.insert(camera_id.clone(), entry);

            tracing::info!(camera_id = %camera_id, generation, "摄像头分析管线启动成功");
            Ok(generation)
        }
        .await;

        if let Err(err) = &result {
            tracing::error!(camera_id = %camera_id, error = %err, "分析管线启动失败，执行逆序回滚");
            if pump_started {
                self.pipeline_mgr.stop_analysis_pump(&camera_id).await;
            }
            if let Some(workers) = workers_opt.take() {
                for (_, _, worker) in workers {
                    if let Some(w) = worker {
                        shutdown_worker(w).await;
                    }
                }
            }
            if let Some(decoder) = decoder_opt.take() {
                dispose_decoder(decoder).await;
            }
            if sub_ai_enabled {
                self.stream_hub
                    .set_ai_enabled(
                        &sub_stream_key,
                        &params.sub_rtsp_url,
                        params.transport_policy,
                        false,
                    )
                    .await;
            }
            if let Some(handle) = main_attach_handle.take() {
                abort_join_handle(handle).await;
            }
            if main_subscribed {
                self.stream_hub.unsubscribe(&main_stream_key).await;
            }
            self.pipeline_mgr.set_ai_active(&camera_id, false).await;
            self.pipeline_mgr
                .remove_pipeline_context_if_idle(&camera_id)
                .await;
        }

        result
    }

    /// 停止指定摄像头的分析管线，并保留停止任务的 JoinError。
    pub async fn stop_camera_pipeline_checked(
        &self,
        camera_id: &str,
    ) -> Result<bool, CoordinatorError> {
        let coordinator = self.clone();
        let camera_id = camera_id.to_string();
        let camera_id_for_task = camera_id.clone();
        tokio::spawn(async move {
            let _guard = coordinator
                .camera_operation_guard(&camera_id_for_task)
                .await;
            coordinator
                .stop_camera_pipeline_locked(&camera_id_for_task)
                .await
        })
        .await
        .map_err(CoordinatorError::TaskJoin)
    }

    /// 停止指定摄像头的分析管线。异常时记录日志并返回 false。
    pub async fn stop_camera_pipeline(&self, camera_id: &str) -> bool {
        match self.stop_camera_pipeline_checked(camera_id).await {
            Ok(stopped) => stopped,
            Err(err) => {
                tracing::error!(camera_id = %camera_id, error = %err, "停止摄像头管线任务异常");
                false
            }
        }
    }

    async fn stop_camera_pipeline_locked(&self, camera_id: &str) -> bool {
        let entry = {
            let mut runtimes = self.runtimes.write().await;
            runtimes.remove(camera_id)
        };

        if let Some(entry) = entry {
            tracing::info!(
                camera_id = %camera_id,
                generation = entry.generation,
                "开始停止摄像头分析管线"
            );
            self.pipeline_mgr.stop_analysis_pump(camera_id).await;
            self.stream_hub
                .set_ai_enabled(
                    &entry.sub_stream_key,
                    &entry.params.sub_rtsp_url,
                    entry.params.transport_policy,
                    false,
                )
                .await;
            abort_join_handle(entry.main_attach_handle).await;
            self.stream_hub.unsubscribe(&entry.main_stream_key).await;
            self.pipeline_mgr.set_ai_active(camera_id, false).await;
            if let Some(ctx) = self.pipeline_mgr.get_pipeline_context(camera_id).await {
                ctx.release_decoder_if_idle().await;
            }
            self.pipeline_mgr
                .remove_pipeline_context_if_idle(camera_id)
                .await;
            tracing::info!(camera_id = %camera_id, generation = entry.generation, "摄像头分析管线已回收");
            true
        } else {
            if self.request_start_cancellation(camera_id).await {
                tracing::info!(camera_id = %camera_id, "已向进行中的摄像头管线启动请求发送取消信号");
            }
            let stopped = self.pipeline_mgr.stop_analysis_pump(camera_id).await;
            self.pipeline_mgr.set_ai_active(camera_id, false).await;
            if let Some(ctx) = self.pipeline_mgr.get_pipeline_context(camera_id).await {
                ctx.release_decoder_if_idle().await;
            }
            self.pipeline_mgr
                .remove_pipeline_context_if_idle(camera_id)
                .await;
            stopped
        }
    }

    /// 停止所有受管摄像头的分析管线（并行停止各独立摄像头）。整个操作同样不受调用方取消影响。
    pub async fn stop_all(&self) {
        let coordinator = self.clone();
        let _ = tokio::spawn(async move {
            let camera_ids: HashSet<String> = {
                let runtimes = coordinator.runtimes.read().await;
                let starting = coordinator.starting.read().await;
                runtimes.keys().chain(starting.keys()).cloned().collect()
            };
            for camera_id in &camera_ids {
                let _ = coordinator.request_start_cancellation(camera_id).await;
            }
            let mut set = tokio::task::JoinSet::new();
            for camera_id in camera_ids {
                let coord = coordinator.clone();
                set.spawn(async move {
                    let _guard = coord.camera_operation_guard(&camera_id).await;
                    coord.stop_camera_pipeline_locked(&camera_id).await;
                });
            }
            while let Some(res) = set.join_next().await {
                if let Err(err) = res {
                    tracing::error!(error = %err, "并行停止摄像头管线任务发生异常");
                }
            }
            coordinator.pipeline_mgr.stop_all_pumps().await;
        })
        .await;
    }

    /// 检查指定摄像头的分析管线是否处于活跃运行状态
    pub async fn is_pipeline_running(&self, camera_id: &str) -> bool {
        let has_entry = {
            let runtimes = self.runtimes.read().await;
            runtimes.contains_key(camera_id)
        };
        has_entry && self.pipeline_mgr.is_analysis_pump_running(camera_id).await
    }

    /// 获取指定摄像头的分析管线运行时详细信息与度量指标
    pub async fn get_runtime_info(&self, camera_id: &str) -> Option<CameraPipelineRuntimeInfo> {
        let entry_info = {
            let runtimes = self.runtimes.read().await;
            runtimes
                .get(camera_id)
                .map(|entry| (entry.generation, entry.params.clone(), entry.started_at_ms))
        };
        let (generation, params, started_at_ms) = entry_info?;
        let is_pump_running = self.pipeline_mgr.is_analysis_pump_running(camera_id).await;
        let metrics = self.pipeline_mgr.get_analysis_pump_metrics(camera_id).await;
        let (frames_decoded, frames_inferred, alarms_triggered) = metrics
            .map(|metrics| {
                (
                    metrics.frames_decoded.load(Ordering::Relaxed),
                    metrics.frames_inferred.load(Ordering::Relaxed),
                    metrics.alarms_triggered.load(Ordering::Relaxed),
                )
            })
            .unwrap_or((0, 0, 0));

        let instances = params
            .instances
            .iter()
            .map(|i| InstanceRuntimeInfo {
                algorithm_id: i.algorithm_id.clone(),
                target_fps: i.target_fps,
            })
            .collect();
        let primary_algo = params.primary_algorithm_id().to_string();
        let primary_fps = params.primary_target_fps();

        Some(CameraPipelineRuntimeInfo {
            camera_id: camera_id.to_string(),
            generation,
            algorithm_id: primary_algo,
            target_fps: primary_fps,
            instances,
            motion_gate_enabled: params.motion_gate_enabled,
            is_pump_running,
            frames_decoded,
            frames_inferred,
            alarms_triggered,
            started_at_ms,
        })
    }
}

async fn shutdown_worker(worker: infer::InferenceWorker) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut worker = worker;
        worker.shutdown()
    })
    .await;
}

async fn shutdown_workers(
    workers: Vec<(
        String,
        infer::InferenceWorkerHandle,
        Option<infer::InferenceWorker>,
    )>,
) {
    for (_, _, worker) in workers {
        if let Some(worker) = worker {
            shutdown_worker(worker).await;
        }
    }
}

async fn dispose_decoder(decoder: Box<dyn media::decoder::VideoDecoder + Send>) {
    let _ = tokio::task::spawn_blocking(move || drop(decoder)).await;
}

async fn abort_join_handle(mut handle: AbortOnDropHandle<()>) {
    handle.abort();
    if tokio::time::timeout(ATTACH_SHUTDOWN_TIMEOUT, &mut handle)
        .await
        .is_err()
    {
        tracing::error!(
            timeout_ms = ATTACH_SHUTDOWN_TIMEOUT.as_millis() as u64,
            "主码流 attach 任务停止超时，已放弃等待"
        );
    }
}
