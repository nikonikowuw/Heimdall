//! 事件录像调度服务
//!
//! 负责把三侧连接起来：
//!
//! ```text
//! Pipeline analysis_event_tx (Tokio broadcast)
//!         │  桥接任务
//!         ▼
//! RecordingWorker (专用 OS 线程, fMP4 写入)
//!         │  on_finished → bounded channel
//!         ▼
//! Tokio 落库任务 → RecordingRepo
//! ```
//!
//! 生命周期：`start_camera` / `stop_camera` 幂等；停机时统一闭合在途录像。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use db::PersistFinishedParams;
use pipeline::recording::{
    FinishedRecording, RecordingConfig, RecordingEventType, RecordingTrigger, RecordingWorker,
};
use pipeline::PipelineAnalysisEvent;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::state::AppState;

/// 落库通道容量：录像闭合是低频事件（每通道每分钟至多数次），
/// 容量 64 足以吸收瞬时突发；溢出时丢弃并计入日志（文件仍在磁盘上，可对账恢复）。
const PERSIST_CHANNEL_CAPACITY: usize = 64;

/// 单通道触发通道容量：告警/识别事件同样低频
const TRIGGER_CHANNEL_CAPACITY: usize = 128;

/// 单通道录像运行时句柄：Worker 与其外部触发发送端。
///
/// 两者同生命周期存放，避免两张表各自增删导致状态漂移；
/// 触发端是 `SyncSender` 的克隆，与内部桥接任务共用同一触发通道。
struct CameraRecordingHandle {
    worker: RecordingWorker,
    trigger_tx: std::sync::mpsc::SyncSender<RecordingTrigger>,
}

/// 事件录像调度服务
pub struct RecordingDispatchService {
    db: sea_orm::DatabaseConnection,
    pipeline: Arc<pipeline::PipelineManager>,
    stream_hub: Arc<media::StreamHub>,
    /// 录像根目录（实际文件位于 `{data_dir}/recordings/{camera_id}/...`）
    data_dir: PathBuf,
    shutdown_tx: broadcast::Sender<()>,
    /// 已启动的录像 Worker，按 camera_id 索引
    workers: Arc<RwLock<HashMap<String, CameraRecordingHandle>>>,
    /// 已完成录像的落库发送端
    persist_tx: mpsc::Sender<FinishedRecording>,
    persist_rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<FinishedRecording>>>>,
}

