use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc, oneshot, Semaphore};

use infer::package::AlgoRegistry;
use media::StreamHub;
use pipeline::PipelineManager;
use sea_orm::DatabaseConnection;

/// 快照配置 actor 的固定请求队列。
const SNAPSHOT_CONFIG_QUEUE_CAPACITY: usize = 16;

#[derive(Clone, Debug)]
pub struct SnapshotConfigService {
    tx: mpsc::Sender<SnapshotConfigCommand>,
}

enum SnapshotConfigCommand {
    Get {
        reply: oneshot::Sender<Result<types::SnapshotSystemConfig, String>>,
    },
    Update {
        config: types::SnapshotSystemConfig,
        reply: oneshot::Sender<Result<types::SnapshotSystemConfig, String>>,
    },
    Initialize {
        reply: oneshot::Sender<Result<types::SnapshotSystemConfig, String>>,
    },
}

impl SnapshotConfigService {
    pub fn new(db: DatabaseConnection, pipeline: Arc<PipelineManager>) -> Self {
        let (tx, mut rx) = mpsc::channel(SNAPSHOT_CONFIG_QUEUE_CAPACITY);
        tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                match command {
                    SnapshotConfigCommand::Get { reply } => {
                        let result = load_snapshot_config(&db, &pipeline).await;
                        let _ = reply.send(result);
                    }
                    SnapshotConfigCommand::Update { config, reply } => {
                        let result = persist_snapshot_config(&db, &pipeline, config).await;
                        let _ = reply.send(result);
                    }
                    SnapshotConfigCommand::Initialize { reply } => {
                        let result = initialize_snapshot_config(&db, &pipeline).await;
                        let _ = reply.send(result);
                    }
                }
            }
        });
        Self { tx }
    }

    pub async fn initialize(&self) -> Result<types::SnapshotSystemConfig, String> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(SnapshotConfigCommand::Initialize { reply })
            .await
            .map_err(|_| "快照配置 actor 已退出".to_string())?;
        result
            .await
            .map_err(|_| "快照配置 actor 未返回结果".to_string())?
    }

    pub async fn get(&self) -> Result<types::SnapshotSystemConfig, String> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(SnapshotConfigCommand::Get { reply })
            .await
            .map_err(|_| "快照配置 actor 已退出".to_string())?;
        result
            .await
            .map_err(|_| "快照配置 actor 未返回结果".to_string())?
    }

    pub async fn update(
        &self,
        config: types::SnapshotSystemConfig,
    ) -> Result<types::SnapshotSystemConfig, String> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(SnapshotConfigCommand::Update { config, reply })
            .await
            .map_err(|_| "快照配置 actor 已退出".to_string())?;
        result
            .await
            .map_err(|_| "快照配置 actor 未返回结果".to_string())?
    }
}

fn dto_from_snapshot_config(config: &pipeline::SnapshotConfig) -> types::SnapshotSystemConfig {
    types::SnapshotSystemConfig {
        main_stream_panoramic_quality: config.main_stream_panoramic_quality,
        main_stream_crop_quality: config.main_stream_crop_quality,
        sub_stream_panoramic_quality: config.sub_stream_panoramic_quality,
        sub_stream_crop_quality: config.sub_stream_crop_quality,
        crop_padding_ratio: config.crop_padding_ratio,
    }
}

fn apply_snapshot_dto(
    current: &pipeline::SnapshotConfig,
    dto: &types::SnapshotSystemConfig,
) -> pipeline::SnapshotConfig {
    let mut config = current.clone();
    config.main_stream_panoramic_quality = dto.main_stream_panoramic_quality;
    config.main_stream_crop_quality = dto.main_stream_crop_quality;
    config.sub_stream_panoramic_quality = dto.sub_stream_panoramic_quality;
    config.sub_stream_crop_quality = dto.sub_stream_crop_quality;
    config.crop_padding_ratio = dto.crop_padding_ratio;
    config
}

async fn load_snapshot_config(
    db: &DatabaseConnection,
    pipeline: &Arc<PipelineManager>,
) -> Result<types::SnapshotSystemConfig, String> {
    let config = match db::SystemConfigRepo::get(db, "snapshot_config")
        .await
        .map_err(|error| format!("读取快照配置失败: {error}"))?
    {
        Some(json) => serde_json::from_str::<types::SnapshotSystemConfig>(&json)
            .map_err(|error| format!("持久化快照配置 JSON 非法: {error}"))?,
        None => dto_from_snapshot_config(&pipeline.snapshot_config()),
    };
    config
        .validate()
        .map_err(|error| format!("持久化快照配置非法: {error}"))?;
    Ok(config)
}

async fn initialize_snapshot_config(
    db: &DatabaseConnection,
    pipeline: &Arc<PipelineManager>,
) -> Result<types::SnapshotSystemConfig, String> {
    let config = load_snapshot_config(db, pipeline).await?;
    let runtime = apply_snapshot_dto(&pipeline.snapshot_config(), &config);
    pipeline
        .update_snapshot_config(runtime)
        .map_err(|error| format!("恢复运行时快照配置失败: {error}"))?;
    Ok(config)
}

