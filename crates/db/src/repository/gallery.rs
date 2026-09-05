use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::entity::gallery::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct GalleryRepo;

impl GalleryRepo {
    pub async fn list_by_gallery(
        db: &DatabaseConnection,
        gallery_id: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::GalleryId.eq(gallery_id))
            .order_by_desc(Column::CreatedAt)
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn find_by_subject_id(
        db: &DatabaseConnection,
        subject_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::SubjectId.eq(subject_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    pub async fn delete_by_subject_id(
        db: &DatabaseConnection,
        subject_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::SubjectId.eq(subject_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }
}
