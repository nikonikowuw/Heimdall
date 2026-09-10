use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use db::{
    AlgorithmRepo, CameraRepo, DatabaseConnection, TaskRepo, UpsertAlgorithmParams,
    UpsertVersionParams,
};
use infer::{
    compute_dir_size, current_platform_id, AlgoManifest, AlgoRegistry, AlgoSandbox,
    ALGO_MANIFEST_FILENAME, DEFAULT_ALGO_PACKAGES_DIR,
};
use pipeline::TaskRuntimeService;
use types::TaskStatus;

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

        // 2. 仅对未入库的新算法包执行 6 步沙箱物理自测校验
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

/// 冷启动恢复持久化启用任务。
///
/// 算法包完成注册后按任务顺序恢复，避免在启动阶段无界并发创建硬件上下文。
/// 摄像头最近一次探活不是健康状态时保留 desired_enabled，并将实际状态置为
/// reconnecting，交给后续探活/人工重试处理。
pub async fn recover_enabled_tasks(
    db: &DatabaseConnection,
    runtime: &dyn TaskRuntimeService,
) -> Result<TaskRecoverySummary> {
    let (tasks, instances_by_task_id) = TaskRepo::list_all_with_instances(db).await?;
    let cameras = CameraRepo::list_all(db).await?;
    let cameras_by_id: HashMap<String, db::entity::camera::Model> = cameras
        .into_iter()
        .map(|camera| (camera.camera_id.clone(), camera))
        .collect();

    let mut summary = TaskRecoverySummary::default();
    for task in tasks.into_iter().filter(|task| task.desired_enabled) {
        summary.attempted += 1;

        let Some(camera) = cameras_by_id.get(&task.camera_id) else {
            update_recovery_status(
                db,
                &task.camera_id,
                TaskStatus::Error,
                "任务关联的摄像头不存在",
            )
            .await?;
            summary.failed += 1;
            continue;
        };

        if !is_recoverable_camera_status(&camera.last_probe_status) {
            update_recovery_status(
                db,
                &task.camera_id,
                TaskStatus::Reconnecting,
                "摄像头启动时未通过健康检查，等待探活恢复",
            )
            .await?;
            summary.deferred += 1;
            continue;
        }

        let task_instances = instances_by_task_id
            .get(&task.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let enabled_instances: Vec<_> = task_instances
            .iter()
            .filter(|instance| instance.enabled)
            .collect();
        if enabled_instances.is_empty() {
            update_recovery_status(
                db,
                &task.camera_id,
                TaskStatus::Error,
                "任务未绑定任何可运行的算法实例",
            )
            .await?;
            summary.failed += 1;
            continue;
        }

        let launch_instances = match parse_launch_instances(&enabled_instances) {
            Ok(instances) => instances,
            Err(message) => {
                update_recovery_status(db, &task.camera_id, TaskStatus::Error, &message).await?;
                summary.failed += 1;
                continue;
            }
        };

        let motion_gate =
            serde_json::from_str::<types::MotionGateConfig>(&task.motion_gate_json).ok();
        let params = api::task_service::build_start_params(
            &task.camera_id,
            camera,
            launch_instances,
            motion_gate.as_ref(),
        );

        match runtime.start_camera_pipeline(params).await {
            Ok(generation) => {
                update_recovery_status(
                    db,
                    &task.camera_id,
                    TaskStatus::Running,
                    &format!("冷启动恢复成功，运行代次 {generation}"),
                )
                .await?;
                summary.recovered += 1;
            }
            Err(err) => {
                let message = format!("冷启动恢复失败: {err}");
                tracing::error!(camera_id = %task.camera_id, error = %err, "启用任务冷启动恢复失败");
                update_recovery_status(db, &task.camera_id, TaskStatus::Error, &message).await?;
                summary.failed += 1;
            }
        }
    }

    Ok(summary)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TaskRecoverySummary {
    pub attempted: usize,
    pub recovered: usize,
    pub deferred: usize,
    pub failed: usize,
}

fn parse_launch_instances(
    instances: &[&db::entity::algorithm_instance::Model],
) -> Result<Vec<pipeline::InstanceLaunchConfig>, String> {
    let mut launch_instances = Vec::with_capacity(instances.len());
    for instance in instances {
        let launch_config = pipeline::InstanceLaunchConfig::from_persisted(
            &instance.algorithm_id,
            &instance.params_json,
            instance.analysis_fps,
        )
        .map_err(|err| format!("算法实例 {} 配置错误: {err}", instance.instance_id))?;
        launch_instances.push(launch_config);
    }
    Ok(launch_instances)
}

fn is_recoverable_camera_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "healthy" | "success" | "online"
    )
}

