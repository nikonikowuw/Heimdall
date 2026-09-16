#![allow(clippy::unwrap_used)]

use db::{
    init_test_db, AlgorithmInstanceRepo, AlgorithmRepo, CameraRepo, DbError,
    SaveTaskAlgorithmInstanceParams, SaveTaskParams, SaveTaskWithInstancesParams, TaskRepo,
    UpsertAlgorithmParams,
};

async fn setup_test_camera_and_algo(db: &db::DatabaseConnection) {
    let now = chrono::Utc::now();

    // 预置摄像头
    let camera_model = db::entity::camera::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        camera_id: sea_orm::ActiveValue::Set("CAM-001".to_string()),
        name: sea_orm::ActiveValue::Set("测试摄像头".to_string()),
        protocol: sea_orm::ActiveValue::Set("rtsp".to_string()),
        rtsp_url: sea_orm::ActiveValue::Set("rtsp://127.0.0.1:8554/live".to_string()),
        sub_rtsp_url: sea_orm::ActiveValue::Set("".to_string()),
        stream_mode: sea_orm::ActiveValue::Set("auto".to_string()),
        remark: sea_orm::ActiveValue::Set("".to_string()),
        last_probe_status: sea_orm::ActiveValue::Set("healthy".to_string()),
        last_probe_at: sea_orm::ActiveValue::Set(None),
        last_probe_error_code: sea_orm::ActiveValue::Set("".to_string()),
        last_success_at: sea_orm::ActiveValue::Set(None),
        last_codec: sea_orm::ActiveValue::Set("h264".to_string()),
        last_width: sea_orm::ActiveValue::Set(1920),
        last_height: sea_orm::ActiveValue::Set(1080),
        last_fps: sea_orm::ActiveValue::Set(25.0),
        gb28181_device_id: sea_orm::ActiveValue::Set(None),
        gb28181_channel_id: sea_orm::ActiveValue::Set(None),
        created_at: sea_orm::ActiveValue::Set(now),
        updated_at: sea_orm::ActiveValue::Set(now),
    };
    CameraRepo::insert(db, camera_model)
        .await
        .expect("insert camera");

    // 预置算法包
    AlgorithmRepo::upsert_algorithm(
        db,
        UpsertAlgorithmParams {
            algorithm_id: "general_detection".to_string(),
            name: "通用检测".to_string(),
            algorithm_type: "detection".to_string(),
            alarm_type_id: "INTRUSION".to_string(),
            active_version: "1.0.0".to_string(),
            description: "通用检测算法".to_string(),
            is_builtin: true,
        },
    )
    .await
    .expect("insert algo");

    AlgorithmRepo::upsert_algorithm(
        db,
        UpsertAlgorithmParams {
            algorithm_id: "face_recognition".to_string(),
            name: "人脸识别".to_string(),
            algorithm_type: "recognition".to_string(),
            alarm_type_id: "FACE".to_string(),
            active_version: "1.0.0".to_string(),
            description: "人脸识别算法".to_string(),
            is_builtin: true,
        },
    )
    .await
    .expect("insert face algo");
}

#[tokio::test]
async fn test_save_task_and_sync_instance_happy_path() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    // 1. 保存任务并绑定算法
    let saved = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "周界防范".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 15,
            algo_params_json: r#"{"confidence":0.5}"#.to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: r#"{"enabled":true}"#.to_string(),
        },
    )
    .await
    .expect("save task and sync instance");

    assert_eq!(saved.camera_id, "CAM-001");
    assert_eq!(saved.name, "周界防范");
    assert!(saved.desired_enabled);

    // 2. 验证自动创建了主算法实例
    let instances = AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .expect("list instances");
    assert_eq!(instances.len(), 1);
    let inst = &instances[0];
    assert_eq!(inst.camera_id, "CAM-001");
    assert_eq!(inst.algorithm_id, "general_detection");
    assert_eq!(inst.analysis_fps, 15);
    assert_eq!(inst.params_json, r#"{"confidence":0.5}"#);
    assert!(inst.enabled);

    // 3. 更新任务配置（修改 FPS 与参数），断言实例同步更新
    let updated = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "周界防范-更新".to_string(),
            desired_enabled: false,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 20,
            algo_params_json: r#"{"confidence":0.8}"#.to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: r#"{"enabled":false}"#.to_string(),
        },
    )
    .await
    .expect("update task");

    assert_eq!(updated.name, "周界防范-更新");
    assert!(!updated.desired_enabled);

    let instances_after = AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .expect("list instances after update");
    assert_eq!(instances_after.len(), 1);
    assert_eq!(instances_after[0].instance_id, inst.instance_id); // 同一实例
    assert_eq!(instances_after[0].analysis_fps, 20);
    assert_eq!(instances_after[0].params_json, r#"{"confidence":0.8}"#);
    assert!(!instances_after[0].enabled);
}

