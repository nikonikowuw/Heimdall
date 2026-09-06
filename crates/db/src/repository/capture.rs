use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

use crate::entity::capture::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct CaptureRepo;

impl CaptureRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        Self::list_filtered(db, camera_id, None, None, None, limit, offset).await
    }

    pub async fn list_filtered(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        target_label: Option<&str>,
        start_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
        end_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find().order_by_desc(Column::CapturedAt);
        if let Some(cid) = camera_id {
            query = query.filter(Column::CameraId.eq(cid));
        }
        if let Some(lbl) = target_label {
            query = query.filter(Column::TargetLabel.eq(lbl));
        }
        if let Some(start) = start_time {
            query = query.filter(Column::CapturedAt.gte(start));
        }
        if let Some(end) = end_time {
            query = query.filter(Column::CapturedAt.lte(end));
        }
        query
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
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
