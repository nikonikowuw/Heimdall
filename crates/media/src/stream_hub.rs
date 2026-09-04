use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{broadcast, Mutex, RwLock};
use types::{EncodedPacket, TransportPolicy};

use crate::error::MediaError;
use crate::rtsp::RtspIngestor;

/// 关键帧与参数集缓存结构（提供 WHEP 秒开首包加速）
#[derive(Debug, Default, Clone)]
pub struct KeyframeCache {
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
    pub last_keyframe: Option<Bytes>,
    pub last_keyframe_pts: i64,
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
    pub ingestor_running: Arc<AtomicBool>,
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

/// 全局流媒体调度与分发中心
#[derive(Debug, Clone)]
pub struct StreamHub {
    sessions: Arc<RwLock<HashMap<String, Arc<CameraStreamSession>>>>,
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
        }
    }

    /// 注册或获取某路摄像头的流媒体会话
    pub async fn get_or_create_session(
        &self,
        camera_id: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
    ) -> Arc<CameraStreamSession> {
        let mut map = self.sessions.write().await;
        if let Some(session) = map.get(camera_id) {
            return session.clone();
        }

        // 默认创建深度为 64 的有界广播通道
        let (broadcast_tx, _) = broadcast::channel(64);
        let session = Arc::new(CameraStreamSession {
            camera_id: camera_id.to_string(),
            rtsp_url: rtsp_url.to_string(),
            transport_policy,
            active_viewers: Arc::new(AtomicUsize::new(0)),
            ai_task_enabled: Arc::new(AtomicBool::new(false)),
            keyframe_cache: Arc::new(RwLock::new(KeyframeCache::default())),
            broadcast_tx,
            cancel_signal: Arc::new(AtomicBool::new(false)),
            ingestor_running: Arc::new(AtomicBool::new(false)),
            cooldown_cancel: Arc::new(Mutex::new(None)),
            consecutive_probe_failures: Arc::new(AtomicUsize::new(0)),
        });

        map.insert(camera_id.to_string(), session.clone());
        session
    }

    /// 移除摄像头会话并停止拉流
    pub async fn remove_session(&self, camera_id: &str) {
        let mut map = self.sessions.write().await;
        if let Some(session) = map.remove(camera_id) {
            session.cancel_signal.store(true, Ordering::SeqCst);
        }
    }

    /// 检查指定摄像头当前是否正在处于活动拉流状态
    pub async fn is_streaming(&self, camera_id: &str) -> bool {
        let map = self.sessions.read().await;
        map.get(camera_id)
            .map(|s| s.ingestor_running.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// 获取某路摄像头的连续探活失败计数
    pub async fn get_failure_count(&self, camera_id: &str) -> usize {
        let map = self.sessions.read().await;
        map.get(camera_id)
            .map(|s| s.consecutive_probe_failures.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// 重置某路摄像头的探活失败计数
    pub async fn reset_failure_count(&self, camera_id: &str) {
        let map = self.sessions.read().await;
        if let Some(s) = map.get(camera_id) {
            s.consecutive_probe_failures.store(0, Ordering::SeqCst);
        }
    }

    /// 累加某路摄像头的探活失败计数
    pub async fn increment_failure_count(&self, camera_id: &str) -> usize {
        let map = self.sessions.read().await;
        if let Some(s) = map.get(camera_id) {
            s.consecutive_probe_failures.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            1
        }
    }

    /// 订阅某路摄像头的实时数据流（增加观众计数，按需唤醒拉流）
    pub async fn subscribe(
        &self,
        camera_id: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
    ) -> Result<broadcast::Receiver<Arc<EncodedPacket>>, MediaError> {
        let session = self
            .get_or_create_session(camera_id, rtsp_url, transport_policy)
            .await;

        session.active_viewers.fetch_add(1, Ordering::SeqCst);
        session.cancel_cooldown().await;

        // 若当前未在拉流，则启动拉流任务
        Self::ensure_ingestor_started(&session);

        Ok(session.broadcast_tx.subscribe())
    }

    /// 退订某路摄像头的实时数据流（减少观众计数，触发 5s 优雅冷却挂起）
    pub async fn unsubscribe(&self, camera_id: &str) {
        let map = self.sessions.read().await;
        if let Some(session) = map.get(camera_id) {
            let prev = session.active_viewers.fetch_sub(1, Ordering::SeqCst);
            if prev <= 1 && !session.ai_task_enabled.load(Ordering::SeqCst) {
                // 观众归零且 AI 未开启，启动 5 秒冷却挂起
                Self::start_cooldown_timer(session.clone()).await;
            }
        }
    }

    /// 更新 AI 分析任务的启用状态（启用时按需拉流，停用时触发冷却）
    pub async fn set_ai_enabled(
        &self,
        camera_id: &str,
        rtsp_url: &str,
        transport_policy: TransportPolicy,
        enabled: bool,
    ) {
        let session = self
            .get_or_create_session(camera_id, rtsp_url, transport_policy)
            .await;

        session.ai_task_enabled.store(enabled, Ordering::SeqCst);

        if enabled {
            session.cancel_cooldown().await;
            Self::ensure_ingestor_started(&session);
        } else if session.active_viewers.load(Ordering::SeqCst) == 0 {
            Self::start_cooldown_timer(session.clone()).await;
        }
    }

    /// 获取某路摄像头当前的秒开关键帧缓存
    pub async fn get_keyframe_cache(&self, camera_id: &str) -> Option<KeyframeCache> {
        let map = self.sessions.read().await;
        if let Some(session) = map.get(camera_id) {
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
            let ingestor = Arc::new(RtspIngestor::new(
                session.camera_id.clone(),
                session.rtsp_url.clone(),
                session.transport_policy,
                session.broadcast_tx.clone(),
            ));

            let session_clone = session.clone();
            let cancel_signal = session.cancel_signal.clone();

            // 启动数据包内部缓存监听（捕获 SPS/PPS/IDR 写入 KeyframeCache）
            let mut internal_rx = session.broadcast_tx.subscribe();
            let cache_arc = session.keyframe_cache.clone();
            tokio::spawn(async move {
                while let Ok(pkt) = internal_rx.recv().await {
                    if pkt.payload.is_empty() {
                        continue;
                    }
                    let nal_type = pkt.payload[0] & 0x1F;
                    let mut cache = cache_arc.write().await;
                    if nal_type == 7 {
                        cache.sps = Some(pkt.payload.clone());
                    } else if nal_type == 8 {
                        cache.pps = Some(pkt.payload.clone());
                    } else if pkt.is_keyframe {
                        cache.last_keyframe = Some(pkt.payload.clone());
                        cache.last_keyframe_pts = pkt.pts_ms;
                    }
                }
            });

            // 启动 RTSP 拉流循环
            tokio::spawn(async move {
                ingestor.run_loop(cancel_signal).await;
                session_clone
                    .ingestor_running
                    .store(false, Ordering::SeqCst);
            });
        }
    }

    /// 启动 5 秒静默冷却定时器
    async fn start_cooldown_timer(session: Arc<CameraStreamSession>) {
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
            }
        });

        let mut guard = session.cooldown_cancel.lock().await;
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
}
