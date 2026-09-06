//! 对时服务：封装宿主 OS 的 NTP 与时间管理能力
//!
//! 检测顺序：timedatectl → chrony → systemd-timesyncd
//! 不自建 NTP 客户端，只封装已有服务。
//!
//! 所有涉及 `Command::output()` 的方法均为同步阻塞版本（`_sync` 后缀），
//! 由调用方通过 `tokio::task::spawn_blocking` 在专用阻塞线程池中执行，
//! 严禁在 async 上下文中直接调用。

use std::process::Command;

use crate::error::ApiError;

/// 手动设置时间的请求体
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SetTimeRequest {
    pub time: String,
}

/// 对时服务
#[derive(Debug)]
pub struct TimeService;

impl TimeService {
    // ─── 同步阻塞版本（在 spawn_blocking 线程中调用） ───

    /// 获取系统时间状态（同步阻塞）
    pub fn get_status_sync() -> Result<types::TimeStatus, ApiError> {
        let system_time = chrono::Utc::now().timestamp_millis();

        // 读取时区
        let timezone = run_command("timedatectl", &["show", "-p", "Timezone", "--value"])
            .unwrap_or_else(|_| "UTC".to_string())
            .trim()
            .to_string();

        // 计算时区偏移（秒）——使用 chrono 本地时间获取真实偏移，不使用硬编码映射
        let timezone_offset = compute_timezone_offset();

        // NTP 同步状态
        let ntp_synced = run_command("timedatectl", &["show", "-p", "NTPSynchronized", "--value"])
            .map(|s| s.trim() == "yes")
            .unwrap_or(false);

        // 检测 NTP 服务
        let ntp_service = detect_ntp_service();

        // NTP 服务器
        let ntp_server = get_ntp_server(&ntp_service);

        // 时间偏移
        let offset_ms = get_time_offset(&ntp_service);

        Ok(types::TimeStatus {
            system_time,
            timezone,
            timezone_offset,
            ntp_synced,
            ntp_service,
            ntp_server,
            offset_ms,
        })
    }

    /// 获取当前配置（从 DB 读取，返回默认值作为 fallback）
    pub async fn get_config(
        db: &sea_orm::DatabaseConnection,
    ) -> Result<types::TimeConfig, ApiError> {
        if let Ok(Some(json_str)) = db::SystemConfigRepo::get(db, "time_config").await {
            if let Ok(config) = serde_json::from_str::<types::TimeConfig>(&json_str) {
                return Ok(config);
            }
        }
        // 从系统读取当前状态作为默认值
        let status = tokio::task::spawn_blocking(Self::get_status_sync)
            .await
            .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))?
            .map_err(|e| ApiError::SystemInfo(e.to_string()))?;
        Ok(types::TimeConfig {
            ntp_enabled: status.ntp_synced || !status.ntp_service.is_empty(),
            ntp_server: status
                .ntp_server
                .unwrap_or_else(|| "pool.ntp.org".to_string()),
            timezone: status.timezone,
        })
    }

    /// 应用时间配置（同步阻塞——仅执行 OS 命令，DB 写入由调用方在 async 上下文中完成）
    pub fn apply_time_config_sync(timezone: &str, ntp_enabled: bool) -> Result<(), ApiError> {
        #[cfg(target_os = "macos")]
        {
            tracing::info!(
                timezone,
                ntp_enabled,
                "macOS 开发调试环境：模拟应用时间配置成功"
            );
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            // 设置时区
            run_command("timedatectl", &["set-timezone", timezone])
                .map_err(|e| ApiError::TimeConfig(format!("设置时区失败: {e}")))?;

            // 启用/禁用 NTP
            let ntp_arg = if ntp_enabled { "true" } else { "false" };
            run_command("timedatectl", &["set-ntp", ntp_arg])
                .map_err(|e| ApiError::TimeConfig(format!("设置 NTP 失败: {e}")))?;

            Ok(())
        }
    }

    /// 强制 NTP 同步（同步阻塞）
    pub fn force_sync_sync() -> Result<bool, ApiError> {
        #[cfg(target_os = "macos")]
        {
            tracing::info!("macOS 开发调试环境：模拟强制 NTP 同步成功");
            Ok(true)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let service = detect_ntp_service();
            match service.as_str() {
                "chrony" => match run_command("chronyc", &["makestep"]) {
                    Ok(_) => Ok(true),
                    Err(e) => {
                        tracing::warn!(error = %e, "chrony 强制同步失败，回退到 timedatectl");
                        run_command("timedatectl", &["set-ntp", "true"])
                            .map(|_| true)
                            .map_err(|e| ApiError::TimeSyncFailed(format!("NTP 同步失败: {e}")))
                    }
                },
                "timesyncd" => run_command("systemctl", &["restart", "systemd-timesyncd"])
                    .map(|_| true)
                    .map_err(|e| ApiError::TimeSyncFailed(format!("timesyncd 重启失败: {e}"))),
                "ntpd" => run_command("ntpd", &["-qg"])
                    .map(|_| true)
                    .map_err(|e| ApiError::TimeSyncFailed(format!("ntpd 同步失败: {e}"))),
                _ => run_command("timedatectl", &["set-ntp", "true"])
                    .map(|_| true)
                    .map_err(|e| ApiError::TimeSyncFailed(format!("NTP 同步失败: {e}"))),
            }
        }
    }

    /// 手动设置系统时间（同步阻塞，高风险操作）
    pub fn set_system_time_sync(time_str: &str) -> Result<(i64, i64), ApiError> {
        // 校验时间格式
        let parsed = chrono::NaiveDateTime::parse_from_str(time_str, "%Y-%m-%d %H:%M:%S")
            .map_err(|e| ApiError::TimeConfig(format!("时间格式无效: {e}")))?;

        // 安全边界校验：不能设置到太远的过去或未来
        let now = chrono::Utc::now();
        let target = parsed.and_utc();
        let diff_days = (target - now).num_days().abs();
        if diff_days > 365 {
            return Err(ApiError::TimeDeltaTooLarge);
        }

        let previous_time = now.timestamp_millis();

        // 记录操作日志
        tracing::warn!(
            old_time = %now.format("%Y-%m-%d %H:%M:%S"),
            new_time = time_str,
            "手动设置系统时间"
        );

        #[cfg(target_os = "macos")]
        {
            tracing::warn!(
                target_time = time_str,
                "macOS 开发调试环境：模拟手动设置系统时间成功"
            );
            Ok((previous_time, parsed.and_utc().timestamp_millis()))
        }

        #[cfg(not(target_os = "macos"))]
        {
            // 检查 NTP 状态——Linux timedatectl 会拒绝在 NTP 开启时执行 set-time
            let ntp_active = run_command("timedatectl", &["show", "-p", "NTP", "--value"])
                .map(|s| s.trim() == "yes")
                .unwrap_or(false);
            if ntp_active {
                tracing::warn!(
                    old_time = %now.format("%Y-%m-%d %H:%M:%S"),
                    new_time = time_str,
                    "检测到 NTP 正在运行，手动设置系统时间前临时停用 NTP 自动同步"
                );
                let _ = run_command("timedatectl", &["set-ntp", "false"]);
            }

            // 执行设置
            run_command("timedatectl", &["set-time", time_str])
                .map_err(|e| ApiError::TimeConfig(format!("设置系统时间失败: {e}")))?;

            // 验证新时间
            let new_time = chrono::Utc::now().timestamp_millis();

            Ok((previous_time, new_time))
        }
    }
}

