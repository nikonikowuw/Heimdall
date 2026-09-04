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
}