impl std::fmt::Debug for RecordingDispatchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingDispatchService")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl RecordingDispatchService {
    /// 显式构造（`AppState` 装配期间使用，此时 `AppState` 尚未成型）。
    pub fn new(
        db: sea_orm::DatabaseConnection,
        pipeline: Arc<pipeline::PipelineManager>,
        stream_hub: Arc<media::StreamHub>,
        shutdown_tx: broadcast::Sender<()>,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        let (persist_tx, persist_rx) = mpsc::channel(PERSIST_CHANNEL_CAPACITY);
        Self {
            db,
            pipeline,
            stream_hub,
            data_dir: data_dir.into(),
            shutdown_tx,
            workers: Arc::new(RwLock::new(HashMap::new())),
            persist_tx,
            persist_rx: Arc::new(tokio::sync::Mutex::new(Some(persist_rx))),
        }
    }

    /// 从 `AppState` 构造。
    pub fn from_state(state: &AppState) -> Self {
        Self::new(
            state.db.clone(),
            state.pipeline.clone(),
            state.stream_hub.clone(),
            state.shutdown_tx.clone(),
            state.recording_storage_root(),
        )
    }

    /// 录像根目录
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 启动某通道的录像 Worker（幂等：已在运行则先停旧实例）。
    ///
    /// `main_rtsp_url` 必须是主码流地址；录像消费者通过 `ConsumerKind::Recording`
    /// 独立订阅，与快照证据环互不干扰。
    pub async fn start_camera(
        self: &Arc<Self>,
        camera_id: &str,
        main_rtsp_url: &str,
        transport_policy: types::TransportPolicy,
        config: RecordingConfig,
    ) -> Result<(), String> {
        if !config.is_enabled() {
            self.stop_camera(camera_id).await;
            return Ok(());
        }

        // 幂等：先停旧实例（配置可能已变更）
        self.stop_camera(camera_id).await;

        let stream_key = format!("recording:{camera_id}");
        let subscription = self
            .stream_hub
            .subscribe_kind(
                &stream_key,
                main_rtsp_url,
                transport_policy,
                media::ConsumerKind::Recording,
            )
            .await
            .map_err(|error| format!("录像消费者订阅主码流失败: {error}"))?;

        // 触发通道：桥接任务（Tokio 侧）→ Worker（OS 线程）
        let (trigger_tx, trigger_rx) = std::sync::mpsc::sync_channel(TRIGGER_CHANNEL_CAPACITY);
        // 落库回调：在 Worker OS 线程内执行，只做非阻塞投递
        let persist_tx = self.persist_tx.clone();
        let camera_owned = camera_id.to_string();
        let on_finished = Box::new(move |rec: FinishedRecording| {
            if let Err(error) = persist_tx.try_send(rec) {
                tracing::error!(
                    camera = %camera_owned,
                    %error,
                    "录像落库通道拥塞，本次落库记录被丢弃（文件仍在磁盘，可由对账恢复）"
                );
            }
        });

        let worker = RecordingWorker::spawn(
            camera_id,
            subscription,
            trigger_rx,
            config.clone(),
            self.data_dir.clone(),
            on_finished,
        );

        if !worker.is_alive() {
            return Err("录像工作线程创建失败".to_string());
        }

        // 桥接 Pipeline 事件广播 → Worker 触发通道
        let mut event_rx = self.pipeline.subscribe_analysis_events();
        let bridge_camera = camera_id.to_string();
        let bridge_stop = self.shutdown_tx.subscribe();
        let worker_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let worker_alive_bridge = Arc::clone(&worker_alive);
        // 外部触发（识别命中路径）用的发送端：先克隆再移交给桥接任务
        let external_trigger_tx = trigger_tx.clone();

        tokio::spawn(async move {
            let mut shutdown_rx = bridge_stop;
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    recv = event_rx.recv() => {
                        match recv {
                            Ok(event) => {
                                let Some(trigger) = to_trigger(&bridge_camera, &event) else {
                                    continue;
                                };
                                // 目标通道已满或 Worker 已退出：丢弃本次触发，绝不反压事件广播
                                if trigger_tx.try_send(trigger).is_err() {
                                    tracing::warn!(
                                        camera = %bridge_camera,
                                        "录像触发通道不可用，丢弃本次触发"
                                    );
                                    worker_alive_bridge
                                        .store(false, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(
                                    camera = %bridge_camera,
                                    skipped,
                                    "录像事件桥接滞后，跳过部分历史事件（不补录）"
                                );
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            tracing::debug!(camera = %bridge_camera, "录像事件桥接任务退出");
        });

        self.workers.write().await.insert(
            camera_id.to_string(),
            CameraRecordingHandle {
                worker,
                // 外部触发（识别命中路径）持有独立克隆，与桥接任务互不干扰
                trigger_tx: external_trigger_tx,
            },
        );
        tracing::info!(camera = %camera_id, "录像 Worker 已启动");
        Ok(())
    }

    /// 识别命中触发录像：由识别对账（1:N 比对）落库成功后调用。
    ///
    /// 非阻塞投递：通道满或该通道未启用录像时直接丢弃本次触发，
    /// 绝不反压识别对账链路（录像缺失只影响证据完整性，不得影响识别结果落地）。
    pub async fn notify_recognition(
        &self,
        camera_id: &str,
        recognition_id: &str,
        event_time_ms: i64,
    ) {
        let trigger = RecordingTrigger {
            event_id: recognition_id.to_string(),
            event_type: RecordingEventType::Recognition,
            event_time_ms,
        };

        let workers = self.workers.read().await;
        let Some(handle) = workers.get(camera_id) else {
            // 该通道未启用录像，属正常路径
            return;
        };
        if handle.trigger_tx.try_send(trigger).is_err() {
            tracing::warn!(
                camera = %camera_id,
                recognition_id,
                "录像触发通道不可用，丢弃本次识别触发"
            );
        }
    }

    /// 停止某通道录像（幂等）。闭合在途文件后线程退出。
    pub async fn stop_camera(&self, camera_id: &str) {
        let handle = self.workers.write().await.remove(camera_id);
        if let Some(handle) = handle {
            handle.worker.request_stop();
            // Worker 的 Drop 会交给看护线程有界回收，此处不阻塞等待
            drop(handle);
            tracing::info!(camera = %camera_id, "录像 Worker 已请求停机");
        }
    }

    /// 停止全部录像 Worker
    pub async fn stop_all(&self) {
        let mut workers = self.workers.write().await;
        for (camera_id, handle) in workers.drain() {
            handle.worker.request_stop();
            tracing::info!(camera = %camera_id, "录像 Worker 已请求停机");
        }
    }

    /// 当前活跃录像 Worker 数量
    pub async fn active_worker_count(&self) -> usize {
        self.workers.read().await.len()
    }

    /// 冷启动恢复：枚举数据库中启用录像的通道并启动 Worker。
    ///
    /// 单通道失败仅告警，不阻断其余通道与服务启动（与任务恢复同一容错口径）。
    pub async fn recover_enabled_cameras(self: &Arc<Self>) -> (usize, usize) {
        let cameras = match db::CameraRepo::list_all(&self.db).await {
            Ok(list) => list,
            Err(error) => {
                tracing::error!(%error, "读取摄像头列表失败，录像冷启动恢复跳过");
                return (0, 0);
            }
        };

        let mut started = 0usize;
        let mut failed = 0usize;
        for camera in cameras {
            let Some(config) = parse_camera_recording_config(&camera.recording_config) else {
                continue;
            };
            if !config.is_enabled() {
                continue;
            }

            match self
                .start_camera(
                    &camera.camera_id,
                    &camera.rtsp_url,
                    types::TransportPolicy::Auto,
                    config,
                )
                .await
            {
                Ok(()) => started += 1,
                Err(error) => {
                    failed += 1;
                    tracing::error!(
                        camera = %camera.camera_id,
                        %error,
                        "录像 Worker 冷启动恢复失败，已跳过本通道"
                    );
                }
            }
        }

        if started > 0 || failed > 0 {
            tracing::info!(started, failed, "录像冷启动恢复完成");
        }
        (started, failed)
    }

    /// 启动落库任务（Tokio 侧）。应在 App 启动时调用一次。
    pub fn start_persist_worker(self: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let rx = self
            .persist_rx
            .try_lock()
            .ok()
            .and_then(|mut guard| guard.take())?;
        let db = self.db.clone();
        let storage_root = self.data_dir.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        Some(tokio::spawn(async move {
            let mut rx = rx;
            tracing::info!("录像落库工作线程已启动");
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        // 停机前排空剩余记录
                        while let Ok(rec) = rx.try_recv() {
                            persist_one(&db, &storage_root, rec).await;
                        }
                        break;
                    }
                    recv = rx.recv() => {
                        let Some(rec) = recv else { break };
                        persist_one(&db, &storage_root, rec).await;
                    }
                }
            }
            tracing::info!("录像落库工作线程已退出");
        }))
    }
}

/// 解析摄像头实体中的录像配置文本；空串或非法 JSON 视为未配置。
pub(crate) fn parse_camera_recording_config(
    raw: &str,
) -> Option<pipeline::recording::RecordingConfig> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<types::CameraRecordingConfig>(trimmed)
        .map(|config| config.normalized())
        .map_err(|error| {
            tracing::warn!(raw = %trimmed, %error, "摄像头录像配置解析失败，按未配置处理");
            error
        })
        .ok()?;

    Some(pipeline::recording::RecordingConfig {
        mode: if parsed.enabled {
            pipeline::recording::RecordingMode::EventOnly
        } else {
            pipeline::recording::RecordingMode::Disabled
        },
        pre_capture_seconds: parsed.pre_capture_seconds,
        post_capture_seconds: parsed.post_capture_seconds,
        max_file_seconds: parsed.max_file_seconds,
        retention_days: parsed.retention_days,
    })
}

