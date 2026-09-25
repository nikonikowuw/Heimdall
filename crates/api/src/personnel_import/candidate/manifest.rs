//! 清单驱动模式：`manifest.csv` 字段映射与编码自适应

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use encoding_rs::GB18030;

use super::scan::{
    collect_payload_files, has_supported_image_extension, SUPPORTED_IMAGE_EXTENSIONS,
};
use super::{cap_photos, ImportCandidate, ParseError};

/// 清单表头字段（中文与英文字段名均接受，降低现场填写门槛）
const HEADER_NAME: &[&str] = &["name", "姓名", "人员姓名"];
const HEADER_SUBJECT_ID: &[&str] = &["subjectid", "subject_id", "编号", "工号", "人员编号"];
const HEADER_ID_CARD: &[&str] = &["idcard", "id_card", "证件号", "身份证", "身份证号"];
const HEADER_REMARK: &[&str] = &["remark", "备注", "部门"];
const HEADER_PHOTOS: &[&str] = &["photos", "photo", "照片", "图片", "照片文件名"];

pub(super) fn parse_from_manifest(
    manifest_path: &Path,
    payload_files: &[PathBuf],
) -> Result<Vec<ImportCandidate>, ParseError> {
    let raw = std::fs::read(manifest_path)
        .map_err(|err| ParseError::ManifestInvalid(format!("读取清单文件失败: {err}")))?;
    let text = decode_manifest_text(&raw);

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    // 归档内所有图片按文件名索引，供清单以「文件名」或「相对路径」两种写法引用
    let image_index = build_image_index(payload_files);

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    let headers = reader
        .headers()
        .map_err(|err| ParseError::ManifestInvalid(format!("读取表头失败: {err}")))?
        .clone();

    let columns =
        ManifestColumns::resolve(&headers).ok_or_else(|| ParseError::MissingNameColumn {
            header_preview: headers.iter().collect::<Vec<_>>().join(", "),
        })?;

    let mut candidates: Vec<ImportCandidate> = Vec::new();
    let mut pending = Vec::new();
    let mut skipped = Vec::new();

    for (row_index, record) in reader.records().enumerate() {
        let record = match record {
            Ok(record) => record,
            Err(err) => {
                // 单行损坏不废弃整包：记录后继续，避免现场因一个引号错位全部重做
                skipped.push(format!("第 {} 行: {err}", row_index + 2));
                continue;
            }
        };

        let name = columns.get(&record, columns.name).trim().to_string();
        if name.is_empty() {
            skipped.push(format!("第 {} 行: 姓名为空", row_index + 2));
            continue;
        }

        let subject_id = columns
            .subject_id
            .and_then(|idx| record.get(idx))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let id_card = columns
            .id_card
            .and_then(|idx| record.get(idx))
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        let remark = columns
            .remark
            .and_then(|idx| record.get(idx))
            .map(str::trim)
            .unwrap_or_default()
            .to_string();

        let photo_paths =
            resolve_manifest_photos(&record, columns.photos, manifest_dir, &image_index, &name);

        let (photo_paths, skipped_photos) = cap_photos(photo_paths);
        pending.push(ImportCandidate {
            name,
            subject_id,
            id_card,
            remark,
            photo_paths,
            skipped_photos,
        });
    }

    if !skipped.is_empty() {
        tracing::warn!(
            skipped_rows = skipped.len(),
            detail = %skipped.join(" | "),
            "人员清单存在被跳过的无效行"
        );
    }

    if pending.is_empty() {
        return Err(ParseError::ManifestInvalid(
            "清单中没有任何有效的人员记录（请检查 name 列是否已填写）".to_string(),
        ));
    }

    candidates.extend(pending);
    Ok(candidates)
}

/// 清单列索引解析结果
struct ManifestColumns {
    name: usize,
    subject_id: Option<usize>,
    id_card: Option<usize>,
    remark: Option<usize>,
    photos: Option<usize>,
}

impl ManifestColumns {
    fn resolve(headers: &csv::StringRecord) -> Option<Self> {
        let normalized: Vec<String> = headers.iter().map(normalize_header).collect();

        let find = |aliases: &[&str]| -> Option<usize> {
            normalized.iter().position(|header| {
                aliases
                    .iter()
                    .any(|alias| header == &normalize_header(alias))
            })
        };

        let name = find(HEADER_NAME)?;
        Some(Self {
            name,
            subject_id: find(HEADER_SUBJECT_ID),
            id_card: find(HEADER_ID_CARD),
            remark: find(HEADER_REMARK),
            photos: find(HEADER_PHOTOS),
        })
    }

