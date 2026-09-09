use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set, TransactionTrait,
};

use crate::entity::algorithm::{Column as AlgoColumn, Entity as AlgoEntity};
use crate::entity::algorithm_instance::{
    ActiveModel as InstActiveModel, Column as InstColumn, Entity as InstEntity,
};
use crate::entity::task::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct SaveTaskParams {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub algo_params_json: String,
    pub rules_json: String,
    pub motion_gate_json: String,
}

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

    /// 原子保存任务并同步关联主算法实例
    pub async fn save_task_and_sync_instance(
        db: &DatabaseConnection,
        params: SaveTaskParams,
    ) -> Result<Model, DbError> {
        // 1. 业务参数基础校验
        if params.analysis_fps < 0 {
            return Err(DbError::Validation(
                "analysis_fps 必须大于等于 0".to_string(),
            ));
        }

        let trimmed_algo_params = params.algo_params_json.trim();
        let valid_algo_params_json = if trimmed_algo_params.is_empty() {
            "{}".to_string()
        } else {
            let parsed: serde_json::Value = serde_json::from_str(trimmed_algo_params)
                .map_err(|e| DbError::Validation(format!("algo_params_json 格式错误: {e}")))?;
            if !parsed.is_object() {
                return Err(DbError::Validation(
                    "algo_params_json 必须为 JSON Object 对象".to_string(),
                ));
            }
            trimmed_algo_params.to_string()
        };

        // 2. 开启事务保证 Task ↔ AlgorithmInstance 双写原子性
        db.transaction::<_, Model, DbError>(|txn| {
            Box::pin(async move {
                // 3. 算法有效性校验：若提供了 algorithm_id，必须确保该算法在 algorithms 表中存在
                if !params.algorithm_id.is_empty() {
                    let algo_exists = AlgoEntity::find()
                        .filter(AlgoColumn::AlgorithmId.eq(&params.algorithm_id))
                        .one(txn)
                        .await?;

                    if algo_exists.is_none() {
                        return Err(DbError::NotFound {
                            entity: "algorithm",
                            key: params.algorithm_id.clone(),
                        });
                    }
                }

                // 4. 保存或更新 analysis_tasks
                let existing_task = Entity::find()
                    .filter(Column::CameraId.eq(&params.camera_id))
                    .one(txn)
                    .await?;

                let saved_task = if let Some(task) = existing_task {
                    let mut active: ActiveModel = task.into();
                    active.name = Set(params.name.clone());
                    active.desired_enabled = Set(params.desired_enabled);
                    active.algorithm_id = Set(params.algorithm_id.clone());
                    active.analysis_fps = Set(params.analysis_fps);
                    active.algo_params_json = Set(valid_algo_params_json.clone());
                    active.rules_json = Set(params.rules_json.clone());
                    active.motion_gate_json = Set(params.motion_gate_json.clone());
                    active.updated_at = Set(chrono::Utc::now());
                    active.update(txn).await.map_err(DbError::from)?
                } else {
                    let now = chrono::Utc::now();
                    let active = ActiveModel {
                        id: sea_orm::ActiveValue::NotSet,
                        camera_id: Set(params.camera_id.clone()),
                        name: Set(params.name.clone()),
                        desired_enabled: Set(params.desired_enabled),
                        actual_status: Set(0),
                        status_message: Set(String::new()),
                        algorithm_id: Set(params.algorithm_id.clone()),
                        analysis_fps: Set(params.analysis_fps),
                        algo_params_json: Set(valid_algo_params_json.clone()),
                        rules_json: Set(params.rules_json.clone()),
                        motion_gate_json: Set(params.motion_gate_json.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    active.insert(txn).await.map_err(DbError::from)?
                };

                // 5. 同步主算法实例 (AlgorithmInstance)
                let existing_inst = InstEntity::find()
                    .filter(InstColumn::CameraId.eq(&params.camera_id))
                    .one(txn)
                    .await?;

                if let Some(inst) = existing_inst {
                    let mut inst_active: InstActiveModel = inst.into();
                    if !params.algorithm_id.is_empty() {
                        inst_active.algorithm_id = Set(params.algorithm_id.clone());
                    }
                    inst_active.analysis_fps = Set(params.analysis_fps);
                    inst_active.params_json = Set(valid_algo_params_json);
                    inst_active.rules_json = Set(params.rules_json.clone());
                    inst_active.motion_gate_json = Set(params.motion_gate_json.clone());
                    inst_active.enabled = Set(params.desired_enabled);
                    inst_active.updated_at = Set(chrono::Utc::now());
                    inst_active.update(txn).await.map_err(DbError::from)?;
                } else if !params.algorithm_id.is_empty() {
                    let now = chrono::Utc::now();
                    let new_inst = InstActiveModel {
                        id: sea_orm::ActiveValue::NotSet,
                        instance_id: Set(uuid::Uuid::new_v4().to_string()),
                        camera_id: Set(params.camera_id.clone()),
                        algorithm_id: Set(params.algorithm_id.clone()),
                        analysis_fps: Set(params.analysis_fps),
                        params_json: Set(valid_algo_params_json),
                        rules_json: Set(params.rules_json.clone()),
                        motion_gate_json: Set(params.motion_gate_json.clone()),
                        enabled: Set(params.desired_enabled),
                        actual_status: Set(0),
                        status_message: Set(String::new()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_inst.insert(txn).await.map_err(DbError::from)?;
                }

                Ok(saved_task)
            })
        })
        .await
        .map_err(DbError::from)
    }

    /// 兼容旧版调用入口
    pub async fn save_or_update(
        db: &DatabaseConnection,
        camera_id: &str,
        name: &str,
        desired_enabled: bool,
        rules_json: &str,
        motion_gate_json: &str,
    ) -> Result<Model, DbError> {
        let existing = Self::find_by_camera_id(db, camera_id).await?;
        let (algorithm_id, analysis_fps, algo_params_json) = if let Some(ref m) = existing {
            (
                m.algorithm_id.clone(),
                m.analysis_fps,
                m.algo_params_json.clone(),
            )
        } else {
            (String::new(), 0, "{}".to_string())
        };

        Self::save_task_and_sync_instance(
            db,
            SaveTaskParams {
                camera_id: camera_id.to_string(),
                name: name.to_string(),
                desired_enabled,
                algorithm_id,
                analysis_fps,
                algo_params_json,
                rules_json: rules_json.to_string(),
                motion_gate_json: motion_gate_json.to_string(),
            },
        )
        .await
    }

    /// 原子同步任务与主算法实例的运行状态及错误描述
    pub async fn update_status(
        db: &DatabaseConnection,
        camera_id: &str,
        actual_status: i32,
        status_message: &str,
    ) -> Result<(), DbError> {
        let cid = camera_id.to_string();
        let msg = status_message.to_string();

        db.transaction::<_, (), DbError>(|txn| {
            Box::pin(async move {
                let now = chrono::Utc::now();
                if let Some(task) = Entity::find()
                    .filter(Column::CameraId.eq(&cid))
                    .one(txn)
                    .await?
                {
                    let mut active: ActiveModel = task.into();
                    active.actual_status = Set(actual_status);
                    active.status_message = Set(msg.clone());
                    active.updated_at = Set(now);
                    active.update(txn).await?;
                }

                // 同步对应主算法实例
                if let Some(inst) = InstEntity::find()
                    .filter(InstColumn::CameraId.eq(&cid))
                    .one(txn)
                    .await?
                {
                    let mut active: InstActiveModel = inst.into();
                    active.actual_status = Set(actual_status);
                    active.status_message = Set(msg);
                    active.updated_at = Set(now);
                    active.update(txn).await?;
                }

                Ok(())
            })
        })
        .await
        .map_err(DbError::from)
    }

    /// 原子删除任务与关联算法实例
    pub async fn delete_task_and_instance(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        let cid = camera_id.to_string();

        db.transaction::<_, u64, DbError>(|txn| {
            Box::pin(async move {
                let _ = InstEntity::delete_many()
                    .filter(InstColumn::CameraId.eq(&cid))
                    .exec(txn)
                    .await?;

                let res = Entity::delete_many()
                    .filter(Column::CameraId.eq(&cid))
                    .exec(txn)
                    .await?;

                Ok(res.rows_affected)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn delete_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        Self::delete_task_and_instance(db, camera_id).await
    }

    pub async fn count_running(db: &DatabaseConnection) -> Result<u64, DbError> {
        let count = Entity::find()
            .filter(Column::ActualStatus.eq(types::TaskStatus::Running as i32))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
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
        assert_eq!(saved.algorithm_id, "");
        assert_eq!(saved.analysis_fps, 0);

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
