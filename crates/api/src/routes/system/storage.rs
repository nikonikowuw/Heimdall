use axum::extract::State;
use axum::Json;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

use super::round_1dp;

/// `GET /api/v1/system/storage/status`
pub async fn get_storage_status(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::StorageStatus>, ApiError> {
    let cleaner = state
        .storage_cleaner
        .as_ref()
        .ok_or_else(|| ApiError::SystemInfo("存储清理器未初始化".into()))?;
    let mut status = cleaner
        .get_storage_status()
        .await
        .map_err(|e| ApiError::SystemInfo(e.to_string()))?;

    let alarm_count = db::AlarmRepo::count_all(&state.db).await.unwrap_or(0) as u32;
    let capture_count = db::CaptureRepo::count_all(&state.db).await.unwrap_or(0) as u32;
    let recognition_count = db::RecognitionRepo::count_all(&state.db).await.unwrap_or(0) as u32;

    status.alarm_count = alarm_count;
    status.alarm_size_mb = round_1dp(alarm_count as f64 * 0.35);
    status.capture_count = capture_count;
    status.capture_size_mb = round_1dp(capture_count as f64 * 0.25);
    status.recognition_count = recognition_count;
    status.recognition_size_mb = round_1dp(recognition_count as f64 * 0.05);

    Ok(ApiResponse::success(status))
}

/// `GET /api/v1/system/storage/config`
pub async fn get_storage_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::StorageConfig>, ApiError> {
    let config = load_storage_config_from_db(&state).await?;
    Ok(ApiResponse::success(config))
}

/// `PUT /api/v1/system/storage/config`
pub async fn update_storage_config(
    State(state): State<AppState>,
    Json(new_config): Json<types::StorageConfig>,
) -> Result<ApiResponse<types::StorageConfig>, ApiError> {
    new_config.validate().map_err(ApiError::StorageConfig)?;

    let json = serde_json::to_string(&new_config)
        .map_err(|e| ApiError::StorageConfig(format!("序列化配置失败: {e}")))?;
    db::SystemConfigRepo::set(&state.db, "storage_config", &json)
        .await
        .map_err(|e| ApiError::StorageConfig(format!("保存配置失败: {e}")))?;

    if let Some(cleaner) = state.storage_cleaner.as_ref() {
        let mut runtime_config = cleaner.get_config().await;
        runtime_config.apply_storage_config(&new_config);
        cleaner.update_config(runtime_config).await;
    }

    Ok(ApiResponse::success(new_config))
}

/// `POST /api/v1/system/storage/cleanup`
pub async fn trigger_storage_cleanup(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::EvictionReport>, ApiError> {
    let cleaner = state
        .storage_cleaner
        .as_ref()
        .ok_or_else(|| ApiError::SystemInfo("存储清理器未初始化".into()))?;

    let adapter = DbEvictionStoreAdapter(state.db.clone());
    let report = cleaner
        .trigger_cleanup(&adapter)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("清理执行失败: {e}")))?;

    Ok(ApiResponse::success(report))
}

/// 适配器：将 `sea_orm::DatabaseConnection` 包装为 `EvictionStore` trait 实现
#[derive(Debug)]
pub struct DbEvictionStoreAdapter(pub sea_orm::DatabaseConnection);

#[async_trait::async_trait]
impl pipeline::storage_cleaner::EvictionStore for DbEvictionStoreAdapter {
    async fn find_oldest_captures(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let models = db::CaptureRepo::find_oldest_batch(&self.0, limit)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("查询最老抓拍失败: {e}")))?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.image_rel_path, m.crop_image_rel_path))
            .collect())
    }

    async fn delete_captures(&self, ids: &[i64]) -> Result<u64, pipeline::PipelineError> {
        db::CaptureRepo::delete_by_ids(&self.0, ids)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("删除抓拍记录失败: {e}")))
    }

    async fn find_oldest_alarms(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let models = db::AlarmRepo::find_oldest_batch(&self.0, limit)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("查询最老告警失败: {e}")))?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.image_rel_path, m.crop_image_rel_path))
            .collect())
    }

    async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, pipeline::PipelineError> {
        db::AlarmRepo::delete_by_ids(&self.0, ids)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("删除告警记录失败: {e}")))
    }

    async fn find_oldest_recognitions(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String)>, pipeline::PipelineError> {
        let models = db::RecognitionRepo::find_oldest_batch(&self.0, limit)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("查询最老识别记录失败: {e}")))?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.field_crop_path))
            .collect())
    }

    async fn delete_recognitions(&self, ids: &[i64]) -> Result<u64, pipeline::PipelineError> {
        db::RecognitionRepo::delete_by_ids(&self.0, ids)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(format!("删除识别记录失败: {e}")))
    }

    async fn find_captures_before(
        &self,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let models = db::CaptureRepo::find_before(&self.0, before, limit)
            .await
            .map_err(|e| {
                pipeline::PipelineError::Snapshot(format!("按保留天数查询抓拍失败: {e}"))
            })?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.image_rel_path, m.crop_image_rel_path))
            .collect())
    }

    async fn find_recognitions_before(
        &self,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<(i64, String)>, pipeline::PipelineError> {
        let models = db::RecognitionRepo::find_before(&self.0, before, limit)
            .await
            .map_err(|e| {
                pipeline::PipelineError::Snapshot(format!("按保留天数查询识别记录失败: {e}"))
            })?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.field_crop_path))
            .collect())
    }

    async fn find_alarms_before(
        &self,
        before: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let models = db::AlarmRepo::find_before(&self.0, before, limit)
            .await
            .map_err(|e| {
                pipeline::PipelineError::Snapshot(format!("按保留天数查询告警失败: {e}"))
            })?;
        Ok(models
            .into_iter()
            .map(|m| (m.id, m.image_rel_path, m.crop_image_rel_path))
            .collect())
    }
}

/// 从 DB 读取 StorageConfig；若无记录返回默认值
async fn load_storage_config_from_db(state: &AppState) -> Result<types::StorageConfig, ApiError> {
    if let Ok(Some(json_str)) = db::SystemConfigRepo::get(&state.db, "storage_config").await {
        if let Ok(config) = serde_json::from_str::<types::StorageConfig>(&json_str) {
            return Ok(config);
        }
    }
    Ok(types::StorageConfig::default())
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/storage/status", axum::routing::get(get_storage_status))
        .route(
            "/storage/config",
            axum::routing::get(get_storage_config).put(update_storage_config),
        )
        .route(
            "/storage/cleanup",
            axum::routing::post(trigger_storage_cleanup),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::fs;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn storage_status_returns_disk_usage_when_cleaner_is_configured() {
        let evidence_dir = std::env::temp_dir().join(format!(
            "heimdall-storage-status-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&evidence_dir).expect("创建临时 evidence 目录失败");

        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline).with_storage_cleaner(evidence_dir.clone());
        let app = router().with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/storage/status")
                    .body(Body::empty())
                    .expect("构造请求失败"),
            )
            .await
            .expect("执行请求失败");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体失败");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("解析存储状态响应失败");
        assert_eq!(payload["code"], 0);
        assert!(payload["data"]["totalGb"].as_f64().unwrap_or(0.0) > 0.0);

        fs::remove_dir_all(evidence_dir).expect("清理临时 evidence 目录失败");
    }
}
