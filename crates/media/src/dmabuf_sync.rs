//! Linux DMA-BUF 缓存一致性与硬件同步栅障模块
//!
//! 深度实现 DMA-BUF 三权分立模型：
//! 1. 缓存一致性 (Cache Coherency)：带 `EINTR`/`EAGAIN` 循环重试与 RAII 保证的 `DMA_BUF_IOCTL_SYNC`；
//! 2. 操作定序与硬件栅障 (Operation Ordering)：基于 `poll(POLLIN)` 验证 Producer 完成写栅障；
//! 3. 显式 Fence 同步 (Explicit Sync)：导出与导入 `sync_file` 文件描述符。

use std::os::raw::c_int;

#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use crate::error::MediaError;

// ============================================================================
// DMA-BUF UAPI 常量与结构体定义（严格对齐 <linux/dma-buf.h>）
// ============================================================================

pub const DMA_BUF_SYNC_READ: u64 = 1;
pub const DMA_BUF_SYNC_WRITE: u64 = 2;
pub const DMA_BUF_SYNC_RW: u64 = DMA_BUF_SYNC_READ | DMA_BUF_SYNC_WRITE;
pub const DMA_BUF_SYNC_START: u64 = 0 << 2;
pub const DMA_BUF_SYNC_END: u64 = 1 << 2;

/// _IOW('b', 0, struct dma_buf_sync)
pub const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x40086200;

/// Linux 5.20+ / 6.0+ DMA_BUF_IOCTL_EXPORT_SYNC_FILE
/// _IOWR('b', 2, struct dma_buf_export_sync_file)
pub const DMA_BUF_IOCTL_EXPORT_SYNC_FILE: libc::c_ulong = 0xc0086202;

