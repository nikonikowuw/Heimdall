//! 批量导入候选人员解析
//!
//! 支持自适应双模输入：
//! 1. **清单优先**：归档根目录存在 `manifest.csv` / `personnel.csv` 时按表格字段映射；
//! 2. **约定推断**：无清单时按文件名 / 子目录聚合照片（`姓名.jpg`、`工号_姓名.jpg`、`姓名/01.jpg`）。
//!
//! 清单编码自适应：先剥 UTF-8 BOM，非合法 UTF-8 时回退 GB18030（覆盖 Windows Excel 导出的 GBK 中文），
//! 避免现场导入因编码问题整包失败。

mod convention;
mod manifest;
mod scan;

use std::path::{Path, PathBuf};

pub use manifest::decode_manifest_text;
pub use scan::resolve_content_root;

use scan::{collect_payload_files, is_manifest_file};

/// 单人可录入的人脸样本上限，与在线录入门禁保持一致
pub const MAX_PHOTOS_PER_CANDIDATE: usize = 5;

/// 导入候选人员（解析产物，尚未进行人脸特征提取）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    /// 人员姓名
    pub name: String,
    /// 人员编号；清单未给出时为空，由服务层分配 UUIDv7
    pub subject_id: Option<String>,
    /// 证件号
    pub id_card: String,
    /// 部门 / 备注
    pub remark: String,
    /// 已按文件名升序排列的照片绝对路径
    pub photo_paths: Vec<PathBuf>,
    /// 因超出单人上限而被忽略的照片数量
    pub skipped_photos: u32,
}

/// 归档解析失败原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 归档内既无清单也无可用图片
    NoUsableContent,
    /// 清单存在但表头缺少必填列
    MissingNameColumn { header_preview: String },
    /// 清单解析失败（结构损坏或字段非法）
    ManifestInvalid(String),
}

impl ParseError {
    /// 面向管理员的中文说明
    pub fn message(&self) -> String {
        match self {
            Self::NoUsableContent => {
                "导入包中未找到可用的图片文件或人员清单 (manifest.csv)".to_string()
            }
            Self::MissingNameColumn { header_preview } => format!(
                "人员清单缺少必填的 name 列，实际表头: {header_preview}；请下载标准模板后重新填写"
            ),
            Self::ManifestInvalid(reason) => format!("人员清单解析失败: {reason}"),
        }
    }
}

/// 解析归档根目录，输出导入候选序列
///
/// 先尝试清单模式，失败或缺失时回退约定推断模式。两种模式均过滤系统干扰文件
/// （`.` 开头、`__MACOSX`、`Thumbs.db`）与非图片文件。
pub fn parse_candidates(root: &Path) -> Result<Vec<ImportCandidate>, ParseError> {
    let files = collect_payload_files(root);

    if let Some(manifest_path) = files
        .iter()
        .find(|path| is_manifest_file(path))
        .map(PathBuf::as_path)
    {
        return manifest::parse_from_manifest(manifest_path, &files);
    }

    convention::parse_from_convention(root, &files)
}

