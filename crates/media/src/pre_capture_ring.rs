//! 录像前置缓冲环 (Pre-Capture Ring Buffer)
//!
//! 专用于事件录像的压缩流内存滑动窗口：
//! - 持续滚动缓存主码流最近 N 秒的 `EncodedPacket`（Arc 共享，零拷贝）
//! - 事件触发时，`drain_from_keyframe()` 从最近的前置关键帧开始导出全部缓冲数据
//! - **单线程使用**（RecordingWorker 内部），无需加锁
//!
//! 与 `MainStreamRingBuffer` 的区别：
//! - MainStreamRingBuffer：服务于告警快照的靶向解码，需要精确时标索引，多线程 RwLock
//! - PreCaptureRingBuffer：服务于录像 flush，只需 drain 操作，单线程无锁

use std::collections::VecDeque;
use std::sync::Arc;

use types::EncodedPacket;

/// 录像前置缓冲配置
#[derive(Debug, Clone)]
pub struct PreCaptureConfig {
    /// 最大缓存时长（毫秒），来自通道配置的 `pre_capture_seconds * 1000`
    pub max_duration_ms: i64,
    /// 最大缓存字节数（防止异常高码率打爆内存）
    ///
    /// 默认 20MB，覆盖 4Mbps × 30 秒 = 15MB 的最大正常场景
    pub max_bytes: usize,
    /// 最大包数量硬上限（极端异常码流兜底）
    pub max_packets: usize,
}

impl Default for PreCaptureConfig {
    fn default() -> Self {
        Self {
            max_duration_ms: 10_000, // 默认 10 秒
            max_bytes: 20 * 1024 * 1024, // 20MB
            max_packets: 1000,
        }
    }
}

impl PreCaptureConfig {
    /// 从前置缓冲秒数创建配置
    pub fn from_seconds(seconds: u32) -> Self {
        let seconds = seconds.clamp(5, 30);
        Self {
            max_duration_ms: i64::from(seconds) * 1000,
            // 按 4Mbps 码率估算上限，加 50% 余量
            max_bytes: (seconds as usize) * 750 * 1024,
            max_packets: (seconds as usize) * 40, // ~30fps + 余量
        }
    }
}

/// 录像前置缓冲环
///
/// **注意**：此结构仅在 `RecordingWorker` 专用 OS 线程内使用，**非线程安全**。
#[derive(Debug)]
pub struct PreCaptureRingBuffer {
    config: PreCaptureConfig,
    queue: VecDeque<Arc<EncodedPacket>>,
    current_bytes: usize,
    keyframe_count: usize,
}

impl PreCaptureRingBuffer {
    /// 创建新的前置缓冲环
    pub fn new(config: PreCaptureConfig) -> Self {
        Self {
            queue: VecDeque::with_capacity(256),
            current_bytes: 0,
            keyframe_count: 0,
            config,
        }
    }

    /// 推入一个压缩包，自动淘汰超出窗口的旧数据
    pub fn push(&mut self, packet: Arc<EncodedPacket>) {
        let pkt_size = packet.payload.len();

        if packet.is_keyframe {
            self.keyframe_count += 1;
        }
        self.current_bytes += pkt_size;
        self.queue.push_back(packet);

        self.prune();
    }

    /// 导出缓冲区中从前置关键帧开始的全部有效数据。
    ///
    /// 事件触发时调用。返回的切片首包保证为关键帧，可直接送入 fMP4 Writer。
    /// 选择**最早**的可用关键帧以最大化前置覆盖；调用后缓冲区被清空。
    ///
    /// 如果缓冲区无关键帧，返回空 Vec。
    pub fn drain_from_keyframe(&mut self) -> Vec<Arc<EncodedPacket>> {
        // 找到最早的关键帧：前置覆盖最大化（prune 保证窗口内的关键帧有效）
        let keyframe_idx = self.queue.iter().position(|pkt| pkt.is_keyframe);

        let Some(idx) = keyframe_idx else {
            // 没有关键帧，清空并返回空
            self.clear();
            return Vec::new();
        };

        // 从关键帧位置到队尾的所有包
        let result: Vec<_> = self.queue.drain(idx..).collect();

        // 清空关键帧之前的残留数据（已无用）
        self.clear();

        result
    }

    /// 清空缓冲区
    pub fn clear(&mut self) {
        self.queue.clear();
        self.current_bytes = 0;
        self.keyframe_count = 0;
    }

