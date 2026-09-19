use std::collections::HashSet;

use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::capture::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct CaptureRepo;

/// 抓拍列表/计数的过滤条件（各字段为 `None` 或空串表示不限制）。
///
/// 收拢为结构体而不是继续加函数参数：过滤维度每加一个，调用点的位置参数就会错位一次，
/// 而 `track_id` 与 `target_label` 都是 `Option`，类型系统无法帮忙发现传串了。
#[derive(Debug, Clone, Default)]
pub struct CaptureFilter<'a> {
    pub camera_id: Option<&'a str>,
    pub target_label: Option<&'a str>,
    /// 轨道过滤：`track_id` 只在单机位追踪器内唯一，调用方必须同时限定 `camera_id`。
    pub track_id: Option<i64>,
    pub start_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
    pub end_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
}

fn build_filter_query(filter: CaptureFilter<'_>) -> sea_orm::Select<Entity> {
    let mut query = Entity::find();
    if let Some(cid) = filter.camera_id.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::CameraId.eq(cid));
    }
    if let Some(lbl) = filter.target_label.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::TargetLabel.eq(lbl));
    }
    // 轨道过滤走 `idx_capture_records_track(camera_id, track_id)`：
    // 「同一个人一次通行」的多次结算共用同一个 track_id，这是行迹回溯的唯一定位键。
    if let Some(tid) = filter.track_id {
        query = query.filter(Column::TrackId.eq(tid));
    }
    if let Some(start) = filter.start_time {
        query = query.filter(Column::CapturedAt.gte(start));
    }
    if let Some(end) = filter.end_time {
        query = query.filter(Column::CapturedAt.lte(end));
    }
    query
}

impl CaptureRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        Self::list_filtered(
            db,
            CaptureFilter {
                camera_id,
                ..CaptureFilter::default()
            },
            limit,
            offset,
        )
        .await
    }

    pub async fn list_filtered(
        db: &DatabaseConnection,
        filter: CaptureFilter<'_>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        build_filter_query(filter)
            .order_by_desc(Column::CapturedAt)
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_filtered(
        db: &DatabaseConnection,
        filter: CaptureFilter<'_>,
    ) -> Result<u64, DbError> {
        build_filter_query(filter)
            .count(db)
            .await
            .map_err(DbError::from)
    }

    /// 查询全部活跃抓拍记录关联的文件相对路径（全景 + 人脸特写 + 人体特写）
    pub async fn find_all_active_image_paths(
        db: &DatabaseConnection,
    ) -> Result<HashSet<String>, DbError> {
        #[derive(FromQueryResult)]
        struct PathRow {
            image_rel_path: String,
            crop_image_rel_path: String,
            body_crop_image_rel_path: String,
        }
        let rows = Entity::find()
            .select_only()
            .column(Column::ImageRelPath)
            .column(Column::CropImageRelPath)
            .column(Column::BodyCropImageRelPath)
            .into_model::<PathRow>()
            .all(db)
            .await?;
        let mut set = HashSet::new();
        for r in rows {
            if !r.image_rel_path.is_empty() {
                set.insert(r.image_rel_path);
            }
            if !r.crop_image_rel_path.is_empty() {
                set.insert(r.crop_image_rel_path);
            }
            if !r.body_crop_image_rel_path.is_empty() {
                set.insert(r.body_crop_image_rel_path);
            }
        }
        Ok(set)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    /// 高频抓拍攒批写入（避免逐事件频繁开启单行 SQLite 事务，内置 ON CONFLICT DO NOTHING 幂等防重）
    pub async fn insert_batch(
        db: &DatabaseConnection,
        active_models: Vec<ActiveModel>,
    ) -> Result<usize, DbError> {
        if active_models.is_empty() {
            return Ok(0);
        }
        let count = active_models.len();
        Entity::insert_many(active_models)
            .on_conflict(
                OnConflict::column(Column::CaptureId)
                    .do_nothing()
                    .to_owned(),
            )
            .exec(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }

    pub async fn find_oldest_batch(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::CapturedAt)
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

    pub async fn delete_by_capture_id(
        db: &DatabaseConnection,
        capture_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::CaptureId.eq(capture_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn count_all(db: &DatabaseConnection) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }

    pub async fn find_before(
        db: &DatabaseConnection,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::CapturedAt.lt(before))
            .order_by_asc(Column::CapturedAt)
            .limit(limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_since(
        db: &DatabaseConnection,
        since: chrono::NaiveDateTime,
    ) -> Result<u64, DbError> {
        let count = Entity::find()
            .filter(Column::CapturedAt.gte(since))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }
}
