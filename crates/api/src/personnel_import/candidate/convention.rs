//! 命名约定推断模式：按目录 / 文件名聚合照片

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::scan::has_supported_image_extension;
use super::{cap_photos, ImportCandidate, ParseError};

pub(super) fn parse_from_convention(
    root: &Path,
    payload_files: &[PathBuf],
) -> Result<Vec<ImportCandidate>, ParseError> {
    let images: Vec<&PathBuf> = payload_files
        .iter()
        .filter(|path| has_supported_image_extension(path))
        .collect();

    if images.is_empty() {
        return Err(ParseError::NoUsableContent);
    }

    // 分组键：优先取「相对归档根目录的首层目录名」（支持 `姓名/01.jpg` 与 `姓名/2024/01.jpg`），
    // 根目录下的散装照片则取去后缀文件名。
    let mut groups: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in images {
        let key = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.components().next())
            .and_then(|component| match component {
                std::path::Component::Normal(value) => value.to_str().map(str::to_string),
                _ => None,
            })
            .filter(|first| {
                // 首层是目录时用目录名分组；首层是文件时用去后缀文件名分组
                root.join(first).is_dir()
            })
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default()
                    .to_string()
            });

        let key = normalize_convention_key(&key);
        groups.entry(key).or_default().push(path.clone());
    }

    let mut candidates = Vec::with_capacity(groups.len());
    for (key, mut photos) in groups {
        photos.sort();
        let (subject_id, name) = split_subject_and_name(&key);
        let (photo_paths, skipped_photos) = cap_photos(photos);
        candidates.push(ImportCandidate {
            name,
            subject_id,
            id_card: String::new(),
            remark: String::new(),
            photo_paths,
            skipped_photos,
        });
    }

    if candidates.is_empty() {
        return Err(ParseError::NoUsableContent);
    }

    Ok(candidates)
}

/// 归一化约定推断的分组键：文件名剥离「_1」「(2)」等序号后缀，但保留 `工号_姓名` 分隔
fn normalize_convention_key(key: &str) -> String {
    let trimmed = key.trim();
    // 去掉常见序号后缀：`张三_1`、`张三(1)`、`张三-1`、`张三 1`
    let mut base = trimmed.to_string();
    for suffix_separator in ['(', '-', ' '] {
        if let Some(position) = base.rfind(suffix_separator) {
            let tail = &base[position + 1..];
            let tail_trimmed = tail.trim_end_matches(')');
            if !tail_trimmed.is_empty() && tail_trimmed.chars().all(|c| c.is_ascii_digit()) {
                base.truncate(position);
            }
        }
    }
    if let Some(position) = base.rfind('_') {
        let tail = &base[position + 1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            base.truncate(position);
        }
    }
    base.trim().to_string()
}

/// 从 `工号_姓名` 形式拆分编号与姓名；无下划线时整体视为姓名
fn split_subject_and_name(key: &str) -> (Option<String>, String) {
    if let Some((subject, name)) = key.split_once('_') {
        let subject = subject.trim();
        let name = name.trim();
        if !subject.is_empty() && !name.is_empty() {
            return (Some(subject.to_string()), name.to_string());
        }
    }
    (None, key.trim().to_string())
}