#[tokio::test]
async fn test_save_task_with_multiple_instances() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    // 一次性挂载 2 个算法实例
    let saved = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "多算法布防".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: r#"{"enabled":true}"#.to_string(),
            status_message: None,
            instances: Some(vec![
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_detection".to_string(),
                    analysis_fps: 15,
                    params_json: r#"{"confidence":0.5}"#.to_string(),
                    enabled: Some(true),
                },
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "face_recognition".to_string(),
                    analysis_fps: 5,
                    params_json: "{}".to_string(),
                    enabled: Some(false),
                },
            ]),
            expected_revision: None,
        },
    )
    .await
    .expect("save task with multiple instances");

    assert_eq!(saved.camera_id, "CAM-001");
    assert!(saved.desired_enabled);

    let instances = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances by task id");
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].algorithm_id, "general_detection");
    assert_eq!(instances[0].analysis_fps, 15);
    assert!(instances[0].enabled);
    assert_eq!(instances[1].algorithm_id, "face_recognition");
    assert_eq!(instances[1].analysis_fps, 5);
    assert!(!instances[1].enabled);

    // 重复算法 ID 应该被拒绝
    let err_dup = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "重复算法".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_detection".to_string(),
                    analysis_fps: 10,
                    params_json: "{}".to_string(),
                    enabled: None,
                },
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_detection".to_string(),
                    analysis_fps: 20,
                    params_json: "{}".to_string(),
                    enabled: None,
                },
            ]),
            expected_revision: None,
        },
    )
    .await;
    assert!(matches!(
        err_dup,
        Err(DbError::Type(types::TypeError::DuplicateAlgorithmId { .. }))
    ));
}

#[tokio::test]
async fn test_save_task_validation_errors() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    // 1. 负数 FPS
    let err_fps = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "非法FPS".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: -1,
            algo_params_json: "{}".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await;
    assert!(matches!(
        err_fps,
        Err(DbError::Type(types::TypeError::InvalidAnalysisFps { .. }))
            | Err(DbError::Validation(_))
    ));

    // 2. 非法 algo_params_json（不是有效 JSON）
    let err_json = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "非法JSON".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 10,
            algo_params_json: "{invalid json".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await;
    assert!(matches!(
        err_json,
        Err(DbError::Json(_)) | Err(DbError::Validation(_))
    ));

    // 3. 非法 algo_params_json（是 JSON Array 而不是 Object）
    let err_arr = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "数组参数".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 10,
            algo_params_json: "[1, 2, 3]".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await;
    assert!(matches!(
        err_arr,
        Err(DbError::Type(types::TypeError::InvalidAlgoParams { .. }))
            | Err(DbError::Validation(_))
    ));

    // 4. 不存在的 algorithm_id（事务回滚，无残留）
    let err_algo = TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "不存在的算法".to_string(),
            desired_enabled: true,
            algorithm_id: "non_existent_algorithm_xyz".to_string(),
            analysis_fps: 10,
            algo_params_json: "{}".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await;
    assert!(matches!(
        err_algo,
        Err(DbError::NotFound {
            entity: "algorithm",
            ..
        })
    ));

    // 断言数据库无脏数据
    let task = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find task");
    assert!(task.is_none());
    let instances = AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .expect("list inst");
    assert!(instances.is_empty());
}

