//! 算法包归档解压与安全校验
//!
//! 支持 .tar.gz / .tar / .zip 格式自动检测，
//! 内置 Tar Slip / Zip Slip 路径穿透攻击防护。

use std::path::{Path, PathBuf};

use infer::ALGO_MANIFEST_FILENAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    Tar,
}

pub fn detect_archive_format(
    bytes: &[u8],
    filename: Option<&str>,
) -> Result<ArchiveFormat, String> {
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return Ok(ArchiveFormat::Zip);
    }
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        return Ok(ArchiveFormat::TarGz);
    }
    if bytes.len() >= 512 && &bytes[257..262] == b"ustar" {
        return Ok(ArchiveFormat::Tar);
    }
    if let Some(name) = filename {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            return Ok(ArchiveFormat::Zip);
        }
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            return Ok(ArchiveFormat::TarGz);
        }
        if lower.ends_with(".tar") {
            return Ok(ArchiveFormat::Tar);
        }
    }
    Err("不支持的归档格式，仅支持 .zip、.tar.gz (.tgz) 与 .tar 格式".to_string())
}

pub fn extract_archive_package_from_file(
    archive_path: &Path,
    filename: Option<&str>,
    dest_dir: &Path,
) -> Result<PathBuf, String> {
    let mut header_file = std::fs::File::open(archive_path)
        .map_err(|e| format!("打开上传归档失败 {}: {e}", archive_path.display()))?;
    let mut header = [0u8; 512];
    let header_len = std::io::Read::read(&mut header_file, &mut header)
        .map_err(|e| format!("读取上传归档头失败: {e}"))?;
    let format = detect_archive_format(&header[..header_len], filename)?;

    match format {
        ArchiveFormat::Zip => {
            let file =
                std::fs::File::open(archive_path).map_err(|e| format!("打开 ZIP 归档失败: {e}"))?;
            extract_zip(file, dest_dir)?;
        }
        ArchiveFormat::TarGz => {
            let file = std::fs::File::open(archive_path)
                .map_err(|e| format!("打开 GZIP 归档失败: {e}"))?;
            let gz = flate2::read::GzDecoder::new(file);
            extract_tar(gz, dest_dir)?;
        }
        ArchiveFormat::Tar => {
            let file =
                std::fs::File::open(archive_path).map_err(|e| format!("打开 TAR 归档失败: {e}"))?;
            extract_tar(file, dest_dir)?;
        }
    }

    find_extracted_package_root(dest_dir)
}

fn extract_zip<R: std::io::Read + std::io::Seek>(reader: R, dest_dir: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("解析 ZIP 文件失败: {e}"))?;

    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let enclosed_name = file
            .enclosed_name()
            .ok_or_else(|| "ZIP 中包含非法路径或路径穿透攻击组件 (Zip Slip)".to_string())?;

        let out_path = dest_dir.join(enclosed_name);
        if !out_path.starts_with(dest_dir) {
            return Err("ZIP 条目解压路径超出目标沙箱目录 (Zip Slip 攻击防护拦截)".to_string());
        }

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {e}", out_path.display()))?;
        } else {
            if let Some(p) = out_path.parent().filter(|p| !p.exists()) {
                std::fs::create_dir_all(p)
                    .map_err(|e| format!("创建父目录失败 {}: {e}", p.display()))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败 {}: {e}", out_path.display()))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("写入文件内容失败 {}: {e}", out_path.display()))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    let _ =
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }
    Ok(())
}

