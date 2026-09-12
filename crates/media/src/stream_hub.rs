use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};
use types::TransportPolicy;

use crate::dispatcher::{
    ConsumerId, ConsumerKind, DispatcherError, KeyframeCacheStore, MediaSubscription,
    PacketDispatcher, PreviewDistributionConfig, StreamItem,
};
use crate::error::MediaError;
use crate::retina_ingest::RetinaIngestor;
use crate::rtsp::parse_and_clean_rtsp_url;

pub use crate::dispatcher::KeyframeCache;
#[cfg(test)]
use bytes::Bytes;
#[cfg(test)]
use types::{CodecType, EncodedPacket, StreamTag};

/// 对 RTSP URL 进行规范化处理（用于跨设备去重与底层物理连接复用）
pub fn canonicalize_rtsp_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match parse_and_clean_rtsp_url(trimmed) {
        Ok(parsed) => parsed.to_canonical_key(),
        Err(_) => trimmed.trim_end_matches('/').to_string(),
    }
}

/// 单路摄像头的流媒体运行时会话
#[derive(Debug)]
pub struct CameraStreamSession {
    pub camera_id: String,
    pub rtsp_url: String,
    pub transport_policy: TransportPolicy,
    pub active_viewers: Arc<AtomicUsize>,
    pub ai_task_enabled: Arc<AtomicBool>,
    /// 由外部控制面持有的 AI 保活状态。
    manual_ai_enabled: Arc<AtomicBool>,
    /// 当前持有该物理 session 的分析 pump 数量。
    /// `ai_task_enabled` 是兼容性的派生状态，不能被单个 pump 直接覆盖。
    pub ai_task_refs: Arc<AtomicUsize>,
    /// 唯一关键帧/GOP 缓存，首屏注入与消费者 Replay 共用。
    pub keyframe_cache: Arc<KeyframeCacheStore>,
    /// 独立消费者 mailbox 的分发器。
    pub dispatcher: Arc<PacketDispatcher>,
    pub cancel_signal: Arc<AtomicBool>,
    pub cancel_tx: tokio::sync::watch::Sender<bool>,
    pub cancel_rx: tokio::sync::watch::Receiver<bool>,
    pub ingestor_running: Arc<AtomicBool>,
    pub last_packet_time: Arc<AtomicI64>,
    pub reconnect_count: Arc<std::sync::atomic::AtomicU64>,
    pub last_error: Arc<parking_lot::Mutex<Option<String>>>,
    pub cooldown_cancel: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub consecutive_probe_failures: Arc<AtomicUsize>,
}

impl CameraStreamSession {
    fn refresh_ai_task_enabled(&self) {
        let enabled = self.manual_ai_enabled.load(Ordering::SeqCst)
            || self.ai_task_refs.load(Ordering::SeqCst) > 0;
        self.ai_task_enabled.store(enabled, Ordering::SeqCst);
    }

    /// 获取一个分析 pump 的 AI 保活引用。
    pub fn acquire_ai_task(&self) {
        self.ai_task_refs.fetch_add(1, Ordering::SeqCst);
        self.refresh_ai_task_enabled();
    }

