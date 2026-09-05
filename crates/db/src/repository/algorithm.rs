use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::entity::algorithm::{
    ActiveModel as AlgoActiveModel, Column as AlgoColumn, Entity as AlgoEntity, Model as AlgoModel,
};
use crate::entity::algorithm_instance::{Column as InstColumn, Entity as InstEntity};
use crate::entity::algorithm_version::{
    ActiveModel as VerActiveModel, Column as VerColumn, Entity as VerEntity, Model as VerModel,
};
use crate::error::DbError;

#[derive(Debug, Clone)]
pub struct UpsertAlgorithmParams {
    pub algorithm_id: String,
    pub name: String,
    pub algorithm_type: String,
    pub alarm_type_id: String,
    pub active_version: String,
    pub description: String,
    pub is_builtin: bool,
}

#[derive(Debug, Clone)]
pub struct UpsertVersionParams {
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub min_adapter_version: String,
    pub package_root: String,
    pub fps_tiers: String,
    pub config_schema: String,
    pub manifest_raw: String,
    pub package_size_bytes: i64,
    pub is_active: bool,
    pub is_builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmStats {
    pub total_algorithms: u64,
    pub total_active_versions: u64,
    pub builtin_algorithms: u64,
    pub custom_algorithms: u64,
}

#[derive(Debug)]
pub struct AlgorithmRepo;

impl AlgorithmRepo {
    /// 分页查询算法列表（支持关键字搜索、算法类型和内置状态过滤）
    pub async fn list_algorithms(
        db: &DatabaseConnection,
        page: u64,
        page_size: u64,
        keyword: Option<&str>,
        algorithm_type: Option<&str>,
        is_builtin: Option<bool>,
    ) -> Result<(Vec<AlgoModel>, u64), DbError> {
        let mut query = AlgoEntity::find();

        if let Some(kw) = keyword {
            let kw_trimmed = kw.trim();
            if !kw_trimmed.is_empty() {
                query = query.filter(
                    AlgoColumn::AlgorithmId
                        .contains(kw_trimmed)
                        .or(AlgoColumn::Name.contains(kw_trimmed)),
                );
            }
        }

        if let Some(at) = algorithm_type {
            let at_trimmed = at.trim();
            if !at_trimmed.is_empty() {
                query = query.filter(AlgoColumn::AlgorithmType.eq(at_trimmed));
            }
        }

        if let Some(builtin) = is_builtin {
            query = query.filter(AlgoColumn::IsBuiltin.eq(builtin));
        }

        query = query.order_by_desc(AlgoColumn::Id);

        let paginator = query.paginate(db, page_size.max(1));
        let total = paginator.num_items().await?;
        let page_idx = page.saturating_sub(1);
        let items = paginator.fetch_page(page_idx).await?;

        Ok((items, total))
    }

