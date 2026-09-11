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
        Ok(inserted)
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
                        // 周期性检查是否有补偿队列积压
                        self.drain_and_persist_pending().await;
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
