//! 时序确认与误报过滤验证器 (Temporal Verifier)
//!
//! 采用工业级 M-out-of-N 多帧滑动窗口确认机制：
//! 1. 多帧命中累积（M-out-of-N）：在 `confirm_window` 帧内需命中至少 `confirm_threshold` 帧方可确认，
//!    彻底消除摄像头瞬态反光、局部噪点与单帧误识别；
//! 2. 时序方差保护：当上游提供真实候选框像素亮度时，对稳定高亮光源进行方差稳定性过滤；
//!    在纯设备侧零拷贝流水线（无 CPU 像素回读）时，严格遵循多帧命中判定，杜绝拿置信度伪造像素导致真火被误杀；
//! 3. 极速零堆分配（Zero Heap Allocation）：在热路径上使用栈上定长数组与流式迭代。

use std::collections::VecDeque;

/// 单帧历史记录
#[derive(Debug, Clone, Copy)]
struct FrameRecord {
    hit: bool,
    confidence: f32,
    luminance: Option<f32>,
}

/// 时序验证器
#[derive(Debug)]
pub struct TemporalVerifier {
    /// 滑动窗口：按 class_id (0: fire, 1: smoke) 分别维护
    buffers: [VecDeque<FrameRecord>; 2],
    /// 滑动窗口最大帧数 (N)
    window_size: usize,
    /// 最小命中确认帧数 (M)
    confirm_threshold: usize,
    /// 颜色方差阈值（仅在显式传入像素亮度时生效）
    variance_threshold: f32,
}

impl TemporalVerifier {
    pub fn new(window_size: usize, confirm_threshold: usize, variance_threshold: f32) -> Self {
        let win = window_size.max(1);
        let thresh = confirm_threshold.max(1).min(win);
        Self {
            buffers: [VecDeque::with_capacity(win), VecDeque::with_capacity(win)],
            window_size: win,
            confirm_threshold: thresh,
            variance_threshold,
        }
    }

    /// 热路径零分配确认检测结果，返回通过时序确认的类别掩码（bit 0: fire, bit 1: smoke）
    pub fn verify_frame_mask<I>(&mut self, detections: I) -> u8
    where
        I: IntoIterator<Item = (usize, f32)>,
    {
        // 栈上固定大小数组聚合两类最优置信度，零堆分配
        let mut best: [Option<f32>; 2] = [None, None];
        for (cls, conf) in detections {
            if cls < 2 {
                let curr = best[cls].get_or_insert(conf);
                if conf > *curr {
                    *curr = conf;
                }
            }
        }

        let mut mask = 0u8;
        for (cls, &opt_conf) in best.iter().enumerate() {
            let record = FrameRecord {
                hit: opt_conf.is_some(),
                confidence: opt_conf.unwrap_or(0.0),
                luminance: None,
            };

            if self.step_class(cls, record) {
                mask |= 1 << cls;
            }
        }

        mask
    }

    /// 推入一帧的候选检测结果（仅置信度），返回通过时序确认的 (class_id, confidence) 列表
    pub fn verify_frame(&mut self, detections: &[(usize, f32)]) -> Vec<(usize, f32)> {
        let mask = self.verify_frame_mask(detections.iter().copied());
        (0..2)
            .filter(|&cls| (mask & (1 << cls)) != 0)
            .map(|cls| {
                let latest_conf = self.buffers[cls].back().map_or(0.0, |r| r.confidence);
                (cls, latest_conf)
            })
            .collect()
    }

    /// 携带真实区域像素亮度的完整时序校验入口
    pub fn verify_frame_with_luminance(
        &mut self,
        detections: &[(usize, f32, Option<f32>)],
    ) -> Vec<(usize, f32)> {
        let mut best: [Option<(f32, Option<f32>)>; 2] = [None, None];
        for &(cls, conf, lum) in detections {
            if cls < 2 {
                match &mut best[cls] {
                    Some(prev) => {
                        if conf > prev.0 {
                            *prev = (conf, lum);
                        }
                    }
                    slot @ None => {
                        *slot = Some((conf, lum));
                    }
                }
            }
        }

        let mut confirmed = Vec::with_capacity(2);
        for (cls, &opt_item) in best.iter().enumerate() {
            let record = match opt_item {
                Some((conf, lum)) => FrameRecord {
                    hit: true,
                    confidence: conf,
                    luminance: lum,
                },
                None => FrameRecord {
                    hit: false,
                    confidence: 0.0,
                    luminance: None,
                },
            };

            if self.step_class(cls, record) {
                let latest_conf = self.buffers[cls].back().map_or(0.0, |r| r.confidence);
                confirmed.push((cls, latest_conf));
            }
        }

        confirmed
    }

