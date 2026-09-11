use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use pipeline::{PipelineAnalysisEvent, PipelineManager, PipelineTrackEvent};
use types::{CameraTracksPayload, TrackDto, TOPIC_CAMERA_TRACKS};

use crate::state::{AppState, WsBroadcastEvent};

/// 默认航迹下发最小节流间隔 (66ms，约对应最大 15 FPS)
pub const DEFAULT_TRACK_BROADCAST_INTERVAL_MS: u64 = 66;

/// 摄像头内部状态追踪 (聚合多算法实例航迹、时间戳与边缘清空状态)
#[derive(Debug, Default)]
struct CameraTrackState {
    last_broadcast_ms: i64,
    last_had_tracks: bool,
    instance_tracks: HashMap<String, Vec<TrackDto>>,
}

/// 摄像头实时 AI 检测框与跟踪元数据流转服务
///
/// 负责订阅管线分析引擎发出的高频航迹追踪事件，并以极低开销分发：
/// 1. 视口感知按需推送 (Zero-Load when Idle)：无活跃预览时跳过序列化；
/// 2. 动态频控节流：限制单路摄像头最高下发约 15 FPS，减少高频网络小包；
/// 3. 空帧边缘清空：画面目标离开瞬间立即单次广播清空帧，消除幽灵框滞留。
#[derive(Debug, Clone)]
pub struct TrackDispatchService {
    pub pipeline: Arc<PipelineManager>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub min_broadcast_interval_ms: u64,
    camera_states: Arc<Mutex<HashMap<String, CameraTrackState>>>,
}

impl TrackDispatchService {
    /// 使用默认节流参数构造服务实例
    pub fn new(
        pipeline: Arc<PipelineManager>,
        event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self::with_interval(
            pipeline,
            event_broadcaster,
            shutdown_tx,
            DEFAULT_TRACK_BROADCAST_INTERVAL_MS,
        )
    }

    /// 指定最小节流毫秒间隔构造服务实例
    pub fn with_interval(
        pipeline: Arc<PipelineManager>,
        event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
        shutdown_tx: broadcast::Sender<()>,
        min_broadcast_interval_ms: u64,
    ) -> Self {
        Self {
            pipeline,
            event_broadcaster,
            shutdown_tx,
            min_broadcast_interval_ms,
            camera_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 从 `AppState` 中提取句柄构造服务实例
    pub fn from_state(state: &AppState) -> Self {
        Self::new(
            state.pipeline.clone(),
            state.event_broadcaster.clone(),
            state.shutdown_tx.clone(),
        )
    }

    /// 处理单次航迹更新事件并决定是否广播
    ///
    /// 返回 `true` 表示生成并广播了 WebSocket 消息，`false` 表示被节流、抑制或视口静默。
    pub async fn handle_track_event(&self, event: &PipelineTrackEvent) -> bool {
        // 1. 视口按需检查：若无活跃预览客户端，完全跳过 JSON 序列化与广播开销
        if !self
            .pipeline
            .has_preview_subscribers(&event.camera_id)
            .await
        {
            return false;
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let dtos: Vec<TrackDto> = event.tracks.iter().map(TrackDto::from).collect();

        let (should_send, tracks_to_send) = {
            let mut states = self.camera_states.lock().unwrap_or_else(|p| p.into_inner());
            let state = states.entry(event.camera_id.clone()).or_default();

            if dtos.is_empty() {
                state.instance_tracks.remove(&event.algorithm_id);
            } else {
                state
                    .instance_tracks
                    .insert(event.algorithm_id.clone(), dtos);
            }

            // 聚合当前摄像头所有活跃算法实例的追踪框，防止多算法并发时相互覆盖或误触发清空
            let all_tracks: Vec<TrackDto> =
                state.instance_tracks.values().flatten().cloned().collect();
            let has_tracks = !all_tracks.is_empty();

            if !has_tracks {
                if state.last_had_tracks {
                    // 边缘触发：上一帧有目标、当前所有算法实例均无目标，必须立刻下发一次清空，清除残留旧框
                    state.last_had_tracks = false;
                    state.last_broadcast_ms = now_ms;
                    (true, Vec::new())
                } else {
                    // 持续无目标状态，保持静默，不重复发送空帧
                    (false, Vec::new())
                }
            } else {
                // 有目标状态：检查节流时间间隔
                let elapsed = now_ms.saturating_sub(state.last_broadcast_ms) as u64;
                if elapsed >= self.min_broadcast_interval_ms {
                    state.last_had_tracks = true;
                    state.last_broadcast_ms = now_ms;
                    (true, all_tracks)
                } else {
                    // 处于节流周期内，丢弃此帧
                    (false, Vec::new())
                }
            }
        };

        if !should_send {
            return false;
        }

        let payload = CameraTracksPayload {
            camera_id: event.camera_id.clone(),
            timestamp: event.timestamp,
            tracks: tracks_to_send,
        };

        if let Ok(payload_value) = serde_json::to_value(&payload) {
            let ws_event = WsBroadcastEvent {
                topic: TOPIC_CAMERA_TRACKS.to_string(),
                payload: payload_value,
                timestamp: now_ms,
            };
            let _ = self.event_broadcaster.send(ws_event);
            return true;
        }

        false
    }

    /// 启动后台常驻工作线程
    pub fn start_worker(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let mut analysis_rx = self.pipeline.subscribe_analysis_events();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            tracing::info!("后台航迹实时流分发与节流工作线程已启动");

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("接收到系统停机信号，航迹分发工作线程准备优雅退出");
                        break;
                    }
                    recv_res = analysis_rx.recv() => {
                        match recv_res {
                            Ok(PipelineAnalysisEvent::Tracks(track_evt)) => {
                                self.handle_track_event(&track_evt).await;
                            }
                            Ok(PipelineAnalysisEvent::Alarm(_)) => {
                                // 告警事件由 AlarmDispatchService 处理
                            }
                            Ok(PipelineAnalysisEvent::Capture(_)) => {
                                // 客观通行抓拍事件由 CaptureDispatchService 处理
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::debug!(skipped, "分析事件广播通道滞后，跳过过旧航迹帧");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("分析事件广播通道已关闭，航迹分发工作线程平稳退出");
                                break;
                            }
                        }
                    }
                }
            }

            tracing::info!("后台航迹实时流分发与节流工作线程已安全停止");
        })
    }
}
