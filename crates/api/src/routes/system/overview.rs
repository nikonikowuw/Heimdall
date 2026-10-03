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

    // “今日”起点必须与**设备本地零点**对齐，而不是 UTC 零点。
    //
    // 告警列表的默认时间窗由前端按本地零点计算（`AlarmsPage.tsx` 的
    // `getInitialTodayRange`），而卡片原本取 UTC 零点：UTC+8 下每天 00:00–08:00
    // 这 8 小时内「今日告警」会少算当天的告警，与列表数字互相矛盾。
    //
    // 时区取进程本地时区（设备时区由系统设置页经 `timedatectl set-timezone` 写入，
    // 与 `TimeStatus.timezone` 同源；单机部署下与浏览器本地时区一致）。
    // 最小根文件系统缺 tzdata 时 `chrono::Local` 回退为 UTC，即退回原先行为，
    // 不会 panic（见下方 `local_today_start`）。
    let today_start = local_today_start();

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
        .and_then(|cleaner| {
            let stat = cleaner.current_fs_stat().ok()?;
            let used_bytes = stat.total_bytes.saturating_sub(stat.available_bytes);
            let raw_pct = if stat.total_bytes > 0 {
                (used_bytes as f64 / stat.total_bytes as f64) * 100.0
            } else {
                0.0
            };
            Some((
                types::DiskMetrics {
                    total_gb: round_1dp(stat.total_bytes as f64 / GIB),
                    used_gb: round_1dp(used_bytes as f64 / GIB),
                    available_gb: round_1dp(stat.available_bytes as f64 / GIB),
                    inode_total: stat.total_inodes,
                    inode_used: stat.total_inodes.saturating_sub(stat.available_inodes),
                    inode_available: stat.available_inodes,
                    mount_info: Some(cleaner.mount_info()),
                },
                round_1dp(raw_pct),
            ))
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

/// 设备本地「今日零点」对应的 UTC 时刻。
///
/// 时间列以 RFC3339 落盘，绑定必须用带时区的 `DateTimeUtc`
/// （见 [数据库规范](../../../.trellis/spec/db/backend/database-guidelines.md#表与查询)），
/// 所以返回 `DateTime<Utc>` 而非 `NaiveDateTime`。
///
/// **不能**用「当前偏移」直接回推零点：在夏令时切换日，零点自身的偏移与此刻不同
/// （如欧洲 DST 切日），会差 1 小时。这里先按当前偏移推断本地日期，
/// 再用**该本地时刻自身**的偏移去解释它（`from_local_datetime`）。
///
/// 公开给集成测试作为 oracle：测试必须与实现同算法，
/// 否则测的是「两边算得一样」而不是行为正确性（参见本函数原先在测试里的手抄副本，
/// 它用固定偏移，在 DST 时区会与实现差 60 分钟）。
///
/// 缺 tzdata 时 `chrono::Local` 退化为 UTC，即退回修复前行为，不会 panic。
pub fn local_today_start() -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;

    let naive_midnight = match (chrono::Utc::now() + *chrono::Local::now().offset())
        .date_naive()
        .and_hms_opt(0, 0, 0)
    {
        Some(naive) => naive,
        None => return chrono::Utc::now(),
    };

    match chrono::Local.from_local_datetime(&naive_midnight) {
        // 常规情况
        chrono::LocalResult::Single(dt) => dt.with_timezone(&chrono::Utc),
        // 推后一小时的 DST 切换日：零点出现两次，取第一次
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&chrono::Utc),
        // 前跳的 DST 切换日：本地不存在零点（如某些时区的 00:00 被跳过），
        // 回退到纯函数按当前偏移估算
        chrono::LocalResult::None => {
            today_start_utc(chrono::Utc::now(), *chrono::Local::now().offset())
        }
    }
}

