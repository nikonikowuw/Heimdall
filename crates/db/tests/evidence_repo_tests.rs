use db::entity::{alarm, camera, capture, recognition};
use db::{
    init_test_db, AlarmFilter, AlarmRepo, CameraRepo, CaptureFilter, CaptureRepo,
    RecognitionFilter, RecognitionRepo, UpdateRecognitionReviewParams,
};
use sea_orm::ActiveValue::Set;

#[tokio::test]
async fn test_capture_and_recognition_repository_lifecycle() {
    let db = init_test_db().await.expect("init in-memory db");

    // 1. 插入抓拍记录
    let now = chrono::Utc::now();
    let new_capture = capture::ActiveModel {
        capture_id: Set("cap_001".to_string()),
        camera_id: Set("cam_01".to_string()),
        track_id: Set(42),
        target_label: Set("person".to_string()),
        confidence: Set(0.92),
        quality_score: Set(85.5),
        bbox_json: Set("[0.1, 0.2, 0.3, 0.4]".to_string()),
        image_id: Set("img_001".to_string()),
        image_rel_path: Set("cam_01/img_001.jpg".to_string()),
        crop_image_id: Set("crop_001".to_string()),
        crop_image_rel_path: Set("cam_01/crop_001.jpg".to_string()),
        image_source: Set("peak_candidate".to_string()),
        image_stream: Set("sub".to_string()),
        image_pts_ms: Set(1_741_100_060_000),
        fused_count: Set(Some(4)),
        template_quality: Set(Some(0.72)),
        captured_at: Set(now),
        ..Default::default()
    };

    let inserted_cap = CaptureRepo::insert(&db, new_capture)
        .await
        .expect("insert capture");
    assert_eq!(inserted_cap.capture_id, "cap_001");

    // 查询列表：证据来源与融合模板元数据必须原样往返，不得在仓储层丢失
    let caps = CaptureRepo::list_recent(&db, Some("cam_01"), 10, 0)
        .await
        .expect("list captures");
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].image_source, "peak_candidate");
    assert_eq!(caps[0].image_stream, "sub");
    assert_eq!(caps[0].image_pts_ms, 1_741_100_060_000);
    assert_eq!(caps[0].fused_count, Some(4));
    assert_eq!(caps[0].template_quality, Some(0.72));

    // 查询最老批次
    let oldest_caps = CaptureRepo::find_oldest_batch(&db, 10)
        .await
        .expect("find oldest");
    assert_eq!(oldest_caps.len(), 1);

    // 批量删除
    let deleted_count = CaptureRepo::delete_by_ids(&db, &[inserted_cap.id])
        .await
        .expect("delete");
    assert_eq!(deleted_count, 1);
    let remaining = CaptureRepo::list_recent(&db, None, 10, 0)
        .await
        .expect("list");
    assert!(remaining.is_empty());

    // 2. 插入识别对账记录
    let new_rec = recognition::ActiveModel {
        recognition_id: Set("rec_001".to_string()),
        camera_id: Set("cam_01".to_string()),
        gallery_id: Set("gal_vip".to_string()),
        subject_id: Set("sub_1001".to_string()),
        subject_name: Set("Alice".to_string()),
        similarity: Set(0.96),
        field_crop_path: Set("cam_01/crop_001.jpg".to_string()),
        field_image_path: Set("cam_01/full_001.jpg".to_string()),
        field_bbox_json: Set("[0.1, 0.2, 0.3, 0.4]".to_string()),
        registered_photo_path: Set("galleries/alice.jpg".to_string()),
        image_source: Set("peak_candidate".to_string()),
        image_stream: Set("sub".to_string()),
        image_pts_ms: Set(1_741_100_060_000),
        fused_count: Set(Some(6)),
        template_quality: Set(Some(0.83)),
        recognized_at: Set(now),
        ..Default::default()
    };

    let inserted_rec = RecognitionRepo::insert(&db, new_rec)
        .await
        .expect("insert recognition");
    assert_eq!(inserted_rec.subject_name, "Alice");
    assert_eq!(inserted_rec.image_source, "peak_candidate");
    assert_eq!(inserted_rec.fused_count, Some(6));
    assert_eq!(inserted_rec.template_quality, Some(0.83));

    // 活跃证据路径集合必须覆盖识别对账记录的全部物理文件。
    // 漏登记 `field_image_path` / `registered_photo_path` 会让孤儿对账把仍被引用的
    // 隔离副本（`recognitions/{id}_gallery.jpg`）判为无主文件并清除。
    let active_paths = RecognitionRepo::find_all_active_image_paths(&db)
        .await
        .expect("active image paths");
    for expected in [
        "cam_01/crop_001.jpg",
        "cam_01/full_001.jpg",
        "galleries/alice.jpg",
    ] {
        assert!(
            active_paths.contains(expected),
            "活跃集合必须包含 {expected}，实际: {active_paths:?}"
        );
    }
    assert_eq!(active_paths.len(), 3, "不得把空串当成活跃路径混入集合");

    let recs = RecognitionRepo::list_recent(&db, Some("cam_01"), 10, 0)
        .await
        .expect("list");
    assert_eq!(recs.len(), 1);

    // 验证更新人工复核状态
    let updated = RecognitionRepo::update_review_status(
        &db,
        UpdateRecognitionReviewParams {
            recognition_id: "rec_001",
            status: "confirmed",
            reviewer_id: Some("admin"),
            selected_subject_id: Some("sub_1001"),
            selected_subject_name: Some("Alice Cooper"),
            selected_photo_path: None,
            selected_similarity: Some(0.92),
        },
    )
    .await
    .expect("update review status")
    .expect("exists");
    assert_eq!(updated.status, "confirmed");
    assert_eq!(updated.subject_name, "Alice Cooper");
    assert_eq!(updated.reviewer_id, Some("admin".to_string()));
}

