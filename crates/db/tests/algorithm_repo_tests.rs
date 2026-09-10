#![allow(clippy::unwrap_used)]

use db::{
    init_test_db, AlgorithmInstanceRepo, AlgorithmRepo, CameraRepo, CreateInstanceParams, DbError,
    TaskRepo, UpdateInstanceParams, UpsertAlgorithmParams, UpsertVersionParams,
};

#[tokio::test]
async fn test_algorithm_and_version_repository_lifecycle() {
    let db = init_test_db().await.expect("init test db");

    // 1. 插入算法主表
    let algo_params = UpsertAlgorithmParams {
        algorithm_id: "yolo_v8".to_string(),
        name: "YOLOv8 Detection".to_string(),
        algorithm_type: "object_detection".to_string(),
        alarm_type_id: "object_detect".to_string(),
        active_version: "1.0.0".to_string(),
        description: "YOLOv8 object detector".to_string(),
        is_builtin: false,
    };
    let algo = AlgorithmRepo::upsert_algorithm(&db, algo_params)
        .await
        .expect("upsert algo");
    assert_eq!(algo.algorithm_id, "yolo_v8");
    assert_eq!(algo.active_version, "1.0.0");

    // 2. 插入版本 v1.0.0 和 v1.1.0
    let ver1_params = UpsertVersionParams {
        algorithm_id: "yolo_v8".to_string(),
        version: "1.0.0".to_string(),
        platform_id: "macos-arm64-coreml".to_string(),
        min_adapter_version: "1.0.0".to_string(),
        package_root: "var/packages/yolo_v8/1.0.0".to_string(),
        fps_tiers: r#"[{"fps":15,"units":100}]"#.to_string(),
        config_schema: r#"{"properties":{"threshold":{"type":"number"}}}"#.to_string(),
        manifest_raw: "{}".to_string(),
        package_size_bytes: 10485760,
        is_active: true,
        is_builtin: false,
    };
    let ver1 = AlgorithmRepo::upsert_version(&db, ver1_params)
        .await
        .expect("upsert v1");
    assert_eq!(ver1.version, "1.0.0");
    assert!(ver1.is_active);

    let ver2_params = UpsertVersionParams {
        algorithm_id: "yolo_v8".to_string(),
        version: "1.1.0".to_string(),
        platform_id: "macos-arm64-coreml".to_string(),
        min_adapter_version: "1.0.0".to_string(),
        package_root: "var/packages/yolo_v8/1.1.0".to_string(),
        fps_tiers: r#"[{"fps":30,"units":200}]"#.to_string(),
        config_schema: r#"{"properties":{"threshold":{"type":"number"}}}"#.to_string(),
        manifest_raw: "{}".to_string(),
        package_size_bytes: 12485760,
        is_active: false,
        is_builtin: false,
    };
    let ver2 = AlgorithmRepo::upsert_version(&db, ver2_params)
        .await
        .expect("upsert v2");
    assert_eq!(ver2.version, "1.1.0");
    assert!(!ver2.is_active);

    // 3. 统计查询
    let stats = AlgorithmRepo::stats(&db).await.expect("stats");
    assert_eq!(stats.total_algorithms, 1);
    assert_eq!(stats.total_active_versions, 1);
    assert_eq!(stats.custom_algorithms, 1);
    assert_eq!(stats.builtin_algorithms, 0);

    // 4. 版本激活切换
    AlgorithmRepo::activate_version(&db, "yolo_v8", "1.1.0", None)
        .await
        .expect("activate v1.1.0");
    let updated_algo = AlgorithmRepo::find_by_algorithm_id(&db, "yolo_v8")
        .await
        .expect("find algo")
        .expect("algo exists");
    assert_eq!(updated_algo.active_version, "1.1.0");

    let v1_fetch = AlgorithmRepo::find_version(&db, "yolo_v8", "1.0.0", None)
        .await
        .expect("fetch v1")
        .unwrap();
    assert!(!v1_fetch.is_active);

    let v2_fetch = AlgorithmRepo::find_version(&db, "yolo_v8", "1.1.0", None)
        .await
        .expect("fetch v2")
        .unwrap();
    assert!(v2_fetch.is_active);

    // 5. 保护校验：内置算法禁止删除
    let builtin_params = UpsertAlgorithmParams {
        algorithm_id: "builtin_det".to_string(),
        name: "Builtin Det".to_string(),
        algorithm_type: "object_detection".to_string(),
        alarm_type_id: "object_detect".to_string(),
        active_version: "1.0.0".to_string(),
        description: "Builtin model".to_string(),
        is_builtin: true,
    };
    AlgorithmRepo::upsert_algorithm(&db, builtin_params)
        .await
        .expect("upsert builtin algo");

    let builtin_ver = UpsertVersionParams {
        algorithm_id: "builtin_det".to_string(),
        version: "1.0.0".to_string(),
        platform_id: "macos-arm64-coreml".to_string(),
        min_adapter_version: "1.0.0".to_string(),
        package_root: "algo-packages/builtin_det".to_string(),
        fps_tiers: "[]".to_string(),
        config_schema: "{}".to_string(),
        manifest_raw: "{}".to_string(),
        package_size_bytes: 5000000,
        is_active: true,
        is_builtin: true,
    };
    AlgorithmRepo::upsert_version(&db, builtin_ver)
        .await
        .expect("upsert builtin ver");

    let uninstall_err = AlgorithmRepo::uninstall_version(&db, "builtin_det", "1.0.0", None).await;
    match uninstall_err {
        Err(DbError::BuiltinAlgoProtected(_)) => {}
        other => panic!("expected BuiltinAlgoProtected, got {other:?}"),
    }

    // 6. 实例绑定与使用中防卸载保护
    let cam = db::entity::camera::ActiveModel {
        camera_id: sea_orm::ActiveValue::Set("cam_01".to_string()),
        name: sea_orm::ActiveValue::Set("Gate Cam".to_string()),
        rtsp_url: sea_orm::ActiveValue::Set("rtsp://localhost/cam".to_string()),
        ..Default::default()
    };
    CameraRepo::insert(&db, cam).await.expect("insert camera");

    // 创建任务行，满足 algorithm_instances.task_id NOT NULL + UNIQUE(task_id, algorithm_id)
    let task = TaskRepo::save_task_with_instances(
        &db,
        db::SaveTaskWithInstancesParams {
            camera_id: "cam_01".into(),
            name: "测试任务".into(),
            desired_enabled: true,
            rules_json: "[]".into(),
            motion_gate_json: "{}".into(),
            status_message: None,
            instances: Some(vec![]),
        },
    )
    .await
    .expect("create task for camera");

    // 创建启用的算法实例，绑定 yolo_v8
    let inst_params = CreateInstanceParams {
        task_id: task.id,
        instance_id: "inst_001".to_string(),
        camera_id: "cam_01".to_string(),
        algorithm_id: "yolo_v8".to_string(),
        analysis_fps: 15,
        params_json: r#"{"confidenceThreshold":0.5}"#.to_string(),
        rules_json: "[]".to_string(),
        motion_gate_json: "{}".to_string(),
        enabled: true,
    };
    let inst = AlgorithmInstanceRepo::create(&db, inst_params)
        .await
        .expect("create instance");
    assert_eq!(inst.instance_id, "inst_001");

    let active_count = AlgorithmRepo::count_active_instances(&db, "yolo_v8")
        .await
        .expect("count active");
    assert_eq!(active_count, 1);

    // 尝试删除在用版本的算法 -> 应报错 AlgoInUse
    let in_use_err = AlgorithmRepo::uninstall_version(&db, "yolo_v8", "1.0.0", None).await;
    match in_use_err {
        Err(DbError::AlgoInUse(_)) => {}
        other => panic!("expected AlgoInUse, got {other:?}"),
    }

    // 停用实例
    AlgorithmInstanceRepo::set_enabled(&db, "inst_001", false)
        .await
        .expect("disable inst");
    assert_eq!(
        AlgorithmRepo::count_active_instances(&db, "yolo_v8")
            .await
            .unwrap(),
        0
    );

    // 此时允许卸载非活跃版本 1.0.0
    let pkg_root = AlgorithmRepo::uninstall_version(&db, "yolo_v8", "1.0.0", None)
        .await
        .expect("uninstall v1.0.0");
    assert_eq!(pkg_root, "var/packages/yolo_v8/1.0.0");

    let remaining_vers = AlgorithmRepo::list_versions_by_algorithm_id(&db, "yolo_v8")
        .await
        .expect("list vers");
    assert_eq!(remaining_vers.len(), 1);
    assert_eq!(remaining_vers[0].version, "1.1.0");

    // 更新实例参数
    let updated_inst = AlgorithmInstanceRepo::update(
        &db,
        "inst_001",
        UpdateInstanceParams {
            analysis_fps: Some(25),
            params_json: None,
            rules_json: None,
            motion_gate_json: None,
            enabled: Some(false),
        },
    )
    .await
    .expect("update inst");
    assert_eq!(updated_inst.analysis_fps, 25);

    // 删除实例
    let deleted_insts =
        AlgorithmInstanceRepo::delete_by_task_id_and_instance_id(&db, task.id, "inst_001")
            .await
            .expect("delete inst");
    assert_eq!(deleted_insts, 1);
}
