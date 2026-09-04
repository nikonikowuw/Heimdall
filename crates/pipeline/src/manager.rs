use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use types::{AnalysisTask, Camera};

use crate::error::PipelineError;

/// 全局多路视频分析管线调度控制器
#[derive(Debug, Default)]
pub struct PipelineManager {
    tasks: Arc<RwLock<HashMap<String, AnalysisTask>>>,
}

impl PipelineManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 启动或更新某路摄像头的分析任务
    pub async fn start_task(
        &self,
        camera: &Camera,
        task: AnalysisTask,
    ) -> Result<(), PipelineError> {
        let mut tasks = self.tasks.write().await;
        tasks.insert(camera.camera_id.clone(), task);
        tracing::info!(camera_id = %camera.camera_id, "分析管线任务已启动/更新");
        Ok(())
    }

    /// 停止某路摄像头的分析任务
    pub async fn stop_task(&self, camera_id: &str) -> Result<(), PipelineError> {
        let mut tasks = self.tasks.write().await;
        if tasks.remove(camera_id).is_some() {
            tracing::info!(camera_id = %camera_id, "分析管线任务已停止");
            Ok(())
        } else {
            Err(PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            })
        }
    }
}
