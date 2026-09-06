use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
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
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub jwt_secret: Arc<RwLock<Vec<u8>>>,
    pub token_invalid_before: Arc<AtomicI64>,
    pub is_initialized: Arc<AtomicBool>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub max_upload_size_bytes: usize,
    pub algorithm_upload_semaphore: Arc<Semaphore>,
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
            event_broadcaster,
            jwt_secret: Arc::new(RwLock::new(jwt_secret)),
            token_invalid_before: Arc::new(AtomicI64::new(0)),
            is_initialized: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
            max_upload_size_bytes,
            algorithm_upload_semaphore: Arc::new(Semaphore::new(
                DEFAULT_MAX_CONCURRENT_ALGORITHM_UPLOADS,
            )),
        }
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
        tracing::info!(
            camera_id = %camera_id,
            status = %params.status,
            codec = %params.codec,
            width = params.width,
            height = params.height,
            fps = params.fps,
            error_code = %params.error_code,
            "更新摄像头探活状态入库并向全网广播 WebSocket 事件"
        );
        let _ = db::CameraRepo::update_probe_status(&self.db, camera_id, params.clone()).await;

        let _ = self.event_broadcaster.send(WsBroadcastEvent {
            topic: types::TOPIC_CAMERA_PROBE_UPDATED.to_string(),
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

    /// 对单台摄像头执行双轨巡检（活跃看门狗优先，待机流轻量探活）
    pub async fn probe_single_camera(&self, cam: db::entity::camera::Model) {
        // 检查主码流、子码流、基础通道或其绑定的底层物理 RTSP 流是否正在健康接收视频帧（4秒内有数据包）
        let main_key = types::StreamKey::main(&cam.camera_id).as_str_key();
        let sub_key = types::StreamKey::sub(&cam.camera_id).as_str_key();
        let is_healthy_main = self.stream_hub.is_healthy_streaming(&main_key, 4000).await;
        let is_healthy_sub = self.stream_hub.is_healthy_streaming(&sub_key, 4000).await;
        let is_healthy_bare = self
            .stream_hub
            .is_healthy_streaming(&cam.camera_id, 4000)
            .await;
        let is_healthy_url = self
            .stream_hub
            .is_healthy_streaming(&cam.rtsp_url, 4000)
            .await;

        if is_healthy_main || is_healthy_sub || is_healthy_bare || is_healthy_url {
            // 活跃拉流且持续有帧流入，看门狗确认为 healthy
            self.stream_hub.reset_failure_count(&cam.camera_id).await;
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
            // 未收到流或待机流：发起轻量 TCP/RTSP 探活
            match media::StreamProber::probe(&cam.rtsp_url, std::time::Duration::from_secs(3)).await
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
                            codec: "",
                            width: 0,
                            height: 0,
                            fps: 0.0,
                            error_code: &err_code,
                        },
                    )
                    .await;
                }
            }
        }
    }

    /// 启动后台摄像头双轨健康巡检任务（带受限并发控制、三态防抖与 WebSocket 状态广播）
    pub fn start_periodic_probe_worker(self: Arc<Self>, interval: std::time::Duration) {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            tracing::info!(
                interval_secs = interval.as_secs(),
                "摄像头后台定时巡检任务已启动 (6 路受限并发)"
            );
            loop {
                if let Ok(cameras) = db::CameraRepo::list_all(&self.db).await {
                    if !cameras.is_empty() {
                        let semaphore = Arc::new(tokio::sync::Semaphore::new(6));
                        let mut join_set = tokio::task::JoinSet::new();

                        for cam in cameras {
                            let sem = semaphore.clone();
                            let state = self.clone();
                            join_set.spawn(async move {
                                let _permit = sem.acquire().await;
                                state.probe_single_camera(cam).await;
                            });
                        }

                        while let Some(res) = join_set.join_next().await {
                            if shutdown_rx.try_recv().is_ok() {
                                tracing::info!("后台巡检任务收到停机信号，中止当前批次并退出");
                                join_set.abort_all();
                                return;
                            }
                            if let Err(e) = res {
                                tracing::debug!("巡检子任务执行中断: {:?}", e);
                            }
                        }
                    }
                }
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("后台巡检任务收到停机信号，安全退出");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {}
                }
            }
        });
    }
}
