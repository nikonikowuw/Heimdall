use axum::{extract::State, Json};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

const GIB: f64 = (1024 * 1024 * 1024) as f64;

#[inline]
fn round_1dp(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ─── 系统概览 ───

/// `GET /api/v1/system/overview`
pub async fn get_overview(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::SystemOverview>, ApiError> {
    // 阻塞 I/O 读取 /proc 与系统信息，不阻塞 tokio runtime
    let raw = tokio::task::spawn_blocking(crate::system_info::read_system_info)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))?;

    // 磁盘使用率（若 storage_cleaner 已装配，以 evidence_dir 所在物理分区为准，保证与存储页一致）
    let (disk_total_bytes, disk_available_bytes) = state
        .storage_cleaner
        .as_ref()
        .and_then(|cleaner| cleaner.current_fs_stat().ok())
        .map(|stat| (stat.total_bytes, stat.available_bytes))
        .unwrap_or((raw.disk.total_bytes, raw.disk.available_bytes));

    let disk_used_bytes = disk_total_bytes.saturating_sub(disk_available_bytes);
    let disk_total_gb = disk_total_bytes as f64 / GIB;
    let disk_used_gb = disk_used_bytes as f64 / GIB;
    let disk_usage_percent = if disk_total_bytes > 0 {
        (disk_used_bytes as f64 / disk_total_bytes as f64) * 100.0
    } else {
        0.0
    };

    // 内存
    let memory_total_mb = raw.memory.total_kb / 1024;
    let memory_used_mb = (raw.memory.total_kb.saturating_sub(raw.memory.available_kb)) / 1024;
    let memory_usage_percent = if raw.memory.total_kb > 0 {
        ((raw.memory.total_kb - raw.memory.available_kb) as f64 / raw.memory.total_kb as f64)
            * 100.0
    } else {
        0.0
    };

    // 业务统计（使用 count 聚合查询，不加载全部记录）
    let db = &state.db;
    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| chrono::Utc::now().naive_utc());

    let total_cameras = db::CameraRepo::count_all(db).await.unwrap_or(0) as u32;
    let active_cameras = db::CameraRepo::count_healthy(db).await.unwrap_or(0) as u32;
    let active_tasks = db::TaskRepo::count_running(db).await.unwrap_or(0) as u32;
    let today_alarms = db::AlarmRepo::count_since(db, today_start)
        .await
        .unwrap_or(0) as u32;
    let today_captures = db::CaptureRepo::count_since(db, today_start)
        .await
        .unwrap_or(0) as u32;

    // NPU 状态（平台相关，不可用时返回 null）
    let npu_usage = detect_npu_usage();

    Ok(ApiResponse::success(types::SystemOverview {
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        device_model: raw.device_model,
        os_info: raw.os_info,
        kernel_version: raw.kernel_version,
        uptime_seconds: raw.uptime as u64,
        cpu_usage_percent: round_1dp(raw.cpu.usage_percent),
        memory_usage_percent: round_1dp(memory_usage_percent),
        memory_used_mb,
        memory_total_mb,
        npu_usage_percent: npu_usage,
        disk_usage_percent: round_1dp(disk_usage_percent),
        disk_used_gb: round_1dp(disk_used_gb),
        disk_total_gb: round_1dp(disk_total_gb),
        active_cameras,
        total_cameras,
        active_tasks,
        today_alarms,
        today_captures,
    }))
}

/// 检测 NPU 使用率（平台相关，不可用时返回 None）
fn detect_npu_usage() -> Option<f64> {
    None
}

// ─── 网络配置 ───

