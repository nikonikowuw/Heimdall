use db::entity::{capture, gallery, recognition};
use db::{init_test_db, CaptureRepo, GalleryRepo, RecognitionRepo};
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
        captured_at: Set(now),
        ..Default::default()
    };

    let inserted_cap = CaptureRepo::insert(&db, new_capture)
        .await
        .expect("insert capture");
    assert_eq!(inserted_cap.capture_id, "cap_001");

    // 查询列表
    let caps = CaptureRepo::list_recent(&db, Some("cam_01"), 10, 0)
        .await
        .expect("list captures");
    assert_eq!(caps.len(), 1);

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
        registered_photo_path: Set("galleries/alice.jpg".to_string()),
        recognized_at: Set(now),
        ..Default::default()
    };

    let inserted_rec = RecognitionRepo::insert(&db, new_rec)
        .await
        .expect("insert recognition");
    assert_eq!(inserted_rec.subject_name, "Alice");

    let recs = RecognitionRepo::list_recent(&db, Some("cam_01"), 10, 0)
        .await
        .expect("list");
    assert_eq!(recs.len(), 1);

    // 3. 插入底库名单
    let new_gal = gallery::ActiveModel {
        gallery_id: Set("gal_vip".to_string()),
        subject_id: Set("sub_1001".to_string()),
        subject_name: Set("Alice".to_string()),
        subject_type: Set("person".to_string()),
        id_card: Set("ID_12345".to_string()),
        plate_number: Set("".to_string()),
        feature_vector: Set(Some(vec![0u8; 512])),
        photo_rel_path: Set("galleries/alice.jpg".to_string()),
        ..Default::default()
    };

    let inserted_gal = GalleryRepo::insert(&db, new_gal)
        .await
        .expect("insert gallery");
    assert_eq!(inserted_gal.subject_id, "sub_1001");

    let found_gal = GalleryRepo::find_by_subject_id(&db, "sub_1001")
        .await
        .expect("find")
        .expect("some");
    assert_eq!(found_gal.subject_name, "Alice");
}
