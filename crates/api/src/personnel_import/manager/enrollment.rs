//! 单个候选人员的原子录入：提取 → 冲撞校验 → 单事务落库 → 增量并入内存索引

use std::path::Path;
use std::sync::Arc;

use db::{DatabaseConnection, GalleryFaceRepo, PersonnelRepo};
use infer::package::AlgoRegistry;
use sea_orm::TransactionTrait;
use types::{ImportFailureDetail, ImportFailureKind};

use crate::error::ApiError;
use crate::gallery_index::{FaceFeatureIndex, RegisteredFace};
use crate::personnel_import::candidate::ImportCandidate;
use crate::personnel_service::{
    process_and_save_face_bytes, validate_personnel_subject_id, DiskRollbackGuard,
    ENROLLMENT_CLASH_RAW_COSINE_THRESHOLD,
};

/// 导入单个候选人员：提取 → 冲撞校验 → 单事务落库 → 增量并入内存索引
///
/// 任一环节失败都会回滚本候选已写入的磁盘暂存文件，不影响同批次其它候选。
pub(super) async fn import_single_candidate(
    db: &DatabaseConnection,
    evidence_base_dir: &Path,
    algo_registry: &Arc<AlgoRegistry>,
    gallery_index: &Arc<FaceFeatureIndex>,
    candidate: ImportCandidate,
) -> Result<(), ImportFailureDetail> {
    let ImportCandidate {
        name,
        subject_id,
        id_card,
        remark,
        photo_paths,
        skipped_photos,
    } = candidate;

    let mut failure = ImportFailureDetail {
        name: name.clone(),
        subject_id: subject_id.clone().unwrap_or_default(),
        kind: ImportFailureKind::Internal,
        reason: String::new(),
        skipped_photos,
    };

    if name.trim().is_empty() {
        failure.kind = ImportFailureKind::Conflict;
        failure.reason = "候选姓名为空，已跳过".to_string();
        return Err(failure);
    }

    if photo_paths.is_empty() {
        failure.kind = ImportFailureKind::NoPhoto;
        failure.reason = "未在导入包中找到该人员关联的可用照片".to_string();
        return Err(failure);
    }

    // 编号解析与占位校验（UUIDv7 在并发导入间不会冲突）
    let subject_id = match subject_id.map(|value| value.trim().to_string()) {
        Some(value) if !value.is_empty() => value,
        _ => uuid::Uuid::now_v7().to_string(),
    };
    failure.subject_id = subject_id.clone();
    if let Err(reason) = validate_personnel_subject_id(&subject_id) {
        failure.kind = ImportFailureKind::Conflict;
        failure.reason = reason.to_string();
        return Err(failure);
    }

    match PersonnelRepo::find_by_subject_id(db, &subject_id).await {
        Ok(Some(_)) => {
            failure.kind = ImportFailureKind::Conflict;
            failure.reason = format!("人员编号已存在: {subject_id}");
            return Err(failure);
        }
        Ok(None) => {}
        Err(err) => {
            failure.reason = format!("查询人员编号失败: {err}");
            return Err(failure);
        }
    }

    // 串行许可：与在线单人录入共享同一 admission semaphore，
    // 避免批量导入与 Web 端并发录入互相看不到对方的在途样本。
    let _permit = gallery_index
        .acquire_enrollment_permit()
        .await
        .map_err(|err| {
            failure.reason = err;
            failure.clone()
        })?;

    let mut rollback_guard = DiskRollbackGuard::new();
    let mut extracted = Vec::with_capacity(photo_paths.len());

    for (index, photo_path) in photo_paths.iter().enumerate() {
        let is_primary = index == 0;
        let face_id = uuid::Uuid::now_v7().to_string();

        let raw = match tokio::fs::read(photo_path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                failure.kind = ImportFailureKind::Transcode;
                failure.reason = format!(
                    "读取照片失败 {}: {err}",
                    photo_path.file_name().unwrap_or_default().to_string_lossy()
                );
                return Err(failure);
            }
        };

        match process_and_save_face_bytes(
            evidence_base_dir,
            algo_registry,
            &subject_id,
            &face_id,
            raw,
            is_primary,
            &mut rollback_guard,
        )
        .await
        {
            Ok(meta) => extracted.push(meta),
            Err(err) => {
                // 该候选整体失败：保留守卫 armed，由 Drop 撤销本候选已落盘的照片。
                let (kind, reason) = classify_pipeline_error(&err);
                failure.kind = kind;
                failure.reason = reason;
                return Err(failure);
            }
        }
    }

    // 跨主体冲撞校验：批次串行处理保证前序候选已并入索引，可拦截同批次内撞脸
    for (index, meta) in extracted.iter().enumerate() {
        let clash = match gallery_index
            .most_similar_cross_subject(&meta.feature_bytes, Some(&subject_id))
            .await
        {
            Ok(clash) => clash,
            Err(err) => {
                failure.kind = ImportFailureKind::Internal;
                failure.reason = format!("无法完成底库冲撞校验，本次导入已中止: {err}");
                return Err(failure);
            }
        };

        if let Some(clash) =
            clash.filter(|candidate| candidate.raw_cosine >= ENROLLMENT_CLASH_RAW_COSINE_THRESHOLD)
        {
            failure.kind = ImportFailureKind::Clash;
            failure.reason = format!(
                "第 {} 张照片与底库人员「{}」样本高度相似: raw cosine {:.4} >= {:.2}",
                index + 1,
                clash.subject_name,
                clash.raw_cosine,
                ENROLLMENT_CLASH_RAW_COSINE_THRESHOLD
            );
            return Err(failure);
        }
    }

    // 单候选独立事务：落库失败只影响本候选
    let now = chrono::Utc::now();
    let primary_photo_path = extracted
        .first()
        .map(|meta| meta.photo_rel_path.clone())
        .unwrap_or_default();

    let txn = match db.begin().await {
        Ok(txn) => txn,
        Err(err) => {
            failure.reason = format!("开启数据库事务失败: {err}");
            return Err(failure);
        }
    };

    let person_model = db::entity::personnel::ActiveModel {
        id: sea_orm::NotSet,
        subject_id: sea_orm::Set(subject_id.clone()),
        name: sea_orm::Set(name.clone()),
        id_card: sea_orm::Set(id_card.trim().to_string()),
        remark: sea_orm::Set(remark.trim().to_string()),
        primary_photo_path: sea_orm::Set(primary_photo_path),
        created_at: sea_orm::Set(now),
        updated_at: sea_orm::Set(now),
    };

    let saved_person = match PersonnelRepo::insert(&txn, person_model).await {
        Ok(person) => person,
        Err(err) => {
            let _ = txn.rollback().await;
            failure.kind = ImportFailureKind::Conflict;
            failure.reason = format!("写入人员档案失败: {err}");
            return Err(failure);
        }
    };

    let mut registered_faces = Vec::with_capacity(extracted.len());
    for meta in extracted {
        let active_face = db::entity::gallery_face::ActiveModel {
            id: sea_orm::NotSet,
            face_id: sea_orm::Set(meta.face_id.clone()),
            subject_id: sea_orm::Set(subject_id.clone()),
            photo_rel_path: sea_orm::Set(meta.photo_rel_path.clone()),
            aligned_rel_path: sea_orm::Set(meta.aligned_rel_path.clone()),
            feature_vector: sea_orm::Set(meta.feature_bytes.clone()),
            quality_score: sea_orm::Set(meta.quality_score),
            detection_score: sea_orm::Set(meta.detection_score),
            is_primary: sea_orm::Set(if meta.is_primary { 1 } else { 0 }),
            created_at: sea_orm::Set(now),
        };

        let saved_face = match GalleryFaceRepo::insert(&txn, active_face).await {
            Ok(face) => face,
            Err(err) => {
                let _ = txn.rollback().await;
                failure.kind = ImportFailureKind::Internal;
                failure.reason = format!("写入人脸样本失败: {err}");
                return Err(failure);
            }
        };

        registered_faces.push(RegisteredFace::from_512_with_id(
            saved_face.id as u64,
            subject_id.clone(),
            name.clone(),
            meta.face_id,
            meta.photo_rel_path,
            meta.vector,
        ));
    }

    if let Err(err) = txn.commit().await {
        failure.kind = ImportFailureKind::Internal;
        failure.reason = format!("提交人员档案事务失败: {err}");
        return Err(failure);
    }
    rollback_guard.disarm();

    tracing::info!(
        subject_id = %saved_person.subject_id,
        faces = registered_faces.len(),
        skipped_photos,
        "批量导入人员档案成功"
    );

    gallery_index.upsert_faces(registered_faces).await;
    Ok(())
}

/// 把人脸提取流水线错误映射为导入报告中的归因分类与可读原因
///
/// 分类依据是**结构化错误类型**而非错误文案：`NoFaceDetected` 与
/// `ExtractionFailed` 都是「没检出脸」，`QualityLow` 才是「检出但质量不足」。
fn classify_pipeline_error(error: &ApiError) -> (ImportFailureKind, String) {
    match error {
        ApiError::BadRequest(message) => (ImportFailureKind::Transcode, message.clone()),
        other => {
            let kind = match other {
                ApiError::FaceQualityRejected(message) => {
                    if message.contains("未在上传照片中检测到有效人脸")
                        || message.contains("人脸特征提取失败")
                    {
                        ImportFailureKind::NoFace
                    } else {
                        ImportFailureKind::QualityLow
                    }
                }
                _ => ImportFailureKind::Internal,
            };
            (kind, other.to_string())
        }
    }
}
