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

    #[error("媒体层错误: {0}")]
    Media(#[from] media::MediaError),

    #[error("推理层错误: {0}")]
    Infer(#[from] infer::InferError),

    #[error("领域类型错误: {0}")]
    Type(#[from] types::TypeError),
}
