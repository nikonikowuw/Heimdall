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

#[test]
fn test_v22_adds_camera_indexes() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    let v1 = include_str!("../src/migration/migrations/V1__init_schema.sql");
    let v22 = include_str!("../src/migration/migrations/V22__camera_indexes.sql");

    conn.execute_batch(v1).expect("apply V1");
    conn.execute_batch(v22).expect("apply V22");

    for index in [
        "idx_cameras_healthy_status",
        "idx_cameras_gb28181_lookup",
        "idx_cameras_protocol",
    ] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
                [index],
                |row| row.get(0),
            )
            .expect("query index presence");
        assert_eq!(found, 1, "missing index {index}");
    }

    // 验证 EXPLAIN QUERY PLAN 使用部分索引
    let query_plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM cameras WHERE last_probe_status = 'healthy';",
            [],
            |row| row.get(3),
        )
        .expect("explain query plan");
    assert!(
        query_plan.contains("idx_cameras_healthy_status"),
        "expected query plan to use idx_cameras_healthy_status, got: {query_plan}"
    );

    // 幂等性测试
    conn.execute_batch(v22).expect("apply V22 twice");
}

/// V23 必须补上"全部通道 + 时间窗"这一默认态所需的排序流索引，并下线死索引。
///
/// 断言方式是 `EXPLAIN QUERY PLAN` 而非仅检查索引存在：索引建了但计划不选它，
/// 对本次修复而言等于没修（默认态原先退化为 SCAN + TEMP B-TREE）。
#[test]
fn test_v23_adds_alarm_query_indexes_and_drops_dead_capture_index() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    // status 列由 V3 添加，死索引由 V12 创建，因此前置链必须跑到 V12。
    apply_migrations_up_to_v12(&conn);
    let v23 = include_str!("../src/migration/migrations/V23__alarm_query_indexes.sql");
    conn.execute_batch(v23).expect("apply V23");

    // AC3 的前提必须显式成立：本用例在**完全没有 sqlite_stat1** 的库上验证计划，
    // 否则就变成「靠统计信息才走对索引」，而 R2.1 的目标恰恰是摆脱这个依赖。
    let stat1_present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'sqlite_stat1';",
            [],
            |row| row.get(0),
        )
        .expect("probe sqlite_stat1");
    assert_eq!(
        stat1_present, 0,
        "AC3：本用例必须在无统计信息的库上验证计划（不得先行 ANALYZE）"
    );

    for index in ["idx_alarm_records_time", "idx_alarm_records_status_time"] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
                [index],
                |row| row.get(0),
            )
            .expect("query index presence");
        assert_eq!(found, 1, "missing index {index}");
    }

    // 默认态（不约束 camera_id）：必须走 idx_alarm_records_time，且不得出现临时排序。
    // id 是排序次键，索引未覆盖它时 SQLite 会追加一行 `USE TEMP B-TREE FOR LAST TERM OF ORDER BY`。
    let plan = explain_plan(
        &conn,
        "SELECT * FROM alarm_records \
         WHERE occurred_at >= '2026-10-02T00:00:00+00:00' \
         ORDER BY occurred_at DESC, id DESC LIMIT 24;",
    );
    // AC2 要求 SEARCH，不是 SCAN：`SCAN ... USING INDEX`（覆盖索引但全扫）也能
    // 包含索引名，只断言 contains(索引名) 会放过它。这里钉死访问方式。
    assert!(
        plan.contains("SEARCH") && plan.contains("idx_alarm_records_time"),
        "默认态必须走 SEARCH idx_alarm_records_time（不得是 SCAN），got: {plan}"
    );
    assert!(
        !plan.contains("SCAN"),
        "默认态不得回退为全表扫，got: {plan}"
    );
    // 必须对**全部** EXPLAIN 行断言：临时排序是第 2 行，只看第 1 行会漏掉。
    assert!(
        !plan.contains("TEMP B-TREE"),
        "默认态仍在做临时排序（id 未进索引？），got: {plan}"
    );

    // 状态筛选：命中 status_time 索引。
    let status_plan = explain_plan(
        &conn,
        "SELECT * FROM alarm_records \
         WHERE status = 'unprocessed' AND occurred_at >= '2026-10-02T00:00:00+00:00' \
         ORDER BY occurred_at DESC, id DESC LIMIT 24;",
    );
    assert!(
        status_plan.contains("SEARCH") && status_plan.contains("idx_alarm_records_status_time"),
        "状态筛选必须走 SEARCH idx_alarm_records_status_time（不得是 SCAN），got: {status_plan}"
    );
    assert!(
        !status_plan.contains("SCAN"),
        "状态筛选不得回退为全表扫，got: {status_plan}"
    );
    assert!(
        !status_plan.contains("TEMP B-TREE"),
        "状态筛选仍在做临时排序，got: {status_plan}"
    );

    // 死索引必须下线。
    let dead: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_capture_records_camera_crop';",
            [],
            |row| row.get(0),
        )
        .expect("query dead index presence");
    assert_eq!(dead, 0, "idx_capture_records_camera_crop 应已被 V23 删除");

    // 幂等性测试
    conn.execute_batch(v23).expect("apply V23 twice");

    // AC3 反向：即使有人事后跑过 ANALYZE，计划也必须保持 SEARCH（不得反向退化）。
    conn.execute_batch("ANALYZE;")
        .expect("analyze to populate stat1");
    let stat1_now: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'sqlite_stat1';",
            [],
            |row| row.get(0),
        )
        .expect("probe sqlite_stat1 after analyze");
    assert_eq!(stat1_now, 1, "ANALYZE 后应存在 sqlite_stat1");

    let plan_after_analyze = explain_plan(
        &conn,
        "SELECT * FROM alarm_records \
         WHERE occurred_at >= '2026-10-02T00:00:00+00:00' \
         ORDER BY occurred_at DESC, id DESC LIMIT 24;",
    );
    assert!(
        plan_after_analyze.contains("SEARCH")
            && plan_after_analyze.contains("idx_alarm_records_time")
            && !plan_after_analyze.contains("TEMP B-TREE"),
        "AC3：有统计信息时同样应走 SEARCH idx_alarm_records_time 且无临时排序，\
         got: {plan_after_analyze}"
    );
}

