use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use db::{AlgorithmRepo, DatabaseConnection, UpsertAlgorithmParams, UpsertVersionParams};
use infer::{
    compute_dir_size, current_platform_id, AlgoManifest, AlgoRegistry, AlgoSandbox,
    ALGO_MANIFEST_FILENAME, DEFAULT_ALGO_PACKAGES_DIR,
};

/// 启动时自愈对齐扫描与装载：
/// 1. 扫描当前平台内置目录 (algo-packages/{platform}) 与已安装目录 (var/packages)
/// 2. 对未入库算法执行沙箱自检，自动在 DB algorithms 与 algorithm_versions 中自愈落库
/// 3. 以 DB 中所有 is_active = true 为事实源，装载至内存 AlgoRegistry
pub async fn reconcile_and_seed_algorithms(
    db: &DatabaseConnection,
    registry: &Arc<AlgoRegistry>,
) -> Result<(usize, usize)> {
    let cur_plat = current_platform_id();
    let base_algo_dir = PathBuf::from(DEFAULT_ALGO_PACKAGES_DIR);
    let canonical_base = base_algo_dir.canonicalize().ok();

    // 候选目录：平台专属目录、顶级目录、已安装二进制根目录
    let mut search_dirs = vec![
        base_algo_dir.join(cur_plat),
        base_algo_dir.clone(),
        PathBuf::from("var/packages"),
    ];

    // 特别兼容 macOS arm64 历史简写与分级路径
    if cur_plat.contains("macos") {
        search_dirs.push(base_algo_dir.join("macos-arm64"));
        search_dirs.push(base_algo_dir.join("macos").join("arm64"));
    }

    let candidate_dirs = infer::discover_package_dirs(&search_dirs);
    let mut seeded_count = 0;

    for dir in candidate_dirs {
        // 1. 先快速解析 manifest，检查是否已在 DB 中，避免每次冷启动触发昂贵的全量前向推理自测
        let manifest_path = dir.join(ALGO_MANIFEST_FILENAME);
        if !manifest_path.is_file() {
            continue;
        }

        let manifest_bytes = match std::fs::read(&manifest_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let preview_manifest: AlgoManifest = match serde_json::from_slice(&manifest_bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let existing_ver = AlgorithmRepo::find_version(
            db,
            &preview_manifest.algorithm_id,
            &preview_manifest.version,
            Some(&preview_manifest.platform_id),
        )
        .await?;

        // 若 DB 中已记录此版本，则直接跳过沙箱自检
        if existing_ver.is_some() {
            continue;
        }

        // 2. 仅对未入库的新算法包执行 7 步沙箱物理自测校验
        match AlgoSandbox::validate_package(&dir, false) {
            Ok(manifest) => {
                let is_builtin = dir.starts_with(&base_algo_dir)
                    || canonical_base
                        .as_ref()
                        .map(|cb| {
                            dir.canonicalize()
                                .map(|c| c.starts_with(cb))
                                .unwrap_or(false)
                        })
                        .unwrap_or(false);

                let dir_str = dir.to_string_lossy().to_string();
                let pkg_size = compute_dir_size(&dir);

                // 读取 config.schema.json
                let schema_path = dir.join("config.schema.json");
                let config_schema_json = if schema_path.is_file() {
                    std::fs::read_to_string(&schema_path).unwrap_or_else(|_| "{}".to_string())
                } else {
                    "{}".to_string()
                };

                // 读取 manifest 原始 JSON 与提取 fps_tiers
                let manifest_raw_json =
                    std::fs::read_to_string(&manifest_path).unwrap_or_else(|_| "{}".to_string());
                let manifest_val: serde_json::Value =
                    serde_json::from_str(&manifest_raw_json).unwrap_or_default();
                let fps_tiers_json = manifest_val
                    .get("resource_profile")
                    .and_then(|rp| rp.get("fps_tiers"))
                    .map(|ft| ft.to_string())
                    .unwrap_or_else(|| "[]".to_string());

                // 插入/更新算法主表
                AlgorithmRepo::upsert_algorithm(
                    db,
                    UpsertAlgorithmParams {
                        algorithm_id: manifest.algorithm_id.clone(),
                        name: manifest.name.clone(),
                        algorithm_type: manifest.algorithm_type.clone(),
                        alarm_type_id: manifest.alarm_type_id.clone(),
                        active_version: manifest.version.clone(),
                        description: manifest.description.clone().unwrap_or_default(),
                        is_builtin,
                    },
                )
                .await?;

                // 插入算法版本表
                AlgorithmRepo::upsert_version(
                    db,
                    UpsertVersionParams {
                        algorithm_id: manifest.algorithm_id.clone(),
                        version: manifest.version.clone(),
                        platform_id: manifest.platform_id.clone(),
                        min_adapter_version: manifest
                            .min_adapter_version
                            .clone()
                            .unwrap_or_default(),
                        package_root: dir_str,
                        fps_tiers: fps_tiers_json,
                        config_schema: config_schema_json,
                        manifest_raw: manifest_raw_json,
                        package_size_bytes: pkg_size,
                        is_active: true,
                        is_builtin,
                    },
                )
                .await?;

                tracing::info!(
                    algorithm_id = %manifest.algorithm_id,
                    version = %manifest.version,
                    is_builtin,
                    "系统冷启动自愈入库算法包成功"
                );
                seeded_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    path = %dir.display(),
                    error = %e,
                    "自愈扫描新算法包校验失败，跳过入库"
                );
            }
        }
    }

    // 以 DB 为唯一事实源装载所有活跃版本至内存 AlgoRegistry
    let active_versions = AlgorithmRepo::list_active_versions(db).await?;
    let mut loaded_count = 0;

    for ver in active_versions {
        let root = Path::new(&ver.package_root);
        if !root.is_dir() {
            tracing::warn!(
                algorithm_id = %ver.algorithm_id,
                version = %ver.version,
                path = %ver.package_root,
                "数据库记录的活跃算法包物理目录不存在或已损坏"
            );
            continue;
        }

        match registry.load_and_register(root, false).await {
            Ok(pkg) => {
                tracing::info!(
                    algorithm_id = %pkg.manifest().algorithm_id,
                    version = %pkg.manifest().version,
                    "成功装载并激活运行算法包"
                );
                loaded_count += 1;
            }
            Err(e) => {
                tracing::error!(
                    algorithm_id = %ver.algorithm_id,
                    version = %ver.version,
                    path = %ver.package_root,
                    error = %e,
                    "装载活跃算法包失败"
                );
            }
        }
    }

    Ok((seeded_count, loaded_count))
}
