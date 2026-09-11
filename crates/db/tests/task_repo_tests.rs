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
        },
    )
    .await
    .expect("update instance and sync task");

    assert_eq!(updated_instance.analysis_fps, 20);
    assert_eq!(updated_instance.rules_json, updated_rules);
    assert_eq!(updated_instance.motion_gate_json, updated_motion);

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