/// 取 `EXPLAIN QUERY PLAN` 的**全部**结果行并拼接。
///
/// 必须收全行而不是只读第 1 行：SQLite 把「用哪个索引」放在第 1 行，
/// 而「临时排序」是单独的一行（`USE TEMP B-TREE FOR LAST TERM OF ORDER BY`）。
/// 只读第 1 行会让 `!contains("TEMP B-TREE")` 永真——断言变成死代码，
/// 索引漏写 `id` 次键这种退化就无法被拦住。
fn explain_plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare explain");
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .expect("run explain")
        .collect::<Result<_, _>>()
        .expect("collect explain rows");
    rows.join(" | ")
}

/// V24 必须物理移除从未被使用的 severity 列，且不丢行、不连带删除既有索引。
///
/// SQLite 的 DROP COLUMN 会重写整表，历史上曾出现"重建时索引丢失"的担忧，
/// 因此这里显式断言 V23 建的索引在 V24 之后依然存活。
#[test]
fn test_v24_drops_alarm_severity_preserving_rows_and_indexes() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");

    apply_migrations_up_to_v12(&conn);
    conn.execute_batch(include_str!(
        "../src/migration/migrations/V23__alarm_query_indexes.sql"
    ))
    .expect("apply V23");

    // 迁移前插入两行（含 V2 默认的 severity 取值）
    conn.execute_batch(
        r#"
        INSERT INTO alarm_records
            (event_id, camera_id, alarm_type_id, occurred_at, target_label, severity)
        VALUES
            ('EVT-1', 'cam-01', 'intrusion', '2026-10-02T09:00:00+00:00', 'person', 'warning'),
            ('EVT-2', 'cam-02', 'intrusion', '2026-10-02T10:00:00+00:00', 'car',    'critical');
        "#,
    )
    .expect("insert pre-migration rows");

    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM alarm_records;", [], |row| row.get(0))
        .expect("count before");

    conn.execute_batch(include_str!(
        "../src/migration/migrations/V24__drop_alarm_severity.sql"
    ))
    .expect("apply V24");

    // 列已物理消失
    let has_severity: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('alarm_records') WHERE name = 'severity';",
            [],
            |row| row.get(0),
        )
        .expect("probe severity column");
    assert_eq!(has_severity, 0, "severity 列应已被 V24 移除");

    // 无行丢失
    let rows_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM alarm_records;", [], |row| row.get(0))
        .expect("count after");
    assert_eq!(
        rows_before, rows_after,
        "DROP COLUMN 不得丢行：before={rows_before} after={rows_after}"
    );

    // 索引在整表重写后仍存活
    for index in [
        "idx_alarm_records_time",
        "idx_alarm_records_status_time",
        "idx_alarm_records_camera_time",
    ] {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1;",
                [index],
                |row| row.get(0),
            )
            .expect("query index survival");
        assert_eq!(found, 1, "V24 整表重写后索引 {index} 丢失");
    }
}

/// 按版本顺序应用到 V12 的前置迁移链。
///
/// V23 断言 `status` 列（V3 添加），V23 下线的死索引由 V12 创建——缺任一前置迁移，
/// 测试会以 "no such column" 或"索引本就不存在"的方式假阳性/假阴性失败。
///
/// 只有 V23/V24 用例需要这条完整链，**刻意不让兄弟用例共用**：
/// 其余用例各自只需 V1+V2 / V1+V8 / V1+V9 这样的最小前置集，
/// 把它们升成完整链会让「某个老迁移是否真的留下必需状态」这类断言失去意义。
/// `include_str!` 是编译期宏，无法用一个运行时集合替代重复列写。
fn apply_migrations_up_to_v12(conn: &Connection) {
    let chain: [(&str, &str); 12] = [
        (
            "V1",
            include_str!("../src/migration/migrations/V1__init_schema.sql"),
        ),
        (
            "V2",
            include_str!("../src/migration/migrations/V2__evidence_triad_and_galleries.sql"),
        ),
        (
            "V3",
            include_str!("../src/migration/migrations/V3__alarm_status_processing.sql"),
        ),
        (
            "V4",
            include_str!("../src/migration/migrations/V4__add_algorithms_and_instances.sql"),
        ),
        (
            "V5",
            include_str!("../src/migration/migrations/V5__bind_algorithm_to_analysis_tasks.sql"),
        ),
        (
            "V6",
            include_str!("../src/migration/migrations/V6__bind_algorithm_instances_to_tasks.sql"),
        ),
        (
            "V7",
            include_str!("../src/migration/migrations/V7__gallery_personnel_and_faces.sql"),
        ),
        (
            "V8",
            include_str!("../src/migration/migrations/V8__add_camera_stream_mode.sql"),
        ),
        (
            "V9",
            include_str!("../src/migration/migrations/V9__operational_logs.sql"),
        ),
        (
            "V10",
            include_str!("../src/migration/migrations/V10__recognition_status_and_topk.sql"),
        ),
        (
            "V11",
            include_str!("../src/migration/migrations/V11__gb28181_tables.sql"),
        ),
        (
            "V12",
            include_str!("../src/migration/migrations/V12__recognition_field_image_path.sql"),
        ),
    ];
    for (name, sql) in chain {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("apply {name} failed: {e}"));
    }
}
