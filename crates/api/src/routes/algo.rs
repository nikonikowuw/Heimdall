use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Multipart, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use infer::{
    current_platform_id, AlgoManifest, AlgoPackage, AlgoSandbox, InferError,
    ALGO_MANIFEST_FILENAME, DEFAULT_ALGO_PACKAGES_DIR,
};

use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPackageRequest {
    pub package_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCheckResultDto {
    pub passed: bool,
    pub steps_total: usize,
    pub steps_passed: usize,
    pub steps: Vec<String>,
    pub error_message: Option<String>,
    pub manifest: Option<AlgoManifest>,
}

fn get_standard_steps() -> Vec<String> {
    vec![
        "1. 路径防穿透与目录结构检查".to_string(),
        "2. SHA256 完整性与安全指纹校验".to_string(),
        "3. 解析 Manifest 与平台拓扑匹配".to_string(),
        "4. Config Schema 参数格式校验".to_string(),
        "5. 派生隔离子进程与超时守护".to_string(),
        "6. 算法库 C ABI 导出符号核对".to_string(),
        "7. 真实前向推理自测与内存复核".to_string(),
    ]
}

fn parse_failed_step_index(err: &InferError) -> usize {
    if let InferError::SandboxValidation { step, .. } = err {
        if let Some(first_char) = step.chars().next() {
            if let Some(digit) = first_char.to_digit(10) {
                return (digit as usize).saturating_sub(1);
            }
        }
    }
    3
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/packages", get(list_packages))
        .route("/verify", post(verify_package))
        .route("/upload", post(upload_package))
        .route("/scan", post(scan_packages))
}

async fn list_packages(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<AlgoManifest>>, ApiError> {
    let list = state.algo_registry.list().await;
    Ok(ApiResponse::success(list))
}

async fn verify_package(
    State(_state): State<AppState>,
    Json(req): Json<VerifyPackageRequest>,
) -> Result<ApiResponse<SandboxCheckResultDto>, ApiError> {
    let steps = get_standard_steps();

    let path_str = req.package_path.unwrap_or_else(|| {
        let cur = current_platform_id();
        format!("algo-packages/{cur}/general_detection")
    });

    let p = PathBuf::from(&path_str);
    if !p.is_dir() {
        return Ok(ApiResponse::success(SandboxCheckResultDto {
            passed: false,
            steps_total: 7,
            steps_passed: 0,
            steps,
            error_message: Some(format!("算法包目录不存在: {path_str}")),
            manifest: None,
        }));
    }

    match AlgoSandbox::validate_package(&p, false) {
        Ok(manifest) => Ok(ApiResponse::success(SandboxCheckResultDto {
            passed: true,
            steps_total: 7,
            steps_passed: 7,
            steps,
            error_message: None,
            manifest: Some(manifest),
        })),
        Err(e) => {
            let failed_idx = parse_failed_step_index(&e);
            Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: failed_idx,
                steps,
                error_message: Some(e.to_string()),
                manifest: None,
            }))
        }
    }
}

async fn scan_packages(State(state): State<AppState>) -> Result<ApiResponse<usize>, ApiError> {
    let base = Path::new(DEFAULT_ALGO_PACKAGES_DIR);
    let count = state
        .algo_registry
        .scan_and_register(base, false)
        .await
        .unwrap_or(0);
    Ok(ApiResponse::success(count))
}

