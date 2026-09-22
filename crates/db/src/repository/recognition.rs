use std::collections::HashSet;

use sea_orm::entity::prelude::DateTimeUtc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, FromQueryResult,
    JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};

use crate::entity::camera;
use crate::entity::recognition::{ActiveModel, Column, Entity, Model, Relation};
use crate::error::DbError;
use crate::repository::query::keyword_pattern;

#[derive(Debug, Clone, Default)]
pub struct UpdateRecognitionReviewParams<'a> {
    pub recognition_id: &'a str,
    pub status: &'a str,
    pub reviewer_id: Option<&'a str>,
    pub selected_subject_id: Option<&'a str>,
    pub selected_subject_name: Option<&'a str>,
    pub selected_photo_path: Option<&'a str>,
    pub selected_similarity: Option<f32>,
}

/// 识别列表/计数的过滤条件（各字段为 `None` 或空串表示不限制）。
#[derive(Debug, Clone, Default)]
pub struct RecognitionFilter<'a> {
    pub camera_id: Option<&'a str>,
    pub status: Option<&'a str>,
    /// 关键字，字面量匹配识别 ID、主体 ID/姓名、通道 ID 与通道名称
    pub keyword: Option<&'a str>,
    pub start_time: Option<DateTimeUtc>,
    pub end_time: Option<DateTimeUtc>,
}

fn build_filter_query(filter: RecognitionFilter<'_>) -> sea_orm::Select<Entity> {
    let mut query = Entity::find();
    if let Some(cid) = filter.camera_id.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::CameraId.eq(cid));
    }
    if let Some(st) = filter.status.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::Status.eq(st));
    }
    if let Some(pattern) = keyword_pattern(filter.keyword) {
        query = query
            .join(JoinType::LeftJoin, Relation::Camera.def())
            .filter(
                Condition::any()
                    .add(Column::RecognitionId.like(pattern.clone()))
                    .add(Column::CameraId.like(pattern.clone()))
                    .add(Column::SubjectId.like(pattern.clone()))
                    .add(Column::SubjectName.like(pattern.clone()))
                    .add(camera::Column::Name.like(pattern)),
            );
    }
    if let Some(start) = filter.start_time {
        query = query.filter(Column::RecognizedAt.gte(start));
    }
    if let Some(end) = filter.end_time {
        query = query.filter(Column::RecognizedAt.lte(end));
    }
    query
}

#[derive(Debug)]
pub struct RecognitionRepo;

impl RecognitionRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        Self::list_filtered(
            db,
            RecognitionFilter {
                camera_id,
                ..RecognitionFilter::default()
            },
            limit,
            offset,
        )
        .await
    }

    pub async fn list_filtered(
        db: &DatabaseConnection,
        filter: RecognitionFilter<'_>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        build_filter_query(filter)
            .order_by_desc(Column::RecognizedAt)
            .order_by_desc(Column::Id)
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_filtered(
        db: &DatabaseConnection,
        filter: RecognitionFilter<'_>,
    ) -> Result<u64, DbError> {
        build_filter_query(filter)
            .count(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn find_by_recognition_id(
        db: &DatabaseConnection,
        recognition_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::RecognitionId.eq(recognition_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn update_review_status(
        db: &DatabaseConnection,
        params: UpdateRecognitionReviewParams<'_>,
    ) -> Result<Option<Model>, DbError> {
        use sea_orm::Set;

        let Some(existing) = Self::find_by_recognition_id(db, params.recognition_id).await? else {
            return Ok(None);
        };

        // 幂等保护：若记录已由人工审核过且本次提交状态未变更，则保留原始审核时间，
        // 防止重复点击覆盖审核痕迹
        let has_reviewer = existing.reviewer_id.is_some();
        let same_status = params.status == existing.status.as_str();
        let should_update_time = !(has_reviewer && same_status);

        let mut active: ActiveModel = existing.into();
        active.status = Set(params.status.to_string());
        active.reviewer_id = Set(params.reviewer_id.map(|s| s.to_string()));
        if should_update_time {
            active.reviewed_at = Set(Some(chrono::Utc::now()));
        }

        if let Some(sub_id) = params.selected_subject_id {
            active.subject_id = Set(sub_id.to_string());
        }
        if let Some(sub_name) = params.selected_subject_name {
            active.subject_name = Set(sub_name.to_string());
        }
        if let Some(photo) = params.selected_photo_path {
            active.registered_photo_path = Set(photo.to_string());
        }
        if let Some(sim) = params.selected_similarity {
            active.similarity = Set(sim);
        }

        active.update(db).await?;
        Self::find_by_recognition_id(db, params.recognition_id).await
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    pub async fn count_all(db: &DatabaseConnection) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }

    /// 查询全部活跃识别记录关联的文件相对路径（现场特写）
    pub async fn find_all_active_image_paths(
        db: &DatabaseConnection,
    ) -> Result<HashSet<String>, DbError> {
        #[derive(FromQueryResult)]
        struct PathRow {
            field_crop_path: String,
        }
        let rows = Entity::find()
            .select_only()
            .column(Column::FieldCropPath)
            .into_model::<PathRow>()
            .all(db)
            .await?;
        let mut set = HashSet::new();
        for r in rows {
            if !r.field_crop_path.is_empty() {
                set.insert(r.field_crop_path);
            }
        }
        Ok(set)
    }

    pub async fn find_before(
        db: &DatabaseConnection,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::RecognizedAt.lt(before))
            .order_by_asc(Column::RecognizedAt)
            .order_by_asc(Column::Id)
            .limit(limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn find_oldest_batch(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::RecognizedAt)
            .order_by_asc(Column::Id)
            .limit(limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn delete_by_ids(db: &DatabaseConnection, ids: &[i64]) -> Result<u64, DbError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let res = Entity::delete_many()
            .filter(Column::Id.is_in(ids.to_vec()))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }
}