    /// 释放一个分析 pump 的 AI 保活引用，并返回剩余引用数。
    pub fn release_ai_task(&self) -> usize {
        let previous = self
            .ai_task_refs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_sub(1))
            })
            .unwrap_or(0);
        let remaining = previous.saturating_sub(1);
        self.refresh_ai_task_enabled();
        remaining
    }

    /// 获取一个分析 pump 的 AI 保活租约（RAII 守卫）。
    pub fn acquire_ai_task_lease(self: &Arc<Self>) -> AiTaskLease {
        AiTaskLease::new(self.clone())
    }

    /// 当前分析 pump 保活引用数。
    pub fn ai_task_ref_count(&self) -> usize {
        self.ai_task_refs.load(Ordering::SeqCst)
    }

    pub async fn cancel_cooldown(&self) {
        let mut guard = self.cooldown_cancel.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }

    /// 构造用于测试或离线注入的模拟流会话。
    pub fn mock(
        stream_key: impl Into<String>,
        rtsp_url: impl Into<String>,
        transport_policy: TransportPolicy,
    ) -> Arc<Self> {
        let cache = Arc::new(KeyframeCacheStore::new());
        let dispatcher = Arc::new(PacketDispatcher::with_cache(
            PreviewDistributionConfig::default(),
            cache.clone(),
        ));
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            camera_id: stream_key.into(),
            rtsp_url: rtsp_url.into(),
            transport_policy,
            active_viewers: Arc::new(AtomicUsize::new(0)),
            ai_task_enabled: Arc::new(AtomicBool::new(false)),
            manual_ai_enabled: Arc::new(AtomicBool::new(false)),
            ai_task_refs: Arc::new(AtomicUsize::new(0)),
            keyframe_cache: cache,
            dispatcher,
            cancel_signal: Arc::new(AtomicBool::new(false)),
            cancel_tx,
            cancel_rx,
            ingestor_running: Arc::new(AtomicBool::new(false)),
            last_packet_time: Arc::new(AtomicI64::new(0)),
            reconnect_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_error: Arc::new(parking_lot::Mutex::new(None)),
            cooldown_cancel: Arc::new(Mutex::new(None)),
            consecutive_probe_failures: Arc::new(AtomicUsize::new(0)),
        })
    }
}

/// 分析任务对流会话的 AI 保活租约（RAII 守卫）。
#[derive(Debug)]
pub struct AiTaskLease {
    session: Arc<CameraStreamSession>,
}

impl AiTaskLease {
    pub fn new(session: Arc<CameraStreamSession>) -> Self {
        session.acquire_ai_task();
        Self { session }
    }

    pub fn session(&self) -> &Arc<CameraStreamSession> {
        &self.session
    }
}

impl Drop for AiTaskLease {
    fn drop(&mut self) {
        self.session.release_ai_task();
    }
}

/// 带物理流生命周期引用的消费者订阅。
#[derive(Debug)]
pub struct StreamSubscription {
    inner: MediaSubscription,
    session: Arc<CameraStreamSession>,
    counts_viewer: bool,
}

impl StreamSubscription {
    pub fn id(&self) -> ConsumerId {
        self.inner.id()
    }

    pub fn kind(&self) -> ConsumerKind {
        self.inner.kind()
    }

    pub async fn recv(&self) -> Option<StreamItem> {
        self.inner.recv().await
    }
}

impl Drop for StreamSubscription {
    fn drop(&mut self) {
        if self.counts_viewer {
            decrement_saturating(&self.session.active_viewers);
        }

        if self.counts_viewer
            && self.session.active_viewers.load(Ordering::SeqCst) == 0
            && !self.session.ai_task_enabled.load(Ordering::SeqCst)
            && self.session.ai_task_ref_count() == 0
        {
            let session = self.session.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(StreamHub::start_cooldown_timer(session));
            }
        }
    }
}

/// 全局流媒体调度与分发中心（按规范化 RTSP URL 复用物理连接）。
#[derive(Debug, Clone)]
pub struct StreamHub {
    /// 物理会话映射：canonical_url -> Arc<CameraStreamSession>
    sessions: Arc<RwLock<HashMap<String, Arc<CameraStreamSession>>>>,
    /// 业务 key 到物理规范化 URL 映射。
    key_to_url: Arc<RwLock<HashMap<String, String>>>,
    /// 规范化 URL 关联的业务 key 集合。
    url_to_keys: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// 摄像机探活失败计数缓存。
    probe_failures: Arc<RwLock<HashMap<String, usize>>>,
    /// 全局消费者准入计数。
    total_consumers: Arc<AtomicUsize>,
    distribution_config: PreviewDistributionConfig,
}

