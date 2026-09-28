#![allow(clippy::unwrap_used)]

use db::entity::gallery_face::ActiveModel as FaceActiveModel;
use db::entity::personnel::ActiveModel as PersonnelActiveModel;
use db::{create_tables_if_not_exist, GalleryFaceRepo, PersonnelRepo};
use sea_orm::{ActiveValue::Set, Database};

#[tokio::test]
async fn test_personnel_and_gallery_faces_crud() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    create_tables_if_not_exist(&db).await.unwrap();

    // 1. 录入人员
    let p1 = PersonnelActiveModel {
        id: sea_orm::NotSet,
        subject_id: Set("emp_001".to_string()),
        name: Set("张三".to_string()),
        id_card: Set("110101199001011234".to_string()),
        remark: Set("研发部".to_string()),
        primary_photo_path: Set("galleries/emp_001/original_face_1.jpg".to_string()),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    };
    let saved_p1 = PersonnelRepo::insert(&db, p1).await.unwrap();
    assert_eq!(saved_p1.subject_id, "emp_001");
    assert_eq!(saved_p1.name, "张三");

    // 2. 录入两张人脸特征样本 (512 维 FP32 = 2048 字节)
    let dummy_vec_1 = vec![0u8; 2048];
    let dummy_vec_2 = vec![1u8; 2048];

    let f1 = FaceActiveModel {
        id: sea_orm::NotSet,
        face_id: Set("face_1".to_string()),
        subject_id: Set("emp_001".to_string()),
        photo_rel_path: Set("galleries/emp_001/original_face_1.jpg".to_string()),
        aligned_rel_path: Set("galleries/emp_001/aligned_face_1.jpg".to_string()),
        feature_vector: Set(dummy_vec_1),
        quality_score: Set(0.95),
        detection_score: Set(0.99),
        is_primary: Set(1),
        created_at: Set(chrono::Utc::now()),
    };
    GalleryFaceRepo::insert(&db, f1).await.unwrap();

    let f2 = FaceActiveModel {
        id: sea_orm::NotSet,
        face_id: Set("face_2".to_string()),
        subject_id: Set("emp_001".to_string()),
        photo_rel_path: Set("galleries/emp_001/original_face_2.jpg".to_string()),
        aligned_rel_path: Set("galleries/emp_001/aligned_face_2.jpg".to_string()),
        feature_vector: Set(dummy_vec_2),
        quality_score: Set(0.88),
        detection_score: Set(0.97),
        is_primary: Set(0),
        created_at: Set(chrono::Utc::now()),
    };
    GalleryFaceRepo::insert(&db, f2).await.unwrap();

    // 3. 验证查询
    let faces = GalleryFaceRepo::list_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    assert_eq!(faces.len(), 2);
    assert_eq!(faces[0].face_id, "face_1");
    assert_eq!(faces[0].is_primary, 1);

    // 4. 设 face_2 为主头像
    GalleryFaceRepo::set_primary(&db, "emp_001", "face_2")
        .await
        .unwrap();
    let faces_updated = GalleryFaceRepo::list_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    assert_eq!(faces_updated[0].face_id, "face_2");
    assert_eq!(faces_updated[0].is_primary, 1);
    assert_eq!(faces_updated[1].face_id, "face_1");
    assert_eq!(faces_updated[1].is_primary, 0);

    // 4.1 更新 face_1 特征向量与对齐路径
    let new_vec_1 = vec![2u8; 2048];
    let updated = GalleryFaceRepo::update_feature(
        &db,
        "face_1",
        new_vec_1.clone(),
        "galleries/emp_001/aligned_face_1_v2.jpg",
        0.98,
        0.99,
    )
    .await
    .unwrap();
    assert!(updated);
    let f1_updated = GalleryFaceRepo::find_by_face_id(&db, "face_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f1_updated.feature_vector, new_vec_1);
    assert_eq!(
        f1_updated.aligned_rel_path,
        "galleries/emp_001/aligned_face_1_v2.jpg"
    );
    assert_eq!(f1_updated.quality_score, 0.98);
    assert_eq!(f1_updated.detection_score, 0.99);

    // 5. 模糊搜索
    let (list, total) = PersonnelRepo::list_filtered(&db, Some("张"), 10, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(list[0].subject_id, "emp_001");

    // 6. 验证全量特征向量列表
    let all_vectors = GalleryFaceRepo::list_all_valid_vectors(&db).await.unwrap();
    assert_eq!(all_vectors.len(), 2);

    // 7. 删除单张样本
    GalleryFaceRepo::delete_by_face_id(&db, "face_1")
        .await
        .unwrap();
    let count_after_delete = GalleryFaceRepo::count_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    assert_eq!(count_after_delete, 1);

    // 8. 级联删除人员及关联样本
    PersonnelRepo::delete_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    let p_after = PersonnelRepo::find_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    assert!(p_after.is_none());
    let faces_after = GalleryFaceRepo::list_by_subject_id(&db, "emp_001")
        .await
        .unwrap();
    assert_eq!(faces_after.len(), 0);
}

#[tokio::test]
/// 同一人员的全部样本常在同一批次内写入，`created_at` 完全相同。
/// 缺 `Id` 次键时详情页照片顺序会在两次请求间跳变，
/// `delete_face` 选出的新主头像也变得不确定。
///
/// 主头像优先 + 创建时间升序不变；并列时按主键升序收敛。
async fn gallery_faces_of_one_subject_have_a_deterministic_order() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    create_tables_if_not_exist(&db).await.unwrap();

    let created_at = chrono::Utc::now();
    PersonnelRepo::insert(
        &db,
        PersonnelActiveModel {
            id: sea_orm::NotSet,
            subject_id: Set("emp_tie".to_string()),
            name: Set("并列顺序".to_string()),
            id_card: Set(String::new()),
            remark: Set(String::new()),
            primary_photo_path: Set(String::new()),
            created_at: Set(created_at),
            updated_at: Set(created_at),
        },
    )
    .await
    .unwrap();

    // 同一时间戳、同一主头像标记：顺序完全依赖主键次键
    for face_id in ["face_a", "face_b", "face_c", "face_d"] {
        GalleryFaceRepo::insert(
            &db,
            FaceActiveModel {
                id: sea_orm::NotSet,
                face_id: Set(face_id.to_string()),
                subject_id: Set("emp_tie".to_string()),
                photo_rel_path: Set(format!("galleries/emp_tie/original_{face_id}.jpg")),
                aligned_rel_path: Set(String::new()),
                feature_vector: Set(vec![0u8; 2048]),
                quality_score: Set(0.9),
                detection_score: Set(0.9),
                is_primary: Set(0),
                created_at: Set(created_at),
            },
        )
        .await
        .unwrap();
    }

    let faces = GalleryFaceRepo::list_by_subject_id(&db, "emp_tie")
        .await
        .unwrap();
    assert_eq!(faces.len(), 4);

    let ids: Vec<i64> = faces.iter().map(|face| face.id).collect();
    let mut expected = ids.clone();
    expected.sort_unstable();
    assert_eq!(ids, expected, "并列行必须按主键升序稳定排序");
    assert_eq!(
        faces
            .iter()
            .map(|face| face.face_id.as_str())
            .collect::<Vec<_>>(),
        vec!["face_a", "face_b", "face_c", "face_d"],
        "插入顺序应能被主键次键可重现地还原"
    );

    // 主头像优先优先级高于时间次键
    GalleryFaceRepo::set_primary(&db, "emp_tie", "face_c")
        .await
        .unwrap();
    let reordered = GalleryFaceRepo::list_by_subject_id(&db, "emp_tie")
        .await
        .unwrap();
    assert_eq!(reordered[0].face_id, "face_c", "主头像必须排在最前");
}

