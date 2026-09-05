//! 墓碑隔离区与跨阶段原子重命名管理 (Tombstone & Quarantine Manager)
//!
//! 在同一文件系统内利用 POSIX 原子目录项重命名 (`rename(2)`)，将待淘汰证据文件
//! 瞬间从业务可见路径移至隐藏的 `.tombstone/` 隔离区。
//! 即使进程随后崩溃，业务层不再存在悬挂引用，系统重启时可通过扫描墓碑区进行自愈回收。

use std::fs;
use std::path::{Path, PathBuf};

use super::path_security::resolve_and_verify_evidence_path;
use crate::error::PipelineError;

pub const TOMBSTONE_DIR_NAME: &str = ".tombstone";

/// 获取并确保墓碑隔离目录物理存在
pub fn ensure_tombstone_dir(evidence_root: &Path) -> Result<PathBuf, PipelineError> {
    let tombstone = evidence_root.join(TOMBSTONE_DIR_NAME);
    if !tombstone.exists() {
        fs::create_dir_all(&tombstone)
            .map_err(|e| PipelineError::Snapshot(format!("创建墓碑隔离目录失败: {e}")))?;
    }
    Ok(tombstone)
}

/// 将指定的证据相对路径文件原子移动至墓碑隔离区
///
/// # 返回值
/// - `Ok(Some(PathBuf))`: 文件成功移入墓碑区，返回位于墓碑区的物理路径；
/// - `Ok(None)`: 物理文件不存在（丢失凭据），已记录审计事实；
/// - `Err(PipelineError)`: 路径安全校验失败或文件系统 IO 错误。
pub fn quarantine_file(
    evidence_root: &Path,
    rel_path: &str,
) -> Result<Option<PathBuf>, PipelineError> {
    let safe_src = resolve_and_verify_evidence_path(evidence_root, rel_path)?;

    if !safe_src.is_file() {
        // 物理文件不存在，可能已被删除或外部篡改
        return Ok(None);
    }

    let tombstone_dir = ensure_tombstone_dir(evidence_root)?;

    // 提取原文件名以保留调试线索，附加纳秒时标与 UUID 确保隔离区内绝对无冲突
    let original_name = safe_src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("evidence.jpg");

    let now_ms = chrono::Utc::now().timestamp_millis();
    let unique_id = uuid::Uuid::new_v4().simple();
    let tombstone_filename = format!("{now_ms}_{unique_id}_{original_name}");
    let dest_path = tombstone_dir.join(tombstone_filename);

    // 同一挂载点内 rename 为原子操作
    if let Err(e) = fs::rename(&safe_src, &dest_path) {
        // 若在并发竞争下源文件已消失，则视同丢失
        if e.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(PipelineError::Snapshot(format!(
            "将文件移入墓碑隔离区失败 [{} -> {}]: {e}",
            safe_src.display(),
            dest_path.display()
        )));
    }

    Ok(Some(dest_path))
}

/// 扫描墓碑隔离区中的全部物理文件
pub fn list_tombstone_files(evidence_root: &Path) -> Result<Vec<PathBuf>, PipelineError> {
    let tombstone_dir = evidence_root.join(TOMBSTONE_DIR_NAME);
    if !tombstone_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let entries = fs::read_dir(&tombstone_dir)
        .map_err(|e| PipelineError::Snapshot(format!("读取墓碑隔离区目录失败: {e}")))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

/// 同步清空墓碑隔离区中的全部遗留文件（通常在启动自愈时调用）
pub fn sweep_tombstones_sync(evidence_root: &Path) -> Result<u64, PipelineError> {
    let files = list_tombstone_files(evidence_root)?;
    let mut count = 0;
    for file in files {
        if let Err(e) = fs::remove_file(&file) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %file.display(), error = %e, "清除遗留墓碑文件失败");
                continue;
            }
        }
        count += 1;
    }
    Ok(count)
}