async fn update_recovery_status(
    db: &DatabaseConnection,
    camera_id: &str,
    status: TaskStatus,
    message: &str,
) -> Result<()> {
    TaskRepo::update_status(db, camera_id, status.as_i32(), message).await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use db::{init_test_db, CameraRepo, SaveTaskWithInstancesParams, UpsertAlgorithmParams};
    use pipeline::coordinator::{
        CameraPipelineRuntimeInfo, CoordinatorError, StartCameraPipelineParams,
    };
    use sea_orm::ActiveValue::Set;
    use sea_orm::ConnectionTrait;

    #[derive(Debug, Default)]
    struct MockTaskRuntime {
        start_count: AtomicUsize,
        should_fail_start: AtomicBool,
        last_params: Mutex<Option<StartCameraPipelineParams>>,
    }

    #[async_trait::async_trait]
    impl TaskRuntimeService for MockTaskRuntime {
        async fn start_camera_pipeline(
            &self,
            params: StartCameraPipelineParams,
        ) -> std::result::Result<u64, CoordinatorError> {
            self.start_count.fetch_add(1, Ordering::Relaxed);
            *self.last_params.lock().unwrap() = Some(params);
            if self.should_fail_start.load(Ordering::Relaxed) {
                Err(CoordinatorError::Validation {
                    reason: "模拟启动失败".to_string(),
                })
            } else {
                Ok(1)
            }
        }

        async fn stop_camera_pipeline(
            &self,
            _camera_id: &str,
        ) -> std::result::Result<bool, CoordinatorError> {
            Ok(true)
        }

        async fn stop_all(&self) {}

        async fn get_runtime_info(&self, _camera_id: &str) -> Option<CameraPipelineRuntimeInfo> {
            None
        }

        async fn list_runtime_infos(&self) -> Vec<CameraPipelineRuntimeInfo> {
            Vec::new()
        }

        async fn has_active_runtime(&self, _camera_id: &str) -> bool {
            false
        }
    }

    async fn seed_test_camera(db: &DatabaseConnection, camera_id: &str, probe_status: &str) {
        let cam = db::entity::camera::ActiveModel {
            camera_id: Set(camera_id.to_string()),
            name: Set(format!("Cam {camera_id}")),
            rtsp_url: Set(format!("rtsp://127.0.0.1/{camera_id}")),
            sub_rtsp_url: Set(String::new()),
            protocol: Set("rtsp".to_string()),
            remark: Set(String::new()),
            last_probe_status: Set(probe_status.to_string()),
            last_probe_error_code: Set(String::new()),
            last_codec: Set("h264".to_string()),
            last_width: Set(1920),
            last_height: Set(1080),
            last_fps: Set(25.0),
            gb28181_device_id: Set(None),
            gb28181_channel_id: Set(None),
            last_probe_at: Set(None),
            last_success_at: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        };
        CameraRepo::insert(db, cam)
            .await
            .expect("insert test camera");
    }

    async fn seed_test_algorithm(db: &DatabaseConnection, algo_id: &str) {
        AlgorithmRepo::upsert_algorithm(
            db,
            UpsertAlgorithmParams {
                algorithm_id: algo_id.to_string(),
                name: format!("Algorithm {algo_id}"),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: String::new(),
                is_builtin: true,
            },
        )
        .await
        .expect("insert test algorithm");
    }

    #[tokio::test]
    async fn test_recover_enabled_tasks_happy_path() {
        let db = init_test_db().await.expect("init db");
        let runtime = MockTaskRuntime::default();

        seed_test_camera(&db, "CAM_HEALTHY", "healthy").await;
        seed_test_algorithm(&db, "general_det").await;

        let _task = TaskRepo::save_task_with_instances(
            &db,
            SaveTaskWithInstancesParams {
                camera_id: "CAM_HEALTHY".to_string(),
                name: "健康任务".to_string(),
                desired_enabled: true,
                rules_json: "[]".to_string(),
                motion_gate_json: r#"{"enabled":true}"#.to_string(),
                status_message: None,
                instances: Some(vec![db::SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_det".to_string(),
                    analysis_fps: 15,
                    params_json: r#"{"threshold":0.5}"#.to_string(),
                    enabled: Some(true),
                }]),
            },
        )
        .await
        .expect("save task");

        let summary = recover_enabled_tasks(&db, &runtime).await.expect("recover");
        assert_eq!(
            summary,
            TaskRecoverySummary {
                attempted: 1,
                recovered: 1,
                deferred: 0,
                failed: 0,
            }
        );
        assert_eq!(runtime.start_count.load(Ordering::Relaxed), 1);

        let latest = TaskRepo::find_by_camera_id(&db, "CAM_HEALTHY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.actual_status, TaskStatus::Running.as_i32());
        assert!(latest.desired_enabled);
        assert!(latest.status_message.contains("冷启动恢复成功"));
    }

    #[tokio::test]
    async fn test_recover_enabled_tasks_defers_unhealthy_camera() {
        let db = init_test_db().await.expect("init db");
        let runtime = MockTaskRuntime::default();

        seed_test_camera(&db, "CAM_OFFLINE", "failed").await;
        seed_test_algorithm(&db, "general_det").await;

        TaskRepo::save_task_with_instances(
            &db,
            SaveTaskWithInstancesParams {
                camera_id: "CAM_OFFLINE".to_string(),
                name: "离线任务".to_string(),
                desired_enabled: true,
                rules_json: "[]".to_string(),
                motion_gate_json: "{}".to_string(),
                status_message: None,
                instances: Some(vec![db::SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_det".to_string(),
                    analysis_fps: 10,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                }]),
            },
        )
        .await
        .expect("save task");

        let summary = recover_enabled_tasks(&db, &runtime).await.expect("recover");
        assert_eq!(
            summary,
            TaskRecoverySummary {
                attempted: 1,
                recovered: 0,
                deferred: 1,
                failed: 0,
            }
        );
        assert_eq!(runtime.start_count.load(Ordering::Relaxed), 0);

        let latest = TaskRepo::find_by_camera_id(&db, "CAM_OFFLINE")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.actual_status, TaskStatus::Reconnecting.as_i32());
        assert!(latest.desired_enabled, "必须保持用户配置的期望启用状态");
        assert!(latest.status_message.contains("未通过健康检查"));
    }

    #[tokio::test]
    async fn test_recover_enabled_tasks_fails_on_missing_camera() {
        let db = init_test_db().await.expect("init db");
        let runtime = MockTaskRuntime::default();

        // 插入有摄像头的任务然后通过原生操作删除摄像头以模拟关联失效
        seed_test_camera(&db, "CAM_GHOST", "healthy").await;
        seed_test_algorithm(&db, "general_det").await;

        TaskRepo::save_task_with_instances(
            &db,
            SaveTaskWithInstancesParams {
                camera_id: "CAM_GHOST".to_string(),
                name: "孤儿任务".to_string(),
                desired_enabled: true,
                rules_json: "[]".to_string(),
                motion_gate_json: "{}".to_string(),
                status_message: None,
                instances: Some(vec![db::SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_det".to_string(),
                    analysis_fps: 10,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                }]),
            },
        )
        .await
        .expect("save task");

        // 在同一个连接批处理中临时关闭外键约束删除摄像头，以构建孤儿任务的防御性分支测试
        db.execute_unprepared(
            "PRAGMA foreign_keys = OFF; DELETE FROM cameras WHERE camera_id = 'CAM_GHOST'; PRAGMA foreign_keys = ON;",
        )
        .await
        .unwrap();

        let summary = recover_enabled_tasks(&db, &runtime).await.expect("recover");
        assert_eq!(
            summary,
            TaskRecoverySummary {
                attempted: 1,
                recovered: 0,
                deferred: 0,
                failed: 1,
            }
        );
        assert_eq!(runtime.start_count.load(Ordering::Relaxed), 0);

        let latest = TaskRepo::find_by_camera_id(&db, "CAM_GHOST")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.actual_status, TaskStatus::Error.as_i32());
        assert!(latest.status_message.contains("摄像头不存在"));
    }

    #[tokio::test]
    async fn test_recover_enabled_tasks_fails_on_empty_instances() {
        let db = init_test_db().await.expect("init db");
        let runtime = MockTaskRuntime::default();

        seed_test_camera(&db, "CAM_NO_INST", "healthy").await;

        TaskRepo::save_task_with_instances(
            &db,
            SaveTaskWithInstancesParams {
                camera_id: "CAM_NO_INST".to_string(),
                name: "空实例任务".to_string(),
                desired_enabled: true,
                rules_json: "[]".to_string(),
                motion_gate_json: "{}".to_string(),
                status_message: None,
                instances: Some(vec![]),
            },
        )
        .await
        .expect("save task");

        let summary = recover_enabled_tasks(&db, &runtime).await.expect("recover");
        assert_eq!(
            summary,
            TaskRecoverySummary {
                attempted: 1,
                recovered: 0,
                deferred: 0,
                failed: 1,
            }
        );

        let latest = TaskRepo::find_by_camera_id(&db, "CAM_NO_INST")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.actual_status, TaskStatus::Error.as_i32());
        assert!(latest.status_message.contains("未绑定任何可运行的算法实例"));
    }
}