    fn get(&self, record: &csv::StringRecord, index: usize) -> String {
        record.get(index).unwrap_or_default().to_string()
    }
}

fn normalize_header(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '_' | '-'))
        .collect()
}

/// 构建「文件名 → 路径」索引。该索引只包含已扫描到的归档文件，
/// 供相对路径直达失败时按 basename 兼容清单只写文件名的情况。
fn build_image_index(payload_files: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut index = BTreeMap::new();
    for path in payload_files {
        if !has_supported_image_extension(path) {
            continue;
        }
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
            index
                .entry(file_name.to_ascii_lowercase())
                .or_insert_with(|| path.clone());
        }
    }
    index
}

/// 解析清单行引用的照片路径
///
/// 依次尝试：清单内安全的相对路径 → 归档内文件名索引 → 姓名目录 / 同名图片。
fn resolve_manifest_photos(
    record: &csv::StringRecord,
    photos_column: Option<usize>,
    manifest_dir: &Path,
    image_index: &BTreeMap<String, PathBuf>,
    name: &str,
) -> Vec<PathBuf> {
    let mut resolved = Vec::new();
    let canonical_manifest_dir = manifest_dir.canonicalize().ok();

    let raw_list = photos_column
        .and_then(|idx| record.get(idx))
        .map(str::trim)
        .unwrap_or_default();

    if !raw_list.is_empty() {
        for token in raw_list.split([';', ',', '、', '|']) {
            let token = token.trim().trim_start_matches("./").replace('\\', "/");
            if token.is_empty() {
                continue;
            }

            let relative_path = Path::new(&token);
            if relative_path.is_absolute()
                || relative_path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                continue;
            }

            if let Some(root) = canonical_manifest_dir.as_deref() {
                let direct = manifest_dir.join(relative_path);
                if let Some(path) = canonical_file_under(root, &direct)
                    .filter(|path| has_supported_image_extension(path))
                {
                    resolved.push(path);
                    continue;
                }
            }

            // 仅回退到归档扫描索引，不使用 token 的路径部分访问宿主文件系统。
            let base = token.rsplit('/').next().unwrap_or(&token);
            if let Some(found) = image_index.get(&base.to_ascii_lowercase()) {
                resolved.push(found.clone());
            }
        }
    }

    if resolved.is_empty() && is_safe_single_path_component(name) {
        let by_directory = manifest_dir.join(name);
        if let Some(root) = canonical_manifest_dir.as_deref() {
            if let Some(directory) =
                canonical_path_under(root, &by_directory).filter(|path| path.is_dir())
            {
                let mut inside: Vec<PathBuf> = collect_payload_files(&directory)
                    .into_iter()
                    .filter(|path| has_supported_image_extension(path))
                    .collect();
                inside.sort();
                resolved.extend(inside);
            }
        }
    }

    if resolved.is_empty() && is_safe_single_path_component(name) {
        if let Some(root) = canonical_manifest_dir.as_deref() {
            for extension in SUPPORTED_IMAGE_EXTENSIONS {
                let candidate = manifest_dir.join(format!("{name}.{extension}"));
                if let Some(path) = canonical_file_under(root, &candidate) {
                    resolved.push(path);
                }
            }
        }
    }

    resolved.sort();
    resolved.dedup();
    resolved
}

fn canonical_file_under(root: &Path, candidate: &Path) -> Option<PathBuf> {
    canonical_path_under(root, candidate).filter(|path| path.is_file())
}

fn canonical_path_under(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let canonical = candidate.canonicalize().ok()?;
    canonical.starts_with(root).then_some(canonical)
}

fn is_safe_single_path_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0'))
        && matches!(
            Path::new(value).components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        )
}

pub fn decode_manifest_text(raw: &[u8]) -> String {
    let trimmed = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    match std::str::from_utf8(trimmed) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let (decoded, _, had_errors) = GB18030.decode(trimmed);
            if had_errors {
                tracing::warn!("人员清单既非合法 UTF-8 也非合法 GB18030，按替换字符解析");
            }
            decoded.into_owned()
        }
    }
}
