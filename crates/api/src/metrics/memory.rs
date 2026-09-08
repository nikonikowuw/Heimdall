//! 内存指标采集器 — Linux `/proc/meminfo` / macOS `sysctl` + `vm_stat`

use types::MemoryMetrics;

#[derive(Debug)]
pub struct MemoryCollector;

impl MemoryCollector {
    pub async fn collect() -> MemoryMetrics {
        #[cfg(target_os = "linux")]
        {
            Self::collect_linux().await
        }
        #[cfg(target_os = "macos")]
        {
            Self::collect_macos().await
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            MemoryMetrics::default()
        }
    }

    #[cfg(target_os = "linux")]
    async fn collect_linux() -> MemoryMetrics {
        let content = tokio::fs::read_to_string("/proc/meminfo")
            .await
            .unwrap_or_default();
        let mut total_kb = 0u64;
        let mut available_kb = 0u64;
        let mut cached_kb = 0u64;
        let mut buffer_kb = 0u64;
        let mut swap_total_kb = 0u64;
        let mut swap_free_kb = 0u64;
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total_kb = Self::parse_kb(v);
            } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                available_kb = Self::parse_kb(v);
            } else if let Some(v) = line.strip_prefix("Cached:") {
                cached_kb = Self::parse_kb(v);
            } else if let Some(v) = line.strip_prefix("Buffers:") {
                buffer_kb = Self::parse_kb(v);
            } else if let Some(v) = line.strip_prefix("SwapTotal:") {
                swap_total_kb = Self::parse_kb(v);
            } else if let Some(v) = line.strip_prefix("SwapFree:") {
                swap_free_kb = Self::parse_kb(v);
            }
        }
        MemoryMetrics {
            total_mb: total_kb / 1024,
            used_mb: (total_kb.saturating_sub(available_kb)) / 1024,
            available_mb: available_kb / 1024,
            cached_mb: cached_kb / 1024,
            buffer_mb: buffer_kb / 1024,
            swap_total_mb: swap_total_kb / 1024,
            swap_used_mb: (swap_total_kb.saturating_sub(swap_free_kb)) / 1024,
        }
    }

    #[cfg(target_os = "macos")]
    async fn collect_macos() -> MemoryMetrics {
        let total_bytes = Self::sysctl_u64("hw.memsize");
        let page_size = {
            let ps = Self::sysctl_u64("hw.pagesize");
            if ps > 0 {
                ps
            } else {
                16384
            }
        };
        let vm_stat = Self::run_cmd("vm_stat", &[]).await;
        let mut pages_free = 0u64;
        let mut pages_inactive = 0u64;
        let mut pages_speculative = 0u64;
        let mut pages_wired = 0u64;
        for line in vm_stat.lines() {
            let value = line
                .split_whitespace()
                .last()
                .and_then(|v| v.trim_end_matches('.').parse::<u64>().ok())
                .unwrap_or(0);
            if line.contains("Pages free") {
                pages_free = value;
            } else if line.contains("Pages inactive") {
                pages_inactive = value;
            } else if line.contains("Pages speculative") {
                pages_speculative = value;
            } else if line.contains("Pages wired") {
                pages_wired = value;
            }
        }
        let available_bytes = (pages_free + pages_inactive + pages_speculative) * page_size;
        let used_bytes = total_bytes.saturating_sub(available_bytes);
        let cached_bytes = (pages_inactive + pages_speculative) * page_size;
        let wired_bytes = pages_wired * page_size;
        let swap_text = Self::run_cmd("sysctl", &["vm.swapusage"]).await;
        let (mut swap_total_mb, mut swap_used_mb) = (0u64, 0u64);
        for part in swap_text.split_whitespace() {
            if part == "total" {
                continue;
            }
            if let Some(val) = part.strip_prefix('=') {
                let val = val.trim_end_matches('M').trim_end_matches('G');
                if let Ok(v) = val.parse::<f64>() {
                    if swap_total_mb == 0 {
                        swap_total_mb = v as u64;
                    } else if swap_used_mb == 0 {
                        swap_used_mb = v as u64;
                    }
                }
            }
        }
        MemoryMetrics {
            total_mb: total_bytes / (1024 * 1024),
            used_mb: used_bytes / (1024 * 1024),
            available_mb: available_bytes / (1024 * 1024),
            cached_mb: cached_bytes / (1024 * 1024),
            buffer_mb: wired_bytes / (1024 * 1024),
            swap_total_mb,
            swap_used_mb,
        }
    }

    #[cfg(target_os = "linux")]
    fn parse_kb(s: &str) -> u64 {
        s.split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    }

    #[cfg(target_os = "macos")]
    fn sysctl_u64(name: &str) -> u64 {
        use std::ffi::CString;
        let c_name = match CString::new(name) {
            Ok(n) => n,
            Err(_) => return 0,
        };
        let mut value: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: sysctlbyname is a standard BSD syscall; pointers and length are valid.
        let ret = unsafe {
            libc::sysctlbyname(
                c_name.as_ptr(),
                &mut value as *mut u64 as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if ret == 0 {
            value
        } else {
            0
        }
    }

    #[cfg(target_os = "macos")]
    async fn run_cmd(cmd: &str, args: &[&str]) -> String {
        tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    }
}
