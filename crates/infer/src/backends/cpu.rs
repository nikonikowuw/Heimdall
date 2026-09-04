use async_trait::async_trait;
use types::{Detection, FrameRef};

use crate::backend::InferenceBackend;
use crate::error::InferError;

/// 开发测试与 CPU 回退推理后端
#[derive(Debug, Default)]
pub struct CpuBackend;

impl CpuBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl InferenceBackend for CpuBackend {
    fn name(&self) -> &'static str {
        "CPU-Fallback"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, InferError> {
        // 骨架桩：返回空检测列表或开发测试模拟数据
        Ok(Vec::new())
    }
}
