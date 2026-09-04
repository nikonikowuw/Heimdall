use thiserror::Error;

/// 推理后端与算子错误枚举
#[derive(Debug, Error)]
pub enum InferError {
    #[error("模型加载失败: {path} (原因: {reason})")]
    ModelLoad { path: String, reason: String },

    #[error("动态库加载失败: {path} (原因: {reason})")]
    LibraryLoad { path: String, reason: String },

    #[error("符号寻址失败: {symbol} (原因: {reason})")]
    SymbolLookup { symbol: String, reason: String },

    #[error("C ABI 虚表无效: {reason}")]
    InvalidAbi { reason: String },

    #[error("C ABI 返回错误码 {code}: {message}")]
    CAbiError { code: i32, message: String },

    #[error("沙箱校验失败 [{step}]: {reason}")]
    SandboxValidation { step: String, reason: String },

    #[error("JSON 解析失败: {reason}")]
    JsonParse { reason: String },

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

impl InferError {
    /// 对应的 API 规范标准 5 位业务错误码 (30010~30099 算法包与推理模块)
    pub fn error_code(&self) -> u32 {
        match self {
            Self::ModelLoad { .. } => 30011,
            Self::LibraryLoad { .. } => 30012,
            Self::SymbolLookup { .. } => 30013,
            Self::InvalidAbi { .. } => 30014,
            Self::CAbiError { .. } => 30015,
            Self::SandboxValidation { .. } => 30016,
            Self::JsonParse { .. } => 30017,
            Self::ShapeMismatch { .. } => 30018,
            Self::BackendUnavailable { .. } => 30019,
            Self::Timeout(_) => 30020,
            Self::Execution { .. } => 30021,
            Self::Frame(_) => 30022,
        }
    }
}
