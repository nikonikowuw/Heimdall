use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

use db::{CaptureRepo, DbError};
use pipeline::{PipelineAnalysisEvent, PipelineCaptureEvent, PipelineManager};
use sea_orm::Set;

use crate::state::AppState;

/// 默认抓拍攒批写入阈值 (每批最多 32 条)
pub const DEFAULT_CAPTURE_BATCH_SIZE: usize = 32;

/// 默认抓拍攒批最大刷新等待时间 (毫秒)
pub const DEFAULT_CAPTURE_FLUSH_INTERVAL_MS: u64 = 500;

/// 客观通行抓拍凭据异步持久化服务
///
/// 核心职责：
/// 1. 订阅管线分析引擎发出的客观通行抓拍事件 (`PipelineCaptureEvent`)；
/// 2. 独立管理行迹抓拍三支柱凭证持久化至 `capture_records`，严格与安全告警 (`alarm_records`) 隔离；
/// 3. 采用有界通道攒批提交 (32 条或 500ms 超时)，避免高频通行事件逐条开启单行事务刷盘；
/// 4. 在冷启动与通道滞后 (`Lagged`) 时通过管线内存补偿队列 (`drain_pending_capture_events`) 无损恢复。
#[derive(Debug, Clone)]
pub struct CaptureDispatchService {
    pub db: sea_orm::DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub batch_size: usize,
    pub flush_interval_ms: u64,
    pub gallery_index: Option<Arc<crate::gallery_index::FaceFeatureIndex>>,
    pub algo_registry: Option<Arc<infer::package::AlgoRegistry>>,
    pub event_broadcaster: Option<broadcast::Sender<crate::state::WsBroadcastEvent>>,
    recognition_lock: Arc<tokio::sync::Mutex<()>>,
}

