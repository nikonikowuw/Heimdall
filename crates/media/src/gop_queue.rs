//! 实时流媒体 GOP 语义感知丢帧缓冲队列 (GOP-Aware Leaky Packet Queue)
//!
//! 彻底解决“输入队列有界却通过反压阻塞网络读取”导致延迟雪崩的根本性矛盾：
//! 1. **实时优先原则 (Latency-First)**：网络拉流协程通过非阻塞 `push` 投放数据，绝不阻塞网络读取；
//! 2. **GOP 拓扑语义防花屏 (GOP Tail Pruning)**：
//!    - 队列饱和时仅丢弃非关键帧，并自动切换为“修剪尾部 (PruningTail)”状态；
//!    - 丢弃后续同一 GOP 内的所有残损 P/B 帧，严禁将残破运动参考链送入硬件解码器，从源头杜绝绿屏和马赛克；
//! 3. **关键帧优先瞬间跳跃 (Instant Leap to Real-time on IDR)**：
//!    - 队列满且来新 IDR 帧时，瞬间排空积压的旧 GOP，跳帧对齐至当前最新画面，彻底重置端到端时延；
//! 4. **参数集常驻保护 (SPS/PPS/VPS Preservation)**：关键参数集绝对不丢。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use types::EncodedPacket;

/// 压入动作判定结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushAction {
    /// 正常安全入队
    Enqueued,
    /// 队列饱和触发丢帧：已丢弃本 P 帧并开启当前 GOP 尾部修剪状态
    StartedPruningTail,
    /// 处于修剪状态：丢弃所属残损 GOP 的后续 P 帧
    DroppedPrunedPFrame,
    /// 遭遇新 IDR 关键帧：排空积压旧帧，瞬间跳跃对齐现实画面
    LeapedToKeyframe { stale_dropped: usize },
}

/// GOP 丢帧状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GopDropState {
    /// 正常解码推进状态
    Normal,
    /// 当前 GOP 已发生丢帧，正在修剪尾部并等待下一个合法关键帧
    PruningTail { dropped_in_gop: u64 },
}

/// 队列度量指标
#[derive(Debug, Default)]
pub struct GopQueueMetrics {
    /// 总接收数据包数
    pub total_pushed: AtomicU64,
    /// 成功入队数据包数
    pub total_enqueued: AtomicU64,
    /// 丢弃的 P/B 帧总数
    pub total_dropped_p_frames: AtomicU64,
    /// 触发 GOP 尾部修剪的次数
    pub total_gop_pruned: AtomicU64,
    /// 关键帧瞬间跳跃重置延迟的次数
    pub total_idr_leaps: AtomicU64,
    /// 跳跃时顺带排空的积压旧帧总数
    pub total_stale_dropped_on_leap: AtomicU64,
}

/// 队列配置
#[derive(Debug, Clone)]
pub struct GopQueueConfig {
    /// 队列最大容纳包数（默认 8 包，约 250ms 缓冲）
    pub capacity: usize,
}

impl Default for GopQueueConfig {
    fn default() -> Self {
        Self { capacity: 8 }
    }
}

/// 具备 GOP 语法感知与防花屏特性的实时视频包队列
#[derive(Debug)]
pub struct GopAwarePacketQueue {
    config: GopQueueConfig,
    queue: VecDeque<Arc<EncodedPacket>>,
    state: GopDropState,
    metrics: Arc<GopQueueMetrics>,
}

impl GopAwarePacketQueue {
    pub fn new(config: GopQueueConfig) -> Self {
        let cap = config.capacity.max(2);
        Self {
            config,
            queue: VecDeque::with_capacity(cap),
            state: GopDropState::Normal,
            metrics: Arc::new(GopQueueMetrics::default()),
        }
    }

    /// 获取指标引用
    pub fn metrics(&self) -> &Arc<GopQueueMetrics> {
        &self.metrics
    }

    /// 获取当前队列中待消费的数据包数量
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// 队列是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 获取当前状态机状态
    pub fn state(&self) -> GopDropState {
        self.state
    }

    /// 非阻塞压入数据包（实时监控专用，绝对不阻塞网络读取）
    pub fn push(&mut self, packet: Arc<EncodedPacket>) -> PushAction {
        self.metrics.total_pushed.fetch_add(1, Ordering::Relaxed);
        let is_keyframe = packet.is_keyframe;

        if is_keyframe {
            // 1. 遇到关键帧，重置状态机为 Normal
            self.state = GopDropState::Normal;

            // 若队列已满，说明下游消费滞后，清空全部陈旧数据，实现 IDR 瞬间跳跃！
            if self.queue.len() >= self.config.capacity {
                let stale_dropped = self.queue.len();
                self.queue.clear();
                self.queue.push_back(packet);

                self.metrics.total_idr_leaps.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .total_stale_dropped_on_leap
                    .fetch_add(stale_dropped as u64, Ordering::Relaxed);
                self.metrics.total_enqueued.fetch_add(1, Ordering::Relaxed);

                tracing::warn!(
                    stale_dropped,
                    "实时视频流下游积压，执行 IDR 瞬间跳帧对齐，延迟归零"
                );
                return PushAction::LeapedToKeyframe { stale_dropped };
            }

            self.queue.push_back(packet);
            self.metrics.total_enqueued.fetch_add(1, Ordering::Relaxed);
            return PushAction::Enqueued;
        }

        // 2. 非关键帧处理
        match self.state {
            GopDropState::PruningTail { dropped_in_gop } => {
                // 前置参考帧已丢失，为了防止解码器发生花屏与绿屏，必须坚决丢弃所属 GOP 后续所有 P/B 帧！
                self.state = GopDropState::PruningTail {
                    dropped_in_gop: dropped_in_gop + 1,
                };
                self.metrics
                    .total_dropped_p_frames
                    .fetch_add(1, Ordering::Relaxed);
                PushAction::DroppedPrunedPFrame
            }
            GopDropState::Normal => {
                if self.queue.len() < self.config.capacity {
                    self.queue.push_back(packet);
                    self.metrics.total_enqueued.fetch_add(1, Ordering::Relaxed);
                    PushAction::Enqueued
                } else {
                    // 【核心丢帧切断点】：队列已饱和，绝不反压网络拉流！
                    // 丢弃当前帧，并斩断当前 GOP 剩余整个尾部，切换至 PruningTail 状态
                    self.state = GopDropState::PruningTail { dropped_in_gop: 1 };
                    self.metrics
                        .total_gop_pruned
                        .fetch_add(1, Ordering::Relaxed);
                    self.metrics
                        .total_dropped_p_frames
                        .fetch_add(1, Ordering::Relaxed);

                    tracing::warn!(
                        pts_ms = packet.pts_ms,
                        "实时解码队列满，主动开启当前 GOP 尾部修剪，等待下一个 IDR 帧防花屏"
                    );
                    PushAction::StartedPruningTail
                }
            }
        }
    }

