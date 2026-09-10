use std::path::{Path, PathBuf};

use db::{AlgorithmRepo, DatabaseConnection};
use infer::current_platform_id;

use crate::algo::dto::AlgorithmVersionItemDto;
use crate::error::ApiError;
use crate::state::AppState;

/// 解析 ID 参数：可能是数字主键，也可能是字符串 algorithm_id
pub async fn resolve_algorithm_id(
    db: &DatabaseConnection,
    param: &str,
) -> Result<String, ApiError> {
    if let Ok(num_id) = param.parse::<i64>() {
        if let Some(m) = AlgorithmRepo::find_by_id(db, num_id).await? {
            return Ok(m.algorithm_id);
        }
    }
    if let Some(m) = AlgorithmRepo::find_by_algorithm_id(db, param).await? {
        return Ok(m.algorithm_id);
    }
    Err(ApiError::NotFound(format!("未找到指定的算法: {param}")))
}

/// 从数据库加载指定算法的所有版本并转换为 DTO
pub async fn load_version_dtos(
    db: &DatabaseConnection,
    algorithm_id: &str,
) -> Result<Vec<AlgorithmVersionItemDto>, ApiError> {
    let raw = AlgorithmRepo::list_versions_by_algorithm_id(db, algorithm_id).await?;
    Ok(raw.into_iter().map(AlgorithmVersionItemDto::from).collect())
}

/// 激活指定版本 (支持多平台并联动管线单进程优雅热重载)
pub async fn activate_version(state: &AppState, id: &str, version: &str) -> Result<(), ApiError> {
    let aid = resolve_algorithm_id(&state.db, id).await?;
    let cur_plat = current_platform_id();

    // 优先匹配当前平台的版本
    let ver_model =
        match AlgorithmRepo::find_version(&state.db, &aid, version, Some(cur_plat)).await? {
            Some(v) => v,
            None => AlgorithmRepo::find_version(&state.db, &aid, version, None)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("未找到算法版本: {aid}:{version}")))?,
        };

    // 事务切换激活状态
    AlgorithmRepo::activate_version(&state.db, &aid, version, Some(&ver_model.platform_id)).await?;

    // 检查并加载进内存注册中心，并通知 Pipeline 优雅热替换
    let root = Path::new(&ver_model.package_root);
    if root.is_dir() {
        if let Ok(pkg) = state.algo_registry.load_and_register(root, false).await {
            let count = state.pipeline.reload_algorithm_on_pumps(&aid, pkg).await;
            tracing::info!(
                algorithm_id = %aid,
                version = %version,
                reloaded_pumps = count,
                "已原子完成单进程算法版本优雅热重载"
            );
        }
    }

    Ok(())
}

/// 安全卸载指定版本 (防孤儿文件与路径逃逸保护)
pub async fn uninstall_version(state: &AppState, id: &str, version: &str) -> Result<(), ApiError> {
    let aid = resolve_algorithm_id(&state.db, id).await?;
    let cur_plat = current_platform_id();

    // 执行卸载，内置算法或使用中算法会自动阻断并返回对应错误
    let package_root =
        match AlgorithmRepo::uninstall_version(&state.db, &aid, version, Some(cur_plat)).await {
            Ok(r) => r,
            Err(db::DbError::NotFound { .. }) => {
                AlgorithmRepo::uninstall_version(&state.db, &aid, version, None).await?
            }
            Err(e) => return Err(e.into()),
        };

    // 清理物理磁盘目录（防孤儿死文件），强约束仅限 var/packages 目录树内
    let root_path = PathBuf::from(&package_root);
    if !package_root.is_empty()
        && root_path.exists()
        && (root_path.starts_with("var/packages")
            || root_path
                .canonicalize()
                .map(|c| c.to_string_lossy().contains("var/packages"))
                .unwrap_or(false))
    {
        let _ = std::fs::remove_dir_all(&root_path);
    }

    // 检查剩余版本是否需要更新内存注册中心
    if let Some(algo) = AlgorithmRepo::find_by_algorithm_id(&state.db, &aid).await? {
        if !algo.active_version.is_empty() {
            if let Some(active_ver) =
                AlgorithmRepo::find_version(&state.db, &aid, &algo.active_version, None).await?
            {
                let p = Path::new(&active_ver.package_root);
                if p.is_dir() {
                    let _ = state.algo_registry.load_and_register(p, false).await;
                }
            }
        }
    } else {
        // 该算法已被彻底删除
        state.algo_registry.unregister(&aid).await;
    }

    Ok(())
}
