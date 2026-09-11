//! 网络指标采集器 — Linux `/sys/class/net/` / macOS `netstat -ib`

use types::NetworkInterfaceMetrics;

#[derive(Debug)]
pub struct NetworkCollector;

impl NetworkCollector {
    pub async fn collect_all() -> Vec<NetworkInterfaceMetrics> {
        #[cfg(target_os = "linux")]
        {
            Self::collect_all_linux().await
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::collect_all_fallback().await
        }
    }

    #[cfg(target_os = "linux")]
    async fn collect_all_linux() -> Vec<NetworkInterfaceMetrics> {
        let mut interfaces = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/net").await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                // 仅统计物理网卡流量，排除虚拟与回环设备
                if crate::network_service::detector::is_virtual_interface(&name) {
                    continue;
                }
                if let Some(m) = Self::collect_interface_linux(&name).await {
                    interfaces.push(m);
                }
            }
        }
        interfaces
    }

    #[cfg(target_os = "linux")]
    async fn collect_interface_linux(name: &str) -> Option<NetworkInterfaceMetrics> {
        let stats = format!("/sys/class/net/{}/statistics", name);
        let base = format!("/sys/class/net/{}", name);
        Some(NetworkInterfaceMetrics {
            name: name.to_string(),
            rx_bytes: Self::read_stat(&stats, "rx_bytes").await,
            tx_bytes: Self::read_stat(&stats, "tx_bytes").await,
            rx_packets: Self::read_stat(&stats, "rx_packets").await,
            tx_packets: Self::read_stat(&stats, "tx_packets").await,
            rx_errors: Self::read_stat(&stats, "rx_errors").await,
            tx_errors: Self::read_stat(&stats, "tx_errors").await,
            rx_dropped: Self::read_stat(&stats, "rx_dropped").await,
            tx_dropped: Self::read_stat(&stats, "tx_dropped").await,
            speed_mbps: tokio::fs::read_to_string(format!("{}/speed", base))
                .await
                .ok()
                .and_then(|s| s.trim().parse().ok()),
            link_up: tokio::fs::read_to_string(format!("{}/operstate", base))
                .await
                .ok()
                .map(|s| s.trim() == "up")
                .unwrap_or(false),
        })
    }

    #[cfg(target_os = "linux")]
    async fn read_stat(path: &str, name: &str) -> u64 {
        tokio::fs::read_to_string(format!("{}/{}", path, name))
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    #[cfg(not(target_os = "linux"))]
    async fn collect_all_fallback() -> Vec<NetworkInterfaceMetrics> {
        use tokio::process::Command;
        let mut interfaces = Vec::new();
        let Ok(output) = Command::new("netstat").arg("-ib").output().await else {
            return interfaces;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let header = lines.next().unwrap_or("");
        let fields: Vec<&str> = header.split_whitespace().collect();
        let ni = fields.iter().position(|&f| f == "Name");
        let bi = fields.iter().position(|&f| f == "Ibytes");
        let oi = fields.iter().position(|&f| f == "Obytes");
        let ipi = fields.iter().position(|&f| f == "Ipkts");
        let opi = fields.iter().position(|&f| f == "Opkts");
        let iei = fields.iter().position(|&f| f == "Ierrs");
        let oei = fields.iter().position(|&f| f == "Oerrs");
        if let (Some(ni), Some(bi), Some(oi)) = (ni, bi, oi) {
            for line in lines {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() <= ni {
                    continue;
                }
                let name = f[ni].to_string();
                const SKIP_PREFIXES: &[&str] = &["utun", "awdl", "anpi", "bridge", "ap", "llw"];
                if name == "lo0" || SKIP_PREFIXES.iter().any(|p| name.starts_with(p)) {
                    continue;
                }
                let rx: u64 = f.get(bi).and_then(|s| s.parse().ok()).unwrap_or(0);
                let tx: u64 = f.get(oi).and_then(|s| s.parse().ok()).unwrap_or(0);
                if rx == 0 && tx == 0 {
                    continue;
                }
                if interfaces.iter().any(|e| e.name == name) {
                    continue;
                }
                interfaces.push(NetworkInterfaceMetrics {
                    name,
                    rx_bytes: rx,
                    tx_bytes: tx,
                    rx_packets: ipi
                        .and_then(|i| f.get(i))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    tx_packets: opi
                        .and_then(|i| f.get(i))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    rx_errors: iei
                        .and_then(|i| f.get(i))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    tx_errors: oei
                        .and_then(|i| f.get(i))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    rx_dropped: 0,
                    tx_dropped: 0,
                    speed_mbps: None,
                    link_up: true,
                });
            }
        }
        interfaces
    }
}
