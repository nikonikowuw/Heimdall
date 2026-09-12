use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use db::entity::gallery_face::ActiveModel as FaceActiveModel;
use db::entity::personnel::ActiveModel as PersonnelActiveModel;
use db::{DatabaseConnection, GalleryFaceRepo, PersonnelRepo};
use sea_orm::ActiveValue::Set;
use sea_orm::TransactionTrait;
use types::{
    GalleryFaceDto, PersonnelDetailDto, PersonnelItemDto, PersonnelStatsDto, UpdatePersonnelRequest,
};

use crate::error::ApiError;
use crate::gallery_index::{
    embedding_to_le_bytes, le_bytes_to_embedding, FaceFeatureIndex, RegisteredFace,
};
use crate::state::AppState;

/// 磁盘写操作异常回滚守卫（RAII：若在提交前发生异常或 panic，自动销毁孤儿临时文件）
struct DiskRollbackGuard {
    paths: Vec<PathBuf>,
    disarmed: bool,
}

impl DiskRollbackGuard {
    fn new() -> Self {
        Self {
            paths: Vec::new(),
            disarmed: false,
        }
    }

    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    fn disarm(mut self) {
        self.disarmed = true;
    }
}

impl Drop for DiskRollbackGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            for path in &self.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// 内部处理产出的人脸特征及磁盘文件元数据
struct ExtractedFaceMeta {
    face_id: String,
    photo_rel_path: String,
    aligned_rel_path: String,
    feature_bytes: Vec<u8>,
    vector: [f32; 512],
    quality_score: f32,
    detection_score: f32,
    is_primary: bool,
}

/// 人员底库与人脸特征领域服务
#[derive(Debug, Clone)]
pub struct PersonnelService {
    db: DatabaseConnection,
    evidence_base_dir: PathBuf,
    algo_registry: Arc<infer::package::AlgoRegistry>,
    gallery_index: Arc<FaceFeatureIndex>,
}

impl PersonnelService {
    pub fn new(
        db: DatabaseConnection,
        evidence_base_dir: PathBuf,
        algo_registry: Arc<infer::package::AlgoRegistry>,
        gallery_index: Arc<FaceFeatureIndex>,
    ) -> Self {
        Self {
            db,
            evidence_base_dir,
            algo_registry,
            gallery_index,
        }
    }

    pub fn from_state(state: &AppState) -> Self {
        Self::new(
            state.db.clone(),
            state
                .pipeline
                .snapshot_engine()
                .base_evidence_dir()
                .to_path_buf(),
            state.algo_registry.clone(),
            state.gallery_index.clone(),
        )
    }

    /// 分页查询人员档案列表
    pub async fn list(
        &self,
        keyword: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<(Vec<PersonnelItemDto>, u64), ApiError> {
        let (records, total) =
            PersonnelRepo::list_filtered(&self.db, keyword, limit, offset).await?;

        // 批量加载人脸数量，避免 N+1 查询
        let subject_ids: Vec<&str> = records.iter().map(|p| p.subject_id.as_str()).collect();
        let face_counts =
            GalleryFaceRepo::count_batch_by_subject_ids(&self.db, &subject_ids).await?;

        let mut items = Vec::with_capacity(records.len());
        for p in records {
            let count = face_counts.get(&p.subject_id).copied().unwrap_or(0);
            items.push(PersonnelItemDto {
                id: p.id,
                subject_id: p.subject_id,
                name: p.name,
                id_card: p.id_card,
                remark: p.remark,
                primary_photo_path: p.primary_photo_path,
                face_count: count as u32,
                created_at: p.created_at.timestamp_millis(),
                updated_at: p.updated_at.timestamp_millis(),
            });
        }

        Ok((items, total))
    }

    /// 获取人员底库统计
    pub async fn get_stats(&self) -> Result<PersonnelStatsDto, ApiError> {
        let total_personnel = PersonnelRepo::count_all(&self.db).await?;
        let total_faces = GalleryFaceRepo::count_all(&self.db).await?;
        let algo_ready = self.algo_registry.is_face_extraction_ready().await;

        Ok(PersonnelStatsDto {
            total_personnel,
            total_faces,
            algo_ready,
        })
    }

    /// 获取单个人员详细档案（含所有人脸样本）
    pub async fn get_detail(&self, subject_id: &str) -> Result<PersonnelDetailDto, ApiError> {
        let person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        let faces = GalleryFaceRepo::list_by_subject_id(&self.db, subject_id).await?;
        let face_dtos: Vec<GalleryFaceDto> = faces.into_iter().map(GalleryFaceDto::from).collect();

        Ok(PersonnelDetailDto {
            id: person.id,
            subject_id: person.subject_id,
            name: person.name,
            id_card: person.id_card,
            remark: person.remark,
            primary_photo_path: person.primary_photo_path,
            faces: face_dtos,
            created_at: person.created_at.timestamp_millis(),
            updated_at: person.updated_at.timestamp_millis(),
        })
    }

    /// 一步式原子录入人员与 1~5 张人脸照片
    pub async fn create(
        &self,
        name: String,
        mut custom_subject_id: Option<String>,
        id_card: String,
        remark: String,
        raw_images: Vec<Vec<u8>>,
    ) -> Result<PersonnelDetailDto, ApiError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(ApiError::BadRequest("人员姓名不能为空".to_string()));
        }

