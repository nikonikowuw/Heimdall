use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};

use crate::entity::recording::{ActiveModel, Column, Entity, Model};
use crate::entity::recording_event;
use crate::error::DbError;

/// 录像记录仓储
#[derive(Debug)]
pub struct RecordingRepo;

/// 创建录像记录的参数
#[derive(Debug, Clone)]
pub struct CreateRecordingParams {
    pub recording_id: String,
    pub camera_id: String,
    pub file_path: String,
    pub start_time: i64,
    pub codec: String,
    pub created_at: i64,
}

/// 关联事件的参数
#[derive(Debug, Clone)]
pub struct LinkEventParams {
    pub recording_id: String,
    pub event_type: String,
    pub event_id: String,
    pub event_time: i64,
    pub offset_ms: i64,
}

/// 完成录像时的更新参数
#[derive(Debug, Clone)]
pub struct FinishRecordingParams {
    pub end_time: i64,
    pub duration_ms: i64,
    pub file_size: i64,
    pub status: String,
}

/// 录像列表过滤条件
#[derive(Debug, Clone, Default)]
pub struct RecordingFilter<'a> {
    pub camera_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
}

impl RecordingRepo {
    /// 按过滤条件分页查询录像（按开始时间倒序）
    pub async fn list_filtered(
        db: &DatabaseConnection,
        filter: RecordingFilter<'_>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find();
        if let Some(camera_id) = filter.camera_id.filter(|s| !s.trim().is_empty()) {
            query = query.filter(Column::CameraId.eq(camera_id));
        }
        if let Some(status) = filter.status.filter(|s| !s.trim().is_empty()) {
            query = query.filter(Column::Status.eq(status));
        }
        if let Some(start) = filter.start_time_ms {
            query = query.filter(Column::StartTime.gte(start));
        }
        if let Some(end) = filter.end_time_ms {
            query = query.filter(Column::StartTime.lte(end));
        }
        let results = query
            .order_by_desc(Column::StartTime)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;
        Ok(results)
    }

    /// 创建一条新的录像记录（状态为 recording）
    pub async fn create(
        db: &DatabaseConnection,
        params: CreateRecordingParams,
    ) -> Result<Model, DbError> {
        let model = ActiveModel {
            recording_id: Set(params.recording_id),
            camera_id: Set(params.camera_id),
            file_path: Set(params.file_path),
            start_time: Set(params.start_time),
            end_time: Set(None),
            duration_ms: Set(None),
            file_size: Set(None),
            codec: Set(params.codec),
            status: Set("recording".to_string()),
            created_at: Set(params.created_at),
            ..Default::default()
        };
        let result = model.insert(db).await?;
        Ok(result)
    }

