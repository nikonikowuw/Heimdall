use std::sync::Arc;
use tokio::sync::broadcast;

use db::{AlarmRepo, CameraRepo, DbError};
use pipeline::{PipelineAlarmEvent, PipelineAnalysisEvent, PipelineManager};
use sea_orm::Set;
use types::{AlarmSeverity, AlarmStatus, DetectionRuleRole, TOPIC_ALARM_TRIGGERED};

use crate::state::{AppState, WsBroadcastEvent};

/// 告警异步持久化与 WebSocket 实时广播服务
///
/// 负责订阅管线分析引擎发出的规则告警事件，并以非阻塞方式：
/// 1. 异步持久化违规告警记录至 `alarm_records`；
/// 2. 同步写入行迹抓拍记录至 `capture_records`（构建安防证据三支柱）；
/// 3. 向全网 WebSocket 广播 `alarm.triggered` 实时事件；
/// 4. 在冷启动与通道滞后 (Lagged) 时通过 `drain_pending_alarm_events` 无损补偿。
#[derive(Debug, Clone)]
pub struct AlarmDispatchService {
    pub db: sea_orm::DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub shutdown_tx: broadcast::Sender<()>,
}

impl AlarmDispatchService {
    /// 从 `AppState` 中提取句柄构造服务实例
    pub fn from_state(state: &AppState) -> Self {
        Self {
            db: state.db.clone(),
            pipeline: state.pipeline.clone(),
            event_broadcaster: state.event_broadcaster.clone(),
            shutdown_tx: state.shutdown_tx.clone(),
        }
    }

    /// 持久化单条告警事件并向 WebSocket 广播
    ///
    /// 具备基于 `event_id` 的幂等性保护，若记录已存在则静默返回 `None`。
    pub async fn process_alarm_event(
        &self,
        event: &PipelineAlarmEvent,
    ) -> Result<Option<db::entity::alarm::Model>, DbError> {
        // 1. 幂等性检查：如果该 event_id 已经落库，直接跳过避免重复插入与重复广播
        if let Some(existing) = AlarmRepo::find_by_event_id(&self.db, &event.event_id).await? {
            return Ok(Some(existing));
        }

        let (image_id, image_rel_path, crop_image_id, crop_image_rel_path) = match &event.snapshot {
            Some(snap) => (
                snap.image_id.as_str(),
                snap.image_rel_path.as_str(),
                snap.crop_image_id.as_str(),
                snap.crop_image_rel_path.as_str(),
            ),
            None => ("", "", "", ""),
        };

        let rule_type = match event.alarm.role {
            DetectionRuleRole::Roi => "roi",
            DetectionRuleRole::Line => "line",
            DetectionRuleRole::Mask => "mask",
        };

        let occurred_at = chrono::DateTime::from_timestamp_millis(event.timestamp)
            .unwrap_or_else(chrono::Utc::now);

        let bbox_json = serde_json::to_string(&event.alarm.tracked_object.bbox)
            .unwrap_or_else(|_| "{}".to_string());

        // 2. 构建违规告警记录 (alarm_records)
        let active_alarm = db::entity::alarm::ActiveModel {
            id: sea_orm::NotSet,
            event_id: Set(event.event_id.clone()),
            camera_id: Set(event.camera_id.clone()),
            alarm_type_id: Set(format!("rule_{}", event.alarm.rule_index)),
            occurred_at: Set(occurred_at),
            target_label: Set(event.alarm.tracked_object.label.clone()),
            confidence: Set(event.alarm.tracked_object.confidence),
            track_id: Set(event.alarm.tracked_object.track_id as i64),
            bbox_json: Set(bbox_json.clone()),
            image_id: Set(image_id.to_string()),
            image_rel_path: Set(image_rel_path.to_string()),
            crop_image_id: Set(crop_image_id.to_string()),
            crop_image_rel_path: Set(crop_image_rel_path.to_string()),
            rule_type: Set(rule_type.to_string()),
            severity: Set(AlarmSeverity::Warning.as_str().to_string()),
            status: Set(AlarmStatus::Unprocessed.as_str().to_string()),
            handled_at: Set(None),
            created_at: Set(chrono::Utc::now()),
        };

        // 3. 构建抓拍凭证 (capture_records，仅在快照成功生成时成对落库，快照失败不创建无图抓拍)
        let active_capture = event
            .snapshot
            .as_ref()
            .map(|snap| db::entity::capture::ActiveModel {
                id: sea_orm::NotSet,
                capture_id: Set(uuid::Uuid::new_v4().to_string()),
                camera_id: Set(event.camera_id.clone()),
                track_id: Set(event.alarm.tracked_object.track_id as i64),
                target_label: Set(event.alarm.tracked_object.label.clone()),
                confidence: Set(event.alarm.tracked_object.confidence),
                quality_score: Set(1.0),
                bbox_json: Set(bbox_json),
                image_id: Set(snap.image_id.clone()),
                image_rel_path: Set(snap.image_rel_path.clone()),
                crop_image_id: Set(snap.crop_image_id.clone()),
                crop_image_rel_path: Set(snap.crop_image_rel_path.clone()),
                captured_at: Set(occurred_at),
                created_at: Set(chrono::Utc::now()),
            });

        // 4. 单一 SQLite 事务原子双写，杜绝产生孤儿记录或半更新状态
        let saved_alarm =
            AlarmRepo::insert_alarm_with_optional_capture(&self.db, active_alarm, active_capture)
                .await?;

        // 5. 解析摄像头展示名称
        let camera_name = match CameraRepo::find_by_camera_id(&self.db, &event.camera_id).await {
            Ok(Some(cam)) if !cam.name.trim().is_empty() => cam.name,
            _ => event.camera_id.clone(),
        };

        // 6. 向 WebSocket 广播实时告警事件
        let ws_event = WsBroadcastEvent {
            topic: TOPIC_ALARM_TRIGGERED.to_string(),
            payload: serde_json::json!({
                "id": saved_alarm.id,
                "eventId": saved_alarm.event_id,
                "cameraId": saved_alarm.camera_id,
                "cameraName": camera_name,
                "targetLabel": saved_alarm.target_label,
                "ruleType": saved_alarm.rule_type,
                "severity": saved_alarm.severity,
                "cropImageRelPath": saved_alarm.crop_image_rel_path,
                "imageRelPath": saved_alarm.image_rel_path,
                "occurredAt": event.timestamp,
            }),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };

        let _ = self.event_broadcaster.send(ws_event);

        tracing::info!(
            alarm_id = saved_alarm.id,
            event_id = %saved_alarm.event_id,
            camera_id = %saved_alarm.camera_id,
            rule_type = %saved_alarm.rule_type,
            target_label = %saved_alarm.target_label,
            "规则告警持久化落库并成功广播 WebSocket 实时事件"
        );

        Ok(Some(saved_alarm))
    }