        if raw_images.is_empty() {
            return Err(ApiError::BadRequest(
                "至少需要上传 1 张有效人脸照片".to_string(),
            ));
        }
        if raw_images.len() > 5 {
            return Err(ApiError::BadRequest(
                "单个人员最多支持上传 5 张人脸照片".to_string(),
            ));
        }

        let subject_id = match custom_subject_id.take().map(|s| s.trim().to_string()) {
            Some(s) if !s.is_empty() => {
                if PersonnelRepo::find_by_subject_id(&self.db, &s)
                    .await?
                    .is_some()
                {
                    return Err(ApiError::BadRequest(format!("人员编号已存在: {s}")));
                }
                s
            }
            _ => format!("sub_{}", uuid::Uuid::new_v4().simple()),
        };

        if !self.algo_registry.is_face_extraction_ready().await {
            return Err(ApiError::FaceAlgorithmNotLoaded(
                "人脸识别算法包未就绪，无法提取特征，请先部署人脸算法".to_string(),
            ));
        }

        // 1. 逐张进行特征提取与磁盘落盘（受回滚守卫保护）
        let mut rollback_guard = DiskRollbackGuard::new();
        let mut extracted_metas = Vec::with_capacity(raw_images.len());
        let mut primary_photo_path = String::new();

        for (idx, raw_img) in raw_images.into_iter().enumerate() {
            let is_primary = idx == 0;
            let face_id = format!("face_{}", uuid::Uuid::new_v4().simple());
            let meta = self
                .process_and_save_face(
                    &subject_id,
                    &face_id,
                    raw_img,
                    is_primary,
                    &mut rollback_guard,
                )
                .await?;

            if is_primary {
                primary_photo_path = meta.photo_rel_path.clone();
            }
            extracted_metas.push(meta);
        }

        // 2. 数据库事务原子写入人员表与人脸表
        let now = chrono::Utc::now();
        let txn = self.db.begin().await.map_err(ApiError::from)?;