/// 给定「当前时刻」与「本地时区偏移」，求该时区下今日零点对应的 UTC 时刻。
///
/// 抽成纯函数以便稳定测试：调用方不从环境读时区，测试也无须改 `TZ`
/// （改进程环境变量会让同进程的其他测试受影响）。
fn today_start_utc(
    now: chrono::DateTime<chrono::Utc>,
    offset: chrono::FixedOffset,
) -> chrono::DateTime<chrono::Utc> {
    (now + offset)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|local_midnight| (local_midnight - offset).and_utc())
        .unwrap_or(now)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, s)
            .single()
            .expect("valid UTC timestamp in tests")
    }

    fn offset_hours(hours: i32) -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(hours * 3600).expect("valid offset in tests")
    }

    /// UTC+8 下，UTC 时间 2026-10-02T02:00Z 的「今日」应从本地零点
    /// 2026-10-01T16:00Z 起算——而不是 UTC 零点 2026-10-02T00:00Z。
    ///
    /// 这正是修复前的缺陷：00:00–08:00（UTC+8）区间内卡片会漏算当天的告警。
    #[test]
    fn today_start_uses_local_midnight_not_utc_midnight() {
        let start = today_start_utc(utc(2026, 10, 2, 2, 0, 0), offset_hours(8));

        assert_eq!(
            start,
            utc(2026, 10, 1, 16, 0, 0),
            "UTC+8 的今日零点应换算为前一日 16:00Z"
        );
        assert_ne!(start, utc(2026, 10, 2, 0, 0, 0), "不得退回 UTC 零点口径");
    }

    /// 边界自洽：起始时刻本身必须落在设备本地零点（偏移后的钟面时间为 00:00:00）。
    #[test]
    fn today_start_lands_on_local_midnight() {
        for hours in [-8i32, -5, 0, 5, 8, 13] {
            let offset = offset_hours(hours);
            // 选一个落在本地 00:30 的时刻，确保「今日」不是前一天
            let local_now = chrono::NaiveDate::from_ymd_opt(2026, 10, 2)
                .expect("valid date")
                .and_hms_opt(0, 30, 0)
                .expect("valid time");
            let now = (local_now - offset).and_utc();

            let start_local = today_start_utc(now, offset) + offset;

            assert_eq!(start_local.hour(), 0, "hours={hours}");
            assert_eq!(start_local.minute(), 0, "hours={hours}");
            assert_eq!(start_local.second(), 0, "hours={hours}");
            assert_eq!(
                start_local.date_naive(),
                chrono::NaiveDate::from_ymd_opt(2026, 10, 2).expect("valid date"),
                "hours={hours}"
            );
        }
    }

    /// 负偏移（美洲时区）同样成立：UTC-5 下本地 00:30 对应 UTC 同日 05:30，
    /// 今日零点应为 UTC 05:00。
    #[test]
    fn today_start_handles_negative_offsets() {
        assert_eq!(
            today_start_utc(utc(2026, 10, 2, 5, 30, 0), offset_hours(-5)),
            utc(2026, 10, 2, 5, 0, 0)
        );
    }

    /// 跨月/跨年边界不得串日期。
    #[test]
    fn today_start_crosses_month_and_year_boundaries() {
        let cst = offset_hours(8);

        // UTC 2026-12-31T17:00Z = 本地 2027-01-01T01:00 → 今日零点 2026-12-31T16:00Z
        assert_eq!(
            today_start_utc(utc(2026, 12, 31, 17, 0, 0), cst),
            utc(2026, 12, 31, 16, 0, 0)
        );

        // UTC 2026-10-31T17:00Z = 本地 2026-11-01T01:00 → 今日零点 2026-10-31T16:00Z
        assert_eq!(
            today_start_utc(utc(2026, 10, 31, 17, 0, 0), cst),
            utc(2026, 10, 31, 16, 0, 0)
        );
    }

    /// 缺 tzdata / 未知时区时偏移为 0，退化为 UTC 零点，不得 panic。
    #[test]
    fn today_start_falls_back_to_utc_when_offset_is_zero() {
        assert_eq!(
            today_start_utc(utc(2026, 10, 2, 11, 30, 0), offset_hours(0)),
            utc(2026, 10, 2, 0, 0, 0)
        );
    }
}
