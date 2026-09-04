use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 操作审计日志模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLog {
    pub id: u64,
    pub username: String,
    pub module: String,
    pub action: String,
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub status_code: u16,
    pub duration_ms: i64,
    pub ip: String,
    pub user_agent: String,
    pub created_at: DateTime<Utc>,
}
