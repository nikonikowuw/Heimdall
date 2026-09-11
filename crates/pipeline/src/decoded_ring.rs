use std::collections::VecDeque;
use types::FrameRef;

/// 解码帧环形缓冲配置
///
/// 硬件约束说明：
/// 真实硬件环境下（如华为昇腾 `DvppBufferPool` 总池量为 20 块，Rockchip MPP 1080P 分配 16 块），
/// 原生 `FrameRef` 持有 DMA-BUF/显存池租约。若环形缓冲池容量过大，将直接抢占解码器 DPB 参考帧
/// 及下游推理流转显存，导致硬件解码器显存池耗尽超时。
/// 针对典型 25fps 下约 2 帧 (~80ms) 的推理匹配需求，默认上限 5 帧 (300ms) 既能确保容差检索命中，
/// 又严格留足空闲显存块供 VPU 与工作线程周转。
#[derive(Debug, Clone)]
pub struct DecodedRingConfig {
    /// 缓存时间窗口上限（毫秒，默认 300ms，在 25fps 下覆盖约 7 帧）
    pub window_duration_ms: i64,
    /// 缓存最大帧数上限（默认 5 帧，防止物理显存池枯竭）
    pub max_frames: usize,
}

impl Default for DecodedRingConfig {
    fn default() -> Self {
        Self {
            window_duration_ms: 300,
            max_frames: 5,
        }
    }
}

/// 零拷贝已解码视频帧环形缓冲池
///
/// 保留最近一段时间内已解码的原生高保真 `FrameRef`，用于方案三【零解码直通】快速检索。
/// 仅持有 `FrameRef` 智能指针（底层为 DMA-BUF / CVPixelBuffer 引用计数），绝不发生像素拷贝。
#[derive(Debug)]
pub struct DecodedFrameRingBuffer {
    config: DecodedRingConfig,
    frames: VecDeque<FrameRef>,
}

impl Default for DecodedFrameRingBuffer {
    fn default() -> Self {
        Self::new(DecodedRingConfig::default())
    }
}

impl DecodedFrameRingBuffer {
    pub fn new(config: DecodedRingConfig) -> Self {
        Self {
            frames: VecDeque::with_capacity(config.max_frames),
            config,
        }
    }

    /// 向环形队列推入最新解码帧（自动执行时序维护与双重水位淘汰）
    pub fn push(&mut self, frame: FrameRef) {
        let current_pts = frame.timestamp;

        // 1. 压入最新帧
        self.frames.push_back(frame);

        // 2. 按最大帧数硬限淘汰旧帧（RAII 自动释放 DMA-BUF / 显存引用）
        while self.frames.len() > self.config.max_frames {
            self.frames.pop_front();
        }

        // 3. 时钟回退与时标异常防护 (PTS 回绕 / NTP 跳变 > 1000ms)：
        // 若检测到队首时标大幅超前于当前帧，说明流发生了时钟重置或逆跳，清空旧时钟帧避免死帧堆积
        if let Some(front) = self.frames.front() {
            if front.timestamp.saturating_sub(current_pts) > 1000 {
                let latest = self.frames.pop_back();
                self.frames.clear();
                if let Some(l) = latest {
                    self.frames.push_back(l);
                }
                return;
            }
        }

        // 4. 按时间窗口上限淘汰超期帧 (使用 saturating_sub 避免算术溢出)
        while let Some(front) = self.frames.front() {
            if current_pts.saturating_sub(front.timestamp) > self.config.window_duration_ms {
                self.frames.pop_front();
            } else {
                break;
            }
        }
    }

    /// 环形队列缓存的时间窗口上限（毫秒）
    #[inline]
    pub fn window_duration_ms(&self) -> i64 {
        self.config.window_duration_ms
    }

    /// 环形队列最大帧数上限
    #[inline]
    pub fn max_frames(&self) -> usize {
        self.config.max_frames
    }

    /// 根据目标 PTS 检索最佳匹配帧
    ///
    /// - `tolerance_ms`：允许的最大绝对时间容差（毫秒）
    /// - 返回匹配的 `(FrameRef, diff_ms)`，未命中容差范围则返回 `None`
    pub fn find_by_pts(&self, target_pts: i64, tolerance_ms: i64) -> Option<(FrameRef, i64)> {
        if self.frames.is_empty() {
            return None;
        }

        let mut best_match: Option<(&FrameRef, i64)> = None;
        let mut min_diff = u64::MAX;

        // 从最新的帧向后反向遍历（因为告警目标帧通常为最近几帧）
        // 环形缓冲深度极小 (默认 <= 5 帧)，全量反向遍历仅需纳秒级，避免脆弱的提前剪枝假设因时标抖动漏配
        for frame in self.frames.iter().rev() {
            let raw_diff = frame.timestamp.abs_diff(target_pts);
            let diff = raw_diff.min(i64::MAX as u64) as i64;
            if best_match.is_none() || raw_diff < min_diff {
                min_diff = raw_diff;
                best_match = Some((frame, diff));
            }
            // 严格零偏差命中，立即短路返回
            if raw_diff == 0 {
                return Some((frame.clone(), 0));
            }
        }

        if tolerance_ms >= 0 && min_diff <= tolerance_ms as u64 {
            best_match.map(|(frame, diff)| (frame.clone(), diff))
        } else {
            None
        }
    }

