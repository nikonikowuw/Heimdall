use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

use crate::entity::alarm::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct AlarmRepo;

impl AlarmRepo {
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
        status: Option<&str>,
        start_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
        end_time: Option<sea_orm::entity::prelude::DateTimeUtc>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find().order_by_desc(Column::OccurredAt);
        if let Some(cid) = camera_id {
            query = query.filter(Column::CameraId.eq(cid));
        }
        if let Some(st) = status {
            query = query.filter(Column::Status.eq(st));
        }
        if let Some(start) = start_time {
            query = query.filter(Column::OccurredAt.gte(start));
        }
        if let Some(end) = end_time {
            query = query.filter(Column::OccurredAt.lte(end));
        }
        query
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn update_status(
        db: &DatabaseConnection,
        id: i64,
        status: &str,
    ) -> Result<Model, DbError> {
        let model = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "alarm_records",
                key: id.to_string(),
            })?;

        let mut active: ActiveModel = model.into();
        active.status = sea_orm::Set(status.to_string());
        active.handled_at = sea_orm::Set(Some(chrono::Utc::now()));
        active.update(db).await.map_err(DbError::from)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    pub async fn delete_by_event_id(
        db: &DatabaseConnection,
        event_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::EventId.eq(event_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn find_oldest_batch(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::OccurredAt)
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

    pub async fn count_all(db: &DatabaseConnection) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }

    pub async fn find_before(
        db: &DatabaseConnection,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::CreatedAt.lt(before))
            .order_by_asc(Column::CreatedAt)
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
            .filter(Column::CreatedAt.gte(since))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }
}