// ─── 辅助函数（同步阻塞） ───

fn run_command(cmd: &str, args: &[&str]) -> Result<String, std::io::Error> {
    let output = Command::new(cmd).args(args).output()?;
    if output.status.success() {
        String::from_utf8(output.stdout)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::other(format!("{cmd} failed: {stderr}")))
    }
}

fn detect_ntp_service() -> String {
    if Command::new("systemctl")
        .args(["is-active", "chrony"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "chrony".to_string();
    }

    if Command::new("systemctl")
        .args(["is-active", "systemd-timesyncd"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "timesyncd".to_string();
    }

    if Command::new("systemctl")
        .args(["is-active", "ntp"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return "ntpd".to_string();
    }

    "none".to_string()
}

fn get_ntp_server(service: &str) -> Option<String> {
    match service {
        "chrony" => {
            let output = Command::new("chronyc")
                .args(["sources", "-n"])
                .output()
                .ok()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.starts_with('*') {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        return Some(parts[1].to_string());
                    }
                }
            }
            None
        }
        "timesyncd" => {
            let output = run_command("timedatectl", &["show", "-p", "Server", "--value"]).ok()?;
            let server = output.trim().to_string();
            if server.is_empty() {
                None
            } else {
                Some(server)
            }
        }
        _ => None,
    }
}

fn get_time_offset(service: &str) -> Option<i64> {
    match service {
        "chrony" => {
            let output = Command::new("chronyc").args(["tracking"]).output().ok()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("Last offset") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 {
                        let val_str = parts[1].split_whitespace().next()?;
                        let val: f64 = val_str.parse().ok()?;
                        return Some((val * 1000.0) as i64);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// 计算时区偏移（秒），使用系统本地时间获取当前真实偏移。
fn compute_timezone_offset() -> i64 {
    chrono::Local::now().offset().local_minus_utc() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_ntp_service() {
        let _service = detect_ntp_service();
        // 不 panic 即可
    }

    #[test]
    fn test_compute_timezone_offset_returns_reasonable_value() {
        let offset = compute_timezone_offset();
        assert!(
            (-12 * 3600..=14 * 3600).contains(&offset),
            "offset should be within UTC-12 to UTC+14, got {offset}"
        );
    }
}