impl Default for StreamHub {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamHub {
    pub fn new() -> Self {
        Self::with_distribution_config(PreviewDistributionConfig::default())
    }

    pub fn with_distribution_config(distribution_config: PreviewDistributionConfig) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            key_to_url: Arc::new(RwLock::new(HashMap::new())),
            url_to_keys: Arc::new(RwLock::new(HashMap::new())),
            probe_failures: Arc::new(RwLock::new(HashMap::new())),
            total_consumers: Arc::new(AtomicUsize::new(0)),
            distribution_config,
        }
    }

    /// 将业务 stream_key 或原始 URL 解析为底层的规范化 URL。
    async fn resolve_canonical_url(&self, key_or_url: &str) -> String {
        {
            let map = self.key_to_url.read().await;
            if let Some(canonical) = map.get(key_or_url) {
                return canonical.clone();
            }
        }
        if key_or_url.starts_with("rtsp://") || key_or_url.starts_with("RTSP://") {
            canonicalize_rtsp_url(key_or_url)
        } else {
            key_or_url.to_string()
        }
    }

    pub fn preview_config(&self) -> PreviewDistributionConfig {
        self.distribution_config.clone()
    }

    /// 注册或获取某路摄像头的流媒体会话。
    pub async fn get_or_create_session(
        &self,
        stream_key: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
    ) -> Arc<CameraStreamSession> {
        let canonical_url = canonicalize_rtsp_url(rtsp_url);
        let effective_url = if canonical_url.is_empty() {
            rtsp_url.to_string()
        } else {
            canonical_url
        };

        {
            let mut key_map = self.key_to_url.write().await;
            key_map.insert(stream_key.to_string(), effective_url.clone());
        }
        {
            let mut url_map = self.url_to_keys.write().await;
            url_map
                .entry(effective_url.clone())
                .or_default()
                .insert(stream_key.to_string());
        }

        let mut map = self.sessions.write().await;
        if let Some(session) = map.get(&effective_url) {
            return session.clone();
        }

        let cache = Arc::new(KeyframeCacheStore::new());
        let dispatcher = Arc::new(PacketDispatcher::with_cache_and_global(
            self.distribution_config.clone(),
            cache.clone(),
            self.total_consumers.clone(),
        ));
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let session = Arc::new(CameraStreamSession {
            camera_id: stream_key.to_string(),
            rtsp_url: effective_url.clone(),
            transport_policy,
            active_viewers: Arc::new(AtomicUsize::new(0)),
            ai_task_enabled: Arc::new(AtomicBool::new(false)),
            manual_ai_enabled: Arc::new(AtomicBool::new(false)),
            ai_task_refs: Arc::new(AtomicUsize::new(0)),
            keyframe_cache: cache,
            dispatcher,
            cancel_signal: Arc::new(AtomicBool::new(false)),
            cancel_tx,
            cancel_rx,
            ingestor_running: Arc::new(AtomicBool::new(false)),
            last_packet_time: Arc::new(AtomicI64::new(0)),
            reconnect_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_error: Arc::new(parking_lot::Mutex::new(None)),
            cooldown_cancel: Arc::new(Mutex::new(None)),
            consecutive_probe_failures: Arc::new(AtomicUsize::new(0)),
        });

        map.insert(effective_url, session.clone());
        session
    }

    /// 移除摄像头会话关联。仍有绑定、观众或 AI 任务时保留物理流。
    pub async fn remove_session(&self, camera_or_stream_key: &str) {
        let keys_to_remove = vec![
            camera_or_stream_key.to_string(),
            types::StreamKey::main(camera_or_stream_key).as_str_key(),
            types::StreamKey::sub(camera_or_stream_key).as_str_key(),
        ];

        let mut affected_urls = Vec::new();
        {
            let mut key_map = self.key_to_url.write().await;
            let mut url_map = self.url_to_keys.write().await;
            for key in &keys_to_remove {
                if let Some(url) = key_map.remove(key) {
                    if let Some(keys) = url_map.get_mut(&url) {
                        keys.remove(key);
                        if keys.is_empty() {
                            url_map.remove(&url);
                        }
                    }
                    affected_urls.push(url);
                }
            }
        }

        let mut map = self.sessions.write().await;
        let url_map = self.url_to_keys.read().await;
        for url in affected_urls {
            let still_bound = url_map.get(&url).map(|s| !s.is_empty()).unwrap_or(false);
            if !still_bound {
                if let Some(session) = map.get(&url) {
                    if session.active_viewers.load(Ordering::SeqCst) == 0
                        && session.ai_task_ref_count() == 0
                        && !session.ai_task_enabled.load(Ordering::SeqCst)
                    {
                        if let Some(session) = map.remove(&url) {
                            session.cancel_signal.store(true, Ordering::SeqCst);
                            let _ = session.cancel_tx.send(true);
                            session.cancel_cooldown().await;
                        }
                    }
                }
            }
        }
    }

    pub async fn is_streaming(&self, key_or_url: &str) -> bool {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        map.get(&canonical)
            .map(|s| s.ingestor_running.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    pub async fn is_healthy_streaming(&self, key_or_url: &str, max_age_ms: i64) -> bool {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        if let Some(session) = map.get(&canonical) {
            if session.ingestor_running.load(Ordering::Relaxed) {
                let last = session.last_packet_time.load(Ordering::Relaxed);
                let now = chrono::Utc::now().timestamp_millis();
                return last > 0 && (now - last) <= max_age_ms;
            }
        }
        false
    }

    pub async fn get_failure_count(&self, camera_id: &str) -> usize {
        let map = self.probe_failures.read().await;
        map.get(camera_id).copied().unwrap_or(0)
    }

    pub async fn reset_failure_count(&self, camera_id: &str) {
        let mut map = self.probe_failures.write().await;
        map.insert(camera_id.to_string(), 0);
    }

    pub async fn increment_failure_count(&self, camera_id: &str) -> usize {
        let mut map = self.probe_failures.write().await;
        let count = map.entry(camera_id.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// 订阅 HTTP-FLV 预览流。
    pub async fn subscribe(
        &self,
        stream_key: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
    ) -> Result<StreamSubscription, MediaError> {
        self.subscribe_kind(
            stream_key,
            rtsp_url,
            transport_policy,
            ConsumerKind::HttpFlv,
        )
        .await
    }

    /// 按消费者类型订阅，分析和证据消费者不计入 active_viewers。
    pub async fn subscribe_kind(
        &self,
        stream_key: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
        kind: ConsumerKind,
    ) -> Result<StreamSubscription, MediaError> {
        let session = self
            .get_or_create_session(stream_key, rtsp_url, transport_policy)
            .await;
        session.cancel_cooldown().await;

        let media_subscription = match session
            .dispatcher
            .subscribe(format!("{kind:?}:{stream_key}"), kind)
        {
            Ok(subscription) => subscription,
            Err(error) => return Err(dispatcher_error_to_media(error)),
        };
        let counts_viewer = matches!(kind, ConsumerKind::HttpFlv | ConsumerKind::WebCodecs);
        if counts_viewer {
            session.active_viewers.fetch_add(1, Ordering::SeqCst);
        }
        Self::ensure_ingestor_started(&session);

        Ok(StreamSubscription {
            inner: media_subscription,
            session,
            counts_viewer,
        })
    }

    /// 为已经创建的会话注册消费者，不启动新的 RTSP ingestor；用于内部挂载和确定性测试。
    pub async fn subscribe_existing_session(
        &self,
        session: Arc<CameraStreamSession>,
        kind: ConsumerKind,
    ) -> Result<StreamSubscription, MediaError> {
        session.cancel_cooldown().await;

        let media_subscription = match session
            .dispatcher
            .subscribe(format!("{kind:?}:{}", session.camera_id), kind)
        {
            Ok(subscription) => subscription,
            Err(error) => return Err(dispatcher_error_to_media(error)),
        };
        let counts_viewer = matches!(kind, ConsumerKind::HttpFlv | ConsumerKind::WebCodecs);
        if counts_viewer {
            session.active_viewers.fetch_add(1, Ordering::SeqCst);
        }

        Ok(StreamSubscription {
            inner: media_subscription,
            session,
            counts_viewer,
        })
    }

    /// 兼容旧控制面的显式退订入口。新媒体 handler 应直接 Drop `StreamSubscription`。
    pub async fn unsubscribe(&self, key_or_url: &str) {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        if let Some(session) = map.get(&canonical) {
            let remaining = decrement_saturating(&session.active_viewers);
            if remaining == 0
                && !session.ai_task_enabled.load(Ordering::SeqCst)
                && session.ai_task_ref_count() == 0
            {
                Self::start_cooldown_timer(session.clone()).await;
            }
        }
    }

    pub async fn set_ai_enabled(
        &self,
        stream_key: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
        enabled: bool,
    ) {
        let session = self
            .get_or_create_session(stream_key, rtsp_url, transport_policy)
            .await;

        if enabled {
            session.manual_ai_enabled.store(true, Ordering::SeqCst);
            session.refresh_ai_task_enabled();
            session.cancel_cooldown().await;
            Self::ensure_ingestor_started(&session);
        } else {
            session.manual_ai_enabled.store(false, Ordering::SeqCst);
            session.refresh_ai_task_enabled();
            if session.active_viewers.load(Ordering::SeqCst) == 0
                && session.ai_task_ref_count() == 0
            {
                Self::start_cooldown_timer(session.clone()).await;
            }
        }
    }

    pub async fn get_keyframe_cache(&self, key_or_url: &str) -> Option<KeyframeCache> {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        map.get(&canonical)
            .map(|session| session.keyframe_cache.snapshot_cache())
    }

    pub async fn stream_health(
        &self,
        key_or_url: &str,
    ) -> Option<crate::dispatcher::StreamHealthSnapshot> {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        map.get(&canonical).map(|session| {
            let source_state = if session.ingestor_running.load(Ordering::SeqCst) {
                let last_ms = session.last_packet_time.load(Ordering::Relaxed);
                if last_ms > 0 && chrono::Utc::now().timestamp_millis() - last_ms > 6000 {
                    "degraded".to_string()
                } else {
                    "running".to_string()
                }
            } else {
                "idle".to_string()
            };
            session.dispatcher.health_snapshot_with_details(
                &session.camera_id,
                source_state,
                session.last_packet_time.load(Ordering::Relaxed),
                session.reconnect_count.load(Ordering::Relaxed),
            )
        })
    }

    pub async fn evict_stalled_consumers(&self) -> usize {
        let sessions: Vec<Arc<CameraStreamSession>> =
            self.sessions.read().await.values().cloned().collect();
        let now_mono_ms = crate::dispatcher::monotonic_ms();
        sessions
            .into_iter()
            .map(|session| session.dispatcher.evict_stalled(now_mono_ms))
            .sum()
    }

    fn ensure_ingestor_started(session: &Arc<CameraStreamSession>) {
        if session
            .ingestor_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            session.cancel_signal.store(false, Ordering::SeqCst);
            let _ = session.cancel_tx.send(false);
            let ingestor = Arc::new(
                RetinaIngestor::new(
                    session.camera_id.clone(),
                    session.rtsp_url.clone(),
                    session.transport_policy,
                    session.dispatcher.clone(),
                )
                .with_last_packet_time(session.last_packet_time.clone())
                .with_reconnect_metrics(
                    session.reconnect_count.clone(),
                    session.last_error.clone(),
                ),
            );

            let session_clone = session.clone();
            let cancel_signal = session.cancel_signal.clone();
            let cancel_rx = session.cancel_rx.clone();
            tokio::spawn(async move {
                ingestor.run_loop(cancel_signal, cancel_rx).await;
                session_clone
                    .ingestor_running
                    .store(false, Ordering::SeqCst);
            });
        }
    }

    async fn start_cooldown_timer(session: Arc<CameraStreamSession>) {
        let mut guard = session.cooldown_cancel.lock().await;
        if let Some(old_handle) = guard.take() {
            old_handle.abort();
        }

        let session_clone = session.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if session_clone.active_viewers.load(Ordering::SeqCst) == 0
                && !session_clone.ai_task_enabled.load(Ordering::SeqCst)
            {
                tracing::info!(
                    camera_id = %session_clone.camera_id,
                    "5秒无新订阅且无AI任务，进入低功耗待机，挂起RTSP拉流"
                );
                session_clone.cancel_signal.store(true, Ordering::SeqCst);
                let _ = session_clone.cancel_tx.send(true);
            }
        });

        *guard = Some(handle);
    }
}