    /// 完成录像：更新结束时间、时长、文件大小和状态
    pub async fn finish(
        db: &DatabaseConnection,
        recording_id: &str,
        params: FinishRecordingParams,
    ) -> Result<(), DbError> {
        let record = Entity::find()
            .filter(Column::RecordingId.eq(recording_id))
            .one(db)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "recording",
                key: recording_id.to_string(),
            })?;

        let mut active: ActiveModel = record.into();
        active.end_time = Set(Some(params.end_time));
        active.duration_ms = Set(Some(params.duration_ms));
        active.file_size = Set(Some(params.file_size));
        active.status = Set(params.status);
        active.update(db).await?;
        Ok(())
    }

    /// 关联一个事件到录像
    pub async fn link_event(
        db: &DatabaseConnection,
        params: LinkEventParams,
    ) -> Result<recording_event::Model, DbError> {
        let model = recording_event::ActiveModel {
            recording_id: Set(params.recording_id),
            event_type: Set(params.event_type),
            event_id: Set(params.event_id),
            event_time: Set(params.event_time),
            offset_ms: Set(params.offset_ms),
            ..Default::default()
        };
        let result = model.insert(db).await?;
        Ok(result)
    }

    /// 根据 recording_id 查询录像记录
    pub async fn find_by_id(
        db: &DatabaseConnection,
        recording_id: &str,
    ) -> Result<Option<Model>, DbError> {
        let result = Entity::find()
            .filter(Column::RecordingId.eq(recording_id))
            .one(db)
            .await?;
        Ok(result)
    }

    /// 查询某个事件关联的录像
    pub async fn find_by_event(
        db: &DatabaseConnection,
        event_type: &str,
        event_id: &str,
    ) -> Result<Option<(Model, recording_event::Model)>, DbError> {
        let event = recording_event::Entity::find()
            .filter(recording_event::Column::EventType.eq(event_type))
            .filter(recording_event::Column::EventId.eq(event_id))
            .one(db)
            .await?;

        let Some(evt) = event else {
            return Ok(None);
        };

        let recording = Entity::find()
            .filter(Column::RecordingId.eq(&evt.recording_id))
            .one(db)
            .await?;

        match recording {
            Some(rec) => Ok(Some((rec, evt))),
            None => Ok(None),
        }
    }

    /// 查询过期的录像记录（用于 TTL 淘汰）
    ///
    /// 返回 `created_at < cutoff_ms` 的已完成录像，按创建时间升序
    pub async fn find_expired(
        db: &DatabaseConnection,
        cutoff_ms: i64,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        let results = Entity::find()
            .filter(Column::CreatedAt.lt(cutoff_ms))
            .filter(Column::Status.ne("recording"))
            .order_by_asc(Column::CreatedAt)
            .all(db)
            .await?;

        Ok(if results.len() as u64 > limit {
            results.into_iter().take(limit as usize).collect()
        } else {
            results
        })
    }

    /// 原子删除录像记录及其关联事件（级联删除由 FK ON DELETE CASCADE 保证）
    ///
    /// 调用方负责在同一逻辑事务中删除物理文件。
    pub async fn delete_by_id(
        db: &DatabaseConnection,
        recording_id: &str,
    ) -> Result<bool, DbError> {
        let txn = db.begin().await?;

        // 先删关联事件（即使 CASCADE 会处理，显式删更安全）
        recording_event::Entity::delete_many()
            .filter(recording_event::Column::RecordingId.eq(recording_id))
            .exec(&txn)
            .await?;

        let result = Entity::delete_many()
            .filter(Column::RecordingId.eq(recording_id))
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(result.rows_affected > 0)
    }

    /// 查询某个录像关联的所有事件
    pub async fn list_events(
        db: &DatabaseConnection,
        recording_id: &str,
    ) -> Result<Vec<recording_event::Model>, DbError> {
        let results = recording_event::Entity::find()
            .filter(recording_event::Column::RecordingId.eq(recording_id))
            .order_by_asc(recording_event::Column::EventTime)
            .all(db)
            .await?;
        Ok(results)
    }

    /// 查询某个通道正在录制中的录像（应最多一个）
    pub async fn find_active_by_camera(
        db: &DatabaseConnection,
        camera_id: &str,
    ) -> Result<Option<Model>, DbError> {
        let result = Entity::find()
            .filter(Column::CameraId.eq(camera_id))
            .filter(Column::Status.eq("recording"))
            .one(db)
            .await?;
        Ok(result)
    }

    /// 查询最老的已闭合录像（status != recording），按创建时间升序
    pub async fn find_oldest_closed(
        db: &DatabaseConnection,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        let results = Entity::find()
            .filter(Column::Status.ne("recording"))
            .order_by_asc(Column::CreatedAt)
            .limit(limit)
            .all(db)
            .await?;
        Ok(results)
    }

    /// 查询创建时间早于 cut-off 的已闭合录像，按创建时间升序
    pub async fn find_closed_before_ms(
        db: &DatabaseConnection,
        cutoff_ms: i64,
        limit: u64,
    ) -> Result<Vec<Model>, DbError> {
        let results = Entity::find()
            .filter(Column::Status.ne("recording"))
            .filter(Column::CreatedAt.lt(cutoff_ms))
            .order_by_asc(Column::CreatedAt)
            .limit(limit)
            .all(db)
            .await?;
        Ok(results)
    }

    /// 按主键 ID 批量删除录像及其关联事件（单事务）
    pub async fn delete_by_ids(db: &DatabaseConnection, ids: &[i64]) -> Result<u64, DbError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let txn = db.begin().await?;

        // 取出待删的 recording_id 以清理关联事件
        let doomed = Entity::find()
            .filter(Column::Id.is_in(ids.iter().copied()))
            .all(&txn)
            .await?;
        let recording_ids: Vec<String> = doomed.iter().map(|r| r.recording_id.clone()).collect();

        if !recording_ids.is_empty() {
            recording_event::Entity::delete_many()
                .filter(recording_event::Column::RecordingId.is_in(recording_ids))
                .exec(&txn)
                .await?;
        }

        let result = Entity::delete_many()
            .filter(Column::Id.is_in(ids.iter().copied()))
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(result.rows_affected)
    }

    /// 查询所有已落库录像的相对文件路径（供孤儿对账保护已引用文件）
    pub async fn find_all_file_paths(db: &DatabaseConnection) -> Result<Vec<String>, DbError> {
        #[derive(FromQueryResult)]
        struct FilePathRow {
            file_path: String,
        }
        let rows = Entity::find()
            .select_only()
            .column(Column::FilePath)
            .into_model::<FilePathRow>()
            .all(db)
            .await?;
        Ok(rows.into_iter().map(|r| r.file_path).collect())
    }

    /// 单事务落库一条已完成的录像及其全部事件关联。
    ///
    /// 录像 Worker 在 OS 线程内闭合文件后调用异步侧，一次性写入：
    /// `recordings` 行（已 completed/truncated） + N 条 `recording_events`。
    /// 任一写入失败则整体回滚，不产生孤儿录像行。
    pub async fn persist_finished(
        db: &DatabaseConnection,
        record: PersistFinishedParams,
    ) -> Result<Model, DbError> {
        let txn = db.begin().await?;

        let recording = ActiveModel {
            recording_id: Set(record.recording_id.clone()),
            camera_id: Set(record.camera_id),
            file_path: Set(record.file_path),
            start_time: Set(record.start_time_ms),
            end_time: Set(Some(record.end_time_ms)),
            duration_ms: Set(Some((record.end_time_ms - record.start_time_ms).max(0))),
            file_size: Set(Some(record.file_size as i64)),
            codec: Set(record.codec),
            status: Set(record.status),
            created_at: Set(record.created_at_ms),
            ..Default::default()
        }
        .insert(&txn)
        .await?;

        for event in record.events {
            recording_event::ActiveModel {
                recording_id: Set(record.recording_id.clone()),
                event_type: Set(event.event_type),
                event_id: Set(event.event_id),
                event_time: Set(event.event_time_ms),
                offset_ms: Set(event.offset_ms),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }

        txn.commit().await?;
        Ok(recording)
    }
}