impl CaptureDispatchService {
    /// 从 `AppState` 中提取句柄构造默认抓拍分发服务实例
    pub fn from_state(state: &AppState) -> Self {
        Self {
            db: state.db.clone(),
            pipeline: state.pipeline.clone(),
            shutdown_tx: state.shutdown_tx.clone(),
            batch_size: DEFAULT_CAPTURE_BATCH_SIZE,
            flush_interval_ms: DEFAULT_CAPTURE_FLUSH_INTERVAL_MS,
            gallery_index: Some(state.gallery_index.clone()),
            algo_registry: Some(state.algo_registry.clone()),
            event_broadcaster: Some(state.event_broadcaster.clone()),
            recognition_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// 指定攒批参数构造服务实例
    pub fn with_options(
        db: sea_orm::DatabaseConnection,
        pipeline: Arc<PipelineManager>,
        shutdown_tx: broadcast::Sender<()>,
        batch_size: usize,
        flush_interval_ms: u64,
    ) -> Self {
        Self {
            db,
            pipeline,
            shutdown_tx,
            batch_size: batch_size.max(1),
            flush_interval_ms: flush_interval_ms.max(10),
            gallery_index: None,
            algo_registry: None,
            event_broadcaster: None,
            recognition_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// 将单个 `PipelineCaptureEvent` 转换为数据库 `ActiveModel`
    fn event_to_active_model(
        event: &PipelineCaptureEvent,
    ) -> Option<db::entity::capture::ActiveModel> {
        let snap = match &event.snapshot {
            Some(s) => s,
            None => {
                tracing::warn!(
                    capture_id = %event.capture_id,
                    camera_id = %event.camera_id,
                    algorithm_id = %event.algorithm_id,
                    target_label = %event.tracked_object.label,
                    "通行抓拍快照未就绪或捕获失败，跳过无图抓拍记录落库"
                );
                return None;
            }
        };

        let captured_at = chrono::DateTime::from_timestamp_millis(event.timestamp)
            .unwrap_or_else(chrono::Utc::now);

        let bbox_json =
            serde_json::to_string(&event.tracked_object.bbox).unwrap_or_else(|_| "{}".to_string());

        let quality_score = event.tracked_object.confidence.clamp(0.0, 1.0);

        Some(db::entity::capture::ActiveModel {
            id: sea_orm::NotSet,
            capture_id: Set(event.capture_id.clone()),
            camera_id: Set(event.camera_id.clone()),
            track_id: Set(event.tracked_object.track_id as i64),
            target_label: Set(event.tracked_object.label.clone()),
            confidence: Set(event.tracked_object.confidence),
            quality_score: Set(quality_score),
            bbox_json: Set(bbox_json),
            image_id: Set(snap.image_id.clone()),
            image_rel_path: Set(snap.image_rel_path.clone()),
            crop_image_id: Set(snap.crop_image_id.clone()),
            crop_image_rel_path: Set(snap.crop_image_rel_path.clone()),
            captured_at: Set(captured_at),
            created_at: Set(chrono::Utc::now()),
        })
    }

    /// 批量持久化通行抓拍凭据至 `capture_records` 表
    pub async fn process_capture_batch(
        &self,
        events: &[PipelineCaptureEvent],
    ) -> Result<usize, DbError> {
        let models: Vec<_> = events
            .iter()
            .filter_map(Self::event_to_active_model)
            .collect();
        if models.is_empty() {
            return Ok(0);
        }

        let inserted = CaptureRepo::insert_batch(&self.db, models).await?;
        tracing::info!(
            count = inserted,
            "客观通行抓拍凭证批量落库成功 (单事务攒批)"
        );

        // 对人脸通行抓拍事件尝试触发 1:N 底库比对与识别对账（仅在底库存在特征时异步分发，且不阻塞抓拍落库）
        if let Some(gallery_index) = &self.gallery_index {
            if gallery_index.count().await > 0 {
                for evt in events {
                    let is_face = evt.tracked_object.label.eq_ignore_ascii_case("face")
                        || evt.algorithm_id.contains("face");
                    if is_face {
                        let this = self.clone();
                        let event = evt.clone();
                        tokio::spawn(async move {
                            this.try_match_and_record_recognition(&event).await;
                        });
                    }
                }
            }
        }

        Ok(inserted)
    }

    /// 针对人脸通行抓拍尝试触发 1:N 底库特征检索并落地识别对账记录
    async fn try_match_and_record_recognition(&self, event: &PipelineCaptureEvent) {
        // 互斥保护：非阻塞单飞模式，已有比对在执行时跳过本次触发，杜绝并发冲击 NPU 硬件推理通道
        let _lock = match self.recognition_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("已有在途人脸识别比对任务执行中，跳过本次抓拍比对以保护 NPU");
                return;
            }
        };

        let (Some(gallery_index), Some(algo_registry)) = (&self.gallery_index, &self.algo_registry)
        else {
            return;
        };

        let is_face = event.tracked_object.label.eq_ignore_ascii_case("face")
            || event.algorithm_id.contains("face");
        if !is_face {
            return;
        }

        let Some(snap) = &event.snapshot else {
            return;
        };
        if snap.crop_image_rel_path.is_empty() {
            return;
        }

        if gallery_index.count().await == 0 {
            return;
        }

        let base_dir = self.pipeline.snapshot_engine().base_evidence_dir();
        let crop_path = base_dir.join(&snap.crop_image_rel_path);
        let crop_bytes = match tokio::fs::read(&crop_path).await {
            Ok(b) => b,
            Err(_) => return,
        };

        let extraction = match algo_registry.extract_face(&crop_bytes).await {
            Ok(ext) => ext,
            Err(_) => return,
        };

        // 遵循 docs/algo/EdgeFace.md 约定的自适应置信度动态微调
        // 质量评分围绕 0.50 基准点浮动 ±0.05，质量越低门槛越高，抑制低质误报
        let base_threshold: f32 = 0.75;
        let quality_adjustment = (extraction.quality_score - 0.5) * 0.1;
        let adaptive_threshold = (base_threshold + quality_adjustment).clamp(0.60, 0.90);

        // 执行 1:N 余弦比对
        if let Some(matched) = gallery_index
            .search(&extraction.embedding, adaptive_threshold)
            .await
        {
            let recognition_id = format!("rec_{}", uuid::Uuid::new_v4().simple());
            let recognized_at = chrono::DateTime::from_timestamp_millis(event.timestamp)
                .unwrap_or_else(chrono::Utc::now);

            // 遵循证据隔离原则：复制一份底库照片至 recognitions 目录
            let reg_photo_src = base_dir.join(&matched.photo_rel_path);
            let rec_gallery_rel = format!("recognitions/{recognition_id}_gallery.jpg");
            let rec_gallery_dest = base_dir.join(&rec_gallery_rel);
            if let Some(parent) = rec_gallery_dest.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::copy(&reg_photo_src, &rec_gallery_dest).await;

            let active_rec = db::entity::recognition::ActiveModel {
                id: sea_orm::NotSet,
                recognition_id: Set(recognition_id.clone()),
                camera_id: Set(event.camera_id.clone()),
                gallery_id: Set("default".to_string()),
                subject_id: Set(matched.subject_id.clone()),
                subject_name: Set(matched.subject_name.clone()),
                similarity: Set(matched.similarity),
                field_crop_path: Set(snap.crop_image_rel_path.clone()),
                registered_photo_path: Set(rec_gallery_rel),
                recognized_at: Set(recognized_at),
                created_at: Set(chrono::Utc::now()),
            };

            if let Ok(saved) = db::RecognitionRepo::insert(&self.db, active_rec).await {
                tracing::info!(
                    recognition_id = %recognition_id,
                    subject_id = %matched.subject_id,
                    similarity = matched.similarity,
                    camera_id = %event.camera_id,
                    "1:N 人脸识别对账命中成功并已持久化"
                );

                if let Some(broadcaster) = &self.event_broadcaster {
                    let _ = broadcaster.send(crate::state::WsBroadcastEvent {
                        topic: types::TOPIC_RECOGNITION_MATCHED.to_string(),
                        payload: serde_json::json!({
                            "recognitionId": saved.recognition_id,
                            "cameraId": saved.camera_id,
                            "galleryId": saved.gallery_id,
                            "subjectId": saved.subject_id,
                            "subjectName": saved.subject_name,
                            "similarity": saved.similarity,
                            "fieldCropPath": saved.field_crop_path,
                            "registeredPhotoPath": saved.registered_photo_path,
                            "recognizedAt": saved.recognized_at.timestamp_millis(),
                        }),
                        timestamp: event.timestamp,
                    });
                }
            }
        }
    }

    /// 刷新当前缓冲区中的抓拍事件批次
    async fn flush_batch(&self, buffer: &mut Vec<PipelineCaptureEvent>, reason: &str) {
        if buffer.is_empty() {
            return;
        }
        let to_flush = std::mem::replace(buffer, Vec::with_capacity(self.batch_size));
        if let Err(err) = self.process_capture_batch(&to_flush).await {
            tracing::error!(error = %err, reason, "通行抓拍批次持久化失败");
        }
    }

    /// 从管线内存补偿队列中排空待落库抓拍并批量持久化
    pub async fn drain_and_persist_pending(&self) -> usize {
        let mut total_processed = 0;
        loop {
            let batch = self.pipeline.drain_pending_capture_events(self.batch_size);
            if batch.is_empty() {
                break;
            }
            match self.process_capture_batch(&batch).await {
                Ok(count) => total_processed += count,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        batch_len = batch.len(),
                        "补偿排空通行抓拍事件批量持久化失败"
                    );
                }
            }
        }
        if total_processed > 0 {
            tracing::debug!(
                count = total_processed,
                "已成功从管线待持久化队列排空并持久化通行抓拍凭证"
            );
        }
        total_processed
    }

