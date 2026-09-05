//! 主码流 NALU 内存环形队列 (Ring Buffer)
//!
//! 专用于主码流高分辨率 (1080P/4K) 压缩数据包的有界缓存：
//! 1. 仅存原始 NALU (零 VPU / CPU 解码开销，内存仅 10~20MB)；
//! 2. 保留最近 2~3.5 秒的完整 GOP 链；
//! 3. 告警发生时，按时标 T 精准提取起始于前置 I 帧的切片序列，供靶向快进解码。

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use types::EncodedPacket;

/// 主码流环形队列配置
#[derive(Debug, Clone)]
pub struct RingBufferConfig {
    /// 最大缓存时长（毫秒，默认 3500ms，即 3.5秒）
    pub max_duration_ms: i64,
    /// 最大包数量上限（防爆，默认 150 包）
    pub max_packets: usize,
}

impl Default for RingBufferConfig {
    fn default() -> Self {
        Self {
            max_duration_ms: 3500,
            max_packets: 150,
        }
    }
}

/// 主码流内存环形队列
#[derive(Debug)]
pub struct MainStreamRingBuffer {
    config: RingBufferConfig,
    queue: RwLock<VecDeque<Arc<EncodedPacket>>>,
}

impl MainStreamRingBuffer {
    pub fn new(config: RingBufferConfig) -> Self {
        Self {
            config,
            queue: RwLock::new(VecDeque::with_capacity(128)),
        }
    }

    /// 向环形队列压入一个主码流压缩数据包
    pub fn push(&self, packet: Arc<EncodedPacket>) {
        let mut queue = self.queue.write().unwrap_or_else(|e| e.into_inner());
        queue.push_back(packet);

        // 执行按容量与时长的智能修剪
        self.prune(&mut queue);
    }

    /// 根据指定时标精确提取从前置关键帧开始直至目标帧的完整 GOP 序列
    ///
    /// 保证返回的切片以关键帧 (I 帧) 为首包，后续帧按 PTS 单调递增，供快进解码。
    pub fn get_gop_for_timestamp(&self, target_pts_ms: i64) -> Option<Vec<Arc<EncodedPacket>>> {
        let queue = self.queue.read().unwrap_or_else(|e| e.into_inner());
        if queue.is_empty() {
            return None;
        }

        // 1. 查找最接近目标时间戳的包索引
        let target_idx = queue
            .iter()
            .enumerate()
            .min_by_key(|(_, pkt)| (pkt.pts_ms - target_pts_ms).abs())
            .map(|(idx, _)| idx)?;

        // 2. 从 target_idx 向前倒序寻找最近的关键帧 (I-Frame)
        let keyframe_idx = (0..=target_idx).rev().find(|&idx| queue[idx].is_keyframe)?;

        // 3. 截取 [keyframe_idx..=target_idx] 范围内的全部包
        Some(queue.range(keyframe_idx..=target_idx).cloned().collect())
    }

    /// 根据时标向后查找最近的前置关键帧 (I-Frame)
    pub fn find_prior_keyframe(&self, target_pts_ms: i64) -> Option<Arc<EncodedPacket>> {
        let queue = self.queue.read().unwrap_or_else(|e| e.into_inner());
        if queue.is_empty() {
            return None;
        }

        let target_idx = queue
            .iter()
            .enumerate()
            .min_by_key(|(_, pkt)| (pkt.pts_ms - target_pts_ms).abs())
            .map(|(idx, _)| idx)?;

        let keyframe_idx = (0..=target_idx).rev().find(|&idx| queue[idx].is_keyframe)?;
        Some(queue[keyframe_idx].clone())
    }

    /// 获取当前队列中最老的数据包时间戳
    pub fn oldest_pts(&self) -> Option<i64> {
        self.queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .front()
            .map(|p| p.pts_ms)
    }

    /// 获取当前队列中最新的数据包时间戳
    pub fn newest_pts(&self) -> Option<i64> {
        self.queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .back()
            .map(|p| p.pts_ms)
    }

    /// 获取当前队列中最新的视频编码格式
    pub fn latest_codec(&self) -> Option<types::CodecType> {
        self.queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .back()
            .map(|p| p.codec)
    }

