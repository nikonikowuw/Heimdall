//! 人脸特征后台异步重新提取任务管理器
//!
//! 在底库规模较大时，避免长 HTTP 连接超时或客户端失联，
//! 采用后台独立 Worker 执行 + 实时进度状态机轮询机制。

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use db::{DatabaseConnection, GalleryFaceRepo, PersonnelRepo};
use infer::package::AlgoRegistry;
use types::{ReextractFaceFailureDetail, ReextractProgressDto, ReextractTaskStatus};

use crate::error::ApiError;
use crate::gallery_index::FaceFeatureIndex;
use crate::personnel_maintenance::MaintenanceGuard;
use crate::personnel_service::reextract_face_sample;
use crate::state::WsBroadcastEvent;

const REEXTRACT_INDEX_BATCH_SIZE: usize = 64;

/// 重提取任务的样本范围
///
/// 全量与单体共用同一套进度状态机、同一把维护闸门与同一个 WS 终态主题：
/// 重提取是串行的重型 NPU 任务，二者天然互斥，不必各写一套状态机。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReextractScope {
    /// 全量底库
    AllFaces,
    /// 指定人员的全部样本
    Subject(String),
}

impl ReextractScope {
    /// 面向日志与错误文案的范围描述
    pub fn label(&self) -> String {
        match self {
            Self::AllFaces => "全量底库".to_string(),
            Self::Subject(subject_id) => format!("人员 {subject_id}"),
        }
    }

    fn subject_id(&self) -> Option<&str> {
        match self {
            Self::AllFaces => None,
            Self::Subject(subject_id) => Some(subject_id),
        }
    }

    async fn load_faces(
        &self,
        db: &DatabaseConnection,
    ) -> Result<Vec<db::entity::gallery_face::Model>, ApiError> {
        match self {
            Self::AllFaces => Ok(GalleryFaceRepo::list_all_valid_vectors(db).await?),
            Self::Subject(subject_id) => {
                Ok(GalleryFaceRepo::list_by_subject_id(db, subject_id).await?)
            }
        }
    }

    /// 指定人员时先确认档案存在：否则空范围会被当成「已完成」而静默成功
    async fn ensure_target_exists(&self, db: &DatabaseConnection) -> Result<(), ApiError> {
        match self {
            Self::AllFaces => Ok(()),
            Self::Subject(subject_id) => {
                if PersonnelRepo::find_by_subject_id(db, subject_id)
                    .await?
                    .is_none()
                {
                    return Err(ApiError::NotFound(format!("人员不存在: {subject_id}")));
                }
                Ok(())
            }
        }
    }
}

/// 后台重提取 Worker 依赖，作为一次任务所有权整体移交。
#[derive(Debug)]
pub struct ReextractTaskDependencies {
    pub db: DatabaseConnection,
    pub evidence_base_dir: PathBuf,
    pub algo_registry: Arc<AlgoRegistry>,
    pub gallery_index: Arc<FaceFeatureIndex>,
    pub maintenance_guard: MaintenanceGuard,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
}

