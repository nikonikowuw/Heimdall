use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{broadcast, Mutex, RwLock};
use types::{CodecType, EncodedPacket, TransportPolicy};

use crate::error::MediaError;
use crate::retina_ingest::RetinaIngestor;
use crate::rtsp::parse_and_clean_rtsp_url;

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

/// 关键帧与参数集缓存结构（提供秒开首包加速与 GOP 完整参考链）
#[derive(Debug, Default, Clone)]
pub struct KeyframeCache {
    pub codec: Option<CodecType>,
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
    pub last_keyframe: Option<Bytes>,
    pub last_keyframe_pts: i64,
    pub gop_packets: Vec<Arc<EncodedPacket>>,
}

/// 单路摄像头的流媒体运行时会话
#[derive(Debug)]
pub struct CameraStreamSession {
    pub camera_id: String,
    pub rtsp_url: String,
    pub transport_policy: TransportPolicy,
    pub active_viewers: Arc<AtomicUsize>,
    pub ai_task_enabled: Arc<AtomicBool>,
    pub keyframe_cache: Arc<RwLock<KeyframeCache>>,
    pub broadcast_tx: broadcast::Sender<Arc<EncodedPacket>>,
    pub cancel_signal: Arc<AtomicBool>,
    pub cancel_tx: tokio::sync::watch::Sender<bool>,
    pub cancel_rx: tokio::sync::watch::Receiver<bool>,
    pub ingestor_running: Arc<AtomicBool>,
    pub last_packet_time: Arc<AtomicI64>,
    pub cooldown_cancel: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub consecutive_probe_failures: Arc<AtomicUsize>,
}

impl CameraStreamSession {
    pub async fn cancel_cooldown(&self) {
        let mut guard = self.cooldown_cancel.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }
}

/// 全局流媒体调度与分发中心 (支持按规范化 RTSP URL 跨设备物理复用)
#[derive(Debug, Clone)]
pub struct StreamHub {
    /// 物理会话映射：canonical_url -> Arc<CameraStreamSession>
    sessions: Arc<RwLock<HashMap<String, Arc<CameraStreamSession>>>>,
    /// 业务 key 到物理规范化 URL 映射：stream_key (如 "cam1:main", "cam2:main") -> canonical_url
    key_to_url: Arc<RwLock<HashMap<String, String>>>,
    /// 规范化 URL 关联的业务 key 集合：canonical_url -> HashSet<String>
    url_to_keys: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// 摄像机探活失败计数缓存 (按业务 camera_id 独立隔离)
    probe_failures: Arc<RwLock<HashMap<String, usize>>>,
}

impl Default for StreamHub {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamHub {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            key_to_url: Arc::new(RwLock::new(HashMap::new())),
            url_to_keys: Arc::new(RwLock::new(HashMap::new())),
            probe_failures: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 将业务 stream_key 或原始 URL 解析为底层的规范化 URL
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

    /// 注册或获取某路摄像头的流媒体会话（基于规范化 RTSP URL 自动跨设备复用底层连接）
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

        // 1. 维护业务 key 到底层规范化 URL 的双向映射
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

        // 2. 检查底层物理 Session 是否已存在（若已存在则直接复用）
        let mut map = self.sessions.write().await;
        if let Some(session) = map.get(&effective_url) {
            return session.clone();
        }

        // 3. 首次接入物理流：创建深度为 64 的有界广播通道并存入 sessions
        let (broadcast_tx, _) = broadcast::channel(64);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let session = Arc::new(CameraStreamSession {
            camera_id: stream_key.to_string(),
            rtsp_url: effective_url.clone(),
            transport_policy,
            active_viewers: Arc::new(AtomicUsize::new(0)),
            ai_task_enabled: Arc::new(AtomicBool::new(false)),
            keyframe_cache: Arc::new(RwLock::new(KeyframeCache::default())),
            broadcast_tx,
            cancel_signal: Arc::new(AtomicBool::new(false)),
            cancel_tx,
            cancel_rx,
            ingestor_running: Arc::new(AtomicBool::new(false)),
            last_packet_time: Arc::new(AtomicI64::new(0)),
            cooldown_cancel: Arc::new(Mutex::new(None)),
            consecutive_probe_failures: Arc::new(AtomicUsize::new(0)),
        });

        map.insert(effective_url, session.clone());
        session
    }

    /// 移除摄像头会话关联。若底层物理流仍被其他设备绑定或仍有活跃观众，则保持物理流运行
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

