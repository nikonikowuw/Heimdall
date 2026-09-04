use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

use media::StreamHub;
use pipeline::PipelineManager;
use sea_orm::DatabaseConnection;

/// WHEP 在线会话上下文
#[derive(Clone, Debug)]
pub struct WhepSessionContext {
    pub camera_id: String,
    pub peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    pub closed: Arc<AtomicBool>,
}

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
    pub stream_hub: Arc<StreamHub>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub jwt_secret: Arc<RwLock<Vec<u8>>>,
    pub token_invalid_before: Arc<AtomicI64>,
    pub is_initialized: Arc<AtomicBool>,
    pub whep_sessions: Arc<tokio::sync::RwLock<HashMap<String, WhepSessionContext>>>,
}

impl AppState {
    pub fn new(db: DatabaseConnection, pipeline: Arc<PipelineManager>) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1024);
        let stream_hub = Arc::new(StreamHub::new());
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
            event_broadcaster,
            jwt_secret: Arc::new(RwLock::new(jwt_secret)),
            token_invalid_before: Arc::new(AtomicI64::new(0)),
            is_initialized: Arc::new(AtomicBool::new(false)),
            whep_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
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

    /// 更新探活状态入库并向全网广播 WebSocket 状态事件
    pub async fn update_and_broadcast_probe(
        &self,
        camera_id: &str,
        params: db::ProbeUpdateParams<'_>,
    ) {
        let _ = db::CameraRepo::update_probe_status(&self.db, camera_id, params.clone()).await;

        let _ = self.event_broadcaster.send(WsBroadcastEvent {
            topic: "camera.probe_updated".to_string(),
            payload: serde_json::json!({
                "cameraId": camera_id,
                "status": params.status,
                "codec": params.codec,
                "width": params.width,
                "height": params.height,
                "fps": params.fps,
                "errorCode": params.error_code
            }),
            timestamp: chrono::Utc::now().timestamp_millis(),
        });
    }

    /// 启动后台摄像头双轨健康巡检任务（带三态防抖与 WebSocket 状态广播）
    pub fn start_periodic_probe_worker(self: Arc<Self>, interval: std::time::Duration) {
        tokio::spawn(async move {
            tracing::info!(
                interval_secs = interval.as_secs(),
                "摄像头后台定时巡检任务已启动"
            );
            loop {
                tokio::time::sleep(interval).await;
                if let Ok(cameras) = db::CameraRepo::list_all(&self.db).await {
                    for cam in cameras {
                        let is_active = self.stream_hub.is_streaming(&cam.camera_id).await;

                        if is_active {
                            // 活跃拉流中，看门狗实时生效，直接标记为 healthy
                            self.update_and_broadcast_probe(
                                &cam.camera_id,
                                db::ProbeUpdateParams {
                                    status: "healthy",
                                    codec: &cam.last_codec,
                                    width: cam.last_width,
                                    height: cam.last_height,
                                    fps: cam.last_fps,
                                    error_code: "",
                                },
                            )
                            .await;
                        } else {
                            // 待机流：发起轻量探活
                            match media::StreamProber::probe(
                                &cam.rtsp_url,
                                std::time::Duration::from_secs(3),
                            )
                            .await
                            {
                                Ok(info) => {
                                    self.stream_hub.reset_failure_count(&cam.camera_id).await;
                                    self.update_and_broadcast_probe(
                                        &cam.camera_id,
                                        db::ProbeUpdateParams {
                                            status: "healthy",
                                            codec: &info.codec,
                                            width: info.width as i32,
                                            height: info.height as i32,
                                            fps: info.fps,
                                            error_code: "",
                                        },
                                    )
                                    .await;
                                }
                                Err(err) => {
                                    let failures = self
                                        .stream_hub
                                        .increment_failure_count(&cam.camera_id)
                                        .await;
                                    let (status_str, err_code) = if failures < 3 {
                                        ("degraded", format!("RETRYING_{failures}"))
                                    } else {
                                        ("failed", format!("PROBE_FAILED: {err}"))
                                    };

                                    self.update_and_broadcast_probe(
                                        &cam.camera_id,
                                        db::ProbeUpdateParams {
                                            status: status_str,
                                            codec: &cam.last_codec,
                                            width: cam.last_width,
                                            height: cam.last_height,
                                            fps: cam.last_fps,
                                            error_code: &err_code,
                                        },
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}