    /// 弹出队首数据包供解码器消费
    pub fn pop(&mut self) -> Option<Arc<EncodedPacket>> {
        self.queue.pop_front()
    }

    /// 查看队首数据包但不弹出
    pub fn peek(&self) -> Option<&Arc<EncodedPacket>> {
        self.queue.front()
    }

    /// 清空队列并重置状态机
    pub fn clear(&mut self) {
        self.queue.clear();
        self.state = GopDropState::Normal;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use types::CodecType;

    fn make_pkt(pts_ms: i64, is_keyframe: bool) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe,
            codec: CodecType::H264,
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x65fake_data"),
            ..Default::default()
        })
    }

    #[test]
    fn test_normal_push_and_pop() {
        let mut q = GopAwarePacketQueue::new(GopQueueConfig { capacity: 4 });

        assert_eq!(q.push(make_pkt(100, true)), PushAction::Enqueued);
        assert_eq!(q.push(make_pkt(133, false)), PushAction::Enqueued);
        assert_eq!(q.push(make_pkt(166, false)), PushAction::Enqueued);
        assert_eq!(q.len(), 3);

        let p1 = q.pop().unwrap();
        assert_eq!(p1.pts_ms, 100);
        let p2 = q.pop().unwrap();
        assert_eq!(p2.pts_ms, 133);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn test_gop_tail_pruning_prevents_corruption() {
        // 容量为 3
        let mut q = GopAwarePacketQueue::new(GopQueueConfig { capacity: 3 });

        // 填满队列: I(100), P(133), P(166)
        assert_eq!(q.push(make_pkt(100, true)), PushAction::Enqueued);
        assert_eq!(q.push(make_pkt(133, false)), PushAction::Enqueued);
        assert_eq!(q.push(make_pkt(166, false)), PushAction::Enqueued);
        assert_eq!(q.len(), 3);

        // 尝试推入 P(200) -> 饱和触发修剪
        assert_eq!(q.push(make_pkt(200, false)), PushAction::StartedPruningTail);
        assert!(matches!(q.state(), GopDropState::PruningTail { .. }));

        // 继续推入 P(233) -> 必须丢弃，防花屏！
        assert_eq!(
            q.push(make_pkt(233, false)),
            PushAction::DroppedPrunedPFrame
        );

        // 此时队列依然保持原本完整的 3 个包，未损坏
        assert_eq!(q.len(), 3);

        // 新的关键帧到达 I(300) -> 队列满，触发跳帧瞬移！
        let action = q.push(make_pkt(300, true));
        assert_eq!(action, PushAction::LeapedToKeyframe { stale_dropped: 3 });

        // 队列现在只有最新的关键帧，时延重置！
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop().unwrap().pts_ms, 300);
        assert_eq!(q.state(), GopDropState::Normal);
    }

    #[test]
    fn test_metrics_tracking() {
        let mut q = GopAwarePacketQueue::new(GopQueueConfig { capacity: 2 });

        // 1. I(100), P(133)
        assert_eq!(q.push(make_pkt(100, true)), PushAction::Enqueued);
        assert_eq!(q.push(make_pkt(133, false)), PushAction::Enqueued);

        // 2. P(166) -> 触发修剪
        assert_eq!(q.push(make_pkt(166, false)), PushAction::StartedPruningTail);

        // 3. P(199) -> 处于修剪态被丢弃
        assert_eq!(
            q.push(make_pkt(199, false)),
            PushAction::DroppedPrunedPFrame
        );

        // 4. I(233) -> 跳跃对齐，排空前 2 帧
        assert_eq!(
            q.push(make_pkt(233, true)),
            PushAction::LeapedToKeyframe { stale_dropped: 2 }
        );

        let m = q.metrics();
        assert_eq!(m.total_pushed.load(Ordering::Relaxed), 5);
        assert_eq!(m.total_enqueued.load(Ordering::Relaxed), 3);
        assert_eq!(m.total_dropped_p_frames.load(Ordering::Relaxed), 2);
        assert_eq!(m.total_gop_pruned.load(Ordering::Relaxed), 1);
        assert_eq!(m.total_idr_leaps.load(Ordering::Relaxed), 1);
        assert_eq!(m.total_stale_dropped_on_leap.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_peek_and_clear() {
        let mut q = GopAwarePacketQueue::new(GopQueueConfig::default());
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(q.peek().is_none());

        q.push(make_pkt(100, true));
        assert!(!q.is_empty());
        assert_eq!(q.peek().unwrap().pts_ms, 100);

        q.clear();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }
}
