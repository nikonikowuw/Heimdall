use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entity::task::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct TaskRepo;

impl TaskRepo {
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

    pub async fn save_or_update(
        db: &DatabaseConnection,
        camera_id: &str,
        name: &str,
        desired_enabled: bool,
        rules_json: &str,
        motion_gate_json: &str,
    ) -> Result<Model, DbError> {
        if let Some(model) = Self::find_by_camera_id(db, camera_id).await? {
            let mut active: ActiveModel = model.into();
            active.name = Set(name.to_string());
            active.desired_enabled = Set(desired_enabled);
            active.rules_json = Set(rules_json.to_string());
            active.motion_gate_json = Set(motion_gate_json.to_string());
            active.updated_at = Set(chrono::Utc::now());
            active.update(db).await.map_err(DbError::from)
        } else {
            let now = chrono::Utc::now();
            let active = ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                camera_id: Set(camera_id.to_string()),
                name: Set(name.to_string()),
                desired_enabled: Set(desired_enabled),
                actual_status: Set(0),
                rules_json: Set(rules_json.to_string()),
                motion_gate_json: Set(motion_gate_json.to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await.map_err(DbError::from)
        }
    }
}
