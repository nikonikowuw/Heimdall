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
use types::{is_effective_main_stream, CodecType, MotionGateConfig, TransportPolicy};

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
    /// 算法实例唯一标识（运行时控制、状态与结果隔离的主键）
    pub instance_id: String,
    /// 算法包唯一标识（用于加载包与展示名称，不作为寻址依据）
    pub algorithm_id: String,
    /// 算法初始化业务自定义参数
    pub algo_params: serde_json::Value,
    /// 分析采样目标帧率 (0 表示不限帧率，1..=60 为目标帧率)
    pub target_fps: u32,
}

impl InstanceLaunchConfig {
    /// 从持久化参数解析构建启动配置并校验
    pub fn from_persisted(
        instance_id: impl Into<String>,
        algorithm_id: impl Into<String>,
        params_json: &str,
        analysis_fps: i32,
    ) -> Result<Self, String> {
        if !(0..=60).contains(&analysis_fps) {
            return Err(format!(
                "analysis_fps 超出 0..=60 范围 (当前: {analysis_fps})"
            ));
        }
        let instance_id = instance_id.into();
        if instance_id.trim().is_empty() {
            return Err("instance_id 不能为空".to_string());
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
            instance_id,
            algorithm_id: algorithm_id.into(),
            algo_params,
            target_fps,
        })
    }
}

/// 媒体输入契约签名（不含算法实例集合）
///
/// 用于判定一次期望配置变更是否真的需要重建解码器与物理连接：
/// 地址、编码格式、传输策略或运动门控变化才重建，实例集合变化走增量收敛。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaContractSignature {
    pub main_rtsp_url: String,
    pub main_codec: CodecType,
    pub sub_rtsp_url: String,
    pub sub_codec: CodecType,
    pub transport_policy: TransportPolicy,
    pub motion_gate: Option<MotionGateConfig>,
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
    /// 已决议的分析码流 RTSP 地址（可以是子码流，也可以是主码流降级/显式主流）
    pub sub_rtsp_url: String,
    /// 已决议的分析码流视频编码格式
    pub sub_codec: CodecType,
    /// 网络流传输协议策略 (Auto / TCP / UDP)
    pub transport_policy: TransportPolicy,
    /// 绑定的算法实例集合
    pub instances: Vec<InstanceLaunchConfig>,
    /// 运动门控配置；`None` 表示不启用门控
    pub motion_gate: Option<MotionGateConfig>,
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
        let algorithm_id = algorithm_id.into();
        Self {
            camera_id: camera_id.into(),
            main_rtsp_url: main_rtsp_url.into(),
            main_codec,
            sub_rtsp_url: sub_rtsp_url.into(),
            sub_codec,
            transport_policy,
            motion_gate: motion_gate_enabled.then(MotionGateConfig::default),
            instances: vec![InstanceLaunchConfig {
                // 兼容单算法构造器没有独立实例身份，退化为以算法 ID 作实例主键。
                instance_id: algorithm_id.clone(),
                algorithm_id,
                algo_params,
                target_fps,
            }],
        }
    }

    /// 媒体输入契约签名：只有这些字段变化才需要重建整路媒体/解码运行时。
    ///
    /// 实例集合（增删算法、改阈值、改抽帧频率）不在此签名内，可增量收敛。
    pub fn media_signature(&self) -> MediaContractSignature {
        MediaContractSignature {
            main_rtsp_url: self.main_rtsp_url.clone(),
            main_codec: self.main_codec,
            sub_rtsp_url: self.sub_rtsp_url.clone(),
            sub_codec: self.sub_codec,
            transport_policy: self.transport_policy,
            motion_gate: self.motion_gate.clone(),
        }
    }

    /// 就地更新（或追加）单个算法实例的期望配置快照
    pub fn upsert_instance(&mut self, launch: &InstanceLaunchConfig) {
        match self
            .instances
            .iter_mut()
            .find(|inst| inst.instance_id == launch.instance_id)
        {
            Some(existing) => *existing = launch.clone(),
            None => self.instances.push(launch.clone()),
        }
    }

    /// 移除单个算法实例的期望配置快照
    pub fn remove_instance(&mut self, instance_id: &str) {
        self.instances
            .retain(|inst| inst.instance_id != instance_id);
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

    /// 判定已决议的分析流是否为主码流。
    ///
    /// `sub_rtsp_url` 在进入协调器前已完成 StreamMode 选择和 Auto 探活降级，
    /// 此处不再重新解释用户原始模式。
    pub fn is_main_stream_analysis(&self) -> bool {
        is_effective_main_stream(&self.main_rtsp_url, &self.sub_rtsp_url)
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
        let mut seen_instance_ids = HashSet::with_capacity(self.instances.len());
        for inst in &self.instances {
            let instance_id = inst.instance_id.trim();
            if instance_id.is_empty() {
                return Err(CoordinatorError::Validation {
                    reason: "instance_id 不能为空".to_string(),
                });
            }
            if instance_id.len() > 128 || instance_id.contains('\0') {
                return Err(CoordinatorError::Validation {
                    reason: "instance_id 长度或字符非法".to_string(),
                });
            }
            if !seen_instance_ids.insert(instance_id) {
                return Err(CoordinatorError::Validation {
                    reason: format!("存在重复的 instance_id: {instance_id}"),
                });
            }

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
            if !seen.insert(algo_id) {
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

/// 单个算法实例的运行时归属
///
/// 租约与 Worker 句柄按 `instanceId` 独立持有，增量增删实例时只动自己这一份，
/// 不影响其他实例与共享解码器。
pub struct ActiveInstanceEntry {
    pub instance_id: String,
    pub algorithm_id: String,
    /// 算力租约；实例卸载时随条目一起释放。
    ///
    /// 注入式启动路径（直接给定 Worker、不经注册表）没有租约，此类实例无法
    /// 就地重建 Worker，只能等到任务重启时重新收敛。
    pub lease: Option<infer::AlgoLease>,
    pub worker_handle: infer::InferenceWorkerHandle,
}

/// 某路摄像头当前活跃的分析运行时条目
pub struct ActiveRuntimeEntry {
    pub camera_id: String,
    pub generation: u64,
    /// 当前期望的启动参数快照（含实例集合）；增量变更后同步更新，
    /// 保证完整启动请求的幂等比较仍基于最新期望配置。
    pub params: StartCameraPipelineParams,
    pub main_stream_key: String,
    pub main_attach_handle: Option<AbortOnDropHandle<()>>,
    pub sub_stream_key: String,
    pub sub_session: Arc<media::stream_hub::CameraStreamSession>,
    /// 实例级运行时归属表
    pub instances: Vec<ActiveInstanceEntry>,
    pub started_at_ms: i64,
}

impl std::fmt::Debug for ActiveInstanceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveInstanceEntry")
            .field("instance_id", &self.instance_id)
            .field("algorithm_id", &self.algorithm_id)
            .field("worker_alive", &self.worker_handle.is_alive())
            .finish()
    }
}

impl ActiveRuntimeEntry {
    pub fn instance_entry(&self, instance_id: &str) -> Option<&ActiveInstanceEntry> {
        self.instances
            .iter()
            .find(|entry| entry.instance_id == instance_id)
    }
}

impl std::fmt::Debug for ActiveRuntimeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveRuntimeEntry")
            .field("camera_id", &self.camera_id)
            .field("generation", &self.generation)
            .field("params", &self.params)
            .field("main_stream_key", &self.main_stream_key)
            .field("sub_stream_key", &self.sub_stream_key)
            .field(
                "instances",
                &self
                    .instances
                    .iter()
                    .map(|entry| (&entry.instance_id, &entry.algorithm_id))
                    .collect::<Vec<_>>(),
            )
            .field("started_at_ms", &self.started_at_ms)
            .finish()
    }
}

