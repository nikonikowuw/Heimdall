use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::entity::recognition::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct RecognitionRepo;

impl RecognitionRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find().order_by_desc(Column::RecognizedAt);
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

    pub async fn find_oldest_batch(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::RecognizedAt)
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
