//! 时序颜色方差验证器：通过检测候选框区域的帧间颜色跳变程度过滤稳定光源误报
//!
//! 核心思路：真实火焰/烟雾在连续帧中颜色快速变化（闪烁/扩散），
//! 而太阳光、灯光等稳定光源颜色几乎不变。对候选框区域计算帧间颜色
//! 方差，低于阈值则判定为误报。

use std::collections::VecDeque;

/// 单帧候选检测框的颜色摘要
#[derive(Debug, Clone, Copy)]
struct RegionSnapshot {
    /// 区域内像素灰度均值（足以衡量颜色稳定性）
    mean_luminance: f32,
}

/// 时序验证器
#[derive(Debug)]
pub struct TemporalVerifier {
    /// 滑动窗口：按 class_id 分别维护
    buffers: [VecDeque<RegionSnapshot>; 2],
    /// 滑动窗口帧数
    window_size: usize,
    /// 颜色方差阈值（低于此值判定为稳定光源）
    variance_threshold: f32,
}

impl TemporalVerifier {
    pub fn new(window_size: usize, variance_threshold: f32) -> Self {
        Self {
            buffers: [
                VecDeque::with_capacity(window_size),
                VecDeque::with_capacity(window_size),
            ],
            window_size: window_size.max(1),
            variance_threshold,
        }
    }

    /// 推入一帧的候选检测结果，返回通过时序验证的 (class_id, confidence) 列表
    pub fn verify_frame(
        &mut self,
        detections: &[(usize, f32, f32)], // (class_id, confidence, mean_luminance)
    ) -> Vec<(usize, f32)> {
        let mut confirmed = Vec::new();

        // 按 class_id 分组收集
        let mut by_class: [Vec<(f32, f32)>; 2] = [Vec::new(), Vec::new()];
        for &(cls, conf, lum) in detections {
            if cls < 2 {
                by_class[cls].push((conf, lum));
            }
        }

        for (cls, class_dets) in by_class.iter().enumerate() {
            if class_dets.is_empty() {
                // 本帧该类别无检测，推入零值保持窗口滑动
                self.buffers[cls].push_back(RegionSnapshot {
                    mean_luminance: 0.0,
                });
                if self.buffers[cls].len() > self.window_size {
                    self.buffers[cls].pop_front();
                }
                continue;
            }

            // 取该类别最高置信度检测的亮度
            let best = class_dets
                .iter()
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .expect("class_dets 非空，max_by 必定返回 Some");

            self.buffers[cls].push_back(RegionSnapshot {
                mean_luminance: best.1,
            });
            if self.buffers[cls].len() > self.window_size {
                self.buffers[cls].pop_front();
            }

            // 窗口内帧数不足时放行（宁可多报不漏报）
            if self.buffers[cls].len() < self.window_size {
                confirmed.push((cls, best.0));
                continue;
            }

            // 计算窗口内亮度方差
            let variance = compute_luminance_variance(&self.buffers[cls]);
            if variance > self.variance_threshold {
                confirmed.push((cls, best.0));
            }
        }

        confirmed
    }
}

/// 计算滑动窗口内亮度值的方差
fn compute_luminance_variance(buffer: &VecDeque<RegionSnapshot>) -> f32 {
    let n = buffer.len();
    if n < 2 {
        return f32::INFINITY; // 帧数不足，返回无穷大（放行）
    }

    let mean: f32 = buffer.iter().map(|s| s.mean_luminance).sum::<f32>() / n as f32;
    let variance: f32 = buffer
        .iter()
        .map(|s| {
            let diff = s.mean_luminance - mean;
            diff * diff
        })
        .sum::<f32>()
        / n as f32;

    variance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stable_source_filtered_after_window_filled() {
        let mut verifier = TemporalVerifier::new(5, 50.0);
        // 前 4 帧：窗口未满，放行（宁可多报不漏报）
        for _ in 0..4 {
            let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
            assert_eq!(result.len(), 1, "窗口未满时应放行");
        }
        // 第 5 帧起：窗口满，稳定光源方差 = 0 < 50.0，应被过滤
        let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
        assert!(result.is_empty(), "稳定光源应被过滤");
        // 第 6 帧仍然稳定
        let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
        assert!(result.is_empty(), "持续稳定光源应继续被过滤");
    }

    #[test]
    fn test_flickering_fire_confirmed() {
        // 连续 5 帧亮度剧烈跳变（真实火焰闪烁）
        let luminances = [180.0, 120.0, 220.0, 90.0, 250.0];
        let mut verifier = TemporalVerifier::new(5, 50.0);
        let mut confirmed = false;
        for &lum in &luminances {
            let result = verifier.verify_frame(&[(0, 0.9, lum)]);
            if !result.is_empty() {
                confirmed = true;
            }
        }
        assert!(confirmed, "闪烁火焰应通过验证");
    }

    #[test]
    fn test_insufficient_frames_always_pass() {
        // 帧数不足窗口大小时放行
        let mut verifier = TemporalVerifier::new(5, 50.0);
        for _ in 0..3 {
            let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
            assert_eq!(result.len(), 1, "帧数不足时应放行");
        }
    }

    #[test]
    fn test_two_classes_independent() {
        let mut verifier = TemporalVerifier::new(5, 50.0);
        // fire 稳定，smoke 跳变
        for &lum in &[200.0, 200.0, 200.0, 200.0, 200.0] {
            verifier.verify_frame(&[(0, 0.9, lum)]);
        }
        // fire 应被过滤
        let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
        assert!(result.iter().all(|(c, _)| *c != 0), "fire 应被过滤");

        // smoke 跳变
        for &lum in &[100.0, 200.0, 80.0, 220.0, 150.0] {
            verifier.verify_frame(&[(1, 0.8, lum)]);
        }
        let result = verifier.verify_frame(&[(1, 0.8, 180.0)]);
        assert!(result.iter().any(|(c, _)| *c == 1), "smoke 应通过");
    }

    #[test]
    fn test_zero_variance_threshold_rejects_stable() {
        let mut verifier = TemporalVerifier::new(5, 0.0);
        // 前 4 帧窗口未满放行
        for _ in 0..4 {
            let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
            assert_eq!(result.len(), 1, "窗口未满放行");
        }
        // 第 5 帧起：方差 = 0，不大于 threshold = 0，应被过滤
        let result = verifier.verify_frame(&[(0, 0.9, 200.0)]);
        assert!(result.is_empty(), "方差为 0 不大于阈值 0，应过滤");
    }
}
