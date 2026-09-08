//! 统一规则引擎、证据三支柱与原子加权淘汰端到端集成测试
#![allow(clippy::unwrap_used)]

use async_trait::async_trait;
use db::entity::{alarm, capture};
use db::{init_test_db, AlarmRepo, CaptureRepo};
use pipeline::{EvictionStore, RuleEvaluator, SimpleTracker, StorageCleaner, StorageCleanerConfig};
use sea_orm::ActiveValue::Set;
use sea_orm::DatabaseConnection;
use std::fs;
use types::{
    BoundingBox, Detection, DetectionLineDirection, DetectionPoint, DetectionRule,
    DetectionRuleRole,
};

/// 将 SeaORM DatabaseConnection 桥接为 EvictionStore
struct DbEvictionStore {
    db: DatabaseConnection,
}

#[async_trait]
impl EvictionStore for DbEvictionStore {
    async fn find_oldest_captures(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let list = CaptureRepo::find_oldest_batch(&self.db, limit)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(e.to_string()))?;
        Ok(list
            .into_iter()
            .map(|c| (c.id, c.image_rel_path, c.crop_image_rel_path))
            .collect())
    }

    async fn delete_captures(&self, ids: &[i64]) -> Result<u64, pipeline::PipelineError> {
        CaptureRepo::delete_by_ids(&self.db, ids)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(e.to_string()))
    }

    async fn find_oldest_alarms(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, pipeline::PipelineError> {
        let list = AlarmRepo::find_oldest_batch(&self.db, limit)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(e.to_string()))?;
        Ok(list
            .into_iter()
            .map(|a| (a.id, a.image_rel_path, a.crop_image_rel_path))
            .collect())
    }

    async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, pipeline::PipelineError> {
        AlarmRepo::delete_by_ids(&self.db, ids)
            .await
            .map_err(|e| pipeline::PipelineError::Snapshot(e.to_string()))
    }
}