    /// 启动常驻异步持久化工作线程
    pub fn start_worker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let mut analysis_rx = self.pipeline.subscribe_analysis_events();
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let flush_interval = Duration::from_millis(self.flush_interval_ms);

        tokio::spawn(async move {
            tracing::info!("后台客观通行抓拍异步持久化工作线程已启动 (攒批写入)");

            // 1. 冷启动初期，先排空启动前积压的补偿队列
            self.drain_and_persist_pending().await;

            let mut batch_buffer: Vec<PipelineCaptureEvent> = Vec::with_capacity(self.batch_size);
            let mut ticker = tokio::time::interval(flush_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("接收到系统停机信号，通行抓拍持久化工作线程准备优雅退出");
                        self.flush_batch(&mut batch_buffer, "停机退出").await;
                        self.drain_and_persist_pending().await;
                        break;
                    }
                    _ = ticker.tick() => {
                        self.flush_batch(&mut batch_buffer, "定时刷新").await;
                        // 注意：严禁在此处无条件周期性调用 drain_and_persist_pending()！
                        // 正常广播事件已在 analysis_rx 中接收；管线补偿队列仅在冷启动、RecvError::Lagged 或停机时才需排空。
                    }
                    recv_res = analysis_rx.recv() => {
                        match recv_res {
                            Ok(PipelineAnalysisEvent::Capture(capture_evt)) => {
                                batch_buffer.push(*capture_evt);
                                if batch_buffer.len() >= self.batch_size {
                                    self.flush_batch(&mut batch_buffer, "满批阈值").await;
                                }
                            }
                            Ok(PipelineAnalysisEvent::Alarm(_)) => {
                                // 违规告警由 AlarmDispatchService 处理
                            }
                            Ok(PipelineAnalysisEvent::Tracks(_)) => {
                                // 航迹由 TrackDispatchService 处理
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(
                                    skipped,
                                    "通行抓拍广播通道滞后，尝试从管线内存队列中补偿恢复待持久化抓拍"
                                );
                                self.drain_and_persist_pending().await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("管线分析广播通道已关闭，退出抓拍工作线程");
                                self.flush_batch(&mut batch_buffer, "通道关闭").await;
                                self.drain_and_persist_pending().await;
                                break;
                            }
                        }
                    }
                }
            }
        })
    }
}
