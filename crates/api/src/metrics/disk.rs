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
            use std::ffi::CString;
            let c_path = match CString::new(path) {
                Ok(p) => p,
                Err(_) => return DiskMetrics::default(),
            };
            // SAFETY: statvfs is a valid zeroable struct; we zero-initialize it before passing to the syscall.
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            // SAFETY: c_path is a valid C string; stat is a zeroed statvfs struct.
            if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
                return DiskMetrics::default();
            }
            let block_size = {
                let frsize = stat.f_frsize as u64;
                if frsize > 0 {
                    frsize
                } else {
                    stat.f_bsize as u64
                }
            };
            let total_bytes = block_size.saturating_mul(stat.f_blocks as u64);
            let available_bytes = block_size.saturating_mul(stat.f_bavail as u64);
            let used_bytes = total_bytes.saturating_sub(available_bytes);
            let inode_total = stat.f_files as u64;
            let inode_available = stat.f_favail as u64;
            let inode_used = inode_total.saturating_sub(inode_available);
            const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
            DiskMetrics {
                total_gb: total_bytes as f64 / GIB,
                used_gb: used_bytes as f64 / GIB,
                available_gb: available_bytes as f64 / GIB,
                inode_total,
                inode_used,
                inode_available,
            }
        })
        .await
        .unwrap_or_default()
    }
}
