//! 磁盘指标采集器 — 使用 `statvfs` 系统调用

use types::DiskMetrics;

#[derive(Debug)]
pub struct DiskCollector;

impl DiskCollector {
    pub async fn collect() -> DiskMetrics {
        Self::collect_path("/").await
    }

    pub async fn collect_path(path: &str) -> DiskMetrics {
        #[cfg(unix)]
        {
            Self::collect_unix(path).await
        }
        #[cfg(not(unix))]
        {
            DiskMetrics::default()
        }
    }

    #[cfg(unix)]
    async fn collect_unix(path: &str) -> DiskMetrics {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let p = std::path::Path::new(&path);
            let stat = match pipeline::storage_cleaner::stat_fs(p) {
                Ok(s) => s,
                Err(_) => return DiskMetrics::default(),
            };
            let mount_info = pipeline::storage_cleaner::detect_mount_info(p);

            const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
            let used_bytes = stat.total_bytes.saturating_sub(stat.available_bytes);
            let inode_used = stat.total_inodes.saturating_sub(stat.available_inodes);

            DiskMetrics {
                total_gb: stat.total_bytes as f64 / GIB,
                used_gb: used_bytes as f64 / GIB,
                available_gb: stat.available_bytes as f64 / GIB,
                inode_total: stat.total_inodes,
                inode_used,
                inode_available: stat.available_inodes,
                mount_info: Some(mount_info),
            }
        })
        .await
        .unwrap_or_default()
    }
}