/// 上传 .zip / .tar.gz / .tar 算法包并在物理沙箱中自检加载
async fn upload_package(
    State(state): State<AppState>,
    _user: AuthUser,
    mut multipart: Multipart,
) -> Result<ApiResponse<SandboxCheckResultDto>, ApiError> {
    let steps = get_standard_steps();
    let mut archive_bytes: Option<Vec<u8>> = None;
    let mut upload_filename: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("解析上传表单失败: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" || name == "package" {
            if let Some(fname) = field.file_name() {
                upload_filename = Some(fname.to_string());
            }
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("读取文件流失败: {e}")))?;
            archive_bytes = Some(data.to_vec());
            break;
        }
    }

    let bytes = match archive_bytes {
        Some(b) if !b.is_empty() => b,
        _ => return Err(ApiError::BadRequest("未找到有效的算法包文件流".to_string())),
    };

    // 1. 解压到临时目录
    let temp_dir =
        std::env::temp_dir().join(format!("argus_pkg_{}", uuid::Uuid::new_v4().simple()));
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        return Err(ApiError::Internal(format!("创建临时解压目录失败: {e}")));
    }

    let src_pkg_dir = match extract_archive_package(&bytes, upload_filename.as_deref(), &temp_dir) {
        Ok(dir) => dir,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: 0,
                steps,
                error_message: Some(err),
                manifest: None,
            }));
        }
    };

    // 2. 预读 manifest 确定 algorithm_id
    let manifest_bytes = std::fs::read(src_pkg_dir.join(ALGO_MANIFEST_FILENAME))
        .map_err(|e| ApiError::BadRequest(format!("读取 {ALGO_MANIFEST_FILENAME} 失败: {e}")))?;
    let manifest: AlgoManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        ApiError::BadRequest(format!("解析 {ALGO_MANIFEST_FILENAME} 格式失败: {e}"))
    })?;

    // 3. 搬迁至本地专用算法目录: algo-packages/{current_platform}/{algorithm_id}
    let cur_plat = current_platform_id();
    let target_dir = PathBuf::from(DEFAULT_ALGO_PACKAGES_DIR)
        .join(cur_plat)
        .join(&manifest.algorithm_id);

    if target_dir.exists() {
        let _ = std::fs::remove_dir_all(&target_dir);
    }
    if let Some(parent) = target_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 递归复制解压内容至目标目录
    let copy_res = copy_dir_all(&src_pkg_dir, &target_dir);
    let _ = std::fs::remove_dir_all(&temp_dir);

    if let Err(e) = copy_res {
        return Err(ApiError::Internal(format!("安装算法包失败: {e}")));
    }

    // 4. 执行完整物理沙箱安全检验（包含 7 步验证）
    match AlgoSandbox::validate_package(&target_dir, false) {
        Ok(validated_manifest) => {
            // 热加载并注入当前运行注册中心
            if let Ok(pkg) = AlgoPackage::load_and_verify(&target_dir, false) {
                state.algo_registry.register(Arc::new(pkg)).await;
            }

            Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: true,
                steps_total: 7,
                steps_passed: 7,
                steps,
                error_message: None,
                manifest: Some(validated_manifest),
            }))
        }
        Err(e) => {
            let failed_idx = parse_failed_step_index(&e);
            Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: failed_idx,
                steps,
                error_message: Some(e.to_string()),
                manifest: Some(manifest),
            }))
        }
    }
}

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
    // 1. 优先通过 Magic Bytes 判定
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        return Ok(ArchiveFormat::TarGz);
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return Ok(ArchiveFormat::Zip);
    }

    // 2. 检查 tar 格式标志 (POSIX tar 在 offset 257 处包含 "ustar")
    if bytes.len() >= 262 && &bytes[257..262] == b"ustar" {
        return Ok(ArchiveFormat::Tar);
    }

    // 3. 基于文件名后缀作为辅助兜底
    if let Some(name) = filename {
        let lower = name.to_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            return Ok(ArchiveFormat::TarGz);
        }
        if lower.ends_with(".tar") {
            return Ok(ArchiveFormat::Tar);
        }
        if lower.ends_with(".zip") {
            return Ok(ArchiveFormat::Zip);
        }
    }

    // 4. 兜底：如果大于 512 字节且非 zip/gz，尝试按 tar 处理
    if bytes.len() >= 512 {
        return Ok(ArchiveFormat::Tar);
    }

    Err("不支持的文件格式，仅支持 .zip、.tar.gz (.tgz) 或 .tar 算法包".to_string())
}

pub fn extract_archive_package(
    bytes: &[u8],
    filename: Option<&str>,
    temp_dir: &Path,
) -> Result<PathBuf, String> {
    let format = detect_archive_format(bytes, filename)?;
    match format {
        ArchiveFormat::Zip => extract_zip(bytes, temp_dir)?,
        ArchiveFormat::TarGz => {
            let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
            extract_tar(gz, temp_dir)?;
        }
        ArchiveFormat::Tar => {
            let cursor = std::io::Cursor::new(bytes);
            extract_tar(cursor, temp_dir)?;
        }
    }
    find_manifest_dir(temp_dir)
}

fn extract_zip(bytes: &[u8], temp_dir: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("无效的 ZIP 压缩文件: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("读取压缩项失败: {e}"))?;
        let enclosed = match file.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };
        let outpath = temp_dir.join(enclosed);
        if (*file.name()).ends_with('/') {
            std::fs::create_dir_all(&outpath).map_err(|e| format!("创建目录失败: {e}"))?;
        } else {
            if let Some(p) = outpath.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("创建父目录失败: {e}"))?;
            }
            let mut outfile =
                std::fs::File::create(&outpath).map_err(|e| format!("创建目标文件失败: {e}"))?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| format!("解压文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn extract_tar<R: std::io::Read>(reader: R, temp_dir: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| format!("读取 TAR 归档项失败: {e}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("读取 TAR 实体失败: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("解析 TAR 路径失败: {e}"))?;

        // 严防路径穿越与非法绝对路径
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }

        let outpath = temp_dir.join(&path);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| format!("创建目录失败: {e}"))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
            }
            entry
                .unpack(&outpath)
                .map_err(|e| format!("解压 TAR 文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn find_manifest_dir(temp_dir: &Path) -> Result<PathBuf, String> {
    if temp_dir.join(ALGO_MANIFEST_FILENAME).is_file() {
        return Ok(temp_dir.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(ALGO_MANIFEST_FILENAME).is_file() {
                return Ok(path);
            }
        }
    }
    Err(format!("算法包内未找到 {ALGO_MANIFEST_FILENAME} 描述文件"))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
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

        // 构造带子目录的 tar.gz 数据
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

        let extracted_dir = extract_archive_package(&gz_bytes, Some("test.tar.gz"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_extract_uncompressed_tar_archive() {
        let temp =
            std::env::temp_dir().join(format!("test_argus_tar_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&temp).unwrap();

        // 构造纯 tar 数据 (无 gzip 压缩)
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

        let extracted_dir = extract_archive_package(&tar_bytes, Some("test.tar"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }
}