        // 检查受影响的物理流：若既无其他绑定设备，又无活跃观众，才真正销毁物理拉流会话
        let mut map = self.sessions.write().await;
        let url_map = self.url_to_keys.read().await;
        for url in affected_urls {
            let still_bound = url_map.get(&url).map(|s| !s.is_empty()).unwrap_or(false);
            if !still_bound {
                if let Some(session) = map.get(&url) {
                    if session.active_viewers.load(Ordering::SeqCst) == 0 {
                        if let Some(s) = map.remove(&url) {
                            s.cancel_signal.store(true, Ordering::SeqCst);
                            let _ = s.cancel_tx.send(true);
                            s.cancel_cooldown().await;
                        }
                    }
                }
            }
        }
    }

    /// 检查指定业务流或物理流当前是否处于活动拉流状态
    pub async fn is_streaming(&self, key_or_url: &str) -> bool {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        map.get(&canonical)
            .map(|s| s.ingestor_running.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// 检查指定流当前是否健康持续接收数据包（指定最大包间隔毫秒）
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

    /// 获取某路摄像头的连续探活失败计数
    pub async fn get_failure_count(&self, camera_id: &str) -> usize {
        let map = self.probe_failures.read().await;
        map.get(camera_id).copied().unwrap_or(0)
    }

    /// 重置某路摄像头的探活失败计数
    pub async fn reset_failure_count(&self, camera_id: &str) {
        let mut map = self.probe_failures.write().await;
        map.insert(camera_id.to_string(), 0);
    }

    /// 累加某路摄像头的探活失败计数
    pub async fn increment_failure_count(&self, camera_id: &str) -> usize {
        let mut map = self.probe_failures.write().await;
        let count = map.entry(camera_id.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// 订阅某路流的实时数据（增加观众计数，按需唤醒拉流，底层自动复用同 URL 物理连接）
    pub async fn subscribe(
        &self,
        stream_key: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
    ) -> Result<broadcast::Receiver<Arc<EncodedPacket>>, MediaError> {
        let session = self
            .get_or_create_session(stream_key, rtsp_url, transport_policy)
            .await;

        session.active_viewers.fetch_add(1, Ordering::SeqCst);
        session.cancel_cooldown().await;

        // 若当前未在拉流，则启动拉流任务
        Self::ensure_ingestor_started(&session);

        Ok(session.broadcast_tx.subscribe())
    }

    /// 退订某路流的实时数据（减少观众计数，触发 5s 优雅冷却挂起）
    pub async fn unsubscribe(&self, key_or_url: &str) {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        if let Some(session) = map.get(&canonical) {
            let mut current = session.active_viewers.load(Ordering::SeqCst);
            let new_val = loop {
                let target = current.saturating_sub(1);
                match session.active_viewers.compare_exchange_weak(
                    current,
                    target,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break target,
                    Err(actual) => current = actual,
                }
            };
            if new_val == 0 && !session.ai_task_enabled.load(Ordering::SeqCst) {
                // 观众归零且 AI 未开启，启动 5 秒冷却挂起
                Self::start_cooldown_timer(session.clone()).await;
            }
        }
    }

    /// 更新 AI 分析任务的启用状态（启用时按需拉流，停用时触发冷却）
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

        session.ai_task_enabled.store(enabled, Ordering::SeqCst);

        if enabled {
            session.cancel_cooldown().await;
            Self::ensure_ingestor_started(&session);
        } else if session.active_viewers.load(Ordering::SeqCst) == 0 {
            Self::start_cooldown_timer(session.clone()).await;
        }
    }

    /// 获取某路流当前的秒开关键帧与 GOP 缓存
    pub async fn get_keyframe_cache(&self, key_or_url: &str) -> Option<KeyframeCache> {
        let canonical = self.resolve_canonical_url(key_or_url).await;
        let map = self.sessions.read().await;
        if let Some(session) = map.get(&canonical) {
            let cache = session.keyframe_cache.read().await;
            Some(cache.clone())
        } else {
            None
        }
    }

    /// 确保后台 RTSP 拉流协程处于运行状态
    fn ensure_ingestor_started(session: &Arc<CameraStreamSession>) {
        if session
            .ingestor_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            session.cancel_signal.store(false, Ordering::SeqCst);
            let _ = session.cancel_tx.send(false);
            let ingestor = Arc::new(RetinaIngestor::new(
                session.camera_id.clone(),
                session.rtsp_url.clone(),
                session.transport_policy,
                session.broadcast_tx.clone(),
            ));

            let session_clone = session.clone();
            let cancel_signal = session.cancel_signal.clone();
            let cancel_rx = session.cancel_rx.clone();

            // 启动数据包内部缓存监听（捕获 SPS/PPS/IDR 写入 KeyframeCache 并刷新时间戳）
            // 明确处理 RecvError::Lagged，避免因消费端落后导致内部关键帧缓存监听器意外退出
            let mut internal_rx = session.broadcast_tx.subscribe();
            let cache_arc = session.keyframe_cache.clone();
            let last_pkt_time = session.last_packet_time.clone();
            let mut internal_cancel_rx = session.cancel_rx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        change_res = internal_cancel_rx.changed() => {
                            if change_res.is_err() || *internal_cancel_rx.borrow() {
                                break;
                            }
                        }
                        recv_res = internal_rx.recv() => {
                            match recv_res {
                                Ok(pkt) => {
                                    last_pkt_time.store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                                    let nalus = crate::sps::split_annex_b_nalus(&pkt.payload);
                                    if nalus.is_empty() {
                                        continue;
                                    }
                                    let mut cache = cache_arc.write().await;
                                    cache.codec = Some(pkt.codec);

                                    // 维护参考链闭环的 GOP 环形缓冲（提供零等待秒开与参考帧完整性）
                                    if pkt.is_keyframe {
                                        cache.gop_packets.clear();
                                        cache.gop_packets.push(pkt.clone());
                                        cache.last_keyframe = Some(pkt.payload.clone());
                                        cache.last_keyframe_pts = pkt.pts_ms;
                                    } else if !cache.gop_packets.is_empty() && cache.gop_packets.len() < 75 {
                                        cache.gop_packets.push(pkt.clone());
                                    }

                                    // 遍历数据包中可能复合包含的全部 NALU 单元，精准提取参数集
                                    for nalu in nalus {
                                        match pkt.codec {
                                            CodecType::H265 if nalu.len() >= 2 => {
                                                match (nalu[0] >> 1) & 0x3F {
                                                    32 => cache.vps = Some(Bytes::copy_from_slice(nalu)),
                                                    33 => cache.sps = Some(Bytes::copy_from_slice(nalu)),
                                                    34 => cache.pps = Some(Bytes::copy_from_slice(nalu)),
                                                    _ => {}
                                                }
                                            }
                                            CodecType::H264 if !nalu.is_empty() => {
                                                match nalu[0] & 0x1F {
                                                    7 => cache.sps = Some(Bytes::copy_from_slice(nalu)),
                                                    8 => cache.pps = Some(Bytes::copy_from_slice(nalu)),
                                                    _ => {}
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                    tracing::warn!(skipped, "StreamHub 内部缓存监听器落后 (Lagged)，继续接收后续数据包");
                                    continue;
                                }
                                Err(broadcast::error::RecvError::Closed) => {
                                    break;
                                }
                            }
                        }
                    }
                }
            });

            // 启动 RTSP 拉流循环
            tokio::spawn(async move {
                ingestor.run_loop(cancel_signal, cancel_rx).await;
                session_clone
                    .ingestor_running
                    .store(false, Ordering::SeqCst);
            });
        }
    }

    /// 启动 5 秒静默冷却定时器（在互斥保护下原子替换旧 handle，杜绝并发竞态丢失取消）
    async fn start_cooldown_timer(session: Arc<CameraStreamSession>) {
        let mut guard = session.cooldown_cancel.lock().await;
        if let Some(old_handle) = guard.take() {
            old_handle.abort();
        }

        let session_clone = session.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // 5 秒冷却到期后再次检查
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_hub_lifecycle_and_ref_count() {
        let hub = StreamHub::new();
        let cam_id = "test-cam-01";
        let rtsp_url = "rtsp://127.0.0.1:8554/live";

        // 1. 创建会话
        let session = hub
            .get_or_create_session(cam_id, rtsp_url, TransportPolicy::Tcp)
            .await;
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 0);
        assert!(!session.ai_task_enabled.load(Ordering::SeqCst));

        // 2. 订阅
        let _rx = hub
            .subscribe(cam_id, rtsp_url, TransportPolicy::Tcp)
            .await
            .expect("subscribe");
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 1);

        // 3. 退订
        hub.unsubscribe(cam_id).await;
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 0);

        // 4. 重复退订测试饱和减法防护，确保不下溢为 usize::MAX
        hub.unsubscribe(cam_id).await;
        hub.unsubscribe(cam_id).await;
        assert_eq!(session.active_viewers.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_keyframe_cache_storage() {
        let hub = StreamHub::new();
        let cam_id = "test-cam-02";
        let rtsp_url = "rtsp://127.0.0.1:8554/live2";

        let session = hub
            .get_or_create_session(cam_id, rtsp_url, TransportPolicy::Auto)
            .await;

        {
            let mut cache = session.keyframe_cache.write().await;
            cache.sps = Some(Bytes::from_static(&[0x67, 0x42, 0x00]));
            cache.pps = Some(Bytes::from_static(&[0x68, 0xCE]));
            cache.last_keyframe = Some(Bytes::from_static(&[0x65, 0x88, 0x00]));
            cache.last_keyframe_pts = 1000;
        }

        let fetched = hub.get_keyframe_cache(cam_id).await.expect("fetch cache");
        assert_eq!(fetched.sps.as_deref(), Some(&[0x67, 0x42, 0x00][..]));
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

        StreamHub::ensure_ingestor_started(&session);

        let compound_payload = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, // PPS
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, // IDR
        ];
        let pkt = Arc::new(EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::copy_from_slice(&compound_payload),
        });

        session.broadcast_tx.send(pkt).expect("send packet");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let cache = session.keyframe_cache.read().await;
        assert_eq!(cache.codec, Some(CodecType::H264));
        assert_eq!(cache.sps.as_deref(), Some(&[0x67, 0x42, 0x00, 0x1E][..]));
        assert_eq!(cache.pps.as_deref(), Some(&[0x68, 0xCE][..]));
        assert_eq!(cache.last_keyframe_pts, 1000);
        assert_eq!(cache.gop_packets.len(), 1);
    }

    #[test]
    fn test_canonicalize_rtsp_url() {
        // 缺省端口自动补齐 :554
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100/Streaming/Channels/101"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        // 显式带 :554 与缺省 :554 规整为相同格式
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        // 末尾斜杠自动剥离
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:12345@192.168.1.100/Streaming/Channels/101/"),
            "rtsp://admin:12345@192.168.1.100:554/Streaming/Channels/101"
        );
        // 大写协议前缀自动转小写
        assert_eq!(
            canonicalize_rtsp_url("RTSP://192.168.1.50/live"),
            "rtsp://192.168.1.50:554/live"
        );
        // 复杂保留字符（密码中含 @, :, # 等）规范化
        assert_eq!(
            canonicalize_rtsp_url("rtsp://admin:p@ss:word#123@192.168.1.100/live/"),
            "rtsp://admin:p@ss:word#123@192.168.1.100:554/live"
        );
    }

    #[tokio::test]
    async fn test_stream_hub_url_multiplexing_and_ref_count() {
        let hub = StreamHub::new();
        // 两个不同的业务相机绑定同一个物理 RTSP 地址（格式微调）
        let url_a = "rtsp://admin:12345@192.168.1.100/live/ch1";
        let url_b = "rtsp://admin:12345@192.168.1.100:554/live/ch1/";

        let session_a = hub
            .get_or_create_session("cam-a:main", url_a, TransportPolicy::Tcp)
            .await;
        let session_b = hub
            .get_or_create_session("cam-b:main", url_b, TransportPolicy::Tcp)
            .await;

        // 验证底层物理会话完全相同（指针指向同一对象）
        assert!(Arc::ptr_eq(&session_a, &session_b));

        // 订阅 cam-a，总观众数累加至 1
        let _rx_a = hub
            .subscribe("cam-a:main", url_a, TransportPolicy::Tcp)
            .await
            .expect("subscribe cam-a");
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 1);

        // 订阅 cam-b，总观众数累加至 2
        let _rx_b = hub
            .subscribe("cam-b:main", url_b, TransportPolicy::Tcp)
            .await
            .expect("subscribe cam-b");
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 2);

        // 退订 cam-a，总观众数减至 1，底层物理会话不受影响
        hub.unsubscribe("cam-a:main").await;
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 1);

        // 移除 cam-a 绑定：由于 cam-b 依然引用且 active_viewers > 0，物理会话继续存活
        hub.remove_session("cam-a").await;
        assert!(
            hub.is_streaming("cam-b:main").await || !session_a.cancel_signal.load(Ordering::SeqCst)
        );

        // 退订 cam-b，总观众数归零
        hub.unsubscribe("cam-b:main").await;
        assert_eq!(session_a.active_viewers.load(Ordering::SeqCst), 0);
    }
}
