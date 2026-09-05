//! 纯 Rust 多目标航迹关联跟踪器 (IoU Association Tracker)
//!
//! 1. 基于空间重合度 (IoU) 与多层级二分图贪婪关联算法；
//! 2. 连续稳定维护目标的全局单调递增 `track_id`；
//! 3. 记录维护有界历史移动轨迹向量 (Trajectory，保留最近 30 点)；
//! 4. 目标遮挡/短暂丢失时支持最大 30 帧容忍期，防抖防丢；
//! 5. 航迹消亡时同步级联清理报警冷却映射，保证常驻内存严格有界。

use std::collections::{HashMap, HashSet};
use types::{BoundingBox, Detection, DetectionRuleRole, TrackedObject};

/// 航迹跟踪内部状态
#[derive(Debug, Clone)]
struct TrackState {
    track_id: u64,
    class_id: usize,
    label: String,
    confidence: f32,
    bbox: BoundingBox,
    trajectory: Vec<(f64, f64)>,
    lost_frames: usize,
}

/// 规则报警防刷屏冷却目标键（零堆内存分配）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CooldownTarget {
    Rule(usize, u8),
    Named(String),
}

/// 纯 Rust 航迹跟踪器
#[derive(Debug)]
pub struct SimpleTracker {
    next_track_id: u64,
    tracks: Vec<TrackState>,
    max_lost_frames: usize,
    iou_threshold: f32,
    max_trajectory_len: usize,
    /// 规则报警防刷屏冷却映射表 ((track_id, CooldownTarget) -> last_alarm_ms)
    alarm_cooldowns: HashMap<(u64, CooldownTarget), i64>,
}

impl Default for SimpleTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleTracker {
    pub fn new() -> Self {
        Self {
            next_track_id: 1,
            tracks: Vec::new(),
            max_lost_frames: 30, // 约 1.2 秒无回包后注销
            iou_threshold: 0.3,
            max_trajectory_len: 30,
            alarm_cooldowns: HashMap::new(),
        }
    }

    /// 输入当前帧检测列表，执行航迹更新并输出当前活跃的 TrackedObject
    pub fn update(&mut self, detections: Vec<Detection>) -> Vec<TrackedObject> {
        let num_tracks = self.tracks.len();
        let num_dets = detections.len();

        let mut matched_tracks = vec![false; num_tracks];
        let mut matched_dets = vec![false; num_dets];

        // 1. 贪婪匹配现有航迹与当前帧检测
        if num_tracks > 0 && num_dets > 0 {
            // 计算所有候选对的 IoU
            let mut matches: Vec<(usize, usize, f32)> = Vec::with_capacity(num_tracks * num_dets);
            for (t_idx, track) in self.tracks.iter().enumerate() {
                for (d_idx, det) in detections.iter().enumerate() {
                    // 同一标签或类别优先匹配
                    if track.class_id == det.class_id {
                        let iou = compute_iou(&track.bbox, &det.bbox);
                        if iou >= self.iou_threshold {
                            matches.push((t_idx, d_idx, iou));
                        }
                    }
                }
            }

            // 按 IoU 降序排列
            matches.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

            for (t_idx, d_idx, _) in matches {
                if !matched_tracks[t_idx] && !matched_dets[d_idx] {
                    matched_tracks[t_idx] = true;
                    matched_dets[d_idx] = true;

                    // 更新航迹状态
                    let det = &detections[d_idx];
                    let track = &mut self.tracks[t_idx];
                    track.bbox = det.bbox;
                    track.confidence = det.confidence;
                    track.lost_frames = 0;

                    let bottom_center = det.bbox.bottom_center();
                    track.trajectory.push(bottom_center);
                    if track.trajectory.len() > self.max_trajectory_len {
                        track.trajectory.remove(0);
                    }
                }
            }
        }

        // 2. 未匹配的检测作为新航迹初始化
        for (d_idx, det) in detections.into_iter().enumerate() {
            if !matched_dets[d_idx] {
                let track_id = self.next_track_id;
                self.next_track_id += 1;

                let bottom_center = det.bbox.bottom_center();
                self.tracks.push(TrackState {
                    track_id,
                    class_id: det.class_id,
                    label: det.label,
                    confidence: det.confidence,
                    bbox: det.bbox,
                    trajectory: vec![bottom_center],
                    lost_frames: 0,
                });
            }
        }

        // 3. 未匹配的航迹增加丢失计数
        for (t_idx, matched) in matched_tracks.into_iter().enumerate() {
            if !matched {
                self.tracks[t_idx].lost_frames += 1;
            }
        }

        // 4. 清理超期航迹
        let max_lost = self.max_lost_frames;
        self.tracks.retain(|t| t.lost_frames <= max_lost);

        // 同步级联清理已注销航迹的报警冷却记录，恪守有界内存契约，杜绝无界增长
        if !self.alarm_cooldowns.is_empty() {
            let active_ids: HashSet<u64> = self.tracks.iter().map(|t| t.track_id).collect();
            self.alarm_cooldowns
                .retain(|(tid, _), _| active_ids.contains(tid));
        }

        // 5. 产出当前存活的活跃航迹
        self.tracks
            .iter()
            .filter(|t| t.lost_frames == 0)
            .map(|t| TrackedObject {
                track_id: t.track_id,
                class_id: t.class_id,
                label: t.label.clone(),
                confidence: t.confidence,
                bbox: t.bbox,
                trajectory: t.trajectory.clone(),
            })
            .collect()
    }