/// 单算法实例运行时查询信息
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRuntimeInfo {
    pub instance_id: String,
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

/// 单个算法实例的期望配置快照（数据库为唯一事实来源）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceDesiredConfig {
    pub camera_id: String,
    pub instance_id: String,
    pub algorithm_id: String,
    /// 分析采样目标帧率；0 表示不限帧率，1..=60 为目标帧率
    pub analysis_fps: i32,
    /// 算法自定义参数 JSON
    pub params_json: String,
    /// 该实例是否应处于运行态
    pub enabled: bool,
    /// 期望配置版本号（`algorithm_instances.desired_revision`）
    pub desired_revision: i64,
}

impl InstanceDesiredConfig {
    /// 从启动配置构造期望快照（整路保存路径没有单实例版本号，`desired_revision` 记 0）
    pub fn from_launch(camera_id: &str, launch: &InstanceLaunchConfig) -> Self {
        Self {
            camera_id: camera_id.to_string(),
            instance_id: launch.instance_id.clone(),
            algorithm_id: launch.algorithm_id.clone(),
            analysis_fps: launch.target_fps as i32,
            params_json: launch.algo_params.to_string(),
            enabled: true,
            desired_revision: 0,
        }
    }

    /// 解析并校验期望配置，得到运行时可直接使用的启动配置
    fn launch_config(&self) -> Result<InstanceLaunchConfig, CoordinatorError> {
        InstanceLaunchConfig::from_persisted(
            &self.instance_id,
            &self.algorithm_id,
            &self.params_json,
            self.analysis_fps,
        )
        .map_err(|reason| CoordinatorError::Validation { reason })
    }

    fn outcome(
        &self,
        apply_state: types::InstanceApplyState,
        mechanism: InstanceApplyMechanism,
        status_message: impl Into<String>,
    ) -> InstanceApplyOutcome {
        let applied_revision =
            (apply_state == types::InstanceApplyState::Applied).then_some(self.desired_revision);
        InstanceApplyOutcome {
            instance_id: self.instance_id.clone(),
            desired_revision: self.desired_revision,
            applied_revision,
            apply_state,
            mechanism,
            status_message: status_message.into(),
        }
    }
}

