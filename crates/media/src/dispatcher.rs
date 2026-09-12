use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::Notify;
use types::{CodecType, EncodedPacket, StreamTag};

use crate::sps::split_annex_b_nalus;

pub type ConsumerId = u64;

/// 消费者类型，用于资源预算、日志和运行时健康快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsumerKind {
    HttpFlv,
    WebCodecs,
    Analysis,
    MainStreamEvidence,
}

/// 媒体消费者收到的显式控制/数据消息。
#[derive(Debug, Clone)]
pub enum StreamItem {
    /// 正常压缩包。
    Packet(Arc<EncodedPacket>),
    /// 从最近完整 GOP 恢复。消费者必须先重置解码/封装状态。
    Replay(Arc<GopSnapshot>),
    /// RTSP 源会话重建。消费者必须丢弃旧参考链。
    SourceReset { epoch: u64 },
}

/// 可被多个消费者共享的完整 GOP 快照。
#[derive(Debug, Clone)]
pub struct GopSnapshot {
    pub epoch: u64,
    pub codec: CodecType,
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
    pub packets: Arc<[Arc<EncodedPacket>]>,
    pub first_pts_ms: i64,
    pub last_pts_ms: i64,
    pub total_payload_bytes: usize,
}

impl GopSnapshot {
    pub fn to_keyframe_cache(&self) -> KeyframeCache {
        KeyframeCache {
            epoch: self.epoch,
            codec: Some(self.codec),
            sps: self.sps.clone(),
            pps: self.pps.clone(),
            vps: self.vps.clone(),
            last_keyframe: self.packets.first().map(|packet| packet.payload.clone()),
            last_keyframe_pts: self.first_pts_ms,
            gop_packets: self.packets.iter().cloned().collect(),
        }
    }
}

/// 与现有 FLV 首屏注入逻辑兼容的缓存视图。
#[derive(Debug, Default, Clone)]
pub struct KeyframeCache {
    pub epoch: u64,
    pub codec: Option<CodecType>,
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
    pub last_keyframe: Option<Bytes>,
    pub last_keyframe_pts: i64,
    pub gop_packets: Vec<Arc<EncodedPacket>>,
}

/// 预览分发资源预算。
#[derive(Debug, Clone)]
pub struct PreviewDistributionConfig {
    pub max_consumers_per_stream: usize,
    pub max_total_consumers: usize,
    pub preview_mailbox_capacity: usize,
    pub analysis_mailbox_capacity: usize,
    pub max_gop_packets: usize,
    pub max_gop_bytes: usize,
    pub max_packet_bytes: usize,
    pub http_merge_flush_ms: u64,
    pub http_merge_max_bytes: usize,
    pub zombie_no_progress_ms: u64,
}

impl Default for PreviewDistributionConfig {
    fn default() -> Self {
        Self {
            max_consumers_per_stream: 16,
            max_total_consumers: 128,
            preview_mailbox_capacity: 96,
            analysis_mailbox_capacity: 32,
            max_gop_packets: 75,
            max_gop_bytes: 8 * 1024 * 1024,
            max_packet_bytes: 8 * 1024 * 1024,
            http_merge_flush_ms: 10,
            http_merge_max_bytes: 64 * 1024,
            zombie_no_progress_ms: 10_000,
        }
    }
}

#[derive(Debug, Default)]
pub struct DispatcherMetrics {
    pub total_published: AtomicU64,
    pub total_dropped: AtomicU64,
    pub total_replays: AtomicU64,
    pub total_replay_packets: AtomicU64,
    pub total_source_resets: AtomicU64,
    pub total_zombie_evictions: AtomicU64,
    pub total_subscribe_rejected: AtomicU64,
    pub total_oversized_packets: AtomicU64,
    pub current_consumers: AtomicUsize,
}

