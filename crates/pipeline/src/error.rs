use thiserror::Error;

/// 分析管线与规则调度错误枚举
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("摄像头管线已存在: {camera_id}")]
    PipelineAlreadyExists { camera_id: String },

    #[error("摄像头管线未找到: {camera_id}")]
    PipelineNotFound { camera_id: String },

    #[error("几何规则格式非法: {reason}")]
    InvalidRule { reason: String },

    #[error("快照抓拍失败: {0}")]
    Snapshot(String),

    #[error("安全沙箱违规: {0}")]
    Security(String),

    #[error("媒体层错误: {0}")]
    Media(#[from] media::MediaError),

    #[error("推理层错误: {0}")]
    Infer(#[from] infer::InferError),

    #[error("领域类型错误: {0}")]
    Type(#[from] types::TypeError),
}

impl PipelineError {
    /// 对应的 API 规范标准 5 位业务错误码 (30000~39999 算法管线模块)
    pub fn error_code(&self) -> u32 {
        match self {
            Self::PipelineNotFound { .. } => 30001,
            Self::InvalidRule { .. } => 30002,
            Self::Snapshot(_) => 30003,
            Self::Security(_) => 30006,
            Self::PipelineAlreadyExists { .. } => 30004,
            Self::Type(_) => 30005,
            Self::Infer(e) => e.error_code(),
            Self::Media(e) => e.error_code(),
        }
    }
}