    /// 从管线内存补偿队列中排空待落库告警并无损持久化
    pub async fn drain_and_persist_pending(&self) -> usize {
        let mut processed_count = 0;
        loop {
            let batch = self.pipeline.drain_pending_alarm_events(64);
            if batch.is_empty() {
                break;
            }
            for evt in batch {
                match self.process_alarm_event(&evt).await {
                    Ok(Some(_)) => processed_count += 1,
                    Ok(None) => {}
                    Err(err) => {
                        tracing::error!(
                            event_id = %evt.event_id,
                            camera_id = %evt.camera_id,
                            error = %err,
                            "补偿排空告警事件持久化失败"
                        );
                    }
                }
            }
        }
        if processed_count > 0 {
            tracing::debug!(
                count = processed_count,
                "已成功从管线待持久化队列处理并持久化告警事件"
            );
        }
        processed_count
    }

    /// 启动后台常驻工作线程
    pub fn start_worker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!("后台告警异步持久化与实时广播工作线程已启动");

            // 启动初期，先排空启动前可能积压的补偿队列
            self.drain_and_persist_pending().await;

            let mut analysis_rx = self.pipeline.subscribe_analysis_events();
            let mut shutdown_rx = self.shutdown_tx.subscribe();

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("接收到系统停机信号，告警分发工作线程准备优雅退出");
                        self.drain_and_persist_pending().await;
                        break;
                    }
                    recv_res = analysis_rx.recv() => {
                        match recv_res {
                            Ok(PipelineAnalysisEvent::Alarm(alarm_evt)) => {
                                let drained = self.drain_and_persist_pending().await;
                                if drained == 0 {
                                    if let Err(err) = self.process_alarm_event(&alarm_evt).await {
                                        tracing::error!(
                                            event_id = %alarm_evt.event_id,
                                            camera_id = %alarm_evt.camera_id,
                                            error = %err,
                                            "实时告警事件持久化失败"
                                        );
                                    }
                                }
                            }
                            Ok(PipelineAnalysisEvent::Tracks(_)) => {
                                // 实时航迹流不在此处理
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(skipped, "分析事件广播落后，触发全量待持久化补偿排空");
                                self.drain_and_persist_pending().await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("分析事件广播通道已关闭，告警工作线程平稳退出");
                                self.drain_and_persist_pending().await;
                                break;
                            }
                        }
                    }
                }
            }

            tracing::info!("后台告警异步持久化与实时广播工作线程已安全停止");
        })
    }
}
