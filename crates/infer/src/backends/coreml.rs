use async_trait::async_trait;
use types::{Detection, FrameRef};

use crate::backend::InferenceBackend;
use crate::error::InferError;

/// Apple Silicon ANE (Core ML) 硬件加速推理后端
#[derive(Debug, Default)]
pub struct CoreMlBackend;

impl CoreMlBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl InferenceBackend for CoreMlBackend {
    fn name(&self) -> &'static str {
        "Apple-ANE-CoreML"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        // 骨架桩：对接 objc2 / VideoToolbox CVPixelBuffer 零拷贝直通 ANE 推理
        Ok(Vec::new())
    }
}
