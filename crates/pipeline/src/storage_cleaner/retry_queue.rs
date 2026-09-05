//! 异步物理 Unlink 调度器与指数退避重试队列 (Async Unlink & Retry Queue)
//!
//! 避免在大批量淘汰时同步执行密集 `unlink` 导致文件系统元数据 IO 阻塞。
//! 遇到短暂的操作系统文件占用时，自动推入有界重试队列并以指数退避重试。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::metrics::EvictionMetrics;

const CHANNEL_CAPACITY: usize = 2048;
const MAX_RETRY_ATTEMPTS: u32 = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(50);

/// 待重试项
#[derive(Debug)]
struct RetryItem {
    path: PathBuf,
    attempts: u32,
    next_retry_at: Instant,
}

/// 异步 Unlink 分发句柄
#[derive(Clone, Debug)]
pub struct UnlinkDispatcher {
    tx: mpsc::Sender<PathBuf>,
    in_flight: Arc<AtomicUsize>,
}

impl UnlinkDispatcher {
    /// 启动后台异步 Unlink 工作协程，返回调度分发器句柄
    pub fn start(metrics: Arc<EvictionMetrics>) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let in_flight = Arc::new(AtomicUsize::new(0));

        let in_flight_clone = Arc::clone(&in_flight);
        let handle = tokio::spawn(async move {
            run_unlink_worker(rx, metrics, in_flight_clone).await;
        });

        (Self { tx, in_flight }, handle)
    }

    /// 将待删除物理文件推入异步 Unlink 管道
    pub async fn dispatch(&self, path: PathBuf) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        if let Err(e) = self.tx.send(path).await {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            tracing::warn!(error = %e, "异步 Unlink 通道已关闭，丢弃删除任务");
        }
    }

    /// 批量推送待删除文件
    pub async fn dispatch_batch(&self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            self.dispatch(path).await;
        }
    }

    /// 阻塞等待当前队列中的在途删除任务完全排空 (用于单元测试与停机同步)
    pub async fn flush_wait(&self) {
        let timeout = Duration::from_secs(5);
        let start = Instant::now();
        while self.in_flight.load(Ordering::SeqCst) > 0 {
            if start.elapsed() > timeout {
                tracing::warn!("Unlink 队列排空等待超时 (5s)");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

async fn run_unlink_worker(
    mut rx: mpsc::Receiver<PathBuf>,
    metrics: Arc<EvictionMetrics>,
    in_flight: Arc<AtomicUsize>,
) {
    let mut retries: Vec<RetryItem> = Vec::new();

    loop {
        // 计算下一个到期的重试等待时间
        let next_retry_delay = retries
            .iter()
            .map(|item| {
                let now = Instant::now();
                if item.next_retry_at > now {
                    item.next_retry_at - now
                } else {
                    Duration::from_millis(0)
                }
            })
            .min();

        tokio::select! {
            // 1. 接收新进入的物理文件删除请求
            maybe_path = rx.recv() => {
                match maybe_path {
                    Some(path) => {
                        process_unlink_path(path, 0, &mut retries, &metrics, &in_flight).await;
                    }
                    None => {
                        // 通道关闭，排空重试队列并退出
                        flush_remaining_retries(&mut retries, &metrics, &in_flight).await;
                        break;
                    }
                }
            }

            // 2. 定时触发重试项
            _ = sleep_or_pending(next_retry_delay) => {
                process_expired_retries(&mut retries, &metrics, &in_flight).await;
            }
        }

        metrics
            .retry_queue_size
            .store(retries.len(), Ordering::Relaxed);
    }
}

async fn sleep_or_pending(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

async fn process_unlink_path(
    path: PathBuf,
    current_attempt: u32,
    retries: &mut Vec<RetryItem>,
    metrics: &Arc<EvictionMetrics>,
    in_flight: &Arc<AtomicUsize>,
) {
    let path_clone = path.clone();
    let res = tokio::task::spawn_blocking(move || std::fs::remove_file(&path_clone)).await;

    match res {
        Ok(Ok(())) => {
            metrics
                .tombstone_reclaimed_total
                .fetch_add(1, Ordering::Relaxed);
            in_flight.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                // 文件已被其他进程清理或不存在，正常计入回收
                metrics
                    .tombstone_reclaimed_total
                    .fetch_add(1, Ordering::Relaxed);
                in_flight.fetch_sub(1, Ordering::SeqCst);
            } else if current_attempt < MAX_RETRY_ATTEMPTS {
                metrics.unlink_retries_total.fetch_add(1, Ordering::Relaxed);
                let backoff = INITIAL_RETRY_DELAY * 2_u32.pow(current_attempt);
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    attempt = current_attempt + 1,
                    backoff_ms = backoff.as_millis(),
                    "物理文件删除失败，已加入延迟重试队列"
                );
                retries.push(RetryItem {
                    path,
                    attempts: current_attempt + 1,
                    next_retry_at: Instant::now() + backoff,
                });
                // 重试中，in_flight 暂不减少
            } else {
                metrics
                    .unlink_failures_total
                    .fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    max_attempts = MAX_RETRY_ATTEMPTS,
                    "物理文件删除多次重试均告失败，发生不可恢复 IO 异常"
                );
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }
        }
        Err(join_err) => {
            tracing::error!(error = %join_err, "异步 Unlink 阻塞任务 Join 失败");
            in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

async fn process_expired_retries(
    retries: &mut Vec<RetryItem>,
    metrics: &Arc<EvictionMetrics>,
    in_flight: &Arc<AtomicUsize>,
) {
    let now = Instant::now();
    let mut remaining = Vec::new();
    let expired: Vec<RetryItem> = retries
        .drain(..)
        .filter_map(|item| {
            if item.next_retry_at <= now {
                Some(item)
            } else {
                remaining.push(item);
                None
            }
        })
        .collect();

    *retries = remaining;

    for item in expired {
        process_unlink_path(item.path, item.attempts, retries, metrics, in_flight).await;
    }
}

async fn flush_remaining_retries(
    retries: &mut Vec<RetryItem>,
    metrics: &Arc<EvictionMetrics>,
    in_flight: &Arc<AtomicUsize>,
) {
    while let Some(item) = retries.pop() {
        let path = item.path;
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(path)).await;
        metrics
            .tombstone_reclaimed_total
            .fetch_add(1, Ordering::Relaxed);
        in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}
