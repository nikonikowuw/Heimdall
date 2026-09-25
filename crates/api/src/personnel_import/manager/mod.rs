//! 人员批量导入任务管理器
//!
//! 与 `PersonnelReextractManager` 同构：`RwLock` 状态机 + Tokio 后台 Worker + 有界 WS 广播。
//!
//! 关键约束：
//! - 归档解压落在独立沙箱 `var/tmp/personnel_import/{task_id}/`，由 [`TempImportSandbox`]
//!   的 `Drop` 在完成、失败、取消与 panic 路径统一物理清理；
//! - 解析与逐人提取**严格串行**（并发度 1），避免与常驻视频流推理争抢 NPU，并让批次内部后续候选
//!   实时看到前序候选已并入的底库索引，从而拦截同批次跨主体撞脸；
//! - 与全量底库特征重提取任务互斥，杜绝双高负荷任务并行。

mod enrollment;
mod sandbox;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use db::DatabaseConnection;
use infer::package::AlgoRegistry;
use tokio::sync::{broadcast, RwLock};
use types::{
    ImportTaskStatus, PersonnelImportProgressDto, TOPIC_PERSONNEL_IMPORT_FINISHED,
    TOPIC_PERSONNEL_IMPORT_PROGRESS,
};

use crate::error::ApiError;
use crate::gallery_index::FaceFeatureIndex;
use crate::personnel_import::candidate::{
    parse_candidates, resolve_content_root, ImportCandidate, ParseError,
};
use crate::personnel_maintenance::MaintenanceGuard;
use crate::state::WsBroadcastEvent;

pub use sandbox::{
    sweep_orphan_sandboxes, TempImportSandbox, IMPORT_SANDBOX_DIR, MAX_IMPORT_ARCHIVE_BYTES,
    MAX_IMPORT_ARCHIVE_ENTRIES, MAX_IMPORT_REQUEST_BYTES, MAX_IMPORT_UNCOMPRESSED_BYTES,
};

use enrollment::import_single_candidate;

/// 进度广播的节流间隔（毫秒）：避免逐人广播在数百人批次中产生 WS 风暴，
/// 但始终保证「处理完成」一帧必达。
pub const PROGRESS_BROADCAST_THROTTLE_MS: u64 = 900;

/// 人员批量导入管理器
#[derive(Debug, Clone)]
pub struct PersonnelImportManager {
    progress: Arc<RwLock<PersonnelImportProgressDto>>,
    cancel_flag: Arc<AtomicBool>,
    sandbox_root: PathBuf,
}

impl PersonnelImportManager {
    /// 以默认沙箱根目录构造
    pub fn new() -> Self {
        Self::with_sandbox_root(PathBuf::from(IMPORT_SANDBOX_DIR))
    }