/// 按文件名升序截取前 N 张，返回（保留照片，被忽略数量）
pub(crate) fn cap_photos(mut photos: Vec<PathBuf>) -> (Vec<PathBuf>, u32) {
    photos.sort();
    photos.dedup();
    if photos.len() <= MAX_PHOTOS_PER_CANDIDATE {
        return (photos, 0);
    }
    let skipped = (photos.len() - MAX_PHOTOS_PER_CANDIDATE) as u32;
    photos.truncate(MAX_PHOTOS_PER_CANDIDATE);
    (photos, skipped)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    pub use super::*;

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "heimdall_import_{tag}_{}",
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn convention_mode_groups_directory_photos_by_person() {
        let root = temp_root("convention_dir");
        write_file(&root.join("张三/01.jpg"), b"a");
        write_file(&root.join("张三/02.jpg"), b"b");
        write_file(&root.join("李四.png"), b"c");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 2);

        let zhang = candidates.iter().find(|c| c.name == "张三").unwrap();
        assert_eq!(zhang.photo_paths.len(), 2);
        assert!(zhang.subject_id.is_none());

        let li = candidates.iter().find(|c| c.name == "李四").unwrap();
        assert_eq!(li.photo_paths.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convention_mode_splits_subject_id_from_filename() {
        let root = temp_root("convention_subject");
        write_file(&root.join("EMP001_王五.jpg"), b"a");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "王五");
        assert_eq!(candidates[0].subject_id.as_deref(), Some("EMP001"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convention_mode_merges_numbered_suffix_into_single_candidate() {
        let root = temp_root("convention_numbered");
        write_file(&root.join("赵六_1.jpg"), b"a");
        write_file(&root.join("赵六_2.jpg"), b"b");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1, "序号后缀应归并为同一候选人");
        assert_eq!(candidates[0].name, "赵六");
        assert_eq!(candidates[0].photo_paths.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convention_mode_ignores_system_noise_files() {
        let root = temp_root("convention_noise");
        write_file(&root.join("张三.jpg"), b"a");
        write_file(&root.join("__MACOSX/._张三.jpg"), b"noise");
        write_file(&root.join(".DS_Store"), b"noise");
        write_file(&root.join("Thumbs.db"), b"noise");
        write_file(&root.join("readme.txt"), b"noise");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].photo_paths.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convention_mode_truncates_photos_beyond_limit() {
        let root = temp_root("convention_cap");
        for index in 1..=7 {
            write_file(&root.join(format!("张三/{index:02}.jpg")), b"a");
        }

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].photo_paths.len(), MAX_PHOTOS_PER_CANDIDATE);
        assert_eq!(candidates[0].skipped_photos, 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_maps_all_declared_fields() {
        let root = temp_root("manifest_fields");
        write_file(
            &root.join("manifest.csv"),
            "name,subjectId,idCard,remark,photos\n张三,EMP001,110101199001010011,安保部,zhang_01.jpg;zhang_02.jpg\n".as_bytes(),
        );
        write_file(&root.join("zhang_01.jpg"), b"a");
        write_file(&root.join("zhang_02.jpg"), b"b");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1);
        let first = &candidates[0];
        assert_eq!(first.name, "张三");
        assert_eq!(first.subject_id.as_deref(), Some("EMP001"));
        assert_eq!(first.id_card, "110101199001010011");
        assert_eq!(first.remark, "安保部");
        assert_eq!(first.photo_paths.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_accepts_chinese_headers() {
        let root = temp_root("manifest_cn_header");
        write_file(
            &root.join("personnel.csv"),
            "姓名,工号,证件号,备注\n李四,G002,ID-2,巡检\n".as_bytes(),
        );
        write_file(&root.join("李四.jpg"), b"a");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates[0].name, "李四");
        assert_eq!(candidates[0].subject_id.as_deref(), Some("G002"));
        assert_eq!(candidates[0].id_card, "ID-2");
        assert_eq!(candidates[0].remark, "巡检");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_decodes_gbk_encoded_csv() {
        let root = temp_root("manifest_gbk");
        // "姓名,备注\n张三,保安\n" 的 GB18030 字节序列
        let gbk_bytes: Vec<u8> = vec![
            0xD0, 0xD5, 0xC3, 0xFB, 0x2C, 0xB1, 0xB8, 0xD7, 0xA2, 0x0A, 0xD5, 0xC5, 0xC8, 0xFD,
            0x2C, 0xB1, 0xA3, 0xB0, 0xB2, 0x0A,
        ];
        write_file(&root.join("manifest.csv"), &gbk_bytes);
        write_file(&root.join("张三.jpg"), b"a");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "张三");
        assert_eq!(candidates[0].remark, "保安");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_strips_utf8_bom() {
        let root = temp_root("manifest_bom");
        let mut content = vec![0xEF, 0xBB, 0xBF];
        content.extend_from_slice("name\n张三\n".as_bytes());
        write_file(&root.join("manifest.csv"), &content);
        write_file(&root.join("张三.jpg"), b"a");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates[0].name, "张三");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_rejects_missing_name_column() {
        let root = temp_root("manifest_no_name");
        write_file(
            &root.join("manifest.csv"),
            "idCard,remark\nID-1,备注\n".as_bytes(),
        );

        let err = parse_candidates(&root).unwrap_err();
        assert!(matches!(err, ParseError::MissingNameColumn { .. }));
        assert!(err.message().contains("name"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_reports_missing_photo_for_named_person() {
        let root = temp_root("manifest_missing_photo");
        write_file(
            &root.join("manifest.csv"),
            "name,photos\n张三,nowhere.jpg\n李四,lisi.jpg\n".as_bytes(),
        );
        write_file(&root.join("lisi.jpg"), b"image");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].photo_paths.is_empty());
        assert_eq!(candidates[1].name, "李四");
        assert_eq!(candidates[1].photo_paths.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_accepts_snake_case_optional_headers() {
        let root = temp_root("manifest_snake_headers");
        write_file(
            &root.join("manifest.csv"),
            "name,subject_id,id_card,remark,photos\n张三,EMP_001,ID-001,安保,zhang.jpg\n"
                .as_bytes(),
        );
        write_file(&root.join("zhang.jpg"), b"image");

        let candidates = parse_candidates(&root).unwrap();
        assert_eq!(candidates[0].subject_id.as_deref(), Some("EMP_001"));
        assert_eq!(candidates[0].id_card, "ID-001");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_mode_rejects_photo_paths_outside_manifest_directory() {
        let root = temp_root("manifest_path_escape");
        let content_root = root.join("payload");
        std::fs::create_dir_all(&content_root).unwrap();
        let outside_name = format!(
            "{}_outside.jpg",
            root.file_name().unwrap().to_string_lossy()
        );
        let outside_path = root.parent().unwrap().join(&outside_name);
        write_file(&outside_path, b"outside image");
        write_file(
            &content_root.join("manifest.csv"),
            format!(
                "name,photos\n张三,../../{outside_name};{}\n",
                outside_path.display()
            )
            .as_bytes(),
        );

        let candidates = parse_candidates(&content_root).unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].photo_paths.is_empty());
        assert!(!candidates[0].photo_paths.contains(&outside_path));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(outside_path);
    }

    #[test]
    fn content_root_descends_single_wrapper_directory() {
        let root = temp_root("content_root_wrapper");
        write_file(&root.join("人员导入包/张三.jpg"), b"a");
        write_file(&root.join("人员导入包/李四.jpg"), b"b");

        let resolved = resolve_content_root(&root);
        assert_eq!(resolved, root.join("人员导入包"));
        let candidates = parse_candidates(&resolved).unwrap();
        assert_eq!(candidates.len(), 2, "包一层目录时不应把目录名当人名");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn content_root_keeps_level_when_images_are_at_top() {
        let root = temp_root("content_root_top");
        write_file(&root.join("张三.jpg"), b"a");
        write_file(&root.join("李四.jpg"), b"b");

        let resolved = resolve_content_root(&root);
        assert_eq!(resolved, root);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn content_root_stays_when_manifest_present() {
        let root = temp_root("content_root_manifest");
        write_file(
            &root.join("manifest.csv"),
            b"name\n\xe5\xbc\xa0\xe4\xb8\x89\n",
        );
        write_file(&root.join("张三.jpg"), b"a");

        let resolved = resolve_content_root(&root);
        assert_eq!(
            resolved, root,
            "含清单时不得下钻，否则会拆散清单与照片的引用关系"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_archive_reports_no_usable_content() {
        let root = temp_root("empty");
        let err = parse_candidates(&root).unwrap_err();
        assert_eq!(err, ParseError::NoUsableContent);

        let _ = std::fs::remove_dir_all(&root);
    }
}