#[tokio::test]
async fn test_evidence_keyword_filters_and_stable_order() {
    let db = init_test_db().await.expect("init in-memory db");
    let now = chrono::Utc::now();

    CameraRepo::insert(
        &db,
        camera::ActiveModel {
            camera_id: Set("cam_search".to_string()),
            name: Set("Front Gate".to_string()),
            rtsp_url: Set("rtsp://localhost/search".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("insert camera");

    AlarmRepo::insert(
        &db,
        alarm::ActiveModel {
            event_id: Set("evt_search".to_string()),
            camera_id: Set("cam_search".to_string()),
            alarm_type_id: Set("intrusion".to_string()),
            occurred_at: Set(now),
            target_label: Set("person".to_string()),
            confidence: Set(0.9),
            track_id: Set(7),
            bbox_json: Set("[]".to_string()),
            image_id: Set("alarm_image".to_string()),
            image_rel_path: Set("alarm.jpg".to_string()),
            crop_image_id: Set("alarm_crop".to_string()),
            crop_image_rel_path: Set("alarm_crop.jpg".to_string()),
            rule_type: Set("intrusion".to_string()),
            status: Set("unprocessed".to_string()),
            handled_at: Set(None),
            created_at: Set(now),
            ..Default::default()
        },
    )
    .await
    .expect("insert alarm");

    let alarms_by_camera_name = AlarmRepo::list_filtered(
        &db,
        AlarmFilter {
            keyword: Some("Front Gate"),
            ..AlarmFilter::default()
        },
        10,
        0,
    )
    .await
    .expect("search alarms by camera name");
    assert_eq!(alarms_by_camera_name.len(), 1);
    assert_eq!(
        AlarmRepo::count_filtered(
            &db,
            AlarmFilter {
                keyword: Some("Front Gate"),
                ..AlarmFilter::default()
            }
        )
        .await
        .expect("count alarm search"),
        1
    );

    let make_capture = |capture_id: &str, target_label: &str| capture::ActiveModel {
        capture_id: Set(capture_id.to_string()),
        camera_id: Set("cam_search".to_string()),
        track_id: Set(7),
        target_label: Set(target_label.to_string()),
        confidence: Set(0.9),
        quality_score: Set(80.0),
        bbox_json: Set("[]".to_string()),
        image_id: Set(format!("{capture_id}_image")),
        image_rel_path: Set(format!("{capture_id}.jpg")),
        crop_image_id: Set(format!("{capture_id}_crop")),
        crop_image_rel_path: Set(format!("{capture_id}_crop.jpg")),
        captured_at: Set(now),
        ..Default::default()
    };

    let first = CaptureRepo::insert(&db, make_capture("cap_percent", "100% person"))
        .await
        .expect("insert first capture");
    let second = CaptureRepo::insert(&db, make_capture("cap_plain", "100X person"))
        .await
        .expect("insert second capture");

    let all = CaptureRepo::list_filtered(
        &db,
        CaptureFilter {
            camera_id: Some("cam_search"),
            ..Default::default()
        },
        10,
        0,
    )
    .await
    .expect("list captures");
    assert_eq!(all.len(), 2);
    assert!(all[0].id > all[1].id, "same-timestamp order must use id");

    let by_camera_name = CaptureRepo::list_filtered(
        &db,
        CaptureFilter {
            keyword: Some("Front Gate"),
            ..Default::default()
        },
        10,
        0,
    )
    .await
    .expect("search by camera name");
    assert_eq!(by_camera_name.len(), 2);

    let by_literal_wildcard = CaptureRepo::list_filtered(
        &db,
        CaptureFilter {
            keyword: Some("100%"),
            ..Default::default()
        },
        10,
        0,
    )
    .await
    .expect("search literal wildcard");
    assert_eq!(by_literal_wildcard.len(), 1);
    assert_eq!(by_literal_wildcard[0].id, first.id);
    assert_ne!(by_literal_wildcard[0].id, second.id);

    let new_rec = recognition::ActiveModel {
        recognition_id: Set("rec_search".to_string()),
        camera_id: Set("cam_search".to_string()),
        gallery_id: Set("gal_search".to_string()),
        subject_id: Set("subject_7".to_string()),
        subject_name: Set("Alice".to_string()),
        similarity: Set(0.96),
        field_crop_path: Set("crop.jpg".to_string()),
        field_image_path: Set("full.jpg".to_string()),
        field_bbox_json: Set("[]".to_string()),
        registered_photo_path: Set("registered.jpg".to_string()),
        recognized_at: Set(now),
        ..Default::default()
    };
    RecognitionRepo::insert(&db, new_rec)
        .await
        .expect("insert recognition");

    let recognition_count = RecognitionRepo::count_filtered(
        &db,
        RecognitionFilter {
            keyword: Some("Alice"),
            ..RecognitionFilter::default()
        },
    )
    .await
    .expect("count recognition search");
    assert_eq!(recognition_count, 1);
}

/// `AlarmRepo::count_since` 必须按帧时间（`occurred_at`）而非入库时间（`created_at`）计数。
///
/// 断言分两层，缺一不可：
///
/// 1. **身份集合**：用列表查询（同样过滤 `occurred_at`）取出命中的 `event_id` 集合，
///    确认恰好是「帧时间在今日」的那几条。这是唯一能防止「漏计 + 多计相互抵消」的断言。
/// 2. **与列表一致**：`count_since` 必须等于该集合大小——这正是需求本身
///    （「统计卡片」与「列表筛选」同基准，不得给出矛盾数字）。
///
/// 构造上刻意让两种实现的总数**不相等**（补录 2 条 vs 时移 1 条）：
/// 若实现回退成 `created_at`，得到的是 2 而非 3，测试立即失败。
/// 仅断言 `count == 2` 且两种形态各一条时，两侧恰好抵消：
/// `occurred_at` 计 [A,B]、`created_at` 计 [A,C]，总数都是 2，测试对回退零保护。
#[tokio::test]
async fn test_alarm_count_since_uses_frame_time_not_insert_time() {
    let db = init_test_db().await.expect("init in-memory db");

    let now = chrono::Utc::now();
    let today_start = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        now.date_naive().and_hms_opt(0, 0, 0).expect("midnight"),
        chrono::Utc,
    );

    // 1. 今日告警：帧时间与入库时间都在今日
    let today_frame = today_start + chrono::Duration::seconds(60);
    // 2. 补录告警 ×2：帧时间在今日，但入库时间在昨日（模拟延迟入库）
    let backfilled_frame_1 = today_start + chrono::Duration::seconds(120);
    let backfilled_frame_2 = today_start + chrono::Duration::seconds(180);
    let yesterday_insert = today_start - chrono::Duration::hours(6);
    // 3. 时移告警 ×1：帧时间在昨日，但入库时间在今日（模拟历史录像回溯分析）
    let yesterday_frame = today_start - chrono::Duration::hours(2);
    let today_insert = today_start + chrono::Duration::seconds(10);

    let rows = [
        ("EVT-TODAY", today_frame, today_frame),
        ("EVT-BACKFILL-1", backfilled_frame_1, yesterday_insert),
        ("EVT-BACKFILL-2", backfilled_frame_2, yesterday_insert),
        ("EVT-SHIFTED", yesterday_frame, today_insert),
    ];
    for (event_id, occurred_at, created_at) in rows {
        AlarmRepo::insert(
            &db,
            alarm::ActiveModel {
                id: sea_orm::NotSet,
                event_id: Set(event_id.to_string()),
                camera_id: Set("cam_01".to_string()),
                alarm_type_id: Set("intrusion".to_string()),
                occurred_at: Set(occurred_at),
                target_label: Set("person".to_string()),
                confidence: Set(0.9),
                track_id: Set(1),
                bbox_json: Set("[0.1, 0.2, 0.3, 0.4]".to_string()),
                image_id: Set(String::new()),
                image_rel_path: Set(String::new()),
                crop_image_id: Set(String::new()),
                crop_image_rel_path: Set(String::new()),
                rule_type: Set("roi".to_string()),
                status: Set("unprocessed".to_string()),
                handled_at: Set(None),
                created_at: Set(created_at),
            },
        )
        .await
        .expect("insert alarm");
    }

    // 第一层：列表查询（与仪表盘同一 `occurred_at` 基准）命中的身份集合。
    let listed = AlarmRepo::list_filtered(
        &db,
        AlarmFilter {
            start_time: Some(today_start),
            ..AlarmFilter::default()
        },
        100,
        0,
    )
    .await
    .expect("list_filtered");

    let mut ids: Vec<&str> = listed.iter().map(|m| m.event_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec!["EVT-BACKFILL-1", "EVT-BACKFILL-2", "EVT-TODAY"],
        "列表必须按 occurred_at 圈定今日告警：帧时间在昨日的 EVT-SHIFTED \
         即使入库时间在今日也不得出现"
    );

    // 第二层：聚合必须与列表同一基准。
    // 若 count_since 回退为 created_at，这里会得到 2（EVT-TODAY + EVT-SHIFTED），
    // 与上面断言的身份集合大小 3 不符，测试失败。
    let count = AlarmRepo::count_since(&db, today_start)
        .await
        .expect("count_since");

    assert_eq!(
        count,
        listed.len() as u64,
        "count_since 必须与列表筛选同基准（occurred_at）：列表命中 {ids:?} 共 {} 条，\
         聚合却得到 {count}。回退成 created_at 会得到 2。",
        listed.len()
    );
}