#[tokio::test]
async fn test_update_status_dual_sync() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    // 创建初始任务
    TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "状态测试".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 10,
            algo_params_json: "{}".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await
    .expect("save task");

    // 更新为 Running (2)
    TaskRepo::update_status(
        &db,
        "CAM-001",
        types::TaskStatus::Running as i32,
        "Pump running smoothly",
    )
    .await
    .expect("update status to running");

    let task = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.actual_status, 2);
    assert_eq!(task.status_message, "Pump running smoothly");

    let instances = AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .unwrap();
    assert_eq!(instances[0].actual_status, 2);
    assert_eq!(instances[0].status_message, "Pump running smoothly");

    // 更新为 Error (5)
    TaskRepo::update_status(
        &db,
        "CAM-001",
        types::TaskStatus::Error as i32,
        "Decoder failed",
    )
    .await
    .expect("update status to error");

    let task_err = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task_err.actual_status, 5);
    assert_eq!(task_err.status_message, "Decoder failed");

    let instances_err = AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .unwrap();
    assert_eq!(instances_err[0].actual_status, 5);
    assert_eq!(instances_err[0].status_message, "Decoder failed");
}

#[tokio::test]
async fn test_delete_task_and_instance_cascade() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    TaskRepo::save_task_and_sync_instance(
        &db,
        SaveTaskParams {
            camera_id: "CAM-001".to_string(),
            name: "待删除任务".to_string(),
            desired_enabled: true,
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 10,
            algo_params_json: "{}".to_string(),
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
        },
    )
    .await
    .expect("save task");

    assert!(TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .is_some());
    assert!(!AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .is_empty());

    let deleted = TaskRepo::delete_task_and_instance(&db, "CAM-001")
        .await
        .expect("delete");
    assert_eq!(deleted, 1);

    // 两侧均已被物理删除
    assert!(TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .is_none());
    assert!(AlgorithmInstanceRepo::list_by_camera_id(&db, "CAM-001")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_update_task_runtime_state_granular() {
    let db = init_test_db().await.expect("init test db");
    setup_test_camera_and_algo(&db).await;

    let saved = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "细粒度状态测试".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "general_detection".to_string(),
                    analysis_fps: 15,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                },
                SaveTaskAlgorithmInstanceParams {
                    algorithm_id: "face_recognition".to_string(),
                    analysis_fps: 10,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                },
            ]),
            expected_revision: None,
        },
    )
    .await
    .expect("save task");

    let instances = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances");
    assert_eq!(instances.len(), 2);

    // 细粒度更新：实例 1 Running，实例 2 Error
    let updates = vec![
        db::TaskInstanceStateUpdate {
            instance_id: instances[0].instance_id.clone(),
            actual_status: types::TaskStatus::Running,
            status_message: "正常运行中".to_string(),
        },
        db::TaskInstanceStateUpdate {
            instance_id: instances[1].instance_id.clone(),
            actual_status: types::TaskStatus::Error,
            status_message: "模型加载失败".to_string(),
        },
    ];

    let task = TaskRepo::update_task_runtime_state(
        &db,
        "CAM-001".to_string(),
        types::TaskStatus::Error,
        "存在异常算法实例".to_string(),
        updates,
    )
    .await
    .expect("update task runtime state");

    assert_eq!(task.actual_status, types::TaskStatus::Error.as_i32());
    assert_eq!(task.status_message, "存在异常算法实例");

    let instances_after = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances after");
    assert_eq!(
        instances_after[0].actual_status,
        types::TaskStatus::Running.as_i32()
    );
    assert_eq!(instances_after[0].status_message, "正常运行中");
    assert_eq!(
        instances_after[1].actual_status,
        types::TaskStatus::Error.as_i32()
    );
    assert_eq!(instances_after[1].status_message, "模型加载失败");
}