    /// 根据数字 ID 获取算法详情
    pub async fn find_by_id(
        db: &DatabaseConnection,
        id: i64,
    ) -> Result<Option<AlgoModel>, DbError> {
        AlgoEntity::find_by_id(id)
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 根据 algorithm_id 获取算法主表记录
    pub async fn find_by_algorithm_id(
        db: &DatabaseConnection,
        algorithm_id: &str,
    ) -> Result<Option<AlgoModel>, DbError> {
        AlgoEntity::find()
            .filter(AlgoColumn::AlgorithmId.eq(algorithm_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 获取算法下的所有版本明细
    pub async fn list_versions_by_algorithm_id(
        db: &DatabaseConnection,
        algorithm_id: &str,
    ) -> Result<Vec<VerModel>, DbError> {
        VerEntity::find()
            .filter(VerColumn::AlgorithmId.eq(algorithm_id))
            .order_by_desc(VerColumn::Id)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 获取特定算法的具体版本
    pub async fn find_version(
        db: &DatabaseConnection,
        algorithm_id: &str,
        version: &str,
        platform_id: Option<&str>,
    ) -> Result<Option<VerModel>, DbError> {
        let mut query = VerEntity::find()
            .filter(VerColumn::AlgorithmId.eq(algorithm_id))
            .filter(VerColumn::Version.eq(version));

        if let Some(pid) = platform_id {
            query = query.filter(VerColumn::PlatformId.eq(pid));
        }

        query.one(db).await.map_err(DbError::from)
    }

    /// 新增或更新算法主表记录
    pub async fn upsert_algorithm(
        db: &DatabaseConnection,
        params: UpsertAlgorithmParams,
    ) -> Result<AlgoModel, DbError> {
        let now = chrono::Utc::now();
        if let Some(existing) = Self::find_by_algorithm_id(db, &params.algorithm_id).await? {
            let mut active: AlgoActiveModel = existing.into();
            active.name = Set(params.name);
            active.algorithm_type = Set(params.algorithm_type);
            active.alarm_type_id = Set(params.alarm_type_id);
            if !params.active_version.is_empty() {
                active.active_version = Set(params.active_version);
            }
            active.description = Set(params.description);
            active.is_builtin = Set(params.is_builtin);
            active.updated_at = Set(now);
            active.update(db).await.map_err(DbError::from)
        } else {
            let active = AlgoActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                algorithm_id: Set(params.algorithm_id),
                name: Set(params.name),
                algorithm_type: Set(params.algorithm_type),
                alarm_type_id: Set(params.alarm_type_id),
                active_version: Set(params.active_version),
                description: Set(params.description),
                is_builtin: Set(params.is_builtin),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await.map_err(DbError::from)
        }
    }

    /// 新增或更新算法版本明细
    pub async fn upsert_version(
        db: &DatabaseConnection,
        params: UpsertVersionParams,
    ) -> Result<VerModel, DbError> {
        let now = chrono::Utc::now();
        let existing = VerEntity::find()
            .filter(VerColumn::AlgorithmId.eq(&params.algorithm_id))
            .filter(VerColumn::Version.eq(&params.version))
            .filter(VerColumn::PlatformId.eq(&params.platform_id))
            .one(db)
            .await?;

        if let Some(m) = existing {
            let mut active: VerActiveModel = m.into();
            active.min_adapter_version = Set(params.min_adapter_version);
            active.package_root = Set(params.package_root);
            active.fps_tiers = Set(params.fps_tiers);
            active.config_schema = Set(params.config_schema);
            active.manifest_raw = Set(params.manifest_raw);
            active.package_size_bytes = Set(params.package_size_bytes);
            active.is_active = Set(params.is_active);
            active.is_builtin = Set(params.is_builtin);
            active.updated_at = Set(now);
            active.update(db).await.map_err(DbError::from)
        } else {
            let active = VerActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                algorithm_id: Set(params.algorithm_id),
                version: Set(params.version),
                platform_id: Set(params.platform_id),
                min_adapter_version: Set(params.min_adapter_version),
                package_root: Set(params.package_root),
                fps_tiers: Set(params.fps_tiers),
                config_schema: Set(params.config_schema),
                manifest_raw: Set(params.manifest_raw),
                package_size_bytes: Set(params.package_size_bytes),
                is_active: Set(params.is_active),
                is_builtin: Set(params.is_builtin),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await.map_err(DbError::from)
        }
    }

    /// 在事务中激活指定版本（将同算法的其他版本置为非激活，并更新主表 active_version）
    pub async fn activate_version(
        db: &DatabaseConnection,
        algorithm_id: &str,
        version: &str,
        platform_id: Option<&str>,
    ) -> Result<(), DbError> {
        let mut query = VerEntity::find()
            .filter(VerColumn::AlgorithmId.eq(algorithm_id))
            .filter(VerColumn::Version.eq(version));

        if let Some(pid) = platform_id {
            query = query.filter(VerColumn::PlatformId.eq(pid));
        }

        let target = query
            .order_by_desc(VerColumn::Id)
            .one(db)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "algorithm_version",
                key: format!("{algorithm_id}:{version}"),
            })?;

        let aid = algorithm_id.to_string();
        let target_id = target.id;
        let ver = version.to_string();

        db.transaction::<_, (), DbError>(|txn| {
            Box::pin(async move {
                let now = chrono::Utc::now();
                // 1. 将该算法所有版本置为非激活，仅激活目标版本
                let versions = VerEntity::find()
                    .filter(VerColumn::AlgorithmId.eq(&aid))
                    .all(txn)
                    .await?;

                for v in versions {
                    let is_target = v.id == target_id;
                    if v.is_active != is_target {
                        let mut active: VerActiveModel = v.into();
                        active.is_active = Set(is_target);
                        active.updated_at = Set(now);
                        active.update(txn).await?;
                    }
                }

                // 2. 更新主表 active_version
                if let Some(algo) = AlgoEntity::find()
                    .filter(AlgoColumn::AlgorithmId.eq(&aid))
                    .one(txn)
                    .await?
                {
                    let mut active: AlgoActiveModel = algo.into();
                    active.active_version = Set(ver);
                    active.updated_at = Set(now);
                    active.update(txn).await?;
                }

                Ok(())
            })
        })
        .await?;

        Ok(())
    }

    /// 统计指定算法当前正在运行（enabled=1）的实例数
    pub async fn count_active_instances(
        db: &DatabaseConnection,
        algorithm_id: &str,
    ) -> Result<u64, DbError> {
        InstEntity::find()
            .filter(InstColumn::AlgorithmId.eq(algorithm_id))
            .filter(InstColumn::Enabled.eq(true))
            .count(db)
            .await
            .map_err(DbError::from)
    }

    /// 安全卸载指定算法版本：
    /// 1. 检查内置保护
    /// 2. 检查使用中保护
    /// 3. 在事务中删除版本记录，若无剩余版本则删除主表，若删除的是当前活跃版本则自动回退
    /// 4. 返回物理包路径以便清理磁盘
    pub async fn uninstall_version(
        db: &DatabaseConnection,
        algorithm_id: &str,
        version: &str,
        platform_id: Option<&str>,
    ) -> Result<String, DbError> {
        let mut query = VerEntity::find()
            .filter(VerColumn::AlgorithmId.eq(algorithm_id))
            .filter(VerColumn::Version.eq(version));

        if let Some(pid) = platform_id {
            query = query.filter(VerColumn::PlatformId.eq(pid));
        }

        let ver_model = query
            .order_by_desc(VerColumn::Id)
            .one(db)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "algorithm_version",
                key: format!("{algorithm_id}:{version}"),
            })?;

        if ver_model.is_builtin {
            return Err(DbError::BuiltinAlgoProtected(format!(
                "算法版本 {algorithm_id}:{version} 为系统内置预置包，禁止删除"
            )));
        }

        let active_count = Self::count_active_instances(db, algorithm_id).await?;
        if active_count > 0 {
            return Err(DbError::AlgoInUse(format!(
                "算法 {algorithm_id} 正在被 {active_count} 个启用的实例使用，禁止卸载"
            )));
        }

        let package_root = ver_model.package_root.clone();
        let aid = algorithm_id.to_string();
        let ver = version.to_string();

        db.transaction::<_, (), DbError>(|txn| {
            Box::pin(async move {
                let now = chrono::Utc::now();
                // 删除版本行
                VerEntity::delete_by_id(ver_model.id).exec(txn).await?;

                // 检查剩余版本
                let remaining = VerEntity::find()
                    .filter(VerColumn::AlgorithmId.eq(&aid))
                    .order_by_desc(VerColumn::Id)
                    .all(txn)
                    .await?;

                if remaining.is_empty() {
                    // 已无任何版本，删除算法主表
                    AlgoEntity::delete_many()
                        .filter(AlgoColumn::AlgorithmId.eq(&aid))
                        .exec(txn)
                        .await?;
                } else {
                    // 若删除的是当前激活版本，将剩余的最新版本设为激活
                    if let Some(algo) = AlgoEntity::find()
                        .filter(AlgoColumn::AlgorithmId.eq(&aid))
                        .one(txn)
                        .await?
                    {
                        if algo.active_version == ver {
                            let next_active = &remaining[0];
                            let next_ver = next_active.version.clone();

                            let mut active_ver: VerActiveModel = next_active.clone().into();
                            active_ver.is_active = Set(true);
                            active_ver.updated_at = Set(now);
                            active_ver.update(txn).await?;

                            let mut active_algo: AlgoActiveModel = algo.into();
                            active_algo.active_version = Set(next_ver);
                            active_algo.updated_at = Set(now);
                            active_algo.update(txn).await?;
                        }
                    }
                }

                Ok(())
            })
        })
        .await?;

        Ok(package_root)
    }

    /// 列出所有当前激活的版本（供启动时装载至 AlgoRegistry）
    pub async fn list_active_versions(db: &DatabaseConnection) -> Result<Vec<VerModel>, DbError> {
        VerEntity::find()
            .filter(VerColumn::IsActive.eq(true))
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 统计全局算法指标
    pub async fn stats(db: &DatabaseConnection) -> Result<AlgorithmStats, DbError> {
        let total_algorithms = AlgoEntity::find().count(db).await?;
        let total_active_versions = VerEntity::find()
            .filter(VerColumn::IsActive.eq(true))
            .count(db)
            .await?;
        let builtin_algorithms = AlgoEntity::find()
            .filter(AlgoColumn::IsBuiltin.eq(true))
            .count(db)
            .await?;
        let custom_algorithms = total_algorithms.saturating_sub(builtin_algorithms);

        Ok(AlgorithmStats {
            total_algorithms,
            total_active_versions,
            builtin_algorithms,
            custom_algorithms,
        })
    }
}
