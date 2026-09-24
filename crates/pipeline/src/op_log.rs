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
use std::time::Instant;

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

/// VPU 配额耗尽按全局时间窗口限频，避免告警高峰刷满运维日志。
static VPU_EXHAUSTED_LAST_MS: AtomicU64 = AtomicU64::new(u64::MAX);
static OP_LOG_CLOCK: OnceLock<Instant> = OnceLock::new();
const VPU_EXHAUSTED_COOLDOWN_MS: u64 = 60_000;

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
    let event = sanitize_event(event);

    // 1. 同步输出到 tracing (结构化字段)
    let msg = event.message();
    let tag = event.tag();
    let target = event.target();
    let camera_id = event.camera_id().unwrap_or("");
    let task_id = event.task_id().unwrap_or_default();
    let algo_id = event.algo_id().unwrap_or("");
    let error = event.error_detail().unwrap_or("");

    match event.level() {
        "error" => {
            tracing::error!(
                event = tag,
                target = target,
                camera_id = camera_id,
                task_id = task_id,
                algo_id = algo_id,
                error = error,
                "{}",
                msg
            )
        }
        "warn" => tracing::warn!(
            event = tag,
            target = target,
            camera_id = camera_id,
            task_id = task_id,
            algo_id = algo_id,
            error = error,
            "{}",
            msg
        ),
        _ => tracing::info!(
            event = tag,
            target = target,
            camera_id = camera_id,
            task_id = task_id,
            algo_id = algo_id,
            error = error,
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

/// 以单调时钟对全局 VPU 配额耗尽事件限频，保留首次发生的摄像头上下文。
pub fn record_vpu_exhausted(camera_id: &str) {
    let elapsed_ms = OP_LOG_CLOCK
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let mut previous = VPU_EXHAUSTED_LAST_MS.load(Ordering::Relaxed);
    loop {
        let last_event = (previous != u64::MAX).then_some(previous);
        if !vpu_cooldown_elapsed(last_event, elapsed_ms) {
            return;
        }
        match VPU_EXHAUSTED_LAST_MS.compare_exchange_weak(
            previous,
            elapsed_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => previous = observed,
        }
    }

    record(OpEvent::VpuExhausted {
        camera_id: camera_id.to_string(),
    });
}

fn vpu_cooldown_elapsed(previous_ms: Option<u64>, now_ms: u64) -> bool {
    previous_ms
        .map(|last_ms| now_ms.saturating_sub(last_ms) >= VPU_EXHAUSTED_COOLDOWN_MS)
        .unwrap_or(true)
}

fn sanitize_event(event: OpEvent) -> OpEvent {
    match event {
        OpEvent::CameraOffline {
            camera_id,
            last_frame_ts,
            reason,
        } => OpEvent::CameraOffline {
            camera_id,
            last_frame_ts,
            reason: sanitize_detail(&reason),
        },
        OpEvent::TaskFailed { task_id, error } => OpEvent::TaskFailed {
            task_id,
            error: sanitize_detail(&error),
        },
        OpEvent::TaskDegraded { task_id, reason } => OpEvent::TaskDegraded {
            task_id,
            reason: sanitize_detail(&reason),
        },
        OpEvent::AlgoLoadFailed { algo_id, error } => OpEvent::AlgoLoadFailed {
            algo_id,
            error: sanitize_detail(&error),
        },
        OpEvent::AlgoSandboxFailed { algo_id, error } => OpEvent::AlgoSandboxFailed {
            algo_id,
            error: sanitize_detail(&error),
        },
        OpEvent::NpuInitFailed { backend, error } => OpEvent::NpuInitFailed {
            backend,
            error: sanitize_detail(&error),
        },
        event => event,
    }
}

fn sanitize_detail(detail: &str) -> String {
    let masked = media::mask_rtsp_url(detail);
    let mut sanitized = String::with_capacity(masked.len().min(512));
    for character in masked.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if sanitized.len() + character.len_utf8() > 512 {
            break;
        }
        sanitized.push(character);
    }
    sanitized
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

#[cfg(test)]
mod tests {
    use super::{sanitize_detail, vpu_cooldown_elapsed, VPU_EXHAUSTED_COOLDOWN_MS};

    #[test]
    fn event_details_redact_rtsp_credentials_and_bound_output() {
        let detail =
            sanitize_detail("failed at rtsp://operator:secret@example.test/live\nretrying");
        assert!(!detail.contains("secret"));
        assert!(!detail.contains('\n'));
        assert!(detail.len() <= 512);
        assert!(sanitize_detail(&"界".repeat(600)).len() <= 512);
    }

    #[test]
    fn vpu_exhaustion_cooldown_suppresses_repeated_events() {
        assert!(vpu_cooldown_elapsed(None, 1_000));
        assert!(!vpu_cooldown_elapsed(
            Some(1_000),
            1_000 + VPU_EXHAUSTED_COOLDOWN_MS - 1
        ));
        assert!(vpu_cooldown_elapsed(
            Some(1_000),
            1_000 + VPU_EXHAUSTED_COOLDOWN_MS
        ));
    }
}