#[tokio::test]
async fn test_full_pipeline_rules_evidence_and_eviction() {
    let temp_evidence_dir = std::env::temp_dir().join(format!(
        "test_evidence_e2e_{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&temp_evidence_dir).unwrap();

    let db = init_test_db().await.expect("init in-memory db");

    // 1. 设置规则：Y = 0.5 单向 A->B 绊线检测
    let rules = vec![DetectionRule {
        role: DetectionRuleRole::Line,
        line_direction: DetectionLineDirection::AToB,
        points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
    }];

    let mut tracker = SimpleTracker::new();
    let evaluator = RuleEvaluator::new();

    // 2. 模拟第 1 帧检测 (目标底中心在 Y = 0.48, 未触及绊线)
    let det1 = vec![Detection {
        class_id: 0,
        label: "person".to_string(),
        confidence: 0.95,
        bbox: BoundingBox::new(0.4, 0.28, 0.6, 0.48),
    }];
    let tracked1 = tracker.update(det1);
    assert_eq!(tracked1.len(), 1);
    let alarms1 = evaluator.evaluate(&rules, &tracked1, &mut tracker, 1000, 5000);
    assert!(alarms1.is_empty(), "尚未跨越绊线，不应报警");

    // 3. 模拟第 2 帧检测 (目标移动至底中心在 Y = 0.54, 产生有效移动向量并跨越 Y = 0.5 绊线)
    let det2 = vec![Detection {
        class_id: 0,
        label: "person".to_string(),
        confidence: 0.96,
        bbox: BoundingBox::new(0.4, 0.34, 0.6, 0.54),
    }];
    let tracked2 = tracker.update(det2);
    assert_eq!(tracked2.len(), 1);
    assert_eq!(
        tracked2[0].track_id, tracked1[0].track_id,
        "必须成功关联同一航迹！"
    );
    assert_eq!(tracked2[0].trajectory.len(), 2, "轨迹包含前后两点");

    let alarms2 = evaluator.evaluate(&rules, &tracked2, &mut tracker, 1040, 5000);
    assert_eq!(alarms2.len(), 1, "跨越绊线必须触发告警");

    // 4. 落地证据切片与数据库记录
    let cam_id = "cam_rules_001";
    let full_rel = format!("{cam_id}/alarm_full.jpg");
    let crop_rel = format!("{cam_id}/alarm_crop.jpg");
    let full_path = temp_evidence_dir.join(&full_rel);
    let crop_path = temp_evidence_dir.join(&crop_rel);
    fs::create_dir_all(full_path.parent().unwrap()).unwrap();
    fs::write(&full_path, b"fake_jpeg_content").unwrap();
    fs::write(&crop_path, b"fake_crop_content").unwrap();

    let now = chrono::Utc::now();
    let alarm_model = alarm::ActiveModel {
        event_id: Set(format!("evt_{}", uuid::Uuid::new_v4().simple())),
        camera_id: Set(cam_id.to_string()),
        alarm_type_id: Set("line_cross".to_string()),
        occurred_at: Set(now),
        target_label: Set("person".to_string()),
        confidence: Set(0.96),
        track_id: Set(tracked2[0].track_id as i64),
        bbox_json: Set("[0.4, 0.6, 0.6, 0.8]".to_string()),
        image_id: Set("img_01".to_string()),
        image_rel_path: Set(full_rel.clone()),
        crop_image_id: Set("crop_01".to_string()),
        crop_image_rel_path: Set(crop_rel.clone()),
        rule_type: Set("line".to_string()),
        severity: Set("critical".to_string()),
        created_at: Set(now),
        ..Default::default()
    };
    AlarmRepo::insert(&db, alarm_model).await.unwrap();

    // 插入普通抓拍记录
    let cap_full_rel = format!("{cam_id}/cap_full.jpg");
    let cap_crop_rel = format!("{cam_id}/cap_crop.jpg");
    let cap_full_path = temp_evidence_dir.join(&cap_full_rel);
    let cap_crop_path = temp_evidence_dir.join(&cap_crop_rel);
    fs::write(&cap_full_path, b"fake_cap_full").unwrap();
    fs::write(&cap_crop_path, b"fake_cap_crop").unwrap();

    let cap_model = capture::ActiveModel {
        capture_id: Set(format!("cap_{}", uuid::Uuid::new_v4().simple())),
        camera_id: Set(cam_id.to_string()),
        track_id: Set(tracked2[0].track_id as i64),
        target_label: Set("person".to_string()),
        confidence: Set(0.95),
        quality_score: Set(90.0),
        bbox_json: Set("[0.4, 0.6, 0.6, 0.8]".to_string()),
        image_id: Set("cap_img_01".to_string()),
        image_rel_path: Set(cap_full_rel.clone()),
        crop_image_id: Set("cap_crop_01".to_string()),
        crop_image_rel_path: Set(cap_crop_rel.clone()),
        captured_at: Set(now),
        ..Default::default()
    };
    CaptureRepo::insert(&db, cap_model).await.unwrap();

    // 5. 验证存储池加权淘汰（优先淘汰普通抓拍以保全告警大图）
    let store = DbEvictionStore { db: db.clone() };
    let cleaner = StorageCleaner::new(StorageCleanerConfig {
        evidence_dir: temp_evidence_dir.clone(),
        min_free_ratio: 0.99, // 触发淘汰
        emergency_free_ratio: 0.01,
        batch_delete_size: 10,
        ..Default::default()
    });

    let report = cleaner
        .clean_if_needed(&store)
        .await
        .unwrap()
        .expect("should trigger eviction");

    // 必须仅淘汰普通抓拍
    assert_eq!(report.captures_deleted, 1);
    assert_eq!(report.alarms_deleted, 0);

    // 等待异步 Unlink 排空以验证物理回收
    cleaner.flush_pending_unlinks().await;

    // 验证“图在案在，图销案销”：抓拍物理文件被删除，但告警文件与数据库行完好无损！
    assert!(!cap_full_path.exists(), "普通抓拍文件必须被清理");
    assert!(full_path.exists(), "核心告警大图必须完好保全");
    assert!(crop_path.exists(), "核心告警特写必须完好保全");

    let remaining_alarms = AlarmRepo::list_recent(&db, None, 10, 0).await.unwrap();
    assert_eq!(remaining_alarms.len(), 1, "告警数据库记录不可被轻易删除");

    let _ = fs::remove_dir_all(&temp_evidence_dir);
}