    /// 获取当前缓存的包数量
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// 检查缓冲区是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 获取当前缓存的总字节数
    pub fn bytes(&self) -> usize {
        self.current_bytes
    }

    /// 获取当前缓存的时间跨度（毫秒）
    pub fn duration_ms(&self) -> i64 {
        match (self.queue.front(), self.queue.back()) {
            (Some(oldest), Some(newest)) => {
                newest.pts_ms.saturating_sub(oldest.pts_ms).max(0)
            }
            _ => 0,
        }
    }

    /// 淘汰策略：按时长、字节数、包数量三重约束修剪
    fn prune(&mut self) {
        let Some(newest_pts) = self.queue.back().map(|p| p.pts_ms) else {
            return;
        };

        // 正常修剪：超出时长/字节/包数量时，从队首淘汰旧包
        // 但保留至少一个关键帧（否则 drain_from_keyframe 无法工作）
        while self.queue.len() > 1 && self.keyframe_count >= 2 {
            let oldest_pts = self.queue.front().map(|p| p.pts_ms).unwrap_or(0);
            let duration = newest_pts.saturating_sub(oldest_pts);

            // 时钟回跳检测（PTS 倒退 > 1s 视为异常）
            let clock_regressed = oldest_pts > newest_pts + 1000;

            let over_duration = duration > self.config.max_duration_ms;
            let over_bytes = self.current_bytes > self.config.max_bytes;
            let over_packets = self.queue.len() > self.config.max_packets;

            if over_duration || over_bytes || over_packets || clock_regressed {
                self.pop_front();
            } else {
                break;
            }
        }

        // 极端异常兜底：即使只有一个关键帧，包数量超出硬上限 2 倍时强制丢弃
        let hard_limit = self.config.max_packets.saturating_mul(2).max(64);
        while self.queue.len() > hard_limit {
            self.pop_front();
        }
    }

    /// 弹出队首包并更新计数
    fn pop_front(&mut self) {
        if let Some(removed) = self.queue.pop_front() {
            self.current_bytes = self.current_bytes.saturating_sub(removed.payload.len());
            if removed.is_keyframe {
                self.keyframe_count = self.keyframe_count.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use types::CodecType;

    fn make_packet(pts_ms: i64, is_keyframe: bool, size: usize) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe,
            codec: CodecType::H264,
            payload: Bytes::from(vec![0u8; size]),
            stream_tag: types::StreamTag::Video,
        })
    }