/// 已完成录像的单事务落库参数
#[derive(Debug, Clone)]
pub struct PersistFinishedParams {
    pub recording_id: String,
    pub camera_id: String,
    pub file_path: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub file_size: u64,
    pub codec: String,
    /// `completed` | `truncated`
    pub status: String,
    pub created_at_ms: i64,
    pub events: Vec<PersistEventParams>,
}

/// 录像关联事件落库参数
#[derive(Debug, Clone)]
pub struct PersistEventParams {
    pub event_type: String,
    pub event_id: String,
    pub event_time_ms: i64,
    pub offset_ms: i64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::migration::init_test_db;

    #[tokio::test]
    async fn test_create_and_find_recording() {
        let db = init_test_db().await.unwrap();

        let params = CreateRecordingParams {
            recording_id: "rec_001".to_string(),
            camera_id: "cam_test".to_string(),
            file_path: "recordings/cam_test/2025-01-15/083052_rec_001.mp4".to_string(),
            start_time: 1705300000000,
            codec: "h264".to_string(),
            created_at: 1705300000000,
        };
        let created = RecordingRepo::create(&db, params).await.unwrap();
        assert_eq!(created.recording_id, "rec_001");
        assert_eq!(created.status, "recording");
        assert!(created.end_time.is_none());

        let found = RecordingRepo::find_by_id(&db, "rec_001").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().camera_id, "cam_test");
    }

    #[tokio::test]
    async fn test_finish_recording() {
        let db = init_test_db().await.unwrap();

        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_002".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "test.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();

        RecordingRepo::finish(
            &db,
            "rec_002",
            FinishRecordingParams {
                end_time: 21000,
                duration_ms: 20000,
                file_size: 1024 * 1024,
                status: "completed".to_string(),
            },
        )
        .await
        .unwrap();

        let rec = RecordingRepo::find_by_id(&db, "rec_002")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rec.status, "completed");
        assert_eq!(rec.end_time, Some(21000));
        assert_eq!(rec.duration_ms, Some(20000));
        assert_eq!(rec.file_size, Some(1024 * 1024));
    }

    #[tokio::test]
    async fn test_link_and_list_events() {
        let db = init_test_db().await.unwrap();

        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_003".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "test.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();

        // 关联两个事件
        RecordingRepo::link_event(
            &db,
            LinkEventParams {
                recording_id: "rec_003".to_string(),
                event_type: "alarm".to_string(),
                event_id: "evt_a1".to_string(),
                event_time: 5000,
                offset_ms: 4000,
            },
        )
        .await
        .unwrap();

        RecordingRepo::link_event(
            &db,
            LinkEventParams {
                recording_id: "rec_003".to_string(),
                event_type: "recognition".to_string(),
                event_id: "evt_r1".to_string(),
                event_time: 8000,
                offset_ms: 7000,
            },
        )
        .await
        .unwrap();

        let events = RecordingRepo::list_events(&db, "rec_003").await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "alarm");
        assert_eq!(events[1].event_type, "recognition");
    }

    #[tokio::test]
    async fn test_find_by_event() {
        let db = init_test_db().await.unwrap();

        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_004".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "test.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();

        RecordingRepo::link_event(
            &db,
            LinkEventParams {
                recording_id: "rec_004".to_string(),
                event_type: "alarm".to_string(),
                event_id: "evt_find_me".to_string(),
                event_time: 5000,
                offset_ms: 4000,
            },
        )
        .await
        .unwrap();

        let result = RecordingRepo::find_by_event(&db, "alarm", "evt_find_me")
            .await
            .unwrap();
        assert!(result.is_some());
        let (rec, evt) = result.unwrap();
        assert_eq!(rec.recording_id, "rec_004");
        assert_eq!(evt.offset_ms, 4000);

        // 不存在的事件
        let none = RecordingRepo::find_by_event(&db, "alarm", "not_exist")
            .await
            .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn test_delete_cascades_events() {
        let db = init_test_db().await.unwrap();

        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_del".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "test.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();

        RecordingRepo::link_event(
            &db,
            LinkEventParams {
                recording_id: "rec_del".to_string(),
                event_type: "alarm".to_string(),
                event_id: "evt_del".to_string(),
                event_time: 5000,
                offset_ms: 4000,
            },
        )
        .await
        .unwrap();

        let deleted = RecordingRepo::delete_by_id(&db, "rec_del").await.unwrap();
        assert!(deleted);

        // 录像和事件都应该被删除
        let rec = RecordingRepo::find_by_id(&db, "rec_del").await.unwrap();
        assert!(rec.is_none());
        let events = RecordingRepo::list_events(&db, "rec_del").await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_find_expired() {
        let db = init_test_db().await.unwrap();

        // 创建一个旧录像（已完成）
        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_old".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "old.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();
        RecordingRepo::finish(
            &db,
            "rec_old",
            FinishRecordingParams {
                end_time: 21000,
                duration_ms: 20000,
                file_size: 1024,
                status: "completed".to_string(),
            },
        )
        .await
        .unwrap();

        // 创建一个新录像（已完成）
        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_new".to_string(),
                camera_id: "cam_test".to_string(),
                file_path: "new.mp4".to_string(),
                start_time: 99999000,
                codec: "h264".to_string(),
                created_at: 99999000,
            },
        )
        .await
        .unwrap();
        RecordingRepo::finish(
            &db,
            "rec_new",
            FinishRecordingParams {
                end_time: 100019000,
                duration_ms: 20000,
                file_size: 1024,
                status: "completed".to_string(),
            },
        )
        .await
        .unwrap();

        // cutoff 在两者之间
        let expired = RecordingRepo::find_expired(&db, 50000, 100).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].recording_id, "rec_old");
    }

    #[tokio::test]
    async fn test_find_active_by_camera() {
        let db = init_test_db().await.unwrap();

        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_active".to_string(),
                camera_id: "cam_active".to_string(),
                file_path: "active.mp4".to_string(),
                start_time: 1000,
                codec: "h264".to_string(),
                created_at: 1000,
            },
        )
        .await
        .unwrap();

        let active = RecordingRepo::find_active_by_camera(&db, "cam_active")
            .await
            .unwrap();
        assert!(active.is_some());
        assert_eq!(active.unwrap().recording_id, "rec_active");

        let none = RecordingRepo::find_active_by_camera(&db, "cam_other")
            .await
            .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn test_persist_finished_with_events() {
        let db = init_test_db().await.unwrap();

        let record = PersistFinishedParams {
            recording_id: "rec_fin_001".to_string(),
            camera_id: "cam_persist".to_string(),
            file_path: "recordings/cam_persist/2025-07-15/083052_rec_fin_0.mp4".to_string(),
            start_time_ms: 1752568252000,
            end_time_ms: 1752568272000,
            file_size: 2_500_000,
            codec: "h264".to_string(),
            status: "completed".to_string(),
            created_at_ms: 1752568273000,
            events: vec![
                PersistEventParams {
                    event_type: "alarm".to_string(),
                    event_id: "evt_1".to_string(),
                    event_time_ms: 1752568260000,
                    offset_ms: 8000,
                },
                PersistEventParams {
                    event_type: "recognition".to_string(),
                    event_id: "evt_2".to_string(),
                    event_time_ms: 1752568269000,
                    offset_ms: 17000,
                },
            ],
        };

        let saved = RecordingRepo::persist_finished(&db, record).await.unwrap();
        assert_eq!(saved.recording_id, "rec_fin_001");
        assert_eq!(saved.status, "completed");
        // done: duration 应由 end - start 推导
        assert_eq!(saved.duration_ms, Some(20_000));
        assert_eq!(saved.file_size, Some(2_500_000));

        // 两个事件关联必须一并写入
        let events = RecordingRepo::list_events(&db, "rec_fin_001")
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "evt_1");
        assert_eq!(events[0].offset_ms, 8000);
        assert_eq!(events[1].event_id, "evt_2");
        assert_eq!(events[1].offset_ms, 17000);

        // 通过事件反查应能命中
        let found = RecordingRepo::find_by_event(&db, "alarm", "evt_1")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().0.recording_id, "rec_fin_001");
    }

    #[tokio::test]
    async fn test_persist_finished_rolls_back_on_duplicate() {
        let db = init_test_db().await.unwrap();

        let make = |id: &str| PersistFinishedParams {
            recording_id: id.to_string(),
            camera_id: "cam_dup".to_string(),
            file_path: "dup.mp4".to_string(),
            start_time_ms: 1000,
            end_time_ms: 2000,
            file_size: 100,
            codec: "h264".to_string(),
            status: "completed".to_string(),
            created_at_ms: 2000,
            events: vec![PersistEventParams {
                event_type: "alarm".to_string(),
                event_id: "evt_dup".to_string(),
                event_time_ms: 1500,
                offset_ms: 500,
            }],
        };

        RecordingRepo::persist_finished(&db, make("rec_dup"))
            .await
            .unwrap();
        // 同一 recording_id 再次写入应失败（UNIQUE 约束）
        let second = RecordingRepo::persist_finished(&db, make("rec_dup")).await;
        assert!(second.is_err(), "重复 recording_id 应报错");

        // 事务回滚：事件不应重复
        let events = RecordingRepo::list_events(&db, "rec_dup").await.unwrap();
        assert_eq!(events.len(), 1, "失败事务不得残留额外事件行");
    }

    #[tokio::test]
    async fn test_batch_delete_and_oldest_closed() {
        let db = init_test_db().await.unwrap();

        // 三条：recording 中 / 已闭合旧 / 已闭合新
        RecordingRepo::create(
            &db,
            CreateRecordingParams {
                recording_id: "rec_active_x".to_string(),
                camera_id: "cam_b".to_string(),
                file_path: "recordings/cam_b/a.mp4".to_string(),
                start_time: 100,
                codec: "h264".to_string(),
                created_at: 100,
            },
        )
        .await
        .unwrap();

        for (id, created) in [("rec_old_x", 200i64), ("rec_new_x", 900)] {
            RecordingRepo::create(
                &db,
                CreateRecordingParams {
                    recording_id: id.to_string(),
                    camera_id: "cam_b".to_string(),
                    file_path: format!("recordings/cam_b/{id}.mp4"),
                    start_time: created,
                    codec: "h264".to_string(),
                    created_at: created,
                },
            )
            .await
            .unwrap();
            RecordingRepo::finish(
                &db,
                id,
                FinishRecordingParams {
                    end_time: created + 1000,
                    duration_ms: 1000,
                    file_size: 512,
                    status: "completed".to_string(),
                },
            )
            .await
            .unwrap();
        }

        // find_oldest_closed 不得返回 recording 中的记录
        let oldest = RecordingRepo::find_oldest_closed(&db, 10).await.unwrap();
        assert_eq!(oldest.len(), 2);
        assert_eq!(oldest[0].recording_id, "rec_old_x");
        assert_eq!(oldest[1].recording_id, "rec_new_x");

        // find_closed_before_ms 只返回早于 cut-off 的
        let before = RecordingRepo::find_closed_before_ms(&db, 500, 10)
            .await
            .unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].recording_id, "rec_old_x");

        // find_all_file_paths 应覆盖全部三条
        let all_paths = RecordingRepo::find_all_file_paths(&db).await.unwrap();
        assert_eq!(all_paths.len(), 3);

        // 批量删除：已闭合两条被清理，recording 中的保留
        let ids: Vec<i64> = oldest.iter().map(|r| r.id).collect();
        let deleted = RecordingRepo::delete_by_ids(&db, &ids).await.unwrap();
        assert_eq!(deleted, 2);
        assert!(RecordingRepo::find_by_id(&db, "rec_old_x")
            .await
            .unwrap()
            .is_none());
        assert!(RecordingRepo::find_by_id(&db, "rec_new_x")
            .await
            .unwrap()
            .is_none());
        assert!(RecordingRepo::find_by_id(&db, "rec_active_x")
            .await
            .unwrap()
            .is_some());
    }
}