/// 期望配置的收敛机制（用于回答「这份参数到底是怎么生效的」）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceApplyMechanism {
    /// 摄像头任务未运行，期望配置停留在持久层
    NoRuntime,
    /// 期望值与运行值一致，无需动作
    Noop,
    /// 在目标 Worker 的硬件上下文内原地热更新
    HotUpdate,
    /// 只在帧边界重设抽帧频率
    FrameRate,
    /// 替换目标实例的 Worker（无法热更新的变更）
    WorkerReplace,
    /// 新增挂载实例
    Mount,
    /// 卸载实例
    Unmount,
    /// 整路解码器重建后重新挂载
    PipelineRestart,
}

/// 单个算法实例期望配置的运行时收敛结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceApplyOutcome {
    pub instance_id: String,
    pub desired_revision: i64,
    /// 收敛后运行时实际生效的版本号；仅 `applied` 状态有值
    pub applied_revision: Option<i64>,
    pub apply_state: types::InstanceApplyState,
    pub mechanism: InstanceApplyMechanism,
    /// 非 `applied` 时的可读原因，直接透出给控制面与前端
    pub status_message: String,
}

/// 整路摄像头实例集合的增量收敛结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInstanceSyncOutcome {
    pub camera_id: String,
    /// 是否发生整路重建（仅媒体输入契约变化时为 true）
    pub restarted: bool,
    /// 逐实例收敛结果；整路重建时逐实例结果为空，以 `restarted` 为准
    pub outcomes: Vec<InstanceApplyOutcome>,
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

