//! 存储路径安全沙箱与规范化校验 (Path Security & Sandboxing)
//!
//! 严格杜绝路径穿越漏洞 (Directory Traversal / Path Manipulation) 以及符号链接逃逸 (Symlink Escape)。
//! 所有针对物理文件的读写、重命名和删除操作，必须强校验限制在基准证据目录 (evidence_root) 之内。

use std::path::{Component, Path, PathBuf};

use crate::error::PipelineError;

/// 安全解析并规范化证据相对路径，确保其物理目标严格位于 `evidence_root` 范围之内
///
/// # 校验规则
/// 1. 剔除前后空格与前导斜杠；
/// 2. 拒绝空路径以及含有空字符 (`\0`) 的畸形路径；
/// 3. 严格禁止路径组件中出现父目录遍历组件 `..` (`Component::ParentDir`)；
/// 4. 严格禁止绝对路径根前缀 (`Component::RootDir`, `Component::Prefix`)；
/// 5. 基准根目录若不存在则原子创建并解析为权威物理绝对路径 (`canonicalize`)；
/// 6. 若物理文件存在，解析符号链接并严格校验 `canonical_target.starts_with(&canonical_root)`；
/// 7. 若物理文件尚不存在，逐级校验其存在的最近祖先目录，确保绝不溢出根目录作用域。
pub fn resolve_and_verify_evidence_path(
    evidence_root: &Path,
    rel_path: &str,
) -> Result<PathBuf, PipelineError> {
    let clean = rel_path
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('\\');
    if clean.is_empty() {
        return Err(PipelineError::Security("证据文件相对路径为空".to_string()));
    }
    if clean.contains('\0') {
        return Err(PipelineError::Security(
            "证据路径包含非法空字符 (Null Byte)".to_string(),
        ));
    }

    let rel = Path::new(clean);
    for comp in rel.components() {
        match comp {
            Component::ParentDir => {
                return Err(PipelineError::Security(format!(
                    "检测到路径穿越尝试 (..): {clean}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(PipelineError::Security(format!(
                    "不允许使用绝对路径或系统前缀: {clean}"
                )));
            }
            Component::Normal(_) | Component::CurDir => {}
        }
    }

    // 确保 evidence_root 存在并获取规范绝对基准路径
    if !evidence_root.exists() {
        std::fs::create_dir_all(evidence_root)
            .map_err(|e| PipelineError::Snapshot(format!("创建证据根目录失败: {e}")))?;
    }

    let canonical_root = evidence_root
        .canonicalize()
        .map_err(|e| PipelineError::Snapshot(format!("规范化证据根目录失败: {e}")))?;

    let target_path = canonical_root.join(clean);

    if target_path.exists() {
        let canonical_target = target_path
            .canonicalize()
            .map_err(|e| PipelineError::Snapshot(format!("规范化目标证据路径失败: {e}")))?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err(PipelineError::Security(format!(
                "目标路径逃逸出证据根目录 (符号链接或非法路径): {}",
                canonical_target.display()
            )));
        }
        Ok(canonical_target)
    } else {
        // 目标文件暂不存在，递归检查其存在的最近父级目录
        let mut probe = target_path.parent();
        while let Some(parent) = probe {
            if parent.exists() {
                let canonical_parent = parent
                    .canonicalize()
                    .map_err(|e| PipelineError::Snapshot(format!("规范化父级目录失败: {e}")))?;
                if !canonical_parent.starts_with(&canonical_root) {
                    return Err(PipelineError::Security(format!(
                        "目标父级目录逃逸出证据根目录: {}",
                        canonical_parent.display()
                    )));
                }
                break;
            }
            probe = parent.parent();
        }
        Ok(target_path)
    }
}

/// 计算绝对路径相对于 evidence_root 的标准相对路径（用 `/` 分隔），同时强校验沙箱范围
pub fn relativize_safe_path(
    evidence_root: &Path,
    full_path: &Path,
) -> Result<String, PipelineError> {
    let canonical_root = evidence_root
        .canonicalize()
        .map_err(|e| PipelineError::Snapshot(format!("规范化证据根目录失败: {e}")))?;

    let canonical_full = full_path
        .canonicalize()
        .map_err(|e| PipelineError::Snapshot(format!("规范化物理路径失败: {e}")))?;

    if !canonical_full.starts_with(&canonical_root) {
        return Err(PipelineError::Security(format!(
            "路径不在证据根目录下: {}",
            canonical_full.display()
        )));
    }

    let rel = canonical_full
        .strip_prefix(&canonical_root)
        .map_err(|_| PipelineError::Security("计算相对路径失败".to_string()))?;

    // 格式化为标准 Unix 风格相对路径
    let mut parts = Vec::new();
    for comp in rel.components() {
        if let Component::Normal(os_str) = comp {
            parts.push(os_str.to_string_lossy().to_string());
        }
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_path_security_rejections() {
        let temp_dir = std::env::temp_dir().join(format!("test_sec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        // 1. 空路径
        assert!(resolve_and_verify_evidence_path(&temp_dir, "").is_err());
        assert!(resolve_and_verify_evidence_path(&temp_dir, "   ").is_err());

        // 2. 路径穿越 (..)
        assert!(resolve_and_verify_evidence_path(&temp_dir, "../foo.jpg").is_err());
        assert!(resolve_and_verify_evidence_path(&temp_dir, "cam1/../../etc/passwd").is_err());
        assert!(resolve_and_verify_evidence_path(&temp_dir, "cam1/../cam2/../../bin").is_err());

        // 3. 含有空字符
        assert!(resolve_and_verify_evidence_path(&temp_dir, "cam1/test\0.jpg").is_err());

        // 4. 正常合法相对路径
        let valid = resolve_and_verify_evidence_path(&temp_dir, "cam_01/snapshot_01.jpg");
        assert!(valid.is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
