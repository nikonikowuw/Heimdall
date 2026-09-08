use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::state::WsBroadcastEvent;

/// 摄像头后台探活与状态广播服务
///
/// 从 `AppState` 中提取的独立服务，负责：
/// - 单路摄像头双轨巡检（活跃看门狗优先，待机流轻量探活）
/// - 探活结果入库与 WebSocket 事件广播
/// - 受限并发的周期性后台巡检调度
#[derive(Debug)]
pub struct CameraProbeService {
    pub db: sea_orm::DatabaseConnection,
    pub pipeline: Arc<pipeline::PipelineManager>,
    pub stream_hub: Arc<media::StreamHub>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub shutdown_tx: broadcast::Sender<()>,
}

impl Clone for CameraProbeService {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            pipeline: self.pipeline.clone(),
            stream_hub: self.stream_hub.clone(),
            event_broadcaster: self.event_broadcaster.clone(),
            shutdown_tx: self.shutdown_tx.clone(),
        }
    }
}

impl CameraProbeService {
    /// 从 `AppState` 中提取探活所需的共享句柄，构造常驻后台巡检服务实例
    ///
    /// 推荐在应用启动（如 `app::main`）时构造一次并调用 `start_periodic_probe_worker`。
    pub fn from_state(state: &crate::state::AppState) -> Self {
        Self {
            db: state.db.clone(),
            pipeline: state.pipeline.clone(),
            stream_hub: state.stream_hub.clone(),
            event_broadcaster: state.event_broadcaster.clone(),
            shutdown_tx: state.shutdown_tx.clone(),
        }
    }

    /// 静态方法：更新探活状态入库并向全网广播 WebSocket 状态事件
    pub async fn broadcast_probe_update(
        db: &sea_orm::DatabaseConnection,
        broadcaster: &broadcast::Sender<WsBroadcastEvent>,
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
        let _ = db::CameraRepo::update_probe_status(db, camera_id, params.clone()).await;

        let _ = broadcaster.send(WsBroadcastEvent {
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

    /// 静态方法：异步触发一次轻量 RTSP 探活并在完成后向全网广播更新
    pub fn spawn_probe_and_broadcast(
        db: sea_orm::DatabaseConnection,
        broadcaster: broadcast::Sender<WsBroadcastEvent>,
        camera_id: String,
        rtsp_url: String,
    ) {
        tokio::spawn(async move {
            tracing::info!(camera_id = %camera_id, rtsp_url = %media::mask_rtsp_url(&rtsp_url), "开始对摄像头执行异步探活...");
            match media::StreamProber::probe(&rtsp_url, Duration::from_secs(5)).await {
                Ok(info) => {
                    tracing::info!(
                        camera_id = %camera_id,
                        codec = %info.codec,
                        width = info.width,
                        height = info.height,
                        fps = info.fps,
                        "摄像头异步探活成功 -> 标记为 healthy"
                    );
                    Self::broadcast_probe_update(
                        &db,
                        &broadcaster,
                        &camera_id,
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
                Err(e) => {
                    let err_str = e.to_string();
                    tracing::warn!(
                        camera_id = %camera_id,
                        error = %err_str,
                        "摄像头异步探活失败 -> 标记为 failed"
                    );
                    Self::broadcast_probe_update(
                        &db,
                        &broadcaster,
                        &camera_id,
                        db::ProbeUpdateParams {
                            status: "failed",
                            codec: "",
                            width: 0,
                            height: 0,
                            fps: 0.0,
                            error_code: &err_str,
                        },
                    )
                    .await;
                }
            }
        });
    }

    /// 更新探活状态入库并向全网广播 WebSocket 状态事件
    pub async fn update_and_broadcast_probe(
        &self,
        camera_id: &str,
        params: db::ProbeUpdateParams<'_>,
    ) {
        Self::broadcast_probe_update(&self.db, &self.event_broadcaster, camera_id, params).await;
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
            match media::StreamProber::probe(&cam.rtsp_url, Duration::from_secs(3)).await {
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
    pub fn start_periodic_probe_worker(self: Arc<Self>, interval: Duration) {
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
                            let svc = self.clone();
                            join_set.spawn(async move {
                                let _permit = sem.acquire().await;
                                svc.probe_single_camera(cam).await;
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
