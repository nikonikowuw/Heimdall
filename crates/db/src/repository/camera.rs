use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entity::camera::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct CameraRepo;

impl CameraRepo {
    pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<Model>, DbError> {
        Entity::find().all(db).await.map_err(DbError::from)
    }

    pub async fn find_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::CameraId.eq(camera_id))
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

    pub async fn delete_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::CameraId.eq(camera_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn update_probe_status(
        db: &DatabaseConnection,
        camera_id: &str,
        status: &str,
        codec: &str,
        width: i32,
        height: i32,
        fps: f64,
    ) -> Result<(), DbError> {
        if let Some(model) = Self::find_by_camera_id(db, camera_id).await? {
            let mut active: ActiveModel = model.into();
            active.last_probe_status = Set(status.to_string());
            active.last_probe_at = Set(Some(chrono::Utc::now()));
            active.last_codec = Set(codec.to_string());
            active.last_width = Set(width);
            active.last_height = Set(height);
            active.last_fps = Set(fps);
            active.updated_at = Set(chrono::Utc::now());
            active.update(db).await?;
        }
        Ok(())
    }
}
