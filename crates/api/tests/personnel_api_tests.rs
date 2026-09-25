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

#[tokio::test]
async fn test_reextract_features_endpoints() {
    let (app, _state, token) = setup_test_app().await;

    // 1. 在未加载人脸算法包环境下发起全局重新提取 -> 应返回 503 (FaceAlgorithmNotLoaded)
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/personnel/reextract")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept-Language", "en")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 50301);
    assert_eq!(
        json["message"],
        "Face recognition algorithm package is not ready. Please deploy or activate a face algorithm package first."
    );

    // 2. 在未加载人脸算法包环境下发起单人重新提取 (繁体测试) -> 应同样返回 503 并本地化为繁体
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/personnel/some_user/reextract")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept-Language", "zh-TW")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 50301);
    assert_eq!(
        json["message"],
        "人臉識別演算法包未就緒，無法擷取特徵，請先部署/啟用人臉演算法"
    );

    // 3. GET /api/v1/personnel/reextract/status -> 正常返回初始 idle 状态
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/personnel/reextract/status")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["status"], "idle");
}

// ============================================================================
// 人员批量导入
// ============================================================================

/// 构造 multipart 归档上传请求体
fn multipart_archive_body(boundary: &str, filename: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(payload);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// 构造一个含指定文件的 ZIP 归档字节流
fn build_zip_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

fn import_request(token: &str, body: Vec<u8>, boundary: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/personnel/import")
        .header("Authorization", format!("Bearer {token}"))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn test_import_status_starts_idle_with_empty_task_id() {
    let (app, _state, token) = setup_test_app().await;

    let req = Request::builder()
        .uri("/api/v1/personnel/import/status")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["status"], "idle");
    assert_eq!(json["data"]["taskId"], "");
    assert_eq!(json["data"]["total"], 0);
}

#[tokio::test]
async fn test_import_template_downloads_utf8_bom_csv() {
    let (app, _state, token) = setup_test_app().await;

    let req = Request::builder()
        .uri("/api/v1/personnel/import/template")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/csv; charset=utf-8"
    );

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    // BOM 必须存在，否则 Windows Excel 打开会中文乱码
    assert_eq!(&body[..3], &[0xEF, 0xBB, 0xBF]);
    let text = std::str::from_utf8(&body[3..]).unwrap();
    assert!(text.starts_with("name,subjectId,idCard,remark,photos"));
}

#[tokio::test]
async fn test_import_rejects_unsupported_archive_format() {
    let (app, _state, token) = setup_test_app().await;

    let boundary = "----heimdall-import-bad";
    let body = multipart_archive_body(boundary, "notes.txt", b"this is not an archive");
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 40001);
    assert!(json["data"].is_null());
}

#[tokio::test]
async fn test_import_accepts_multipart_body_larger_than_axum_default_limit() {
    let (app, _state, token) = setup_test_app().await;

    let payload = vec![b'x'; 3 * 1024 * 1024];
    let boundary = "----heimdall-import-over-2mib";
    let body = multipart_archive_body(boundary, "large.zip", &payload);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 40001);
    assert!(json["data"].is_null());
}

#[tokio::test]
async fn test_import_rejects_archive_without_usable_content() {
    let (app, _state, token) = setup_test_app().await;

    let archive = build_zip_archive(&[("readme.txt", b"no images here")]);
    let boundary = "----heimdall-import-empty";
    let body = multipart_archive_body(boundary, "bundle.zip", &archive);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 40001);
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .contains("未找到可用的图片"),
        "应给出可执行的解析失败提示, got {}",
        json["message"]
    );
}

#[tokio::test]
async fn test_import_rejects_manifest_missing_name_column() {
    let (app, _state, token) = setup_test_app().await;

    let archive = build_zip_archive(&[
        ("manifest.csv", b"idCard,remark\nID-1,bad header\n"),
        ("a.jpg", b"fake"),
    ]);
    let boundary = "----heimdall-import-badheader";
    let body = multipart_archive_body(boundary, "bundle.zip", &archive);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["message"].as_str().unwrap().contains("name"));
}

#[tokio::test]
async fn test_import_without_algorithm_package_reports_not_loaded() {
    let (app, _state, token) = setup_test_app().await;

    // 归档本身合法，但系统未加载人脸算法包 → 503，且不得预占维护闸门
    let archive = build_zip_archive(&[("张三.jpg", b"fake-image")]);
    let boundary = "----heimdall-import-noalgo";
    let body = multipart_archive_body(boundary, "bundle.zip", &archive);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 50301);
}

#[tokio::test]
async fn test_import_cancel_without_running_task_is_rejected() {
    let (app, _state, token) = setup_test_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/personnel/import/cancel")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 40001);
}

#[tokio::test]
async fn test_import_is_mutually_exclusive_with_reextract() {
    let (app, state, token) = setup_test_app().await;

    // 模拟全量特征重提取已持有维护闸门（绕过算法就绪前置条件）
    let guard = state
        .maintenance_gate
        .acquire(api::MaintenanceTaskKind::Reextract)
        .unwrap();

    let archive = build_zip_archive(&[("张三.jpg", b"fake-image")]);
    let boundary = "----heimdall-import-exclusive";
    let body = multipart_archive_body(boundary, "bundle.zip", &archive);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 40902);
    // 冲突信息需指明当前占用者，避免管理员无从判断该等谁
    assert!(json["message"].as_str().unwrap().contains("特征重新提取"));

    drop(guard);
}

#[tokio::test]
async fn test_import_zip_slip_payload_is_rejected() {
    let (app, _state, token) = setup_test_app().await;

    // 构造带 ../ 穿透的 ZIP：解压必须被安全原语拦截
    let mut buffer = Vec::new();
    {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        writer.start_file("../escaped.jpg", options).unwrap();
        writer.write_all(b"evil").unwrap();
        writer.finish().unwrap();
    }

    let boundary = "----heimdall-import-zipslip";
    let body = multipart_archive_body(boundary, "evil.zip", &buffer);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["message"].as_str().unwrap().contains("Slip"),
        "应明确报告路径穿透拦截, got {}",
        json["message"]
    );
}

#[tokio::test]
async fn test_import_release_gate_after_rejected_request() {
    let (app, state, token) = setup_test_app().await;

    let archive = build_zip_archive(&[("readme.txt", b"nothing")]);
    let boundary = "----heimdall-import-gate-release";
    let body = multipart_archive_body(boundary, "bundle.zip", &archive);
    let res = app
        .oneshot(import_request(&token, body, boundary))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 被拒绝的请求必须释放闸门，否则后续导入/重提取将永久被阻塞
    assert_eq!(
        state.maintenance_gate.current(),
        None,
        "解析失败路径不得残留维护闸门占用"
    );
}
