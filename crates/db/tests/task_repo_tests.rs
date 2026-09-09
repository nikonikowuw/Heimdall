#![allow(clippy::unwrap_used)]

use db::{
    init_test_db, AlgorithmInstanceRepo, AlgorithmRepo, CameraRepo, DbError, SaveTaskParams,
    TaskRepo, UpsertAlgorithmParams,
};
use sea_orm::Set;

async fn setup_test_camera_and_algo(db: &sea_orm::DatabaseConnection) {
    let camera_model = db::entity::camera::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        camera_id: Set("CAM-001".to_string()),
        name: Set("测试摄像头".to_string()),
        protocol: Set("rtsp".to_string()),
        rtsp_url: Set("rtsp://127.0.0.1:8554/live".to_string()),
        sub_rtsp_url: Set("".to_string()),
        remark: Set("".to_string()),
        last_probe_status: Set("healthy".to_string()),
        last_probe_at: Set(None),
        last_probe_error_code: Set("".to_string()),
        last_success_at: Set(None),
        last_codec: Set("h264".to_string()),
        last_width: Set(1920),
        last_height: Set(1080),
        last_fps: Set(25.0),
        gb28181_device_id: Set(None),
        gb28181_channel_id: Set(None),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    };
    CameraRepo::insert(db, camera_model)
        .await
        .expect("insert camera");

    AlgorithmRepo::upsert_algorithm(
        db,
        UpsertAlgorithmParams {
            algorithm_id: "general_detection".to_string(),
            name: "通用目标检测".to_string(),
            algorithm_type: "detection".to_string(),
            alarm_type_id: "PERSON_INTRUSION".to_string(),
            active_version: "1.0.0".to_string(),
            description: "测试算法包".to_string(),
            is_builtin: true,
        },
    )
    .await
    .expect("upsert algo");
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
    assert_eq!(saved.algorithm_id, "general_detection");
    assert_eq!(saved.analysis_fps, 15);
    assert_eq!(saved.algo_params_json, r#"{"confidence":0.5}"#);

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
    assert_eq!(updated.analysis_fps, 20);

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
    assert!(matches!(err_fps, Err(DbError::Validation(_))));

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
    assert!(matches!(err_json, Err(DbError::Validation(_))));

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
    assert!(matches!(err_arr, Err(DbError::Validation(_))));

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
