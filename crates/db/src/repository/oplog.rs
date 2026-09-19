use sea_orm::sea_query::LikeExpr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use crate::entity::oplog::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

/// 状态码分类过滤，与 `GET /logs/operations` 的 `status` 参数取值一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    /// HTTP 2xx
    Success,
    /// HTTP 4xx 与 5xx
    Failed,
}

/// 审计日志分页查询参数
#[derive(Debug, Default)]
pub struct ListParams<'a> {
    pub module: Option<&'a str>,
    pub status: Option<StatusClass>,
    /// 关键字，字面量匹配操作人、模块、动作、请求路径与客户端 IP
    pub keyword: Option<&'a str>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: u64,
    pub offset: u64,
}

/// 转义 LIKE 通配符，让用户输入按字面量匹配而不是被当成模式
fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[derive(Debug)]
pub struct OplogRepo;

impl OplogRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        params: &ListParams<'_>,
    ) -> Result<Vec<Model>, DbError> {
        // id 参与排序：created_at 相同时顺序仍然确定，offset 翻页不会重复或漏行
        let mut query = Entity::find()
            .order_by_desc(Column::CreatedAt)
            .order_by_desc(Column::Id);
        if let Some(m) = params.module {
            query = query.filter(Column::Module.eq(m));
        }
        if let Some(status) = params.status {
            query = match status {
                StatusClass::Success => {
                    query.filter(Column::StatusCode.gte(200).and(Column::StatusCode.lt(300)))
                }
                StatusClass::Failed => query.filter(Column::StatusCode.gte(400)),
            };
        }
        if let Some(keyword) = params.keyword {
            let pattern = LikeExpr::new(format!("%{}%", escape_like(keyword))).escape('\\');
            query = query.filter(
                Condition::any()
                    .add(Column::Username.like(pattern.clone()))
                    .add(Column::Module.like(pattern.clone()))
                    .add(Column::Action.like(pattern.clone()))
                    .add(Column::Path.like(pattern.clone()))
                    .add(Column::Ip.like(pattern)),
            );
        }
        if let Some(from) = params.from_ms {
            if let Some(from_dt) = chrono::DateTime::from_timestamp_millis(from) {
                query = query.filter(Column::CreatedAt.gte(from_dt));
            }
        }
        if let Some(to) = params.to_ms {
            if let Some(to_dt) = chrono::DateTime::from_timestamp_millis(to) {
                query = query.filter(Column::CreatedAt.lte(to_dt));
            }
        }
        query
            .limit(params.limit)
            .offset(params.offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        db: &DatabaseConnection,
        username: &str,
        module: &str,
        action: &str,
        method: &str,
        path: &str,
        query: &str,
        body: &str,
        status_code: i32,
        duration_ms: i64,
        ip: &str,
        user_agent: &str,
    ) -> Result<Model, DbError> {
        let active = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            username: Set(username.to_string()),
            module: Set(module.to_string()),
            action: Set(action.to_string()),
            method: Set(method.to_string()),
            path: Set(path.to_string()),
            query: Set(query.to_string()),
            body: Set(body.to_string()),
            status_code: Set(status_code),
            duration_ms: Set(duration_ms),
            ip: Set(ip.to_string()),
            user_agent: Set(user_agent.to_string()),
            created_at: Set(chrono::Utc::now()),
        };
        active.insert(db).await.map_err(DbError::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn seed(db: &DatabaseConnection) {
        let rows: [(&str, &str, &str, &str, &str, &str, i32); 5] = [
            (
                "admin",
                "camera",
                "create",
                "POST",
                "/api/v1/cameras",
                "10.0.0.1",
                201,
            ),
            (
                "admin",
                "camera",
                "update",
                "PUT",
                "/api/v1/cameras/cam-1",
                "10.0.0.2",
                200,
            ),
            (
                "operator",
                "task",
                "delete",
                "DELETE",
                "/api/v1/tasks/cam-1",
                "10.0.0.3",
                404,
            ),
            (
                "operator",
                "task",
                "read",
                "GET",
                "/api/v1/tasks",
                "10.0.0.4",
                500,
            ),
            (
                "ops",
                "system",
                "read",
                "GET",
                "/api/v1/system/status",
                "10.0.0.5",
                302,
            ),
        ];

        for (username, module, action, method, path, ip, status_code) in rows {
            OplogRepo::record(
                db,
                username,
                module,
                action,
                method,
                path,
                "",
                "",
                status_code,
                5,
                ip,
                "test-agent",
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn test_oplog_status_class_filter_splits_success_and_failed() {
        let db = crate::init_test_db()
            .await
            .expect("init in-memory db failed");
        seed(&db).await;

        let success = OplogRepo::list_recent(
            &db,
            &ListParams {
                status: Some(StatusClass::Success),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(success.len(), 2);
        assert!(success
            .iter()
            .all(|row| (200..300).contains(&row.status_code)));

        let failed = OplogRepo::list_recent(
            &db,
            &ListParams {
                status: Some(StatusClass::Failed),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(failed.len(), 2);
        assert!(failed.iter().all(|row| row.status_code >= 400));

        // 3xx 既不算成功也不算失败，两个分类都不应包含它
        assert!(success.iter().all(|row| row.status_code != 302));
        assert!(failed.iter().all(|row| row.status_code != 302));
    }

    #[tokio::test]
    async fn test_oplog_keyword_matches_multiple_columns_literally() {
        let db = crate::init_test_db()
            .await
            .expect("init in-memory db failed");
        seed(&db).await;

        // 命中 username
        let by_username = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("operator"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_username.len(), 2);

        // 命中 path 片段
        let by_path = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("/api/v1/cameras"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_path.len(), 2);

        // 命中 ip 前缀
        let by_ip = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("10.0.0."),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_ip.len(), 5);

        // 通配符按字面量处理：`%` 不能退化成“匹配全部”
        let literal_wildcard = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("%"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(literal_wildcard.is_empty());

        let literal_underscore = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("_"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(literal_underscore.is_empty());

        let no_match = OplogRepo::list_recent(
            &db,
            &ListParams {
                keyword: Some("no-such-operator"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(no_match.is_empty());
    }

    #[tokio::test]
    async fn test_oplog_filters_combine_and_paging_stays_stable() {
        let db = crate::init_test_db()
            .await
            .expect("init in-memory db failed");
        seed(&db).await;

        let combined = OplogRepo::list_recent(
            &db,
            &ListParams {
                module: Some("camera"),
                status: Some(StatusClass::Success),
                keyword: Some("cameras"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(combined.len(), 2);

        // 同一时间戳下的 offset 翻页必须由 id 兜底排序保证不重复
        let first_page = OplogRepo::list_recent(
            &db,
            &ListParams {
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let second_page = OplogRepo::list_recent(
            &db,
            &ListParams {
                limit: 2,
                offset: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(first_page.len(), 2);
        assert_eq!(second_page.len(), 2);
        assert!(first_page
            .iter()
            .all(|row| second_page.iter().all(|other| row.id != other.id)));
    }

    #[test]
    fn test_escape_like_escapes_wildcards_and_backslash() {
        assert_eq!(escape_like("plain"), "plain");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("c:\\tmp"), "c:\\\\tmp");
    }
}
