//! 算法包错误体系与 C ABI 状态码双向映射

use std::ffi::c_int;
use thiserror::Error;

use crate::c_abi::*;

/// 算法包通用错误类型
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AlgoError {
    #[error("配置解析失败: {reason}")]
    ConfigParse { reason: String },

    #[error("模型加载失败: {reason}")]
    ModelLoad { reason: String },

    #[error("推理执行失败: {reason}")]
    Inference { reason: String },

    #[error("图像预处理失败: {reason}")]
    Preprocess { reason: String },

    #[error("不兼容的帧格式: {reason}")]
    IncompatibleFrame { reason: String },

    #[error("内存不足")]
    OutOfMemory,

    #[error("不支持的 API 接口")]
    UnsupportedApi,

    #[error("未实现的能力")]
    NotImplemented,

    #[error("超时")]
    Timeout,

    #[error("内部错误: {reason}")]
    Internal { reason: String },
}

impl AlgoError {
    /// 转换为对应的 C ABI 负数状态码
    pub fn to_c_status(&self) -> c_int {
        match self {
            Self::ConfigParse { .. } => AV_ERR_CONFIG_INVALID,
            Self::ModelLoad { .. } => AV_ERR_MODEL_LOAD_FAILED,
            Self::Inference { .. } => AV_ERR_INFERENCE_FAILED,
            Self::Preprocess { .. } => AV_ERR_INVALID_ARG,
            Self::IncompatibleFrame { .. } => AV_ERR_INCOMPATIBLE_FRAME,
            Self::OutOfMemory => AV_ERR_OUT_OF_MEMORY,
            Self::UnsupportedApi => AV_ERR_UNSUPPORTED_API,
            Self::NotImplemented => AV_ERR_NOT_IMPLEMENTED,
            Self::Timeout => AV_ERR_TIMEOUT,
            Self::Internal { .. } => AV_ERR_INTERNAL,
        }
    }

    /// 从 C ABI 状态码转换为 Result<(), AlgoError>
    pub fn from_c_status(code: c_int) -> Result<(), Self> {
        match code {
            AV_OK => Ok(()),
            AV_ERR_UNSUPPORTED_API => Err(Self::UnsupportedApi),
            AV_ERR_INVALID_ARG => Err(Self::Preprocess {
                reason: "无效参数".to_string(),
            }),
            AV_ERR_INCOMPATIBLE_FRAME => Err(Self::IncompatibleFrame {
                reason: "帧格式不匹配".to_string(),
            }),
            AV_ERR_CONFIG_INVALID => Err(Self::ConfigParse {
                reason: "配置无效".to_string(),
            }),
            AV_ERR_MODEL_LOAD_FAILED => Err(Self::ModelLoad {
                reason: "模型加载失败".to_string(),
            }),
            AV_ERR_INFERENCE_FAILED => Err(Self::Inference {
                reason: "推理失败".to_string(),
            }),
            AV_ERR_OUT_OF_MEMORY => Err(Self::OutOfMemory),
            AV_ERR_NOT_IMPLEMENTED => Err(Self::NotImplemented),
            AV_ERR_TIMEOUT => Err(Self::Timeout),
            other => Err(Self::Internal {
                reason: format!("C ABI 错误码: {other}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_to_c_status_and_back() {
        let errs = [
            (
                AlgoError::ConfigParse {
                    reason: "bad json".into(),
                },
                AV_ERR_CONFIG_INVALID,
            ),
            (
                AlgoError::ModelLoad {
                    reason: "file missing".into(),
                },
                AV_ERR_MODEL_LOAD_FAILED,
            ),
            (
                AlgoError::Inference {
                    reason: "npu error".into(),
                },
                AV_ERR_INFERENCE_FAILED,
            ),
            (
                AlgoError::Preprocess {
                    reason: "stride mismatch".into(),
                },
                AV_ERR_INVALID_ARG,
            ),
            (
                AlgoError::IncompatibleFrame {
                    reason: "need nv12".into(),
                },
                AV_ERR_INCOMPATIBLE_FRAME,
            ),
            (AlgoError::OutOfMemory, AV_ERR_OUT_OF_MEMORY),
            (AlgoError::UnsupportedApi, AV_ERR_UNSUPPORTED_API),
            (AlgoError::NotImplemented, AV_ERR_NOT_IMPLEMENTED),
            (AlgoError::Timeout, AV_ERR_TIMEOUT),
            (
                AlgoError::Internal {
                    reason: "unknown".into(),
                },
                AV_ERR_INTERNAL,
            ),
        ];

        for (err, expected_code) in errs {
            assert_eq!(err.to_c_status(), expected_code);
            let converted = AlgoError::from_c_status(expected_code);
            assert!(converted.is_err());
        }

        assert_eq!(AlgoError::from_c_status(AV_OK), Ok(()));
    }
}
