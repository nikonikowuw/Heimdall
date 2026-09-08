//! 算法包上传处理服务
//!
//! 负责归档解压调度、沙箱校验、落盘与配置提取。
//! 通过 `spawn_blocking` 在专用线程执行同步 I/O，不阻塞 Tokio worker。

use std::path::{Path, PathBuf};

use axum::extract::multipart::{Field, MultipartError};
use axum::extract::Multipart;
use db::{AlgorithmRepo, UpsertAlgorithmParams, UpsertVersionParams};
use infer::{compute_dir_size, AlgoManifest, AlgoSandbox, ALGO_MANIFEST_FILENAME};
use tokio::io::AsyncWriteExt;

use super::archive::{
    copy_dir_all, extract_archive_package_from_file, TempDirGuard, TempFileGuard,
};
use super::dto::{get_standard_steps, SandboxCheckResultDto, UploadVersionInfo};
use crate::error::ApiError;
use crate::state::AppState;

pub const BYTES_PER_MEGABYTE: usize = 1024 * 1024;

#[derive(Debug)]
pub struct ProcessedUploadResult {
    pub manifest: AlgoManifest,
    pub target_dir_str: String,
    pub pkg_size: i64,
    pub config_schema_json: String,
    pub manifest_raw_json: String,
    pub fps_tiers_json: String,
}

#[derive(Debug)]
pub struct ProcessedUploadError {
    pub failed_idx: usize,
    pub message: String,
    pub manifest: Option<AlgoManifest>,
}

pub(crate) fn parse_failed_step_index(err: &infer::InferError) -> usize {
    if let infer::InferError::SandboxValidation { step, .. } = err {
        if let Some(first_char) = step.chars().next() {
            if let Some(digit) = first_char.to_digit(10) {
                return (digit as usize).saturating_sub(1);
            }
        }
    }
    3
}

pub fn is_multipart_limit_exceeded(error: &MultipartError) -> bool {
    if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }

    let mut text = error.to_string();
    use std::error::Error;
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(" -> ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }

    text.contains("limit")
        || text.contains("PayloadTooLarge")
        || text.contains("length limit exceeded")
        || text.contains("request body exceeded")
}

pub fn upload_size_limit_error(max_bytes: usize) -> ApiError {
    let max_mb = max_bytes.div_ceil(BYTES_PER_MEGABYTE);
    ApiError::BadRequest(format!(
        "算法包文件体积超出系统配置的最大限制 ({} MB)，可在配置文件中调大 max_package_size_mb",
        max_mb
    ))
}

pub fn map_multipart_error(error: MultipartError, operation: &str, max_bytes: usize) -> ApiError {
    if is_multipart_limit_exceeded(&error) {
        upload_size_limit_error(max_bytes)
    } else {
        ApiError::BadRequest(format!("{operation}: {error}"))
    }
}

pub async fn write_upload_field_to_temp_file(
    mut field: Field<'_>,
    max_bytes: usize,
) -> Result<TempFileGuard, ApiError> {
    let path = std::env::temp_dir().join(format!(
        "argus_upload_{}.part",
        uuid::Uuid::new_v4().simple()
    ));
    let guard = TempFileGuard::new(path);
    let write_path = guard.path().to_path_buf();

    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&write_path)
            .await
            .map_err(|error| ApiError::Internal(format!("创建算法包临时文件失败: {error}")))?;
        let mut total_bytes = 0usize;

        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|error| map_multipart_error(error, "读取文件流失败", max_bytes))?
        {
            total_bytes = total_bytes
                .checked_add(chunk.len())
                .ok_or_else(|| upload_size_limit_error(max_bytes))?;
            if total_bytes > max_bytes {
                return Err(upload_size_limit_error(max_bytes));
            }

            file.write_all(&chunk)
                .await
                .map_err(|error| ApiError::Internal(format!("写入算法包临时文件失败: {error}")))?;
        }

        file.flush()
            .await
            .map_err(|error| ApiError::Internal(format!("刷新算法包临时文件失败: {error}")))?;

        if total_bytes == 0 {
            return Err(ApiError::BadRequest("未找到有效的算法包文件流".to_string()));
        }

        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(guard),
        Err(error) => {
            drop(guard);
            Err(error)
        }
    }
}

