use std::collections::HashSet;

use sea_orm::entity::prelude::DateTimeUtc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, FromQueryResult,
    JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
    TransactionTrait,
};

use crate::entity::alarm::{ActiveModel, Column, Entity, Model, Relation};
use crate::entity::camera;
use crate::error::DbError;
use crate::repository::query::keyword_pattern;

#[derive(Debug)]
pub struct AlarmRepo;

/// 告警列表/计数的过滤条件（各字段为 `None` 或空串表示不限制）。
///
/// 收拢为结构体而不是继续加函数参数：过滤维度每加一个，调用点的位置参数就会错位一次，
/// 而这里多数字段都是 `Option`，类型系统无法帮忙发现传串了。
#[derive(Debug, Clone, Default)]
pub struct AlarmFilter<'a> {
    pub camera_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub target_label: Option<&'a str>,
    pub rule_type: Option<&'a str>,
    pub severity: Option<&'a str>,
    /// 关键字，字面量匹配事件 ID、类别、规则、通道 ID 与通道名称
    pub keyword: Option<&'a str>,
    pub start_time: Option<DateTimeUtc>,
    pub end_time: Option<DateTimeUtc>,
}

fn build_filter_query(filter: AlarmFilter<'_>) -> sea_orm::Select<Entity> {
    let mut query = Entity::find();
    if let Some(cid) = filter.camera_id.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::CameraId.eq(cid));
    }
    if let Some(st) = filter.status.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::Status.eq(st));
    }
    if let Some(lbl) = filter.target_label.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::TargetLabel.eq(lbl));
    }
    if let Some(rt) = filter.rule_type.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::RuleType.eq(rt));
    }
    if let Some(sev) = filter.severity.filter(|s| !s.trim().is_empty()) {
        query = query.filter(Column::Severity.eq(sev));
    }
    if let Some(pattern) = keyword_pattern(filter.keyword) {
        query = query
            .join(JoinType::LeftJoin, Relation::Camera.def())
            .filter(
                Condition::any()
                    .add(Column::EventId.like(pattern.clone()))
                    .add(Column::CameraId.like(pattern.clone()))
                    .add(Column::AlarmTypeId.like(pattern.clone()))
                    .add(Column::TargetLabel.like(pattern.clone()))
                    .add(Column::RuleType.like(pattern.clone()))
                    .add(camera::Column::Name.like(pattern)),
            );
    }
    if let Some(start) = filter.start_time {
        query = query.filter(Column::OccurredAt.gte(start));
    }
    if let Some(end) = filter.end_time {
        query = query.filter(Column::OccurredAt.lte(end));
    }
    query
}

impl AlarmRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        camera_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        Self::list_filtered(
            db,
            AlarmFilter {
                camera_id,
                ..AlarmFilter::default()
            },
            limit,
            offset,
        )
        .await
    }

    pub async fn list_filtered(
        db: &DatabaseConnection,
        filter: AlarmFilter<'_>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        build_filter_query(filter)
            .order_by_desc(Column::OccurredAt)
            .order_by_desc(Column::Id)
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_filtered(
        db: &DatabaseConnection,
        filter: AlarmFilter<'_>,
    ) -> Result<u64, DbError> {
        build_filter_query(filter)
            .count(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn update_status(
        db: &DatabaseConnection,
        id: i64,
        status: &str,
    ) -> Result<Model, DbError> {
        let model = Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "alarm_records",
                key: id.to_string(),
            })?;

        let mut active: ActiveModel = model.into();
        active.status = sea_orm::Set(status.to_string());
        active.handled_at = sea_orm::Set(Some(chrono::Utc::now()));
        active.update(db).await.map_err(DbError::from)
    }

    pub async fn update_status_by_ids(
        db: &DatabaseConnection,
        ids: &[i64],
        status: &str,
    ) -> Result<Vec<Model>, DbError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let now = chrono::Utc::now();
        let ids_vec = ids.to_vec();
        let status_str = status.to_string();
        db.transaction::<_, Vec<Model>, DbError>(|txn| {
            Box::pin(async move {
                let records = Entity::find()
                    .filter(Column::Id.is_in(ids_vec))
                    .all(txn)
                    .await?;

                let mut updated = Vec::with_capacity(records.len());
                for r in records {
                    let mut active: ActiveModel = r.into();
                    active.status = sea_orm::Set(status_str.clone());
                    active.handled_at = sea_orm::Set(Some(now));
                    let saved = active.update(txn).await.map_err(DbError::from)?;
                    updated.push(saved);
                }
                Ok(updated)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn insert(
        db: &DatabaseConnection,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    pub async fn insert_alarm_with_optional_capture(
        db: &DatabaseConnection,
        alarm: ActiveModel,
        capture: Option<crate::entity::capture::ActiveModel>,
    ) -> Result<Model, DbError> {
        db.transaction::<_, Model, DbError>(|txn| {
            Box::pin(async move {
                let saved_alarm = alarm.insert(txn).await.map_err(DbError::from)?;
                if let Some(cap) = capture {
                    cap.insert(txn).await.map_err(DbError::from)?;
                }
                Ok(saved_alarm)
            })
        })
        .await
        .map_err(DbError::from)
    }

    pub async fn find_by_event_id(
        db: &DatabaseConnection,
        event_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::EventId.eq(event_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn delete_by_event_id(
        db: &DatabaseConnection,
        event_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::EventId.eq(event_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    /// 淘汰扫描的取批游标：`occurred_at` 相同时必须由 `id` 兜底，
    /// 否则每次以相同 `limit` 重查会拿到不确定顺序，可能重复取到已删除的批次。
    pub async fn find_oldest_batch(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::OccurredAt)
            .order_by_asc(Column::Id)
            .limit(limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn delete_by_ids(db: &DatabaseConnection, ids: &[i64]) -> Result<u64, DbError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let res = Entity::delete_many()
            .filter(Column::Id.is_in(ids.to_vec()))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn count_all(db: &DatabaseConnection) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }

    /// 查询全部活跃告警记录关联的文件相对路径（全景 + 特写）
    pub async fn find_all_active_image_paths(
        db: &DatabaseConnection,
    ) -> Result<HashSet<String>, DbError> {
        #[derive(FromQueryResult)]
        struct PathRow {
            image_rel_path: String,
            crop_image_rel_path: String,
        }
        let rows = Entity::find()
            .select_only()
            .column(Column::ImageRelPath)
            .column(Column::CropImageRelPath)
            .into_model::<PathRow>()
            .all(db)
            .await?;
        let mut set = HashSet::new();
        for r in rows {
            if !r.image_rel_path.is_empty() {
                set.insert(r.image_rel_path);
            }
            if !r.crop_image_rel_path.is_empty() {
                set.insert(r.crop_image_rel_path);
            }
        }
        Ok(set)
    }

    pub async fn find_before(
        db: &DatabaseConnection,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::CreatedAt.lt(before))
            .order_by_asc(Column::CreatedAt)
            .order_by_asc(Column::Id)
            .limit(limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    pub async fn count_since(
        db: &DatabaseConnection,
        since: chrono::NaiveDateTime,
    ) -> Result<u64, DbError> {
        let count = Entity::find()
            .filter(Column::CreatedAt.gte(since))
            .count(db)
            .await
            .map_err(DbError::from)?;
        Ok(count)
    }
}
