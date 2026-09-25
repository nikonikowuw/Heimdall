//! 归档载荷扫描、系统干扰项过滤与内容根定位

use std::path::{Path, PathBuf};

/// 支持的图片扩展名（小写比较）
pub(super) const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp"];

/// 归档内识别到的清单文件名候选，按优先级排列
const MANIFEST_FILENAMES: &[&str] = &["manifest.csv", "personnel.csv"];

/// 递归收集归档载荷内的可用文件（已剔除系统干扰文件）
pub(super) fn collect_payload_files(root: &Path) -> Vec<PathBuf> {
    let mut collected = Vec::new();
    collect_payload_files_inner(root, &mut collected);
    collected.sort();
    collected
}

fn collect_payload_files_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if is_payload_noise(&name_str) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_payload_files_inner(&path, out);
        } else if file_type.is_file() {
            out.push(path);
        }
    }
}

/// 判断是否为需要静默忽略的系统干扰项
pub(super) fn is_payload_noise(name: &str) -> bool {
    name.starts_with('.') || name == "__MACOSX" || name == "Thumbs.db" || name == "__MACOSX/"
}

/// 扩展名是否受支持（大小写不敏感）
pub(super) fn has_supported_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| SUPPORTED_IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

pub(super) fn is_manifest_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .is_some_and(|name| MANIFEST_FILENAMES.contains(&name.as_str()))
}

/// 定位解压后的实际内容根目录
///
/// 用户压缩时常多套一层目录（如 `人员导入包/张三.jpg`），此时需下钻一层，
/// 否则约定推断会把「人员导入包」误当成人名分组。含清单的目录直接作为根。
pub fn resolve_content_root(extract_dir: &Path) -> PathBuf {
    let mut current = extract_dir.to_path_buf();

    for _ in 0..2 {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return current;
        };

        let payload: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !is_payload_noise(name))
            })
            .collect();

        // 当前层已有清单：就是内容根，不再下钻（否则会拆散清单与照片的引用关系）
        if payload
            .iter()
            .any(|path| path.is_file() && is_manifest_file(path))
        {
            return current;
        }

        let image_dirs: Vec<PathBuf> = payload
            .iter()
            .filter(|path| {
                path.is_dir()
                    && collect_payload_files(path)
                        .iter()
                        .any(|inner| has_supported_image_extension(inner))
            })
            .cloned()
            .collect();

        let root_has_images = payload
            .iter()
            .any(|path| path.is_file() && has_supported_image_extension(path));

        // 只有「根目录无散装图片、且唯一含图目录只有一个」时才下钻，
        // 否则保留当前层，由约定推断按首层目录名分组。
        match (root_has_images, image_dirs.len()) {
            (false, 1) => {
                if let Some(next) = image_dirs.first() {
                    current = next.clone();
                } else {
                    return current;
                }
            }
            _ => return current,
        }
    }

    current
}
