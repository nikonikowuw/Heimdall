#![allow(clippy::unwrap_used)]

use rusqlite::Connection;

#[test]
fn test_v5_to_v6_migration_upgrade_and_deduplication() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        "#,
    )
    .expect("enable foreign keys");

    // 1. 逐步执行 V1 到 V5 迁移
    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v2 = include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql");
    let v3 = include_str!("../src/migration/migrations/V3__alarm_status_processing.sql");
    let v4 = include_str!("../src/migration/migrations/V4__add_algorithms_and_instances.sql");
    let v5 = include_str!("../src/migration/migrations/V5__bind_algorithm_to_analysis_tasks.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v2).expect("apply V2");
    conn.execute_batch(v3).expect("apply V3");
    conn.execute_batch(v4).expect("apply V4");
    conn.execute_batch(v5).expect("apply V5");

    // 2. 插入迁移前的测试数据
    // (1) 插入已注册算法
    conn.execute(
        r#"
        INSERT INTO algorithms (
            algorithm_id, name, algorithm_type, alarm_type_id, active_version, description, is_builtin
        ) VALUES (
            'person_det', 'Person Detection', 'det', 'INTRUSION', '1.0.0', 'Valid model', 1
        );
        "#,
        [],
    )
    .expect("insert algorithm");

    // (2) 插入摄像头
    conn.execute(
        r#"
        INSERT INTO cameras (
            camera_id, name, protocol, rtsp_url, sub_rtsp_url, remark,
            last_probe_status, last_probe_error_code, last_codec, last_width, last_height, last_fps
        ) VALUES
        ('CAM_001', 'Cam 1', 'rtsp', 'rtsp://127.0.0.1/1', '', '', 'healthy', '', 'h264', 1920, 1080, 25.0),
        ('CAM_002', 'Cam 2', 'rtsp', 'rtsp://127.0.0.1/2', '', '', 'healthy', '', 'h264', 1920, 1080, 25.0);
        "#,
        [],
    )
    .expect("insert cameras");

    // (3) 插入 V5 格式的任务 (CAM_001)
    conn.execute(
        r#"
        INSERT INTO analysis_tasks (
            id, camera_id, name, desired_enabled, algorithm_id, analysis_fps,
            algo_params_json, rules_json, motion_gate_json, actual_status, status_message
        ) VALUES
        (1, 'CAM_001', 'Task 1', 1, 'person_det', 15, '{"conf":0.6}', '[]', '{}', 2, 'running');
        "#,
        [],
    )
    .expect("insert tasks");

    // (4) 插入旧 algorithm_instances 数据：
    // - CAM_001 上有两个相同 algorithm_id ('person_det') 的实例 (id: 10 与 id: 20)，用于测试按 MIN(id) 幂等去重
    // - CAM_001 上有一个未注册算法的实例 (id: 30)
    conn.execute(
        r#"
        INSERT INTO algorithm_instances (
            id, instance_id, camera_id, algorithm_id, analysis_fps,
            params_json, rules_json, motion_gate_json, enabled, actual_status, status_message
        ) VALUES
        (10, 'inst_old_first', 'CAM_001', 'person_det', 15, '{"conf":0.6}', '[]', '{}', 1, 2, 'ok'),
        (20, 'inst_old_dup', 'CAM_001', 'person_det', 15, '{"conf":0.6}', '[]', '{}', 1, 2, 'dup'),
        (30, 'inst_unregistered', 'CAM_001', 'missing_algo', 5, '{}', '[]', '{}', 1, 2, 'ok');
        "#,
        [],
    )
    .expect("insert old instances");

    // 3. 执行 V6 迁移
    let v6 = include_str!("../src/migration/migrations/V6__bind_algorithm_instances_to_tasks.sql");
    conn.execute_batch(v6).expect("apply V6");

    // 4. 验证迁移结果
    // (1) 验证去重：CAM_001 下的 'person_det' 只保留了 id 最小的 'inst_old_first'
    let mut stmt = conn
        .prepare(
            "SELECT instance_id, task_id, actual_status FROM algorithm_instances WHERE task_id = 1 AND algorithm_id = 'person_det';",
        )
        .expect("prepare select");
    let mut rows = stmt.query([]).expect("query rows");
    let first_row = rows.next().expect("fetch row").expect("must have 1 row");
    let inst_id: String = first_row.get(0).expect("instance_id");
    let task_id: i64 = first_row.get(1).expect("task_id");
    let actual_status: i32 = first_row.get(2).expect("actual_status");
    assert_eq!(inst_id, "inst_old_first");
    assert_eq!(task_id, 1);
    assert_eq!(actual_status, 2);
    assert!(rows.next().expect("no second row").is_none());

    // (2) 验证未注册算法安全标记为 Error (5)
    let (unreg_status, unreg_msg): (i32, String) = conn
        .query_row(
            "SELECT actual_status, status_message FROM algorithm_instances WHERE instance_id = 'inst_unregistered';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query unregistered instance");
    assert_eq!(unreg_status, 5);
    assert!(unreg_msg.contains("算法包数据校验缺失"));

    // (3) 验证 UNIQUE(task_id, algorithm_id) 约束已被强制生效
    let dup_insert = conn.execute(
        r#"
        INSERT INTO algorithm_instances (
            instance_id, task_id, camera_id, algorithm_id, analysis_fps,
            params_json, rules_json, motion_gate_json, enabled, actual_status, status_message
        ) VALUES (
            'inst_conflict', 1, 'CAM_001', 'person_det', 10, '{}', '[]', '{}', 1, 0, ''
        );
        "#,
        [],
    );
    assert!(
        dup_insert.is_err(),
        "必须由 UNIQUE(task_id, algorithm_id) 阻止重复绑定"
    );

    // (4) 验证级联删除：删除 task 1，关联的所有 algorithm_instances 必须被自动物理级联删除
    conn.execute("DELETE FROM analysis_tasks WHERE id = 1;", [])
        .expect("delete task 1");
    let remaining_instances_cam1: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM algorithm_instances WHERE task_id = 1;",
            [],
            |row| row.get(0),
        )
        .expect("count remaining");
    assert_eq!(remaining_instances_cam1, 0);
}