impl TaskRuntimeCoordinator {
    /// 收敛单个算法实例的期望配置（增量路径）
    ///
    /// 只影响目标实例：解码器、物理连接、其他实例及其 Worker 保持不动。
    /// 返回结果必须区分 `applied` / `pending` / `failed`，不允许用「数据库写成功」
    /// 冒充「运行时已生效」。
    pub async fn apply_instance_config(
        &self,
        desired: InstanceDesiredConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError> {
        let _operation_guard = self.camera_operation_guard(&desired.camera_id).await;
        self.apply_instance_locked(&desired).await
    }

    /// 从运行中的摄像头任务卸载单个算法实例（禁用与删除共用）
    ///
    /// `Ok(false)` 表示该实例本来就不在运行时时无需卸载。
    pub async fn remove_instance_runtime(
        &self,
        camera_id: &str,
        instance_id: &str,
    ) -> Result<bool, CoordinatorError> {
        let _operation_guard = self.camera_operation_guard(camera_id).await;
        let descriptors = self.pipeline_mgr.get_instance_descriptors(camera_id).await;
        if !descriptors
            .iter()
            .any(|descriptor| descriptor.instance_id == instance_id)
        {
            return Ok(false);
        }
        let removed = self
            .pipeline_mgr
            .remove_pump_instance(camera_id, instance_id)
            .await?;
        if removed {
            self.forget_runtime_instance(camera_id, instance_id).await;
        }
        Ok(removed)
    }

    /// 按最新期望配置收敛某路摄像头的算法实例集合
    ///
    /// 媒体输入契约未变化时走增量路径：只增删/更新目标实例。
    /// 仅当主/子码流地址、编码格式、传输策略或运动门控变化时才整路重建。
    pub async fn sync_camera_instances(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<CameraInstanceSyncOutcome, CoordinatorError> {
        let camera_id = params.camera_id.clone();
        let restarted = self.requires_media_restart(&params).await;
        if restarted {
            let generation = self.start_camera_pipeline(params).await?;
            tracing::info!(
                camera_id = %camera_id,
                generation,
                "媒体输入契约已变化，整路重建摄像头分析管线"
            );
            return Ok(CameraInstanceSyncOutcome {
                camera_id,
                restarted: true,
                outcomes: Vec::new(),
            });
        }

        let _operation_guard = self.camera_operation_guard(&camera_id).await;
        let desired: Vec<InstanceLaunchConfig> = params.instances.clone();
        let outcomes = self.reconcile_instances_locked(&params, &desired).await?;
        // 期望快照与本次提交对齐，保证后续整路启动请求的幂等比较基于最新期望
        {
            let mut runtimes = self.runtimes.write().await;
            if let Some(entry) = runtimes.get_mut(&camera_id) {
                entry.params = params;
            }
        }
        Ok(CameraInstanceSyncOutcome {
            camera_id,
            restarted: false,
            outcomes,
        })
    }

    /// 实例集合增量收敛：先卸载再挂载，单个实例失败不影响其他实例
    async fn reconcile_instances_locked(
        &self,
        params: &StartCameraPipelineParams,
        desired: &[InstanceLaunchConfig],
    ) -> Result<Vec<InstanceApplyOutcome>, CoordinatorError> {
        let camera_id = params.camera_id.as_str();
        let mut outcomes = Vec::with_capacity(desired.len());

        // 1. 先卸载不再期望挂载的实例，尽早归还 NPU / 内存配额
        let current = self.pipeline_mgr.get_instance_descriptors(camera_id).await;
        for descriptor in current {
            if desired
                .iter()
                .any(|inst| inst.instance_id == descriptor.instance_id)
            {
                continue;
            }
            let stale = InstanceDesiredConfig {
                camera_id: camera_id.to_string(),
                instance_id: descriptor.instance_id.clone(),
                algorithm_id: descriptor.algorithm_id,
                analysis_fps: 0,
                params_json: "{}".to_string(),
                enabled: false,
                desired_revision: 0,
            };
            outcomes.push(self.unmount_instance_locked(camera_id, &stale).await?);
        }

        // 2. 再按期望挂载或更新
        for launch in desired {
            let instance_desired = InstanceDesiredConfig::from_launch(camera_id, launch);
            outcomes.push(self.apply_instance_locked(&instance_desired).await?);
        }
        Ok(outcomes)
    }

    /// 判断给定期望配置是否需要整路重建。
    ///
    /// 只有媒体输入契约（地址、编码格式、传输策略、运动门控）变化才需要重建解码器与
    /// 物理连接；算法实例集合变化一律走增量收敛。
    pub async fn requires_media_restart(&self, params: &StartCameraPipelineParams) -> bool {
        match self.current_media_signature(&params.camera_id).await {
            Some(signature) => signature != params.media_signature(),
            None => true,
        }
    }

    /// 读取当前运行时的媒体输入契约签名
    async fn current_media_signature(&self, camera_id: &str) -> Option<MediaContractSignature> {
        let runtimes = self.runtimes.read().await;
        runtimes
            .get(camera_id)
            .map(|entry| entry.params.media_signature())
    }

    /// 收敛单个实例（调用方必须已持有 camera 级操作锁）
    async fn apply_instance_locked(
        &self,
        desired: &InstanceDesiredConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError> {
        let launch = desired.launch_config()?;
        let camera_id = desired.camera_id.as_str();

        if !self.is_pipeline_running(camera_id).await {
            // 运行时不在：期望配置只在持久层，任务启动时统一收敛。
            return Ok(desired.outcome(
                types::InstanceApplyState::Applied,
                InstanceApplyMechanism::NoRuntime,
                "摄像头任务未运行，期望配置已持久化，将在下次启动时收敛",
            ));
        }

        let descriptor = self
            .pipeline_mgr
            .get_instance_descriptors(camera_id)
            .await
            .into_iter()
            .find(|descriptor| descriptor.instance_id == desired.instance_id);

        if !desired.enabled {
            return self.unmount_instance_locked(camera_id, desired).await;
        }

        let Some(descriptor) = descriptor else {
            return self.mount_instance_locked(desired, &launch).await;
        };

        // 已挂载：优先在目标 Worker 的硬件上下文内原地热更新
        let mut mechanism = InstanceApplyMechanism::Noop;
        let desired_config_json = launch_config_json(&launch);
        if !config_json_matches(
            descriptor.config_json.as_deref(),
            desired_config_json.as_deref(),
        ) {
            let update = self
                .pipeline_mgr
                .update_pump_instance_config(
                    camera_id,
                    &desired.instance_id,
                    desired_config_json.as_deref().unwrap_or("{}"),
                )
                .await?;
            match update {
                crate::pump::InstanceConfigUpdateOutcome::Applied => {
                    mechanism = InstanceApplyMechanism::HotUpdate;
                    tracing::info!(
                        camera_id = %camera_id,
                        instance_id = %desired.instance_id,
                        "算法实例配置已在目标 Worker 硬件上下文内原地热更新"
                    );
                }
                crate::pump::InstanceConfigUpdateOutcome::Unsupported => {
                    tracing::info!(
                        camera_id = %camera_id,
                        instance_id = %desired.instance_id,
                        "算法包不支持配置热更新，降级为仅替换目标实例 Worker"
                    );
                    if let Err(outcome) = self
                        .replace_instance_worker_locked(camera_id, desired, &launch)
                        .await?
                    {
                        return Ok(outcome);
                    }
                    mechanism = InstanceApplyMechanism::WorkerReplace;
                }
                crate::pump::InstanceConfigUpdateOutcome::NotFound => {
                    return self.mount_instance_locked(desired, &launch).await;
                }
                crate::pump::InstanceConfigUpdateOutcome::Rejected { reason } => {
                    return Ok(desired.outcome(
                        types::InstanceApplyState::Failed,
                        InstanceApplyMechanism::HotUpdate,
                        format!("算法包拒绝该配置: {reason}"),
                    ));
                }
            }
        }

        // 抽帧频率在帧边界由 governor 生效，不重建 Worker
        if descriptor.target_fps != launch.target_fps {
            let applied = self
                .pipeline_mgr
                .set_pump_instance_fps(camera_id, &desired.instance_id, launch.target_fps)
                .await?;
            if !applied {
                return Ok(desired.outcome(
                    types::InstanceApplyState::Failed,
                    mechanism,
                    format!("抽帧频率未能在帧边界生效 (目标 {} FPS)", launch.target_fps),
                ));
            }
            if mechanism == InstanceApplyMechanism::Noop {
                mechanism = InstanceApplyMechanism::FrameRate;
            }
        }

        if mechanism != InstanceApplyMechanism::Noop {
            // 运行时快照与期望对齐，避免后续整路启动请求误判为参数变化
            let mut runtimes = self.runtimes.write().await;
            if let Some(entry) = runtimes.get_mut(camera_id) {
                entry.params.upsert_instance(&launch);
            }
        }
        Ok(desired.outcome(types::InstanceApplyState::Applied, mechanism, ""))
    }

    /// 卸载目标实例并归还其算力租约
    async fn unmount_instance_locked(
        &self,
        camera_id: &str,
        desired: &InstanceDesiredConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError> {
        let mounted = self
            .pipeline_mgr
            .get_instance_descriptors(camera_id)
            .await
            .into_iter()
            .any(|descriptor| descriptor.instance_id == desired.instance_id);
        if !mounted {
            return Ok(desired.outcome(
                types::InstanceApplyState::Applied,
                InstanceApplyMechanism::Noop,
                "",
            ));
        }

        let removed = self
            .pipeline_mgr
            .remove_pump_instance(camera_id, &desired.instance_id)
            .await?;
        if !removed {
            // 保留运行时归属，等待控制面重试；此时实例已停止发帧但不驱逐租约。
            return Ok(desired.outcome(
                types::InstanceApplyState::Failed,
                InstanceApplyMechanism::Unmount,
                "目标实例 Worker 关停未确认，已保留运行时归属等待重试",
            ));
        }
        self.forget_runtime_instance(camera_id, &desired.instance_id)
            .await;
        tracing::info!(
            camera_id = %camera_id,
            instance_id = %desired.instance_id,
            "算法实例已从运行中的分析泵卸载并归还算力租约"
        );
        Ok(desired.outcome(
            types::InstanceApplyState::Applied,
            InstanceApplyMechanism::Unmount,
            "",
        ))
    }

    /// 新增挂载目标实例
    ///
    /// 资源准入失败时不得驱逐任何已运行实例：租约获取或 Worker 创建失败直接把
    /// `failed` 回给控制面，由用户显式决策。
    async fn mount_instance_locked(
        &self,
        desired: &InstanceDesiredConfig,
        launch: &InstanceLaunchConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError> {
        let camera_id = desired.camera_id.as_str();
        let lease = match self
            .algo_registry
            .acquire_lease(&desired.algorithm_id)
            .await
        {
            Ok(lease) => lease,
            Err(err) => {
                let message = err.to_string();
                let outcome = if message.contains("未在注册中心就绪") {
                    desired.outcome(
                        types::InstanceApplyState::Pending,
                        InstanceApplyMechanism::Mount,
                        format!(
                            "算法包 {} 未在注册中心就绪，等待就绪后重新应用",
                            desired.algorithm_id
                        ),
                    )
                } else {
                    desired.outcome(
                        types::InstanceApplyState::Failed,
                        InstanceApplyMechanism::Mount,
                        format!("获取算法算力租约失败: {err}"),
                    )
                };
                return Ok(outcome);
            }
        };

        let package = lease.package().clone();
        let algorithm_type = package.manifest().algorithm_type.clone();
        let config_json = launch_config_json(launch);
        let worker = match self
            .create_instance_worker(&package, &desired.instance_id, config_json.as_deref())
            .await
        {
            Ok(worker) => worker,
            Err(err) => {
                tracing::error!(
                    camera_id = %camera_id,
                    instance_id = %desired.instance_id,
                    error = %err,
                    "增量挂载算法实例失败，保留现有实例运行状态"
                );
                return Ok(desired.outcome(
                    types::InstanceApplyState::Failed,
                    InstanceApplyMechanism::Mount,
                    format!("创建推理 Worker 失败: {err}"),
                ));
            }
        };

        let handle = worker.handle();
        let added = self
            .pipeline_mgr
            .add_pump_instance(
                camera_id,
                crate::pump::WorkerInstanceConfig {
                    instance_id: desired.instance_id.clone(),
                    algorithm_id: desired.algorithm_id.clone(),
                    algorithm_type,
                    target_fps: launch.target_fps,
                    config_json,
                },
                worker,
            )
            .await?;
        if !added {
            return Ok(desired.outcome(
                types::InstanceApplyState::Failed,
                InstanceApplyMechanism::Mount,
                "分析泵已停止或目标实例已挂载，增量挂载未生效",
            ));
        }

        {
            let mut runtimes = self.runtimes.write().await;
            if let Some(entry) = runtimes.get_mut(camera_id) {
                entry
                    .instances
                    .retain(|item| item.instance_id != desired.instance_id);
                entry.instances.push(ActiveInstanceEntry {
                    instance_id: desired.instance_id.clone(),
                    algorithm_id: desired.algorithm_id.clone(),
                    lease: Some(lease),
                    worker_handle: handle,
                });
                entry.params.upsert_instance(launch);
            }
        }
        tracing::info!(
            camera_id = %camera_id,
            instance_id = %desired.instance_id,
            algorithm_id = %desired.algorithm_id,
            "算法实例已增量挂载至运行中的分析泵"
        );
        Ok(desired.outcome(
            types::InstanceApplyState::Applied,
            InstanceApplyMechanism::Mount,
            "",
        ))
    }

    /// 仅替换目标实例的 Worker（模型路径、算法版本等无法热更新的变更）
    ///
    /// 先创建新 Worker 再切换：新 Worker 分配失败时旧 Worker 继续服务，
    /// 避免 NPU 双份内存导致整路实例被驱逐。
    async fn replace_instance_worker_locked(
        &self,
        camera_id: &str,
        desired: &InstanceDesiredConfig,
        launch: &InstanceLaunchConfig,
    ) -> Result<Result<(), InstanceApplyOutcome>, CoordinatorError> {
        let Some(package) = self.instance_package(camera_id, &desired.instance_id).await else {
            return Ok(Err(desired.outcome(
                types::InstanceApplyState::Pending,
                InstanceApplyMechanism::WorkerReplace,
                "目标实例当前未持有算法包租约，无法就地重建 Worker，等待任务重启后收敛",
            )));
        };

        let config_json = launch_config_json(launch);
        let worker = match self
            .create_instance_worker(&package, &desired.instance_id, config_json.as_deref())
            .await
        {
            Ok(worker) => worker,
            Err(err) => {
                return Ok(Err(desired.outcome(
                    types::InstanceApplyState::Failed,
                    InstanceApplyMechanism::WorkerReplace,
                    format!("创建替换 Worker 失败，保留原 Worker 继续服务: {err}"),
                )));
            }
        };

        match self
            .pipeline_mgr
            .replace_pump_instance_worker(camera_id, &desired.instance_id, worker)
            .await
        {
            Ok(true) => {
                tracing::info!(
                    camera_id = %camera_id,
                    instance_id = %desired.instance_id,
                    "目标实例 Worker 已在帧边界完成增量替换"
                );
                Ok(Ok(()))
            }
            Ok(false) => Ok(Err(desired.outcome(
                types::InstanceApplyState::Failed,
                InstanceApplyMechanism::WorkerReplace,
                "目标实例槽位不存在，Worker 替换未生效",
            ))),
            Err(PipelineError::PipelineNotFound { .. }) => Ok(Err(desired.outcome(
                types::InstanceApplyState::Pending,
                InstanceApplyMechanism::WorkerReplace,
                "摄像头分析泵已停止，等待任务重启后收敛",
            ))),
            Err(err) => Err(CoordinatorError::Pipeline(err)),
        }
    }

    /// 在专用阻塞线程内创建推理 Worker（NPU 上下文绑定线程，不得占用 Tokio worker）
    async fn create_instance_worker(
        &self,
        package: &Arc<infer::package::AlgoPackage>,
        instance_id: &str,
        config_json: Option<&str>,
    ) -> Result<infer::InferenceWorker, CoordinatorError> {
        let package = package.clone();
        let backend = package.manifest().platform_id.clone();
        let worker_name = format!(
            "infer-worker-{}-{instance_id}",
            package.manifest().algorithm_id
        );
        let instance_id = instance_id.to_string();
        let config_json = config_json.map(str::to_owned);
        match tokio::task::spawn_blocking(move || {
            let worker_config = infer::InferenceWorkerConfig {
                worker_name,
                ..Default::default()
            };
            package.create_worker(&instance_id, config_json.as_deref(), worker_config)
        })
        .await
        {
            Ok(Ok(worker)) => Ok(worker),
            Ok(Err(err)) => {
                let reason = err.to_string();
                if matches!(
                    &err,
                    infer::InferError::BackendUnavailable { .. }
                        | infer::InferError::Execution { .. }
                        | infer::InferError::Timeout(_)
                ) {
                    crate::op_log::record(types::OpEvent::NpuInitFailed {
                        backend,
                        error: reason.clone(),
                    });
                }
                Err(CoordinatorError::AlgorithmInstance { reason })
            }
            Err(err) => Err(CoordinatorError::TaskJoin(err)),
        }
    }

    /// 读取目标实例当前持有的算力租约对应的算法包
    async fn instance_package(
        &self,
        camera_id: &str,
        instance_id: &str,
    ) -> Option<Arc<infer::package::AlgoPackage>> {
        let runtimes = self.runtimes.read().await;
        runtimes
            .get(camera_id)?
            .instance_entry(instance_id)?
            .lease
            .as_ref()
            .map(|lease| lease.package().clone())
    }

    /// 释放目标实例的运行时归属（算力租约随条目一起归还）
    async fn forget_runtime_instance(&self, camera_id: &str, instance_id: &str) {
        let dropped = {
            let mut runtimes = self.runtimes.write().await;
            runtimes.get_mut(camera_id).and_then(|entry| {
                let before = entry.instances.len();
                entry
                    .instances
                    .retain(|item| item.instance_id != instance_id);
                entry.params.remove_instance(instance_id);
                (entry.instances.len() != before).then_some(entry.instances.len())
            })
        };
        if dropped.is_some() {
            tracing::debug!(
                camera_id = %camera_id,
                instance_id = %instance_id,
                "已释放算法实例运行时归属"
            );
        }
    }
}

/// 实例配置 JSON 是否等价（按解析后的结构比较，忽略键顺序与空白差异）
fn config_json_matches(current: Option<&str>, desired: Option<&str>) -> bool {
    match (current, desired) {
        (None, None) => true,
        (Some(current), Some(desired)) => {
            match (
                serde_json::from_str::<serde_json::Value>(current),
                serde_json::from_str::<serde_json::Value>(desired),
            ) {
                (Ok(current), Ok(desired)) => current == desired,
                _ => current.trim() == desired.trim(),
            }
        }
        (None, Some(desired)) => desired
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim()
            .is_empty(),
        (Some(current), None) => current
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim()
            .is_empty(),
    }
}

/// 构造算法实例的创建配置 JSON（空配置不传，保持与冷启动路径一致）
fn launch_config_json(launch: &InstanceLaunchConfig) -> Option<String> {
    (!launch.algo_params.is_null()).then(|| launch.algo_params.to_string())
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

    /// 按最新期望配置收敛某路摄像头的算法实例集合
    ///
    /// 媒体输入契约未变化时只增删/更新目标实例；变化时整路重建。
    async fn sync_camera_instances(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<CameraInstanceSyncOutcome, CoordinatorError>;

    /// 收敛单个算法实例的期望配置（增量路径，不重启整路管线）
    async fn apply_instance_config(
        &self,
        desired: InstanceDesiredConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError>;

    /// 从运行中的摄像头任务卸载单个算法实例
    async fn remove_instance_runtime(
        &self,
        camera_id: &str,
        instance_id: &str,
    ) -> Result<bool, CoordinatorError>;
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
        // 取景区域由 PipelineManager::set_camera_rules 从同一份规则中提取，此处不重复提取：
        // 两处各自提取会出现「一处清、一处不清」的语义分叉。
        self.pipeline_mgr.set_camera_rules(camera_id, rules).await;
    }

    async fn sync_camera_instances(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<CameraInstanceSyncOutcome, CoordinatorError> {
        self.sync_camera_instances(params).await
    }

    async fn apply_instance_config(
        &self,
        desired: InstanceDesiredConfig,
    ) -> Result<InstanceApplyOutcome, CoordinatorError> {
        self.apply_instance_config(desired).await
    }

    async fn remove_instance_runtime(
        &self,
        camera_id: &str,
        instance_id: &str,
    ) -> Result<bool, CoordinatorError> {
        self.remove_instance_runtime(camera_id, instance_id).await
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

            let algorithm_type = pkg.manifest().algorithm_type.clone();
            // 插件可见的实例身份使用算法实例 ID（而非 camera_id）：同一摄像头挂载
            // 多个同款算法时身份必须互不冲突，且需与增量挂载路径保持一致。
            let worker_instance_id = inst.instance_id.clone();
            let worker_config_json = if inst.algo_params.is_null() {
                None
            } else {
                Some(inst.algo_params.to_string())
            };
            let worker_algorithm_id = inst.algorithm_id.clone();
            let instance_result = tokio::task::spawn_blocking(move || {
                let worker_config = infer::InferenceWorkerConfig {
                    worker_name: format!("infer-worker-{worker_algorithm_id}-{worker_instance_id}"),
                    ..Default::default()
                };
                pkg.create_worker(
                    &worker_instance_id,
                    worker_config_json.as_deref(),
                    worker_config,
                )
            })
            .await;
            let worker = match instance_result {
                Ok(Ok(worker)) => worker,
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
            let handle = worker.handle();
            instance_configs.push(crate::pump::WorkerInstanceConfig {
                instance_id: inst.instance_id.clone(),
                algorithm_id: inst.algorithm_id.clone(),
                algorithm_type,
                target_fps: inst.target_fps,
                config_json: if inst.algo_params.is_null() {
                    None
                } else {
                    Some(inst.algo_params.to_string())
                },
            });
            workers.push((inst.instance_id.clone(), handle, Some(worker)));
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
                instance_id: crate::pump::LEGACY_SINGLE_WORKER_ID.to_string(),
                algorithm_id: "default".to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: 0,
                config_json: None,
            });
            workers.push((
                crate::pump::LEGACY_SINGLE_WORKER_ID.to_string(),
                handle,
                managed_worker_opt,
            ));
        } else {
            for inst in &params.instances {
                instance_configs.push(crate::pump::WorkerInstanceConfig {
                    instance_id: inst.instance_id.clone(),
                    algorithm_id: inst.algorithm_id.clone(),
                    algorithm_type: "detection".to_string(),
                    target_fps: inst.target_fps,
                    config_json: None,
                });
                workers.push((
                    inst.instance_id.clone(),
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
        let mut main_attach_handle: Option<AbortOnDropHandle<()>> = None;
        let mut sub_ai_enabled = false;
        let mut pump_started = false;
        let mut decoder_opt = Some(decoder);
        let mut workers_opt = Some(workers);

        // 实例级运行时归属：按 instanceId 组装（workers 与 leases 均与 params.instances 同序）
        let worker_handles = workers_opt
            .as_ref()
            .map(|w| {
                w.iter()
                    .map(|(id, h, _)| (id.clone(), h.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let result: Result<u64, CoordinatorError> = async {
            // 主码流分析模式下，分析泵已独占消费并常驻解码主码流，证据直接复用推理原生帧，
            // 压缩包证据环没有任何消费者。此时再订阅一次只会在同一条物理连接上多挂一个
            // mailbox 并整环缓存 4s 主码流 NALU；只有分析走独立子码流的双流分工才需要
            // 主码流裸 NALU 进环，供告警时按需前向解码取证。
            if !params.is_main_stream_analysis() {
                let main_rx = self
                    .stream_hub
                    .subscribe(
                        &main_stream_key,
                        &params.main_rtsp_url,
                        params.transport_policy,
                    )
                    .await?;

                main_attach_handle = Some(AbortOnDropHandle::new(
                    self.pipeline_mgr.attach_main_stream(&camera_id, main_rx),
                ));
            }

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

            // 在 pump 接收首个解码帧前设置有效分析流状态，避免首帧告警误走 VPU 追帧。
            self.pipeline_mgr
                .set_main_stream_analysis(&camera_id, params.is_main_stream_analysis())
                .await;

            // 注入主码流/分析流的接入时延锚点。子码流分工时检测帧与主码流证据帧位于两条
            // 由各自 `PLAY` 应答决定的独立 PTS 轴，必须靠实测接入时延换算；主码流分析模式下
            // 两路是同一条物理连接（主/子 URL 相同，StreamHub 按规范化 URL 去重），传同一
            // 实例即恒等换算。
            let analysis_clock = sub_session.clock_anchor.clone();
            let main_clock = if params.is_main_stream_analysis() {
                analysis_clock.clone()
            } else {
                self.stream_hub
                    .get_or_create_session(
                        &main_stream_key,
                        &params.main_rtsp_url,
                        params.transport_policy,
                    )
                    .await
                    .clock_anchor
                    .clone()
            };
            self.pipeline_mgr
                .set_stream_clock_anchors(&camera_id, main_clock, analysis_clock)
                .await;

            self.pipeline_mgr
                .start_analysis_pump_multi_worker(
                    &camera_id,
                    sub_session.clone(),
                    active_decoder,
                    instance_configs,
                    active_workers,
                    params.motion_gate.clone(),
                )
                .await;
            pump_started = true;
            self.pipeline_mgr.set_ai_active(&camera_id, true).await;

            let mut instances = Vec::with_capacity(params.instances.len());
            // 注入式启动路径不提供租约，此时实例归属仍按 instanceId 建立，只是缺少
            // 就地重建 Worker 的能力（`lease = None`）。
            let mut leases = algo_leases.into_iter().map(Some);
            for (inst, (_, handle)) in params.instances.iter().zip(worker_handles) {
                instances.push(ActiveInstanceEntry {
                    instance_id: inst.instance_id.clone(),
                    algorithm_id: inst.algorithm_id.clone(),
                    lease: leases.next().flatten(),
                    worker_handle: handle,
                });
            }

            let generation = self.generation_counter.fetch_add(1, Ordering::SeqCst);
            let entry = ActiveRuntimeEntry {
                camera_id: camera_id.clone(),
                generation,
                params: params.clone(),
                main_stream_key: main_stream_key.clone(),
                main_attach_handle: main_attach_handle.take(),
                sub_stream_key: sub_stream_key.clone(),
                sub_session,
                instances,
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
            self.pipeline_mgr.set_ai_active(&camera_id, false).await;
            self.pipeline_mgr
                .set_main_stream_analysis(&camera_id, false)
                .await;
            self.pipeline_mgr
                .clear_stream_clock_anchors(&camera_id)
                .await;
            // pump 退出时已自清理，此处为幂等双保险确保管线停止后资源干净
            self.pipeline_mgr.clear_decoded_ring(&camera_id).await;
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
            if let Some(handle) = entry.main_attach_handle {
                abort_join_handle(handle).await;
            }
            self.pipeline_mgr.set_ai_active(camera_id, false).await;
            self.pipeline_mgr
                .set_main_stream_analysis(camera_id, false)
                .await;
            self.pipeline_mgr
                .clear_stream_clock_anchors(camera_id)
                .await;
            // pump 退出时已自清理，此处为幂等双保险确保管线停止后资源干净
            self.pipeline_mgr.clear_decoded_ring(camera_id).await;
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
            self.pipeline_mgr
                .set_main_stream_analysis(camera_id, false)
                .await;
            self.pipeline_mgr
                .clear_stream_clock_anchors(camera_id)
                .await;
            // pump 退出时已自清理，此处为幂等双保险确保管线停止后资源干净
            self.pipeline_mgr.clear_decoded_ring(camera_id).await;
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
                instance_id: i.instance_id.clone(),
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
            motion_gate_enabled: params
                .motion_gate
                .as_ref()
                .is_some_and(|config| config.enabled),
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
