use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;

/// 统一媒体流拉流与接入抽象契约 (RTSP, GB28181 等)
#[async_trait]
pub trait MediaIngestor: Send + Sync + 'static {
    /// 运行拉流主循环，由外部传入生命周期取消信号
    async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    );
}