fn dispatcher_error_to_media(error: DispatcherError) -> MediaError {
    match error {
        DispatcherError::TooManyConsumers { max } => MediaError::TooManyConsumers { max },
        DispatcherError::InvalidCapacity => MediaError::Protocol("媒体消费者容量配置无效".into()),
    }
}

fn decrement_saturating(counter: &AtomicUsize) -> usize {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let next = current.saturating_sub(1);
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h264_keyframe() -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
                0x00, 0x00, 0x00, 0x01, 0x65, 0x88,
            ]),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn test_stream_hub_lifecycle_and_ref_count() {
        let hub = StreamHub::new();
        let session = hub
            .get_or_create_session(
                "test-cam-01",
                "rtsp://127.0.0.1:8554/live",
                TransportPolicy::Tcp,
            )
            .await;
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 0);

        let subscription = hub
            .subscribe(
                "test-cam-01",
                "rtsp://127.0.0.1:8554/live",
                TransportPolicy::Tcp,
            )
            .await
            .expect("subscribe");
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 1);
        drop(subscription);
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_keyframe_cache_storage() {
        let hub = StreamHub::new();
        let session = hub
            .get_or_create_session(
                "test-cam-02",
                "rtsp://127.0.0.1:8554/live2",
                TransportPolicy::Auto,
            )
            .await;
        session.dispatcher.publish(h264_keyframe());

        let fetched = hub
            .get_keyframe_cache("test-cam-02")
            .await
            .expect("fetch cache");
        assert_eq!(fetched.sps.as_deref(), Some(&[0x67, 0x42, 0x00, 0x1E][..]));
        assert_eq!(fetched.pps.as_deref(), Some(&[0x68, 0xCE][..]));
        assert_eq!(fetched.last_keyframe_pts, 1000);
    }

    #[tokio::test]
    async fn test_stream_hub_compound_annex_b_cache_extraction() {
        let hub = StreamHub::new();
        let session = hub
            .get_or_create_session(
                "cam-compound:main",
                "rtsp://127.0.0.1:8554/live",
                TransportPolicy::Tcp,
            )
            .await;
        session.dispatcher.publish(h264_keyframe());

        let cache = session.keyframe_cache.snapshot_cache();
        assert_eq!(cache.codec, Some(CodecType::H264));
        assert_eq!(cache.sps.as_deref(), Some(&[0x67, 0x42, 0x00, 0x1E][..]));
        assert_eq!(cache.pps.as_deref(), Some(&[0x68, 0xCE][..]));
        assert_eq!(cache.last_keyframe_pts, 1000);
        assert_eq!(cache.gop_packets.len(), 1);
    }

    #[tokio::test]
    async fn test_stream_hub_audio_packet_does_not_pollute_keyframe_cache() {
        let hub = StreamHub::new();
        let session = hub
            .get_or_create_session(
                "cam-audio-ignore:main",
                "rtsp://127.0.0.1:8554/live",
                TransportPolicy::Tcp,
            )
            .await;
        let audio_pkt = Arc::new(EncodedPacket {
            pts_ms: 1000,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from_static(&[0xFF, 0xF1, 0x50, 0x80, 0x00, 0x00, 0x01, 0xAA]),
            stream_tag: StreamTag::Audio,
        });
        session.dispatcher.publish(audio_pkt);

        let cache = session.keyframe_cache.snapshot_cache();
        assert_eq!(cache.codec, None);
        assert!(cache.gop_packets.is_empty());
    }

    #[test]
    fn test_canonicalize_rtsp_url() {
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100/Streaming/Channels/101"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100/Streaming/Channels/101/"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        assert_eq!(
            canonicalize_rtsp_url("RTSP://192.168.1.50/live"),
            "rtsp://192.168.1.50:554/live"
        );
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:p@ss:word#123@192.168.1.100/live/"),
            "rtsp://admin:p@ss:word#123@192.168.1.100:554/live"
        );
    }

    #[tokio::test]
    async fn test_stream_hub_url_multiplexing_and_ref_count() {
        let hub = StreamHub::new();
        let url_a = "rtsp://admin:12345@192.168.1.100/live/ch1";
        let url_b = "rtsp://admin:12345@192.168.1.100:554/live/ch1/";
        let session_a = hub
            .get_or_create_session("cam-a:main", url_a, TransportPolicy::Tcp)
            .await;
        let session_b = hub
            .get_or_create_session("cam-b:main", url_b, TransportPolicy::Tcp)
            .await;
        assert!(Arc::ptr_eq(&session_a, &session_b));

        let rx_a = hub
            .subscribe("cam-a:main", url_a, TransportPolicy::Tcp)
            .await
            .expect("subscribe cam-a");
        let rx_b = hub
            .subscribe("cam-b:main", url_b, TransportPolicy::Tcp)
            .await
            .expect("subscribe cam-b");
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 2);

        drop(rx_a);
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 1);
        hub.remove_session("cam-a").await;
        assert!(!session_a.cancel_signal.load(Ordering::SeqCst));
        drop(rx_b);
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_stream_hub_ai_task_ref_counting_and_retention() {
        let hub = StreamHub::new();
        let url = "rtsp://127.0.0.1:8554/shared_sub";
        let session_a = hub
            .get_or_create_session("cam-1:sub", url, TransportPolicy::Tcp)
            .await;
        let session_b = hub
            .get_or_create_session("cam-2:sub", url, TransportPolicy::Tcp)
            .await;
        assert!(Arc::ptr_eq(&session_a, &session_b));

        session_a.acquire_ai_task();
        session_b.acquire_ai_task();
        hub.set_ai_enabled("cam-1:sub", url, TransportPolicy::Tcp, false)
            .await;
        assert!(session_a.ai_task_enabled.load(Ordering::SeqCst));
        hub.remove_session("cam-1").await;
        assert!(!session_a.cancel_signal.load(Ordering::SeqCst));

        assert_eq!(session_a.release_ai_task(), 1);
        assert!(session_a.ai_task_enabled.load(Ordering::SeqCst));
        assert_eq!(session_b.release_ai_task(), 0);
        assert!(!session_a.ai_task_enabled.load(Ordering::SeqCst));

        let lease = session_a.acquire_ai_task_lease();
        hub.set_ai_enabled("cam-1:sub", url, TransportPolicy::Tcp, true)
            .await;
        drop(lease);
        assert!(session_a.ai_task_enabled.load(Ordering::SeqCst));
        hub.set_ai_enabled("cam-1:sub", url, TransportPolicy::Tcp, false)
            .await;
        assert!(!session_a.ai_task_enabled.load(Ordering::SeqCst));
    }
}