/// 同步执行归档解压、沙箱进程驱动、文件复制与资源配置提取 (供 spawn_blocking 调度)
pub fn process_uploaded_package_archive_sync(
    archive_path: &Path,
    upload_filename: Option<&str>,
) -> Result<ProcessedUploadResult, Box<ProcessedUploadError>> {
    let temp_dir =
        std::env::temp_dir().join(format!("argus_pkg_{}", uuid::Uuid::new_v4().simple()));
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        return Err(Box::new(ProcessedUploadError {
            failed_idx: 0,
            message: format!("创建临时解压目录失败: {e}"),
            manifest: None,
        }));
    }
    let _temp_guard = TempDirGuard(temp_dir.clone());

    let src_pkg_dir =
        match extract_archive_package_from_file(archive_path, upload_filename, &temp_dir) {
            Ok(dir) => dir,
            Err(err) => {
                return Err(Box::new(ProcessedUploadError {
                    failed_idx: 0,
                    message: err,
                    manifest: None,
                }));
            }
        };

    let manifest_path = src_pkg_dir.join(ALGO_MANIFEST_FILENAME);
    let manifest_bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) => {
            return Err(Box::new(ProcessedUploadError {
                failed_idx: 0,
                message: format!("读取 {ALGO_MANIFEST_FILENAME} 失败: {e}"),
                manifest: None,
            }));
        }
    };
    let manifest: AlgoManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(m) => m,
        Err(e) => {
            return Err(Box::new(ProcessedUploadError {
                failed_idx: 0,
                message: format!("解析 {ALGO_MANIFEST_FILENAME} 格式失败: {e}"),
                manifest: None,
            }));
        }
    };

    let validated_manifest = match AlgoSandbox::validate_package(&src_pkg_dir, false) {
        Ok(m) => m,
        Err(e) => {
            let failed_idx = parse_failed_step_index(&e);
            return Err(Box::new(ProcessedUploadError {
                failed_idx,
                message: e.to_string(),
                manifest: Some(manifest),
            }));
        }
    };

    let target_dir = PathBuf::from("var/packages")
        .join(&validated_manifest.algorithm_id)
        .join(&validated_manifest.version);

    if target_dir.exists() {
        let _ = std::fs::remove_dir_all(&target_dir);
    }
    if let Some(parent) = target_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let copy_res = copy_dir_all(&src_pkg_dir, &target_dir);
    // _temp_guard drops here (or at function exit) to clean up temp_dir

    if let Err(e) = copy_res {
        return Err(Box::new(ProcessedUploadError {
            failed_idx: 0,
            message: format!("安装算法包失败: {e}"),
            manifest: Some(validated_manifest),
        }));
    }

    let pkg_size = compute_dir_size(&target_dir);
    let schema_path = target_dir.join("config.schema.json");
    let config_schema_json = if schema_path.is_file() {
        std::fs::read_to_string(&schema_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    };

    let manifest_raw_json = std::fs::read_to_string(target_dir.join(ALGO_MANIFEST_FILENAME))
        .unwrap_or_else(|_| "{}".to_string());
    let manifest_val: serde_json::Value =
        serde_json::from_str(&manifest_raw_json).unwrap_or_default();
    let fps_tiers_json = manifest_val
        .get("resource_profile")
        .and_then(|rp| rp.get("fps_tiers"))
        .map(|ft| ft.to_string())
        .unwrap_or_else(|| "[]".to_string());

    let target_dir_str = target_dir.to_string_lossy().to_string();

    Ok(ProcessedUploadResult {
        manifest: validated_manifest,
        target_dir_str,
        pkg_size,
        config_schema_json,
        manifest_raw_json,
        fps_tiers_json,
    })
}

