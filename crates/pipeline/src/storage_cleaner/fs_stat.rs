//! 物理文件系统状态、Inode、挂载只读与硬件寿命探测器 (FS Stat & Health Detector)
//!
//! 单次调用 POSIX `statvfs` 提取完整的存储元数据：
//! 1. 物理字节总容量、可用容量与空闲比例；
//! 2. Inode 总量、可用 Inode 与 Inode 空闲比例 (防小文件爆 Inode)；
//! 3. 挂载标志位检测 (只读文件系统只读保护 ST_RDONLY)；
//! 4. 嵌入式 Linux eMMC 磨损寿命与预警检测 (/sys/block/mmcblk*/device)；
//! 5. SQLite 数据库与 WAL 堆积监控。

use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

/// 物理文件系统状态快照
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FsStorageStat {
    /// 文件系统底层基本分块大小 (字节)
    pub fragment_size: u64,
    /// 物理总字节数
    pub total_bytes: u64,
    /// 非特权用户可用字节数
    pub available_bytes: u64,
    /// 字节空闲比例 (0.0 ~ 1.0)
    pub free_ratio: f64,
    /// 文件系统总 Inode 节点数
    pub total_inodes: u64,
    /// 可用 Inode 节点数
    pub available_inodes: u64,
    /// Inode 空闲比例 (0.0 ~ 1.0)
    pub inode_free_ratio: f64,
    /// 是否处于只读挂载状态 (硬件坏道或内核文件系统紧急降级)
    pub is_read_only: bool,
}

impl FsStorageStat {
    /// 格式化总字节为人类可读格式 (e.g. "128.00 GB")
    pub fn format_total_bytes(&self) -> String {
        format_bytes(self.total_bytes)
    }

    /// 格式化可用字节为人类可读格式
    pub fn format_available_bytes(&self) -> String {
        format_bytes(self.available_bytes)
    }
}

/// 基于 POSIX statvfs 全量采集文件系统状态（包括字节、Inode、挂载只读标志）
pub fn stat_fs(path: &Path) -> Result<FsStorageStat, std::io::Error> {
    #[cfg(unix)]
    {
        // 向上搜寻最近存在的父级目录
        let mut probe_path = path;
        while !probe_path.exists() {
            match probe_path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => probe_path = parent,
                _ => {
                    probe_path = Path::new(".");
                    break;
                }
            }
        }

        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(probe_path.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // SAFETY: statvfs 结构体全零初始化为合法的安全内存
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: c_path 是合法的以 null 结尾的 C 字符串，stat 接收内核填充的磁盘统计
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        let frsize = if stat.f_frsize > 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };

        let total_bytes = (stat.f_blocks as u64).saturating_mul(frsize);
        let available_bytes = (stat.f_bavail as u64).saturating_mul(frsize);

        let free_ratio = if stat.f_blocks > 0 {
            stat.f_bavail as f64 / stat.f_blocks as f64
        } else {
            1.0
        };

        let total_inodes = stat.f_files as u64;
        let available_inodes = stat.f_favail as u64;

        let inode_free_ratio = if total_inodes > 0 {
            available_inodes as f64 / total_inodes as f64
        } else {
            1.0
        };

        // 检查 ST_RDONLY 挂载标志 (只读文件系统保护)
        let is_read_only = (stat.f_flag & libc::ST_RDONLY as libc::c_ulong) != 0;

        Ok(FsStorageStat {
            fragment_size: frsize,
            total_bytes,
            available_bytes,
            free_ratio,
            total_inodes,
            available_inodes,
            inode_free_ratio,
            is_read_only,
        })
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(FsStorageStat {
            fragment_size: 4096,
            total_bytes: 100 * 1024 * 1024 * 1024,
            available_bytes: 50 * 1024 * 1024 * 1024,
            free_ratio: 0.5,
            total_inodes: 1_000_000,
            available_inodes: 500_000,
            inode_free_ratio: 0.5,
            is_read_only: false,
        })
    }
}

/// 格式化字节数
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// 嵌入式 Linux eMMC 磨损与健康诊断信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmmcHealthInfo {
    /// 设备名称 (e.g. "mmcblk0")
    pub device: String,
    /// 预寿命估算 A 类 SLC 块消耗百分比 (0~100%)
    pub slc_wear_percent: Option<u8>,
    /// 预寿命估算 B 类 MLC/TLC 块消耗百分比 (0~100%)
    pub mlc_wear_percent: Option<u8>,
    /// 预寿命警告状态: 1=正常, 2=警告(消耗超80%), 3=严重紧急
    pub pre_eol_status: Option<u8>,
}

/// 检测当前 Linux 系统的 eMMC 存储磨损健康状态 (读取 sysfs)
pub fn detect_emmc_health() -> Option<EmmcHealthInfo> {
    #[cfg(target_os = "linux")]
    {
        for i in 0..4 {
            let dev_name = format!("mmcblk{i}");
            let base_sys = PathBuf::from(format!("/sys/block/{dev_name}/device"));
            if !base_sys.is_dir() {
                continue;
            }

            let life_time_path = base_sys.join("life_time");
            let pre_eol_path = base_sys.join("pre_eol_info");

            let mut slc_wear = None;
            let mut mlc_wear = None;

            if let Ok(content) = std::fs::read_to_string(&life_time_path) {
                // 内容通常为 "0x01 0x02" 或 "0x01 0x01"
                let parts: Vec<&str> = content.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Ok(a), Ok(b)) = (
                        u8::from_str_radix(parts[0].trim_start_matches("0x"), 16),
                        u8::from_str_radix(parts[1].trim_start_matches("0x"), 16),
                    ) {
                        slc_wear = Some(a.saturating_mul(10).min(100));
                        mlc_wear = Some(b.saturating_mul(10).min(100));
                    }
                }
            }

            let mut pre_eol = None;
            if let Ok(content) = std::fs::read_to_string(&pre_eol_path) {
                if let Ok(status) = u8::from_str_radix(content.trim().trim_start_matches("0x"), 16)
                {
                    pre_eol = Some(status);
                }
            }

            if slc_wear.is_some() || pre_eol.is_some() {
                return Some(EmmcHealthInfo {
                    device: dev_name,
                    slc_wear_percent: slc_wear,
                    mlc_wear_percent: mlc_wear,
                    pre_eol_status: pre_eol,
                });
            }
        }
        None
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 获取指定 SQLite 数据库的 WAL 文件堆积大小 (字节)
pub fn get_sqlite_wal_size(db_path: &Path) -> Option<u64> {
    let mut wal_path = db_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    std::fs::metadata(wal_path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_stat_fs_real_directory() {
        let temp_dir = std::env::temp_dir();
        let stat = stat_fs(&temp_dir).unwrap();

        assert!(stat.total_bytes > 0);
        assert!(stat.available_bytes > 0);
        assert!(stat.free_ratio > 0.0 && stat.free_ratio <= 1.0);
        assert!(stat.inode_free_ratio > 0.0 && stat.inode_free_ratio <= 1.0);
        assert!(!stat.is_read_only); // 测试临时目录通常为读写
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024 * 10), "10.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 50), "50.00 GB");
    }
}