#[tokio::test]
async fn test_update_instance_and_sync_task_propagates_rules_and_motion_gate() {
    let db = init_test_db().await.expect("init db");
    setup_test_camera_and_algo(&db).await;

    let saved = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "初始任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: r#"{"enabled":false}"#.to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                algorithm_id: "general_detection".to_string(),
                analysis_fps: 10,
                params_json: "{}".to_string(),
                enabled: Some(true),
            }]),
            expected_revision: None,
        },
    )
    .await
    .expect("save task");

    let instances = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances");
    assert_eq!(instances.len(), 1);
    let instance_id = &instances[0].instance_id;

    // 更新实例时同步更新 rules_json 与 motion_gate_json
    let updated_rules = r#"[{"role":"line","points":[{"x":0.0,"y":0.5},{"x":1.0,"y":0.5}]}]"#;
    let updated_motion = r#"{"enabled":true,"threshold":30}"#;
    let updated_instance = TaskRepo::update_instance_and_sync_task(
        &db,
        instance_id,
        db::UpdateTaskInstanceParams {
            analysis_fps: Some(20),
            params_json: Some(r#"{"confidence":0.7}"#.to_string()),
            rules_json: Some(updated_rules.to_string()),
            motion_gate_json: Some(updated_motion.to_string()),
            enabled: Some(true),
            bump_revision: true,
        },
    )
    .await
    .expect("update instance and sync task");

    assert_eq!(updated_instance.analysis_fps, 20);
    assert_eq!(updated_instance.rules_json, updated_rules);
    assert_eq!(updated_instance.motion_gate_json, updated_motion);
    // 期望配置变更必须推进代际并标记为等待运行时收敛，不能直接伪装成已生效。
    assert_eq!(updated_instance.desired_revision, 1);
    assert_eq!(updated_instance.applied_revision, 0);
    assert_eq!(
        updated_instance.runtime_apply_state,
        types::InstanceApplyState::Pending.as_i32()
    );
    assert_eq!(
        AlgorithmInstanceRepo::list_unapplied(&db)
            .await
            .expect("list unapplied")
            .len(),
        1,
        "期望配置未收敛的实例必须出现在恢复列表中"
    );

    // 运行时确认收敛后，期望与实际代际对齐并回到 applied。
    let applied = AlgorithmInstanceRepo::mark_apply_applied(&db, instance_id, 1)
        .await
        .expect("mark applied")
        .expect("instance exists");
    assert_eq!(applied.applied_revision, 1);
    assert_eq!(
        applied.runtime_apply_state,
        types::InstanceApplyState::Applied.as_i32()
    );
    assert!(AlgorithmInstanceRepo::list_unapplied(&db)
        .await
        .expect("list unapplied")
        .is_empty());

    // 验证父任务的 rules_json 与 motion_gate_json 也已原子同步
    let parent_task = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("query task")
        .expect("task exists");
    assert_eq!(parent_task.rules_json, updated_rules);
    assert_eq!(parent_task.motion_gate_json, updated_motion);
}

#[tokio::test]
async fn test_camera_repo_update_stream_mode() {
    let db = init_test_db().await.expect("init db");
    setup_test_camera_and_algo(&db).await;

    // 初始 stream_mode 为 auto
    let initial = CameraRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find camera")
        .expect("camera exists");
    assert_eq!(initial.stream_mode, "auto");

    // 更新为 main
    let updated = CameraRepo::update_stream_mode(&db, "CAM-001", "main")
        .await
        .expect("update stream mode")
        .expect("camera updated");
    assert_eq!(updated.stream_mode, "main");

    // 再次查询校验持久化结果
    let reloaded = CameraRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find camera")
        .expect("camera exists");
    assert_eq!(reloaded.stream_mode, "main");

    // 重复更新相同值应幂等安全返回
    let no_op = CameraRepo::update_stream_mode(&db, "CAM-001", "main")
        .await
        .expect("no-op update")
        .expect("camera exists");
    assert_eq!(no_op.stream_mode, "main");
}

