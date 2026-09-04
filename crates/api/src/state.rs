use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

use pipeline::PipelineManager;
use sea_orm::DatabaseConnection;

/// WebSocket 广播事件模型
#[derive(Debug, Clone, serde::Serialize)]
pub struct WsBroadcastEvent {
    pub topic: String,
    pub payload: serde_json::Value,
    pub timestamp: i64,
}

/// Axum 共享应用状态句柄
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub jwt_secret: Arc<RwLock<Vec<u8>>>,
    pub token_invalid_before: Arc<AtomicI64>,
    pub is_initialized: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(db: DatabaseConnection, pipeline: Arc<PipelineManager>) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1024);
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
            event_broadcaster,
            jwt_secret: Arc::new(RwLock::new(jwt_secret)),
            token_invalid_before: Arc::new(AtomicI64::new(0)),
            is_initialized: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 获取当前的 JWT 签名密钥
    pub fn get_jwt_secret(&self) -> Vec<u8> {
        self.jwt_secret
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// 启动时从数据库同步初始化状态、失效时间戳以及持久化 JWT Secret
    pub async fn sync_auth_state(&self) {
        // 同步并持久化 JWT Secret（如果未通过环境变量注入）
        if std::env::var("ARGUS_JWT_SECRET")
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            if let Ok(persisted_secret) =
                db::SystemConfigRepo::get_or_set_with(&self.db, "jwt_secret", || {
                    let u1 = uuid::Uuid::new_v4();
                    let u2 = uuid::Uuid::new_v4();
                    format!("{u1}{u2}")
                })
                .await
            {
                if let Ok(mut guard) = self.jwt_secret.write() {
                    *guard = persisted_secret.into_bytes();
                }
            }
        }

        if let Ok(count) = db::AdminUserRepo::count(&self.db).await {
            let initialized = count > 0;
            self.is_initialized.store(initialized, Ordering::Relaxed);
            if initialized {
                if let Ok(Some(first_admin)) = db::AdminUserRepo::get_first_admin(&self.db).await {
                    self.token_invalid_before
                        .store(first_admin.token_invalid_before, Ordering::Relaxed);
                }
            }
        }
    }
}