#[tokio::test]
async fn personnel_keyword_is_literal_and_tied_rows_have_stable_order() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    create_tables_if_not_exist(&db).await.unwrap();
    let created_at = chrono::Utc::now();

    for (subject_id, name) in [
        ("percent", "Worker 100% complete"),
        ("wildcard", "Worker 100X complete"),
        ("underscore", "Worker A_B complete"),
    ] {
        PersonnelRepo::insert(
            &db,
            PersonnelActiveModel {
                id: sea_orm::NotSet,
                subject_id: Set(subject_id.to_string()),
                name: Set(name.to_string()),
                id_card: Set(String::new()),
                remark: Set(String::new()),
                primary_photo_path: Set(String::new()),
                created_at: Set(created_at),
                updated_at: Set(created_at),
            },
        )
        .await
        .unwrap();
    }

    let (percent_matches, total) = PersonnelRepo::list_filtered(&db, Some("100%"), 20, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(percent_matches[0].subject_id, "percent");

    let (underscore_matches, total) = PersonnelRepo::list_filtered(&db, Some("A_B"), 20, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(underscore_matches[0].subject_id, "underscore");

    let (ordered, _) = PersonnelRepo::list_filtered(&db, None, 20, 0)
        .await
        .unwrap();
    let ids: Vec<i64> = ordered.iter().map(|person| person.id).collect();
    let mut expected_ids = ids.clone();
    // 次键必须与时间列**同向**：DESC 时间 + DESC 主键。
    // 只需断言确定性与同向性，不必断言具体数值。
    expected_ids.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(
        ids, expected_ids,
        "同一 created_at 时必须按主键同向（DESC）稳定排序"
    );

    // 翻页不得重复或漏行：同一时间戳集合下，两页应恰好拼出全集
    let (first_page, _) = PersonnelRepo::list_filtered(&db, None, 2, 0).await.unwrap();
    let (second_page, _) = PersonnelRepo::list_filtered(&db, None, 2, 2).await.unwrap();
    let mut paged: Vec<i64> = first_page
        .iter()
        .chain(second_page.iter())
        .map(|person| person.id)
        .collect();
    let unpaged = ids.len();
    paged.sort_unstable();
    paged.dedup();
    assert_eq!(paged.len(), unpaged, "同毫秒并列行翻页时必须无重复、无漏行");
}
