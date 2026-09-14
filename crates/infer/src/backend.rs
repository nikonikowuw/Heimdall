use async_trait::async_trait;
use types::{Detection, FaceEmbedding, FrameRef};

use crate::error::InferError;

/// 推理结果与低频特征 sidecar。
///
/// `embeddings` 与 `detections` 按索引对应。普通检测没有 sidecar，
/// 只有算法包在 best-shot 帧显式输出时才会出现非空项。
#[derive(Debug, Default)]
pub struct InferenceResult {
    pub detections: Vec<Detection>,
    pub embeddings: Vec<Option<FaceEmbedding>>,
}

impl InferenceResult {
    pub fn without_embeddings(detections: Vec<Detection>) -> Self {
        let embeddings = vec![None; detections.len()];
        Self {
            detections,
            embeddings,
        }
    }

    pub fn into_parts(self) -> (Vec<Detection>, Vec<Option<FaceEmbedding>>) {
        (self.detections, self.embeddings)
    }
}

/// 平台推理后端通用抽象接口。
///
/// 后端不声明 `Send`/`Sync`：C ABI/NPU session 可能绑定创建它的 OS Worker。
/// `InferenceWorker` 在线程闭包内部创建并独占调用后端；跨线程只传递 Worker 的控制句柄。
#[async_trait(?Send)]
pub trait InferenceBackend: 'static {
    /// 获取后端名称（如 "CoreML-ANE", "CPU-ORT"）
    fn name(&self) -> &'static str;

    /// 将解码后的原生 `FrameRef` 交给算法后端；算法后端自行完成预处理。
    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;

    /// 返回检测结果及仅供后端识别链路使用的低频特征 sidecar。
    async fn detect_with_metadata(&self, frame: &FrameRef) -> Result<InferenceResult, InferError> {
        let detections = self.detect(frame).await?;
        Ok(InferenceResult::without_embeddings(detections))
    }
}

#[async_trait(?Send)]
impl<B: InferenceBackend + ?Sized> InferenceBackend for Box<B> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        (**self).detect(frame).await
    }

    async fn detect_with_metadata(&self, frame: &FrameRef) -> Result<InferenceResult, InferError> {
        (**self).detect_with_metadata(frame).await
    }
}

#[async_trait(?Send)]
impl<B: InferenceBackend + ?Sized> InferenceBackend for std::sync::Arc<B> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        (**self).detect(frame).await
    }

    async fn detect_with_metadata(&self, frame: &FrameRef) -> Result<InferenceResult, InferError> {
        (**self).detect_with_metadata(frame).await
    }
}