        let person_model = PersonnelActiveModel {
            id: sea_orm::NotSet,
            subject_id: Set(subject_id.clone()),
            name: Set(name.clone()),
            id_card: Set(id_card.trim().to_string()),
            remark: Set(remark.trim().to_string()),
            primary_photo_path: Set(primary_photo_path.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let saved_person = PersonnelRepo::insert(&txn, person_model).await?;

        let mut registered_faces_for_index = Vec::with_capacity(extracted_metas.len());
        let mut face_dtos = Vec::with_capacity(extracted_metas.len());

        for meta in extracted_metas {
            let active_face = FaceActiveModel {
                id: sea_orm::NotSet,
                face_id: Set(meta.face_id.clone()),
                subject_id: Set(subject_id.clone()),
                photo_rel_path: Set(meta.photo_rel_path.clone()),
                aligned_rel_path: Set(meta.aligned_rel_path.clone()),
                feature_vector: Set(meta.feature_bytes),
                quality_score: Set(meta.quality_score),
                detection_score: Set(meta.detection_score),
                is_primary: Set(if meta.is_primary { 1 } else { 0 }),
                created_at: Set(now),
            };
            let saved_face = GalleryFaceRepo::insert(&txn, active_face).await?;

            registered_faces_for_index.push(RegisteredFace {
                subject_id: subject_id.clone(),
                subject_name: name.clone(),
                face_id: meta.face_id,
                photo_rel_path: meta.photo_rel_path,
                vector: meta.vector,
            });
            face_dtos.push(GalleryFaceDto::from(saved_face));
        }

        txn.commit().await.map_err(ApiError::from)?;
        rollback_guard.disarm();

        // 3. 增量原子更新底库特征内存索引
        self.gallery_index
            .upsert_faces(registered_faces_for_index)
            .await;

        Ok(PersonnelDetailDto {
            id: saved_person.id,
            subject_id: saved_person.subject_id,
            name: saved_person.name,
            id_card: saved_person.id_card,
            remark: saved_person.remark,
            primary_photo_path: saved_person.primary_photo_path,
            faces: face_dtos,
            created_at: saved_person.created_at.timestamp_millis(),
            updated_at: saved_person.updated_at.timestamp_millis(),
        })
    }

    /// 更新人员基础信息
    pub async fn update(
        &self,
        subject_id: &str,
        req: UpdatePersonnelRequest,
    ) -> Result<PersonnelDetailDto, ApiError> {
        let person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        let mut active: PersonnelActiveModel = person.into();
        let mut name_changed_to: Option<String> = None;

        if let Some(name) = req.name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(ApiError::BadRequest("姓名不能为空".to_string()));
            }
            active.name = Set(trimmed.to_string());
            name_changed_to = Some(trimmed.to_string());
        }
        if let Some(id_card) = req.id_card {
            active.id_card = Set(id_card.trim().to_string());
        }
        if let Some(remark) = req.remark {
            active.remark = Set(remark.trim().to_string());
        }
        active.updated_at = Set(chrono::Utc::now());

        let updated = PersonnelRepo::update(&self.db, active).await?;

        if let Some(new_name) = name_changed_to {
            self.gallery_index
                .update_subject_name(subject_id, &new_name)
                .await;
        }

        let faces = GalleryFaceRepo::list_by_subject_id(&self.db, subject_id).await?;
        let face_dtos: Vec<GalleryFaceDto> = faces.into_iter().map(GalleryFaceDto::from).collect();

