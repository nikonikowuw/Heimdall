//! 运维事件日志落库 Worker
//!
//! 负责从内存 channel 批量拉取 `RecordedEvent` 并通过 SQLite 事务撮批持久化，
//! 并在系统停机时完整排空缓冲区中的残留事件。

use pipeline::op_log::{RecordedEvent, BATCH_SIZE};
use tokio::sync::{mpsc, watch};

/// flush worker：攒批写入 SQLite，每 `BATCH_SIZE` 条或 1 秒刷新一次。
///
/// 当 `shutdown` channel 发出停止信号后，排空接收端剩余所有事件并完成最后写入。
pub async fn flush_worker(
    mut rx: mpsc::Receiver<RecordedEvent>,
    db: db::DatabaseConnection,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut batch: Vec<RecordedEvent> = Vec::with_capacity(BATCH_SIZE);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(e) => {
                        batch.push(e);
                        if batch.len() >= BATCH_SIZE {
                            flush_batch(&db, &mut batch).await;
                        }
                    }
                    None => break,
                }
            }
            _ = interval.tick() => {
                if !batch.is_empty() {
                    flush_batch(&db, &mut batch).await;
                }
            }
            _ = shutdown.changed() => {
                // 收到停机通知：非阻塞排空 channel 中所有积压事件
                while let Ok(e) = rx.try_recv() {
                    batch.push(e);
                    if batch.len() >= BATCH_SIZE {
                        flush_batch(&db, &mut batch).await;
                    }
                }
                break;
            }
        }
    }

    // 停机前最后一次 flush
    if !batch.is_empty() {
        flush_batch(&db, &mut batch).await;
    }
}

async fn flush_batch(db: &db::DatabaseConnection, batch: &mut Vec<RecordedEvent>) {
    if batch.is_empty() {
        return;
    }

    let entries: Vec<db::repository::operational_log::InsertEntry> = batch
        .drain(..)
        .map(|recorded| db::repository::operational_log::InsertEntry {
            ts_ms: recorded.ts_ms,
            level: recorded.event.level().to_string(),
            event: recorded.event.tag().to_string(),
            target: recorded.event.target().to_string(),
            message: recorded.event.message(),
            camera_id: recorded.event.camera_id().map(String::from),
            extra_json: recorded.event.extra_json(),
        })
        .collect();

    if let Err(e) = db::OperationalLogRepo::insert_batch(db, entries).await {
        tracing::error!(error = %e, "运维事件日志攒批写入失败");
    }
}
