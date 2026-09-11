use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, Semaphore};

use infer::package::AlgoRegistry;
use media::StreamHub;
use pipeline::PipelineManager;
use sea_orm::DatabaseConnection;

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
