use async_trait::async_trait;
use types::{Detection, FrameRef};

use crate::error::InferError;

/// 平台推理后端通用抽象接口
#[async_trait]
pub trait InferenceBackend: Send + Sync + 'static {
    /// 获取后端名称（如 "CoreML-ANE", "CPU-ORT"）
    fn name(&self) -> &'static str;

    /// 输入零拷贝 FrameRef，执行 NPU/ANE 目标检测推理
    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;
}
