#![allow(clippy::unwrap_used)]

use db::{
    AlgorithmInstanceRepo, AlgorithmRepo, CameraRepo, DbError, SaveTaskAlgorithmInstanceParams,
    SaveTaskWithInstancesParams, TaskRepo, UpsertAlgorithmParams,
};
use sea_orm::Set;

async fn setup_db_camera_and_algos() -> (sea_orm::DatabaseConnection, String) {
    let db = db::init_test_db().await.expect("init test db");
    let cam_id = "CAM-AFF-01".to_string();

    CameraRepo::insert(
        &db,
        db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set(cam_id.clone()),
            name: Set("测试摄像头".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set("rtsp://127.0.0.1:8554/live".to_string()),
            sub_rtsp_url: Set("".to_string()),
            stream_mode: Set("auto".to_string()),
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
            recording_config: Set(String::new()),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        },
    )
    .await
    .expect("insert camera");

    AlgorithmRepo::upsert_algorithm(
        &db,
        UpsertAlgorithmParams {
            algorithm_id: "algo_det".to_string(),
            name: "目标检测".to_string(),
            algorithm_type: "detection".to_string(),
            alarm_type_id: "intrusion".to_string(),
            active_version: "1.0.0".to_string(),
            description: "test".to_string(),
            is_builtin: true,
        },
    )
    .await
    .expect("upsert algo_det");

    AlgorithmRepo::upsert_algorithm(
        &db,
        UpsertAlgorithmParams {
            algorithm_id: "algo_face".to_string(),
            name: "人脸识别".to_string(),
            algorithm_type: "recognition".to_string(),
            alarm_type_id: "face".to_string(),
            active_version: "1.0.0".to_string(),
            description: "test".to_string(),
            is_builtin: true,
        },
    )
    .await
    .expect("upsert algo_face");

    (db, cam_id)
}

/// T04: Stale mark_apply failure/applied rejection
#[tokio::test]
async fn test_stale_mark_apply_rejection() {
    let (db, cam_id) = setup_db_camera_and_algos().await;

    let task = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-001".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: Some(r#"{"mode":"auto","policy":"spread"}"#.to_string()),
            }]),
            expected_revision: None,
            stream_mode: None,
        },
    )
    .await
    .expect("save task");

    let instances = TaskRepo::list_instances_by_task_id(&db, task.id)
        .await
        .expect("list instances");
    assert_eq!(instances[0].desired_revision, 0);

    // 假设配置被更新，desired_revision 递增到 1
    let updated_task = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-001".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 20, // 帧率变动
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: Some(r#"{"mode":"auto","policy":"spread"}"#.to_string()),
            }]),
            expected_revision: Some(task.config_revision),
            stream_mode: None,
        },
    )
    .await
    .expect("save updated task");

    let instances = TaskRepo::list_instances_by_task_id(&db, updated_task.id)
        .await
        .expect("list instances");
    assert_eq!(instances[0].desired_revision, 1);
    assert_eq!(
        instances[0].runtime_apply_state,
        types::InstanceApplyState::Pending.as_i32()
    );

    // 旧版本（revision = 0）的 mark_apply_applied 应该被条件更新忽略，返回 Ok(None)
    let stale_apply = AlgorithmInstanceRepo::mark_apply_applied(&db, "inst-001", 0)
        .await
        .expect("stale mark_apply_applied should not error");
    assert!(
        stale_apply.is_none(),
        "过期的 applied_revision 更新不应影响数据库"
    );

    // 验证状态依旧是 Pending，并没有被修改为 Applied
    let current_inst = AlgorithmInstanceRepo::find_by_instance_id(&db, "inst-001")
        .await
        .expect("find inst")
        .expect("must exist");
    assert_eq!(
        current_inst.runtime_apply_state,
        types::InstanceApplyState::Pending.as_i32()
    );

    // 旧版本（revision = 0）的 mark_apply_failed 同样被忽略，返回 Ok(None)
    let stale_fail = AlgorithmInstanceRepo::mark_apply_failed(&db, "inst-001", 0, "旧错误信息")
        .await
        .expect("stale mark_apply_failed should not error");
    assert!(
        stale_fail.is_none(),
        "过期的 failed_revision 更新不应影响数据库"
    );

    // 正确目标版本（revision = 1）的 mark_apply_applied 成功生效
    let valid_apply = AlgorithmInstanceRepo::mark_apply_applied(&db, "inst-001", 1)
        .await
        .expect("valid mark_apply_applied")
        .expect("should return updated model");
    assert_eq!(valid_apply.applied_revision, 1);
    assert_eq!(
        valid_apply.runtime_apply_state,
        types::InstanceApplyState::Applied.as_i32()
    );
}

/// T05: Task save with atomic stream_mode
#[tokio::test]
async fn test_save_task_with_atomic_stream_mode() {
    let (db, cam_id) = setup_db_camera_and_algos().await;

    // 1. 成功保存：任务创建的同时，原子将 cameras.stream_mode 从 auto 改为 main
    let task = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: None,
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: None,
            }]),
            expected_revision: None,
            stream_mode: Some(types::StreamMode::Main),
        },
    )
    .await
    .expect("save task with stream_mode");

    let camera = CameraRepo::find_by_camera_id(&db, &cam_id)
        .await
        .expect("find camera")
        .expect("camera exists");
    assert_eq!(camera.stream_mode, "main");
    assert_eq!(task.config_revision, 1);

    // 2. 失败保存：传入冲突的 expected_revision，整个事务必须回滚，cameras.stream_mode 不得被篡改为 sub
    let conflict_err = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "冲突任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: None,
            expected_revision: Some(999), // 错误的预期版本
            stream_mode: Some(types::StreamMode::Sub),
        },
    )
    .await;

    assert!(matches!(
        conflict_err,
        Err(DbError::RevisionConflict { .. })
    ));

    // 校验 cameras.stream_mode 依然保持 "main"，原子回滚生效
    let camera_after_fail = CameraRepo::find_by_camera_id(&db, &cam_id)
        .await
        .expect("find camera")
        .expect("camera exists");
    assert_eq!(camera_after_fail.stream_mode, "main");
}