impl DispatcherMetrics {
    fn snapshot(&self) -> DispatcherMetricsSnapshot {
        DispatcherMetricsSnapshot {
            total_published: self.total_published.load(Ordering::Relaxed),
            total_dropped: self.total_dropped.load(Ordering::Relaxed),
            total_replays: self.total_replays.load(Ordering::Relaxed),
            total_replay_packets: self.total_replay_packets.load(Ordering::Relaxed),
            total_source_resets: self.total_source_resets.load(Ordering::Relaxed),
            total_zombie_evictions: self.total_zombie_evictions.load(Ordering::Relaxed),
            total_subscribe_rejected: self.total_subscribe_rejected.load(Ordering::Relaxed),
            total_oversized_packets: self.total_oversized_packets.load(Ordering::Relaxed),
            current_consumers: self.current_consumers.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatcherMetricsSnapshot {
    pub total_published: u64,
    pub total_dropped: u64,
    pub total_replays: u64,
    pub total_replay_packets: u64,
    pub total_source_resets: u64,
    pub total_zombie_evictions: u64,
    pub total_subscribe_rejected: u64,
    pub total_oversized_packets: u64,
    pub current_consumers: usize,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DispatcherError {
    #[error("媒体消费者数量已达到上限 ({max})")]
    TooManyConsumers { max: usize },
    #[error("媒体消费者 mailbox 容量必须大于 0")]
    InvalidCapacity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerHealthSnapshot {
    pub consumer_id: ConsumerId,
    pub name: String,
    pub kind: ConsumerKind,
    pub queue_depth: usize,
    pub queue_capacity: usize,
    pub dropped_packets: u64,
    pub replay_count: u64,
    pub recovering: bool,
    pub last_progress_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamHealthSnapshot {
    pub stream_key: String,
    pub source_state: String,
    pub source_epoch: u64,
    pub last_packet_at_ms: i64,
    pub reconnect_count: u64,
    pub active_consumers: usize,
    pub max_consumers: usize,
    pub gop_packets: usize,
    pub gop_bytes: usize,
    pub metrics: DispatcherMetricsSnapshot,
    pub consumers: Vec<ConsumerHealthSnapshot>,
}

#[derive(Debug)]
struct MailboxState {
    queue: VecDeque<StreamItem>,
    closed: bool,
    recovering: bool,
}

#[derive(Debug)]
pub struct ConsumerMailbox {
    state: Mutex<MailboxState>,
    notify: Notify,
    capacity: usize,
    last_progress_mono_ms: AtomicU64,
    last_progress_at_ms: std::sync::atomic::AtomicI64,
    dropped_packets: AtomicU64,
    replay_count: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfferResult {
    Accepted,
    NeedsReplay { dropped: u64 },
    Closed,
}

impl ConsumerMailbox {
    fn new(capacity: usize) -> Result<Self, DispatcherError> {
        if capacity == 0 {
            return Err(DispatcherError::InvalidCapacity);
        }
        Ok(Self {
            state: Mutex::new(MailboxState {
                queue: VecDeque::with_capacity(capacity),
                closed: false,
                recovering: false,
            }),
            notify: Notify::new(),
            capacity,
            last_progress_mono_ms: AtomicU64::new(monotonic_ms()),
            last_progress_at_ms: std::sync::atomic::AtomicI64::new(
                chrono::Utc::now().timestamp_millis(),
            ),
            dropped_packets: AtomicU64::new(0),
            replay_count: AtomicU64::new(0),
        })
    }

    fn offer_packet(&self, packet: Arc<EncodedPacket>) -> OfferResult {
        let result = {
            let mut state = self.state.lock();
            if state.closed {
                OfferResult::Closed
            } else if state.recovering {
                self.dropped_packets.fetch_add(1, Ordering::Relaxed);
                OfferResult::NeedsReplay { dropped: 1 }
            } else if state.queue.len() < self.capacity {
                state.queue.push_back(StreamItem::Packet(packet));
                OfferResult::Accepted
            } else {
                let dropped = state.queue.len() as u64 + 1;
                state.queue.clear();
                state.recovering = true;
                self.dropped_packets.fetch_add(dropped, Ordering::Relaxed);
                OfferResult::NeedsReplay { dropped }
            }
        };

        if result == OfferResult::Accepted {
            self.notify.notify_one();
        }
        result
    }

    fn offer_replay(&self, snapshot: Arc<GopSnapshot>) -> bool {
        let accepted = {
            let mut state = self.state.lock();
            if state.closed {
                false
            } else {
                state.queue.push_back(StreamItem::Replay(snapshot));
                state.recovering = false;
                self.replay_count.fetch_add(1, Ordering::Relaxed);
                true
            }
        };

        if accepted {
            self.notify.notify_one();
        }
        accepted
    }

    fn mark_recovering(&self) {
        let mut state = self.state.lock();
        if !state.closed {
            state.queue.clear();
            state.recovering = true;
        }
    }

    fn source_reset(&self, epoch: u64) {
        {
            let mut state = self.state.lock();
            if state.closed {
                return;
            }
            state.queue.clear();
            state.recovering = true;
            state.queue.push_back(StreamItem::SourceReset { epoch });
        }
        self.notify.notify_one();
    }

    pub fn close(&self) {
        {
            let mut state = self.state.lock();
            state.closed = true;
            state.queue.clear();
            state.recovering = false;
        }
        self.notify.notify_waiters();
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().closed
    }

    pub fn queue_depth(&self) -> usize {
        self.state.lock().queue.len()
    }

    pub fn is_recovering(&self) -> bool {
        self.state.lock().recovering
    }

    pub fn dropped_packets(&self) -> u64 {
        self.dropped_packets.load(Ordering::Relaxed)
    }

    pub fn replay_count(&self) -> u64 {
        self.replay_count.load(Ordering::Relaxed)
    }

    pub fn last_progress_mono_ms(&self) -> u64 {
        self.last_progress_mono_ms.load(Ordering::Relaxed)
    }

    pub fn last_progress_at_ms(&self) -> i64 {
        self.last_progress_at_ms.load(Ordering::Relaxed)
    }

    /// 通过 Notify 异步等待，不进行固定周期轮询。
    pub async fn recv(&self) -> Option<StreamItem> {
        loop {
            let notified = self.notify.notified();
            let result = {
                let mut state = self.state.lock();
                if let Some(item) = state.queue.pop_front() {
                    Some(Ok(item))
                } else if state.closed {
                    Some(Err(()))
                } else {
                    None
                }
            };

            match result {
                Some(Ok(item)) => {
                    self.last_progress_mono_ms
                        .store(monotonic_ms(), Ordering::Relaxed);
                    self.last_progress_at_ms
                        .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                    return Some(item);
                }
                Some(Err(())) => return None,
                None => notified.await,
            }
        }
    }
}

#[derive(Debug)]
struct ConsumerLease {
    released: AtomicBool,
    counter: Arc<AtomicUsize>,
}

impl ConsumerLease {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        Self {
            released: AtomicBool::new(false),
            counter,
        }
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            decrement_saturating(&self.counter);
        }
    }
}

#[derive(Debug)]
struct Consumer {
    id: ConsumerId,
    name: String,
    kind: ConsumerKind,
    mailbox: Arc<ConsumerMailbox>,
    lease: Arc<ConsumerLease>,
}
#[derive(Debug)]
pub struct MediaSubscription {
    id: ConsumerId,
    kind: ConsumerKind,
    mailbox: Arc<ConsumerMailbox>,
    dispatcher: Arc<PacketDispatcher>,
    lease: Arc<ConsumerLease>,
}

impl MediaSubscription {
    pub fn id(&self) -> ConsumerId {
        self.id
    }

    pub fn kind(&self) -> ConsumerKind {
        self.kind
    }

    pub fn mailbox(&self) -> &Arc<ConsumerMailbox> {
        &self.mailbox
    }

    pub async fn recv(&self) -> Option<StreamItem> {
        self.mailbox.recv().await
    }
}

impl Drop for MediaSubscription {
    fn drop(&mut self) {
        self.dispatcher.unsubscribe(self.id);
        self.lease.release();
    }
}

fn valid_h265_vps(nalu: &[u8]) -> bool {
    nalu.len() >= 3 && nalu[0] & 0x80 == 0 && (nalu[0] >> 1) & 0x3F == 32 && nalu[1] & 0x07 != 0
}

/// 唯一的关键帧/GOP 缓存。首屏注入与丢帧恢复共享此状态。
#[derive(Debug)]
pub struct KeyframeCacheStore {
    inner: RwLock<KeyframeCache>,
}

impl Default for KeyframeCacheStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyframeCacheStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(KeyframeCache::default()),
        }
    }

    pub fn clear_for_epoch(&self, epoch: u64) {
        let mut cache = self.inner.write();
        *cache = KeyframeCache {
            epoch,
            ..KeyframeCache::default()
        };
    }

    pub fn snapshot_cache(&self) -> KeyframeCache {
        self.inner.read().clone()
    }

    pub fn update(
        &self,
        packet: &Arc<EncodedPacket>,
        epoch: u64,
        config: &PreviewDistributionConfig,
    ) {
        if packet.stream_tag == StreamTag::Audio || !packet.codec.is_video() {
            return;
        }

        let nalus = split_annex_b_nalus(&packet.payload);
        if nalus.is_empty() {
            return;
        }

        let mut cache = self.inner.write();
        if cache.epoch != epoch {
            *cache = KeyframeCache {
                epoch,
                ..KeyframeCache::default()
            };
        }
        cache.codec = Some(packet.codec);

        if packet.is_keyframe {
            cache.gop_packets.clear();
            cache.last_keyframe = Some(packet.payload.clone());
            cache.last_keyframe_pts = packet.pts_ms;
        }

        match packet.codec {
            CodecType::H264 => {
                for nalu in &nalus {
                    if nalu.is_empty() {
                        continue;
                    }
                    match nalu[0] & 0x1F {
                        7 => cache.sps = Some(Bytes::copy_from_slice(nalu)),
                        8 => cache.pps = Some(Bytes::copy_from_slice(nalu)),
                        _ => {}
                    }
                }
            }
            CodecType::H265 => {
                for nalu in &nalus {
                    if nalu.len() < 2 {
                        continue;
                    }
                    match (nalu[0] >> 1) & 0x3F {
                        32 => {
                            if valid_h265_vps(nalu) {
                                cache.vps = Some(Bytes::copy_from_slice(nalu));
                            } else {
                                cache.vps = None;
                            }
                        }
                        33 => cache.sps = Some(Bytes::copy_from_slice(nalu)),
                        34 => cache.pps = Some(Bytes::copy_from_slice(nalu)),
                        _ => {}
                    }
                }
            }
            CodecType::Aac => return,
        }

        if !packet.is_keyframe && cache.gop_packets.is_empty() {
            return;
        }

        let next_bytes = cache
            .gop_packets
            .iter()
            .map(|item| item.payload.len())
            .sum::<usize>()
            .saturating_add(packet.payload.len());
        if cache.gop_packets.len() >= config.max_gop_packets || next_bytes > config.max_gop_bytes {
            cache.gop_packets.clear();
            return;
        }

        cache.gop_packets.push(packet.clone());
    }

    pub fn snapshot(&self, config: &PreviewDistributionConfig) -> Option<Arc<GopSnapshot>> {
        let cache = self.inner.read();
        let codec = cache.codec?;
        let first = cache.gop_packets.first()?;
        if !first.is_keyframe || cache.sps.is_none() || cache.pps.is_none() {
            return None;
        }
        if codec == CodecType::H265 && cache.vps.is_none() {
            return None;
        }
        let total_payload_bytes = cache
            .gop_packets
            .iter()
            .map(|packet| packet.payload.len())
            .sum::<usize>();
        if cache.gop_packets.len() > config.max_gop_packets
            || total_payload_bytes > config.max_gop_bytes
        {
            return None;
        }

        Some(Arc::new(GopSnapshot {
            epoch: cache.epoch,
            codec,
            sps: cache.sps.clone(),
            pps: cache.pps.clone(),
            vps: cache.vps.clone(),
            packets: cache.gop_packets.clone().into(),
            first_pts_ms: first.pts_ms,
            last_pts_ms: cache.gop_packets.last()?.pts_ms,
            total_payload_bytes,
        }))
    }
}

/// 多消费者媒体分发器。
#[derive(Debug)]
pub struct PacketDispatcher {
    consumers: RwLock<HashMap<ConsumerId, Arc<Consumer>>>,
    next_id: AtomicU64,
    epoch: AtomicU64,
    cache: Arc<KeyframeCacheStore>,
    limits: PreviewDistributionConfig,
    global_consumers: Arc<AtomicUsize>,
    pub metrics: Arc<DispatcherMetrics>,
}

impl PacketDispatcher {
    pub fn new(limits: PreviewDistributionConfig) -> Self {
        Self::with_cache(limits, Arc::new(KeyframeCacheStore::new()))
    }

    pub fn with_cache(limits: PreviewDistributionConfig, cache: Arc<KeyframeCacheStore>) -> Self {
        Self::with_cache_and_global(limits, cache, Arc::new(AtomicUsize::new(0)))
    }

    pub fn with_cache_and_global(
        limits: PreviewDistributionConfig,
        cache: Arc<KeyframeCacheStore>,
        global_consumers: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            consumers: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            epoch: AtomicU64::new(0),
            cache,
            limits,
            global_consumers,
            metrics: Arc::new(DispatcherMetrics::default()),
        }
    }

    pub fn cache(&self) -> Arc<KeyframeCacheStore> {
        self.cache.clone()
    }

    pub fn subscribe(
        self: &Arc<Self>,
        name: impl Into<String>,
        kind: ConsumerKind,
    ) -> Result<MediaSubscription, DispatcherError> {
        let capacity = match kind {
            ConsumerKind::Analysis | ConsumerKind::MainStreamEvidence => {
                self.limits.analysis_mailbox_capacity
            }
            ConsumerKind::HttpFlv | ConsumerKind::WebCodecs => self.limits.preview_mailbox_capacity,
        };
        let mailbox = Arc::new(ConsumerMailbox::new(capacity)?);
        if reserve_total(&self.global_consumers, self.limits.max_total_consumers).is_none() {
            self.metrics
                .total_subscribe_rejected
                .fetch_add(1, Ordering::Relaxed);
            return Err(DispatcherError::TooManyConsumers {
                max: self.limits.max_total_consumers,
            });
        }
        let mut consumers = self.consumers.write();
        if consumers.len() >= self.limits.max_consumers_per_stream {
            self.metrics
                .total_subscribe_rejected
                .fetch_add(1, Ordering::Relaxed);
            decrement_saturating(&self.global_consumers);
            return Err(DispatcherError::TooManyConsumers {
                max: self.limits.max_consumers_per_stream,
            });
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let lease = Arc::new(ConsumerLease::new(self.global_consumers.clone()));
        consumers.insert(
            id,
            Arc::new(Consumer {
                id,
                name: name.into(),
                kind,
                mailbox: mailbox.clone(),
                lease: lease.clone(),
            }),
        );
        self.metrics
            .current_consumers
            .fetch_add(1, Ordering::Relaxed);

        Ok(MediaSubscription {
            id,
            kind,
            mailbox,
            dispatcher: self.clone(),
            lease,
        })
    }

    pub fn unsubscribe(&self, id: ConsumerId) {
        let consumer = self.consumers.write().remove(&id);
        if let Some(consumer) = consumer {
            consumer.mailbox.close();
            consumer.lease.release();
            self.metrics
                .current_consumers
                .fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn publish(&self, packet: Arc<EncodedPacket>) {
        self.metrics.total_published.fetch_add(1, Ordering::Relaxed);

        if packet.payload.len() > self.limits.max_packet_bytes {
            self.metrics
                .total_oversized_packets
                .fetch_add(1, Ordering::Relaxed);
            self.metrics.total_dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                payload_bytes = packet.payload.len(),
                max_packet_bytes = self.limits.max_packet_bytes,
                "丢弃超过保护上限的媒体包"
            );
            let epoch = self.epoch.load(Ordering::Acquire);
            self.cache.clear_for_epoch(epoch);
            let consumers: Vec<Arc<Consumer>> = self.consumers.read().values().cloned().collect();
            for consumer in consumers {
                consumer.mailbox.mark_recovering();
            }
            return;
        }

        let epoch = self.epoch.load(Ordering::Acquire);
        self.cache.update(&packet, epoch, &self.limits);

        let consumers: Vec<Arc<Consumer>> = self.consumers.read().values().cloned().collect();
        let mut replay_targets = Vec::new();
        let mut closed_ids = Vec::new();

        for consumer in consumers {
            match consumer.mailbox.offer_packet(packet.clone()) {
                OfferResult::Accepted => {}
                OfferResult::NeedsReplay { dropped } => {
                    replay_targets.push(consumer.mailbox.clone());
                    self.metrics
                        .total_dropped
                        .fetch_add(dropped, Ordering::Relaxed);
                }
                OfferResult::Closed => closed_ids.push(consumer.id),
            }
        }

        if !replay_targets.is_empty() {
            if let Some(snapshot) = self.cache.snapshot(&self.limits) {
                for mailbox in replay_targets {
                    if mailbox.offer_replay(snapshot.clone()) {
                        self.metrics.total_replays.fetch_add(1, Ordering::Relaxed);
                        self.metrics
                            .total_replay_packets
                            .fetch_add(snapshot.packets.len() as u64, Ordering::Relaxed);
                    }
                }
            }
        }

        for id in closed_ids {
            self.unsubscribe(id);
        }
    }

    pub fn source_reset(&self) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.cache.clear_for_epoch(epoch);
        self.metrics
            .total_source_resets
            .fetch_add(1, Ordering::Relaxed);

        let consumers: Vec<Arc<Consumer>> = self.consumers.read().values().cloned().collect();
        for consumer in consumers {
            consumer.mailbox.source_reset(epoch);
        }
        epoch
    }

    pub fn health_snapshot(&self) -> StreamHealthSnapshot {
        self.health_snapshot_with_details("unknown", "running", 0, 0)
    }

    pub fn health_snapshot_with_details(
        &self,
        stream_key: impl Into<String>,
        source_state: impl Into<String>,
        last_packet_at_ms: i64,
        reconnect_count: u64,
    ) -> StreamHealthSnapshot {
        let consumers: Vec<Arc<Consumer>> = self.consumers.read().values().cloned().collect();
        let cache = self.cache.snapshot_cache();
        let gop_bytes = cache
            .gop_packets
            .iter()
            .map(|packet| packet.payload.len())
            .sum();
        StreamHealthSnapshot {
            stream_key: stream_key.into(),
            source_state: source_state.into(),
            source_epoch: self.epoch.load(Ordering::Acquire),
            last_packet_at_ms,
            reconnect_count,
            active_consumers: consumers.len(),
            max_consumers: self.limits.max_consumers_per_stream,
            gop_packets: cache.gop_packets.len(),
            gop_bytes,
            metrics: self.metrics.snapshot(),
            consumers: consumers
                .into_iter()
                .map(|consumer| ConsumerHealthSnapshot {
                    consumer_id: consumer.id,
                    name: consumer.name.clone(),
                    kind: consumer.kind,
                    queue_depth: consumer.mailbox.queue_depth(),
                    queue_capacity: consumer.mailbox.capacity,
                    dropped_packets: consumer.mailbox.dropped_packets(),
                    replay_count: consumer.mailbox.replay_count(),
                    recovering: consumer.mailbox.is_recovering(),
                    last_progress_at_ms: consumer.mailbox.last_progress_at_ms(),
                })
                .collect(),
        }
    }

    pub fn evict_stalled(&self, now_mono_ms: u64) -> usize {
        let candidates: Vec<ConsumerId> = self
            .consumers
            .read()
            .values()
            .filter(|consumer| {
                consumer.mailbox.is_recovering()
                    && now_mono_ms.saturating_sub(consumer.mailbox.last_progress_mono_ms())
                        >= self.limits.zombie_no_progress_ms
            })
            .map(|consumer| consumer.id)
            .collect();

        for id in &candidates {
            self.unsubscribe(*id);
            self.metrics
                .total_zombie_evictions
                .fetch_add(1, Ordering::Relaxed);
        }
        candidates.len()
    }
}

fn reserve_total(counter: &AtomicUsize, max: usize) -> Option<()> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= max {
            return None;
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(()),
            Err(actual) => current = actual,
        }
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

pub(crate) fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(pts_ms: i64, keyframe: bool) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe: keyframe,
            codec: CodecType::H264,
            payload: Bytes::from_static(if keyframe {
                b"\x00\x00\x00\x01\x67\x42\x00\x1e\x00\x00\x00\x01\x68\xce\x00\x00\x00\x01\x65\x88"
            } else {
                b"\x00\x00\x00\x01\x41\x01"
            }),
            ..Default::default()
        })
    }

    fn h265_packet(pts_ms: i64, keyframe: bool, valid_vps: bool) -> Arc<EncodedPacket> {
        let vps = if valid_vps {
            b"\x00\x00\x00\x01\x40\x01\xaa"
        } else {
            b"\x00\x00\x00\x01\xc0\x01\xaa"
        };
        let mut payload = Vec::with_capacity(32);
        payload.extend_from_slice(vps);
        payload.extend_from_slice(b"\x00\x00\x00\x01\x42\x01\xbb");
        payload.extend_from_slice(b"\x00\x00\x00\x01\x44\x01\xcc");
        payload.extend_from_slice(if keyframe {
            b"\x00\x00\x00\x01\x26\x01\xdd"
        } else {
            b"\x00\x00\x00\x01\x02\x01\xee"
        });
        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe: keyframe,
            codec: CodecType::H265,
            payload: Bytes::from(payload),
            ..Default::default()
        })
    }

    fn test_config() -> PreviewDistributionConfig {
        PreviewDistributionConfig {
            max_consumers_per_stream: 2,
            max_total_consumers: 4,
            preview_mailbox_capacity: 2,
            analysis_mailbox_capacity: 2,
            max_gop_packets: 4,
            max_gop_bytes: 1024,
            max_packet_bytes: 1024,
            http_merge_flush_ms: 10,
            http_merge_max_bytes: 1024,
            zombie_no_progress_ms: 1,
        }
    }

    #[test]
    fn h265_cache_requires_a_valid_vps() {
        let cache = KeyframeCacheStore::new();
        let config = test_config();
        let invalid = h265_packet(1000, true, false);
        cache.update(&invalid, 0, &config);
        assert!(cache.snapshot(&config).is_none());

        let valid = h265_packet(1040, true, true);
        cache.update(&valid, 0, &config);
        let snapshot = cache
            .snapshot(&config)
            .expect("valid VPS should recover GOP");
        assert_eq!(snapshot.codec, CodecType::H265);
        assert!(snapshot.vps.is_some());
    }

    #[tokio::test]
    async fn slow_consumer_isolated_and_replayed() {
        let dispatcher = Arc::new(PacketDispatcher::new(test_config()));
        let fast = dispatcher
            .subscribe("fast", ConsumerKind::HttpFlv)
            .expect("fast subscription");
        let slow = dispatcher
            .subscribe("slow", ConsumerKind::HttpFlv)
            .expect("slow subscription");

        dispatcher.publish(packet(1000, true));
        let _ = fast.recv().await.expect("fast keyframe");

        dispatcher.publish(packet(1040, false));
        dispatcher.publish(packet(1080, false));
        let fast_item = fast.recv().await.expect("fast packet");
        assert!(matches!(fast_item, StreamItem::Packet(_)));

        dispatcher.publish(packet(1120, false));
        let fast_item = fast.recv().await.expect("fast packet");
        assert!(matches!(fast_item, StreamItem::Packet(_)));

        let slow_item = slow.recv().await.expect("slow replay");
        assert!(matches!(slow_item, StreamItem::Replay(_)));
        assert_eq!(dispatcher.metrics.total_replays.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stalled_consumer_eviction_releases_global_lease() {
        let global = Arc::new(AtomicUsize::new(0));
        let dispatcher = Arc::new(PacketDispatcher::with_cache_and_global(
            test_config(),
            Arc::new(KeyframeCacheStore::new()),
            global.clone(),
        ));
        let subscription = dispatcher
            .subscribe("stalled", ConsumerKind::HttpFlv)
            .expect("subscription");

        dispatcher.publish(packet(1000, false));
        dispatcher.publish(packet(1040, false));
        dispatcher.publish(packet(1080, false));
        assert!(subscription.mailbox().is_recovering());

        let now = subscription.mailbox().last_progress_mono_ms() + 1;
        assert_eq!(dispatcher.evict_stalled(now), 1);
        assert_eq!(global.load(Ordering::Acquire), 0);
        assert_eq!(dispatcher.health_snapshot().active_consumers, 0);
        assert_eq!(
            dispatcher
                .metrics
                .total_zombie_evictions
                .load(Ordering::Relaxed),
            1
        );
        assert!(subscription.recv().await.is_none());
    }
    #[tokio::test]
    async fn close_wakes_waiting_consumer() {
        let dispatcher = Arc::new(PacketDispatcher::new(test_config()));
        let subscription = dispatcher
            .subscribe("waiting", ConsumerKind::WebCodecs)
            .expect("subscription");
        let mailbox = subscription.mailbox().clone();
        let waiter = tokio::spawn(async move { mailbox.recv().await });
        tokio::task::yield_now().await;
        drop(subscription);
        assert!(waiter.await.expect("waiter join").is_none());
    }

    #[tokio::test]
    async fn source_reset_clears_cache_and_notifies_consumers() {
        let dispatcher = Arc::new(PacketDispatcher::new(test_config()));
        let subscription = dispatcher
            .subscribe("reset", ConsumerKind::Analysis)
            .expect("subscription");
        dispatcher.publish(packet(1000, true));
        let epoch = dispatcher.source_reset();
        assert_eq!(epoch, 1);
        let health = dispatcher.health_snapshot();
        assert_eq!(health.source_epoch, 1);
        assert_eq!(health.gop_packets, 0);
        assert_eq!(health.active_consumers, 1);
        assert!(matches!(
            subscription.recv().await,
            Some(StreamItem::SourceReset { epoch: 1 })
        ));
        drop(subscription);
    }

    #[test]
    fn global_consumer_budget_is_shared_and_released_once() {
        let global = Arc::new(AtomicUsize::new(0));
        let mut config = test_config();
        config.max_total_consumers = 2;
        let first_dispatcher = Arc::new(PacketDispatcher::with_cache_and_global(
            config.clone(),
            Arc::new(KeyframeCacheStore::new()),
            global.clone(),
        ));
        let second_dispatcher = Arc::new(PacketDispatcher::with_cache_and_global(
            config,
            Arc::new(KeyframeCacheStore::new()),
            global.clone(),
        ));

        let first = first_dispatcher
            .subscribe("first", ConsumerKind::HttpFlv)
            .expect("first subscription");
        let second = second_dispatcher
            .subscribe("second", ConsumerKind::Analysis)
            .expect("second subscription");
        assert_eq!(global.load(Ordering::Acquire), 2);

        let error = second_dispatcher
            .subscribe("third", ConsumerKind::WebCodecs)
            .expect_err("global limit must reject");
        assert_eq!(error, DispatcherError::TooManyConsumers { max: 2 });

        drop(first);
        assert_eq!(global.load(Ordering::Acquire), 1);
        drop(second);
        assert_eq!(global.load(Ordering::Acquire), 0);

        let recovered = second_dispatcher
            .subscribe("recovered", ConsumerKind::WebCodecs)
            .expect("released global lease must be reusable");
        assert_eq!(global.load(Ordering::Acquire), 1);
        drop(recovered);
        assert_eq!(global.load(Ordering::Acquire), 0);
    }
    #[test]
    fn oversized_packet_enters_recovery_and_clears_cache() {
        let mut config = test_config();
        config.max_packet_bytes = 1;
        let dispatcher = Arc::new(PacketDispatcher::new(config));
        let subscription = dispatcher
            .subscribe("oversized", ConsumerKind::HttpFlv)
            .expect("subscription");

        dispatcher.publish(packet(1000, true));
        assert!(subscription.mailbox().is_recovering());
        assert_eq!(subscription.mailbox().queue_depth(), 0);
        assert_eq!(dispatcher.health_snapshot().gop_packets, 0);
        assert_eq!(
            dispatcher
                .metrics
                .total_oversized_packets
                .load(Ordering::Relaxed),
            1
        );
    }
    #[test]
    fn rejects_consumer_over_limit() {
        let dispatcher = Arc::new(PacketDispatcher::new(test_config()));
        let _first = dispatcher
            .subscribe("first", ConsumerKind::HttpFlv)
            .expect("first subscription");
        let _second = dispatcher
            .subscribe("second", ConsumerKind::HttpFlv)
            .expect("second subscription");
        let error = dispatcher
            .subscribe("third", ConsumerKind::HttpFlv)
            .expect_err("limit must reject");
        assert_eq!(error, DispatcherError::TooManyConsumers { max: 2 });
    }
}