    /// 以指定沙箱根目录构造（测试注入隔离目录）
    pub fn with_sandbox_root(sandbox_root: PathBuf) -> Self {
        Self {
            progress: Arc::new(RwLock::new(PersonnelImportProgressDto::default())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            sandbox_root,
        }
    }

    /// 沙箱根目录
    pub fn sandbox_root(&self) -> &Path {
        &self.sandbox_root
    }

    /// 查询当前任务进度快照
    pub async fn get_progress(&self) -> PersonnelImportProgressDto {
        self.progress.read().await.clone()
    }

    /// 是否有导入任务正在执行
    pub async fn is_running(&self) -> bool {
        self.progress.read().await.is_running()
    }

    /// 请求取消当前任务（幂等）：Worker 会在当前候选处理结束后退出并清理沙箱
    pub async fn request_cancel(&self) -> Result<PersonnelImportProgressDto, ApiError> {
        let snapshot = self.progress.read().await.clone();
        if !snapshot.is_running() {
            return Err(ApiError::BadRequest(
                "当前没有正在执行的人员导入任务".to_string(),
            ));
        }
        self.cancel_flag.store(true, Ordering::SeqCst);
        tracing::info!(task_id = %snapshot.task_id, "已受理人员批量导入取消请求");
        Ok(snapshot)
    }

    /// 登记归档体积并解压到沙箱，返回解析出的候选列表
    ///
    /// 解压与解析都是同步阻塞 I/O，统一在专用阻塞线程内执行。
    pub async fn prepare_candidates(
        &self,
        archive_path: PathBuf,
        archive_name: Option<String>,
        task_id: String,
    ) -> Result<(TempImportSandbox, Vec<ImportCandidate>), ApiError> {
        let sandbox_root = self.sandbox_root.clone();
        let sandbox = TempImportSandbox::create(&sandbox_root, &task_id)?;
        let extract_dir = sandbox.path().to_path_buf();

        let candidates = tokio::task::spawn_blocking(move || {
            // 复用算法包归档的安全解压原语（Zip-Slip / Tar-Slip 防护 + 格式自动识别），
            // 但不校验 manifest.json —— 人员导入包没有该文件。
            crate::algo::archive::extract_archive_to_dir_with_limits(
                &archive_path,
                archive_name.as_deref(),
                &extract_dir,
                crate::algo::archive::ArchiveExtractionLimits {
                    max_entries: MAX_IMPORT_ARCHIVE_ENTRIES,
                    max_uncompressed_bytes: MAX_IMPORT_UNCOMPRESSED_BYTES,
                },
            )
            .map_err(ParseError::ManifestInvalid)?;

            let package_root = resolve_content_root(&extract_dir);
            parse_candidates(&package_root)
        })
        .await
        .map_err(|err| ApiError::Internal(format!("导入归档解析任务异常: {err}")))?;

        match candidates {
            Ok(list) => Ok((sandbox, list)),
            Err(err) => Err(ApiError::BadRequest(err.message())),
        }
    }

    /// 启动批量导入后台任务
    ///
    /// 调用方需已完成解压解析（`prepare_candidates`）、持有底库重型任务闸门
    /// （`MaintenanceGate::acquire`）并把沙箱所有权移交进来；
    /// 沙箱与闸门守卫都随 Worker 结束（含取消与 panic）自动释放。
    #[allow(clippy::too_many_arguments)]
    pub async fn start_task(
        &self,
        task_id: String,
        sandbox: TempImportSandbox,
        candidates: Vec<ImportCandidate>,
        db: DatabaseConnection,
        evidence_base_dir: PathBuf,
        algo_registry: Arc<AlgoRegistry>,
        gallery_index: Arc<FaceFeatureIndex>,
        maintenance_guard: MaintenanceGuard,
        event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    ) -> Result<PersonnelImportProgressDto, ApiError> {
        if !algo_registry.is_face_extraction_ready().await {
            return Err(ApiError::FaceAlgorithmNotLoaded(
                "人脸识别算法包未就绪，无法提取特征，请先部署/激活人脸算法".to_string(),
            ));
        }

        let mut guard = self.progress.write().await;
        if guard.is_running() {
            return Err(ApiError::FaceExtractionConflict(
                "已有人员批量导入任务正在执行，请勿重复发起".to_string(),
            ));
        }

        let now = Some(chrono::Utc::now().timestamp_millis());
        let initial = PersonnelImportProgressDto {
            task_id: task_id.clone(),
            status: ImportTaskStatus::Running,
            total: candidates.len() as u64,
            started_at: now,
            ..Default::default()
        };
        *guard = initial.clone();
        drop(guard);

        self.cancel_flag.store(false, Ordering::SeqCst);
        let cancel_flag = self.cancel_flag.clone();
        let progress_ref = self.progress.clone();

        tokio::spawn(async move {
            // 沙箱守卫随 Worker 作用域结束自动清理临时解压产物；
            // 闸门守卫同时持有，保证任务全程独占底库重型任务槽位。
            let _sandbox_guard = sandbox;
            let _maintenance_guard = maintenance_guard;

            let mut last_broadcast = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    PROGRESS_BROADCAST_THROTTLE_MS,
                ))
                .unwrap_or_else(std::time::Instant::now);

            let total = candidates.len() as u64;
            let mut cancelled = false;

            for candidate in candidates {
                if cancel_flag.load(Ordering::SeqCst) {
                    cancelled = true;
                    break;
                }

                let candidate_name = candidate.name.clone();
                {
                    let mut p = progress_ref.write().await;
                    p.current_name = Some(candidate_name.clone());
                }
                broadcast_progress(
                    &event_broadcaster,
                    &progress_ref,
                    &mut last_broadcast,
                    false,
                )
                .await;

                let outcome = import_single_candidate(
                    &db,
                    &evidence_base_dir,
                    &algo_registry,
                    &gallery_index,
                    candidate,
                )
                .await;

                let mut p = progress_ref.write().await;
                p.processed += 1;
                match outcome {
                    Ok(()) => p.succeeded += 1,
                    Err(failure) => {
                        p.failed += 1;
                        tracing::info!(
                            task_id = %task_id,
                            name = %failure.name,
                            kind = failure.kind.as_str(),
                            reason = %failure.reason,
                            "批量导入候选人员失败"
                        );
                        p.failures.push(failure);
                    }
                }
                drop(p);

                broadcast_progress(
                    &event_broadcaster,
                    &progress_ref,
                    &mut last_broadcast,
                    false,
                )
                .await;
            }

            let final_snapshot = {
                let mut p = progress_ref.write().await;
                p.current_name = None;
                p.finished_at = Some(chrono::Utc::now().timestamp_millis());
                p.status = if cancelled {
                    ImportTaskStatus::Cancelled
                } else if p.succeeded == 0 && p.total > 0 {
                    ImportTaskStatus::Failed
                } else {
                    ImportTaskStatus::Completed
                };
                p.clone()
            };

            let _ = event_broadcaster.send(WsBroadcastEvent {
                topic: TOPIC_PERSONNEL_IMPORT_FINISHED.to_string(),
                payload: serde_json::to_value(&final_snapshot).unwrap_or_default(),
                timestamp: chrono::Utc::now().timestamp_millis(),
            });

            tracing::info!(
                task_id = %task_id,
                total,
                succeeded = final_snapshot.succeeded,
                failed = final_snapshot.failed,
                status = final_snapshot.status.as_str(),
                "人员批量导入任务结束"
            );
        });

