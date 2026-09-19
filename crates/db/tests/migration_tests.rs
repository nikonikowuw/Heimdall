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

#[test]
fn test_v12_migration_backfills_recognition_field_image_path() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v2 = include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql");
    let v12 = include_str!("../src/migration/migrations/V12__recognition_field_image_path.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v2).expect("apply V2");
    conn.execute(
        r#"
        INSERT INTO capture_records (
            capture_id, camera_id, track_id, target_label, confidence, quality_score,
            bbox_json, image_id, image_rel_path, crop_image_id, crop_image_rel_path,
            captured_at
        ) VALUES (
            'capture_v12', 'CAM_V12', 7, 'person', 0.95, 0.88,
            '[]', 'image_v12', 'CAM_V12/image_v12.jpg', 'crop_v12', 'CAM_V12/crop_v12.jpg',
            CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert capture before migration");
    conn.execute(
        r#"
        INSERT INTO recognition_records (
            recognition_id, camera_id, gallery_id, subject_id, subject_name, similarity,
            field_crop_path, registered_photo_path, recognized_at
        ) VALUES (
            'recognition_v12', 'CAM_V12', 'default', 'subject_v12', 'V12', 0.9,
            'CAM_V12/crop_v12.jpg', 'galleries/subject_v12.jpg', CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert recognition before migration");

    conn.execute_batch(v12).expect("apply V12");

    let field_image_path: String = conn
        .query_row(
            "SELECT field_image_path FROM recognition_records WHERE recognition_id = 'recognition_v12';",
            [],
            |row| row.get(0),
        )
        .expect("query backfilled field image path");
    assert_eq!(field_image_path, "CAM_V12/image_v12.jpg");
}

#[test]
fn test_v13_migration_backfills_recognition_field_bbox_json() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v2 = include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql");
    let v12 = include_str!("../src/migration/migrations/V12__recognition_field_image_path.sql");
    let v13 = include_str!("../src/migration/migrations/V13__recognition_field_bbox_json.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v2).expect("apply V2");
    conn.execute(
        r#"
        INSERT INTO capture_records (
            capture_id, camera_id, track_id, target_label, confidence, quality_score,
            bbox_json, image_id, image_rel_path, crop_image_id, crop_image_rel_path,
            captured_at
        ) VALUES (
            'capture_bbox_v13', 'CAM_V13', 8, 'person', 0.96, 0.89,
            '{"body":[0.1,0.2,0.4,0.6],"face":{"bbox":[0.15,0.22,0.25,0.35],"qualityScore":0.88}}',
            'image_v13', 'CAM_V13/image_v13.jpg', 'crop_v13', 'CAM_V13/crop_v13.jpg',
            CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert capture before migration");
    conn.execute(
        r#"
        INSERT INTO recognition_records (
            recognition_id, camera_id, gallery_id, subject_id, subject_name, similarity,
            field_crop_path, registered_photo_path, recognized_at
        ) VALUES (
            'recognition_bbox_v13', 'CAM_V13', 'default', 'subject_v13', 'V13', 0.9,
            'CAM_V13/crop_v13.jpg', 'galleries/subject_v13.jpg', CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert recognition before migration");

    conn.execute_batch(v12).expect("apply V12");
    conn.execute_batch(v13).expect("apply V13");

    let field_bbox_json: String = conn
        .query_row(
            "SELECT field_bbox_json FROM recognition_records WHERE recognition_id = 'recognition_bbox_v13';",
            [],
            |row| row.get(0),
        )
        .expect("query backfilled field bbox json");
    assert_eq!(
        field_bbox_json,
        r#"{"body":[0.1,0.2,0.4,0.6],"face":{"bbox":[0.15,0.22,0.25,0.35],"qualityScore":0.88}}"#
    );
}

#[test]
fn test_v8_migration_adds_stream_mode_with_default_auto() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v8 = include_str!("../src/migration/migrations/V8__add_camera_stream_mode.sql");

    conn.execute_batch(v1).expect("apply V1");

    // 插入迁移前（无 stream_mode 列）的旧摄像头数据
    conn.execute(
        r#"
        INSERT INTO cameras (
            camera_id, name, protocol, rtsp_url, sub_rtsp_url, remark,
            last_probe_status, last_probe_error_code, last_codec, last_width, last_height, last_fps
        ) VALUES
        ('CAM_TEST', 'Test Old Cam', 'rtsp', 'rtsp://127.0.0.1/live/1', '', '', 'healthy', '', 'h264', 1920, 1080, 25.0);
        "#,
        [],
    )
    .expect("insert camera before V8");

    // 应用 V8 迁移
    conn.execute_batch(v8).expect("apply V8");

    // 查询 stream_mode，必须为默认值 'auto'
    let stream_mode: String = conn
        .query_row(
            "SELECT stream_mode FROM cameras WHERE camera_id = 'CAM_TEST';",
            [],
            |row| row.get(0),
        )
        .expect("query stream_mode");
    assert_eq!(stream_mode, "auto");
}

#[test]
fn test_v14_migration_backfills_evidence_origin_without_guessing_unknowns() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v2 = include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql");
    let v14 = include_str!(
        "../src/migration/migrations/V14__evidence_image_source_and_template_metadata.sql"
    );

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v2).expect("apply V2");
    conn.execute(
        r#"
        INSERT INTO capture_records (
            capture_id, camera_id, track_id, target_label, confidence, quality_score,
            bbox_json, image_id, image_rel_path, crop_image_id, crop_image_rel_path,
            captured_at
        ) VALUES (
            'capture_origin_v14', 'CAM_V14', 9, 'face', 0.97, 0.9,
            '[]', 'image_v14', 'CAM_V14/image_v14.jpg', 'crop_v14', 'CAM_V14/crop_v14.jpg',
            CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert capture before migration");
    conn.execute(
        r#"
        INSERT INTO recognition_records (
            recognition_id, camera_id, gallery_id, subject_id, subject_name, similarity,
            field_crop_path, registered_photo_path, recognized_at
        ) VALUES (
            'recognition_origin_v14', 'CAM_V14', 'default', 'subject_v14', 'V14', 0.9,
            'CAM_V14/crop_v14.jpg', 'galleries/subject_v14.jpg', CURRENT_TIMESTAMP
        )
        "#,
        [],
    )
    .expect("insert recognition before migration");

    conn.execute_batch(v14).expect("apply V14");

    type Origin = (String, String, i64, Option<i64>, Option<f64>);
    let capture: Origin = conn
        .query_row(
            "SELECT image_source, image_stream, image_pts_ms, fused_count, template_quality
             FROM capture_records WHERE capture_id = 'capture_origin_v14';",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("query capture origin");

    // 1. 峰值候选机制与本迁移同批引入，更早的记录必然来自靶向快拍路径 → 可安全回填
    assert_eq!(capture.0, "targeted");
    // 2. 其余三列在旧 schema 中无等价来源：保持「未标注 / 未记录」，不得用近似值伪装已知事实
    assert_eq!(capture.1, "");
    assert_eq!(capture.2, 0);
    assert_eq!(capture.3, None);
    assert_eq!(capture.4, None);

    let recognition: Origin = conn
        .query_row(
            "SELECT image_source, image_stream, image_pts_ms, fused_count, template_quality
             FROM recognition_records WHERE recognition_id = 'recognition_origin_v14';",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("query recognition origin");
    assert_eq!(recognition.0, "targeted");
    assert_eq!(recognition.1, "");
    assert_eq!(recognition.2, 0);
    assert_eq!(recognition.3, None);
    assert_eq!(recognition.4, None);
}

#[test]
fn test_v17_adds_indexes_backing_the_server_side_log_filters() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v9 = include_str!("../src/migration/migrations/V9__operational_logs.sql");
    let v17 = include_str!("../src/migration/migrations/V17__log_filter_indexes.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v9).expect("apply V9");
    conn.execute_batch(v17).expect("apply V17");

    for index in ["idx_operation_logs_status_time", "idx_oplog_target_ts"] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
                [index],
                |row| row.get(0),
            )
            .expect("query index presence");
        assert_eq!(found, 1, "missing index {index}");
    }

    // 迁移器只按版本号跳过，SQL 自身仍需幂等
    conn.execute_batch(v17).expect("apply V17 twice");
}

#[test]
fn test_v19_drops_legacy_galleries_table() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v2 = include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql");
    let v19 = include_str!("../src/migration/migrations/V19__drop_legacy_galleries_table.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v2).expect("apply V2");

    let count_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'galleries';",
            [],
            |row| row.get(0),
        )
        .expect("query galleries table presence before");
    assert_eq!(count_before, 1, "galleries table should exist after V2");

    conn.execute_batch(v19).expect("apply V19");

    let count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'galleries';",
            [],
            |row| row.get(0),
        )
        .expect("query galleries table presence after");
    assert_eq!(count_after, 0, "galleries table should be dropped by V19");

    // 幂等性测试
    conn.execute_batch(v19).expect("apply V19 twice");
}
