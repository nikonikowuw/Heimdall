#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use db::entity::gallery_face::ActiveModel as FaceActiveModel;
use db::entity::personnel::ActiveModel as PersonnelActiveModel;
use db::{GalleryFaceRepo, PersonnelRepo};
use sea_orm::Set;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, api::AppState, String) {
    let db = db::init_test_db().await.unwrap();
    let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
    let state = api::AppState::new(db, pipeline);
    api::sync_auth_state(&state).await;

    let password_hash = api::crypto::hash_password_async("adminPassword123".to_string()).await;
    db::AdminUserRepo::create_admin(&state.db, "admin", &password_hash)
        .await
        .unwrap();
    state
        .is_initialized
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let claims = types::AuthClaims {
        sub: "admin".to_string(),
        iat: chrono::Utc::now().timestamp_millis(),
        exp: chrono::Utc::now().timestamp_millis() + 86400000,
    };
    let token = api::crypto::generate_jwt(&claims, &state.get_jwt_secret()).unwrap();

    let app = api::create_app(state.clone());
    (app, state, token)
}

#[tokio::test]
async fn test_face_feature_index_search_and_reload() {
    let (_, state, _) = setup_test_app().await;

    // 1. 创建测试人员与两个人脸特征样本 (单位向量)
    let p = PersonnelActiveModel {
        id: sea_orm::NotSet,
        subject_id: Set("sub_alice".to_string()),
        name: Set("Alice".to_string()),
        id_card: Set("ID12345".to_string()),
        remark: Set("VIP".to_string()),
        primary_photo_path: Set("galleries/sub_alice/original_face_1.jpg".to_string()),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    };
    PersonnelRepo::insert(&state.db, p).await.unwrap();

    // 向量 1: [1.0, 0.0, ...]
    let mut vec1 = [0.0f32; 512];
    vec1[0] = 1.0;
    let mut vec1_bytes = Vec::with_capacity(2048);
    for v in vec1 {
        vec1_bytes.extend_from_slice(&v.to_ne_bytes());
    }

    let f1 = FaceActiveModel {
        id: sea_orm::NotSet,
        face_id: Set("face_1".to_string()),
        subject_id: Set("sub_alice".to_string()),
        photo_rel_path: Set("galleries/sub_alice/original_face_1.jpg".to_string()),
        aligned_rel_path: Set("galleries/sub_alice/aligned_face_1.jpg".to_string()),
        feature_vector: Set(vec1_bytes),
        quality_score: Set(0.95),
        detection_score: Set(0.98),
        is_primary: Set(1),
        created_at: Set(chrono::Utc::now()),
    };
    GalleryFaceRepo::insert(&state.db, f1).await.unwrap();

    // 2. 从 DB 载入特征索引
    let count = state.gallery_index.reload(&state.db).await.unwrap();
    assert_eq!(count, 1);
    assert_eq!(state.gallery_index.count().await, 1);

    // 3. 执行检索：相同方向的向量应高度匹配 (相似度 ~ 1.0)
    let query_match = vec1;
    let result = state
        .gallery_index
        .search(&query_match, 0.75)
        .await
        .expect("应检索命中");
    assert_eq!(result.subject_id, "sub_alice");
    assert_eq!(result.subject_name, "Alice");
    assert_eq!(result.face_id, "face_1");
    assert!((result.similarity - 1.0).abs() < 1e-4);

    // 4. 正交向量 [0.0, 1.0, ...]：点积应为 0.0，低于阈值 0.75，不命中
    let mut query_unmatch = [0.0f32; 512];
    query_unmatch[1] = 1.0;
    assert!(state
        .gallery_index
        .search(&query_unmatch, 0.75)
        .await
        .is_none());
}

#[tokio::test]
async fn test_personnel_api_crud_lifecycle() {
    let (app, state, token) = setup_test_app().await;

    // 1. 预置数据
    let p = PersonnelActiveModel {
        id: sea_orm::NotSet,
        subject_id: Set("emp_bob".to_string()),
        name: Set("Bob".to_string()),
        id_card: Set("ID6789".to_string()),
        remark: Set("Security".to_string()),
        primary_photo_path: Set("galleries/emp_bob/original_face_1.jpg".to_string()),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    };
    PersonnelRepo::insert(&state.db, p).await.unwrap();

    let dummy_vec = vec![0u8; 2048];
    let f1 = FaceActiveModel {
        id: sea_orm::NotSet,
        face_id: Set("face_b1".to_string()),
        subject_id: Set("emp_bob".to_string()),
        photo_rel_path: Set("galleries/emp_bob/original_face_1.jpg".to_string()),
        aligned_rel_path: Set("galleries/emp_bob/aligned_face_1.jpg".to_string()),
        feature_vector: Set(dummy_vec.clone()),
        quality_score: Set(0.85),
        detection_score: Set(0.92),
        is_primary: Set(1),
        created_at: Set(chrono::Utc::now()),
    };
    GalleryFaceRepo::insert(&state.db, f1).await.unwrap();

    let f2 = FaceActiveModel {
        id: sea_orm::NotSet,
        face_id: Set("face_b2".to_string()),
        subject_id: Set("emp_bob".to_string()),
        photo_rel_path: Set("galleries/emp_bob/original_face_2.jpg".to_string()),
        aligned_rel_path: Set("galleries/emp_bob/aligned_face_2.jpg".to_string()),
        feature_vector: Set(dummy_vec),
        quality_score: Set(0.90),
        detection_score: Set(0.95),
        is_primary: Set(0),
        created_at: Set(chrono::Utc::now()),
    };
    GalleryFaceRepo::insert(&state.db, f2).await.unwrap();

    // 2. GET /api/v1/personnel/stats
    let req = Request::builder()
        .uri("/api/v1/personnel/stats")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["totalPersonnel"], 1);
    assert_eq!(json["data"]["totalFaces"], 2);

    // 3. GET /api/v1/personnel (列表查询)
    let req = Request::builder()
        .uri("/api/v1/personnel?keyword=Bob")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["total"], 1);
    assert_eq!(json["data"]["items"][0]["subjectId"], "emp_bob");
    assert_eq!(json["data"]["items"][0]["faceCount"], 2);

    // 4. GET /api/v1/personnel/emp_bob (详情查询)
    let req = Request::builder()
        .uri("/api/v1/personnel/emp_bob")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["faces"].as_array().unwrap().len(), 2);

    // 5. PUT /api/v1/personnel/emp_bob/faces/face_b2/primary (设为主头像)
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/personnel/emp_bob/faces/face_b2/primary")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. DELETE /api/v1/personnel/emp_bob/faces/face_b1 (删除单张人脸样本)
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/personnel/emp_bob/faces/face_b1")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"]["faces"].as_array().unwrap().len(), 1);

    // 7. 再次尝试删除最后一张人脸样本 -> 应 400 拦截
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/personnel/emp_bob/faces/face_b2")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 8. PUT /api/v1/personnel/emp_bob (更新基本资料)
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/personnel/emp_bob")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "name": "Bobby",
                "remark": "Promoted"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 9. DELETE /api/v1/personnel/emp_bob (删除人员)
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/personnel/emp_bob")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 确认已删除
    let check = PersonnelRepo::find_by_subject_id(&state.db, "emp_bob")
        .await
        .unwrap();
    assert!(check.is_none());
}