    /// 获取当前队列中包的数量
    pub fn len(&self) -> usize {
        self.queue.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// 检查队列是否为空
    pub fn is_empty(&self) -> bool {
        self.queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// 清空队列
    pub fn clear(&self) {
        self.queue
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// 队列修剪逻辑：保持最新 2~3.5 秒，且始终保全最近的完整 GOP
    fn prune(&self, queue: &mut VecDeque<Arc<EncodedPacket>>) {
        if queue.is_empty() {
            return;
        }

        let newest_pts = match queue.back() {
            Some(p) => p.pts_ms,
            None => return,
        };

        let mut keyframe_count = queue.iter().filter(|p| p.is_keyframe).count();

        // 当超出时间跨度或达到包上限，并且队列里至少有两个关键帧时，可以安全丢弃老关键帧及其之前的包
        while queue.len() > 1 && keyframe_count >= 2 {
            let oldest_pts = queue.front().map(|p| p.pts_ms).unwrap_or(0);
            let duration = newest_pts - oldest_pts;

            if duration > self.config.max_duration_ms || queue.len() > self.config.max_packets {
                if let Some(removed) = queue.pop_front() {
                    if removed.is_keyframe {
                        keyframe_count = keyframe_count.saturating_sub(1);
                    }
                }
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use types::CodecType;

    fn make_packet(pts_ms: i64, is_keyframe: bool) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe,
            codec: CodecType::H264,
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x67fake_nalu"),
        })
    }

    #[test]
    fn test_ring_buffer_push_and_query_gop() {
        let rb = MainStreamRingBuffer::new(RingBufferConfig {
            max_duration_ms: 3000,
            max_packets: 50,
        });

        // 模拟推入一个 GOP: 1 个 I 帧 + 4 个 P 帧 (40ms 一帧)
        rb.push(make_packet(1000, true)); // I
        rb.push(make_packet(1040, false)); // P
        rb.push(make_packet(1080, false)); // P
        rb.push(make_packet(1120, false)); // P
        rb.push(make_packet(1160, false)); // P

        // 查询时标 1080ms 的 GOP
        let gop = rb.get_gop_for_timestamp(1080).expect("should find gop");
        assert_eq!(gop.len(), 3); // 1000(I), 1040(P), 1080(P)
        assert!(gop[0].is_keyframe);
        assert_eq!(gop[0].pts_ms, 1000);
        assert_eq!(gop[2].pts_ms, 1080);
    }

    #[test]
    fn test_ring_buffer_prune_preserves_latest_gop() {
        let rb = MainStreamRingBuffer::new(RingBufferConfig {
            max_duration_ms: 100, // 仅保留 100ms
            max_packets: 10,
        });

        // GOP 1
        rb.push(make_packet(1000, true));
        rb.push(make_packet(1040, false));
        rb.push(make_packet(1080, false));

        // GOP 2
        rb.push(make_packet(1120, true));
        rb.push(make_packet(1160, false));
        rb.push(make_packet(1200, false));

        // GOP 3 (新关键帧触发了老 GOP 1 的安全淘汰)
        rb.push(make_packet(1240, true));
        rb.push(make_packet(1280, false));

        // 验证 GOP 3 完整保留
        let gop = rb.get_gop_for_timestamp(1280).expect("should find gop 3");
        assert_eq!(gop[0].pts_ms, 1240);
        assert!(gop[0].is_keyframe);
        assert_eq!(gop[1].pts_ms, 1280);
    }

    #[test]
    fn test_ring_buffer_find_prior_keyframe() {
        let rb = MainStreamRingBuffer::new(RingBufferConfig::default());
        // GOP 1: 1000(I), 1040(P)
        rb.push(make_packet(1000, true));
        rb.push(make_packet(1040, false));
        // GOP 2: 2000(I), 2040(P), 2080(P)
        rb.push(make_packet(2000, true));
        rb.push(make_packet(2040, false));
        rb.push(make_packet(2080, false));

        // 查找 1040 对应的关键帧 -> 1000
        let kf1 = rb.find_prior_keyframe(1040).expect("should find kf1");
        assert_eq!(kf1.pts_ms, 1000);

        // 查找 2060 对应的关键帧 -> 2000
        let kf2 = rb.find_prior_keyframe(2060).expect("should find kf2");
        assert_eq!(kf2.pts_ms, 2000);
    }
}