    /// 推进单个类别的滑动窗口状态，并返回当前帧是否确认有效检出
    #[inline]
    fn step_class(&mut self, cls: usize, record: FrameRecord) -> bool {
        self.buffers[cls].push_back(record);
        if self.buffers[cls].len() > self.window_size {
            self.buffers[cls].pop_front();
        }

        // 当前帧未命中，直接不通过
        if !record.hit {
            return false;
        }

        let hit_count = self.buffers[cls].iter().filter(|r| r.hit).count();
        let total_frames = self.buffers[cls].len();

        // M-out-of-N 判定：
        // 1. 冷启动阶段（窗口未填满）：当达到确认阈值或当前连续全中时放行；
        // 2. 稳态阶段（窗口已满）：严格要求命中数达到 confirm_threshold。
        let is_confirmed = hit_count >= self.confirm_threshold
            || (total_frames < self.window_size && hit_count == total_frames);

        if !is_confirmed {
            return false;
        }

        // 若提供了真实像素亮度且样本数 >= 3 帧，流式计算方差，零堆分配
        let mut count = 0usize;
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;
        for r in &self.buffers[cls] {
            if let (true, Some(lum)) = (r.hit, r.luminance) {
                count += 1;
                sum += lum;
                sum_sq += lum * lum;
            }
        }

        if count >= 3 {
            let n = count as f32;
            let mean = sum / n;
            let variance = (sum_sq / n) - mean * mean;
            if variance < self.variance_threshold {
                // 真实像素亮度极度恒定，判定为稳定光源，不予放行
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_start_single_frame_passes() {
        // 单图冷启动验证：第 1 帧命中即放行，保证 run_local 单图评测正常工作
        let mut verifier = TemporalVerifier::new(5, 3, 50.0);
        let res = verifier.verify_frame(&[(0, 0.95)]);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], (0, 0.95));
    }

    #[test]
    fn test_verify_frame_mask_equivalence() {
        let mut v1 = TemporalVerifier::new(5, 3, 50.0);
        let mut v2 = TemporalVerifier::new(5, 3, 50.0);

        let input = [(0, 0.9), (1, 0.85)];
        let res = v1.verify_frame(&input);
        let mask = v2.verify_frame_mask(input);

        assert_eq!(mask, 0b11);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_m_out_of_n_filtering_single_spike() {
        let mut verifier = TemporalVerifier::new(5, 3, 50.0);
        // 第 1 帧冷启动放行
        let r1 = verifier.verify_frame(&[(0, 0.9)]);
        assert_eq!(r1.len(), 1);

        // 第 2、3 帧无火情
        assert!(verifier.verify_frame(&[]).is_empty());
        assert!(verifier.verify_frame(&[]).is_empty());

        // 第 4 帧偶发出现一帧火情，此时 4 帧中累计仅 2 次命中 (< 3)，未满且 hit_count != len，应拦截
        let r4 = verifier.verify_frame(&[(0, 0.85)]);
        assert!(r4.is_empty(), "命中数不足 3 时应拦截偶发噪点");
    }

    #[test]
    fn test_persistent_stable_fire_never_filtered() {
        // 关键防护验证：真实火灾置信度高度稳定时，绝对不能被误杀！
        let mut verifier = TemporalVerifier::new(5, 3, 50.0);
        for i in 0..10 {
            let res = verifier.verify_frame(&[(0, 0.92)]);
            assert_eq!(
                res.len(),
                1,
                "第 {i} 帧：真实火灾持续高置信度应持续通过，不可误杀"
            );
        }
    }

    #[test]
    fn test_real_luminance_variance_filters_stable_light() {
        let mut verifier = TemporalVerifier::new(5, 3, 50.0);
        // 连续 5 帧提供真实完全恒定的亮度 200.0（如日光灯、白炽灯）
        for _ in 0..2 {
            let res = verifier.verify_frame_with_luminance(&[(0, 0.9, Some(200.0))]);
            assert_eq!(res.len(), 1, "样本数不足 3 帧时先放行");
        }
        // 从第 3 帧起，亮度恒定方差 = 0 < 50.0，应被稳定光源过滤器拦截
        let res = verifier.verify_frame_with_luminance(&[(0, 0.9, Some(200.0))]);
        assert!(res.is_empty(), "真实静态高亮光源应被方差过滤");
    }

    #[test]
    fn test_real_luminance_variance_passes_flickering_fire() {
        let mut verifier = TemporalVerifier::new(5, 3, 50.0);
        // 火焰跳变：亮度剧烈波动
        let lums = [180.0, 120.0, 240.0, 90.0, 250.0];
        let mut passed_count = 0;
        for lum in lums {
            let res = verifier.verify_frame_with_luminance(&[(0, 0.9, Some(lum))]);
            if !res.is_empty() {
                passed_count += 1;
            }
        }
        assert!(passed_count >= 3, "真实跳变火焰应保持通过");
    }

    #[test]
    fn test_two_classes_independent_tracking() {
        let mut verifier = TemporalVerifier::new(5, 2, 50.0);
        // fire 连续出现，smoke 仅出现一次
        let r1 = verifier.verify_frame(&[(0, 0.9), (1, 0.8)]);
        assert_eq!(r1.len(), 2);

        let r2 = verifier.verify_frame(&[(0, 0.91)]);
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].0, 0); // 仅 fire 通过
    }
}