    /// 获取当前缓冲区中最新的一帧
    pub fn latest_frame(&self) -> Option<FrameRef> {
        self.frames.back().cloned()
    }

    /// 当前缓存的帧数量
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// 缓冲区是否为空
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// 清空缓冲区（用于网络重连、码流中断或管线停止）
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FrameHandle, PixelFormat, StrideInfo};

    fn make_test_frame(pts: i64) -> FrameRef {
        FrameRef::new(
            "cam_test".to_string(),
            pts,
            1920,
            1080,
            StrideInfo::new(1920, 1080),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![0u8; 100].into()),
        )
    }

    #[test]
    fn test_decoded_ring_safe_hardware_defaults() {
        let config = DecodedRingConfig::default();
        // 确保默认帧数绝不超过硬件池安全上限 (MPP 16 / DVPP 20)
        assert!(
            config.max_frames <= 8,
            "默认帧数过多将耗尽硬件解码器显存池 (当前: {})",
            config.max_frames
        );
        assert!(config.window_duration_ms <= 500);
    }

    #[test]
    fn test_decoded_ring_capacity_and_time_eviction() {
        let config = DecodedRingConfig {
            window_duration_ms: 500,
            max_frames: 5,
        };
        let mut ring = DecodedFrameRingBuffer::new(config);

        // 压入 5 帧 (间隔 40ms)
        for i in 0..5 {
            ring.push(make_test_frame(1000 + i * 40));
        }
        assert_eq!(ring.len(), 5);

        // 压入第 6 帧，触发 max_frames 淘汰
        ring.push(make_test_frame(1200));
        assert_eq!(ring.len(), 5);
        assert_eq!(
            ring.frames
                .front()
                .expect("ring should have front")
                .timestamp,
            1040
        );

        // 压入一个大跳跃时标 (超过 500ms 窗口)，触发时间淘汰
        ring.push(make_test_frame(2000));
        assert_eq!(ring.len(), 1);
        assert_eq!(
            ring.latest_frame()
                .expect("ring should have latest")
                .timestamp,
            2000
        );
    }

    #[test]
    fn test_decoded_ring_clock_regression_resilience() {
        let mut ring = DecodedFrameRingBuffer::default();
        ring.push(make_test_frame(100_000));
        ring.push(make_test_frame(100_040));
        assert_eq!(ring.len(), 2);

        // 模拟时钟逆跳 (如流断开重连，PTS 重置回 1000)
        ring.push(make_test_frame(1000));
        assert_eq!(ring.len(), 1);
        assert_eq!(
            ring.latest_frame()
                .expect("ring should have latest frame")
                .timestamp,
            1000
        );
    }

    #[test]
    fn test_decoded_ring_find_by_pts_precision() {
        let mut ring = DecodedFrameRingBuffer::default();
        for pts in [1000, 1040, 1080, 1120, 1160] {
            ring.push(make_test_frame(pts));
        }

        // 1. 严格精确匹配
        let (frame, diff) = ring.find_by_pts(1080, 50).expect("应精确命中");
        assert_eq!(frame.timestamp, 1080);
        assert_eq!(diff, 0);

        // 2. 容差范围内模糊匹配 (目标 1090ms，最近 1080ms，偏差 10ms <= 20ms)
        let (frame, diff) = ring.find_by_pts(1090, 20).expect("应容差命中");
        assert_eq!(frame.timestamp, 1080);
        assert_eq!(diff, 10);

        // 3. 超出容差范围不匹配 (目标 1300ms，最近 1160ms，偏差 140ms > 50ms)
        assert!(ring.find_by_pts(1300, 50).is_none());

        // 4. 极值时间戳不应因 i64::MIN.abs() 溢出
        let mut extreme_ring = DecodedFrameRingBuffer::default();
        extreme_ring.push(make_test_frame(i64::MAX));
        let extreme_result = extreme_ring.find_by_pts(i64::MIN, i64::MAX);
        assert!(extreme_result.is_none());
    }
}
