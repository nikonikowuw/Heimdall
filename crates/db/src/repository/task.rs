use std::collections::HashMap;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, Statement,
    TransactionTrait,
};

use crate::entity::algorithm_instance::{
    ActiveModel as InstActiveModel, Column as InstColumn, Entity as InstEntity, Model as InstModel,
};
use crate::entity::task::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct SaveTaskWithInstancesParams {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub rules_json: String,
    pub motion_gate_json: String,
    pub status_message: Option<String>,
    pub instances: Option<Vec<SaveTaskAlgorithmInstanceParams>>,
}

#[derive(Debug, Clone)]
pub struct SaveTaskAlgorithmInstanceParams {
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub params_json: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct TaskInstanceStateUpdate {
    pub instance_id: String,
    pub actual_status: types::TaskStatus,
    pub status_message: String,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateTaskInstanceParams {
    pub analysis_fps: Option<i32>,
    pub params_json: Option<String>,
    pub rules_json: Option<String>,
    pub motion_gate_json: Option<String>,
    pub enabled: Option<bool>,
}

/// 兼容单算法参数结构
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

    /// 批量加载所有任务及其关联实例，返回 (tasks, instances_by_task_id)
    pub async fn list_all_with_instances(
        db: &DatabaseConnection,
    ) -> Result<(Vec<Model>, HashMap<i64, Vec<InstModel>>), DbError> {
        let tasks = Entity::find()
            .order_by_asc(Column::Id)
            .limit(1000)
            .all(db)
            .await?;
        let task_ids: Vec<i64> = tasks.iter().map(|t| t.id).collect();
        if task_ids.is_empty() {
            return Ok((tasks, HashMap::new()));
        }
        let instances = InstEntity::find()
            .filter(InstColumn::TaskId.is_in(task_ids.iter().copied()))
            .order_by_asc(InstColumn::Id)
            .limit(10_000)
            .all(db)
            .await?;
        let mut map: HashMap<i64, Vec<InstModel>> = HashMap::with_capacity(task_ids.len());
        for m in instances {
            map.entry(m.task_id).or_default().push(m);
        }
        Ok((tasks, map))
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

    async fn find_by_camera_id_txn(
        txn: &DatabaseTransaction,
        camera_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::CameraId.eq(camera_id))
            .one(txn)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_running(db: &DatabaseConnection) -> Result<u64, DbError> {
        let count = Entity::find()
            .filter(Column::ActualStatus.eq(types::TaskStatus::RUNNING))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }

    pub async fn list_instances_by_task_id(
        db: &DatabaseConnection,
        task_id: i64,
    ) -> Result<Vec<InstModel>, DbError> {
        InstEntity::find()
            .filter(InstColumn::TaskId.eq(task_id))
            .order_by_asc(InstColumn::Id)
            .limit(1000)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn list_instances_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<Vec<InstModel>, DbError> {
        InstEntity::find()
            .filter(InstColumn::CameraId.eq(camera_id))
            .order_by_asc(InstColumn::Id)
            .limit(1000)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn save_task_with_instances(
        db: &DatabaseConnection,
        params: SaveTaskWithInstancesParams,
    ) -> Result<Model, DbError> {
        db.transaction::<_, Model, DbError>(|txn| {
            Box::pin(async move { Self::save_task_with_instances_txn(txn, params).await })
        })
        .await
        .map_err(DbError::from)
    }

    async fn save_task_with_instances_txn(
        txn: &DatabaseTransaction,
        params: SaveTaskWithInstancesParams,
    ) -> Result<Model, DbError> {
        let task =
            if let Some(existing) = Self::find_by_camera_id_txn(txn, &params.camera_id).await? {
                let mut active: ActiveModel = existing.into();
                active.name = Set(params.name.clone());
                active.desired_enabled = Set(params.desired_enabled);
                if !params.desired_enabled {
                    active.actual_status = Set(types::TaskStatus::STOPPED);
                }
                active.rules_json = Set(params.rules_json.clone());
                active.motion_gate_json = Set(params.motion_gate_json.clone());
                if let Some(msg) = params.status_message {
                    active.status_message = Set(msg);
                }
                active.updated_at = Set(chrono::Utc::now());
                active.update(txn).await?
            } else {
                let now = chrono::Utc::now();
                let active = ActiveModel {
                    id: sea_orm::ActiveValue::NotSet,
                    camera_id: Set(params.camera_id.clone()),
                    name: Set(params.name.clone()),
                    desired_enabled: Set(params.desired_enabled),
                    actual_status: Set(types::TaskStatus::STOPPED),
                    rules_json: Set(params.rules_json.clone()),
                    motion_gate_json: Set(params.motion_gate_json.clone()),
                    status_message: Set(params.status_message.unwrap_or_default()),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                active.insert(txn).await?
            };

        // 一次性加载该任务的所有现有实例，避免 N+1 查询
        let existing_instances: HashMap<String, InstModel> = InstEntity::find()
            .filter(InstColumn::TaskId.eq(task.id))
            .limit(1000)
            .all(txn)
            .await?
            .into_iter()
            .map(|m| (m.algorithm_id.clone(), m))
            .collect();

        if let Some(instances) = params.instances {
            let domain_configs: Vec<types::TaskAlgorithmInstanceConfig> = instances
                .iter()
                .map(|ar| {
                    let parsed: serde_json::Value = serde_json::from_str(&ar.params_json)?;
                    Ok(types::TaskAlgorithmInstanceConfig {
                        instance_id: None,
                        algorithm_id: ar.algorithm_id.clone(),
                        analysis_fps: Some(ar.analysis_fps),
                        algo_params: Some(parsed),
                        enabled: ar.enabled,
                    })
                })
                .collect::<Result<Vec<_>, DbError>>()?;

            types::validate_task_algorithm_instances(&domain_configs)?;

            // 事务内批量检查算法包是否存在
            let algo_ids: Vec<&str> = instances.iter().map(|i| i.algorithm_id.as_str()).collect();
            if !algo_ids.is_empty() {
                let existing_algos: std::collections::HashSet<String> =
                    crate::entity::algorithm::Entity::find()
                        .filter(
                            crate::entity::algorithm::Column::AlgorithmId
                                .is_in(algo_ids.iter().copied()),
                        )
                        .limit(1000)
                        .all(txn)
                        .await?
                        .into_iter()
                        .map(|m| m.algorithm_id)
                        .collect();
                for ar in &instances {
                    if !existing_algos.contains(&ar.algorithm_id) {
                        return Err(DbError::NotFound {
                            entity: "algorithm",
                            key: ar.algorithm_id.clone(),
                        });
                    }
                }
            }

            let mut desired_ids = Vec::with_capacity(instances.len());

            for instance in &instances {
                let instance_enabled = if params.desired_enabled {
                    instance.enabled.unwrap_or(true)
                } else {
                    false
                };

                if let Some(model) = existing_instances.get(&instance.algorithm_id) {
                    desired_ids.push(model.id);
                    let mut active: InstActiveModel = model.clone().into();
                    active.analysis_fps = Set(instance.analysis_fps);
                    active.params_json = Set(instance.params_json.clone());
                    active.rules_json = Set(params.rules_json.clone());
                    active.motion_gate_json = Set(params.motion_gate_json.clone());
                    active.enabled = Set(instance_enabled);
                    active.camera_id = Set(params.camera_id.clone());
                    active.updated_at = Set(chrono::Utc::now());
                    active.update(txn).await?;
                } else {
                    let now = chrono::Utc::now();
                    let inserted = InstActiveModel {
                        id: sea_orm::ActiveValue::NotSet,
                        instance_id: Set(uuid::Uuid::new_v4().to_string()),
                        task_id: Set(task.id),
                        camera_id: Set(params.camera_id.clone()),
                        algorithm_id: Set(instance.algorithm_id.clone()),
                        analysis_fps: Set(instance.analysis_fps),
                        params_json: Set(instance.params_json.clone()),
                        rules_json: Set(params.rules_json.clone()),
                        motion_gate_json: Set(params.motion_gate_json.clone()),
                        enabled: Set(instance_enabled),
                        actual_status: Set(types::TaskStatus::STOPPED),
                        status_message: Set(String::new()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?;
                    desired_ids.push(inserted.id);
                }
            }

            let mut delete_query = InstEntity::delete_many().filter(InstColumn::TaskId.eq(task.id));
            if !desired_ids.is_empty() {
                delete_query = delete_query.filter(InstColumn::Id.is_not_in(desired_ids));
            }
            delete_query.exec(txn).await?;
        } else {
            // instances 为 None 时保留现有实例配置，同步更新启停状态与规则/门控镜像
            for (_, model) in existing_instances {
                let mut active: InstActiveModel = model.into();
                active.enabled = Set(params.desired_enabled);
                active.rules_json = Set(params.rules_json.clone());
                active.motion_gate_json = Set(params.motion_gate_json.clone());
                active.updated_at = Set(chrono::Utc::now());
                active.update(txn).await?;
            }
        }

        // 聚合持久化任务状态：从该任务的最新实例状态计算任务级 actual_status 并落库
        let task = Self::sync_task_actual_status_txn(txn, task.id).await?;

        Ok(task)
    }

    pub async fn update_task_runtime_state(
        db: &DatabaseConnection,
        camera_id: String,
        task_status: types::TaskStatus,
        task_message: String,
        instance_updates: Vec<TaskInstanceStateUpdate>,
    ) -> Result<Model, DbError> {
        db.transaction::<_, Model, DbError>(|txn| {
            Box::pin(async move {
                let task = Self::find_by_camera_id_txn(txn, &camera_id)
                    .await?
                    .ok_or_else(|| DbError::NotFound {
                        entity: "analysis_task",
                        key: camera_id.clone(),
                    })?;

                let mut active: ActiveModel = task.into();
                active.actual_status = Set(task_status.code());
                active.status_message = Set(task_message);
                active.updated_at = Set(chrono::Utc::now());
                let task = active.update(txn).await?;

                let instance_uuids: Vec<&str> = instance_updates
                    .iter()
                    .map(|u| u.instance_id.as_str())
                    .collect();
                let existing_instances: HashMap<String, InstModel> = InstEntity::find()
                    .filter(InstColumn::TaskId.eq(task.id))
                    .filter(InstColumn::InstanceId.is_in(instance_uuids.iter().copied()))
                    .limit(1000)
                    .all(txn)
                    .await?
                    .into_iter()
                    .map(|m| (m.instance_id.clone(), m))
                    .collect();

                for update in instance_updates {
                    let model = existing_instances.get(&update.instance_id).ok_or_else(|| {
                        DbError::NotFound {
                            entity: "algorithm_instance",
                            key: update.instance_id.clone(),
                        }
                    })?;
                    let mut active: InstActiveModel = model.clone().into();
                    active.actual_status = Set(update.actual_status.code());
                    active.status_message = Set(update.status_message);
                    active.updated_at = Set(chrono::Utc::now());
                    active.update(txn).await?;
                }

                Ok(task)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn add_instance_to_task(
        db: &DatabaseConnection,
        camera_id: &str,
        instance: SaveTaskAlgorithmInstanceParams,
    ) -> Result<InstModel, DbError> {
        let camera_id = camera_id.to_string();
        db.transaction::<_, InstModel, DbError>(|txn| {
            Box::pin(async move {
                let task = match Self::find_by_camera_id_txn(txn, &camera_id).await? {
                    Some(t) => t,
                    None => {
                        let now = chrono::Utc::now();
                        let active = ActiveModel {
                            id: sea_orm::ActiveValue::NotSet,
                            camera_id: Set(camera_id.clone()),
                            name: Set(format!("Task-{}", camera_id)),
                            desired_enabled: Set(false),
                            actual_status: Set(types::TaskStatus::Stopped.as_i32()),
                            status_message: Set(String::new()),
                            rules_json: Set("[]".to_string()),
                            motion_gate_json: Set("{}".to_string()),
                            created_at: Set(now),
                            updated_at: Set(now),
                        };
                        active.insert(txn).await?
                    }
                };

                let existing_instances = InstEntity::find()
                    .filter(InstColumn::TaskId.eq(task.id))
                    .limit(1000)
                    .all(txn)
                    .await?;

                if existing_instances
                    .iter()
                    .any(|i| i.algorithm_id == instance.algorithm_id)
                {
                    return Err(DbError::Type(types::TypeError::DuplicateAlgorithmId {
                        algorithm_id: instance.algorithm_id.clone(),
                    }));
                }

                let mut instances_params: Vec<SaveTaskAlgorithmInstanceParams> = existing_instances
                    .into_iter()
                    .map(|i| SaveTaskAlgorithmInstanceParams {
                        algorithm_id: i.algorithm_id,
                        analysis_fps: i.analysis_fps,
                        params_json: i.params_json,
                        enabled: Some(i.enabled),
                    })
                    .collect();
                instances_params.push(instance.clone());

                let saved_task = Self::save_task_with_instances_txn(
                    txn,
                    SaveTaskWithInstancesParams {
                        camera_id: camera_id.to_string(),
                        name: task.name,
                        desired_enabled: task.desired_enabled,
                        rules_json: task.rules_json,
                        motion_gate_json: task.motion_gate_json,
                        status_message: None,
                        instances: Some(instances_params),
                    },
                )
                .await?;

                let created = InstEntity::find()
                    .filter(InstColumn::TaskId.eq(saved_task.id))
                    .filter(InstColumn::AlgorithmId.eq(&instance.algorithm_id))
                    .one(txn)
                    .await?
                    .ok_or_else(|| DbError::NotFound {
                        entity: "algorithm_instance",
                        key: instance.algorithm_id,
                    })?;

                Ok(created)
            })
        })
        .await
        .map_err(DbError::from)
    }

    async fn sync_task_actual_status_txn(
        txn: &DatabaseTransaction,
        task_id: i64,
    ) -> Result<Model, DbError> {
        let task =
            Entity::find_by_id(task_id)
                .one(txn)
                .await?
                .ok_or_else(|| DbError::NotFound {
                    entity: "analysis_task",
                    key: task_id.to_string(),
                })?;

        let instances = InstEntity::find()
            .filter(InstColumn::TaskId.eq(task_id))
            .limit(1000)
            .all(txn)
            .await?;
        let instance_statuses: Vec<types::TaskStatus> = instances
            .iter()
            .filter_map(|m| types::TaskStatus::try_from(m.actual_status).ok())
            .collect();
        let aggregated =
            types::aggregate_task_instance_status(task.desired_enabled, &instance_statuses);

        let mut task_active: ActiveModel = task.into();
        task_active.actual_status = Set(aggregated.code());
        let updated = task_active.update(txn).await?;
        Ok(updated)
    }

    pub async fn update_instance_and_sync_task(
        db: &DatabaseConnection,
        instance_id: &str,
        params: UpdateTaskInstanceParams,
    ) -> Result<InstModel, DbError> {
        let instance_id = instance_id.to_string();
        db.transaction::<_, InstModel, DbError>(|txn| {
            Box::pin(async move {
                let model = InstEntity::find()
                    .filter(InstColumn::InstanceId.eq(&instance_id))
                    .one(txn)
                    .await?
                    .ok_or_else(|| DbError::NotFound {
                        entity: "algorithm_instance",
                        key: instance_id.clone(),
                    })?;

                let task_id = model.task_id;
                let mut active: InstActiveModel = model.into();
                if let Some(fps) = params.analysis_fps {
                    active.analysis_fps = Set(fps);
                }
                if let Some(pj) = params.params_json {
                    active.params_json = Set(pj);
                }
                if let Some(rj) = params.rules_json.as_ref() {
                    active.rules_json = Set(rj.clone());
                }
                if let Some(mg) = params.motion_gate_json.as_ref() {
                    active.motion_gate_json = Set(mg.clone());
                }
                if let Some(en) = params.enabled {
                    active.enabled = Set(en);
                }
                active.updated_at = Set(chrono::Utc::now());
                let updated = active.update(txn).await?;

                if params.rules_json.is_some() || params.motion_gate_json.is_some() {
                    if let Some(task_model) = Entity::find_by_id(task_id).one(txn).await? {
                        let mut task_active: ActiveModel = task_model.into();
                        if let Some(rj) = params.rules_json {
                            task_active.rules_json = Set(rj);
                        }
                        if let Some(mg) = params.motion_gate_json {
                            task_active.motion_gate_json = Set(mg);
                        }
                        task_active.updated_at = Set(chrono::Utc::now());
                        task_active.update(txn).await?;
                    }
                }

                Self::sync_task_actual_status_txn(txn, task_id).await?;

                Ok(updated)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn set_instance_enabled_and_sync_task(
        db: &DatabaseConnection,
        instance_id: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        Self::update_instance_and_sync_task(
            db,
            instance_id,
            UpdateTaskInstanceParams {
                enabled: Some(enabled),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    pub async fn delete_instance_and_sync_task(
        db: &DatabaseConnection,
        instance_id: &str,
    ) -> Result<u64, DbError> {
        let instance_id = instance_id.to_string();
        db.transaction::<_, u64, DbError>(|txn| {
            Box::pin(async move {
                let model = InstEntity::find()
                    .filter(InstColumn::InstanceId.eq(&instance_id))
                    .one(txn)
                    .await?
                    .ok_or_else(|| DbError::NotFound {
                        entity: "algorithm_instance",
                        key: instance_id.clone(),
                    })?;

                let task_id = model.task_id;
                let res = InstEntity::delete_many()
                    .filter(InstColumn::TaskId.eq(task_id))
                    .filter(InstColumn::InstanceId.eq(&instance_id))
                    .exec(txn)
                    .await?;

                Self::sync_task_actual_status_txn(txn, task_id).await?;

                Ok(res.rows_affected)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn delete_task_with_instances(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        let camera_id = camera_id.to_string();
        db.transaction::<_, u64, DbError>(|txn| {
            Box::pin(async move {
                let Some(task) = Self::find_by_camera_id_txn(txn, &camera_id).await? else {
                    return Ok(0);
                };

                InstEntity::delete_many()
                    .filter(InstColumn::TaskId.eq(task.id))
                    .exec(txn)
                    .await?;

                Entity::delete_by_id(task.id).exec(txn).await?;

                let remaining = InstEntity::find()
                    .filter(InstColumn::TaskId.eq(task.id))
                    .count(txn)
                    .await?;
                if remaining > 0 {
                    return Err(DbError::Query(sea_orm::DbErr::Custom(format!(
                        "删除任务 {} 失败：仍残留 {} 个算法实例",
                        task.id, remaining
                    ))));
                }

                Ok(1)
            })
        })
        .await
        .map_err(DbError::from)
    }

    // ── 兼容单算法调用桥接 ────────────────────────────────────────────────

    /// 兼容接口：保存任务并同步单个算法实例
    pub async fn save_task_and_sync_instance(
        db: &DatabaseConnection,
        params: SaveTaskParams,
    ) -> Result<Model, DbError> {
        let instances = if params.algorithm_id.trim().is_empty() {
            None
        } else {
            Some(vec![SaveTaskAlgorithmInstanceParams {
                algorithm_id: params.algorithm_id,
                analysis_fps: params.analysis_fps,
                params_json: params.algo_params_json,
                enabled: Some(params.desired_enabled),
            }])
        };

        Self::save_task_with_instances(
            db,
            SaveTaskWithInstancesParams {
                camera_id: params.camera_id,
                name: params.name,
                desired_enabled: params.desired_enabled,
                rules_json: params.rules_json,
                motion_gate_json: params.motion_gate_json,
                status_message: None,
                instances,
            },
        )
        .await
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
        Self::save_task_with_instances(
            db,
            SaveTaskWithInstancesParams {
                camera_id: camera_id.to_string(),
                name: name.to_string(),
                desired_enabled,
                rules_json: rules_json.to_string(),
                motion_gate_json: motion_gate_json.to_string(),
                status_message: None,
                instances: None,
            },
        )
        .await
    }

    /// 兼容接口：更新任务状态（同时同步其所有关联实例）
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
                    let mut active: ActiveModel = task.clone().into();
                    active.actual_status = Set(actual_status);
                    active.status_message = Set(msg.clone());
                    active.updated_at = Set(now);
                    active.update(txn).await?;

                    // 批量同步所有关联算法实例状态，避免逐条 UPDATE 的 N+1 开销
                    txn.execute(Statement::from_sql_and_values(
                        sea_orm::DatabaseBackend::Sqlite,
                        "UPDATE algorithm_instances SET actual_status = ?, status_message = ?, updated_at = ? WHERE task_id = ?",
                        [
                            actual_status.into(),
                            msg.clone().into(),
                            now.into(),
                            task.id.into(),
                        ],
                    ))
                    .await?;
                }
                Ok(())
            })
        })
        .await
        .map_err(DbError::from)
    }

    /// 兼容接口：删除任务与关联算法实例
    pub async fn delete_task_and_instance(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        Self::delete_task_with_instances(db, camera_id).await
    }

    /// 兼容接口：根据 cameraId 删除任务
    pub async fn delete_by_camera_id(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<u64, DbError> {
        Self::delete_task_with_instances(db, camera_id).await
    }
}
