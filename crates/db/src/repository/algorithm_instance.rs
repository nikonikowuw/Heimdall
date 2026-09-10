use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};

use crate::entity::algorithm_instance::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct CreateInstanceParams {
    pub task_id: i64,
    pub instance_id: String,
    pub camera_id: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub params_json: String,
    pub rules_json: String,
    pub motion_gate_json: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateInstanceParams {
    pub analysis_fps: Option<i32>,
    pub params_json: Option<String>,
    pub rules_json: Option<String>,
    pub motion_gate_json: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug)]
pub struct AlgorithmInstanceRepo;

impl AlgorithmInstanceRepo {
    /// 查询指定摄像头的算法实例列表
    pub async fn list_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::CameraId.eq(camera_id))
            .order_by_asc(Column::Id)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 查询所有算法实例
    pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::Id)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 根据唯一实例 UUID 获取实例
    pub async fn find_by_instance_id(
        db: &DatabaseConnection,
        instance_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::InstanceId.eq(instance_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 根据 task_id 与 algorithm_id 查询唯一算法实例
    pub async fn find_by_task_id_and_algorithm_id(
        db: &DatabaseConnection,
        task_id: i64,
        algorithm_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::TaskId.eq(task_id))
            .filter(Column::AlgorithmId.eq(algorithm_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 创建新的算法实例
    pub async fn create(
        db: &DatabaseConnection,
        params: CreateInstanceParams,
    ) -> Result<Model, DbError> {
        let now = chrono::Utc::now();
        let active = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            instance_id: Set(params.instance_id),
            task_id: Set(params.task_id),
            camera_id: Set(params.camera_id),
            algorithm_id: Set(params.algorithm_id),
            analysis_fps: Set(params.analysis_fps),
            params_json: Set(params.params_json),
            rules_json: Set(params.rules_json),
            motion_gate_json: Set(params.motion_gate_json),
            enabled: Set(params.enabled),
            actual_status: Set(types::TaskStatus::STOPPED),
            status_message: Set(String::new()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(db).await.map_err(DbError::from)
    }

    /// 更新算法实例参数与规则
    pub async fn update(
        db: &DatabaseConnection,
        instance_id: &str,
        params: UpdateInstanceParams,
    ) -> Result<Model, DbError> {
        let model = Self::find_by_instance_id(db, instance_id)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "algorithm_instance",
                key: instance_id.to_string(),
            })?;

        let mut active: ActiveModel = model.into();
        if let Some(fps) = params.analysis_fps {
            active.analysis_fps = Set(fps);
        }
        if let Some(pj) = params.params_json {
            active.params_json = Set(pj);
        }
        if let Some(rj) = params.rules_json {
            active.rules_json = Set(rj);
        }
        if let Some(mg) = params.motion_gate_json {
            active.motion_gate_json = Set(mg);
        }
        if let Some(en) = params.enabled {
            active.enabled = Set(en);
        }
        active.updated_at = Set(chrono::Utc::now());

        active.update(db).await.map_err(DbError::from)
    }

    /// 切换启停状态
    pub async fn set_enabled(
        db: &DatabaseConnection,
        instance_id: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        let model = Self::find_by_instance_id(db, instance_id)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "algorithm_instance",
                key: instance_id.to_string(),
            })?;

        let mut active: ActiveModel = model.into();
        active.enabled = Set(enabled);
        active.updated_at = Set(chrono::Utc::now());
        active.update(db).await?;
        Ok(())
    }

    /// 删除算法实例（要求校验任务 ID）
    pub async fn delete_by_task_id_and_instance_id(
        db: &DatabaseConnection,
        task_id: i64,
        instance_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::TaskId.eq(task_id))
            .filter(Column::InstanceId.eq(instance_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    /// 统计指定算法启用的实例数
    pub async fn count_active_by_algo(
        db: &DatabaseConnection,
        algorithm_id: &str,
    ) -> Result<u64, DbError> {
        Entity::find()
            .filter(Column::AlgorithmId.eq(algorithm_id))
            .filter(Column::Enabled.eq(true))
            .count(db)
            .await
            .map_err(DbError::from)
    }
}