        Ok(initial)
    }
}

impl Default for PersonnelImportManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 按节流策略广播运行中进度
async fn broadcast_progress(
    broadcaster: &broadcast::Sender<WsBroadcastEvent>,
    progress_ref: &Arc<RwLock<PersonnelImportProgressDto>>,
    last_broadcast: &mut std::time::Instant,
    force: bool,
) {
    if !force && last_broadcast.elapsed().as_millis() < u128::from(PROGRESS_BROADCAST_THROTTLE_MS) {
        return;
    }
    let snapshot = progress_ref.read().await.clone();
    *last_broadcast = std::time::Instant::now();
    let _ = broadcaster.send(WsBroadcastEvent {
        topic: TOPIC_PERSONNEL_IMPORT_PROGRESS.to_string(),
        payload: serde_json::to_value(&snapshot).unwrap_or_default(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_guard_removes_directory_on_drop() {
        let root = std::env::temp_dir().join(format!(
            "heimdall_sandbox_drop_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let task_id = "task-1";

        {
            let sandbox = TempImportSandbox::create(&root, task_id).unwrap();
            std::fs::write(sandbox.path().join("payload.jpg"), b"x").unwrap();
            assert!(sandbox.path().join("payload.jpg").is_file());
        }

        assert!(
            !root.join(task_id).exists(),
            "沙箱守卫析构后必须物理清空任务目录"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn orphan_sweep_removes_stale_task_directories() {
        let root = std::env::temp_dir().join(format!(
            "heimdall_sandbox_sweep_{}",
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(root.join("stale-task")).unwrap();
        std::fs::write(root.join("stale-task/a.jpg"), b"x").unwrap();

        sweep_orphan_sandboxes(&root);

        assert!(!root.join("stale-task").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn orphan_sweep_is_noop_for_missing_root() {
        let root = std::env::temp_dir().join(format!(
            "heimdall_sandbox_missing_{}",
            uuid::Uuid::now_v7().simple()
        ));
        // 目录不存在时不 panic，也不创建目录
        sweep_orphan_sandboxes(&root);
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn cancel_request_rejected_when_idle() {
        let manager = PersonnelImportManager::new();
        let err = manager.request_cancel().await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }
}