/// 转换 Pipeline 事件为录像触发信号。
///
/// 这里只接规则告警：抓拍是「客观通行事实」，每次轨迹结算都会产生，
/// 若以其为触发源，录像会被无差别写满，绝大多数片段没有检索价值。
/// 识别命中的触发由 `CaptureDispatchService` 在 1:N 比对落库成功后单独投递
/// （见 `notify_recognition`），使录像只覆盖告警与有识别结果的记录。
fn to_trigger(camera_id: &str, event: &PipelineAnalysisEvent) -> Option<RecordingTrigger> {
    match event {
        PipelineAnalysisEvent::Alarm(alarm) => {
            if alarm.camera_id != camera_id {
                return None;
            }
            Some(RecordingTrigger {
                event_id: alarm.event_id.clone(),
                event_type: RecordingEventType::Alarm,
                event_time_ms: alarm.timestamp,
            })
        }
        PipelineAnalysisEvent::Capture(_)
        | PipelineAnalysisEvent::Tracks(_)
        | PipelineAnalysisEvent::Telemetry(_) => None,
    }
}

/// 单条录像落库（失败仅告警，不回滚已写文件）
async fn persist_one(
    db: &sea_orm::DatabaseConnection,
    storage_root: &Path,
    rec: FinishedRecording,
) {
    // DB 统一存相对存储根的路径，便于存储根迁移后仍可解析
    let relative_path = rec
        .file_path
        .strip_prefix(storage_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| rec.file_path.to_string_lossy().into_owned());

    let params = PersistFinishedParams {
        recording_id: rec.recording_id.clone(),
        camera_id: rec.camera_id.clone(),
        file_path: relative_path,
        start_time_ms: rec.start_time_ms,
        end_time_ms: rec.end_time_ms,
        file_size: rec.file_size,
        codec: rec.codec.as_str().to_string(),
        status: rec.status.to_string(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        events: rec
            .events
            .iter()
            .map(|e| db::PersistEventParams {
                event_type: e.event_type.as_str().to_string(),
                event_id: e.event_id.clone(),
                event_time_ms: e.event_time_ms,
                offset_ms: e.offset_ms,
            })
            .collect(),
    };

    match db::RecordingRepo::persist_finished(db, params).await {
        Ok(saved) => tracing::info!(
            recording_id = %saved.recording_id,
            camera = %saved.camera_id,
            status = %saved.status,
            "录像记录已落库"
        ),
        Err(error) => tracing::error!(
            recording_id = %rec.recording_id,
            camera = %rec.camera_id,
            %error,
            "录像记录落库失败（物理文件保留，可由对账恢复）"
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_to_trigger_filters_by_camera() {
        use pipeline::{EvidenceStatus, PipelineAnalysisEvent};

        fn tracked() -> types::TrackedObject {
            types::TrackedObject {
                track_id: 1,
                class_id: 0,
                label: "person".into(),
                confidence: 0.9,
                quality_score: None,
                bbox: types::BoundingBox::new(0.1, 0.1, 0.2, 0.2),
                face: None,
                embedding: None,
                trajectory: vec![],
            }
        }

        let alarm = pipeline::PipelineAlarmEvent {
            event_id: "evt_a".into(),
            camera_id: "cam_1".into(),
            algorithm_id: "algo".into(),
            alarm: pipeline::TriggeredAlarm {
                rule_index: 0,
                role: types::DetectionRuleRole::Roi,
                tracked_object: tracked(),
                occurred_at_ms: 1000,
            },
            snapshot: None,
            evidence_status: EvidenceStatus::Failed,
            evidence_error: None,
            timestamp: 1000,
        };
        let event = PipelineAnalysisEvent::Alarm(Box::new(alarm));

        // 匹配通道
        let trigger = to_trigger("cam_1", &event).expect("should match");
        assert_eq!(trigger.event_id, "evt_a");
        assert_eq!(trigger.event_type, RecordingEventType::Alarm);
        assert_eq!(trigger.event_time_ms, 1000);

        // 不匹配通道
        assert!(to_trigger("cam_other", &event).is_none());
    }

    /// 抓拍是客观通行事实（每次轨迹结算都产生），不得作为录像触发源；
    /// 录像只应由告警与识别命中驱动。
    #[test]
    fn test_to_trigger_ignores_capture() {
        use pipeline::{PipelineAnalysisEvent, PipelineCaptureEvent};

        let capture = PipelineCaptureEvent {
            capture_id: "cap_1".into(),
            camera_id: "cam_1".into(),
            algorithm_id: "algo".into(),
            tracked_object: types::TrackedObject {
                track_id: 1,
                class_id: 0,
                label: "person".into(),
                confidence: 0.9,
                quality_score: None,
                bbox: types::BoundingBox::new(0.1, 0.1, 0.2, 0.2),
                face: None,
                embedding: None,
                trajectory: vec![],
            },
            snapshot: None,
            timestamp: 2000,
        };
        let event = PipelineAnalysisEvent::Capture(Box::new(capture));

        // 无论通道是否匹配，抓拍都不再触发录像
        assert!(to_trigger("cam_1", &event).is_none());
        assert!(to_trigger("cam_other", &event).is_none());
    }

    #[test]
    fn test_to_trigger_ignores_tracks() {
        let event = PipelineAnalysisEvent::Tracks(pipeline::PipelineTrackEvent {
            camera_id: "cam_1".into(),
            algorithm_id: "algo".into(),
            timestamp: 1000,
            tracks: vec![],
        });
        assert!(to_trigger("cam_1", &event).is_none());
    }
}