        Ok(PersonnelDetailDto {
            id: updated.id,
            subject_id: updated.subject_id,
            name: updated.name,
            id_card: updated.id_card,
            remark: updated.remark,
            primary_photo_path: updated.primary_photo_path,
            faces: face_dtos,
            created_at: updated.created_at.timestamp_millis(),
            updated_at: updated.updated_at.timestamp_millis(),
        })
    }

    /// 物理删除人员及其关联的所有样本照与数据库记录（单事务原子删除）
    pub async fn delete(&self, subject_id: &str) -> Result<(), ApiError> {
        let person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        // 1. 事务内级联删除数据库记录
        let txn = self.db.begin().await.map_err(ApiError::from)?;
        GalleryFaceRepo::delete_by_subject_id(&txn, &person.subject_id).await?;
        PersonnelRepo::delete_by_subject_id(&txn, &person.subject_id).await?;
        txn.commit().await.map_err(ApiError::from)?;

        // 2. 物理清除磁盘目录: var/data/evidence/galleries/{subject_id}/
        let person_disk_dir = self
            .evidence_base_dir
            .join("galleries")
            .join(&person.subject_id);
        if person_disk_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&person_disk_dir).await;
        }

        // 3. 增量清除内存索引
        self.gallery_index.remove_subject(&person.subject_id).await;

        Ok(())
    }

    /// 追加照片样本
    pub async fn add_faces(
        &self,
        subject_id: &str,
        raw_images: Vec<Vec<u8>>,
    ) -> Result<PersonnelDetailDto, ApiError> {
        let person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        let current_count = GalleryFaceRepo::count_by_subject_id(&self.db, subject_id).await?;
        if raw_images.is_empty() {
            return Err(ApiError::BadRequest("未选择要追加的照片".to_string()));
        }
        if current_count + raw_images.len() as u64 > 5 {
            return Err(ApiError::BadRequest(format!(
                "当前已有 {current_count} 张照片，追加后超出 5 张上限限制"
            )));
        }

        if !self.algo_registry.is_face_extraction_ready().await {
            return Err(ApiError::FaceAlgorithmNotLoaded(
                "人脸识别算法包未就绪，无法提取特征，请先部署人脸算法".to_string(),
            ));
        }

        let mut rollback_guard = DiskRollbackGuard::new();
        let mut extracted_metas = Vec::with_capacity(raw_images.len());

        for raw_img in raw_images {
            let face_id = format!("face_{}", uuid::Uuid::new_v4().simple());
            let meta = self
                .process_and_save_face(subject_id, &face_id, raw_img, false, &mut rollback_guard)
                .await?;
            extracted_metas.push(meta);
        }

        let now = chrono::Utc::now();
        let txn = self.db.begin().await.map_err(ApiError::from)?;
        let mut new_registered_faces = Vec::with_capacity(extracted_metas.len());

        for meta in extracted_metas {
            let active_face = FaceActiveModel {
                id: sea_orm::NotSet,
                face_id: Set(meta.face_id.clone()),
                subject_id: Set(subject_id.to_string()),
                photo_rel_path: Set(meta.photo_rel_path.clone()),
                aligned_rel_path: Set(meta.aligned_rel_path.clone()),
                feature_vector: Set(meta.feature_bytes),
                quality_score: Set(meta.quality_score),
                detection_score: Set(meta.detection_score),
                is_primary: Set(0),
                created_at: Set(now),
            };
            GalleryFaceRepo::insert(&txn, active_face).await?;

            new_registered_faces.push(RegisteredFace {
                subject_id: subject_id.to_string(),
                subject_name: person.name.clone(),
                face_id: meta.face_id,
                photo_rel_path: meta.photo_rel_path,
                vector: meta.vector,
            });
        }

        txn.commit().await.map_err(ApiError::from)?;
        rollback_guard.disarm();

        self.gallery_index.upsert_faces(new_registered_faces).await;

        self.get_detail(subject_id).await
    }

    /// 删除单张人脸特征样本
    pub async fn delete_face(
        &self,
        subject_id: &str,
        face_id: &str,
    ) -> Result<PersonnelDetailDto, ApiError> {
        let _person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        let faces = GalleryFaceRepo::list_by_subject_id(&self.db, subject_id).await?;
        if faces.len() <= 1 {
            return Err(ApiError::BadRequest(
                "人员至少必须保留 1 张有效人脸照片，若需清理请直接删除该人员".to_string(),
            ));
        }

        let target_face = faces
            .iter()
            .find(|f| f.face_id == face_id)
            .ok_or_else(|| ApiError::NotFound(format!("照片样本不存在: {face_id}")))?;

        let was_primary = target_face.is_primary != 0;
        let photo_path = self.evidence_base_dir.join(&target_face.photo_rel_path);
        let aligned_path = if target_face.aligned_rel_path.is_empty() {
            None
        } else {
            Some(self.evidence_base_dir.join(&target_face.aligned_rel_path))
        };

        // 1. 事务内执行删除与主头像平移
        let txn = self.db.begin().await.map_err(ApiError::from)?;
        GalleryFaceRepo::delete_by_face_id(&txn, face_id).await?;

        let remaining_faces = GalleryFaceRepo::list_by_subject_id(&txn, subject_id).await?;
        if was_primary && !remaining_faces.is_empty() {
            let new_primary = &remaining_faces[0];
            GalleryFaceRepo::set_primary(&txn, subject_id, &new_primary.face_id).await?;
            PersonnelRepo::update_primary_photo(&txn, subject_id, &new_primary.photo_rel_path)
                .await?;
        }
        txn.commit().await.map_err(ApiError::from)?;

        // 2. 清理磁盘文件
        let _ = tokio::fs::remove_file(photo_path).await;
        if let Some(p) = aligned_path {
            let _ = tokio::fs::remove_file(p).await;
        }

        // 3. 增量移除内存索引
        self.gallery_index.remove_face(face_id).await;

        self.get_detail(subject_id).await
    }

    /// 设置某张人脸为主头像
    pub async fn set_primary_face(
        &self,
        subject_id: &str,
        face_id: &str,
    ) -> Result<PersonnelDetailDto, ApiError> {
        let _person = PersonnelRepo::find_by_subject_id(&self.db, subject_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("人员不存在: {subject_id}")))?;

        let target_face = GalleryFaceRepo::find_by_face_id(&self.db, face_id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("照片样本不存在: {face_id}")))?;

        if target_face.subject_id != subject_id {
            return Err(ApiError::BadRequest("照片样本不属于该人员".to_string()));
        }

        let txn = self.db.begin().await.map_err(ApiError::from)?;
        GalleryFaceRepo::set_primary(&txn, subject_id, face_id).await?;
        PersonnelRepo::update_primary_photo(&txn, subject_id, &target_face.photo_rel_path).await?;
        txn.commit().await.map_err(ApiError::from)?;

        self.get_detail(subject_id).await
    }

    /// 提取单张人脸特征并落盘
    async fn process_and_save_face(
        &self,
        subject_id: &str,
        face_id: &str,
        raw_img: Vec<u8>,
        is_primary: bool,
        rollback_guard: &mut DiskRollbackGuard,
    ) -> Result<ExtractedFaceMeta, ApiError> {
        let jpeg_bytes = ensure_jpeg_bytes_async(raw_img).await?;

        // 调用人脸算法提取特征（底层已通过 spawn_blocking 隔离 C ABI）
        let extraction = match self.algo_registry.extract_face(&jpeg_bytes).await {
            Ok(ext) => ext,
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("NO_FACE_DETECTED") {
                    return Err(ApiError::FaceQualityRejected(
                        "未在上传照片中检测到有效人脸，请上传正面清晰免冠照".to_string(),
                    ));
                }
                return Err(ApiError::FaceQualityRejected(format!(
                    "人脸特征提取失败: {msg}"
                )));
            }
        };

        // 质量门禁校验 (>= 0.50)
        if extraction.quality_score < 0.50 {
            return Err(ApiError::FaceQualityRejected(format!(
                "人脸质量评分过低 ({:.2})，未满足 0.50 门禁要求，请上传光线充足的正面照片",
                extraction.quality_score
            )));
        }

        // 目录规划: var/data/evidence/galleries/{subject_id}/
        let subject_dir = self.evidence_base_dir.join("galleries").join(subject_id);
        tokio::fs::create_dir_all(&subject_dir)
            .await
            .map_err(|e| ApiError::Internal(format!("创建底库存储目录失败: {e}")))?;

        let photo_rel_path = format!("galleries/{subject_id}/original_{face_id}.jpg");
        let aligned_rel_path = format!("galleries/{subject_id}/aligned_{face_id}.jpg");

        let photo_abs = self.evidence_base_dir.join(&photo_rel_path);
        let aligned_abs = self.evidence_base_dir.join(&aligned_rel_path);

        // 写入原始 JPEG 并纳入回滚保护
        tokio::fs::write(&photo_abs, &jpeg_bytes)
            .await
            .map_err(|e| ApiError::Internal(format!("写入原始照片失败: {e}")))?;
        rollback_guard.track(photo_abs);

        // 写入 112x112 对齐切片
        let aligned_data = if !extraction.aligned_jpeg.is_empty() {
            &extraction.aligned_jpeg
        } else {
            &jpeg_bytes
        };
        tokio::fs::write(&aligned_abs, aligned_data)
            .await
            .map_err(|e| ApiError::Internal(format!("写入对齐人脸切片失败: {e}")))?;
        rollback_guard.track(aligned_abs);

        // 小端字节序打包 512 维 FP32 向量 (2048 字节)
        let feature_bytes = embedding_to_le_bytes(&extraction.embedding);
        let vector = le_bytes_to_embedding(&feature_bytes).unwrap_or([0.0f32; 512]);

        Ok(ExtractedFaceMeta {
            face_id: face_id.to_string(),
            photo_rel_path,
            aligned_rel_path,
            feature_bytes,
            vector,
            quality_score: extraction.quality_score,
            detection_score: extraction.detection_score,
            is_primary,
        })
    }
}

/// 异步调用阻塞线程池保证图片转码为标准 JPEG
async fn ensure_jpeg_bytes_async(raw: Vec<u8>) -> Result<Vec<u8>, ApiError> {
    if raw.len() >= 2 && raw[0] == 0xFF && raw[1] == 0xD8 {
        return Ok(raw);
    }
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&raw)
            .map_err(|e| ApiError::BadRequest(format!("不支持的图片格式或文件损坏: {e}")))?;
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
            .map_err(|e| ApiError::Internal(format!("转码为 JPEG 失败: {e}")))?;
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("图片转码任务调度异常: {e}")))?
}
