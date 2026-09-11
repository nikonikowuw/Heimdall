use std::collections::HashSet;

use axum::extract::State;

use crate::error::ApiError;
use crate::metrics::{
    CpuCollector, DiskCollector, MemoryCollector, NetworkCollector, ThermalCollector,
};
use crate::response::ApiResponse;
use crate::state::AppState;

use super::round_1dp;

/// `GET /api/v1/system/overview`
pub async fn get_overview(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::SystemOverview>, ApiError> {
    // 并行采集所有系统指标
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

    let npu_metrics = tokio::task::spawn_blocking(detect_npu_metrics)
        .await
        .unwrap_or(None);

    let host_info = tokio::task::spawn_blocking(crate::system_info::read_host_metadata)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))?;

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

    let mut thermal_overview = thermal_metrics;
    for zone in &mut thermal_overview.zones {
        zone.temperature = round_1dp(zone.temperature as f64) as f32;
    }

    let memory_usage_percent = if memory_metrics.total_mb > 0 {
        (memory_metrics.used_mb as f64 / memory_metrics.total_mb as f64) * 100.0
    } else {
        0.0
    };

    let (primary_interface, ip_address, mac_address) =
        crate::network_service::detector::detect_primary_network_identity().await;

    Ok(ApiResponse::success(types::SystemOverview {
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        device_model: host_info.device_model,
        os_info: host_info.os_info,
        kernel_version: host_info.kernel_version,
        uptime_seconds: host_info.uptime_seconds,
        ip_address,
        mac_address,
        primary_interface,
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
        cpu: cpu_overview,
        memory: memory_metrics,
        npu: npu_metrics,
        network: network_metrics,
        thermal: thermal_overview,
        disk: disk_overview,
    }))
}

/// 检测 NPU 指标（统一调用 infer 跨平台接口，不可用时返回 None）
fn detect_npu_metrics() -> Option<types::NpuMetrics> {
    let monitor = infer::global_monitor();
    let devices = monitor.collect_all();
    if devices.is_empty() {
        return None;
    }

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