async fn persist_snapshot_config(
    db: &DatabaseConnection,
    pipeline: &Arc<PipelineManager>,
    config: types::SnapshotSystemConfig,
) -> Result<types::SnapshotSystemConfig, String> {
    config
        .validate()
        .map_err(|error| format!("快照配置非法: {error}"))?;
    let json =
        serde_json::to_string(&config).map_err(|error| format!("序列化快照配置失败: {error}"))?;
    let runtime = apply_snapshot_dto(&pipeline.snapshot_config(), &config);

    // 严格遵循先持久化后生效准则：数据库写入成功后再更新运行时管线
    db::SystemConfigRepo::set(db, "snapshot_config", &json)
        .await
        .map_err(|error| format!("保存快照配置失败: {error}"))?;

    pipeline
        .update_snapshot_config(runtime)
        .map_err(|error| format!("更新运行时快照配置失败: {error}"))?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_snapshot_config_persistence_rolls_back_runtime_on_db_failure() {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = Arc::new(PipelineManager::new());
        let previous = dto_from_snapshot_config(&pipeline.snapshot_config());
        let mut next = previous.clone();
        next.main_stream_panoramic_quality = 81;

        db.clone().close().await.expect("关闭测试数据库失败");
        let result = persist_snapshot_config(&db, &pipeline, next).await;

        assert!(result.is_err());
        assert_eq!(
            dto_from_snapshot_config(&pipeline.snapshot_config()),
            previous
        );
    }
}

/// WebSocket 广播事件模型
#[derive(Debug, Clone, serde::Serialize)]
pub struct WsBroadcastEvent {
    pub topic: String,
    pub payload: serde_json::Value,
    pub timestamp: i64,
}

/// 算法包上传默认最大上限 (1024MB 即 1GB)
pub const DEFAULT_MAX_PACKAGE_SIZE_BYTES: usize = 1024 * 1024 * 1024;
/// 算法包沙箱处理固定为单路并发，避免多个大包同时占满内存与 CPU。
pub const DEFAULT_MAX_CONCURRENT_ALGORITHM_UPLOADS: usize = 1;

/// Axum 共享应用状态句柄
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub snapshot_config: SnapshotConfigService,
    pub stream_hub: Arc<StreamHub>,
    pub algo_registry: Arc<AlgoRegistry>,
    pub task_coordinator: Arc<dyn pipeline::TaskRuntimeService>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub jwt_secret: Arc<RwLock<Vec<u8>>>,
    pub token_invalid_before: Arc<AtomicI64>,
    pub is_initialized: Arc<AtomicBool>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub max_upload_size_bytes: usize,
    pub algorithm_upload_semaphore: Arc<Semaphore>,
    pub storage_cleaner: Option<Arc<pipeline::storage_cleaner::StorageCleaner>>,
    pub gallery_index: Arc<crate::gallery_index::FaceFeatureIndex>,
}

impl AppState {
    pub fn new(db: DatabaseConnection, pipeline: Arc<PipelineManager>) -> Self {
        Self::new_with_limit(db, pipeline, DEFAULT_MAX_PACKAGE_SIZE_BYTES)
    }

    pub fn new_with_limit(
        db: DatabaseConnection,
        pipeline: Arc<PipelineManager>,
        max_upload_size_bytes: usize,
    ) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1024);
        let (shutdown_tx, _) = broadcast::channel(16);
        let stream_hub = Arc::new(StreamHub::new());
        let algo_registry = Arc::new(AlgoRegistry::new());
        let gallery_index = Arc::new(crate::gallery_index::FaceFeatureIndex::new());
        let snapshot_config = SnapshotConfigService::new(db.clone(), pipeline.clone());
        let task_coordinator: Arc<dyn pipeline::TaskRuntimeService> =
            Arc::new(pipeline::TaskRuntimeCoordinator::new(
                pipeline.clone(),
                stream_hub.clone(),
                algo_registry.clone(),
            ));
        let jwt_secret = match std::env::var("ARGUS_JWT_SECRET") {
            Ok(secret) if !secret.trim().is_empty() => secret.into_bytes(),
            _ => {
                let uuid1 = uuid::Uuid::new_v4();
                let uuid2 = uuid::Uuid::new_v4();
                let mut bytes = Vec::with_capacity(32);
                bytes.extend_from_slice(uuid1.as_bytes());
                bytes.extend_from_slice(uuid2.as_bytes());
                bytes
            }
        };

        Self {
            db,
            pipeline,
            snapshot_config,
            stream_hub,
            algo_registry,
            task_coordinator,
            event_broadcaster,
            jwt_secret: Arc::new(RwLock::new(jwt_secret)),
            token_invalid_before: Arc::new(AtomicI64::new(0)),
            is_initialized: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
            max_upload_size_bytes,
            algorithm_upload_semaphore: Arc::new(Semaphore::new(
                DEFAULT_MAX_CONCURRENT_ALGORITHM_UPLOADS,
            )),
            storage_cleaner: None,
            gallery_index,
        }
    }

    /// 为 API 状态注入自定义的分析任务运行时服务 (主要用于测试隔离与 Mock)
    pub fn with_task_coordinator(
        mut self,
        coordinator: Arc<dyn pipeline::TaskRuntimeService>,
    ) -> Self {
        self.task_coordinator = coordinator;
        self
    }

    /// 为 API 状态装配使用指定 evidence 目录的存储清理器。
    pub fn with_storage_cleaner(mut self, evidence_dir: impl Into<std::path::PathBuf>) -> Self {
        self.storage_cleaner = Some(Arc::new(pipeline::storage_cleaner::StorageCleaner::new(
            pipeline::storage_cleaner::StorageCleanerConfig {
                evidence_dir: evidence_dir.into(),
                ..Default::default()
            },
        )));
        self
    }

    /// 触发全局服务停机广播通知
    pub fn notify_shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// 获取当前的 JWT 签名密钥
    pub fn get_jwt_secret(&self) -> Vec<u8> {
        self.jwt_secret
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}
