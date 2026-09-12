//! 运维事件日志模块
//!
//! 通过有界 `tokio::sync::mpsc` channel 将运维事件异步暂存，
//! 由 app 层的 flush worker 攒批写入 SQLite，不阻塞业务线程。
//! 任何模块调用 `record()` 即可录入事件。
//!
//! - channel 容量固定（1024），满载时丢弃新事件并递增丢弃计数器
//! - 事件携带录制时刻时间戳，保留原始发生时间

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use tokio::sync::mpsc;
use types::OpEvent;

/// channel 容量上限（满载时丢弃新事件，绝不阻塞调用方）
const CHANNEL_CAPACITY: usize = 1024;

/// 攒批刷新阈值
pub const BATCH_SIZE: usize = 32;

/// 全局 channel 发送端（bounded，try_send 非阻塞，可从任意线程调用）
static OP_LOG_TX: OnceLock<mpsc::Sender<RecordedEvent>> = OnceLock::new();

/// 累计因 channel 满载而丢弃的事件数（供监控查询）
static DROPPED_COUNT: AtomicU64 = AtomicU64::new(0);

/// 带录制时间戳的运维事件
#[derive(Debug)]
pub struct RecordedEvent {
    pub ts_ms: i64,
    pub event: OpEvent,
}

/// 录入一条运维事件。
///
/// - 同步输出到 tracing（携带结构化元数据）
/// - 异步通过 channel 发送到后台 flush worker 写入 SQLite
/// - channel 满或未初始化时静默丢弃，绝不阻塞调用方
pub fn record(event: OpEvent) {
    // 1. 同步输出到 tracing (结构化字段)
    let msg = event.message();
    let tag = event.tag();
    let target = event.target();
    let camera_id = event.camera_id().unwrap_or("");

    match event.level() {
        "error" => {
            tracing::error!(
                event = tag,
                target = target,
                camera_id = camera_id,
                "{}",
                msg
            )
        }
        "warn" => tracing::warn!(
            event = tag,
            target = target,
            camera_id = camera_id,
            "{}",
            msg
        ),
        _ => tracing::info!(
            event = tag,
            target = target,
            camera_id = camera_id,
            "{}",
            msg
        ),
    }

    // 2. 非阻塞发送到 flush worker（携带录制时刻时间戳）
    if let Some(tx) = OP_LOG_TX.get() {
        let recorded = RecordedEvent {
            ts_ms: chrono::Utc::now().timestamp_millis(),
            event,
        };
        if tx.try_send(recorded).is_err() {
            DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 返回累计因 channel 满载而丢弃的事件数
pub fn dropped_count() -> u64 {
    DROPPED_COUNT.load(Ordering::Relaxed)
}

/// 初始化运维日志 channel，返回接收端供 flush worker 消费。
///
/// 必须在应用启动时调用一次。未初始化时 `record()` 仅输出日志不落库。
pub fn init() -> mpsc::Receiver<RecordedEvent> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let _ = OP_LOG_TX.set(tx);
    rx
}
