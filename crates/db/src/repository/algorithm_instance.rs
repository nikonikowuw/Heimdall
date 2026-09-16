use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
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
            // 新建实例尚无运行时收敛历史，期望与实际天然相等（代际 0）。
            desired_revision: Set(0),
            applied_revision: Set(0),
            runtime_apply_state: Set(types::InstanceApplyState::Applied.as_i32()),
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

    /// 回写运行时配置收敛成功：实际代际追上期望代际并置为 applied。
    ///
    /// 仅当调用方持有的 `desired_revision == applied_revision` 时才代表已收敛；
    /// 并发提交的更高代际不会被本次结果覆盖（由 SQL 条件保证幂等与不倒退）。
    pub async fn mark_apply_applied(
        db: &DatabaseConnection,
        instance_id: &str,
        applied_revision: i64,
    ) -> Result<Option<Model>, DbError> {
        let Some(model) = Self::find_by_instance_id(db, instance_id).await? else {
            return Ok(None);
        };
        if model.desired_revision != applied_revision {
            // 更高代际已在排队：本次结果已经过时，保持 pending 交由后续收敛处理。
            return Ok(Some(model));
        }

        let previous_state = model.runtime_apply_state;
        let mut active: ActiveModel = model.into();
        active.applied_revision = Set(applied_revision);
        active.runtime_apply_state = Set(types::InstanceApplyState::Applied.as_i32());
        if types::InstanceApplyState::from_i32(previous_state)
            == Some(types::InstanceApplyState::Failed)
        {
            // 失败原因属于已收敛的旧代际，收敛成功后清理，避免长期显示过期错误。
            active.status_message = Set(String::new());
        }
        active.updated_at = Set(chrono::Utc::now());
        let updated = active.update(db).await?;
        Ok(Some(updated))
    }

    /// 回写运行时仍在收敛：保留 pending 状态，并把等待原因透出给控制面。
    ///
    /// 仅在期望代际尚未变化时写入，避免把已过时的等待原因覆盖到新代际上。
    pub async fn mark_apply_pending(
        db: &DatabaseConnection,
        instance_id: &str,
        desired_revision: i64,
        reason: &str,
    ) -> Result<Option<Model>, DbError> {
        let Some(model) = Self::find_by_instance_id(db, instance_id).await? else {
            return Ok(None);
        };
        if model.desired_revision != desired_revision {
            return Ok(Some(model));
        }

        let mut active: ActiveModel = model.into();
        active.runtime_apply_state = Set(types::InstanceApplyState::Pending.as_i32());
        active.status_message = Set(reason.to_string());
        active.updated_at = Set(chrono::Utc::now());
        let updated = active.update(db).await?;
        Ok(Some(updated))
    }

    /// 回写运行时配置应用失败：保留期望代际，只记录失败状态与可展示原因。
    ///
    /// 旧 Worker 继续使用上一份已生效配置，因此 `applied_revision` 不回退。
    pub async fn mark_apply_failed(
        db: &DatabaseConnection,
        instance_id: &str,
        reason: &str,
    ) -> Result<Option<Model>, DbError> {
        let Some(model) = Self::find_by_instance_id(db, instance_id).await? else {
            return Ok(None);
        };

        let mut active: ActiveModel = model.into();
        active.runtime_apply_state = Set(types::InstanceApplyState::Failed.as_i32());
        active.status_message = Set(reason.to_string());
        active.updated_at = Set(chrono::Utc::now());
        let updated = active.update(db).await?;
        Ok(Some(updated))
    }

    /// 列出期望配置尚未在运行时收敛的实例（服务重启后的恢复入口）
    ///
    /// 不变量：`runtime_apply_state` 非 `Applied` 当且仅当 `desired_revision != applied_revision`；
    /// 回写路径只在期望代际已追上时才置回 `applied`，因此单列过滤不会漏掉待收敛实例。
    pub async fn list_unapplied(db: &DatabaseConnection) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::RuntimeApplyState.ne(types::InstanceApplyState::Applied.as_i32()))
            .order_by_asc(Column::Id)
            .limit(1000)
            .all(db)
            .await
            .map_err(DbError::from)
    }
}