    /// 基于规则索引与角色执行防刷屏判定（零堆内存分配）
    pub fn check_and_mark_rule(
        &mut self,
        track_id: u64,
        rule_idx: usize,
        role: DetectionRuleRole,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        let role_code = match role {
            DetectionRuleRole::Roi => 1,
            DetectionRuleRole::Line => 2,
            DetectionRuleRole::Mask => 3,
        };
        self.check_and_mark_cooldown(
            track_id,
            CooldownTarget::Rule(rule_idx, role_code),
            now_ms,
            cooldown_ms,
        )
    }

    /// 检查指定 track_id 是否可以在给定命名规则下触发报警（兼容旧命名调用）
    pub fn check_and_mark_alarm(
        &mut self,
        track_id: u64,
        rule_id: &str,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        self.check_and_mark_cooldown(
            track_id,
            CooldownTarget::Named(rule_id.to_string()),
            now_ms,
            cooldown_ms,
        )
    }

    fn check_and_mark_cooldown(
        &mut self,
        track_id: u64,
        target: CooldownTarget,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        let key = (track_id, target);
        if let Some(&last_time) = self.alarm_cooldowns.get(&key) {
            if now_ms - last_time < cooldown_ms {
                return false;
            }
        }
        self.alarm_cooldowns.insert(key, now_ms);
        true
    }
}

/// 计算两边界框的交并比 (IoU)
fn compute_iou(a: &BoundingBox, b: &BoundingBox) -> f32 {
    let inter_x1 = a.x1.max(b.x1);
    let inter_y1 = a.y1.max(b.y1);
    let inter_x2 = a.x2.min(b.x2);
    let inter_y2 = a.y2.min(b.y2);

    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_a = (a.x2 - a.x1).max(0.0) * (a.y2 - a.y1).max(0.0);
    let area_b = (b.x2 - b.x1).max(0.0) * (b.y2 - b.y1).max(0.0);

    let union_area = area_a + area_b - inter_area;
    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_continuous_tracking() {
        let mut tracker = SimpleTracker::new();

        // Frame 1: 目标在 [0.1, 0.1, 0.2, 0.2]
        let dets1 = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            bbox: BoundingBox::new(0.1, 0.1, 0.2, 0.2),
        }];
        let res1 = tracker.update(dets1);
        assert_eq!(res1.len(), 1);
        let track_id = res1[0].track_id;

        // Frame 2: 目标微移到 [0.12, 0.12, 0.22, 0.22] (高重叠)
        let dets2 = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.91,
            bbox: BoundingBox::new(0.12, 0.12, 0.22, 0.22),
        }];
        let res2 = tracker.update(dets2);
        assert_eq!(res2.len(), 1);
        assert_eq!(res2[0].track_id, track_id, "Track ID 必须在帧间保持连续！");
        assert_eq!(res2[0].trajectory.len(), 2, "轨迹历史记录递增");
    }

    #[test]
    fn test_tracker_alarm_cooldown() {
        let mut tracker = SimpleTracker::new();
        let dets = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            bbox: BoundingBox::new(0.3, 0.3, 0.5, 0.5),
        }];
        let res = tracker.update(dets);
        let tid = res[0].track_id;

        // 第一次触发：成功
        assert!(tracker.check_and_mark_alarm(tid, "rule_line_1", 1000, 5000));

        // 2 秒后再次触发：应被 5 秒冷却拦截
        assert!(!tracker.check_and_mark_alarm(tid, "rule_line_1", 3000, 5000));

        // 6 秒后（距离首次 1000 超过 5000ms）：允许再次触发
        assert!(tracker.check_and_mark_alarm(tid, "rule_line_1", 6001, 5000));
    }

    #[test]
    fn test_tracker_alarm_cooldown_memory_reclamation() {
        let mut tracker = SimpleTracker::new();
        let dets = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            bbox: BoundingBox::new(0.3, 0.3, 0.5, 0.5),
        }];
        let res = tracker.update(dets);
        let tid = res[0].track_id;

        assert!(tracker.check_and_mark_rule(tid, 0, DetectionRuleRole::Roi, 1000, 5000));
        assert_eq!(tracker.alarm_cooldowns.len(), 1);

        // 连续空包 31 帧，导致该目标航迹超期注销
        for _ in 0..31 {
            let _ = tracker.update(vec![]);
        }

        // 验证已注销航迹的报警冷却记录已被自动级联清理，避免无界内存泄漏
        assert_eq!(tracker.alarm_cooldowns.len(), 0);
    }
}
