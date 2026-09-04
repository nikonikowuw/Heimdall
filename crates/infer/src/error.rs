use thiserror::Error;

/// 推理后端与算子错误枚举
#[derive(Debug, Error)]
pub enum InferError {
    #[error("模型加载失败: {path} (原因: {reason})")]
    ModelLoad { path: String, reason: String },

    #[error("输入张量形状不匹配: 期望 {expected:?}, 实际 {actual:?}")]
    ShapeMismatch {
        expected: Vec<usize>,
        actual: Vec<usize>,
    },

    #[error("推理后端 {backend} 未编译进本次构建")]
    BackendUnavailable { backend: &'static str },

    #[error("硬件推理超时，超过 {0:?}")]
    Timeout(std::time::Duration),

    #[error("硬件推理执行失败: {reason}")]
    Execution { reason: String },

    #[error("帧载体错误: {0}")]
    Frame(#[from] types::FrameError),
}
