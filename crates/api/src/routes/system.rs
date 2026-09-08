use axum::{extract::State, Json};

use std::collections::HashSet;

use crate::error::ApiError;
use crate::metrics::{
    CpuCollector, DiskCollector, MemoryCollector, NetworkCollector, ThermalCollector,
};
use crate::response::ApiResponse;
use crate::state::AppState;

#[inline]
fn round_1dp(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ─── 系统概览 ───

/// `GET /api/v1/system/overview`
pub async fn get_overview(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::SystemOverview>, ApiError> {
    // 并行采集所有系统指标
    // 磁盘采集使用根路径；若 storage_cleaner 可用，后续以 cleaner 的 fs_stat 覆盖
    let (
        cpu_metrics,
        memory_metrics,
        disk_metrics,
        network_metrics,
        thermal_metrics,
        top_processes,
    ) = tokio::join!(
        CpuCollector::collect(),
        MemoryCollector::collect(),
        DiskCollector::collect(),
        NetworkCollector::collect_all(),
        ThermalCollector::collect(),
        CpuCollector::get_top_processes(5),
    );

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

    // NPU 状态（平台相关，不可用时返回 None）
    // detect_npu_metrics 内部涉及 sysfs 探测，必须在阻塞线程中执行
    let npu_metrics = tokio::task::spawn_blocking(detect_npu_metrics)
        .await
        .unwrap_or(None);

    // 获取基础设备与系统运行元数据（使用 spawn_blocking 避免阻塞，耗时 < 1ms，无冗余休眠）
    let host_info = tokio::task::spawn_blocking(crate::system_info::read_host_metadata)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))?;

    // 收集器已直接产出 types:: API 类型，无需逐字段转换
    // 仅对需要精度控制的字段应用 round_1dp
    let mut cpu_overview = cpu_metrics;
    cpu_overview.overall_percent = round_1dp(cpu_overview.overall_percent);
    for core in &mut cpu_overview.per_core {
        core.usage_percent = round_1dp(core.usage_percent);
    }
    let mut procs = top_processes;
    for proc in &mut procs {
        proc.cpu_percent = round_1dp(proc.cpu_percent);
    }
    cpu_overview.top_processes = procs;

    // 磁盘：若 storage_cleaner 已装配，以 evidence_dir 所在物理分区为准（保证与存储页一致）；
    // 否则使用 collector 采集的根分区数据作为 fallback
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let (disk_overview, disk_usage_percent) = state
        .storage_cleaner
        .as_ref()
        .and_then(|cleaner| cleaner.current_fs_stat().ok())
        .map(|stat| {
            let used_bytes = stat.total_bytes.saturating_sub(stat.available_bytes);
            let raw_pct = if stat.total_bytes > 0 {
                (used_bytes as f64 / stat.total_bytes as f64) * 100.0
            } else {
                0.0
            };
            (
                types::DiskMetrics {
                    total_gb: round_1dp(stat.total_bytes as f64 / GIB),
                    used_gb: round_1dp(used_bytes as f64 / GIB),
                    available_gb: round_1dp(stat.available_bytes as f64 / GIB),
                    inode_total: stat.total_inodes,
                    inode_used: stat.total_inodes.saturating_sub(stat.available_inodes),
                    inode_available: stat.available_inodes,
                },
                round_1dp(raw_pct),
            )
        })
        .unwrap_or_else(|| {
            let raw_pct = if disk_metrics.total_gb > 0.0 {
                (disk_metrics.used_gb / disk_metrics.total_gb) * 100.0
            } else {
                0.0
            };
            (disk_metrics, round_1dp(raw_pct))
        });

    // 温度：对各 zone 温度应用精度控制
    let mut thermal_overview = thermal_metrics;
    for zone in &mut thermal_overview.zones {
        zone.temperature = round_1dp(zone.temperature as f64) as f32;
    }

    // 计算旧字段的兼容值
    let memory_usage_percent = if memory_metrics.total_mb > 0 {
        (memory_metrics.used_mb as f64 / memory_metrics.total_mb as f64) * 100.0
    } else {
        0.0
    };

    Ok(ApiResponse::success(types::SystemOverview {
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        device_model: host_info.device_model,
        os_info: host_info.os_info,
        kernel_version: host_info.kernel_version,
        uptime_seconds: host_info.uptime_seconds,
        // 旧字段（向后兼容）
        cpu_usage_percent: cpu_overview.overall_percent,
        memory_usage_percent: round_1dp(memory_usage_percent),
        memory_used_mb: memory_metrics.used_mb,
        memory_total_mb: memory_metrics.total_mb,
        npu_usage_percent: npu_metrics
            .as_ref()
            .and_then(|m| m.avg_utilization())
            .map(round_1dp),
        disk_usage_percent,
        disk_used_gb: disk_overview.used_gb,
        disk_total_gb: disk_overview.total_gb,
        active_cameras,
        total_cameras,
        active_tasks,
        today_alarms,
        today_captures,
        // 新字段
        cpu: cpu_overview,
        memory: memory_metrics,
        npu: npu_metrics,
        network: network_metrics,
        thermal: thermal_overview,
        disk: disk_overview,
    }))
}