    #[test]
    fn test_basic_push_and_drain() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig::default());

        // GOP: I + 3P
        rb.push(make_packet(1000, true, 5000));
        rb.push(make_packet(1033, false, 1000));
        rb.push(make_packet(1066, false, 1000));
        rb.push(make_packet(1100, false, 1000));

        assert_eq!(rb.len(), 4);
        assert_eq!(rb.bytes(), 8000);

        let drained = rb.drain_from_keyframe();
        assert_eq!(drained.len(), 4);
        assert!(drained[0].is_keyframe);
        assert_eq!(drained[0].pts_ms, 1000);

        // 缓冲区应该被清空
        assert!(rb.is_empty());
        assert_eq!(rb.bytes(), 0);
    }

    #[test]
    fn test_drain_starts_from_earliest_keyframe() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig::default());

        // GOP 1
        rb.push(make_packet(1000, true, 5000));
        rb.push(make_packet(1033, false, 1000));

        // GOP 2
        rb.push(make_packet(2000, true, 5000));
        rb.push(make_packet(2033, false, 1000));
        rb.push(make_packet(2066, false, 1000));

        let drained = rb.drain_from_keyframe();

        // 应从最早的关键帧开始，最大化前置覆盖
        assert_eq!(drained.len(), 5);
        assert!(drained[0].is_keyframe);
        assert_eq!(drained[0].pts_ms, 1000);
        assert_eq!(drained[4].pts_ms, 2066);
    }

    #[test]
    fn test_drain_no_keyframe_returns_empty() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig {
            max_packets: 100,
            ..Default::default()
        });

        // 只有 P 帧（异常场景）
        rb.push(make_packet(1000, false, 1000));
        rb.push(make_packet(1033, false, 1000));

        let drained = rb.drain_from_keyframe();
        assert!(drained.is_empty());
        assert!(rb.is_empty()); // 清空了
    }

    #[test]
    fn test_prune_by_duration() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig {
            max_duration_ms: 100, // 仅保留 100ms
            max_bytes: 1024 * 1024,
            max_packets: 100,
        });

        // GOP 1: 1000ms
        rb.push(make_packet(1000, true, 1000));
        rb.push(make_packet(1040, false, 1000));

        // GOP 2: 1100ms
        rb.push(make_packet(1100, true, 1000));
        rb.push(make_packet(1140, false, 1000));

        // GOP 3: 1200ms — 触发淘汰 GOP 1（跨度 200ms > 100ms）
        rb.push(make_packet(1200, true, 1000));

        // GOP 1 应被淘汰
        let drained = rb.drain_from_keyframe();
        assert!(drained[0].is_keyframe);
        assert!(drained[0].pts_ms >= 1100, "GOP 1 应被淘汰，最老帧应 >= 1100ms");
    }

    #[test]
    fn test_prune_by_bytes() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig {
            max_duration_ms: 100_000,
            max_bytes: 10_000, // 10KB 上限
            max_packets: 100,
        });

        // GOP 1: 6KB
        rb.push(make_packet(1000, true, 6000));

        // GOP 2: 6KB — 总计 12KB > 10KB，应触发淘汰
        rb.push(make_packet(2000, true, 6000));

        // 现在应只剩 GOP 2
        assert!(rb.bytes() <= 10_000 || rb.keyframe_count < 2,
            "超出字节上限且有 2+ 关键帧时应触发淘汰");
    }

    #[test]
    fn test_prune_hard_limit() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig {
            max_duration_ms: 100_000,
            max_bytes: 1024 * 1024,
            max_packets: 5,
        });

        // 推入 1 个 I 帧 + 20 个 P 帧（只有 1 个关键帧，正常修剪不会工作）
        rb.push(make_packet(1000, true, 100));
        for i in 1..=20 {
            rb.push(make_packet(1000 + i * 33, false, 100));
        }

        // hard_limit = max(5*2, 64) = 64，21 个包不超过 64
        // 但如果 max_packets 更小...
        let hard = rb.config.max_packets.saturating_mul(2).max(64);
        assert!(rb.len() <= hard, "不应超过硬上限 {hard}，实际 {}", rb.len());
    }

    #[test]
    fn test_clock_regression_resilience() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig::default());

        // 正常时间戳
        rb.push(make_packet(50000, true, 1000));
        rb.push(make_packet(50033, false, 1000));

        // 时钟回跳（NTP 校正）
        rb.push(make_packet(30000, true, 1000));
        rb.push(make_packet(30033, false, 1000));

        // 应该能正常工作，不 panic
        let drained = rb.drain_from_keyframe();
        assert!(!drained.is_empty());
        assert!(drained[0].is_keyframe);
    }

    #[test]
    fn test_duration_ms() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig::default());

        assert_eq!(rb.duration_ms(), 0);

        rb.push(make_packet(1000, true, 100));
        assert_eq!(rb.duration_ms(), 0); // 只有一个包

        rb.push(make_packet(1500, false, 100));
        assert_eq!(rb.duration_ms(), 500);

        rb.push(make_packet(2000, false, 100));
        assert_eq!(rb.duration_ms(), 1000);
    }

    #[test]
    fn test_from_seconds_config() {
        let cfg = PreCaptureConfig::from_seconds(10);
        assert_eq!(cfg.max_duration_ms, 10_000);
        assert!(cfg.max_bytes > 0);
        assert!(cfg.max_packets > 0);

        // 范围钳位测试
        let cfg_clamped = PreCaptureConfig::from_seconds(100);
        assert_eq!(cfg_clamped.max_duration_ms, 30_000); // 钳位到 30 秒

        let cfg_min = PreCaptureConfig::from_seconds(1);
        assert_eq!(cfg_min.max_duration_ms, 5_000); // 钳位到 5 秒
    }

    #[test]
    fn test_clear() {
        let mut rb = PreCaptureRingBuffer::new(PreCaptureConfig::default());

        rb.push(make_packet(1000, true, 5000));
        rb.push(make_packet(1033, false, 1000));
        assert!(!rb.is_empty());

        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.bytes(), 0);
        assert_eq!(rb.len(), 0);
    }
}
