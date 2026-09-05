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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_repo_lifecycle_and_deletion() {
        let db = crate::init_test_db().await.expect("init test db");

        // 先创建对应的摄像头设备
        let camera_model = crate::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-TEST-01".to_string()),
            name: Set("测试摄像头".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set("rtsp://127.0.0.1:8554/live".to_string()),
            sub_rtsp_url: Set("".to_string()),
            remark: Set("".to_string()),
            last_probe_status: Set("healthy".to_string()),
            last_probe_at: Set(None),
            last_probe_error_code: Set("".to_string()),
            last_success_at: Set(None),
            last_codec: Set("h264".to_string()),
            last_width: Set(1920),
            last_height: Set(1080),
            last_fps: Set(25.0),
            gb28181_device_id: Set(None),
            gb28181_channel_id: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        };
        crate::CameraRepo::insert(&db, camera_model)
            .await
            .expect("insert camera");

        // 插入或更新任务
        let saved = TaskRepo::save_or_update(
            &db,
            "CAM-TEST-01",
            "测试布防任务",
            true,
            "[]",
            r#"{"enabled":true}"#,
        )
        .await
        .expect("save task");

        assert_eq!(saved.camera_id, "CAM-TEST-01");
        assert_eq!(saved.name, "测试布防任务");
        assert!(saved.desired_enabled);

        // 查询任务
        let found = TaskRepo::find_by_camera_id(&db, "CAM-TEST-01")
            .await
            .expect("find task");
        assert!(found.is_some());

        // 删除任务
        let deleted_count = TaskRepo::delete_by_camera_id(&db, "CAM-TEST-01")
            .await
            .expect("delete task");
        assert_eq!(deleted_count, 1);

        // 验证已删除
        let after_delete = TaskRepo::find_by_camera_id(&db, "CAM-TEST-01")
            .await
            .expect("find after delete");
        assert!(after_delete.is_none());
    }
}
