use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entity::camera::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

/// 探活结果更新参数
#[derive(Debug, Clone)]
pub struct ProbeUpdateParams<'a> {
    pub status: &'a str,
    pub codec: &'a str,
    pub width: i32,
    pub height: i32,
    pub fps: f64,
    pub error_code: &'a str,
}

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
        params: ProbeUpdateParams<'_>,
    ) -> Result<(), DbError> {
        if let Some(model) = Self::find_by_camera_id(db, camera_id).await? {
            let mut active: ActiveModel = model.into();
            active.last_probe_status = Set(params.status.to_string());
            active.last_probe_at = Set(Some(chrono::Utc::now()));
            active.last_codec = Set(params.codec.to_string());
            active.last_width = Set(params.width);
            active.last_height = Set(params.height);
            active.last_fps = Set(params.fps);
            active.last_probe_error_code = Set(params.error_code.to_string());
            if params.status == "healthy" || params.status == "success" {
                active.last_success_at = Set(Some(chrono::Utc::now()));
            }
            active.updated_at = Set(chrono::Utc::now());
            active.update(db).await?;
        }
        Ok(())
    }
}