/// 上传算法包完整业务编排：限流、流式落盘、沙箱自检、数据库记录与即时热装载
pub async fn handle_package_upload(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<SandboxCheckResultDto, ApiError> {
    let _upload_permit = state
        .algorithm_upload_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::Internal("算法包上传并发控制器已关闭".to_string()))?;
    let steps = get_standard_steps();
    let mut upload_filename: Option<String> = None;
    let mut upload_guard: Option<TempFileGuard> = None;

    while let Some(field) = multipart.next_field().await.map_err(|error| {
        map_multipart_error(error, "解析上传表单失败", state.max_upload_size_bytes)
    })? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" || name == "package" {
            if let Some(filename) = field.file_name() {
                upload_filename = Some(filename.to_string());
            }
            upload_guard =
                Some(write_upload_field_to_temp_file(field, state.max_upload_size_bytes).await?);
            break;
        }
    }

    let upload_guard = match upload_guard {
        Some(guard) => guard,
        None => return Err(ApiError::BadRequest("未找到有效的算法包文件流".to_string())),
    };
    let process_path = upload_guard.path().to_path_buf();
    let process_res = tokio::task::spawn_blocking(move || {
        let _upload_guard = upload_guard;
        process_uploaded_package_archive_sync(&process_path, upload_filename.as_deref())
    })
    .await;

    let process_res = process_res
        .map_err(|error| ApiError::Internal(format!("执行沙箱解包自检任务异常: {error}")))?;

    let processed = match process_res {
        Ok(res) => res,
        Err(boxed_err) => {
            return Ok(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: boxed_err.failed_idx,
                steps,
                error_message: Some(boxed_err.message),
                version: None,
                manifest: boxed_err.manifest,
            });
        }
    };

    let validated_manifest = processed.manifest;
    let target_dir_str = processed.target_dir_str;

    // 写入 algorithms
    AlgorithmRepo::upsert_algorithm(
        &state.db,
        UpsertAlgorithmParams {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            name: validated_manifest.name.clone(),
            algorithm_type: validated_manifest.algorithm_type.clone(),
            alarm_type_id: validated_manifest.alarm_type_id.clone(),
            active_version: validated_manifest.version.clone(),
            description: validated_manifest.description.clone().unwrap_or_default(),
            is_builtin: false,
        },
    )
    .await?;

    // 写入 algorithm_versions
    AlgorithmRepo::upsert_version(
        &state.db,
        UpsertVersionParams {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            version: validated_manifest.version.clone(),
            platform_id: validated_manifest.platform_id.clone(),
            min_adapter_version: validated_manifest
                .min_adapter_version
                .clone()
                .unwrap_or_default(),
            package_root: target_dir_str.clone(),
            fps_tiers: processed.fps_tiers_json,
            config_schema: processed.config_schema_json,
            manifest_raw: processed.manifest_raw_json,
            package_size_bytes: processed.pkg_size,
            is_active: true,
            is_builtin: false,
        },
    )
    .await?;

    // 在事务中激活该版本
    AlgorithmRepo::activate_version(
        &state.db,
        &validated_manifest.algorithm_id,
        &validated_manifest.version,
        Some(&validated_manifest.platform_id),
    )
    .await?;

    // 热加载至内存注册中心，并热重载给运行中管线 (零停机、不断流)
    let target_dir = PathBuf::from(&target_dir_str);
    if let Ok(pkg) = state
        .algo_registry
        .load_and_register(&target_dir, false)
        .await
    {
        tracing::info!(
            algorithm_id = %pkg.manifest().algorithm_id,
            version = %pkg.manifest().version,
            "算法包上传成功并已即时热装载"
        );
        if let Ok(inst) = pkg.create_instance("{}", None) {
            let worker = infer::InferenceWorker::new(std::sync::Arc::new(inst));
            state
                .pipeline
                .reload_algorithm_on_pumps(worker.handle())
                .await;
        }
    }

    Ok(SandboxCheckResultDto {
        passed: true,
        steps_total: 7,
        steps_passed: 7,
        steps,
        error_message: None,
        version: Some(UploadVersionInfo {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            version: validated_manifest.version.clone(),
            platform_id: validated_manifest.platform_id.clone(),
            package_root: target_dir_str,
            is_active: true,
        }),
        manifest: Some(validated_manifest),
    })
}
