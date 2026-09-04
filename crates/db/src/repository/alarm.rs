use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
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
        let mut query = Entity::find().order_by_desc(Column::OccurredAt);
        if let Some(cid) = camera_id {
            query = query.filter(Column::CameraId.eq(cid));
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
}
