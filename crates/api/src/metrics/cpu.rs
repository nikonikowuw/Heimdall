//! CPU 指标采集器
//!
//! Linux: 从 `/proc/stat` 读取两次采样计算差值，得到准确的瞬时 CPU 使用率。
//! macOS: 通过 Mach `host_processor_info` 获取瞬时每核心 CPU tick 采样。

use types::{CoreMetrics, CpuMetrics, ProcessMetrics};

/// `/proc/stat` 中解析出的单行 tick 计数
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Default)]
struct CpuTicks {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
}

#[cfg(target_os = "linux")]
impl CpuTicks {
    fn busy(&self) -> u64 {
        self.user + self.nice + self.system
    }

    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait
    }
}

/// CPU 采集器
#[derive(Debug)]
pub struct CpuCollector;

impl CpuCollector {
    /// 采集 CPU 指标（两次采样取差值，间隔 50ms）
    pub async fn collect() -> CpuMetrics {
        #[cfg(target_os = "linux")]
        {
            Self::collect_linux().await
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::collect_fallback().await
        }
    }

    /// Linux 实现：两次采样取差值，得到准确的瞬时使用率
    #[cfg(target_os = "linux")]
    async fn collect_linux() -> CpuMetrics {
        let snapshot1 = Self::read_proc_stat().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let snapshot2 = Self::read_proc_stat().await;

        let overall_percent =
            if let (Some(t1), Some(t2)) = (snapshot1.get("cpu"), snapshot2.get("cpu")) {
                Self::compute_usage_pct(t1, t2)
            } else {
                0.0
            };

        let mut per_core = Vec::new();
        for (name, t1) in &snapshot1 {
            if name == "cpu" {
                continue;
            }
            if let Some(t2) = snapshot2.get(name) {
                let core_id: u32 = name.trim_start_matches("cpu").parse().unwrap_or(0);
                let usage = Self::compute_usage_pct(t1, t2);
                let frequency_mhz = Self::read_core_frequency_linux(core_id).await;
                per_core.push(CoreMetrics {
                    core_id,
                    usage_percent: usage,
                    frequency_mhz,
                    temperature: None,
                });
            }
        }

        // 按 core_id 升序排列
        per_core.sort_by_key(|c| c.core_id);

        let temperature = Self::read_temperature_linux().await;
        let frequency_mhz = Self::read_frequency_linux().await;

        CpuMetrics {
            overall_percent,
            per_core,
            temperature,
            frequency_mhz,
            top_processes: Vec::new(),
        }
    }

    #[cfg(target_os = "linux")]
    fn compute_usage_pct(prev: &CpuTicks, curr: &CpuTicks) -> f64 {
        let total_delta = curr.total().saturating_sub(prev.total());
        if total_delta == 0 {
            return 0.0;
        }
        let busy_delta = curr.busy().saturating_sub(prev.busy());
        (busy_delta as f64 / total_delta as f64) * 100.0
    }