fn extract_tar<R: std::io::Read>(reader: R, dest_dir: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| format!("读取 TAR 条目失败: {e}"))?;

    for entry_res in entries {
        let mut entry = entry_res.map_err(|e| format!("解压 TAR 条目异常: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("读取 TAR 路径失败: {e}"))?
            .into_owned();

        // 强安全校验：严禁绝对路径及 ParentDir / RootDir 路径穿透组件
        if path.is_absolute() {
            return Err("TAR 中包含非法绝对路径 (Tar Slip 攻击防护拦截)".to_string());
        }

        for comp in path.components() {
            if matches!(
                comp,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            ) {
                return Err("TAR 中包含非法相对路径组件或前缀 (Tar Slip 攻击防护拦截)".to_string());
            }
        }

        let out_path = dest_dir.join(&path);
        if !out_path.starts_with(dest_dir) {
            return Err("TAR 条目解压路径超出目标沙箱目录 (Tar Slip 攻击防护拦截)".to_string());
        }

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent().filter(|p| !p.exists()) {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建父目录失败 {}: {e}", parent.display()))?;
            }
            entry
                .unpack(&out_path)
                .map_err(|e| format!("解压文件失败 {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

fn find_extracted_package_root(base_dir: &Path) -> Result<PathBuf, String> {
    if base_dir.join(ALGO_MANIFEST_FILENAME).is_file() {
        return Ok(base_dir.to_path_buf());
    }

    if let Ok(entries) = std::fs::read_dir(base_dir) {
        let valid_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .is_none_or(|n| !n.to_string_lossy().starts_with('.'))
            })
            .collect();

        if valid_dirs.len() == 1 && valid_dirs[0].join(ALGO_MANIFEST_FILENAME).is_file() {
            return Ok(valid_dirs[0].clone());
        }

        for d in valid_dirs {
            if d.join(ALGO_MANIFEST_FILENAME).is_file() {
                return Ok(d);
            }
        }
    }

    Err(format!(
        "解压归档包后未在根目录或单层子目录中找到 {ALGO_MANIFEST_FILENAME} 文件"
    ))
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// RAII 临时目录守卫，Drop 时自动清理
#[derive(Debug)]
pub struct TempDirGuard(pub PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 临时上传文件守卫，持有任务结束、取消或异常时由 Drop 清理。
#[derive(Debug)]
pub struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_archive_format() {
        // Zip magic: PK\x03\x04
        let zip_magic = b"PK\x03\x04\x14\x00\x00\x00";
        assert_eq!(
            detect_archive_format(zip_magic, None).unwrap(),
            ArchiveFormat::Zip
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.zip")).unwrap(),
            ArchiveFormat::Zip
        );

        // Gzip magic: 0x1f, 0x8b
        let gz_magic = &[0x1f, 0x8b, 0x08, 0x00];
        assert_eq!(
            detect_archive_format(gz_magic, None).unwrap(),
            ArchiveFormat::TarGz
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tar.gz")).unwrap(),
            ArchiveFormat::TarGz
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tgz")).unwrap(),
            ArchiveFormat::TarGz
        );

        // Tar magic at offset 257: "ustar"
        let mut tar_magic = vec![0u8; 512];
        tar_magic[257..262].copy_from_slice(b"ustar");
        assert_eq!(
            detect_archive_format(&tar_magic, None).unwrap(),
            ArchiveFormat::Tar
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tar")).unwrap(),
            ArchiveFormat::Tar
        );
    }

    #[test]
    fn test_extract_tar_gz_archive() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let temp = std::env::temp_dir().join(format!(
            "test_argus_targz_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar_builder = tar::Builder::new(&mut gz_encoder);
            let manifest_content = br#"{"algorithmId": "test_targz_pkg"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_path("pkg_dir/manifest.json").unwrap();
            header.set_size(manifest_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &manifest_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }
        let gz_bytes = gz_encoder.finish().unwrap();
        let archive_path = temp.join("input.tar.gz");
        std::fs::write(&archive_path, gz_bytes).unwrap();

        let extracted_dir =
            extract_archive_package_from_file(&archive_path, Some("test.tar.gz"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_extract_uncompressed_tar_archive() {
        let temp =
            std::env::temp_dir().join(format!("test_argus_tar_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&temp).unwrap();

        let mut tar_bytes = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes);
            let manifest_content = br#"{"algorithmId": "test_pure_tar_pkg"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_path("manifest.json").unwrap();
            header.set_size(manifest_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &manifest_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let archive_path = temp.join("input.tar");
        std::fs::write(&archive_path, &tar_bytes).unwrap();
        let extracted_dir =
            extract_archive_package_from_file(&archive_path, Some("test.tar"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_extract_tar_rejects_absolute_path_and_parent_dir() {
        let temp = std::env::temp_dir().join(format!(
            "test_argus_tar_slip_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        // 构造包含恶意绝对路径的 TAR (直接修改底层 header 名字段模拟恶意包)
        let mut tar_bytes = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes);
            let evil_content = b"malicious content";
            let mut header = tar::Header::new_gnu();
            let name_bytes = b"/etc/passwd_fake";
            header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
            header.set_size(evil_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &evil_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(&tar_bytes);
        let res = extract_tar(cursor, &temp);
        assert!(res.is_err(), "Must reject tar with absolute path");
        assert!(
            res.unwrap_err().contains("Tar Slip"),
            "Error message must mention Tar Slip protection"
        );

        // 构造包含 .. 相对路径穿透的 TAR
        let mut tar_bytes_slip = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes_slip);
            let evil_content = b"malicious content";
            let mut header = tar::Header::new_gnu();
            let name_bytes = b"../../etc/evil.conf";
            header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
            header.set_size(evil_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &evil_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(&tar_bytes_slip);
        let res_slip = extract_tar(cursor, &temp);
        assert!(
            res_slip.is_err(),
            "Must reject tar with parent dir component"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }
}