/// `GET /api/v1/system/network/interfaces`
pub async fn get_network_interfaces(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::NetworkInterfacesResponse>, ApiError> {
    let response = crate::network_service::NetworkService::list_with_pending().await?;
    Ok(ApiResponse::success(response))
}

/// `GET /api/v1/system/network/interfaces/:name`
pub async fn get_network_interface(
    State(_state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<ApiResponse<types::NetworkInterface>, ApiError> {
    let iface = crate::network_service::NetworkService::get_interface(&name).await?;
    Ok(ApiResponse::success(iface))
}

/// `PUT /api/v1/system/network/interfaces/:name`
pub async fn update_network_interface(
    State(_state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(config): Json<types::IpConfig>,
) -> Result<ApiResponse<types::NetworkUpdateResult>, ApiError> {
    let result = crate::network_service::NetworkService::update_interface(&name, &config).await?;
    Ok(ApiResponse::success(result))
}

/// `GET /api/v1/system/network/changes/pending`
pub async fn get_pending_network_change(
    State(_state): State<AppState>,
) -> Result<ApiResponse<Option<types::NetworkChangeOperation>>, ApiError> {
    // 首版：管理接口变更操作暂不实现
    Ok(ApiResponse::success(None))
}

/// `POST /api/v1/system/network/changes/:id/confirm`
pub async fn confirm_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(_id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    // 首版：管理接口变更确认暂不实现
    Err(ApiError::NetworkFailed("管理接口变更确认暂未实现".into()))
}

/// `POST /api/v1/system/network/changes/:id/cancel`
pub async fn cancel_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(_id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    // 首版：管理接口变更取消暂不实现
    Err(ApiError::NetworkFailed("管理接口变更取消暂未实现".into()))
}

// ─── 存储与保留策略 ───

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

    // 从 DB 动态获取各类型证据真实数量及预估体积（KB 转换为 MB）
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
    // 校验水位层级关系
    new_config.validate().map_err(ApiError::StorageConfig)?;

    // 持久化到 DB
    let json = serde_json::to_string(&new_config)
        .map_err(|e| ApiError::StorageConfig(format!("序列化配置失败: {e}")))?;
    db::SystemConfigRepo::set(&state.db, "storage_config", &json)
        .await
        .map_err(|e| ApiError::StorageConfig(format!("保存配置失败: {e}")))?;

    // 热更新运行时 StorageCleaner 的水位与保留参数
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

// ─── 对时服务 ───

/// `GET /api/v1/system/time/status`
pub async fn get_time_status(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::TimeStatus>, ApiError> {
    let status = tokio::task::spawn_blocking(crate::time_service::TimeService::get_status_sync)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;
    Ok(ApiResponse::success(status))
}

/// `GET /api/v1/system/time/config`
pub async fn get_time_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::TimeConfig>, ApiError> {
    let config = crate::time_service::TimeService::get_config(&state.db).await?;
    Ok(ApiResponse::success(config))
}

/// `PUT /api/v1/system/time/config`
///
/// 将命令执行（阻塞）和 DB 写入（异步）拆分：命令在 spawn_blocking 中执行，
/// DB 写入在 async 上下文中完成，避免 block_on 嵌套问题。
pub async fn update_time_config(
    State(state): State<AppState>,
    Json(config): Json<types::TimeConfig>,
) -> Result<ApiResponse<types::TimeConfig>, ApiError> {
    // 1. 在阻塞线程中执行 OS 命令（timedatectl）
    let timezone = config.timezone.clone();
    let ntp_enabled = config.ntp_enabled;
    tokio::task::spawn_blocking(move || {
        crate::time_service::TimeService::apply_time_config_sync(&timezone, ntp_enabled)
    })
    .await
    .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;

    // 2. 在 async 上下文中执行 DB 写入（不阻塞线程池）
    let json = serde_json::to_string(&config)
        .map_err(|e| ApiError::TimeConfig(format!("序列化配置失败: {e}")))?;
    db::SystemConfigRepo::set(&state.db, "time_config", &json)
        .await
        .map_err(|e| ApiError::TimeConfig(format!("保存配置失败: {e}")))?;

    Ok(ApiResponse::success(config))
}

/// `POST /api/v1/system/time/sync`
pub async fn force_time_sync(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::ForceSyncResponse>, ApiError> {
    let synced = tokio::task::spawn_blocking(crate::time_service::TimeService::force_sync_sync)
        .await
        .map_err(|e| ApiError::TimeSyncFailed(format!("spawn_blocking 失败: {e}")))??;
    Ok(ApiResponse::success(types::ForceSyncResponse { synced }))
}

/// `POST /api/v1/system/time/set`
pub async fn set_system_time(
    State(_state): State<AppState>,
    Json(body): Json<crate::time_service::SetTimeRequest>,
) -> Result<ApiResponse<types::SetTimeResponse>, ApiError> {
    let time_str = body.time.clone();
    let (previous_time, new_time) = tokio::task::spawn_blocking(move || {
        crate::time_service::TimeService::set_system_time_sync(&time_str)
    })
    .await
    .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;

    Ok(ApiResponse::success(types::SetTimeResponse {
        applied: true,
        previous_time,
        new_time,
    }))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        // 系统概览
        .route("/overview", axum::routing::get(get_overview))
        // 网络配置
        .route(
            "/network/interfaces",
            axum::routing::get(get_network_interfaces),
        )
        .route(
            "/network/interfaces/{name}",
            axum::routing::get(get_network_interface).put(update_network_interface),
        )
        .route(
            "/network/changes/pending",
            axum::routing::get(get_pending_network_change),
        )
        .route(
            "/network/changes/{id}/confirm",
            axum::routing::post(confirm_network_change),
        )
        .route(
            "/network/changes/{id}/cancel",
            axum::routing::post(cancel_network_change),
        )
        // 存储与保留策略
        .route("/storage/status", axum::routing::get(get_storage_status))
        .route(
            "/storage/config",
            axum::routing::get(get_storage_config).put(update_storage_config),
        )
        .route(
            "/storage/cleanup",
            axum::routing::post(trigger_storage_cleanup),
        )
        // 对时服务
        .route("/time/status", axum::routing::get(get_time_status))
        .route(
            "/time/config",
            axum::routing::get(get_time_config).put(update_time_config),
        )
        .route("/time/sync", axum::routing::post(force_time_sync))
        .route("/time/set", axum::routing::post(set_system_time))
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

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn network_interfaces_returns_macos_interfaces() {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let app = router().with_state(AppState::new(db, pipeline));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/network/interfaces")
                    .body(Body::empty())
                    .expect("构造请求失败"),
            )
            .await
            .expect("执行请求失败");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体失败");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("解析网卡响应失败");
        assert_eq!(payload["code"], 0);
        let interfaces = payload["data"]["interfaces"]
            .as_array()
            .expect("interfaces 应为数组");
        assert!(!interfaces.is_empty());
        assert!(interfaces
            .iter()
            .any(|interface| interface["name"] == "lo0"));
    }
}