/// T17 / T18: Revision bumping behavior on affinity change vs equivalence
#[tokio::test]
async fn test_affinity_change_and_equivalence_revision_bump() {
    let (db, cam_id) = setup_db_camera_and_algos().await;

    // 初始创建，设置自动 spread 亲和
    let task1 = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-aff-1".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: Some(r#"{"mode":"auto","policy":"spread"}"#.to_string()),
            }]),
            expected_revision: None,
            stream_mode: None,
        },
    )
    .await
    .expect("init task");

    let inst1 = AlgorithmInstanceRepo::find_by_instance_id(&db, "inst-aff-1")
        .await
        .expect("find")
        .expect("must exist");
    assert_eq!(inst1.desired_revision, 0);

    // 1. 等价自动亲和意图提交：policy 为空字符串或 "spread"，不触发 desired_revision 递增
    let task2 = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-aff-1".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: Some(r#"{"mode":"auto","policy":""}"#.to_string()),
            }]),
            expected_revision: Some(task1.config_revision),
            stream_mode: None,
        },
    )
    .await
    .expect("update with equivalent affinity");

    let inst2 = AlgorithmInstanceRepo::find_by_instance_id(&db, "inst-aff-1")
        .await
        .expect("find")
        .expect("must exist");
    assert_eq!(
        inst2.desired_revision, 0,
        "等价亲和意图不应触发 desired_revision 递增"
    );

    // 2. 变更亲和意图（Auto -> Manual 设备 0 核心 1）：必须触发 desired_revision 递增，并置为 Pending
    let _task3 = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "任务1".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-aff-1".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: Some(
                    r#"{"mode":"manual","deviceId":"npu-0","coreIndex":1}"#.to_string(),
                ),
            }]),
            expected_revision: Some(task2.config_revision),
            stream_mode: None,
        },
    )
    .await
    .expect("update with manual affinity");

    let inst3 = AlgorithmInstanceRepo::find_by_instance_id(&db, "inst-aff-1")
        .await
        .expect("find")
        .expect("must exist");
    assert_eq!(
        inst3.desired_revision, 1,
        "变更亲和意图必须触发 desired_revision 递增"
    );
    assert_eq!(
        inst3.runtime_apply_state,
        types::InstanceApplyState::Pending.as_i32()
    );
    assert_eq!(
        inst3.affinity_intent(),
        types::AffinityIntent::Manual {
            device_id: "npu-0".to_string(),
            core_index: 1,
        }
    );
}

/// T22 / T23: Explicit instance_id validation
#[tokio::test]
async fn test_explicit_instance_id_validation() {
    let (db, cam_id) = setup_db_camera_and_algos().await;

    // 1. 同一批次内重复 instanceId 必须被拒绝
    let dup_err = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "重复ID".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![
                SaveTaskAlgorithmInstanceParams {
                    instance_id: Some("same-id".to_string()),
                    algorithm_id: "algo_det".to_string(),
                    analysis_fps: 15,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                    affinity_json: None,
                },
                SaveTaskAlgorithmInstanceParams {
                    instance_id: Some("same-id".to_string()),
                    algorithm_id: "algo_face".to_string(),
                    analysis_fps: 10,
                    params_json: "{}".to_string(),
                    enabled: Some(true),
                    affinity_json: None,
                },
            ]),
            expected_revision: None,
            stream_mode: None,
        },
    )
    .await;

    assert!(matches!(dup_err, Err(DbError::Validation(_))));

    // 2. 正常初始化任务
    let task = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "正常任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("inst-algo-det-01".to_string()),
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: None,
            }]),
            expected_revision: None,
            stream_mode: None,
        },
    )
    .await
    .expect("init task");

    // 3. 提交不匹配库内已分配 instanceId 的修改必须被拒绝
    let mismatch_err = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "正常任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: Some("mismatched-id".to_string()),
                algorithm_id: "algo_det".to_string(), // 算法还是 algo_det，但 instanceId 变了
                analysis_fps: 15,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: None,
            }]),
            expected_revision: Some(task.config_revision),
            stream_mode: None,
        },
    )
    .await;

    assert!(matches!(mismatch_err, Err(DbError::Validation(_))));

    // 4. 省略 instanceId 提交：能正确匹配既有 algorithm_id 并保留原有 instance_id
    let omitted_id_update = TaskRepo::save_task_with_instances(
        &db,
        SaveTaskWithInstancesParams {
            camera_id: cam_id.clone(),
            name: "更新任务".to_string(),
            desired_enabled: true,
            rules_json: "[]".to_string(),
            motion_gate_json: "{}".to_string(),
            status_message: None,
            instances: Some(vec![SaveTaskAlgorithmInstanceParams {
                instance_id: None, // 缺省 instance_id
                algorithm_id: "algo_det".to_string(),
                analysis_fps: 20,
                params_json: "{}".to_string(),
                enabled: Some(true),
                affinity_json: None,
            }]),
            expected_revision: Some(task.config_revision),
            stream_mode: None,
        },
    )
    .await
    .expect("update without explicit instance_id");

    let instances = TaskRepo::list_instances_by_task_id(&db, omitted_id_update.id)
        .await
        .expect("list instances");
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].instance_id, "inst-algo-det-01");
    assert_eq!(instances[0].analysis_fps, 20);
}