/// Linux 5.20+ / 6.0+ DMA_BUF_IOCTL_IMPORT_SYNC_FILE
/// _IOW('b', 3, struct dma_buf_import_sync_file)
pub const DMA_BUF_IOCTL_IMPORT_SYNC_FILE: libc::c_ulong = 0x40086203;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaBufSync {
    pub flags: u64,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaBufExportSyncFile {
    pub flags: u32,
    pub fd: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DmaBufImportSyncFile {
    pub flags: u32,
    pub fd: i32,
}

/// DMA-BUF CPU 访问方向
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DmaBufSyncDirection {
    Read,
    Write,
    ReadWrite,
}

impl DmaBufSyncDirection {
    #[inline]
    pub fn to_flags(self) -> u64 {
        match self {
            Self::Read => DMA_BUF_SYNC_READ,
            Self::Write => DMA_BUF_SYNC_WRITE,
            Self::ReadWrite => DMA_BUF_SYNC_RW,
        }
    }
}

// ============================================================================
// Linux 平台原生系统调用实现
// ============================================================================

#[cfg(target_os = "linux")]
/// 执行 DMA-BUF CPU 缓存一致性同步
///
/// 关键防护：
/// 1. 循环重试 `EINTR`（POSIX 信号打断）与 `EAGAIN`（临时资源锁竞争）；
/// 2. 针对非标准驱动或虚拟设备上的 `ENOTTY` / `EINVAL` 容错放行并记录 trace 日志，杜绝 Panic；
/// 3. 对其他不可恢复的 OS 错误返回 `MediaError::Decode`。
pub fn dmabuf_sync_cpu(fd: RawFd, start: bool, dir: DmaBufSyncDirection) -> Result<(), MediaError> {
    if fd < 0 {
        return Err(MediaError::Decode {
            reason: format!("非法 DMA-BUF fd: {fd}"),
        });
    }

    let stage = if start {
        DMA_BUF_SYNC_START
    } else {
        DMA_BUF_SYNC_END
    };
    let mut sync = DmaBufSync {
        flags: dir.to_flags() | stage,
    };

    loop {
        // SAFETY: fd 为有效描述符，sync 位于栈上生命周期与内存布局满足 ioctl 要求
        let ret = unsafe { libc::ioctl(fd, DMA_BUF_IOCTL_SYNC, &mut sync) };
        if ret == 0 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        if let Some(errno) = err.raw_os_error() {
            if errno == libc::EINTR || errno == libc::EAGAIN {
                // 收到信号中断或内核锁竞争，严格重试
                continue;
            }
            if errno == libc::ENOTTY || errno == libc::EINVAL {
                tracing::trace!(
                    fd,
                    errno,
                    start,
                    "底层驱动不支持 DMA_BUF_IOCTL_SYNC，降级放行"
                );
                return Ok(());
            }
        }

        return Err(MediaError::Decode {
            reason: format!("DMA_BUF_IOCTL_SYNC (start={start}) 失败: {err}"),
        });
    }
}

#[cfg(target_os = "linux")]
/// 等待硬件 Producer 完成写栅障（Operation Ordering 证明）
///
/// 在标准 Linux DMA-BUF 驱动规范中，`poll(POLLIN)` 会阻塞等待挂载在 `dma_resv` 上的写完成 fence。
/// 此调用确保硬件（如 VPU 或 RGA）写入完全落盘至 DDR 后，CPU 或下游消费者才开始读取，杜绝读撕裂。
pub fn wait_dmabuf_readable(fd: RawFd, timeout_ms: i32) -> Result<(), MediaError> {
    if fd < 0 {
        return Err(MediaError::Decode {
            reason: format!("非法 DMA-BUF fd: {fd}"),
        });
    }

    let mut poll_fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    loop {
        // SAFETY: poll_fd 位于栈上，长度为 1
        let ret = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if ret > 0 {
            if (poll_fd.revents & (libc::POLLERR | libc::POLLNVAL)) != 0 {
                return Err(MediaError::Decode {
                    reason: format!(
                        "DMA-BUF poll 硬件栅障异常 (revents=0x{:x})",
                        poll_fd.revents
                    ),
                });
            }
            // 写入完成，已可安全读取
            return Ok(());
        } else if ret == 0 {
            return Err(MediaError::Decode {
                reason: format!("等待 DMA-BUF 硬件 Producer 写入完成超时 ({timeout_ms}ms)"),
            });
        } else {
            let err = std::io::Error::last_os_error();
            if let Some(libc::EINTR) = err.raw_os_error() {
                continue;
            }
            if let Some(errno) = err.raw_os_error() {
                if errno == libc::EINVAL || errno == libc::ENOTTY {
                    tracing::trace!(fd, errno, "驱动不支持 poll 栅障轮询，降级放行");
                    return Ok(());
                }
            }
            return Err(MediaError::Decode {
                reason: format!("poll DMA-BUF 硬件栅障失败: {err}"),
            });
        }
    }
}

#[cfg(target_os = "linux")]
/// 从 DMA-BUF 导出显式 sync_file 文件描述符
pub fn export_dmabuf_sync_file(
    dmabuf_fd: RawFd,
    dir: DmaBufSyncDirection,
) -> Result<OwnedFd, MediaError> {
    let mut exp = DmaBufExportSyncFile {
        flags: dir.to_flags() as u32,
        fd: -1,
    };

    loop {
        // SAFETY: exp 结构体有效
        let ret = unsafe { libc::ioctl(dmabuf_fd, DMA_BUF_IOCTL_EXPORT_SYNC_FILE, &mut exp) };
        if ret == 0 {
            if exp.fd >= 0 {
                // SAFETY: exp.fd 为内核导出的有效文件描述符，转由 OwnedFd 管理所有权
                return Ok(unsafe { OwnedFd::from_raw_fd(exp.fd) });
            } else {
                return Err(MediaError::Decode {
                    reason: "内核导出了非法的 sync_file fd".to_string(),
                });
            }
        }

        let err = std::io::Error::last_os_error();
        if let Some(libc::EINTR) = err.raw_os_error() {
            continue;
        }
        return Err(MediaError::Decode {
            reason: format!("DMA_BUF_IOCTL_EXPORT_SYNC_FILE 失败: {err}"),
        });
    }
}

#[cfg(target_os = "linux")]
/// 将显式 sync_file 导入 DMA-BUF 的 reservation object
pub fn import_dmabuf_sync_file(
    dmabuf_fd: RawFd,
    dir: DmaBufSyncDirection,
    sync_file_fd: RawFd,
) -> Result<(), MediaError> {
    let mut imp = DmaBufImportSyncFile {
        flags: dir.to_flags() as u32,
        fd: sync_file_fd,
    };

    loop {
        // SAFETY: imp 结构体有效
        let ret = unsafe { libc::ioctl(dmabuf_fd, DMA_BUF_IOCTL_IMPORT_SYNC_FILE, &mut imp) };
        if ret == 0 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        if let Some(libc::EINTR) = err.raw_os_error() {
            continue;
        }
        return Err(MediaError::Decode {
            reason: format!("DMA_BUF_IOCTL_IMPORT_SYNC_FILE 失败: {err}"),
        });
    }
}

// ============================================================================
// RAII CPU Cache 访问守卫
// ============================================================================

/// DMA-BUF CPU 访问缓存守卫
///
/// 创建时执行 `DMA_BUF_SYNC_START`，析构时强制触发 `DMA_BUF_SYNC_END`。
/// 杜绝在色彩转换或处理提前返回/Panic 时导致内核 cache 状态不一致。
#[derive(Debug)]
pub struct DmaBufSyncGuard {
    #[cfg(target_os = "linux")]
    fd: RawFd,
    #[cfg(target_os = "linux")]
    dir: DmaBufSyncDirection,
    active: bool,
}

impl DmaBufSyncGuard {
    #[cfg(target_os = "linux")]
    pub fn acquire(fd: RawFd, dir: DmaBufSyncDirection) -> Result<Self, MediaError> {
        dmabuf_sync_cpu(fd, true, dir)?;
        Ok(Self {
            fd,
            dir,
            active: true,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn acquire(_fd: c_int, _dir: DmaBufSyncDirection) -> Result<Self, MediaError> {
        Ok(Self { active: true })
    }
}

impl Drop for DmaBufSyncGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if self.active {
            let _ = dmabuf_sync_cpu(self.fd, false, self.dir);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = self.active;
    }
}

// ============================================================================
// 单元测试（全平台可运行的契约验证）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_flags_values() {
        assert_eq!(DMA_BUF_SYNC_READ, 1);
        assert_eq!(DMA_BUF_SYNC_WRITE, 2);
        assert_eq!(DMA_BUF_SYNC_RW, 3);
        assert_eq!(DMA_BUF_SYNC_START, 0);
        assert_eq!(DMA_BUF_SYNC_END, 4);

        assert_eq!(DmaBufSyncDirection::Read.to_flags(), 1);
        assert_eq!(DmaBufSyncDirection::Write.to_flags(), 2);
        assert_eq!(DmaBufSyncDirection::ReadWrite.to_flags(), 3);
    }

    #[test]
    fn test_dma_buf_sync_struct_layout() {
        use std::mem::{align_of, size_of};
        assert_eq!(size_of::<DmaBufSync>(), 8);
        assert_eq!(align_of::<DmaBufSync>(), 8);

        assert_eq!(size_of::<DmaBufExportSyncFile>(), 8);
        assert_eq!(size_of::<DmaBufImportSyncFile>(), 8);
    }

    #[test]
    fn test_raii_guard_lifecycle() {
        let guard = DmaBufSyncGuard::acquire(999, DmaBufSyncDirection::Read);
        assert!(guard.is_ok());
        drop(guard);
    }
}