/// 检测 NPU 指标（统一调用 infer 跨平台接口，不可用时返回 None）
///
/// 合并所有 NPU 设备的核心列表，避免多设备场景下静默丢弃额外设备。
/// 注意：此函数涉及 sysfs/驱动探测，调用方应通过 `spawn_blocking` 执行。
fn detect_npu_metrics() -> Option<types::NpuMetrics> {
    let monitor = infer::global_monitor();
    let devices = monitor.collect_all();
    if devices.is_empty() {
        return None;
    }

    // 合并所有设备的核心列表，汇总内存、温度与推理次数
    let mut all_cores = Vec::new();
    let mut total_memory_mb = 0u64;
    let mut used_memory_mb = 0u64;
    let mut max_temperature: Option<f32> = None;
    let mut total_sessions = 0u32;
    let mut total_inference_count = 0u64;
    let mut device_type_set: HashSet<String> = HashSet::new();

    for device in &devices {
        total_memory_mb += device.total_memory_mb;
        used_memory_mb += device.used_memory_mb;
        total_sessions += device.active_sessions;
        total_inference_count += device.inference_count;
        max_temperature = match (max_temperature, device.temperature) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (None, Some(b)) => Some(b),
            (a, None) => a,
        };
        device_type_set.insert(device.device_type.to_string());
        for c in &device.cores {
            all_cores.push(types::NpuCoreMetrics {
                core_id: c.core_id,
                utilization_percent: round_1dp(c.utilization_percent),
                frequency_mhz: c.frequency_mhz,
                power_watts: c.power_watts,
            });
        }
    }

    let mut device_types: Vec<String> = device_type_set.into_iter().collect();
    device_types.sort();
    let device_type = device_types.join(", ");

    Some(types::NpuMetrics {
        device_type,
        cores: all_cores,
        total_memory_mb,
        used_memory_mb,
        temperature: max_temperature,
        active_sessions: total_sessions,
        inference_count: total_inference_count,
    })
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
    let op = crate::network_service::NetworkService::get_pending_operation().await?;
    Ok(ApiResponse::success(op))
}

/// `POST /api/v1/system/network/changes/:id/confirm`
pub async fn confirm_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    let result = crate::network_service::NetworkService::confirm_operation(&id).await?;
    Ok(ApiResponse::success(result))
}

/// `POST /api/v1/system/network/changes/:id/cancel`
pub async fn cancel_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    let result = crate::network_service::NetworkService::cancel_operation(&id).await?;
    Ok(ApiResponse::success(result))
}

/// `POST /api/v1/system/network/diagnose`
pub async fn diagnose_network(
    State(_state): State<AppState>,
    axum::Json(req): axum::Json<types::NetworkDiagnosticRequest>,
) -> Result<ApiResponse<types::NetworkDiagnosticResult>, ApiError> {
    let result = crate::network_service::NetworkService::diagnose(&req).await?;
    Ok(ApiResponse::success(result))
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
        .route("/network/diagnose", axum::routing::post(diagnose_network))
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
