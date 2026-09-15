//! 人脸特征后台异步重新提取任务管理器
//!
//! 在底库规模较大时，避免长 HTTP 连接超时或客户端失联，
//! 采用后台独立 Worker 执行 + 实时进度状态机轮询机制。

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use db::{DatabaseConnection, GalleryFaceRepo};
use infer::package::AlgoRegistry;
use types::{ReextractFaceFailureDetail, ReextractProgressDto, ReextractTaskStatus};

use crate::error::ApiError;
use crate::gallery_index::FaceFeatureIndex;
use crate::personnel_service::reextract_face_sample;
use crate::state::WsBroadcastEvent;

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

    /// 启动全量底库特征后台异步重新提取任务
    pub async fn start_task(
        &self,
        db: DatabaseConnection,
        evidence_base_dir: PathBuf,
        algo_registry: Arc<AlgoRegistry>,
        gallery_index: Arc<FaceFeatureIndex>,
        event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    ) -> Result<ReextractProgressDto, ApiError> {
        if !algo_registry.is_face_extraction_ready().await {
            return Err(ApiError::FaceAlgorithmNotLoaded(
                "人脸识别算法包未就绪，无法提取特征，请先部署/激活人脸算法".to_string(),
            ));
        }

        let mut guard = self.progress.write().await;
        if guard.status == ReextractTaskStatus::Running {
            return Err(ApiError::FaceExtractionConflict(
                "人脸特征重新提取任务正在执行中，请勿重复发起".to_string(),
            ));
        }

        let faces = GalleryFaceRepo::list_all_valid_vectors(&db).await?;
        let total = faces.len() as u64;

        let now = Some(chrono::Utc::now().timestamp_millis());
        if total == 0 {
            let empty_dto = ReextractProgressDto {
                status: ReextractTaskStatus::Completed,
                started_at: now,
                finished_at: now,
                ..Default::default()
            };
            *guard = empty_dto.clone();
            return Ok(empty_dto);
        }

        let initial_dto = ReextractProgressDto {
            status: ReextractTaskStatus::Running,
            total,
            started_at: now,
            ..Default::default()
        };
        *guard = initial_dto.clone();
        drop(guard);

        // 启动后台异步 Worker 处理
        let progress_ref = self.progress.clone();
        tokio::spawn(async move {
            let mut succeeded_count = 0u64;

            for face in faces {
                {
                    let mut p = progress_ref.write().await;
                    p.current_face_id = Some(face.face_id.clone());
                }

                let res =
                    reextract_face_sample(&db, &evidence_base_dir, &algo_registry, &face).await;
                let mut p = progress_ref.write().await;
                p.processed += 1;
                match res {
                    Ok(()) => {
                        succeeded_count += 1;
                        p.succeeded += 1;
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

            // 全部处理完成后重载常驻内存索引
            if succeeded_count > 0 {
                if let Err(err) = gallery_index.reload(&db).await {
                    tracing::error!("重新提取后重载底库内存特征索引失败: {err}");
                }
            }

            // 更新任务为已完成终态
            let final_snapshot = {
                let mut p = progress_ref.write().await;
                p.status = ReextractTaskStatus::Completed;
                p.current_face_id = None;
                p.finished_at = Some(chrono::Utc::now().timestamp_millis());
                p.clone()
            };

            let _ = event_broadcaster.send(WsBroadcastEvent {
                topic: "personnel.reextract.finished".to_string(),
                payload: serde_json::to_value(&final_snapshot).unwrap_or_default(),
                timestamp: chrono::Utc::now().timestamp_millis(),
            });

            tracing::info!(
                total,
                succeeded = succeeded_count,
                "底库人脸特征后台异步重新提取任务圆满完成"
            );
        });

        Ok(initial_dto)
    }
}