/// 整体下发写入必须携带快照版本：不匹配即拒绝，避免旧快照覆盖并发会话的提交。
#[tokio::test]
async fn test_save_task_with_instances_enforces_config_revision() {
    let db = init_test_db().await.expect("init db");
    setup_test_camera_and_algo(&db).await;

    let make_params =
        |expected_revision: Option<i64>, name: &str, fps: i32| SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: name.to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: r#"{"enabled":true}"#.to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                algorithm_id: "general_detection".to_string(),
                analysis_fps: fps,
                params_json: "{}".to_string(),
                enabled: Some(true),
            }]),
            expected_revision,
        };

    // 首次创建：无快照可校验，写入后版本号为 1
    let created = TaskRepo::save_task_with_instances(&db, make_params(None, "初始任务", 10))
        .await
        .expect("create task");
    assert_eq!(created.config_revision, 1);

    // 携带当前版本号：写入成功并递增
    let updated = TaskRepo::save_task_with_instances(&db, make_params(Some(1), "会话A", 15))
        .await
        .expect("update with current revision");
    assert_eq!(updated.config_revision, 2);
    assert_eq!(updated.name, "会话A");

    // 陈旧版本号（会话B仍持有 1）：拒绝写入，库中内容保持不变
    let conflict = TaskRepo::save_task_with_instances(&db, make_params(Some(1), "会话B", 30))
        .await
        .expect_err("stale revision must be rejected");
    match conflict {
        DbError::RevisionConflict { expected, actual } => {
            assert_eq!(expected, 1);
            assert_eq!(actual, 2);
        }
        other => panic!("期望 RevisionConflict，实际 {other:?}"),
    }

    let after_conflict = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find task")
        .expect("task exists");
    assert_eq!(after_conflict.name, "会话A");
    assert_eq!(after_conflict.config_revision, 2);
    let instances = TaskRepo::list_instances_by_task_id(&db, after_conflict.id)
        .await
        .expect("list instances");
    assert_eq!(instances[0].analysis_fps, 15, "冲突请求不得写入实例配置");

    // 客户端持有版本号但任务已被删除：同样视为冲突（actual = 0 表示任务不存在），
    // 否则会静默把别人的删除操作反转成一次无版本约束的新建。
    TaskRepo::delete_by_camera_id(&db, "CAM-001")
        .await
        .expect("delete task");
    let deleted = TaskRepo::save_task_with_instances(&db, make_params(Some(2), "删除后写入", 10))
        .await
        .expect_err("deleted task with revision must conflict");
    match deleted {
        DbError::RevisionConflict { expected, actual } => {
            assert_eq!(expected, 2);
            assert_eq!(actual, 0);
        }
        other => panic!("期望 RevisionConflict，实际 {other:?}"),
    }

    // 版本 0 是「读取时该通道还没有任务」的乐观断言：任务确实不存在时，创建就是期望结果
    let fresh = TaskRepo::save_task_with_instances(&db, make_params(Some(0), "快速创建", 10))
        .await
        .expect("expected 0 on absent task is a creation");
    assert_eq!(fresh.config_revision, 1);

    // 同一个断言落到已存在的任务上必须失败，否则并发创建会静默覆盖对方配置
    let race = TaskRepo::save_task_with_instances(&db, make_params(Some(0), "并发创建", 30))
        .await
        .expect_err("expected 0 on existing task must conflict");
    match race {
        DbError::RevisionConflict { expected, actual } => {
            assert_eq!(expected, 0);
            assert_eq!(actual, 1);
        }
        other => panic!("期望 RevisionConflict，实际 {other:?}"),
    }
    let after_race = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find task")
        .expect("task exists");
    assert_eq!(after_race.name, "快速创建");

    // 不带版本号仍可写入（脚本与旧客户端兼容路径），版本号照常推进
    let no_check = TaskRepo::save_task_with_instances(&db, make_params(None, "脚本更新", 10))
        .await
        .expect("write without revision");
    assert_eq!(no_check.config_revision, 2);
}