    #[cfg(target_os = "linux")]
    async fn read_proc_stat() -> std::collections::HashMap<String, CpuTicks> {
        let content = tokio::fs::read_to_string("/proc/stat")
            .await
            .unwrap_or_default();

        let mut map = std::collections::HashMap::new();
        for line in content.lines() {
            if !line.starts_with("cpu") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let name = parts[0].to_string();
            let ticks = CpuTicks {
                user: parts[1].parse().unwrap_or(0),
                nice: parts[2].parse().unwrap_or(0),
                system: parts[3].parse().unwrap_or(0),
                idle: parts[4].parse().unwrap_or(0),
                iowait: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0),
            };
            map.insert(name, ticks);
        }
        map
    }

    #[cfg(target_os = "linux")]
    async fn read_core_frequency_linux(core_id: u32) -> Option<u32> {
        let freq_path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq",
            core_id
        );
        let content = tokio::fs::read_to_string(&freq_path).await.ok()?;
        let freq_khz: u64 = content.trim().parse().ok()?;
        Some((freq_khz / 1000) as u32)
    }

    #[cfg(target_os = "linux")]
    async fn read_temperature_linux() -> Option<f32> {
        for zone_id in 0..10 {
            let path = format!("/sys/class/thermal/thermal_zone{}/temp", zone_id);
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Ok(temp_millideg) = content.trim().parse::<i32>() {
                    let temp = temp_millideg as f32 / 1000.0;
                    if temp > 20.0 && temp < 120.0 {
                        return Some(temp);
                    }
                }
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    async fn read_frequency_linux() -> Option<u32> {
        let path = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq";
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Ok(freq_khz) = content.trim().parse::<u64>() {
                return Some((freq_khz / 1000) as u32);
            }
        }
        None
    }

    /// macOS 实现：使用 Mach host_processor_info 两次采样差值
    #[cfg(target_os = "macos")]
    async fn collect_fallback() -> CpuMetrics {
        let sample1 = super::macos_ticks::sample_macos_per_core_ticks();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let sample2 = super::macos_ticks::sample_macos_per_core_ticks();

        match (sample1, sample2) {
            (Some(cores1), Some(cores2)) if cores1.len() == cores2.len() && !cores1.is_empty() => {
                let mut per_core = Vec::with_capacity(cores1.len());
                let mut total_busy_delta = 0u64;
                let mut total_delta_all = 0u64;

                for (idx, ((all1, idle1), (all2, idle2))) in
                    cores1.into_iter().zip(cores2).enumerate()
                {
                    let delta_all = all2.saturating_sub(all1);
                    let delta_idle = idle2.saturating_sub(idle1);
                    let usage = if delta_all > 0 {
                        let delta_busy = delta_all.saturating_sub(delta_idle);
                        total_busy_delta = total_busy_delta.saturating_add(delta_busy);
                        total_delta_all = total_delta_all.saturating_add(delta_all);
                        ((delta_busy as f64 / delta_all as f64) * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    };
                    per_core.push(CoreMetrics {
                        core_id: idx as u32,
                        usage_percent: usage,
                        frequency_mhz: None,
                        temperature: None,
                    });
                }

                let overall_percent = if total_delta_all > 0 {
                    ((total_busy_delta as f64 / total_delta_all as f64) * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };

                CpuMetrics {
                    overall_percent,
                    per_core,
                    temperature: None,
                    frequency_mhz: None,
                    top_processes: Vec::new(),
                }
            }
            _ => CpuMetrics::default(),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    async fn collect_fallback() -> CpuMetrics {
        CpuMetrics::default()
    }

    /// 读取系统总内存 (MB)，用于将 ps %mem 转换为绝对值
    #[cfg(target_os = "linux")]
    async fn total_memory_mb() -> u64 {
        tokio::fs::read_to_string("/proc/meminfo")
            .await
            .ok()
            .and_then(|c| {
                c.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| {
                        l.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse::<u64>().ok())
                    })
            })
            .map(|kb| kb / 1024)
            .unwrap_or(0)
    }

    #[cfg(not(target_os = "linux"))]
    async fn total_memory_mb() -> u64 {
        0
    }

    /// 读取 top N 进程（按 CPU 使用率排序）
    pub async fn get_top_processes(n: usize) -> Vec<ProcessMetrics> {
        use tokio::process::Command;

        #[cfg(target_os = "linux")]
        let args: &[&str] = &["-eo", "pid,comm,%cpu,%mem", "--sort=-%cpu"];
        #[cfg(target_os = "macos")]
        let args: &[&str] = &["-eo", "pid,comm,%cpu,%mem", "-r"];
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let args: &[&str] = &["-eo", "pid,comm,%cpu,%mem"];

        let (output, total_mb) = tokio::join!(
            Command::new("ps").args(args).output(),
            Self::total_memory_mb(),
        );

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut processes = Vec::new();

                for (i, line) in stdout.lines().enumerate() {
                    if i == 0 || processes.len() >= n {
                        continue;
                    }

                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        if let Ok(pid) = parts[0].parse::<u32>() {
                            let name = parts[1].to_string();
                            let cpu_percent = parts[2].parse::<f64>().unwrap_or(0.0);
                            let mem_percent = parts[3].parse::<f64>().unwrap_or(0.0);
                            let memory_mb = if total_mb > 0 {
                                (mem_percent / 100.0 * total_mb as f64) as u64
                            } else {
                                0
                            };

                            processes.push(ProcessMetrics {
                                pid,
                                name,
                                cpu_percent,
                                memory_mb,
                            });
                        }
                    }
                }

                processes
            }
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cpu_collector() {
        let metrics = CpuCollector::collect().await;
        assert!(metrics.overall_percent >= 0.0);
        assert!(metrics.overall_percent <= 100.0);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_compute_usage_pct() {
        let prev = CpuTicks {
            user: 1000,
            nice: 100,
            system: 200,
            idle: 5000,
            iowait: 50,
        };
        let curr = CpuTicks {
            user: 1200,
            nice: 110,
            system: 220,
            idle: 5100,
            iowait: 50,
        };
        let pct = CpuCollector::compute_usage_pct(&prev, &curr);
        assert!((pct - 69.7).abs() < 1.0);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_compute_usage_pct_zero_delta() {
        let t = CpuTicks {
            user: 100,
            nice: 0,
            system: 0,
            idle: 100,
            iowait: 0,
        };
        assert_eq!(CpuCollector::compute_usage_pct(&t, &t), 0.0);
    }
}
