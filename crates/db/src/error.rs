use thiserror::Error;

/// 数据库访问层错误枚举
#[derive(Debug, Error)]
pub enum DbError {
    #[error("数据库连接失败: {url}")]
    Connection {
        url: String,
        #[source]
        source: sea_orm::DbErr,
    },

    #[error("数据查询失败: {0}")]
    Query(#[from] sea_orm::DbErr),

    #[error("未找到记录: {entity} (key={key})")]
    NotFound { entity: &'static str, key: String },

    #[error("JSON 反序列化错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("领域类型错误: {0}")]
    Type(#[from] types::TypeError),

    #[error("数据库迁移错误: {0}")]
    Migration(String),

    #[error("算法包正在使用中: {0}")]
    AlgoInUse(String),

    #[error("系统内置算法包受保护，禁止删除: {0}")]
    BuiltinAlgoProtected(String),
}

impl From<sea_orm::TransactionError<DbError>> for DbError {
    fn from(err: sea_orm::TransactionError<DbError>) -> Self {
        match err {
            sea_orm::TransactionError::Connection(e) => DbError::Query(e),
            sea_orm::TransactionError::Transaction(e) => e,
        }
    }
}
