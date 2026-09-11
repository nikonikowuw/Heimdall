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
