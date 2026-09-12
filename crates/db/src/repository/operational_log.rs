use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement,
};

use crate::entity::operational_log::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

/// 分页查询参数
#[derive(Debug, Default)]
pub struct ListParams<'a> {
    pub level: Option<&'a str>,
    pub event: Option<&'a str>,
    pub camera_id: Option<&'a str>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub before: Option<i64>,
    pub limit: u64,
}

#[derive(Debug)]
pub struct OperationalLogRepo;

impl OperationalLogRepo {
    /// 批量插入运维事件（攒批调用，单事务）
    pub async fn insert_batch(
        db: &DatabaseConnection,
        entries: Vec<InsertEntry>,
    ) -> Result<u64, DbError> {
        if entries.is_empty() {
            return Ok(0);
        }

        let models: Vec<ActiveModel> = entries
            .into_iter()
            .map(|e| ActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                ts_ms: Set(e.ts_ms),
                level: Set(e.level),
                event: Set(e.event),
                target: Set(e.target),
                message: Set(e.message),
                camera_id: Set(e.camera_id),
                extra_json: Set(e.extra_json),
            })
            .collect();

        let result = Entity::insert_many(models).exec(db).await?;
        Ok(result.last_insert_id as u64)
    }

    /// 分页查询运维日志（按时间倒序，before 游标分页）
    pub async fn list(
        db: &DatabaseConnection,
        params: &ListParams<'_>,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find()
            .order_by_desc(Column::TsMs)
            .order_by_desc(Column::Id);

        if let Some(lv) = params.level {
            query = query.filter(Column::Level.eq(lv));
        }
        if let Some(ev) = params.event {
            query = query.filter(Column::Event.eq(ev));
        }
        if let Some(cam) = params.camera_id {
            query = query.filter(Column::CameraId.eq(cam));
        }
        if let Some(from) = params.from_ms {
            query = query.filter(Column::TsMs.gte(from));
        }
        if let Some(to) = params.to_ms {
            query = query.filter(Column::TsMs.lte(to));
        }
        if let Some(before) = params.before {
            query = query.filter(Column::TsMs.lt(before));
        }

        query
            .limit(params.limit)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 删除超过指定时间戳的旧记录，返回删除行数
    pub async fn delete_before(db: &DatabaseConnection, cutoff_ms: i64) -> Result<u64, DbError> {
        let result = Entity::delete_many()
            .filter(Column::TsMs.lt(cutoff_ms))
            .exec(db)
            .await?;
        Ok(result.rows_affected)
    }

    /// 仅保留最新的 N 条记录，删除多余的旧记录 (直接通过 SQLite 子查询删除，避免超限)
    pub async fn retain_latest(db: &DatabaseConnection, max_rows: u64) -> Result<u64, DbError> {
        let total = Entity::find().count(db).await?;
        if total <= max_rows {
            return Ok(0);
        }

        let excess = (total - max_rows) as i64;
        let res = db
            .execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "DELETE FROM operational_logs WHERE id IN (SELECT id FROM operational_logs ORDER BY ts_ms ASC, id ASC LIMIT ?)",
                [excess.into()],
            ))
            .await
            .map_err(DbError::from)?;
        Ok(res.rows_affected())
    }
}

/// 批量插入条目
#[derive(Debug, Clone)]
pub struct InsertEntry {
    pub ts_ms: i64,
    pub level: String,
    pub event: String,
    pub target: String,
    pub message: String,
    pub camera_id: Option<String>,
    pub extra_json: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_operational_log_repository_lifecycle() {
        let db = crate::init_test_db()
            .await
            .expect("init in-memory db failed");

        // 1. 批量写入事件
        let entries = vec![
            InsertEntry {
                ts_ms: 1000,
                level: "info".to_string(),
                event: "service_started".to_string(),
                target: "system".to_string(),
                message: "服务启动".to_string(),
                camera_id: None,
                extra_json: None,
            },
            InsertEntry {
                ts_ms: 2000,
                level: "info".to_string(),
                event: "camera_online".to_string(),
                target: "media".to_string(),
                message: "摄像头 cam-1 在线".to_string(),
                camera_id: Some("cam-1".to_string()),
                extra_json: None,
            },
            InsertEntry {
                ts_ms: 3000,
                level: "warn".to_string(),
                event: "camera_offline".to_string(),
                target: "media".to_string(),
                message: "摄像头 cam-1 离线".to_string(),
                camera_id: Some("cam-1".to_string()),
                extra_json: Some(r#"{"reason":"timeout"}"#.to_string()),
            },
            InsertEntry {
                ts_ms: 4000,
                level: "error".to_string(),
                event: "task_failed".to_string(),
                target: "pipeline".to_string(),
                message: "任务执行失败".to_string(),
                camera_id: Some("cam-1".to_string()),
                extra_json: None,
            },
        ];

        let count = OperationalLogRepo::insert_batch(&db, entries)
            .await
            .unwrap();
        assert!(count >= 4);

        // 2. 基础列表查询 (降序)
        let list = OperationalLogRepo::list(
            &db,
            &ListParams {
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].ts_ms, 4000);
        assert_eq!(list[3].ts_ms, 1000);

        // 3. 条件过滤：按级别与摄像头
        let warn_logs = OperationalLogRepo::list(
            &db,
            &ListParams {
                level: Some("warn"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(warn_logs.len(), 1);
        assert_eq!(warn_logs[0].event, "camera_offline");

        // 4. before 游标分页
        let page1 = OperationalLogRepo::list(
            &db,
            &ListParams {
                before: Some(3500),
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].ts_ms, 3000);
        assert_eq!(page1[1].ts_ms, 2000);

        // 5. 按时间淘汰 delete_before
        let deleted = OperationalLogRepo::delete_before(&db, 2500).await.unwrap();
        assert_eq!(deleted, 2); // 1000 与 2000 被删除

        let remaining = OperationalLogRepo::list(
            &db,
            &ListParams {
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(remaining.len(), 2);

        // 6. retain_latest 保留最新 N 条
        let pruned = OperationalLogRepo::retain_latest(&db, 1).await.unwrap();
        assert_eq!(pruned, 1); // 3000 被淘汰，仅保留 4000

        let latest = OperationalLogRepo::list(
            &db,
            &ListParams {
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].ts_ms, 4000);
    }
}
