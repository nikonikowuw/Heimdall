use async_trait::async_trait;
use types::{Detection, FrameRef};

use crate::error::InferError;

/// 平台推理后端通用抽象接口
#[async_trait]
pub trait InferenceBackend: Send + Sync + 'static {
    /// 获取后端名称（如 "CoreML-ANE", "CPU-ORT"）
    fn name(&self) -> &'static str;

    /// 将解码后的原生 `FrameRef` 交给算法后端；算法后端自行完成模型输入所需的预处理。
    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;
}
