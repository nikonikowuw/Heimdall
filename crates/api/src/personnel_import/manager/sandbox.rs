//! 导入归档临时沙箱：RAII 生命周期与启动自愈清理

use std::path::{Path, PathBuf};

use crate::error::ApiError;

/// 批量导入归档的临时沙箱根目录（相对进程工作目录），与证据目录分离
pub const IMPORT_SANDBOX_DIR: &str = "var/tmp/personnel_import";

/// 上传归档压缩体积上限（100 MiB）；解压后另受 512 MiB 与 20,000 条目上限约束。
pub const MAX_IMPORT_ARCHIVE_BYTES: usize = 100 * 1024 * 1024;
/// 最大解压体积，限制高压缩比归档对边缘设备磁盘的占用。
pub const MAX_IMPORT_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
/// 最大归档条目数，避免大量空文件耗尽 inode、内存和解析时间。
pub const MAX_IMPORT_ARCHIVE_ENTRIES: usize = 20_000;
/// multipart 头部、边界及单个文件字段之外的请求开销预算。
pub const MAX_IMPORT_MULTIPART_OVERHEAD_BYTES: usize = 64 * 1024;
pub const MAX_IMPORT_REQUEST_BYTES: usize =
    MAX_IMPORT_ARCHIVE_BYTES + MAX_IMPORT_MULTIPART_OVERHEAD_BYTES;

/// 归档临时沙箱 RAII 守卫
///
/// `Drop` 时递归删除 `{root}/{task_id}/`，覆盖任务正常完成、解析失败、主动取消与 panic 全部路径。
#[derive(Debug)]
pub struct TempImportSandbox {
    dir: PathBuf,
}

impl TempImportSandbox {
    /// 创建沙箱目录（含 task_id 子目录）并返回守卫
    pub fn create(root: &Path, task_id: &str) -> Result<Self, ApiError> {
        let dir = root.join(task_id);
        std::fs::create_dir_all(&dir)
            .map_err(|err| ApiError::Internal(format!("创建导入临时沙箱失败: {err}")))?;
        Ok(Self { dir })
    }

    /// 沙箱目录路径
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempImportSandbox {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(
                dir = %self.dir.display(),
                error = %err,
                "清理导入临时沙箱失败"
            ),
        }
    }
}

/// 清理启动时残留的导入沙箱（非正常掉电后的自愈）
///
/// 沙箱目录本身是任务级隔离的中间产物，进程重启后不存在可接续的任务，
/// 因此整体清空是安全的。
pub fn sweep_orphan_sandboxes(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::info!(
            removed,
            root = %root.display(),
            "清理历史遗留的人员导入临时沙箱"
        );
    }
}