/// 运行时状态回写不得推进配置版本号，否则轮询会作废在途的配置保存。
#[tokio::test]
async fn test_runtime_state_updates_do_not_bump_config_revision() {
    let db = init_test_db().await.expect("init db");
    setup_test_camera_and_algo(&db).await;

    let saved = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                algorithm_id: "general_detection".to_string(),
                analysis_fps: 10,
                params_json: "{}".to_string(),
                enabled: Some(true),
            }]),
            expected_revision: None,
        },
    )
    .await
    .expect("save task");
    assert_eq!(saved.config_revision, 1);
    let instance_id = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances")[0]
        .instance_id
        .clone();

    TaskRepo::update_task_runtime_state(
        &db,
        "CAM-001".to_string(),
        types::TaskStatus::Running,
        "运行中".to_string(),
        Vec::new(),
    )
    .await
    .expect("update runtime state");
    AlgorithmInstanceRepo::mark_apply_applied(&db, &instance_id, saved.config_revision)
        .await
        .expect("mark applied");

    let after = TaskRepo::find_by_camera_id(&db, "CAM-001")
        .await
        .expect("find task")
        .expect("task exists");
    assert_eq!(
        after.config_revision, 1,
        "运行时簿记不得推进配置版本号，否则客户端在途保存会被误判为冲突"
    );
}

/// 布防开关是状态动词：只写布防意图，不改动名称、规则、门控与实例集合，
/// 但必须推进配置版本号，否则基于旧快照的整体下发会把开关状态静默改回去。
#[tokio::test]
async fn test_set_task_enabled_only_toggles_intent() {
    let db = init_test_db().await.expect("init db");
    setup_test_camera_and_algo(&db).await;

    let saved = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: "CAM-001".to_string(),
            name: "周界防护".to_string(),
            desired_enabled: false,
            rules_json: r#"[{"role":"line","points":[{"x":0.1,"y":0.5},{"x":0.9,"y":0.5}]}]"#
                .to_string(),
            motion_gate_json: r#"{"enabled":true,"threshold":30}"#.to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                algorithm_id: "general_detection".to_string(),
                analysis_fps: 15,
                params_json: r#"{"confidence":0.6}"#.to_string(),
                enabled: Some(true),
            }]),
            expected_revision: None,
        },
    )
    .await
    .expect("save task");
    assert_eq!(saved.config_revision, 1);
    let instance_before = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances")
        .remove(0);

    // 布防：只翻转意图，其余配置逐字节保持
    let armed = TaskRepo::set_task_enabled(&db, "CAM-001", true)
        .await
        .expect("arm task");
    assert!(armed.desired_enabled);
    assert_eq!(armed.config_revision, 2);
    assert_eq!(armed.name, saved.name);
    assert_eq!(armed.rules_json, saved.rules_json);
    assert_eq!(armed.motion_gate_json, saved.motion_gate_json);

    let instance_after = TaskRepo::list_instances_by_task_id(&db, saved.id)
        .await
        .expect("list instances")
        .remove(0);
    assert_eq!(instance_after.params_json, instance_before.params_json);
    assert_eq!(instance_after.analysis_fps, instance_before.analysis_fps);
    assert_eq!(instance_after.enabled, instance_before.enabled);
    assert_eq!(
        instance_after.desired_revision, instance_before.desired_revision,
        "状态动词不得制造需要运行时收敛的新代际"
    );

    // 撤防：意图变化 + 视为停机，版本号继续推进
    let disarmed = TaskRepo::set_task_enabled(&db, "CAM-001", false)
        .await
        .expect("disarm task");
    assert!(!disarmed.desired_enabled);
    assert_eq!(disarmed.config_revision, 3);
    assert_eq!(disarmed.actual_status, types::TaskStatus::Stopped.as_i32());

    // 任务不存在：状态动词不隐式创建任务，创建仍由整体下发负责
    let missing = TaskRepo::set_task_enabled(&db, "CAM-UNKNOWN", true).await;
    assert!(matches!(missing, Err(DbError::NotFound { .. })));
}