async fn publish_terminal_progress(
    progress: &Arc<RwLock<ReextractProgressDto>>,
    worker_result: &Result<u64, tokio::task::JoinError>,
    event_broadcaster: &broadcast::Sender<WsBroadcastEvent>,
) -> ReextractProgressDto {
    let status = if worker_result.is_ok() {
        ReextractTaskStatus::Completed
    } else {
        ReextractTaskStatus::Failed
    };
    let snapshot = {
        let mut progress = progress.write().await;
        progress.status = status;
        progress.current_face_id = None;
        progress.finished_at = Some(chrono::Utc::now().timestamp_millis());
        if status == ReextractTaskStatus::Failed {
            progress.error_message = None;
        }
        progress.clone()
    };

    let _ = event_broadcaster.send(WsBroadcastEvent {
        topic: types::TOPIC_PERSONNEL_REEXTRACT_FINISHED.to_string(),
        payload: serde_json::to_value(&snapshot).unwrap_or_default(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    });
    snapshot
}

/// 人脸底库特征后台重新提取管理器（线程安全共享状态）
#[derive(Debug, Clone)]
pub struct PersonnelReextractManager {
    progress: Arc<RwLock<ReextractProgressDto>>,
}

impl Default for PersonnelReextractManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PersonnelReextractManager {
    pub fn new() -> Self {
        Self {
            progress: Arc::new(RwLock::new(ReextractProgressDto::default())),
        }
    }

    /// 查询当前任务进度快照
    pub async fn get_progress(&self) -> ReextractProgressDto {
        self.progress.read().await.clone()
    }

    /// 检查是否有后台任务正在执行
    pub async fn is_running(&self) -> bool {
        self.progress.read().await.status == ReextractTaskStatus::Running
    }

    /// 启动底库特征后台异步重新提取任务（全量或指定人员）
    ///
    /// 同步返回初始进度快照；实际提取在后台 Worker 内串行执行，
    /// 调用方通过 `get_progress` / WS 终态主题观测结果。
    pub async fn start_task(
        &self,
        scope: ReextractScope,
        dependencies: ReextractTaskDependencies,
    ) -> Result<ReextractProgressDto, ApiError> {
        let ReextractTaskDependencies {
            db,
            evidence_base_dir,
            algo_registry,
            gallery_index,
            maintenance_guard,
            event_broadcaster,
        } = dependencies;
        if !algo_registry.is_face_extraction_ready().await {
            return Err(ApiError::FaceAlgorithmNotLoaded(
                "人脸识别算法包未就绪，无法提取特征，请先部署/激活人脸算法".to_string(),
            ));
        }

        // 单人重提以前持有录入许可直到所有样本与索引更新完毕。在线增删、改名
        // 也使用同一许可，因此后台化后必须把许可移动进 Worker，不能只在请求里持有。
        let enrollment_permit = if scope.subject_id().is_some() {
            Some(
                gallery_index
                    .acquire_enrollment_permit()
                    .await
                    .map_err(ApiError::Internal)?,
            )
        } else {
            None
        };

        // 先校验目标存在再读取其样本；单人路径已持有录入许可，检查与快照之间
        // 不会被并发删除或追加打断。
        scope.ensure_target_exists(&db).await?;
        let faces = scope.load_faces(&db).await?;
        let total = faces.len() as u64;
        let scope_label = scope.label();
        let subject_id = scope.subject_id().map(str::to_owned);

        // 不持有进度锁执行数据库 I/O；只在任务状态变更时短暂写锁。
        let mut guard = self.progress.write().await;
        if guard.status == ReextractTaskStatus::Running {
            return Err(ApiError::FaceExtractionConflict(
                "人脸特征重新提取任务正在执行中，请勿重复发起".to_string(),
            ));
        }

        let now = Some(chrono::Utc::now().timestamp_millis());
        if total == 0 {
            let empty_dto = ReextractProgressDto {
                status: ReextractTaskStatus::Completed,
                subject_id,
                started_at: now,
                finished_at: now,
                ..Default::default()
            };
            *guard = empty_dto.clone();
            return Ok(empty_dto);
        }

        let initial_dto = ReextractProgressDto {
            status: ReextractTaskStatus::Running,
            subject_id,
            total,
            started_at: now,
            ..Default::default()
        };
        *guard = initial_dto.clone();
        drop(guard);

        // 启动后台任务与监督器。监督器保留 RAII 许可并等待 Worker，
        // Worker panic 时也能把进度收敛到 failed，而不是永久停留在 running。
        let progress_ref = self.progress.clone();
        tokio::spawn(async move {
            let _maintenance_guard = maintenance_guard;
            let _enrollment_permit = enrollment_permit;
            let worker_progress = progress_ref.clone();
            let worker = tokio::spawn(async move {
                let mut succeeded_count = 0u64;
                let mut refreshed: Vec<(String, Vec<u8>)> =
                    Vec::with_capacity(faces.len().min(REEXTRACT_INDEX_BATCH_SIZE));

                for face in faces {
                    {
                        let mut p = worker_progress.write().await;
                        p.current_face_id = Some(face.face_id.clone());
                    }

                    let res =
                        reextract_face_sample(&db, &evidence_base_dir, &algo_registry, &face).await;
                    {
                        let mut p = worker_progress.write().await;
                        p.processed += 1;
                        match res {
                            Ok(reextracted) => {
                                succeeded_count += 1;
                                p.succeeded += 1;
                                refreshed.push((reextracted.face_id, reextracted.feature_bytes));
                            }
                            Err(reason) => {
                                p.failed += 1;
                                p.failures.push(ReextractFaceFailureDetail {
                                    face_id: face.face_id,
                                    subject_id: face.subject_id,
                                    reason,
                                });
                            }
                        }
                    }

                    if refreshed.len() >= REEXTRACT_INDEX_BATCH_SIZE {
                        gallery_index
                            .replace_face_features(std::mem::take(&mut refreshed))
                            .await;
                        refreshed = Vec::with_capacity(REEXTRACT_INDEX_BATCH_SIZE);
                    }
                }

                if !refreshed.is_empty() {
                    gallery_index.replace_face_features(refreshed).await;
                }
                succeeded_count
            });

            let worker_result = worker.await;
            publish_terminal_progress(&progress_ref, &worker_result, &event_broadcaster).await;

            match worker_result {
                Ok(succeeded_count) => tracing::info!(
                    total,
                    scope = %scope_label,
                    succeeded = succeeded_count,
                    "底库人脸特征后台异步重新提取任务圆满完成"
                ),
                Err(error) => tracing::error!(
                    total,
                    scope = %scope_label,
                    error = %error,
                    "底库人脸特征后台异步重新提取任务异常终止"
                ),
            }
        });

        Ok(initial_dto)
    }
}

#[cfg(test)]
mod tests {
    use db::entity::gallery_face::ActiveModel as FaceActiveModel;
    use db::entity::personnel::ActiveModel as PersonnelActiveModel;
    use db::{GalleryFaceRepo, PersonnelRepo};
    use sea_orm::Set;

    use super::*;

    #[tokio::test]
    async fn subject_scope_loads_only_target_faces_and_serializes_progress_scope() {
        let db = db::init_test_db()
            .await
            .expect("test database initialization should succeed");
        let now = chrono::Utc::now();
        for subject_id in ["target", "other"] {
            PersonnelRepo::insert(
                &db,
                PersonnelActiveModel {
                    id: sea_orm::NotSet,
                    subject_id: Set(subject_id.to_string()),
                    name: Set(subject_id.to_string()),
                    id_card: Set(String::new()),
                    remark: Set(String::new()),
                    primary_photo_path: Set(String::new()),
                    created_at: Set(now),
                    updated_at: Set(now),
                },
            )
            .await
            .expect("test personnel insertion should succeed");
        }

        for (face_id, subject_id) in [
            ("target-face-1", "target"),
            ("target-face-2", "target"),
            ("other-face", "other"),
        ] {
            GalleryFaceRepo::insert(
                &db,
                FaceActiveModel {
                    id: sea_orm::NotSet,
                    face_id: Set(face_id.to_string()),
                    subject_id: Set(subject_id.to_string()),
                    photo_rel_path: Set(format!("{face_id}.jpg")),
                    aligned_rel_path: Set(String::new()),
                    feature_vector: Set(vec![0; 2048]),
                    quality_score: Set(0.9),
                    detection_score: Set(0.9),
                    is_primary: Set(0),
                    created_at: Set(now),
                },
            )
            .await
            .expect("test gallery face insertion should succeed");
        }

        let scope = ReextractScope::Subject("target".to_string());
        scope
            .ensure_target_exists(&db)
            .await
            .expect("target personnel should exist");
        let faces = scope
            .load_faces(&db)
            .await
            .expect("subject face query should succeed");
        assert_eq!(
            faces
                .iter()
                .map(|face| face.face_id.as_str())
                .collect::<Vec<_>>(),
            ["target-face-1", "target-face-2"]
        );

        let progress = ReextractProgressDto {
            status: ReextractTaskStatus::Running,
            subject_id: scope.subject_id().map(str::to_owned),
            total: faces.len() as u64,
            ..Default::default()
        };
        let payload = serde_json::to_value(progress).expect("progress DTO should serialize");
        assert_eq!(payload["subjectId"], "target");
        assert_eq!(payload["total"], 2);
    }

    #[tokio::test]
    async fn completed_worker_publishes_terminal_progress() {
        let progress = Arc::new(RwLock::new(ReextractProgressDto {
            status: ReextractTaskStatus::Running,
            subject_id: Some("subject-a".to_string()),
            current_face_id: Some("face-1".to_string()),
            ..Default::default()
        }));
        let (event_broadcaster, mut events) = broadcast::channel(1);

        let snapshot = publish_terminal_progress(&progress, &Ok(1), &event_broadcaster).await;

        assert_eq!(snapshot.status, ReextractTaskStatus::Completed);
        assert_eq!(snapshot.subject_id.as_deref(), Some("subject-a"));
        assert!(snapshot.current_face_id.is_none());
        assert!(snapshot.finished_at.is_some());
        let event = events
            .try_recv()
            .expect("terminal progress should be broadcast");
        assert_eq!(event.topic, types::TOPIC_PERSONNEL_REEXTRACT_FINISHED);
        assert_eq!(event.payload["status"], "completed");
        assert_eq!(event.payload["subjectId"], "subject-a");
    }

    #[tokio::test]
    async fn cancelled_worker_publishes_failed_terminal_progress() {
        let progress = Arc::new(RwLock::new(ReextractProgressDto {
            status: ReextractTaskStatus::Running,
            subject_id: Some("subject-a".to_string()),
            current_face_id: Some("face-1".to_string()),
            ..Default::default()
        }));
        let (event_broadcaster, mut events) = broadcast::channel(1);
        let worker = tokio::spawn(std::future::pending::<()>());
        worker.abort();
        let worker_result = worker.await;
        assert!(
            worker_result.is_err(),
            "aborted worker must return JoinError"
        );

        let snapshot =
            publish_terminal_progress(&progress, &worker_result.map(|()| 0), &event_broadcaster)
                .await;

        assert_eq!(snapshot.status, ReextractTaskStatus::Failed);
        assert_eq!(snapshot.subject_id.as_deref(), Some("subject-a"));
        assert!(snapshot.current_face_id.is_none());
        assert!(snapshot.finished_at.is_some());
        assert!(snapshot.error_message.is_none());
        let event = events
            .try_recv()
            .expect("failed terminal progress should be broadcast");
        assert_eq!(event.topic, types::TOPIC_PERSONNEL_REEXTRACT_FINISHED);
        assert_eq!(event.payload["status"], "failed");
    }

    #[tokio::test]
    async fn subject_scope_rejects_a_missing_personnel_record() {
        let db = db::init_test_db()
            .await
            .expect("test database initialization should succeed");
        let result = ReextractScope::Subject("missing".to_string())
            .ensure_target_exists(&db)
            .await;
        assert!(matches!(result, Err(ApiError::NotFound(_))));
    }
}
